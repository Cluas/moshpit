import Foundation
import Testing
@testable import Moshpit

/// Covers the pure, connection-free logic in `SSHKeyFactory`: key generation
/// (structure of the emitted OpenSSH artifacts), best-effort import/algorithm
/// detection, the SHA256 fingerprint helper, and the passphrase-strength
/// heuristic. Nothing here touches the network or persists to the Keychain.
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

    @Test("RSA-4096 generation emits an ssh-rsa line wrapping a real PKCS#1 key")
    func generateRSA4096() throws {
        let gen = try SSHKeyFactory.generate(algorithm: .rsa4096, comment: "server")
        #expect(gen.algorithm == .rsa4096)
        #expect(gen.publicKeyLine.hasPrefix("ssh-rsa "))
        #expect(gen.fingerprint.hasPrefix("SHA256:"))
        let pem = String(decoding: gen.privateBlob, as: UTF8.self)
        #expect(pem.contains("-----BEGIN RSA PRIVATE KEY-----"))
        // The middle field must be valid base64 of the ssh-rsa blob.
        let parts = gen.publicKeyLine.split(separator: " ")
        #expect(parts.count == 3)
        #expect(Data(base64Encoded: String(parts[1])) != nil)
    }

    // MARK: - Generate: unsupported

    @Test("ecdsa-sk generation is rejected as unsupported (lives on hardware)")
    func generateEcdsaSKThrows() {
        #expect(throws: SSHKeyFactory.KeyError.self) {
            _ = try SSHKeyFactory.generate(algorithm: .ecdsaSK, comment: "yk")
        }
    }

    // MARK: - Import: algorithm detection

    @Test("import rejects a body without a PRIVATE KEY marker")
    func importRejectsNonPEM() {
        #expect(throws: SSHKeyFactory.KeyError.self) {
            _ = try SSHKeyFactory.importKey(
                privatePEM: "just some pasted text", publicKeyLine: nil, comment: "")
        }
    }

    @Test("import detects RSA from a BEGIN RSA header")
    func importDetectsRSAFromHeader() throws {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----"
        let imported = try SSHKeyFactory.importKey(privatePEM: pem, publicKeyLine: nil, comment: "")
        #expect(imported.algorithm == .rsa4096)
    }

    @Test("import detects RSA from an ssh-rsa public line even with a generic PEM header")
    func importDetectsRSAFromPublicLine() throws {
        let pem = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----"
        let imported = try SSHKeyFactory.importKey(
            privatePEM: pem, publicKeyLine: "ssh-rsa AAAAB3Nza comment", comment: "")
        #expect(imported.algorithm == .rsa4096)
    }

    @Test("import detects ecdsa-sk from an ecdsa-sha2 public line")
    func importDetectsEcdsaFromPublicLine() throws {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----"
        let imported = try SSHKeyFactory.importKey(
            privatePEM: pem, publicKeyLine: "ecdsa-sha2-nistp256 AAAA comment", comment: "")
        #expect(imported.algorithm == .ecdsaSK)
    }

    @Test("import falls back to ed25519 for a generic OpenSSH PEM with no public line")
    func importDefaultsToEd25519() throws {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----"
        let imported = try SSHKeyFactory.importKey(privatePEM: pem, publicKeyLine: nil, comment: "")
        #expect(imported.algorithm == .ed25519)
        // No public line → no fingerprint, no public line echoed back.
        #expect(imported.fingerprint.isEmpty)
        #expect(imported.publicKeyLine.isEmpty)
    }

    @Test("import computes a fingerprint from a decodable public-key blob")
    func importComputesFingerprint() throws {
        // A public line whose middle field is valid base64 → fingerprint is derived.
        let blob = Data("public-blob".utf8).base64EncodedString()
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----"
        let imported = try SSHKeyFactory.importKey(
            privatePEM: pem, publicKeyLine: "ssh-ed25519 \(blob) me", comment: "")
        #expect(imported.fingerprint == SSHKeyFactory.fingerprint(of: Data("public-blob".utf8)))
    }

    @Test("import trims surrounding whitespace from the pasted PEM before storing")
    func importTrimsWhitespace() throws {
        let core = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----"
        let imported = try SSHKeyFactory.importKey(
            privatePEM: "\n\n  \(core)  \n", publicKeyLine: nil, comment: "")
        #expect(String(decoding: imported.privateBlob, as: UTF8.self) == core)
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

    // MARK: - Passphrase strength heuristic

    @Test("empty passphrase scores exactly 0")
    func strengthEmpty() {
        #expect(SSHKeyFactory.passphraseStrength("") == 0)
    }

    @Test("strength is clamped to the 0…1 range")
    func strengthClamped() {
        let strong = SSHKeyFactory.passphraseStrength("Corr3ctH0rse!BatteryStaple#Longer$Still")
        #expect(strong <= 1.0)
        #expect(strong > 0)
    }

    @Test("adding character classes raises the score for equal-length inputs")
    func strengthRewardsVariety() {
        // Same length, widening alphabet each step → strictly increasing entropy.
        let lower = SSHKeyFactory.passphraseStrength("aaaaaaaa")
        let mixed = SSHKeyFactory.passphraseStrength("aaaaAAAA")
        let withDigits = SSHKeyFactory.passphraseStrength("aaAA1234")
        let withSymbols = SSHKeyFactory.passphraseStrength("aA1!aA1!")
        #expect(lower < mixed)
        #expect(mixed < withDigits)
        #expect(withDigits < withSymbols)
    }

    @Test("a longer passphrase from the same alphabet scores higher")
    func strengthRewardsLength() {
        let short = SSHKeyFactory.passphraseStrength("abc")
        let long = SSHKeyFactory.passphraseStrength("abcabcabc")
        #expect(long > short)
    }
}
