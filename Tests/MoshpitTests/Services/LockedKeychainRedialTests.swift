import Citadel
import Foundation
import NIOCore
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
@Suite("locked keychain: retried on the unlock (B211)")
@MainActor
struct LockedKeychainRedialTests {

    nonisolated static let ref = "moshpit.secret.b211"

    private func makeConnection(_ authMethod: AuthMethod = .password) -> ServerConnection {
        var connection = ServerConnection(
            name: "locked",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: authMethod,
            keychainRef: Self.ref)
        connection.multiplexer = .none   // a plain shell: the shortest start() path
        return connection
    }

    private func makeService(_ phone: LockablePhone) -> SSHService {
        SSHService(keychain: KeychainService(backend: phone),
                   hostKeyValidator: .inMemory(),
                   clientProvider: HandshakeProvider(),
                   protectedDataAvailable: { phone.unlocked })
    }

    /// `SSHService.connect` as the session dials it, signed in once the dial
    /// got past the credential (the stub provider's marker: Citadel's client
    /// can't be made in a test).
    private func dialer(_ service: SSHService, _ dials: DialCounter) -> TerminalViewModel.Dialer {
        { connection, _ in
            dials.bump()
            do {
                _ = try await service.connect(connection)
            } catch SSHError.underlying(let error) where error is SignedInMarker {
                return SSHSession(connection: connection, transport: QuietTransport())
            }
            throw DialFailed()   // the stub provider never returns a client
        }
    }

    private func makeHub(_ service: SSHService, _ dials: DialCounter,
                         unlocks: NotificationCenter) -> SessionHub {
        let hub = SessionHub(metrics: SessionMetricsRegistry(), unlocks: unlocks)
        let dial = dialer(service, dials)
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: dial)
            session.probeTimeout = 0.2
            session.foregroundProbeTimeout = 0.2
            session.stalledDialGrace = 0
            return session
        }
        return hub
    }

    private func unlock(_ phone: LockablePhone, on center: NotificationCenter) {
        phone.unlocked = true
        center.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
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
            do {
                _ = try await service.connect(connection)
                Issue.record("\(name): signed in with the keychain locked")
            } catch SSHError.credentialLocked {
                // The lock: retried, worded as the lock.
            } catch {
                Issue.record("\(name): \(error) — before B211 every refusal was credentialUnavailable")
            }
        }
        // The Rust session reads through the same call.
        let rust = await failure { _ = try await service.rustCoreCredential(makeConnection(.password)) { _, _ in nil } }
        #expect(rust.isLocked, "\(String(describing: rust))")
        #expect(phone.reads == 4, "every attempt asked the keychain; none was answered from a cache")

        // Nothing about the lock is remembered: unlocked, the same read signs in.
        phone.unlocked = true
        let signedIn = await failure { _ = try await service.connect(makeConnection(.password)) }
        guard case .underlying(let error)? = signedIn, error is SignedInMarker else {
            Issue.record("unlocked, the dial didn't get past the credential: \(String(describing: signedIn))")
            return
        }
    }

    @Test("a credential that is really gone or refused still waits for the person")
    func missingCredentialStaysPermanent() async {
        // Unlocked, and the item isn't there: lost to a re-install.
        let gone = makeService(LockablePhone(items: [:]))
        let lost = await failure { _ = try await gone.connect(makeConnection()) }
        #expect(lost.isUnavailable, "\(String(describing: lost))")
        // Face ID cancelled, by the person or the system, or failed too often
        // (B392): its own failure, not a lost credential — and not retried.
        for status in [errSecUserCanceled, errSecAuthFailed] {
            let declined = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
            declined.refusal = status
            let service = makeService(declined)
            let refused = await failure { _ = try await service.connect(makeConnection()) }
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
        let sealed = await failure { _ = try await service.connect(makeConnection()) }
        #expect(sealed.isLocked, "an empty read before the unlock taken for a lost credential: \(String(describing: sealed))")
    }

    @Test("the lock is retried and waits for the unlock; a lost credential is neither")
    func classification() {
        #expect(TerminalViewModel.retryable(SSHError.credentialLocked))
        #expect(TerminalViewModel.waitsForUnlock(SSHError.credentialLocked))
        #expect(TerminalViewModel.waitsForUnlock(SSHError.hostKeysLocked))
        #expect(TerminalViewModel.waitsForUnlock(KnownHostsUnreadable(reason: "locked", locked: true)))
        #expect(!TerminalViewModel.retryable(SSHError.credentialUnavailable))
        #expect(!TerminalViewModel.waitsForUnlock(SSHError.credentialUnavailable))
        #expect(!TerminalViewModel.retryable(SSHError.credentialCancelled))
        #expect(!TerminalViewModel.waitsForUnlock(SSHError.credentialCancelled))
        #expect(!TerminalViewModel.waitsForUnlock(SSHError.connectionFailed))
        #expect(!TerminalViewModel.waitsForUnlock(SSHError.authenticationFailed))
        // Its own words: not the re-install advice.
        #expect(SSHError.credentialLocked.description != SSHError.credentialUnavailable.description)
        #expect(SSHError.credentialCancelled.description != SSHError.credentialUnavailable.description)
    }

    // MARK: The session

    /// The field shape: connected, backgrounded (the cached secret wiped, as
    /// `SessionHub.setForeground(false)` wipes it), the phone locked, and
    /// the transport dies while away. The reconnect can't read the keychain.
    @Test("a reconnect the locked keychain refused dials again when the device unlocks")
    func lockedReconnectRedialsOnUnlock() async throws {
        let phone = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
        let service = makeService(phone)
        let dials = DialCounter()
        let center = NotificationCenter()
        let hub = makeHub(service, dials, unlocks: center)
        let session = hub.prepare(makeConnection())
        await hub.start(session, theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.status == .connected)

        await service.clearAllCachedSecrets()
        phone.unlocked = false
        await session.connectionDropped(reason: "b211")
        #expect(dials.count == 2)
        #expect(session.viewModel.status == .failed(SSHError.credentialLocked.description),
                "the lock read as a lost credential: \(session.viewModel.status)")
        #expect(session.viewModel.failureRetryable)
        #expect(session.viewModel.failureWaitsForUnlock)
        // Not news: the screen keeps saying it is reconnecting, no card.
        #expect(session.viewModel.connState == .reconnecting)
        #expect(session.viewModel.errorMessage == nil)

        unlock(phone, on: center)
        #expect(await eventually { session.viewModel.status == .connected },
                "the unlock dialled nothing: \(session.viewModel.status)")
        #expect(dials.count == 3)
        #expect(!session.viewModel.failureWaitsForUnlock)
        await hub.disconnect(session.id)
    }

    /// Face ID's item read in the background: the device is unlocked, but no
    /// prompt can show, and the keychain refuses the same way. Coming back
    /// to the app dials it again — before B211, never.
    @Test("a credential refused in the background is dialled again on the foreground return")
    func lockedFailureRedialsOnForeground() async throws {
        let phone = LockablePhone(items: [Self.ref: Data("hunter2".utf8)])
        phone.refusal = errSecInteractionNotAllowed   // refused even though unlocked
        let service = makeService(phone)
        let dials = DialCounter()
        let hub = makeHub(service, dials, unlocks: NotificationCenter())
        let session = hub.prepare(makeConnection())
        await hub.start(session, theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.status == .failed(SSHError.credentialLocked.description))
        #expect(session.viewModel.failureRetryable)

        phone.refusal = nil   // in the foreground, Face ID can ask
        await hub.resumeAll(force: false)
        #expect(session.viewModel.status == .connected)
        #expect(dials.count == 2)
        await hub.disconnect(session.id)
    }

    @Test("the unlock leaves a lost credential, and every other failure, to their own rules")
    func unlockLeavesOtherFailuresAlone() async {
        let center = NotificationCenter()
        let gone = LockablePhone(items: [:])
        let goneDials = DialCounter()
        let goneHub = makeHub(makeService(gone), goneDials, unlocks: center)
        let lost = goneHub.prepare(makeConnection())
        await goneHub.start(lost, theme: .githubDark, fontSize: 14)
        #expect(lost.viewModel.status == .failed(SSHError.credentialUnavailable.description))

        // A host it couldn't reach: retried by the tick and the foreground
        // return, not by an unlock.
        let unreachable = DialScript { _ in throw SSHError.connectionFailed }
        let flakyHub = SessionHub(metrics: SessionMetricsRegistry(), unlocks: center)
        flakyHub.makeSession = { SessionHub.ActiveSession(connection: $0, dial: unreachable.dialer) }
        let flaky = flakyHub.prepare(makeConnection())
        await flakyHub.start(flaky, theme: .githubDark, fontSize: 14)

        center.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        await goneHub.deviceUnlocked()
        await flakyHub.deviceUnlocked()
        try? await Task.sleep(for: .milliseconds(100))
        #expect(goneDials.count == 1, "a lost credential was dialled again on the unlock")
        #expect(unreachable.dials == 1, "the unlock is not a network change")
        #expect(lost.viewModel.status == .failed(SSHError.credentialUnavailable.description))
        await goneHub.disconnect(lost.id)
        await flakyHub.disconnect(flaky.id)
    }

    /// The trusted host keys are one keychain item too: crates/session's
    /// `waits_for_unlock` covers both.
    @Test("host keys the locked keychain kept shut are dialled again on the unlock")
    func lockedHostKeysRedialOnUnlock() async {
        let script = DialScript { _ in
            try await Task.sleep(for: .milliseconds(20))
            throw SSHError.map(KnownHostsUnreadable(reason: "the device is locked", locked: true))
        }
        let center = NotificationCenter()
        let hub = SessionHub(metrics: SessionMetricsRegistry(), unlocks: center)
        hub.makeSession = { SessionHub.ActiveSession(connection: $0, dial: script.dialer) }
        let session = hub.prepare(makeConnection())
        await hub.start(session, theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.failureWaitsForUnlock)

        script.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        center.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        #expect(await eventually { session.viewModel.status == .connected })
        #expect(script.dials == 2)
        await hub.disconnect(session.id)
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

    @Test("the Rust session words the lock as the Swift session does")
    func rustCopyIsTheSame() {
        #expect(RustCopy.failure(kind: .credentialsLocked, cause: .credentials(why: .locked))
                == SSHError.credentialLocked.description)
        // B392: a cancel — Face ID's or the passphrase card's — is said as
        // one, not as a key that needs its passphrase.
        let cancelled = RustCopy.failure(kind: .credentials, cause: .credentials(why: .cancelled))
        #expect(cancelled == SSHError.credentialCancelled.description)
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

/// Where a sign-in got past the credential: the dial itself, answered after a
/// handshake's worth of time with this marker.
struct SignedInMarker: Error, Equatable {}

private struct HandshakeProvider: SSHClientProvider {
    func connect(host: String, port: Int,
                 authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        try await Task.sleep(for: .milliseconds(30))
        throw SignedInMarker()
    }

    func connect(on channel: Channel,
                 authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        try await Task.sleep(for: .milliseconds(30))
        throw SignedInMarker()
    }
}
