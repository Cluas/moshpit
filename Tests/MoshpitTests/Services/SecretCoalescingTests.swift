import Foundation
import Testing
@testable import Moshpit

/// `SSHService.resolveSecret`'s in-flight coalescing: concurrent resolutions of
/// one connection must share ONE keychain read — each read is a Face ID prompt,
/// and the actor's reentrancy otherwise turns one user action into two sheets.
/// Not hypothetical: sessions resume together on the way back, and the Rust
/// core asks for a connection's credential on every dial
/// (``SSHService/rustCoreCredential(_:onKeyPassphrase:)``).

/// In-memory backend that counts loads and holds each one open for `delay`.
/// The hold is the point: it pins the first resolution inside the keychain
/// actor long enough for the second caller to hit `resolveSecret` while the
/// first is still in flight — the exact reentrancy window being tested. A
/// zero-delay backend would mostly test the cache instead.
private final class SleepyCountingBackend: KeychainBackend, @unchecked Sendable {
    private let lock = NSLock()
    private var store: [String: Data]
    private var loadCount = 0
    private let delay: TimeInterval

    init(seed: [String: Data], delay: TimeInterval) {
        self.store = seed
        self.delay = delay
    }

    var loads: Int {
        lock.lock(); defer { lock.unlock() }
        return loadCount
    }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        lock.lock(); defer { lock.unlock() }
        store[account] = secret
    }

    func load(account: String, prompt: String) throws -> Data {
        lock.lock()
        loadCount += 1
        let data = store[account]
        lock.unlock()
        // Blocks the keychain actor's thread, not SSHService — callers of
        // `resolveSecret` are suspended at an await, so the service stays
        // reentrant, which is what lets a second sign-in race in.
        Thread.sleep(forTimeInterval: delay)
        guard let data else { throw KeychainError.notFound }
        return data
    }

    func delete(account: String) throws {
        lock.lock(); defer { lock.unlock() }
        store.removeValue(forKey: account)
    }

    func exists(account: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return store[account] != nil
    }
}

@Suite("SSHService — secret coalescing")
struct SecretCoalescingTests {

    private static let ref = "coalesce-test-ref"

    private func makeConnection() -> ServerConnection {
        ServerConnection(
            name: "test",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password,
            keychainRef: Self.ref)
    }

    private func makeService(backend: SleepyCountingBackend) -> SSHService {
        SSHService(keychain: KeychainService(backend: backend), protectedDataAvailable: { true })
    }

    /// The credential the Rust core's sign-in asks for; its failure is
    /// dropped, the reads it made are what is counted.
    private func signIn(_ service: SSHService, _ connection: ServerConnection) async {
        _ = try? await service.rustCoreCredential(connection) { _, _ in nil }
    }

    @Test("two concurrent sign-ins share one keychain read")
    func concurrentSignInsCoalesce() async {
        let backend = SleepyCountingBackend(
            seed: [Self.ref: Data("hunter2".utf8)], delay: 0.15)
        let service = makeService(backend: backend)
        let connection = makeConnection()

        async let first: Void = signIn(service, connection)
        async let second: Void = signIn(service, connection)
        _ = await (first, second)

        #expect(backend.loads == 1,
                "concurrent resolutions must join the in-flight read — every extra read is an extra Face ID sheet")
    }

    @Test("a failed read is not pinned — the next attempt reads again")
    func failureIsNotCached() async {
        // No seed: every load throws `notFound`.
        let backend = SleepyCountingBackend(seed: [:], delay: 0)
        let service = makeService(backend: backend)
        let connection = makeConnection()

        await signIn(service, connection)
        await signIn(service, connection)

        #expect(backend.loads == 2,
                "a failed resolution left registered would replay its error to every future sign-in")
    }

    @Test("a background wipe is not undone by a read that finishes after it")
    func wipeSurvivesInFlightRead() async throws {
        let backend = SleepyCountingBackend(
            seed: [Self.ref: Data("hunter2".utf8)], delay: 0.3)
        let service = makeService(backend: backend)
        let connection = makeConnection()

        // The first sign-in starts its (slow) keychain read…
        let inFlight = Task { await signIn(service, connection) }
        // …the app backgrounds mid-read and wipes every cached secret…
        try await Task.sleep(for: .milliseconds(60))
        await service.clearAllCachedSecrets()
        await inFlight.value

        // …so the next sign-in must go back to the keychain. If the late
        // completion had repopulated the cache, this would be a cache hit and
        // the wipe's "authenticate on every read survives backgrounding"
        // guarantee would be quietly gone.
        await signIn(service, connection)
        #expect(backend.loads == 2)
    }
}
