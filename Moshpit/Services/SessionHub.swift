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

    /// The degrade notice for a chosen-but-absent multiplexer. `nil` for
    /// `.none`, which needs nothing installed and so can never degrade.
    ///
    /// Note there is deliberately no "fall back to the other multiplexer"
    /// path: tmux and herdr hold unrelated sessions, so quietly attaching the
    /// one that happens to be installed would show the user work that isn't
    /// the work they asked for.
    static func forMissing(_ multiplexer: Multiplexer) -> DegradeNotice? {
        switch multiplexer {
        case .none:  return nil
        case .tmux:  return DegradeNotice(missing: .tmux)
        case .herdr: return DegradeNotice(missing: .herdr)
        }
    }

    static func == (lhs: DegradeNotice, rhs: DegradeNotice) -> Bool {
        lhs.missing == rhs.missing
    }
}

/// How long a tmux control-mode attach is given to confirm before the home
/// card gives up waiting. `-CC attach` boots the boot line then waits for
/// tmux's `%session-changed` line — a real SSH round-trip, comfortably done
/// in well under this even on a slow host. Found via a real device on a
/// high-latency SSH host where a NORMAL attach took 10-20+ seconds with zero
/// progress feedback (reads as "stuck"), and a genuinely broken attach
/// (dropped connection, unresponsive host, a `%session-changed` line that
/// fails the tmux-id guard) hung on "Attaching session…" forever with no
/// error and no way out.
let tmuxAttachTimeoutSeconds: Double = 22

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
        private(set) var tmuxController: TmuxSessionController?

        /// The tmux controller a reconnect just retired, kept alive so the
        /// screen can keep showing its last frame while the replacement dials,
        /// attaches and paints — the frozen frame IS the reconnect's cover.
        /// Nil once the replacement reveals its first frame (or the fallback
        /// in `armFirstFrameWait` gives up) and on a full teardown. Only ever
        /// the oldest frame with content on it: a second drop before the
        /// first replacement painted keeps the ghost people were reading, not
        /// the veiled one that never showed anything.
        private(set) var retiredTmuxController: TmuxSessionController?

        /// True from the start of a tmux attempt until the new controller
        /// reveals its first frame: the window in which the transport may
        /// already say `.live` while the pane has nothing worth uncovering.
        /// The screen keeps its cover (ghost or poster) up for exactly this
        /// long — measured on paint, not on a guessed grace period.
        private(set) var awaitingFirstFrame = false
        @ObservationIgnored private var firstFrameFallback: Task<Void, Never>?

        /// Input typed while the session is live but where it goes isn't
        /// decided yet: an SSH session's PTY is open (and the session reads
        /// connected) from the moment the dial lands, while the capability
        /// probe, the multiplexer decision and tmux's attach still follow.
        /// Typed then, it went straight into the login shell — ahead of the
        /// `-CC attach` line, which then ran as `<keys>tmux -CC attach` and
        /// failed (B169). Held here instead, in order, and handed on once the
        /// decision is made (`releaseHeldInput`): to the shell of a plain
        /// session, to tmux's active pane once attached — never into the
        /// shell under a multiplexer. nil when nothing is being held.
        @ObservationIgnored private var heldInput: [Data]?
        /// How long the cover may wait for a revealed frame before giving up
        /// once tmux has ANSWERED — attached (a pane with nothing on it), or
        /// refused (a server with no sessions never paints; the empty-state
        /// card behind the cover settles at 4s). Measured from the attach when
        /// there is one, not from the boot line — see ``firstFrameWaitHolds``.
        @ObservationIgnored var firstFrameFallbackSeconds: Double = 6
        /// How long a link that has not answered the boot line at all may hold
        /// the cover before it is dropped anyway; aligned with the attach
        /// timeout that latches `attachStalled`, so both give up together.
        @ObservationIgnored var firstFrameAttachBoundSeconds: Double = tmuxAttachTimeoutSeconds
        /// How long a server that has answered must then stay quiet, still
        /// unattached, before the answer reads as a refusal. The pump stamps a
        /// chunk the moment it lands, ahead of the parser and the main actor,
        /// so an attach can be in flight behind a fresh stamp — a fallback
        /// poll landing in that gap took it for a refusal and dropped the
        /// ghost (public CI, 2026-09-16 and 2026-10-19).
        @ObservationIgnored var firstFrameAnswerSettleSeconds: Double = 0.5

        /// Window pins and pristine layouts a retired controller still owed
        /// the server, carried to its replacement so the debt is settled on
        /// the eventual real disconnect instead of being paid — over a socket
        /// already judged dead — right now.
        @ObservationIgnored private(set) var inheritedPinLedger: TmuxSessionController.PinLedger?
        /// The retired controller's agent stamps, handed to the replacement
        /// the same way (see ``TmuxSessionController/takeAgentHooks()``), so
        /// the breadcrumb and the Home tree keep their agents across the
        /// rebuild instead of losing and regaining them.
        @ObservationIgnored private(set) var inheritedAgentHooks: [String: AgentHook]?
        /// Single-pane data path (non-tmux).
        @ObservationIgnored let coordinator = SwiftTerminalView.Coordinator()
        /// Set when this session runs on the Rust core (``RustSessionSwitch``,
        /// decided when it starts): the transport methods below hand over to it.
        @ObservationIgnored var rust: RustSessionDriver?
        /// Set when the connection runs over the real mosh UDP transport.
        @ObservationIgnored private(set) var moshTransport: MoshTransport?
        /// tmux -CC control plane for mosh+tmux, on a SEPARATE lightweight
        /// SSH connection. mosh renders the interactive tmux TUI (roaming,
        /// low latency); SSH — which CAN carry -CC's line-framed control
        /// stream — feeds the same tmux daemon's state to the native
        /// breadcrumb + sheets with live push (%window-add, %layout-change…).
        private(set) var moshControl: TmuxSessionController?
        /// The SSH session under `moshControl`; owned here (the controller
        /// never closes its transport).
        @ObservationIgnored private var sidecarSSH: SSHSession?

        /// herdr's surface on the Rust core (``RustHerdrController``): its
        /// tree, agents and skew where the chrome reads herdr, its actions
        /// intents to the core. Set by the core's session (``rustSetHerdr(_:)``).
        private(set) var herdrControl: RustHerdrController?

        /// The active tmux control surface, whichever transport is in use.
        var tmuxControl: TmuxSessionController? { tmuxController ?? moshControl }

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
        /// "the control plane right now". Over SSH the controller is assigned
        /// inside `start()` and the first check resolves; over mosh the tmux
        /// control plane is wired by a DETACHED task (`attachMoshTmux`)
        /// that can spend ~15s on a second SSH handshake
        /// and attach retries after `start()` has returned — exactly the
        /// window a one-shot read samples nil in, which is how mosh+tmux
        /// shipped with the island never tracking (herdr's, on the Rust core,
        /// raced the same gap). Returns nil once the session stops/fails or after
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

        /// SSH channel available for file transfer — SFTP rides the already-
        /// authenticated connection, so no re-auth and no Face ID. The in-band
        /// SSH session when there is one; the mosh sidecar otherwise (same
        /// pick as `recheckCapabilities`). nil on pure mosh: `startMosh`
        /// closes the bootstrap SSH right after handoff, so there is nothing
        /// to ride — callers degrade honestly instead of dialling on their own.
        /// On the Rust core, the session's own connection, lent call by call
        /// and never waited for (``RustSideChannel``).
        var fileTransferSSH: (any HostConnection)? {
            if let rust = lendingRust { return rust.side.immediate }
            return viewModel.session ?? sidecarSSH
        }

        /// On-demand SSH dialled for file transfer when the session has no
        /// live SSH at all — pure mosh, where `startMosh` closes its
        /// bootstrap connection after handoff. Cached rather than
        /// closed-per-upload: an idle authenticated channel costs almost
        /// nothing, iOS suspends it with the app anyway, and the second
        /// upload gets it for free. Closed in `stop()`.
        @ObservationIgnored private var onDemandTransferSSH: SSHSession?

        /// The retention sweep for `~/.moshpit/uploads/`. `-mtime +N` is
        /// "strictly older than N days", matching the setting's promise.
        /// Static and pure so the exact line is pinned by a unit test —
        /// a quoting slip in a `find … -delete` is not a bug to discover
        /// on someone's server.
        nonisolated static func uploadCleanupCommand(days: Int) -> String {
            "find \"$HOME/.moshpit/uploads\" -type f -mtime +\(days) -delete 2>/dev/null || true"
        }

        /// The SSH channel an upload should ride, dialling one if the
        /// session has none. Credentials come from the in-memory secret
        /// cache, so the dial never re-prompts Face ID mid-flow.
        ///
        /// PROBED, not trusted: `isConnected` is a local `!closed` flag and
        /// lies after a suspension (iOS kills TCP without NIO seeing a
        /// close — the same lesson `isTransportAlive` documents). Trusting
        /// it handed uploads a half-open sidecar and failed them with an
        /// opaque SSHError; a retry minutes later worked because keepalive
        /// had rebuilt the sidecar by then ("时好时坏", user report,
        /// 2026-08-17). A dead channel either errors fast or times out, and
        /// the flow moves on to dialling a fresh one — the false-dead cost
        /// is one redundant dial, far cheaper than a failed upload.
        ///
        /// On the Rust core nothing is dialled (W14b): the session's own
        /// connection, once it answers — a reconnect under way waited for, a
        /// session that isn't connected said so (``RustSideChannel``).
        func acquireFileTransferSSH() async throws -> any HostConnection {
            if let rust = lendingRust { return try await rust.side.ready() }
            if let ssh = fileTransferSSH, await answersQuickly(ssh) { return ssh }
            if let cached = onDemandTransferSSH, await answersQuickly(cached) { return cached }
            let fresh = try await sidecarConnect(connection)
            onDemandTransferSSH = fresh
            return fresh
        }

        /// True if a trivial command round-trips within 4s. Shorter than
        /// `isTransportAlive`'s 8s: a false "dead" there tears down a whole
        /// session, here it only costs an extra dial.
        private func answersQuickly(_ ssh: any HostConnection) async -> Bool {
            await withTimeoutValue(4) { try await ssh.executeCommand("true") } != nil
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
            if let ssh = fileTransferSSH ?? onDemandTransferSSH, let hits = await ask(ssh) { return hits }
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
            if let tmux = control as? TmuxSessionController, tmux === tmuxController {
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

        @ObservationIgnored private var pumpTask: Task<Void, Never>?

        /// The mosh+tmux control-plane bootstrap (`attachMoshTmux`), which can
        /// take up to ~15s (a second SSH handshake, polling for the sidecar to
        /// attach, retrying the renderer's `tmux attach`, pinning the window
        /// size). Unlike `pumpTask` this used to be pure fire-and-forget —
        /// `stop()` had no handle on it, so switching protocol (or any
        /// teardown) while it was still mid-flight left it running against a
        /// dead session: it would go on to open its own SSH connection and
        /// assign `moshControl`/`sidecarSSH` on an ActiveSession `stop()`
        /// already tore down, leaking a connection and racing the NEW
        /// session's own bootstrap over the shared `SSHService.shared` host-key
        /// handler state — the "switch to mosh+tmux, screen never loads" bug.
        /// With no session on the server it lasts until there is one (B212).
        @ObservationIgnored private var moshTmuxAttachTask: Task<Void, Never>?
        /// The nonce of the tmux renderer mosh-server runs for this connect:
        /// its pid file (``tmuxRendererPidPath(connectionId:nonce:)``).
        @ObservationIgnored private var moshTmuxRendererNonce = ""
        /// Every window's layout before mosh-server started the tmux
        /// renderer (B270), for the first sidecar of the connect.
        @ObservationIgnored private var moshTmuxLayoutsBeforeRenderer: String?

        /// What the mosh+tmux attach waits on (``attachMoshTmux``): the
        /// window the control plane gets to land on a session, the renderer's
        /// attach checks, the wait for a first session. Tests move a virtual
        /// one by hand.
        @ObservationIgnored var moshAttachClock: any Clock<Duration> = ContinuousClock()

        /// How long a tmux control-mode attach may take before the card says
        /// it didn't complete (``attachStalled``). Tests shorten it.
        @ObservationIgnored var attachTimeoutSeconds: Double = tmuxAttachTimeoutSeconds

        // MARK: Lifecycle-test seams

        /// How side connections (mosh sidecars, the
        /// on-demand transfer connection) are dialed. Production goes through
        /// the shared service; lifecycle tests inject a factory yielding
        /// sessions built over a scripted `SSHClientTransport` — which is
        /// how "half-open socket after a long suspension" becomes a unit
        /// test instead of a field report.
        @ObservationIgnored var sidecarConnect:
            @Sendable (ServerConnection) async throws -> SSHSession =
            { try await SSHService.shared.connect($0) }

        /// How `startMosh` builds its transport from the bootstrap's
        /// credentials. Production opens the UDP flow they name; lifecycle
        /// tests hand back a transport over a fake datagram channel — and see
        /// where it was pointed.
        @ObservationIgnored var makeMoshTransport: (MoshCredentials) throws -> MoshTransport = {
            try MoshTransport(credentials: $0)
        }

        /// Test-only state installation: lifecycle scenarios need a session
        /// that BELIEVES it is a live mosh connection without running the
        /// network bootstrap. Production never calls this.
        ///
        /// `moshServerPid`/`moshHadFirstContact` default to nil/false — the
        /// same "no pid captured" state a non-mosh (or bootstrap-less) test
        /// session has today — so every EXISTING call site is unaffected:
        /// `stop()`'s fallback reap is gated on `moshServerPid != nil` and
        /// stays inert unless a test deliberately opts in.
        func installForTesting(moshTransport transport: MoshTransport?,
                               sidecarSSH ssh: SSHSession? = nil,
                               moshControl: TmuxSessionController? = nil,
                               tmuxController: TmuxSessionController? = nil,
                               moshServerPid pid: Int? = nil,
                               moshHadFirstContact hadContact: Bool = false,
                               armFirstFrame: Bool = false) {
            self.moshTransport = transport
            self.sidecarSSH = ssh
            if let moshControl { self.moshControl = moshControl }
            if let tmuxController {
                self.tmuxController = tmuxController
                if armFirstFrame { armFirstFrameWait(for: tmuxController) }
            }
            self.moshServerPid = pid
            self.moshHadFirstContact = hadContact
            if transport != nil { viewModel.markConnected() }
        }

        /// True while the user is intentionally tearing this session down, so a
        /// concurrent keepalive / death callback doesn't fight it by reconnecting.
        @ObservationIgnored var isStopping = false
        /// Guards against overlapping reconnect attempts (death callback +
        /// keepalive timer can both fire).
        @ObservationIgnored private var isReconnecting = false
        /// Guards against overlapping `resumeIfNeeded()` attempts. That path
        /// (mosh sidecar rebuild, or the plain-SSH tear-down+restart tail) has
        /// several `await` points with no single flag stopping a second entry
        /// — `setForeground(true)`'s `resumeAll` and the periodic keepalive
        /// tick both funnel into it, and either can fire while the other is
        /// still mid-flight. Without this, two overlapping mosh-sidecar
        /// rebuilds could each get partway through detaching the old control
        /// channel and re-establishing a new one before the first finishes,
        /// double-assigning `moshControl`/`sidecarSSH` and orphaning one SSH
        /// connection. Cross-checked against `isReconnecting` (and vice versa
        /// in `reconnect()`) since both ultimately do the same
        /// stop()-then-start() dance and must not run concurrently either.
        @ObservationIgnored private var isResuming = false
        /// The mosh sidecar's controller declared its `-CC` channel dead while
        /// a resume pass was already running (``moshSidecarWentSilent(_:)``):
        /// the next pass rebuilds it whatever its probe says.
        @ObservationIgnored private var moshSidecarSilent = false

        /// Appearance used at `start()`, kept for transparent reconnects — an
        /// automatic reconnect (triggered internally by `connectionDropped()`,
        /// not by `TerminalScreen` remounting) rebuilds the `TmuxSessionController`
        /// from scratch and only ever reapplies THESE remembered values, since
        /// nothing else re-invokes `TerminalScreen.applyAppearance()` for it.
        /// Previously only theme/fontSize were remembered — cursor shape/color/
        /// blink and font name silently reset to `configureAppearance`'s
        /// defaults on every reconnect (the "cursor style lost on reconnect" bug).
        @ObservationIgnored private var lastTheme: TerminalTheme = .githubDark
        @ObservationIgnored private var lastFontSize: Double = 13
        @ObservationIgnored private var lastFontName: String = "system"
        @ObservationIgnored private var lastCursorShape: CursorShape = .block
        @ObservationIgnored private var lastCursorColorId: String = "teal"
        @ObservationIgnored private var lastCursorBlink: Bool = true

        /// The session/window/pane the user was viewing, captured before a
        /// teardown so a reconnect lands back there instead of on tmux's
        /// most-recently-active (the "doesn't return to my pane" bug, worst with
        /// multiple sessions). Replayed via the new controller's pendingRestore.
        /// Persisted PER CONNECTION (not on this object): a protocol switch or
        /// app relaunch destroys the ActiveSession, and the memory must survive
        /// that teardown to be of any use on the next connect.
        private var lastSelection: TmuxSelection? {
            get { TmuxSelectionStore.load(connection.id) }
            set { TmuxSelectionStore.save(newValue, for: connection.id) }
        }

        /// Snapshot the active selection off whichever controller is live, before
        /// it's torn down. tmux ids survive a re-attach, so they replay cleanly.
        private func captureSelection() {
            guard let snap = tmuxControl?.snapshot, snap.isAttached,
                  let session = snap.activeSessionId else { return }
            lastSelection = TmuxSelection(session: session,
                                          window: snap.activeWindowId,
                                          pane: snap.activePaneId)
            Log.island.info("land: captured selection pane=\(snap.activePaneId ?? "-", privacy: .public) window=\(snap.activeWindowId ?? "-", privacy: .public)")
        }

        /// Returning Home leaves the connection alive (for background monitoring
        /// + fast resume) but stops rendering, so the tmux window-size pin we
        /// placed for the phone grid would strand a desktop client sharing those
        /// windows at the phone width. Hand the pins back here; `repinForeground()`
        /// re-applies them when the terminal returns. Covers SSH + the mosh sidecar.
        func releaseWindowPinsForBackground() {
            guard rust == nil else { return }   // the core does it (visibility)
            tmuxController?.releaseWindowPins()
            moshControl?.releaseWindowPins()
        }

        /// Wait for the commands ``releaseWindowPinsForBackground()`` queued to
        /// actually reach the server.
        ///
        /// Queueing them is not sending them — they ride the control-mode write
        /// chain and need a round trip. On the way to the background that gap is
        /// where iOS suspends us, and the pins stay applied: a desktop attaching
        /// later gets the phone grid, which is the bug the release exists to
        /// prevent. Awaited under a ``BackgroundAssertion``.
        func flushBackgroundTeardown() async {
            guard rust == nil else { return }
            await tmuxController?.flushPendingWrites()
            await moshControl?.flushPendingWrites()
        }

        /// Re-pin the active window to the phone grid when the terminal returns to
        /// the foreground after `releaseWindowPinsForBackground()`.
        func repinForeground() {
            guard rust == nil else { return }
            tmuxController?.repinActiveWindow()
            moshControl?.repinActiveWindow()
        }

        /// Called on the main actor with (srttMs, roaming) from the mosh
        /// transport so the hub can mirror it into `SessionMetrics`.
        @ObservationIgnored var metricsSink: ((Double, Bool) -> Void)?

        /// Called on the main actor with live protocol counters from the mosh
        /// transport, mirrored into `SessionMetrics.moshDiagnostics` for the
        /// terminal pill's long-press diagnostic overlay.
        @ObservationIgnored var diagnosticsSink: ((MoshDiagnostics) -> Void)?

        /// Called on the main actor when the remote end finished this session
        /// for good (see `moshSessionEnded`, and `ptyChannelEnded` for a plain
        /// SSH shell that exited): the hub retires it the way Disconnect does,
        /// so neither the keepalive nor a foreground resume redials it.
        @ObservationIgnored var endedSink: (() -> Void)?

        /// Capabilities probed over the first SSH channel (tmux / mosh-server
        /// presence, OS, package manager). `nil` until the probe completes —
        /// treated optimistically (full feature) in the meantime.
        private(set) var capabilities: HostCapabilities?

        /// Set when the session ran at less than the user's chosen transport
        /// because the host lacked a dependency (no tmux / no mosh-server).
        /// Drives the dismissible host banner; cleared by the user or by a
        /// successful re-probe after they install the missing tool.
        var degrade: DegradeNotice?

        /// Set when a tmux control-mode attach didn't confirm within
        /// `tmuxAttachTimeoutSeconds` — no `%session-changed` ever arrived.
        /// Drives a dismissible "attach didn't complete" notice on the home
        /// card (Retry / dismiss) instead of leaving it on "Attaching
        /// session…" forever. Cleared automatically the moment the attach
        /// actually succeeds (see `beginAttachTimeout`), so a slow-but-
        /// successful attach never shows a false-positive error.
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

        /// The remote mosh-server's PID for the CURRENT mosh bootstrap, from
        /// `MoshCredentials.serverPid` — set the moment `startMosh()` gets the
        /// creds back, which is also the earliest point a reap is possible.
        /// `onReturnPathDead` (see `startMosh()`) is the primary reaper, but
        /// its watchdog only fires a few seconds after bootstrap; a user who
        /// backs out or kills the app before then — including during the
        /// bootstrap's own remaining awaits (herdr/tmux renderer probes,
        /// `MoshTransport` construction) — races past it, and that
        /// mosh-server waits forever for a client that is never coming (7
        /// zombies on one host, 2026-08-19). `stop()` is the backstop, and
        /// the one that stops a server the user closed: see its use of this
        /// with `MoshServerReap.onTeardown`. nil once reaped (or never
        /// captured, or not a mosh session, or ended by the server itself) —
        /// never re-armed for an old bootstrap by a later one; `startMosh()`
        /// overwrites it.
        @ObservationIgnored private var moshServerPid: Int?

        /// True once this mosh session's transport has heard back from its
        /// server at least once (`MoshDiagnostics.datagramsReceived > 0` —
        /// the same signal `MoshTransport.sawAnyDatagram` gates its own
        /// watchdog on).
        ///
        /// What it decides (`MoshServerReap`): a server that never answered
        /// is reaped at any teardown, a reconnect included — it waits forever
        /// for a client that is never coming. One that did answer holds the
        /// user's shell, and is left alone by a silence (B165) and by a
        /// reconnect. But not by the user closing the session: it used to be
        /// left running then too, on the belief that mosh-server outlives a
        /// client so a later connect can resume it — this app keeps no session
        /// key and starts a new server on every connect, so nothing ever came
        /// back to it, and every disconnect and protocol switch left a
        /// mosh-server and its shell on the host (B164).
        @ObservationIgnored private var moshHadFirstContact = false

        /// Bounds the wait started by `beginAttachTimeout`. Stored (not
        /// fire-and-forget) so `stop()` can cancel it — a stalled watcher
        /// left running past teardown could otherwise fire `attachStalled`
        /// against a controller that's already gone, or race a fresh
        /// attach's own watcher. Same class of bug as `moshTmuxAttachTask`
        /// (see its doc comment).
        @ObservationIgnored private var attachTimeoutTask: Task<Void, Never>?

        init(connection: ServerConnection, dial: TerminalViewModel.Dialer? = nil) {
            self.connection = connection
            self.viewModel = TerminalViewModel(connection: connection, dial: dial)
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

        // Mosh+tmux scrollback: copy-mode driven on the MOSH client itself (the
        // rendering client), via the mosh keystroke channel — the -CC sidecar's
        // copy-mode does not repaint this separate client. `moshPrefix` is the
        // tmux prefix (looked up at runtime; C-b default) so `prefix [` enters
        // copy-mode; PageUp/PageDown page history; `q` exits.
        @ObservationIgnored private var moshInCopyMode = false
        @ObservationIgnored private var moshPrefix = Data([0x02])   // C-b
        @ObservationIgnored private var moshPrefixResolved = false
        @ObservationIgnored private var lastMoshPageAt: Date = .distantPast
        /// Orders mosh keystroke writes so rapid-fire input (type a word, then
        /// immediately tap an arrow key) can't reorder at the transport — see
        /// the matching `writeChain` in TmuxSessionController.enqueue for the
        /// same bug on the -CC control path (independently-spawned `Task`s
        /// have no FIFO guarantee relative to each other).
        @ObservationIgnored private let moshWriteLane = WriteLane()

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
        private var activeTerminal: TerminalSurface? {
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
        /// `TmuxSessionController.sendInput`; herdr on the Rust core),
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
            if let rust { rust.sendPaste(text); return }
            guard viewModel.connState == .live else { return }
            // Undecided where it goes: held with the typing around it (B169).
            guard inputDecided() else {
                routeInput(pastePayload(text))
                return
            }
            if let controller = tmuxController,
               let paneId = controller.snapshot.activePaneId
                    ?? controller.snapshot.activePanes.first?.id {
                controller.sendPaste(text, paneId: paneId)
                return
            }
            sendInput(pastePayload(text))
        }

        /// The bytes a paste goes out as when no tmux controller encodes it:
        /// mosh and plain SSH go by the local emulator, with any marker in
        /// the text taken out (``BracketedPaste/payload(_:bracketed:)``):
        /// wrapped as it was, a pasted `ESC[201~` ended the paste and the
        /// line after it ran.
        private func pastePayload(_ text: String) -> Data {
            let bracketed = activeTerminal?.modes.bracketedPaste == true
            return BracketedPaste.payload(text, bracketed: bracketed)
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
            if let rust { rust.sendShortcut(data); return }
            routeInput(data)
        }

        /// ``sendInput(_:)``'s routing, saying what became of the bytes: true
        /// when they were handed to a transport, false when they were
        /// dropped (nothing live, no pane to take them).
        /// The callers that keep what they deliver until it is delivered
        /// (``deliverInput(_:toPane:)``) need to know which (B172).
        @discardableResult
        private func routeInput(_ data: Data) -> Bool {
            guard !data.isEmpty else { return false }
            if let rust { rust.sendInput(data); return true }
            // No input unless live. While the connecting poster is up the pane
            // is invisible, so every touch that lands here is accidental — and
            // some transports make accidents durable: mosh QUEUES keystrokes
            // typed while disconnected and replays them after resume (its
            // type-ahead feature, useful only when you can SEE the terminal).
            // The reported shape: a stray brush of the shortcut bar's arrows
            // during a reconnect popped Claude Code's prompt history after the
            // session came back. The bar sits OUTSIDE the cover overlay, so it
            // must be gated here, at the single choke point, not in the UI.
            // (deliverInput/deliverPaste stay ungated on purpose — the lock
            // screen and share-queue paths do their own reconnect-first dance.)
            guard viewModel.connState == .live else { return false }
            // Live, but where typing goes is still being decided: held, in
            // the order typed (B169).
            guard inputDecided() else {
                heldInput?.append(data)
                return true
            }
            // (SSH+tmux scrollback is LOCAL now — copy-mode is invisible to a
            // -CC client, so nothing server-side needs leaving here. The
            // controller's sendInput releases the pane's local scroll-hold so
            // typing snaps the reader back to live. Mosh copy-mode is handled
            // on its own branch below.)
            if let controller = tmuxController {
                let paneId = controller.snapshot.activePaneId
                    ?? controller.snapshot.activePanes.first?.id
                guard let paneId else { return false }
                controller.sendInput(data, paneId: paneId)
                return true
            } else if let transport = moshTransport {
                // Leaving mosh copy-mode? Prepend `q` (cancel) on the SAME channel
                // so it's strictly ordered before the keystrokes — no cross-channel
                // race.
                let leavingCopyMode = moshInCopyMode
                let out = Self.moshInputKeys(data, inCopyMode: leavingCopyMode)
                moshInCopyMode = false
                // Chained, not a bare `Task { }` — independently-spawned tasks
                // have no ordering guarantee, so typing a word then quickly
                // tapping the D-pad could deliver the arrow bytes before the
                // last character or two (the "history search stops matching
                // what I typed" bug).
                sendOverMosh(transport, out, predict: !leavingCopyMode)
                return true
            } else {
                guard viewModel.session != nil else { return false }
                viewModel.send(data)
                return true
            }
        }

        /// A payload that is exactly one arrow-key sequence, as its key name
        /// (tmux's `send-keys`) — either DECCKM variant, since the sender's guess is
        /// exactly what can't be trusted here. Anything else (batched keys,
        /// other sequences, plain text) returns nil and flows unmodified.
        static func arrowKeyName(_ data: Data) -> String? {
            guard data.count == 3, data[data.startIndex] == 0x1B else { return nil }
            let second = data[data.startIndex + 1]
            guard second == UInt8(ascii: "[") || second == UInt8(ascii: "O") else { return nil }
            switch data[data.startIndex + 2] {
            case UInt8(ascii: "A"): return "up"
            case UInt8(ascii: "B"): return "down"
            case UInt8(ascii: "C"): return "right"
            case UInt8(ascii: "D"): return "left"
            default: return nil
            }
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
            guard !text.isEmpty else { return false }
            if let rust { return await rust.deliverPaste(text, toPane: paneId) }
            guard inputDecided() else { return false }
            if let control = tmuxControl {
                return await control.pasteConfirmed(text, paneId: paneId)
            }
            return await deliverInput(pastePayload(text), toPane: paneId)
        }

        func deliverInput(_ data: Data, toPane paneId: String) async -> Bool {
            guard !data.isEmpty else { return false }
            if let rust { return await rust.deliverInput(data, toPane: paneId) }
            // Where it would land isn't decided yet (and the shell under a
            // multiplexer is the wrong place): not now — its sender keeps it.
            guard inputDecided() else { return false }
            if connection.multiplexer == .tmux, tmuxController == nil {
                // A tmux pane's input over mosh is placed by the `-CC`
                // sidecar, by the pane's id. Before it was up — the first
                // second or two of every connect, and while it is rebuilt —
                // it went down the mosh shell, which is tmux's own client (or
                // the login shell before it attaches): it landed in tmux's
                // ACTIVE pane, whichever the desktop left focused, or ran as a
                // command line, and was said sent either way. It waits for the
                // sidecar now (``moshSidecarDeliveryWait``), as crates/session
                // does. A host with no tmux — and a plain SSH shell where tmux
                // was asked for — has no pane to place it in.
                guard moshTransport != nil, degrade?.missing != .tmux,
                      let control = await moshSidecar(knowing: paneId),
                      viewModel.connState == .live else { return false }
                control.sendInput(data, paneId: paneId)
                return true
            }
            if let control = tmuxControl {
                // Before the attach the controller drops what it is given,
                // and a pane it doesn't know has nowhere to take it.
                guard viewModel.connState == .live, control.snapshot.isAttached,
                      control.snapshot.panes[paneId] != nil else { return false }
                control.sendInput(data, paneId: paneId)
                return true
            }
            // herdr runs on the Rust core only (E1): nothing here can place
            // one of its panes' input. Not sent: its sender says so.
            if connection.multiplexer == .herdr { return false }
            // Plain SSH / mosh: one shell, one place the bytes can go — while
            // it is live. A plain SSH shell says whether the write went out.
            guard viewModel.connState == .live else { return false }
            if moshTransport == nil, tmuxController == nil,
               viewModel.session != nil {
                return await viewModel.deliver(data)
            }
            return routeInput(data)
        }

        /// How long a tmux pane's input over mosh waits for the `-CC` sidecar
        /// to be up and list its pane (``moshSidecar(knowing:)``).
        @ObservationIgnored var moshSidecarDeliveryWait: TimeInterval = 10

        /// The mosh+tmux sidecar once it is attached and lists `paneId`; nil
        /// when it lists its panes without this one (a pane that is gone),
        /// or isn't there within ``moshSidecarDeliveryWait``.
        private func moshSidecar(knowing paneId: String) async -> TmuxSessionController? {
            var found: TmuxSessionController?
            _ = await waitUntil(timeout: moshSidecarDeliveryWait) { [weak self] in
                guard let self, !isStopping, moshTransport != nil else { return true }
                guard let control = moshControl, control.snapshot.isAttached else { return false }
                if control.snapshot.panes[paneId] != nil {
                    found = control
                    return true
                }
                return !control.snapshot.panes.isEmpty
            }
            return found
        }

        /// Poll `condition` on the main actor until it holds or `timeout`
        /// passes. The things being awaited (focus echo, frame retarget) are
        /// themselves poll-driven, so there is nothing better than sampling to
        /// hook into — and 120ms keeps worst-case added latency invisible next
        /// to the SSH round-trip it rides on.
        private func waitUntil(timeout: TimeInterval,
                               condition: @MainActor () -> Bool) async -> Bool {
            let deadline = Date().addingTimeInterval(timeout)
            while !condition() {
                guard Date() < deadline else { return false }
                try? await Task.sleep(for: .milliseconds(120))
            }
            return true
        }

        /// Send bytes into the mosh keystroke channel, queued on
        /// `moshWriteLane` so independently-spawned writes — a keystroke, a
        /// wheel-forwarded scroll, a copy-mode page — can't reorder at the
        /// transport (see the property's doc comment). Every write into this
        /// channel must go through here rather than a bare
        /// `Task { await transport.send(...) }`.
        ///
        /// `predict`: guess the echo locally (mosh predictive echo). Only
        /// plain typing on the live screen qualifies — copy-mode keys, wheel
        /// and clicks move the screen in ways the engine can't model, so they
        /// take every guess down instead.
        private func sendOverMosh(_ transport: MoshTransport, _ data: Data, predict: Bool = false) {
            let batch: Int?
            if predict {
                batch = coordinator.predict(data)
            } else {
                coordinator.dropPredictions()
                batch = nil
            }
            moshWriteLane.enqueue { [weak self] in
                let userState = await transport.send(data)
                if let batch { self?.coordinator.predictionSent(batch: batch, userState: userState) }
            }
        }

        /// Scroll the visible terminal's scrollback (driven by the bar's scroll
        /// thumb / the swipe). tmux sessions — SSH or mosh — page tmux's real
        /// scrollback via copy-mode over the control channel; a plain shell pages
        /// the local SwiftTerm buffer. Positive = older output, negative = newer.
        func scrollActiveTerminal(lines: Int) {
            if let rust { rust.scroll(lines: lines); return }
            // Same gate as sendInput, with one exception: a swipe on a NOT-live
            // session must never write to the wire (mosh would queue wheel or
            // copy-mode bytes and replay them after resume — the same accident
            // as stray arrows, wearing mouse clothes), but paging the LOCAL
            // buffer is read-only and stays useful — an offline plain shell's
            // scrollback is still readable under the red pill.
            guard viewModel.connState == .live else {
                if moshTransport == nil, tmuxController == nil {
                    coordinator.scrollLocal(lines: lines)
                }
                return
            }
            if let transport = moshTransport, let control = moshControl {
                // mosh+tmux: a mouse app (Claude Code etc.) gets a wheel forwarded
                // over the mosh transport — tmux routes it to the app, which
                // scrolls itself with no copy-mode (so typing keeps working). A
                // plain shell gets copy-mode keystrokes on the mosh-rendered
                // client (the sidecar's copy-mode doesn't repaint it).
                if control.activePaneWantsMouse {
                    // The centre of the grid mosh paints tmux's client on
                    // (either view's; 80×24 only before there is one).
                    let grid = coordinator.surface?.grid
                    let col = max(1, (grid?.cols ?? 80) / 2)
                    let row = max(1, (grid?.rows ?? 24) / 2)
                    // Self-heal the first-swipe race: if a tick entered copy-mode
                    // before the mouse flag refreshed, prepend `q` (consumed by
                    // copy-mode, never the app) on the SAME channel so it's
                    // strictly ordered before the wheel.
                    var out = Data()
                    if moshInCopyMode { out += Self.moshCopyExitKey; moshInCopyMode = false }
                    out += Self.wheelBytes(lines: lines, col: col, row: row)
                    if !out.isEmpty { sendOverMosh(transport, out) }
                } else {
                    moshCopyScroll(transport: transport, lines: lines)
                }
            } else if let control = tmuxController {
                // SSH+tmux: the -CC controller IS the renderer, so it decides
                // wheel (mouse app) vs copy-mode and drives the active pane.
                control.scroll(lines: lines)
            } else {
                // No tmux (plain SSH, or mosh degraded to a bare shell): a local
                // mouse app gets synthesized wheel; otherwise page the local
                // buffer. sendWheel/scrollLocal directly (not coordinator.scroll)
                // so the mosh onScroll closure that routes here can't recurse.
                if coordinator.localAppWantsMouse {
                    coordinator.sendWheel(lines: lines)
                } else {
                    coordinator.scrollLocal(lines: lines)
                }
            }
        }

        /// Report a click at a 0-based, viewport-relative cell to whatever is on
        /// screen — a tap asking a mouse-aware program (Claude Code's prompt,
        /// vim) to move its cursor there.
        ///
        /// Same routing tree as ``scrollActiveTerminal(lines:)``, and the same
        /// `#{mouse_any_flag}` gate: a pane running a plain shell gets nothing,
        /// because the report would land in its command line as text.
        func clickActiveTerminal(col: Int, row: Int) {
            if let rust { rust.click(col: col, row: row); return }
            if let transport = moshTransport, let control = moshControl {
                guard control.activePaneWantsMouse else { return }
                // Leave copy-mode first if a scroll put us there, on the SAME
                // channel so it's strictly ordered before the click (see
                // scrollActiveTerminal).
                var out = Data()
                if moshInCopyMode { out += Self.moshCopyExitKey; moshInCopyMode = false }
                out += Self.clickBytes(col: col, row: row)
                sendOverMosh(transport, out)
            } else if let control = tmuxController {
                control.click(col: col, row: row)
            } else {
                // No multiplexer route: let the local terminal encode it in
                // the app's negotiated protocol. clickLocal, NOT click — the
                // mosh path wires coordinator.onClick to THIS method, and
                // click() consults onClick first, so calling it from here is
                // unbounded mutual recursion. Hit for real when a channel
                // failed to attach (no mosh
                // sidecar, no tmux): one tap, 9000 frames, SIGSEGV.
                coordinator.clickLocal(col: col, row: row)
            }
        }

        /// SGR press-then-release for a left click at a 0-based cell — what a
        /// program with the mouse on reads as "the pointer was clicked here", and
        /// what moves its cursor. Button 0 = left; the release repeats the
        /// position with a lowercase `m`, which is how SGR distinguishes it.
        /// Pure + nonisolated for unit testing.
        nonisolated static func clickBytes(col: Int, row: Int) -> Data {
            // SGR coordinates are 1-based on the wire; saturating, so Int.max
            // can't trap (B149, as moshpit-tmux's `click_bytes`).
            let c = max(1, col == .max ? col : col + 1), r = max(1, row == .max ? row : row + 1)
            return Data("\u{1b}[<0;\(c);\(r)M\u{1b}[<0;\(c);\(r)m".utf8)
        }

        /// Page the tmux scrollback over the mosh transport by sending copy-mode
        /// keystrokes to the mosh client. Throttled to page granularity so a
        /// fast-repeating thumb/swipe doesn't rip through history.
        private func moshCopyScroll(transport: MoshTransport, lines: Int) {
            resolveMoshPrefixIfNeeded()
            let now = Date()
            guard now.timeIntervalSince(lastMoshPageAt) >= 0.18 else { return }
            lastMoshPageAt = now
            guard let keys = Self.moshCopyKeys(lines: lines, prefix: moshPrefix,
                                               alreadyInCopyMode: moshInCopyMode) else { return }
            if lines > 0 { moshInCopyMode = true }
            sendOverMosh(transport, keys)
        }

        /// The exact bytes to send to the mosh client for one scroll step, or nil
        /// if there's nothing to send (scroll-down while already live). Pure +
        /// static so it's unit-testable; `scripts/verify/verify-tmux-scroll.py` proves
        /// these same bytes scroll a real tmux client.
        ///   up  : [prefix `[`] + PageUp   (prefix+[ only when entering copy-mode)
        ///   down: PageDown                (only while in copy-mode)
        nonisolated static func moshCopyKeys(lines: Int, prefix: Data, alreadyInCopyMode: Bool) -> Data? {
            let pageUp = Data([0x1b, 0x5b, 0x35, 0x7e])    // ESC [ 5 ~
            let pageDown = Data([0x1b, 0x5b, 0x36, 0x7e])  // ESC [ 6 ~
            guard lines != 0 else { return nil }
            if lines > 0 {
                var keys = Data()
                if !alreadyInCopyMode {
                    keys.append(prefix)   // tmux prefix …
                    keys.append(0x5b)     // … '[' → enter copy-mode
                }
                keys.append(pageUp)
                return keys
            }
            return alreadyInCopyMode ? pageDown : nil
        }

        /// `q` cancels copy-mode; sendInput prepends it when leaving.
        nonisolated static let moshCopyExitKey = Data([0x71])

        /// SGR-encoded scroll-wheel events for a tmux mouse app (Claude Code,
        /// vim/less --mouse). tmux with `mouse on` FORWARDS these to the pane's
        /// program, which scrolls itself — no copy-mode is entered, so typing
        /// afterwards still reaches the app. Button 64 = wheel-up (older), 65 =
        /// wheel-down (newer); `col`/`row` are the 1-based cell the pointer is
        /// "over" (pane center is fine for a single full-screen pane). Clamped so
        /// a fast flick doesn't fire dozens. Pure + nonisolated for unit testing;
        /// `scripts/verify/verify-tmux-wheel.py` proves these bytes scroll a real client.
        nonisolated static func wheelBytes(lines: Int, col: Int, row: Int) -> Data {
            guard lines != 0 else { return Data() }
            let button = lines > 0 ? 64 : 65
            // `magnitude`: `abs` traps on Int.min (B149).
            let count = Int(min(lines.magnitude, 6))
            let c = max(1, col), r = max(1, row)
            let event = "\u{1b}[<\(button);\(c);\(r)M"
            return Data(String(repeating: event, count: count).utf8)
        }

        /// Bytes to actually send the mosh client for a keystroke: when we'd put
        /// the client into copy-mode for scrolling, prepend `q` so the keystroke
        /// leaves copy-mode first and reaches the shell (the reported "can't type
        /// / can't exit in copy-mode" bug). Pure + nonisolated for unit testing;
        /// `scripts/verify/verify-tmux-scroll.py` proves `q` exits a real client.
        nonisolated static func moshInputKeys(_ data: Data, inCopyMode: Bool) -> Data {
            inCopyMode ? moshCopyExitKey + data : data
        }

        /// The predictive-echo mode a mosh session runs with. A connection
        /// left at the default (Adaptive) follows Settings → Predictive Echo;
        /// one set to anything else keeps its own choice.
        nonisolated static func effectivePredictMode(connection: PredictMode, app: PredictMode) -> PredictMode {
            connection == .adaptive ? app : connection
        }

        /// Look up the server's tmux prefix once (via the sidecar) so `prefix [`
        /// enters copy-mode for non-default prefixes. Defaults to C-b until known.
        private func resolveMoshPrefixIfNeeded() {
            guard !moshPrefixResolved, let control = moshControl, control.snapshot.isAttached
            else { return }
            control.queryPrefixKey { [weak self] key in
                self?.moshPrefix = Self.prefixBytes(from: key)
                self?.moshPrefixResolved = true
            }
        }

        /// Parse a tmux `prefix` option value (e.g. "C-b", "C-a") into the byte
        /// the key sends (Ctrl-letter → 0x01…0x1a). Falls back to C-b (0x02).
        nonisolated static func prefixBytes(from option: String) -> Data {
            let s = option.trimmingCharacters(in: .whitespaces)
            if s.hasPrefix("C-"), let ch = s.dropFirst(2).first, let a = ch.asciiValue {
                let upper = (a >= 97 && a <= 122) ? a - 32 : a   // lower → upper
                if upper >= 64 && upper <= 95 { return Data([upper - 64]) }  // @A–Z[\]^_
            }
            return Data([0x02])   // C-b
        }

        /// Single-quote a value that will be spliced into a command typed
        /// into the remote LOGIN SHELL (as opposed to tmux's own control
        /// channel — see `TmuxSessionController.tmuxNameArgument` for that separate
        /// context). Used for tmux SESSION NAMES, which can originate from
        /// the remote server or another client (created, renamed) and are
        /// not otherwise validated here — a name containing a `'` would
        /// otherwise close the surrounding quote early and let the rest of
        /// the string execute as arbitrary shell commands. The app's one
        /// quoter, ``Shell/quote(_:)``.
        nonisolated static func shellQuote(_ value: String) -> String {
            Shell.quote(value)
        }

        /// Open SSH, request a PTY, then either pump bytes into the single
        /// terminal or boot tmux control mode. Mosh connections take a
        /// separate path (SSH only bootstraps the UDP transport).
        /// - Parameter automatic: true when nobody asked for this — the transport
        ///   dropped or the app woke to a dead socket and we're rebuilding it
        ///   behind their back. Those failures stay inside the connecting screen
        ///   instead of raising a modal; see `TerminalViewModel`'s
        ///   `isAutoReconnectInFlight`.
        func start(theme: TerminalTheme, fontSize: Double, fontName: String = "system",
                   cursorShape: CursorShape = .block, cursorColorId: String = "teal",
                   cursorBlink: Bool = true, automatic: Bool = false) async {
            if await startOnRustCoreIfChosen(theme: theme, fontSize: fontSize, fontName: fontName,
                                             cursorShape: cursorShape, cursorColorId: cursorColorId,
                                             cursorBlink: cursorBlink, automatic: automatic) { return }
            // Idempotent for live sessions: re-entering the Terminal screen
            // while connecting/connected must not re-run the bootstrap (the
            // mosh path would exec a second mosh-server). Dead sessions
            // (failed / disconnected) reset and retry. A session being torn
            // down for good starts nothing: every step would stop at
            // `isCurrent` with a connection half made.
            guard !isStopping else { return }
            switch viewModel.status {
            case .connecting, .connected, .reconnecting:
                return
            case .failed, .disconnected, .ended:
                viewModel.resetForReconnect()
            case .idle:
                break
            }
            viewModel.beginAttempt(automatic: automatic)
            dialBegan = Date()
            startGeneration += 1
            let attempt = startGeneration
            // A tmux attempt has nothing to show until its controller reveals a
            // frame; declare that up front so the cover doesn't drop in the gap
            // between the transport coming up and the controller existing.
            if connection.connectionProtocol == .ssh, connection.multiplexer == .tmux {
                awaitingFirstFrame = true
            }
            // Fresh attempt — clear any stale degrade/stall notices so a retry
            // (or a reused-but-reset session) doesn't inherit the last run's
            // banner before it has re-earned it.
            moshReturnPathDead = false
            // Same hygiene for the terminal's input parser: this coordinator
            // is reused across reconnects, and a weak-network drop routinely
            // cuts the old stream mid-character or mid-OSC — parse state that
            // would otherwise poison the new stream's first bytes (see
            // resetInputParser's doc). Covers mosh, plain SSH and the SSH
            // degrade path; tmux panes mint fresh terminals per attach.
            coordinator.resetInputParser()
            lastTheme = theme
            lastFontSize = fontSize
            lastFontName = fontName
            lastCursorShape = cursorShape
            lastCursorColorId = cursorColorId
            lastCursorBlink = cursorBlink
            // Through sendInput, not straight to viewModel.send — it is the
            // documented single entry point for user input, and the not-live
            // gate lives there. (For a plain shell the tmux/mosh branches
            // inside are all nil and the bytes land on viewModel.send anyway.)
            coordinator.onInput = { [weak self] data in self?.sendInput(data) }
            coordinator.onSizeChange = { [viewModel] cols, rows in
                viewModel.resize(rows: rows, cols: cols)
            }

            if connection.connectionProtocol == .mosh {
                await startMosh(attempt: attempt)
                return
            }

            // Seed a phone-sized grid up front so the FIRST PTY / tmux
            // refresh-client size isn't the 80×24 default — a late resize
            // can't reflow lines a program already emitted, and that gap is
            // very visible over high-RTT links (wide text, wrong wrapping).
            // The exact size still follows from the view's layout.
            let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: fontSize)
            let startBegan = Date()
            Log.ssh.debug("resume: start() ssh connect begin")
            // The PTY opens (and the session reads connected) before the
            // multiplexer has anything to show: hold what is typed until then
            // (B169).
            heldInput = []
            // What the terminal runs is decided before it opens, and is its
            // own program — tmux's control client — never
            // a line typed into the login shell (B267): a shell still
            // starting swallowed it (a startup that flushes pending input,
            // Powerlevel10k's instant prompt), and nothing attached.
            var decided: (caps: HostCapabilities, multiplexer: Multiplexer)?
            await viewModel.start(rows: grid.rows, cols: grid.cols) { session in
                let caps = await self.terminalCapabilities(over: session)
                let multiplexer = self.multiplexerToRun(given: caps)
                decided = (caps, multiplexer)
                return self.terminalProgram(for: multiplexer)
            }
            if attempt == startGeneration, viewModel.declinedHostKey {
                retireDeclined()
                return
            }
            // A redial replaced this attempt while its dial was out: the
            // session now on the view model is the replacement's, and the
            // replacement's own `start()` is wiring it up. Not twice. (Nor
            // after a stop: see `isCurrent`.)
            guard isCurrent(attempt) else { return }
            guard let session = viewModel.session else {
                discardHeldInput()   // the dial failed: nothing will be decided
                return
            }
            Log.ssh.debug("resume: ssh+pty up after \(Int(Date().timeIntervalSince(startBegan) * 1000), privacy: .public)ms")
            // Auto-reconnect the moment the transport drops on its own (server
            // reboot, network change, NIO seeing the socket die on resume) —
            // and end the session when the shell it carries exits instead.
            await session.setOnUnexpectedClose { [weak self, weak session] end in
                Task { @MainActor [weak self] in
                    guard let session else { return }
                    await self?.ptyChannelEnded(end, on: session)
                }
            }
            guard isCurrent(attempt) else { return }
            if let size = coordinator.lastReportedSize {
                viewModel.resize(rows: size.rows, cols: size.cols)
            }

            // Decided before the terminal opened (`terminalCapabilities`).
            guard let (caps, multiplexer) = decided else { return }
            Log.ssh.debug("resume: capabilities ready after \(Int(Date().timeIntervalSince(startBegan) * 1000), privacy: .public)ms")
            // Retention sweep for uploaded images, on the channel that's
            // already warm. Fire-and-forget and silent on failure — a host
            // without the folder, or without `find`, owes nobody an error.
            // POSIX tools only, so no PATH prefix needed.
            let retentionDays = AppSettings.shared.uploadRetentionDays
            if retentionDays > 0 {
                let sweep = Self.uploadCleanupCommand(days: retentionDays)
                Task { _ = try? await session.executeCommand(sweep) }
            }
            if multiplexer != connection.multiplexer {
                degrade = DegradeNotice.forMissing(connection.multiplexer)
            }
            // No controller is coming: the plain pane paints on its own — and
            // a frame kept from a tmux attach before this reconnect has
            // nothing to wait for (left up, it froze the screen, B166). What
            // was typed meanwhile goes to the shell now, in order (B169);
            // under tmux it waits for the attach's first frame.
            if multiplexer != .tmux { endFirstFrameWait() }

            if multiplexer == .tmux {
                let controller = TmuxSessionController(sshSession: session)
                // "New session" with nothing attached (the last session gone,
                // the shell back at its prompt): `-CC new` in that shell, the
                // person's own (B228).
                controller.createsFirstSession = { [weak self] name in
                    self?.createFirstTmuxSession(named: name)
                }
                if let ledger = inheritedPinLedger {
                    controller.inheritPinLedger(ledger)
                    inheritedPinLedger = nil
                }
                if let hooks = inheritedAgentHooks {
                    controller.inheritAgentHooks(hooks)
                    inheritedAgentHooks = nil
                }
                controller.transportIsLive = { [weak viewModel] in
                    viewModel?.connState == .live
                }
                // A half-dead link or a wedged tmux stalls only the -CC
                // channel; the keepAlive exec-probe opens a fresh channel and
                // keeps passing, so the controller's own watchdog is the one
                // that notices (真机取证 2026-08-19: frozen screen, 381
                // unanswered commands, no reconnect).
                controller.onChannelSilent = { [weak self] in
                    Task { @MainActor [weak self] in
                        await self?.connectionDropped(reason: "tmux-channel-silent")
                    }
                }
                controller.pendingRestore = lastSelection   // land back on our pane
                controller.configureAppearance(theme: theme, fontSize: fontSize, fontName: fontName,
                                                cursorShape: cursorShape, cursorColorId: cursorColorId,
                                                cursorBlink: cursorBlink)
                controller.setInitialClientSize(cols: grid.cols, rows: grid.rows)
                // Pump + callbacks first, but NO discovery yet — tmux isn't
                // running. %session-changed (fired by `new -A`) triggers it.
                await controller.beginControlMode()
                guard isCurrent(attempt) else { return await abandon(controller, over: session) }
                // Nothing to write: the terminal's program is tmux's control
                // client on the user's EXISTING sessions
                // (`tmuxControlAttachProgram`), already running — what it has
                // printed waits in the session's stream for the controller.
                // Moshpit never creates or modifies sessions on its own — if
                // the server has none, the terminal shows an empty state and
                // the user creates the first one explicitly — and it sets
                // none of the server's options either.
                Log.ssh.debug("resume: tmux controller up after \(Int(Date().timeIntervalSince(startBegan) * 1000), privacy: .public)ms")
                tmuxController = controller
                armFirstFrameWait(for: controller)
                controlPlaneWired(controller)
                beginAttachTimeout(controller: controller)
            } else {
                let coordinator = self.coordinator
                pumpTask = Task { [weak self] in
                    for await chunk in session.dataStream {
                        if Task.isCancelled { break }
                        coordinator.feed(data: chunk)
                        // Proof of life for the keepalive — see noteInbound().
                        await MainActor.run { self?.noteInbound() }
                    }
                    _ = self
                }
            }
        }

        /// The herdr banner button's tap: the Rust core runs its remedy
        /// (restart the server, "Open here", retry).
        func runHerdrNoticeAction() {
            rust?.runHerdrNoticeAction()
        }

        /// Probe (or read cached) host capabilities and publish them on the
        /// session. Uses any cached value immediately for a fast first paint,
        /// then re-probes in the foreground so a freshly-installed tool is
        /// reflected this session. Tolerant: a probe that can't run keeps the
        /// last known capabilities (or the optimistic default on a cold
        /// session) so a transient hiccup never degrades a healthy host — and
        /// never *upgrades* a known-degraded one back to "all present".
        @discardableResult
        func probeCapabilities(over ssh: any HostConnection) async -> HostCapabilities {
            let cache = HostCapabilityCache.shared
            if let cached = cache.capabilities(for: connection.id) {
                capabilities = cached
            }
            // Only a probe that actually ran may overwrite what we know. A
            // failed probe returns nil; clobbering a known-degraded host with
            // the optimistic default would retry `-CC attach` on a tmux-less
            // box and stall.
            if let fresh = await cache.probe(over: ssh, for: connection.id) {
                capabilities = fresh
                return fresh
            }
            return capabilities ?? .unknown
        }

        /// What the host has, for deciding the SSH terminal's program before
        /// the terminal opens. A known host decides on what we know and
        /// refreshes behind it: waiting for the probe cost a full round trip
        /// on every reconnect for an answer that changes about once per host
        /// lifetime, and a stale "has tmux" still fails safe (the attach
        /// doesn't confirm and the empty state offers Install/Create). A
        /// first connect waits for the probe.
        private func terminalCapabilities(over session: SSHSession) async -> HostCapabilities {
            if let known = HostCapabilityCache.shared.capabilities(for: connection.id) {
                capabilities = known
                let id = connection.id
                Task { @MainActor [weak self] in
                    if let fresh = await HostCapabilityCache.shared.probe(over: session, for: id) {
                        self?.capabilities = fresh
                    }
                }
                return known
            }
            return await probeCapabilities(over: session)
        }

        /// The multiplexer this connection runs on a host with `caps`: the
        /// chosen one, or — missing, and no custom binary path vouching for
        /// it — none: a plain shell with a banner, never the OTHER
        /// multiplexer, instead of stalling on a failing attach.
        private func multiplexerToRun(given caps: HostCapabilities) -> Multiplexer {
            let chosen = connection.multiplexer
            let available = connection.multiplexerPath != nil || caps.has(chosen)
            return available ? chosen : .none
        }

        /// The SSH terminal's own program for `multiplexer` (nil: the login
        /// shell): tmux's control client; herdr runs on the Rust core only
        /// (E1). Never a line typed into a shell (B267).
        private func terminalProgram(for multiplexer: Multiplexer) -> String? {
            switch multiplexer {
            case .none, .herdr:
                return nil
            case .tmux:
                return Self.tmuxControlAttachProgram(tmux: connection.tmuxPath ?? "tmux")
            }
        }

        /// Re-run the probe over whichever SSH channel this session owns
        /// (in-band SSH session or the mosh sidecar). Returns the fresh caps,
        /// or nil if no SSH channel is available. Used by Install Assist's
        /// "Re-check" after the user runs the install command.
        @discardableResult
        func recheckCapabilities() async -> HostCapabilities? {
            if let rust = lendingRust {
                // The Rust session's own connection (W14b), or nil (→
                // "reconnect to apply") when it isn't up.
                guard let host = try? await rust.side.ready() else { return nil }
                return await probeCapabilities(over: host)
            }
            // Re-check needs a LIVE channel. Mosh closes its bootstrap SSH after
            // handoff, so there may be none — return nil (→ "reconnect to apply")
            // rather than probing a dead session, which would falsely read as
            // "everything installed".
            let ssh = viewModel.session ?? sidecarSSH
            guard let ssh, await ssh.isConnected else { return nil }
            return await probeCapabilities(over: ssh)
        }

        /// Create the FIRST tmux session on a server that has none. Only
        /// valid pre-attach. Over SSH the terminal's program is waiting for a
        /// session (``tmuxControlAttachProgram(tmux:)``): the session is made
        /// detached by exec on the same connection, and the waiting control
        /// client attaches it within a second. It used to be a `-CC new` line
        /// typed into the shell the failed attach left behind (B267: nothing
        /// is typed into a shell). Over mosh the session is made over the
        /// sidecar, and the waiting renderer and chain attach it.
        func createFirstTmuxSession(named name: String? = nil) {
            if let rust { rust.createFirstTmuxSession(); return }
            if let control = moshControl, tmuxController == nil {
                // mosh: the shell on screen is the renderer's, waiting; the
                // session is made over the sidecar (B228).
                control.newSession(named: name)
                return
            }
            guard let controller = tmuxController, !controller.snapshot.isAttached,
                  let ssh = viewModel.session else { return }
            let command = Self.tmuxFirstSessionCommand(tmux: connection.tmuxPath ?? "tmux", name: name)
            Task { [weak controller] in
                let output = await withTimeoutValue(10) { try await ssh.executeCommand(command) }
                let text = output.map { String(decoding: $0, as: UTF8.self) } ?? ""
                var lines = text.split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
                let status = lines.popLast()
                guard status != "moshpit-status:0" else {
                    Log.ssh.info("tmux: made the first session the person asked for")
                    return
                }
                Log.ssh.error("tmux: the first session wasn't made (\(status ?? "no answer", privacy: .public))")
                controller?.noteSessionNotCreated(lines.last ?? "")
            }
        }

        /// The first session on a mosh + tmux server, made over the control
        /// plane's connection when the person asks (B228) — as crates/session
        /// does (`tmux_leg.rs`'s `first_session`): `new-session -d`, detached;
        /// the sidecar's attach chain and the renderer's waiting line both
        /// land on it by themselves. tmux's refusal ("duplicate session") is
        /// the controller's notice.
        private func createFirstMoshSession(named name: String?, notifying control: TmuxSessionController?) async {
            guard let ssh = sidecarSSH else {
                control?.noteSessionNotCreated(String(localized: "Not connected to tmux yet. Try again in a moment."))
                return
            }
            let command = Self.tmuxFirstSessionCommand(tmux: connection.tmuxPath ?? "tmux", name: name)
            let output = await withTimeoutValue(10) { try await ssh.executeCommand(command) }
            let text = output.map { String(decoding: $0, as: UTF8.self) } ?? ""
            var lines = text.split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
            let status = lines.popLast()
            guard status != "moshpit-status:0" else {
                Log.ssh.info("mosh+tmux: made the first session the person asked for")
                return
            }
            Log.ssh.error("mosh+tmux: the first session wasn't made (\(status ?? "no answer", privacy: .public))")
            control?.noteSessionNotCreated(lines.last ?? "")
        }

        /// The exec that makes the first session: detached, named when the
        /// person named it, tmux's own words and its status on the way back.
        nonisolated static func tmuxFirstSessionCommand(tmux: String, name: String?) -> String {
            let namePart = name.map { " -s " + TmuxSessionController.tmuxNameShellWord($0) } ?? ""
            return "\(tmux) new-session -d\(namePart) 2>&1; echo \"moshpit-status:$?\""
        }

        /// Whether the server has no tmux session at all, asked over the
        /// control plane's connection; nil when it can't be told.
        private func serverHasNoSession(tmux: String) async -> Bool? {
            guard let ssh = sidecarSSH,
                  let output = await withTimeoutValue(5, {
                      try await ssh.executeCommand("\(tmux) has-session 2>/dev/null; echo $?")
                  })
            else { return nil }
            switch String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines) {
            case "0": return false
            case "1": return true
            default: return nil
            }
        }

        /// Mosh path: authenticate SSH (no PTY), exec `mosh-server` to learn
        /// the UDP port + key, drop SSH, then run the real SSP transport.
        ///
        /// `attempt` is `start()`'s: every await resumes into ``isCurrent(_:)``
        /// (B168). Before, a disconnect while mosh-server was starting left a
        /// transport running and a server nobody reaped outside the hub, and
        /// an attempt a redial had replaced could mark the new one failed or
        /// install a second transport over it.
        private func startMosh(attempt: Int) async {
            // Reconnect on a REUSED coordinator: a scroll-hold engaged before
            // the disconnect would silently buffer the fresh connection's
            // output (frozen screen); the held bytes belong to the dead screen
            // — discard, don't replay. Same for the mosh copy-mode flag, whose
            // q-prepend would eat the first keystroke.
            coordinator.discardScrollHold()
            coordinator.attachPrediction(nil)
            moshInCopyMode = false
            guard let ssh = await viewModel.connectForExec() else {
                if viewModel.declinedHostKey, isCurrent(attempt) { retireDeclined() }
                return
            }
            // The view model's connection: a stop closes it, a newer attempt
            // owns the view model.
            guard isCurrent(attempt) else { return }

            // Probe before spending a round-trip on `mosh-server new`. A custom
            // server path means the user vouches for it — trust it. Otherwise a
            // missing mosh-server degrades to plain SSH (which we already have
            // authenticated right here) with a banner, instead of surfacing a
            // raw `command not found`.
            let caps = await probeCapabilities(over: ssh)
            guard isCurrent(attempt) else { return }
            if connection.moshServerPath == nil && !caps.hasMoshServer {
                degrade = DegradeNotice(missing: .moshServer)
                await fallbackToSSH(over: ssh, capabilities: caps, attempt: attempt)
                return
            }

            do {
                // UDP goes where this SSH connection went: `-s` binds the
                // server to that address, and a fresh lookup of a name with
                // several addresses can pick another — connected and black
                // (B171). Through a proxy there is no such address: the name.
                // The multiplexer's renderer is mosh-server's own program,
                // decided (and the last connects' renderers retired) before
                // the server starts — never a line typed into the mosh shell
                // once it is up (B267).
                let renderer = await moshRendererProgram(over: ssh, capabilities: caps)
                guard isCurrent(attempt) else { return }
                let creds = try await MoshBootstrap.start(
                    over: ssh,
                    host: ssh.peerAddress ?? connection.host,
                    serverBinary: connection.moshServerPath ?? "mosh-server",
                    // Settings' ports, for every connection (B430).
                    portRangeStart: AppSettings.shared.udpRangeStart,
                    portRangeEnd: AppSettings.shared.udpRangeEnd,
                    program: renderer)
                // Stopped or replaced while the server was starting: nothing
                // else will ever know this pid — reap it here (B168). From the
                // line below on, a stop() that comes knows it, and reaps it.
                guard isCurrent(attempt) else {
                    await reapAbandonedServer(creds.serverPid, over: ssh)
                    return
                }
                // Captured as early as possible: every await between here and
                // `onReturnPathDead` being wired below (the herdr/tmux
                // renderer probes, `ssh.close()`, `MoshTransport(credentials:)`)
                // is a window where a user backing out reaches `stop()` with
                // no reaper armed at all otherwise. See `moshServerPid`'s doc
                // comment for the fallback that covers it.
                //
                // `moshHadFirstContact` resets alongside it — this session
                // (an `ActiveSession` reused across a `reconnect()`) may carry
                // `true` from a PREVIOUS bootstrap's transport, and this new
                // pid's own contact status starts over at false exactly like
                // the fresh `MoshTransport` below starts `sawAnyDatagram` at
                // false. Safe to reset here, before this new pid is even
                // reapable: nothing between here and `stop()` reads this flag.
                moshServerPid = creds.serverPid
                moshHadFirstContact = false
                await ssh.close()   // mosh-server has daemonized; UDP from here
                // The pid is the session's now: a stop that ran meanwhile has
                // reaped it (or the next one will). Nothing to install.
                guard isCurrent(attempt) else { return }

                let transport = try makeMoshTransport(creds)
                self.moshTransport = transport

                await MainActor.run {
                    self.coordinator.attachPrediction(MoshPrediction(
                        mode: Self.effectivePredictMode(connection: self.connection.predictMode,
                                                        app: AppSettings.shared.predictMode),
                        trail: AppSettings.shared.trailOnPredict))
                    transport.onMetrics = { [weak self] srtt, roaming in
                        self?.metricsSink?(srtt, roaming)
                        self?.coordinator.prediction?.srttMs = srtt
                    }
                    transport.onDiagnostics = { [weak self] diagnostics in
                        self?.diagnosticsSink?(diagnostics)
                        // First real contact — see `moshHadFirstContact`'s
                        // doc comment. Mirrors `sawAnyDatagram` exactly
                        // (`datagramsReceived` only increments where that
                        // flips), so this can never disagree with the
                        // watchdog `onReturnPathDead` below is gated on.
                        if diagnostics.datagramsReceived > 0 {
                            self?.moshHadFirstContact = true
                        }
                    }
                    transport.onSessionEnded = { [weak self, weak transport] in
                        guard let transport else { return }
                        self?.moshSessionEnded(transport)
                    }
                    transport.onReturnPathDead = { [weak self, weak transport] heardFromServer in
                        guard let self, let transport, self.moshTransport === transport else { return }
                        // The banner offering SSH, either way; the session
                        // stays connected.
                        self.moshReturnPathDead = true
                        // Only a server that never heard from us is reaped:
                        // it will wait forever for a first contact that isn't
                        // coming, and every dead-UDP attempt used to leave one
                        // (7 on one host, 2026-08-19). This also fires for a
                        // session that HAD talked to its server and then went
                        // quiet through three flow rebuilds — about 15 s of
                        // UDP silence — and that server holds the user's
                        // shell: killing it here is what B165 was.
                        let heard = heardFromServer || self.moshHadFirstContact
                        guard MoshServerReap.onReturnPathDead(heardFromServer: heard),
                              let pid = creds.serverPid else { return }
                        self.moshServerPid = nil   // handled; stop()'s fallback need not repeat it
                        Task { [weak self] in
                            guard let self else { return }
                            if let ssh = try? await self.sidecarConnect(self.connection) {
                                _ = try? await ssh.executeCommand("kill \(pid) 2>/dev/null")
                                await ssh.close()
                            }
                        }
                    }
                }
                // Installed: a stop from here on closes the transport itself.
                guard isCurrent(attempt) else { return }

                // Always render mosh as a single rendered terminal. (tmux -CC
                // control mode CANNOT run over mosh — mosh transmits screen
                // diffs, not the raw line-framed control stream -CC needs;
                // verified empirically. Our native Sessions/Windows/Pane
                // sheets therefore require SSH+tmux.) When tmux is picked we
                // still give the user tmux multiplexing by launching the plain
                // full-screen tmux TUI inside the mosh shell — Ctrl-b drives
                // it, and it roams with mosh.
                // Route keyboard input through sendInput (not straight to the
                // transport) so it leaves copy-mode first — otherwise, after a
                // scroll, typed keys are eaten by tmux copy-mode and input dies.
                coordinator.onInput = { [weak self] data in self?.sendInput(data) }
                coordinator.onSizeChange = { [weak self] cols, rows in
                    // On the mosh write lane, in order with the keystrokes and
                    // with each other: a Task per resize gave a burst (a
                    // rotation, the keyboard animating in) no order, and an
                    // older size landing last would leave mosh-server there.
                    self?.moshWriteLane.enqueue { await transport.resize(cols: cols, rows: rows) }
                    // mosh+tmux: also pin the tmux window via the -CC sidecar,
                    // else its 80×24 PTY strands the TUI at 80×24 (the renderer
                    // is raw mosh and can't size the window itself).
                    self?.moshControl?.syncMoshWindow(cols: cols, rows: rows)
                }
                // The mosh terminal lives on this single coordinator, which —
                // unlike SSH tmux panes (wired in mintTerminal) — never had
                // onScroll set, so the swipe was dead over mosh. Route it to the
                // -CC sidecar's copy-mode (scrollActiveTerminal); the no-tmux
                // degrade case falls back to the local buffer there.
                coordinator.onScroll = { [weak self] lines in
                    self?.scrollActiveTerminal(lines: lines)
                }
                // Refresh the active pane's mouse flag when a drag starts so the
                // wheel-vs-copy-mode decision is fresh (catches an app launched
                // in-place, with no pane switch to refresh it).
                coordinator.onScrollBegin = { [weak self] in
                    self?.moshControl?.refreshActivePaneMouse()
                }
                // Tap-to-position over mosh: the click rides the mosh transport
                // (the renderer IS the tmux client), gated on the -CC sidecar's
                // view of the pane's mouse flag. Refreshed alongside, same as
                // the scroll path, so an app launched in-place is caught.
                coordinator.onClick = { [weak self] col, row in
                    guard let self else { return }
                    self.clickActiveTerminal(col: col, row: row)
                    self.moshControl?.refreshActivePaneMouse()
                }
                // Horizontal swipe: switch pane/window via the -CC sidecar, which
                // moves the mosh-rendered client too (same session, shared current
                // window/pane) — the same path the breadcrumb uses.
                coordinator.onSwitch = { [weak self] forward in
                    self?.moshControl?.switchPaneOrWindow(forward: forward)
                }
                let coordinator = self.coordinator
                pumpTask = Task {
                    for await frame in transport.hostStream {
                        if Task.isCancelled { break }
                        coordinator.feed(moshFrame: frame)
                    }
                }
                // The view usually finishes layout during the SSH bootstrap,
                // so its one-and-only sizeChanged fired before this transport
                // existed. Start at the real grid size and re-assert it after
                // start (resize() de-dupes) — otherwise mosh-server stays at
                // 80×24 and tmux renders into the top half of the screen with
                // the status line stranded mid-viewport.
                let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: lastFontSize)
                await transport.start(cols: grid.cols, rows: grid.rows)
                if let size = coordinator.lastReportedSize {
                    await transport.resize(cols: size.cols, rows: size.rows)
                }
                guard isCurrent(attempt) else { return }
                // Multiplexer chosen but absent on the host: stay on the bare
                // mosh shell (don't send an attach line that errors, don't
                // start the retrying sidecar — see the design's degrade
                // matrix) and raise a banner. A custom binary path means the
                // user vouches for it.
                let chosen = connection.multiplexer
                let muxDegraded = connection.multiplexerPath == nil && !caps.has(chosen)
                if muxDegraded { degrade = DegradeNotice.forMissing(chosen) }

                switch muxDegraded ? .none : chosen {
                case .tmux:
                    // The renderer — mosh-server's program — attaches the
                    // remembered session, or the most recent, or waits for
                    // the first; the -CC control plane comes up beside it and
                    // learns which client it is. Stored (not fire-and-forget)
                    // so `stop()` can cancel it — see the property's doc
                    // comment.
                    let nonce = moshTmuxRendererNonce
                    moshTmuxAttachTask = Task { [weak self] in await self?.attachMoshTmux(transport, rendererNonce: nonce) }
                case .herdr, .none:
                    break
                }
                // The mosh terminal paints itself: a frame kept from an SSH
                // fallback's tmux before this reconnect has nothing to wait
                // for (B166).
                endFirstFrameWait()
                viewModel.markConnected()
            } catch {
                // A replaced attempt's failure is not the new one's (B168).
                guard isCurrent(attempt) else { return }
                let message = String(describing: error)
                viewModel.fail(String(localized: "Mosh: \(message)"))
            }
        }

        /// mosh-server ended the session — the remote shell exited (`exit`),
        /// or the server was stopped — and the transport acked its shutdown
        /// state and closed (`MoshTransport.onSessionEnded`), as mosh-client
        /// exits. It used to apply that state and then hear nothing: a frozen
        /// screen, flow rebuilds, and the dead-return-path banner offering to
        /// switch to SSH. There is nothing to roam back to: the session has
        /// ended (``endedByServer(_:)``) — the reason on the terminal's
        /// "Session ended" card, and out of the hub the way Disconnect takes
        /// it (a dead session left in the hub is redialled by the next
        /// keepalive).
        func moshSessionEnded(_ transport: MoshTransport) {
            guard moshTransport === transport, !isStopping else { return }
            // The server quit after its goodbye: its pid is nobody's to kill
            // any more (and may soon be another process's).
            moshServerPid = nil
            moshReturnPathDead = false
            endedByServer(String(localized: "The session ended: the remote shell exited or mosh-server was stopped."))
        }

        /// The SSH PTY channel ended on its own — never through our own
        /// `close()` (see `SSHSession.setOnUnexpectedClose`).
        ///
        /// A connection that dropped is redialled, as it always was. A shell
        /// that EXITED — `exit` or Ctrl-D in a plain SSH session, sshd closing
        /// the channel in order behind its exit status — used to be taken for
        /// the same thing and redialled too: the shell the user had just left
        /// came straight back as a fresh login. When that channel carries the
        /// user's own shell, its end is the session's end
        /// (``remoteShellExited(on:)``). When it carries a control plane
        /// Moshpit runs in it — tmux `-CC` — the sessions
        /// the user works in live on the server, beyond the shell carrying
        /// them, and a redial brings them back as before.
        func ptyChannelEnded(_ end: SSHChannelEnd, on session: SSHSession) async {
            // A channel of a connection already replaced (or being torn down)
            // is not this session's news.
            guard !isStopping, viewModel.session === session else { return }
            if case .exited(let status) = end, tmuxController == nil {
                Log.ssh.notice("health: the remote shell exited (status \(status ?? 0, privacy: .public)) — ending the session")
                await remoteShellExited(on: session)
                return
            }
            await connectionDropped(reason: end == .dropped ? "transport-closed" : "control-shell-exited")
        }

        /// The plain shell's end: the session is over, the way mosh-server's
        /// shutdown ends a mosh session (``moshSessionEnded(_:)``, then
        /// ``endedByServer(_:)``).
        private func remoteShellExited(on session: SSHSession) async {
            // The shell's last words (`logout`, the last command's output) are
            // still in the stream ahead of its end. The pump ends with the
            // stream; let it paint them before the teardown below cancels it.
            await pumpTask?.value
            guard !isStopping, viewModel.session === session else { return }
            endedByServer(String(localized: "The session ended: the remote shell exited."))
        }

        /// The user said no to the server's key (B151): nothing to say about
        /// it, and nothing automatic may dial it again — a foreground resume
        /// redials a down session and would put the same card back up — so
        /// the hub takes it out as it does a Disconnect (`endedSink`).
        private func retireDeclined() {
            isStopping = true
            endedSink?()
        }

        /// The remote end finished this session for good: say so, as an end
        /// and not as an error (`TerminalViewModel.end`), and let nothing
        /// automatic bring it back — the keepalive, a foreground resume, a
        /// late death callback all stand down as they do for a Disconnect —
        /// while the hub takes it out (`endedSink`). Reconnecting is the
        /// user's call, from the terminal's card.
        private func endedByServer(_ reason: String) {
            viewModel.end(reason)
            isStopping = true
            endedSink?()
        }

        /// Degrade path for a mosh connection whose host has no mosh-server:
        /// reuse the already-authenticated SSH session as a plain single-pane
        /// shell (request the PTY `connectForExec` skipped, then pump). If the
        /// host has tmux and the user asked for it, boot tmux -CC exactly like
        /// the SSH path so multiplexing still works over the fallback.
        private func fallbackToSSH(over session: SSHSession,
                                   capabilities caps: HostCapabilities,
                                   attempt: Int) async {
            // Same routing note as attempt(): sendInput is the choke point.
            coordinator.onInput = { [weak self] data in self?.sendInput(data) }
            coordinator.onSizeChange = { [viewModel] cols, rows in
                viewModel.resize(rows: rows, cols: cols)
            }
            let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: lastFontSize)
            // If the user wanted a multiplexer and the host has it, give it to
            // them over this SSH fallback so multiplexing still works — as the
            // terminal's own program, never a line typed into its shell
            // (B267). If it's also missing, the mosh-server banner already
            // points at the same Install Assist sheet, so we don't stack a
            // second banner — just run the bare shell.
            let multiplexer = multiplexerToRun(given: caps)
            do {
                try await session.requestPTY(rows: grid.rows, cols: grid.cols,
                                             command: terminalProgram(for: multiplexer))
            } catch {
                // A replaced attempt's failure is not the new one's (B168).
                guard isCurrent(attempt) else { return }
                viewModel.fail((error as? SSHError)?.description ?? "\(error)")
                return
            }
            // Every await below resumes into the same check as startMosh's:
            // a stop or a newer attempt since means none of this is ours to
            // install (B168).
            guard isCurrent(attempt) else { return }
            // The same watch as the SSH path's: this PTY is the session now.
            // Without it an `exit` here left a dead shell on a screen that
            // still read connected, and a drop waited for the keepalive.
            await session.setOnUnexpectedClose { [weak self, weak session] end in
                Task { @MainActor [weak self] in
                    guard let session else { return }
                    await self?.ptyChannelEnded(end, on: session)
                }
            }
            guard isCurrent(attempt) else { return }
            if let size = coordinator.lastReportedSize {
                viewModel.resize(rows: size.rows, cols: size.cols)
            }

            if multiplexer == .tmux {
                let controller = TmuxSessionController(sshSession: session)
                // "New session" with nothing attached (the last session gone,
                // the shell back at its prompt): `-CC new` in that shell, the
                // person's own (B228).
                controller.createsFirstSession = { [weak self] name in
                    self?.createFirstTmuxSession(named: name)
                }
                // The pins the controller this reconnect retired still owes
                // the server: this one settles them, as the SSH path's does.
                // Left behind, the next reconnect overwrote them and nothing
                // ever handed the desktop its window size back (B167).
                if let ledger = inheritedPinLedger {
                    controller.inheritPinLedger(ledger)
                    inheritedPinLedger = nil
                }
                if let hooks = inheritedAgentHooks {
                    controller.inheritAgentHooks(hooks)
                    inheritedAgentHooks = nil
                }
                controller.transportIsLive = { [weak viewModel] in
                    viewModel?.connState == .live
                }
                // A half-dead link or a wedged tmux stalls only the -CC
                // channel; the keepAlive exec-probe opens a fresh channel and
                // keeps passing, so the controller's own watchdog is the one
                // that notices (真机取证 2026-08-19: frozen screen, 381
                // unanswered commands, no reconnect).
                controller.onChannelSilent = { [weak self] in
                    Task { @MainActor [weak self] in
                        await self?.connectionDropped(reason: "tmux-channel-silent")
                    }
                }
                controller.pendingRestore = lastSelection
                controller.configureAppearance(theme: lastTheme, fontSize: lastFontSize, fontName: lastFontName,
                                                cursorShape: lastCursorShape, cursorColorId: lastCursorColorId,
                                                cursorBlink: lastCursorBlink)
                controller.setInitialClientSize(cols: grid.cols, rows: grid.rows)
                await controller.beginControlMode()
                guard isCurrent(attempt) else { return await abandon(controller, over: session) }
                // The session reads connected below, a round trip before tmux
                // has attached and named a pane: what is typed in between was
                // dropped without a word — there was no pane to send it to
                // (B197; B169 held it on the SSH path only). Held instead, as
                // there, and handed to the active pane in order once the
                // attach has landed (`inputDecided`, `endFirstFrameWait`);
                // dropped, never typed into the shell, if tmux refuses it.
                heldInput = []
                // Attach only, as the SSH path does, and nothing to write:
                // the terminal's program is tmux's control client.
                tmuxController = controller
                // As the SSH path does: after a reconnect the dropped
                // connection's last frame stands in until this controller
                // paints, and only this wait takes it down. Without it the
                // old frame stayed for good — frozen, and the terminal deaf
                // to touches, after every reconnect of a mosh session fallen
                // back to SSH with tmux (B166).
                armFirstFrameWait(for: controller)
                controlPlaneWired(controller)
                beginAttachTimeout(controller: controller)
            } else {
                // The plain pane paints on its own; a kept tmux frame over it
                // would never come down.
                endFirstFrameWait()
                let coordinator = self.coordinator
                pumpTask = Task { [weak self] in
                    for await chunk in session.dataStream {
                        if Task.isCancelled { break }
                        coordinator.feed(data: chunk)
                        // Proof of life for the keepalive — see noteInbound().
                        await MainActor.run { self?.noteInbound() }
                    }
                    _ = self
                }
            }
            viewModel.markConnected()
        }

        /// What mosh-server runs for this connection's multiplexer — its
        /// renderer — instead of the login shell; nil for a plain shell (no
        /// multiplexer, or the chosen one missing). Decided over the
        /// bootstrap's connection before the server starts, and the last
        /// connects' renderers retired first: orphaned ones (mosh-server
        /// survives disconnects by design) must not contest the pane or the
        /// window size, and a cleanup run after the start would stop the new
        /// renderer too.
        ///
        /// The renderer was a line typed into the mosh shell once it was up,
        /// and a shell still starting swallowed it (Powerlevel10k's instant
        /// prompt flushes pending input); for tmux it was typed again up to
        /// four times, and a copy that ran late landed in the pane as
        /// keystrokes (B170), its control bytes acting before its quotes did
        /// (B140). As the session's program it runs at once, under no shell
        /// that could take it for typing (B267).
        private func moshRendererProgram(over ssh: SSHSession, capabilities caps: HostCapabilities) async -> String? {
            let chosen = connection.multiplexer
            guard connection.multiplexerPath != nil || caps.has(chosen) else { return nil }
            switch chosen {
            case .none:
                return nil
            case .tmux:
                _ = try? await ssh.executeCommand(Self.tmuxRendererCleanupCommand(connectionId: connection.id))
                // The windows as they are before the renderer sizes them to
                // the phone: the sidecar records what it squashed (B270).
                let tmux = connection.tmuxPath ?? "tmux"
                moshTmuxLayoutsBeforeRenderer = await withTimeoutValue(5) {
                    try await ssh.executeCommand(
                        "\(tmux) list-windows -a -F '\(TmuxSessionController.sizeProbeFormat)' 2>/dev/null")
                }.map { String(decoding: $0, as: UTF8.self) }
                let nonce = String(UUID().uuidString.prefix(8))
                moshTmuxRendererNonce = nonce
                return Self.tmuxRendererProgram(tmux: connection.tmuxPath ?? "tmux",
                                                remembered: lastSelection?.session,
                                                connectionId: connection.id, nonce: nonce)
            case .herdr:
                // herdr runs on the Rust core only (E1): the login shell.
                return nil
            }
        }

        /// Attach tmux over mosh. The renderer is mosh-server's program
        /// (``tmuxRendererProgram(tmux:remembered:connectionId:nonce:)``): it
        /// attaches the remembered session, else the most recent, else waits
        /// for the first. The -CC sidecar comes up beside it on the user's
        /// existing sessions (the remembered one restored), and is told which
        /// client the renderer is by the pid the program recorded.
        /// Moshpit never creates a session on the user's behalf.
        /// Pin the tmux window to the current phone grid through the mosh -CC
        /// sidecar, retrying briefly until it reports an active window. Without
        /// this the mosh-rendered TUI inherits the sidecar's 80×24 PTY and is
        /// stranded in the corner. No-op until the sidecar is attached.
        private func pinMoshWindow() async {
            let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: lastFontSize)
            for _ in 0..<15 {
                guard !isStopping else { return }
                if let control = moshControl, control.snapshot.activeWindowId != nil {
                    control.syncMoshWindow(cols: grid.cols, rows: grid.rows)
                    return
                }
                try? await Task.sleep(nanoseconds: 200_000_000)
            }
        }

        private func attachMoshTmux(_ transport: MoshTransport, rendererNonce nonce: String) async {
            moshServerHadNoSession = false
            await startMoshControlPlane()
            guard attaching(on: transport) else { return }
            // Same stall latch the SSH+tmux path arms at its `-CC attach`
            // boots: without it, a sidecar whose attach never confirms (host
            // dying mid-connect, a wedged tmux server) leaves the home card
            // on "Attaching session…" forever with no way out — the mosh
            // transport itself is up, so nothing else ever fails. Verified
            // live against a host that went to sleep mid-attach.
            if let control = moshControl { beginAttachTimeout(controller: control) }

            let tmux = connection.tmuxPath ?? "tmux"
            // Give the control plane a few seconds to land on a session.
            for _ in 0..<24 {
                guard attaching(on: transport) else { return }
                if let control = moshControl, control.snapshot.isAttached { break }
                await moshAttachPause(.milliseconds(250))
            }
            guard attaching(on: transport) else { return }
            // Nothing for the control plane to land on: no session on the
            // server (it attaches within a second or two of any there is), or
            // no control plane at all. The renderer waits for a session
            // itself and attaches the first there is — the person's, made
            // from the empty state or anywhere else (B212). Moshpit makes
            // none.
            //
            // Which of the two it is, asked (B228): with no session there,
            // the attach isn't stalled — it waits for the person's "New
            // session", which Home and the terminal now offer.
            if moshControl?.snapshot.isAttached == false,
               await serverHasNoSession(tmux: tmux) == true,
               attaching(on: transport), moshControl?.snapshot.isAttached == false {
                moshServerHadNoSession = true
                attachStalled = false
            }
            await adoptMoshRenderer(transport, nonce: nonce)
        }

        /// Whether the mosh+tmux attach for `transport` is still the
        /// session's to make: not cancelled (a stop, a reconnect), not
        /// stopping, the transport still this session's.
        private func attaching(on transport: MoshTransport) -> Bool {
            !Task.isCancelled && !isStopping && moshTransport === transport
        }

        /// A pause of the mosh+tmux attach, on ``moshAttachClock``; over
        /// early when the attach is cancelled.
        private func moshAttachPause(_ duration: Duration) async {
            try? await Self.sleep(on: moshAttachClock, for: duration)
        }

        private static func sleep<C: Clock>(on clock: C, for duration: C.Duration) async throws {
            try await clock.sleep(until: clock.now.advanced(by: duration), tolerance: nil)
        }

        /// The renderer mosh-server runs attaches by itself; once the control
        /// plane has attached too, the control plane is told which client it
        /// is — by the pid the renderer's program recorded, never by counting
        /// clients around an attach (B84;
        /// ``TmuxSessionController/adoptRenderer(pid:)``) — and the window is
        /// pinned to the phone, and a copy-mode left over from the previous
        /// connection cancelled (it would show the renderer an old
        /// scrolled-away view and make the first swipe jump to that stale
        /// offset). Waits as long as the session lasts — with no session on
        /// the server the person makes the first when they choose — at the
        /// cost of a look at the control plane's own state once a second: no
        /// round trip until it attached.
        private func adoptMoshRenderer(_ transport: MoshTransport, nonce: String) async {
            var pid: Int?
            var looks = 0
            while attaching(on: transport) {
                if let control = moshControl, control.snapshot.isAttached, let ssh = sidecarSSH {
                    if pid == nil { pid = await waitingRendererPid(over: ssh, nonce: nonce) }
                    if let pid, await withTimeoutValue(2, { await control.adoptRenderer(pid: pid) }) == true {
                        guard attaching(on: transport) else { return }
                        await pinMoshWindow()
                        control.cancelStaleCopyMode()
                        return
                    }
                    // The control plane is on a session, so the renderer —
                    // asking tmux every second — attaches within one. Not
                    // there after ten: it isn't coming (^C gave the shell
                    // back), and the screen is the person's own.
                    looks += 1
                    if looks >= 10 {
                        Log.ssh.info("mosh+tmux: the renderer never attached — leaving the shell as it is")
                        return
                    }
                }
                await moshAttachPause(.seconds(1))
            }
        }

        /// The pid the renderer's program recorded, read over the control
        /// plane's connection; nil while there is none to read.
        private func waitingRendererPid(over ssh: SSHSession, nonce: String) async -> Int? {
            let path = Self.tmuxRendererPidPath(connectionId: connection.id, nonce: nonce)
            guard let output = await withTimeoutValue(5, { try await ssh.executeCommand("cat \"\(path)\"") })
            else { return nil }
            let text = String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            guard let pid = Int(text), pid > 1 else { return nil }
            return pid
        }

        // MARK: mosh+tmux renderer hygiene

        /// Pidfile the renderer's program writes before exec'ing tmux
        /// (``tmuxRendererProgram(tmux:remembered:connectionId:nonce:)``):
        /// its shell's pid is written pre-exec and `exec` preserves it, so the
        /// recorded pid IS the tmux client — killing it detaches the
        /// renderer, and mosh-server, whose program it is, ends with it.
        static func tmuxRendererPidPath(connectionId: UUID, nonce: String) -> String {
            "$HOME/.moshpit/tmuxr-\(connectionId.uuidString)-\(nonce).pid"
        }

        /// mosh-server's program with tmux — the phone's renderer: tmux's
        /// client on the remembered session (`remembered`, tmux's `$N`) when
        /// it is still there, else the most recent one, and with no session
        /// at all it waits for one (the person makes it from the empty state,
        /// through the control plane), never makes one — crates/session's
        /// `mosh_tmux_program`. Under `sh -c` (B138), as mosh-server's own
        /// program (B267): typed into the mosh shell, a shell still starting
        /// swallowed the line, and the copies typed again landed in the pane
        /// as keystrokes once an earlier one had attached (B170), their
        /// control bytes acting before their quotes (B140).
        ///
        /// The pid is recorded first (``tmuxRendererPidPath(connectionId:nonce:)``):
        /// `exec` keeps it, so the file names the tmux client — how the
        /// control plane knows its renderer (``adoptRenderer(pid:)``), and
        /// what the next connect's cleanup stops
        /// (``tmuxRendererCleanupCommand(connectionId:)``): mosh-server
        /// survives disconnects by design, and an immortal phone-sized client
        /// per connect fought the desktop over `window-size latest` ("SSH 乱码",
        /// 2026-08-19). Detaching ends the mosh session, as the `exec tmux
        /// attach` the line ran did.
        static func tmuxRendererProgram(tmux: String, remembered: String?,
                                        connectionId: UUID, nonce: String) -> String {
            let pid = tmuxRendererPidPath(connectionId: connectionId, nonce: nonce)
            var script = "mkdir -p \"$HOME/.moshpit\"; echo $$ > \"\(pid)\"; "
            if let id = remembered, id.hasPrefix("$"), TmuxSessionController.isValidTmuxId(id) {
                let target = shellQuote(id)
                script += "if \(tmux) has-session -t \(target) 2>/dev/null; then exec \(tmux) attach -t \(target); fi; "
            }
            script += "until \(tmux) has-session 2>/dev/null; do sleep 1; done; exec \(tmux) attach"
            return "/bin/sh -c " + shellQuote(script)
        }

        /// A session target that matches `name` and nothing else.
        ///
        /// A bare name is only tmux's FIRST try: with no session of exactly
        /// that name, `-t` goes on to take the one session whose name starts
        /// with it, then a pattern — a remembered `dev` renamed or killed
        /// meanwhile, and the renderer or the sidecar attached the user's
        /// `devops` (tmux 3.6a, private socket: `-CC attach -t devo` lands on
        /// `devops`, `-t d*s` too; `-t =devo` is refused). The `=` prefix
        /// makes it exact; the attach then fails, which the attach line's
        /// `has-session` gate and the sidecar's fallback chain already
        /// handle.
        nonisolated static func tmuxExactSession(_ name: String) -> String {
            "=" + name
        }

        /// The SSH terminal's program: tmux control mode on the user's existing
        /// sessions (the most recent one), and nothing else to their server.
        /// With no session to attach (or no server), it waits for one — the
        /// person's "New session" from the empty state, made by exec
        /// (``createFirstTmuxSession(named:)``) — and attaches it; after tmux's
        /// client is gone (a detach, the server gone), the account's login
        /// shell takes the channel, as it had it when the attach was typed.
        ///
        /// The terminal channel's own program (`requestPTY(command:)`, run as
        /// `sh -c`), never a line typed into the login shell (B267): a shell
        /// still starting swallowed it — a 3 s zsh rc that flushes pending
        /// input, as Powerlevel10k's instant prompt does — and nothing
        /// attached. tmux is found as the probe finds it: a non-interactive
        /// shell's PATH lacks Homebrew's and `~/.local/bin`.
        ///
        /// Every control-client boot — this one, the mosh fallback's, the mosh
        /// sidecar's chain (``tmuxSidecarAttachChain(tmux:preferred:)``) — used
        /// to start with `tmux set -g history-limit 50000`: a global,
        /// persistent change to the user's own server, made on every attach,
        /// that LOWERED a limit the user had set higher (tmux 3.6a, private
        /// socket: 200000 → 50000, and every pane made after it got 50000).
        /// Moshpit has no need of it: a pane's backfill reads 400 lines of
        /// history (`backfillHistoryLines`), well inside tmux's default 2000,
        /// and the phone keeps its own scrollback of the live output. How
        /// much history the user's panes keep is the user's setting.
        nonisolated static func tmuxControlAttachProgram(tmux: String) -> String {
            "\(probedPath); \(tmux) -CC attach || { until \(tmux) has-session 2>/dev/null; do sleep 1; done; "
                + "exec \(tmux) -CC attach; }; \(loginShellAfter)"
        }

        /// The sidecar terminal's program: its attach chain
        /// (``tmuxSidecarAttachChain(tmux:preferred:)``) with tmux found as the
        /// probe finds it, then the account's login shell — the channel's
        /// own program, never typed into its shell (B267).
        nonisolated static func tmuxSidecarAttachProgram(tmux: String, preferred: SidecarReturn?) -> String {
            "\(probedPath); \(tmuxSidecarChain(tmux: tmux, preferred: preferred)); \(loginShellAfter)"
        }

        /// PATH for the programs: what the command gets, then the directories
        /// the capability probe searched.
        nonisolated private static var probedPath: String {
            "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\""
        }

        /// The account's login shell, in the place of a program of ours that
        /// is done: the channel stays as sshd would have opened it.
        nonisolated static let loginShellAfter = "exec \"${SHELL:-/bin/sh}\" -l"

        /// The session a rebuilt sidecar goes back to: the one the old
        /// sidecar was following, by id and by name.
        struct SidecarReturn: Equatable {
            /// tmux's `$N` — stable across a rename.
            var id: String
            var name: String?
        }

        /// What the sidecar's SSH shell runs to put it in control mode: the
        /// `preferred` session (a rebuild going back to where it was) by its
        /// id, then by its exact name, else — or when both are gone — the most
        /// recent session, retried every 2s while the server has none. Attach
        /// only: Moshpit never creates a session on its own.
        ///
        /// By id first: a session renamed while the app was suspended (the
        /// desktop's `rename-session`, an agent's) is no longer found by the
        /// name the sidecar knew — `-t '=work'` fails once `work` is `play`
        /// (tmux 3.6a) — and the rebuild fell through to the most recent
        /// session, someone else's as likely as not. tmux never reuses an id
        /// while its server lives; the name still finds a session made again
        /// under it (a server restarted since numbers its ids afresh).
        nonisolated static func tmuxSidecarAttachChain(tmux: String, preferred: SidecarReturn?) -> String {
            // Under `sh -c` (B138): typed into a fish login shell, the
            // `while … do … done` was a syntax error and the sidecar never
            // attached.
            Shell.portable(tmuxSidecarChain(tmux: tmux, preferred: preferred))
        }

        nonisolated private static func tmuxSidecarChain(tmux: String, preferred: SidecarReturn?) -> String {
            var chain = "while ! \(tmux) -CC attach 2>/dev/null; do sleep 2; done"
            if let name = preferred?.name {
                chain = "\(tmux) -CC attach -t \(shellQuote(tmuxExactSession(name))) 2>/dev/null || "
                    + chain
            }
            if let id = preferred?.id, id.hasPrefix("$"), TmuxSessionController.isValidTmuxId(id) {
                chain = "\(tmux) -CC attach -t \(shellQuote(id)) 2>/dev/null || " + chain
            }
            return chain
        }

        /// Kill any previous connect's renderer stack for this connection.
        static func tmuxRendererCleanupCommand(connectionId: UUID) -> String {
            let base = "$HOME/.moshpit/tmuxr-\(connectionId.uuidString)"
            return "for f in \"\(base)-\"*.pid; do "
                + "kill $(cat \"$f\" 2>/dev/null) 2>/dev/null; done; "
                + "rm -f \"\(base)-\"*.pid; true"
        }

        /// Replace the mosh+tmux control plane — its SSH connection and its
        /// `-CC` controller — back on the session it was following. The pins
        /// and agent stamps the old one held travel to the replacement, as in
        /// `stop()`.
        private func rebuildMoshTmuxSidecar() async {
            captureSelection()   // remember window/pane before rebuild
            let preferred = moshControl.flatMap { c in
                c.snapshot.activeSessionId.map { SidecarReturn(id: $0, name: c.snapshot.sessions[$0]?.name) }
            }
            if let control = moshControl {
                // Its pins travel to the rebuilt sidecar — see stop().
                owePins(control.takePinLedger())
                inheritedAgentHooks = control.takeAgentHooks() ?? inheritedAgentHooks
                await control.detach()
            }
            moshControl = nil
            if let ssh = sidecarSSH { await ssh.close() }
            sidecarSSH = nil
            await startMoshControlPlane(preferred: preferred)
            // Rebuilt sidecar = fresh controller with everAttached
            // false, so the card shows "Attaching…" again — bound
            // it with the same stall latch as the initial attach.
            if let control = moshControl { beginAttachTimeout(controller: control) }
            // The rebuilt sidecar starts at 80×24 again — re-pin the
            // window to the phone grid or the TUI snaps to the corner.
            await pinMoshWindow()
        }

        /// The mosh sidecar's own controller declared its `-CC` channel dead
        /// (`onChannelSilent`): commands pending and nothing coming back, its
        /// parser failed, or its stream ended.
        ///
        /// Nobody was listening. The SSH paths reconnect on it, but over mosh
        /// the only check was the keepalive's exec probe — a fresh channel on
        /// the same connection, which keeps passing while the `-CC` channel
        /// itself is stalled (a wedged tmux, a stuck channel), so the
        /// breadcrumb and the sheets froze for good. The mosh session is
        /// fine; only its control plane is rebuilt. A rebuild or resume
        /// already running picks it up through `moshSidecarSilent` (now, or
        /// at the next keepalive tick).
        func moshSidecarWentSilent(_ controller: TmuxSessionController) async {
            guard !isStopping, moshControl === controller, moshTransport != nil else { return }
            Log.ssh.notice("health: mosh sidecar dropped (tmux-channel-silent) — rebuilding the control plane")
            moshSidecarSilent = true
            guard !isResuming, !isReconnecting else { return }
            isResuming = true
            defer { isResuming = false }
            await rebuildMoshTmuxSidecar()
        }

        /// Open the control-plane SSH session and run a `tmux -CC` control
        /// client against the user's existing sessions. The control client
        /// stays sizeless (`rendersOutput: false`), so it never receives pane
        /// output and never affects window sizing; it exists purely to push
        /// live session/window/pane state into the native breadcrumb + sheets.
        private func startMoshControlPlane(preferred: SidecarReturn? = nil) async {
            // No TOFU handlers are wired here on purpose: this sidecar dials
            // the SAME `connection.host`/`connection.port` as the mosh
            // bootstrap's primary SSH connection, which — every call path
            // into this function (startMosh's initial attach, and
            // resumeIfNeeded's rebuild) — has already completed (and had its
            // host key judged) strictly before this runs. `HostKeyValidator`
            // is keyed by host:port and persists the trust decision, so
            // `connect()`'s decision here always resolves to `.trusted`
            // without either handler firing. Leaving both at `connect()`'s
            // default (deny) is intentional defense-in-depth: if that
            // invariant is ever violated by a future change, the sidecar
            // fails closed (no control-plane breadcrumb) instead of silently
            // auto-trusting a host key nobody actually confirmed.
            guard let ssh = try? await sidecarConnect(connection) else { return }
            // The session may have been torn down (stop()) while this SSH
            // handshake was in flight — don't wire a fresh connection onto a
            // dead session (leaked, never-closed, and racing the NEW
            // session's own bootstrap over this same shared SSHService).
            guard !isStopping else { await ssh.close(); return }
            // -CC needs a controlling terminal; the PTY size is irrelevant
            // (control clients are excluded from sizing until they report one,
            // which we never do). Its program is the attach chain: the
            // preferred session (resume) or the most recent one, retried
            // while the server has none — attach only, Moshpit never creates
            // sessions on its own, and sets nothing on the user's server. Never
            // a line typed into the login shell (B267): a shell still
            // starting swallowed it and the sidecar never attached.
            let tmux = connection.tmuxPath ?? "tmux"
            do {
                try await ssh.requestPTY(rows: 24, cols: 80,
                                         command: Self.tmuxSidecarAttachProgram(tmux: tmux, preferred: preferred))
            } catch {
                await ssh.close()
                return
            }
            guard !isStopping else { await ssh.close(); return }
            sidecarSSH = ssh

            let controller = TmuxSessionController(sshSession: ssh, rendersOutput: false)
            // The person's "New session" before the first attach: an exec on
            // this connection (B228).
            controller.createsFirstSession = { [weak self, weak controller] name in
                Task { @MainActor [weak self, weak controller] in
                    await self?.createFirstMoshSession(named: name, notifying: controller)
                }
            }
            // A new control plane: whatever was said about the last one is
            // settled. Its own watchdog tells on it the way the SSH paths'
            // does — see `moshSidecarWentSilent`.
            moshSidecarSilent = false
            controller.onChannelSilent = { [weak self, weak controller] in
                Task { @MainActor [weak self] in
                    guard let self, let controller else { return }
                    await self.moshSidecarWentSilent(controller)
                }
            }
            if let ledger = inheritedPinLedger {
                controller.inheritPinLedger(ledger)
                inheritedPinLedger = nil
            }
            if let hooks = inheritedAgentHooks {
                controller.inheritAgentHooks(hooks)
                inheritedAgentHooks = nil
            }
            controller.pendingRestore = lastSelection   // restore window/pane too
            if let listing = moshTmuxLayoutsBeforeRenderer {
                moshTmuxLayoutsBeforeRenderer = nil
                controller.noteLayoutsBeforeRenderer(listing)
            }
            await controller.beginControlMode()
            moshControl = controller
            controlPlaneWired(controller)
        }

        /// Bound the wait for `controller`'s tmux control-mode attach to
        /// confirm (`snapshot.isAttached`, flipped by `%session-changed`).
        /// Polls rather than requiring a completion callback from
        /// `TmuxSessionController` — cheap, and it means success is detected
        /// (and the watcher exits) within one poll tick with no separate
        /// cancellation wiring needed on the controller side. Checks the LIVE
        /// snapshot at the end too, so an attach that lands just as the
        /// deadline passes is still treated as success, not a false-positive
        /// timeout. Called at every fresh boot of a `-CC attach` line — the
        /// tmux-over-SSH call sites AND the mosh sidecar's control plane
        /// (initial attach + resume rebuild); clears any stale watcher/notice
        /// from a previous cycle first.
        private func beginAttachTimeout(controller: TmuxSessionController) {
            attachTimeoutTask?.cancel()
            attachStalled = false
            let deadline = Date().addingTimeInterval(attachTimeoutSeconds)
            attachTimeoutTask = Task { [weak self, weak controller] in
                while Date() < deadline {
                    if Task.isCancelled { return }
                    guard let controller, !controller.snapshot.isAttached else { return }
                    try? await Task.sleep(nanoseconds: 500_000_000)
                }
                guard !Task.isCancelled, controller?.snapshot.isAttached == false else { return }
                // A server with no session isn't a stalled attach: it waits
                // for the person's first session (B228).
                if self?.awaitingFirstSession != true { self?.attachStalled = true }
                // An attach that lands after all lifts the notice — a mosh
                // server that had no session until the person made one
                // (B212), a host slower than the bound. It used to stay up
                // over the tree it had been waiting for. Nothing held while
                // waiting: a controller let go ends the wait.
                while !Task.isCancelled, controller?.snapshot.isAttached == false {
                    try? await Task.sleep(nanoseconds: 500_000_000)
                }
                if !Task.isCancelled, controller?.snapshot.isAttached == true {
                    self?.attachStalled = false
                }
            }
        }

        /// Tear the transport down.
        ///
        /// `forReconnect` says the caller is about to `start()` again on a
        /// transport it has already judged dead (or is replacing anyway). Two
        /// things change: no courtesy round trip is spent on the old
        /// connection — measured on the rig, a half-open socket held the
        /// tmux unpin exec for Citadel's full 15s channel-open timeout, which
        /// was the whole of "reconnect is slow" — and the tmux controller is
        /// kept as ``retiredTmuxController`` so its last frame stays on screen
        /// until the replacement paints. The pins it owed the server travel
        /// with it (``inheritedPinLedger``); the replacement re-pins the same
        /// window on attach and settles the rest on the real disconnect.
        func stop(forReconnect: Bool = false) async {
            // An attempt still on its way is nobody's from here (`isCurrent`),
            // and neither is what was typed for it.
            startGeneration += 1
            discardHeldInput()
            if let rust { await rust.stop(); return }
            let stopBegan = Date()
            defer { Log.ssh.debug("resume: stop(forReconnect: \(forReconnect, privacy: .public)) took \(Int(Date().timeIntervalSince(stopBegan) * 1000), privacy: .public)ms") }
            if forReconnect {
                cancelFirstFrameWait()
                // Retire the ghost BEFORE the first suspension point: the
                // screen already reads `.reconnecting`, and with no ghost in
                // place it reaches for the poster for as long as the teardown
                // takes — seen on the rig as a faint poster flashing through
                // the frozen frame. Only a frame with content on it qualifies,
                // and never over an older ghost (see the property).
                if let controller = tmuxController, controller.hasPaintedSinceAttach,
                   retiredTmuxController == nil {
                    retiredTmuxController = controller
                }
            } else {
                endFirstFrameWait()
            }
            captureSelection()   // remember where we were, for the next attach
            pumpTask?.cancel()
            pumpTask = nil
            attachTimeoutTask?.cancel()
            attachTimeoutTask = nil
            moshTmuxAttachTask?.cancel()
            moshTmuxAttachTask = nil
            if let controller = tmuxController {
                if forReconnect {
                    // Not one byte more on this transport — see the doc
                    // comment. The debt moves to the replacement.
                    owePins(controller.takePinLedger())
                    inheritedAgentHooks = controller.takeAgentHooks() ?? inheritedAgentHooks
                } else if let ssh = viewModel.session {
                    // Hand back the pinned window sizes so other clients get
                    // their full width back. Over a one-shot EXEC channel
                    // (executeCommand awaits server completion) rather than the
                    // in-band -CC write, which raced the teardown and left
                    // windows stuck at the phone size (`resize-window -A`
                    // clears the manual size override; tmux then re-sizes to
                    // the desktop client). The split layout our pin squashed
                    // (and the zoom that hid it) go back over the same channel,
                    // strictly AFTER the unpin: replayed while the window is
                    // still pinned, the layout is just refitted to the phone
                    // grid and the restore is lost. See
                    // TmuxSessionController.pristineLayouts.
                    let tmux = connection.tmuxPath ?? "tmux"
                    // Unset the per-window override — the real "back to
                    // automatic" (`resize-window -A` re-pins, still manual) —
                    // and the layouts, each window's only while it is still
                    // in a layout of ours (round thirty-four): one script
                    // that reads them all before anything changes a size.
                    var commands = controller.pinHandBackScript(tmux: tmux, first: []).map { [$0] } ?? []
                    // And the size lease: another device following this
                    // window must learn it is free without waiting for our
                    // client to be noticed missing.
                    commands += controller.sizeLeaseReleaseCommands().map { "\(tmux) \($0)" }
                    await Self.runCourtesy(commands, over: ssh)
                }
                await controller.detach()
            }
            tmuxController = nil
            if let control = moshControl {
                if forReconnect {
                    owePins(control.takePinLedger())
                    inheritedAgentHooks = control.takeAgentHooks() ?? inheritedAgentHooks
                } else {
                    // Restore the status bar over the LIVE control channel (a
                    // round-trip blocks until tmux applies it). The exec channel
                    // mangled the `$id` session target through the login shell,
                    // so status never came back; the control channel speaks
                    // straight to tmux like the suppression did.
                    await control.restoreSuppressedStatusAndFlush()
                    // Restore window sizes over a ONE-SHOT exec channel on the
                    // sidecar SSH connection (these target windows by `@id`,
                    // which survives the shell), then the layout/zoom — same
                    // order and same reasons as the SSH path above.
                    if let ssh = sidecarSSH {
                        let tmux = connection.tmuxPath ?? "tmux"
                        // The phone's renderer out first (B270): attached,
                        // it sizes the windows straight back to the phone
                        // when they go back to automatic sizing. The session
                        // is ending; so is its mosh.
                        let renderer = control.rendererDetachCommand(tmux: tmux).map { [$0] } ?? []
                        let commands = control.pinHandBackScript(tmux: tmux, first: renderer).map { [$0] } ?? []
                        await Self.runCourtesy(commands, over: ssh)
                    }
                }
                await control.detach()
            }
            moshControl = nil
            // The session's real end: pins a retired controller owed that no
            // controller took over are handed back too (B167).
            if !forReconnect { await payOwedPins() }
            if let ssh = sidecarSSH {
                await ssh.close()
            }
            sidecarSSH = nil
            if let ssh = onDemandTransferSSH {
                await ssh.close()
            }
            onDemandTransferSSH = nil
            if let transport = moshTransport {
                await transport.close()
            }
            moshTransport = nil
            coordinator.attachPrediction(nil)
            // The mosh-server this session started, stopped when nothing can
            // come back to it (`MoshServerReap.onTeardown`): one the user
            // closed — disconnected, switched to SSH, reconnected afresh —
            // contacted or not (B164: the app keeps no key, so it can never
            // be resumed), and at a reconnect only one never reached (also
            // the fallback for a server `onReturnPathDead`'s watchdog never
            // got to — see `moshServerPid`'s doc comment for the window). A
            // session the server ended has nothing left to stop.
            // `withTimeoutValue` bounds it so a host that's gone dark can't
            // hang teardown; `sidecarConnect`/`connection` are copied out
            // rather than capturing `self` into the escaping closure.
            let teardown: MoshServerReap.Teardown =
                forReconnect ? .reconnect : (viewModel.endedByServer ? .ended : .userClosed)
            if let pid = moshServerPid,
               MoshServerReap.onTeardown(teardown, contacted: moshHadFirstContact) {
                moshServerPid = nil   // best-effort, once — a second kill is harmless but pointless
                let reap = sidecarConnect
                let target = connection
                _ = await withTimeoutValue(4) {
                    if let ssh = try? await reap(target) {
                        _ = try? await ssh.executeCommand("kill \(pid) 2>/dev/null")
                        await ssh.close()
                    }
                }
            }
            await viewModel.disconnect()
        }

        /// Whether start attempt `attempt` still owns this session: no stop
        /// and no newer attempt since it began. Every await in the start paths
        /// (`start`, `startMosh`, `fallbackToSSH`) resumes into this check,
        /// and an attempt that fails it lets go of what it made instead of
        /// wiring it up. They checked the dial alone: a disconnect during the
        /// capability probe still got a tmux controller, its timers and its
        /// monitor (B122); one while mosh-server was starting left a transport
        /// running and a server nobody reaped; and an attempt a redial had
        /// replaced could mark the new one failed or install a second
        /// transport over it (B168). Every stop is a generation of its own.
        private func isCurrent(_ attempt: Int) -> Bool {
            attempt == startGeneration && !isStopping
        }

        /// A tmux controller its attempt never installed: what it took over —
        /// the retired controller's pins and agent stamps — goes back to the
        /// session (and is handed to the server now if the session is over),
        /// and it lets go of the channel.
        private func abandon(_ controller: TmuxSessionController, over session: SSHSession) async {
            owePins(controller.takePinLedger())
            inheritedAgentHooks = controller.takeAgentHooks() ?? inheritedAgentHooks
            await controller.detach()
            if isStopping { await payOwedPins(preferring: session) }
        }

        /// Stops the mosh-server an attempt started and was superseded before
        /// it could tell the session its pid: nothing else ever will (B168).
        /// Over the bootstrap's own connection while it is still open — no
        /// new sign-in — else over one of its own, bounded.
        private func reapAbandonedServer(_ pid: Int?, over ssh: SSHSession) async {
            guard let pid else { return }
            let command = "kill \(pid) 2>/dev/null"
            if await ssh.isConnected, (try? await ssh.executeCommand(command)) != nil { return }
            let dial = sidecarConnect
            let target = connection
            _ = await withTimeoutValue(4) {
                guard let reaper = try? await dial(target) else { return }
                _ = try? await reaper.executeCommand(command)
                await reaper.close()
            }
        }

        /// Add `ledger` to the pins this session still owes the server —
        /// never in their place. Every retirement used to assign it, so a
        /// second one before a controller took the first debt over dropped
        /// that debt for good (B167).
        private func owePins(_ ledger: TmuxSessionController.PinLedger) {
            guard let owed = inheritedPinLedger, !owed.isEmpty else {
                inheritedPinLedger = ledger
                return
            }
            guard !ledger.isEmpty else { return }
            // Through a controller: the ledger's records are its business.
            // The older debt first — a window's pristine layout must predate
            // its first pin, and the first record of it wins.
            let tally = Self.pinTally()
            tally.inheritPinLedger(owed)
            tally.inheritPinLedger(ledger)
            inheritedPinLedger = tally.takePinLedger()
        }

        /// Hand back pins a retired tmux controller owed that no controller
        /// took over — the redial failed, or landed on a plain shell — at the
        /// session's real end: over its own connection while it has a live
        /// one, else over a connection of their own (crates/session's
        /// `pay_pin_debt`). They used to be dropped: after a drop, a failed
        /// redial and a disconnect, the desktop's windows stayed at the
        /// phone's size (B167).
        private func payOwedPins(preferring held: SSHSession? = nil) async {
            guard let owed = inheritedPinLedger else { return }
            inheritedPinLedger = nil
            guard !owed.isEmpty else { return }
            let tally = Self.pinTally()
            tally.inheritPinLedger(owed)
            let tmux = connection.tmuxPath ?? "tmux"
            // The phone's mosh renderer out first (B270), then unpin, then the
            // layouts — see stop()'s own courtesy.
            let renderer = tally.rendererDetachCommand(tmux: tmux).map { [$0] } ?? []
            let commands = tally.pinHandBackScript(tmux: tmux, first: renderer).map { [$0] } ?? []
            guard !commands.isEmpty else { return }
            for ssh in [held, viewModel.session].compactMap({ $0 }) where await ssh.isConnected {
                await Self.runCourtesy(commands, over: ssh)
                return
            }
            let dial = sidecarConnect
            let target = connection
            _ = await withTimeoutValue(8) {
                guard let ssh = try? await dial(target) else { return }
                for command in commands { _ = try? await ssh.executeCommand(command) }
                await ssh.close()
            }
        }

        /// A controller that never connects, for reading and merging pin
        /// ledgers.
        private static func pinTally() -> TmuxSessionController {
            TmuxSessionController(sshSession: DetachedTmuxTransport(), rendersOutput: false)
        }

        /// Teardown round trips the server is owed but nobody is waiting on
        /// — bounded as a whole, so a socket that died between the decision
        /// to disconnect and this line can't hold the teardown hostage (each
        /// exec on a half-open socket otherwise waits out Citadel's 15s).
        private static func runCourtesy(_ commands: [String], over ssh: SSHSession) async {
            guard !commands.isEmpty else { return }
            _ = await withTimeoutValue(3) {
                for command in commands { _ = try? await ssh.executeCommand(command) }
            }
        }

        /// Start the first-frame wait for a freshly installed controller —
        /// see ``awaitingFirstFrame``. Ends on the controller's first revealed
        /// frame, or on the fallback deadline for a server that never paints.
        ///
        /// The deadline is not a plain timer. It first waits the fallback,
        /// then keeps waiting for as long as ``firstFrameWaitHolds`` says the
        /// silence is the LINK's and not the server's: on a slow cellular path
        /// the SSH connect alone can eat the budget, and a deadline that then
        /// dropped the ghost over a still-blank pane left the next drop with
        /// no frame to keep — the poster came back on every second reconnect
        /// (phone over 5G, 2026-09-15).
        private func armFirstFrameWait(for controller: TmuxSessionController) {
            awaitingFirstFrame = true
            controller.onFirstFramePainted = { [weak self] in
                Log.ssh.debug("resume: first frame revealed")
                self?.endFirstFrameWait()
            }
            firstFrameFallback?.cancel()
            let armedAt = Date()
            firstFrameFallback = Task { [weak self, weak controller] in
                try? await Task.sleep(for: .seconds(self?.firstFrameFallbackSeconds ?? 0))
                while !Task.isCancelled, let self, let controller,
                      self.firstFrameWaitHolds(controller, armedAt: armedAt) {
                    try? await Task.sleep(for: .milliseconds(250))
                }
                guard !Task.isCancelled else { return }
                Log.ssh.debug("resume: no frame revealed in time — dropping the cover")
                self?.endFirstFrameWait()
            }
        }

        /// Whether the first-frame fallback, past its deadline, should keep
        /// waiting rather than drop the cover:
        /// - attached: the pane is hosted and its first capture is in flight,
        ///   so give it the fallback measured from the ATTACH;
        /// - not attached, but the server has answered the boot line and gone
        ///   quiet for ``firstFrameAnswerSettleSeconds``: the attach was
        ///   refused (no sessions / no server) — the empty-state card behind
        ///   the cover is the thing to show, drop. A fresher answer may be the
        ///   attach itself, still on its way to the main actor: hold;
        /// - not attached and not a byte back since the boot line: the link
        ///   is slow or dead, hold — up to the attach bound, past which the
        ///   attach timeout has latched `attachStalled` and holding a frozen
        ///   frame stops being honest.
        func firstFrameWaitHolds(_ controller: TmuxSessionController, armedAt: Date,
                                 now: Date = Date()) -> Bool {
            if let attachedAt = controller.attachedAt {
                return now.timeIntervalSince(attachedAt) < firstFrameFallbackSeconds
            }
            let withinBound = now.timeIntervalSince(armedAt) < firstFrameAttachBoundSeconds
            let answeredAt = controller.lastInboundAt
            if answeredAt > armedAt {
                return withinBound && now.timeIntervalSince(answeredAt) < firstFrameAnswerSettleSeconds
            }
            return withinBound
        }

        /// Stop waiting AND drop the ghost: the replacement has painted (or
        /// never will), so the frozen frame has nothing left to stand in for.
        private func endFirstFrameWait() {
            cancelFirstFrameWait()
            retiredTmuxController = nil
            // A tmux attach's first frame (or the wait's end): the decision
            // the held input waited for. Nothing is held on other paths.
            releaseHeldInput()
        }

        /// Hand on what was typed while the attach was undecided
        /// (`heldInput`), in order, to where typing goes now: a plain
        /// session's shell, or the active pane of an attached tmux. With tmux
        /// never attached (no session, a refusal) there is no such place, and
        /// it is dropped.
        private func releaseHeldInput() {
            guard let held = heldInput else { return }
            heldInput = nil
            guard !held.isEmpty else { return }
            if let controller = tmuxController, !controller.snapshot.isAttached {
                Log.input.info("input typed before the attach dropped: tmux never attached")
                return
            }
            for data in held { routeInput(data) }
        }

        /// Whether input may go out now: nothing is held, or what is held
        /// waited for a tmux attach that has landed — then it goes first, so
        /// the order typed stands (it would otherwise wait for the first
        /// frame, and a keystroke or paste after the attach overtake it).
        private func inputDecided() -> Bool {
            guard heldInput != nil else { return true }
            guard let controller = tmuxController, controller.snapshot.isAttached else { return false }
            releaseHeldInput()
            return true
        }

        /// Drop what is held: the attempt is torn down, or what it is
        /// becoming takes no input on the way.
        private func discardHeldInput() {
            if let held = heldInput, !held.isEmpty {
                Log.input.info("input typed before the attach dropped: \(held.count, privacy: .public) writes")
            }
            heldInput = nil
        }

        /// Stop waiting but keep the ghost: the attempt being torn down is
        /// about to be replaced by another, which re-arms the wait itself.
        private func cancelFirstFrameWait() {
            firstFrameFallback?.cancel()
            firstFrameFallback = nil
            awaitingFirstFrame = false
        }

        /// The transport died on its own — flag the UI and reconnect.
        /// `reason` is for the log, not for logic: when a user reports "the
        /// connection is unstable", the only useful question is WHICH of the
        /// several death detectors fired, and that is unanswerable after the
        /// fact unless each one says so at the time.
        func connectionDropped(reason: String = "unspecified") async {
            guard rust == nil else { return }   // the core redials on its own
            guard !isStopping else { return }
            Log.ssh.notice("health: dropped (\(reason, privacy: .public)) — reconnecting")
            viewModel.markReconnecting()
            await reconnect()
        }

        /// Periodic keepalive + health check (driven by the hub timer while the
        /// app is foreground). Doubles as NAT-warmth (the probe is real
        /// traffic) and fast death detection, and retries a stalled reconnect.
        func keepAlive() async {
            guard rust == nil else { return }   // the core keeps its own alive
            guard !isStopping, !isReconnecting else { return }
            // mosh owns its own UDP resume/roam; don't probe its closed bootstrap.
            if moshTransport != nil { await resumeIfNeeded(); return }
            switch viewModel.status {
            case .connected:
                // Bytes arrived recently: the connection is demonstrably
                // working, so don't spend a channel asking.
                guard inboundAge() >= Self.inboundProofWindow else {
                    probeTimeouts = 0
                    return
                }
                switch await probeTransport() {
                case .alive:
                    probeTimeouts = 0
                case .failed:
                    Log.ssh.notice("health: probe failed — reconnecting")
                    probeTimeouts = 0
                    viewModel.markReconnecting()
                    await reconnect()
                case .timedOut:
                    probeTimeouts += 1
                    guard probeTimeouts >= 2 else {
                        // One silent probe is not proof of death. Hold the
                        // session and ask again on the next tick; a genuinely
                        // dead socket errors rather than going quiet, and that
                        // path above still reconnects immediately.
                        Log.ssh.notice("health: probe timed out (1) — holding")
                        return
                    }
                    Log.ssh.notice("health: probe timed out twice — reconnecting")
                    probeTimeouts = 0
                    viewModel.markReconnecting()
                    await reconnect()
                }
            case .failed:
                // A refused host key or sign-in waits for the user: retried
                // here, the card came back and the server was sent the wrong
                // password every tick (B163).
                guard viewModel.failureRetryable else { return }
                await reconnect()   // retry on the next tick (acts as backoff)
            case .reconnecting, .disconnected:
                await reconnect()   // retry on the next tick (acts as backoff)
            case .idle, .connecting:
                break               // initial connect in flight — leave it
            case .ended:
                break               // the server ended it: the user's call
            }
        }

        /// Tear down the dead transport and run the start flow again. tmux
        /// `attach` reattaches to the same server-side session, so panes + their
        /// scrollback come right back.
        ///
        /// `automatic` is the attempt's provenance for the error surface (see
        /// `TerminalViewModel.beginAttempt`): a transport that dropped on its
        /// own is redialed as automatic, while a redial of a connect the user
        /// asked for keeps its right to a modal if it fails too.
        func reconnect(automatic: Bool = true) async {
            guard rust == nil else { return }
            guard !isStopping, !isReconnecting, !isResuming else { return }
            isReconnecting = true
            defer { isReconnecting = false }
            await stop(forReconnect: true)
            viewModel.resetForReconnect()
            await start(theme: lastTheme, fontSize: lastFontSize, fontName: lastFontName,
                        cursorShape: lastCursorShape, cursorColorId: lastCursorColorId,
                        cursorBlink: lastCursorBlink, automatic: automatic)
        }

        /// Unconditional resume after a long suspension — skip the (unreliable
        /// on a half-open socket) probe and just reconnect. mosh keeps its own
        /// UDP resume path.
        func forceResume() async {
            guard rust == nil else { return }   // the core's own foreground rules
            guard !isStopping else { return }
            Log.ssh.debug("resume: forceResume begin")
            if let transport = moshTransport {
                // A REAL suspension (>20s background). The UDP flow may be a
                // zombie — .ready in name, blackhole in practice — so replace
                // it outright (roam-equivalent, server re-homes on the first
                // flush) before the sidecar rebuild runs.
                await transport.resume(force: true)
                await resumeIfNeeded()
                return
            }
            // However long the app was away, a refused host key or sign-in
            // is still the user's to retry (B163).
            if case .failed = viewModel.status, !viewModel.failureRetryable { return }
            // A dial that never landed is the same attempt continued (see
            // retryIfDown); a transport that dropped is redialed as automatic.
            let automatic = isDialing ? viewModel.isAutoReconnectInFlight : true
            viewModel.markReconnecting()
            await reconnect(automatic: automatic)
        }

        /// Foreground-resume. iOS kills TCP while the app is suspended, so an
        /// SSH/tmux session that looks alive may be a corpse — probe it and
        /// rebuild transparently (tmux's `new -A` re-attaches to the same
        /// server-side session; fresh pane terminals re-backfill history).
        /// mosh just restarts its UDP flow.
        ///
        /// Reentrancy-guarded by `isResuming` (see its doc comment) — both the
        /// mosh sidecar rebuild and the plain-SSH tear-down+restart below have
        /// several `await` points, and `setForeground(true)`'s `resumeAll`
        /// plus the periodic keepalive tick can both call this for the same
        /// session.
        func resumeIfNeeded(probeTimeout: Double? = nil) async {
            guard rust == nil else { return }
            guard !isStopping, !isReconnecting, !isResuming else { return }
            // Nothing up yet, or nothing up any more: there is no transport
            // to probe, so redial / reconnect now instead of returning.
            if await retryIfDown(stalledDialAfter: stalledDialGrace, reason: "foreground") { return }
            isResuming = true
            defer { isResuming = false }
            if let transport = moshTransport {
                await transport.resume()
                // The tmux control plane rides SSH, which iOS kills during
                // suspension — rebuild it.
                if connection.multiplexer == .tmux {
                    // Rebuild the -CC sidecar when its SSH is dead OR it never
                    // came back from a previous attempt (moshControl == nil):
                    // the breadcrumb is driven entirely by this sidecar's
                    // snapshot, so a failed re-establish would otherwise leave it
                    // blank until the app restarts. Since keepalive funnels
                    // through here, a nil controller is retried every cycle —
                    // and so is a controller whose own watchdog declared its
                    // channel dead while a pass like this one was running.
                    let alive = await isSidecarAlive(timeout: probeTimeout)
                    if !alive || moshControl == nil || moshSidecarSilent {
                        await rebuildMoshTmuxSidecar()
                    }
                }
                return
            }
            guard viewModel.session != nil else { return }
            // `isConnected` is just a local `!closed` flag — iOS silently kills
            // the TCP socket during suspension without NIO seeing a close, so it
            // lies (home shows "connected" but -CC can't attach). Actively
            // round-trip a trivial command with a short timeout instead.
            if await isTransportAlive(timeout: probeTimeout) { return }

            // Dead transport: tear down without touching the UI layer, then
            // run the exact same start flow (tmux `new -A` reuses the session).
            await stop(forReconnect: true)
            viewModel.resetForReconnect()
            await start(theme: lastTheme, fontSize: lastFontSize, fontName: lastFontName,
                        cursorShape: lastCursorShape, cursorColorId: lastCursorColorId,
                        cursorBlink: lastCursorBlink, automatic: true)
        }

        /// True if a trivial command round-trips within `timeout`. A dead
        /// half-open socket either errors fast or never replies — both ⇒ dead.
        ///
        /// The timeout must absorb a genuinely slow link, not just a healthy
        /// LAN: an exec round-trip is ~3 protocol round-trips, so a 500–700ms
        /// RTT path (measured against a Tailscale host relayed through a
        /// cellular egress) needs ~2s when nothing is lost, and one TCP
        /// retransmit pushes past 2.5s. A false "dead" here is the expensive
        /// case — it tears down a working session and forces a full SSH
        /// re-handshake (6–9s on that same link). 8s stays under the 12s
        /// keepalive tick while making slow-link false positives vanishingly
        /// rare; truly dead sockets still fail fast (write error / RST).
        /// Wall clock of the last bytes this connection delivered, from any of
        /// its pumps. A connection that is demonstrably carrying data does not
        /// need to be asked whether it is alive — and asking costs a whole
        /// fresh exec channel every 12s on top of whatever the control plane
        /// is already doing, which is real load on
        /// exactly the flaky links where it hurts most.
        @ObservationIgnored private var lastInboundAt: Date?

        /// Stamp inbound traffic. Called from every pump that reads this
        /// connection; cheap enough to call per chunk.
        func noteInbound() { lastInboundAt = Date() }

        /// Seconds since this connection last delivered anything, across the
        /// hub's own pumps AND the tmux -CC controller's (which has its own
        /// stamp, written from a detached pump).
        func inboundAge() -> TimeInterval {
            var newest = lastInboundAt ?? .distantPast
            if let controlStream = tmuxController?.lastInboundAt, controlStream > newest {
                newest = controlStream
            }
            return Date().timeIntervalSince(newest)
        }

        /// Traffic newer than this counts as proof of life, so the probe is
        /// skipped entirely. Just under the keepalive tick, so a connection
        /// that goes quiet is still probed on the very next one.
        private static let inboundProofWindow: TimeInterval = 11

        /// Consecutive probe TIMEOUTS (not failures) — see `keepAlive`.
        @ObservationIgnored private var probeTimeouts = 0

        /// Probe deadline for `isTransportAlive`/`isSidecarAlive`. 8s absorbs
        /// a genuinely slow link (see the rationale above `isTransportAlive`);
        /// lifecycle tests shrink it so a scripted hang resolves in
        /// milliseconds instead of a wall-clock wait.
        @ObservationIgnored var probeTimeout: Double = 8

        /// When the current attempt began dialing — see `isDialing`.
        @ObservationIgnored private(set) var dialBegan: Date?

        /// Numbers `start()` attempts, so an attempt whose dial was superseded
        /// by a redial (see `TerminalViewModel.dialGeneration`) stops at the
        /// first check instead of wiring up the replacement's session again.
        @ObservationIgnored private var startGeneration = 0

        /// The attempt is still waiting on its TCP/SSH dial: no transport of
        /// any kind has come up. This is the state a SYN into a black hole
        /// leaves us in — a Tailscale address with the tunnel down, a host
        /// behind a Wi-Fi that just went away — and it lasts the whole
        /// connect timeout, because nothing on the wire ever says no.
        var isDialing: Bool {
            guard case .connecting = viewModel.status else { return false }
            return viewModel.session == nil && moshTransport == nil
        }

        /// How long a dial must have been out before a foreground return
        /// restarts it. Bouncing to another app for a second mid-handshake on
        /// a slow link should not cost the handshake; tests shrink it.
        @ObservationIgnored var stalledDialGrace: TimeInterval = 2

        /// The "nothing is up" half of coming back. A dial that has been out
        /// for `stalledDialAfter` is redialed on the spot; a session whose
        /// transport died is reconnected now rather than on the next keepalive
        /// tick. Both are what the user expects after fixing the network — the
        /// tunnel is up, the Wi-Fi is back — and neither used to happen: the
        /// resume path only knew how to probe a transport it already had, and
        /// the tick left a dial alone for as long as it took (30s into a black
        /// hole, then 12s to the tick). Returns true when it acted, so the
        /// caller skips its probe.
        @discardableResult
        func retryIfDown(stalledDialAfter: TimeInterval, reason: String) async -> Bool {
            if rust != nil { return true }   // the core heard the same change
            guard !isStopping, !isReconnecting, !isResuming else { return false }
            if isDialing {
                let age = dialBegan.map { Date().timeIntervalSince($0) } ?? .infinity
                guard age >= stalledDialAfter else { return false }
                Log.ssh.notice("resume: redial (\(reason, privacy: .public)) after \(Int(age), privacy: .public)s dialing")
                // The same attempt on a fresh socket — see `reconnect(automatic:)`.
                await reconnect(automatic: viewModel.isAutoReconnectInFlight)
                return true
            }
            // mosh keeps its own UDP resume path; its sidecar rebuild lives in
            // resumeIfNeeded's transport branch and must not be preempted.
            guard moshTransport == nil else { return false }
            switch viewModel.status {
            case .failed where !viewModel.failureRetryable:
                // Nothing to do about it but ask the user again (B163).
                return false
            case .failed, .disconnected:
                Log.ssh.notice("resume: retry (\(reason, privacy: .public)) of a down session")
                viewModel.markReconnecting()
                await reconnect()
                return true
            default:
                return false
            }
        }

        /// The device was unlocked: a session the locked keychain stopped
        /// (``TerminalViewModel/failureWaitsForUnlock`` — the saved
        /// credential or the trusted host keys couldn't be read) dials again
        /// now, as crates/session's `Lifecycle::unlocked` does (B211).
        /// Whatever else is down keeps to its own rules: the keepalive tick,
        /// the foreground return, the person. Whether it acted.
        @discardableResult
        func retryAfterUnlock() async -> Bool {
            if rust != nil { return false }   // the core hears the unlock itself (RustCore)
            guard !isStopping, !isReconnecting, !isResuming else { return false }
            guard case .failed = viewModel.status, viewModel.failureWaitsForUnlock else { return false }
            Log.ssh.notice("resume: redial (unlock) of a session the locked keychain stopped")
            viewModel.markReconnecting()
            await reconnect()
            return true
        }

        /// Probe deadline for the foreground-return check specifically. A
        /// socket iOS held through a suspension is dead far more often than
        /// one the periodic tick is asking about, and the person is watching
        /// the screen — so the wait for silence is shorter here. Still wide
        /// enough for a 500–700ms RTT path's three round trips plus one
        /// retransmit (see `isTransportAlive`'s arithmetic).
        @ObservationIgnored var foregroundProbeTimeout: Double = 4

        func isTransportAlive(timeout: Double? = nil) async -> Bool {
            if let rust { return rust.isLive }
            return await probeTransport(timeout: timeout) == .alive
        }

        /// What a liveness probe actually found. The distinction matters:
        /// a dead socket ERRORS (write failure, RST) and that is proof, while
        /// a timeout is only an absence of evidence — and on a lossy link the
        /// absence is routine. Treating the two the same is what makes a
        /// working session get torn down and re-handshaked (6-9s on the link
        /// where it happens) because one probe was slow.
        enum ProbeResult { case alive, failed, timedOut }

        func probeTransport(timeout: Double? = nil) async -> ProbeResult {
            if let rust { return rust.isLive ? .alive : .failed }
            guard let ssh = viewModel.session else { return .failed }
            return await Self.probe(ssh, timeout: timeout ?? probeTimeout)
        }

        /// The probe itself, over any session — a trivial command and a
        /// deadline. Static so a test can drive all three outcomes without
        /// standing up a whole connection.
        static func probe(_ ssh: SSHSession, timeout: Double) async -> ProbeResult {
            let answered = await withTimeoutValue(timeout) { () -> Bool in
                do { _ = try await ssh.executeCommand("true"); return true } catch { return false }
            }
            switch answered {
            case .some(true): return .alive
            case .some(false): return .failed
            case .none: return .timedOut
            }
        }

        /// Same real round-trip probe, aimed at the mosh sidecar. This must
        /// never be `sidecarSSH?.isConnected`: that is a local `!closed` flag
        /// and iOS kills TCP during suspension without NIO seeing a close, so
        /// after a LONG background it reports a corpse as alive — the rebuild
        /// in `resumeIfNeeded` then never fires and the breadcrumb/sheets stay
        /// dead until app restart ("tmux 控制切换不见了", report, build 362).
        /// Short suspensions sometimes got away with it because the resume
        /// occasionally surfaces an RST that flips the flag; long ones don't.
        func isSidecarAlive(timeout: Double? = nil) async -> Bool {
            guard let ssh = sidecarSSH else { return false }
            return await Self.probe(ssh, timeout: timeout ?? probeTimeout) == .alive
        }
    }

    private(set) var sessions: [UUID: ActiveSession] = [:]

    @ObservationIgnored private let metrics: SessionMetricsRegistry
    @ObservationIgnored private var keepAliveTimer: Timer?
    @ObservationIgnored private var lastBackgroundedAt: Date?
    /// Foreground per `setForeground` — the path watch only acts while
    /// true, and no pane is on screen while false (``PaneOnScreen``, B438).
    @ObservationIgnored private(set) var isForeground = false
    /// Started on the first foreground and never stopped: iOS suspends us in
    /// the background anyway, and whatever changed while we were away is
    /// delivered as one update on resume — which is exactly the case this
    /// exists for (the user left to turn Tailscale on and came back).
    @ObservationIgnored private var pathWatch: NetworkPathWatch?
    /// Coalesces the burst a tunnel coming up produces into one kick.
    @ObservationIgnored private var pathKick: Task<Void, Never>?
    /// A dial has to be at least this old for a path change to restart it. The
    /// watch reports a change a beat after it happened, so a younger dial most
    /// likely began after the change and is already on the new path.
    @ObservationIgnored var pathRedialAfter: TimeInterval = 1
    /// How `prepare` builds a session. Tests swap in sessions with a scripted
    /// dialer so hub-level recovery runs without a network.
    @ObservationIgnored var makeSession: @MainActor (ServerConnection) -> ActiveSession = {
        ActiveSession(connection: $0)
    }
    /// How often to probe live sessions while foreground (NAT-warmth + death
    /// detection + reconnect retry). iOS suspends timers in the background.
    private let keepAliveInterval: TimeInterval = 12
    /// Background longer than this → force a fresh reconnect on return rather
    /// than trusting the liveness probe (iOS has almost certainly killed TCP).
    private let forceReconnectAfter: TimeInterval = 20
    /// Where the device's unlock is heard: the sessions the locked keychain
    /// stopped dial again then (``deviceUnlocked()``).
    @ObservationIgnored private let unlocks = UnlockWatch()

    /// `unlocks`: where `protectedDataDidBecomeAvailable` is posted (tests
    /// post on their own).
    init(metrics: SessionMetricsRegistry, unlocks center: NotificationCenter = .default) {
        self.metrics = metrics
        unlocks.observe(center) { [weak self] in
            Task { @MainActor [weak self] in await self?.deviceUnlocked() }
        }
    }

    deinit { unlocks.stop() }

    /// The device was unlocked: every session the locked keychain stopped —
    /// its saved credential, or the trusted host keys, unreadable until now —
    /// dials again at once (``ActiveSession/retryAfterUnlock()``), as
    /// crates/session's sessions do on `device_unlocked` (B201, B211). It
    /// used to wait for the next keepalive tick or foreground return — and
    /// before the credential's lock counted as retryable at all, for the
    /// person. Concurrent, as ``resumeAll(force:)``. A session on the Rust
    /// core hears the unlock through ``RustCore``.
    func deviceUnlocked() async {
        await withTaskGroup(of: Void.self) { group in
            for session in sessions.values {
                group.addTask { @MainActor in _ = await session.retryAfterUnlock() }
            }
        }
    }

    /// Drive keepalive off the app's foreground state. Active → probe now and
    /// every `keepAliveInterval`; background → stop (iOS would suspend us anyway).
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
        if active {
            startPathWatchIfNeeded()
            // A long suspension almost certainly means iOS killed the socket,
            // and the liveness probe can false-positive on a half-open channel
            // (green pill, frozen -CC pane). Past the threshold, force a fresh
            // reconnect instead of trusting the probe.
            let away = lastBackgroundedAt.map { Date().timeIntervalSince($0) } ?? 0
            lastBackgroundedAt = nil
            let force = away > forceReconnectAfter
            Log.island.info("land: foreground after \(Int(away), privacy: .public)s force=\(force, privacy: .public)")
            // Take back the phone-grid pin for the terminal on screen (released
            // on backgrounding below) — unless that terminal is about to be
            // torn down and rebuilt anyway: the replacement pins on attach,
            // and queueing commands on a channel we are about to declare dead
            // only re-lists the window as a pin the teardown must then undo.
            if !force { visibleSession?.repinForeground() }
            Task { await resumeAll(force: force) }
            guard keepAliveTimer == nil else { return }
            keepAliveTimer = Timer.scheduledTimer(withTimeInterval: keepAliveInterval, repeats: true) { [weak self] _ in
                Task { @MainActor [weak self] in await self?.keepAliveAll() }
            }
        } else {
            lastBackgroundedAt = Date()
            keepAliveTimer?.invalidate()
            keepAliveTimer = nil
            // Drop every in-memory decrypted secret so a re-foreground forces
            // a fresh (biometry-gated) keychain read — otherwise the cache
            // that exists purely to skip repeated Face ID prompts during
            // foreground auto-reconnects would silently downgrade the
            // keychain's "authenticate on every read" (.userPresence) to
            // "once per app session", with plaintext lingering in process
            // memory across backgrounding. Live sessions are unaffected: this
            // only forces the NEXT resolveSecret() (e.g. on resume/reconnect)
            // back through the keychain.
            // iOS may suspend or kill us at ANY point from here — hand every
            // window pin back to the server NOW (and take our clients out of
            // `window-size latest` sizing), so a desktop attaching while we're
            // gone gets its full size instead of the phone grid. Verified: this
            // doesn't stop %output, so background monitoring keeps working.
            //
            // Queue them synchronously, then hold a background assertion while
            // they drain. "NOW" was aspirational before: these are control-mode
            // commands with a round trip, and a suspension between queueing and
            // flushing left the pins applied — exactly the stranded-desktop bug
            // the release is here to prevent. The assertion is not a keepalive
            // (iOS has none to give); it is tens of seconds to finish what we
            // already started. See ``BackgroundAssertion``.
            for session in sessions.values {
                session.releaseWindowPinsForBackground()
            }
            let teardown = BackgroundAssertion(name: "moshpit.background-teardown")
            let draining = Array(sessions.values)
            Task { @MainActor in
                // Secrets first: it is the cheap half, and dropping plaintext
                // sooner is worth more than draining sockets sooner.
                await SSHService.shared.clearAllCachedSecrets()
                for session in draining {
                    await session.flushBackgroundTeardown()
                }
                teardown.end()
            }
        }
    }

    private func startPathWatchIfNeeded() {
        guard pathWatch == nil else { return }
        let watch = NetworkPathWatch { [weak self] in
            Task { @MainActor [weak self] in self?.networkPathChanged() }
        }
        watch.start()
        pathWatch = watch
    }

    /// The device's network path changed — an interface came or went, a VPN
    /// tunnel opened or closed. Sessions with nothing up get their dial or
    /// reconnect now; live ones take a keepalive tick early, which probes
    /// only if bytes haven't flowed lately. Background changes wait: the app
    /// can't hold a socket there, and the foreground return resumes anyway.
    func networkPathChanged() {
        guard isForeground else { return }
        pathKick?.cancel()
        pathKick = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled, let self else { return }
            await self.kickSessionsAfterPathChange()
        }
    }

    /// The kick itself, reachable synchronously for tests.
    func kickSessionsAfterPathChange() async {
        Log.ssh.debug("resume: network path changed")
        let redialAfter = pathRedialAfter
        await withTaskGroup(of: Void.self) { group in
            for session in sessions.values {
                group.addTask { @MainActor in
                    if await session.retryIfDown(stalledDialAfter: redialAfter, reason: "path") { return }
                    await session.keepAlive()
                }
            }
        }
    }

    /// Concurrent for the same reason as ``resumeAll(force:)`` — and it bites
    /// harder here: `keepAlive`'s liveness probe is a timeout race, so one
    /// half-open socket serially burning its timeout used to delay every
    /// session behind it in the dictionary by that much per 12s tick.
    private func keepAliveAll() async {
        await withTaskGroup(of: Void.self) { group in
            for session in sessions.values {
                group.addTask { @MainActor in
                    await session.keepAlive()
                }
            }
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

    /// Probe every live session after the app returns to the foreground.
    /// Resume every session **concurrently**. This used to be a serial loop,
    /// which made returning to the foreground wait for the SUM of every
    /// session's reconnect instead of the slowest one — with a mosh+tmux
    /// bootstrap allowed ~15s and a tmux attach 22s, a second connection could
    /// sit visibly dead for that long purely because it was second in the
    /// dictionary. Everything here is main-actor state, so "concurrent" means
    /// the network waits overlap; the state mutations still interleave on one
    /// actor, exactly as before. Concurrent secret reads are safe now:
    /// `SSHService.resolveSecret` coalesces per-connection, so this can't fan
    /// one foreground-return into a stack of Face ID prompts.
    func resumeAll(force: Bool = false) async {
        await withTaskGroup(of: Void.self) { group in
            for session in sessions.values {
                group.addTask { @MainActor in
                    if force {
                        await session.forceResume()
                    } else {
                        await session.resumeIfNeeded(probeTimeout: session.foregroundProbeTimeout)
                    }
                }
            }
        }
    }

    /// Vibe Island control surface — deliver a keystroke (Allow / Deny / Reply)
    /// from a notification action or Live Activity button straight to a live
    /// agent pane, transparently reconnecting first if iOS killed the socket
    /// while the app was suspended. Best-effort: a session that's been fully
    /// disconnected (removed from the hub) can't be revived from a background
    /// action — returns false so the caller can fall back to opening the app.
    @discardableResult
    func deliverAgentInput(_ bytes: Data, connectionId: UUID, paneId: String) async -> Bool {
        guard !bytes.isEmpty, let active = sessions[connectionId] else { return false }
        if !(await active.isTransportAlive()) {
            await active.forceResume()
            guard await active.isTransportAlive() else { return false }
        }
        return await active.deliverInput(bytes, toPane: paneId)
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

/// A tmux transport that carries nothing, for a controller that only reads
/// and merges pin ledgers (`ActiveSession.pinTally`).
private struct DetachedTmuxTransport: TmuxTransport {
    let dataStream = AsyncStream<Data> { $0.finish() }
    func write(_ data: Data) async throws {}
}

// MARK: - Sessions on the Rust core (W14a)

extension SessionHub.ActiveSession {
    /// Runs this session on the Rust core when the switch says so for its
    /// connection (read once, when the session first starts: a running
    /// session keeps the path it started on). Whether it did.
    func startOnRustCoreIfChosen(theme: TerminalTheme, fontSize: Double, fontName: String,
                                 cursorShape: CursorShape, cursorColorId: String,
                                 cursorBlink: Bool, automatic: Bool) async -> Bool {
        let appearance = RustTmuxController.Appearance(
            theme: theme, fontSize: fontSize, fontName: fontName, cursorShape: cursorShape,
            cursorColorId: cursorColorId, cursorBlink: cursorBlink)
        let driver: RustSessionDriver
        if let rust {
            driver = rust
        } else {
            guard RustSessionSwitch.runsOnRust(connection) else { return false }
            driver = RustSessionDriver(session: self, core: .shared, appearance: appearance)
            rust = driver
            Log.ssh.info("session \(self.connection.id.uuidString, privacy: .public): on the Rust core")
        }
        lastTheme = theme
        lastFontSize = fontSize
        lastFontName = fontName
        lastCursorShape = cursorShape
        lastCursorColorId = cursorColorId
        lastCursorBlink = cursorBlink
        if !automatic { moshReturnPathDead = false }
        await driver.start(appearance: appearance, automatic: automatic)
        return true
    }

    /// The Rust session's tmux surface, as the chrome reads a controller.
    func rustSetTmux(_ controller: TmuxSessionController?) {
        tmuxController = controller
    }

    /// The previous attach's surface, frozen on screen until the new one paints.
    func rustSetRetired(_ controller: TmuxSessionController?) {
        retiredTmuxController = controller
    }

    func rustSetAwaitingFirstFrame(_ awaiting: Bool) {
        awaitingFirstFrame = awaiting
    }

    /// mosh + tmux on the core: the `-CC` sidecar's tree, where the chrome
    /// reads the mosh control plane.
    func rustSetMoshControl(_ controller: TmuxSessionController?) {
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

    /// What the core's probe found about the host, published as the Swift
    /// path's `probeCapabilities` publishes it (B336).
    func rustSetCapabilities(_ capabilities: HostCapabilities) {
        self.capabilities = capabilities
    }

    /// The mosh diagnostics' Repaint: mosh-server redraws the screen from
    /// blank, on the core's mosh as on the Swift transport. It did nothing
    /// on the core: the Swift transport isn't there (B338).
    func requestRepaint() {
        if let rust {
            rust.repaint()
            return
        }
        Task { await moshTransport?.requestFullRedraw() }
    }
}
