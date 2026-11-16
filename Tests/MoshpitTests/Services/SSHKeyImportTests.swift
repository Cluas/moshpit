import Foundation
import Testing
@testable import Moshpit

/// A key that imports has to connect. The connect path signs with one form
/// only — an unencrypted openssh-key-v1 file — and import used to store
/// whatever was pasted, so PEM keys and every key behind a
/// passphrase imported fine and then failed each connection. Import now
/// converts to that form, asking for the passphrase, and refuses what it can't.
/// Each form's conversion is the core's (crates/ssh `keyfile/tests.rs`);
/// here, what the Swift side says when import refuses.
@Suite("SSH key import: what imports, connects")
struct SSHKeyImportTests {

    /// Whether the connect path signs in with `stored` as a key's Keychain
    /// item, stored the way Add Key stores it: the key the Rust core is
    /// handed (``SSHService/rustCoreCredential(_:onKeyPassphrase:)``, which
    /// converts a key kept as pasted) is one its key reader signs with.
    private static func connectPathAccepts(_ stored: Data) async throws -> Bool {
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.saveSecret(stored, forRef: ref, requireBiometry: false)
        let service = SSHService(keychain: keychain, protectedDataAvailable: { true })
        let connection = ServerConnection(name: "imported", host: "192.0.2.1", username: "tester",
                                          authMethod: .key, keychainRef: ref)
        guard case .key(let key)? = try? await service.rustCoreCredential(connection, onKeyPassphrase: { _, _ in nil })
        else { return false }
        return (try? sshKeySigningParts(text: key)) != nil
    }

    private static func fixture(_ name: String) -> SSHKeyFixtures.Key {
        SSHKeyFixtures.all.first { $0.name == name }!
    }

    // MARK: Passphrases

    @Test("A key behind a passphrase isn't imported without it", arguments: SSHKeyFixtures.encrypted)
    func passphraseRequired(_ key: SSHKeyFixtures.Key) {
        #expect(throws: SSHKeyImportError.passphraseRequired) {
            _ = try SSHKeyFactory.importKey(privatePEM: key.privateKey, passphrase: nil, publicKeyLine: nil, comment: "")
        }
        #expect(throws: SSHKeyImportError.passphraseRequired) {
            _ = try SSHKeyFactory.importKey(privatePEM: key.privateKey, passphrase: "", publicKeyLine: nil, comment: "")
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
