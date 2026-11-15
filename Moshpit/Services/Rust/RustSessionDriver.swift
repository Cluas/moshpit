import Foundation
import MoshpitFFI
import MoshpitKit
import UIKit

/// One ``SessionHub/ActiveSession`` running on the Rust core (every session
/// does, ``RustCore/available``): the core dials, signs in,
/// keeps the connection alive, redials, runs tmux's controller, herdr's
/// client or mosh; this puts what it says where the app's screens already
/// look.
///
/// - State: into the session's ``TerminalViewModel`` (the cover, the
///   capsule, the transport pill, the cards) — a reconnect is `.reconnecting`
///   over the kept frame, never a full-screen cover. A failure's words are the app's
///   (``RustCopy``): the core says only what happened (B202).
/// - The host-key card, a key's passphrase card: the view model's prompts.
/// - A plain shell or mosh: the pane's byte channel into the session's own
///   coordinator (``RustPaneFeeder``), its input and size back. mosh's
///   predictive echo judges its guesses by the echo acknowledgement the
///   core puts in that channel, after the frame it came with.
/// - tmux over SSH: a ``RustTmuxController`` per attach as the session's
///   `tmuxController` — the chrome reads it as it reads the Swift one — and
///   the previous attach's kept as the reconnect's frozen frame until the new
///   one has painted.
/// - tmux over mosh: mosh paints tmux's own client on the session's terminal;
///   the core's `-CC` sidecar is a ``RustTmuxController`` with no pane
///   terminals, as the session's `moshControl`.
/// - herdr over SSH: a ``RustHerdrController`` as the session's
///   `herdrControl`; the session's terminal follows the focused pane's byte
///   channel (herdr renders one pane at a time), its veil and reveal
///   arriving as marks; the banner is the core's notice, worded here.
/// - herdr over mosh: mosh paints the raw-attach loop (one pane, steered by
///   the core's control client beside mosh) on the session's terminal; the
///   same ``RustHerdrController`` for the chrome; a switch's veil and its
///   lifting arrive as marks in the shell's channel; typing, replies and
///   "Open here" go through the core, which decides where they may land.
@MainActor
final class RustSessionDriver: RustSessionListener {
    private unowned let session: SessionHub.ActiveSession
    private let core: RustCore
    private var viewModel: TerminalViewModel { session.viewModel }
    /// The registry, for the deinit: a session dropped without a stop (a
    /// dead one replaced by an edited connection) still leaves the core.
    private let registry: CoreSessionsProtocol
    nonisolated(unsafe) private var openSession: UInt64?
    /// Settings' predictive echo, heard while this is a mosh session (B533).
    nonisolated(unsafe) private var predictionChanges: NSObjectProtocol?

    /// The core's session, once connected.
    private(set) var id: UInt64? {
        didSet { openSession = closed ? nil : id }
    }
    private(set) var state: SessionStateValue = .connecting
    /// The connection as last saved while this session ran (B329); nil:
    /// as it started.
    private var adopted: ServerConnection?
    /// A save since the session's last connect: its next one is the saved
    /// connection's.
    private var adoptedPending = false
    /// What a save left for after that next connect (or the session's end):
    /// dropping the keychain item the session started with, which its
    /// reconnects read until then (B325, B329).
    private var releasesAfterReconnect: [() async -> Void] = []
    /// The shell pane's channel (a plain shell, mosh, a degraded connect),
    /// or herdr's focused pane's.
    private var shell: RustPaneFeeder?
    /// The current attach's tmux surface; nil without tmux.
    private(set) var tmux: RustTmuxController?
    /// herdr's surface; nil without herdr.
    private(set) var herdrSurface: RustHerdrController?
    private var appearance: RustTmuxController.Appearance
    private var hostKeyPrompt: UInt64?
    /// The diagnostics were told of the older host-key algorithm (B564).
    private var saidOlderHostKey = false
    private var signInQuestion: UInt64?
    /// The reads waiting on the passphrase card now, and which card that is
    /// (B631): every read that asks while it is up gets its one answer.
    private var passphraseWaiters: [CheckedContinuation<String?, Never>] = []
    private var passphraseCard = 0
    private var refusedHostKey = false
    private var settleWaiters: [CheckedContinuation<Void, Never>] = []
    private var closeWaiters: [CheckedContinuation<Void, Never>] = []
    /// herdr requests and deliveries waiting for their answer, by token.
    private var answers: [UInt64: CheckedContinuation<HerdrAnswerValue, Never>] = [:]
    private var deliveries: [UInt64: CheckedContinuation<Bool, Never>] = [:]
    private var closed = false
    private var firstFrameFallback: Task<Void, Never>?

    /// The session's connection, lent to what runs beside its terminal
    /// (W14b): the preview, uploads, the path probe, host maintenance.
    private(set) lazy var side = RustSideChannel(driver: self, session: session, sessions: registry)

    /// The session has closed for good (the core said so, or the wait for it
    /// to ended).
    var hasClosed: Bool { closed }

    private var connection: ServerConnection { session.connection }
    /// Whether this connection paints through tmux pane hosts.
    private var expectsTmux: Bool {
        connection.connectionProtocol == .ssh && connection.multiplexer == .tmux
    }
    /// mosh with tmux: the sidecar's tree beside mosh's screen.
    private var expectsSidecar: Bool {
        connection.connectionProtocol == .mosh && connection.multiplexer == .tmux
    }
    /// herdr over SSH: its panes' frames on the core's frame channel.
    private var expectsHerdr: Bool {
        connection.connectionProtocol == .ssh && connection.multiplexer == .herdr
    }
    /// herdr over mosh: mosh paints the loop, the control plane beside it.
    private var expectsMoshHerdr: Bool {
        connection.connectionProtocol == .mosh && connection.multiplexer == .herdr
    }
    /// herdr's chrome, on either transport.
    private var expectsHerdrSurface: Bool { expectsHerdr || expectsMoshHerdr }
    private var isMosh: Bool { connection.connectionProtocol == .mosh }

    init(session: SessionHub.ActiveSession, core: RustCore,
         appearance: RustTmuxController.Appearance) {
        self.session = session
        self.core = core
        self.registry = core.sessions
        self.appearance = appearance
    }

    deinit {
        if let openSession { registry.disconnect(session: openSession) }
        if let predictionChanges { NotificationCenter.default.removeObserver(predictionChanges) }
    }

    // MARK: Lifecycle

    /// Connects (or, after a failure or an end, connects again) and returns
    /// once the session is up, failed or ended — what the Swift `start()`
    /// awaits too, host-key card and Face ID included.
    func start(appearance: RustTmuxController.Appearance, automatic: Bool) async {
        self.appearance = appearance
        tmux?.configure(appearance)
        if let id {
            switch state {
            case .failed, .ended:
                // A new attempt: the last one's error is not its own
                // (B513).
                viewModel.errorMessage = nil
                viewModel.adoptRustStatus(.connecting, automatic: automatic)
                core.sessions.reconnect(session: id)
            case .connecting, .connected, .reconnecting:
                return
            }
        } else {
            connect(automatic: automatic)
        }
        await settled()
    }

    private func connect(automatic: Bool) {
        core.credentials.register(connection) { [weak self] wrong in
            await self?.askPassphrase(wrong: wrong)
        }
        wireShellCoordinator()
        if isMosh { attachPrediction() }
        let grid = session.coordinator.lastReportedSize
            ?? SessionHub.ActiveSession.estimateGrid(fontSize: appearance.fontSize)
        let id = core.sessions.connect(spec: Self.spec(for: connection, grid: grid))
        self.id = id
        core.listen(id, self)
        if RustCore.visibleSession === session { core.setVisible(id) }
        Log.ssh.info("rust session \(id, privacy: .public): connecting \(self.connection.connectionProtocol.rawValue, privacy: .public)+\(self.connection.multiplexer.rawValue, privacy: .public)")
        viewModel.adoptRustStatus(.connecting, automatic: automatic)
        if expectsTmux || expectsHerdr { session.rustSetAwaitingFirstFrame(true) }
    }

    /// What the core connects to. mosh-server's UDP ports are Settings'
    /// (`udp`, ``AppSettings/udpRange``): the one source, "set in Settings,
    /// for every connection". A connection's stored range is only what the
    /// setting was at its last save, kept for older builds (J1) — dialled
    /// from, a narrowed setting never reached a saved connection (B430).
    static func spec(for connection: ServerConnection, grid: (cols: Int, rows: Int),
                     udp: (low: Int, high: Int) = AppSettings.shared.udpRange) -> SessionSpec {
        let proxy = connection.useSOCKSProxy == true ? connection.socksProxyHost : nil
        return SessionSpec(
            connectionId: connection.id.uuidString,
            user: connection.username,
            host: connection.host,
            port: UInt16(clamping: connection.port),
            proxyHost: proxy,
            proxyPort: connection.socksProxyPort.map { UInt16(clamping: $0) },
            mosh: connection.connectionProtocol == .mosh,
            multiplexer: {
                switch connection.multiplexer {
                case .none: return .none
                case .tmux: return .tmux
                case .herdr: return .herdr
                }
            }(),
            tmuxPath: connection.tmuxPath,
            moshServerPath: connection.moshServerPath,
            moshPortLow: Int64(udp.low),
            moshPortHigh: Int64(udp.high),
            herdrPath: connection.herdrPath,
            locale: "en_US.UTF-8",
            cols: UInt16(clamping: grid.cols),
            rows: UInt16(clamping: grid.rows),
            // B783: a new host's card says it was connected to before.
            connectedBefore: connection.lastConnectedAt != nil)
    }

    /// Closes the session for good, holding the process awake while the
    /// core hands back what it holds on the server (tmux's pins, a
    /// mosh-server it started), bounded.
    func stop() async {
        firstFrameFallback?.cancel()
        settle()
        viewModel.hostKeyPrompt?.decide(false)
        viewModel.keyPassphrasePrompt?.decide(nil)
        viewModel.signInQuestion?.decide(nil)
        guard let id, !closed else { return }
        core.sessions.disconnect(session: id)
        openSession = nil
        let assertion = BackgroundAssertion(name: "moshpit.rust-disconnect")
        await waitClosed(atMost: .seconds(10))
        assertion.end()
        teardown()
        switch state {
        case .failed, .ended: break
        default: viewModel.adoptRustStatus(.disconnected, automatic: false)
        }
    }

    /// The connection was saved while this session runs (B329): its next
    /// connect — a reconnect after a drop, or the person's — goes to the
    /// saved address as the saved user, through the saved proxy, with the
    /// saved credential, not the ones it started with. The transport live
    /// now stays where it is, and so do the protocol and the multiplexer.
    func adopt(_ saved: ServerConnection) {
        guard saved.id == connection.id, !Self.dialsTheSame(saved, adopted ?? connection) else { return }
        adopted = saved
        core.credentials.register(saved) { [weak self] wrong in
            await self?.askPassphrase(wrong: wrong)
        }
        guard let id, !closed else { return }
        let grid = session.coordinator.lastReportedSize
            ?? SessionHub.ActiveSession.estimateGrid(fontSize: appearance.fontSize)
        core.sessions.respec(session: id, spec: Self.spec(for: saved, grid: grid))
        adoptedPending = true
        Log.ssh.info("rust session \(id, privacy: .public): its connection was saved; the next connect uses it")
    }

    /// Whether a connect to `a` goes where one to `b` does, as the same
    /// user with the same credential (a save that changed only the name, or
    /// a last-connected stamp, changes nothing here).
    static func dialsTheSame(_ a: ServerConnection, _ b: ServerConnection) -> Bool {
        a.host == b.host && a.port == b.port && a.sshPort == b.sshPort && a.username == b.username
            && a.authMethod == b.authMethod && a.keychainRef == b.keychainRef && a.sshKeyId == b.sshKeyId
            && a.useSOCKSProxy == b.useSOCKSProxy && a.socksProxyHost == b.socksProxyHost
            && a.socksProxyPort == b.socksProxyPort
    }

    /// Run `release` once this session has connected again as its
    /// connection was saved, or has ended (B329).
    func releaseAfterReconnect(_ release: @escaping () async -> Void) {
        releasesAfterReconnect.append(release)
    }

    private func runReleases() {
        let releases = releasesAfterReconnect
        releasesAfterReconnect = []
        guard !releases.isEmpty else { return }
        Task { for release in releases { await release() } }
    }

    private func teardown() {
        shell?.detach()
        shell = nil
        tmux?.detachAll()
        session.retiredTmuxController.flatMap { $0 as? RustTmuxController }?.detachAll()
        if let id { core.forget(id) }
        core.credentials.unregister(connection)
        runReleases()
        session.coordinator.attachPrediction(nil)
        for (_, waiter) in answers { waiter.resume(returning: .done) }
        answers = [:]
        for (_, waiter) in deliveries { waiter.resume(returning: false) }
        deliveries = [:]
    }

    var isLive: Bool { state == .connected }

    // MARK: Events

    func handle(_ event: SessionEventKind) {
        switch event {
        case .state(let state):
            apply(state)
        case .panes(let attach, let panes):
            panesChanged(attach: attach, panes: panes)
        case .firstFrame(let attach):
            firstFrame(attach: attach)
        case .treeChanged:
            mirrorTree()
        case .paneActivity(let pane):
            tmux?.onPaneActivity?(pane)
        case .paneBell(let pane):
            tmux?.onPaneBell?(pane)
        case .paneSnapshot(let pane, let rows):
            tmux?.onPaneSnapshot?(pane, rows)
            herdrSurface?.onPaneSnapshot?(pane, rows)
        case .agentHooksUpdated:
            mirrorTree()
            tmux?.onAgentHooksUpdated?()
            herdrSurface?.onAgentHooksUpdated?()
        case .attachStalled:
            session.attachStalled = true
            endFirstFrameWait()
        case .tmuxActionsDropped:
            // Commands given while nothing carried them (mosh's control
            // plane down, B401; the connection dialled again, B483) waited
            // past their bound; an attach is back now.
            tmux?.noteRefused(RustCopy.tmuxActionsDropped)
        case .hostKeyPrompt(let prompt, let host, let port, let fingerprint, let previous, let older,
                            let conflicting, let connectedBefore):
            presentHostKey(prompt: prompt, host: host, port: Int(port),
                           fingerprint: fingerprint, previous: previous, older: older,
                           conflicting: conflicting, connectedBefore: connectedBefore)
        case .olderHostKeyAlgorithm(let algorithm):
            // B564: for the diagnostics, once a session.
            if !saidOlderHostKey {
                saidOlderHostKey = true
                Log.ssh.notice("rust session \(self.id ?? 0, privacy: .public): this server uses an older host-key algorithm (\(algorithm, privacy: .public), SHA-1)")
            }
        case .hostKeyPromptWithdrawn(let prompt):
            if hostKeyPrompt == prompt {
                hostKeyPrompt = nil
                viewModel.hostKeyPrompt = nil
                PromptNotifier.shared.answered(PromptNotice.id(connection.id, .hostKey))
            }
        case .signInQuestion(let prompt, let host, let name, let instructions, let text, let echo):
            presentQuestion(prompt: prompt, host: host,
                            words: Self.serverWords(name: name, instructions: instructions, prompt: text),
                            echo: echo)
        case .signInQuestionWithdrawn(let prompt):
            if signInQuestion == prompt {
                signInQuestion = nil
                viewModel.signInQuestion = nil
                PromptNotifier.shared.answered(PromptNotice.id(connection.id, .question))
            }
        case .degraded(let degrade):
            publishCapabilities()
            session.degrade = Self.notice(for: degrade)
        case .returnPathDead:
            session.moshReturnPathDead = true
        case .returnPathAlive:
            // mosh hears the server again: the offer of SSH goes (B400).
            session.moshReturnPathDead = false
        case .moshMetrics(let srtt, let roaming):
            session.metricsSink?(srtt, roaming)
            session.coordinator.prediction?.srttMs = srtt
        case .moshDiagnostics(let datagrams, let parseFailures, let gaps, let applied, let sent,
                              let user, let acked, let channel, let held):
            // The pill's long-press popover reads these (B263: never filled
            // on the core's path before).
            session.diagnosticsSink?(MoshDiagnostics(
                datagramsReceived: datagrams, parseFailures: parseFailures, gapEvents: gaps,
                appliedHostNum: applied, packetsSent: sent, lastUserNum: user,
                serverAckedUserNum: acked, channelState: channel, inputHeld: held))
        case .inputDiscarded(let stream, let bytes, let why):
            Log.input.info("rust: \(bytes, privacy: .public) bytes for stream \(stream, privacy: .public) not sent (\(String(describing: why), privacy: .public))")
            if why == .reconnected { session.rustNoteInputLost() }
        case .resumeChecked(let awayMs, let waitedMs, let outcome, let throughTmux, let held):
            // Resume in place's data (the core always resumes in place): how often
            // iOS keeps a suspended socket, and what asking it costs. Notice,
            // so it stays in the device's log for a sysdiagnose.
            Log.ssh.notice("rust session \(self.id ?? 0, privacy: .public): back after \(awayMs / 1000, privacy: .public) s away: \(Self.resumeWords(outcome), privacy: .public) after \(waitedMs, privacy: .public) ms on \(throughTmux ? "tmux" : "ssh", privacy: .public), \(held, privacy: .public) held writes")
        case .herdrAnswer(let token, let answer):
            answers.removeValue(forKey: token)?.resume(returning: answer)
        case .delivered(let token, let delivered):
            deliveries.removeValue(forKey: token)?.resume(returning: delivered)
        case .closed:
            closed = true
            openSession = nil
            let waiters = closeWaiters
            closeWaiters = []
            for waiter in waiters { waiter.resume() }
        case .flushed:
            break
        }
        #if DEBUG
        RustSessionSeed.observe(event, driver: self)
        #endif
    }

    private func apply(_ new: SessionStateValue) {
        let previous = state
        state = new
        side.stateChanged(new)
        switch new {
        case .connecting:
            viewModel.adoptRustStatus(.connecting, automatic: false)
        case .reconnecting:
            retireTmuxForReconnect()
            session.coordinator.dropPredictions()
            viewModel.adoptRustStatus(.reconnecting, automatic: true)
        case .connected:
            if adoptedPending, previous != .connected {
                // Connected again as the connection was saved: what it
                // started with is no longer read.
                adoptedPending = false
                runReleases()
            }
            refusedHostKey = false
            session.attachStalled = false
            session.moshReturnPathDead = false
            publishCapabilities()
            viewModel.adoptRustStatus(.connected, automatic: false)
            wireTmuxIfExpected()
            wireSidecarIfExpected()
            wireHerdrIfExpected()
            settle()
        case .ended(let reason):
            endFirstFrameWait()
            switch reason {
            case .disconnected:
                viewModel.adoptRustStatus(.disconnected, automatic: false)
            case .shellExited:
                let message = String(localized: "The session ended: the remote shell exited.")
                viewModel.adoptRustStatus(.ended(message), automatic: false, error: message)
                session.endedSink?()
            case .moshServerEnded:
                let message = String(localized: "The session ended: the remote shell exited or mosh-server was stopped.")
                viewModel.adoptRustStatus(.ended(message), automatic: false, error: message)
                session.endedSink?()
            }
            settle()
        case .failed(let kind, let cause, let log, let retryable):
            endFirstFrameWait()
            Log.ssh.notice("rust session: failed (\(String(describing: kind), privacy: .public)): \(log, privacy: .public)")
            if kind == .hostKey && refusedHostKey {
                // The person's own no to the server's key (B151): the
                // connect ends quietly — no card, the terminal leaves — and
                // the hub takes the session out, so neither a dead card nor a
                // redial is left behind (B334).
                viewModel.adoptRustDeclined()
                settle()
                session.rustRetireDeclined()
                return
            }
            // A modal for a connect the person asked for — and for a failure
            // nothing will try again on its own (rejected credentials, a
            // refused key: the core leaves them failed), whoever started the
            // attempt: an automatic redial that ends there is news, not a
            // "Line dropped" with no reason forever (B163, B335). A failure
            // the core retries stays quiet behind the redial.
            let asked = previous == .connecting
            let message = RustCopy.failure(kind: kind, cause: cause)
            viewModel.adoptRustStatus(.failed(message), automatic: false,
                                      error: asked || !retryable ? message : nil)
            settle()
        }
    }

    /// What the core's probe found about the host, on the session as the
    /// Swift path publishes it (`probeCapabilities`): Install Assist's
    /// package manager and install command, the empty state's "not
    /// installed". The core saves its probe under the app's own
    /// `hostcaps.<UUID>` before it routes the connect, so it is there by the
    /// time the session is connected or degraded; nothing else published it
    /// on this path, and Install Assist had no command until its Re-check
    /// (B336).
    private func publishCapabilities() {
        guard let known = HostCapabilityCache.shared.reload(for: connection.id) else { return }
        session.rustSetCapabilities(known)
    }

    /// The diagnostics' word for what the way back found.
    nonisolated static func resumeWords(_ outcome: ResumeOutcomeValue) -> String {
        switch outcome {
        case .kept: return "kept"
        case .silent: return "no answer, redialled"
        case .dead: return "socket dead, redialled"
        }
    }

    private static func notice(for degrade: DegradeValue) -> DegradeNotice? {
        switch degrade {
        case .moshServerMissing: return DegradeNotice(missing: .moshServer)
        case .tmuxMissing: return DegradeNotice(missing: .tmux)
        case .herdrMissing, .herdrUnavailable: return DegradeNotice(missing: .herdr)
        }
    }

    // MARK: Panes

    private func panesChanged(attach: UInt64, panes: [PaneValue]) {
        if panes.count == 1, let pane = panes.first, pane.pane == "shell" {
            attachShell(stream: pane.stream)
            if tmux != nil, !expectsSidecar {
                // tmux went away under a degrade: the plain pane paints.
                tmux?.detachAll()
                tmux = nil
                session.rustSetTmux(nil)
            }
            session.rustSetRetired(nil)
            endFirstFrameWait()
            return
        }
        guard !panes.isEmpty else { return }
        if expectsHerdr {
            // herdr renders one pane at a time: the terminal follows the
            // focused pane's channel.
            let active = panes.first(where: \.active)
            if let focused = active ?? panes.first {
                // Which channel the terminal reads, against where herdr's
                // frames go (B445).
                let model = id.flatMap { core.sessions.herdrView(session: $0) }
                let listed = panes.map { "\($0.pane):\($0.stream)\($0.active ? "*" : "")" }.joined(separator: " ")
                Log.ssh.notice("rust session \(self.id ?? 0, privacy: .public): herdr panes \(listed, privacy: .public); the terminal reads \(focused.pane, privacy: .public) (\(active == nil ? "no active pane: the first" : "active", privacy: .public)); frames for \(model?.frameTarget ?? "-", privacy: .public), attached \(model?.attachedPane ?? "-", privacy: .public), server focus \(model?.activePaneId ?? "-", privacy: .public)")
                attachHerdrPane(stream: focused.stream, pane: focused.pane)
            }
            return
        }
        let adapter: RustTmuxController
        if let tmux, tmux.attach == nil || tmux.attach == attach {
            adapter = tmux
        } else {
            adapter = installTmux()
        }
        adapter.attach = attach
        adapter.updatePanes(panes)
    }

    private func attachShell(stream: UInt64) {
        guard shell?.stream != stream else { return }
        shell?.detach()
        let feeder = RustPaneFeeder(stream: stream, coordinator: session.coordinator)
        feeder.onMark = { [weak self] mark in self?.shellMark(mark) }
        session.coordinator.onMark = { [weak self] mark in self?.shellViewMark(mark) }
        // A shell's program (herdr's own client over mosh too) asks this
        // terminal, and is answered.
        session.coordinator.answersQueries = true
        shell = feeder
        if expectsMoshHerdr {
            // mosh paints herdr's own screen here: swipes and taps are
            // herdr's, not the local buffer's (see `attachHerdrPane`).
            session.coordinator.onScroll = { [weak self] lines in self?.scroll(lines: lines) }
            session.coordinator.onClick = { [weak self] col, row in self?.click(col: col, row: row) }
        } else if expectsSidecar {
            // mosh paints tmux's own client here: a swipe pages tmux's
            // history (copy-mode on that client) or reaches a mouse app as
            // its wheel, as the core decides (B362). Left to the terminal it
            // scrolled a local buffer mosh never fills: nothing moved. The
            // drag's start re-reads the active pane's mouse flag.
            session.coordinator.onScroll = { [weak self] lines in self?.scroll(lines: lines) }
            session.coordinator.onScrollBegin = { [weak self] in
                // The wheel goes where the drag began (B578).
                if let aim = self?.session.coordinator.wheelAim {
                    self?.perform(.aimWheel(col: Int64(aim.col), row: Int64(aim.row)))
                }
                self?.perform(.scrollBegan)
            }
        }
        if let size = session.coordinator.lastReportedSize {
            feeder.resize(cols: size.cols, rows: size.rows)
        }
    }

    /// herdr's focused pane, on the session's terminal: its veil and its
    /// reveal come as marks among its frames.
    private func attachHerdrPane(stream: UInt64, pane: String) {
        guard shell?.stream != stream else { return }
        shell?.detach()
        let coordinator = session.coordinator
        // herdr's frames are paintings of a pane herdr emulates and answers
        // for: nothing here answers a query (B575).
        coordinator.answersQueries = false
        let feeder = RustPaneFeeder(stream: stream, coordinator: coordinator)
        // What reaches the terminal from it in its first two seconds (B445:
        // a blank herdr terminal is bytes that never came, or bytes drawn
        // and not shown).
        let session = self.id ?? 0
        Log.ssh.notice("rust session \(session, privacy: .public): herdr's terminal follows \(pane, privacy: .public) on stream \(stream, privacy: .public)")
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak feeder] in
            guard let feeder else { return }
            Log.ssh.notice("rust session \(session, privacy: .public): herdr stream \(stream, privacy: .public) (\(pane, privacy: .public)) fed \(feeder.bytesFed, privacy: .public) bytes in its first 2 s")
        }
        // Among the frames' bytes, acted on once the bytes before each are
        // on screen (``CoreTerminalView/feed(mark:)``).
        feeder.onMark = { [weak coordinator] mark in
            coordinator?.feed(mark: TerminalMark(code: mark.code, arg: mark.arg))
        }
        coordinator.onMark = { [weak self] mark in self?.shellViewMark(mark) }
        shell = feeder
        // Swipes and taps are herdr's, as the Swift path wires them: left
        // unset, the terminal scrolled its own buffer of repaint frames.
        coordinator.onScroll = { [weak self] lines in self?.scroll(lines: lines) }
        coordinator.onClick = { [weak self] col, row in self?.click(col: col, row: row) }
        if let size = coordinator.lastReportedSize {
            feeder.resize(cols: size.cols, rows: size.rows)
        }
    }

    /// A mark in the shell's channel: mosh's echo acknowledgement, after the
    /// frame it came with — predictive echo judges its guesses by it; and,
    /// with herdr over mosh, a pane switch's veil and its lifting (once the
    /// new pane's repaint went quiet). The veil is all that may be done: mosh
    /// diffs against its own model of the screen, so cells wiped behind its
    /// back would stay wiped.
    private func shellMark(_ mark: MpStreamMark) {
        let coordinator = session.coordinator
        switch mark.code {
        case RustTmuxController.Mark.echoAck:
            // Judged after the bytes of its frame (the terminal's own queue).
            coordinator.feed(moshFrame: MoshHostFrame(bytes: Data(),
                                                      echoAck: UInt64(max(0, mark.arg))))
        default:
            // Among the bytes: acted on once those before it are on screen.
            coordinator.feed(mark: TerminalMark(code: mark.code, arg: mark.arg))
        }
    }

    /// A shell mark whose bytes are on screen: herdr's veil, its lifting.
    private func shellViewMark(_ mark: TerminalMark) {
        let coordinator = session.coordinator
        switch mark.code {
        case RustTmuxController.Mark.veil:
            coordinator.veilForSwitch()
        case RustTmuxController.Mark.extendCover:
            coordinator.extendCoverTimeout(by: TimeInterval(mark.arg) / 1000)
        case RustTmuxController.Mark.reveal:
            coordinator.reveal()
        default:
            break
        }
    }

    /// mosh's predictive echo on the session's terminal, as the Swift path
    /// sets it up; its evidence comes in the channel (``shellMark(_:)``).
    /// Settings' mode or trail changed while it runs is taken at once
    /// (B533): it used to wait for the next connect.
    private func attachPrediction() {
        if predictionChanges == nil {
            predictionChanges = NotificationCenter.default.addObserver(
                forName: .moshpitPredictionChanged, object: AppSettings.shared, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.predict() }
            }
        }
        predict()
    }

    /// The prediction Settings and the connection say now.
    private func predict() {
        session.coordinator.attachPrediction(MoshPrediction(
            mode: connection.predictOverride ?? AppSettings.shared.predictMode,
            trail: AppSettings.shared.trailOnPredict))
    }

    /// The session's own coordinator (the plain path's): its keys and sizes
    /// to the shell pane's channel. Scrolling and taps stay the
    /// coordinator's own (local scrollback, the local app's mouse).
    private func wireShellCoordinator() {
        let coordinator = session.coordinator
        coordinator.onInput = { [weak session] data in session?.sendInput(data) }
        coordinator.onSizeChange = { [weak self] cols, rows in
            self?.shell?.resize(cols: cols, rows: rows)
        }
    }

    // MARK: tmux

    /// Connected over SSH with tmux: the surface the chrome reads, ready
    /// before tmux confirms the attach (an empty server shows its empty
    /// state through it). A reconnect gets a fresh one; the previous one is
    /// the frozen frame until the new attach paints.
    private func wireTmuxIfExpected() {
        guard expectsTmux, session.degrade == nil else { return }
        if let tmux, tmux.attach == nil { return }
        installTmux()
        session.rustSetAwaitingFirstFrame(true)
        armFirstFrameFallback()
    }

    /// Connected over mosh with tmux: the sidecar's surface (one for the
    /// session: a reconnect's sidecar mirrors into the same one).
    private func wireSidecarIfExpected() {
        guard expectsSidecar, session.degrade == nil, tmux == nil else { return }
        let grid = session.coordinator.lastReportedSize
            ?? SessionHub.ActiveSession.estimateGrid(fontSize: appearance.fontSize)
        let adapter = RustTmuxController(driver: self, appearance: appearance, grid: grid,
                                         sidecar: true)
        tmux = adapter
        session.rustSetMoshControl(adapter)
        session.rustControlPlaneWired(adapter)
    }

    @discardableResult
    private func installTmux() -> RustTmuxController {
        let grid = session.coordinator.lastReportedSize
            ?? SessionHub.ActiveSession.estimateGrid(fontSize: appearance.fontSize)
        let adapter = RustTmuxController(driver: self, appearance: appearance, grid: grid)
        if let old = tmux, old !== session.retiredTmuxController {
            old.detachAll()
        }
        tmux = adapter
        session.rustSetTmux(adapter)
        session.rustControlPlaneWired(adapter)
        // Not mirrored yet: until the new attach's first tree, the core's
        // read model still holds the dropped one's.
        return adapter
    }

    /// The connection dropped: the painted surface stays on screen, frozen,
    /// until the replacement paints (never over an older one).
    private func retireTmuxForReconnect() {
        guard !expectsSidecar, let tmux, tmux.paintedSinceAttach,
              session.retiredTmuxController == nil else { return }
        session.rustSetRetired(tmux)
    }

    private func firstFrame(attach: UInt64) {
        if expectsHerdr {
            endFirstFrameWait()
            return
        }
        guard let tmux, tmux.attach == attach else { return }
        tmux.notePainted()
        if let ghost = session.retiredTmuxController as? RustTmuxController, ghost !== tmux {
            ghost.detachAll()
        }
        session.rustSetRetired(nil)
        endFirstFrameWait()
    }

    /// A server with nothing to attach never paints: after a while the
    /// cover gives way to the empty state, as the Swift path's fallback does.
    private func armFirstFrameFallback() {
        firstFrameFallback?.cancel()
        firstFrameFallback = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(6))
            guard !Task.isCancelled, let self else { return }
            if self.expectsHerdr {
                // The reconnect veil waits on herdr's first frame now (B323):
                // one that never comes must not hold it up.
                self.endFirstFrameWait()
            } else if self.tmux?.snapshot.isAttached != true || self.tmux?.paintedSinceAttach == false {
                self.endFirstFrameWait()
            }
        }
    }

    private func endFirstFrameWait() {
        firstFrameFallback?.cancel()
        firstFrameFallback = nil
        session.rustSetAwaitingFirstFrame(false)
    }

    private func mirrorTree() {
        guard let id else { return }
        if let tmux, let model = core.sessions.tmuxView(session: id) {
            tmux.mirror(model)
            // The server has no session (B228): the core asked it. The flag
            // the Swift path's mosh attach sets, so the terminal shows the
            // empty state over mosh's waiting renderer and Home doesn't read
            // it as attaching (B337).
            if session.moshServerHadNoSession != model.noSession {
                session.moshServerHadNoSession = model.noSession
            }
        }
        if let herdrSurface, let model = core.sessions.herdrView(session: id) {
            herdrSurface.mirror(model)
            let (notice, action) = RustCopy.herdrNotice(model.notice, action: model.noticeAction)
            session.rustSetHerdrNotice(notice, action: action)
        }
    }

    /// What the tmux surface asks of the core; what the core refused to
    /// send, if it did (B250: a typed name with a control character).
    @discardableResult
    func perform(_ intent: TmuxIntent) -> TmuxRefused? {
        guard let id else { return nil }
        do {
            try core.sessions.tmux(session: id, intent: intent)
            return nil
        } catch let refused as TmuxRefused {
            return refused
        } catch {
            return nil
        }
    }

    // MARK: herdr

    /// Connected with herdr: its surface, one for the session (a
    /// reconnect's client mirrors into the same one). Over mosh the screen
    /// is mosh's: no frame to wait for.
    private func wireHerdrIfExpected() {
        guard expectsHerdrSurface, session.degrade == nil else { return }
        if herdrSurface == nil {
            let surface = RustHerdrController(driver: self)
            herdrSurface = surface
            session.rustSetHerdr(surface)
            session.rustControlPlaneWired(surface)
        }
        if expectsHerdr {
            session.rustSetAwaitingFirstFrame(true)
            armFirstFrameFallback()
        }
        mirrorTree()
    }

    /// An intent for herdr, with nothing to wait for.
    func herdr(_ intent: HerdrIntent) {
        guard let id else { return }
        _ = core.sessions.herdr(session: id, intent: intent)
    }

    /// An intent for herdr, and the core's answer to it (every intent is
    /// answered once, bounded on the core's side).
    func ask(_ intent: HerdrIntent) async -> HerdrAnswerValue {
        guard let id, !closed else { return .done }
        return await withCheckedContinuation { continuation in
            let token = core.sessions.herdr(session: id, intent: intent)
            answers[token] = continuation
        }
    }

    /// The banner's button.
    func runHerdrNoticeAction() {
        herdr(.noticeAction)
    }

    // MARK: Input

    /// The shortcut bar, the keyboard, a paste's bytes: into the pane on
    /// screen. While the session isn't live they go to the core all the
    /// same, which under a reconnect's veil follows B396's rule (B322): a
    /// pane's keys wait for the pane, a plain shell's are not sent and that
    /// is said. They used to stop here, unsaid. Over mosh, typing is
    /// guessed first while live (predictive echo), and the guess is told
    /// where in the channel its bytes end — what the core's echo
    /// acknowledgement counts in.
    func sendInput(_ data: Data) {
        let live = viewModel.connState == .live
        if let tmux, !tmux.sidecar {
            tmux.sendToActivePane(data)
        } else if isMosh, let shell {
            let batch = live ? session.coordinator.predict(data) : nil
            shell.write(data)
            if let batch {
                session.coordinator.predictionSent(batch: batch, userState: shell.written)
            }
        } else {
            shell?.write(data)
        }
    }

    /// The shortcut bar's keys: as typing while the session is live; under
    /// a reconnect's veil the core drops them rather than keep them for the
    /// pane (B476: a stray tap must not land in an agent's prompt later).
    func sendShortcut(_ data: Data) {
        let live = viewModel.connState == .live
        if let tmux, !tmux.sidecar {
            tmux.sendBarToActivePane(data)
        } else if isMosh, let shell {
            let batch = live ? session.coordinator.predict(data) : nil
            shell.writeBar(data)
            if let batch {
                session.coordinator.predictionSent(batch: batch, userState: shell.written)
            }
        } else {
            shell?.writeBar(data)
        }
    }

    /// A paste rides the pane's channel, as typing does, so it keeps its
    /// place among the keys typed before it and waits with them while a
    /// return from the background waits for its answer (B361), or a
    /// reconnect brings the pane back (B322); a pane with no channel yet
    /// gets it as an intent, as before.
    func sendPaste(_ text: String) {
        if let tmux, !tmux.sidecar, let pane = tmux.inputPaneId {
            tmux.paste(text, toPane: pane)
        } else if expectsHerdr, let id, let pane = herdrSurface?.snapshot.activePaneId {
            // herdr fits one bracketed paste to the pane's real mode (B43).
            if shell?.paste(text) == true { return }
            core.sessions.paste(session: id, pane: pane, text: text)
        } else if expectsMoshHerdr, let id {
            // Through the renderer's rule, as typing is: only into the pane
            // on screen by the host's word.
            session.coordinator.dropPredictions()
            if shell?.paste(text) == true { return }
            core.sessions.paste(session: id, pane: herdrSurface?.snapshot.activePaneId ?? "shell",
                                text: text)
        } else {
            let bracketed = session.coordinator.modes?.bracketedPaste == true
            let payload = BracketedPaste.payload(text, bracketed: bracketed)
            if !payload.isEmpty {
                session.coordinator.dropPredictions()
                shell?.write(payload)
            }
        }
    }

    /// Bytes for one pane on purpose (a notification's reply, a share): the
    /// core waits out a reconnect for it (bounded), switches herdr to the
    /// pane and holds it for the pane's first frame, and says whether it
    /// went out.
    func deliverInput(_ data: Data, toPane pane: String) async -> Bool {
        await deliver(data, toPane: pane, paste: false)
    }

    func deliverPaste(_ text: String, toPane pane: String) async -> Bool {
        if !expectsTmux, !expectsSidecar, !expectsHerdrSurface {
            // A plain shell: the local terminal knows the program's paste mode.
            let bracketed = session.coordinator.modes?.bracketedPaste == true
            return await deliver(BracketedPaste.payload(text, bracketed: bracketed),
                                 toPane: "shell", paste: false)
        }
        return await deliver(Data(text.utf8), toPane: pane, paste: true)
    }

    private func deliver(_ data: Data, toPane pane: String, paste: Bool) async -> Bool {
        guard let id, !closed, !data.isEmpty else { return false }
        let target = (expectsTmux || expectsSidecar || expectsHerdrSurface) ? pane : "shell"
        return await withCheckedContinuation { continuation in
            let token = core.sessions.deliver(session: id, pane: target, bytes: data, paste: paste)
            deliveries[token] = continuation
        }
    }

    func scroll(lines: Int) {
        if let tmux {
            guard viewModel.connState == .live else { return }
            session.coordinator.dropPredictions()
            perform(.scrollLines(lines: Int64(lines)))
            return
        }
        if expectsHerdrSurface {
            // herdr owns the pane's history (and a full-screen program's
            // own scrolling): a swipe is herdr's scroll, never the local
            // buffer — that holds only repaint frames, so scrolling it
            // slid the whole screen, input line and all, over stale frames.
            guard viewModel.connState == .live, let id,
                  let pane = herdrSurface?.snapshot.activePaneId else { return }
            _ = core.sessions.herdr(session: id, intent: .scroll(pane: pane, lines: Int64(lines),
                                                                  pageKeys: false))
            return
        }
        let coordinator = session.coordinator
        if viewModel.connState == .live, coordinator.localAppWantsMouse {
            coordinator.dropPredictions()
            coordinator.sendWheel(lines: lines)
        } else {
            coordinator.scrollLocal(lines: lines)
        }
    }

    func click(col: Int, row: Int) {
        if let tmux {
            session.coordinator.dropPredictions()
            perform(.click(col: Int64(col), row: Int64(row)))
        } else if expectsHerdrSurface {
            // A tap is a left click in herdr's pane (herdr ≥ 0.9.2 takes
            // mouse input), as on the Swift path.
            guard viewModel.connState == .live, let id,
                  let pane = herdrSurface?.snapshot.activePaneId else { return }
            for action in [HerdrMouseAction.down, .up] {
                _ = core.sessions.herdr(session: id, intent: .mouse(
                    pane: pane, action: action, button: .left,
                    column: Int64(col), row: Int64(row), modifiers: 0))
            }
        } else {
            session.coordinator.dropPredictions()
            session.coordinator.clickLocal(col: col, row: row)
        }
    }

    /// The mosh diagnostics' Repaint, on the core's mosh (B338).
    func repaint() {
        guard let id else { return }
        core.sessions.repaint(session: id)
    }

    func createFirstTmuxSession() {
        perform(.newSession(name: nil))
    }

    // MARK: Cards

    private func presentHostKey(prompt: UInt64, host: String, port: Int, fingerprint: String,
                                previous: String?, older: String?, conflicting: Bool,
                                connectedBefore: Bool = false) {
        hostKeyPrompt = prompt
        // In the background, a notification says it waits (B632).
        PromptNotifier.shared.asked(.hostKey(connection: connection.id, host: host,
                                             changed: previous != nil || conflicting))
        viewModel.hostKeyPrompt = TerminalViewModel.HostKeyPrompt(
            host: host, port: port, fingerprint: fingerprint, previousFingerprint: previous,
            olderAlgorithm: older, conflicting: conflicting, connectedBefore: connectedBefore
        ) { [weak self] trusted in
            guard let self else { return }
            // B631: only the prompt still up is cleared, and only its no is
            // the person's refusal. A late answer — a sheet on its way out
            // after the core withdrew its prompt and asked the next — wiped
            // the next one, and the dial waited on a card nobody showed.
            // The core takes the answer only while that prompt is asked.
            if self.hostKeyPrompt == prompt {
                self.viewModel.hostKeyPrompt = nil
                self.hostKeyPrompt = nil
                PromptNotifier.shared.answered(PromptNotice.id(self.connection.id, .hostKey))
                if !trusted {
                    self.refusedHostKey = true
                }
            }
            if let id = self.id {
                self.core.sessions.answerHostKey(session: id, prompt: prompt, trust: trusted)
            }
        }
    }

    private func presentQuestion(prompt: UInt64, host: String, words: String, echo: Bool) {
        signInQuestion = prompt
        PromptNotifier.shared.asked(.question(connection: connection.id, host: host))
        viewModel.signInQuestion = TerminalViewModel.SignInQuestion(
            host: host, words: words, echo: echo
        ) { [weak self] answer in
            guard let self else { return }
            // B631: a late answer clears nothing but its own question.
            if self.signInQuestion == prompt {
                self.viewModel.signInQuestion = nil
                self.signInQuestion = nil
                PromptNotifier.shared.answered(PromptNotice.id(self.connection.id, .question))
            }
            if let id = self.id {
                self.core.sessions.answerQuestion(session: id, prompt: prompt, answer: answer)
            }
        }
    }

    /// What the server said with its question, shown as it was sent: its
    /// non-empty parts on lines of their own, without terminal escapes or
    /// other control characters (noise on a card), bounded.
    static func serverWords(name: String, instructions: String, prompt: String) -> String {
        let parts = [name, instructions, prompt].map { part in
            let plain = part.replacingOccurrences(of: terminalEscape, with: "",
                                                  options: .regularExpression)
            return String(String.UnicodeScalarView(plain.unicodeScalars.filter {
                $0 == "\n" || !CharacterSet.controlCharacters.contains($0)
            })).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return String(parts.filter { !$0.isEmpty }.joined(separator: "\n").prefix(600))
    }

    /// A CSI sequence (colours, cursor moves) or an OSC one (a title).
    private static let terminalEscape =
        "\u{1B}\\[[0-?]*[ -/]*[@-~]|\u{1B}\\][^\u{07}\u{1B}]*(\u{07}|\u{1B}\\\\)?"

    /// The passphrase of the connection's key, from the person (B631):
    ///
    /// - A read that asks while the card is up waits on that card. A second
    ///   card replaced the first, and the read that raised the first waited
    ///   forever (the session's dial and a side connection both read the
    ///   key).
    /// - The card is answered once. A second answer — Cancel racing
    ///   Unlock, `stop()` racing either — resumed the read again, and a
    ///   continuation resumed twice traps.
    /// - A late answer from a card already answered touches the next one
    ///   not at all.
    private func askPassphrase(wrong: Bool) async -> String? {
        await withCheckedContinuation { continuation in
            passphraseWaiters.append(continuation)
            guard viewModel.keyPassphrasePrompt == nil else { return }
            passphraseCard += 1
            let card = passphraseCard
            PromptNotifier.shared.asked(.passphrase(connection: connection.id, host: session.connection.host))
            viewModel.keyPassphrasePrompt = TerminalViewModel.KeyPassphrasePrompt(
                host: session.connection.host, wrong: wrong
            ) { [weak self] passphrase in
                guard let self, self.passphraseCard == card, !self.passphraseWaiters.isEmpty else { return }
                let waiters = self.passphraseWaiters
                self.passphraseWaiters = []
                self.viewModel.keyPassphrasePrompt = nil
                PromptNotifier.shared.answered(PromptNotice.id(self.connection.id, .passphrase))
                for waiter in waiters {
                    waiter.resume(returning: passphrase)
                }
            }
        }
    }

    /// How many reads wait on the passphrase card (tests).
    var passphraseWaiting: Int { passphraseWaiters.count }

    // MARK: Waiting

    private func settled() async {
        switch state {
        case .connected, .failed, .ended: return
        default: break
        }
        await withCheckedContinuation { settleWaiters.append($0) }
    }

    private func settle() {
        let waiters = settleWaiters
        settleWaiters = []
        for waiter in waiters { waiter.resume() }
    }

    private func waitClosed(atMost limit: Duration) async {
        if closed { return }
        await withCheckedContinuation { continuation in
            closeWaiters.append(continuation)
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: limit)
                guard let self, !self.closed else { return }
                self.closed = true
                let waiters = self.closeWaiters
                self.closeWaiters = []
                for waiter in waiters { waiter.resume() }
            }
        }
    }
}
