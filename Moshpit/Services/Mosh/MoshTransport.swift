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
    /// are dying on the way back). Idle, both climb by one every 3 s
    /// (`MoshTransport.appendIdleState`).
    var lastUserNum: UInt64 = 0
    var serverAckedUserNum: UInt64 = 0
    /// NWConnection state, verbatim. Anything but "ready" while the pill
    /// says live is the story.
    var channelState: String = "?"
    /// Expired keystrokes the server may already hold are unacked, so new
    /// input waits for the server's answer (`MoshTransport.holdExpiredInput`).
    var inputHeld = false
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
///   - Send by mosh's own sender discipline (transportsender-impl.h, see
///     `nextSendMs`): new input at most once per send interval, unacked
///     input again only after a retransmission timeout, output acked
///     within 100 ms, a datagram at least every 3 s.
///   - Echo timestamps for RTT, surfacing a smoothed SRTT + roaming flag.
///
/// Host output is delivered through `hostStream`, one ``MoshHostFrame`` per
/// applied diff, so the terminal layer feeds mosh much like SSH.
actor MoshTransport {

    // MARK: Public output

    nonisolated let hostStream: AsyncStream<MoshHostFrame>
    private let hostContinuation: AsyncStream<MoshHostFrame>.Continuation

    /// Called on the main actor with (srttMs, roaming) whenever they change.
    @MainActor var onMetrics: ((Double, Bool) -> Void)?

    /// Called on the main actor after every processed datagram with the
    /// running protocol counters below. There's no way to reproduce a
    /// black-screen report over loopback — real packet loss/reordering and a
    /// given remote's shell config don't happen on localhost — so this is a
    /// user-screenshottable diagnostic (see the terminal pill's long-press)
    /// rather than something exercised by this app's own tests.
    @MainActor var onDiagnostics: ((MoshDiagnostics) -> Void)?

    /// Called on the main actor when nothing comes back from the server, so
    /// the session layer can stop staring at a black screen and offer SSH
    /// (commonly a VPN/proxy or firewall that passes our outbound UDP but
    /// drops the server's replies; SSH/TCP to the same host works then).
    /// Twice at most: when the UDP connection reached `.ready` and we've been
    /// flushing for `returnPathDeadline` seconds with **zero** datagrams back
    /// (`heardFromServer` false), and when a session that HAD heard from its
    /// server goes silent through three flow rebuilds (`heardFromServer`
    /// true, see `checkLiveness`).
    ///
    /// The two are different news, and only the first means the server is
    /// waiting for a client that will never come: one we have talked to holds
    /// the user's shell, and the path that went quiet may well come back
    /// (crates/session's `ReturnPathDead { first_contact }`). The session
    /// layer used to take both for the first and kill the server — about 15 s
    /// of UDP silence mid-session cost the user their shell (B165).
    @MainActor var onReturnPathDead: ((_ heardFromServer: Bool) -> Void)?

    /// Called on the main actor, once, when mosh-server ended the session:
    /// its command exited (`exit` in the remote shell) or it was told to
    /// stop, it sent its shutdown state, and a datagram acking that went
    /// out. The transport has closed itself (the host stream finishes after
    /// the last frame). There is nothing to roam back to — the session
    /// layer ends the session instead of waiting for a return path.
    @MainActor var onSessionEnded: (() -> Void)?

    // MARK: Connection

    private var channel: DatagramChannel
    /// Builds a REPLACEMENT datagram channel — flow rebuild's raw material.
    /// nil (tests without a factory) means rebuildFlow() re-wires the same
    /// injected fake, which still exercises the sequencing.
    private let makeChannel: (() -> DatagramChannel)?
    private var started = false
    private var closed = false
    /// Which flow the armed receive belongs to: bumped when `rebuildFlow`
    /// replaces the channel, so the old flow's last receive completing (a
    /// cancelled NWConnection completes it, with no data and no error)
    /// neither arms a second receive on the new flow nor counts as its own.
    private var flowGeneration: UInt64 = 0
    /// A receive is armed on the current flow (see `receiveNext`).
    private var receiveArmed = false

    // MARK: Crypto + protocol state

    private let crypto: MoshCrypto
    private var sendSeq: UInt64 = 0

    /// One outgoing user-input state: a keystroke batch or a resize.
    private struct UserState {
        let num: UInt64
        var events: [UserEvent]
        /// Monotonic ms it was queued at; drives the keystroke delivery TTL.
        let queuedAtMs: UInt64
        /// Went out in at least one instruction, so the server may hold it.
        var sent: Bool
        /// When an instruction last carried it (meaningful once `sent`):
        /// mosh's `TimestampedState::timestamp` for a sent state. Never
        /// decreases along the list: an instruction carries every state
        /// after the one it starts from.
        var sentAtMs: UInt64 = 0

        var hasKeystroke: Bool { events.contains(where: \.isKeystroke) }
    }

    /// Outgoing user-input states; `[0]` is the empty baseline, kept forever.
    private var userStates = [UserState(num: 0, events: [], queuedAtMs: 0, sent: true)]
    private var serverAckedUserNum: UInt64 = 0
    /// The newest state that carries input (not an idle one, see
    /// `appendIdleState`): what ``waitUntilDelivered()`` waits for.
    private var inputNum: UInt64 = 0
    /// When a datagram last carried a user state mosh-server had not been
    /// sent before (see `appendIdleState`).
    private var lastNewStateMs: UInt64 = 0
    /// mosh's `ACK_INTERVAL`: how long after the last new user state an
    /// idle client sends another (see `appendIdleState`), and how long
    /// after its last datagram a client with nothing to say says it anyway.
    static let idleStateIntervalMs: UInt64 = 3_000
    /// mosh's `ACK_DELAY`: output from the server is acked this soon, and a
    /// state sent this long past the retransmission timeout without an ack
    /// is presumed lost.
    static let ackDelayMs: UInt64 = 100
    /// mosh's `ACTIVE_RETRY_TIMEOUT`: unacked input is retried at frame rate
    /// only this long after the server was last heard; after that, with
    /// every 3 s ack.
    static let activeRetryTimeoutMs: UInt64 = 10_000
    /// mosh's frame-rate bounds (`SEND_INTERVAL_MIN`, `SEND_INTERVAL_MAX`)
    /// and retransmission timeout bounds (`MIN_RTO`, `MAX_RTO`), in ms.
    private static let sendIntervalMinMs = 20.0
    private static let sendIntervalMaxMs = 250.0
    private static let minRTOMs = 50.0
    private static let maxRTOMs = 1_000.0
    /// mosh's estimate before the first RTT sample (network.cc).
    private static let initialSRTTMs = 1_000.0
    private static let initialRTTVarMs = 500.0
    /// mosh's prophylactic resend (`attempt_prospective_resend_optimization`):
    /// everything unacked goes again when that is no longer than the new
    /// part alone, or shorter than this and at most `resendSlack` longer.
    private static let resendSmall = 1_000
    private static let resendSlack = 100

    /// The send timer runs: the channel has been ready (see `tick()`).
    private var ticking = false
    /// mosh's `next_ack_time`: a datagram goes out then even with nothing
    /// new to say (the 3 s ack, output acked within `ackDelayMs`).
    private var nextAckMs: UInt64?
    /// When the server last gave us a new state (mosh's sender
    /// `last_heard`), or the first datagram went out.
    private var lastHeardMs: UInt64?
    /// The send timer: one sleeping task, re-armed whenever the send
    /// deadline moves (see `scheduleWake`).
    private var wakeTask: Task<Void, Never>?
    private var wakeAtMs: UInt64?
    private var wakeGeneration: UInt64 = 0
    /// Callers parked in ``waitUntilDelivered()``, each with the newest user
    /// state that existed when it asked.
    private var deliveryWaiters: [(num: UInt64, continuation: CheckedContinuation<Void, Never>)] = []

    /// How long an un-acked keystroke keeps retransmitting before it is
    /// treated as lost. SSP's default — retransmit forever — is right for a
    /// blip (keys typed through a roam land once the path heals) but wrong
    /// across a long-dead link: keystrokes typed into a frozen screen
    /// suddenly replay into the shell whenever connectivity returns, minutes
    /// later ("the terminal typed my old input by itself") — and a command
    /// the user already retyped after reconnecting runs twice. Resize events
    /// are exempt: they're idempotent state, not actions. Injectable so
    /// tests can expire input without a 10-second wait. How an expired
    /// keystroke the server may already hold is dropped: `holdExpiredInput`.
    private let keystrokeTTLMs: UInt64
    static let defaultKeystrokeTTLMs: UInt64 = 10_000

    /// When input started being held back (see `holdExpiredInput`).
    private var heldSinceMs: UInt64?
    /// How long into a hold a datagram from the server, still not acking the
    /// held keystrokes, proves they never reached it: mosh's bound on a
    /// plausible round trip (its RTT samples stop at 5 s). Injectable for
    /// tests.
    private let expiryProofDelayMs: UInt64
    static let defaultExpiryProofDelayMs: UInt64 = 5_000

    /// Monotonic milliseconds. Injectable so tests can step time.
    private let clock: @Sendable () -> UInt64

    /// Most bytes of encoded user events per instruction (see
    /// ``send(_:)``). Injectable so tests can split small input.
    private let instructionBudget: Int
    static let defaultInstructionBudget = 64 * 1024

    /// Highest server display state we've applied (and therefore ack).
    private var appliedHostNum: UInt64 = 0
    /// mosh's number for the shutdown state (`uint64_t(-1)`): mosh-server
    /// sends it once its command exited or it was told to stop, and quits
    /// once it is acked.
    static let shutdownStateNum = UInt64.max
    /// mosh-server ended the session (see `onSessionEnded`).
    private(set) var sessionEnded = false
    /// The server's echo ack as of `appliedHostNum` — see ``MoshHostFrame``.
    private var echoAck: UInt64 = 0
    private var receiveAssembler = FragmentAssembler()
    private var nextInstructionId: UInt64 = 1

    // MARK: Diagnostics (see `onDiagnostics`)

    private var datagramsReceived: UInt64 = 0
    private var parseFailures: UInt64 = 0
    private var gapEvents: UInt64 = 0

    private var cols = 80
    private var rows = 24

    /// A full redraw in progress (see ``requestFullRedraw()``).
    private enum Redraw: Equatable {
        /// The narrower size is on its way; waiting for a frame at it.
        case narrowing(cols: Int, rows: Int)
        /// The real size is on its way; frames are hidden until its repaint.
        case restoring(sinceMs: UInt64)
    }
    private var redraw: Redraw?
    /// How long a redraw may hide frames while waiting for its repaint.
    static let redrawHideLimitMs: UInt64 = 5_000

    // MARK: Timestamps / SRTT

    /// mosh's "no timestamp", in both timestamp fields (network.cc).
    static let noTimestamp: UInt16 = 0xFFFF
    /// The server's last timestamp and when it arrived (local ms), until
    /// it is echoed.
    private var savedRemoteTimestamp: (timestamp: UInt16, atMs: UInt64)?
    /// One past the newest sequence number received: an older datagram
    /// (reordered or replayed) feeds neither the RTT nor the echo.
    private var expectedReceiverSeq: UInt64 = 0
    private var srtt: Double = 0
    /// mosh's `RTTVAR`, for the retransmission timeout.
    private var rttvar: Double = MoshTransport.initialRTTVarMs
    /// Whether `srtt` holds a sample yet (0 ms is a real one).
    private var rttHit = false
    private var roaming = false

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
    /// Whether anything was sent yet (`lastSendMs` can be a real 0 under a
    /// test clock).
    private var sentAnything = false
    /// Flow rebuilds since the last inbound datagram — Plan C's dryness
    /// counter. 0 the moment anything arrives.
    private var rebuildsSinceInbound = 0
    private var flowRebuilds: UInt64 = 0
    private var livenessMonitor: Task<Void, Never>?
    /// How long "sent but heard nothing" may last before the flow is
    /// declared a zombie and rebuilt. Both sides send at least every 3 s,
    /// so a healthy link never goes this long silent in both directions.
    static let livenessDeadlineMs: UInt64 = 9_000
    /// How often the liveness check runs (`startLivenessMonitor`). Injectable
    /// so tests can run three checks without a 9-second real-time wait.
    private let livenessInterval: Duration
    static let defaultLivenessInterval: Duration = .seconds(3)
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
         keystrokeTTLMs: UInt64 = MoshTransport.defaultKeystrokeTTLMs,
         expiryProofDelayMs: UInt64 = MoshTransport.defaultExpiryProofDelayMs,
         instructionBudget: Int = MoshTransport.defaultInstructionBudget,
         livenessInterval: Duration = MoshTransport.defaultLivenessInterval,
         clock: @escaping @Sendable () -> UInt64 = MoshTransport.monotonicMs) throws {
        self.crypto = try MoshCrypto(key: credentials.key)
        self.channel = channel
        self.makeChannel = makeChannel
        self.returnPathDeadlineNanos = returnPathDeadlineNanos
        self.livenessInterval = livenessInterval
        self.keystrokeTTLMs = keystrokeTTLMs
        self.expiryProofDelayMs = expiryProofDelayMs
        self.instructionBudget = instructionBudget
        self.clock = clock

        var cont: AsyncStream<MoshHostFrame>.Continuation!
        self.hostStream = AsyncStream<MoshHostFrame>(bufferingPolicy: .unbounded) { cont = $0 }
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
        flowGeneration &+= 1
        receiveArmed = false
        wireChannel()
        // .ready → handleState → nudge() announces us from the new socket.
    }

    func close() { shutDown(lettingSendsFinish: false) }

    /// Stop everything. `lettingSendsFinish`: the datagrams just handed to
    /// the channel still go out before it is torn down — the last one is the
    /// ack mosh-server quits on (see `endSession`), and cancelling an
    /// NWConnection drops the sends it has not processed yet.
    private func shutDown(lettingSendsFinish: Bool) {
        guard !closed else { return }
        closed = true
        ticking = false
        nextAckMs = nil
        wakeTask?.cancel()
        wakeTask = nil
        wakeAtMs = nil
        returnPathWatchdog?.cancel()
        returnPathWatchdog = nil
        livenessMonitor?.cancel()
        livenessMonitor = nil
        if lettingSendsFinish { channel.cancelAfterSends() } else { channel.cancel() }
        hostContinuation.finish()
        resumeDeliveryWaiters()
    }

    /// Throw away everything we believe about the screen and ask the server
    /// to paint it from scratch: its next frame is a COMPLETE repaint
    /// rather than an increment.
    ///
    /// This is the self-heal for the "white blocks" class of report: this
    /// client renders host bytes straight into SwiftTerm with no framebuffer
    /// model of its own, so if SwiftTerm's idea of a cell ever diverges from
    /// mosh-server's (root-caused: an orphaned wide-char continuation cell —
    /// see MoshSwitchReplayTests), the divergence PERSISTS: the server never
    /// resends cells it believes are already right.
    ///
    /// The remedy is a WIDTH NUDGE, not an ack rewind. The first shipped
    /// version acked display state 0, betting the server would re-diff from
    /// blank — but a stock mosh-server CULLS sent states once acked past and
    /// silently ignores an ack naming a culled one (verified against 1.4.0:
    /// zero bytes come back), and rewinding `appliedHostNum` is actively
    /// harmful — every subsequent in-order diff then lands in the gap branch
    /// and the screen freezes until reconnect. What DOES work: mosh's
    /// `Display::new_frame` resets its `initialized` flag on ANY size change
    /// and emits a complete repaint.
    ///
    /// The size must change between two frames the server RENDERS, though.
    /// The second shipped version sent one column narrower and back as two
    /// resizes in one user state: mosh-server applies both before it
    /// renders, sees the size it already had, and sends nothing (live,
    /// 1.4.0: zero bytes) — the repaint button and the forced resume after a
    /// long suspension did nothing. So the narrower width goes alone; the
    /// frame the server renders at it is applied and acked but not shown
    /// (it is drawn for the wrong width); only then is the real width
    /// restored, and the server's next frame is a complete repaint. Frames
    /// in between are hidden too, for at most `redrawHideLimitMs`. A real
    /// resize cancels the redraw (it repaints anyway). The applied state is
    /// never rewound.
    func requestFullRedraw() {
        guard !closed else { return }
        let width = cols > 2 ? cols - 1 : cols + 1
        redraw = .narrowing(cols: width, rows: rows)
        appendUserState([.resize(width: width, height: rows)])
        tick()
    }

    /// Whether an applied frame is shown, advancing a redraw in progress
    /// (see ``requestFullRedraw()``). `resize` is the frame's own resize, if
    /// it carries one: the server's framebuffer changed size with it.
    private func redrawShows(resize: (width: Int, height: Int)?, now: UInt64) -> Bool {
        switch (redraw, resize) {
        case let (.narrowing(narrowCols, narrowRows)?, size?)
            where size.width == narrowCols && size.height == narrowRows:
            // The frame at the narrow size: restore the real one.
            redraw = .restoring(sinceMs: now)
            appendUserState([.resize(width: cols, height: rows)])
            return false
        case let (.restoring(since)?, nil)
            where Self.elapsed(since: since, now: now) <= Self.redrawHideLimitMs:
            return false
        case (.restoring?, _):
            // The repaint at the real width (or a size we did not ask for,
            // or a server that never sent one): show from here on.
            redraw = nil
            return true
        default:
            return true
        }
    }

    /// Called when the app returns to the foreground. mosh's SSP is
    /// connectionless, so a suspended session usually heals by itself — but
    /// the NWConnection may have entered `.failed`/`.waiting` while iOS had
    /// the sockets. Restart it if so, and nudge a packet out immediately
    /// (whatever is due, an ack at least) so the server re-homes to our
    /// (possibly new) address.
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
        case .failed:
            // A failed NWConnection never comes back — `restart()` retries
            // only one that is waiting and is ignored otherwise
            // (nw_connection_restart) — so the flow is replaced.
            rebuildFlow()
        case .waiting:
            channel.restart()
        default:
            break
        }
        nudge()
    }

    // MARK: User input

    /// Queue raw keystroke bytes as new user states, sent when their frame
    /// is due (at once unless a datagram went out within the send interval,
    /// see `nextSendMs`). Returns the last state's number — what the
    /// server's echo ack will name once it has had time to echo these bytes
    /// (predictive echo keys its guesses on it). nil when there was nothing
    /// to send.
    ///
    /// mosh-server inflates an instruction into a 4 MiB buffer
    /// (compressor.cc) and drops one that does not fit as a "Crypto
    /// exception", never acked. A flush sent the whole unacked stream as one
    /// instruction, so a paste over 4 MiB was refused by every flush until
    /// its keystrokes expired, then silently lost (live, mosh-server 1.4.0:
    /// 4,194,500 bytes, 13 refusals in 14 s, nothing arrived), and one just
    /// under it made mosh-server peak at 700 MiB. Input is split into states
    /// whose event fits `instructionBudget`, and `sendDue` sends them in
    /// instructions under it, chained.
    @discardableResult
    func send(_ data: Data) -> UInt64? {
        guard !data.isEmpty else { return nil }
        let chunk = Self.keystrokeChunk(instructionBudget)
        var start = data.startIndex
        while start < data.endIndex {
            let end = data.index(start, offsetBy: chunk, limitedBy: data.endIndex) ?? data.endIndex
            appendUserState([.keystroke(keys: Data(data[start..<end]))])
            start = end
        }
        let num = inputNum
        tick()
        return num
    }

    /// Keystroke bytes per user state: their event encodes in at most
    /// `budget` bytes (three nested length-delimited fields, at most 18
    /// bytes of tags and lengths).
    static func keystrokeChunk(_ budget: Int) -> Int { max(1, budget - 32) }

    func resize(cols: Int, rows: Int) {
        guard cols != self.cols || rows != self.rows else { return }
        // A real size change repaints everything anyway.
        redraw = nil
        self.cols = cols
        self.rows = rows
        appendUserState([.resize(width: cols, height: rows)])
        tick()
    }

    /// Suspend until the server has acknowledged every user state queued so
    /// far. mosh-server acks a state only after writing its keystrokes into
    /// the remote PTY, so this is "what was sent before now has LANDED" — a
    /// stronger promise than ``send(_:)`` returning, which only means the
    /// datagram left.
    ///
    /// For input that must follow these bytes over a DIFFERENT channel
    /// (herdr's semantic arrow keys go as an SSH exec): a lost datagram is
    /// retransmitted a timeout later and could otherwise arrive after it.
    ///
    /// Returns at once when nothing is outstanding or the transport is
    /// closed. Unbounded otherwise — a dead return path never acks — so the
    /// caller brings its own budget.
    func waitUntilDelivered() async {
        // Input only: an idle state is acked when the server next speaks,
        // up to 3 s later when it has nothing to say.
        let newest = inputNum
        guard newest > serverAckedUserNum, !closed else { return }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            deliveryWaiters.append((newest, continuation))
        }
    }

    /// Wake every ``waitUntilDelivered()`` caller whose state the server has
    /// now acknowledged — or all of them, once closed.
    private func resumeDeliveryWaiters() {
        guard !deliveryWaiters.isEmpty else { return }
        let acked = serverAckedUserNum
        let ready = deliveryWaiters.filter { closed || $0.num <= acked }
        deliveryWaiters.removeAll { closed || $0.num <= acked }
        for waiter in ready { waiter.continuation.resume() }
    }

    private func appendUserState(_ events: [UserEvent]) {
        let num = (userStates.last?.num ?? 0) + 1
        userStates.append(UserState(num: num, events: events, queuedAtMs: clock(), sent: false))
        inputNum = num
    }

    /// mosh's client sends every empty ack as a new user state
    /// (transportsender-impl.h, `send_empty_ack`: `new_num = back + 1`, at
    /// least every `ACK_INTERVAL`, 3 s), and mosh-server dates its client by
    /// the newest user state it received (mosh-server.cc,
    /// `time_since_remote_state`): MOSH_SERVER_NETWORK_TMOUT (or SIGUSR1 with
    /// MOSH_SERVER_SIGNAL_TMOUT) reaps a session whose client has sent none
    /// for that long, and a frame lost on the way back is retried at frame
    /// rate only within 10 s of one (transportsender-impl.h,
    /// `ACTIVE_RETRY_TIMEOUT`), afterwards at the server's 3 s ack tick.
    /// Heartbeats and acks here only repeated the acked state, so an idle
    /// but connected session was reaped (live: at 8 s with TMOUT=8) and a
    /// lost frame took 3 s to come back.
    ///
    /// So a datagram that goes out 3 s after the last new state went out
    /// adds an empty one: same events, a new number; idle, that is the 3 s
    /// ack. Not while input is held (its diff from the acked state would
    /// carry the held keystrokes) and not before ``start(cols:rows:)``. (The
    /// acked state is always held, so the number after the newest is free:
    /// an ack is taken only for a state sent, B105.)
    private func appendIdleState(now: UInt64) {
        guard started, let newest = userStates.last, newest.sent,
              Self.elapsed(since: lastNewStateMs, now: now) >= Self.idleStateIntervalMs else { return }
        userStates.append(UserState(num: newest.num + 1, events: [], queuedAtMs: now, sent: false))
    }

    /// Keystrokes past their delivery TTL (see `keystrokeTTLMs`): drops
    /// those never sent, and says whether this flush must hold user input
    /// back. Runs on every flush, so both recovery paths are covered in one
    /// place: a foreground stall that heals by itself (the send timer's next
    /// flush is what would have replayed the queue) and a suspend →
    /// `resume()` cycle.
    ///
    /// A keystroke that already went out must NOT simply be stripped: the
    /// server may have received it and only its acks were lost (a return
    /// path dropping packets for 10 s). mosh-server diffs user states by
    /// position (`UserStream::diff_from`, whose asserts release builds
    /// compile out), so a state that no longer extends the one it processed
    /// made it drop as many new events as were stripped, or read past the
    /// end of the shorter state: SIGSEGV, and the remote shell died with it
    /// (type, lose the return path for over 10 s, type again).
    ///
    /// So once a sent keystroke expires, flushes carry no user state past
    /// the acked one (they still ack host states and keep the session
    /// alive) until the server answers — see `settleHeldInput`. No state
    /// the server may hold is ever rewritten.
    private func holdExpiredInput(now: UInt64) -> Bool {
        let acked = serverAckedUserNum
        var sentExpired = false
        for index in userStates.indices where userStates[index].num > acked {
            let state = userStates[index]
            guard Self.elapsed(since: state.queuedAtMs, now: now) > keystrokeTTLMs,
                  state.hasKeystroke else { continue }
            if state.sent {
                sentExpired = true
            } else {
                userStates[index].events.removeAll(where: \.isKeystroke)
            }
        }
        if heldSinceMs == nil, sentExpired { heldSinceMs = now }
        return heldSinceMs != nil
    }

    /// Resolves held input on a datagram from the server; true when input
    /// flows again. An ack covering every sent keystroke state means they
    /// arrived after all: nothing is dropped. A datagram arriving
    /// `expiryProofDelayMs` or more after holding began, while some are
    /// still unacked, means those never arrived (none has been sent since,
    /// and mosh treats a longer transit as implausible): only then are the
    /// expired keystrokes dropped. State numbers stay — the server tracks
    /// the input stream by num — so expired states drain as empty diffs.
    private func settleHeldInput(now: UInt64) -> Bool {
        guard let since = heldSinceMs else { return false }
        let acked = serverAckedUserNum
        let inDoubt = userStates.contains { $0.num > acked && $0.sent && $0.hasKeystroke }
        if inDoubt {
            guard Self.elapsed(since: since, now: now) >= expiryProofDelayMs else { return false }
            for index in userStates.indices where userStates[index].num > acked
                && Self.elapsed(since: userStates[index].queuedAtMs, now: now) > keystrokeTTLMs {
                userStates[index].events.removeAll(where: \.isKeystroke)
            }
        }
        heldSinceMs = nil
        return true
    }

    // MARK: Connection state

    private func handleState(_ state: NWConnection.State) {
        switch state {
        case .ready:
            guard !closed else { return }
            // Ready again (after waiting, or a restart): if an error ended
            // its last receive, the flow gets one now.
            receiveNext()
            // First packet announces our address so the server can push the
            // initial screen; then the send timer keeps NAT + acks alive.
            ticking = true
            nudge()
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
            // report, 2026-08-17). Replace the flow instead; the send timer and
            // foreground resume() re-arm delivery, and the server re-sends
            // whatever the client hasn't acked. Replace, not restart: a failed
            // NWConnection never comes back, and `restart()` is ignored outside
            // `.waiting` (nw_connection_restart), so the old restart left a
            // dead flow — its receive failed with it — until the liveness
            // check rebuilt it 9 to 12 s later. Delayed a beat so a persistent
            // failure can't spin replace→failed→replace hot.
            guard !closed else { hostContinuation.finish(); return }
            let flow = flowGeneration
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(1))
                await self?.replaceAfterFailure(flow)
            }
        default:
            break
        }
    }

    /// The delayed half of the `.failed` handling above — isolated so the
    /// closed-flag read and the replacement happen on the actor. A flow
    /// already replaced since (a forced resume, the liveness check) stays.
    private func replaceAfterFailure(_ flow: UInt64) {
        guard !closed, flow == flowGeneration else { return }
        rebuildFlow()
        nudge()
    }

    private func setRoaming(_ value: Bool) {
        guard roaming != value else { return }
        roaming = value
        publishMetrics()
        if value {
            // Nudge a packet out the new path so the server re-homes to it.
            nudge()
        }
    }

    /// Arm the send timer for the current send deadline (see `tick()`):
    /// one task sleeping until then, replaced whenever the deadline moves.
    /// Only once the channel has been ready, as the heartbeat it replaces.
    private func scheduleWake() {
        guard ticking, !closed, let at = sendDeadline() else {
            wakeTask?.cancel()
            wakeTask = nil
            wakeAtMs = nil
            return
        }
        if wakeAtMs == at, wakeTask != nil { return }
        wakeTask?.cancel()
        wakeAtMs = at
        wakeGeneration &+= 1
        let generation = wakeGeneration
        let now = clock()
        let delay = at > now ? at - now : 0
        wakeTask = Task { [weak self] in
            // A frame is 20 to 250 ms: the system's default timer leeway is
            // a sizeable part of it. On the uptime clock, as `clock()`.
            if delay > 0 {
                try? await Task.sleep(for: .milliseconds(Int64(clamping: delay)),
                                      tolerance: .milliseconds(1), clock: .suspending)
            }
            guard !Task.isCancelled else { return }
            await self?.wake(generation)
        }
    }

    private func wake(_ generation: UInt64) {
        // A task the actor replaced while it was on its way in.
        guard generation == wakeGeneration else { return }
        wakeTask = nil
        wakeAtMs = nil
        tick()
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
        Task { @MainActor in onReturnPathDead?(false) }
    }

    // MARK: Send path

    /// Send whatever mosh's sender would send now (`TransportSender::tick`):
    /// new input once its frame is due, unacked input again once its
    /// retransmission timeout ran out, the ack when its timer is up. The
    /// send timer runs it (see `scheduleWake`); input, resizes and received
    /// datagrams run it too.
    func tick() {
        guard !closed else { return }
        let now = clock()
        if let due = sendDeadline(), due <= now { sendDue(now: now) }
        scheduleWake()
    }

    /// A datagram now, whatever else is due with it, an ack at least: the
    /// channel became ready, the path changed, the app came back. mosh's
    /// client has no such moment; this is its ack timer brought forward.
    private func nudge() {
        guard !closed else { return }
        nextAckMs = clock()
        tick()
    }

    /// When the next datagram is due: the earlier of the ack timer and
    /// `nextSendMs()`.
    private func sendDeadline() -> UInt64? {
        [nextAckMs, nextSendMs()].compactMap { $0 }.min()
    }

    /// mosh's `next_send_time` (`TransportSender::calculate_timers`): new
    /// input one send interval after the last datagram (`SEND_MINDELAY` is
    /// 0 here: mosh-client's 1 ms gathers the bytes of one burst of reads,
    /// and ``send(_:)`` is handed whole inputs); input the server has not
    /// acked, when the oldest of it has gone a retransmission timeout plus
    /// `ackDelayMs` since it was last sent (then the server is no longer
    /// assumed to hold it, see `diffBase`), but only while the server was
    /// heard within `activeRetryTimeoutMs` (otherwise it goes with the 3 s
    /// ack); nothing while input is held.
    ///
    /// Every flush used to send every unacked state, and a flush ran for
    /// each datagram received and every second: a 4 MB paste made this
    /// client send 88.6 MB and the Rust port 33.4 MB (live, mosh-server
    /// 1.4.0), and keystrokes in flight went again with every frame of
    /// output. Unlike mosh, which refreshes only the newest state's time on
    /// a retransmission and so retries every send interval until the ack
    /// comes, a retransmission here restamps every state it carries, so the
    /// next waits a full timeout: one copy per timeout, not two per round
    /// trip.
    private func nextSendMs() -> UInt64? {
        guard heldSinceMs == nil else { return nil }
        let acked = serverAckedUserNum
        let paced = sentAnything ? lastSendMs + sendIntervalMs() : 0
        let unacked = userStates.lazy.filter { $0.num > acked }
        if unacked.contains(where: { !$0.sent }) { return paced }
        // mosh compares contents: when no unacked state carries an event,
        // the newest is the acked one, and an ack is all it takes.
        guard unacked.contains(where: { !$0.events.isEmpty }), let oldest = unacked.first,
              let heard = lastHeardMs else { return nil }
        let at = max(oldest.sentAtMs + retransmitAfterMs(), paced)
        return heard + Self.activeRetryTimeoutMs > at ? at : nil
    }

    /// mosh's `assumed_receiver_state` (`update_assumed_receiver_state`),
    /// then its prophylactic resend (`attempt_prospective_resend_optimization`):
    /// the state to send from. The newest sent state is assumed to have
    /// arrived while every state after the acked one went out within a
    /// retransmission timeout plus `ackDelayMs`; otherwise the acked state
    /// is the only one the server is known to hold. Everything after the
    /// acked state goes again anyway when that is no longer than the new
    /// part, or small (a few keystrokes: under `resendSmall` bytes, at most
    /// `resendSlack` more).
    private func diffBase(now: UInt64) -> UInt64 {
        let acked = serverAckedUserNum
        let limit = retransmitAfterMs()
        var assumed = acked
        for state in userStates where state.num > acked {
            guard state.sent, Self.elapsed(since: state.sentAtMs, now: now) < limit else { break }
            assumed = state.num
        }
        guard assumed != acked else { return acked }
        func cost(from: UInt64) -> Int {
            userStates.lazy.filter { $0.num > from }
                .reduce(0) { total, state in state.events.reduce(total) { $0 + $1.encodedLength } }
        }
        let resend = cost(from: acked), incremental = cost(from: assumed)
        if resend <= incremental || (resend < Self.resendSmall && resend - incremental < Self.resendSlack) {
            return acked
        }
        return assumed
    }

    /// mosh's retransmission timeout (`Connection::timeout`): SRTT plus four
    /// RTT variances, 50 ms to 1 s; plus `ackDelayMs`, the time the server
    /// may take to ack.
    private func retransmitAfterMs() -> UInt64 {
        let (srtt, rttvar) = rttEstimate()
        return UInt64(min(max((srtt + 4 * rttvar).rounded(.up), Self.minRTOMs), Self.maxRTOMs))
            + Self.ackDelayMs
    }

    /// mosh's frame interval (`TransportSender::send_interval`): half the
    /// SRTT, 20 to 250 ms.
    private func sendIntervalMs() -> UInt64 {
        let (srtt, _) = rttEstimate()
        return UInt64(min(max((srtt / 2).rounded(.up), Self.sendIntervalMinMs), Self.sendIntervalMaxMs))
    }

    /// SRTT and RTTVAR for the timers: mosh's 1 s and 0.5 s until the first
    /// sample (the published SRTT stays 0 until then).
    private func rttEstimate() -> (Double, Double) {
        rttHit ? (srtt, rttvar) : (Self.initialSRTTMs, Self.initialRTTVarMs)
    }

    /// Send from the state the server is assumed to hold (see `diffBase`)
    /// every user state after it (none while input is held), as
    /// instructions of at most `instructionBudget` bytes of events, each
    /// from the state the one before it reached: mosh-server applies them
    /// in turn (one too large for its 4 MiB buffer it drops, see
    /// ``send(_:)``). A state larger than the budget alone (a resize under a
    /// tiny test budget) goes alone. With nothing to send, a plain ack: old
    /// and new the same state, so the server can only read its ack (it
    /// holds that state, or not the old one), never take the instruction
    /// for a state built without the events in flight.
    private func sendDue(now: UInt64) {
        let acked = serverAckedUserNum
        // Held (see holdExpiredInput): nothing past the acked state goes out.
        let held = holdExpiredInput(now: now)
        if !held { appendIdleState(now: now) }
        let from = held ? acked : diffBase(now: now)
        // Consecutive states, `instructionBudget` bytes of events at most a run.
        var runs: [Range<Int>] = []
        if !held {
            var size = 0
            for index in userStates.indices where userStates[index].num > from {
                let cost = userStates[index].events.reduce(0) { $0 + $1.encodedLength }
                if let last = runs.last, size + cost <= instructionBudget {
                    runs[runs.count - 1] = last.lowerBound..<(index + 1)
                    size += cost
                } else {
                    runs.append(index..<(index + 1))
                    size = cost
                }
            }
        }
        if runs.isEmpty {
            _ = sendInstruction(old: from, new: from, events: [], now: now)
        }
        var old = from
        for run in runs {
            let new = userStates[run.upperBound - 1].num
            // Should compression ever fail, what is left goes a frame later.
            guard sendInstruction(old: old, new: new, events: userStates[run].flatMap(\.events), now: now)
            else { break }
            if userStates[run].contains(where: { !$0.sent }) { lastNewStateMs = now }
            for index in run {
                userStates[index].sent = true
                userStates[index].sentAtMs = now
            }
            old = new
        }
        lastSendMs = now
        sentAnything = true
        nextAckMs = now + Self.idleStateIntervalMs
        // mosh's client retries at frame rate only within 10 s of hearing
        // the server, and before it ever has (`last_heard` starts at 0) only
        // every 3 s; here the window opens with the first datagram, so a
        // lost first one goes again within a retransmission timeout, as the
        // old 1 s heartbeat sent it again.
        if lastHeardMs == nil { lastHeardMs = now }
        startLivenessMonitor()
        // Send-side vitals refresh on every datagram — deliberately not only
        // on receive, because "typing does nothing" is exactly the state
        // where nothing is being received, and the popover must stay honest
        // there.
        publishDiagnostics()
        if appliedHostNum == Self.shutdownStateNum { endSession() }
    }

    /// mosh-client's clean exit (`counterparty_shutdown_ack_sent`): once a
    /// datagram acking the server's shutdown state went out, the session is
    /// over. mosh-server quits on that ack (and after 16 tries or 10 s
    /// without it); this client used to apply the state, ack it, and then
    /// hear nothing: a frozen screen, flow rebuilds, and seconds later the
    /// dead-return-path banner offering to switch to SSH.
    private func endSession() {
        guard !sessionEnded else { return }
        sessionEnded = true
        Task { @MainActor in onSessionEnded?() }
        shutDown(lettingSendsFinish: true)
    }

    /// Output arrived (or a diff we could not apply): ack it within mosh's
    /// `ackDelayMs`, with whatever else is due by then.
    private func ackSoon(now: UInt64) {
        let at = now + Self.ackDelayMs
        nextAckMs = min(nextAckMs ?? at, at)
    }

    /// One instruction from user state `old` to `new`, fragmented and
    /// sealed. False when it could not be compressed.
    private func sendInstruction(old: UInt64, new: UInt64, events: [UserEvent], now: UInt64) -> Bool {
        var instruction = TransportInstruction()
        instruction.oldNum = old
        instruction.newNum = new
        instruction.ackNum = appliedHostNum
        // The oldest user state we may still diff from, as mosh's own client
        // says (transportsender-impl.h: `sent_states.front().num`, the acked
        // state). mosh-server keeps every user state it received until this
        // passes it, and once it holds more than 1024 it takes one new state
        // per 15 s and drops the rest (networktransport-impl.h,
        // `Transport::recv`). Left at 0 nothing was ever dropped: after 1024
        // keystroke batches and resizes in one session, typing reached the
        // remote shell up to 15 s late for the rest of it. Never past `old`:
        // the server finds `old` before it throws away, and must still hold
        // a state afterwards. In a run of instructions each names its own
        // old state: applying it, mosh-server holds that state, and lets the
        // run's earlier states go (a long paste is not kept whole in its
        // memory). The same goes for new input sent from the state the
        // server is assumed to hold (`diffBase`): mosh-server reads the
        // throwaway only once it found that state, and a later resend from
        // the acked state it then ignores waits for its ack, which names the
        // newer state, instead of growing its queue.
        instruction.throwawayNum = old
        instruction.diff = events.isEmpty ? Data() : UserMessage(events: events).encoded()

        guard let compressed = try? MoshCompression.compress(instruction.encoded()) else { return false }
        let fragments = TransportFragment.fragment(payload: compressed, id: nextInstructionId, mtu: 1300)
        nextInstructionId += 1

        for fragment in fragments {
            let message = MoshCrypto.Message(
                timestamp: Self.timestamp16(now),
                timestampReply: takeTimestampReply(now: now),
                payload: fragment.encoded())
            guard let packet = try? crypto.seal(message, seq: sendSeq, direction: .toServer) else { continue }
            sendSeq += 1
            channel.send(packet)
        }
        return true
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
        livenessMonitor = Task { [weak self, interval = livenessInterval] in
            while !Task.isCancelled {
                try? await Task.sleep(for: interval)
                await self?.checkLiveness()
            }
        }
    }

    private func checkLiveness() {
        guard !closed, sawAnyDatagram else { return }
        let now = clock()
        guard lastSendMs > 0,
              now &- lastInboundMs > Self.livenessDeadlineMs,
              lastSendMs > lastInboundMs else { return }
        rebuildsSinceInbound += 1
        if rebuildsSinceInbound == 3 {
            // A server we have heard from: news for the banner, never a
            // reason to stop it (see `onReturnPathDead`).
            Task { @MainActor in onReturnPathDead?(true) }
        }
        rebuildFlow()
        // Give the fresh flow something to announce itself with.
        nudge()
    }

    // MARK: Receive path

    /// Arm the flow's one receive. Every completion is handled by
    /// `received(_:error:flow:)` on the actor: a datagram fully before the
    /// next receive is armed (see there), an error ending the loop until
    /// the flow is ready again or replaced, a replaced flow's last word
    /// dropped.
    private func receiveNext() {
        guard !closed, !receiveArmed else { return }
        receiveArmed = true
        let flow = flowGeneration
        channel.receive { [weak self] data, error in
            guard let self else { return }
            Task { await self.received(data, error: error, flow: flow) }
        }
    }

    private func received(_ data: Data?, error: Error?, flow: UInt64) {
        // A replaced flow's last receive — a cancelled NWConnection completes
        // it, with no data and no error, which used to arm a second receive
        // loop on the replacement: two completion tasks, datagrams handled in
        // either order, every rebuild adding another.
        guard flow == flowGeneration else { return }
        receiveArmed = false
        // One actor-owned task processes this datagram fully *before*
        // re-arming the next receive. Splitting these into two independent
        // `Task`s let datagram N+1 begin handling before N finished — the
        // actor serializes state, not task-submission order — which breaks
        // `handleDatagram`'s order-sensitive apply logic (it only advances
        // `appliedHostNum` when `oldNum == appliedHostNum`). Re-arming only
        // after the current datagram is done also restores natural
        // backpressure: we don't ask the socket for more until we've
        // handled what we have.
        if let data, !data.isEmpty { handleDatagram(data) }
        guard error != nil else { receiveNext(); return }
        // NWConnection fails a receive when the flow fails (it is replaced
        // then, and `rebuildFlow` arms the new flow's receive) or meets a
        // socket error while it stays ready. The old loop simply stopped:
        // the server's datagrams went unread until the liveness check
        // rebuilt the flow 9 to 12 s later. As mosh-client pauses 200 ms
        // after a network error and reads on, the receive is armed again
        // then (`handleState` arms it sooner if the flow turns ready).
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(Self.receiveRetryMs))
            await self?.receiveAgain(flow)
        }
    }

    /// mosh-client's pause after a network error (stmclient.cc: 0.2 s).
    static let receiveRetryMs: Int64 = 200

    private func receiveAgain(_ flow: UInt64) {
        guard flow == flowGeneration else { return }
        receiveNext()
    }

    func handleDatagram(_ data: Data) {
        let now = clock()
        datagramsReceived += 1
        // The return path is alive the moment ANY datagram lands — even one we
        // can't decrypt/parse — so retire the watchdog on first sight, before
        // the direction/parse guards below can `return` early.
        lastInboundMs = now
        rebuildsSinceInbound = 0
        if !sawAnyDatagram {
            sawAnyDatagram = true
            returnPathWatchdog?.cancel()
            returnPathWatchdog = nil
        }
        guard let message = try? crypto.open(data), message.direction == .toClient else { return }

        // Roaming resolves the moment an authenticated reply lands.
        if roaming { roaming = false }
        noteTimestamps(of: message, now: now)

        guard let fragment = try? TransportFragment(parsing: message.payload),
              let payload = receiveAssembler.receive(fragment),
              let instructionBytes = try? MoshCompression.decompress(payload),
              let instruction = try? TransportInstruction(parsing: instructionBytes)
        else { publishMetrics(); return }

        // Server tells us which user states it has applied → drop ours. Only
        // for a state sent and still held, as mosh's
        // `process_acknowledgment_through` ignores an ack for a state it does
        // not hold (B105). Taken for any number, an ack for a state never
        // sent (only a bad server sends one) became the acked number: every
        // state below it went, the next got a number no greater than it, and
        // input was never sent again while every waiter was told delivered.
        if instruction.ackNum > serverAckedUserNum,
           userStates.contains(where: { $0.num == instruction.ackNum && $0.sent }) {
            serverAckedUserNum = instruction.ackNum
            userStates.removeAll { $0.num < serverAckedUserNum && $0.num != 0 }
            resumeDeliveryWaiters()
        }
        // The server answered: held input may flow again (see
        // settleHeldInput); the tick below sends it when it is due.
        _ = settleHeldInput(now: now)

        // Output is acked within `ackDelayMs` (mosh's `set_data_ack`), not
        // by a datagram per frame. So is what does not apply: the server
        // re-diffs from the state we ack, the one state we hold.
        // Apply a display diff only when it extends exactly the state we hold.
        if instruction.newNum > appliedHostNum, instruction.oldNum == appliedHostNum {
            if instruction.diff.isEmpty {
                // Legitimately empty diff (e.g. a pure ack/heartbeat) —
                // nothing to apply, safe to advance, nothing to ack early
                // (mosh acks only output early).
                appliedHostNum = instruction.newNum
                lastHeardMs = now
            } else if let host = try? HostMessage(parsing: instruction.diff) {
                // A diff may be nothing but a moved echo ack — still worth
                // delivering: it is what settles pending predictions.
                let echoMoved = (host.echoAck ?? 0) > echoAck
                if echoMoved, let ack = host.echoAck { echoAck = ack }
                // A redraw hides the frames drawn at its narrower width.
                let bytes = redrawShows(resize: host.resize, now: now) ? host.hostBytes : Data()
                if !bytes.isEmpty || echoMoved {
                    hostContinuation.yield(MoshHostFrame(bytes: bytes, echoAck: echoAck))
                }
                appliedHostNum = instruction.newNum
                lastHeardMs = now
                ackSoon(now: now)
            } else {
                // Parse failure: do NOT advance/ack. This diff's content
                // (the login banner, a command's output, anything) would
                // otherwise be silently and PERMANENTLY lost — mosh's
                // server-side diffing treats an ack as ground truth and
                // never resends content once acked, so falsely claiming
                // "applied" here left the terminal blank (just a live
                // cursor from later, unrelated cursor-move diffs still
                // landing fine against an otherwise-untouched buffer) with
                // no way to recover for the rest of the session. Acking
                // without updating `appliedHostNum` acks our TRUE (still
                // old) state instead, so the server keeps this content in
                // its next diff rather than considering it delivered.
                parseFailures += 1
                ackSoon(now: now)
            }
        } else if instruction.newNum > appliedHostNum {
            // Gap: ack what we have so the server re-diffs from our real state.
            gapEvents += 1
            ackSoon(now: now)
        }
        // mosh acks the shutdown state a frame after its last datagram
        // ("speed up shutdown sequence", `calculate_timers`), then quits.
        if appliedHostNum == Self.shutdownStateNum {
            nextAckMs = sentAnything ? lastSendMs + sendIntervalMs() : now
        }
        // Whatever is due now: input a hold released, a redraw's restored
        // size, an ack that was due anyway.
        tick()
        publishMetrics()
        publishDiagnostics()
    }

    // MARK: Timestamps & metrics

    /// The RTT sample and the timestamp to echo, by mosh's rules
    /// (network.cc, `Connection::recv_one`). 0xFFFF means "no timestamp" in
    /// both fields: the server sends it as the reply whenever it has nothing
    /// fresh of ours to echo. Treating 0 as the marker instead took each
    /// 0xFFFF reply for an RTT sample of `now16 + 1` ms whenever the clock's
    /// low 16 bits were under 5000 — the SRTT in the terminal pill went
    /// bogus for about 5 s out of every 65.5 s. A datagram older than one
    /// already seen (reordered or replayed) feeds neither; the first sample
    /// seeds the estimate whatever its value.
    private func noteTimestamps(of message: MoshCrypto.Message, now: UInt64) {
        guard message.seq >= expectedReceiverSeq else { return }
        expectedReceiverSeq = message.seq + 1
        if message.timestamp != Self.noTimestamp {
            savedRemoteTimestamp = (message.timestamp, now)
        }
        guard message.timestampReply != Self.noTimestamp else { return }
        let rtt = Double(Self.timestamp16(now) &- message.timestampReply)   // ms, wraps at 65536
        guard rtt < 5000 else { return }                                    // ignore implausible echoes
        // RTTVAR from the SRTT before this sample, as mosh orders them; it
        // only feeds the retransmission timeout.
        if rttHit {
            rttvar = 0.75 * rttvar + 0.25 * abs(srtt - rtt)
            srtt = 0.875 * srtt + 0.125 * rtt
        } else {
            srtt = rtt
            rttvar = rtt / 2
        }
        rttHit = true
    }

    /// The reply for the next datagram (mosh `Connection::new_packet`): the
    /// server's last timestamp advanced by how long we held it, once, and
    /// only within a second; otherwise 0xFFFF. Sending 0 before the first
    /// reply gave mosh-server a bogus first RTT sample (it seeds its SRTT,
    /// and so its frame interval) in about 7.6% of sessions (5000 of the
    /// 65536 values of its clock), and the old echo repeated one timestamp
    /// on every datagram for as long as nothing newer came in.
    private func takeTimestampReply(now: UInt64) -> UInt16 {
        guard let saved = savedRemoteTimestamp,
              Self.elapsed(since: saved.atMs, now: now) < Self.timestampHoldLimitMs
        else { return Self.noTimestamp }
        savedRemoteTimestamp = nil
        return saved.timestamp &+ UInt16(truncatingIfNeeded: Self.elapsed(since: saved.atMs, now: now))
    }

    /// A saved server timestamp is echoed only this fresh.
    private static let timestampHoldLimitMs: UInt64 = 1_000

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
            channelState: String(describing: channel.state),
            inputHeld: heldSinceMs != nil)
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
            appliedHostNum: appliedHostNum,
            packetsSent: sendSeq,
            lastUserNum: userStates.last?.num ?? 0,
            serverAckedUserNum: serverAckedUserNum,
            inputHeld: heldSinceMs != nil)
    }

    /// Monotonic milliseconds: the production clock.
    static let monotonicMs: @Sendable () -> UInt64 = {
        DispatchTime.now().uptimeNanoseconds / 1_000_000
    }

    /// `now - since`, 0 if the clock reads earlier than `since`.
    private static func elapsed(since: UInt64, now: UInt64) -> UInt64 {
        now >= since ? now - since : 0
    }

    /// mosh's 16-bit millisecond clock, which never reads 0xFFFF: that
    /// value means "no timestamp", so mosh skips it to 0.
    static func timestamp16(_ now: UInt64) -> UInt16 {
        let timestamp = UInt16(truncatingIfNeeded: now)
        return timestamp == noTimestamp ? 0 : timestamp
    }
}

private extension UserEvent {
    var isKeystroke: Bool {
        if case .keystroke = self { return true }
        return false
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
    /// Tear down the connection once the datagrams already handed to
    /// `send` have left (the last may be the ack mosh-server quits on).
    func cancelAfterSends()
    /// Send one datagram (fire-and-forget, mirroring UDP semantics).
    func send(_ datagram: Data)
    /// Arm a single-datagram receive; the completion fires once with the next
    /// datagram (or an error). The caller re-arms by calling `receive` again.
    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void)
}

extension DatagramChannel {
    /// A channel whose `send` is done when it returns has nothing to wait for.
    func cancelAfterSends() { cancel() }
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

    /// Sends handed to the connection and not yet processed, and whether to
    /// cancel when they are; both only ever touched on `queue`.
    private var sendsInFlight = 0
    private var cancelWhenSent = false

    /// `cancel()` drops the sends NWConnection has not processed yet, so
    /// this waits for their completions (on `queue`, after the counting
    /// below). A send stuck behind a flow that never became ready does not
    /// keep the socket open past a second.
    func cancelAfterSends() {
        queue.async { [self] in
            guard sendsInFlight > 0 else { connection.cancel(); return }
            cancelWhenSent = true
            queue.asyncAfter(deadline: .now() + 1) { [self] in connection.cancel() }
        }
    }

    func send(_ datagram: Data) {
        queue.async { [self] in sendsInFlight += 1 }
        connection.send(content: datagram, completion: .contentProcessed { [weak self] _ in
            guard let self else { return }
            sendsInFlight -= 1
            if cancelWhenSent, sendsInFlight == 0 { connection.cancel() }
        })
    }

    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {
        connection.receiveMessage { data, _, _, error in completion(data, error) }
    }
}

// MARK: - Host frames

/// One applied server diff, as the terminal layer consumes it.
struct MoshHostFrame: Equatable, Sendable {
    /// Terminal output to feed (may be empty when only the echo ack moved).
    var bytes: Data
    /// The newest user state (``MoshTransport/send(_:)``'s return value) the
    /// server had received at least 50ms before this frame — its echo, if
    /// the remote echoes at all, is in the screen by now. Monotonic.
    var echoAck: UInt64
}
