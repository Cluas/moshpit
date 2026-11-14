import Citadel
import Foundation
import NIOCore
import Testing
@testable import Moshpit

/// Counts whether `SSHService.connect` got as far as dialing: it only does
/// once it has built an authentication method from the stored key, so being
/// reached is the proof the connect path's key loading accepted the key.
private final class AuthProbe: SSHClientProvider, @unchecked Sendable {
    private let lock = NSLock()
    private var reached = false

    var wasReached: Bool { lock.withLock { reached } }

    func connect(host: String, port: Int, authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        lock.withLock { reached = true }
        throw CancellationError()
    }

    func connect(on channel: Channel, authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        lock.withLock { reached = true }
        throw CancellationError()
    }
}

/// A key that imports has to connect. `SSHService` signs with one form only —
/// an unencrypted openssh-key-v1 Ed25519, RSA or ECDSA P-256 key — and import
/// used to store whatever was pasted, so PEM keys and every key behind a
/// passphrase imported fine and then failed each connection. Import now
/// converts to that form, asking for the passphrase, and refuses what it can't.
@Suite("SSH key import: what imports, connects")
struct SSHKeyImportTests {

    /// Whether `SSHService.connect` builds an authentication method from
    /// `stored` as a key's Keychain item, stored the way Add Key stores it.
    private static func connectPathAccepts(_ stored: Data) async throws -> Bool {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.saveSecret(stored, forRef: ref, requireBiometry: false)
        let probe = AuthProbe()
        let service = SSHService(keychain: keychain, hostKeyValidator: .inMemory(), clientProvider: probe)
        let connection = ServerConnection(name: "imported", host: "192.0.2.1", username: "tester",
                                          authMethod: .key, keychainRef: ref)
        _ = try? await service.connect(connection)
        return probe.wasReached
    }

    private static func fixture(_ name: String) -> SSHKeyFixtures.Key {
        SSHKeyFixtures.all.first { $0.name == name }!
    }

    // MARK: Every form converts to one that connects

    @Test("Every form ssh-keygen and OpenSSL write imports into a key the connect path loads",
          arguments: SSHKeyFixtures.all)
    func importedKeyConnects(_ key: SSHKeyFixtures.Key) async throws {
        let imported = try SSHKeyFactory.importKey(
            privatePEM: key.privateKey, passphrase: key.passphrase, publicKeyLine: nil, comment: "")

        #expect(imported.algorithm == key.algorithm)
        #expect(imported.fingerprint == key.fingerprint, "not the key ssh-keygen made")
        #expect(imported.publicKeyLine.split(separator: " ").prefix(2) == key.publicKey.split(separator: " ").prefix(2))
        let stored = String(decoding: imported.privateBlob, as: UTF8.self)
        #expect(stored.hasPrefix("-----BEGIN OPENSSH PRIVATE KEY-----\n"))
        #expect(!SSHKeyFactory.isEncrypted(stored), "stored still behind its passphrase")
        #expect(try await Self.connectPathAccepts(imported.privateBlob),
                "the connect path can't load the imported \(key.name)")
    }

    /// Before B125 none of these reached the server. The connect path now
    /// converts a key stored as pasted on its first use (StoredKey), as
    /// import does — but not one behind a passphrase nobody gives.
    @Test("Stored as pasted — what import used to do — the connect path's reader can't read these",
          arguments: SSHKeyFixtures.all.filter { $0.isEncrypted || !$0.privateKey.contains("OPENSSH PRIVATE KEY") })
    func pastedAsIsIsNotWhatTheConnectPathReads(_ key: SSHKeyFixtures.Key) async throws {
        // An encrypted openssh-key-v1 file reads up to its public half.
        #expect(!StoredKey.connectPathReads(key.privateKey) || key.isEncrypted)
        #expect(try await Self.connectPathAccepts(Data(key.privateKey.utf8)) == !key.isEncrypted)
    }

    @Test("A key whose line breaks didn't survive the paste imports into one that loads")
    func mangledPasteStillConnects() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        let crlf = key.privateKey.replacingOccurrences(of: "\n", with: "\r\n")
        let oneLine = key.privateKey.replacingOccurrences(of: "\n", with: " ")
        for pasted in [crlf, oneLine, "\n  " + key.privateKey + "  \n\n"] {
            let imported = try SSHKeyFactory.importKey(privatePEM: pasted, publicKeyLine: nil, comment: "")
            #expect(imported.fingerprint == key.fingerprint)
            #expect(try await Self.connectPathAccepts(imported.privateBlob))
        }
        // Stored as pasted, the connect path's reader can't read the first
        // two; the connect converts them before it signs in (B125).
        #expect(!StoredKey.connectPathReads(crlf))
        #expect(!StoredKey.connectPathReads(oneLine))
        #expect(try await Self.connectPathAccepts(Data(crlf.utf8)))
    }

    // MARK: Passphrases

    @Test("Import knows which pasted keys need a passphrase", arguments: SSHKeyFixtures.all)
    func knowsWhenToAsk(_ key: SSHKeyFixtures.Key) {
        #expect(SSHKeyFactory.isEncrypted(key.privateKey) == key.isEncrypted)
    }

    @Test("A key behind a passphrase isn't imported without it", arguments: SSHKeyFixtures.encrypted)
    func passphraseRequired(_ key: SSHKeyFixtures.Key) {
        #expect(throws: SSHKeyImportError.passphraseRequired) {
            _ = try SSHKeyFactory.importKey(privatePEM: key.privateKey, passphrase: nil, publicKeyLine: nil, comment: "")
        }
        #expect(throws: SSHKeyImportError.passphraseRequired) {
            _ = try SSHKeyFactory.importKey(privatePEM: key.privateKey, passphrase: "", publicKeyLine: nil, comment: "")
        }
    }

    @Test("A wrong passphrase is refused as wrong, in each encryption scheme",
          arguments: ["Ed25519 · OpenSSH · passphrase",
                      "RSA 2048 · PEM (ssh-keygen -m PEM) · passphrase",
                      "ECDSA P-256 · PKCS #8 (ssh-keygen -m PKCS8) · passphrase"])
    func wrongPassphrase(_ name: String) {
        #expect(throws: SSHKeyImportError.wrongPassphrase) {
            _ = try SSHKeyFactory.importKey(privatePEM: Self.fixture(name).privateKey, passphrase: "correct horse",
                                        publicKeyLine: nil, comment: "")
        }
    }

    // MARK: Refused, saying why

    @Test("Keys the connect path can't sign with are refused at import, saying why")
    func refusedWithReason() async throws {
        // B126: P-384 (and P-521) sign in now, so the key this test refused
        // imports, and the connect path loads it.
        let p384 = try SSHKeyFactory.importKey(privatePEM: SSHKeyFixtures.ecdsaP384, publicKeyLine: nil, comment: "")
        #expect(p384.algorithm == .ecdsa)
        #expect(p384.publicKeyLine.hasPrefix("ecdsa-sha2-nistp384 "))
        #expect(try await Self.connectPathAccepts(p384.privateBlob))
        // A security key's file holds a handle; the private half is on the token.
        let blob = Self.sshString("sk-ecdsa-sha2-nistp256@openssh.com") + Self.sshString("nistp256")
            + Self.sshString(Data(repeating: 4, count: 65)) + Self.sshString("ssh:")
        #expect(throws: SSHKeyImportError.securityKey) {
            _ = try SSHKeyFactory.importKey(privatePEM: Self.openSSHContainer(publicBlob: blob),
                                        publicKeyLine: nil, comment: "")
        }
        #expect(!SSHKeyFactory.isEncrypted(Self.openSSHContainer(publicBlob: blob, cipher: "aes256-ctr")),
                "no passphrase to ask for when the key is refused anyway")
        // ChaCha20-Poly1305, which the Swift reader couldn't undo (no
        // ChaCha20 in CommonCrypto), opens in the shared Rust reader.
        let chacha = try SSHKeyFactory.importKey(privatePEM: SSHKeyFixtures.chachaEncrypted,
                                                 passphrase: SSHKeyFixtures.passphrase, publicKeyLine: nil, comment: "")
        #expect(chacha.algorithm == .ed25519)
        #expect(throws: SSHKeyImportError.wrongPassphrase) {
            _ = try SSHKeyFactory.importKey(privatePEM: SSHKeyFixtures.chachaEncrypted,
                                            passphrase: "correct horse", publicKeyLine: nil, comment: "")
        }
        #expect(throws: SSHKeyImportError.unsupportedEncryption("PKCS #12 PBE")) {
            _ = try SSHKeyFactory.importKey(privatePEM: SSHKeyFixtures.pkcs12Encrypted,
                                        passphrase: SSHKeyFixtures.passphrase, publicKeyLine: nil, comment: "")
        }
        let key = Self.fixture("RSA 2048 · OpenSSH")
        #expect(throws: SSHKeyImportError.unrecognized) {
            _ = try SSHKeyFactory.importKey(privatePEM: key.publicKey, publicKeyLine: nil, comment: "")
        }
        let lines = key.privateKey.split(separator: "\n")
        let cutShort = (lines.prefix(lines.count / 2) + [lines.last!]).joined(separator: "\n")
        #expect(throws: SSHKeyImportError.damaged) {
            _ = try SSHKeyFactory.importKey(privatePEM: cutShort, publicKeyLine: nil, comment: "")
        }
        for refusal in [SSHKeyImportError.securityKey, .unsupportedType("DSA"), .passphraseRequired] {
            #expect(!(refusal.errorDescription ?? "").isEmpty)
        }
    }

    // MARK: The public half

    @Test("The public key line carries the comment typed, else the pasted .pub's, else the key's own")
    func publicLineComment() throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        let typeAndBlob = key.publicKey.split(separator: " ").prefix(2).joined(separator: " ")

        let typed = try SSHKeyFactory.importKey(privatePEM: key.privateKey, publicKeyLine: key.publicKey,
                                                comment: "work laptop")
        #expect(typed.publicKeyLine == typeAndBlob + " work laptop")

        let pasted = try SSHKeyFactory.importKey(privatePEM: key.privateKey,
                                                 publicKeyLine: typeAndBlob + " from-the-pub\n", comment: "")
        #expect(pasted.publicKeyLine == typeAndBlob + " from-the-pub")

        // Nothing typed, no .pub: the comment ssh-keygen wrote into the key.
        let own = try SSHKeyFactory.importKey(privatePEM: key.privateKey, publicKeyLine: nil, comment: "")
        #expect(own.publicKeyLine == key.publicKey)
    }

    // MARK: Helpers

    private static func sshString(_ data: Data) -> Data {
        var length = UInt32(data.count).bigEndian
        return Data(bytes: &length, count: 4) + data
    }

    private static func sshString(_ text: String) -> Data { sshString(Data(text.utf8)) }

    /// An openssh-key-v1 file around `publicBlob`, its private section a
    /// stand-in: what matters is what the clear public half says.
    private static func openSSHContainer(publicBlob: Data, cipher: String = "none") -> String {
        var payload = Data("openssh-key-v1\0".utf8)
        payload += sshString(cipher)
        payload += sshString(cipher == "none" ? "none" : "bcrypt")
        payload += sshString(cipher == "none" ? Data() : sshString(Data(repeating: 7, count: 16)) + Data([0, 0, 0, 16]))
        payload += Data([0, 0, 0, 1])
        payload += sshString(publicBlob)
        payload += sshString(Data(repeating: 0, count: 16))
        return "-----BEGIN OPENSSH PRIVATE KEY-----\n\(payload.base64EncodedString())\n-----END OPENSSH PRIVATE KEY-----"
    }
}
