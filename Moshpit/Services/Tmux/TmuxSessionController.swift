import Foundation
import MoshpitKit
import Observation
import SwiftUI
import UIKit
import os

/// One pane's raw `@moshpit_*` hook stamp, as read off the tmux control channel
/// (Phase B "hook stamp" bridge). Deliberately framework-free and string-typed:
/// the controller does not own the `AgentActivityAttributes.AgentState` mapping
/// (that's the monitor's job), and an unset tmux user option reads back as an
/// empty string, normalised to `nil` here.
///
///   - `state`: `"working"` | `"attention"` | `"done"`, or `nil` when the pane
///     has no precise hook data (unset or unrecognised → monitor falls back to
///     the output heuristic for that pane).
///   - `agent`:  `@moshpit_agent`, e.g. `"claude"` / `"codex"` (drives the row label).
///   - `since`:  `@moshpit_since` as a `Date`, or `nil` (caller uses `Date()`).
///   - `title`:  optional `@moshpit_title` headline; `nil` when unset.
struct AgentHook: Equatable, Sendable {
    var state: String?
    var agent: String?
    var since: Date?
    /// What is actually RUNNING in the pane right now — polled fresh every
    /// round, unlike the discovery snapshot's copy, which can be minutes stale.
    /// The monitor uses it to spot stamps whose agent has since died: hooks
    /// only ever write the options, and a killed agent fires no final hook, so
    /// `attention` freezes at the moment of death unless something asks what
    /// the pane is doing NOW.
    var command: String?
    var title: String?
}

/// State owner for a tmux control-mode session.
///
/// ### Contract (do not break)
///   - The **only** observation-tracked surface is ``snapshot``. SwiftUI must
///     never observe `paneTerminals` or `paneCoordinators` directly; they are
///     intentionally `@ObservationIgnored` so window switches do not destroy
///     any terminal view.
///   - ``surface(for:)`` returns the **same** terminal (``TerminalSurface``)
///     for the same `paneId` for the entire lifetime of that pane. New
///     terminals are minted lazily the first time a pane is encountered.
///   - `%output` bytes from the parser are routed *directly* to the
///     coordinator's `feed(data:)`. There is no per-pane output buffer.
///
/// ### Threading
/// Everything is `@MainActor`. Terminal views and SwiftUI view-tree updates
/// both require the main actor, and parser callbacks
/// dispatch back to us via `Task { @MainActor in … }`.
@Observable
@MainActor
class TmuxSessionController: MultiplexerControlling {

    /// `MultiplexerControlling.multiplexer` — drives the vocabulary and key
    /// hints the sheets print.
    nonisolated var multiplexer: Multiplexer { .tmux }

    // MARK: - Snapshot

    /// Snapshot type now lives top-level as ``TmuxSnapshot`` so the mosh
    /// sidecar can produce it too; kept as a nested alias for call sites and
    /// tests that reference `TmuxSessionController.Snapshot`.
    typealias Snapshot = TmuxSnapshot

    // MARK: - Observable state

    /// **The single observation-tracked surface.** SwiftUI views must only
    /// read fields off this snapshot — everything else on the controller is
    /// `@ObservationIgnored` and is accessed through dedicated methods.
    private(set) var snapshot = Snapshot()

    // MARK: - Non-observed terminal pool

    /// Persistent terminal per pane. Created lazily the first time a pane is
    /// seen, kept until tmux closes the pane / window.
    @ObservationIgnored
    private var paneTerminals: [String: TerminalSurface] = [:]

    /// Coordinators paired with each terminal. The coordinator owns the
    /// data-feed path and forwards user input back through ``sendInput``.
    @ObservationIgnored
    private var paneCoordinators: [String: SwiftTerminalView.Coordinator] = [:]

    /// Makes each pane terminal's path links (`RemotePathLinks`) — the
    /// session supplies it, since the probe rides its file-transfer SSH.
    /// Panes minted before it is set get theirs when it is.
    @ObservationIgnored
    var pathLinksFactory: ((_ paneId: String) -> RemotePathLinks)? {
        didSet {
            for (paneId, coordinator) in paneCoordinators where coordinator.pathLinks == nil {
                coordinator.pathLinks = pathLinksFactory?(paneId)
            }
        }
    }

    /// A tapped host-path link in any pane.
    @ObservationIgnored
    var onOpenRemotePath: ((String) -> Void)?

    /// Coordinator of the pane currently on screen — the target for one-shot
    /// input modifiers (sticky Ctrl) and other "whatever the user is typing
    /// into right now" concerns.
    var activeCoordinator: SwiftTerminalView.Coordinator? {
        guard let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id
        else { return nil }
        return paneCoordinators[paneId]
    }

    /// Pinch-zoom commits bubble here (per-pane coordinators are private);
    /// the screen persists the size into AppSettings.
    @ObservationIgnored var onFontSizeCommitted: ((Double) -> Void)?

    /// Active theme/font applied to newly minted terminals. Updated by
    /// ``configureAppearance(theme:fontSize:)``.
    @ObservationIgnored
    private var currentTheme: TerminalTheme = .dracula

    @ObservationIgnored
    private var currentFontSize: Double = 14

    @ObservationIgnored
    private var currentFontName: String = "system"

    @ObservationIgnored
    private var cursorShape: CursorShape = .block
    @ObservationIgnored
    private var cursorColorId: String = "teal"
    @ObservationIgnored
    private var cursorBlink: Bool = true

    // MARK: - Transport + parser

    @ObservationIgnored
    private let sshSession: TmuxTransport

    /// `true` (default): this controller renders pane content into SwiftTerm
    /// views (the SSH+tmux golden path). `false`: control-plane-only — the
    /// mosh sidecar mode, where mosh renders the screen and this controller
    /// only tracks sessions/windows/panes for the breadcrumb + sheets. In
    /// that mode the control client deliberately never reports a size, so
    /// tmux suppresses %output to it AND ignores it for window sizing.
    @ObservationIgnored
    private let rendersOutput: Bool

    /// A selection captured before a reconnect, replayed once discovery has the
    /// ids again so the user lands back on the session/window/pane they were
    /// viewing (instead of tmux's most-recently-active). Set by the hub right
    /// after this controller is created; consumed once.
    @ObservationIgnored
    var pendingRestore: TmuxSelection?

    @ObservationIgnored
    let client = TmuxControlClient()

    @ObservationIgnored
    private var pumpTask: Task<Void, Never>?

    // MARK: - Agent monitor hooks (Vibe Island)

    /// Fired (main actor) whenever a pane produces output. Used by the agent
    /// activity monitor to infer working / idle state.
    @ObservationIgnored
    var onPaneActivity: ((String) -> Void)?

    /// Fired when a pane rings the bell — the strongest "agent needs
    /// attention" signal we get from CLI coding agents.
    @ObservationIgnored
    var onPaneBell: ((String) -> Void)?

    /// Precise per-pane agent state stamped by coding-agent hooks into tmux
    /// `@moshpit_*` USER options (Phase B). Keyed by paneId (`%N`) — the SAME
    /// key as `snapshot.panes` and the Phase A agent ids, so the monitor joins
    /// it directly. Rebuilt wholesale by every ``pollAgentHooks()`` so a pane
    /// whose options were unset drops out (the monitor then reverts it to the
    /// output heuristic). `@ObservationIgnored`: the monitor reads it
    /// imperatively on its own sweep, preserving the file's "snapshot is the
    /// only observed surface" contract.
    @ObservationIgnored
    private(set) var agentHooks: [String: AgentHook] = [:]

    /// True once ``agentHooks`` has been rebuilt from a poll reply or taken
    /// over from a retired controller (``inheritAgentHooks``). Until then the
    /// empty dictionary is "not read yet", which the breadcrumb treats as
    /// incomplete rather than as "no agent here".
    private(set) var agentHooksLoaded = false

    /// Invoked on the main actor at the end of each ``pollAgentHooks()`` after
    /// ``agentHooks`` has been rebuilt, so the monitor can re-sync the Vibe
    /// Island immediately instead of waiting for its next sweep.
    @ObservationIgnored
    var onAgentHooksUpdated: (() -> Void)?

    /// Pending callbacks for `%begin…%end` blocks, in send order. tmux
    /// responds to EVERY control-mode command with a block — including
    /// fire-and-forget ones like `send-keys` and `refresh-client` — so every
    /// command must occupy a slot (`nil` = response intentionally ignored)
    /// or the FIFO desyncs and replies land in the wrong parser.
    @ObservationIgnored
    private var pendingCallbacks: [PendingCommand] = []

    /// One slot of ``pendingCallbacks``.
    private struct PendingCommand {
        let command: String
        let callback: ((TmuxCommandResponse) -> Void)?
        /// Someone is awaiting this reply (a client count, a pane's path, a
        /// flush): if the channel closes first, the callback gets
        /// ``TmuxCommandResponse/channelClosed`` rather than never running.
        /// Every other callback updates state from a reply and is simply
        /// dropped — an error in its place would read as "tmux listed
        /// nothing" and empty the snapshot under the last frame on screen.
        let waits: Bool
    }

    // MARK: - Diagnostic tap
    //
    // `-MOSHPIT_CC_TAP <dir>` (Debug tooling, simulator e2e): append-only
    // traces of the raw -CC stream, every command↔response pairing, and every
    // frame fed to a pane — the ground truth for chasing pairing shifts
    // ("protocol text painted into a pane") without guessing from screenshots.
    nonisolated static let ccTapDir: String? = {
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-MOSHPIT_CC_TAP"), i + 1 < args.count {
            // On a real device no host path is writable — the literal value
            // "documents" resolves to the app sandbox's Documents directory,
            // which `devicectl device copy from --domain-type appDataContainer`
            // can pull afterwards.
            if args[i + 1] == "documents" {
                return FileManager.default
                    .urls(for: .documentDirectory, in: .userDomainMask)
                    .first?.path
            }
            return args[i + 1]
        }
        // Test runners can't add launch arguments, but TEST_RUNNER_-prefixed
        // env vars pass straight through (the tmux-cc-lab e2e uses this).
        return ProcessInfo.processInfo.environment["MOSHPIT_CC_TAP"]
    }()

    nonisolated static func ccTap(_ file: String, _ data: Data) {
        guard let dir = ccTapDir else { return }
        let url = URL(fileURLWithPath: dir).appendingPathComponent(file)
        if let h = try? FileHandle(forWritingTo: url) {
            defer { try? h.close() }
            _ = try? h.seekToEnd()
            try? h.write(contentsOf: data)
        } else {
            try? data.write(to: url)
        }
    }

    private func ccTapLine(_ text: String) {
        Self.ccTap("cc-pairing.log", Data((text + "\n").utf8))
    }

    /// Serializes writes so commands hit tmux in the same order their
    /// callback slots were queued. A detached Task per send() can reorder.
    @ObservationIgnored
    private var writeChain: Task<Void, Never>?

    /// "Is the transport under this controller live right now?" — set by the
    /// session that owns it. Gates GESTURE-shaped input (pane keyboard) the
    /// same way ActiveSession.sendInput gates the shortcut bar: a keystroke
    /// while the reconnect poster is up is accidental, and the write chain
    /// can deliver it late, after resume, into a pane the user never saw.
    /// Deliberately NOT consulted by programmatic delivery (`sendInput` from
    /// the lock-screen Allow/Deny path), which reconnects first and must not
    /// race the state flip. nil (tests, sidecar) means "always live".
    @ObservationIgnored
    var transportIsLive: (() -> Bool)?

    // MARK: - Init

    /// `sshSession` is typed as the narrow ``TmuxTransport`` protocol so tests
    /// can drive the controller with a `MockTmuxTransport` without touching the
    /// real SSH stack. `SSHSession` conforms naturally; production callers
    /// pass an `SSHSession` instance unchanged.
    /// The view this controller's pane terminals get: the core's when
    /// ``CoreTermSwitch`` was on when the controller was made (one per
    /// attach; a terminal keeps the view it was made with).
    let surfaceKind: TerminalSurfaceKind

    init(sshSession: TmuxTransport, rendersOutput: Bool = true,
         surfaceKind: TerminalSurfaceKind = .current) {
        self.sshSession = sshSession
        self.rendersOutput = rendersOutput
        self.surfaceKind = surfaceKind
    }

    // MARK: - Public API

    /// Apply (and remember) the theme + font size + cursor style used when
    /// creating new pane terminals. Existing terminals are updated in-place,
    /// and only where something changed: this runs each time a terminal
    /// screen appears and on every reconnect, and a font or colour set
    /// again costs a pane a whole-screen repaint (and the font its
    /// selection) for nothing.
    func configureAppearance(theme: TerminalTheme, fontSize: Double,
                             fontName: String = "system",
                             cursorShape: CursorShape = .block,
                             cursorColorId: String = "teal",
                             cursorBlink: Bool = true) {
        currentTheme = theme
        currentFontSize = fontSize
        currentFontName = fontName
        self.cursorShape = cursorShape
        self.cursorColorId = cursorColorId
        self.cursorBlink = cursorBlink
        let font = TerminalFont.font(id: fontName, size: CGFloat(fontSize))
        for terminal in paneTerminals.values {
            terminal.apply(font: font)
            // The theme and the user's cursor again: the cursor is re-asserted
            // after every feed from here on — pane output carries
            // DECSCUSR/OSC-12 (vim, zsh plugins, coding agents) and would
            // otherwise override the user's choice until the next Settings
            // change (the "sometimes wrong" cursor).
            terminal.reassert(look: look)
        }
    }

    /// Wire up parser callbacks and start pumping SSH bytes WITHOUT sending
    /// discovery commands. Use this when tmux has not started yet (the
    /// `tmux -CC new` line is written to the shell *after* this returns) —
    /// discovery fires automatically once `%session-changed` arrives.
    func beginControlMode() async {
        await installCallbacks()
        startPumping()
    }

    /// Wire up parser callbacks, start pumping SSH bytes, and run the
    /// initial discovery sequence (`list-sessions`, `list-windows`,
    /// `list-panes`). Only valid when tmux control mode is already running.
    func attach() async {
        await beginControlMode()
        await sendInitialDiscovery()
    }

    /// Stop pumping bytes and reset the parser. Does **not** close the
    /// underlying SSH session — the caller owns that lifecycle.
    func detach() async {
        pumpTask?.cancel()
        pumpTask = nil
        eventPump?.cancel()
        eventPump = nil
        channelWatchdogTask?.cancel()
        channelWatchdogTask = nil
        await client.reset()
        snapshot.isAttached = false
        closePendingCommands(reason: "detach")
        // Replies for anything still queued will never arrive, so nothing
        // would ever release a parked resync — drop the holds rather than
        // leave the panes permanently unable to repaint.
        backfillsInFlight.removeAll()
        deferredResyncs.removeAll()
        for task in deferredResizeTimeouts.values { task.cancel() }
        deferredResizeTimeouts.removeAll()
        for paneId in paneResizeAwaitingCapture { paneCoordinators[paneId]?.applyDeferredResize() }
        paneResizeAwaitingCapture.removeAll()
        for task in backfillTimeouts.values { task.cancel() }
        backfillTimeouts.removeAll()
        for task in backfillRetryTasks.values { task.cancel() }
        backfillRetryTasks.removeAll()
        backfillRejectStreak.removeAll()
        // Same reasoning for the foreign-width gate: a repair frame that will
        // never arrive must not keep the next connection's output gated.
        activeWindowAtForeignWidth = false
        foreignQuietRepinTask?.cancel()
        foreignQuietRepinTask = nil
        // No reply is coming for a renderer listing still on its way.
        rendererLocateInFlight = false
        // The lease cache names a client that is about to stop existing.
        sizeLeases.removeAll()
        pendingAttachLease = nil
        sizeLeaseProbesInFlight.removeAll()
        ourClientName = nil
        sizeFollow = nil
        foregroundProbeFallback?.cancel()
        foregroundProbeFallback = nil
        resyncRetryTask?.cancel()
        resyncRetryTask = nil
        resyncRejectStreak.removeAll()
        convergenceResyncPending = false
        convergenceSawOutput = false
        convergenceResyncTask?.cancel()
        convergenceResyncTask = nil
        convergenceDeadlineTask?.cancel()
        convergenceDeadlineTask = nil
    }

    /// The user's look, as configured.
    private var look: TerminalLook {
        TerminalLook(theme: currentTheme, cursorShape: cursorShape, cursorColorId: cursorColorId,
                     cursorBlink: cursorBlink)
    }

    /// Returns the persistent terminal for `paneId`. If one does not
    /// yet exist (pane discovered after attach), it is minted on demand
    /// using the currently configured theme/font.
    ///
    /// **The same instance is returned for the same paneId across calls.**
    /// SwiftUI's `UIViewRepresentable.makeUIView` relies on this so window
    /// switches don't destroy the terminal view.
    func surface(for paneId: String) -> TerminalSurface {
        if let existing = paneTerminals[paneId] { return existing }
        return mintTerminal(for: paneId)
    }

    /// The coordinator minted alongside ``surface(for:)`` — the pane
    /// host wires it to its ``TerminalHostContainer`` so keyboard transitions
    /// can freeze the pane's frame (the keyboard/IME garble fix).
    func coordinator(for paneId: String) -> SwiftTerminalView.Coordinator? {
        paneCoordinators[paneId]
    }

    /// Switch the currently active window. Updates the snapshot
    /// optimistically so the UI reflects the change immediately, then asks
    /// tmux to follow with `select-window`.
    func selectWindow(_ windowId: String) {
        // See the matching comment in selectPane: a manual switch must not
        // get silently overridden by a still-pending reconnect restore.
        pendingRestore = nil
        releaseScrollHolds()
        // Cross-session: the tree shows every session's windows, so the target
        // may live in a session we're not attached to — switch there first.
        if let sid = snapshot.windows[windowId]?.sessionId, !sid.isEmpty,
           sid != snapshot.activeSessionId {
            selectSession(sid)
        }
        let oldWindow = snapshot.activeWindowId
        if oldWindow != windowId {
            // Hand the old window's width pin back before switching.
            if let oldWindow { releaseWindowPin(oldWindow) }
            snapshot.activeWindowId = windowId
            // Point activePaneId at the new window's pane RIGHT NOW. The view
            // renders whichever pane activePaneId names; without this it keeps
            // showing the previous window's pane until async discovery lands
            // (the "switch shows the last pane" bug).
            if let pane = snapshot.panes.values.first(where: { $0.windowId == windowId && $0.isActive })
                ?? snapshot.panes.values.first(where: { $0.windowId == windowId }) {
                // Veil the pane we're about to show BEFORE SwiftUI swaps it in:
                // its persistent terminal still holds stale content until the
                // resync frame lands, and a clean background veil (revealed by
                // resyncPane) reads as a transition instead of a glitch.
                paneCoordinators[pane.id]?.veilForSwitch()
                // See the matching comment in selectPane — the resync capture
                // can queue behind a reconnect's discovery backlog and easily
                // outlast the veil's default 0.8s safety timeout.
                paneCoordinators[pane.id]?.extendCoverTimeout(by: coverGrace())
                // Explicit withAnimation, not a view-side `.animation(value:)` —
                // this mutation originates from a UIKit gesture-recognizer
                // callback (TerminalScrollGesture), not a native SwiftUI action,
                // and `.animation(value:)` alone silently failed to pick up
                // changes from that call path (verified: the pane swap landed
                // as one un-animated cut, frame-by-frame, with no in-between
                // slide). Bracketing the mutation itself is what actually
                // establishes the transaction TmuxPaneSplitView's `.transition`
                // needs to animate.
                withAnimation(.easeInOut(duration: 0.22)) {
                    snapshot.activePaneId = pane.id
                }
            }
        }
        // Pre-size the target to OUR width BEFORE activating it. `select-window`
        // makes tmux start streaming the window's %output; if the window is
        // currently desktop-wide (another client touched it, or we handed it to a
        // desktop client's size while backgrounded), that wide output lands in our
        // 70-col pane with CJK in the wrong cells → `？`/tofu on switch. tmux runs
        // the two in send order, so resizing first means the post-select stream
        // already arrives at our width. (A no-op width change is harmless.)
        // A switch is the person's doing: the target window becomes ours.
        requestPin(windowId, reason: .user)
        if !rendersOutput {
            // B270: the select moves the phone's renderer, which sizes the
            // window to the phone at once — ahead of a pin that may wait on
            // the lease: its layout is read before it.
            capturePristineLayout(windowId)
        }
        send(rawCommand: "select-window -t \(windowId)")
        // Landing on a split window: zoom its active pane so we show a single
        // full-screen pane.
        ensureImmersiveZoom()
        refreshActivePaneMouse()   // wheel-vs-copy-mode signal for the new pane
        // Fill whatever this window's panes still need. On attach only the
        // on-screen window is dumped, so arriving here is where the rest earn
        // their `capture-pane` — deferred, never dropped.
        ensureTerminalsForAllPanes()
        // The pane's local buffer may have drifted while hidden (%output only
        // carries new bytes; resizes reflow the local buffer independently of
        // tmux). Repaint it from tmux's model — FIFO ordering puts the capture
        // after the select/resize above, so the frame arrives at the new size.
        // Two passes, because the resize above just triggered the target
        // app's own SIGWINCH repaint and the immediate capture races it —
        // see resyncActivePaneSettling.
        resyncActivePaneSettling()
    }

    /// Windows we forced to the phone's size so full-screen TUIs reflow to
    /// our width. Reset to automatic on disconnect — the hub does this over a
    /// one-shot exec channel (`resize-window -A`), since the in-band -CC write
    /// races the teardown and the restore was getting lost (window stayed
    /// phone-sized for desktop clients). Exposed read-only for that path.
    @ObservationIgnored
    private(set) var resizedWindows: Set<String> = []

    // MARK: - Window size ownership (two Moshpit devices on one window)

    /// Who a window's size belongs to, as recorded in the tmux window option
    /// `@moshpit_size_owner` (`<client_name>|<label>|<stamp>`).
    ///
    /// tmux's own `window-size latest` cannot arbitrate between two `-CC`
    /// clients — control traffic never counts as activity — and without an
    /// arbiter two Moshpit devices on one window each re-pinned it to their
    /// own grid on every drift, forever: the 2s quiet re-pin on each side kept
    /// the other side's fight alive (iPad and iPhone open together, 2026-09-14).
    /// The lease names the device the window belongs to; the other device
    /// FOLLOWS — keeps its last frame under a veil — instead of reclaiming,
    /// until the person touches it. The stamp is the holder's last
    /// interaction (0 for a passive claim), so a device coming to the front
    /// can take an idle window but never one someone is typing into.
    struct SizeLease: Equatable {
        /// tmux `#{client_name}` of the holder — its liveness check: a lease
        /// left behind by a client that is no longer attached (a crash, a
        /// reconnect) is nobody's.
        var clientName: String
        /// What the holder calls itself on the follower's veil ("iPad").
        var label: String
        /// Unix time of the holder's last interaction; 0 for a passive claim.
        var stamp: TimeInterval
        /// Whether `clientName` was attached when last probed.
        var live: Bool

        init(clientName: String, label: String, stamp: TimeInterval, live: Bool) {
            self.clientName = clientName
            self.label = label
            self.stamp = stamp
            self.live = live
        }

        /// `nil` for anything that is not a lease (unset expands to "", and a
        /// test harness's generic reply is a bare `0 0`).
        init?(parsing raw: String, clients: Set<String>) {
            let parts = raw.tmuxFields(separator: "|", maxSplits: 2, omittingEmptySubsequences: false)
            guard parts.count == 3, !parts[0].isEmpty, !parts[1].isEmpty,
                  let stamp = TimeInterval(parts[2]) else { return nil }
            self.clientName = parts[0]
            self.label = parts[1]
            self.stamp = stamp
            // A reply with no clients at all is a failed listing, not an
            // empty server — we are a client ourselves. Err toward "live":
            // following a ghost costs one tap, stealing a window costs a fight.
            self.live = clients.isEmpty || clients.contains(clientName)
        }

        var encoded: String { "\(clientName)|\(label)|\(Int(stamp))" }
    }

    /// What we last learned about a window's lease. No entry: never probed.
    enum SizeLeaseState: Equatable {
        case unset
        case held(SizeLease)
    }

    @ObservationIgnored private var sizeLeases: [String: SizeLeaseState] = [:]
    /// The lease of the session's current window, read at attach before the
    /// window list lands, adopted by the first pin.
    @ObservationIgnored private var pendingAttachLease: SizeLeaseState?
    @ObservationIgnored private var sizeLeaseProbesInFlight: Set<String> = []
    /// Our own tmux `#{client_name}`, read once per attach; leases we write
    /// carry it. Nil until the reply lands (a claim before that is skipped —
    /// the next interaction writes it). Settable for tests, whose attach path
    /// skips the `%session-changed` that reads it.
    @ObservationIgnored var ourClientName: String?
    /// The device name written into our leases and shown on the other
    /// device's veil. Injectable for tests.
    @ObservationIgnored var sizeOwnerLabel: String = UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
    /// Seconds a holder may sit idle before a device attaching or coming to
    /// the front may take its window. A drift or a re-check never steals,
    /// however long the holder idles.
    @ObservationIgnored var foregroundStealAfter: TimeInterval = 10
    /// A stamp refresh per keystroke would be a control command per
    /// keystroke; once in this many seconds says "in use" just as well.
    @ObservationIgnored var leaseStampRefreshInterval: TimeInterval = 5
    @ObservationIgnored private var lastLeaseStampWrite: Date = .distantPast
    /// Bounds the foreground probe: if tmux never answers, pin anyway.
    @ObservationIgnored var foregroundProbeTimeout: TimeInterval = 1.5
    @ObservationIgnored private var foregroundProbeFallback: Task<Void, Never>?

    /// Non-nil while the active window is sized for another live device: the
    /// view dims, freezes on its last frame and offers a tap to take over.
    private(set) var sizeFollow: SizeFollow?
    struct SizeFollow: Equatable {
        var windowId: String
        var ownerLabel: String
    }

    /// Why a pin is wanted — decides how much the lease may be overridden.
    enum PinReason {
        /// First pin after discovery.
        case attach
        /// Return from the background.
        case foreground
        /// A layout-change at a width other than ours (backoff applies).
        case drift
        /// The quiet-spell timer, or the follow-up to a probe.
        case recheck
        /// Our own grid changed, or a capture came back oversized.
        case resize
        /// The person did something to this window: always ours.
        case user
    }

    /// The one gate every pin goes through.
    ///
    /// `user` claims and pins. `attach`/`foreground` take the window unless
    /// another live device interacted within `foregroundStealAfter`; they
    /// probe first when the lease is unknown. `drift`/`resize`/`recheck` take
    /// it only when it is unowned, ours, or its holder is gone — a live
    /// holder keeps it however long it idles. An unprobed window pins right
    /// away on drift (today's behaviour, and what a desktop-terminal war
    /// needs) and learns afterwards; a re-check always probes, which is what
    /// notices a holder releasing or vanishing while we follow.
    private func requestPin(_ windowId: String, reason: PinReason) {
        guard rendersOutput else { return }
        if reason == .user {
            claimSizeOwnership(windowId, stamp: Date().timeIntervalSince1970)
            pinNow(windowId, reason: reason)
            return
        }
        if reason == .attach, let pending = pendingAttachLease {
            pendingAttachLease = nil
            if sizeLeases[windowId] == nil { sizeLeases[windowId] = pending }
        }
        let known = sizeLeases[windowId]
        switch reason {
        case .recheck:
            probeSizeLease(windowId) { [weak self] in self?.decidePin(windowId, reason: reason) }
        case .attach, .foreground:
            if known == nil {
                probeSizeLease(windowId) { [weak self] in self?.decidePin(windowId, reason: reason) }
            } else {
                decidePin(windowId, reason: reason)
            }
        case .drift, .resize:
            decidePin(windowId, reason: reason)
            // Verify what we just assumed; a wrong assumption costs one flap,
            // and the follow-up turns it into following. The follow-up never
            // pins on its own: the drift already did (or its backoff declined
            // to, which a probe reply must not bypass — that backoff is what
            // keeps a desktop tug-of-war from flashing). An unowned window we
            // are using gets our passive claim, so a device arriving later
            // knows to defer.
            if !isHeldByAnotherLiveDevice(known) {
                probeSizeLease(windowId) { [weak self] in
                    guard let self else { return }
                    let learned = self.sizeLeases[windowId]
                    if self.isHeldByAnotherLiveDevice(learned) {
                        self.decidePin(windowId, reason: reason)
                    } else if learned == .unset, self.snapshot.activeWindowId == windowId {
                        self.claimSizeOwnership(windowId, stamp: 0)
                    }
                }
            }
        case .user:
            break
        }
    }

    private func isHeldByAnotherLiveDevice(_ state: SizeLeaseState?) -> Bool {
        guard case .held(let lease)? = state else { return false }
        return lease.live && lease.clientName != ourClientName
    }

    private func decidePin(_ windowId: String, reason: PinReason) {
        guard rendersOutput, snapshot.isAttached else { return }
        let known = sizeLeases[windowId]
        if reason == .foreground { endForegroundProbe() }
        if case .held(let lease)? = known, lease.live, lease.clientName != ourClientName {
            let idle = Date().timeIntervalSince1970 - lease.stamp
            let mayTake = (reason == .attach || reason == .foreground) && idle > foregroundStealAfter
            if !mayTake {
                follow(windowId, lease)
                return
            }
            claimSizeOwnership(windowId, stamp: 0)
        } else if case .held(let lease)? = known, lease.clientName == ourClientName {
            // Ours already; nothing to write.
        } else if reason != .drift || known != nil {
            // Unset, or a holder that is no longer attached. A drift on a
            // never-probed window pins without writing — we do not know yet
            // whether anyone holds it (the probe that follows may say so).
            claimSizeOwnership(windowId, stamp: 0)
        }
        if reason == .recheck, !activeWindowAtForeignWidth, sizeFollow == nil {
            // Nothing to fix: the window is already at our width and we were
            // not following. (A re-check exists to end a follow or a war.)
            return
        }
        pinNow(windowId, reason: reason)
    }

    private func pinNow(_ windowId: String, reason: PinReason) {
        if sizeFollow?.windowId == windowId { sizeFollow = nil }
        switch reason {
        case .drift:
            reclaimWindowOnDrift(windowId)
        case .foreground:
            fitWindowToClient(windowId)
            // `releaseWindowPins` handed the split layout back (which
            // unzooms), so the single-pane presentation has to be
            // re-established — it is no longer left standing while we're away.
            ensureImmersiveZoom()
        case .attach, .recheck, .resize, .user:
            fitWindowToClient(windowId)
        }
    }

    /// The window belongs to `lease`'s device: leave its size alone. The veil
    /// (and the output gate behind it) only make sense while the window is
    /// actually at the other size; at ours there is nothing to hide — the
    /// lease alone keeps us from reclaiming when it drifts.
    private func follow(_ windowId: String, _ lease: SizeLease) {
        guard windowId == snapshot.activeWindowId, activeWindowAtForeignWidth else { return }
        let next = SizeFollow(windowId: windowId, ownerLabel: lease.label)
        if sizeFollow != next {
            Log.ssh.debug("size: following \(lease.label, privacy: .public) on \(windowId, privacy: .public)")
            sizeFollow = next
        }
    }

    /// Read a window's lease and who is attached, in one FIFO pair. The
    /// clients reply lands first (send order), so the option callback can
    /// judge liveness. One probe per window at a time.
    private func probeSizeLease(_ windowId: String, then completion: @escaping () -> Void) {
        guard snapshot.isAttached, !sizeLeaseProbesInFlight.contains(windowId) else { return }
        sizeLeaseProbesInFlight.insert(windowId)
        final class Box { var clients: Set<String> = [] }
        let box = Box()
        sendCommand("list-clients -F '#{client_name}'") { response in
            box.clients = Set(response.lines.map { $0.trimmingCharacters(in: .whitespaces) }
                .filter { !$0.isEmpty })
        }
        sendCommand("show-options -wqv -t \(windowId) @moshpit_size_owner") { [weak self] response in
            guard let self else { return }
            self.sizeLeaseProbesInFlight.remove(windowId)
            let raw = response.lines.first?.trimmingCharacters(in: .whitespaces) ?? ""
            if let lease = SizeLease(parsing: raw, clients: box.clients) {
                self.sizeLeases[windowId] = .held(lease)
            } else {
                self.sizeLeases[windowId] = .unset
            }
            completion()
        }
    }

    /// Write our lease on `windowId`. `stamp` 0 is a passive claim (attach,
    /// foreground, a dead holder's window); a real interaction stamps now.
    private func claimSizeOwnership(_ windowId: String, stamp: TimeInterval) {
        guard rendersOutput, let name = ourClientName else { return }
        let lease = SizeLease(clientName: name, label: sizeOwnerLabel, stamp: stamp, live: true)
        sizeLeases[windowId] = .held(lease)
        // As a command-string literal: the client name is whatever the reply
        // to `display-message -p '#{client_name}'` said, and a `"`, `$` or `;`
        // in it ran on into the line (B139).
        send(rawCommand: "set-option -w -t \(windowId) @moshpit_size_owner "
            + Self.tmuxCommandStringLiteral(lease.encoded))
        lastLeaseStampWrite = stamp == 0 ? .distantPast : Date()
        if sizeFollow?.windowId == windowId { sizeFollow = nil }
    }

    /// Unset our lease on `windowId` if it is ours. Called wherever the pin is
    /// handed back (window switch, background); a full disconnect does it
    /// over the courtesy channel via ``sizeLeaseReleaseCommands``.
    private func releaseSizeOwnership(_ windowId: String) {
        if case .held(let lease)? = sizeLeases[windowId], lease.clientName == ourClientName {
            send(rawCommand: "set-option -wu -t \(windowId) @moshpit_size_owner")
            sizeLeases[windowId] = .unset
        }
        if sizeFollow?.windowId == windowId { sizeFollow = nil }
    }

    /// The lease unsets a full teardown owes, for the hub's one-shot courtesy
    /// channel (an in-band write races the detach). Consumes the records.
    func sizeLeaseReleaseCommands() -> [String] {
        var commands: [String] = []
        for windowId in sizeLeases.keys.sorted() {
            if case .held(let lease)? = sizeLeases[windowId], lease.clientName == ourClientName {
                commands.append("set-option -wu -t \(windowId) @moshpit_size_owner")
            }
        }
        sizeLeases.removeAll()
        return commands
    }

    /// The person did something to the active window — a keystroke, a paste,
    /// a wheel tick. Following ends here: the window comes back to our grid.
    /// Otherwise the stamp says "in use" so another device coming to the
    /// front leaves this window alone (throttled).
    private func noteUserInteraction() {
        guard rendersOutput, snapshot.isAttached, let win = snapshot.activeWindowId else { return }
        if sizeFollow?.windowId == win || isHeldByAnotherLiveDevice(sizeLeases[win]) {
            requestPin(win, reason: .user)
            return
        }
        guard Date().timeIntervalSince(lastLeaseStampWrite) > leaseStampRefreshInterval else { return }
        claimSizeOwnership(win, stamp: Date().timeIntervalSince1970)
    }

    /// The follower's tap: take the active window back.
    func takeSizeOwnership() {
        guard let win = snapshot.activeWindowId else { return }
        requestPin(win, reason: .user)
    }

    private func endForegroundProbe() {
        foregroundProbeFallback?.cancel()
        foregroundProbeFallback = nil
        pinsReleased = false
    }

    /// A window's pane layout as it was BEFORE we touched it: the raw
    /// `#{window_layout}` string, the pane the USER had zoomed (nil when the
    /// window wasn't zoomed) and the pane count that string describes.
    fileprivate struct PristineLayout {
        let layout: String
        let zoomedPane: String?
        let paneCount: Int
        /// The layouts our sizing left the window in (round thirty-four):
        /// the one tmux reflowed the record into at the phone's grid, and
        /// those the phone's later sizes reflowed that into. A window in none
        /// of these has been rearranged by someone else since — a desktop —
        /// and the hand-back leaves it alone. Empty: unknown, restored as
        /// before. crates/tmux's `PristineLayout::ours`.
        var ours: [String] = []
    }

    /// How many of a window's own layouts are kept, the latest last.
    private static let oursKept = 8

    /// What a controller still owes the server for the windows it shaped:
    /// the phone-grid pins (``resizedWindows``) and the layouts recorded
    /// beneath them (``pristineLayouts``). A reconnect hands this from the
    /// controller it retires to the replacement — the old transport is dead,
    /// so paying the debt there would only wait out a channel timeout; the
    /// replacement re-pins the same window on attach and the hub settles
    /// whatever is left on the real disconnect. See
    /// `SessionHub.ActiveSession.stop(forReconnect:)`. It also carries the
    /// mosh sidecar's knowledge of which client is the phone's renderer.
    struct PinLedger {
        fileprivate var resizedWindows: Set<String>
        fileprivate var pristineLayouts: [String: PristineLayout]
        /// Not a debt, but the same trip: which client is the phone's mosh
        /// renderer (``rendererClient``). A sidecar rebuilt under a live mosh
        /// connection is never told the renderer's pid again — the hub hands
        /// it over once per mosh-server — so it inherits what it was told.
        fileprivate var renderer: TmuxClientIdentity?
        /// The status line turned off on a session Moshpit created
        /// (``statusHiddenSessionId``), and which sessions those are: a
        /// rebuilt sidecar gives it back when the phone leaves that session,
        /// or the next real disconnect does, instead of nobody ever owing it.
        fileprivate var hiddenStatus: String?
        fileprivate var ownedSessions: Set<String>

        fileprivate init(resizedWindows: Set<String>, pristineLayouts: [String: PristineLayout],
                         renderer: TmuxClientIdentity?, hiddenStatus: String?,
                         ownedSessions: Set<String>) {
            self.resizedWindows = resizedWindows
            self.pristineLayouts = pristineLayouts
            self.renderer = renderer
            self.hiddenStatus = hiddenStatus
            self.ownedSessions = ownedSessions
        }

        /// Bare pins with nothing recorded beneath them — what a test hands
        /// a controller to stand in for a retired one.
        init(resizedWindows: Set<String>) {
            self.init(resizedWindows: resizedWindows, pristineLayouts: [:], renderer: nil,
                      hiddenStatus: nil, ownedSessions: [])
        }

        var isEmpty: Bool {
            resizedWindows.isEmpty && pristineLayouts.isEmpty && renderer == nil
                && hiddenStatus == nil && ownedSessions.isEmpty
        }
        /// The windows still pinned to a phone grid — what the tests check
        /// made it across a handoff.
        var pinnedWindows: Set<String> { resizedWindows }
    }

    /// Hand over everything this controller still owes the server and forget
    /// it — the caller is retiring this controller without a teardown round
    /// trip and will give the ledger to the replacement.
    func takePinLedger() -> PinLedger {
        defer {
            resizedWindows.removeAll()
            pristineLayouts.removeAll()
            rendererClient = nil
            rendererSessionId = nil
            statusHiddenSessionId = nil
        }
        return PinLedger(resizedWindows: resizedWindows, pristineLayouts: pristineLayouts,
                         renderer: rendererClient, hiddenStatus: statusHiddenSessionId,
                         ownedSessions: ownedSessionIds)
    }

    /// Take on a retired controller's debt. Pins merge; a layout this
    /// controller has already recorded for a window wins over the inherited
    /// one (it is newer — the pristine record must predate the FIRST pin, and
    /// the inherited record is exactly that).
    func inheritPinLedger(_ ledger: PinLedger) {
        resizedWindows.formUnion(ledger.resizedWindows)
        for (windowId, layout) in ledger.pristineLayouts where pristineLayouts[windowId] == nil {
            pristineLayouts[windowId] = layout
        }
        // Same rule as the layouts: what this controller learned itself is newer.
        if rendererClient == nil { rendererClient = ledger.renderer }
        // A status line still off is owed by whoever runs next: the first
        // syncStatusLine here gives it back unless the phone is still on
        // that session, and the disconnect restore covers it either way.
        ownedSessionIds.formUnion(ledger.ownedSessions)
        if statusHiddenSessionId == nil { statusHiddenSessionId = ledger.hiddenStatus }
    }

    /// The agent stamps as last read, for the replacement controller — nil
    /// if this one never got to read them. The server's `@moshpit_*` options
    /// did not change because our client dropped, so the replacement can
    /// show them from its first frame instead of a bare command for the
    /// second or two until its own poll answers.
    func takeAgentHooks() -> [String: AgentHook]? {
        agentHooksLoaded ? agentHooks : nil
    }

    /// Start from a retired controller's stamps; the first poll of this
    /// attach replaces them wholesale.
    func inheritAgentHooks(_ hooks: [String: AgentHook]) {
        guard !agentHooksLoaded else { return }
        agentHooks = hooks
        agentHooksLoaded = true
    }

    /// Whether a pane of THIS connection has been revealed on screen — a frame
    /// from this attach is what the person is looking at. The hub keeps the
    /// retired connection's last frame up until this flips, and drops the
    /// cover the moment it does; nothing else in the reconnect is measured on
    /// paint rather than on a guessed grace period.
    @ObservationIgnored
    private(set) var hasPaintedSinceAttach = false

    /// When this connection's tmux attach landed (`%session-changed` or the
    /// discovery that follows it); nil until then. The hub's first-frame
    /// fallback measures its grace from HERE, not from the boot line: over a
    /// slow link the attach alone can outlast a deadline that was meant for
    /// "attached, but nothing to paint".
    @ObservationIgnored
    private(set) var attachedAt: Date?

    /// Fires once, on the first revealed pane after attach.
    @ObservationIgnored
    var onFirstFramePainted: (() -> Void)?

    private func notePaneRevealed() {
        guard !hasPaintedSinceAttach else { return }
        hasPaintedSinceAttach = true
        onFirstFramePainted?()
    }

    /// Pre-Moshpit layouts for the windows we pinned or zoomed, put back
    /// verbatim when we hand a window over (leave / background / disconnect).
    ///
    /// Why this is needed: the phone-sized pin is not free. tmux's
    /// `resize_window()` unzooms, runs `layout_resize()` over the WHOLE layout
    /// tree, then re-zooms — so pinning a split window to the phone grid
    /// squashes the user's real layout, and once panes hit their minimum size
    /// the proportions are gone for good (measured on tmux 3.6: an 8-pane
    /// 203x60 layout round-tripped through 39x20 comes back with completely
    /// different splits). The zoom itself is lossless; the PIN is what
    /// destroys the layout, which is why the record must predate it.
    ///
    /// Re-captured on every re-pin (a restore consumes the record), so a layout
    /// the user rearranged from the desktop while we were backgrounded becomes
    /// the new pristine state instead of being clobbered by a stale one.
    @ObservationIgnored
    private var pristineLayouts: [String: PristineLayout] = [:]

    /// Windows whose capture round-trip is still in flight — stops a pin
    /// followed immediately by a zoom from firing two queries (the second would
    /// read the already-squashed layout back).
    @ObservationIgnored
    private var pristineCapturesInFlight: Set<String> = []

    /// Remember a window's layout before we pin or zoom it. MUST be called
    /// BEFORE the command that changes it — tmux processes control-channel
    /// commands in FIFO order, so a query queued ahead of the pin reports the
    /// pre-pin state even though its reply lands later.
    ///
    /// `#{window_layout}` is the UNZOOMED layout even while a pane is zoomed
    /// (verified on tmux 3.6), so a zoom we already applied can't corrupt the
    /// record — a pin would, hence the `resizedWindows` guard: a layout
    /// captured at phone size must never be replayed at a desktop client.
    private func capturePristineLayout(_ windowId: String) {
        guard snapshot.isAttached,
              pristineLayouts[windowId] == nil,
              !pristineCapturesInFlight.contains(windowId),
              !resizedWindows.contains(windowId) else { return }
        pristineCapturesInFlight.insert(windowId)
        sendCommand("display-message -p -t '\(windowId)' '\(Self.pristineProbeFormat)'") { [weak self] response in
            guard let self else { return }
            self.pristineCapturesInFlight.remove(windowId)
            guard !response.isError, let line = response.lines.first else { return }
            self.notePristineLayout(windowId, fields: line.tmuxFields(separator: " "))
        }
    }

    /// `#{window_panes} #{window_zoomed_flag} #{pane_id} #{window_layout}` —
    /// shared by ``capturePristineLayout`` and ``ensureImmersiveZoom`` so
    /// landing on a window costs ONE probe, not two (the zoom needs the first
    /// two fields, the record needs all four).
    static let pristineProbeFormat =
        "#{window_panes} #{window_zoomed_flag} #{pane_id} #{window_layout}"

    /// File a probe reply as a window's pristine record, if it qualifies.
    ///
    /// Callers own the "is this reply pre-pin?" question: ``capturePristineLayout``
    /// queues its probe ahead of the pin, so its reply always is; a probe queued
    /// after one (``ensureImmersiveZoom``'s) describes the already-squashed
    /// layout and must be filed only when no pin is in effect.
    private func notePristineLayout(_ windowId: String, fields: [String]) {
        guard pristineLayouts[windowId] == nil else { return }
        // Single-pane windows have nothing to restore, and an unrecognisable
        // layout must never be spliced into `select-layout`.
        guard fields.count >= 4, let panes = Int(fields[0]), panes > 1,
              Self.isPlausibleLayout(fields[3]) else { return }
        let zoomedPane = (fields[1] == "1" && Self.isValidTmuxId(fields[2])) ? fields[2] : nil
        pristineLayouts[windowId] = PristineLayout(layout: fields[3],
                                                   zoomedPane: zoomedPane,
                                                   paneCount: panes)
    }

    /// `layout` is one our sizing left `windowId` in (round thirty-four).
    private func noteOurs(_ windowId: String, _ layout: String) {
        guard var record = pristineLayouts[windowId], layout != record.layout,
              !record.ours.contains(layout), Self.isPlausibleLayout(layout) else { return }
        record.ours.append(layout)
        if record.ours.count > Self.oursKept { record.ours.removeFirst() }
        pristineLayouts[windowId] = record
    }

    /// A tmux layout string: four hex checksum digits, a comma, then nothing but
    /// digits and layout punctuation. Cheap sanity gate so a surprise reply (an
    /// error line, a truncated field) can't reach `select-layout`. nonisolated
    /// so it's unit-testable without a live tmux server.
    ///
    /// tmux prints layouts in ASCII, so the digits are ASCII digits: the
    /// Character checks alone took any numeral ("２００", "٢٠٠") and a digit
    /// carrying a combining mark for one.
    nonisolated static func isPlausibleLayout(_ layout: String) -> Bool {
        guard layout.count > 6, layout.count < 4096 else { return false }
        let parts = layout.tmuxFields(separator: ",", maxSplits: 1)
        guard parts.count == 2, parts[0].count == 4,
              parts[0].allSatisfy({ $0.isASCII && $0.isHexDigit }) else { return false }
        return parts[1].allSatisfy { $0.isASCII && ($0.isNumber || "x,[]{}".contains($0)) }
    }

    /// True when the captured record still describes the window we'd replay it
    /// at. Panes coming or going since the capture (a split from the phone, a
    /// shell exiting) makes it describe a window that no longer exists —
    /// replaying it would rearrange panes the user never asked us to touch.
    private func pristineStillApplies(_ windowId: String, _ pristine: PristineLayout) -> Bool {
        guard let window = snapshot.windows[windowId] else { return true }
        return window.paneCount == pristine.paneCount
    }

    /// Put a window's captured layout (and the user's own zoom) back, consuming
    /// the record.
    ///
    /// Order is the CALLER's job: the phone-sized pin must already be released,
    /// or tmux refits the layout we just replayed straight back down to the
    /// phone grid (measured: restoring while still pinned is a no-op).
    /// `select-layout` with an explicit string resizes the window to the
    /// layout's own size, so the pre-Moshpit state comes back byte-identical
    /// even with no other client attached — and it unzooms, which is why the
    /// zoom is re-applied afterwards only when the user had zoomed it
    /// themselves (`resize-pane -Z` on a pane target also re-selects it, which
    /// is exactly the state they left).
    @discardableResult
    ///
    /// The mosh sidecar keeps the record (B270): the phone's renderer is
    /// still attached and still sizes the session. A replay at another size
    /// resizes the window to the layout's for a moment — so the check after
    /// it read the record back and dropped it — and tmux's next sizing put
    /// the window back at the phone's grid, squashed, with nothing left to
    /// restore on the disconnect.
    private func restorePristineLayout(_ windowId: String) -> PristineLayout? {
        replayPristineLayout(windowId, kept: !rendersOutput)
    }

    /// ``restorePristineLayout(_:)``, with whether the record is `kept`
    /// spelled out: not once the renderer has left the window's session.
    @discardableResult
    private func replayPristineLayout(_ windowId: String, kept: Bool) -> PristineLayout? {
        guard let pristine = pristineLayouts.removeValue(forKey: windowId) else { return nil }
        guard pristineStillApplies(windowId, pristine) else { return nil }
        send(rawCommand: "select-layout -t '\(windowId)' '\(pristine.layout)'")
        if let pane = pristine.zoomedPane {
            send(rawCommand: "resize-pane -Z -t '\(pane)'")
        }
        zoomEpoch &+= 1
        if kept {
            pristineLayouts[windowId] = pristine
            return nil
        }
        return pristine
    }

    /// Replay a window's record, then read the window back: a replay that
    /// didn't take keeps the record.
    private func checkReplay(_ windowId: String, kept: Bool) {
        guard let pristine = replayPristineLayout(windowId, kept: kept) else { return }
        sendCommand("display-message -p -t '\(windowId)' '#{window_layout}'") { [weak self] response in
            guard let self, !response.isError, self.pristineLayouts[windowId] == nil,
                  let line = response.lines.first else { return }
            let read = line.trimmingCharacters(in: .whitespaces)
            if read != pristine.layout {
                self.pristineLayouts[windowId] = pristine
                // Refitted to the phone's grid: a layout of ours.
                self.noteOurs(windowId, read)
            }
        }
    }

    /// The mosh sidecar's renderer has left `sessionId` for another one: no
    /// window of it is the phone's any more. The window it pinned (`window`)
    /// goes back to automatic sizing, and every window of the session with a
    /// record gets its layout back — the record consumed this time, the
    /// check after it, as a `-CC` client's leave does: nothing of the phone
    /// sizes that session now (with the phone alone tmux's only client, its
    /// windows are nobody's, and a replay resizes them to their record for
    /// good). crates/tmux's `hand_back_left_session`.
    private func handBackLeftSession(_ sessionId: String, window: String?) {
        if let window, resizedWindows.remove(window) != nil {
            send(rawCommand: "set-option -u -w -t \(window) window-size")
        }
        let recorded = pristineLayouts.keys
            .filter { snapshot.windows[$0]?.sessionId == sessionId }
            .sorted(by: Self.idOrder)
        for windowId in recorded {
            checkReplay(windowId, kept: false)
        }
    }

    /// A window's size, from its `#{window_layout}` (`<sum>,<W>x<H>,…`).
    struct GridSize: Equatable {
        let cols: Int
        let rows: Int
    }

    /// The size in a `#{window_layout}`; nil unless both are positive.
    nonisolated static func windowSize(_ layout: String) -> GridSize? {
        let parts = layout.tmuxFields(separator: ",")
        guard parts.count >= 2 else { return nil }
        let size = parts[1].tmuxFields(separator: "x")
        guard size.count == 2, let cols = Int(size[0]), let rows = Int(size[1]),
              cols > 0, rows > 0 else { return nil }
        return GridSize(cols: cols, rows: rows)
    }

    /// `@2` before `@10`: ids by their number.
    nonisolated static func idOrder(_ a: String, _ b: String) -> Bool {
        if a.first == b.first, let x = Int(a.dropFirst()), let y = Int(b.dropFirst()) { return x < y }
        return a < b
    }

    /// The size to report as ours when handing the windows back (B150): the
    /// recorded size of the active window, else the largest recorded one;
    /// nil without a record that still applies.
    private func handBackSize(_ layouts: [String: PristineLayout]) -> GridSize? {
        func applying(_ windowId: String) -> GridSize? {
            guard let pristine = layouts[windowId], pristineStillApplies(windowId, pristine) else { return nil }
            return Self.windowSize(pristine.layout)
        }
        if let active = snapshot.activeWindowId, let size = applying(active) { return size }
        var best: GridSize?
        for windowId in layouts.keys.sorted(by: Self.idOrder) {
            guard let size = applying(windowId) else { continue }
            if best.map({ size.cols * size.rows > $0.cols * $0.rows }) ?? true { best = size }
        }
        return best
    }

    /// A client name that can go single-quoted into a teardown command
    /// through a login shell: a tty path or tmux's `client-<pid>`.
    nonisolated static func isPlainClientName(_ name: String) -> Bool {
        !name.isEmpty && name.utf8.allSatisfy {
            ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x41 && $0 <= 0x5A) || ($0 >= 0x61 && $0 <= 0x7A)
                || $0 == UInt8(ascii: "/") || $0 == UInt8(ascii: ".") || $0 == UInt8(ascii: "_")
                || $0 == UInt8(ascii: "-")
        }
    }

    // MARK: The layouts our size request squashes (B150)

    /// The window id, then ``pristineProbeFormat``'s fields: a session's
    /// layouts around our size request.
    static let sizeProbeFormat =
        "#{window_id} #{window_panes} #{window_zoomed_flag} #{pane_id} #{window_layout}"

    /// The session's layouts just before our size request, by window (the
    /// probe's fields after the window id); nil until that reply.
    @ObservationIgnored private var presizeLayouts: [String: [String]]?
    /// A before-probe went out and its after-probe has not.
    @ObservationIgnored private var presizeAwaitsAfter = false

    /// Our own size request squashes too: tmux sizes a session's windows by
    /// its latest client, and a phone that just reported its size is that
    /// client — the only one, or the latest of several — so every window of
    /// the session is at the phone's grid before discovery, and the probe
    /// ahead of the pin read the squashed layout. The session's layouts are
    /// read just before the request, and again just after
    /// (``probeLayoutsAfterSizing``).
    private func probeLayoutsBeforeSizing(_ sessionId: String) {
        guard rendersOutput, Self.isValidTmuxId(sessionId) else { return }
        presizeLayouts = nil
        presizeAwaitsAfter = true
        sendCommand("list-windows -t '\(sessionId)' -F '\(Self.sizeProbeFormat)'") { [weak self] response in
            self?.sizeProbeAnswered(before: true, response)
        }
    }

    /// The layouts of `sessionId`'s windows around a switch to it — ours,
    /// or the mosh sidecar's renderer's: the client that lands on a session
    /// becomes its latest, and tmux sizes its windows by it. Sent just
    /// before the switch; the second read goes out when the first one's
    /// reply is in (B150, round thirty-four): tmux sizes a session for a
    /// switch at the end of its loop, after the commands that came with the
    /// switch have run, so a read sent right behind it still found every
    /// window as it was, and nothing was recorded. A command that reaches
    /// tmux after a reply it sent runs in a later loop. crates/tmux's
    /// `probe_switch_layouts`.
    private func probeSwitchLayouts(_ sessionId: String) {
        guard Self.isValidTmuxId(sessionId) else { return }
        sendCommand("list-windows -t '\(sessionId)' -F '\(Self.sizeProbeFormat)'") { [weak self] response in
            guard let self, !response.isError else { return }
            let earlier = Self.sizeRows(response.lines)
            self.sendCommand("list-windows -t '\(sessionId)' -F '\(Self.sizeProbeFormat)'") { [weak self] response in
                guard let self, !response.isError else { return }
                let rows = Self.sizeRows(response.lines)
                for windowId in rows.keys.sorted() {
                    guard let then = earlier[windowId], let now = rows[windowId] else { continue }
                    if Self.windowSize(then[3]) != Self.windowSize(now[3]) {
                        self.notePristineLayout(windowId, fields: then)
                        self.noteOurs(windowId, now[3])
                    }
                }
            }
        }
    }

    /// The second read, when a first one went out.
    private func probeLayoutsAfterSizing() {
        guard presizeAwaitsAfter else { return }
        presizeAwaitsAfter = false
        guard let sessionId = snapshot.activeSessionId else { return }
        sendCommand("list-windows -t '\(sessionId)' -F '\(Self.sizeProbeFormat)'") { [weak self] response in
            self?.sizeProbeAnswered(before: false, response)
        }
    }

    /// The first read is kept; on the second, every window the size request
    /// resized files its earlier layout as its record (if it qualifies, and
    /// has none yet). A window it left alone isn't ours to restore.
    private func sizeProbeAnswered(before: Bool, _ response: TmuxCommandResponse) {
        let rows = response.isError ? [:] : Self.sizeRows(response.lines)
        if before {
            presizeLayouts = rows
            return
        }
        guard let earlier = presizeLayouts else { return }
        presizeLayouts = nil
        for windowId in rows.keys.sorted() {
            guard let then = earlier[windowId], let now = rows[windowId] else { continue }
            if Self.windowSize(then[3]) != Self.windowSize(now[3]) {
                notePristineLayout(windowId, fields: then)
                noteOurs(windowId, now[3])
            }
        }
    }

    /// A size probe's rows (window id, then ``pristineProbeFormat``'s
    /// fields), by window.
    private static func sizeRows<S: StringProtocol>(_ lines: [S]) -> [String: [String]] {
        var rows: [String: [String]] = [:]
        for line in lines {
            let fields = String(line).tmuxFields(separator: " ")
            if fields.count >= 5, fields[0].hasPrefix("@"), isValidTmuxId(fields[0]) {
                rows[fields[0]] = Array(fields[1...])
            }
        }
        return rows
    }

    // MARK: The renderer's squash (B270)

    /// The mosh sidecar: every window's layout before mosh-server started the
    /// phone's renderer, by window; compared with a listing at this sidecar's
    /// first attach.
    @ObservationIgnored private var layoutsBeforeRenderer: [String: [String]]?

    /// The mosh sidecar is told how every window was before mosh-server
    /// started the phone's renderer: `listing`, the host's `list-windows -a`
    /// in ``sizeProbeFormat``, read over the bootstrap's connection just
    /// before. The renderer is tmux's client, sized to the phone and sizing
    /// the session from the moment it attaches — with the phone the only
    /// client, every window of the session is at the phone's grid before
    /// this sidecar exists, and the probe ahead of its pin read the squashed
    /// layout. At the sidecar's first attach the windows are listed again,
    /// and every window the renderer resized files its earlier layout as its
    /// record, as our own size request's do (B150). crates/tmux's
    /// `note_layouts_before_renderer`.
    func noteLayoutsBeforeRenderer(_ listing: String) {
        guard !rendersOutput else { return }
        layoutsBeforeRenderer = Self.sizeRows(listing.split(separator: "\n"))
    }

    /// The listing after the renderer, at the sidecar's attach, when one was
    /// taken before it.
    private func probeLayoutsAfterRenderer() {
        guard let rows = layoutsBeforeRenderer else { return }
        layoutsBeforeRenderer = nil
        presizeLayouts = rows
        sendCommand("list-windows -a -F '\(Self.sizeProbeFormat)'") { [weak self] response in
            self?.sizeProbeAnswered(before: false, response)
        }
    }

    /// The command that takes the phone's mosh renderer out of the session
    /// before the windows are handed back (B270): `detach-client`, only while
    /// tmux still lists that client by its pid AND name. With the phone the
    /// only client, the renderer — tmux's client in the mosh session — sized
    /// the session to the end: the hand-back's last step, back to automatic
    /// sizing, sized every window straight back to the phone's grid, and the
    /// renderer, stopped right after with its server, left them there.
    /// Detached first, no client is left to size them, and they stay at
    /// their record. nil but for the sidecar that knows its renderer. Only on
    /// the session's real end: detached, the renderer ends the mosh session.
    func rendererDetachCommand(tmux: String) -> String? {
        guard !rendersOutput, let renderer = rendererClient else { return nil }
        return Self.rendererDetachCommand(tmux: tmux, renderer: renderer)
    }

    /// crates/tmux's `boot::renderer_detach_command`, the same string.
    nonisolated static func rendererDetachCommand(tmux: String, renderer: TmuxClientIdentity) -> String {
        let row = Shell.quote("\(renderer.pid) \(renderer.name)")
        return "\(tmux) list-clients -F '#{client_pid} #{client_name}' 2>/dev/null | grep -qxF \(row) "
            + "&& \(tmux) detach-client -t \(Shell.quote(renderer.name)); true"
    }

    /// The same restore as ``restorePristineLayout``, handed out as commands for
    /// teardown paths that must run OUTSIDE the -CC channel: an in-band write
    /// races the detach and gets lost (the reason pin release moved to a
    /// one-shot exec channel too). Consumes the records. Targets are `@id`/
    /// `%id`, which survive the login shell that channel goes through (a `$id`
    /// would not — see ``restoreSuppressedStatusAndFlush``), and the layout
    /// string is single-quoted so its `{}` reach tmux intact.
    ///
    /// B150: with our client still attached they first take it out of the
    /// sizing math and report the recorded size as its own (see
    /// ``releaseWindowPins``); a ledger's records, whose client is gone, size
    /// each window to its record first and unset that again — with no client
    /// attached tmux keeps a window's size, and a layout replayed at another
    /// size is refitted.
    func pristineLayoutRestoreCommands() -> [String] {
        let layouts = pristineLayouts
        pristineLayouts.removeAll()
        var commands: [String] = []
        let client = rendersOutput ? ourClientName.flatMap { Self.isPlainClientName($0) ? $0 : nil } : nil
        if let name = client, let size = handBackSize(layouts) {
            commands.append("refresh-client -t '\(name)' -f ignore-size")
            commands.append("refresh-client -t '\(name)' -C \(size.cols)x\(size.rows)")
        }
        for windowId in layouts.keys.sorted() {
            guard let pristine = layouts[windowId],
                  pristineStillApplies(windowId, pristine) else { continue }
            commands += Self.windowRestoreCommands(windowId, pristine, sized: client == nil)
        }
        return commands
    }

    /// One window's record as teardown commands (no tmux prefix): sized to
    /// the record first when no client of ours is left to size it (`sized`),
    /// the layout, back to automatic sizing, the user's own zoom.
    private static func windowRestoreCommands(_ windowId: String, _ pristine: PristineLayout,
                                              sized: Bool) -> [String] {
        var commands: [String] = []
        let size = sized ? windowSize(pristine.layout) : nil
        if let size {
            commands.append("resize-window -t \(windowId) -x \(size.cols) -y \(size.rows)")
        }
        commands.append("select-layout -t \(windowId) '\(pristine.layout)'")
        if size != nil {
            commands.append("set-option -u -w -t \(windowId) window-size")
        }
        if let pane = pristine.zoomedPane {
            commands.append("resize-pane -Z -t \(pane)")
        }
        return commands
    }

    /// The whole hand-back a full teardown owes, as ONE command for a
    /// channel outside `-CC` (round thirty-four): `first` (the phone's mosh
    /// renderer out, B270), then the pins unset, then the layouts — each
    /// only if the window is still in a layout of ours. Consumes the pins
    /// and the records; nil when nothing is owed.
    ///
    /// With a desktop attached, the disconnect used to replay a record over
    /// whatever the window had become: a desktop that rearranged the panes
    /// since (the same count) had its arrangement replaced by the one from
    /// before the phone came. Every window's layout is read first, before
    /// anything here changes a size; a record is replayed only when that
    /// layout is its own or one our sizing left the window in
    /// (``PristineLayout/ours``), and a record that knows none is replayed as
    /// before. Under `sh -c`, for any login shell. crates/tmux's
    /// `pin_hand_back_script`, the same string.
    func pinHandBackScript(tmux: String, first: [String]) -> String? {
        let pinned = resizedWindows.filter(Self.isValidTmuxId).sorted(by: Self.idOrder)
        resizedWindows.removeAll()
        let layouts = pristineLayouts
        pristineLayouts.removeAll()
        let client = rendersOutput ? ourClientName.flatMap { Self.isPlainClientName($0) ? $0 : nil } : nil
        var reads: [String] = []
        var steps = first
        steps += pinned.map { "\(tmux) set-option -u -w -t \($0) window-size" }
        if let name = client, let size = handBackSize(layouts) {
            steps.append("\(tmux) refresh-client -t '\(name)' -f ignore-size")
            steps.append("\(tmux) refresh-client -t '\(name)' -C \(size.cols)x\(size.rows)")
        }
        var restores = 0
        for (index, windowId) in layouts.keys.sorted().enumerated() {
            guard let pristine = layouts[windowId], Self.isValidTmuxId(windowId),
                  pristineStillApplies(windowId, pristine) else { continue }
            restores += 1
            let commands = Self.windowRestoreCommands(windowId, pristine, sized: client == nil)
                .map { "\(tmux) \($0)" }
            if pristine.ours.isEmpty {
                steps += commands
                continue
            }
            let read = "l\(index)"
            reads.append("\(read)=$(\(tmux) display-message -p -t \(windowId) '#{window_layout}' 2>/dev/null)")
            let patterns = ([pristine.layout] + pristine.ours).map { "\"\($0)\"" }
            steps.append("case \"$\(read)\" in \(patterns.joined(separator: "|"))) \(commands.joined(separator: "; "));; esac")
        }
        if pinned.isEmpty, restores == 0, first.isEmpty { return nil }
        return "sh -c " + Shell.quote((reads + steps + ["true"]).joined(separator: "; "))
    }

    /// Which window `recentReclaimTimestamps` is currently tracking — reset
    /// (along with the timestamps and any suspension) the moment a DIFFERENT
    /// window drifts, so one window's tug-of-war can't suppress reclaiming a
    /// window that's actually drifting for the first time.
    @ObservationIgnored
    private var reclaimTrackedWindow: String?
    /// Recent AUTOMATIC reclaim-on-drift timestamps for `reclaimTrackedWindow`
    /// (`handleLayoutChange` only — explicit reclaims from `selectWindow`/
    /// `commitClientSize` call `fitWindowToClient` directly and never touch
    /// this). If another attached client is concurrently active on the SAME
    /// window (commonly `window-size latest` following whichever client last
    /// interacted), our reclaim and its own auto-follow can fight forever:
    /// every round is a real `resize-window`, visible on the phone as a
    /// continuous flash/resize ("打开这个 panel 会闪屏幕，一直在 resize"). Three
    /// reclaims inside `reclaimBackoffWindow` means it's a live fight, not a
    /// one-off drift — back off entirely for that same span instead of
    /// escalating forever.
    @ObservationIgnored
    private var recentReclaimTimestamps: [Date] = []
    private let reclaimBackoffThreshold = 3
    private let reclaimBackoffWindow: TimeInterval = 5
    @ObservationIgnored
    private var reclaimSuspendedUntil: Date?

    /// Force a window to our client size. `window-size latest` plus a wide
    /// desktop client can leave a window at e.g. 298 cols, so a full-screen
    /// TUI (Claude Code, vim) draws a 298-wide layout that wraps into a mess
    /// on a 47-col phone. Resizing the window makes the program redraw at our
    /// width. No-op in control-plane mode (mosh sidecar renders nothing).
    private func fitWindowToClient(_ windowId: String) {
        guard rendersOutput else { return }
        capturePristineLayout(windowId)
        send(rawCommand: "resize-window -t \(windowId) -x \(lastClientSize.cols) -y \(lastClientSize.rows)")
        resizedWindows.insert(windowId)
    }

    /// Take the client with process id `pid` for the phone's renderer
    /// (``rendererClient``) and follow it to its session — the renderer that
    /// attached by itself: mosh-server's own program, tmux's client on the
    /// remembered session, the most recent, or the first there is (B212,
    /// B267). Its pid names it: the program records its shell's before it
    /// execs tmux's client, and the `exec` keeps it — so a desktop attaching
    /// at the same moment is never taken for it (B84). False while no
    /// non-control client has that pid (not attached yet), and when tmux has
    /// no answer.
    func adoptRenderer(pid: Int) async -> Bool {
        guard !rendersOutput, snapshot.isAttached else { return false }
        let switches = sessionSwitches
        return await withCheckedContinuation { continuation in
            sendCommand("list-clients -F '\(TmuxClientRow.format)'", waits: true) { [weak self] response in
                guard let self, !response.isError else {
                    continuation.resume(returning: false)
                    return
                }
                let clients = response.lines.compactMap(TmuxClientRow.init).filter { !$0.isControl }
                guard let renderer = clients.first(where: { $0.pid == pid })?.identity else {
                    continuation.resume(returning: false)
                    return
                }
                self.rendererClient = renderer
                Log.island.info("land: sidecar renderer is \(renderer.name, privacy: .public) pid=\(renderer.pid, privacy: .public) (attached by itself)")
                self.noteRendererRows(clients, switches: switches)
                continuation.resume(returning: true)
            }
        }
    }

    /// The phone's own mosh renderer: the one tmux client a sidecar session
    /// switch moves (see ``selectSession(_:)``). nil while unknown — and
    /// then the switch moves no client at all.
    ///
    /// Learned from the renderer's own pid (``adoptRenderer(pid:)``: the
    /// renderer is mosh-server's program and records it), never guessed from
    /// where clients sit or from which client appeared between two counts of
    /// clients (B84: a desktop attaching beside a renderer whose attach
    /// failed was taken for it). A sidecar rebuilt under a live mosh
    /// connection inherits it with the ``PinLedger``: the renderer did not
    /// change, only our SSH did.
    @ObservationIgnored
    private(set) var rendererClient: TmuxClientIdentity?

    /// The session the phone's renderer (``rendererClient``) sits on — what
    /// the person sees, so what the mosh sidecar follows: its active session,
    /// and the only session whose window it sizes to the phone. nil while the
    /// renderer is unknown or not attached.
    ///
    /// Learned from rows that name the renderer by pid and name — the listing
    /// that identifies it, a re-listing (``locateRenderer()``), the switch
    /// that moves it — never from `#{session_attached}`. That flag counts
    /// every client, the desktop's and this sidecar's own included, and the
    /// sidecar used to take the last session in name order with it set: with
    /// the phone on `aaa` and the desktop on `zzz`, every re-list put the
    /// sidecar on `zzz`, the breadcrumb named the desktop's session, and the
    /// next rotation or keyboard pinned the DESKTOP's window to the phone
    /// grid.
    @ObservationIgnored
    private(set) var rendererSessionId: String?

    /// Bumped when ``selectSession(_:)`` starts a switch and again when it
    /// sends the moves. A client listing sent in between, or before, shows
    /// the renderer where the switch is taking it away from, and must not
    /// drag the sidecar back there.
    @ObservationIgnored
    private var sessionSwitches = 0

    /// A ``locateRenderer()`` listing is on its way. tmux answers in order,
    /// with notifications in the same stream, so the reply to one already in
    /// flight is at least as new as anything that asks for another.
    @ObservationIgnored
    private var rendererLocateInFlight = false

    /// Ask tmux where the phone's renderer is now, and follow it there. The
    /// renderer moves on its own too: `prefix s` in the mosh TUI, a tmux
    /// session ending under it. Then syncs the status line, which a
    /// ``handleSessionChanged`` with a known renderer leaves to this reply
    /// (the session it should be decided for is the renderer's). Nothing
    /// while the renderer is unknown — there is nothing to follow.
    private func locateRenderer() {
        guard !rendersOutput, snapshot.isAttached, rendererClient != nil else {
            syncStatusLine()
            return
        }
        guard !rendererLocateInFlight else { return }
        rendererLocateInFlight = true
        let switches = sessionSwitches
        sendCommand("list-clients -F '\(TmuxClientRow.format)'") { [weak self] response in
            guard let self else { return }
            self.rendererLocateInFlight = false
            if !response.isError {
                self.noteRendererRows(response.lines.compactMap(TmuxClientRow.init), switches: switches)
            }
            self.syncStatusLine()
        }
    }

    /// Where `rows` — one client listing, sent when ``sessionSwitches`` was
    /// `switches` — put the renderer: follow it to its session, or forget
    /// the session when it is not among them (detached, its mosh gone).
    private func noteRendererRows(_ rows: [TmuxClientRow], switches: Int) {
        guard !rendersOutput, let renderer = rendererClient else { return }
        guard let row = rows.first(where: { !$0.isControl && $0.identity == renderer }) else {
            if rendererSessionId != nil {
                Log.island.info("land: sidecar renderer \(renderer.name, privacy: .public) pid=\(renderer.pid, privacy: .public) is not attached")
            }
            rendererSessionId = nil
            return
        }
        // Listed before a switch we started: that switch is moving it, and
        // its own listing says where to.
        guard switches == sessionSwitches else { return }
        followRenderer(to: row.sessionId)
    }

    /// The sidecar takes the renderer's session as its own: the snapshot
    /// shows it (window and pane as discovery last knew them, corrected by
    /// the re-listing below), and this control client moves there too —
    /// tmux tells a control client about layout changes only for windows of
    /// its own session, and a bare `new-window` lands on its own session.
    private func followRenderer(to sessionId: String) {
        guard !rendersOutput, Self.isValidTmuxId(sessionId) else { return }
        rendererSessionId = sessionId
        guard snapshot.activeSessionId != sessionId else { return }
        Log.island.info("land: sidecar follows the renderer to \(sessionId, privacy: .public) (was \(self.snapshot.activeSessionId ?? "-", privacy: .public))")
        snapshot.activeSessionId = sessionId
        let window = snapshot.windows.values.first { $0.sessionId == sessionId && $0.isActive }
        snapshot.activeWindowId = window?.id
        snapshot.activePaneId = window.flatMap { window in
            snapshot.panes.values.first { $0.windowId == window.id && $0.isActive }?.id
        }
        syncStatusLine()
        send(rawCommand: "switch-client -t \(sessionId)")
        refreshWindowsAndPanes()
    }

    /// The window the mosh sidecar may size to the phone: the active one,
    /// when it is on the session the phone's renderer sits on. nil when the
    /// renderer is unknown, not attached, or elsewhere — the window it would
    /// size is then somebody else's.
    private var rendererWindowId: String? {
        guard let session = rendererSessionId, snapshot.activeSessionId == session,
              let window = snapshot.activeWindowId,
              snapshot.windows[window]?.sessionId == session else { return nil }
        return window
    }

    /// Why ``rendererWindowId`` is nil, for the island log.
    private var rendererWindowAbsence: String {
        guard let renderer = rendererClient else { return "the renderer is not known" }
        guard let session = rendererSessionId else {
            return "renderer \(renderer.name) pid=\(renderer.pid) is not attached"
        }
        return "the active window is not on the renderer's session \(session)"
    }

    /// Mosh sidecar (`rendersOutput == false`) window sizing, driven by the hub.
    ///
    /// The mosh-rendered tmux TUI is raw mosh — it can't issue control commands,
    /// and `fitWindowToClient` no-ops for a sidecar — so nothing would pin the
    /// window to the phone grid: pin the active window to it (full-screen TUIs
    /// redraw at phone width). Pinned windows are handed back on disconnect in
    /// `SessionHub.stop()`.
    ///
    /// This client reports no size of its own (B270). It used to report the
    /// mosh grid first, against its 80×24 PTY dragging tmux's sizing down —
    /// but a control client that never reported a size is no part of tmux's
    /// sizing (3.6a: a `-CC` client on an 80×24 PTY leaves a 200×60 window
    /// alone), and one that did is: with the phone tmux's only client, the
    /// windows handed back on the disconnect went straight back to the
    /// phone's grid with it.
    ///
    /// Only the window the phone's renderer is looking at (``rendererWindowId``):
    /// with the renderer unknown, detached or elsewhere, nothing is sized —
    /// not even this client, whose size counts towards its session's windows.
    func syncMoshWindow(cols: Int, rows: Int) {
        guard !rendersOutput, snapshot.isAttached else { return }
        lastClientSize = (cols, rows)
        guard let win = rendererWindowId else {
            Log.island.info("land: sidecar sizes no window — \(self.rendererWindowAbsence, privacy: .public)")
            return
        }
        capturePristineLayout(win)
        send(rawCommand: "resize-window -t \(win) -x \(cols) -y \(rows)")
        resizedWindows.insert(win)
    }

    /// Switch the attached client to another session, then re-discover its
    /// windows/panes so the snapshot follows.
    func selectSession(_ sessionId: String) {
        // See the matching comment in selectPane: a manual switch must not
        // get silently overridden by a still-pending reconnect restore.
        pendingRestore = nil
        sessionSwitches &+= 1
        // The sidecar's renderer leaves these (read before the snapshot
        // moves on).
        let left = rendererSessionId
        let leftWindow = rendererWindowId
        if snapshot.activeSessionId != sessionId {
            snapshot.activeSessionId = sessionId
        }
        // Leaving a session Moshpit created gives its status line back;
        // arriving on one turns it off. Any other session: nothing.
        syncStatusLine()
        if rendersOutput {
            // B150: the switch sizes the session's windows by us (its latest
            // client): what it squashes is read on both sides.
            probeSwitchLayouts(sessionId)
            send(rawCommand: "switch-client -t \(sessionId)")
        } else {
            // Control-plane mode: `switch-client` alone would move only THIS
            // (invisible) control client. The screen the user sees is the
            // phone's mosh renderer — it comes along, and NOTHING else does:
            // every other client is a screen this phone does not own, the
            // desktop's terminal first of all.
            //
            // It used to be every non-control client on the session being
            // left. A desktop sits on that session more often than not, and
            // its terminal jumped sessions with the phone; the session the
            // sidecar follows is itself a guess from tmux's attached flags;
            // and with no session known at all the filter let every client
            // through, desktops on other sessions included. Now the renderer
            // is one specific client (``rendererClient``), and when it is not
            // known — or not attached any more — no client moves and the log
            // says so.
            sendCommand("list-clients -F '\(TmuxClientRow.format)'") { [weak self] response in
                guard let self else { return }
                let rows = response.isError ? [] : response.lines.compactMap(TmuxClientRow.init)
                let renderer = self.rendererClient
                let moving = renderer.map { client in
                    rows.filter { !$0.isControl && $0.identity == client }
                } ?? []
                if moving.isEmpty {
                    let why = renderer.map { "renderer \($0.name) pid=\($0.pid) is not attached" }
                        ?? "the renderer is not known"
                    Log.island.info("land: sidecar switch to \(sessionId, privacy: .public) moves no client — \(why, privacy: .public)")
                }
                // The renderer sizes the session it lands on (its latest
                // client, the only one when the phone is alone): what the
                // move squashes is read on both sides of it, as our own
                // switch's is (B150).
                if !moving.isEmpty { self.probeSwitchLayouts(sessionId) }
                for row in moving {
                    self.send(rawCommand: "switch-client -c \(row.name) -t \(sessionId)")
                }
                // Where the renderer sits from here: the session it is being
                // moved to, or — not attached — nowhere the sidecar may size.
                if renderer != nil, !response.isError {
                    self.rendererSessionId = moving.isEmpty ? nil : sessionId
                }
                self.send(rawCommand: "switch-client -t \(sessionId)")
                // A listing sent before these moves (a re-list the switch
                // itself set off) still shows the renderer where it was.
                self.sessionSwitches &+= 1
                // The session it left is handed back.
                if !moving.isEmpty, let left, left != sessionId {
                    self.handBackLeftSession(left, window: leftWindow)
                }
            }
        }
        refreshWindowsAndPanes()
    }

    /// Send an arbitrary tmux command on the control channel, ignoring the
    /// response. Used by the mosh control plane for owned-session setup
    /// (`set-option … status off` / restore). Dropped until attached.
    func runCommand(_ command: String) {
        send(rawCommand: command)
    }

    /// Focus a pane and show it full-screen. Moshpit always displays ONE pane,
    /// so the active pane is zoomed to fill the window; switching to a pane in
    /// another window first restores the old window's split layout (other
    /// clients may be viewing it).
    func selectPane(_ paneId: String) {
        // A user-initiated switch always wins over a reconnect's queued
        // restore-to-last-pane — otherwise a still-pending `applyPendingRestoreIfReady()`
        // (waiting on its own `list-panes` round trip) could land moments
        // later and silently snap back to the pre-reconnect pane, stacking a
        // second veil→resync cycle on top of this one (the reported garbled
        // flash on switch-during-reconnect).
        Log.island.info("land: selectPane \(paneId, privacy: .public) from=\(self.snapshot.activePaneId ?? "-", privacy: .public) attached=\(self.snapshot.isAttached, privacy: .public) droppedRestore=\(self.pendingRestore?.pane ?? "-", privacy: .public)")
        pendingRestore = nil
        // A pane known only from its own %output has no window yet — it
        // streamed before discovery placed it, which is exactly what an agent
        // mid-turn does on a fresh attach. Switching now made "" the active
        // window, and the discovery reply right behind read that as "ours is
        // gone" and adopted tmux's current window: whichever one a desktop
        // client sharing the session was looking at. That is where a tapped
        // notification used to land after a reconnect (真机 2026-10-06: asked
        // for islander %5, got the desktop's window). Hold the choice as the
        // pending restore instead; the next `list-panes` places the pane and
        // replays it.
        if let pane = snapshot.panes[paneId], pane.windowId.isEmpty {
            pendingRestore = TmuxSelection(session: snapshot.activeSessionId ?? "",
                                           window: nil, pane: paneId)
            refreshWindowsAndPanes()
            return
        }
        releaseScrollHolds()
        let oldWindow = snapshot.activeWindowId
        let newWindow = snapshot.panes[paneId]?.windowId ?? oldWindow
        // Cross-session: switch the client if this pane's window is elsewhere.
        if let nw = newWindow, let sid = snapshot.windows[nw]?.sessionId, !sid.isEmpty,
           sid != snapshot.activeSessionId {
            selectSession(sid)
        }
        if let newWindow, newWindow != oldWindow {
            if let oldWindow { releaseWindowPin(oldWindow) }
            snapshot.activeWindowId = newWindow
            // Pre-size before activating — see selectWindow: avoids streaming a
            // desktop-wide window's %output into our narrow pane (CJK `？` on switch).
            requestPin(newWindow, reason: .user)
            send(rawCommand: "select-window -t \(newWindow)")
            // Fill the window we just brought on screen. `selectPane` handles a
            // cross-window jump ITSELF rather than calling `selectWindow`, so
            // the fill hook there does not cover this path — and a deep link
            // (`moshpit://connection/<id>?pane=%N`) lands here, not there. A
            // real-device run caught it: the window switched, the pane resynced,
            // and its scrollback never arrived.
            ensureTerminalsForAllPanes()
        }
        // Veil before the SwiftUI swap — see selectWindow.
        paneCoordinators[paneId]?.veilForSwitch()
        // The veil's default 0.8s safety timeout assumes a near-instant
        // resync round trip. Right after a reconnect, this pane's
        // capture-pane sits behind a backlog of discovery commands
        // (list-sessions/windows/panes, per-pane backfill) in the same FIFO,
        // which can easily outlast 0.8s over a real connection's RTT — the
        // timeout was firing and revealing stale/incomplete content before
        // resyncPane's own reveal() arrived (the reported garbled flash on
        // switch-during-reconnect). Same fix as commitClientSize's resize path.
        paneCoordinators[paneId]?.extendCoverTimeout(by: coverGrace())
        // Explicit withAnimation — see the matching comment in selectWindow.
        withAnimation(.easeInOut(duration: 0.22)) {
            snapshot.activePaneId = paneId
        }
        // -Z: stay zoomed while switching — the new pane replaces the old one
        // full-screen in a single layout change, instead of the unzoom → select
        // → re-zoom dance (three redraws, visible split flash).
        send(rawCommand: "select-pane -Z -t \(paneId)")
        ensureImmersiveZoom()                          // first landing on an unzoomed split
        refreshActivePaneMouse()                       // wheel-vs-copy-mode signal
        // Repaint from tmux's model, two passes — activePaneId is this pane
        // by now, and the resize/zoom above race its SIGWINCH redraw exactly
        // like selectWindow's (see resyncActivePaneSettling).
        resyncActivePaneSettling()
    }

    /// Horizontal-swipe navigation: cycle the active window's panes when it has
    /// more than one, otherwise cycle the active session's windows. Wraps around.
    /// `forward` = next (swipe left), false = previous (swipe right). Reuses
    /// ``selectPane``/``selectWindow`` — the same path the breadcrumb uses — so it
    /// drives SSH+tmux and mosh+tmux alike (select-* changes the session's
    /// current pane/window, which the mosh-rendered client follows too).
    func switchPaneOrWindow(forward: Bool) {
        guard snapshot.isAttached, let windowId = snapshot.activeWindowId else { return }
        snapshot.lastSwitchForward = forward
        snapshot.hasSwipedSinceAttach = true
        let panes = snapshot.panes(inWindow: windowId)
        if panes.count > 1 {
            if let next = Self.cycled(panes.map(\.id),
                                      from: snapshot.activePaneId ?? panes.first?.id,
                                      forward: forward) {
                selectPane(next)
            }
            return
        }
        // One pane (or unknown) → cycle windows in the active session.
        if let next = Self.cycled(snapshot.sortedWindows.map(\.id), from: windowId, forward: forward) {
            selectWindow(next)
        }
    }

    /// Pure index cycling for swipe navigation: given ordered ids, the active id,
    /// and direction (`forward` = next, else previous), return the id to switch
    /// to, wrapping around the ends. nil when there's nothing to do (<2 ids, the
    /// active id isn't in the list, or it would land back on itself). Static +
    /// nonisolated so it's unit-testable without a live tmux server.
    nonisolated static func cycled(_ ids: [String], from active: String?, forward: Bool) -> String? {
        guard ids.count > 1, let active, let idx = ids.firstIndex(of: active) else { return nil }
        let next = ids[(idx + (forward ? 1 : -1) + ids.count) % ids.count]
        return next == active ? nil : next
    }

    /// True iff `id` matches tmux's actual id shape: `%<digits>` (pane),
    /// `@<digits>` (window), or `$<digits>` (session) — see tmux(1). Every
    /// pane/window/session id that reaches this controller — discovery
    /// replies, live `%…` notifications, and any caller-supplied pane id
    /// (e.g. a Live Activity / App Intent action routed through
    /// `SessionHub.deliverAgentInput`) — gets spliced UNQUOTED into a `-t
    /// <id>` argument of a raw `-CC` control command (`select-window -t
    /// \(id)`, `send-keys -t \(id) …`). None of that is remote-controlled in
    /// the well-behaved case, but the remote tmux server (or another client
    /// sharing the session) ultimately produces every id this file sees, and
    /// a value that doesn't match this shape could otherwise inject
    /// additional control-mode commands, or — for the App Intent path — let
    /// a caller-supplied string target something other than a real pane.
    /// Reject anything else at the boundary rather than trust it blindly.
    nonisolated static func isValidTmuxId(_ id: String) -> Bool {
        guard let first = id.first, first == "%" || first == "@" || first == "$" else { return false }
        let digits = id.dropFirst()
        return !digits.isEmpty && digits.allSatisfy { $0.isASCII && $0.isNumber }
    }

    /// Render `text` as a double-quoted tmux COMMAND-STRING literal, safe to
    /// splice into a `-CC` control-mode line (used by ``sendPaste(_:paneId:)``
    /// to build `set-buffer`'s `data` argument).
    ///
    /// Control mode is line-framed and parses each line with tmux's own
    /// command-string grammar — the SAME parser `.tmux.conf` and
    /// `command-prompt` use (`man tmux`, COMMAND PARSING AND EXECUTION) —
    /// which is NOT the parser that applies when argv is already split by a
    /// shell before a CLI process sees it. That distinction bit the first
    /// version of this escaper: piping the identical escaped string through
    /// `tmux set-buffer -b x "<arg>"` as pre-split argv left every backslash
    /// untouched (argv splitting bypasses tmux's command-string parser
    /// entirely), while running the exact same line through `source-file`
    /// (which — like control mode — DOES run that parser) decoded it
    /// correctly (verified against tmux 3.6a, private `-L` lab socket,
    /// 2026-08-19).
    ///
    /// Inside that grammar, double-quoted text still performs `$VAR`
    /// expansion, leading-`~` expansion, and recognizes `\n \r \t \e`,
    /// `\uXXXX`, `\ooo` — so every character that could trigger one of those,
    /// plus the quote/backslash delimiting the string itself, is escaped
    /// here. A raw newline byte can never appear in the output at all: the
    /// control channel reads one line as one command, so an un-escaped
    /// newline would truncate the command rather than land inside the
    /// buffer. Everything else — including multi-byte Unicode and emoji —
    /// passes through as literal UTF-8; `#` and `#{...}` are safe unescaped
    /// too (comments only trigger on an UNQUOTED `#`, and `set-buffer`'s
    /// `data` argument is not treated as a format string, so `#{pane_id}`
    /// does not expand). All of this was checked round-trip against tmux
    /// 3.6a, not just read off the man page: `$`, leading and mid-string `~`,
    /// `"`, `'`, `\`, tab, `#{...}`, CJK, and emoji all survived intact.
    nonisolated static func tmuxCommandStringLiteral(_ text: String) -> String {
        var out = "\""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\\": out += "\\\\"
            case "\"": out += "\\\""
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            case "$": out += "\\$"
            case "~": out += "\\~"
            default:
                if scalar.value < 0x20 || scalar.value == 0x7f {
                    out += String(format: "\\u%04x", scalar.value)
                } else {
                    out.unicodeScalars.append(scalar)
                }
            }
        }
        out += "\""
        return out
    }

    /// True when any capture-pane line is laid out for a grid WIDER than
    /// `cols` — the tell of a frame captured mid-flap at another client's
    /// size. Width is estimated conservatively (combining marks 0, only
    /// unambiguously-wide East Asian ranges 2, everything else 1), so a
    /// legitimate our-width line can never be overestimated past `cols`:
    /// tmux never captures a line wider than the pane it captured at.
    /// Escape sequences from `capture-pane -e` are skipped: CSI (SGR), and —
    /// critically — OSC/DCS strings, whose PAYLOAD is invisible: Claude Code
    /// emits OSC 8 hyperlinks whose file:// URL made a 65-column line
    /// estimate as 128 and every repair frame containing one got falsely
    /// rejected, starving the repair loop until the fail-open opened the
    /// gate with nothing repainted (真机取证 2026-08-19, 42 consecutive
    /// rejects — the sustained-scroll overlap). Under-skipping is unsafe,
    /// over-skipping merely under-counts, which this check tolerates.
    nonisolated static func frameExceedsWidth(_ lines: [String], cols: Int) -> Bool {
        for line in lines {
            var width = 0
            var scalars = line.unicodeScalars[...]
            while let scalar = scalars.first {
                scalars = scalars.dropFirst()
                if scalar.value == 0x1b {                     // ESC …
                    guard let intro = scalars.first else { continue }
                    if intro.value == 0x5b {                  // CSI: params, final
                        scalars = scalars.dropFirst()
                        while let p = scalars.first, (0x20...0x3f).contains(p.value) {
                            scalars = scalars.dropFirst()
                        }
                        if scalars.first != nil { scalars = scalars.dropFirst() }
                    } else if intro.value == 0x5d || intro.value == 0x50 {
                        // OSC / DCS: payload runs to BEL or ST (ESC \) and
                        // renders as NOTHING.
                        scalars = scalars.dropFirst()
                        while let p = scalars.first {
                            scalars = scalars.dropFirst()
                            if p.value == 0x07 { break }
                            if p.value == 0x1b {
                                if scalars.first?.value == 0x5c { scalars = scalars.dropFirst() }
                                break
                            }
                        }
                    } else {
                        // ESC + intermediates (0x20–0x2F) + one final byte
                        // (charset selects like ESC ( B, and ST's ESC \).
                        while let p = scalars.first, (0x20...0x2f).contains(p.value) {
                            scalars = scalars.dropFirst()
                        }
                        if scalars.first != nil { scalars = scalars.dropFirst() }
                    }
                    continue
                }
                width += Self.estimatedColumns(scalar)
                if width > cols { return true }
            }
        }
        return false
    }

    /// Conservative wcwidth: 0 for combining/zero-width, 2 only for
    /// unambiguously wide ranges, else 1. Under-counting is safe here (the
    /// caller only needs "definitely wider than our grid"); over-counting
    /// a legitimate line is what must never happen.
    private nonisolated static func estimatedColumns(_ s: Unicode.Scalar) -> Int {
        switch s.value {
        case 0x0300...0x036f, 0x200b...0x200f, 0xfe00...0xfe0f, 0x20d0...0x20ff:
            return 0
        case 0x1100...0x115f, 0x2e80...0x303e, 0x3041...0x33ff,
             0x3400...0x4dbf, 0x4e00...0x9fff, 0xa000...0xa4cf,
             0xac00...0xd7a3, 0xf900...0xfaff, 0xfe30...0xfe4f,
             0xff00...0xff60, 0xffe0...0xffe6,
             0x1f300...0x1f64f, 0x1f900...0x1f9ff,
             0x20000...0x2fffd, 0x30000...0x3fffd:
            return 2
        default:
            return 1
        }
    }

    /// Flush any output held for scrollback reading across every pane. Called on
    /// pane/window switches so a pane left scrolled-up doesn't sit frozen
    /// (buffering output it never shows) when the user comes back to it.
    func releaseScrollHolds() {
        for coordinator in paneCoordinators.values { coordinator.releaseScrollHold() }
    }

    /// When each pane last produced `%output`.
    @ObservationIgnored private var lastPaneOutputAt: [String: Date] = [:]

    /// How recently a pane must have written to count as still streaming.
    ///
    /// Long enough to bridge the gaps between an agent's tokens, short enough
    /// that a finished answer stops being "busy" while the user is still
    /// deciding to scroll. Settable so tests assert the BEHAVIOUR without
    /// racing a wall clock — a test that has to beat 0.5s passes alone and
    /// fails under a full-suite load.
    @ObservationIgnored var streamingWindow: TimeInterval = 0.5

    /// True while this pane is actively painting — see ``scroll(lines:)``.
    private func isStreaming(_ paneId: String) -> Bool {
        guard let last = lastPaneOutputAt[paneId] else { return false }
        return Date().timeIntervalSince(last) < streamingWindow
    }

    /// Whether the ACTIVE pane's program has the mouse on (`#{mouse_any_flag}`).
    /// The signal that decides scroll strategy for a tmux pane: a mouse app
    /// (Claude Code, vim --mouse, less --mouse) gets a forwarded wheel; a plain
    /// shell gets copy-mode. Read off tmux directly because under `mouse on` the
    /// rendered terminal can't tell (tmux owns the outer terminal's mouse).
    /// Refreshed on pane/window switch and at the start of each scroll drag.
    @ObservationIgnored private(set) var activePaneWantsMouse = false

    /// Whether the active pane is on the ALTERNATE screen (`#{alternate_on}`).
    /// Decisive for scroll strategy: an alt-screen pane has NO tmux history
    /// (`history_size` 0 — measured on Claude Code 2.1.233), so copy-mode has
    /// literally nothing to scroll. Every alt-screen TUI must get the wheel
    /// and only the wheel.
    @ObservationIgnored private(set) var activePaneOnAltScreen = false

    /// Re-read the active pane's `#{mouse_any_flag}` + `#{alternate_on}` and
    /// cache them. Cheap; one control round-trip. No-op until attached / with
    /// no active pane.
    func refreshActivePaneMouse() {
        guard snapshot.isAttached,
              let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        sendCommand("display-message -p -t \(paneId) '#{mouse_any_flag} #{alternate_on}'") { [weak self] response in
            let flags = response.lines.first?
                .trimmingCharacters(in: .whitespaces)
                .tmuxFields(separator: " ") ?? []
            self?.activePaneWantsMouse = flags.first == "1"
            self?.activePaneOnAltScreen = flags.count > 1 && flags[1] == "1"
        }
    }

    /// Scroll the active pane (swipe / thumb). Decides strategy from whether the
    /// pane's app wants the mouse:
    ///  - mouse app → synthesize a wheel into the pane (`send-keys` the SGR
    ///    bytes); the app scrolls itself and tmux never enters copy-mode, so
    ///    typing keeps working.
    ///  - plain shell → page the LOCAL SwiftTerm scrollback (seeded by
    ///    ``backfill``, appended by `%output`). Positive = older (up),
    ///    negative = newer (down).
    ///
    /// Copy-mode is NEVER driven from here. This controller is a control-mode
    /// (-CC) client, and tmux draws copy-mode only for regular tty clients: a
    /// `-CC` client receives ZERO `%output` while copy-mode scrolls (measured
    /// against tmux 3.6a, scripts/spikes/tmux-cc-lab `copymode` scenario,
    /// 2026-08-19). Driving copy-mode from this path scrolled a screen the
    /// phone can never see — and hijacked any desktop client sharing the
    /// window into copy-mode. (The mosh+tmux path is different: its renderer
    /// IS a regular client, so SessionHub's copy-mode driver stays correct.)
    /// Where a scroll tick over the active pane goes. One decision shared by
    /// the row and pixel entry points so they can never disagree.
    private enum ScrollRoute { case wheel, drop, local }

    private func scrollRoute() -> (paneId: String, route: ScrollRoute)? {
        guard let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return nil }
        // An ALT-SCREEN mouse app (Claude Code) gets the wheel unconditionally:
        // its own repaint after a wheel updated `lastPaneOutputAt`, which
        // read as "streaming" and flipped the NEXT tick into the parked path,
        // where the sticky latch kept every later swipe. First swipe scrolled,
        // the rest were dead (the 347 report).
        if activePaneWantsMouse && activePaneOnAltScreen {
            return (paneId, .wheel)
        }
        // NON-alt panes: forwarding the wheel to an app that is CURRENTLY
        // repainting makes the two fight over the same viewport: the app
        // scrolls where we asked, then its next frame pins back to the
        // bottom, then the next swipe scrolls again — the reported judder
        // while output streams. Park the viewport in the LOCAL scrollback
        // instead: the coordinator's scroll-hold buffers live output while
        // the user reads, exactly like the plain-SSH path.
        if activePaneWantsMouse && !isStreaming(paneId) {
            return (paneId, .wheel)
        }
        if activePaneOnAltScreen {
            // Alt-screen pane whose wheel we can't forward (no mouse, or the
            // flags haven't settled): the LOCAL buffer holds no real history
            // for it — tmux never replays an already-drawn TUI on attach, so
            // local scrollback is just backfill's single stale seed frame.
            // Paging it shows an old screenshot overlapping the live screen
            // (真机取证 2026-08-19), and the scroll-hold it engages can never
            // release by scrolling (the remote is alt-screen; there is no
            // local bottom to return to that means anything), freezing live
            // output until the next keystroke. Drop the ticks instead: for a
            // mouse app the next tick forwards the wheel once the flags
            // settle.
            return (paneId, .drop)
        }
        return (paneId, .local)
    }

    func scroll(lines: Int) {
        guard lines != 0, let routed = scrollRoute() else { return }
        switch routed.route {
        case .wheel:
            sendWheel(lines: lines, paneId: routed.paneId)
        case .drop:
            return
        case .local:
            paneCoordinators[routed.paneId]?.scrollLocal(lines: lines)
        }
    }

    /// Sub-row finger travel not yet turned into a wheel tick; reset when a
    /// drag starts (``refreshActivePaneMouse`` is that moment).
    @ObservationIgnored private var wheelCarry = ScrollRowCarry()

    /// Pixel-precise sibling of ``scroll(lines:)`` for the pan gesture
    /// (positive = older). Same routing; the local scrollback moves by the
    /// exact distance so the content follows the finger, the wheel gets whole
    /// rows with the remainder carried. Returns the distance consumed — 0 for
    /// a dropped tick or an exhausted buffer.
    @discardableResult
    func scroll(pixels dy: CGFloat) -> CGFloat {
        guard dy != 0, let routed = scrollRoute() else { return 0 }
        switch routed.route {
        case .wheel:
            let cellHeight = paneCoordinators[routed.paneId]?.cellHeight ?? 16
            let rows = wheelCarry.take(pixels: dy, cellHeight: cellHeight)
            if rows != 0 { sendWheel(lines: rows, paneId: routed.paneId) }
            return dy
        case .drop:
            return 0
        case .local:
            return paneCoordinators[routed.paneId]?.scrollLocal(pixels: dy) ?? 0
        }
    }

    /// The drag came to rest: let a locally
    /// scrolled pane dock onto the bottom if it stopped within half a row.
    func scrollDidEnd() {
        guard let routed = scrollRoute(), routed.route == .local else { return }
        paneCoordinators[routed.paneId]?.settleLocalScroll()
    }

    /// Synthesize SGR wheel bytes into the pane's program.
    private func sendWheel(lines: Int, paneId: String) {
        let grid = paneTerminals[paneId]?.grid
        let col = max(1, (grid?.cols ?? 80) / 2)
        let row = max(1, (grid?.rows ?? 24) / 2)
        let wheel = SessionHub.ActiveSession.wheelBytes(lines: lines, col: col, row: row)
        if !wheel.isEmpty { sendInput(wheel, paneId: paneId) }
    }

    /// Forward a click at a 0-based, viewport-relative cell to the active pane's
    /// program — a tap asking it to move its cursor there.
    ///
    /// Gated on the same `#{mouse_any_flag}` as the wheel, and for the same
    /// reason: `send-keys` puts these bytes straight into the pane, past tmux's
    /// own mouse handling, so a pane running a plain shell would simply echo
    /// `0;12;3M` into its command line. Under `mouse on` tmux would otherwise
    /// keep the click for itself (pane focus, selection) and the program would
    /// never see it.
    func click(col: Int, row: Int) {
        guard activePaneWantsMouse,
              let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        sendInput(SessionHub.ActiveSession.clickBytes(col: col, row: row), paneId: paneId)
    }

    /// Look up the server's tmux `prefix` (e.g. "C-b") so the mosh copy-mode
    /// driver can enter copy-mode with `prefix [` on the mosh-rendered client.
    func queryPrefixKey(_ completion: @escaping (String) -> Void) {
        sendCommand("show-options -gqv prefix") { response in
            completion(response.lines.first ?? "C-b")
        }
    }

    /// The pane's working directory as tmux reports it — where the pane's
    /// program runs, which is what an agent's relative paths are relative
    /// to. nil when not attached, on a tmux error, or after 4s without a
    /// reply (a dying connection never answers).
    func panePath(_ paneId: String) async -> String? {
        // Spliced into the command bare: a tmux id or nothing (B146, B139).
        guard snapshot.isAttached, Self.isValidTmuxId(paneId) else { return nil }
        let answer: String?? = await withTimeoutValue(4) { [weak self] in
            await withCheckedContinuation { (continuation: CheckedContinuation<String?, Never>) in
                Task { @MainActor [weak self] in
                    guard let self, self.snapshot.isAttached else {
                        continuation.resume(returning: nil)
                        return
                    }
                    self.sendCommand("display-message -p -t \(paneId) '#{pane_current_path}'",
                                     waits: true) { response in
                        // The line exactly as tmux printed it: the parser has
                        // already cut it at `\n` and dropped the `\r` before
                        // that. Any space or tab left belongs to the
                        // directory's name — tmux 3.6a prints a pane in
                        // `proj ` as `…/proj \r\n` — and trimming it sent the
                        // agent's relative paths and the files key to `proj`,
                        // which is another directory or none at all.
                        let path = response.lines.first ?? ""
                        // Absolute = the first BYTE is `/`. `hasPrefix("/")`
                        // compares Characters, and a cwd whose first component
                        // starts with a combining mark ("/\u{301}x") begins
                        // with the Character "/\u{301}" — turned away as if
                        // tmux had answered nothing.
                        let isAbsolute = path.utf8.first == UInt8(ascii: "/")
                        continuation.resume(returning: !response.isError && isAbsolute ? path : nil)
                    }
                }
            }
        }
        return answer ?? nil
    }

    // A pane-targeted paste (e.g. the Home card's per-agent image entry)
    // used to decide bracketing HERE, by reading back
    // the pane terminal's bracketed-paste mode — the local terminal's
    // memory of having SEEN the pane's own `ESC[?2004h` go by.
    // That memory is permanently wrong after a fresh SSH+tmux attach: tmux
    // does not replay mode-setting sequences on `-CC attach` (lab-verified,
    // `scripts/spikes/tmux-cc-lab`, 2026-08-19), so the flag starts (and stays)
    // false even for a pane whose program has bracketed paste on — Claude
    // Code then read a pasted image path as hand-typed text instead of
    // `[Image #N]`. `sendPaste(_:paneId:)` (near `sendInput(_:paneId:)`)
    // asks tmux itself instead (`paste-buffer -p`), which tracks the real
    // per-pane state server-side.

    /// Cancel a stale copy-mode on the ACTIVE pane — a leftover from a
    /// connection that died mid-scroll (the exit keystroke never got sent, and
    /// tmux preserves the mode + scroll offset server-side). Used by the mosh
    /// path right after its renderer attaches: the renderer is a regular tmux
    /// client, so it would otherwise come up showing the old scrolled-away
    /// view, and the first swipe would continue from that stale offset. The
    /// SSH path gets the same treatment per-pane in `backfill`.
    func cancelStaleCopyMode() {
        guard snapshot.isAttached,
              let pane = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        sendCommand("display-message -p -t \(pane) '#{pane_in_mode}'") { [weak self] resp in
            guard let self,
                  resp.lines.first?.trimmingCharacters(in: .whitespaces) == "1" else { return }
            self.send(rawCommand: "send-keys -t \(pane) -X cancel")
        }
    }

    /// The window id printed by an in-flight ``newWindow`` (`new-window -P`),
    /// consumed by that same call's discovery completion. Held here rather
    /// than captured in a closure so both callbacks of the one round trip can
    /// see it without boxing a local `var` across two escaping closures.
    @ObservationIgnored private var pendingWindowFocus: String?

    /// Create a new window in the current session, optionally named, and land
    /// ON it.
    ///
    /// `-P -F '#{window_id}'` prints the new id so we can `selectWindow` it
    /// explicitly. tmux does make the new window its session's current one,
    /// but ``parseListWindows`` deliberately refuses to follow tmux's current
    /// window when ours is still alive — that's what stopped another client's
    /// switch from yanking the phone's view (the "自己跳 window" fix) — so
    /// discovery alone leaves us sitting on the old window and "New window"
    /// reads as "did nothing". Same shape as ``newSession``.
    ///
    /// No `releaseWindowPin` up front: ``selectWindow`` hands the old window
    /// back as part of the switch, and doing it here would strand a desktop
    /// client at the phone width on the path where `new-window` fails.
    func newWindow(named name: String? = nil) {
        createWindow(inSession: nil, named: name)
    }

    /// The Home tree's session-row "New window": the target need not be the
    /// session we're attached to. `new-window -t <session>:` creates at that
    /// session's next free index and makes it current there; the create+jump
    /// below then switches us over (`selectWindow` handles the cross-session
    /// hop). Explicit `-t` rather than switching first: in control-plane mode
    /// `selectSession` moves the visible client from a `list-clients`
    /// completion — AFTER a bare `new-window` would already have run in the
    /// old session.
    func newWindow(inSession sessionId: String, named name: String?) {
        let elsewhere = Self.isValidTmuxId(sessionId) && sessionId != snapshot.activeSessionId
        createWindow(inSession: elsewhere ? sessionId : nil, named: name)
    }

    /// The Home tree's window-row "New pane": split `windowId` even when it is
    /// not on screen. Switching to it first (cross-session included) points
    /// `activePaneId` at its pane synchronously, so the plain split — which
    /// targets that pane and zooms that window — lands in the right place;
    /// the control channel is FIFO, so tmux sees the switch before the split.
    func newPane(inWindow windowId: String) {
        if snapshot.windows[windowId] != nil, windowId != snapshot.activeWindowId {
            selectWindow(windowId)
        }
        newPane()
    }

    private func createWindow(inSession sessionId: String?, named name: String?) {
        let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if let refusal = MultiplexerName.refusal(trimmed) {
            flashNotice(refusal)
            return
        }
        let namePart = trimmed.isEmpty ? "" : " -n \(Self.tmuxNameArgument(trimmed))"
        let targetPart = sessionId.map { " -t \($0):" } ?? ""
        pendingWindowFocus = nil
        sendCommand("new-window\(targetPart) -P -F '#{window_id}'\(namePart)") { [weak self] response in
            guard let self else { return }
            if response.isError {
                self.flashNotice(response.lines.first ?? "new-window failed")
                return
            }
            // `isValidTmuxId` (not just `hasPrefix("@")`) — the id is spliced
            // unquoted into `-t` arguments all through selectWindow.
            guard let id = response.lines.first?.trimmingCharacters(in: .whitespaces),
                  Self.isValidTmuxId(id) else { return }
            self.pendingWindowFocus = id
        }
        // Sent in the SAME batch, so the whole create+jump costs one round
        // trip: tmux answers in strict send order, so the id above is already
        // stashed by the time this completion runs — and by then the snapshot
        // knows the window's session and pane, which selectWindow needs.
        refreshWindowsAndPanes { [weak self] in
            guard let self, let id = self.pendingWindowFocus else { return }
            self.pendingWindowFocus = nil
            guard self.snapshot.windows[id] != nil else { return }
            self.selectWindow(id)
        }
    }

    /// Hand the window we're LEAVING back to other clients: drop our
    /// phone-sized pin so a desktop client viewing it gets its full width
    /// back. Only the currently-active window stays pinned to our size; on
    /// return, fitWindowToClient re-pins it.
    ///
    /// The split layout and zoom state we found are restored too — see
    /// ``pristineLayouts`` for why the pin destroys the layout and
    /// ``restorePristineLayout`` for why the unpin has to come first.
    ///
    /// While we still size the session (its only client, or its latest) the
    /// replay is refitted to our grid: the record is kept for the hand-back
    /// that can restore it (B150).
    private func releaseWindowPin(_ windowId: String) {
        if resizedWindows.contains(windowId) {
            resizedWindows.remove(windowId)
            // Unset the per-window override — the REAL "back to automatic".
            // (`resize-window -A` merely re-pins at "largest session", still
            // manual, which kept stranding desktop clients.)
            send(rawCommand: "set-option -u -w -t \(windowId) window-size")
        }
        releaseSizeOwnership(windowId)
        checkReplay(windowId, kept: !rendersOutput)
    }

    /// True between `releaseWindowPins()` and the next `repinActiveWindow()` —
    /// i.e. while the terminal is backgrounded (user returned Home). Guards the
    /// async client-size query in `releaseWindowPins` against a fast Back→re-enter
    /// race: if we re-pinned for the foreground before the query's reply lands,
    /// the stale callback must not re-size the window back to the desktop.
    @ObservationIgnored private var pinsReleased = false

    /// Per-pane bell scanners, used only while ``pinsReleased`` keeps output out
    /// of SwiftTerm's parser. See ``BellDetector`` for why a plain byte scan is
    /// wrong (it turns every title update into a notification).
    @ObservationIgnored private var bellDetectors: [String: BellDetector] = [:]

    /// Hand our pinned windows back to a desktop client sharing this session,
    /// WITHOUT tearing the connection down. Called when the terminal leaves the
    /// foreground (user returned Home) — a plain Back keeps the connection alive
    /// for background monitoring, so our phone-grid pin would otherwise strand a
    /// desktop client at the phone width.
    ///
    /// A plain `resize-window -A` (back to automatic) is NOT enough here: the
    /// session runs `window-size latest`, and our `-CC` client keeps re-winning
    /// "latest" every time the 2s agent-hook poll fires — so automatic sizing
    /// snaps right back to the phone width. Instead we look up the largest real
    /// (non-control) client and MANUALLY pin our windows to its size; a manual
    /// pin is immune to the "latest" churn. `repinActiveWindow()` restores the
    /// phone grid on return; `SessionHub.stop()` does a full `-A` on disconnect.
    func releaseWindowPins() {
        // A keyboard-dismiss resize is usually still in its debounce window
        // when the user backs out — cancel it (before any guard) so it can't
        // fire after we leave and pin a window nobody is looking at.
        pendingResize?.cancel()
        pendingSettleResync?.cancel()
        guard snapshot.isAttached else { return }
        pinsReleased = true
        // Two server-side facts make windows small for a desktop after we
        // leave, and both must be undone HERE — once iOS suspends/kills us we
        // never get another chance to run code, so the server must already be
        // in the right state ("desktop connects later and sees tiny" bug):
        //  1. Our client's reported size + the 2s agent poll keep re-winning
        //     `window-size latest`. `ignore-size` removes us from the sizing
        //     math entirely (verified: %output keeps flowing, so background
        //     monitoring is unaffected).
        //  2. Our manual `resize-window -x` pins. `resize-window -A` does NOT
        //     release them — it just re-pins at "largest session" (still
        //     manual). Unsetting the per-window `window-size` override is what
        //     actually returns a window to automatic sizing.
        send(rawCommand: "refresh-client -f ignore-size")
        //  1b. B150: alone, tmux sizes the windows by us whatever the flag
        //     (it honours `ignore-size` only while another client is
        //     attached): at the recorded size they return to where they were,
        //     and the layouts replayed below fit exactly.
        if rendersOutput, let size = handBackSize(pristineLayouts) {
            send(rawCommand: "refresh-client -C \(size.cols)x\(size.rows)")
        }
        for win in resizedWindows {
            send(rawCommand: "set-option -u -w -t \(win) window-size")
        }
        resizedWindows.removeAll()
        //  2b. Our size lease goes with the pin: a device we were keeping
        //     out of this window may have it now. Following ends too — the
        //     veil belongs to a visible terminal.
        for win in sizeLeases.keys.sorted() { releaseSizeOwnership(win) }
        // And forget what we knew: by the time we come back the other device
        // may hold any of these, so the foreground pin has to ask afresh (a
        // cached `.unset` would let it pin without probing).
        sizeLeases.removeAll()
        sizeFollow = nil
        foregroundProbeFallback?.cancel()
        foregroundProbeFallback = nil
        //  3. Our phone-sized pin also squashed any split layout it covered,
        //     and our immersive zoom hid it. Both go back now — AFTER the
        //     unpin above, or the replayed layout is just refitted to the
        //     phone grid. `repinActiveWindow()` re-captures and re-zooms.
        for win in Array(pristineLayouts.keys) { restorePristineLayout(win) }
        // From here output goes to the bell scanners instead of SwiftTerm's
        // parser; start them clean so a sequence split across this boundary
        // can't leave one stuck mid-string.
        for key in bellDetectors.keys { bellDetectors[key]?.reset() }
    }

    /// Re-pin the active window to our client size when the terminal comes back
    /// to the foreground after `releaseWindowPins()`. Rejoins the sizing math
    /// (`!ignore-size`), re-asserts `lastClientSize` (kept current by
    /// `resizeClient` on SSH and `syncMoshWindow` on the mosh sidecar), then
    /// pins — covering both paths, unlike `fitWindowToClient`, which no-ops for
    /// the sidecar. Snapping the width back immediately avoids a transient
    /// desktop-wide frame (and its CJK reflow garble) before rendering.
    func repinActiveWindow() {
        guard snapshot.isAttached, let win = snapshot.activeWindowId else {
            pinsReleased = false
            return
        }
        // The mosh sidecar sizes only the window the phone's renderer is on
        // (see syncMoshWindow) — and stays out of the sizing math otherwise.
        if !rendersOutput, rendererWindowId == nil {
            pinsReleased = false
            Log.island.info("land: sidecar repins no window — \(self.rendererWindowAbsence, privacy: .public)")
            return
        }
        // B150: what our size is about to squash, read on both sides of it.
        if rendersOutput, let session = snapshot.activeSessionId { probeLayoutsBeforeSizing(session) }
        // The mosh sidecar stays sizeless (B270): a size reported here made
        // it a client tmux sizes the session by — the windows went back to
        // the phone's grid with it, after the renderer was gone, whenever
        // they were handed back to automatic sizing (and tmux streamed it
        // every pane's output it never reads). Its pin is `resize-window`.
        if rendersOutput {
            send(rawCommand: "refresh-client -f !ignore-size")
            send(rawCommand: "refresh-client -C \(lastClientSize.cols)x\(lastClientSize.rows)")
        }
        probeLayoutsAfterSizing()
        if rendersOutput {
            // Another device may have taken the window while we were away
            // (we released our lease on leaving). Ask before pinning — the
            // gate stays closed for that one round trip, so nothing sized for
            // the other device paints here meanwhile; if tmux never answers,
            // pin anyway rather than sit frozen.
            foregroundProbeFallback?.cancel()
            foregroundProbeFallback = Task { [weak self] in
                try? await Task.sleep(for: .seconds(self?.foregroundProbeTimeout ?? 1.5))
                guard !Task.isCancelled, let self, self.pinsReleased else { return }
                Log.ssh.debug("size: foreground probe timed out — pinning")
                self.endForegroundProbe()
                self.pinNow(win, reason: .foreground)
            }
            requestPin(win, reason: .foreground)
        } else {
            pinsReleased = false
            capturePristineLayout(win)
            send(rawCommand: "resize-window -t \(win) -x \(lastClientSize.cols) -y \(lastClientSize.rows)")
            resizedWindows.insert(win)
            // `releaseWindowPins` handed the split layout back (which unzooms), so
            // the single-pane presentation has to be re-established here — it is no
            // longer left standing while we're away.
            ensureImmersiveZoom()
        }
        // Catch the screen up to everything that happened while we were away.
        //
        // `handlePaneOutput` drops `%output` for as long as the pin is handed
        // back (it's laid out for someone else's width), so the buffer still
        // holds the last frame we painted — stale, but correctly wrapped, which
        // is a perfectly good thing to look at for the one round trip this
        // takes. NO cover here on purpose: a veil is a flat fill, and blanking
        // content that is merely a few seconds out of date reads worse than the
        // brief staleness it hides.
        //
        // One pass, not the immediate + settled pair `commitClientSize()` uses:
        // with nothing covering the terminal, a capture that raced the
        // program's post-resize redraw would be shown rather than silently
        // corrected, so it's better to wait for the settle and repaint once.
        guard rendersOutput else { return }
        // 334-model: no gate — stragglers paint and the settle repaint plus
        // the convergence capture make the grid right (see handlePaneOutput).
        armConvergenceResync()
        pendingSettleResync?.cancel()
        pendingSettleResync = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            self?.resyncActivePane()
        }
    }

    /// Split the active window to add a pane, then show that new pane
    /// full-screen (Moshpit shows one pane at a time, never a split). The new
    /// pane becomes active; we zoom it and refresh discovery.
    func newPane() {
        let target = snapshot.activePaneId ?? snapshot.activeWindowId
        if let target {
            send(rawCommand: "split-window -t \(target)")
        } else {
            send(rawCommand: "split-window")
        }
        // The split left the window with >1 pane and the new one active —
        // zoom it to full-screen so we keep the single-pane presentation.
        if let windowId = snapshot.activeWindowId {
            send(rawCommand: "resize-pane -Z -t \(windowId)")
            zoomEpoch &+= 1
        }
        refreshWindowsAndPanes()
    }

    /// True while a user-initiated ``refresh()`` round-trip is in flight, so the
    /// refresh control can show a spinner. Observation-tracked (the one tracked
    /// surface besides ``snapshot``).
    private(set) var isRefreshing = false

    /// `MultiplexerControlling.refresh` — re-run discovery (the -CC path already
    /// pushes updates live, but an explicit refresh covers manual sheet pulls).
    func refresh() {
        // Nothing to refresh (and the trailing round-trip below would be dropped,
        // leaving the spinner stuck) until we're attached.
        guard snapshot.isAttached else { return }
        isRefreshing = true
        // Full re-discovery so the button visibly reflects sessions created or
        // killed elsewhere — not just windows/panes of the current session.
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-sessions") else { return }
            self.parseListSessions(rows)
        }
        refreshWindowsAndPanes()
        // Also converge the visible pane's screen with tmux's model — the
        // user-facing meaning of "refresh" when rendering has gone stale.
        resyncActivePane()
        // tmux answers in FIFO order, so this lands after the discovery above.
        // Hold the spinner a beat longer so a fast (localhost) refresh still
        // registers as one rather than flickering.
        sendCommand("display-message -p done") { [weak self] _ in
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 350_000_000)
                self?.isRefreshing = false
            }
        }
    }

    /// Sessions created BY Moshpit through this controller (never the user's
    /// own). In control-plane mode these get the native look: no status line
    /// while the phone is on them (``syncStatusLine()``).
    ///
    /// By id: the restore used to target the NAME it was created with, and
    /// after a rename from the desktop that name meant nothing — or, through
    /// tmux's prefix match on `-t`, some other session of the user's, whose
    /// own status option the restore then unset.
    @ObservationIgnored
    private(set) var ownedSessionIds: Set<String> = []

    /// The one session whose status line this controller turned off — a
    /// session Moshpit created, while the phone is on it — or nil. It comes
    /// back (`-u status`: the inherited default) when the phone leaves that
    /// session and on disconnect; a reconnect hands it to the replacement
    /// controller in the ``PinLedger``, so a rebuilt sidecar still owes it.
    @ObservationIgnored
    private var statusHiddenSessionId: String?

    /// The status line over mosh: off on a session Moshpit created while the
    /// phone is on it, and on every other session left exactly as it is — no
    /// option set, none unset.
    ///
    /// Every `%session-changed` used to set `status off` on whatever session
    /// the sidecar landed on, the user's own included. `status` is a session
    /// option, so the desktop's status bar vanished along with the phone's,
    /// and only a clean disconnect gave it back: an app killed in the
    /// background left it off for good. tmux has no per-client status line,
    /// so on the user's sessions the phone shows theirs. No-op outside
    /// control-plane mode, and while the phone's session is not known yet —
    /// a rebuilt sidecar must not give back a status line before it knows
    /// the phone has left that session.
    private func syncStatusLine() {
        guard !rendersOutput, snapshot.isAttached, let current = snapshot.activeSessionId else { return }
        let wanted = ownedSessionIds.contains(current) ? current : nil
        guard wanted != statusHiddenSessionId else { return }
        if let hidden = statusHiddenSessionId {
            send(rawCommand: "set-option -t '\(hidden)' -u status")
            send(rawCommand: "set-option -t '\(hidden)' -u \(Self.statusHiddenOption)")
        }
        if let wanted {
            send(rawCommand: "set-option -t '\(wanted)' status off")
            send(rawCommand: Self.hideStampCommand(wanted))
        }
        statusHiddenSessionId = wanted
    }

    // MARK: Sessions Moshpit made, known from the server (B86)

    /// The session option that marks a session Moshpit made: the mark lives
    /// on the server, so a sidecar that never saw the session made — the app
    /// killed and opened again — still knows it. crates/tmux's `OWNED_OPTION`.
    static let ownedOption = "@moshpit_owned"
    /// The session option that says which control client turned that
    /// session's status line off (its `#{client_name}`): one no longer
    /// attached left it off when it died, and the next sidecar gives it back.
    static let statusHiddenOption = "@moshpit_status_hidden"

    /// Stamps `sessionId`'s hidden status line with this control client's own
    /// name (`-F`: tmux expands it for the client that runs the command).
    static func hideStampCommand(_ sessionId: String) -> String {
        "set-option -t '\(sessionId)' -F \(statusHiddenOption) '#{client_name}'"
    }

    /// The sessions Moshpit made, and the status lines a dead sidecar left
    /// off, read from the server at the sidecar's attach: who is attached,
    /// then every session's marks.
    ///
    /// The sessions Moshpit created were known only in memory (and the
    /// ledger, across a rebuilt sidecar): an app killed while the phone sat
    /// on one left its status line off, and the next launch never knew the
    /// session for Moshpit's — the status line stayed off for good, the
    /// desktop's with it. crates/tmux's `read_owned_sessions`.
    private func readOwnedSessions() {
        var clients: Set<String>?
        sendCommand("list-clients -F '#{client_name}'") { response in
            guard !response.isError else { return }
            clients = Set(response.lines.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty })
        }
        sendCommand("list-sessions -F '#{session_id} owned=#{\(Self.ownedOption)} hidden=#{\(Self.statusHiddenOption)}'") { [weak self] response in
            guard let self, !response.isError else { return }
            self.ownedSessionsAnswered(response.lines, clients: clients)
        }
    }

    /// Take the marked sessions as Moshpit's; give a status line back that a
    /// client no longer attached turned off — unless the phone is on that
    /// session, where it stays off, ours now. Nothing is judged dead without
    /// the clients' listing.
    private func ownedSessionsAnswered(_ lines: [String], clients: Set<String>?) {
        let current = snapshot.activeSessionId
        for line in lines {
            let words = line.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: false).map(String.init)
            guard words.count == 3, words[0].hasPrefix("$"), Self.isValidTmuxId(words[0]),
                  words[1].hasPrefix("owned="), words[2].hasPrefix("hidden=") else { continue }
            let id = words[0]
            let owned = words[1].dropFirst("owned=".count) == "1"
            let stamp = words[2].dropFirst("hidden=".count).trimmingCharacters(in: .whitespaces)
            if owned { ownedSessionIds.insert(id) }
            guard let clients, !stamp.isEmpty, !clients.contains(stamp) else { continue }
            if owned, current == id {
                // The phone is on it: it stays off, stamped ours.
                if statusHiddenSessionId == id { send(rawCommand: Self.hideStampCommand(id)) }
                continue
            }
            if statusHiddenSessionId == id { statusHiddenSessionId = nil }
            send(rawCommand: "set-option -t '\(id)' -u status")
            send(rawCommand: "set-option -t '\(id)' -u \(Self.statusHiddenOption)")
        }
        syncStatusLine()
    }

    /// Disconnect-time status restore — over the LIVE -CC control channel, the
    /// same way the status line was turned off.
    ///
    /// We previously restored over a one-shot SSH exec channel, but that runs
    /// through the remote login shell, which mangles the `set-option -t '$id'`
    /// session target (the `$id`) — so the status bar never came back. tmux's
    /// own control channel parses the command directly (no shell), so the
    /// quoting that worked for suppression works here too. A trailing
    /// round-trip blocks until tmux has applied it, so it can't race the
    /// teardown that follows (the original reason exec was chosen).
    func restoreSuppressedStatusAndFlush() async {
        guard snapshot.isAttached, let hidden = statusHiddenSessionId else { return }
        statusHiddenSessionId = nil
        send(rawCommand: "set-option -t '\(hidden)' -u status")
        send(rawCommand: "set-option -t '\(hidden)' -u \(Self.statusHiddenOption)")
        await flushControlChannel(timeout: 2)
    }

    /// Await a single tmux round-trip. Commands are processed in FIFO order, so
    /// once tmux answers this everything queued before it has been applied.
    /// Bounded so a dying channel can't hang teardown.
    private func flushControlChannel(timeout: TimeInterval) async {
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            var resumed = false
            func fire() { if !resumed { resumed = true; continuation.resume() } }
            sendCommand("display-message -p ok", waits: true) { _ in fire() }
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                fire()
            }
        }
    }

    /// Set when navigation lands somewhere new but the window list isn't in
    /// yet (initial attach, session switch) — consumed by the next
    /// `parseListWindows`, which then zooms the active pane.
    @ObservationIgnored
    private var pendingImmersiveZoom = false

    /// Moshpit shows ONE pane full-screen (both SSH and mosh). If the active
    /// window has splits and isn't zoomed, zoom its active pane so it uses the
    /// full width. Single-pane windows need nothing. Undone when we hand the
    /// window back (leave / background / disconnect), together with the layout
    /// the pin squashed — see ``pristineLayouts``.
    private func ensureImmersiveZoom() {
        guard let windowId = snapshot.activeWindowId else { return }
        let epoch = zoomEpoch
        sendCommand("display-message -p -t '\(windowId)' '\(Self.pristineProbeFormat)'") { [weak self] response in
            guard let self, let line = response.lines.first else { return }
            let fields = line.tmuxFields(separator: " ")
            // Same reply, two jobs: the pane count and zoom flag this needs,
            // and the pristine record — but only on the paths where the zoom is
            // all we do (mosh sidecar, a window we never pinned). Behind a pin,
            // this reply already describes the squashed layout, and the probe
            // `fitWindowToClient` queued ahead of that pin is the real record.
            if !self.resizedWindows.contains(windowId) {
                self.notePristineLayout(windowId, fields: fields)
            }
            guard fields.count >= 2, (Int(fields[0]) ?? 1) > 1, fields[1] == "0" else { return }
            // Newer than every zoom change we sent: toggle. Older: it may
            // read "unzoomed" for a window our own toggle zoomed since — ask
            // again.
            if epoch == self.zoomEpoch {
                self.send(rawCommand: Self.zoomCommand(windowId))
                self.zoomEpoch &+= 1
            } else {
                self.ensureImmersiveZoom()
            }
        }
    }

    /// Bumped by every zoom change we send (a toggle, a replayed layout); a
    /// zoom probe remembers the value it was sent at.
    @ObservationIgnored private var zoomEpoch = 0

    /// Zoom a window's active pane: a toggle, sent only on a probe that saw
    /// it unzoomed and is newer than every zoom change we sent
    /// (``ensureImmersiveZoom()``). crates/tmux's `zoom_command`.
    ///
    /// `resize-pane -Z` alone is a toggle, and the probe that leads here can
    /// be one of two in flight: a reconnect's attach zoom and the restore's
    /// `selectPane` both asked before either zoom landed, both read
    /// "unzoomed", and the second toggle undid the first — the window stayed
    /// pinned to the phone grid with its split squashed beneath (真机
    /// 2026-10-09). The epoch keeps the second from toggling.
    ///
    /// It used to be decided by tmux as it ran (`if-shell -F … 'resize-pane
    /// -Z'`). tmux answers an `if-shell` that runs its command with two reply
    /// blocks — its own, then the command's (3.6a, a private server) — and
    /// replies are matched to callbacks in send order: the command sent right
    /// after the zoom got the zoom's empty block, and its own reply went to
    /// the next one (round thirty-four: over mosh, the sidecar's renderer
    /// lookup at the attach, which then never found the renderer).
    static func zoomCommand(_ windowId: String) -> String {
        "resize-pane -Z -t \(windowId)"
    }

    /// Makes the person's first session while nothing is attached, with the
    /// name they gave (nil: tmux's own) — set by the hub, which can reach
    /// the server outside control mode (B228). Moshpit still makes no
    /// session on its own: only ``newSession(named:)`` calls it.
    @ObservationIgnored var createsFirstSession: ((String?) -> Void)?

    /// A notice that a session the person asked for couldn't be made — the
    /// hub's word on a first session (``createsFirstSession``).
    func noteSessionNotCreated(_ detail: String) {
        flashNotice(detail.isEmpty ? "new-session failed" : detail)
    }

    /// Create (and switch to) a new detached-name session.
    /// Transient user-facing notice (tmux command failures). Auto-clears.
    private(set) var notice: String?
    @ObservationIgnored private var noticeClear: Task<Void, Never>?

    private func flashNotice(_ message: String) {
        notice = message
        noticeClear?.cancel()
        noticeClear = Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            self?.notice = nil
        }
    }

    /// Surface a command's %error to the user instead of dropping it — create/
    /// rename/kill used fire-and-forget nil callbacks, so "duplicate session"
    /// or a bad name failed in total silence.
    private func sendSurfacingErrors(_ command: String, verb: String) {
        sendCommand(command) { [weak self] response in
            guard response.isError else { return }
            let detail = response.lines.first ?? ""
            self?.flashNotice(detail.isEmpty ? "\(verb) failed" : detail)
        }
    }

    func newSession(named name: String? = nil) {
        let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        // Quote the name (spaces would split the command — "my project" used to
        // fail silently) and print the new id so we can land ON the session:
        // "Create" that leaves you where you were reads as "did nothing".
        if let refusal = MultiplexerName.refusal(trimmed) {
            flashNotice(refusal)
            return
        }
        // Nothing attached — a server with no session yet, or none left:
        // control mode isn't live, and a command written now would be typed
        // into the shell (`enqueue` drops it). It used to be dropped right
        // there, and the person's "New session" did nothing (B228). The one
        // who can make a session outside control mode makes it — the hub:
        // an exec on mosh's sidecar, `tmux -CC new` in the SSH shell — and
        // the attach that follows lands on it. Still only when asked.
        guard snapshot.isAttached else {
            if let createsFirstSession {
                createsFirstSession(trimmed.isEmpty ? nil : trimmed)
            } else {
                Log.ssh.info("tmux: no session can be made before the attach here")
            }
            return
        }
        let namePart = trimmed.isEmpty ? "" : " -s \(Self.tmuxNameArgument(trimmed))"
        sendCommand("new-session -d -P -F '#{session_id}'\(namePart)") { [weak self] response in
            guard let self else { return }
            if response.isError {
                self.flashNotice(response.lines.first ?? "new-session failed")
                return
            }
            // `isValidTmuxId` (not just `hasPrefix("$")`) so a malformed reply
            // can't slip a non-numeric tail into the `-t \(id)` targets below.
            guard let id = response.lines.first?.trimmingCharacters(in: .whitespaces),
                  Self.isValidTmuxId(id) else { return }
            if !self.rendersOutput, !trimmed.isEmpty {
                // mosh renders tmux's full TUI; a session Moshpit just created
                // adopts the native look (breadcrumb navigation, no status
                // line while the phone is on it — the switch below turns it
                // off, see syncStatusLine). User-created sessions are never
                // styled.
                self.ownedSessionIds.insert(id)
                // B86: on the session itself, so a sidecar after this one —
                // the app killed and opened again — still knows.
                self.send(rawCommand: "set-option -t '\(id)' \(Self.ownedOption) 1")
            }
            self.selectSession(id)
        }
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-sessions") else { return }
            self.parseListSessions(rows)
        }
    }

    // MARK: - Rename / kill (Home session-tree long-press)

    /// A session or window name as one word of a control-mode line, for the
    /// commands that take one (`new-session -s`, `new-window -n`,
    /// `rename-session`, `rename-window`) — exactly the name, as tmux 3.6a
    /// stores it (B139, checked live on a private server):
    ///
    /// * a command-string literal (``tmuxCommandStringLiteral(_:)``): no line
    ///   break can reach the line, and with it no second command — the
    ///   single-quoted word this was let a name holding one end the command
    ///   and run the rest (`run-shell` included) as a command of its own;
    /// * every `#` doubled: tmux expands the name as a format before it keeps
    ///   it, so `#{session_name}` was stored as the session's name and
    ///   `##` comes back as `#`. By scalar, so a `#` that a following mark
    ///   joins into one Character (a keycap) is doubled too.
    ///
    /// A positional name goes after `--`, or one starting with `-` is read as
    /// a flag (`rename-window -t @1 '-x'`: "unknown flag -x").
    nonisolated static func tmuxNameArgument(_ name: String) -> String {
        var doubled = String.UnicodeScalarView()
        for scalar in name.unicodeScalars {
            doubled.append(scalar)
            if scalar == "#" { doubled.append(scalar) }
        }
        return tmuxCommandStringLiteral(String(doubled))
    }

    /// A session name as one word of a SHELL command line, for the commands
    /// that make a session outside control mode (`tmux -CC new -s`, `tmux
    /// new-session -d -s` over an exec): single-quoted, every `#` doubled —
    /// tmux expands the name as a format there too (tmux 3.6a, private
    /// socket: `-s 'a#{session_id}b'` is stored as `ab`, `c##d` as `c#d`).
    nonisolated static func tmuxNameShellWord(_ name: String) -> String {
        var doubled = String.UnicodeScalarView()
        for scalar in name.unicodeScalars {
            doubled.append(scalar)
            if scalar == "#" { doubled.append(scalar) }
        }
        return Shell.quote(String(doubled))
    }

    /// Rename a session, then re-list so the snapshot picks up the new name.
    /// No-op on an empty name (tmux would reject it anyway).
    func renameSession(_ sessionId: String, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        // The target by its id, spliced bare: an id or nothing.
        guard !trimmed.isEmpty, Self.isValidTmuxId(sessionId) else { return }
        if let refusal = MultiplexerName.refusal(trimmed) {
            flashNotice(refusal)
            return
        }
        sendSurfacingErrors("rename-session -t \(sessionId) -- \(Self.tmuxNameArgument(trimmed))", verb: "rename")
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-sessions") else { return }
            self.parseListSessions(rows)
        }
    }

    /// Kill a session and re-list. tmux itself guards the last session (the
    /// server exits, which surfaces as the control channel closing); we don't
    /// block it here so the user keeps full control of their own sessions.
    func killSession(_ sessionId: String) {
        guard Self.isValidTmuxId(sessionId) else { return }
        sendSurfacingErrors("kill-session -t \(sessionId)", verb: "kill session")
        // Prune locally first: if this was the last session the server exits and
        // the follow-up list-sessions never returns, so the tree must update
        // from our side. If others remain, the re-list (and the %session-changed
        // tmux sends when it switches the client) reconcile it.
        snapshot.sessions[sessionId] = nil
        // Drop the killed session's windows/panes from the local snapshot so a
        // surviving session never shows the dead one's windows while the re-list
        // is in flight.
        for win in snapshot.windows.values where win.sessionId == sessionId {
            snapshot.windows.removeValue(forKey: win.id)
            for pane in snapshot.panes.values where pane.windowId == win.id {
                snapshot.panes.removeValue(forKey: pane.id)
            }
        }
        if snapshot.activeSessionId == sessionId {
            snapshot.activeSessionId = snapshot.sessions.keys.min(by: TmuxSnapshot.idPrecedes)
        }
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-sessions") else { return }
            self.parseListSessions(rows)
        }
        // Re-list windows/panes for every surviving session so the tree is
        // correct even when tmux switched our client to another session.
        refreshWindowsAndPanes()
    }

    /// Rename a window, then re-run window discovery so the tree updates.
    /// (`%window-renamed` also fires, but the explicit re-list covers the
    /// SSH-exec sidecar path that has no live notifications.)
    func renameWindow(_ windowId: String, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, Self.isValidTmuxId(windowId) else { return }
        if let refusal = MultiplexerName.refusal(trimmed) {
            flashNotice(refusal)
            return
        }
        sendSurfacingErrors("rename-window -t \(windowId) -- \(Self.tmuxNameArgument(trimmed))", verb: "rename")
        refreshWindowsAndPanes()
    }

    /// Kill a window and re-discover. Local snapshot is also pruned by
    /// `handleWindowClose` on the `%window-close` notification.
    func killWindow(_ windowId: String) {
        guard Self.isValidTmuxId(windowId) else { return }
        sendSurfacingErrors("kill-window -t \(windowId)", verb: "kill window")
        refreshWindowsAndPanes()
    }

    /// Kill a single pane and re-discover. Panes have no user-facing name in
    /// tmux, so there is no rename counterpart.
    func killPane(_ paneId: String) {
        guard Self.isValidTmuxId(paneId) else { return }
        sendSurfacingErrors("kill-pane -t \(paneId)", verb: "kill pane")
        refreshWindowsAndPanes()
    }

    /// Re-run window/pane discovery (after session switches etc.).
    ///
    /// `isFreshAttach`: this pass follows a genuine (re)attach rather than a
    /// routine re-discovery — see ``ensureTerminalsForAllPanes(isFreshAttach:)``.
    /// `completion` runs on the main actor once the pane list has been parsed
    /// and the snapshot is fully repopulated.
    private func refreshWindowsAndPanes(isFreshAttach: Bool = false,
                                        then completion: (() -> Void)? = nil) {
        sendCommand("list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} #{window_active} #{window_panes} #{window_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-windows") else { return }
            self.parseListWindows(rows)
        }
        // `alternate_on` and `pane_in_mode` ride along here, ahead of the
        // command (which is last because it can contain spaces). They used to be
        // fetched per pane by `backfill`, with a `display-message` whose only
        // purpose was to decide the capture range — a WHOLE extra round-trip
        // depth before any pane could be filled, measured on a real attach with
        // `-MOSHPIT_CC_TAP`. list-panes already runs and already round-trips;
        // two more format fields on it cost nothing.
        sendCommand("list-panes -a -F \(Self.paneDiscoveryFormat)") { [weak self] response in
            guard let self else { return }
            if let rows = self.listing(response, of: "list-panes") {
                self.parseListPanes(rows)
                self.ensureTerminalsForAllPanes(isFreshAttach: isFreshAttach)
                self.applyPendingRestoreIfReady()
            }
            completion?()
        }
    }

    /// After a reconnect, replay the user's prior selection once discovery has
    /// repopulated the ids. ``selectPane`` restores session+window+pane in one
    /// call (it switches session/window if the pane lives elsewhere); fall back
    /// to the window, then the session, if the finer target is gone. One-shot.
    private func applyPendingRestoreIfReady() {
        guard let restore = pendingRestore, snapshot.isAttached,
              !snapshot.sessions.isEmpty else { return }
        Log.island.info("land: restore pane=\(restore.pane ?? "-", privacy: .public) window=\(restore.window ?? "-", privacy: .public) present=\(restore.pane.map { self.snapshot.panes[$0] != nil } ?? false, privacy: .public)")
        if let pane = restore.pane, snapshot.panes[pane] != nil {
            pendingRestore = nil
            selectPane(pane)
        } else if let window = restore.window, snapshot.windows[window] != nil {
            pendingRestore = nil
            selectWindow(window)
        } else if snapshot.sessions[restore.session] != nil {
            pendingRestore = nil
            selectSession(restore.session)
        } else if !snapshot.panes.isEmpty {
            // Discovery has run but the targets are gone — give up gracefully.
            pendingRestore = nil
        }
    }

    /// Forward keyboard input from the focused pane to tmux. Uses
    /// `send-keys -H <hex>` so control bytes and binary data round-trip
    /// untouched — except a lone arrow key, which goes as tmux's own key
    /// name so tmux encodes it (see the comment in the body).
    ///
    /// `paneId` isn't always sourced from `snapshot` — `SessionHub
    /// .deliverAgentInput` (the Vibe Island lock-screen Allow/Deny/Reply
    /// action, and the `AgentControlIntent` App Intent behind it) passes a
    /// caller-supplied pane id straight through. Validate it here rather
    /// than assume: it's the one call site where the id could be something
    /// other than tmux's own `%<digits>` shape.
    func sendInput(_ data: Data, paneId: String) {
        guard !data.isEmpty, Self.isValidTmuxId(paneId) else { return }
        // A real keystroke snaps a scrolled-up reader back to the live bottom
        // first (releasing the local scroll-hold replays everything buffered),
        // so the echo of what they type is actually visible.
        paneCoordinators[paneId]?.releaseScrollHold()
        if snapshot.panes[paneId]?.windowId == snapshot.activeWindowId { noteUserInteraction() }
        // A lone arrow (either DECCKM variant) goes as a key NAME, which tmux
        // encodes against the pane's REAL cursor-key mode: `ESC O A` while
        // the pane is in application mode, `ESC [ A` otherwise. Whoever sent
        // these bytes chose the variant from the LOCAL emulator's flag, and
        // that flag is only ever learned by watching the pane's `ESC[?1h`
        // go by — which a fresh -CC attach never replays, and which
        // `handlePaneOutput` drops while our pins are handed back (Home,
        // background). The capture that repaints the screen afterwards
        // carries no modes. So a window opened from Home came up with the
        // flag stuck at normal mode, the D-pad sent `ESC [ A`, and
        // oh-my-zsh — which binds its prefix history search to `kcuu1`, the
        // application-mode sequence — recalled the previous entry instead
        // of completing the typed line (user report, 2026-10-18). Same
        // reasoning as `sendPaste` for bracketed paste.
        if let key = SessionHub.ActiveSession.arrowKeyName(data) {
            send(rawCommand: "send-keys -t \(paneId) \(key.capitalized)")
            return
        }
        let hex = data.map { String(format: "%02x", $0) }.joined(separator: " ")
        send(rawCommand: "send-keys -t \(paneId) -H \(hex)")
    }

    /// Paste `text` into `paneId` by round-tripping it through a tmux paste
    /// buffer instead of bracket-wrapping it ourselves and forwarding raw
    /// bytes through ``sendInput(_:paneId:)``.
    ///
    /// Claude Code only turns a pasted path into `[Image #N]` when it arrives
    /// wrapped in bracketed-paste markers (`ESC[200~ … ESC[201~`); bare bytes
    /// read as hand-typed text. Whether to wrap is the pane's OWN DECSET 2004
    /// state, which this controller has no reliable way to read locally: the
    /// local SwiftTerm's `bracketedPasteMode` flag only ever gets set by
    /// OBSERVING that escape go by, and tmux does not replay mode-setting
    /// sequences on a fresh `-CC` attach (lab-verified, `scripts/spikes/tmux-cc-lab`,
    /// 2026-08-19) — nor is there a format to read it instead:
    /// `#{pane_bracket_paste}` doesn't exist in tmux 3.6a (checked live; an
    /// unknown format silently expands to empty rather than erroring, so
    /// this was confirmed, not assumed off the man page). `paste-buffer -p`
    /// asks tmux to make the call itself, from the state it already tracks
    /// server-side (`man tmux`: "paste bracket control codes are inserted
    /// around the buffer if the application has requested bracketed paste
    /// mode") — verified live against tmux 3.6a on a private `-L` lab
    /// socket: a pane with 2004 on received a multi-line payload wrapped
    /// exactly once around the whole block, newlines intact; the identical
    /// paste into a plain-shell pane arrived bare.
    ///
    /// One fixed buffer name is safe to reuse: `set-buffer` and `paste-buffer
    /// -d` below are two back-to-back ``send(rawCommand:)`` calls with no
    /// `await` between them, so on this controller's single write chain no
    /// other command can land in between and no other call can observe the
    /// buffer mid-use.
    ///
    /// tmux brackets the buffer as it is: a paste marker inside it reaches
    /// the pane untouched, and an `ESC[201~` there ends the paste early — the
    /// line after it ran in zsh (tmux 3.6a, private socket). So the markers
    /// come out of the text first (``BracketedPaste/sanitized(_:)``).
    func sendPaste(_ text: String, paneId: String) {
        let text = BracketedPaste.sanitized(text)
        guard !text.isEmpty, Self.isValidTmuxId(paneId) else { return }
        let literal = Self.tmuxCommandStringLiteral(text)
        send(rawCommand: "set-buffer -b moshpit-paste -- \(literal)")
        send(rawCommand: "paste-buffer -p -b moshpit-paste -d -t \(paneId)")
        if snapshot.panes[paneId]?.windowId == snapshot.activeWindowId { noteUserInteraction() }
    }

    /// ``sendPaste(_:paneId:)``, answered: true once tmux has pasted `text`
    /// into `paneId`. False when nothing went — not attached yet (commands
    /// before the attach are dropped, see `enqueue`), a pane tmux doesn't
    /// know, an error, the channel closing first, or no answer within
    /// `timeout`. For a sender that keeps the text until it is delivered: the
    /// share queue deleted its entry on a paste dropped like that (B172).
    func pasteConfirmed(_ text: String, paneId: String, timeout: TimeInterval = 8) async -> Bool {
        let text = BracketedPaste.sanitized(text)
        guard !text.isEmpty, Self.isValidTmuxId(paneId), snapshot.isAttached else { return false }
        let literal = Self.tmuxCommandStringLiteral(text)
        let answer: Bool? = await withTimeoutValue(timeout) { [weak self] in
            await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
                Task { @MainActor [weak self] in
                    guard let self, self.snapshot.isAttached else {
                        continuation.resume(returning: false)
                        return
                    }
                    // The same two commands as sendPaste, back to back on the
                    // one write chain; the paste's own reply is the answer (a
                    // buffer that failed to set leaves nothing to paste).
                    self.send(rawCommand: "set-buffer -b moshpit-paste -- \(literal)")
                    self.sendCommand("paste-buffer -p -b moshpit-paste -d -t \(paneId)", waits: true) { response in
                        continuation.resume(returning: !response.isError)
                    }
                    if self.snapshot.panes[paneId]?.windowId == self.snapshot.activeWindowId {
                        self.noteUserInteraction()
                    }
                }
            }
        }
        return answer ?? false
    }

    /// Tell tmux the client viewport changed size. Driven by the container
    /// view; tmux resizes each visible pane to fit.
    ///
    /// DEBOUNCED. A keyboard show/hide or an IME switch (the Chinese candidate
    /// bar is taller than the English keyboard) animates the view through
    /// several intermediate heights, each firing `sizeChanged`. Forwarding the
    /// whole storm makes tmux resize the pane repeatedly; the app's redraw for
    /// size N arrives while the local terminal is already at size N+1, so rows
    /// land shifted — box borders split, characters spill ("字符溢出"). Worse,
    /// SwiftTerm reflows its local buffer on every resize with xterm.js
    /// semantics while tmux resizes its pane model with tmux semantics, so the
    /// two diverge — and a diffing renderer (Claude Code) then patches against
    /// its own model, so the garble never self-corrects. Collapse the storm to
    /// the final size, then `resyncActivePane()` repaints from tmux's
    /// authoritative screen so any divergence is erased.
    @discardableResult
    func resizeClient(rows: Int, cols: Int) -> Bool {
        // The dedupe must not swallow the FIRST report from a real terminal
        // view. Until one arrives, `lastClientSize` is only a GUESS
        // (``setInitialClientSize``, from an estimated grid), and a guess that
        // happens to be right looked exactly like "nothing changed" — so
        // `commitClientSize()` never ran for the initial layout: no
        // `refresh-client`, no window pin, and above all no `resyncActivePane`.
        // Anything painted before the view was laid out therefore stood
        // uncorrected until the user changed the layout themselves, which is
        // what "the tmux screen doesn't render until I tap to raise the
        // keyboard" was — the tap resized the grid, which finally differed and
        // repainted. A wrong guess self-healed (the sizes differ), so this only
        // ever bit the phones whose estimate was exact.
        guard (cols, rows) != lastClientSize || !clientSizeConfirmed else { return false }
        clientSizeConfirmed = true
        lastClientSize = (cols, rows)
        pendingResize?.cancel()
        pendingResize = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled else { return }
            self?.commitClientSize()
        }
        return true
    }

    /// Apply the settled client size: one `refresh-client -C`, re-pin the
    /// visible window (covers the keyboard show/hide resize too), then repaint
    /// the visible pane from tmux's model (see `resizeClient`).
    ///
    /// The resync runs TWICE: once immediately (kills the local-reflow garble
    /// right away) and once after a grace period. The immediate capture races
    /// the app's own SIGWINCH redraw — a diff-rendering TUI (Claude Code) can
    /// take hundreds of ms to repaint after the resize, and a frame captured
    /// mid-redraw is faithfully wrong (over a real connection's RTT, this is
    /// not just theoretical: it reproduces as a beat of duplicated or blank
    /// content flashing on screen — the keyboard-freeze cover, expected to
    /// hide exactly this, was being lifted by the immediate pass's own
    /// completion). So only the immediate pass's FRAME FEED runs now; it does
    /// NOT reveal. It still silently corrects the buffer under the cover. The
    /// second, settled pass captures the (much more likely correct) settled
    /// screen and is the one that actually reveals — same-content repaints
    /// are invisible (single synchronous feed), so revealing later costs
    /// nothing when the immediate capture already happened to be right.
    private func commitClientSize() {
        send(rawCommand: "refresh-client -C \(lastClientSize.cols)x\(lastClientSize.rows)")
        if !pinsReleased, let win = snapshot.activeWindowId {
            requestPin(win, reason: .resize)
        }
        // 334-model: our own resize's in-flight bytes paint at the previous
        // grid's layout (briefly mis-wrapped) and the settling double-pass +
        // convergence capture repaint — no gate (see handlePaneOutput).
        resyncActivePaneSettling()
    }

    /// The two-pass repaint from ``commitClientSize()``'s doc, shared with the
    /// window/pane switch paths — which issue the very same
    /// `resize-window`/`resize-pane -Z` and therefore race the very same
    /// SIGWINCH redraw, but used to get a single immediate `reveal: true`
    /// capture: over a weak network that capture is reliably mid-redraw, and
    /// with no settled pass behind it the garble stood until %output slowly
    /// caught up ("切 window 先乱几秒再自己好").
    private func resyncActivePaneSettling() {
        // Every settling disruption (resize, war repair, window/pane switch)
        // ends with a convergence capture once the pane app's own redraw has
        // drained — see `armConvergenceResync`.
        armConvergenceResync()
        resyncActivePane(reveal: false)
        // The settled pass lands a debounce PLUS a full round trip from now —
        // give an active keyboard-freeze/pane-switch cover enough rope to
        // survive until then instead of its default 0.8s cap, which a real
        // connection's RTT can easily outlast (see extendCoverTimeout's doc
        // and resyncPane's `reveal` doc for what happens when it doesn't).
        extendActiveCoverTimeout(by: coverGrace())
        pendingSettleResync?.cancel()
        pendingSettleResync = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(700))
            guard !Task.isCancelled else { return }
            self?.extendActiveCoverTimeout(by: self?.coverGrace() ?? 2.0)
            self?.resyncActivePane()
        }
    }

    /// Round trip of the most recent resync capture, measured send→frame-fed.
    /// The veil timeouts below scale from it: a fixed cap tuned on a fast
    /// link expires while the weak-network frame is still in flight — the
    /// cover lifts exactly onto the mid-redraw garble it existed to hide.
    @ObservationIgnored private var lastResyncRoundTrip: TimeInterval = 0

    /// How long a cover may outlive its default cap while a resync is in
    /// flight: generous against the measured round trip, floored at the old
    /// fixed 2s for the first capture, ceilinged so a dead connection can't
    /// pin a veil to the screen indefinitely (the frame's own arrival is
    /// what normally lifts it — this is only the safety cap).
    private func coverGrace() -> TimeInterval {
        min(max(2.0, lastResyncRoundTrip * 1.5 + 0.5), 8.0)
    }

    /// See ``commitClientSize()``. Resolves the same "active pane" as
    /// ``resyncActivePane(reveal:)`` so the extension always targets the
    /// pane the upcoming resync will actually reveal.
    private func extendActiveCoverTimeout(by seconds: TimeInterval) {
        guard let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        paneCoordinators[paneId]?.extendCoverTimeout(by: seconds)
    }

    @ObservationIgnored
    private var pendingResize: Task<Void, Never>?

    @ObservationIgnored
    private var pendingSettleResync: Task<Void, Never>?

    @ObservationIgnored
    private var lastClientSize: (cols: Int, rows: Int) = (80, 24)

    /// Whether a real terminal view has reported its grid yet, as opposed to
    /// ``setInitialClientSize``'s estimate. Gates the first
    /// ``resizeClient(rows:cols:)`` through even when the estimate turns out to
    /// be exactly right — see the guard there for what silently broke without
    /// it.
    @ObservationIgnored
    private var clientSizeConfirmed = false

    /// Seed the client size before control mode starts, so the very first
    /// `refresh-client -C` carries a phone-sized grid instead of 80×24 —
    /// otherwise programs render wide and a late resize can't reflow
    /// already-emitted lines (worse the higher the link's RTT).
    func setInitialClientSize(cols: Int, rows: Int) {
        lastClientSize = (cols, rows)
        clientSizeConfirmed = false   // this is a guess, not a view's report
    }

    /// Whether a pane is on the alternate screen, and whether it is sitting in
    /// copy-mode. Both are read from the discovery `list-panes`, which already
    /// round-trips, rather than probed per pane — see ``sendInitialDiscovery``.
    struct ScreenState { var isAlternate: Bool; var isInCopyMode: Bool }

    @ObservationIgnored private var paneScreenState: [String: ScreenState] = [:]

    /// Panes whose pre-attach screen contents have been replayed into their
    /// terminal views via `capture-pane`.
    @ObservationIgnored private var backfilledPanes: Set<String> = []

    /// How many lines of pre-attach scrollback to replay into a pane on first
    /// sight. Kept deliberately small: every line is captured over the wire and
    /// fed through SwiftTerm's parser on the main actor, so a deep backfill
    /// stalls window switches into panes with long history (a 3 000-line
    /// node/Claude Code pane) — the gap a desktop tmux never has. 400 fills
    /// several screens of scroll-up instantly; deeper history is a future
    /// scroll-to-top on-demand fetch. tmux still keeps 50 000 lines server-side.
    ///
    /// The paragraph above was already here, arguing for 400, while the constant
    /// said 2 000 — the reasoning had been written down and never applied.
    /// Measured on a 4-pane session with 3 000 lines each: 2 000 costs ~20 KB
    /// per pane on the wire and the same again through the parser, for
    /// scrollback nobody has scrolled to yet.
    @ObservationIgnored
    private let backfillHistoryLines = 400

    /// Replay a pane's existing scrollback (everything printed before we
    /// attached) into its terminal. `%output` only carries *new* bytes, so
    /// without this a freshly-attached or freshly-switched pane would start
    /// blank above the cursor. `-S -N` reaches into tmux history; `-e` keeps
    /// colours. Trailing blank padding lines are trimmed so the prompt isn't
    /// pushed off-screen.
    private func backfill(paneId: String) {
        guard !backfilledPanes.contains(paneId) else { return }
        // Only claim the pane once the commands can actually be sent —
        // `enqueue` silently drops everything pre-attach, and a dropped
        // command means no reply, which would leave `backfillsInFlight`
        // stuck and mute every later resync for this pane.
        guard snapshot.isAttached else { return }
        backfilledPanes.insert(paneId)
        // Full-screen TUIs on the alternate screen (vim, top, Claude Code)
        // are special: tmux does NOT re-emit an already-drawn TUI on attach,
        // so we must capture *something* or the pane is black — but replaying
        // SCROLLBACK history into a TUI corrupts it (a capture-pane text dump
        // has no cursor positioning, and the stray lines bleed through on
        // resize). Compromise: alternate panes get only the current visible
        // screen (one frame, minimal pollution); primary-screen panes get the
        // full scrollback so scroll-up works.
        // NOTE: an attempt to hold this until `clientSizeConfirmed` (so the
        // capture is laid out for the view's real grid instead of
        // `setInitialClientSize`'s estimate) was measured and backed out. It is
        // right in principle — a `-MOSHPIT_CC_TAP` trace shows the whole
        // backfill captured at a guessed 80x53, then a `resize-window 76x68`
        // right behind it — but it moves the dump from "just after discovery" to
        // "just after the first size commit", which lands in the middle of every
        // resize/cover test and re-orders a sequence six of them pin in detail.
        // The measured payoff was smaller than the projection: the trace showed
        // no `frameExceedsWidth` rejection, so nothing was actually re-fetched.
        // Worth revisiting behind an explicit ordering test, not as a drive-by.
        beginBackfillCapture(paneId)
    }

    private func beginBackfillCapture(_ paneId: String) {
        backfillsInFlight.insert(paneId)
        // Safety net for that hold: if the dump's reply never lands (a desynced
        // response FIFO, a connection dying mid-command), release it anyway. A
        // resync fed slightly out of order costs one repaint; a hold that never
        // lifts would mute this pane's repaints for the rest of the connection.
        backfillTimeouts[paneId]?.cancel()
        backfillTimeouts[paneId] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(5))
            guard !Task.isCancelled else { return }
            self?.finishBackfill(paneId)
        }
        // The screen state comes from the discovery `list-panes`, which has
        // already round-tripped. Probing per pane cost a whole extra depth
        // before anything could be filled — visible in a `-MOSHPIT_CC_TAP`
        // trace as four `display-message` sends sitting between discovery and
        // the first `capture-pane`.
        //
        // The probe survives as a fallback for a pane discovery has not
        // described yet (one that appeared via `%window-add` between passes),
        // because guessing "primary screen" for an alternate-screen TUI would
        // replay scrollback into it and corrupt the frame.
        if let state = paneScreenState[paneId] {
            issueBackfillCapture(paneId: paneId, state: state)
        } else {
            sendCommand("display-message -p -t \(paneId) '#{alternate_on} #{pane_in_mode}'") { [weak self] resp in
                guard let self else { return }
                let fields = resp.lines.first?.tmuxFields(separator: " ") ?? []
                let state = ScreenState(isAlternate: fields.first == "1",
                                        isInCopyMode: fields.count >= 2 && fields[1] == "1")
                self.paneScreenState[paneId] = state
                self.issueBackfillCapture(paneId: paneId, state: state)
            }
        }
    }

    /// Send the one `capture-pane` that fills a pane, with the range its screen
    /// state calls for.
    ///
    /// Full-screen TUIs on the alternate screen (vim, top, Claude Code) are
    /// special: tmux does NOT re-emit an already-drawn TUI on attach, so we must
    /// capture *something* or the pane is black — but replaying SCROLLBACK
    /// history into a TUI corrupts it (a capture-pane text dump has no cursor
    /// positioning, and the stray lines bleed through on resize). Compromise:
    /// alternate panes get only the current visible screen (one frame, minimal
    /// pollution); primary-screen panes get the scrollback so scroll-up works.
    private func issueBackfillCapture(paneId: String, state: ScreenState) {
        // A pane still in copy-mode on FIRST sight after attach is a stale
        // leftover — our previous connection died mid-scroll and nobody sent the
        // exit. The scroll POSITION survives server-side, so the user's first
        // post-reconnect swipe would continue copy-mode from that old offset
        // (way up in history — the "reconnect scrolls to the top" bug). Cancel
        // it so the reconnect starts from a clean live view. (Once per pane per
        // attach, so it cannot kick another client's deliberate reading beyond
        // this one moment.)
        if state.isInCopyMode {
            send(rawCommand: "send-keys -t \(paneId) -X cancel")
        }
        // Primary-screen panes stop at the scrollback ABOVE the visible screen
        // (`-E -1`), because the visible screen itself arrives from the
        // fresh-attach resync in ``ensureTerminalsForAllPanes`` — capturing it
        // here too draws it twice, leaving a phantom extra prompt that looked
        // like a stray Return.
        //
        // Do NOT reintroduce "tmux repaints the visible screen on attach" here.
        // Measured against tmux 3.6a with a raw control client: a `-CC attach`
        // emits no `%output` at all, and neither does a `refresh-client -C` that
        // names the size the window already has. tmux only repaints when the
        // size actually CHANGES — which is why the app's own resync is what
        // paints a fresh attach.
        let range = state.isAlternate ? "" : "-S -\(backfillHistoryLines) -E -1"
        sendCommand("capture-pane -p -e \(range) -t \(paneId)") { [weak self] response in
            guard let self else { return }
            Log.ssh.debug("resume: backfill reply for \(paneId, privacy: .public) (\(response.lines.count, privacy: .public) lines)")
            // Whatever this reply turns out to hold, the dump is over —
            // release any resync that was waiting behind it.
            defer { self.finishBackfill(paneId) }
            var lines = response.lines
            while let last = lines.last, last.trimmingCharacters(in: .whitespaces).isEmpty {
                lines.removeLast()
            }
            guard !lines.isEmpty else { return }
            // A dump captured while the pane sits at ANOTHER client's
            // width (真机取证 2026-08-19: 62-row desktop frames fed
            // straight into background panes) is laid out for a grid
            // this terminal doesn't have — feeding it wraps every line
            // and seeds the local scrollback with permanent garble.
            // Skip it and un-claim the pane so a later discovery pass
            // retries once the sizes settle; the parked resync (which
            // validates its own frame) still runs via the defer.
            guard !Self.frameExceedsWidth(lines, cols: self.lastClientSize.cols) else {
                self.ccTapLine("BACKFILL-REJECT pane=\(paneId) lines=\(lines.count) "
                    + "ours=\(self.lastClientSize.rows)x\(self.lastClientSize.cols)")
                self.backfilledPanes.remove(paneId)
                self.scheduleBackfillRetry(paneId)
                return
            }
            // A retried dump lands AFTER the live screen was painted by the
            // attach's own resync, so its history scrolls that screen away.
            // Ask for the repaint the first dump's ordering got for free:
            // `finishBackfill` (the defer above) runs whatever is parked here
            // once the dump has been fed, frame on top of history.
            if (self.backfillRejectStreak[paneId] ?? 0) > 0 {
                self.backfillRejectStreak[paneId] = 0
                self.deferredResyncs[paneId] = true
            }
            // History is followed by %output's visible repaint, which
            // repositions the cursor — terminate with CRLF so the two
            // don't run together, but add nothing after a bare frame.
            let text = lines.joined(separator: "\r\n") + (state.isAlternate ? "" : "\r\n")
            self.paneCoordinators[paneId]?.feed(data: Data(text.utf8))
        }
    }

    /// Panes whose ``backfill`` scrollback dump is still in flight, and the
    /// resyncs parked behind them (`paneId` → that resync's `reveal`).
    ///
    /// ``backfill``'s dump lands a round trip after it is asked for, and
    /// anything a caller queues in between — `applyPendingRestoreIfReady()` →
    /// `selectPane()` → `resyncPane()`, or the fresh-attach repaint — is
    /// therefore fed BEFORE the dump, and the dump's ~2 000 lines of history
    /// then scroll the authoritative frame straight off the screen. What's
    /// left on screen is the tail of the scrollback: the last thing the user
    /// typed before the connection dropped, with the live viewport nowhere in
    /// sight ("重连出现上次输入的内容"). Every keystroke after that lands
    /// against a screen tmux disagrees with, so the app's diff-based redraws
    /// patch the wrong cells and editing keys look dead. Park the resync and
    /// run it once the dump has landed.
    @ObservationIgnored private var backfillsInFlight: Set<String> = []
    @ObservationIgnored private var deferredResyncs: [String: Bool] = [:]
    @ObservationIgnored private var backfillTimeouts: [String: Task<Void, Never>] = [:]

    /// Dumps rejected for another client's width, per pane, and the retries
    /// waiting on them — see ``scheduleBackfillRetry``.
    @ObservationIgnored private var backfillRejectStreak: [String: Int] = [:]
    @ObservationIgnored private var backfillRetryTasks: [String: Task<Void, Never>] = [:]

    /// A backfill dump came back laid out for a width this grid doesn't have:
    /// the capture ran while tmux still held the window at
    /// `setInitialClientSize`'s guess (or a desktop client's size), with our
    /// `resize-window` landing right behind it — a `-MOSHPIT_CC_TAP` trace of a
    /// fresh attach shows 353 history lines wrapped at 80 columns arriving for
    /// a 76-column view. Un-claiming the pane on its own promised a retry
    /// "once a later discovery pass runs", but discovery never runs again on
    /// its own after attach, so the pane simply had no local history: the
    /// first swipe over anything printed before the attach moved nothing.
    /// Recapture after the sizes have had a settle's worth of time, bounded
    /// the same way as `scheduleResyncRetry` so a size war can't loop it.
    private func scheduleBackfillRetry(_ paneId: String) {
        let streak = (backfillRejectStreak[paneId] ?? 0) + 1
        backfillRejectStreak[paneId] = streak
        guard streak <= 4 else { return }
        backfillRetryTasks[paneId]?.cancel()
        backfillRetryTasks[paneId] = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(700))
            guard !Task.isCancelled, let self else { return }
            self.backfillRetryTasks.removeValue(forKey: paneId)
            self.backfill(paneId: paneId)
        }
    }

    /// A backfill's dump has landed (or its safety timeout fired): drop the
    /// hold and run whatever resync was parked behind it.
    private func finishBackfill(_ paneId: String) {
        backfillTimeouts.removeValue(forKey: paneId)?.cancel()
        guard backfillsInFlight.remove(paneId) != nil,
              let reveal = deferredResyncs.removeValue(forKey: paneId) else { return }
        // The wait cost an extra round trip — re-arm the veil's safety cap
        // from NOW so it can't expire mid-resync and flash the dump we're
        // covering. Over a weak network the 2 000-line dump routinely
        // outlives the original cap entirely, and extendCoverTimeout on an
        // already-lifted cover is a silent no-op — so if the veil is gone,
        // raise it again first: without one, the dump plus resync frame play
        // out in the open ("重连出现上次输入的内容" in slow motion).
        if let coordinator = paneCoordinators[paneId] {
            if !coordinator.isCoverPresented { coordinator.veilForSwitch() }
            coordinator.extendCoverTimeout(by: coverGrace())
        }
        resyncPane(paneId, reveal: reveal)
    }

    /// Repaint the active pane's local screen from tmux's authoritative model.
    ///
    /// `%output` only carries *new* app bytes — tmux never reconciles our local
    /// screen. So whenever the local terminal's state diverges from tmux's pane
    /// model (SwiftTerm reflows on resize with xterm.js semantics, tmux resizes
    /// with its own; a hidden pane's buffer can go stale), nothing ever corrects
    /// it: a diffing TUI patches relative to ITS model and the garble persists
    /// until a manual refresh. This is that refresh, done automatically: capture
    /// tmux's current visible screen and repaint from it.
    ///
    /// Race-free by construction: the -CC stream is a single ordered channel, so
    /// every `%output` emitted before our capture reply is already applied (and
    /// about to be overwritten by the frame, which includes its effect), and
    /// every one after corresponds to app writes after the capture point —
    /// applying them on top of the captured frame is exactly a terminal state
    /// machine resuming from a correct snapshot.
    ///
    /// `reveal`: whether THIS capture, once fed, is allowed to lift a
    /// keyboard-freeze/pane-switch cover. Pass `false` for a capture that's
    /// known to race the app's own redraw (see `commitClientSize`) — the
    /// frame still corrects the buffer, it just doesn't get shown until a
    /// later, more trustworthy capture (or the cover's safety timeout) says
    /// so. Defaults to `true`, matching every call site except that one.
    func resyncActivePane(reveal: Bool = true) {
        guard rendersOutput, snapshot.isAttached,
              let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        resyncPane(paneId, reveal: reveal)
    }

    private func resyncPane(_ paneId: String, reveal: Bool = true) {
        // Never race this pane's own backfill — see `backfillsInFlight`. A
        // parked request coalesces with any earlier one, and a `reveal: true`
        // request wins: the single capture that eventually runs is the only
        // thing left to lift the cover.
        if backfillsInFlight.contains(paneId) {
            deferredResyncs[paneId] = reveal || (deferredResyncs[paneId] ?? false)
            return
        }
        // One capture in flight per pane, and at most one queued behind it.
        //
        // Every disruption — a resize, a window switch, a war repair, the
        // convergence arm — calls in here, and a burst of them used to mean a
        // burst of `capture-pane` + cursor pairs: a `-MOSHPIT_CC_TAP` trace of a
        // single attach showed the active pane captured TEN times, each a full
        // round trip carrying a whole screen. They were redundant by
        // construction: the last frame to arrive is the only one that survives
        // on screen, so the ones before it paid for a repaint nobody saw.
        //
        // Coalescing rather than time-debouncing on purpose. A fixed timer has
        // to be tuned for a link, and the links this app runs on differ by an
        // order of magnitude; an in-flight gate adapts by itself — a fast link
        // barely coalesces, a slow one collapses the whole burst into the one
        // capture that was going to matter anyway.
        //
        // The two-pass settle (`resyncActivePaneSettling`) is unaffected: its
        // second pass is scheduled well after the first has landed, which is the
        // point of it — it exists to catch the app's own SIGWINCH redraw.
        if resyncsInFlight.contains(paneId) {
            queuedResyncs[paneId] = reveal || (queuedResyncs[paneId] ?? false)
            return
        }
        resyncsInFlight.insert(paneId)
        // Safety net, same shape as the backfill's: a connection that dies
        // between the capture and its cursor reply would otherwise leave this
        // pane permanently "in flight", and every later resync would coalesce
        // into a follow-up that never runs — a pane that silently stops
        // repainting for the rest of the session.
        resyncTimeouts[paneId]?.cancel()
        resyncTimeouts[paneId] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(10))
            guard !Task.isCancelled else { return }
            self?.finishResync(paneId)
        }
        // Frame FIRST, cursor SECOND, fed in two steps — the order matters.
        // The pipeline is stream-ordered, so any %output between the two
        // replies is (a) applied AFTER the frame feed, at its correct place on
        // the fresh frame, and (b) already reflected in the later cursor
        // reply. Nothing is lost and the final CUP is exact. (The previous
        // cursor-first order pinned the caret at a PRE-redraw position: the
        // post-resize app repaint raced in between the two replies, so the
        // frame included it but the restored cursor predated it — the "cursor
        // drifts after an IME switch" bug.)
        // The capture's own round trip is the best live probe of what "the
        // network right now" means for every veil cap above (coverGrace) —
        // it rides the same FIFO the reveal-bearing frame does.
        let sentAt = ContinuousClock.now
        sendCommand("capture-pane -p -e -t \(paneId)") { [weak self] response in
            guard let self else { return }
            let elapsed = ContinuousClock.now - sentAt
            self.lastResyncRoundTrip = Double(elapsed.components.seconds)
                + Double(elapsed.components.attoseconds) / 1e18
            var lines = response.lines
            while let last = lines.last, last.trimmingCharacters(in: .whitespaces).isEmpty {
                lines.removeLast()
            }
            // A capture taken while the window sat at ANOTHER client's size is
            // authoritative for a grid this terminal doesn't have — feeding a
            // 62-row/499-column frame into a 46-row/70-column view wraps every
            // line sevenfold (phone byte capture, 2026-08-19: one of seven
            // resync frames was captured mid-flap at the desktop's size).
            // More rows than our grid is one tell (fewer is normal — trailing
            // blanks are trimmed above); a LINE wider than our columns is the
            // other, and it catches the rows-check's blind spot: a desktop
            // frame whose bottom rows are blank trims to fewer rows than ours
            // and sails through, then wraps sevenfold anyway. Drop it, re-pin,
            // and RETRY — rejecting without a follow-up capture starved the
            // repair whenever both settling passes hit mid-flap moments, and
            // the gate's dropped span then stayed missing until the app
            // happened to repaint those exact cells.
            guard lines.count <= self.lastClientSize.rows,
                  !Self.frameExceedsWidth(lines, cols: self.lastClientSize.cols) else {
                self.ccTapLine("FRAME-REJECT pane=\(paneId) rows=\(lines.count) "
                    + "ours=\(self.lastClientSize.rows)x\(self.lastClientSize.cols)")
                if let win = self.snapshot.activeWindowId { self.requestPin(win, reason: .resize) }
                self.scheduleResyncRetry(paneId, reveal: reveal)
                return
            }
            self.resyncRejectStreak[paneId] = 0
            // Home + erase-display (ED 2 leaves scrollback intact), the
            // frame, SGR reset. No cursor move yet — that's step two.
            let text = "\u{1b}[H\u{1b}[2J" + lines.joined(separator: "\r\n") + "\u{1b}[0m"
            self.ccTapLine("FRAME pane=\(paneId) reveal=\(reveal) lines=\(lines.count) "
                + "first=\(lines.first?.prefix(60) ?? "")")
            // A held resize and the capture that answers it land in ONE turn:
            // resize (which is where the buffer loses its bottom rows), then
            // paint tmux's full screen over the result, with nothing in
            // between ever drawn.
            self.releaseDeferredResize(paneId)
            self.paneCoordinators[paneId]?.feed(data: Data(text.utf8))
            // Once the frame is on screen — reveal the terminal if a keyboard-freeze
            // or pane-switch veil is covering it, UNLESS this capture is known
            // to race the app's own redraw (reveal == false): the buffer is
            // corrected either way, but showing THIS particular frame would
            // expose whatever the diffing TUI had drawn mid-repaint (a stray
            // duplicated or blank region) for the beat until the next capture
            // corrects it again — worse than staying covered a little longer.
            if reveal {
                self.paneCoordinators[paneId]?.revealWhenShown()
            }
        }
        sendCommand("display-message -p -t \(paneId) '#{cursor_x} #{cursor_y}'") { [weak self] resp in
            guard let self else { return }
            // The cursor is the SECOND half of the pair, so this is where the
            // resync is actually over.
            defer { self.finishResync(paneId) }
            let parts = resp.lines.first?.tmuxFields(separator: " ").compactMap { Int($0) } ?? []
            let (col, row) = parts.count == 2 ? (parts[0], parts[1]) : (0, 0)
            // CUP is 1-based; cursor_x/y are 0-based. In 128 bits: a reply the
            // FIFO paired with the wrong command can carry any number, and
            // `row + 1` on Int.max trapped (B142) — as moshpit-tmux formats it
            // (Fix (cursor-overflow)). SwiftTerm's parser bounds the digits.
            self.paneCoordinators[paneId]?.feed(
                data: Data("\u{1b}[\(Int128(row) + 1);\(Int128(col) + 1)H".utf8))
        }
    }

    /// Panes with a `capture-pane`+cursor pair on the wire, and the one request
    /// that arrived while it was there.
    @ObservationIgnored private var resyncsInFlight: Set<String> = []
    @ObservationIgnored private var queuedResyncs: [String: Bool] = [:]
    @ObservationIgnored private var resyncTimeouts: [String: Task<Void, Never>] = [:]

    private func finishResync(_ paneId: String) {
        resyncTimeouts.removeValue(forKey: paneId)?.cancel()
        guard resyncsInFlight.remove(paneId) != nil else { return }
        guard let reveal = queuedResyncs.removeValue(forKey: paneId) else { return }
        // Exactly one follow-up, however many arrived while we were waiting: the
        // frame it captures is current as of now, which is what every one of
        // them was asking for.
        resyncPane(paneId, reveal: reveal)
    }

    // MARK: - Agent hook bridge (Phase B)

    /// tmux `-F` format that reads every pane's `@moshpit_*` user options in ONE
    /// control command. An UNSET user option expands to an EMPTY string (tmux
    /// substitutes "" rather than erroring), so a never-stamped pane yields
    /// `"%5||||"`. `|` delimits five fixed fields:
    /// `pane_id | state | agent | since | command | title`. `@moshpit_title`
    /// may itself contain `|`, so the parser keeps the last field intact
    /// (maxSplits: 5).
    private static let agentHookFormat =
        "'#{pane_id}|#{@moshpit_state}|#{@moshpit_agent}|#{@moshpit_since}|#{pane_current_command}|#{@moshpit_title}'"

    /// Read every pane's hook stamp in a single cheap `list-panes -a` control
    /// command and rebuild ``agentHooks``. The monitor calls this on a timer.
    /// Skips the round-trip entirely when not attached (the same scope as the
    /// existing `list-panes -a` discovery — every session's panes are covered,
    /// so cross-session agents all appear). `enqueue` already drops pre-attach
    /// commands, but guarding here avoids the wasted command + callback slot.
    func pollAgentHooks() {
        guard snapshot.isAttached else { return }
        sendCommand("list-panes -a -F \(Self.agentHookFormat)") { [weak self] response in
            // Refused, it says nothing about the stamps: the monitor keeps
            // the last ones rather than reading every pane as gone.
            guard let self, let rows = self.listing(response, of: "list-panes (agent stamps)") else { return }
            self.parseAgentHooks(rows)
        }
    }

    /// Parse the `pane_id|state|agent|since|command|title` lines from
    /// ``pollAgentHooks`` and replace ``agentHooks`` wholesale. Empty fields
    /// (unset user options) become `nil`. Panes with an empty/unrecognised
    /// `@moshpit_state` are recorded with `state == nil` (NOT dropped) so the
    /// monitor can tell "pane has no hook data → fall back to heuristic" apart
    /// from "pane gone".
    private func parseAgentHooks(_ lines: [String]) {
        var parsed: [String: AgentHook] = [:]
        for line in lines {
            // omittingEmptySubsequences:false keeps empty fields in position;
            // maxSplits:5 preserves any literal `|` inside a user-set title.
            // Cut on bytes: an agent, command or title that starts with a
            // combining mark must not swallow the `|` in front of it.
            let parts = line.tmuxFields(separator: "|", maxSplits: 5,
                                        omittingEmptySubsequences: false)
            guard parts.count == 6, !parts[0].isEmpty else { continue }
            let paneId = parts[0]
            let stateRaw = parts[1]
            // Only the three hook-emitted states are valid; anything else
            // (empty, or a garbage value) means "no precise hook data".
            let state: String?
            switch stateRaw {
            case "working", "attention", "done": state = stateRaw
            default: state = nil
            }
            let agent = parts[2].isEmpty ? nil : parts[2]
            // since may be empty even with a state if a stamp partially failed;
            // garbage parses to nil too. Callers substitute Date() for nil.
            let since = TimeInterval(parts[3]).map { Date(timeIntervalSince1970: $0) }
            let command = parts[4].isEmpty ? nil : parts[4]
            let title = parts[5].isEmpty ? nil : parts[5]
            parsed[paneId] = AgentHook(state: state, agent: agent, since: since,
                                       command: command, title: title)
        }
        agentHooks = parsed
        agentHooksLoaded = true
        onAgentHooksUpdated?()
    }

    // MARK: - Wiring

    /// One parser event, in byte-stream order. The parser invokes its callbacks
    /// strictly in stream order, but each callback used to hop to the main
    /// actor in its OWN `Task { @MainActor }` — and Swift does NOT guarantee
    /// FIFO between independently-submitted tasks. Under load (a resize storm's
    /// %output flood racing a capture-pane reply) events ran out of order: a
    /// resync frame could be applied BEFORE %output that preceded it in the
    /// stream, smearing stale bytes over the fresh frame — and two %output
    /// chunks for the same pane could even swap. Funneling every event through
    /// ONE AsyncStream with ONE main-actor consumer restores end-to-end
    /// ordering; every ordering argument in this file rests on it.
    private enum ControlEvent: Sendable {
        case paneOutput(String, Data)
        case layoutChange(windowId: String, layout: String, zoomed: Bool)
        case windowAdd(String)
        case windowClose(String)
        case windowRenamed(windowId: String, name: String)
        case sessionChanged(sessionId: String, name: String)
        case sessionRenamed(sessionId: String, name: String)
        case sessionWindowChanged(sessionId: String, windowId: String)
        case sessionsChanged
        case unlinkedWindowAdd(String)
        case unlinkedWindowClose(String)
        case unlinkedWindowRenamed(windowId: String, name: String)
        case clientSessionChanged(client: String, sessionId: String, name: String)
        case activePaneChanged(windowId: String, paneId: String)
        case pause(String)
        case continued(String)
        case clientDetached(String)
        case exit
        case commandResponse(TmuxCommandResponse)
    }

    /// Single consumer of ``ControlEvent``s; cancelled on detach/re-attach.
    @ObservationIgnored private var eventPump: Task<Void, Never>?

    private func installCallbacks() async {
        eventPump?.cancel()
        let (events, emit) = AsyncStream<ControlEvent>.makeStream()
        // The client actor runs these synchronously while draining its byte
        // buffer, so yields happen in stream order; the single loop below
        // preserves it. Callbacks stay trivial — routing lives in handle(_:).
        await client.setCallbacks(
            onPaneOutput: { emit.yield(.paneOutput($0, $1)) },
            onLayoutChange: { emit.yield(.layoutChange(windowId: $0, layout: $1, zoomed: $2)) },
            onWindowAdd: { emit.yield(.windowAdd($0)) },
            onWindowClose: { emit.yield(.windowClose($0)) },
            onWindowRenamed: { emit.yield(.windowRenamed(windowId: $0, name: $1)) },
            onSessionChanged: { emit.yield(.sessionChanged(sessionId: $0, name: $1)) },
            onSessionRenamed: { emit.yield(.sessionRenamed(sessionId: $0, name: $1)) },
            onSessionWindowChanged: { emit.yield(.sessionWindowChanged(sessionId: $0, windowId: $1)) },
            onSessionsChanged: { emit.yield(.sessionsChanged) },
            onUnlinkedWindowAdd: { emit.yield(.unlinkedWindowAdd($0)) },
            onUnlinkedWindowClose: { emit.yield(.unlinkedWindowClose($0)) },
            onUnlinkedWindowRenamed: { emit.yield(.unlinkedWindowRenamed(windowId: $0, name: $1)) },
            onClientSessionChanged: { emit.yield(.clientSessionChanged(client: $0, sessionId: $1, name: $2)) },
            onActivePaneChanged: { emit.yield(.activePaneChanged(windowId: $0, paneId: $1)) },
            onPause: { emit.yield(.pause($0)) },
            onContinue: { emit.yield(.continued($0)) },
            // NOTE: do NOT touch isAttached here. `%client-detached` is a
            // server-wide broadcast about SOME client on the server — often
            // an unrelated one (e.g. a desktop client on another session).
            // Acting on it dropped us to the empty state mid-navigation when
            // any other client detached. OUR control session ending arrives
            // as `%exit` (below) or the byte stream closing (detach()). What
            // it can say is that the phone's mosh renderer is gone.
            onClientDetached: { emit.yield(.clientDetached($0)) },
            onExit: { _ in emit.yield(.exit) },
            onCommandResponse: { emit.yield(.commandResponse($0)) },
            onProtocolError: nil
        )
        eventPump = Task { @MainActor [weak self] in
            for await event in events {
                guard let self else { break }
                self.handle(event)
            }
        }
    }

    /// Single chokepoint for every LIVE `%…` notification tmux pushes over
    /// the control channel. Every case below either carries an id straight
    /// into a raw command (`.pause` → `refresh-client -A \(paneId):+`) or
    /// stores it into `snapshot`, from which dozens of other call sites
    /// later splice it unquoted into a `-t <id>` argument — so this is where
    /// every id gets checked against tmux's real shape (`isValidTmuxId`)
    /// before either happens. A malformed id (from a hostile/misbehaving
    /// server, or a parser bug) is dropped rather than acted on.
    private func handle(_ event: ControlEvent) {
        switch event {
        case .paneOutput(let paneId, let data):
            guard Self.isValidTmuxId(paneId) else { return }
            handlePaneOutput(paneId: paneId, data: data)
        case .layoutChange(let windowId, let layout, let zoomed):
            guard Self.isValidTmuxId(windowId) else { return }
            handleLayoutChange(windowId: windowId, layout: layout, zoomed: zoomed)
        case .windowAdd(let windowId):
            guard Self.isValidTmuxId(windowId) else { return }
            handleWindowAdd(windowId)
        case .windowClose(let windowId):
            guard Self.isValidTmuxId(windowId) else { return }
            handleWindowClose(windowId)
        case .windowRenamed(let windowId, let name):
            guard Self.isValidTmuxId(windowId) else { return }
            handleWindowRenamed(windowId, name: name)
        case .sessionChanged(let sessionId, let name):
            guard Self.isValidTmuxId(sessionId) else { return }
            handleSessionChanged(sessionId, name: name)
        case .sessionRenamed(let sessionId, let name):
            guard Self.isValidTmuxId(sessionId) else { return }
            handleSessionRenamed(sessionId, name: name)
        case .sessionWindowChanged(let sessionId, let windowId):
            guard Self.isValidTmuxId(sessionId), Self.isValidTmuxId(windowId) else { return }
            // tmux broadcasts this for EVERY session on the server, including
            // ones other clients are attached to — only follow window changes
            // of OUR session.
            guard sessionId == snapshot.activeSessionId else { return }
            // Remembered even when not followed below: killing the window on
            // screen, tmux says where its current window went BEFORE it says
            // the window closed, and that is where handleWindowClose lands.
            serverCurrentWindow = (sessionId, windowId)
            // The phone's viewed window is LOCAL state. tmux's current window
            // is shared across every client of the session, so following this
            // event unconditionally let a desktop client (or any server-side
            // automation) yank the phone off the window the user was reading
            // (the "自己跳 window" bug). Adopt the server's current window only
            // when our own choice is gone/invalid — fresh attach, session
            // switch, our window just died — or when this is the echo of our
            // own select-window (same id; fall through so zoom still runs).
            let current = snapshot.activeWindowId
            let currentIsOurs =
                current.flatMap { snapshot.windows[$0] }?.sessionId == snapshot.activeSessionId
            // B80: over mosh the screen IS tmux's current window — the
            // renderer is a regular client of the session, whose current
            // window all its clients share — so the sidecar always follows
            // it. Keeping the SSH rule left the breadcrumb (and the next pin
            // and zoom) on the old window after `prefix n` in the mosh TUI
            // or a desktop's select-window. (The Rust port's rule.)
            if rendersOutput, currentIsOurs, current != windowId { return }
            if current != windowId {
                Log.island.info("land: adopt server window \(windowId, privacy: .public) (was \(current ?? "-", privacy: .public))")
            }
            snapshot.activeWindowId = windowId
            // Window changed: land immersive — zoom the new window's active
            // pane if it has splits.
            ensureImmersiveZoom()
        case .sessionsChanged:
            // A session created or destroyed anywhere on the server: the
            // sheet and the Home tree list every session. Only a manual
            // refresh used to show it.
            relistSessions()
        case .unlinkedWindowAdd(let windowId):
            guard Self.isValidTmuxId(windowId) else { return }
            // A window linked into a session that isn't ours — created
            // there, or the first window of a new session. Its session is
            // not known from the line, so no placeholder: the listing says.
            relistWindows()
        case .unlinkedWindowClose(let windowId):
            guard Self.isValidTmuxId(windowId) else { return }
            // tmux 3.6a sends this, not %window-close, for EVERY window
            // closed — one killed in our own session included, since the
            // window is already unlinked when the notification goes out
            // (crates/tmux captures/layout.bin, `kill-window`). Same
            // handling: the window and its panes go, and if it was on
            // screen the phone moves on (see handleWindowClose).
            handleWindowClose(windowId)
        case .unlinkedWindowRenamed(let windowId, let name):
            guard Self.isValidTmuxId(windowId) else { return }
            handleWindowRenamed(windowId, name: name)
        case .clientSessionChanged(let client, let sessionId, _):
            guard Self.isValidTmuxId(sessionId) else { return }
            // Another client attached or switched: the attached flags the
            // session sheet shows changed.
            relistSessions()
            // The phone's renderer moving by itself (`prefix s` in the mosh
            // TUI) takes the sidecar along — once its pid confirms it is the
            // renderer, not a later client on the same tty.
            if !rendersOutput, let renderer = rendererClient, client == renderer.name {
                locateRenderer()
            }
        case .activePaneChanged(let windowId, let paneId):
            guard Self.isValidTmuxId(windowId), Self.isValidTmuxId(paneId) else { return }
            // Same server-wide broadcast caveat: ignore pane focus changes in
            // windows that aren't ours.
            guard snapshot.windows[windowId] != nil else { return }
            if var pane = snapshot.panes[paneId] {
                pane.isActive = true
                snapshot.panes[paneId] = pane
            }
            if windowId == snapshot.activeWindowId {
                if snapshot.activePaneId != paneId {
                    Log.island.info("land: server pane change \(paneId, privacy: .public) (was \(self.snapshot.activePaneId ?? "-", privacy: .public))")
                }
                snapshot.activePaneId = paneId
            }
        case .pause(let paneId):
            guard Self.isValidTmuxId(paneId) else { return }
            // Auto-resume; we don't throttle output yet. The argument's
            // vocabulary is `on/off/continue/pause` — the `:+` this used to
            // send is not in it, so tmux answered %error and the pane stayed
            // paused with its output stopped for good. %pause only fires when
            // the client is seconds behind (`pause-after` flag), which makes a
            // weak network the one place the wrong verb actually bit.
            send(rawCommand: "refresh-client -A \(paneId):continue")
        case .continued(let paneId):
            guard Self.isValidTmuxId(paneId) else { return }
            // tmux dropped this pane's output on the floor while it was
            // paused; %continue only means "streaming again", not "caught
            // up". Repaint from tmux's model to close the gap.
            resyncPane(paneId)
        case .clientDetached(let name):
            // Some client left the server — the mosh sidecar cares only
            // whether it was the phone's renderer: then there is no window
            // left that it may size (see rendererSessionId).
            if !rendersOutput, let renderer = rendererClient, name == renderer.name,
               rendererSessionId != nil {
                Log.island.info("land: sidecar renderer \(name, privacy: .public) detached")
                rendererSessionId = nil
            }
        case .exit:
            snapshot.isAttached = false
            closePendingCommands(reason: "exit")
        case .commandResponse(let response):
            handleCommandResponse(response)
        }
    }

    private func startPumping() {
        pumpTask?.cancel()
        let stream = sshSession.dataStream
        let client = self.client
        pumpTask = Task.detached(priority: .userInitiated) { [weak self] in
            for await chunk in stream {
                if Task.isCancelled { break }
                Self.ccTap("cc-raw.bin", chunk)
                self?.noteIncoming()
                await client.feed(chunk)
            }
            // The stream ENDING is a transport death, not an idle state —
            // without this the pump used to just return and the controller
            // kept queueing commands into the void forever.
            guard !Task.isCancelled, let self else { return }
            await self.closePendingCommands(reason: "stream-ended")
            await self.reportChannelDead(reason: "stream-ended")
        }
        startChannelWatchdog()
    }

    // MARK: - Dead-channel detection

    /// Wall-clock of the last chunk the -CC stream delivered — written from
    /// the pump (detached task), read by the watchdog (main). A lock, not an
    /// actor hop: one hop per chunk would be pure churn.
    private nonisolated let lastIncomingAt =
        OSAllocatedUnfairLock<Date>(initialState: .distantPast)
    private nonisolated func noteIncoming() {
        lastIncomingAt.withLock { $0 = Date() }
    }

    /// When the -CC stream last delivered anything. The hub reads this to
    /// answer "is this connection alive" without spending a probe channel —
    /// a control stream that is still talking has already answered.
    nonisolated var lastInboundAt: Date { lastIncomingAt.withLock { $0 } }

    /// Fired once when the -CC channel is judged dead — the hub tears the
    /// transport down and reconnects. This exists because the hub's
    /// keepAlive probe (`executeCommand "true"`) opens a FRESH channel: a
    /// half-dead link or a wedged tmux can stall only the -CC channel while
    /// that probe keeps passing (真机取证 2026-08-19: 381 commands piled up
    /// unanswered over minutes, screen frozen, no reconnect).
    @ObservationIgnored var onChannelSilent: (() -> Void)?
    @ObservationIgnored private var channelWatchdogTask: Task<Void, Never>?

    /// Declare the channel dead: log, mark detached, notify the hub once.
    private func reportChannelDead(reason: String) {
        ccTapLine("CHANNEL-DEAD reason=\(reason) pending=\(pendingCallbacks.count)")
        channelWatchdogTask?.cancel()
        channelWatchdogTask = nil
        onChannelSilent?()
    }

    /// Watch for commands piling up while the stream delivers nothing. The
    /// silence is counted in OBSERVED ticks, not wall clock, so an app
    /// suspension (tasks frozen, timestamps stale) can't fake a death the
    /// moment we foreground: the strikes only accumulate while the process
    /// is actually running and the channel is actually silent.
    private func startChannelWatchdog() {
        channelWatchdogTask?.cancel()
        channelWatchdogTask = Task { [weak self] in
            var strikes = 0
            var popStrikes = 0
            var lastSeen = Date.distantPast
            var lastPops: UInt64 = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(4))
                guard let self, !Task.isCancelled else { return }
                let incoming = self.lastIncomingAt.withLock { $0 }
                let hasPendings = self.snapshot.isAttached && !self.pendingCallbacks.isEmpty
                strikes = (hasPendings && incoming == lastSeen) ? strikes + 1 : 0
                // A wedged parser keeps bytes flowing while replies stop —
                // track pop PROGRESS separately, with more patience (a slow
                // link's capture can legitimately take several seconds).
                popStrikes = (hasPendings && self.popCounter == lastPops) ? popStrikes + 1 : 0
                lastSeen = incoming
                lastPops = self.popCounter
                // ≈12s of silence with pendings — but scaled by what this
                // link's round trips actually cost. A fixed 12s is generous on
                // a LAN and trigger-happy on a lossy cellular path, where one
                // stalled retransmit can swallow that long without the channel
                // being dead at all; declaring death there costs a full
                // re-handshake and lands the session in a reconnect loop
                // exactly when the network can least afford it.
                let silenceStrikes = max(3, Int((self.lastResyncRoundTrip * 6 / 4).rounded(.up)))
                if strikes >= silenceStrikes {
                    self.reportChannelDead(reason: "silent")
                    return
                }
                if popStrikes >= 8 {     // ≈32s with pendings and zero replies
                    self.reportChannelDead(reason: "no-pops")
                    return
                }
            }
        }
    }

    // MARK: - Initial discovery

    /// The pane discovery format, in ONE place. It is parsed by field position
    /// in `parseListPanes`, and a second call site that still spoke the older
    /// seven-field version silently dropped every pane it described — the parse
    /// requires all nine and a short line is skipped, not repaired.
    static let paneDiscoveryFormat =
        "'#{pane_id} #{window_id} #{pane_index} #{pane_width} #{pane_height} "
        + "#{pane_active} #{alternate_on} #{pane_in_mode} #{pane_current_command}'"

    private func sendInitialDiscovery() async {
        // Mark attached first so the UI can show progress while list-* fly.
        snapshot.isAttached = true
        snapshot.everAttached = true
        if attachedAt == nil { attachedAt = Date() }

        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-sessions") else { return }
            self.parseListSessions(rows)
        }
        sendCommand("list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} #{window_active} #{window_panes} #{window_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-windows") else { return }
            self.parseListWindows(rows)
        }
        sendCommand("list-panes -a -F \(Self.paneDiscoveryFormat)") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-panes") else { return }
            self.parseListPanes(rows)
            self.ensureTerminalsForAllPanes(isFreshAttach: true)
            self.applyPendingRestoreIfReady()
        }
        // B86: which sessions Moshpit made, from the server — after the
        // discovery: an attach's first three commands are the discovery's,
        // whoever answers them. crates/tmux's `send_initial_discovery`.
        if !rendersOutput { readOwnedSessions() }
    }

    /// The rows of a listing's reply — or nil when tmux refused it.
    ///
    /// A `list-*` answered with `%error` carries tmux's complaint, not rows,
    /// and every listing used to parse it as an answer all the same: the
    /// complaint is no row, so the sessions, windows or panes it was asked
    /// for were replaced by none — the tree, the breadcrumb and the panes'
    /// terminals gone until the next listing came back whole. A refusal says
    /// nothing about what the server holds; the snapshot keeps what it had.
    private func listing(_ response: TmuxCommandResponse, of what: String) -> [String]? {
        guard response.isError else { return response.lines }
        Log.ssh.notice("tmux: \(what, privacy: .public) refused (\(response.lines.first ?? "", privacy: .public)) — keeping what was listed before")
        return nil
    }

    /// `locatingRenderer`: false where the caller has just asked where the
    /// mosh renderer is (``handleSessionChanged``), so one listing does.
    func parseListSessions(_ lines: [String], locatingRenderer: Bool = true) {
        var newSessions: [String: SessionInfo] = [:]
        var lastAttached: String?
        for line in lines {
            // Format: "#{session_id} #{session_attached} #{session_name}" —
            // the name comes LAST because it may contain spaces. Cut on bytes
            // (see `tmuxFields`): a name starting with a combining mark used
            // to fuse with the space before it and the session went missing.
            // Unlike a window's, a session's name is never empty (tmux 3.6a
            // refuses `-s ''` and `rename-session ''`: "invalid session").
            let parts = line.tmuxFields(separator: " ", maxSplits: 2)
            guard parts.count >= 3 else { continue }
            let id = parts[0]
            // Every id in `snapshot` ultimately gets spliced unquoted into a
            // `-t <id>` control-mode argument elsewhere in this file — reject
            // a malformed reply here rather than let it in.
            guard Self.isValidTmuxId(id) else { continue }
            let isAttached = parts[1] != "0"
            let name = parts[2]
            newSessions[id] = SessionInfo(id: id, name: name, isAttached: isAttached)
            if isAttached { lastAttached = id }
        }
        snapshot.sessions = newSessions
        if rendersOutput {
            // Our client's session is what tmux told US — %session-changed,
            // or the switch we just issued. `session_attached` counts EVERY
            // client, so with the desktop sitting on another session it names
            // that one, and following it here dragged the active session back
            // over after every re-list (each %session-changed triggers one):
            // the phone could not switch back to a session the desktop had
            // left, while with the desktop on the same session the last
            // attached happened to be ours and everything seemed fine. Only a
            // session we no longer have — killed, or none known yet — falls
            // back to a guess.
            if let current = snapshot.activeSessionId, newSessions[current] != nil { return }
            snapshot.activeSessionId = lastAttached ?? newSessions.keys.min(by: TmuxSnapshot.idPrecedes)
        } else {
            // Control-plane sidecar: the session the person sees is the one
            // the phone's mosh renderer sits on, and only that client's own
            // row says which (see rendererSessionId). The attached flag
            // counts every client — the desktop's, this sidecar's own — and
            // following the last one set put the sidecar on whichever session
            // sorted last, the desktop's more often than not. Unknown
            // renderer: follow nothing.
            if locatingRenderer { locateRenderer() }
        }
    }

    private func parseListWindows(_ lines: [String]) {
        var newWindows: [String: WindowInfo] = [:]
        var tmuxActiveWindowId: String?
        for line in lines {
            // Format: "#{session_id} #{window_id} #{window_index}
            // #{window_layout} #{window_active} #{window_panes} #{window_name}"
            // — name LAST because programs rename windows to titles with spaces.
            // Cut on bytes, like parseListSessions, and KEEP empty fields: a
            // window's name can be empty (`rename-window ''`, or a program's
            // empty `ESC k ESC \` title under allow-rename on and
            // automatic-rename off), and tmux then ends the row on the
            // separator — `$0 @11 4 5960,80x24,0,0,11 0 1 ` (3.6a).
            // Dropped as empty, the name left the row a field short: the
            // window was skipped, and every pane in it lost its terminal with
            // it. No other field is ever empty, so a row with one empty is
            // not tmux's and is still skipped.
            let parts = line.tmuxFields(separator: " ", maxSplits: 6, omittingEmptySubsequences: false)
            guard parts.count == 7, !parts.dropLast().contains(where: \.isEmpty) else { continue }
            let sessionId = parts[0]
            let id = parts[1]
            // See parseListSessions — reject ids that don't match tmux's
            // real shape before they can reach a spliced -t argument.
            guard Self.isValidTmuxId(sessionId), Self.isValidTmuxId(id) else { continue }
            let index = Int(parts[2]) ?? 0
            let layout = parts[3]
            let isActive = parts[4] == "1"
            let paneCount = Int(parts[5]) ?? 1
            let name = parts[6]
            newWindows[id] = WindowInfo(
                id: id,
                sessionId: sessionId,
                name: name,
                index: index,
                layout: layout,
                isActive: isActive,
                paneCount: paneCount
            )
            // `list-windows -a` flags the active window of EVERY session; only
            // the attached session's active window is tmux's answer for OURS.
            if isActive, sessionId == snapshot.activeSessionId { tmuxActiveWindowId = id }
        }
        snapshot.windows = newWindows
        // Discovery refreshes fire for all kinds of server events. Following
        // tmux's shared current window here let any other client's window
        // switch yank the phone's view (the "自己跳 window" bug) — so adopt it
        // only when OUR choice is gone or belongs to another session (fresh
        // attach, session switch, our window died between refreshes).
        let current = snapshot.activeWindowId
        let currentIsOurs =
            current.flatMap { newWindows[$0] }?.sessionId == snapshot.activeSessionId
        // Over mosh the screen is tmux's current window: always (B80, see
        // `.sessionWindowChanged`).
        if !currentIsOurs || !rendersOutput, let tmuxActiveWindowId {
            Log.island.info("land: discovery adopts window \(tmuxActiveWindowId, privacy: .public) (was \(current ?? "-", privacy: .public))")
            snapshot.activeWindowId = tmuxActiveWindowId
        }
        // Navigation that didn't know its window yet (initial attach,
        // session switch) zooms now that the active window is known.
        if pendingImmersiveZoom {
            pendingImmersiveZoom = false
            ensureImmersiveZoom()
        }
    }

    private func parseListPanes(_ lines: [String]) {
        var newPanes: [String: PaneInfo] = [:]
        var activePaneId: String? = snapshot.activePaneId
        for line in lines {
            // Format: "#{pane_id} #{window_id} #{pane_index} #{pane_width}
            // #{pane_height} #{pane_active} #{alternate_on} #{pane_in_mode}
            // #{pane_current_command}" — command LAST since it may contain
            // spaces. Cut on bytes and keep empty fields, like
            // parseListWindows: the command reads empty for a program tmux
            // cannot name — a binary called `--` lists as
            // `%3 @1 1 80 11 0 0 0 ` (3.6a) — and that pane used to vanish.
            let parts = line.tmuxFields(separator: " ", maxSplits: 8, omittingEmptySubsequences: false)
            guard parts.count == 9, !parts.dropLast().contains(where: \.isEmpty) else { continue }
            let id = parts[0]
            let windowId = parts[1]
            // See parseListSessions — reject ids that don't match tmux's
            // real shape before they can reach a spliced -t argument.
            guard Self.isValidTmuxId(id), Self.isValidTmuxId(windowId) else { continue }
            let index = Int(parts[2]) ?? 0
            let width = Int(parts[3]) ?? 80
            let height = Int(parts[4]) ?? 24
            let isActive = parts[5] == "1"
            paneScreenState[id] = ScreenState(isAlternate: parts[6] == "1",
                                              isInCopyMode: parts[7] == "1")
            let command = parts[8]
            newPanes[id] = PaneInfo(
                id: id,
                windowId: windowId,
                index: index,
                command: command,
                width: width,
                height: height,
                isActive: isActive
            )
            if isActive && windowId == snapshot.activeWindowId {
                activePaneId = id
            }
        }
        snapshot.panes = newPanes
        if activePaneId != nil, activePaneId != snapshot.activePaneId {
            Log.island.info("land: discovery adopts pane \(activePaneId ?? "-", privacy: .public) (was \(self.snapshot.activePaneId ?? "-", privacy: .public))")
        }
        if activePaneId != nil { snapshot.activePaneId = activePaneId }
    }

    /// Mint terminals for any pane we haven't seen yet. Called after every
    /// `list-panes` reply so the persistent pool stays in sync with tmux's
    /// view of the world. `list-panes -a` returns every session's panes, but
    /// only the attached session streams %output, so we mint/backfill just its
    /// panes — others get terminals when we switch onto them.
    ///
    /// `isFreshAttach`: true only from `sendInitialDiscovery()` — the ONE
    /// discovery pass that follows a genuine (re)attach, as opposed to a
    /// routine re-discovery (`refreshWindowsAndPanes()`, e.g. after a session
    /// switch) on a connection that's been continuously live the whole time.
    /// See the veil+resync below for why this distinction matters.
    private func ensureTerminalsForAllPanes(isFreshAttach: Bool = false) {
        guard rendersOutput else { return }
        let activePaneIds = snapshot.panes.values
            .filter { snapshot.windows[$0.windowId]?.sessionId == snapshot.activeSessionId }
            .map(\.id)
        // Minting is local and cheap — every pane in the session gets a terminal
        // so a switch has somewhere to paint immediately.
        for paneId in activePaneIds where paneTerminals[paneId] == nil {
            _ = mintTerminal(for: paneId)
        }
        // Filling is NOT cheap: one `capture-pane` per pane, each carrying that
        // pane's scrollback. Only the panes actually on screen — the active
        // window's — are worth that on attach. A session with four windows of
        // three panes was paying twelve dumps to show three, and the other nine
        // are for windows the user may never open.
        //
        // Deferred, not dropped: `selectWindow` calls back here, so a window
        // fills the moment it is switched to. `backfilledPanes` makes the
        // repeat a no-op for panes already done.
        let onScreen = activePaneIds.filter {
            snapshot.panes[$0]?.windowId == snapshot.activeWindowId
        }
        for paneId in (onScreen.isEmpty ? activePaneIds : onScreen) {
            backfill(paneId: paneId)
        }
        // backfill() deliberately skips the live screen for primary-screen
        // panes, trusting tmux to repaint it via a fresh %output on attach —
        // but that repaint is a passive side effect with no ordering
        // guarantee against OUR OWN discovery finishing first, and
        // `snapshot.activePaneId` (set moments ago by parseListPanes, from
        // whichever pane tmux itself reports active) is otherwise only ever
        // veiled+resynced when it happens to ALSO match `pendingRestore` —
        // an unrelated, opportunistic mechanism. When the two don't line up
        // (or pendingRestore is nil/stale/already consumed), nothing ever
        // requests the live screen at all: the pane sits showing backfill's
        // scrollback — ending in whatever was last typed before the
        // reconnect — with nothing painted for the current viewport below
        // it (reported as "reconnect probabilistically leaves the last
        // input line behind"). Veil + explicitly resync the active pane
        // here too on a fresh attach, so it's covered on its own regardless
        // of whether a restore also fires — redundant (harmless) when
        // selectPane() etc. resyncs the same or a different pane moments
        // later, but never silently skipped. Gated to fresh attaches only —
        // an unconditional version fired on every routine re-discovery too
        // (a needless extra round trip, and a visible veil flash on a pane
        // the user might already be looking at/typing into).
        if isFreshAttach, let activePaneId = snapshot.activePaneId {
            paneCoordinators[activePaneId]?.veilForSwitch()
            paneCoordinators[activePaneId]?.extendCoverTimeout(by: 2.0)
            // Claim the window for the phone grid HERE, which is what
            // `connectAndAttach`'s "the normal flow pins as windows are
            // discovered" always assumed happened — it didn't. The only other
            // `fitWindowToClient` call is in `commitClientSize()`, which is
            // driven by a size REPORT from the terminal view, and SwiftTerm only
            // reports when the grid actually CHANGES: a pane whose first layout
            // matches the grid it was minted at reports nothing at all, and even
            // before panes were minted at that grid, a phone whose pre-connect
            // estimate happened to be exact reported the same numbers and was
            // deduped away.
            //
            // Unclaimed, `window-size latest` leaves the window at whatever an
            // already-attached desktop client made it. The pane's program then
            // renders to THAT width while this narrower view hard-wraps every
            // line, spilling a character or two onto the next one — reported as
            // "the first connect wraps everything wrong until I tap the
            // terminal", where the tap resized the grid and finally pinned it.
            if !pinsReleased, let win = snapshot.activeWindowId {
                requestPin(win, reason: .attach)
            }
            resyncPane(activePaneId)
            // The pin makes the pane's program repaint, and a capture taken now
            // can catch it mid-redraw — take a second one once it has settled,
            // for the same reason `commitClientSize()` does.
            pendingSettleResync?.cancel()
            pendingSettleResync = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(700))
                guard !Task.isCancelled else { return }
                self?.extendActiveCoverTimeout(by: 2.0)
                self?.resyncActivePane()
            }
        }
    }

    // MARK: - Notification handlers

    private func handlePaneOutput(paneId: String, data: Data) {
        // The coordinator's feed(data:) route is the canonical path. If the
        // coordinator doesn't exist yet (output arrived before list-panes
        // came back), mint a terminal on demand so we never drop bytes.
        onPaneActivity?(paneId)
        // Stamped before every early return below: "is this pane painting right
        // now" is about bytes arriving, not about whether we chose to draw them.
        lastPaneOutputAt[paneId] = Date()
        guard rendersOutput else { return }
        // While our window pin is handed back, don't paint.
        //
        // `releaseWindowPins()` gives the window up whenever the terminal goes
        // off screen or the app leaves the foreground, so that a desktop client
        // attaching meanwhile isn't stranded at phone width — and tmux keeps
        // sending `%output` throughout. Those bytes are laid out for whatever
        // width the window then took, and feeding them into this phone-sized
        // grid is what made coming back land on a visibly mis-wrapped screen.
        // Covering that up only traded it for a blank one; not painting it at
        // all is what actually fixes it. `repinActiveWindow()` repaints from
        // tmux on the way back.
        //
        // Nothing is lost by dropping these: a tmux pane's scrollback lives on
        // the server and the scroll gesture pages THAT, never this local buffer.
        // The bell is the exception that has to get through — it's the
        // agent-needs-you signal, and it normally reaches us through SwiftTerm's
        // parser, which this path skips.
        // Following another device's size is the same picture: the window is
        // laid out for a grid that is not ours, and painting that here is the
        // shredded-fragments garble. The frame the person last saw stays
        // under the veil until they take the window back (the takeover's
        // resync repaints it whole).
        // (Compared through the follow, not optional-to-optional: with no
        // follow, `nil == nil` for a pane discovery has not named yet would
        // gate the very output that mints its view.)
        let followingThisWindow = sizeFollow.map { $0.windowId == snapshot.panes[paneId]?.windowId } ?? false
        if pinsReleased || followingThisWindow {
            var detector = bellDetectors[paneId] ?? BellDetector()
            let rang = detector.containsBell(data)
            bellDetectors[paneId] = detector
            if rang { onPaneBell?(paneId) }
            return
        }
        // NO mid-session gate. Build 373-375 dropped output here while the
        // window sat at a foreign width (and until a repair frame landed),
        // relying on a capture choreography to repaint the gap — and every
        // bug anywhere in that chain became a PERMANENT diff-peer divergence
        // (weeks of them: stale immediate frames, starved retries, false
        // width rejects). Build 334 — user-verified clean under identical
        // conditions (desktop client attached, heavy streaming, background
        // cycles) — never dropped a mid-session byte: foreign-width bytes
        // paint (briefly mis-wrapped, usually under a veil), and the
        // reclaim + resync repaint makes the grid right again. Painting
        // wrong-then-repaired beats dropped-then-maybe-repaired: the former
        // is transient by construction, the latter is permanent on any slip.
        // (The BACKGROUND drop above stays — 334 had it too: while
        // suspended we cannot paint anyway, and the foreground repin's
        // resync is a single, well-tested repair point.)
        //
        // Post-disruption convergence: while a capture is pending as the
        // last word, every fed byte for the visible window restarts the
        // quiet debounce (see `armConvergenceResync`).
        if convergenceResyncPending,
           snapshot.panes[paneId]?.windowId == snapshot.activeWindowId {
            // First byte of a redraw switches the debounce from "is one even
            // coming?" to "let it drain" — see convergenceFirstDebounce().
            convergenceSawOutput = true
            scheduleConvergenceResync()
        }
        if let coordinator = paneCoordinators[paneId] {
            coordinator.feed(data: data)
            return
        }
        if snapshot.panes[paneId] == nil {
            // We don't yet know which window this pane belongs to; leave it
            // blank and let the next list-panes / layout-change repair it.
            snapshot.panes[paneId] = PaneInfo(id: paneId, windowId: "")
        }
        _ = mintTerminal(for: paneId)
        paneCoordinators[paneId]?.feed(data: data)
    }

    private func handleLayoutChange(windowId: String, layout: String, zoomed: Bool) {
        let panes = Self.paneCount(fromLayout: layout)
        var previousPanes: Int?
        var previousLayout: String?
        if var window = snapshot.windows[windowId] {
            previousPanes = Self.paneCount(fromLayout: window.layout) ?? window.paneCount
            previousLayout = window.layout
            window.layout = layout
            window.isZoomed = zoomed
            if let panes { window.paneCount = panes }
            snapshot.windows[windowId] = window
        } else {
            // Live notifications are for the attached session.
            snapshot.windows[windowId] = WindowInfo(
                id: windowId, sessionId: snapshot.activeSessionId ?? "",
                layout: layout, paneCount: panes ?? 1, isZoomed: zoomed)
        }
        // Our layouts, followed through a resize (round thirty-four): a
        // window of ours (its record's layout, or one our sizing left it in)
        // reflowed to another size is still ours — the phone's grid moved
        // (the keyboard, a rotation) and tmux refitted it. The same size in
        // another layout is a rearrangement, and not ours.
        if let previous = previousLayout, let record = pristineLayouts[windowId],
           Self.windowSize(previous) != Self.windowSize(layout),
           previous == record.layout || record.ours.contains(previous) {
            noteOurs(windowId, layout)
        }
        // tmux unzooms a window before it splits it, kills a pane in it or
        // reaps a pane whose process exited — and the phone is then looking
        // at the layout the pin squashed underneath the zoom, as often as not
        // a side pane crushed to one column (Claude Code's subagent panes,
        // 真机 2026-09-16: the screen one character wide). Moshpit shows one
        // pane at a time, so put the zoom back. Only when the pane count
        // moved: an unzoom that keeps the count is a person's prefix-z from
        // another client, and theirs to make. Never a blind `resize-pane -Z`
        // (a toggle): ``ensureImmersiveZoom`` zooms only if tmux still finds
        // the window unzoomed when the command runs, and the holder of the
        // window's size lease is the one device that acts.
        if rendersOutput, !pinsReleased, !zoomed, windowId == snapshot.activeWindowId,
           let panes, panes > 1, panes != previousPanes,
           sizeFollow == nil, !isHeldByAnotherLiveDevice(sizeLeases[windowId]) {
            ensureImmersiveZoom()
        }
        // Reclaim the window to OUR width if another client (or `window-size
        // latest`) sized it differently — otherwise a window a wide desktop tmux
        // client touched renders wrapped on the phone (the box-wrap + CJK tofu
        // bug). Only the active window we render, and only on real drift:
        // re-pinning to the same size is a no-op, so there's no feedback loop.
        // Suppressed while backgrounded (`pinsReleased`): we deliberately handed
        // the window to the desktop client's size there, and our own resize emits
        // a layout-change — re-pinning would immediately cramp it back.
        if rendersOutput, !pinsReleased, windowId == snapshot.activeWindowId,
           let width = Self.windowWidth(fromLayout: layout) {
            let foreign = width != lastClientSize.cols
            if foreign {
                // While the window sits at the other client's width, its
                // %output is laid out for THAT grid — handlePaneOutput gates
                // it (bell-only) exactly like the backgrounded case, because
                // painting 499-column repaints into a 70-column terminal is
                // the shredded-fragments garble (phone byte capture,
                // 2026-08-19: layout flapped 70×46 ↔ 499×62 thirteen times
                // in one scroll session — every app background/foreground
                // hands the pin back and forth against a desktop client on
                // the same session).
                activeWindowAtForeignWidth = true
                requestPin(windowId, reason: .drift)
                armForeignQuietRepin(windowId)
            } else if activeWindowAtForeignWidth {
                foreignQuietRepinTask?.cancel()
                foreignQuietRepinTask = nil
                // At our width there is nothing to follow; the lease record
                // stays, so a later drift still defers to a live holder.
                if sizeFollow?.windowId == windowId { sizeFollow = nil }
                // Back at our width: repaint the gap the gate dropped, with
                // the settling double-pass — this transition usually follows
                // our own fitWindowToClient, so the immediate capture races
                // the pane app's SIGWINCH repaint like every other resize.
                // 334-model: no gate — the settling repaint + convergence
                // capture cover whatever the war painted (handlePaneOutput).
                activeWindowAtForeignWidth = false
                resyncActivePaneSettling()
            }
        }
    }

    /// True while the ACTIVE window's layout reports a width other than ours
    /// — a desktop client and `window-size latest` mid-fight. Cleared the
    /// moment a layout-change reports our width again. See
    /// `handleLayoutChange` / `handlePaneOutput`.
    @ObservationIgnored private var activeWindowAtForeignWidth = false

    /// One-shot re-pin for a war that ENDS at the other client's width. The
    /// only exit from `activeWindowAtForeignWidth` is a return-to-our-width
    /// layout-change — and if the last reclaim was swallowed by the
    /// anti-flash backoff just as the desktop went quiet, no further
    /// layout-change ever comes: the gate stays closed forever and the pane
    /// freezes on its last frame, bell-only. Re-armed on every foreign
    /// layout-change, so it fires only once the war has actually gone quiet;
    /// bypasses the reclaim backoff on purpose (one resize after a quiet
    /// spell is not the flash-fight the backoff exists to stop).
    @ObservationIgnored private var foreignQuietRepinTask: Task<Void, Never>?
    /// Injectable for tests (production: seconds of layout quiet before the
    /// forced re-pin).
    @ObservationIgnored var foreignQuietRepinDelay: TimeInterval = 2.0

    private func armForeignQuietRepin(_ windowId: String) {
        foreignQuietRepinTask?.cancel()
        foreignQuietRepinTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(self?.foreignQuietRepinDelay ?? 2.0))
            guard !Task.isCancelled, let self, self.activeWindowAtForeignWidth,
                  self.rendersOutput, !self.pinsReleased, self.snapshot.isAttached,
                  self.snapshot.activeWindowId == windowId else { return }
            // Through the lease gate: against a desktop terminal this is the
            // forced re-pin it always was; while we FOLLOW another Moshpit
            // device it is the periodic re-check that notices the holder
            // releasing (background), vanishing (crash, reconnect) or the
            // window otherwise coming free — and then takes it.
            self.requestPin(windowId, reason: .recheck)
            // The resize's own layout-change (at our width) is what clears
            // the state and repairs the screen; if that notification is lost
            // on a weak link, keep trying at the same quiet cadence.
            self.armForeignQuietRepin(windowId)
        }
    }

    /// Consecutive rejected resync captures per pane — bounds the reject →
    /// re-pin → recapture loop (see `scheduleResyncRetry`).
    @ObservationIgnored private var resyncRejectStreak: [String: Int] = [:]
    @ObservationIgnored private var resyncRetryTask: Task<Void, Never>?

    /// True from a disruption (gate reopen, resize settle) until the closing
    /// convergence capture has run. `handlePaneOutput` restarts the quiet
    /// debounce on every fed byte so the capture lands once the pane app has
    /// actually finished writing — and a hard deadline fires it anyway under
    /// CONTINUOUS streaming (an agent pane never goes quiet; 真机取证
    /// 2026-08-19: the debounce alone starved and the divergence sat on
    /// screen). A mid-stream capture is safe: the -CC channel is ordered, so
    /// the frame bakes everything before it and later diffs apply to a base
    /// that matches the app's model.
    @ObservationIgnored private var convergenceResyncPending = false
    @ObservationIgnored private var convergenceResyncTask: Task<Void, Never>?
    @ObservationIgnored private var convergenceDeadlineTask: Task<Void, Never>?
    /// Whether the visible window has painted anything since this convergence
    /// pass was armed — i.e. whether there is a redraw to wait for at all.
    @ObservationIgnored private var convergenceSawOutput = false

    /// Quiet debounce once output IS flowing: long enough for a full-screen
    /// app's post-SIGWINCH repaint to drain before the last-word capture.
    private static let convergenceDrainDebounce: Duration = .milliseconds(600)

    /// Quiet debounce before anything has painted.
    ///
    /// This is what a switch normally hits, and the 600ms drain window is the
    /// wrong answer for it: a pane switch on an idle shell or a settled agent
    /// produces NO output at all (measured against a real tmux: zero %output
    /// lines, capture reply in single-digit ms), so the veil sat over a pane
    /// that had been ready for half a second. Waiting for a redraw to drain
    /// only makes sense once a redraw has started.
    ///
    /// Scaled off the measured capture round trip rather than fixed at some
    /// small number: the -CC stream is ordered, so a repaint tmux has already
    /// forwarded is in hand within a round trip, and the 1.5× buys the pane
    /// app its scheduling slack. An app that paints later than this still
    /// self-corrects — its bytes are live `%output`, and the settling pass
    /// re-captures behind them.
    private func convergenceFirstDebounce() -> Duration {
        .milliseconds(Int(min(0.6, max(0.2, lastResyncRoundTrip * 1.5)) * 1000))
    }

    /// Panes whose local resize is held, waiting for their capture.
    @ObservationIgnored private var paneResizeAwaitingCapture: Set<String> = []
    @ObservationIgnored private var deferredResizeTimeouts: [String: Task<Void, Never>] = [:]

    /// A capture that never comes must not leave the view frozen at a size the
    /// server has already left. Generous, because the capture rides the same
    /// FIFO as everything else and a busy reconnect can queue it behind
    /// discovery.
    private static let deferredResizeTimeout: Duration = .milliseconds(1500)

    private func scheduleDeferredResizeTimeout(_ paneId: String) {
        deferredResizeTimeouts[paneId]?.cancel()
        deferredResizeTimeouts[paneId] = Task { [weak self] in
            try? await Task.sleep(for: Self.deferredResizeTimeout)
            guard let self, !Task.isCancelled else { return }
            self.releaseDeferredResize(paneId)
        }
    }

    /// Apply a pane's held resize, if it has one. Idempotent.
    private func releaseDeferredResize(_ paneId: String) {
        deferredResizeTimeouts.removeValue(forKey: paneId)?.cancel()
        guard paneResizeAwaitingCapture.remove(paneId) != nil else { return }
        paneCoordinators[paneId]?.applyDeferredResize()
    }

    private func armConvergenceResync() {
        convergenceResyncPending = true
        convergenceSawOutput = false
        scheduleConvergenceResync()
        if convergenceDeadlineTask == nil {
            convergenceDeadlineTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(3))
                guard !Task.isCancelled else { return }
                self?.fireConvergenceResync()
            }
        }
    }

    /// Restart the quiet debounce (no-op unless armed).
    private func scheduleConvergenceResync() {
        guard convergenceResyncPending else { return }
        convergenceResyncTask?.cancel()
        let debounce = convergenceSawOutput
            ? Self.convergenceDrainDebounce
            : convergenceFirstDebounce()
        convergenceResyncTask = Task { [weak self] in
            try? await Task.sleep(for: debounce)
            guard !Task.isCancelled else { return }
            self?.fireConvergenceResync()
        }
    }

    private func fireConvergenceResync() {
        guard convergenceResyncPending else { return }
        convergenceResyncPending = false
        convergenceResyncTask?.cancel()
        convergenceResyncTask = nil
        convergenceDeadlineTask?.cancel()
        convergenceDeadlineTask = nil
        resyncActivePane()
    }

    /// A resync capture came back at a size this grid can't hold. Re-pinning
    /// alone (the 373 seal) starved the repair when BOTH settling passes hit
    /// mid-flap moments: the gate's dropped span then stayed missing forever,
    /// and the pane app's later diffs — computed against its own model —
    /// left stale cells sitting in the scrolled content ("部分字符没有滚动").
    /// Keep recapturing, bounded, until one lands at our size.
    private func scheduleResyncRetry(_ paneId: String, reveal: Bool) {
        let streak = (resyncRejectStreak[paneId] ?? 0) + 1
        resyncRejectStreak[paneId] = streak
        guard streak <= 4 else { return }   // deadline task fails open past here
        resyncRetryTask?.cancel()
        resyncRetryTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            self?.resyncPane(paneId, reveal: reveal)
        }
    }

    /// Gate for `handleLayoutChange`'s reclaim — see `recentReclaimTimestamps`.
    /// `selectWindow`/`commitClientSize` call `fitWindowToClient` directly and
    /// skip this: an explicit user-driven reclaim always gets its one shot.
    private func reclaimWindowOnDrift(_ windowId: String) {
        let now = Date()
        if reclaimTrackedWindow != windowId {
            reclaimTrackedWindow = windowId
            recentReclaimTimestamps.removeAll()
            reclaimSuspendedUntil = nil
        }
        if let suspendedUntil = reclaimSuspendedUntil {
            guard now >= suspendedUntil else { return }
            reclaimSuspendedUntil = nil
            recentReclaimTimestamps.removeAll()
        }
        recentReclaimTimestamps.append(now)
        recentReclaimTimestamps.removeAll { now.timeIntervalSince($0) > reclaimBackoffWindow }
        guard recentReclaimTimestamps.count < reclaimBackoffThreshold else {
            reclaimSuspendedUntil = now.addingTimeInterval(reclaimBackoffWindow)
            recentReclaimTimestamps.removeAll()
            return
        }
        fitWindowToClient(windowId)
    }

    /// Window width from a tmux layout string ("<checksum>,<W>x<H>,<x>,<y>{…}").
    nonisolated static func windowWidth(fromLayout layout: String) -> Int? {
        let fields = layout.tmuxFields(separator: ",")
        guard fields.count >= 2 else { return nil }
        return fields[1].tmuxFields(separator: "x").first.flatMap { Int($0) }
    }

    /// How many panes a `#{window_layout}` string describes: one per leaf
    /// `WxH,X,Y,ID`. A container stops after `WxH,X,Y` and opens a bracket,
    /// so it never matches. nil when the string holds no pane at all.
    nonisolated static func paneCount(fromLayout layout: String) -> Int? {
        let leaves = layout.matches(of: #/[0-9]+x[0-9]+,[0-9]+,[0-9]+,[0-9]+/#).count
        return leaves > 0 ? leaves : nil
    }

    private func handleWindowAdd(_ windowId: String) {
        if snapshot.windows[windowId] == nil {
            snapshot.windows[windowId] = WindowInfo(
                id: windowId, sessionId: snapshot.activeSessionId ?? "")
        }
        // Re-list so we pick up name, index, layout, and any new panes.
        sendCommand("list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} #{window_active} #{window_panes} #{window_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-windows") else { return }
            self.parseListWindows(rows)
        }
        sendCommand("list-panes -a -F \(Self.paneDiscoveryFormat)") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-panes") else { return }
            self.parseListPanes(rows)
            self.ensureTerminalsForAllPanes()
        }
    }

    private func handleWindowClose(_ windowId: String) {
        snapshot.windows.removeValue(forKey: windowId)
        let casualties = snapshot.panes.values.filter { $0.windowId == windowId }.map(\.id)
        for paneId in casualties {
            snapshot.panes.removeValue(forKey: paneId)
            paneTerminals.removeValue(forKey: paneId)
            paneCoordinators.removeValue(forKey: paneId)
        }
        guard snapshot.activeWindowId == windowId else { return }
        if !rendersOutput {
            // Over mosh the screen is the renderer's: tmux's own current
            // window, which tmux has already moved (its %session-window-
            // changed comes BEFORE the close, tmux 3.6a, and was turned away
            // because the dying window was still ours). Ask which one it is
            // instead of guessing; the listing adopts it and zooms it.
            snapshot.activeWindowId = nil
            snapshot.activePaneId = nil
            pendingImmersiveZoom = true
            relistWindows()
            return
        }
        // The window we were showing died (killed here or by another client).
        // Land on a surviving window LOCALLY, and point activePaneId at one of
        // its panes right away — leaving it on a dead pane blanks the screen
        // until the next discovery. Deliberately NO select-window: tmux is
        // already moving its own current window, and pushing ours would yank
        // desktop clients around the same way we refuse to be yanked. Where
        // tmux moved it (said just before the close) is the natural landing
        // — the window used last — and the first by index otherwise.
        let serverChoice = serverCurrentWindow.flatMap { remembered in
            remembered.session == snapshot.activeSessionId && snapshot.windows[remembered.window] != nil
                ? remembered.window : nil
        }
        let fallback = serverChoice ?? snapshot.sortedWindows.first?.id
        snapshot.activeWindowId = fallback
        if let fallback,
           let pane = snapshot.panes.values.first(where: { $0.windowId == fallback && $0.isActive })
               ?? snapshot.panes.values.first(where: { $0.windowId == fallback }) {
            paneCoordinators[pane.id]?.veilForSwitch()
            // See the matching comment in selectPane.
            paneCoordinators[pane.id]?.extendCoverTimeout(by: 2.0)
            snapshot.activePaneId = pane.id
            ensureImmersiveZoom()
            resyncPane(pane.id)
        } else {
            snapshot.activePaneId = nil
        }
    }

    /// The last `%session-window-changed` for our session — tmux's own
    /// current window there, followed or not.
    @ObservationIgnored private var serverCurrentWindow: (session: String, window: String)?

    private func handleWindowRenamed(_ windowId: String, name: String) {
        guard var window = snapshot.windows[windowId] else { return }
        window.name = name
        snapshot.windows[windowId] = window
    }

    /// A re-listing notifications ask for: at most one of a kind on its way.
    /// One that arrives while it is out asks for exactly one more, sent
    /// with the reply — it may describe something tmux did after it
    /// listed. A burst (a session created elsewhere says `%unlinked-window-add`,
    /// `%sessions-changed` and `%unlinked-window-renamed` together) costs
    /// one listing, never one each.
    private struct Relist {
        var inFlight = false
        var again = false
    }
    @ObservationIgnored private var sessionsRelist = Relist()
    @ObservationIgnored private var windowsRelist = Relist()

    /// Re-list the sessions for `%sessions-changed` and
    /// `%client-session-changed`, coalesced (see ``Relist``).
    private func relistSessions() {
        guard snapshot.isAttached else { return }
        guard !sessionsRelist.inFlight else {
            sessionsRelist.again = true
            return
        }
        sessionsRelist.inFlight = true
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self else { return }
            self.sessionsRelist.inFlight = false
            if let rows = self.listing(response, of: "list-sessions") { self.parseListSessions(rows) }
            if self.sessionsRelist.again {
                self.sessionsRelist.again = false
                self.relistSessions()
            }
        }
    }

    /// Re-list windows and panes for `%unlinked-window-add`, coalesced.
    private func relistWindows() {
        guard snapshot.isAttached else { return }
        guard !windowsRelist.inFlight else {
            windowsRelist.again = true
            return
        }
        windowsRelist.inFlight = true
        refreshWindowsAndPanes { [weak self] in
            guard let self else { return }
            self.windowsRelist.inFlight = false
            if self.windowsRelist.again {
                self.windowsRelist.again = false
                self.relistWindows()
            }
        }
    }

    /// Any client may rename any session — the desktop usually. tmux tells
    /// every control client; without this the name stayed whatever the last
    /// `list-sessions` said until something re-listed, and the breadcrumb,
    /// the session sheet and the hub's attach-by-name kept the old one.
    /// Nothing in this controller matches on a session name — ids do that —
    /// so a rename is a plain field update.
    private func handleSessionRenamed(_ sessionId: String, name: String) {
        guard var session = snapshot.sessions[sessionId] else { return }
        session.name = name
        snapshot.sessions[sessionId] = session
    }

    private func handleSessionChanged(_ sessionId: String, name: String) {
        // The FIRST %session-changed after the control channel comes up IS the
        // (re)attach. Every production path boots through `beginControlMode()`,
        // which deliberately sends no discovery of its own (tmux isn't running
        // yet) and lets this notification start it — `sendInitialDiscovery()`
        // only ever runs from `attach()`, which nothing but the tests calls. So
        // without this the fresh-attach repaint below was unreachable in the
        // shipping app: a reconnect left the pane showing backfill's scrollback
        // tail — the last thing typed before the drop — and every keystroke
        // after that (backspace especially) landed against a screen that no
        // longer matched tmux's. Later %session-changed events are session
        // switches on a live connection and must NOT re-veil.
        let isFreshAttach = !snapshot.isAttached
        Log.island.info("land: session-changed \(sessionId, privacy: .public) fresh=\(isFreshAttach, privacy: .public)")
        if isFreshAttach { Log.ssh.debug("resume: tmux %session-changed (attach)") }
        snapshot.activeSessionId = sessionId
        if var session = snapshot.sessions[sessionId] {
            session.name = name
            session.isAttached = true
            snapshot.sessions[sessionId] = session
        } else {
            snapshot.sessions[sessionId] = SessionInfo(id: sessionId, name: name, isAttached: true)
        }
        // tmux is alive and switched us onto a session — (re)discover its
        // windows and panes. This is also the boot signal for the
        // beginControlMode() path, where discovery must wait until the
        // `tmux -CC new` line has actually started tmux.
        snapshot.isAttached = true
        snapshot.everAttached = true
        if attachedAt == nil { attachedAt = Date() }
        // Control clients have no size until they report one; without it
        // tmux suppresses %output entirely. Report a sane default before
        // discovery — the real size follows from the terminal view layout.
        // Control-plane-only mode (mosh sidecar) must NOT report a size:
        // staying sizeless keeps %output suppressed (mosh renders the
        // screen) and keeps this client out of tmux's window-size math.
        if rendersOutput {
            // B150: what our size is about to squash, read on both sides of it
            // (see ``probeLayoutsBeforeSizing``).
            if isFreshAttach { probeLayoutsBeforeSizing(sessionId) }
            send(rawCommand: "refresh-client -C \(lastClientSize.cols)x\(lastClientSize.rows)")
            probeLayoutsAfterSizing()
            // Who we are to tmux, and whose the current window already is —
            // both ahead of discovery, so the first pin (in the list-panes
            // callback, FIFO behind these) can defer to a device that is
            // using it instead of yanking it to our grid.
            sendCommand("display-message -p '#{client_name}'") { [weak self] response in
                let name = response.lines.first?.trimmingCharacters(in: .whitespaces) ?? ""
                if !name.isEmpty { self?.ourClientName = name }
            }
            final class Box { var clients: Set<String> = [] }
            let box = Box()
            sendCommand("list-clients -F '#{client_name}'") { response in
                box.clients = Set(response.lines.map { $0.trimmingCharacters(in: .whitespaces) }
                    .filter { !$0.isEmpty })
            }
            sendCommand("show-options -wqv @moshpit_size_owner") { [weak self] response in
                let raw = response.lines.first?.trimmingCharacters(in: .whitespaces) ?? ""
                if let lease = SizeLease(parsing: raw, clients: box.clients) {
                    self?.pendingAttachLease = .held(lease)
                } else {
                    self?.pendingAttachLease = .unset
                }
            }
        } else {
            // mosh renders tmux's full TUI, status line included. It goes only
            // on a session Moshpit created: `status` is a session option, and
            // turning it off on the user's own session took the desktop's
            // status bar with it — for good, if the app never got to say
            // goodbye. See syncStatusLine.
            //
            // That is this client's session; the phone shows the renderer's.
            // With the renderer known, ask where it is before the re-listing
            // below (FIFO: a rebuilt sidecar's first window list then already
            // belongs to the renderer's session) and let that reply decide
            // the status line. Unknown, this session is all there is.
            //
            // B270: what the renderer's attach squashed, read again first.
            probeLayoutsAfterRenderer()
            // B86: which sessions Moshpit made, from the server.
            if isFreshAttach { readOwnedSessions() }
            locateRenderer()
        }
        // Both modes show ONE full-screen pane: zoom the active pane once the
        // window list lands (initial attach / session switch).
        pendingImmersiveZoom = true
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            guard let self, let rows = self.listing(response, of: "list-sessions") else { return }
            self.parseListSessions(rows, locatingRenderer: false)
        }
        refreshWindowsAndPanes(isFreshAttach: isFreshAttach)
        // The agent stamps ride FIFO right behind the tree, so the first
        // breadcrumb of this attach already has its agent. Left to the
        // monitor's 2s sweep they arrived a beat after the tree — the bar
        // showed the bare command, then grew the agent — and with the island
        // and notifications both off they never arrived at all.
        if isFreshAttach { pollAgentHooks() }
    }

    private func handleCommandResponse(_ response: TmuxCommandResponse) {
        // A block no command of ours asked for (flags 0: an `after-<command>`
        // hook one of ours fired, the late half of an `if-shell -b`) answers
        // none of them (B275). With `set-hook -g after-split-window
        // 'select-layout tiled'` in a tmux.conf, the new-pane button's reply
        // was followed by the hook's, and every reply after it landed one
        // command off until the queue drained. Only once this server has
        // flagged a reply of ours 1, though (the first discovery reply on any
        // tmux that does): a server writing 0 on every block is read the old
        // way, every block a reply, rather than never loading. The Rust
        // controller's rule (crates/tmux, B273).
        if response.fromClient {
            serverFlagsReplies = true
        } else if serverFlagsReplies {
            ccTapLine("SKIP  num=\(response.commandNum) err=\(response.isError) "
                + "lines=\(response.lines.count) flags 0: no command of ours "
                + "first=\(response.lines.first?.prefix(60) ?? "")")
            return
        }
        // tmux responds in strict send order; pop the head of the queue.
        guard !pendingCallbacks.isEmpty else {
            ccTapLine("POP!  num=\(response.commandNum) err=\(response.isError) "
                + "lines=\(response.lines.count) QUEUE-EMPTY first=\(response.lines.first?.prefix(60) ?? "")")
            return
        }
        let entry = pendingCallbacks.removeFirst()
        popCounter &+= 1
        ccTapLine("POP   num=\(response.commandNum) err=\(response.isError) "
            + "lines=\(response.lines.count) for=[\(entry.command.prefix(50))] "
            + "first=\(response.lines.first?.prefix(60) ?? "")")
        entry.callback?(response)
    }

    /// The control channel is gone — ``detach()``, `%exit`, the byte stream
    /// ending — and nothing still queued will be answered.
    ///
    /// Whoever waits on a reply is told so (``TmuxCommandResponse/channelClosed``)
    /// instead of waiting forever: the hub's attach check counted clients
    /// over the mosh sidecar and hung for good when the sidecar died with the
    /// count unanswered. The queue is emptied too: the sidecar's boot chain
    /// can start another control session on the same stream after `%exit`,
    /// and its replies must pair with its own commands, not with these.
    private func closePendingCommands(reason: String) {
        // Re-listings on their way are not coming back either.
        sessionsRelist = Relist()
        windowsRelist = Relist()
        guard !pendingCallbacks.isEmpty else { return }
        let waiting = pendingCallbacks.compactMap { $0.waits ? $0.callback : nil }
        ccTapLine("CLOSE reason=\(reason) dropped=\(pendingCallbacks.count) waiting=\(waiting.count)")
        pendingCallbacks.removeAll()
        // No reply is coming for a renderer listing or a layout probe on its
        // way either; left marked, they would block the next ones for good.
        rendererLocateInFlight = false
        pristineCapturesInFlight.removeAll()
        presizeLayouts = nil
        presizeAwaitsAfter = false
        for callback in waiting { callback(.channelClosed) }
    }

    /// Total replies popped — the watchdog's progress probe. A wedged parser
    /// (a reply block whose terminator was lost in transit) keeps INCOMING
    /// bytes flowing while pops stop dead, which the silence check alone
    /// can't see.
    @ObservationIgnored private var popCounter: UInt64 = 0

    /// This server flagged a reply to one of our lines 1 (B275): from here on
    /// a block flagged 0 answers nothing of ours. Kept for the controller's
    /// life, as the Rust controller keeps it: one controller talks to one
    /// host's tmux.
    @ObservationIgnored private var serverFlagsReplies = false

    // MARK: - Sending commands

    /// Fire-and-forget write to the SSH transport. Used for `send-keys`,
    /// `select-window`, `refresh-client` and similar. Still occupies a
    /// (nil) response slot — tmux replies to every command.
    /// Wait until every command queued so far has reached the socket.
    ///
    /// `writeChain` is what gives control-mode writes their FIFO order; awaiting
    /// its tail is what turns "these commands were queued" into "these commands
    /// are out". Needed when the app is backgrounding: the window-pin release
    /// has to land on the server before iOS suspends us, and queueing it is not
    /// the same as sending it. See ``BackgroundAssertion``.
    func flushPendingWrites() async {
        await writeChain?.value
    }

    private func send(rawCommand command: String) {
        enqueue(command, callback: nil)
    }

    /// Send a command and register a callback for its response block.
    /// `waits`: the callback resumes someone awaiting the reply — see
    /// ``PendingCommand/waits``.
    private func sendCommand(_ command: String, waits: Bool = false,
                             callback: @escaping (TmuxCommandResponse) -> Void) {
        enqueue(command, waits: waits, callback: callback)
    }

    private func enqueue(_ command: String, waits: Bool = false,
                         callback: ((TmuxCommandResponse) -> Void)?) {
        // Before tmux is attached, anything we write lands in the SHELL, not
        // tmux — it would type garbage at the prompt AND leave an orphan
        // callback slot that desyncs the response FIFO for the entire
        // session. Drop commands until control mode is live. (Layout-driven
        // resizeClient calls are the usual pre-attach offender.) Whoever
        // waits on the reply hears now that none is coming.
        guard snapshot.isAttached else {
            ccTapLine("DROP  (pre-attach) [\(command.prefix(50))]")
            if waits { callback?(.channelClosed) }
            return
        }
        // One line is one command in control mode: a line break inside one
        // ends it there and runs the rest as a command of its own, and every
        // reply after it lands on the wrong callback (B139). Nothing the
        // controller builds holds one; anything that does is not sent.
        guard !command.unicodeScalars.contains(where: { $0 == "\n" || $0 == "\r" }) else {
            ccTapLine("DROP  (line break) [\(command.prefix(50))]")
            Log.ssh.error("tmux: a command holding a line break was not sent")
            if waits { callback?(.channelClosed) }
            return
        }
        guard let data = (command + "\n").data(using: .utf8) else { return }
        pendingCallbacks.append(PendingCommand(command: command, callback: callback, waits: waits))
        ccTapLine("SEND  depth=\(pendingCallbacks.count) [\(command.prefix(50))]")
        let session = sshSession
        let previous = writeChain
        writeChain = Task(priority: .userInitiated) {
            await previous?.value
            try? await session.write(data)
        }
    }

    // MARK: - Terminal minting

    /// Create the persistent terminal + ``Coordinator`` pair for `paneId`,
    /// wire input/resize back through the controller, and stash both in the
    /// non-observed pool. The view is ``surfaceKind``'s.
    ///
    /// Nothing here may count on the bytes being parsed when `feed` returns
    /// (the core parses on a thread of its own): a capture's reveal is a
    /// mark after its bytes (``SwiftTerminalView/Coordinator/
    /// revealWhenShown()``), a held resize is applied before them.
    @discardableResult
    private func mintTerminal(for paneId: String) -> TerminalSurface {
        if let existing = paneTerminals[paneId] {
            return existing
        }
        let font = TerminalFont.font(id: currentFontName, size: CGFloat(currentFontSize))
        // Born at the client's grid, not at `.zero`.
        //
        // A pane terminal is minted during discovery — before SwiftUI has laid
        // it out even once — and a view born at `.zero` derives its grid
        // from that frame, so it starts one column wide. Everything tmux hands
        // us at attach (the authoritative screen from `resyncPane`, the
        // backfill, any `%output` that beats the first layout) was then parsed
        // into a one-column terminal: a full-screen TUI's repaint collapses
        // into a column of single characters and a diff-rendered agent frame
        // into nothing at all — "the tmux screen is blank except for a cursor
        // until I tap". Sizing it from `lastClientSize` (already the phone grid
        // by the time any pane is minted) means a pre-layout feed lands in the
        // shape tmux drew it for; the first real layout then agrees and costs
        // no reflow.
        let terminal = TerminalSurfaces.make(
            surfaceKind, font: font, grid: TerminalGrid(cols: lastClientSize.cols, rows: lastClientSize.rows))
        terminal.followTmuxClears()
        // The user's cursor is enforced after every feed — pane output (vim,
        // zsh plugins, coding agents) carries DECSCUSR/OSC-12 that would
        // otherwise override the user's Settings choice.
        terminal.reassert(look: look)

        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.onReveal = { [weak self] in self?.notePaneRevealed() }
        // Delivered synchronously, straight onto the write chain. The terminal
        // calls this from the main thread — UIKit key input, and the replies
        // the emulator generates while being fed, which happens on the main
        // actor — so there is nothing to hop for, and a hop is what must not
        // be here: a Task per keystroke is FIFO only in practice, and the
        // byte path takes no unordered hop (the garbled-rendering saga).
        coordinator.onInput = { [weak self, paneId] data in
            MainActor.assumeIsolated {
                guard let self else { return }
                // See transportIsLive — drop keystrokes while not live so
                // they can't queue up and replay into the pane after resume.
                guard self.transportIsLive?() ?? true else { return }
                self.sendInput(data, paneId: paneId)
            }
        }
        // The app's own layout reporting its grid — the only reliable answer to
        // "what size is this really", since `onSizeChange` fires only on a
        // CHANGE and a pane minted at the right grid never changes. Without it
        // the client size stays whatever `estimateGrid` guessed, and the window
        // gets pinned to a width this view does not have.
        coordinator.onGridReport = { [weak self, paneId] cols, rows in
            guard let self else { return }
            guard self.snapshot.activePaneId == paneId
                    || self.snapshot.activePanes.count <= 1 else { return }
            self.resizeClient(rows: rows, cols: cols)
        }
        // Hold a resize until the capture that replaces what it destroys.
        //
        // Shrinking a buffer is LOSSY: SwiftTerm removes rows by popping the
        // lines BELOW the cursor, and a full-screen TUI keeps its own
        // furniture there — Claude Code's input box sits two rows above its
        // own footer, and raising the keyboard simply deleted that footer
        // until a repaint came back (user screenshots, 379 through 382).
        // tmux's own reflow does NOT lose those rows, so the capture this
        // path always takes after a resize has them; all that was needed is
        // to not apply the local resize until that capture is in hand.
        //
        // The grid has to be reported from HERE too: the layout callback that
        // normally reports it runs after the frame change being held, so
        // otherwise the client would never resize and the capture never come.
        coordinator.shouldDeferResize = { [weak self, paneId] cols, rows in
            guard let self, self.rendersOutput, self.snapshot.isAttached else { return false }
            guard self.snapshot.activePaneId == paneId
                    || self.snapshot.activePanes.count <= 1 else { return false }
            // No client resize means no capture is coming, and holding for
            // one that never arrives just stalls the frame until the timeout.
            guard self.resizeClient(rows: rows, cols: cols) else { return false }
            self.paneResizeAwaitingCapture.insert(paneId)
            self.scheduleDeferredResizeTimeout(paneId)
            return true
        }
        coordinator.onSizeChange = { [weak self, paneId] cols, rows in
            Task { @MainActor [weak self] in
                guard let self else { return }
                // The client viewport drives the window size; tmux divides it
                // among panes per the layout. Only the active (visible,
                // full-area when single) pane reports.
                if self.snapshot.activePaneId == paneId || self.snapshot.activePanes.count <= 1 {
                    self.resizeClient(rows: rows, cols: cols)
                }
            }
        }
        coordinator.onBell = { [weak self, paneId] in
            Task { @MainActor [weak self] in
                self?.onPaneBell?(paneId)
            }
        }
        // tmux panes scroll server-side: the controller decides wheel (mouse
        // app — forwarded so the app scrolls itself) vs copy-mode (plain shell).
        // Refresh the active pane's mouse flag at drag start so it's fresh.
        coordinator.onScroll = { [weak self] lines in
            self?.scroll(lines: lines)
        }
        coordinator.onScrollPixels = { [weak self] dy in
            self?.scroll(pixels: dy) ?? 0
        }
        coordinator.onScrollEnd = { [weak self] in
            self?.scrollDidEnd()
        }
        coordinator.onScrollBegin = { [weak self] in
            self?.wheelCarry.reset()
            self?.refreshActivePaneMouse()
        }
        // Tap-to-position: forwarded as a click when the pane's program wants
        // the mouse. Refresh the flag alongside it so a program launched
        // in-place (no pane switch, no scroll since) is recognised by the next
        // tap rather than the one after it.
        coordinator.onClick = { [weak self] col, row in
            guard let self else { return }
            self.click(col: col, row: row)
            self.refreshActivePaneMouse()
        }
        // Horizontal swipe: switch pane (if the window has splits) or window.
        coordinator.onSwitch = { [weak self] forward in
            self?.switchPaneOrWindow(forward: forward)
        }
        // Globe-key IME switch: same height keyboards → no resize event, so
        // none of the resize-driven healing runs. Nothing local is repainted
        // (a composition draws in a subview of its own, fork patch 5, so a
        // redraw of the same rows changed nothing); converge with tmux's
        // model instead, which repaints whatever cells composing teardown
        // really damaged.
        coordinator.onInputModeChange = { [weak self, paneId] in
            self?.resyncPane(paneId)
        }
        coordinator.onFontSizeCommit = { [weak self] size in
            self?.onFontSizeCommitted?(size)
        }
        // Submitting a line (IME commit + Enter) is the other divergence
        // hotspot — marked-text teardown + echo + the app clearing its input
        // box interleave, and stale cells ("the sent text is still in the
        // input bar") persist because nothing repaints. Resync shortly after,
        // once the app's own redraw has settled.
        coordinator.onReturnSend = { [weak self, paneId] in
            guard let self else { return }
            self.pendingSettleResync?.cancel()
            self.pendingSettleResync = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(450))
                guard !Task.isCancelled else { return }
                self?.resyncPane(paneId)
            }
        }
        coordinator.pathLinks = pathLinksFactory?(paneId)
        coordinator.onOpenRemotePath = { [weak self] path in self?.onOpenRemotePath?(path) }
        coordinator.attach(terminal)

        paneTerminals[paneId] = terminal
        paneCoordinators[paneId] = coordinator
        return terminal
    }
}

extension TmuxCommandResponse {
    /// What a command still awaiting its reply gets when the control channel
    /// closes first (see `TmuxSessionController.closePendingCommands`). Never
    /// a reply tmux sends: its command ids and numbers are not negative.
    static let channelClosed = TmuxCommandResponse(commandId: -1, commandNum: -1, isError: true, lines: [])
}

// MARK: - The Rust session's tmux (W14a)

extension TmuxSessionController {
    /// What the Rust core's tmux controller says, put where the chrome reads
    /// a controller (``RustTmuxController``): the tree, the notice, the
    /// refresh spinner, the follower's veil, the agents' stamps. Only what
    /// changed is assigned; a new pane on screen slides in as a switch made
    /// here does (``selectPane(_:)``).
    func mirrorRust(snapshot new: Snapshot, notice: String?, isRefreshing: Bool,
                    sizeFollow: SizeFollow?, agentHooks: [String: AgentHook],
                    agentHooksLoaded: Bool) {
        if snapshot != new {
            if snapshot.activePaneId != nil, snapshot.activePaneId != new.activePaneId {
                withAnimation(.easeInOut(duration: 0.22)) { snapshot = new }
            } else {
                snapshot = new
            }
        }
        if self.notice != notice { self.notice = notice }
        if self.isRefreshing != isRefreshing { self.isRefreshing = isRefreshing }
        if self.sizeFollow != sizeFollow { self.sizeFollow = sizeFollow }
        if self.agentHooks != agentHooks { self.agentHooks = agentHooks }
        if self.agentHooksLoaded != agentHooksLoaded { self.agentHooksLoaded = agentHooksLoaded }
    }

    /// The core said this attach painted its first frame.
    func mirrorRustFirstFrame() {
        hasPaintedSinceAttach = true
    }
}
