import Foundation
import Security
import Testing
import UIKit
@testable import Moshpit

/// B211: a keychain the locked device keeps shut is not a lost credential.
///
/// Every failed read of a connection's saved password or key used to become
/// `credentialUnavailable` — "re-enter your password after a re-install" —
/// and round twenty-two's B163 rightly never retries that one. So a
/// reconnect made while the phone was locked (the transport died in the
/// background, a reply from the lock screen, a launch before the first
/// unlock) read `errSecInteractionNotAllowed`, gave up for good, and told
/// the person to type a password that was there all along. crates/session's
/// rule since W10b (B201): the lock is `CredentialsLocked` — retried on the
/// unlock and the foreground return, with its own quiet words — and a
/// credential that is really gone or unreadable still waits for the person.
/// The retries are crates/session's (`lifecycle.rs`); what the keychain's
/// answer means is this app's, and is tested here.
@Suite("locked keychain: retried on the unlock (B211)")
@MainActor
struct LockedKeychainRedialTests {

    nonisolated static let ref = "moshpit.secret.b211"

    private func makeConnection(_ authMethod: AuthMethod = .password) -> ServerConnection {
        ServerConnection(
            name: "locked",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: authMethod,
            keychainRef: Self.ref)
    }

    private func makeService(_ phone: LockablePhone) -> SSHService {
        SSHService(keychain: KeychainService(backend: phone),
                   protectedDataAvailable: { phone.unlocked })
    }

    /// The credential the Rust core's sign-in asks for.
    private func credential(_ service: SSHService, _ connection: ServerConnection) async throws -> RustCoreCredential {
        try await service.rustCoreCredential(connection) { _, _ in nil }
    }

    // MARK: What the keychain's answer means

    @Test("a read the locked keychain refuses is the lock, for every kind of credential")
    func lockedReadsAreTheLock() async throws {
        let phone = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
        phone.unlocked = false
        let service = makeService(phone)

        var secureEnclave = makeConnection(.key)
        secureEnclave.sshKeyAlgorithmRaw = SSHKeyAlgorithm.seP256.rawValue
        for (name, connection) in [("password", makeConnection(.password)), ("key", makeConnection(.key)),
                                   ("Secure Enclave key", secureEnclave)] {
            let failed = await failure { _ = try await credential(service, connection) }
            #expect(failed.isLocked, "\(name): \(String(describing: failed)) — before B211 every refusal was credentialUnavailable")
        }
        #expect(phone.reads == 3, "every attempt asked the keychain; none was answered from a cache")

        // Nothing about the lock is remembered: unlocked, the same read signs in.
        phone.unlocked = true
        let signedIn = try await credential(service, makeConnection(.password))
        #expect(signedIn == .password("hunter2"))
    }

    @Test("a credential that is really gone or refused still waits for the person")
    func missingCredentialStaysPermanent() async {
        // Unlocked, and the item isn't there: lost to a re-install.
        let gone = makeService(LockablePhone(items: [:]))
        let lost = await failure { _ = try await credential(gone, makeConnection()) }
        #expect(lost.isUnavailable, "\(String(describing: lost))")
        // Face ID cancelled, by the person or the system, or failed too often
        // (B392): its own failure, not a lost credential — and not retried.
        for status in [errSecUserCanceled, errSecAuthFailed] {
            let declined = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
            declined.refusal = status
            let service = makeService(declined)
            let refused = await failure { _ = try await credential(service, makeConnection()) }
            let cancelled = refused.isCancelled
            #expect(cancelled, "\(status): \(String(describing: refused))")
        }
    }

    /// The sealed-launch rule: before the first unlock the keychain refuses —
    /// or reads empty. An empty read then says nothing about the item.
    @Test("a keychain that reads empty while protected data is unavailable is the lock, not a lost item")
    func emptyWhileSealedIsTheLock() async {
        let phone = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
        phone.unlocked = false
        phone.readsEmptyWhileLocked = true
        let service = makeService(phone)
        let sealed = await failure { _ = try await credential(service, makeConnection()) }
        #expect(sealed.isLocked, "an empty read before the unlock taken for a lost credential: \(String(describing: sealed))")
    }

    // MARK: The Rust session

    @Test("the Rust core hears the lock as Locked even where the flag reads unlocked")
    func rustCoreHearsTheLock() async {
        let phone = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
        phone.refusal = errSecInteractionNotAllowed
        let host = RustCredentialHost(service: makeService(phone), protectedData: { true })
        let connection = makeConnection()
        host.register(connection) { _ in nil }
        do {
            _ = try await host.credential(connectionId: connection.id.uuidString)
            Issue.record("a credential with the keychain refusing")
        } catch CredentialFailure.Locked {
            // Retried by the core on the unlock and the way back (B201).
        } catch {
            Issue.record("\(error): the core never retries this one")
        }
    }

    /// B392: Face ID cancelled crossed as Unavailable — "re-install and
    /// re-enter your password" on the card.
    @Test("the Rust core hears a cancelled Face ID as Canceled")
    func rustCoreHearsTheCancel() async {
        let phone = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
        phone.refusal = errSecUserCanceled
        let host = RustCredentialHost(service: makeService(phone), protectedData: { true })
        let connection = makeConnection()
        host.register(connection) { _ in nil }
        do {
            _ = try await host.credential(connectionId: connection.id.uuidString)
            Issue.record("a credential with Face ID cancelled")
        } catch CredentialFailure.Canceled {
            // Said as cancelled; connecting again asks once more.
        } catch {
            Issue.record("\(error): read as a lost credential")
        }
        let signing = RustCredentialHost.signFailure(SSHError.credentialCancelled, protectedDataAvailable: true)
        #expect(signing == CredentialFailure.Canceled)
    }

    @Test("the device's unlock reaches the Rust core's sessions")
    func unlockReachesTheRustCore() async {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost(protectedData: { true }))
        let center = NotificationCenter()
        core.watchUnlocks(on: center)
        core.watchUnlocks(on: center)   // once per registry
        center.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        #expect(await eventually { fake.unlocked >= 1 })
        try? await Task.sleep(for: .milliseconds(50))
        #expect(fake.unlocked == 1, "told twice")
        _ = core
    }

    @Test("the Rust session words the lock as the lock, not the re-install advice")
    func rustCopyIsTheSame() {
        // Its own words: not the re-install advice.
        #expect(SSHError.credentialLocked.description != SSHError.credentialUnavailable.description)
        #expect(SSHError.credentialCancelled.description != SSHError.credentialUnavailable.description)
        #expect(RustCopy.failure(kind: .credentialsLocked, cause: .credentials(why: .locked))
                == SSHError.credentialLocked.description)
        // B392: a cancel — Face ID's or the passphrase card's — is said as
        // one, not as a key that needs its passphrase.
        let cancelled = RustCopy.failure(kind: .credentials, cause: .credentials(why: .cancelled))
        #expect(cancelled == SSHError.credentialCancelled.description)
    }

    /// B782: a phone restored from a backup has the connection and not its
    /// keychain item. The failed connect said to re-sign the app via
    /// SideStore; it says whose secret isn't here and what to do.
    @Test("a saved secret that isn't on this iPhone says whose, never SideStore")
    func missingSecretNamesTheConnection() {
        var nas = ServerConnection()
        nas.name = "nas"
        nas.host = "nas.example.com"
        nas.username = "me"
        nas.keychainRef = "moshpit.secret.nas"
        let unavailable = FailureCauseValue.credentials(why: .unavailable)
        #expect(RustCopy.failure(kind: .credentials, cause: unavailable, connection: nas)
                == "The password for nas isn't on this iPhone. Enter it again.")
        nas.authMethod = .key
        #expect(RustCopy.failure(kind: .credentials, cause: unavailable, connection: nas)
                == "The private key for nas isn't on this iPhone. Paste it again.")
        nas.sshKeyId = UUID()
        #expect(RustCopy.failure(kind: .credentials, cause: unavailable, connection: nas)
                == "The key nas signs in with isn't on this iPhone. Open Edit to pick another key.")
        nas.name = ""
        nas.authMethod = .password
        nas.sshKeyId = nil
        #expect(RustCopy.failure(kind: .credentials, cause: unavailable, connection: nas)
                == "The password for me@nas.example.com isn't on this iPhone. Enter it again.")
        let unnamed = RustCopy.failure(kind: .credentials, cause: unavailable)
        #expect(unnamed == SSHError.credentialUnavailable.description)
        #expect(!unnamed.contains("SideStore"))
    }

    /// The SSHError `body` failed with; nil when it didn't fail, or failed
    /// with anything else.
    private func failure(_ body: () async throws -> Void) async -> SSHError? {
        do {
            try await body()
            return nil
        } catch {
            return error as? SSHError
        }
    }

    private func eventually(_ timeout: Double = 3, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }
}

// MARK: - Fakes

private extension Optional where Wrapped == SSHError {
    var isLocked: Bool {
        if case .credentialLocked? = self { return true }
        return false
    }

    var isUnavailable: Bool {
        if case .credentialUnavailable? = self { return true }
        return false
    }

    var isCancelled: Bool {
        if case .credentialCancelled? = self { return true }
        return false
    }
}

/// The keychain of a phone that can be locked, holding items the way the
/// app keeps them (`WhenUnlockedThisDeviceOnly`, Face ID or not): while
/// locked, every read is refused as the real keychain refuses it —
/// `errSecInteractionNotAllowed`, through the production
/// `KeychainError(osStatus:)` — after a securityd round trip.
/// `readsEmptyWhileLocked` answers "not found" instead, the other way a
/// sealed store reads before the first unlock. `refusal` refuses every read
/// with that status even unlocked: Face ID declined (`errSecUserCanceled`),
/// or asked for in the background (`errSecInteractionNotAllowed`).
final class LockablePhone: KeychainBackend, @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String: Data]
    private var _unlocked = true
    private var _readsEmptyWhileLocked = false
    private var _refusal: OSStatus?
    private var _reads = 0

    init(items: [String: Data]) { self.items = items }

    var unlocked: Bool {
        get { lock.withLock { _unlocked } }
        set { lock.withLock { _unlocked = newValue } }
    }
    var readsEmptyWhileLocked: Bool {
        get { lock.withLock { _readsEmptyWhileLocked } }
        set { lock.withLock { _readsEmptyWhileLocked = newValue } }
    }
    var refusal: OSStatus? {
        get { lock.withLock { _refusal } }
        set { lock.withLock { _refusal = newValue } }
    }
    var reads: Int { lock.withLock { _reads } }

    func load(account: String, prompt: String) throws -> Data {
        let (open, empty, refusal, item): (Bool, Bool, OSStatus?, Data?) = lock.withLock {
            _reads += 1
            return (_unlocked, _readsEmptyWhileLocked, _refusal, items[account])
        }
        Thread.sleep(forTimeInterval: 0.015)   // securityd's round trip
        if !open {
            throw KeychainError(osStatus: empty ? errSecItemNotFound : errSecInteractionNotAllowed)
        }
        if let refusal { throw KeychainError(osStatus: refusal) }
        guard let item else { throw KeychainError(osStatus: errSecItemNotFound) }
        return item
    }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        try lock.withLock {
            guard _unlocked else { throw KeychainError(osStatus: errSecInteractionNotAllowed) }
            items[account] = secret
        }
    }

    func delete(account: String) throws {
        _ = lock.withLock { items.removeValue(forKey: account) }
    }

    func exists(account: String) -> Bool { lock.withLock { items[account] != nil } }
}
