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
/// Since B132 the delegate asks nobody: a key the user has to judge fails the
/// handshake with a `HostKeyNeedsDecision` naming the host and the key, and
/// `SSHService.connect` puts the question through that call's own handlers.
/// What remains worth proving here is that two concurrent delegates sharing
/// ONE `HostKeyValidator` — exactly the shape of `SSHService.shared`, whose
/// single validator is reused by every session — never cross-talk: each
/// names its own host and key, and a trust given for one decides only that
/// one. Each delegate is exercised directly (no network socket needed:
/// `validateHostKey` is a plain method taking an `EventLoopPromise`).
@Suite("TOFUHostKeyDelegate — concurrent handshakes stay isolated")
struct TOFUHostKeyDelegateTests {
    // Freshly generated ed25519 OpenSSH public keys (no private material
    // involved) — `NIOSSHPublicKey` has no simpler test-only constructor.
    private static let keyA = try! NIOSSHPublicKey(
        openSSHPublicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDgQhgqywG4eG3cXK2gEELsrEXf5Q4v4JzrxSrrRVcFg")
    private static let keyB = try! NIOSSHPublicKey(
        openSSHPublicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMFa+0q0K1zmVeI1zHCP+qXugextFVAzuNqxA0jtU957")

    private static func decision(_ promise: EventLoopPromise<Void>) async -> HostKeyNeedsDecision? {
        do {
            try await promise.futureResult.get()
            return nil
        } catch {
            return error as? HostKeyNeedsDecision
        }
    }

    @Test("two concurrent unknown-host handshakes each name only their OWN host and key")
    func concurrentHandshakesDontCrossWire() async throws {
        // ONE shared validator — the realistic shape: SSHService.shared's
        // single HostKeyValidator instance, reused by every ActiveSession
        // (primary connections AND the mosh -CC sidecar alike).
        let validator = HostKeyValidator.inMemory()
        let delegateA = TOFUHostKeyDelegate(validator: validator, host: "host-a.example", port: 22)
        let delegateB = TOFUHostKeyDelegate(validator: validator, host: "host-b.example", port: 22)

        // A REAL event loop, not `EmbeddedEventLoop`: `validateHostKey` hops
        // onto an unstructured `Task` (a Swift-concurrency thread) to await
        // the validator, then fulfills the promise from THAT thread —
        // `EmbeddedEventLoop` requires same-thread confinement.
        let group = MultiThreadedEventLoopGroup(numberOfThreads: 1)
        let loop = group.next()
        let promiseA: EventLoopPromise<Void> = loop.makePromise()
        let promiseB: EventLoopPromise<Void> = loop.makePromise()

        // Both handshakes in flight at once.
        delegateA.validateHostKey(hostKey: Self.keyA, validationCompletePromise: promiseA)
        delegateB.validateHostKey(hostKey: Self.keyB, validationCompletePromise: promiseB)

        let askA = try #require(await Self.decision(promiseA), "A was let through unasked")
        let askB = try #require(await Self.decision(promiseB), "B was let through unasked")
        #expect(askA == HostKeyNeedsDecision(host: "host-a.example", port: 22,
                                             fingerprint: HostKeyValidator.fingerprint(of: Self.keyA), previous: nil))
        #expect(askB == HostKeyNeedsDecision(host: "host-b.example", port: 22,
                                             fingerprint: HostKeyValidator.fingerprint(of: Self.keyB), previous: nil))
        #expect(await validator.snapshot().isEmpty, "nothing may be trusted before the user says so")

        // The user trusts A only: A's next handshake passes, B's still asks,
        // and B's key presented for A is a CHANGED key, not a pass.
        await validator.trust(host: askA.host, port: askA.port, fingerprint: askA.fingerprint)
        let againA: EventLoopPromise<Void> = loop.makePromise()
        let againB: EventLoopPromise<Void> = loop.makePromise()
        let swapped: EventLoopPromise<Void> = loop.makePromise()
        delegateA.validateHostKey(hostKey: Self.keyA, validationCompletePromise: againA)
        delegateB.validateHostKey(hostKey: Self.keyB, validationCompletePromise: againB)
        delegateA.validateHostKey(hostKey: Self.keyB, validationCompletePromise: swapped)
        try await againA.futureResult.get()
        #expect(await Self.decision(againB)?.host == "host-b.example")
        #expect(await Self.decision(swapped)?.previous == askA.fingerprint)

        let snap = await validator.snapshot()
        #expect(snap["host-a.example:22"] == askA.fingerprint)
        #expect(snap["host-b.example:22"] == nil)

        try await group.shutdownGracefully()
    }
}
