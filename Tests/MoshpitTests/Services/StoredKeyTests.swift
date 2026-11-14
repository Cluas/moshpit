import Citadel
import Foundation
import NIOCore
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

    private static func indented(_ text: String) -> String {
        text.split(separator: "\n", omittingEmptySubsequences: false).map { "    " + $0 }.joined(separator: "\n")
    }

    // MARK: Assessing a stored key

    @Test("Every form a key was pasted in comes out as an openssh-key-v1 file of the same key",
          arguments: SSHKeyFixtures.all)
    func everyFormConverts(_ key: SSHKeyFixtures.Key) throws {
        let expected = Self.keygenBlob(key)
        let stored = key.privateKey
        var assessment = try StoredKey.assess(stored)
        if key.isEncrypted {
            #expect(assessment == .needsPassphrase)
            assessment = try StoredKey.assess(stored, passphrase: key.passphrase)
        }
        switch assessment {
        case .usable:
            #expect(StoredKey.connectPathReads(stored))
            #expect(try Self.publicBlob(of: stored) == expected)
        case .converted(let converted):
            #expect(converted.hasPrefix("-----BEGIN OPENSSH PRIVATE KEY-----\n"))
            #expect(StoredKey.connectPathReads(converted), "Citadel still can't read it")
            #expect(!SSHKeyFactory.isEncrypted(converted))
            #expect(try Self.publicBlob(of: converted) == expected, "not the same key")
            #expect(!StoredKey.connectPathReads(stored) || key.isEncrypted,
                    "a key the connect path read as stored was converted anyway")
        case .needsPassphrase:
            Issue.record("still wants a passphrase")
        }
    }

    @Test("Windows line endings and indented lines: converted, the same key")
    func whitespaceConverts() throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        for stored in [Self.crlf(key.privateKey), Self.indented(key.privateKey),
                       Self.crlf(Self.indented(key.privateKey))] {
            #expect(!StoredKey.connectPathReads(stored), "the connect path reads it as stored after all")
            guard case .converted(let converted) = try StoredKey.assess(stored) else {
                Issue.record("not converted")
                continue
            }
            #expect(StoredKey.connectPathReads(converted))
            #expect(try Self.publicBlob(of: converted) == Self.keygenBlob(key))
        }
    }

    @Test("A key that can't sign in in any form says which kind it is")
    func unusableKeysSayWhy() throws {
        // B126: a P-384 key signs in, as stored.
        #expect(try StoredKey.assess(SSHKeyFixtures.ecdsaP384) == .usable)
        #expect(throws: SSHKeyImportError.unsupportedType("DSA")) {
            _ = try StoredKey.assess(Self.dsaKey)
        }
        let key = Self.fixture("RSA 2048 · OpenSSH · passphrase")
        #expect(throws: SSHKeyImportError.wrongPassphrase) {
            _ = try StoredKey.assess(key.privateKey, passphrase: "not it")
        }
    }

    @Test("The picker can tell a key that can't sign in from its public key line")
    func refusedTypes() {
        // B126: P-384 and P-521 are no longer refused.
        #expect(StoredKey.refusedType(ofPublicKeyLine: "ecdsa-sha2-nistp384 AAAA x") == nil)
        #expect(StoredKey.refusedType(ofPublicKeyLine: "ecdsa-sha2-nistp521 AAAA") == nil)
        #expect(StoredKey.refusedType(ofPublicKeyLine: "ssh-dss AAAA") == "DSA")
        for usable in ["ssh-ed25519 AAAA", "ssh-rsa AAAA c", "ecdsa-sha2-nistp256 AAAA", "", "garbage"] {
            #expect(StoredKey.refusedType(ofPublicKeyLine: usable) == nil, "\(usable)")
        }
    }

    // MARK: At connect

    private static func service(_ keychain: KeychainService, _ provider: StopAtDialProvider) -> SSHService {
        SSHService(keychain: keychain, hostKeyValidator: .inMemory(), clientProvider: provider)
    }

    private static func connection(_ ref: String) -> ServerConnection {
        ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me", authMethod: .key, keychainRef: ref)
    }

    /// Connects until the dial; the dial's sentinel means the key was read,
    /// any other error is returned.
    private static func connectToDial(_ service: SSHService, _ connection: ServerConnection,
                                      onKeyPassphrase: @escaping SSHService.KeyPassphraseHandler = { _, _ in nil })
        async -> Error? {
        do {
            _ = try await service.connect(connection, onKeyPassphrase: onKeyPassphrase)
            return nil
        } catch SSHError.underlying(let error) where error is ReachedTheDial {
            return nil
        } catch {
            return error
        }
    }

    @Test("A legacy unencrypted key signs in, and its keychain item becomes the converted key",
          arguments: SSHKeyFixtures.all.filter { !$0.isEncrypted && !$0.name.hasSuffix("· OpenSSH") })
    func legacyKeyIsConvertedAtConnect(_ key: SSHKeyFixtures.Key) async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let provider = StopAtDialProvider()

        let error = await Self.connectToDial(Self.service(keychain, provider), Self.connection(ref))
        #expect(error == nil, "\(String(describing: error))")
        #expect(provider.dials >= 1, "never reached the server")

        let stored = try await keychain.loadPrivateKey(forRef: ref)
        #expect(stored != key.privateKey, "the original stayed")
        #expect(StoredKey.connectPathReads(stored))
        #expect(try Self.publicBlob(of: stored) == Self.keygenBlob(key))
    }

    @Test("Windows line endings stored as pasted: signs in, stored converted")
    func legacyCRLFKeyIsConvertedAtConnect() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(Self.crlf(key.privateKey), forRef: ref, requireBiometry: false)
        let provider = StopAtDialProvider()
        #expect(await Self.connectToDial(Self.service(keychain, provider), Self.connection(ref)) == nil)
        #expect(provider.dials == 1)
        #expect(StoredKey.connectPathReads(try await keychain.loadPrivateKey(forRef: ref)))
    }

    @Test("A key that already signs in is left exactly as stored")
    func usableKeyIsUntouched() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH")
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        #expect(await Self.connectToDial(Self.service(keychain, StopAtDialProvider()), Self.connection(ref)) == nil)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == key.privateKey)
    }

    @Test("A legacy key behind a passphrase asks once, then never again")
    func passphraseIsAskedOnce() async throws {
        let key = Self.fixture("RSA 2048 · PEM (ssh-keygen -m PEM) · passphrase")
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let asked = Asked()
        let answer: SSHService.KeyPassphraseHandler = { _, wrong in
            asked.note(wrong)
            return wrong ? SSHKeyFixtures.passphrase : "not it"
        }

        let service = Self.service(keychain, StopAtDialProvider())
        #expect(await Self.connectToDial(service, Self.connection(ref), onKeyPassphrase: answer) == nil)
        #expect(asked.calls == [false, true], "asked \(asked.calls)")
        #expect(try Self.publicBlob(of: try await keychain.loadPrivateKey(forRef: ref)) == Self.keygenBlob(key))

        // Again, on the same service (its cache) and on a fresh one (the keychain).
        #expect(await Self.connectToDial(service, Self.connection(ref), onKeyPassphrase: answer) == nil)
        let fresh = Self.service(keychain, StopAtDialProvider())
        #expect(await Self.connectToDial(fresh, Self.connection(ref), onKeyPassphrase: answer) == nil)
        #expect(asked.calls == [false, true], "asked again: \(asked.calls)")
    }

    @Test("Cancelling the passphrase fails the connect plainly and keeps the key as it was")
    func passphraseCancelled() async throws {
        let key = Self.fixture("Ed25519 · OpenSSH · passphrase")
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let provider = StopAtDialProvider()
        let error = await Self.connectToDial(Self.service(keychain, provider), Self.connection(ref))
        guard case SSHError.keyNeedsPassphrase? = error else {
            Issue.record("\(String(describing: error))")
            return
        }
        #expect(provider.dials == 0)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == key.privateKey)
        #expect(!(SSHError.keyNeedsPassphrase.description.isEmpty))
    }

    @Test("B126: a P-384 key signs in, left exactly as stored")
    func p384SignsIn() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(SSHKeyFixtures.ecdsaP384, forRef: ref, requireBiometry: false)
        let provider = StopAtDialProvider()
        let error = await Self.connectToDial(Self.service(keychain, provider), Self.connection(ref))
        #expect(error == nil, "\(String(describing: error))")
        #expect(provider.dials == 1)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == SSHKeyFixtures.ecdsaP384)
    }

    @Test("A DSA key fails at once, saying so, without dialling")
    func dsaSaysWhy() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(Self.dsaKey, forRef: ref, requireBiometry: false)
        let provider = StopAtDialProvider()
        let error = await Self.connectToDial(Self.service(keychain, provider), Self.connection(ref))
        guard case SSHError.keyUnusable(.unsupportedType("DSA"))? = error else {
            Issue.record("\(String(describing: error))")
            return
        }
        #expect(provider.dials == 0)
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
        let provider = StopAtDialProvider()
        #expect(await Self.connectToDial(Self.service(keychain, provider), Self.connection(ref)) == nil)
        #expect(provider.dials == 1)
        #expect(backend.replaceAttempts == 1)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == key.privateKey)
    }

    @MainActor
    @Test("The terminal's view model asks through its card and hands the answer back")
    func viewModelAsksThroughItsCard() async throws {
        let answer = Answer()
        let viewModel = TerminalViewModel(connection: Self.connection("r"), dial: { connection, prompts in
            answer.set(await prompts.onKeyPassphrase(connection, false))
            throw SSHError.keyNeedsPassphrase
        })
        let start = Task { await viewModel.start() }
        var waited = 0
        while viewModel.keyPassphrasePrompt == nil, waited < 300 {
            waited += 1
            try await Task.sleep(for: .milliseconds(10))
        }
        let prompt = try #require(viewModel.keyPassphrasePrompt)
        #expect(prompt.host == "192.0.2.1")
        #expect(!prompt.wrong)
        prompt.decide("pw")
        await start.value
        #expect(answer.value == "pw")
        #expect(viewModel.keyPassphrasePrompt == nil)
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

/// What `StopAtDialProvider` throws once the connect path reached the dial.
private struct ReachedTheDial: Error {}

private final class StopAtDialProvider: SSHClientProvider, @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var dials: Int { lock.withLock { count } }

    func connect(host: String, port: Int, authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        lock.withLock { count += 1 }
        throw ReachedTheDial()
    }

    func connect(on channel: Channel, authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        lock.withLock { count += 1 }
        throw ReachedTheDial()
    }
}

private final class Answer: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: String?
    var value: String? { lock.withLock { stored } }
    func set(_ value: String?) { lock.withLock { stored = value } }
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
