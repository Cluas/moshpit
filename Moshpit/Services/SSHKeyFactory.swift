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

    private static func generateRSA4096(comment: String) throws -> Generated {
        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeRSA,
            kSecAttrKeySizeInBits as String: 4096,
        ]
        var error: Unmanaged<CFError>?
        guard let secKey = SecKeyCreateRandomKey(attributes as CFDictionary, &error),
              let publicKey = SecKeyCopyPublicKey(secKey),
              let privDER = SecKeyCopyExternalRepresentation(secKey, &error) as Data?,
              let pubDER = SecKeyCopyExternalRepresentation(publicKey, &error) as Data?
        else {
            let why = (error?.takeRetainedValue()).map(String.init(describing:)) ?? "SecKey failure"
            throw KeyError.generationFailed(why)
        }

        guard let (modulus, exponent) = parsePKCS1RSAPublicKey(pubDER) else {
            throw KeyError.generationFailed("could not parse RSA public key")
        }
        let blob = sshString("ssh-rsa") + sshMPInt(exponent) + sshMPInt(modulus)
        let pem = pemArmor(privDER, label: "RSA PRIVATE KEY")
        return Generated(
            algorithm: .rsa4096,
            privateBlob: Data(pem.utf8),
            publicKeyLine: "ssh-rsa \(blob.base64EncodedString()) \(comment)",
            fingerprint: fingerprint(of: blob))
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

    /// Minimal DER walk for PKCS#1 RSAPublicKey: SEQUENCE { INTEGER n, INTEGER e }.
    private static func parsePKCS1RSAPublicKey(_ der: Data) -> (modulus: Data, exponent: Data)? {
        var idx = der.startIndex

        func readLength() -> Int? {
            guard idx < der.endIndex else { return nil }
            let first = der[idx]; idx = der.index(after: idx)
            if first & 0x80 == 0 { return Int(first) }
            let count = Int(first & 0x7F)
            guard count <= 4, der.distance(from: idx, to: der.endIndex) >= count else { return nil }
            var length = 0
            for _ in 0..<count {
                length = length << 8 | Int(der[idx])
                idx = der.index(after: idx)
            }
            return length
        }

        func expect(tag: UInt8) -> Data? {
            guard idx < der.endIndex, der[idx] == tag else { return nil }
            idx = der.index(after: idx)
            guard let length = readLength(),
                  der.distance(from: idx, to: der.endIndex) >= length else { return nil }
            let body = der[idx..<der.index(idx, offsetBy: length)]
            idx = der.index(idx, offsetBy: length)
            return Data(body)
        }

        guard der.first == 0x30, idx < der.endIndex else { return nil }
        idx = der.index(after: idx)
        guard readLength() != nil else { return nil }
        guard let n = expect(tag: 0x02), let e = expect(tag: 0x02) else { return nil }
        return (n, e)
    }
}
