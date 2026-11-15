import CryptoKit
import Foundation
import Testing
@testable import Moshpit

/// What `SSHKeyFactory` decides on the Swift side: which key generation
/// asks for (the Secure Enclave with its software fallback, RSA's size,
/// refusing what lives on hardware) and decoding a picked file. Import, the
/// fingerprint and the generated keys themselves are the core's
/// (crates/ssh `keyfile/tests.rs`). Nothing here touches the network or
/// persists to the Keychain.
@Suite("SSHKeyFactory")
struct SSHKeyFactoryTests {

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
}
