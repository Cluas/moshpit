import Foundation
import Observation
import UIKit
import MoshpitKit

/// Run `op`, returning its value, or nil if it throws or doesn't finish within
/// `seconds`. Used for liveness probes where a dead half-open socket can hang.
///
/// The contract that matters: **this function RETURNS at the deadline no
/// matter what `op` does.** The previous implementation raced inside a
/// `withTaskGroup`, whose scope waits for every child — `cancelAll()` only
/// REQUESTS cancellation, and an op that never checks it (an SSH exec bridged
/// off a NIO future, awaiting a reply that a half-open socket will never
/// deliver) kept the whole group, and therefore this function, hanging. That
/// single hang wedged `isResuming` forever, which then gate-blocked every
/// keepalive, sidecar rebuild, reconnect, and protocol switch for the rest of
/// the session — the "control plane never comes back after a long
/// background" family of reports, on the exact builds that were supposed to
/// fix it. Unstructured race instead: the deadline arm resumes the caller
/// unconditionally; a straggler op task is cancelled best-effort and, if it
/// ignores that, LEAKS as a parked task that dies with its dead connection —
/// harmless, and infinitely better than parking the caller.
func withTimeoutValue<T: Sendable>(_ seconds: Double,
                                   _ op: @Sendable @escaping () async throws -> T) async -> T? {
    await withCheckedContinuation { (continuation: CheckedContinuation<T?, Never>) in
        let once = OnceContinuation<T?>(continuation)
        let work = Task { once.resume(try? await op()) }
        Task {
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            once.resume(nil)
            work.cancel()
        }
    }
}

/// First resume wins, the loser is dropped — the once-guard that lets the
/// deadline arm and the (possibly cancellation-ignoring) op arm both try to
/// answer without a double-resume crash.
private final class OnceContinuation<V>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<V, Never>?
    init(_ continuation: CheckedContinuation<V, Never>) { self.continuation = continuation }
    func resume(_ value: V) {
        lock.lock()
        let taken = continuation
        continuation = nil
        lock.unlock()
        taken?.resume(returning: value)
    }
}

/// A live-session degradation: the host lacked a dependency, so Moshpit dropped
/// to a working-but-lesser transport. Surfaced as a dismissible banner with an
/// "Install …" action that opens the Install Assist sheet.
struct DegradeNotice: Identifiable, Equatable {
    enum Missing: Equatable {
        case tmux           // multiplexer == .tmux but host has no tmux → plain pane
        case herdr          // multiplexer == .herdr but host has no herdr → plain pane
        case moshServer     // mosh protocol but host has no mosh-server → SSH
    }
    let id = UUID()
    let missing: Missing

    /// Packages Install Assist offers to install for this notice. We install
    /// both tmux and mosh together when a dependency is missing — the user
    /// almost always wants both, and a single command is less friction.
    var packages: [String] {
        switch missing {
        case .tmux:       return ["tmux"]
        case .herdr:      return ["herdr"]
        case .moshServer: return ["mosh"]
        }
    }

    static func == (lhs: DegradeNotice, rhs: DegradeNotice) -> Bool {
        lhs.missing == rhs.missing
    }
}

/// App-wide registry of live terminal sessions.
///
/// Sessions outlive navigation: the Home screen's expanded connection card
/// reads the tmux tree of a connected server, and re-entering the Terminal
/// screen re-attaches to the same `ActiveSession` instead of reconnecting.
@Observable
@MainActor
final class SessionHub {

    @Observable
    @MainActor
    final class ActiveSession: Identifiable {
        let connection: ServerConnection
        let viewModel: TerminalViewModel
        /// Set when the connection runs in tmux control mode.
        private(set) var tmuxController: RustTmuxController?

        /// The tmux controller a reconnect just retired, kept alive so the
        /// screen can keep showing its last frame while the replacement dials,
        /// attaches and paints — the frozen frame IS the reconnect's cover.
        /// Nil once the replacement reveals its first frame (or the core's
        /// fallback gives up) and on a full teardown. Only ever
        /// the oldest frame with content on it: a second drop before the
        /// first replacement painted keeps the ghost people were reading, not
        /// the veiled one that never showed anything.
        private(set) var retiredTmuxController: RustTmuxController?

        /// True from the start of a tmux attempt until the new controller
        /// reveals its first frame: the window in which the transport may
        /// already say `.live` while the pane has nothing worth uncovering.
        /// The screen keeps its cover (the ghost under the veil) up for exactly this
        /// long — measured on paint, not on a guessed grace period.
        private(set) var awaitingFirstFrame = false

        /// Single-pane data path (non-tmux).
        @ObservationIgnored let coordinator = CoreTerminalHost.Coordinator()
        /// The session on the Rust core, made when it first starts
        /// (``startOnRustCore``): the transport methods below hand over to it.
        @ObservationIgnored var rust: RustSessionDriver?
        /// tmux -CC control plane for mosh+tmux, on a SEPARATE lightweight
        /// SSH connection. mosh renders the interactive tmux TUI (roaming,
        /// low latency); SSH — which CAN carry -CC's line-framed control
        /// stream — feeds the same tmux daemon's state to the native
        /// breadcrumb + sheets with live push (%window-add, %layout-change…).
        private(set) var moshControl: RustTmuxController?

        /// herdr's surface on the Rust core (``RustHerdrController``): its
        /// tree, agents and skew where the chrome reads herdr, its actions
        /// intents to the core. Set by the core's session (``rustSetHerdr(_:)``).
        private(set) var herdrControl: RustHerdrController?

        /// The active tmux control surface, whichever transport is in use.
        var tmuxControl: RustTmuxController? { tmuxController ?? moshControl }

        /// Fired every time a multiplexer control plane is (re)wired — the
        /// initial attach, mosh's late detached-task wiring, and the REBUILD a
        /// mid-session reconnect performs.
        ///
        /// That last one is why this exists. `awaitMultiplexerControl` answers
        /// once, and `AgentActivityMonitor` held the controller it got weakly;
        /// a reconnect (`stop()` + `start()`) deallocates that instance and
        /// builds a new one, so the monitor's ref went nil and hook polling
        /// silently stopped — no new attention notifications after any
        /// reconnect, for as long as the terminal screen stayed up. Found by
        /// the real-device harness's positive control, not by any unit test:
        /// the zero it produces is indistinguishable from the zero the
        /// episode-suppression fix produces, until you ask a NEW question and
        /// nothing rings.
        var onMultiplexerControlChanged: ((any MultiplexerControlling) -> Void)?

        /// The multiplexer control plane, whenever it materializes — for
        /// callers that need "the control plane, once it exists" rather than
        /// "the control plane right now". Over mosh the core's tmux sidecar
        /// comes up on a second SSH connection after `start()` has returned
        /// (``rustSetMoshControl(_:)``), up to ~15s later — exactly the
        /// window a one-shot read samples nil in, which is how mosh+tmux
        /// shipped with the island never tracking (herdr's raced the same
        /// gap). Returns nil once the session stops/fails or after
        /// `timeout` — a degraded session with no multiplexer on the host has
        /// nothing to wait for.
        func awaitMultiplexerControl(timeout: TimeInterval = 60) async -> (any MultiplexerControlling)? {
            let deadline = Date().addingTimeInterval(timeout)
            while Date() < deadline {
                if isStopping { return nil }
                switch viewModel.status {
                case .disconnected, .failed, .ended: return nil
                default: break
                }
                if let control = tmuxControl { return control }
                if let herdr = herdrControl { return herdr }
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
            return nil
        }

        /// The connection file transfer rides: the session's own, lent call
        /// by call and never waited for (``RustSideChannel``); nil before the
        /// session is on the core.
        var fileTransferSSH: (any HostConnection)? {
            rust?.side.immediate
        }

        /// The retention sweep for `~/.moshpit/uploads/`. `-mtime +N` is
        /// "strictly older than N days", matching the setting's promise.
        /// Static and pure so the exact line is pinned by a unit test —
        /// a quoting slip in a `find … -delete` is not a bug to discover
        /// on someone's server.
        nonisolated static func uploadCleanupCommand(days: Int) -> String {
            "find \"$HOME/.moshpit/uploads\" -type f -mtime +\(days) -delete 2>/dev/null || true"
        }

        /// The connection an upload rides: the session's own, once it answers
        /// — a reconnect under way waited for, a session that isn't connected
        /// said so (``RustSideChannel``). Nothing is dialled (W14b).
        func acquireFileTransferSSH() async throws -> any HostConnection {
            guard let rust else { throw HostConnectionError.notConnected }
            return try await rust.side.ready()
        }

        /// Numbered log of images uploaded this session (#1, #2, …). Lives on
        /// the session — not the screen — so navigating Home and back keeps
        /// every number addressable from the image chip's long-press menu.
        var imageUploads = ImageUploadLog()

        // MARK: Host paths → preview on the phone

        /// What a tapped host path opened (download, QuickLook, folder) —
        /// presented by the terminal screen, cleared when it's dismissed.
        var remotePreview: RemotePreviewModel?

        /// The working directory the plain terminal's shell last reported
        /// over OSC 7, for shells that do (most don't; tmux and herdr panes
        /// are asked directly instead).
        @ObservationIgnored private var reportedDirectory: String?

        /// The file-transfer channel, packaged for the preview.
        var remoteFileAccess: RemoteFileAccess {
            RemoteFileAccess(scope: connection.id.uuidString) { [weak self] in
                guard let self else { throw SSHError.sessionClosed }
                return try await self.acquireFileTransferSSH()
            }
        }

        /// Open whatever a tapped path link names on the host.
        func openRemotePath(_ path: String) {
            remotePreview?.cancel()
            let model = RemotePreviewModel(path: path, files: remoteFileAccess)
            remotePreview = model
            model.start()
        }

        /// The shortcut bar's Files key: the folder sheet at the focused
        /// pane's working directory, or the login directory when nothing
        /// reports one (a plain shell without OSC 7).
        func openWorkingDirectory() {
            remotePreview?.cancel()
            let files = remoteFileAccess
            let model = RemotePreviewModel(files: files) { [weak self] in
                if let directory = await self?.focusedWorkingDirectory() { return directory }
                return try await files.channel().remoteHomeDirectory()
            }
            remotePreview = model
            model.start()
        }

        /// Where the pane on screen runs: tmux is asked about its active
        /// pane, everything else goes through `plainWorkingDirectory`.
        private func focusedWorkingDirectory() async -> String? {
            if let tmuxController {
                guard let pane = tmuxController.snapshot.activePaneId
                        ?? tmuxController.snapshot.activePanes.first?.id else { return nil }
                return await tmuxController.panePath(pane)
            }
            return await plainWorkingDirectory()
        }

        /// Ask the host which of `batch` exist (`PathProbe`). Rides the
        /// channel that's already up without `acquireFileTransferSSH`'s
        /// liveness ping — this runs whenever new paths scroll by — and only
        /// dials when that fails. nil = no answer (not "none exist").
        func probeRemotePaths(cwd: String?,
                              batch: [TerminalPathScanner.Candidate]) async -> [Int: PathProbe.Hit]? {
            let command = PathProbe.command(cwd: cwd, candidates: batch)
            func ask(_ ssh: any HostConnection) async -> [Int: PathProbe.Hit]? {
                guard let output = await withTimeoutValue(8, { try await ssh.executeCommand(command) })
                else { return nil }
                return PathProbe.parse(String(decoding: output, as: UTF8.self), count: batch.count)
            }
            if let ssh = fileTransferSSH, let hits = await ask(ssh) { return hits }
            guard let ssh = try? await acquireFileTransferSSH() else { return nil }
            return await ask(ssh)
        }

        private func makePathLinks(workingDirectory: @escaping () async -> String?) -> RemotePathLinks {
            RemotePathLinks(workingDirectory: workingDirectory) { [weak self] cwd, batch in
                await self?.probeRemotePaths(cwd: cwd, batch: batch)
            }
        }

        /// The plain terminal's working directory: the pane mosh is showing
        /// (asked of the sidecar), herdr's focused pane, or OSC 7.
        private func plainWorkingDirectory() async -> String? {
            if let moshControl, let pane = moshControl.snapshot.activePaneId {
                return await moshControl.panePath(pane)
            }
            if let herdrControl, let pane = herdrControl.snapshot.activePaneId {
                return herdrControl.workingDirectory(ofPane: pane)
            }
            return reportedDirectory
        }

        /// Every multiplexer control plane passes through here as it's wired
        /// (attach, reconnect rebuild, mosh's late sidecar), so each tmux
        /// controller's panes get path links however it came to be.
        private func controlPlaneWired(_ control: any MultiplexerControlling) {
            if let tmux = control as? RustTmuxController, tmux === tmuxController {
                tmux.onOpenRemotePath = { [weak self] path in self?.openRemotePath(path) }
                tmux.pathLinksFactory = { [weak self, weak tmux] paneId in
                    RemotePathLinks(workingDirectory: { await tmux?.panePath(paneId) }) { [weak self] cwd, batch in
                        await self?.probeRemotePaths(cwd: cwd, batch: batch)
                    }
                }
            }
            coordinator.pathLinks?.invalidateWorkingDirectory()
            onMultiplexerControlChanged?(control)
        }

        nonisolated var id: UUID { connection.id }

        /// True while the user is intentionally tearing this session down, so a
        /// concurrent keepalive / death callback doesn't fight it by reconnecting.
        @ObservationIgnored var isStopping = false

        /// Called on the main actor with (srttMs, roaming) from the mosh
        /// transport so the hub can mirror it into `SessionMetrics`.
        @ObservationIgnored var metricsSink: ((Double, Bool) -> Void)?

        /// Called on the main actor with live protocol counters from the mosh
        /// transport, mirrored into `SessionMetrics.moshDiagnostics` for the
        /// terminal pill's long-press diagnostic overlay.
        @ObservationIgnored var diagnosticsSink: ((MoshDiagnostics) -> Void)?

        /// Called on the main actor when this session is finished for good
        /// (``retireDeclined()``): the hub retires it the way Disconnect does,
        /// so neither the keepalive nor a foreground resume redials it.
        @ObservationIgnored var endedSink: (() -> Void)?

        /// Capabilities probed over the first SSH channel (tmux / mosh-server
        /// presence, OS, package manager). `nil` until the probe completes —
        /// treated optimistically (full feature) in the meantime.
        private(set) var capabilities: HostCapabilitiesValue?

        /// Set when the session ran at less than the user's chosen transport
        /// because the host lacked a dependency (no tmux / no mosh-server).
        /// Drives the dismissible host banner; cleared by the user or by a
        /// successful re-probe after they install the missing tool.
        var degrade: DegradeNotice?

        /// Set when the core says a tmux control-mode attach didn't confirm
        /// in time (its `attach_stalled` event): no `%session-changed` ever
        /// arrived. Drives a dismissible "attach didn't complete" notice on
        /// the home card (Retry / dismiss) instead of leaving it on
        /// "Attaching session…" forever. Cleared the moment the session is
        /// live, so a slow-but-successful attach never shows a false error.
        var attachStalled = false

        /// mosh + tmux: the server had no session for the control plane to
        /// land on (asked, not guessed: `has-session` over the sidecar), so
        /// the person makes the first one (B228). Reset at every attach. On
        /// the Rust core, the core's own answer for its tmux, whichever the
        /// transport (`TmuxReadModel.noSession`, B337).
        var moshServerHadNoSession = false

        /// mosh + tmux with no session on the server and none attached yet:
        /// not an attach that stalled, but one waiting for the person's
        /// "New session" (B228). Home shows the empty SESSIONS list with its
        /// +, not "Attaching…" and then the stalled notice; the terminal
        /// shows the empty state, not the bare shell.
        var awaitingFirstSession: Bool {
            moshServerHadNoSession && moshControl?.snapshot.everAttached != true
        }

        /// Set when a mosh session's UDP return path is dead — the socket went
        /// `.ready` and we kept flushing, but the server's replies never
        /// arrived (commonly a VPN/proxy or firewall that passes outbound UDP
        /// but drops the inbound datagrams). The terminal would otherwise sit
        /// on a live cursor over a permanently black screen with no
        /// explanation; this drives a dismissible "Mosh isn't receiving data"
        /// banner that offers a one-tap switch to SSH (which rides TCP and
        /// works through the same proxy). Fires at most once per transport.
        var moshReturnPathDead = false

        init(connection: ServerConnection) {
            self.connection = connection
            self.viewModel = TerminalViewModel(connection: connection)
            coordinator.pathLinks = makePathLinks { [weak self] in await self?.plainWorkingDirectory() }
            coordinator.onOpenRemotePath = { [weak self] path in
                Task { @MainActor [weak self] in self?.openRemotePath(path) }
            }
            // The terminal's own Paste and ⌘V take the same road as every
            // other paste. Called synchronously (SwiftTerm is on the main
            // thread): a hop would let a Return typed right after the paste
            // overtake it.
            coordinator.onPaste = { [weak self] text in self?.sendPaste(text) }
            coordinator.onHostDirectoryChange = { [weak self] directory in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    self.reportedDirectory = directory.flatMap(RemotePathLink.path(from:))
                    self.coordinator.pathLinks?.invalidateWorkingDirectory()
                }
            }
        }

        /// Rough on-screen grid for a font size, used to seed the terminal
        /// before the view lays out — so the first size handed to tmux / the
        /// PTY is phone-sized, not 80×24. Portrait width drives columns (the
        /// dimension that controls line wrapping); rows are approximate.
        @MainActor
        static func estimateGrid(fontSize: Double) -> (cols: Int, rows: Int) {
            let b = UIScreen.main.bounds.size
            let charW = max(fontSize * 0.6, 1)
            let charH = max(fontSize * 1.2, 1)
            let cols = max(20, Int((min(b.width, b.height) - 8) / charW))
            let rows = max(10, Int((max(b.width, b.height) * 0.6) / charH))
            return (cols, rows)
        }

        // MARK: herdr's banner (the Rust core's notice, worded)

        /// What herdr's side says about the pane on screen — another device
        /// has it, the server refused it, input that couldn't be sent — and
        /// the input-lost notice (``rustNoteInputLost()``).
        private(set) var herdrNotice: String?
        /// Label for the notice's one-tap remedy, when the notice has one
        /// (restart the server, "Open here", retry). nil = plain notice.
        private(set) var herdrNoticeAction: String?
        /// The input-lost notice waits for the session (``rustNoteInputLost()``).
        @ObservationIgnored private var inputLostWaiting = false

        /// The terminal backing whichever pane is on screen right now — tmux
        /// resolves the zoomed active pane's; mosh/plain SSH have just the one
        /// coordinator's.
        private var activeTerminal: CoreTerminalView? {
            if let controller = tmuxController,
               let paneId = controller.snapshot.activePaneId
                    ?? controller.snapshot.activePanes.first?.id {
                return controller.surface(for: paneId)
            }
            return coordinator.surface
        }

        /// Whether the remote program has put the terminal in application
        /// cursor-key mode (DECCKM, `ESC[?1h`). xterm's terminfo bundles this
        /// with the `smkx`/`rmkx` keypad-transmit sequences that shells send on
        /// entering/leaving interactive line editing — so zsh/bash have it on
        /// whenever they're reading a command line, not just inside curses
        /// apps. The D-pad and arrow shortcuts must match this bit or the
        /// shell's terminfo-driven `bindkey` for that key never fires (e.g.
        /// zsh's history-prefix-search widget is bound to `kcuu1`, which for
        /// `xterm-256color` IS the application-mode sequence, not `ESC[A`).
        ///
        /// Only plain SSH and mosh actually go by it. tmux and herdr re-encode
        /// a lone arrow server-side against the pane's real mode (see
        /// crates/tmux `send_input`; herdr on the Rust core),
        /// because there this local flag goes stale.
        var applicationCursorKeys: Bool {
            activeTerminal?.modes.applicationCursorKeys ?? false
        }

        /// Paste-aware send: wraps the clipboard text in bracketed-paste markers
        /// when the active terminal's app requested them (DECSET 2004 — Claude
        /// Code, vim, modern shells). Raw bytes executed a multi-line prompt
        /// line by line; bracketed, the app receives it as one paste block.
        /// Bracketing also decides whether Claude Code images a pasted path
        /// (`[Image #N]`) or reads it as hand-typed text.
        ///
        /// SSH+tmux routes through the controller's own `paste-buffer -p`
        /// instead of the `activeTerminal?.bracketedPasteMode` check below:
        /// that flag is only ever set by OBSERVING the pane's `ESC[?2004h`
        /// go by, and tmux does not replay mode-setting sequences on a fresh
        /// `-CC` attach (lab-verified, `scripts/spikes/tmux-cc-lab`, 2026-08-19) — so
        /// after a fresh attach it is permanently false, and every pasted
        /// image path landed as literal text instead of `[Image #N]`. Mosh
        /// does not have this problem (its renderer is a regular tmux
        /// client, not a fresh `-CC` attach each time), so that path is
        /// untouched. Gated on `connState == .live` for the
        /// same reason `sendInput` is — a paste triggered while the cover
        /// overlay is up (e.g. a queued Home-card action firing
        /// mid-reconnect) must not become durable input on a transport that
        /// queues it (mosh type-ahead).
        func sendPaste(_ text: String) {
            rust?.sendPaste(text)
        }

        /// Single entry point for user input (keyboard, shortcut bar, paste).
        /// Routes to the correct transport so callers don't have to know
        /// whether this session is tmux / mosh / plain SSH.
        func sendInput(_ data: Data) {
            routeInput(data)
        }

        /// The shortcut bar's keys: typing, except under a reconnect's veil,
        /// where the Rust core doesn't keep them for later as it does the
        /// keyboard's (B476).
        func sendShortcutInput(_ data: Data) {
            guard !data.isEmpty else { return }
            rust?.sendShortcut(data)
        }

        /// ``sendInput(_:)``'s routing, saying what became of the bytes: true
        /// when they were handed to a transport, false when they were
        /// dropped (nothing live, no pane to take them).
        /// The callers that keep what they deliver until it is delivered
        /// (``deliverInput(_:toPane:)``) need to know which (B172).
        @discardableResult
        private func routeInput(_ data: Data) -> Bool {
            guard !data.isEmpty, let rust else { return false }
            rust.sendInput(data)
            return true
        }

        /// Paste text into one SPECIFIC pane — bracketed like `sendPaste`,
        /// but judged by the TARGET pane's mode rather than the active one's.
        /// The Home card's per-agent image entry delivers here without ever
        /// opening the terminal screen.
        ///
        /// tmux-controlled sessions (SSH+tmux via `tmuxController`, mosh+tmux
        /// via the `moshControl` sidecar) go through the controller's own
        /// `paste-buffer -p` — the same server-side-truth fix as
        /// ``sendPaste(_:)`` — rather than guessing from a local terminal
        /// that, for a pane other than the active one, usually doesn't even
        /// exist to guess from.
        ///
        /// Both say what really happened — true only once the bytes reached
        /// something that takes them: tmux's own answer to the paste, the SSH
        /// write, the live mosh transport. They used to say true whatever
        /// happened, and the share queue deletes an entry it is told was
        /// delivered: a paste into a tmux controller still attaching (dropped
        /// before the attach), into a pane that is gone, or into a session no
        /// longer live lost the shared image for good (B172).
        func deliverPaste(_ text: String, toPane paneId: String) async -> Bool {
            guard !text.isEmpty, let rust else { return false }
            return await rust.deliverPaste(text, toPane: paneId)
        }

        func deliverInput(_ data: Data, toPane paneId: String) async -> Bool {
            guard !data.isEmpty, let rust else { return false }
            return await rust.deliverInput(data, toPane: paneId)
        }

        /// Scroll the visible terminal's scrollback (driven by the bar's scroll
        /// thumb / the swipe). tmux sessions page tmux's real scrollback via
        /// the control channel; a plain shell pages the local buffer.
        /// Positive = older output, negative = newer.
        func scrollActiveTerminal(lines: Int) {
            rust?.scroll(lines: lines)
        }

        /// Runs the session on the Rust core (``startOnRustCore``),
        /// which dials, signs in and redials on its own.
        /// - Parameter automatic: true when nobody asked for this — the transport
        ///   dropped or the app woke to a dead socket and we're rebuilding it
        ///   behind their back. Those failures stay inside the connecting screen
        ///   instead of raising a modal; see `TerminalViewModel`'s
        ///   `isAutoReconnectInFlight`.
        func start(theme: TerminalTheme, fontSize: Double, fontName: String = "system",
                   cursorShape: CursorShape = .block, cursorColorId: String = "teal",
                   cursorBlink: Bool = true, automatic: Bool = false) async {
            if await startOnRustCore(theme: theme, fontSize: fontSize, fontName: fontName,
                                     cursorShape: cursorShape, cursorColorId: cursorColorId,
                                     cursorBlink: cursorBlink, automatic: automatic) { return }
            // No Rust core in this build (its library isn't the one the app
            // was compiled against): nothing else connects.
            Log.ssh.fault("session \(self.connection.id.uuidString, privacy: .public): no Rust core in this build")
        }

        /// The herdr banner button's tap: the Rust core runs its remedy
        /// (restart the server, "Open here", retry).
        func runHerdrNoticeAction() {
            rust?.runHerdrNoticeAction()
        }

        /// Probe the host again over the session's own connection (W14b),
        /// for Install Assist's "Re-check" after the person ran the install
        /// command: the core's probe and its reading of the answer. Returns
        /// the fresh capabilities, or nil (→ "reconnect to apply") when the
        /// connection isn't up or the probe couldn't run. Not saved: the
        /// core's connect is the one writer of `hostcaps.<id>`, and probes
        /// again on the next connect.
        @discardableResult
        func recheckCapabilities() async -> HostCapabilitiesValue? {
            guard let rust, let host = try? await rust.side.ready(),
                  let output = try? await host.executeCommand(hostCapabilitiesProbeCommand()),
                  let text = String(data: output, encoding: .utf8) else { return nil }
            let fresh = hostCapabilitiesParse(output: text)
            capabilities = fresh
            return fresh
        }

        /// Create the FIRST tmux session on a server that has none, on the
        /// Rust core (B228): made detached on the session's own connection,
        /// and the waiting control client attaches it.
        func createFirstTmuxSession(named name: String? = nil) {
            rust?.createFirstTmuxSession()
        }

        /// The user said no to the server's key (B151): nothing to say about
        /// it, and nothing automatic may dial it again — a foreground resume
        /// redials a down session and would put the same card back up — so
        /// the hub takes it out as it does a Disconnect (`endedSink`).
        private func retireDeclined() {
            isStopping = true
            endedSink?()
        }

        /// Tear the session down: the Rust core closes it. `forReconnect` is
        /// the caller's (a protocol switch, a rebuilt session); the core
        /// decides what that means for its connection.
        func stop(forReconnect: Bool = false) async {
            await rust?.stop()
        }

    }

    private(set) var sessions: [UUID: ActiveSession] = [:]

    @ObservationIgnored private let metrics: SessionMetricsRegistry
    /// Foreground per `setForeground`: no pane is on screen while false
    /// (``PaneOnScreen``, B438).
    @ObservationIgnored private(set) var isForeground = false
    /// How `prepare` builds a session. Tests swap in their own.
    @ObservationIgnored var makeSession: @MainActor (ServerConnection) -> ActiveSession = {
        ActiveSession(connection: $0)
    }
    /// A session signed in, as the connection it started with: its managed
    /// key's last use (B601, ``SSHKeyStore/signedIn(with:)``).
    @ObservationIgnored var signedIn: @MainActor (ServerConnection) -> Void = { _ in }
    /// The sessions' unlock, network-path and keepalive rules are the Rust
    /// core's (``RustCore``): it hears the device's unlock and the path
    /// changes itself.
    init(metrics: SessionMetricsRegistry) {
        self.metrics = metrics
    }

    /// The session whose TerminalScreen is currently presented (nil on Home).
    /// Set/cleared by TerminalScreen; the foreground transition re-pins only
    /// this one — re-pinning windows nobody is looking at would shrink them
    /// for desktop clients for no benefit.
    weak var visibleSession: ActiveSession? {
        didSet {
            if oldValue !== visibleSession { RustCore.visibleSessionChanged(visibleSession) }
        }
    }

    func setForeground(_ active: Bool) {
        RustCore.current?.setForeground(active)
        isForeground = active
        if !active {
            // Drop every in-memory decrypted secret so a re-foreground forces
            // a fresh (biometry-gated) keychain read — otherwise the cache
            // that exists purely to skip repeated Face ID prompts during
            // foreground auto-reconnects would silently downgrade the
            // keychain's "authenticate on every read" (.userPresence) to
            // "once per app session", with plaintext lingering in process
            // memory across backgrounding. Live sessions are unaffected: this
            // only forces the NEXT resolveSecret() (e.g. on resume/reconnect)
            // back through the keychain. (The window pins a phone holds are
            // the core's to hand back: it hears the visibility itself.)
            Task { await SSHService.shared.clearAllCachedSecrets() }
        }
    }

    func session(for connection: ServerConnection) -> ActiveSession? {
        sessions[connection.id]
    }

    /// Synchronously create + register the session WITHOUT starting the
    /// transport. The caller must bind UI to the returned session BEFORE
    /// awaiting ``start(_:theme:fontSize:)`` — the host-key TOFU prompt
    /// fires DURING the SSH handshake via `viewModel.hostKeyPrompt`, and if
    /// nothing is observing it yet the handshake deadlocks (prompt can never
    /// be answered) and dies with a channel error.
    func prepare(_ connection: ServerConnection) -> ActiveSession {
        if let existing = sessions[connection.id] {
            // A session captures its config at init (`let connection`), so an
            // edit — e.g. switching the protocol to mosh — would silently keep
            // the old transport on reconnect. A LIVE session can't hot-swap
            // its transport; a dead one is rebuilt with the fresh config.
            let isDead: Bool
            switch existing.viewModel.status {
            case .idle, .failed, .disconnected, .ended: isDead = true
            case .connecting, .connected, .reconnecting: isDead = false
            }
            if !isDead || existing.connection == connection {
                // A dead session is about to start again: the error its last
                // attempt left (one nobody dismissed, a redial that failed
                // behind Home) is history, not this attempt's. Kept, Home's
                // card showed it the moment Connect was tapped, and its OK
                // disconnected the attempt just started (B513).
                if isDead { existing.viewModel.errorMessage = nil }
                return existing
            }
        }
        let active = makeSession(connection)
        sessions[connection.id] = active

        // Live SRTT + roaming from the mosh transport → SessionMetrics, so the
        // terminal's transport pill and the amber roam banner reflect reality.
        let id = connection.id
        active.metricsSink = { [weak self] srtt, roaming in
            guard let self else { return }
            var m = self.metrics.metrics[id] ?? SessionMetrics()
            m.srttMs = srtt > 0 ? srtt : m.srttMs
            m.isRoaming = roaming
            if roaming { m.state = .roaming } else if m.state == .roaming { m.state = .live }
            if srtt > 0 {
                m.srttHistory.append(srtt)
                if m.srttHistory.count > 24 { m.srttHistory.removeFirst(m.srttHistory.count - 24) }
            }
            self.metrics.metrics[id] = m
        }
        active.diagnosticsSink = { [weak self] diagnostics in
            guard let self else { return }
            var m = self.metrics.metrics[id] ?? SessionMetrics()
            m.moshDiagnostics = diagnostics
            self.metrics.metrics[id] = m
        }
        // The server ended the session (a mosh `exit`): out of the hub, as
        // Disconnect takes it, unless something newer took its place.
        active.endedSink = { [weak self, weak active] in
            guard let self, let active, self.sessions[id] === active else { return }
            Task { await self.disconnect(id) }
        }
        return active
    }

    /// Open the transport for a prepared session (idempotent — a session
    /// that is already past `.idle` is left untouched by ActiveSession.start).
    func start(_ active: ActiveSession, theme: TerminalTheme, fontSize: Double, fontName: String = "system",
               cursorShape: CursorShape = .block, cursorColorId: String = "teal",
               cursorBlink: Bool = true) async {
        await active.start(theme: theme, fontSize: fontSize, fontName: fontName,
                            cursorShape: cursorShape, cursorColorId: cursorColorId, cursorBlink: cursorBlink)

        var m = metrics.metrics[active.connection.id] ?? SessionMetrics()
        if case .connected = active.viewModel.status {
            m.state = .live
            m.connectedAt = Date()
            signedIn(active.connection)
        } else {
            m.state = .offline
        }
        // Reflect a mosh→SSH fallback on the home card's transport pill.
        m.moshDegraded = active.degrade?.missing == .moshServer
        metrics.metrics[active.connection.id] = m

        // Images the share extension queued for THIS connection can be
        // delivered now that it's live.
        if case .connected = active.viewModel.status {
            Task { await ShareQueueDrainer.drain(hub: self) }
        }
    }

    /// Connection `saved` was saved while its session runs (B329): on the
    /// Rust core its next connect uses the saved address, user, proxy and
    /// credential, not the ones it started with. The Swift path keeps the
    /// ones it started with until the person connects again (B329: Rust
    /// 已对，Swift 不修).
    func adopt(_ saved: ServerConnection) {
        sessions[saved.id]?.rust?.adopt(saved)
    }

    /// Hands `release` to connection `id`'s session on the Rust core, to run
    /// once it has connected again as the connection was saved or has
    /// ended (B329). False: no such session.
    func releaseAfterReconnect(_ id: UUID, _ release: @escaping () async -> Void) -> Bool {
        guard let rust = sessions[id]?.rust, !rust.hasClosed else { return false }
        rust.releaseAfterReconnect(release)
        return true
    }

    func disconnect(_ connectionId: UUID) async {
        guard let active = sessions[connectionId] else { return }
        active.isStopping = true   // suppress keepalive/auto-reconnect during teardown
        await active.stop()
        await SSHService.shared.clearCachedSecret(for: connectionId)
        sessions[connectionId] = nil
        var m = metrics.metrics[connectionId] ?? SessionMetrics()
        m.state = .saved
        m.connectedAt = nil
        m.isRoaming = false
        metrics.metrics[connectionId] = m
    }
}

// MARK: - Sessions on the Rust core (W14a)

extension SessionHub.ActiveSession {
    /// Runs this session on the Rust core, or redials the one it runs on.
    /// Whether it did: false only in a build whose Rust library isn't the
    /// one the app was compiled against (``RustCore/available``).
    func startOnRustCore(theme: TerminalTheme, fontSize: Double, fontName: String,
                                 cursorShape: CursorShape, cursorColorId: String,
                                 cursorBlink: Bool, automatic: Bool) async -> Bool {
        let appearance = RustTmuxController.Appearance(
            theme: theme, fontSize: fontSize, fontName: fontName, cursorShape: cursorShape,
            cursorColorId: cursorColorId, cursorBlink: cursorBlink)
        let driver: RustSessionDriver
        if let rust {
            driver = rust
        } else {
            guard RustCore.available else { return false }
            driver = RustSessionDriver(session: self, core: .shared, appearance: appearance)
            rust = driver
            Log.ssh.info("session \(self.connection.id.uuidString, privacy: .public): on the Rust core")
        }
        if !automatic { moshReturnPathDead = false }
        await driver.start(appearance: appearance, automatic: automatic)
        return true
    }

    /// The Rust session's tmux surface, as the chrome reads a controller.
    func rustSetTmux(_ controller: RustTmuxController?) {
        tmuxController = controller
    }

    /// The previous attach's surface, frozen on screen until the new one paints.
    func rustSetRetired(_ controller: RustTmuxController?) {
        retiredTmuxController = controller
    }

    func rustSetAwaitingFirstFrame(_ awaiting: Bool) {
        awaitingFirstFrame = awaiting
    }

    /// mosh + tmux on the core: the `-CC` sidecar's tree, where the chrome
    /// reads the mosh control plane.
    func rustSetMoshControl(_ controller: RustTmuxController?) {
        moshControl = controller
    }

    /// herdr on the core: its surface, where the chrome reads herdr.
    func rustSetHerdr(_ client: RustHerdrController?) {
        herdrControl = client
    }

    /// herdr's banner and its button, worded from the core's notice.
    func rustSetHerdrNotice(_ notice: String?, action: String?) {
        if herdrNotice != notice { herdrNotice = notice }
        if herdrNoticeAction != action { herdrNoticeAction = action }
    }

    /// How long the input-lost notice waits for the session to be live again
    /// (a reconnect can take minutes), and how long it then shows
    /// (``rustNoteInputLost()``).
    static var inputLostWait: Duration = .seconds(600)
    static var inputLostShown: Duration = .seconds(4)

    /// What was typed on a plain shell while the way back asked a connection
    /// that turned out dead (B396), or under a reconnect's veil (B322),
    /// wasn't sent: the redial is a new shell. Said once the session is live
    /// again, once however many keys it was, for a few seconds, in the
    /// banner's slot — never over a notice of herdr's that is showing (that
    /// one stays, and the log line says this) — then cleared, unless
    /// something else has taken the slot meanwhile. Not when the session
    /// gave up instead.
    func rustNoteInputLost() {
        guard !inputLostWaiting else { return }
        inputLostWaiting = true
        let notice = String(localized: "What you just typed wasn't sent: the connection dropped, and it's back now")
        let deadline = ContinuousClock.now + Self.inputLostWait
        Task { @MainActor [weak self] in
            defer { self?.inputLostWaiting = false }
            while self?.viewModel.connState != .live {
                guard let state = self?.viewModel.connState, state != .offline,
                      ContinuousClock.now < deadline else { return }
                try? await Task.sleep(for: .milliseconds(100))
            }
            guard let self, self.herdrNotice == nil else { return }
            self.herdrNotice = notice
            self.herdrNoticeAction = nil
            try? await Task.sleep(for: Self.inputLostShown)
            if self.herdrNotice == notice { self.herdrNotice = nil }
        }
    }

    /// A control plane on the core is wired as the Swift path's are: tmux's
    /// panes get their host-path links and taps (the probe, the pane's
    /// working directory and the preview on the session's own connection,
    /// W14b), and whoever waits for the control plane hears of it.
    func rustControlPlaneWired(_ control: any MultiplexerControlling) {
        controlPlaneWired(control)
    }

    /// The person said no to the server's key on the core's card: the hub
    /// takes the session out, as the Swift path's ``retireDeclined()``
    /// does (B151, B334).
    func rustRetireDeclined() {
        retireDeclined()
    }

    /// What the core's probe found about the host, published on the
    /// session (B336).
    func rustSetCapabilities(_ capabilities: HostCapabilitiesValue) {
        self.capabilities = capabilities
    }

    /// The mosh diagnostics' Repaint: mosh-server redraws the screen from
    /// blank (B338).
    func requestRepaint() {
        rust?.repaint()
    }
}
