import Foundation
import Testing
@testable import Moshpit

/// B125: keys stored as they were pasted — every key imported before B29, and
/// any key pasted into a connection's own form — sign in. Only an
/// unencrypted openssh-key-v1 file did: a PEM key, one behind a passphrase,
/// or one with Windows line endings failed every connection, for good. Now
/// such a key is converted on its first use (its passphrase asked for once),
/// and the keychain item replaced only once the converted key reads back to
/// the same public key.
@Suite("Stored keys: converted on first use")
struct StoredKeyTests {

    private static func fixture(_ name: String) -> SSHKeyFixtures.Key {
        SSHKeyFixtures.all.first { $0.name == name }!
    }

    /// The key blob of a fixture's `.pub` line — what ssh-keygen wrote.
    private static func keygenBlob(_ key: SSHKeyFixtures.Key) -> Data {
        Data(base64Encoded: String(key.publicKey.split(separator: " ")[1]))!
    }

    private static func publicBlob(of text: String) throws -> Data {
        try SSHKeyFactory.publicKeyBlob(of: text)
    }

    private static func crlf(_ text: String) -> String {
        text.replacingOccurrences(of: "\n", with: "\r\n")
    }

    // MARK: At connect

    private static func service(_ keychain: KeychainService) -> SSHService {
        SSHService(keychain: keychain, protectedDataAvailable: { true })
    }

    private static func connection(_ ref: String) -> ServerConnection {
        ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me", authMethod: .key, keychainRef: ref)
    }

    /// The key the Rust core is handed to sign in with
    /// (``SSHService/rustCoreCredential(_:onKeyPassphrase:)``), checked to
    /// be one its key reader signs with; else the error the credential
    /// failed with.
    private static func signIn(_ service: SSHService, _ connection: ServerConnection,
                               onKeyPassphrase: @escaping SSHService.KeyPassphraseHandler = { _, _ in nil })
        async -> Result<String, Error> {
        do {
            guard case .key(let key) = try await service.rustCoreCredential(connection,
                                                                              onKeyPassphrase: onKeyPassphrase) else {
                return .failure(NotAKey())
            }
            _ = try sshKeySigningParts(text: key)
            return .success(key)
        } catch {
            return .failure(error)
        }
    }

    @Test("A legacy unencrypted key signs in, and its keychain item becomes the converted key",
          arguments: SSHKeyFixtures.all.filter { !$0.isEncrypted && !$0.name.hasSuffix("· OpenSSH") })
    func legacyKeyIsConvertedAtConnect(_ key: SSHKeyFixtures.Key) async throws {
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let signedIn = await Self.signIn(Self.service(keychain), Self.connection(ref))
        #expect(signedIn.signs, "\(signedIn)")

        let stored = try await keychain.loadPrivateKey(forRef: ref)
        #expect(stored != key.privateKey, "the original stayed")
        #expect(StoredKey.connectPathReads(stored))
        #expect(try Self.publicBlob(of: stored) == Self.keygenBlob(key))
    }

    @Test("Windows line endings stored as pasted: signs in, stored converted")
    func legacyCRLFKeyIsConvertedAtConnect() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(Self.crlf(key.privateKey), forRef: ref, requireBiometry: false)
        #expect(await Self.signIn(Self.service(keychain), Self.connection(ref)).signs)
        #expect(StoredKey.connectPathReads(try await keychain.loadPrivateKey(forRef: ref)))
    }

    @Test("A key that already signs in is left exactly as stored")
    func usableKeyIsUntouched() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        #expect(await Self.signIn(Self.service(keychain), Self.connection(ref)).signs)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == key.privateKey)
    }

    @Test("A legacy key behind a passphrase asks once, then never again")
    func passphraseIsAskedOnce() async throws {
        let key = Self.fixture("RSA 2048 · PEM (ssh-keygen -m PEM) · passphrase")
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let asked = Asked()
        let answer: SSHService.KeyPassphraseHandler = { _, wrong in
            asked.note(wrong)
            return wrong ? SSHKeyFixtures.passphrase : "not it"
        }

        let service = Self.service(keychain)
        #expect(await Self.signIn(service, Self.connection(ref), onKeyPassphrase: answer).signs)
        #expect(asked.calls == [false, true], "asked \(asked.calls)")
        #expect(try Self.publicBlob(of: try await keychain.loadPrivateKey(forRef: ref)) == Self.keygenBlob(key))

        // Again, on the same service (its cache) and on a fresh one (the keychain).
        #expect(await Self.signIn(service, Self.connection(ref), onKeyPassphrase: answer).signs)
        let fresh = Self.service(keychain)
        #expect(await Self.signIn(fresh, Self.connection(ref), onKeyPassphrase: answer).signs)
        #expect(asked.calls == [false, true], "asked again: \(asked.calls)")
    }

    @Test("Cancelling the passphrase fails the connect plainly and keeps the key as it was")
    func passphraseCancelled() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH · passphrase")
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let signedIn = await Self.signIn(Self.service(keychain), Self.connection(ref))
        guard case .failure(SSHError.keyNeedsPassphrase) = signedIn else {
            Issue.record("\(signedIn)")
            return
        }
        #expect(try await keychain.loadPrivateKey(forRef: ref) == key.privateKey)
        #expect(!(SSHError.keyNeedsPassphrase.description.isEmpty))
    }

    @Test("B126: a P-384 key signs in, left exactly as stored")
    func p384SignsIn() async throws {
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(SSHKeyFixtures.ecdsaP384, forRef: ref, requireBiometry: false)
        let signedIn = await Self.signIn(Self.service(keychain), Self.connection(ref))
        #expect(signedIn.signs, "\(signedIn)")
        #expect(try await keychain.loadPrivateKey(forRef: ref) == SSHKeyFixtures.ecdsaP384)
    }

    @Test("A DSA key fails at once, saying so, before the core dials")
    func dsaSaysWhy() async throws {
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(Self.dsaKey, forRef: ref, requireBiometry: false)
        let signedIn = await Self.signIn(Self.service(keychain), Self.connection(ref))
        guard case .failure(let error) = signedIn,
              case SSHError.keyUnusable(.unsupportedType("DSA")) = error else {
            Issue.record("\(signedIn)")
            return
        }
        #expect((error as? SSHError)?.description.contains("DSA") == true)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == Self.dsaKey)
    }

    /// An openssh-key-v1 file whose clear public half is a DSA key's: what
    /// is behind it never matters, the kind is refused.
    static let dsaKey: String = {
        func string(_ data: Data) -> Data {
            var length = UInt32(data.count).bigEndian
            return Data(bytes: &length, count: 4) + data
        }
        let blob = string(Data("ssh-dss".utf8)) + string(Data(repeating: 1, count: 20))
        var payload = Data("openssh-key-v1\0".utf8)
        payload += string(Data("none".utf8)) + string(Data("none".utf8)) + string(Data())
        payload += Data([0, 0, 0, 1]) + string(blob) + string(Data(repeating: 0, count: 16))
        return "-----BEGIN OPENSSH PRIVATE KEY-----\n\(payload.base64EncodedString())\n-----END OPENSSH PRIVATE KEY-----\n"
    }()

    @Test("When the keychain won't take the converted key, the original stays and the connect still goes ahead")
    func replacementRefusedKeepsTheOriginal() async throws {
        let key = Self.fixture("ECDSA P-256 · PEM (ssh-keygen -m PEM)")
        let ref = "moshpit.secret.test"
        let backend = ReplaceRefusingBackend(seed: [ref: Data(key.privateKey.utf8)])
        let keychain = KeychainService(backend: backend)
        #expect(await Self.signIn(Self.service(keychain), Self.connection(ref)).signs)
        #expect(backend.replaceAttempts == 1)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == key.privateKey)
    }

    @Test("Replacing never creates an item that isn't there")
    func replaceNeedsAnItem() async throws {
        let keychain = KeychainService.inMemory()
        await #expect(throws: KeychainError.notFound) {
            try await keychain.replacePrivateKey("x", forRef: "moshpit.secret.none")
        }
        #expect(await keychain.contains(ref: "moshpit.secret.none") == false)
    }
}

/// A credential that isn't a key, where a key was saved.
private struct NotAKey: Error {}

private extension Result where Success == String, Failure == Error {
    /// A key the Rust core signs in with was handed over.
    var signs: Bool {
        if case .success = self { return true }
        return false
    }
}

private final class Asked: @unchecked Sendable {
    private let lock = NSLock()
    private var log: [Bool] = []
    var calls: [Bool] { lock.withLock { log } }
    func note(_ wrong: Bool) { lock.withLock { log.append(wrong) } }
}

/// An in-memory keychain whose in-place replacement always fails.
private final class ReplaceRefusingBackend: KeychainBackend, @unchecked Sendable {
    private let inner: InMemoryKeychainBackend
    private let lock = NSLock()
    private var attempts = 0
    var replaceAttempts: Int { lock.withLock { attempts } }

    init(seed: [String: Data]) { inner = InMemoryKeychainBackend(seed: seed) }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        try inner.save(secret: secret, account: account, requireBiometry: requireBiometry)
    }
    func load(account: String, prompt: String) throws -> Data { try inner.load(account: account, prompt: prompt) }
    func delete(account: String) throws { try inner.delete(account: account) }
    func exists(account: String) -> Bool { inner.exists(account: account) }
    func replace(secret: Data, account: String, prompt: String) throws {
        lock.withLock { attempts += 1 }
        throw KeychainError.interactionNotAllowed
    }
}
