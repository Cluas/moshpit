import Foundation
import Observation
import SwiftTerm
import UIKit

/// Run `op`, returning its value, or nil if it throws or doesn't finish within
/// `seconds`. Used for liveness probes where a dead half-open socket can hang.
func withTimeoutValue<T: Sendable>(_ seconds: Double,
                                   _ op: @Sendable @escaping () async throws -> T) async -> T? {
    await withTaskGroup(of: T?.self) { group in
        group.addTask { try? await op() }
        group.addTask {
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            return nil
        }
        let first = await group.next() ?? nil
        group.cancelAll()
        return first
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
        /// Single-pane data path (non-tmux).
        @ObservationIgnored let coordinator = SwiftTerminalView.Coordinator()
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

        /// herdr's control plane. Unlike tmux's, one client covers BOTH
        /// transports: it only ever runs `herdr api snapshot` over an exec
        /// channel, so over SSH it rides the session we already have, and over
        /// mosh it rides the same lightweight sidecar connection `moshControl`
        /// would have used. Nothing about it needs a PTY or control mode.
        private(set) var herdrControl: HerdrControlClient?

        /// The active tmux control surface, whichever transport is in use.
        var tmuxControl: TmuxSessionController? { tmuxController ?? moshControl }

        /// The multiplexer control plane, whenever it materializes — for
        /// callers that need "the control plane, once it exists" rather than
        /// "the control plane right now". Over SSH the controller is assigned
        /// inside `start()` and the first check resolves; over mosh both
        /// control planes are wired by DETACHED tasks (`attachMoshTmux` /
        /// `startHerdrSidecar`) that can spend ~15s on a second SSH handshake
        /// and attach retries after `start()` has returned — exactly the
        /// window a one-shot read samples nil in, which is how mosh+tmux
        /// shipped with the island never tracking (and mosh+herdr raced the
        /// same gap). Returns nil once the session stops/fails or after
        /// `timeout` — a degraded session with no multiplexer on the host has
        /// nothing to wait for.
        func awaitMultiplexerControl(timeout: TimeInterval = 60) async -> (any MultiplexerControlling)? {
            let deadline = Date().addingTimeInterval(timeout)
            while Date() < deadline {
                if isStopping { return nil }
                switch viewModel.status {
                case .disconnected, .failed: return nil
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
        var fileTransferSSH: SSHSession? { viewModel.session ?? sidecarSSH }

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
        func acquireFileTransferSSH() async throws -> SSHSession {
            if let ssh = fileTransferSSH, await answersQuickly(ssh) { return ssh }
            if let cached = onDemandTransferSSH, await answersQuickly(cached) { return cached }
            let fresh = try await SSHService.shared.connect(connection)
            onDemandTransferSSH = fresh
            return fresh
        }

        /// True if a trivial command round-trips within 4s. Shorter than
        /// `isTransportAlive`'s 8s: a false "dead" there tears down a whole
        /// session, here it only costs an extra dial.
        private func answersQuickly(_ ssh: SSHSession) async -> Bool {
            await withTimeoutValue(4) { try await ssh.executeCommand("true") } != nil
        }

        /// Numbered log of images uploaded this session (#1, #2, …). Lives on
        /// the session — not the screen — so navigating Home and back keeps
        /// every number addressable from the image chip's long-press menu.
        var imageUploads = ImageUploadLog()

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
        @ObservationIgnored private var moshTmuxAttachTask: Task<Void, Never>?

        /// The mosh+herdr control-plane bootstrap. Stored for the same reason
        /// `moshTmuxAttachTask` is: it opens its own SSH connection, so a
        /// teardown mid-handshake must be able to cancel it instead of letting
        /// it wire a fresh connection onto a dead session.
        @ObservationIgnored private var herdrSidecarTask: Task<Void, Never>?

        // MARK: herdr push upgrade (Phase 0 of docs/design/roaming-transport.md)

        /// Push-mode upgrade for the herdr control plane: a dedicated SSH
        /// connection whose PTY hosts the socket pump (`HerdrPushBoot`), with
        /// the driver turning `events.subscribe` pushes into `quicken()`
        /// calls. nil = polling only (no python3 on the host, pipe never came
        /// up, or the upgrade hasn't run yet) — polling is never interrupted
        /// either way, push only narrows its latency.
        @ObservationIgnored private var herdrPush: HerdrPushDriver?
        @ObservationIgnored private var herdrPushSSH: SSHSession?
        @ObservationIgnored private var herdrPushPump: Task<Void, Never>?
        @ObservationIgnored private var herdrPushTask: Task<Void, Never>?

        // MARK: mosh+herdr raw-attach renderer

        /// Host's herdr supports `terminal attach` (probed at mosh bootstrap).
        /// Decides the mosh renderer: raw single-pane loop vs full TUI + zoom.
        @ObservationIgnored private var herdrRawAttach = false
        /// The terminal id the raw-attach loop was last pointed at — the
        /// idempotence guard for `retargetMoshRawAttach` (apply() re-reports
        /// focus every poll; only a real change should bounce the attach).
        @ObservationIgnored private var moshRawAttachTarget: String?
        /// This boot's renderer generation (connection id + per-boot nonce).
        /// Retargets only ever write THIS generation's state files, which is
        /// what keeps orphaned loops from previous mosh sessions out of the
        /// takeover fight — see `HerdrLaunch.moshRendererKey`.
        @ObservationIgnored private var moshRendererKey: String?

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
        }

        /// Returning Home leaves the connection alive (for background monitoring
        /// + fast resume) but stops rendering, so the tmux window-size pin we
        /// placed for the phone grid would strand a desktop client sharing those
        /// windows at the phone width. Hand the pins back here; `repinForeground()`
        /// re-applies them when the terminal returns. Covers SSH + the mosh sidecar.
        func releaseWindowPinsForBackground() {
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
            await tmuxController?.flushPendingWrites()
            await moshControl?.flushPendingWrites()
        }

        /// Re-pin the active window to the phone grid when the terminal returns to
        /// the foreground after `releaseWindowPinsForBackground()`.
        func repinForeground() {
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

        /// Set when a mosh session's UDP return path is dead — the socket went
        /// `.ready` and we kept flushing, but the server's replies never
        /// arrived (commonly a VPN/proxy or firewall that passes outbound UDP
        /// but drops the inbound datagrams). The terminal would otherwise sit
        /// on a live cursor over a permanently black screen with no
        /// explanation; this drives a dismissible "Mosh isn't receiving data"
        /// banner that offers a one-tap switch to SSH (which rides TCP and
        /// works through the same proxy). Fires at most once per transport.
        var moshReturnPathDead = false

        /// Bounds the wait started by `beginAttachTimeout`. Stored (not
        /// fire-and-forget) so `stop()` can cancel it — a stalled watcher
        /// left running past teardown could otherwise fire `attachStalled`
        /// against a controller that's already gone, or race a fresh
        /// attach's own watcher. Same class of bug as `moshTmuxAttachTask`
        /// (see its doc comment).
        @ObservationIgnored private var attachTimeoutTask: Task<Void, Never>?

        init(connection: ServerConnection) {
            self.connection = connection
            self.viewModel = TerminalViewModel(connection: connection)
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
        /// Chains mosh keystroke writes so rapid-fire input (type a word, then
        /// immediately tap an arrow key) can't reorder at the transport — see
        /// the matching `writeChain` in TmuxSessionController.enqueue for the
        /// same bug on the -CC control path (independently-spawned `Task`s
        /// have no FIFO guarantee relative to each other).
        @ObservationIgnored private var moshWriteChain: Task<Void, Never>?

        // MARK: herdr frame channel (SSH only)

        /// The pane the frame channel is currently rendering, or nil when the
        /// channel isn't running. Also the "am I in frame mode?" flag that
        /// `sendInput` and the scroll routing branch on.
        @ObservationIgnored private var herdrFrameTarget: String?
        /// The channel carrying frames — the session's own PTY, so retargeting
        /// costs a command, not a connection.
        @ObservationIgnored private var herdrFrameSSH: SSHSession?
        /// Serializes stdin writes to the frame channel. Same hazard the mosh
        /// path has: independently-spawned writes can reorder, and a resize
        /// landing before the keystroke it was meant to follow repaints wrong.
        @ObservationIgnored private var herdrWriteChain: Task<Void, Never>?
        /// In-flight retarget, cancelled if another one starts first.
        @ObservationIgnored private var herdrRetargetTask: Task<Void, Never>?
        /// Set when another client keeps stealing this pane's attach, so the
        /// UI can say so instead of flickering silently. Cleared once we hold
        /// the channel again.
        private(set) var herdrNotice: String?
        /// Label for the notice's one-tap remedy, when the notice has one
        /// (herdr version skew → restart the server). nil = plain notice.
        private(set) var herdrNoticeAction: String?
        /// When the eviction storm subsides enough to retry. herdr's direct
        /// attach is exclusive per terminal, so two Moshpits on the same pane
        /// evict each other forever — each re-asserting every poll. Backing
        /// off turns a silent 2-second fistfight into one visible retry every
        /// half minute, which recovers on its own the moment the other client
        /// goes away. (Seen live: a stale simulator session and a phone
        /// trading the same pane back and forth, output repainting while every
        /// keystroke went to a channel that had just been evicted.)
        @ObservationIgnored private var herdrRetryAfter: Date?
        /// Timestamps of recent evictions — closes we did NOT ask for.
        @ObservationIgnored private var herdrEvictions: [Date] = []
        /// Three evictions inside this window means somebody else wants the
        /// pane; one or two is just a reconnect racing its own old channel.
        private static let herdrContentionWindow: TimeInterval = 30
        private static let herdrContentionBackoff: TimeInterval = 30

        /// Channel closes we caused ourselves by releasing during a retarget.
        ///
        /// Releasing makes the server emit `terminal.closed` for the pane we
        /// just left, and that arrives AFTER the new target is already set.
        /// Treating it as "the pane went away" would clear `herdrFrameTarget`
        /// and silently drop the session back to raw-byte input — keystrokes
        /// then go to the shell as plain text, where the frame channel ignores
        /// them: output keeps flowing, typing dies. (Found exactly that way.)
        @ObservationIgnored private var herdrExpectedCloses = 0

        /// Set while a retarget's veil is up, waiting for the new pane's first
        /// FULL frame to reveal on. Incremental frames are absolutely-positioned
        /// cell updates, so painting one over the pane we just left would show
        /// the old agent's screen with a few new cells stamped into it — worse
        /// than the veil it replaced. A full frame carries its own clear+home,
        /// which is the first moment the screen is honestly the new pane's.
        @ObservationIgnored private var herdrAwaitingFullFrame = false

        /// The SwiftTerm terminal backing whichever pane is on screen right now —
        /// tmux resolves the zoomed active pane's view; mosh/plain SSH have just
        /// the one coordinator view.
        private var activeTerminal: Terminal? {
            if let controller = tmuxController,
               let paneId = controller.snapshot.activePaneId
                    ?? controller.snapshot.activePanes.first?.id {
                return controller.terminalView(for: paneId).getTerminal()
            }
            return coordinator.terminalView?.getTerminal()
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
        var applicationCursorKeys: Bool { activeTerminal?.applicationCursor ?? false }

        /// Paste-aware send: wraps the clipboard text in bracketed-paste markers
        /// when the active terminal's app requested them (DECSET 2004 — Claude
        /// Code, vim, modern shells). Raw bytes executed a multi-line prompt
        /// line by line; bracketed, the app receives it as one paste block.
        func sendPaste(_ text: String) {
            var payload = Data(text.utf8)
            if activeTerminal?.bracketedPasteMode == true {
                payload = Data("\u{1b}[200~".utf8) + payload + Data("\u{1b}[201~".utf8)
            }
            sendInput(payload)
        }

        /// Single entry point for user input (keyboard, shortcut bar, paste).
        /// Routes to the correct transport so callers don't have to know
        /// whether this session is tmux / mosh / plain SSH.
        func sendInput(_ data: Data) {
            guard !data.isEmpty else { return }
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
            guard viewModel.connState == .live else { return }
            // If we scrolled the pane into tmux copy-mode, leave it first so the
            // keystrokes reach the shell (in copy-mode tmux would eat them).
            tmuxControl?.exitCopyMode()
            if herdrFrameTarget != nil {
                // Frame mode: keystrokes are a protocol message, not raw bytes
                // on the wire. base64 so escape sequences and partial UTF-8
                // survive the JSON round trip.
                writeFrameCommand(HerdrFrameCommand.input(data))
            } else if let controller = tmuxController {
                let paneId = controller.snapshot.activePaneId
                    ?? controller.snapshot.activePanes.first?.id
                if let paneId { controller.sendInput(data, paneId: paneId) }
            } else if let transport = moshTransport {
                // Leaving mosh copy-mode? Prepend `q` (cancel) on the SAME channel
                // so it's strictly ordered before the keystrokes — no cross-channel
                // race.
                let out = Self.moshInputKeys(data, inCopyMode: moshInCopyMode)
                moshInCopyMode = false
                // Chained, not a bare `Task { }` — independently-spawned tasks
                // have no ordering guarantee, so typing a word then quickly
                // tapping the D-pad could deliver the arrow bytes before the
                // last character or two (the "history search stops matching
                // what I typed" bug).
                sendOverMosh(transport, out)
            } else {
                viewModel.send(data)
            }
        }

        /// Deliver bytes to one SPECIFIC pane — the lock-screen Allow / Deny /
        /// Reply path, where "whatever pane happens to be focused" is wrong.
        ///
        /// tmux can address a pane directly (`send-keys -t`). herdr cannot:
        /// its frame channel types into whichever pane it is attached to, so
        /// the pane must be FOCUSED first (`agent focus`), the channel given
        /// time to retarget, and only then the keystroke written. Skipping
        /// that — as the first version did by falling back to `sendInput` —
        /// sent the lock-screen "Allow" into whatever pane the app was last
        /// showing, which with two concurrent agents is someone else's Enter.
        ///
        /// Returns false when the pane could not be focused in time (eviction
        /// backoff, dead poller), so the caller can say "open the app" rather
        /// than pretend the tap worked.
        /// Paste text into one SPECIFIC pane — bracketed like `sendPaste`,
        /// but judged by the TARGET pane's mode rather than the active one's.
        /// The Home card's per-agent image entry delivers here without ever
        /// opening the terminal screen.
        func deliverPaste(_ text: String, toPane paneId: String) async -> Bool {
            var payload = Data(text.utf8)
            let bracketed = tmuxControl?.paneUsesBracketedPaste(paneId)
                ?? (activeTerminal?.bracketedPasteMode == true)
            if bracketed {
                payload = Data("\u{1b}[200~".utf8) + payload + Data("\u{1b}[201~".utf8)
            }
            return await deliverInput(payload, toPane: paneId)
        }

        func deliverInput(_ data: Data, toPane paneId: String) async -> Bool {
            guard !data.isEmpty else { return false }
            if let control = tmuxControl {
                control.sendInput(data, paneId: paneId)
                return true
            }
            if let herdr = herdrControl {
                if herdrFrameSSH != nil {
                    // Native frame mode (SSH). Focus first unless already there.
                    if herdrFrameTarget != paneId {
                        herdr.selectPane(paneId)
                        guard await waitUntil(timeout: 5.0,
                                              condition: { [weak self] in self?.herdrFrameTarget == paneId })
                        else { return false }
                    }
                    // The retarget task writes `release`, sleeps, then writes the
                    // new `start` line. A keystroke enqueued inside that gap
                    // lands on the bare shell and dies — so wait the task out;
                    // the write chain then orders our input after the start.
                    await herdrRetargetTask?.value
                    guard herdrFrameTarget == paneId else { return false }
                    writeFrameCommand(HerdrFrameCommand.input(data))
                    return true
                }
                // TUI mode (mosh+herdr): keystrokes go to herdr's own client,
                // which types into the server-side focused pane. Same recipe,
                // different confirmation signal.
                if herdr.snapshot.activePaneId != paneId {
                    herdr.selectPane(paneId)
                    guard await waitUntil(timeout: 5.0,
                                          condition: { [weak herdr] in herdr?.snapshot.activePaneId == paneId })
                    else { return false }
                }
                sendInput(data)
                return true
            }
            // Plain SSH / mosh: one shell, one place the bytes can go.
            sendInput(data)
            return true
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

        /// Send bytes into the mosh keystroke channel, chained through
        /// `moshWriteChain` so independently-spawned writes — a keystroke, a
        /// wheel-forwarded scroll, a copy-mode page — can't reorder at the
        /// transport (see the property's doc comment). Every write into this
        /// channel must go through here rather than a bare
        /// `Task { await transport.send(...) }`.
        private func sendOverMosh(_ transport: MoshTransport, _ data: Data) {
            let previous = moshWriteChain
            moshWriteChain = Task {
                await previous?.value
                await transport.send(data)
            }
        }

        /// Scroll the visible terminal's scrollback (driven by the bar's scroll
        /// thumb / the swipe). tmux sessions — SSH or mosh — page tmux's real
        /// scrollback via copy-mode over the control channel; a plain shell pages
        /// the local SwiftTerm buffer. Positive = older output, negative = newer.
        func scrollActiveTerminal(lines: Int) {
            // Same gate as sendInput, with one exception: a swipe on a NOT-live
            // session must never write to the wire (mosh would queue wheel or
            // copy-mode bytes and replay them after resume — the same accident
            // as stray arrows, wearing mouse clothes), but paging the LOCAL
            // buffer is read-only and stays useful — an offline plain shell's
            // scrollback is still readable under the red pill.
            guard viewModel.connState == .live else {
                if herdrFrameTarget == nil, moshTransport == nil, tmuxController == nil {
                    coordinator.scrollLocal(lines: lines)
                }
                return
            }
            if herdrFrameTarget != nil {
                // The one place herdr strictly beats tmux here: the SERVER
                // decides whether a wheel becomes a mouse report (an app that
                // grabbed the mouse — Claude Code, vim) or scrollback movement.
                // No `#{mouse_any_flag}` lookup, no copy-mode dance, no
                // first-swipe race to self-heal.
                writeFrameCommand(HerdrFrameCommand.scroll(up: lines > 0, lines: abs(lines)))
            } else if let transport = moshTransport, let control = moshControl {
                // mosh+tmux: a mouse app (Claude Code etc.) gets a wheel forwarded
                // over the mosh transport — tmux routes it to the app, which
                // scrolls itself with no copy-mode (so typing keeps working). A
                // plain shell gets copy-mode keystrokes on the mosh-rendered
                // client (the sidecar's copy-mode doesn't repaint it).
                if control.activePaneWantsMouse {
                    let terminal = coordinator.terminalView?.getTerminal()
                    let col = max(1, (terminal?.cols ?? 80) / 2)
                    let row = max(1, (terminal?.rows ?? 24) / 2)
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
        /// because the report would land in its command line as text. herdr is
        /// the one path with nothing to send — its frame protocol has no click
        /// command, so a tap there still just focuses.
        func clickActiveTerminal(col: Int, row: Int) {
            if herdrFrameTarget != nil {
                return
            } else if let transport = moshTransport, let control = moshControl {
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
                // unbounded mutual recursion. Hit for real when a herdr frame
                // channel failed to attach (herdrFrameTarget nil, no mosh
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
            // SGR coordinates are 1-based on the wire.
            let c = max(1, col + 1), r = max(1, row + 1)
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
        /// static so it's unit-testable; `scripts/verify-tmux-scroll.py` proves
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
        /// `scripts/verify-tmux-wheel.py` proves these bytes scroll a real client.
        nonisolated static func wheelBytes(lines: Int, col: Int, row: Int) -> Data {
            guard lines != 0 else { return Data() }
            let button = lines > 0 ? 64 : 65
            let count = min(abs(lines), 6)
            let c = max(1, col), r = max(1, row)
            let event = "\u{1b}[<\(button);\(c);\(r)M"
            return Data(String(repeating: event, count: count).utf8)
        }

        /// Bytes to actually send the mosh client for a keystroke: when we'd put
        /// the client into copy-mode for scrolling, prepend `q` so the keystroke
        /// leaves copy-mode first and reaches the shell (the reported "can't type
        /// / can't exit in copy-mode" bug). Pure + nonisolated for unit testing;
        /// `scripts/verify-tmux-scroll.py` proves `q` exits a real client.
        nonisolated static func moshInputKeys(_ data: Data, inCopyMode: Bool) -> Data {
            inCopyMode ? moshCopyExitKey + data : data
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
        /// channel — see `TmuxSessionController.tmuxQuote` for that separate
        /// context). Used for tmux SESSION NAMES, which can originate from
        /// the remote server or another client (created, renamed) and are
        /// not otherwise validated here — a name containing a `'` would
        /// otherwise close the surrounding quote early and let the rest of
        /// the string execute as arbitrary shell commands. Standard POSIX
        /// single-quote escaping: close the quote, emit an escaped literal
        /// `'`, reopen it — `'\''`.
        nonisolated static func shellQuote(_ value: String) -> String {
            "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
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
            // Idempotent for live sessions: re-entering the Terminal screen
            // while connecting/connected must not re-run the bootstrap (the
            // mosh path would exec a second mosh-server). Dead sessions
            // (failed / disconnected) reset and retry.
            switch viewModel.status {
            case .connecting, .connected, .reconnecting:
                return
            case .failed, .disconnected:
                viewModel.resetForReconnect()
            case .idle:
                break
            }
            viewModel.beginAttempt(automatic: automatic)
            // Fresh attempt — clear any stale degrade/stall notices so a retry
            // (or a reused-but-reset session) doesn't inherit the last run's
            // banner before it has re-earned it.
            moshReturnPathDead = false
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
                await startMosh()
                return
            }

            // Seed a phone-sized grid up front so the FIRST PTY / tmux
            // refresh-client size isn't the 80×24 default — a late resize
            // can't reflow lines a program already emitted, and that gap is
            // very visible over high-RTT links (wide text, wrong wrapping).
            // The exact size still follows from the view's layout.
            let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: fontSize)
            await viewModel.start(rows: grid.rows, cols: grid.cols)
            guard let session = viewModel.session else { return }
            // Auto-reconnect the moment the transport drops on its own (server
            // reboot, network change, NIO seeing the socket die on resume).
            await session.setOnUnexpectedClose { [weak self] in
                Task { @MainActor [weak self] in await self?.connectionDropped() }
            }
            if let size = coordinator.lastReportedSize {
                viewModel.resize(rows: size.rows, cols: size.cols)
            }

            // Probe the host before committing to a multiplexer. A custom
            // binary path in the form means the user vouches for it — trust it
            // and skip the degrade check (the probe walks PATH, not custom
            // locations). Otherwise a missing multiplexer degrades to a plain
            // pane with a banner instead of stalling on a failing attach.
            let caps = await probeCapabilities(over: session)
            // Retention sweep for uploaded images, on the channel that's
            // already warm. Fire-and-forget and silent on failure — a host
            // without the folder, or without `find`, owes nobody an error.
            // POSIX tools only, so no PATH prefix needed.
            let retentionDays = AppSettings.shared.uploadRetentionDays
            if retentionDays > 0 {
                let sweep = Self.uploadCleanupCommand(days: retentionDays)
                Task { _ = try? await session.executeCommand(sweep) }
            }
            let chosen = connection.multiplexer
            let muxDegraded = connection.multiplexerPath == nil && !caps.has(chosen)
            if muxDegraded { degrade = DegradeNotice.forMissing(chosen) }
            // Degraded → a plain shell, never the OTHER multiplexer.
            let multiplexer: Multiplexer = muxDegraded ? .none : chosen

            if multiplexer == .tmux {
                let controller = TmuxSessionController(sshSession: session)
                controller.transportIsLive = { [weak viewModel] in
                    viewModel?.connState == .live
                }
                controller.pendingRestore = lastSelection   // land back on our pane
                controller.configureAppearance(theme: theme, fontSize: fontSize, fontName: fontName,
                                                cursorShape: cursorShape, cursorColorId: cursorColorId,
                                                cursorBlink: cursorBlink)
                controller.setInitialClientSize(cols: grid.cols, rows: grid.rows)
                // Pump + callbacks first, but NO discovery yet — tmux isn't
                // running. %session-changed (fired by `new -A`) triggers it.
                await controller.beginControlMode()
                let tmux = connection.tmuxPath ?? "tmux"
                // Raise server history-limit before attaching so future panes
                // retain deep scrollback (Claude Code output), then start
                // control mode. The `set` runs in the shell, pre control-mode.
                // Attach the user's EXISTING sessions (most recent). Moshpit
                // never creates or modifies sessions on its own — if the
                // server has none, the terminal shows an empty state and the
                // user creates the first one explicitly.
                let boot = "\(tmux) set -g history-limit 50000 2>/dev/null; \(tmux) -CC attach\r"
                if let bytes = boot.data(using: .utf8) {
                    try? await session.write(bytes)
                }
                tmuxController = controller
                beginAttachTimeout(controller: controller)
            } else if multiplexer == .herdr {
                // Native single-pane rendering: this PTY carries herdr's frame
                // protocol rather than herdr's own TUI, so the phone shows one
                // full-width pane with Moshpit's chrome instead of a desktop
                // sidebar eating a third of the screen.
                //
                // Control plane first — it supplies the pane id the frame
                // channel targets, and its `onFocusedPaneChanged` is what
                // starts the very first one.
                startHerdrFrames(over: session)
                startHerdrControlPlane(over: session)
                // The frame channel owns this connection's PTY, so push rides
                // a dedicated connection — same cost mosh mode already pays
                // for its sidecar, and Phase 1's bridge collapses them all.
                startHerdrPushUpgrade()
            } else {
                let coordinator = self.coordinator
                pumpTask = Task { [weak self] in
                    for await chunk in session.dataStream {
                        if Task.isCancelled { break }
                        coordinator.feed(data: chunk)
                    }
                    _ = self
                }
            }
        }

        /// Start (or restart) herdr's snapshot poller over `ssh`. Safe to call
        /// again — a live client is left alone rather than doubled up.
        func startHerdrControlPlane(over ssh: SSHSession) {
            guard herdrControl == nil else { return }
            let client = HerdrControlClient(runner: ssh, customPath: connection.herdrPath)
            // Follow the server's focus. The frame channel renders exactly one
            // pane, so "which pane" is whatever herdr says is focused —
            // whether that came from our own sheet, a keystroke inside the
            // pane, or the user's laptop client.
            client.onFocusedPaneChanged = { [weak self] paneId in
                // SSH renders per-pane frames; mosh steers the raw-attach
                // loop. Each is a no-op on the other transport.
                self?.retargetHerdrFrames(to: paneId)
                self?.retargetMoshRawAttach(to: paneId)
            }
            // Version skew (protocol_mismatch) reads as a banner, not as the
            // "no server running" empty state — the server IS running, and
            // the remedy (restart/upgrade herdr on the host) is the user's,
            // offered as the banner's one-tap action.
            client.onProtocolMismatch = { [weak self] message in
                self?.offerHerdrServerRestart(message: message)
            }
            client.start()
            herdrControl = client
        }

        /// Put the skew notice up with its remedy attached. Never runs the
        /// remedy itself — `server stop` exits pane processes, and only the
        /// user knows whether what's in them survives a restart.
        func offerHerdrServerRestart(message: String) {
            herdrNotice = message
            herdrNoticeAction = String(localized: "Restart herdr server")
        }

        /// The banner button's tap — restart the host's herdr server with the
        /// binary our own commands resolve to (guaranteeing the protocol we
        /// speak afterwards), then let the poller re-attach everything.
        func runHerdrNoticeAction() {
            guard herdrNoticeAction != nil, let client = herdrControl else { return }
            herdrNoticeAction = nil
            herdrNotice = String(localized: "Restarting herdr server…")
            Task { [weak self] in
                let ok = await client.restartServer()
                guard let self else { return }
                if ok {
                    self.herdrNotice = nil
                    self.herdrRetryAfter = nil
                    // Frame target was cleared by the fault; the next focus
                    // poll re-asserts it against the fresh server.
                    client.quicken()
                } else {
                    self.herdrNotice = String(
                        localized: "Restart didn't take — on the host, run: herdr server stop && herdr")
                    self.herdrNoticeAction = String(localized: "Retry")
                }
            }
        }

        // MARK: - herdr frame channel

        /// Render herdr natively, one pane at a time, instead of letting it
        /// draw its own TUI.
        ///
        /// The channel is the session's PTY running
        /// `herdr terminal session control`, which speaks newline-delimited
        /// JSON: screen updates out, keystrokes / resizes / scrolls in. Only
        /// the DECODED frame bytes reach the terminal view — the shell prompt
        /// that appears between two targets, and any banner the login shell
        /// prints, are parsed away and never painted.
        ///
        /// SSH only. mosh transmits rendered screen diffs, which destroy the
        /// line framing this needs (the same reason tmux `-CC` can't ride it),
        /// so mosh+herdr keeps running herdr's own TUI.
        private func startHerdrFrames(over session: SSHSession) {
            herdrFrameSSH = session
            let coordinator = self.coordinator

            // Keystrokes, size and scroll all become protocol messages now.
            coordinator.onInput = { [weak self] data in self?.sendInput(data) }
            coordinator.onSizeChange = { [weak self] cols, rows in
                self?.writeFrameCommand(HerdrFrameCommand.resize(cols: cols, rows: rows))
            }
            coordinator.onScroll = { [weak self] lines in
                self?.scrollActiveTerminal(lines: lines)
            }

            pumpTask = Task { [weak self] in
                var parser = HerdrFrameParser()
                for await chunk in session.dataStream {
                    if Task.isCancelled { break }
                    for frame in parser.consume(chunk) {
                        switch frame {
                        case .screen(_, _, _, let full, let bytes):
                            // Frames are ready-to-write escape sequences, and a
                            // `full` one already starts with its own clear +
                            // home — so both kinds just get fed.
                            coordinator.feed(data: bytes)
                            await MainActor.run {
                                // Painting again means we hold the channel, so
                                // any contention warning is stale.
                                if self?.herdrNotice != nil { self?.herdrNotice = nil }
                                if self?.herdrNoticeAction != nil { self?.herdrNoticeAction = nil }
                                // A retarget's veil lifts on the first full
                                // repaint — the frame we just fed IS the new
                                // pane's screen, so there is nothing stale left
                                // to hide.
                                if full, self?.herdrAwaitingFullFrame == true {
                                    self?.herdrAwaitingFullFrame = false
                                    self?.coordinator.reveal()
                                }
                            }
                        case .closed:
                            await MainActor.run {
                                guard let self else { return }
                                if self.herdrExpectedCloses > 0 {
                                    // Our own release during a retarget — the
                                    // new target is already set, leave it be.
                                    self.herdrExpectedCloses -= 1
                                } else {
                                    // Somebody else took the pane, or it went
                                    // away. Either way stop claiming to render
                                    // it; the poller re-asserts the target.
                                    self.herdrFrameTarget = nil
                                    self.noteHerdrEviction()
                                    // Re-attaching rides the control poll, so
                                    // don't let it wait out an idle interval.
                                    self.herdrControl?.quicken()
                                    // The full frame this retarget was veiling
                                    // for is never coming — the attach is gone.
                                    // Same principle as the cover's own safety
                                    // timeout: a beat of stale text beats a
                                    // black rectangle sitting over a session
                                    // that is now backing off for 30 seconds.
                                    if self.herdrAwaitingFullFrame {
                                        self.herdrAwaitingFullFrame = false
                                        self.coordinator.reveal()
                                    }
                                }
                            }
                        case .fault(let message):
                            await MainActor.run {
                                guard let self else { return }
                                // herdr refused the attach and said why —
                                // version skew is the live example. Show the
                                // words, stop claiming the pane, and back off:
                                // this doesn't heal until a human restarts the
                                // server, so re-attaching every poll would
                                // just harvest the same refusal forever.
                                if HerdrSnapshot.looksLikeVersionSkew(message) {
                                    self.offerHerdrServerRestart(message: message)
                                } else {
                                    self.herdrNotice = message
                                }
                                self.herdrFrameTarget = nil
                                self.herdrRetryAfter = Date()
                                    .addingTimeInterval(Self.herdrContentionBackoff)
                                if self.herdrAwaitingFullFrame {
                                    self.herdrAwaitingFullFrame = false
                                    self.coordinator.reveal()
                                }
                            }
                        }
                    }
                }
                _ = self
            }
        }

        /// Point the frame channel at a different pane.
        ///
        /// The channel is one long-lived shell, so switching means: release the
        /// current attach (herdr's direct attach is exclusive per terminal),
        /// let the command exit and the shell come back, then start it again
        /// with the new target. The pause is timing, not ceremony — verified
        /// against a real server, where the released command's `terminal.closed`
        /// lands first and the shell prompt follows.
        /// Called on every poll, not just on a focus change — see
        /// `HerdrControlClient.onFocusedPaneChanged`. The equality guard makes
        /// the repeat free, and the repeat is what recovers a channel that was
        /// evicted by another client's `--takeover` (herdr's direct attach is
        /// exclusive per terminal, so that can happen with the focus unchanged).
        /// Record a channel close we didn't ask for and decide whether to keep
        /// re-attaching. Repeated evictions mean another client wants this
        /// exact pane, and only one of us can have it.
        private func noteHerdrEviction() {
            let now = Date()
            herdrEvictions.append(now)
            herdrEvictions.removeAll { now.timeIntervalSince($0) > Self.herdrContentionWindow }
            guard herdrEvictions.count >= 3 else { return }
            herdrEvictions.removeAll()
            herdrRetryAfter = now.addingTimeInterval(Self.herdrContentionBackoff)
            herdrNotice = String(localized: "Another client is using this pane — retrying shortly")
        }

        private func retargetHerdrFrames(to paneId: String) {
            if let retryAfter = herdrRetryAfter {
                guard Date() >= retryAfter else { return }
                herdrRetryAfter = nil
            }
            guard herdrFrameSSH != nil, paneId != herdrFrameTarget else { return }
            let hadTarget = herdrFrameTarget != nil
            herdrFrameTarget = paneId
            // Cover the pane we're leaving for the length of the swap. The
            // release, the settle, the reattach and herdr's repaint are all
            // round trips, and until the last one lands the emulator still
            // holds the PREVIOUS agent's screen — fully painted, indis-
            // tinguishable from live. Same treatment tmux's selectPane gives
            // the identical hazard: a clean veil reads as a transition, stale
            // content reads as a bug.
            //
            // Only when there was a target to leave. The first attach of a
            // session has nothing stale to hide, and veiling it would just
            // delay the first paint.
            if hadTarget {
                coordinator.veilForSwitch()
                // The 0.8s default assumes a near-instant resync. This swap
                // spends 0.4s settling before it even asks for the new pane,
                // then waits out a round trip and a full repaint — on a real
                // link that routinely outruns 0.8s, and the timeout firing
                // early is exactly the stale frame we're hiding.
                coordinator.extendCoverTimeout(by: 1.6)
                herdrAwaitingFullFrame = true
            }
            herdrRetargetTask?.cancel()
            herdrRetargetTask = Task { [weak self] in
                guard let self else { return }
                if hadTarget {
                    herdrExpectedCloses += 1
                    writeFrameCommand(HerdrFrameCommand.release)
                    try? await Task.sleep(for: .milliseconds(400))
                    guard !Task.isCancelled else { return }
                }
                // No parser reset needed: a truncated line from the old target
                // fails to parse and is dropped, and the new target opens with
                // a full repaint — so at worst one dead line is skipped.
                let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: lastFontSize)
                writeFrameCommand(HerdrFrameCommand.start(
                    target: paneId, cols: grid.cols, rows: grid.rows,
                    customPath: connection.herdrPath))
            }
        }

        /// Write one protocol line into the frame channel, chained so writes
        /// can't reorder (see `herdrWriteChain`).
        private func writeFrameCommand(_ line: String) {
            guard let ssh = herdrFrameSSH else { return }
            let previous = herdrWriteChain
            herdrWriteChain = Task {
                await previous?.value
                try? await ssh.write(Data((line + "\n").utf8))
            }
        }

        /// mosh+herdr: open a lightweight SSH connection just for the control
        /// plane. Cheaper than tmux's equivalent — no PTY, no control mode,
        /// just an exec channel per poll.
        ///
        /// Host keys: same reasoning as `startMoshControlPlane` — this dials
        /// the host the mosh bootstrap already authenticated, so the trust
        /// decision is cached and neither TOFU handler fires. Left at
        /// `connect()`'s deny-by-default so that if that ever stops being true
        /// we lose the breadcrumb rather than silently trusting a new key.
        private func startHerdrSidecar() async {
            guard herdrControl == nil else { return }
            guard let ssh = try? await SSHService.shared.connect(connection) else { return }
            guard !isStopping else { await ssh.close(); return }
            sidecarSSH = ssh
            startHerdrControlPlane(over: ssh)
            if herdrRawAttach {
                // The renderer is a raw-attach loop showing exactly one pane;
                // focus changes steer it through `retargetMoshRawAttach`
                // (wired via onFocusedPaneChanged, which apply() repeats on
                // every poll — the last-target guard makes repeats free).
                // Nothing server-shared changes: no zoom, no TUI chrome.
            } else {
                // Old herdr (no `terminal attach`): mosh renders the full
                // TUI, one shared screen — so "one pane, full-screen" is
                // produced server-side, mirroring mosh+tmux's immersive
                // zoom. Shared-state tradeoff documented on `immersiveZoom`.
                herdrControl?.immersiveZoom = true
                herdrControl?.requestImmersiveZoom()
            }
            startHerdrPushUpgrade()
        }

        /// Steer the mosh raw-attach loop at a pane's terminal. Idempotent by
        /// design: apply() re-reports the focused pane on every poll (the
        /// takeover-recovery pattern), so this compares against the last
        /// target it wrote and only pays the exec — and the reattach flicker —
        /// on a real change.
        func retargetMoshRawAttach(to paneId: String) {
            guard herdrRawAttach, moshTransport != nil,
                  let rendererKey = moshRendererKey else { return }
            guard let terminalId = herdrControl?.terminalIds[paneId],
                  terminalId != moshRawAttachTarget else { return }
            moshRawAttachTarget = terminalId
            guard let ssh = sidecarSSH else { return }
            let command = HerdrLaunch.retargetCommand(
                terminalId: terminalId, rendererKey: rendererKey)
            Task { _ = try? await ssh.executeCommand(command) }
        }

        /// Try to upgrade the herdr control plane from polling to push.
        ///
        /// Fire-and-forget and strictly additive: on any failure the poller
        /// keeps its 2–8s cadence and nothing is surfaced — push is an
        /// upgrade, not a dependency. Gated on the python3 probe only when
        /// the probe has actually said no; the optimistic-unknown case costs
        /// one subscribe timeout and then settles on polling.
        private func startHerdrPushUpgrade() {
            guard herdrPushTask == nil, herdrPush == nil else { return }
            guard capabilities?.hasPython3 != false else { return }
            herdrPushTask = Task { [weak self] in await self?.connectHerdrPush() }
        }

        private func connectHerdrPush() async {
            guard let ssh = try? await SSHService.shared.connect(connection) else { return }
            guard !isStopping, herdrControl != nil else { await ssh.close(); return }
            // The pump needs a PTY: Citadel's exec channels have no writable
            // stdin (re-verified on 0.12.1 — `ExecCommandStream` is
            // stdout/stderr only), and the PTY + `stty raw -echo` shape is
            // the one the frame channel already proved out.
            try? await ssh.requestPTY(rows: 24, cols: 80)
            let driver = HerdrPushDriver(
                write: { data in try await ssh.write(data) },
                onInvalidate: { [weak self] in self?.herdrControl?.quicken() })
            herdrPushSSH = ssh
            herdrPushPump = Task { [weak driver] in
                for await chunk in ssh.dataStream {
                    if Task.isCancelled { break }
                    await driver?.feed(chunk)
                }
                // Transport gone (drop, teardown): the driver goes inactive;
                // the poller underneath never stopped.
                await driver?.shutdown()
            }
            try? await ssh.write(Data((HerdrPushBoot.bootLine() + "\r").utf8))
            if await driver.activate() {
                herdrPush = driver
                // Fold the subscribe's bootstrap replay into one fresh read.
                herdrControl?.quicken()
            } else {
                herdrPushPump?.cancel()
                herdrPushPump = nil
                herdrPushSSH = nil
                await ssh.close()
            }
        }

        /// Probe (or read cached) host capabilities and publish them on the
        /// session. Uses any cached value immediately for a fast first paint,
        /// then re-probes in the foreground so a freshly-installed tool is
        /// reflected this session. Tolerant: a probe that can't run keeps the
        /// last known capabilities (or the optimistic default on a cold
        /// session) so a transient hiccup never degrades a healthy host — and
        /// never *upgrades* a known-degraded one back to "all present".
        @discardableResult
        func probeCapabilities(over ssh: SSHSession) async -> HostCapabilities {
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

        /// Re-run the probe over whichever SSH channel this session owns
        /// (in-band SSH session or the mosh sidecar). Returns the fresh caps,
        /// or nil if no SSH channel is available. Used by Install Assist's
        /// "Re-check" after the user runs the install command.
        @discardableResult
        func recheckCapabilities() async -> HostCapabilities? {
            // Re-check needs a LIVE channel. Mosh closes its bootstrap SSH after
            // handoff, so there may be none — return nil (→ "reconnect to apply")
            // rather than probing a dead session, which would falsely read as
            // "everything installed".
            let ssh = viewModel.session ?? sidecarSSH
            guard let ssh, await ssh.isConnected else { return nil }
            return await probeCapabilities(over: ssh)
        }

        /// Create the FIRST tmux session on a server that has none. Only
        /// valid pre-attach: the boot `tmux -CC attach` has failed and left
        /// the shell at its prompt, so a fresh `-CC new` line boots control
        /// mode. (Over mosh the visible shell + retrying sidecar cover this.)
        func createFirstTmuxSession() {
            guard let controller = tmuxController, !controller.snapshot.isAttached else { return }
            let tmux = connection.tmuxPath ?? "tmux"
            viewModel.send(Data("\(tmux) -CC new\r".utf8))
        }

        /// Mosh path: authenticate SSH (no PTY), exec `mosh-server` to learn
        /// the UDP port + key, drop SSH, then run the real SSP transport.
        private func startMosh() async {
            // Reconnect on a REUSED coordinator: a scroll-hold engaged before
            // the disconnect would silently buffer the fresh connection's
            // output (frozen screen); the held bytes belong to the dead screen
            // — discard, don't replay. Same for the mosh copy-mode flag, whose
            // q-prepend would eat the first keystroke.
            coordinator.discardScrollHold()
            moshInCopyMode = false
            guard let ssh = await viewModel.connectForExec() else { return }

            // Probe before spending a round-trip on `mosh-server new`. A custom
            // server path means the user vouches for it — trust it. Otherwise a
            // missing mosh-server degrades to plain SSH (which we already have
            // authenticated right here) with a banner, instead of surfacing a
            // raw `command not found`.
            let caps = await probeCapabilities(over: ssh)
            if connection.moshServerPath == nil && !caps.hasMoshServer {
                degrade = DegradeNotice(missing: .moshServer)
                await fallbackToSSH(over: ssh, capabilities: caps)
                return
            }

            do {
                let creds = try await MoshBootstrap.start(
                    over: ssh,
                    host: connection.host,
                    serverBinary: connection.moshServerPath ?? "mosh-server",
                    portRangeStart: connection.moshPortRangeStart,
                    portRangeEnd: connection.moshPortRangeEnd)
                // mosh+herdr renderer probe, while the bootstrap channel is
                // still open (zero extra connections): does this herdr know
                // `terminal attach`? That raw single-pane stream — no
                // sidebar, no header, pane PTY sized to this client alone —
                // is the renderer the phone wants; an older herdr keeps the
                // full-TUI + immersive-zoom fallback.
                if connection.multiplexer == .herdr {
                    let probe = (try? await ssh.executeCommand(
                        HerdrLaunch.rawAttachProbeCommand(customPath: connection.herdrPath)))
                        .map { String(decoding: $0, as: UTF8.self) } ?? ""
                    herdrRawAttach = probe.contains("MOSHPIT_RAW_ATTACH_OK")
                    if herdrRawAttach {
                        // Retire previous generations' renderers before this
                        // one boots — orphaned loops (mosh-server survives
                        // disconnects by design) must not contest the pane.
                        _ = try? await ssh.executeCommand(
                            HerdrLaunch.staleRendererCleanupCommand(connectionId: connection.id))
                    }
                }
                await ssh.close()   // mosh-server has daemonized; UDP from here

                let transport = try MoshTransport(credentials: creds)
                self.moshTransport = transport

                await MainActor.run {
                    transport.onMetrics = { [weak self] srtt, roaming in
                        self?.metricsSink?(srtt, roaming)
                    }
                    transport.onDiagnostics = { [weak self] diagnostics in
                        self?.diagnosticsSink?(diagnostics)
                    }
                    transport.onReturnPathDead = { [weak self] in
                        self?.moshReturnPathDead = true
                    }
                }

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
                    Task { await transport.resize(cols: cols, rows: rows) }
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
                    for await chunk in transport.hostStream {
                        if Task.isCancelled { break }
                        coordinator.feed(data: chunk)
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
                    // Control plane first: the -CC client lands on the user's
                    // most recent existing session, then the mosh terminal
                    // attaches the SAME one — deterministic, no most-recent
                    // race between the two clients. Stored (not fire-and-
                    // forget) so `stop()` can cancel it — see the property's
                    // doc comment.
                    moshTmuxAttachTask = Task { [weak self] in await self?.attachMoshTmux(transport) }
                case .herdr:
                    // Launch the TUI in the mosh shell (`sendInput` routes
                    // through the ordered mosh write chain), then bring up the
                    // control plane on its own SSH connection — mosh closed
                    // the bootstrap one, and mosh can't carry the exec channel
                    // `herdr api snapshot` needs.
                    //
                    // No ordering constraint between the two, unlike mosh+tmux:
                    // that path has to attach its -CC sidecar FIRST so both
                    // clients land on the same session. herdr has one server
                    // and one focus, so the poller simply reports whatever the
                    // TUI is showing, whenever it comes up.
                    // Raw-attach loop when the host's herdr supports it: the
                    // mosh screen becomes ONE pane's raw stream (the sidecar
                    // steers which one); otherwise the classic full TUI.
                    let boot: String
                    if herdrRawAttach {
                        let key = HerdrLaunch.moshRendererKey(
                            connectionId: connection.id,
                            nonce: String(UUID().uuidString.prefix(8)))
                        moshRendererKey = key
                        boot = HerdrLaunch.rawAttachLoopCommand(
                            rendererKey: key, customPath: connection.herdrPath) + "\r"
                    } else {
                        boot = HerdrLaunch.attachCommand(customPath: connection.herdrPath) + "\r"
                    }
                    // Straight onto the transport, NOT through sendInput: this
                    // runs before markConnected(), and sendInput drops
                    // everything while connState isn't .live (the reconnect
                    // input gate). The gate eating this boot line meant herdr's
                    // TUI never launched over mosh — the session sat at a bare
                    // shell with an empty agents tree (user report,
                    // 2026-08-17). Same direct-send the mosh+tmux renderer
                    // attach already uses.
                    await transport.send(Data(boot.utf8))
                    herdrSidecarTask = Task { [weak self] in await self?.startHerdrSidecar() }
                case .none:
                    break
                }
                viewModel.markConnected()
            } catch {
                let message = (error as? CustomStringConvertible)?.description ?? "\(error)"
                viewModel.fail(String(localized: "Mosh: \(message)"))
            }
        }

        /// Degrade path for a mosh connection whose host has no mosh-server:
        /// reuse the already-authenticated SSH session as a plain single-pane
        /// shell (request the PTY `connectForExec` skipped, then pump). If the
        /// host has tmux and the user asked for it, boot tmux -CC exactly like
        /// the SSH path so multiplexing still works over the fallback.
        private func fallbackToSSH(over session: SSHSession,
                                   capabilities caps: HostCapabilities) async {
            // Same routing note as attempt(): sendInput is the choke point.
            coordinator.onInput = { [weak self] data in self?.sendInput(data) }
            coordinator.onSizeChange = { [viewModel] cols, rows in
                viewModel.resize(rows: rows, cols: cols)
            }
            let grid = coordinator.lastReportedSize ?? Self.estimateGrid(fontSize: lastFontSize)
            do {
                try await session.requestPTY(rows: grid.rows, cols: grid.cols)
            } catch {
                viewModel.fail((error as? SSHError)?.description ?? "\(error)")
                return
            }
            if let size = coordinator.lastReportedSize {
                viewModel.resize(rows: size.rows, cols: size.cols)
            }

            // If the user wanted a multiplexer and the host has it, give it to
            // them over this SSH fallback so multiplexing still works. If it's
            // also missing, the mosh-server banner already points at the same
            // Install Assist sheet, so we don't stack a second banner — just
            // run the bare shell.
            let chosen = connection.multiplexer
            let available = connection.multiplexerPath != nil || caps.has(chosen)
            let multiplexer: Multiplexer = available ? chosen : .none
            if multiplexer == .tmux {
                let controller = TmuxSessionController(sshSession: session)
                controller.transportIsLive = { [weak viewModel] in
                    viewModel?.connState == .live
                }
                controller.pendingRestore = lastSelection
                controller.configureAppearance(theme: lastTheme, fontSize: lastFontSize, fontName: lastFontName,
                                                cursorShape: lastCursorShape, cursorColorId: lastCursorColorId,
                                                cursorBlink: lastCursorBlink)
                controller.setInitialClientSize(cols: grid.cols, rows: grid.rows)
                await controller.beginControlMode()
                let tmux = connection.tmuxPath ?? "tmux"
                let boot = "\(tmux) set -g history-limit 50000 2>/dev/null; \(tmux) -CC attach\r"
                if let bytes = boot.data(using: .utf8) {
                    try? await session.write(bytes)
                }
                tmuxController = controller
                beginAttachTimeout(controller: controller)
            } else {
                let coordinator = self.coordinator
                pumpTask = Task { [weak self] in
                    for await chunk in session.dataStream {
                        if Task.isCancelled { break }
                        coordinator.feed(data: chunk)
                    }
                    _ = self
                }
                if multiplexer == .herdr {
                    // Same native frame rendering as the direct-SSH path —
                    // this fallback IS an SSH session, it just arrived here
                    // because mosh-server was missing.
                    startHerdrFrames(over: session)
                    startHerdrControlPlane(over: session)
                }
            }
            viewModel.markConnected()
        }

        /// Attach tmux over mosh, control-plane first:
        ///   1. The -CC sidecar attaches the user's existing session (most
        ///      recent). If the server has none it retries every 2s — the
        ///      moment the user creates one (here or anywhere) it latches on.
        ///   2. The mosh terminal then attaches the SAME session by name.
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

        private func attachMoshTmux(_ transport: MoshTransport) async {
            await startMoshControlPlane()
            guard !isStopping else { return }
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
                guard !isStopping else { return }
                if let control = moshControl, control.snapshot.isAttached,
                   let id = control.snapshot.activeSessionId,
                   let fallback = control.snapshot.sessions[id]?.name {
                    // Prefer the REMEMBERED session (persisted selection) over
                    // whatever the -CC chain landed on ("most recent"): after a
                    // protocol switch or relaunch, the renderer must return to
                    // the user's session. The sidecar aligns itself (and the
                    // window/pane) via pendingRestore; select-window/pane are
                    // session-scoped, so they move this renderer too — but a
                    // renderer on the WRONG SESSION could never be moved from
                    // the sidecar, which is why the name must be right here.
                    let name = lastSelection.flatMap {
                        control.snapshot.sessions[$0.session]?.name
                    } ?? fallback
                    await attachMoshRenderer(transport, control: control, tmux: tmux, session: name)
                    // Pin the tmux window to the phone grid through the sidecar
                    // so the freshly-attached mosh TUI fills the screen instead
                    // of inheriting the sidecar's 80×24.
                    await pinMoshWindow()
                    // A copy-mode left over from the previous connection would
                    // show the renderer an old scrolled-away view and make the
                    // first swipe jump to that stale offset — cancel it.
                    control.cancelStaleCopyMode()
                    return
                }
                try? await Task.sleep(nanoseconds: 250_000_000)
            }
            if sidecarSSH == nil {
                // Control plane unreachable (SSH blocked?) — run tmux
                // standalone over mosh; its own message explains an empty
                // server ("no sessions").
                await transport.send(Data("\(tmux) attach\r".utf8))
            }
            // Sidecar alive but unattached = no sessions on the server.
            // Leave the mosh shell untouched; the user creates the first
            // session explicitly and the retrying sidecar follows.
        }

        /// Type the attach line into the mosh shell and VERIFY — via the
        /// sidecar, whose `list-clients` is authoritative — that a new
        /// non-control client actually appeared. A login shell that is still
        /// initialising can eat pre-typed input (Powerlevel10k's instant
        /// prompt famously flushes pending stdin), which left the renderer
        /// sitting at a bare prompt: "mosh connected but never attached".
        /// Retry with a leading ^U (kill-line clears any half-swallowed
        /// junk) until the client count rises past the pre-attach baseline —
        /// a delta, because desktop clients may already be attached.
        private func attachMoshRenderer(_ transport: MoshTransport,
                                        control: TmuxSessionController,
                                        tmux: String, session name: String) async {
            let baseline = await nonControlClientCount(control)
            for _ in 0..<4 {
                guard !isStopping else { return }
                await transport.send(Data("\u{15}\(tmux) attach -t \(Self.shellQuote(name))\r".utf8))
                try? await Task.sleep(nanoseconds: 1_500_000_000)
                let now = await nonControlClientCount(control)
                if now < 0 || now > max(baseline, 0) { return }   // attached (or sidecar gone — stop)
            }
        }

        /// `list-clients` non-control count over the sidecar, bounded by a 2s
        /// timeout so a dying control channel can't hang the attach loop.
        private func nonControlClientCount(_ control: TmuxSessionController) async -> Int {
            await withTaskGroup(of: Int.self) { group in
                group.addTask { @MainActor in
                    await withCheckedContinuation { cont in
                        control.countNonControlClients { cont.resume(returning: $0) }
                    }
                }
                group.addTask {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    return -1
                }
                let first = await group.next() ?? -1
                group.cancelAll()
                return first
            }
        }

        /// Open the control-plane SSH session and run a `tmux -CC` control
        /// client against the user's existing sessions. The control client
        /// stays sizeless (`rendersOutput: false`), so it never receives pane
        /// output and never affects window sizing; it exists purely to push
        /// live session/window/pane state into the native breadcrumb + sheets.
        private func startMoshControlPlane(preferred: String? = nil) async {
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
            guard let ssh = try? await SSHService.shared.connect(connection) else { return }
            // The session may have been torn down (stop()) while this SSH
            // handshake was in flight — don't wire a fresh connection onto a
            // dead session (leaked, never-closed, and racing the NEW
            // session's own bootstrap over this same shared SSHService).
            guard !isStopping else { await ssh.close(); return }
            // -CC needs a controlling terminal; the PTY size is irrelevant
            // (control clients are excluded from sizing until they report one,
            // which we never do).
            try? await ssh.requestPTY(rows: 24, cols: 80)
            sidecarSSH = ssh

            let controller = TmuxSessionController(sshSession: ssh, rendersOutput: false)
            controller.pendingRestore = lastSelection   // restore window/pane too
            await controller.beginControlMode()
            let tmux = connection.tmuxPath ?? "tmux"
            // Attach the preferred session (resume) or the most recent one;
            // with an empty server, retry until a session appears. Attach
            // only — Moshpit never creates sessions on its own.
            var chain = "while ! \(tmux) -CC attach 2>/dev/null; do sleep 2; done"
            if let preferred {
                chain = "\(tmux) -CC attach -t \(Self.shellQuote(preferred)) 2>/dev/null || " + chain
            }
            let boot = "\(tmux) set -g history-limit 50000 2>/dev/null; " + chain + "\r"
            try? await ssh.write(Data(boot.utf8))
            moshControl = controller
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
            let deadline = Date().addingTimeInterval(tmuxAttachTimeoutSeconds)
            attachTimeoutTask = Task { [weak self, weak controller] in
                while Date() < deadline {
                    if Task.isCancelled { return }
                    guard let controller, !controller.snapshot.isAttached else { return }
                    try? await Task.sleep(nanoseconds: 500_000_000)
                }
                guard !Task.isCancelled, let self, let controller,
                      !controller.snapshot.isAttached else { return }
                self.attachStalled = true
            }
        }

        func stop() async {
            captureSelection()   // remember where we were, for the next attach
            pumpTask?.cancel()
            pumpTask = nil
            attachTimeoutTask?.cancel()
            attachTimeoutTask = nil
            moshTmuxAttachTask?.cancel()
            moshTmuxAttachTask = nil
            herdrSidecarTask?.cancel()
            herdrSidecarTask = nil
            herdrRetargetTask?.cancel()
            herdrRetargetTask = nil
            if herdrFrameTarget != nil {
                // Hand the attach back: herdr's direct attach is exclusive per
                // terminal, so leaving it claimed would make the next connect
                // rely on `--takeover` to evict a client that no longer exists.
                // Awaited through the write chain so it actually reaches the
                // wire before the channel closes.
                writeFrameCommand(HerdrFrameCommand.release)
                await herdrWriteChain?.value
            }
            herdrFrameTarget = nil
            herdrFrameSSH = nil
            herdrWriteChain = nil
            // The raw-attach loop dies with the mosh shell; only the local
            // steering state needs resetting so a reconnect re-targets.
            moshRawAttachTarget = nil
            moshRendererKey = nil
            herdrRawAttach = false
            herdrPushTask?.cancel()
            herdrPushTask = nil
            herdrPushPump?.cancel()
            herdrPushPump = nil
            if let push = herdrPush { await push.shutdown() }
            herdrPush = nil
            if let ssh = herdrPushSSH { await ssh.close() }
            herdrPushSSH = nil
            // Just a poll timer — nothing to hand back to the server the way
            // tmux's pinned window sizes have to be.
            herdrControl?.stop()
            herdrControl = nil
            if let controller = tmuxController {
                // Hand back the pinned window sizes so other clients get their
                // full width back. Do it over a one-shot EXEC channel
                // (executeCommand awaits server completion) rather than the
                // in-band -CC write, which raced the teardown and left windows
                // stuck at the phone size (`resize-window -A` clears the manual
                // size override; tmux then re-sizes to the desktop client).
                // Zoom is deliberately left as-is: Moshpit's model is "the user
                // picks one pane, full-screen" — the split layout isn't ours
                // to restore (`prefix z` brings it back anywhere).
                if let ssh = viewModel.session {
                    let tmux = connection.tmuxPath ?? "tmux"
                    for win in controller.resizedWindows {
                        // Unset the per-window override — the real "back to
                        // automatic" (`resize-window -A` re-pins, still manual).
                        _ = try? await ssh.executeCommand("\(tmux) set-option -u -w -t \(win) window-size")
                    }
                }
                await controller.detach()
            }
            tmuxController = nil
            if let control = moshControl {
                // Restore the status bar over the LIVE control channel (a
                // round-trip blocks until tmux applies it). The exec channel
                // mangled the `$id` session target through the login shell, so
                // status never came back; the control channel speaks straight
                // to tmux like the suppression did.
                await control.restoreSuppressedStatusAndFlush()
                // Restore window sizes over a ONE-SHOT exec channel on the
                // sidecar SSH connection (these target windows by `@id`, which
                // survives the shell). Zoom is left as-is — see the SSH path.
                if let ssh = sidecarSSH {
                    let tmux = connection.tmuxPath ?? "tmux"
                    // Un-pin the windows we resized to the phone grid so other
                    // clients (and the next desktop attach) get automatic sizing
                    // back — unset the per-window override (`resize-window -A`
                    // re-pins, still manual). Over the one-shot exec channel so
                    // it can't race teardown (same as the SSH path above).
                    for win in control.resizedWindows {
                        _ = try? await ssh.executeCommand("\(tmux) set-option -u -w -t \(win) window-size")
                    }
                }
                await control.detach()
            }
            moshControl = nil
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
            await viewModel.disconnect()
        }

        /// The transport died on its own — flag the UI and reconnect.
        func connectionDropped() async {
            guard !isStopping else { return }
            viewModel.markReconnecting()
            await reconnect()
        }

        /// Periodic keepalive + health check (driven by the hub timer while the
        /// app is foreground). Doubles as NAT-warmth (the probe is real
        /// traffic) and fast death detection, and retries a stalled reconnect.
        func keepAlive() async {
            guard !isStopping, !isReconnecting else { return }
            // mosh owns its own UDP resume/roam; don't probe its closed bootstrap.
            if moshTransport != nil { await resumeIfNeeded(); return }
            switch viewModel.status {
            case .connected:
                if !(await isTransportAlive()) {
                    viewModel.markReconnecting()
                    await reconnect()
                }
            case .reconnecting, .disconnected, .failed:
                await reconnect()   // retry on the next tick (acts as backoff)
            case .idle, .connecting:
                break               // initial connect in flight — leave it
            }
        }

        /// Tear down the dead transport and run the start flow again. tmux
        /// `attach` reattaches to the same server-side session, so panes + their
        /// scrollback come right back.
        func reconnect() async {
            guard !isStopping, !isReconnecting, !isResuming else { return }
            isReconnecting = true
            defer { isReconnecting = false }
            await stop()
            viewModel.resetForReconnect()
            await start(theme: lastTheme, fontSize: lastFontSize, fontName: lastFontName,
                        cursorShape: lastCursorShape, cursorColorId: lastCursorColorId,
                        cursorBlink: lastCursorBlink, automatic: true)
        }

        /// Unconditional resume after a long suspension — skip the (unreliable
        /// on a half-open socket) probe and just reconnect. mosh keeps its own
        /// UDP resume path.
        func forceResume() async {
            guard !isStopping else { return }
            if let transport = moshTransport {
                // A REAL suspension (>20s background). The UDP flow may be a
                // zombie — .ready in name, blackhole in practice — so replace
                // it outright (roam-equivalent, server re-homes on the first
                // flush) before the sidecar rebuild runs.
                await transport.resume(force: true)
                await resumeIfNeeded()
                return
            }
            viewModel.markReconnecting()
            await reconnect()
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
        func resumeIfNeeded() async {
            guard !isStopping, !isReconnecting, !isResuming else { return }
            isResuming = true
            defer { isResuming = false }
            if let transport = moshTransport {
                await transport.resume()
                // Both control planes ride SSH, which iOS kills during
                // suspension — rebuild whichever this connection uses.
                if connection.multiplexer == .herdr {
                    // Cheaper than the tmux rebuild below: nothing to re-target
                    // (herdr's focus lives on the server) and no window to
                    // re-pin, so just replace the dead connection under the
                    // poller. Retried every keepalive tick while it's down.
                    let alive = await sidecarSSH?.isConnected ?? false
                    if !alive || herdrControl == nil {
                        herdrControl?.stop()
                        herdrControl = nil
                        if let ssh = sidecarSSH { await ssh.close() }
                        sidecarSSH = nil
                        await startHerdrSidecar()
                    }
                }
                if connection.multiplexer == .tmux {
                    // Rebuild the -CC sidecar when its SSH is dead OR it never
                    // came back from a previous attempt (moshControl == nil):
                    // the breadcrumb is driven entirely by this sidecar's
                    // snapshot, so a failed re-establish would otherwise leave it
                    // blank until the app restarts. Since keepalive funnels
                    // through here, a nil controller is retried every cycle.
                    let alive = await sidecarSSH?.isConnected ?? false
                    if !alive || moshControl == nil {
                        captureSelection()   // remember window/pane before rebuild
                        let preferred = moshControl.flatMap { c in
                            c.snapshot.activeSessionId.flatMap { c.snapshot.sessions[$0]?.name }
                        }
                        if let control = moshControl { await control.detach() }
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
                }
                return
            }
            guard viewModel.session != nil else { return }
            // `isConnected` is just a local `!closed` flag — iOS silently kills
            // the TCP socket during suspension without NIO seeing a close, so it
            // lies (home shows "connected" but -CC can't attach). Actively
            // round-trip a trivial command with a short timeout instead.
            if await isTransportAlive() { return }

            // Dead transport: tear down without touching the UI layer, then
            // run the exact same start flow (tmux `new -A` reuses the session).
            await stop()
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
        func isTransportAlive(timeout: Double = 8) async -> Bool {
            guard let ssh = viewModel.session else { return false }
            return await withTimeoutValue(timeout) { try await ssh.executeCommand("true") } != nil
        }
    }

    private(set) var sessions: [UUID: ActiveSession] = [:]

    @ObservationIgnored private let metrics: SessionMetricsRegistry
    @ObservationIgnored private var keepAliveTimer: Timer?
    @ObservationIgnored private var lastBackgroundedAt: Date?
    /// How often to probe live sessions while foreground (NAT-warmth + death
    /// detection + reconnect retry). iOS suspends timers in the background.
    private let keepAliveInterval: TimeInterval = 12
    /// Background longer than this → force a fresh reconnect on return rather
    /// than trusting the liveness probe (iOS has almost certainly killed TCP).
    private let forceReconnectAfter: TimeInterval = 20

    init(metrics: SessionMetricsRegistry) {
        self.metrics = metrics
    }

    /// Drive keepalive off the app's foreground state. Active → probe now and
    /// every `keepAliveInterval`; background → stop (iOS would suspend us anyway).
    /// The session whose TerminalScreen is currently presented (nil on Home).
    /// Set/cleared by TerminalScreen; the foreground transition re-pins only
    /// this one — re-pinning windows nobody is looking at would shrink them
    /// for desktop clients for no benefit.
    weak var visibleSession: ActiveSession?

    func setForeground(_ active: Bool) {
        if active {
            // A long suspension almost certainly means iOS killed the socket,
            // and the liveness probe can false-positive on a half-open channel
            // (green pill, frozen -CC pane). Past the threshold, force a fresh
            // reconnect instead of trusting the probe.
            let away = lastBackgroundedAt.map { Date().timeIntervalSince($0) } ?? 0
            lastBackgroundedAt = nil
            let force = away > forceReconnectAfter
            // Take back the phone-grid pin for the terminal on screen (released
            // on backgrounding below).
            visibleSession?.repinForeground()
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
            case .idle, .failed, .disconnected: isDead = true
            case .connecting, .connected, .reconnecting: isDead = false
            }
            if !isDead || existing.connection == connection {
                return existing
            }
        }
        let active = ActiveSession(connection: connection)
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

    /// Returns the existing live session or creates + starts a new one.
    /// Prefer prepare()+start() from screens that present the host-key
    /// prompt; this convenience keeps non-UI callers working.
    @discardableResult
    func connect(_ connection: ServerConnection,
                 theme: TerminalTheme,
                 fontSize: Double) async -> ActiveSession {
        let active = prepare(connection)
        await start(active, theme: theme, fontSize: fontSize)
        return active
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
                        await session.resumeIfNeeded()
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
