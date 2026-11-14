import CryptoKit
import Foundation
import Testing
@testable import Moshpit

/// Covers the pure, connection-free logic in `SSHKeyFactory`: key generation
/// (structure of the emitted OpenSSH artifacts), what import refuses and how it
/// names a key, and the SHA256 fingerprint helper. Real keys in every form,
/// and whether what imports connects, are `SSHKeyImportTests`. Nothing here
/// touches the network or persists to the Keychain.
@Suite("SSHKeyFactory")
struct SSHKeyFactoryTests {

    // MARK: - Fingerprint

    @Test("fingerprint is deterministic, SHA256-prefixed, and base64 with no padding")
    func fingerprintShape() {
        let blob = Data("some-public-key-blob".utf8)
        let fp = SSHKeyFactory.fingerprint(of: blob)
        #expect(fp.hasPrefix("SHA256:"))
        // Same input → same digest.
        #expect(fp == SSHKeyFactory.fingerprint(of: blob))
        // OpenSSH-style fingerprints drop the '=' base64 padding.
        #expect(!fp.contains("="))
    }

    @Test("different blobs produce different fingerprints")
    func fingerprintDistinguishes() {
        let a = SSHKeyFactory.fingerprint(of: Data("a".utf8))
        let b = SSHKeyFactory.fingerprint(of: Data("b".utf8))
        #expect(a != b)
    }

    // MARK: - Generate: ED25519

    @Test("ed25519 generation emits an authorized_keys line, PEM blob, and fingerprint")
    func generateEd25519() throws {
        let gen = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "alice@laptop")
        #expect(gen.algorithm == .ed25519)
        #expect(gen.publicKeyLine.hasPrefix("ssh-ed25519 "))
        #expect(gen.publicKeyLine.hasSuffix(" alice@laptop"))
        #expect(gen.fingerprint.hasPrefix("SHA256:"))

        let pem = String(decoding: gen.privateBlob, as: UTF8.self)
        #expect(pem.contains("-----BEGIN OPENSSH PRIVATE KEY-----"))
        #expect(pem.contains("-----END OPENSSH PRIVATE KEY-----"))
    }

    @Test("each ed25519 generation produces fresh, distinct key material")
    func ed25519IsRandom() throws {
        let a = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "c")
        let b = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "c")
        #expect(a.publicKeyLine != b.publicKeyLine)
        #expect(a.fingerprint != b.fingerprint)
    }

    @Test("the generated public line is itself importable and re-derives the same fingerprint")
    func ed25519RoundTripsThroughImport() throws {
        let gen = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "alice")
        let pem = String(decoding: gen.privateBlob, as: UTF8.self)
        let imported = try SSHKeyFactory.importKey(
            privatePEM: pem, publicKeyLine: gen.publicKeyLine, comment: "alice")
        #expect(imported.algorithm == .ed25519)
        #expect(imported.fingerprint == gen.fingerprint)
    }

    // MARK: - Generate: Secure Enclave P-256 (software fallback on simulator)

    @Test("SE P-256 generation emits an ecdsa-nistp256 authorized_keys line")
    func generateSecureEnclaveP256() throws {
        // On the simulator the Secure Enclave is unavailable, so this exercises
        // the documented software-P256 fallback path.
        let gen = try SSHKeyFactory.generate(algorithm: .seP256, comment: "phone")
        #expect(gen.algorithm == .seP256)
        #expect(gen.publicKeyLine.hasPrefix("ecdsa-sha2-nistp256 "))
        #expect(gen.publicKeyLine.hasSuffix(" phone"))
        #expect(gen.fingerprint.hasPrefix("SHA256:"))
        #expect(!gen.privateBlob.isEmpty)
    }

    // MARK: - Generate: RSA-4096

    @Test("RSA-4096 generation emits an ssh-rsa line over an openssh-key-v1 key")
    func generateRSA4096() throws {
        let gen = try SSHKeyFactory.generate(algorithm: .rsa4096, comment: "server")
        #expect(gen.algorithm == .rsa4096)
        #expect(gen.publicKeyLine.hasPrefix("ssh-rsa "))
        #expect(gen.fingerprint.hasPrefix("SHA256:"))
        let pem = String(decoding: gen.privateBlob, as: UTF8.self)
        // openssh-key-v1, NOT the PKCS#1 container SecKey hands back: Citadel's
        // detectPrivateKeyType hard-requires the former, so a PKCS#1 key can
        // never authenticate. That is the bug this armor exists to fix, which
        // makes the container the thing worth asserting.
        #expect(pem.contains("-----BEGIN OPENSSH PRIVATE KEY-----"))
        #expect(pem.contains("-----END OPENSSH PRIVATE KEY-----"))
        #expect(!pem.contains("-----BEGIN RSA PRIVATE KEY-----"))
        // The middle field must be valid base64 of the ssh-rsa blob.
        let parts = gen.publicKeyLine.split(separator: " ")
        #expect(parts.count == 3)
        #expect(Data(base64Encoded: String(parts[1])) != nil)
        #expect(gen.bits == 4096)
        #expect(SSHKeyFactory.rsaBits(ofPublicKeyLine: gen.publicKeyLine) == 4096)
    }

    // MARK: - Generate: unsupported

    @Test("ecdsa-sk generation is rejected as unsupported (lives on hardware)")
    func generateEcdsaSKThrows() {
        #expect(throws: SSHKeyFactory.KeyError.self) {
            _ = try SSHKeyFactory.generate(algorithm: .ecdsaSK, comment: "yk")
        }
    }

    // MARK: - Import: what isn't a private key it can use

    @Test("import refuses text that isn't a private key")
    func importRejectsNonPEM() {
        #expect(throws: SSHKeyImportError.unrecognized) {
            _ = try SSHKeyFactory.importKey(
                privatePEM: "just some pasted text", publicKeyLine: nil, comment: "")
        }
    }

    @Test("a PEM body with no key in it is refused, not named from its header",
          arguments: ["RSA PRIVATE KEY", "EC PRIVATE KEY", "PRIVATE KEY", "OPENSSH PRIVATE KEY"])
    func importRefusesHollowBody(label: String) {
        // These used to import — RSA and ECDSA named from the header alone —
        // and then fail every connection.
        let pem = "-----BEGIN \(label)-----\nAAAA\n-----END \(label)-----"
        #expect(throws: SSHKeyImportError.damaged) {
            _ = try SSHKeyFactory.importKey(privatePEM: pem, publicKeyLine: nil, comment: "")
        }
    }

    @Test("a public key line can't stand in for a key body")
    func importNeedsTheKeyItself() {
        let pem = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----"
        let blob = Data("rsa-blob".utf8).base64EncodedString()
        #expect(throws: SSHKeyImportError.damaged) {
            _ = try SSHKeyFactory.importKey(
                privatePEM: pem, publicKeyLine: "ssh-rsa \(blob) comment", comment: "")
        }
    }

    @Test("import ignores whitespace around the pasted key")
    func importTrimsWhitespace() throws {
        let generated = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "c")
        let core = try #require(String(bytes: generated.privateBlob, encoding: .utf8))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let imported = try SSHKeyFactory.importKey(
            privatePEM: "\n\n  \(core)  \n", publicKeyLine: nil, comment: "")
        #expect(imported.fingerprint == generated.fingerprint)
        #expect(imported.publicKeyLine == generated.publicKeyLine)
    }

    // MARK: - Import: OpenSSH keys name themselves

    @Test("an OpenSSH ECDSA key is ECDSA, with or without its .pub")
    func importOpenSSHECDSA() throws {
        let key = Self.openSSHP256(comment: "p256@host")
        let alone = try SSHKeyFactory.importKey(privatePEM: key.pem, publicKeyLine: nil, comment: "")
        #expect(alone.algorithm == .ecdsa)
        #expect(alone.fingerprint == SSHKeyFactory.fingerprint(of: key.publicBlob))

        // With the .pub alongside it was ECDSA-sk — a hardware-token key, which
        // the connection form's key picker leaves out.
        let pubLine = "ecdsa-sha2-nistp256 \(key.publicBlob.base64EncodedString()) p256@host"
        let withPub = try SSHKeyFactory.importKey(privatePEM: key.pem, publicKeyLine: pubLine, comment: "")
        #expect(withPub.algorithm == .ecdsa)
        #expect(withPub.publicKeyLine == pubLine)
    }

    @Test("B126: OpenSSH ECDSA keys on the larger curves sign in, so one is refused only as what it is — here a stand-in",
          arguments: [("ecdsa-sha2-nistp384", "ECDSA P-384"), ("ecdsa-sha2-nistp521", "ECDSA P-521")])
    func importReadsLargerCurves(type: String, name: String) {
        let curve = String(type.dropFirst("ecdsa-sha2-".count))
        let blob = Self.sshString(type) + Self.sshString(curve) + Self.sshString(Data(repeating: 4, count: 97))
        #expect(throws: SSHKeyImportError.damaged) {
            _ = try SSHKeyFactory.importKey(
                privatePEM: Self.openSSHPEM(publicBlob: blob), publicKeyLine: nil, comment: "")
        }
    }

    @Test("a security-key ECDSA (sk-ecdsa) is refused: its private half is on the token")
    func importRefusesSecurityKey() {
        let type = "sk-ecdsa-sha2-nistp256@openssh.com"
        let blob = Self.sshString(type) + Self.sshString("nistp256")
            + Self.sshString(Data(repeating: 4, count: 65)) + Self.sshString("ssh:")
        #expect(throws: SSHKeyImportError.securityKey) {
            _ = try SSHKeyFactory.importKey(
                privatePEM: Self.openSSHPEM(publicBlob: blob), publicKeyLine: nil, comment: "")
        }
    }

    @Test("an OpenSSH key of a type the app can't sign with is refused")
    func importOpenSSHUnknownType() {
        let blob = Self.sshString("ssh-dss") + Self.sshString(Data(repeating: 1, count: 20))
        #expect(throws: SSHKeyImportError.unsupportedType("DSA")) {
            _ = try SSHKeyFactory.importKey(
                privatePEM: Self.openSSHPEM(publicBlob: blob), publicKeyLine: nil, comment: "")
        }
    }

    // MARK: - Import: the comment

    @Test("the comment typed on import ends the public key line")
    func importKeepsComment() throws {
        let generated = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "generated")
        let pem = try #require(String(bytes: generated.privateBlob, encoding: .utf8))
        let imported = try SSHKeyFactory.importKey(privatePEM: pem, publicKeyLine: nil, comment: "work laptop")

        let fields = generated.publicKeyLine.split(separator: " ")
        #expect(imported.publicKeyLine == "\(fields[0]) \(fields[1]) work laptop")
        #expect(imported.fingerprint == generated.fingerprint)
    }

    @Test("with no comment typed, the pasted .pub keeps its own — without the file's newline")
    func importKeepsPastedComment() throws {
        let generated = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "alice@laptop")
        let pem = try #require(String(bytes: generated.privateBlob, encoding: .utf8))
        // What "Import File…" puts in the field: the .pub file, newline and all.
        let imported = try SSHKeyFactory.importKey(
            privatePEM: pem, publicKeyLine: generated.publicKeyLine + "\n", comment: "")
        #expect(imported.publicKeyLine == generated.publicKeyLine)
    }

    // MARK: - Import: from file

    @Test("decodeImportedText decodes valid UTF-8 bytes, e.g. a picked PEM file")
    func decodeImportedTextDecodesUTF8() throws {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n"
        let decoded = try SSHKeyFactory.decodeImportedText(Data(pem.utf8))
        #expect(decoded == pem)
    }

    @Test("decodeImportedText throws on non-UTF-8 bytes instead of producing garbage text")
    func decodeImportedTextRejectsBinary() {
        // Lone continuation byte — never valid at the start of a UTF-8 sequence.
        let binary = Data([0xFF, 0xFE, 0x00, 0x80])
        #expect(throws: SSHKeyFactory.KeyError.self) {
            _ = try SSHKeyFactory.decodeImportedText(binary)
        }
    }

    // MARK: - Helpers

    static func sshString(_ data: Data) -> Data {
        var length = UInt32(data.count).bigEndian
        return Data(bytes: &length, count: 4) + data
    }

    static func sshString(_ text: String) -> Data { sshString(Data(text.utf8)) }

    static func mpint(_ raw: Data) -> Data {
        var bytes = Data(raw.drop(while: { $0 == 0 }))
        if let first = bytes.first, first & 0x80 != 0 { bytes.insert(0, at: bytes.startIndex) }
        return sshString(bytes)
    }

    /// An openssh-key-v1 container around `publicBlob`, armored as
    /// `ssh-keygen` writes it. The private section defaults to a stand-in,
    /// for keys refused on what their public half says.
    static func openSSHPEM(publicBlob: Data,
                           privateSection: Data = Data(repeating: 0, count: 8),
                           cipher: String = "none", kdf: String = "none",
                           kdfOptions: Data = Data()) -> String {
        var payload = Data("openssh-key-v1\0".utf8)
        payload += sshString(cipher) + sshString(kdf) + sshString(kdfOptions)
        payload += Data([0, 0, 0, 1]) + sshString(publicBlob) + sshString(privateSection)
        let body = payload.base64EncodedString(options: [.lineLength64Characters, .endLineWithLineFeed])
        return "-----BEGIN OPENSSH PRIVATE KEY-----\n\(body)\n-----END OPENSSH PRIVATE KEY-----\n"
    }

    /// A real, unencrypted OpenSSH ECDSA P-256 key, laid out the way
    /// `ssh-keygen -t ecdsa` writes one.
    static func openSSHP256(comment: String) -> (pem: String, publicBlob: Data) {
        let key = P256.Signing.PrivateKey()
        let point = key.publicKey.x963Representation
        let publicBlob = sshString("ecdsa-sha2-nistp256") + sshString("nistp256") + sshString(point)
        var section = Data([9, 9, 9, 9, 9, 9, 9, 9])                  // the two check ints
        section += sshString("ecdsa-sha2-nistp256") + sshString("nistp256") + sshString(point)
        section += mpint(key.rawRepresentation) + sshString(comment)
        var pad: UInt8 = 1
        while section.count % 8 != 0 { section.append(pad); pad += 1 }
        return (openSSHPEM(publicBlob: publicBlob, privateSection: section), publicBlob)
    }
}
