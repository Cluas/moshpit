import CryptoKit
import Foundation

// MARK: - Errors

/// Why a pasted private key can't be imported — each one something the user
/// can act on. The reading is Rust's (`moshpit_ssh::keyfile`, through
/// ``KeyImportFailure``); the words are the app's.
enum SSHKeyImportError: Error, Equatable, LocalizedError {
    /// Not a private key this can read at all.
    case unrecognized
    /// A private key, but what's inside doesn't add up: cut short, or garbled.
    case damaged
    case passphraseRequired
    case wrongPassphrase
    /// A FIDO security key (`sk-ecdsa…`, `sk-ssh-ed25519…`): the file holds a
    /// handle, and the private half lives on the token.
    case securityKey
    /// A key type no connect path can sign with ("DSA", "Ed448", …).
    case unsupportedType(String)
    /// Encrypted with a scheme this can't undo.
    case unsupportedEncryption(String)
    /// Making or encoding the key failed.
    case failed(String)

    init(_ failure: KeyImportFailure) {
        switch failure {
        case .Unrecognized: self = .unrecognized
        case .Damaged: self = .damaged
        case .NeedsPassphrase: self = .passphraseRequired
        case .WrongPassphrase: self = .wrongPassphrase
        case .SecurityKey: self = .securityKey
        case .Unsupported(let kind): self = .unsupportedType(kind)
        case .UnsupportedEncryption(let scheme): self = .unsupportedEncryption(scheme)
        case .Failed(let message): self = .failed(message)
        }
    }

    var errorDescription: String? {
        switch self {
        case .unrecognized:
            return String(localized: "This isn't a private key Moshpit can read. Paste an OpenSSH or PEM private key, from its BEGIN line to its END line.")
        case .damaged:
            return String(localized: "This private key is damaged or incomplete. Copy it again, from its BEGIN line to its END line.")
        case .passphraseRequired:
            return String(localized: "This key is protected by a passphrase. Enter it to import the key.")
        case .wrongPassphrase:
            return String(localized: "That passphrase doesn't unlock this key.")
        case .securityKey:
            return String(localized: "Security keys (sk keys), such as a YubiKey's, aren't supported yet.")
        case .unsupportedType(let type):
            return String(localized: "Moshpit can't sign in with \(type) keys. Ed25519, RSA and ECDSA keys work.")
        case .unsupportedEncryption(let scheme):
            return String(localized: "This key is encrypted with \(scheme), which Moshpit can't unlock. Re-encrypt it with “ssh-keygen -p” and import it again.")
        case .failed(let why):
            // Not something a person can act on, as SSHKeyFactory.KeyError.
            return "Key generation failed: \(why)"
        }
    }
}

/// Runs a call into the key reader, its refusal as an ``SSHKeyImportError``.
private func keyReader<T>(_ call: () throws -> T) throws -> T {
    do {
        return try call()
    } catch let failure as KeyImportFailure {
        throw SSHKeyImportError(failure)
    }
}

// MARK: - Making and importing keys

/// Generates and imports SSH keys (prototype screen 9) — the reading,
/// writing and checking in Rust (`moshpit_ssh::keyfile`, the one key reader
/// every platform shares), the Secure Enclave's keys here.
///
/// Private keys are produced as OpenSSH-format text (or a Secure Enclave
/// key's data representation) and handed to `KeychainService` by the
/// caller — nothing here persists anything.
enum SSHKeyFactory {

    struct Generated {
        let algorithm: SSHKeyAlgorithm
        let privateBlob: Data      // OpenSSH PEM (utf8) or SE dataRepresentation
        let publicKeyLine: String  // authorized_keys line
        let fingerprint: String    // "SHA256:…"
        /// An RSA key's modulus size; nil for the others.
        var bits: Int?

        init(algorithm: SSHKeyAlgorithm, privateBlob: Data, publicKeyLine: String, fingerprint: String,
             bits: Int? = nil) {
            self.algorithm = algorithm
            self.privateBlob = privateBlob
            self.publicKeyLine = publicKeyLine
            self.fingerprint = fingerprint
            self.bits = bits
        }

        /// What the key reader made or read, as a record names it.
        fileprivate init(_ key: ImportedKey) throws {
            guard let algorithm = SSHKeyAlgorithm(rawValue: key.algorithm) else {
                throw SSHKeyImportError.unsupportedType(key.algorithm)
            }
            self.init(algorithm: algorithm, privateBlob: Data(key.openssh.utf8),
                      publicKeyLine: key.publicKeyLine, fingerprint: key.fingerprint,
                      bits: key.bits.map(Int.init))
        }
    }

    enum KeyError: LocalizedError {
        case unsupported(String)
        case generationFailed(String)

        var errorDescription: String? {
            switch self {
            case .unsupported(let what): return "Unsupported: \(what)"
            case .generationFailed(let why): return "Key generation failed: \(why)"
            }
        }
    }

    // MARK: - Import from file

    /// Decodes a picked file's raw bytes as UTF-8 text. The one part of
    /// file-based import pure enough to unit test — the URL / security-scope
    /// handling around it needs a real file on disk and stays in the view.
    static func decodeImportedText(_ data: Data) throws -> String {
        guard let text = String(data: data, encoding: .utf8) else {
            throw KeyError.unsupported("file is not valid UTF-8 text")
        }
        return text
    }

    // MARK: - Generate

    static func generate(algorithm: SSHKeyAlgorithm, comment: String) throws -> Generated {
        switch algorithm {
        case .ed25519:
            return try Generated(keyReader { try sshKeyGenerate(kind: .ed25519, comment: comment) })
        case .rsa4096:
            return try Generated(keyReader { try sshKeyGenerate(kind: .rsa(bits: 4096), comment: comment) })
        case .seP256:
            return try generateSecureEnclaveP256(comment: comment)
        case .ecdsaSK:
            throw KeyError.unsupported("security keys (sk keys) aren't supported yet")
        case .ecdsa:
            throw KeyError.unsupported("ECDSA keys are imported, not generated here.")
        }
    }

    /// Whether a software P-256 key may stand in when the Secure Enclave
    /// can't make one: only on the simulator, which has no enclave, so the
    /// flow can be walked there.
    #if targetEnvironment(simulator)
    static let enclaveStandIn = true
    #else
    static let enclaveStandIn = false
    #endif

    /// A P-256 key in the Secure Enclave, whose private half never leaves
    /// it; its public half becomes the record's line and fingerprint.
    private static func generateSecureEnclaveP256(comment: String) throws -> Generated {
        try generateSecureEnclaveP256(comment: comment, make: { try SecureEnclave.P256.Signing.PrivateKey() },
                                      standIn: enclaveStandIn)
    }

    /// ``generateSecureEnclaveP256(comment:)`` with the enclave's key made
    /// by `make`; a software key stands in when it can't only while
    /// `standIn` (the simulator).
    ///
    /// B604: a software key stood in wherever the enclave couldn't make
    /// one, on a phone too, and was saved as a Secure Enclave key — "SE ·
    /// P256", the page saying its private half never leaves the chip. It
    /// was a key in the Keychain, and it could never sign: signing reads
    /// the stored bytes as an enclave key's handle. Off the simulator, the
    /// enclave failing is the error the person sees, and nothing is saved.
    static func generateSecureEnclaveP256(comment: String,
                                          make: () throws -> SecureEnclave.P256.Signing.PrivateKey,
                                          standIn: Bool) throws -> Generated {
        let publicKey: P256.Signing.PublicKey
        let privateBlob: Data
        do {
            let key = try make()
            publicKey = key.publicKey
            privateBlob = key.dataRepresentation
        } catch {
            guard standIn else {
                throw KeyError.generationFailed(
                    "the Secure Enclave couldn't make a key (\(error.localizedDescription))")
            }
            // The simulator: a software P-256 key, so the flow can be walked.
            let key = P256.Signing.PrivateKey()
            publicKey = key.publicKey
            privateBlob = key.rawRepresentation
        }
        let line = try keyReader {
            try sshKeyHardwarePublicKey(publicKey: publicKey.x963Representation, comment: comment)
        }
        return Generated(algorithm: .seP256, privateBlob: privateBlob,
                         publicKeyLine: line.publicKeyLine, fingerprint: line.fingerprint)
    }

    // MARK: - Import

    /// Import a pasted private key, stored the one way every connect path
    /// loads a key: an unencrypted openssh-key-v1 file.
    ///
    /// The key is read whatever form it came in: what `ssh-keygen` writes by
    /// default (any cipher), its `-m PEM` and `-m PKCS8` forms, OpenSSL's and
    /// PuTTY's — decrypted with `passphrase` when it has one. A key no connect
    /// path signs with (a security key, DSA) is refused here, saying why.
    ///
    /// `comment` ends the public key line, as it does for a generated key.
    /// Left empty, a pasted `.pub` line's comment stays, else the key file's
    /// own. The `.pub` line contributes nothing else.
    static func importKey(privatePEM: String, passphrase: String? = nil,
                          publicKeyLine: String?, comment: String) throws -> Generated {
        try Generated(keyReader {
            try sshKeyImport(text: privatePEM, passphrase: passphrase, publicKeyLine: publicKeyLine,
                             comment: comment)
        })
    }

    /// Whether `text` holds a private key that needs a passphrase to read.
    /// False for anything unreadable — reading it says why — and for a key
    /// refused whatever the passphrase.
    static func isEncrypted(_ text: String) -> Bool {
        sshKeyIsEncrypted(text: text)
    }

    /// What a stored private key is, read off the key alone — for a
    /// keychain item no record names any more (``KeychainReferences``): its
    /// type, public key line, fingerprint and size. The stored bytes are
    /// never rewritten. A key file in any form the app reads (unencrypted),
    /// or a Secure Enclave key's handle; nil for anything else — a
    /// password, a key behind a passphrase, a key the app can't sign with.
    static func describeStored(_ secret: Data) -> Generated? {
        if let text = String(data: secret, encoding: .utf8), text.contains("-----BEGIN") {
            return sshKeyDescribeStored(text: text).flatMap { try? Generated($0) }
        }
        guard SecureEnclave.isAvailable,
              let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: secret),
              let line = try? sshKeyHardwarePublicKey(publicKey: key.publicKey.x963Representation, comment: "")
        else { return nil }
        return Generated(algorithm: .seP256, privateBlob: secret, publicKeyLine: line.publicKeyLine,
                         fingerprint: line.fingerprint)
    }

    /// The modulus size of the `ssh-rsa` key on an `authorized_keys` line;
    /// nil for any other line.
    static func rsaBits(ofPublicKeyLine line: String) -> Int? {
        sshKeyRsaBits(publicKeyLine: line).map(Int.init)
    }

    /// `SHA256:…` of an SSH public key blob, as `ssh-keygen -l` prints it.
    static func fingerprint(of publicBlob: Data) -> String {
        sshKeyFingerprint(publicBlob: publicBlob)
    }

    /// The SSH public key blob of the unencrypted private key in `text`.
    static func publicKeyBlob(of text: String) throws -> Data {
        try keyReader { try sshKeyPublicBlob(text: text) }
    }
}
