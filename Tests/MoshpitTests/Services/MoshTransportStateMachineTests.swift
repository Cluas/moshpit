import Foundation
import Network
import Testing
@testable import Moshpit

/// Exercises `MoshTransport.handleDatagram`'s state machine — the apply / gap /
/// parse-failure branches and SRTT smoothing — by injecting a fake
/// `DatagramChannel` and feeding synthetic, real-crypto datagrams. No live UDP
/// connection is involved, so the order-sensitive apply logic that the
/// production receive loop depends on can be asserted deterministically.
///
/// The parse-failure case guards the fix from e61a9b3 ("stop silently acking
/// content that failed to decode"): a diff that fails to parse must NOT advance
/// `appliedHostNum`, or mosh's server treats the (lost) content as delivered
/// and never resends it — the blank-screen bug.
@Suite("MoshTransport state machine")
struct MoshTransportStateMachineTests {

    // MARK: Fake channel

    /// A `DatagramChannel` that records sends and lets a test hand datagrams to
    /// the armed receive completion — no sockets, fully synchronous.
    final class FakeDatagramChannel: DatagramChannel, @unchecked Sendable {
        var state: NWConnection.State = .ready
        var onStateChange: (@Sendable (NWConnection.State) -> Void)?
        var onBetterPath: (@Sendable (Bool) -> Void)?
        private(set) var sent: [Data] = []
        private var pendingReceive: (@Sendable (Data?, Error?) -> Void)?

        private(set) var startCalls = 0
        private(set) var cancelCalls = 0
        func start() { startCalls += 1 }
        func restart() {}
        func cancel() { cancelCalls += 1 }
        func send(_ datagram: Data) { sent.append(datagram) }
        func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {
            pendingReceive = completion
        }

        /// Hand a datagram to the currently-armed receive completion.
        func deliver(_ datagram: Data) {
            let completion = pendingReceive
            pendingReceive = nil
            completion?(datagram, nil)
        }
    }

    // MARK: Fixtures

    let key = Data((0..<16).map { UInt8($0) })

    func makeTransport() throws -> (MoshTransport, FakeDatagramChannel, MoshCrypto) {
        let creds = MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key)
        let channel = FakeDatagramChannel()
        let transport = try MoshTransport(credentials: creds, channel: channel)
        let crypto = try MoshCrypto(key: key)
        return (transport, channel, crypto)
    }

    /// Variant with a short return-path deadline so the watchdog can be tripped
    /// without an 8-second real-time wait.
    func makeTransport(deadlineNanos: UInt64) throws -> (MoshTransport, FakeDatagramChannel, MoshCrypto) {
        let creds = MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key)
        let channel = FakeDatagramChannel()
        let transport = try MoshTransport(
            credentials: creds, channel: channel, returnPathDeadlineNanos: deadlineNanos)
        let crypto = try MoshCrypto(key: key)
        return (transport, channel, crypto)
    }

    /// Variant with a short keystroke TTL so input expiry can be exercised
    /// without a 10-second real-time wait.
    func makeTransport(keystrokeTTLMs: UInt64) throws -> (MoshTransport, FakeDatagramChannel, MoshCrypto) {
        let creds = MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key)
        let channel = FakeDatagramChannel()
        let transport = try MoshTransport(
            credentials: creds, channel: channel, keystrokeTTLMs: keystrokeTTLMs)
        let crypto = try MoshCrypto(key: key)
        return (transport, channel, crypto)
    }

    /// A main-actor box for observing the `@MainActor onReturnPathDead` callback.
    @MainActor final class FiredBox { var fired = false }

    /// Decode the client→server instruction diff out of a sealed datagram the
    /// transport sent — the payload a real mosh-server would apply as input.
    func clientDiff(of datagram: Data, crypto: MoshCrypto) throws -> Data {
        let message = try crypto.open(datagram)
        var assembler = FragmentAssembler()
        let fragment = try TransportFragment(parsing: message.payload)
        guard let payload = assembler.receive(fragment) else { return Data() }
        let bytes = try MoshCompression.decompress(payload)
        return try TransportInstruction(parsing: bytes).diff
    }

    /// Encode a server→client transport instruction the way mosh-server would:
    /// protobuf → zlib → single transport fragment → sealed datagram.
    func makeDatagram(
        crypto: MoshCrypto,
        seq: UInt64,
        id: UInt64,
        oldNum: UInt64,
        newNum: UInt64,
        ackNum: UInt64 = 0,
        diff: Data = Data(),
        timestamp: UInt16 = 0,
        timestampReply: UInt16 = 0
    ) throws -> Data {
        var instruction = TransportInstruction()
        instruction.oldNum = oldNum
        instruction.newNum = newNum
        instruction.ackNum = ackNum
        instruction.diff = diff

        let compressed = try MoshCompression.compress(instruction.encoded())
        let fragments = TransportFragment.fragment(payload: compressed, id: id, mtu: 1300)
        #expect(fragments.count == 1, "test payloads should fit a single fragment")
        let payload = fragments[0].encoded()

        let message = MoshCrypto.Message(
            timestamp: timestamp, timestampReply: timestampReply, payload: payload)
        return try crypto.seal(message, seq: seq, direction: .toClient)
    }

    /// A well-formed host diff carrying `text` as terminal output — mirrors the
    /// hostinput.proto layout `HostMessage(parsing:)` expects (Instruction[1] →
    /// HostBytes[2] → hoststring[4]).
    func hostDiff(_ text: String) -> Data {
        var hostBytes = ProtoWriter()
        hostBytes.appendField(4, bytes: Data(text.utf8))
        var instruction = ProtoWriter()
        instruction.appendField(2, bytes: hostBytes.bytes)
        var message = ProtoWriter()
        message.appendField(1, bytes: instruction.bytes)
        return message.data
    }

    /// mosh's 16-bit millisecond clock, matching `MoshTransport`.
    func nowMs() -> UInt64 { DispatchTime.now().uptimeNanoseconds / 1_000_000 }
    func ts16() -> UInt16 { UInt16(truncatingIfNeeded: nowMs()) }

    // MARK: Apply

    @Test("an in-order diff applies its host bytes and advances appliedHostNum")
    func inOrderApply() async throws {
        let (transport, _, crypto) = try makeTransport()

        let packet = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 1, diff: hostDiff("hello"))
        await transport.handleDatagram(packet)

        var iterator = transport.hostStream.makeAsyncIterator()
        let chunk = await iterator.next()
        #expect(chunk == Data("hello".utf8))

        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 1)
        #expect(diag.parseFailures == 0)
        #expect(diag.gapEvents == 0)
        #expect(diag.datagramsReceived == 1)
    }

    @Test("requestFullRedraw nudges the width and never rewinds the applied state")
    func fullRedrawResync() async throws {
        let (transport, channel, crypto) = try makeTransport()
        await transport.start(cols: 80, rows: 24)

        // Screen at state 3.
        let first = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 3, diff: hostDiff("screen"))
        await transport.handleDatagram(first)
        #expect(await transport.currentDiagnostics.appliedHostNum == 3)

        let before = channel.sent.count
        await transport.requestFullRedraw()

        // A resize-pair flush went out (the width nudge that makes mosh's
        // Display reset `initialized` and repaint every cell)…
        #expect(channel.sent.count > before)
        let diff = try clientDiff(of: channel.sent[channel.sent.count - 1], crypto: crypto)
        #expect(!diff.isEmpty, "the nudge must carry the resize events")

        // …and the applied state is UNTOUCHED. The first shipped version
        // acked state 0 instead; a stock server culls old states and ignores
        // that ack, while the rewind turned every subsequent in-order diff
        // into a gap — a frozen screen. This is the regression guard.
        #expect(await transport.currentDiagnostics.appliedHostNum == 3)

        // In-order diffs keep applying — no gap, no freeze.
        let next = try makeDatagram(
            crypto: crypto, seq: 1, id: 2, oldNum: 3, newNum: 4, diff: hostDiff("more"))
        await transport.handleDatagram(next)
        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 4)
        #expect(diag.gapEvents == 0)

        // The transport's own idea of its size survived the nudge round trip:
        // a same-size resize is still de-duped into silence.
        let quiet = channel.sent.count
        await transport.resize(cols: 80, rows: 24)
        #expect(channel.sent.count == quiet)
    }

    @Test("consecutive in-order diffs apply in sequence")
    func inOrderSequence() async throws {
        let (transport, _, crypto) = try makeTransport()

        let first = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 1, diff: hostDiff("a"))
        let second = try makeDatagram(
            crypto: crypto, seq: 1, id: 2, oldNum: 1, newNum: 2, diff: hostDiff("b"))
        await transport.handleDatagram(first)
        await transport.handleDatagram(second)

        var iterator = transport.hostStream.makeAsyncIterator()
        #expect(await iterator.next() == Data("a".utf8))
        #expect(await iterator.next() == Data("b".utf8))

        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 2)
        #expect(diag.parseFailures == 0)
        #expect(diag.gapEvents == 0)
    }

    @Test("an empty diff advances appliedHostNum without emitting host bytes")
    func emptyDiffAdvances() async throws {
        let (transport, _, crypto) = try makeTransport()

        let packet = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 3, diff: Data())
        await transport.handleDatagram(packet)

        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 3)
        #expect(diag.parseFailures == 0)
        #expect(diag.gapEvents == 0)
    }

    // MARK: Gap

    @Test("a gap (oldNum doesn't match) is counted and does not advance state")
    func gapDoesNotAdvance() async throws {
        let (transport, _, crypto) = try makeTransport()

        // appliedHostNum is 0, but this diff claims to extend state 5.
        let packet = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 5, newNum: 6, diff: hostDiff("skipped"))
        await transport.handleDatagram(packet)

        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 0, "gap must not advance the applied state")
        #expect(diag.gapEvents == 1)
        #expect(diag.parseFailures == 0)
    }

    // MARK: Parse failure (regression guard for e61a9b3)

    @Test("a diff that fails to parse does NOT advance appliedHostNum")
    func parseFailureDoesNotAdvance() async throws {
        let (transport, channel, crypto) = try makeTransport()

        // A non-empty diff whose bytes are not valid host protobuf: field 1 /
        // wire-type 2 declaring a 5-byte length with only 1 byte to follow.
        let malformed = Data([0x0A, 0x05, 0x01])
        #expect((try? HostMessage(parsing: malformed)) == nil, "fixture must fail to parse")

        let packet = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 1, diff: malformed)
        await transport.handleDatagram(packet)

        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 0,
                "silently acking unparseable content is the blank-screen bug from e61a9b3")
        #expect(diag.parseFailures == 1)
        #expect(diag.gapEvents == 0)
        // We still flush — acking our TRUE (unchanged) state so the server
        // keeps re-sending the content rather than considering it delivered.
        #expect(!channel.sent.isEmpty)
    }

    @Test("a later valid diff still can't apply on top of an un-advanced state")
    func parseFailureThenGap() async throws {
        let (transport, _, crypto) = try makeTransport()

        let bad = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 1, diff: Data([0x0A, 0x05, 0x01]))
        await transport.handleDatagram(bad)

        // Server's next diff builds on state 1, which we never applied → gap.
        let next = try makeDatagram(
            crypto: crypto, seq: 1, id: 2, oldNum: 1, newNum: 2, diff: hostDiff("x"))
        await transport.handleDatagram(next)

        let diag = await transport.currentDiagnostics
        #expect(diag.appliedHostNum == 0)
        #expect(diag.parseFailures == 1)
        #expect(diag.gapEvents == 1)
    }

    // MARK: Roaming

    @Test("an authenticated reply clears the roaming flag")
    func authenticatedReplyClearsRoaming() async throws {
        let (transport, channel, crypto) = try makeTransport()
        await transport.start(cols: 80, rows: 24)

        // Flip roaming on via the better-path callback the channel exposes.
        channel.onBetterPath?(true)
        // Give the actor hop a moment to land, then confirm it flipped.
        try await Task.sleep(nanoseconds: 50_000_000)
        #expect(await transport.currentRoaming == true)

        let packet = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 1, diff: hostDiff("hi"))
        await transport.handleDatagram(packet)
        #expect(await transport.currentRoaming == false)
    }

    // MARK: Return-path watchdog

    @Test("the return-path watchdog fires when a ready socket never receives a datagram")
    func returnPathDeadFiresWhenSilent() async throws {
        let (transport, channel, _) = try makeTransport(deadlineNanos: 100_000_000)  // 100ms
        let box = await FiredBox()
        await MainActor.run { transport.onReturnPathDead = { box.fired = true } }

        await transport.start(cols: 80, rows: 24)
        // Socket becomes ready but the server's replies never arrive (dead
        // return path — VPN/proxy dropping inbound UDP).
        channel.onStateChange?(.ready)

        // Past the deadline with zero datagrams delivered.
        try await Task.sleep(nanoseconds: 400_000_000)
        #expect(await MainActor.run { box.fired } == true)
    }

    @Test("the return-path watchdog does not fire once any datagram arrives")
    func returnPathDeadSilentWhenDataArrives() async throws {
        let (transport, channel, crypto) = try makeTransport(deadlineNanos: 300_000_000)  // 300ms
        let box = await FiredBox()
        await MainActor.run { transport.onReturnPathDead = { box.fired = true } }

        await transport.start(cols: 80, rows: 24)
        channel.onStateChange?(.ready)

        // A reply lands well before the deadline → watchdog must stand down.
        let packet = try makeDatagram(
            crypto: crypto, seq: 0, id: 1, oldNum: 0, newNum: 1, diff: hostDiff("hi"))
        channel.deliver(packet)

        try await Task.sleep(nanoseconds: 600_000_000)
        #expect(await MainActor.run { box.fired } == false)
        #expect(await transport.currentDiagnostics.datagramsReceived >= 1)
    }

    // MARK: Keystroke delivery TTL

    @Test("keystrokes past their TTL stop retransmitting; resizes survive the prune")
    func expiredKeystrokesDropOnFlush() async throws {
        let (transport, channel, crypto) = try makeTransport(keystrokeTTLMs: 50)
        await transport.start(cols: 80, rows: 24)          // resize queued as state 1
        await transport.send(Data("STALE-SECRET".utf8))    // keystroke state 2, flushes now

        // The link stays dead past the TTL — nothing acked, nothing received.
        try await Task.sleep(nanoseconds: 200_000_000)

        let before = channel.sent.count
        await transport.resume()                           // the recovery-time flush
        #expect(channel.sent.count > before)

        let diff = try clientDiff(of: channel.sent[channel.sent.count - 1], crypto: crypto)
        #expect(diff.range(of: Data("STALE-SECRET".utf8)) == nil,
                "an expired keystroke replaying into the shell is the ghost-input bug")
        #expect(!diff.isEmpty, "the resize event is idempotent state and must survive")
    }

    @Test("fresh keystrokes keep retransmitting — typing through a brief blip still delivers")
    func freshKeystrokesSurviveFlush() async throws {
        let (transport, channel, crypto) = try makeTransport(keystrokeTTLMs: 10_000)
        await transport.start(cols: 80, rows: 24)
        await transport.send(Data("FRESH-KEYS".utf8))

        let before = channel.sent.count
        await transport.resume()
        #expect(channel.sent.count > before)

        let diff = try clientDiff(of: channel.sent[channel.sent.count - 1], crypto: crypto)
        #expect(diff.range(of: Data("FRESH-KEYS".utf8)) != nil,
                "un-expired keystrokes are exactly what SSP must keep retransmitting")
    }

    // MARK: SRTT

    @Test("SRTT smooths toward the sampled round-trip time and damps spikes")
    func srttConvergesAndDamps() async throws {
        let (transport, _, crypto) = try makeTransport()
        var seq: UInt64 = 0
        var id: UInt64 = 1

        // Six synthetic round trips each reporting a ~120ms RTT. Using
        // oldNum == newNum == 0 keeps this to the SRTT path with no apply/gap.
        for _ in 0..<6 {
            let reply = ts16() &- 120
            let packet = try makeDatagram(
                crypto: crypto, seq: seq, id: id, oldNum: 0, newNum: 0, timestampReply: reply)
            seq += 1; id += 1
            await transport.handleDatagram(packet)
        }
        let converged = await transport.currentSRTT
        // Generous bounds: the measured RTT is inflated slightly by encode time,
        // but must land near the 120ms sample and nowhere near 0 or 600.
        #expect(converged > 90 && converged < 220,
                "SRTT should converge near the 120ms sample, got \(converged)")

        // A single large spike is blended in (0.875·srtt + 0.125·sample), not
        // adopted wholesale — so it moves up but stays far below 600ms.
        let spike = try makeDatagram(
            crypto: crypto, seq: seq, id: id, oldNum: 0, newNum: 0, timestampReply: ts16() &- 600)
        await transport.handleDatagram(spike)
        let afterSpike = await transport.currentSRTT
        #expect(afterSpike > converged, "the spike should pull SRTT upward")
        #expect(afterSpike < 360, "but the EWMA should damp it well below the 600ms sample, got \(afterSpike)")
    }

    @Test("a forced resume replaces the flow instead of diagnosing it")
    func forcedResumeRebuildsFlow() async throws {
        // The zombie-ready case: after a real suspension NWConnection can
        // report .ready over a flow iOS already reclaimed, so restart() (legal
        // only from .failed/.waiting) does nothing and every send blackholes.
        // resume(force:) must not consult the state at all — cancel, redial,
        // rewire. The factory hands back a fresh channel; both the teardown of
        // the old and the start of the new are the observable contract.
        let creds = MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key)
        let old = FakeDatagramChannel()
        let fresh = FakeDatagramChannel()
        let transport = try MoshTransport(credentials: creds, channel: old,
                                          makeChannel: { fresh })
        await transport.start(cols: 80, rows: 24)
        #expect(old.startCalls == 1)

        await transport.resume(force: true)
        #expect(old.cancelCalls == 1, "the zombie flow must be torn down")
        #expect(fresh.startCalls == 1, "the replacement flow must be started")

        // The un-forced path stays conservative: a .ready channel is left
        // alone (restart is a no-op there anyway), only flushed.
        await transport.resume()
        #expect(fresh.cancelCalls == 0)
    }
}
