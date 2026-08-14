import Citadel
import Foundation
import NIOCore
import Testing
@testable import Moshpit

/// `SSHService.resolveSecret`'s in-flight coalescing: concurrent resolutions of
/// one connection must share ONE keychain read — each read is a Face ID prompt,
/// and the actor's reentrancy otherwise turns one user action into two sheets.
/// Not hypothetical: mosh+tmux authenticates two SSH connections (bootstrap +
/// `-CC` sidecar) with the same credential, and `resumeAll` is now concurrent.

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
        // reentrant, which is what lets a second connect race in.
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

/// Never opens a socket; throws so the test can tell "credential resolution
/// succeeded and dispatch reached the network" apart from a pre-flight failure.
private struct StubProviderError: Error {}
private struct StubClientProvider: SSHClientProvider {
    func connect(host: String, port: Int,
                 authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SSHClient {
        throw StubProviderError()
    }
    func connect(on channel: Channel,
                 authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SSHClient {
        throw StubProviderError()
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
        SSHService(
            keychain: KeychainService(backend: backend),
            hostKeyValidator: .inMemory(),
            clientProvider: StubClientProvider())
    }

    @Test("two concurrent connects share one keychain read")
    func concurrentConnectsCoalesce() async {
        let backend = SleepyCountingBackend(
            seed: [Self.ref: Data("hunter2".utf8)], delay: 0.15)
        let service = makeService(backend: backend)
        let connection = makeConnection()

        // Both fail at the stub provider — AFTER credential resolution, which
        // is the part under test.
        async let first: Void = { _ = try? await service.connect(connection) }()
        async let second: Void = { _ = try? await service.connect(connection) }()
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

        _ = try? await service.connect(connection)
        _ = try? await service.connect(connection)

        #expect(backend.loads == 2,
                "a failed resolution left registered would replay its error to every future connect")
    }

    @Test("a background wipe is not undone by a read that finishes after it")
    func wipeSurvivesInFlightRead() async throws {
        let backend = SleepyCountingBackend(
            seed: [Self.ref: Data("hunter2".utf8)], delay: 0.3)
        let service = makeService(backend: backend)
        let connection = makeConnection()

        // First connect starts its (slow) keychain read…
        let inFlight = Task { _ = try? await service.connect(connection) }
        // …the app backgrounds mid-read and wipes every cached secret…
        try await Task.sleep(for: .milliseconds(60))
        await service.clearAllCachedSecrets()
        await inFlight.value

        // …so the next connect must go back to the keychain. If the late
        // completion had repopulated the cache, this would be a cache hit and
        // the wipe's "authenticate on every read survives backgrounding"
        // guarantee would be quietly gone.
        _ = try? await service.connect(connection)
        #expect(backend.loads == 2)
    }
}
