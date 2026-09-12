import Foundation
import NIOCore
import NIOPosix
import NIOSSH
import Testing
@testable import Moshpit

/// Regression coverage for the TOFU host-key race: `SSHService.connect(...)`
/// used to read `onUnknownHost`/`onChangedHost` off two separate MUTABLE
/// actor properties, installed by a prior `setHostKeyHandlers()` call. Since
/// install-then-connect are two separate `await`s on the shared
/// `SSHService.shared` singleton (every session, plus background
/// reconnect/keepalive and the mosh -CC sidecar, all funnel through it), one
/// session's `setHostKeyHandlers` could land in the gap between another
/// session's install and its own connect — silently swapping which handler
/// judged which host's key. The fix threads the handlers through `connect(...)`
/// as PARAMETERS instead, so each call is self-contained.
///
/// `SSHService` itself has no shared handler state left to race on, so what
/// remains worth proving is the invariant the fix relies on: two concurrent
/// `TOFUHostKeyDelegate`s sharing ONE `HostKeyValidator` — exactly the shape
/// of `SSHService.shared`, whose single validator is reused by every
/// session — never cross-talk. Each delegate is exercised directly (no
/// network socket needed: `validateHostKey` is a plain method taking an
/// `EventLoopPromise`), simulating two sessions' host-key handshakes
/// overlapping in flight.
@Suite("TOFUHostKeyDelegate — concurrent handshakes stay isolated")
struct TOFUHostKeyDelegateTests {
    // Freshly generated ed25519 OpenSSH public keys (no private material
    // involved) — `NIOSSHPublicKey` has no simpler test-only constructor.
    private static let keyA = try! NIOSSHPublicKey(
        openSSHPublicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDgQhgqywG4eG3cXK2gEELsrEXf5Q4v4JzrxSrrRVcFg")
    private static let keyB = try! NIOSSHPublicKey(
        openSSHPublicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMFa+0q0K1zmVeI1zHCP+qXugextFVAzuNqxA0jtU957")

    @Test("two concurrent unknown-host handshakes each fire only their OWN handler")
    func concurrentHandshakesDontCrossWire() async throws {
        // ONE shared validator — the realistic shape: SSHService.shared's
        // single HostKeyValidator instance, reused by every ActiveSession
        // (primary connections AND the mosh -CC sidecar alike).
        let validator = HostKeyValidator.inMemory()

        let firedA = Captured()
        let firedB = Captured()

        // Host A's handler trusts; host B's denies — if the two delegates
        // ever cross-wired (the old bug's shape), A would observe B's deny
        // or vice versa.
        let delegateA = TOFUHostKeyDelegate(
            validator: validator, host: "host-a.example", port: 22,
            onNewHost: { _, _, _ in firedA.fire(); return true },
            onChangedHost: { _, _, _, _ in false })
        let delegateB = TOFUHostKeyDelegate(
            validator: validator, host: "host-b.example", port: 22,
            onNewHost: { _, _, _ in firedB.fire(); return false },
            onChangedHost: { _, _, _, _ in false })

        // A REAL event loop, not `EmbeddedEventLoop`: `validateHostKey` hops
        // onto an unstructured `Task` (a Swift-concurrency thread) to await
        // the validator/handler, then fulfills the promise from THAT thread —
        // `EmbeddedEventLoop` requires same-thread confinement and NIO logs
        // "API misuse" (soon a hard crash) if fulfilled cross-thread. A
        // `MultiThreadedEventLoopGroup`-backed loop is built for exactly this
        // cross-thread fulfillment.
        let group = MultiThreadedEventLoopGroup(numberOfThreads: 1)
        let loop = group.next()
        let promiseA: EventLoopPromise<Void> = loop.makePromise()
        let promiseB: EventLoopPromise<Void> = loop.makePromise()

        // Fire both handshakes without awaiting between them — this is the
        // "overlapping in flight" scenario the old shared-mutable-property
        // design could scramble.
        delegateA.validateHostKey(hostKey: Self.keyA, validationCompletePromise: promiseA)
        delegateB.validateHostKey(hostKey: Self.keyB, validationCompletePromise: promiseB)

        // A's own handler trusted → success. B's own handler denied →
        // HostKeyRejected. Neither swapped.
        try await promiseA.futureResult.get()
        await #expect(throws: HostKeyRejected.self) {
            try await promiseB.futureResult.get()
        }

        #expect(firedA.wasFired, "host A's own onNewHost should have fired")
        #expect(firedB.wasFired, "host B's own onNewHost should have fired")

        // The validator persisted exactly what EACH delegate's own closure
        // decided, keyed by ITS OWN host — not the other's.
        let snap = await validator.snapshot()
        #expect(snap["host-a.example:22"] != nil, "A's accept should be persisted")
        #expect(snap["host-b.example:22"] == nil, "B's deny must not be persisted")

        try await group.shutdownGracefully()
    }
}

/// Sendable bool-flag holder so test closures can mark themselves invoked.
private final class Captured: @unchecked Sendable {
    private let lock = NSLock()
    private var fired = false
    var wasFired: Bool {
        lock.lock(); defer { lock.unlock() }
        return fired
    }
    func fire() {
        lock.lock(); defer { lock.unlock() }
        fired = true
    }
}
