import Foundation
import CryptoKit
import Security

/// Generates and imports SSH key material (prototype screen 9).
///
/// Private keys are produced as OpenSSH-format PEM blobs (or Secure Enclave
/// data representations) and handed to `KeychainService` by the caller —
/// nothing here persists anything.
enum SSHKeyFactory {

    struct Generated {
        let algorithm: SSHKeyAlgorithm
        let privateBlob: Data      // OpenSSH PEM (utf8) or SE dataRepresentation
        let publicKeyLine: String  // authorized_keys line
        let fingerprint: String    // "SHA256:…"
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
            return try generateEd25519(comment: comment)
        case .seP256:
            return try generateSecureEnclaveP256(comment: comment)
        case .rsa4096:
            return try generateRSA4096(comment: comment)
        case .ecdsaSK:
            throw KeyError.unsupported("ECDSA-sk keys live on a hardware token — import the public half instead.")
        }
    }

    // MARK: ED25519

    private static func generateEd25519(comment: String) throws -> Generated {
        let key = Curve25519.Signing.PrivateKey()
        let pub = key.publicKey.rawRepresentation          // 32 bytes
        let seed = key.rawRepresentation                   // 32 bytes

        let publicBlob = sshString("ssh-ed25519") + sshString(pub)

        // openssh-key-v1 container (cipher "none")
        var payload = Data("openssh-key-v1\0".utf8)
        payload += sshString("none")                       // cipher
        payload += sshString("none")                       // kdf
        payload += sshString(Data())                       // kdf options
        payload += uint32(1)                               // n keys
        payload += sshString(publicBlob)

        var priv = Data()
        let check = UInt32.random(in: .min ... .max)
        priv += uint32(check) + uint32(check)
        priv += sshString("ssh-ed25519")
        priv += sshString(pub)
        priv += sshString(seed + pub)                      // 64-byte private
        priv += sshString(comment)
        var pad: UInt8 = 1
        while priv.count % 8 != 0 { priv.append(pad); pad += 1 }
        payload += sshString(priv)

        let pem = pemArmor(payload, label: "OPENSSH PRIVATE KEY")
        return Generated(
            algorithm: .ed25519,
            privateBlob: Data(pem.utf8),
            publicKeyLine: "ssh-ed25519 \(publicBlob.base64EncodedString()) \(comment)",
            fingerprint: fingerprint(of: publicBlob))
    }

    // MARK: Secure Enclave P-256

    private static func generateSecureEnclaveP256(comment: String) throws -> Generated {
        let publicKey: P256.Signing.PublicKey
        let privateBlob: Data
        do {
            let key = try SecureEnclave.P256.Signing.PrivateKey()
            publicKey = key.publicKey
            privateBlob = key.dataRepresentation
        } catch {
            // Simulator / devices without SE: fall back to a software P-256
            // key so the flow stays testable end-to-end.
            let key = P256.Signing.PrivateKey()
            publicKey = key.publicKey
            privateBlob = key.rawRepresentation
        }
        let point = publicKey.x963Representation             // 0x04 || X || Y
        let blob = sshString("ecdsa-sha2-nistp256") + sshString("nistp256") + sshString(point)
        return Generated(
            algorithm: .seP256,
            privateBlob: privateBlob,
            publicKeyLine: "ecdsa-sha2-nistp256 \(blob.base64EncodedString()) \(comment)",
            fingerprint: fingerprint(of: blob))
    }

    // MARK: RSA-4096

    /// `SecKeyCopyExternalRepresentation` hands back a PKCS#1 `RSAPrivateKey`
    /// DER — that's a real key, but not one anything downstream can
    /// authenticate with: `SSHKeyDetection.detectPrivateKeyType` (Citadel)
    /// hard-requires the openssh-key-v1 container, and PKCS#1 isn't it. Build
    /// that container ourselves, mirroring `generateEd25519`'s framing below.
    private static func generateRSA4096(comment: String) throws -> Generated {
        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeRSA,
            kSecAttrKeySizeInBits as String: 4096,
        ]
        var error: Unmanaged<CFError>?
        guard let secKey = SecKeyCreateRandomKey(attributes as CFDictionary, &error),
              let privDER = SecKeyCopyExternalRepresentation(secKey, &error) as Data?
        else {
            let why = (error?.takeRetainedValue()).map(String.init(describing:)) ?? "SecKey failure"
            throw KeyError.generationFailed(why)
        }

        guard let fields = parsePKCS1RSAPrivateKey(privDER) else {
            throw KeyError.generationFailed("could not parse generated RSA private key")
        }

        // SSH's own (e, n) order for the public blob (RFC 4253) — already
        // exercised by the authorized_keys line below.
        let publicBlob = sshString("ssh-rsa") + sshMPInt(fields.e) + sshMPInt(fields.n)

        var payload = Data("openssh-key-v1\0".utf8)
        payload += sshString("none")                       // cipher
        payload += sshString("none")                       // kdf
        payload += sshString(Data())                       // kdf options
        payload += uint32(1)                               // n keys
        payload += sshString(publicBlob)

        var priv = Data()
        let check = UInt32.random(in: .min ... .max)
        priv += uint32(check) + uint32(check)
        priv += sshString("ssh-rsa")
        // openssh-key-v1's own (n, e, d, iqmp, p, q) order for the private
        // blob — NOT the same order as the public blob above, and not PKCS#1's
        // order either. Confirmed against Citadel's own reader
        // (`Insecure.RSA.PrivateKey.read(consuming:)`, Citadel/OpenSSHKey.swift)
        // rather than assumed, since getting this wrong silently produces a
        // key that just fails to authenticate.
        priv += sshMPInt(fields.n)
        priv += sshMPInt(fields.e)
        priv += sshMPInt(fields.d)
        priv += sshMPInt(fields.iqmp)
        priv += sshMPInt(fields.p)
        priv += sshMPInt(fields.q)
        priv += sshString(comment)
        var pad: UInt8 = 1
        while priv.count % 8 != 0 { priv.append(pad); pad += 1 }
        payload += sshString(priv)

        let pem = pemArmor(payload, label: "OPENSSH PRIVATE KEY")
        return Generated(
            algorithm: .rsa4096,
            privateBlob: Data(pem.utf8),
            publicKeyLine: "ssh-rsa \(publicBlob.base64EncodedString()) \(comment)",
            fingerprint: fingerprint(of: publicBlob))
    }

    // MARK: - Import

    /// Best-effort import of a pasted private key. Detects the algorithm from
    /// the PEM body; computes a fingerprint when a public key line is supplied.
    static func importKey(privatePEM: String, publicKeyLine: String?, comment: String) throws -> Generated {
        let trimmed = privatePEM.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.contains("PRIVATE KEY") else {
            throw KeyError.unsupported("not a PEM private key")
        }
        let algorithm: SSHKeyAlgorithm
        if trimmed.contains("BEGIN RSA") || (publicKeyLine?.hasPrefix("ssh-rsa") ?? false) {
            algorithm = .rsa4096
        } else if publicKeyLine?.hasPrefix("ecdsa-sha2") ?? trimmed.contains("BEGIN EC") {
            algorithm = .ecdsaSK
        } else {
            algorithm = .ed25519
        }
        var fp = ""
        var pubLine = publicKeyLine ?? ""
        if let line = publicKeyLine {
            let parts = line.split(separator: " ")
            if parts.count >= 2, let blob = Data(base64Encoded: String(parts[1])) {
                fp = fingerprint(of: blob)
            }
        } else {
            pubLine = ""
        }
        return Generated(
            algorithm: algorithm,
            privateBlob: Data(trimmed.utf8),
            publicKeyLine: pubLine,
            fingerprint: fp)
    }

    // MARK: - Passphrase strength (prototype §3.8)

    /// Rough entropy heuristic mapped to 0…1 for the strength meter.
    static func passphraseStrength(_ s: String) -> Double {
        guard !s.isEmpty else { return 0 }
        var classes = 0.0
        if s.rangeOfCharacter(from: .lowercaseLetters) != nil { classes += 26 }
        if s.rangeOfCharacter(from: .uppercaseLetters) != nil { classes += 26 }
        if s.rangeOfCharacter(from: .decimalDigits) != nil { classes += 10 }
        if s.rangeOfCharacter(from: CharacterSet.alphanumerics.inverted) != nil { classes += 32 }
        let entropy = Double(s.count) * log2(max(classes, 1))
        return min(entropy / 80.0, 1.0)
    }

    // MARK: - Wire helpers

    private static func uint32(_ value: UInt32) -> Data {
        var be = value.bigEndian
        return Data(bytes: &be, count: 4)
    }

    private static func sshString(_ data: Data) -> Data {
        uint32(UInt32(data.count)) + data
    }

    private static func sshString(_ s: String) -> Data {
        sshString(Data(s.utf8))
    }

    /// SSH mpint: big-endian, leading 0x00 if the high bit is set.
    private static func sshMPInt(_ raw: Data) -> Data {
        var bytes = Data(raw.drop(while: { $0 == 0 }))
        if let first = bytes.first, first & 0x80 != 0 {
            bytes.insert(0, at: bytes.startIndex)
        }
        return sshString(bytes)
    }

    static func fingerprint(of publicBlob: Data) -> String {
        let digest = SHA256.hash(data: publicBlob)
        let b64 = Data(digest).base64EncodedString()
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
        return "SHA256:\(b64)"
    }

    private static func pemArmor(_ payload: Data, label: String) -> String {
        let b64 = payload.base64EncodedString()
        let wrapped = stride(from: 0, to: b64.count, by: 70).map { offset -> Substring in
            let start = b64.index(b64.startIndex, offsetBy: offset)
            let end = b64.index(start, offsetBy: 70, limitedBy: b64.endIndex) ?? b64.endIndex
            return b64[start..<end]
        }.joined(separator: "\n")
        return "-----BEGIN \(label)-----\n\(wrapped)\n-----END \(label)-----\n"
    }

    /// Minimal sequential DER reader — only what's needed to walk a flat
    /// SEQUENCE of INTEGERs, which is all a PKCS#1 RSA public or private key
    /// body is. Shared so the private-key walk added for openssh-key-v1
    /// export runs the same, already-exercised tag/length logic as the
    /// public-key walk below rather than a second hand-rolled copy of it.
    private struct DERReader {
        private let data: Data
        private var idx: Data.Index

        init(_ data: Data) {
            self.data = data
            self.idx = data.startIndex
        }

        private mutating func readLength() -> Int? {
            guard idx < data.endIndex else { return nil }
            let first = data[idx]; idx = data.index(after: idx)
            if first & 0x80 == 0 { return Int(first) }
            let count = Int(first & 0x7F)
            guard count <= 4, data.distance(from: idx, to: data.endIndex) >= count else { return nil }
            var length = 0
            for _ in 0..<count {
                length = length << 8 | Int(data[idx])
                idx = data.index(after: idx)
            }
            return length
        }

        /// Consumes a SEQUENCE's tag + length, leaving `idx` at its first child.
        mutating func enterSequence() -> Bool {
            guard idx < data.endIndex, data[idx] == 0x30 else { return false }
            idx = data.index(after: idx)
            return readLength() != nil
        }

        mutating func expect(tag: UInt8) -> Data? {
            guard idx < data.endIndex, data[idx] == tag else { return nil }
            idx = data.index(after: idx)
            guard let length = readLength(),
                  data.distance(from: idx, to: data.endIndex) >= length else { return nil }
            let body = data[idx..<data.index(idx, offsetBy: length)]
            idx = data.index(idx, offsetBy: length)
            return Data(body)
        }

        mutating func integer() -> Data? { expect(tag: 0x02) }
    }

    /// PKCS#1 RSAPublicKey: SEQUENCE { INTEGER n, INTEGER e }.
    private static func parsePKCS1RSAPublicKey(_ der: Data) -> (modulus: Data, exponent: Data)? {
        var reader = DERReader(der)
        guard reader.enterSequence(), let n = reader.integer(), let e = reader.integer() else { return nil }
        return (n, e)
    }

    /// PKCS#1 RSAPrivateKey (RFC 8017 §A.1.2): SEQUENCE { version, n, e, d,
    /// p, q, exponent1, exponent2, coefficient }. `SecKeyCopyExternalRepresentation`
    /// emits exactly this for an RSA private key. `coefficient` here is the
    /// same value openssh-key-v1 calls `iqmp` (q⁻¹ mod p) — Apple's SecKey
    /// already computed it, so no modular inverse needs computing by hand.
    private static func parsePKCS1RSAPrivateKey(
        _ der: Data
    ) -> (n: Data, e: Data, d: Data, p: Data, q: Data, iqmp: Data)? {
        var reader = DERReader(der)
        guard reader.enterSequence(),
              reader.integer() != nil,                 // version (0)
              let n = reader.integer(),
              let e = reader.integer(),
              let d = reader.integer(),
              let p = reader.integer(),
              let q = reader.integer(),
              reader.integer() != nil,                 // exponent1 = d mod (p-1), unused
              reader.integer() != nil,                 // exponent2 = d mod (q-1), unused
              let iqmp = reader.integer()
        else { return nil }
        return (n, e, d, p, q, iqmp)
    }
}
