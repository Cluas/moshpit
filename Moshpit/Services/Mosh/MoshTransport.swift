import Foundation
import Network

/// Running protocol counters surfaced for on-device diagnosis of a black
/// screen (cursor + input work, but no content ever renders) — a failure
/// mode this app's tests can't reproduce, since it needs real packet
/// loss/reordering or a specific remote shell config that loopback testing
/// never exercises. Screenshotting this (see the terminal pill's
/// long-press) is the only practical way to see which branch a live,
/// unreproducible session actually hit.
struct MoshDiagnostics: Hashable {
    /// Total UDP datagrams received (before decrypt/parse — a low count
    /// here despite a "connected" pill points at network/NAT, not us).
    var datagramsReceived: UInt64 = 0
    /// `HostMessage(parsing:)` failures on an otherwise-applicable diff.
    /// Before e61a9b3 these were silently acked (permanent content loss);
    /// now they fall through to re-diff instead — but a high count still
    /// means something about this remote's output isn't decoding.
    var parseFailures: UInt64 = 0
    /// Diffs received with a gap (newNum > appliedHostNum but oldNum
    /// doesn't match) — expected occasionally on lossy links; a count that
    /// keeps climbing without `appliedHostNum` ever advancing means we're
    /// stuck re-diffing and never landing a diff we can apply.
    var gapEvents: UInt64 = 0
    /// Highest display state actually applied. Stuck at 0 with datagrams
    /// arriving means every diff so far has hit the gap or parse-failure
    /// branch — the server has content queued that we've never accepted.
    var appliedHostNum: UInt64 = 0

    // Send side — the input path's vitals, added for the "typing stopped
    // working over mosh" report (2026-08-17) that no loopback rig
    // reproduces. Screenshot the popover while it's happening:
    /// Encrypted datagrams sent. Frozen while typing = flush isn't reaching
    /// the socket (channel wedged / send loop broken).
    var packetsSent: UInt64 = 0
    /// Newest queued user state vs the newest the server acked. `lastUserNum`
    /// climbing with keystrokes while `serverAckedUserNum` stands still =
    /// input is leaving the app and dying on the wire (or the server's acks
    /// are dying on the way back).
    var lastUserNum: UInt64 = 0
    var serverAckedUserNum: UInt64 = 0
    /// NWConnection state, verbatim. Anything but "ready" while the pill
    /// says live is the story.
    var channelState: String = "?"
}

/// The mosh State Synchronization Protocol client over UDP.
///
/// Responsibilities (transportsender.cc / transport.cc, client side):
///   - Encrypt/decrypt datagrams (`MoshCrypto`), fragment/reassemble
///     (`TransportFragment`), and zlib (`MoshCompression`) the transport
///     `Instruction`s.
///   - Maintain the client→server `UserStream` (keystrokes + resize) and the
///     server→client display state numbers, applying host bytes to the
///     terminal **only** when a diff's `old_num` matches the state we've
///     already applied (so the ANSI composes correctly).
///   - Echo timestamps for RTT, surfacing a smoothed SRTT + roaming flag.
///
/// Host output is delivered through `hostStream`, mirroring `SSHSession`, so
/// the terminal layer consumes mosh and SSH identically.
actor MoshTransport {

    // MARK: Public output

    nonisolated let hostStream: AsyncStream<Data>
    private let hostContinuation: AsyncStream<Data>.Continuation

    /// Called on the main actor with (srttMs, roaming) whenever they change.
    @MainActor var onMetrics: ((Double, Bool) -> Void)?

    /// Called on the main actor after every processed datagram with the
    /// running protocol counters below. There's no way to reproduce a
    /// black-screen report over loopback — real packet loss/reordering and a
    /// given remote's shell config don't happen on localhost — so this is a
    /// user-screenshottable diagnostic (see the terminal pill's long-press)
    /// rather than something exercised by this app's own tests.
    @MainActor var onDiagnostics: ((MoshDiagnostics) -> Void)?

    /// Called on the main actor, at most once, when the UDP connection reached
    /// `.ready` and we've been flushing for `returnPathDeadline` seconds but
    /// have received **zero** datagrams back — i.e. the return path is dead
    /// (commonly a VPN/proxy or firewall that passes our outbound UDP but drops
    /// the server's replies). SSH/TCP to the same host works in that case, so
    /// the session layer uses this to stop staring at a black screen and offer
    /// a fall back to SSH, instead of retrying UDP into a void forever.
    @MainActor var onReturnPathDead: (() -> Void)?

    // MARK: Connection

    private let credentials: MoshCredentials
    private var channel: DatagramChannel
    /// Builds a REPLACEMENT datagram channel — flow rebuild's raw material.
    /// nil (tests without a factory) means rebuildFlow() re-wires the same
    /// injected fake, which still exercises the sequencing.
    private let makeChannel: (() -> DatagramChannel)?
    private var started = false
    private var closed = false

    // MARK: Crypto + protocol state

    private let crypto: MoshCrypto
    private var sendSeq: UInt64 = 0

    /// Outgoing user-input states. num 0 == empty baseline. `queuedAtMs`
    /// (monotonic, `nowMs()`) drives the keystroke delivery TTL — see
    /// `pruneExpiredKeystrokes`.
    private var userStates: [(num: UInt64, events: [UserEvent], queuedAtMs: UInt64)] = [(0, [], 0)]
    private var serverAckedUserNum: UInt64 = 0

    /// How long an un-acked keystroke keeps retransmitting before it is
    /// treated as lost. SSP's default — retransmit forever — is right for a
    /// blip (keys typed through a roam land once the path heals) but wrong
    /// across a long-dead link: keystrokes typed into a frozen screen
    /// suddenly replay into the shell whenever connectivity returns, minutes
    /// later ("the terminal typed my old input by itself") — and a command
    /// the user already retyped after reconnecting runs twice. Resize events
    /// are exempt: they're idempotent state, not actions. Injectable so
    /// tests can expire input without a 10-second wait.
    private let keystrokeTTLMs: UInt64
    static let defaultKeystrokeTTLMs: UInt64 = 10_000

    /// Highest server display state we've applied (and therefore ack).
    private var appliedHostNum: UInt64 = 0
    private var receiveAssembler = FragmentAssembler()
    private var nextInstructionId: UInt64 = 1

    // MARK: Diagnostics (see `onDiagnostics`)

    private var datagramsReceived: UInt64 = 0
    private var parseFailures: UInt64 = 0
    private var gapEvents: UInt64 = 0

    private var cols = 80
    private var rows = 24

    // MARK: Timestamps / SRTT

    private var savedRemoteTimestamp: UInt16 = 0
    private var savedRemoteTimestampAt: UInt64 = 0   // local ms
    private var srtt: Double = 0
    private var roaming = false
    private var heartbeat: Task<Void, Never>?

    // MARK: Return-path watchdog (see `onReturnPathDead`)

    /// Set true the instant the first datagram arrives; gates the watchdog so a
    /// working (even very slow) link never trips it, and a mid-session stall
    /// (a different, roaming-recoverable case) is left alone.
    private var sawAnyDatagram = false
    private var returnPathWatchdog: Task<Void, Never>?

    // MARK: Mid-session liveness (Plan B — the "typing died after a while")

    /// Monotonic ms of the last datagram that arrived / the last flush that
    /// sent. "Sent since heard" for longer than the liveness deadline is the
    /// zombie-flow signature: we're talking, nobody's answering, and
    /// NWConnection still claims .ready.
    private var lastInboundMs: UInt64 = 0
    private var lastSendMs: UInt64 = 0
    /// Flow rebuilds since the last inbound datagram — Plan C's dryness
    /// counter. 0 the moment anything arrives.
    private var rebuildsSinceInbound = 0
    private var flowRebuilds: UInt64 = 0
    private var livenessMonitor: Task<Void, Never>?
    /// How long "sent but heard nothing" may last before the flow is
    /// declared a zombie and rebuilt. Heartbeats flush every few seconds, so
    /// a healthy link never goes this long silent in both directions.
    static let livenessDeadlineMs: UInt64 = 9_000
    /// How long to keep flushing with no reply before declaring the return path
    /// dead. Generous: the first reply is one RTT after our first flush, so
    /// even a multi-second-latency link answers well inside this; only a link
    /// that returns *nothing* runs it out. Injectable so tests can trip the
    /// watchdog without an 8-second real-time wait.
    private let returnPathDeadlineNanos: UInt64
    static let defaultReturnPathDeadlineNanos: UInt64 = 8 * 1_000_000_000

    // MARK: Init

    init(credentials: MoshCredentials) throws {
        // This replaces `UInt16(credentials.udpPort)` on an `Int` port, which is
        // where an out-of-range value from the remote's connect line trapped.
        // `udpPort` is a `UInt16` now, so the narrowing is gone entirely and
        // this resolution cannot actually fail — `NWEndpoint.Port(rawValue:)` is
        // total over `UInt16`, zero included. It is written as a guard rather
        // than a `!` so the channel below can take an already-resolved port and
        // the file carries no force unwrap to re-examine later.
        guard let port = NWEndpoint.Port(rawValue: credentials.udpPort) else {
            throw MoshBootstrap.BootstrapError.unusablePort(Int(credentials.udpPort))
        }
        try self.init(
            credentials: credentials,
            channel: NWConnectionChannel(host: credentials.host, port: port),
            makeChannel: { NWConnectionChannel(host: credentials.host, port: port) })
    }

    /// Designated init taking an injectable `DatagramChannel`. Production goes
    /// through `init(credentials:)` above (which builds an `NWConnectionChannel`);
    /// tests inject a fake channel so they can drive `handleDatagram`'s state
    /// machine with synthetic datagrams and no live UDP socket.
    init(credentials: MoshCredentials,
         channel: DatagramChannel,
         makeChannel: (() -> DatagramChannel)? = nil,
         returnPathDeadlineNanos: UInt64 = MoshTransport.defaultReturnPathDeadlineNanos,
         keystrokeTTLMs: UInt64 = MoshTransport.defaultKeystrokeTTLMs) throws {
        self.credentials = credentials
        self.crypto = try MoshCrypto(key: credentials.key)
        self.channel = channel
        self.makeChannel = makeChannel
        self.returnPathDeadlineNanos = returnPathDeadlineNanos
        self.keystrokeTTLMs = keystrokeTTLMs

        var cont: AsyncStream<Data>.Continuation!
        self.hostStream = AsyncStream<Data>(bufferingPolicy: .unbounded) { cont = $0 }
        self.hostContinuation = cont
    }

    // MARK: Lifecycle

    func start(cols: Int, rows: Int) {
        guard !started else { return }
        started = true
        self.cols = cols
        self.rows = rows
        // Announce the window size as the first user event. mosh-server
        // boots its terminal at 80×24 and only learns the real size from an
        // explicit Resize instruction — storing the values locally is not
        // negotiation, and resize()'s de-dupe guard would silently swallow
        // any later call with the same size.
        appendUserState([.resize(width: cols, height: rows)])

        wireChannel()
    }

    /// Wire handlers into the CURRENT channel and start it — shared by
    /// start() and rebuildFlow(), which swaps the channel out underneath.
    private func wireChannel() {
        channel.onStateChange = { [weak self] state in
            guard let self else { return }
            Task { await self.handleState(state) }
        }
        // A better path appearing (Wi-Fi ↔ cellular) is our roam signal.
        channel.onBetterPath = { [weak self] better in
            guard let self else { return }
            Task { await self.setRoaming(better) }
        }
        channel.start()
        receiveNext()
    }

    /// Throw the UDP flow away and dial a fresh one — same crypto, same
    /// session, new socket. The one lesson every mosh bug this week taught:
    /// after an iOS suspension the NWConnection can sit in a ZOMBIE .ready —
    /// state says fine, flow is gone, every send blackholes, and restart()
    /// (legal only from .failed/.waiting) does nothing. mosh doesn't care
    /// about socket identity — the first flush announces the new source and
    /// the server re-homes, exactly like a roam — so on any real doubt the
    /// honest move is to stop diagnosing the socket and replace it.
    private func rebuildFlow() {
        guard !closed else { return }
        flowRebuilds &+= 1
        channel.onStateChange = nil
        channel.onBetterPath = nil
        channel.cancel()
        if let makeChannel { channel = makeChannel() }
        wireChannel()
        // .ready → handleState → flush() announces us from the new socket.
    }

    func close() {
        guard !closed else { return }
        closed = true
        heartbeat?.cancel()
        heartbeat = nil
        returnPathWatchdog?.cancel()
        returnPathWatchdog = nil
        livenessMonitor?.cancel()
        livenessMonitor = nil
        channel.cancel()
        hostContinuation.finish()
    }

    /// Throw away everything we believe about the screen and ask the server
    /// to paint it from scratch: reset the applied display state to 0 (the
    /// empty baseline every SSP session starts from), so the next ack makes
    /// the server's next diff a COMPLETE repaint rather than an increment.
    ///
    /// This is the self-heal for the "white blocks" class of report: this
    /// client renders host bytes straight into SwiftTerm with no framebuffer
    /// model of its own, so if SwiftTerm's idea of a cell ever diverges from
    /// mosh-server's (the prime suspect is a wide-glyph width disagreement —
    /// the blocks cluster at the ends of CJK-wrapped lines), the divergence
    /// PERSISTS: the server never resends cells it believes are already
    /// right, and no amount of new output repaints them. Stock mosh cannot
    /// have the bug (it renders from its own synced framebuffer). The
    /// protocol hands us the remedy for free — acking state 0 is always
    /// valid, and the server answers by diffing from blank: clear + full
    /// redraw, divergence erased. Loopback never reproduces the report
    /// because a loss-free link ships complete frames that mask divergence;
    /// only a lossy link's partial diffs leave it standing.
    func requestFullRedraw() {
        guard !closed else { return }
        appliedHostNum = 0
        receiveAssembler = FragmentAssembler()
        flush()
    }

    /// Called when the app returns to the foreground. mosh's SSP is
    /// connectionless, so a suspended session usually heals by itself — but
    /// the NWConnection may have entered `.failed`/`.waiting` while iOS had
    /// the sockets. Restart it if so, and nudge a packet out immediately so
    /// the server re-homes to our (possibly new) address.
    func resume(force: Bool = false) {
        guard !closed else { return }
        if force {
            // Returning from a REAL suspension (hub saw >20s of background):
            // don't diagnose the socket — replace it. See rebuildFlow().
            // And repaint from scratch: a long suspension is also peak
            // render-divergence risk, and the full redraw costs one screen.
            rebuildFlow()
            requestFullRedraw()
            return
        }
        switch channel.state {
        case .failed, .waiting:
            channel.restart()
        default:
            break
        }
        flush()
    }

    // MARK: User input

    /// Queue raw keystroke bytes as a new user state and flush immediately.
    func send(_ data: Data) {
        guard !data.isEmpty else { return }
        appendUserState([.keystroke(keys: data)])
        flush()
    }


    func resize(cols: Int, rows: Int) {
        guard cols != self.cols || rows != self.rows else { return }
        self.cols = cols
        self.rows = rows
        appendUserState([.resize(width: cols, height: rows)])
        flush()
    }

    private func appendUserState(_ events: [UserEvent]) {
        let num = (userStates.last?.num ?? 0) + 1
        userStates.append((num, events, Self.nowMs()))
    }

    /// Strip keystrokes past their delivery TTL from every still-unacked
    /// state (see `keystrokeTTLMs`). Runs on every flush, so both recovery
    /// paths are covered in one place: a foreground stall that heals by
    /// itself (the heartbeat's next flush is what would have replayed the
    /// queue) and a suspend → `resume()` cycle. The state *numbers* must
    /// survive — the server tracks the input stream by num — so expired
    /// states drain as empty diffs the server acks, rather than being
    /// removed and rewriting history.
    private func pruneExpiredKeystrokes() {
        let now = Self.nowMs()
        userStates = userStates.map { state in
            guard state.num > serverAckedUserNum,
                  now &- state.queuedAtMs > keystrokeTTLMs,
                  state.events.contains(where: { if case .keystroke = $0 { return true } else { return false } })
            else { return state }
            let kept = state.events.filter { if case .keystroke = $0 { return false } else { return true } }
            return (state.num, kept, state.queuedAtMs)
        }
    }

    // MARK: Connection state

    private func handleState(_ state: NWConnection.State) {
        switch state {
        case .ready:
            // First packet announces our address so the server can push the
            // initial screen; then a steady heartbeat keeps NAT + acks alive.
            flush()
            startHeartbeat()
            startReturnPathWatchdog()
        case .cancelled:
            // Deliberate close() — the only way this stream should ever end.
            hostContinuation.finish()
        case .failed:
            // A UDP "connection" failing is a fiction worth ignoring — mosh's
            // whole contract is that the session survives network death.
            // Finishing the host stream here made any transient failure
            // PERMANENT: the render pump ended, the screen froze, and the only
            // visible way back was a manual full reconnect with the connecting
            // cover mosh exists to avoid ("mosh doesn't feel fast" — user
            // report, 2026-08-17). Restart the flow instead; the heartbeat and
            // foreground resume() re-arm delivery, and the server re-sends
            // whatever the client hasn't acked. Delayed a beat so a persistent
            // failure (airplane mode) can't spin restart→failed→restart hot.
            guard !closed else { hostContinuation.finish(); return }
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(1))
                await self?.restartAfterFailure()
            }
        default:
            break
        }
    }

    /// The delayed half of the `.failed` handling above — isolated so the
    /// closed-flag read and the restart happen on the actor.
    private func restartAfterFailure() {
        guard !closed else { return }
        channel.restart()
    }

    private func setRoaming(_ value: Bool) {
        guard roaming != value else { return }
        roaming = value
        publishMetrics()
        if value {
            // Nudge a packet out the new path so the server re-homes to it.
            flush()
        }
    }

    private func startHeartbeat() {
        heartbeat?.cancel()
        heartbeat = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                await self?.flush()
            }
        }
    }

    /// Arm the return-path watchdog once we're `.ready` and flushing. If no
    /// datagram has arrived by the deadline, the return path is dead — signal
    /// it once. Skipped if we've already heard from the server (a restart on
    /// resume of a session that was previously receiving fine).
    private func startReturnPathWatchdog() {
        guard !sawAnyDatagram, returnPathWatchdog == nil else { return }
        returnPathWatchdog = Task { [weak self, deadline = returnPathDeadlineNanos] in
            try? await Task.sleep(nanoseconds: deadline)
            await self?.checkReturnPath()
        }
    }

    private func checkReturnPath() {
        guard !closed, !sawAnyDatagram else { return }
        Task { @MainActor in onReturnPathDead?() }
    }

    // MARK: Send path

    private func flush() {
        guard !closed else { return }
        pruneExpiredKeystrokes()
        let old = serverAckedUserNum
        let new = userStates.last?.num ?? 0
        let events = userStates.filter { $0.num > old }.flatMap(\.events)

        var instruction = TransportInstruction()
        instruction.oldNum = old
        instruction.newNum = new
        instruction.ackNum = appliedHostNum
        instruction.diff = events.isEmpty ? Data() : UserMessage(events: events).encoded()

        guard let compressed = try? MoshCompression.compress(instruction.encoded()) else { return }
        let fragments = TransportFragment.fragment(payload: compressed, id: nextInstructionId, mtu: 1300)
        nextInstructionId += 1

        for fragment in fragments {
            let message = MoshCrypto.Message(
                timestamp: Self.timestamp16(),
                timestampReply: outgoingTimestampReply(),
                payload: fragment.encoded())
            guard let packet = try? crypto.seal(message, seq: sendSeq, direction: .toServer) else { continue }
            sendSeq += 1
            channel.send(packet)
        }
        lastSendMs = Self.nowMs()
        startLivenessMonitor()
        // Send-side vitals refresh on every flush — deliberately not only on
        // receive, because "typing does nothing" is exactly the state where
        // nothing is being received, and the popover must stay honest there.
        publishDiagnostics()
    }

    /// Plan B: a repeating check that catches the flow dying MID-SESSION —
    /// the "typing stopped working after a while" report. The signature is
    /// asymmetric silence: we've sent within the window, heard nothing for
    /// the whole deadline, yet the channel still claims .ready (a zombie
    /// left by suspension or a path change NWConnection never surfaced).
    /// Response: rebuild the flow (cheap, roam-equivalent). Plan C: after 3
    /// consecutive rebuilds with still nothing inbound, surface the dead-
    /// return-path banner — but keep trying, because this also self-heals
    /// the moment the network comes back.
    private func startLivenessMonitor() {
        guard livenessMonitor == nil, !closed else { return }
        livenessMonitor = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                await self?.checkLiveness()
            }
        }
    }

    private func checkLiveness() {
        guard !closed, sawAnyDatagram else { return }
        let now = Self.nowMs()
        guard lastSendMs > 0,
              now &- lastInboundMs > Self.livenessDeadlineMs,
              lastSendMs > lastInboundMs else { return }
        rebuildsSinceInbound += 1
        if rebuildsSinceInbound == 3 {
            Task { @MainActor in onReturnPathDead?() }
        }
        rebuildFlow()
        // Give the fresh flow something to announce itself with.
        flush()
    }

    // MARK: Receive path

    private func receiveNext() {
        channel.receive { [weak self] data, error in
            guard let self else { return }
            // One actor-owned task processes this datagram fully *before*
            // re-arming the next receive. Splitting these into two independent
            // `Task`s let datagram N+1 begin handling before N finished — the
            // actor serializes state, not task-submission order — which breaks
            // `handleDatagram`'s order-sensitive apply logic (it only advances
            // `appliedHostNum` when `oldNum == appliedHostNum`). Re-arming only
            // after the current datagram is done also restores natural
            // backpressure: we don't ask the socket for more until we've
            // handled what we have.
            Task {
                if let data, !data.isEmpty {
                    await self.handleDatagram(data)
                }
                if error == nil {
                    await self.receiveNext()
                }
            }
        }
    }

    func handleDatagram(_ data: Data) {
        datagramsReceived += 1
        // The return path is alive the moment ANY datagram lands — even one we
        // can't decrypt/parse — so retire the watchdog on first sight, before
        // the direction/parse guards below can `return` early.
        lastInboundMs = Self.nowMs()
        rebuildsSinceInbound = 0
        if !sawAnyDatagram {
            sawAnyDatagram = true
            returnPathWatchdog?.cancel()
            returnPathWatchdog = nil
        }
        guard let message = try? crypto.open(data), message.direction == .toClient else { return }

        // Roaming resolves the moment an authenticated reply lands.
        if roaming { roaming = false }
        updateSRTT(reply: message.timestampReply)
        savedRemoteTimestamp = message.timestamp
        savedRemoteTimestampAt = Self.nowMs()

        guard let fragment = try? TransportFragment(parsing: message.payload),
              let payload = receiveAssembler.receive(fragment),
              let instructionBytes = try? MoshCompression.decompress(payload),
              let instruction = try? TransportInstruction(parsing: instructionBytes)
        else { publishMetrics(); return }

        // Server tells us which user states it has applied → drop ours.
        if instruction.ackNum > serverAckedUserNum {
            serverAckedUserNum = instruction.ackNum
            userStates.removeAll { $0.num < serverAckedUserNum && $0.num != 0 }
        }

        // Apply a display diff only when it extends exactly the state we hold.
        if instruction.newNum > appliedHostNum, instruction.oldNum == appliedHostNum {
            if instruction.diff.isEmpty {
                // Legitimately empty diff (e.g. a pure ack/heartbeat) —
                // nothing to apply, safe to advance.
                appliedHostNum = instruction.newNum
                flush()
            } else if let host = try? HostMessage(parsing: instruction.diff) {
                if !host.hostBytes.isEmpty {
                    hostContinuation.yield(host.hostBytes)
                }
                appliedHostNum = instruction.newNum
                // Ack the freshly applied state right away.
                flush()
            } else {
                // Parse failure: do NOT advance/ack. This diff's content
                // (the login banner, a command's output, anything) would
                // otherwise be silently and PERMANENTLY lost — mosh's
                // server-side diffing treats an ack as ground truth and
                // never resends content once acked, so falsely claiming
                // "applied" here left the terminal blank (just a live
                // cursor from later, unrelated cursor-move diffs still
                // landing fine against an otherwise-untouched buffer) with
                // no way to recover for the rest of the session. Flushing
                // without updating `appliedHostNum` acks our TRUE (still
                // old) state instead, so the server keeps this content in
                // its next diff rather than considering it delivered.
                parseFailures += 1
                flush()
            }
        } else if instruction.newNum > appliedHostNum {
            // Gap: ack what we have so the server re-diffs from our real state.
            gapEvents += 1
            flush()
        }
        publishMetrics()
        publishDiagnostics()
    }

    // MARK: Timestamps & metrics

    private func updateSRTT(reply: UInt16) {
        guard reply != 0 else { return }
        let now = Self.timestamp16()
        let rtt = Double((now &- reply) & 0xFFFF)   // ms, wraps at 65536
        guard rtt < 5000 else { return }            // ignore implausible echoes
        srtt = srtt == 0 ? rtt : (0.875 * srtt + 0.125 * rtt)
    }

    private func outgoingTimestampReply() -> UInt16 {
        guard savedRemoteTimestampAt != 0 else { return 0 }
        let elapsed = Self.nowMs() &- savedRemoteTimestampAt
        return savedRemoteTimestamp &+ UInt16(truncatingIfNeeded: elapsed)
    }

    private func publishMetrics() {
        let s = srtt, r = roaming
        Task { @MainActor in onMetrics?(s, r) }
    }

    private func publishDiagnostics() {
        let d = MoshDiagnostics(
            datagramsReceived: datagramsReceived,
            parseFailures: parseFailures,
            gapEvents: gapEvents,
            appliedHostNum: appliedHostNum,
            packetsSent: sendSeq,
            lastUserNum: userStates.last?.num ?? 0,
            serverAckedUserNum: serverAckedUserNum,
            channelState: String(describing: channel.state))
        Task { @MainActor in onDiagnostics?(d) }
    }

    // MARK: Test observation

    /// Current smoothed SRTT (ms). Exposed for tests to assert convergence
    /// without racing the async `onMetrics` main-actor callback.
    var currentSRTT: Double { srtt }
    /// Current roaming flag, for the same reason as `currentSRTT`.
    var currentRoaming: Bool { roaming }
    /// Current running counters + highest applied host state, for tests to
    /// assert the apply/gap/parse-failure branches deterministically.
    var currentDiagnostics: MoshDiagnostics {
        MoshDiagnostics(
            datagramsReceived: datagramsReceived,
            parseFailures: parseFailures,
            gapEvents: gapEvents,
            appliedHostNum: appliedHostNum)
    }

    /// Monotonic milliseconds.
    private static func nowMs() -> UInt64 {
        DispatchTime.now().uptimeNanoseconds / 1_000_000
    }

    /// mosh's 16-bit millisecond timestamp.
    private static func timestamp16() -> UInt16 {
        UInt16(truncatingIfNeeded: nowMs())
    }
}

// MARK: - Datagram channel seam

/// The UDP datagram surface `MoshTransport` needs from its socket, factored
/// behind a protocol so tests can inject a fake channel and feed synthetic
/// datagrams to the receive path (and observe acks on the send path) without
/// opening a live UDP connection. Production uses `NWConnectionChannel`.
protocol DatagramChannel: AnyObject {
    /// Current connection state — drives `resume()`'s restart decision.
    var state: NWConnection.State { get }
    /// Invoked whenever the connection state changes (set before `start()`).
    var onStateChange: (@Sendable (NWConnection.State) -> Void)? { get set }
    /// Invoked when a better network path appears (the roam signal).
    var onBetterPath: (@Sendable (Bool) -> Void)? { get set }
    /// Begin connecting / start delivering state + path callbacks.
    func start()
    /// Restart the underlying connection (post-suspension recovery).
    func restart()
    /// Tear down the connection.
    func cancel()
    /// Send one datagram (fire-and-forget, mirroring UDP semantics).
    func send(_ datagram: Data)
    /// Arm a single-datagram receive; the completion fires once with the next
    /// datagram (or an error). The caller re-arms by calling `receive` again.
    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void)
}

/// Production `DatagramChannel` backed by an `NWConnection` UDP socket.
final class NWConnectionChannel: DatagramChannel, @unchecked Sendable {
    private let connection: NWConnection
    private let queue = DispatchQueue(label: "moshpit.mosh.udp")

    var onStateChange: (@Sendable (NWConnection.State) -> Void)?
    var onBetterPath: (@Sendable (Bool) -> Void)?

    /// Takes an already-resolved `NWEndpoint.Port` rather than the `UInt16` it
    /// used to force-unwrap. That `!` was in fact safe —
    /// `NWEndpoint.Port(rawValue:)` is total over `UInt16` — but the caller has
    /// to resolve the port anyway now that the range check moved to parse time,
    /// so passing the resolved value leaves nothing here to audit.
    init(host: String, port: NWEndpoint.Port) {
        let params = NWParameters.udp
        params.serviceClass = .interactiveVoice          // low-latency hint
        self.connection = NWConnection(
            host: NWEndpoint.Host(host),
            port: port,
            using: params)
    }

    var state: NWConnection.State { connection.state }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in self?.onStateChange?(state) }
        connection.betterPathUpdateHandler = { [weak self] better in self?.onBetterPath?(better) }
        connection.start(queue: queue)
    }

    func restart() { connection.restart() }

    func cancel() { connection.cancel() }

    func send(_ datagram: Data) {
        connection.send(content: datagram, completion: .contentProcessed { _ in })
    }

    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {
        connection.receiveMessage { data, _, _, error in completion(data, error) }
    }
}
