import Foundation
import Observation
import SwiftTerm
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
///   - ``terminalView(for:)`` returns the **same** ``TerminalView`` instance
///     for the same `paneId` for the entire lifetime of that pane. New
///     terminals are minted lazily the first time a pane is encountered.
///   - `%output` bytes from the parser are routed *directly* to the
///     coordinator's `feed(data:)`. There is no per-pane output buffer.
///
/// ### Threading
/// Everything is `@MainActor`. SwiftTerm `TerminalView` instances and SwiftUI
/// view-tree updates both require the main actor, and parser callbacks
/// dispatch back to us via `Task { @MainActor in … }`.
@Observable
@MainActor
final class TmuxSessionController: MultiplexerControlling {

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

    /// Persistent `TerminalView` per pane. Created lazily the first time a
    /// pane is seen, kept until tmux closes the pane / window.
    @ObservationIgnored
    private var paneTerminals: [String: TerminalView] = [:]

    /// Coordinators paired with each `TerminalView`. The coordinator owns the
    /// data-feed path and forwards user input back through ``sendInput``.
    @ObservationIgnored
    private var paneCoordinators: [String: SwiftTerminalView.Coordinator] = [:]

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
    private var pendingCallbacks: [(command: String, callback: ((TmuxCommandResponse) -> Void)?)] = []

    // MARK: - Diagnostic tap
    //
    // `-MOSHPIT_CC_TAP <dir>` (Debug tooling, simulator e2e): append-only
    // traces of the raw -CC stream, every command↔response pairing, and every
    // frame fed to a pane — the ground truth for chasing pairing shifts
    // ("protocol text painted into a pane") without guessing from screenshots.
    static let ccTapDir: String? = {
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
    init(sshSession: TmuxTransport, rendersOutput: Bool = true) {
        self.sshSession = sshSession
        self.rendersOutput = rendersOutput
    }

    // MARK: - Public API

    /// Apply (and remember) the theme + font size + cursor style used when
    /// creating new pane terminals. Existing terminals are updated in-place.
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
        for (paneId, terminal) in paneTerminals {
            terminal.font = font
            theme.apply(to: terminal)
            // Install the applied pair as the coordinator's enforced cursor:
            // pane output carries DECSCUSR/OSC-12 (vim, zsh plugins, coding
            // agents) and would otherwise override the user's choice until
            // the next Settings change — the "sometimes wrong" cursor.
            paneCoordinators[paneId]?.enforcedCursor = TerminalCursor.apply(
                shape: cursorShape, colorId: cursorColorId, blink: cursorBlink, to: terminal)
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
        // Same reasoning for the foreign-width gate: a repair frame that will
        // never arrive must not keep the next connection's output gated.
        activeWindowAtForeignWidth = false
        foreignQuietRepinTask?.cancel()
        foreignQuietRepinTask = nil
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

    /// Returns the persistent ``TerminalView`` for `paneId`. If one does not
    /// yet exist (pane discovered after attach), it is minted on demand
    /// using the currently configured theme/font.
    ///
    /// **The same instance is returned for the same paneId across calls.**
    /// SwiftUI's `UIViewRepresentable.makeUIView` relies on this so window
    /// switches don't destroy the terminal view.
    func terminalView(for paneId: String) -> TerminalView {
        if let existing = paneTerminals[paneId] { return existing }
        return mintTerminal(for: paneId)
    }

    /// The coordinator minted alongside ``terminalView(for:)`` — the pane
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
        fitWindowToClient(windowId)
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

    /// A window's pane layout as it was BEFORE we touched it: the raw
    /// `#{window_layout}` string, the pane the USER had zoomed (nil when the
    /// window wasn't zoomed) and the pane count that string describes.
    private struct PristineLayout {
        let layout: String
        let zoomedPane: String?
        let paneCount: Int
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
            self.notePristineLayout(windowId, fields: line.split(separator: " ").map(String.init))
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

    /// A tmux layout string: four hex checksum digits, a comma, then nothing but
    /// digits and layout punctuation. Cheap sanity gate so a surprise reply (an
    /// error line, a truncated field) can't reach `select-layout`. nonisolated
    /// so it's unit-testable without a live tmux server.
    nonisolated static func isPlausibleLayout(_ layout: String) -> Bool {
        guard layout.count > 6, layout.count < 4096 else { return false }
        let parts = layout.split(separator: ",", maxSplits: 1)
        guard parts.count == 2, parts[0].count == 4,
              parts[0].allSatisfy(\.isHexDigit) else { return false }
        return parts[1].allSatisfy { $0.isNumber || "x,[]{}".contains($0) }
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
    private func restorePristineLayout(_ windowId: String) {
        guard let pristine = pristineLayouts.removeValue(forKey: windowId) else { return }
        guard pristineStillApplies(windowId, pristine) else { return }
        send(rawCommand: "select-layout -t '\(windowId)' '\(pristine.layout)'")
        if let pane = pristine.zoomedPane {
            send(rawCommand: "resize-pane -Z -t '\(pane)'")
        }
    }

    /// The same restore as ``restorePristineLayout``, handed out as commands for
    /// teardown paths that must run OUTSIDE the -CC channel: an in-band write
    /// races the detach and gets lost (the reason pin release moved to a
    /// one-shot exec channel too). Consumes the records. Targets are `@id`/
    /// `%id`, which survive the login shell that channel goes through (a `$id`
    /// would not — see ``restoreSuppressedStatusAndFlush``), and the layout
    /// string is single-quoted so its `{}` reach tmux intact.
    func pristineLayoutRestoreCommands() -> [String] {
        defer { pristineLayouts.removeAll() }
        var commands: [String] = []
        for windowId in pristineLayouts.keys.sorted() {
            guard let pristine = pristineLayouts[windowId],
                  pristineStillApplies(windowId, pristine) else { continue }
            commands.append("select-layout -t \(windowId) '\(pristine.layout)'")
            if let pane = pristine.zoomedPane {
                commands.append("resize-pane -Z -t \(pane)")
            }
        }
        return commands
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

    /// Count clients attached WITHOUT control mode — real renderers (the mosh
    /// TUI client, desktop terminals), as opposed to `-CC` sidecars like us.
    /// The hub uses the delta to verify the mosh renderer's `tmux attach`
    /// actually landed. Calls back with -1 when not attached (the command
    /// would be dropped and the callback would otherwise never run).
    func countNonControlClients(_ completion: @escaping (Int) -> Void) {
        guard snapshot.isAttached else { completion(-1); return }
        sendCommand("list-clients -F '#{client_control_mode}'") { response in
            completion(response.lines.filter {
                $0.trimmingCharacters(in: .whitespaces) == "0"
            }.count)
        }
    }

    /// Mosh sidecar (`rendersOutput == false`) window sizing, driven by the hub.
    ///
    /// The mosh-rendered tmux TUI is raw mosh — it can't issue control commands,
    /// and `fitWindowToClient` no-ops for a sidecar — so nothing would pin the
    /// window to the phone grid. Meanwhile this control client's own 80×24 PTY
    /// drags tmux's window-size math down, stranding the TUI at 80×24 in the
    /// corner. Fix both: match our control-client size to the mosh grid, then
    /// pin the active window to it (full-screen TUIs redraw at phone width).
    /// Pinned windows are reverted (`resize-window -A`) on disconnect in
    /// `SessionHub.stop()`.
    func syncMoshWindow(cols: Int, rows: Int) {
        guard !rendersOutput, snapshot.isAttached else { return }
        lastClientSize = (cols, rows)
        send(rawCommand: "refresh-client -C \(cols)x\(rows)")
        if let win = snapshot.activeWindowId {
            capturePristineLayout(win)
            send(rawCommand: "resize-window -t \(win) -x \(cols) -y \(rows)")
            resizedWindows.insert(win)
        }
    }


    /// Switch the attached client to another session, then re-discover its
    /// windows/panes so the snapshot follows.
    func selectSession(_ sessionId: String) {
        // See the matching comment in selectPane: a manual switch must not
        // get silently overridden by a still-pending reconnect restore.
        pendingRestore = nil
        // Leaving the session: its status bar must come back whatever the
        // zoom state we left behind (the new session re-evaluates via its
        // own layout events).
        setStatusSuppressed(false)
        let previousSessionName = snapshot.activeSessionId
            .flatMap { snapshot.sessions[$0]?.name }
        if snapshot.activeSessionId != sessionId {
            snapshot.activeSessionId = sessionId
        }
        if rendersOutput {
            send(rawCommand: "switch-client -t \(sessionId)")
        } else {
            // Control-plane mode: `switch-client` alone would move only THIS
            // (invisible) control client. The screen the user sees is the
            // mosh-rendered client attached to the same session — move every
            // non-control client currently on OUR session along with us,
            // without touching clients on other sessions (e.g. the user's
            // desktop tmux).
            sendCommand("list-clients -F '#{client_control_mode}|#{client_session}|#{client_name}'") { [weak self] response in
                guard let self else { return }
                for line in response.lines {
                    let parts = line.split(separator: "|", maxSplits: 2)
                    guard parts.count == 3, parts[0] == "0" else { continue }
                    guard previousSessionName == nil || parts[1] == previousSessionName! else { continue }
                    self.send(rawCommand: "switch-client -c \(parts[2]) -t \(sessionId)")
                }
                self.send(rawCommand: "switch-client -t \(sessionId)")
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
        pendingRestore = nil
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
            fitWindowToClient(newWindow)
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
                .split(separator: " ") ?? []
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
    /// against tmux 3.6a, scripts/tmux-cc-lab `copymode` scenario,
    /// 2026-08-19). Driving copy-mode from this path scrolled a screen the
    /// phone can never see — and hijacked any desktop client sharing the
    /// window into copy-mode. (The mosh+tmux path is different: its renderer
    /// IS a regular client, so SessionHub's copy-mode driver stays correct.)
    func scroll(lines: Int) {
        guard lines != 0,
              let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        // An ALT-SCREEN mouse app (Claude Code) gets the wheel unconditionally:
        // its own repaint after a wheel updated `lastPaneOutputAt`, which
        // read as "streaming" and flipped the NEXT tick into the parked path,
        // where the sticky latch kept every later swipe. First swipe scrolled,
        // the rest were dead (the 347 report).
        if activePaneWantsMouse && activePaneOnAltScreen {
            sendWheel(lines: lines, paneId: paneId)
            return
        }
        // NON-alt panes: forwarding the wheel to an app that is CURRENTLY
        // repainting makes the two fight over the same viewport: the app
        // scrolls where we asked, then its next frame pins back to the
        // bottom, then the next swipe scrolls again — the reported judder
        // while output streams. Park the viewport in the LOCAL scrollback
        // instead: the coordinator's scroll-hold buffers live output while
        // the user reads, exactly like the plain-SSH path.
        let appDrivesItsOwnScroll = activePaneWantsMouse && !isStreaming(paneId)
        if appDrivesItsOwnScroll {
            sendWheel(lines: lines, paneId: paneId)
        } else if activePaneOnAltScreen {
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
            return
        } else {
            paneCoordinators[paneId]?.scrollLocal(lines: lines)
        }
    }

    /// Synthesize SGR wheel bytes into the pane's program.
    private func sendWheel(lines: Int, paneId: String) {
        let terminal = paneTerminals[paneId]?.getTerminal()
        let col = max(1, (terminal?.cols ?? 80) / 2)
        let row = max(1, (terminal?.rows ?? 24) / 2)
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

    // A pane-targeted paste (e.g. the Home card's per-agent image entry)
    // used to decide bracketing HERE, by reading back
    // `paneTerminals[paneId]?.getTerminal().bracketedPasteMode` — the local
    // SwiftTerm's memory of having SEEN the pane's own `ESC[?2004h` go by.
    // That memory is permanently wrong after a fresh SSH+tmux attach: tmux
    // does not replay mode-setting sequences on `-CC attach` (lab-verified,
    // `scripts/tmux-cc-lab`, 2026-08-19), so the flag starts (and stays)
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
        let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let namePart = trimmed.isEmpty ? "" : " -n \(tmuxQuote(trimmed))"
        pendingWindowFocus = nil
        sendCommand("new-window -P -F '#{window_id}'\(namePart)") { [weak self] response in
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
    private func releaseWindowPin(_ windowId: String) {
        if resizedWindows.contains(windowId) {
            resizedWindows.remove(windowId)
            // Unset the per-window override — the REAL "back to automatic".
            // (`resize-window -A` merely re-pins at "largest session", still
            // manual, which kept stranding desktop clients.)
            send(rawCommand: "set-option -u -w -t \(windowId) window-size")
        }
        restorePristineLayout(windowId)
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
        for win in resizedWindows {
            send(rawCommand: "set-option -u -w -t \(win) window-size")
        }
        resizedWindows.removeAll()
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
        pinsReleased = false
        guard snapshot.isAttached, let win = snapshot.activeWindowId else { return }
        send(rawCommand: "refresh-client -f !ignore-size")
        send(rawCommand: "refresh-client -C \(lastClientSize.cols)x\(lastClientSize.rows)")
        capturePristineLayout(win)
        send(rawCommand: "resize-window -t \(win) -x \(lastClientSize.cols) -y \(lastClientSize.rows)")
        resizedWindows.insert(win)
        // `releaseWindowPins` handed the split layout back (which unzooms), so
        // the single-pane presentation has to be re-established here — it is no
        // longer left standing while we're away.
        ensureImmersiveZoom()
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
            self?.parseListSessions(response.lines)
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
    /// own). In control-plane mode these get the native look (status bar
    /// off); the hub restores them on disconnect.
    @ObservationIgnored
    private(set) var ownedSessionNames: Set<String> = []

    /// Session whose status bar is TEMPORARILY hidden because the active
    /// window is zoomed (immersive full-screen pane over mosh). Strictly
    /// zoom-scoped: suppression follows the `*Z` flag from %layout-change,
    /// so it tracks zooms/unzooms from ANY client (including Ctrl-b z in
    /// the shell) and is restored on unzoom, window/session switch away,
    /// and disconnect. Never persists.
    @ObservationIgnored
    private var statusSuppressedSessionId: String?

    /// Hide/restore the active session's status bar in lockstep with the
    /// zoom state. `-u` restores the inherited default, leaving the user's
    /// configuration untouched. No-ops outside control-plane mode.
    private func setStatusSuppressed(_ suppressed: Bool) {
        guard !rendersOutput else { return }
        if suppressed {
            guard statusSuppressedSessionId == nil,
                  let sessionId = snapshot.activeSessionId else { return }
            statusSuppressedSessionId = sessionId
            send(rawCommand: "set-option -t '\(sessionId)' status off")
        } else if let sessionId = statusSuppressedSessionId {
            statusSuppressedSessionId = nil
            send(rawCommand: "set-option -t '\(sessionId)' -u status")
        }
    }

    /// Disconnect-time status restore — over the LIVE -CC control channel, the
    /// same way suppression was applied.
    ///
    /// We previously restored over a one-shot SSH exec channel, but that runs
    /// through the remote login shell, which mangles the `set-option -t '$id'`
    /// session target (the `$id`) — so the status bar never came back. tmux's
    /// own control channel parses the command directly (no shell), so the
    /// quoting that worked for suppression works here too. A trailing
    /// round-trip blocks until tmux has applied it, so it can't race the
    /// teardown that follows (the original reason exec was chosen).
    func restoreSuppressedStatusAndFlush() async {
        guard snapshot.isAttached else { return }
        var targets = Array(ownedSessionNames)
        if let sessionId = statusSuppressedSessionId {
            targets.append(sessionId)
            statusSuppressedSessionId = nil
        }
        guard !targets.isEmpty else { return }
        for target in targets {
            send(rawCommand: "set-option -t '\(target)' -u status")
        }
        await flushControlChannel(timeout: 2)
    }

    /// Await a single tmux round-trip. Commands are processed in FIFO order, so
    /// once tmux answers this everything queued before it has been applied.
    /// Bounded so a dying channel can't hang teardown.
    private func flushControlChannel(timeout: TimeInterval) async {
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            var resumed = false
            func fire() { if !resumed { resumed = true; continuation.resume() } }
            sendCommand("display-message -p ok") { _ in fire() }
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                fire()
            }
        }
    }

    /// The session whose status bar is currently suppressed, if any.
    var suppressedStatusSessionId: String? { statusSuppressedSessionId }

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
        sendCommand("display-message -p -t '\(windowId)' '\(Self.pristineProbeFormat)'") { [weak self] response in
            guard let self, let line = response.lines.first else { return }
            let fields = line.split(separator: " ").map(String.init)
            // Same reply, two jobs: the pane count and zoom flag this needs,
            // and the pristine record — but only on the paths where the zoom is
            // all we do (mosh sidecar, a window we never pinned). Behind a pin,
            // this reply already describes the squashed layout, and the probe
            // `fitWindowToClient` queued ahead of that pin is the real record.
            if !self.resizedWindows.contains(windowId) {
                self.notePristineLayout(windowId, fields: fields)
            }
            guard fields.count >= 2, (Int(fields[0]) ?? 1) > 1, fields[1] == "0" else { return }
            self.send(rawCommand: "resize-pane -Z -t '\(windowId)'")
        }
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
        let namePart = trimmed.isEmpty ? "" : " -s \(tmuxQuote(trimmed))"
        sendCommand("new-session -d -P -F '#{session_id}'\(namePart)") { [weak self] response in
            guard let self else { return }
            if response.isError {
                self.flashNotice(response.lines.first ?? "new-session failed")
                return
            }
            // `isValidTmuxId` (not just `hasPrefix("$")`) so a malformed reply
            // can't slip a non-numeric tail into `set-option -t \(id)` below.
            guard let id = response.lines.first?.trimmingCharacters(in: .whitespaces),
                  Self.isValidTmuxId(id) else { return }
            if !self.rendersOutput, !trimmed.isEmpty {
                // mosh renders tmux's full TUI; a session Moshpit just created
                // adopts the native look (breadcrumb navigation, no status
                // bar). User-created sessions are never styled.
                self.send(rawCommand: "set-option -t \(id) status off")
                self.ownedSessionNames.insert(trimmed)
            }
            self.selectSession(id)
        }
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            self?.parseListSessions(response.lines)
        }
    }

    // MARK: - Rename / kill (Home session-tree long-press)

    /// Single-quote a tmux argument so names with spaces or shell-special
    /// characters survive the control channel. Embedded single quotes are
    /// closed, escaped, and reopened (`'…'\''…'`).
    private func tmuxQuote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    /// Rename a session, then re-list so the snapshot picks up the new name.
    /// No-op on an empty name (tmux would reject it anyway).
    func renameSession(_ sessionId: String, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        sendSurfacingErrors("rename-session -t \(sessionId) \(tmuxQuote(trimmed))", verb: "rename")
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            self?.parseListSessions(response.lines)
        }
    }

    /// Kill a session and re-list. tmux itself guards the last session (the
    /// server exits, which surfaces as the control channel closing); we don't
    /// block it here so the user keeps full control of their own sessions.
    func killSession(_ sessionId: String) {
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
            snapshot.activeSessionId = snapshot.sessions.keys.sorted().first
        }
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            self?.parseListSessions(response.lines)
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
        guard !trimmed.isEmpty else { return }
        sendSurfacingErrors("rename-window -t \(windowId) \(tmuxQuote(trimmed))", verb: "rename")
        refreshWindowsAndPanes()
    }

    /// Kill a window and re-discover. Local snapshot is also pruned by
    /// `handleWindowClose` on the `%window-close` notification.
    func killWindow(_ windowId: String) {
        sendSurfacingErrors("kill-window -t \(windowId)", verb: "kill window")
        refreshWindowsAndPanes()
    }

    /// Kill a single pane and re-discover. Panes have no user-facing name in
    /// tmux, so there is no rename counterpart.
    func killPane(_ paneId: String) {
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
            self?.parseListWindows(response.lines)
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
            self.parseListPanes(response.lines)
            self.ensureTerminalsForAllPanes(isFreshAttach: isFreshAttach)
            self.applyPendingRestoreIfReady()
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
    /// untouched.
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
    /// sequences on a fresh `-CC` attach (lab-verified, `scripts/tmux-cc-lab`,
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
    func sendPaste(_ text: String, paneId: String) {
        guard !text.isEmpty, Self.isValidTmuxId(paneId) else { return }
        let literal = Self.tmuxCommandStringLiteral(text)
        send(rawCommand: "set-buffer -b moshpit-paste -- \(literal)")
        send(rawCommand: "paste-buffer -p -b moshpit-paste -d -t \(paneId)")
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
            fitWindowToClient(win)
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
                let fields = resp.lines.first?.split(separator: " ").map(String.init) ?? []
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
                return
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
                if let win = self.snapshot.activeWindowId { self.fitWindowToClient(win) }
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
            // The frame is on screen — reveal the terminal if a keyboard-freeze
            // or pane-switch veil is covering it, UNLESS this capture is known
            // to race the app's own redraw (reveal == false): the buffer is
            // corrected either way, but showing THIS particular frame would
            // expose whatever the diffing TUI had drawn mid-repaint (a stray
            // duplicated or blank region) for the beat until the next capture
            // corrects it again — worse than staying covered a little longer.
            if reveal {
                self.paneCoordinators[paneId]?.reveal()
            }
        }
        sendCommand("display-message -p -t \(paneId) '#{cursor_x} #{cursor_y}'") { [weak self] resp in
            guard let self else { return }
            // The cursor is the SECOND half of the pair, so this is where the
            // resync is actually over.
            defer { self.finishResync(paneId) }
            let parts = resp.lines.first?.split(separator: " ").compactMap { Int($0) } ?? []
            let (col, row) = parts.count == 2 ? (parts[0], parts[1]) : (0, 0)
            // CUP is 1-based; cursor_x/y are 0-based.
            self.paneCoordinators[paneId]?.feed(data: Data("\u{1b}[\(row + 1);\(col + 1)H".utf8))
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
            self?.parseAgentHooks(response.lines)
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
            // maxSplits:4 preserves any literal `|` inside a user-set title.
            let parts = line.split(separator: "|", maxSplits: 5,
                                   omittingEmptySubsequences: false)
            guard parts.count == 6, !parts[0].isEmpty else { continue }
            let paneId = String(parts[0])
            let stateRaw = String(parts[1])
            // Only the three hook-emitted states are valid; anything else
            // (empty, or a garbage value) means "no precise hook data".
            let state: String?
            switch stateRaw {
            case "working", "attention", "done": state = stateRaw
            default: state = nil
            }
            let agent = parts[2].isEmpty ? nil : String(parts[2])
            // since may be empty even with a state if a stamp partially failed;
            // garbage parses to nil too. Callers substitute Date() for nil.
            let since = TimeInterval(parts[3]).map { Date(timeIntervalSince1970: $0) }
            let command = parts[4].isEmpty ? nil : String(parts[4])
            let title = parts[5].isEmpty ? nil : String(parts[5])
            parsed[paneId] = AgentHook(state: state, agent: agent, since: since,
                                       command: command, title: title)
        }
        agentHooks = parsed
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
        case sessionWindowChanged(sessionId: String, windowId: String)
        case activePaneChanged(windowId: String, paneId: String)
        case pause(String)
        case continued(String)
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
            onSessionWindowChanged: { emit.yield(.sessionWindowChanged(sessionId: $0, windowId: $1)) },
            onActivePaneChanged: { emit.yield(.activePaneChanged(windowId: $0, paneId: $1)) },
            onPause: { emit.yield(.pause($0)) },
            onContinue: { emit.yield(.continued($0)) },
            // NOTE: do NOT touch isAttached here. `%client-detached` is a
            // server-wide broadcast about SOME client on the server — often
            // an unrelated one (e.g. a desktop client on another session).
            // Acting on it dropped us to the empty state mid-navigation when
            // any other client detached. OUR control session ending arrives
            // as `%exit` (below) or the byte stream closing (detach()).
            onClientDetached: { _ in },
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
        case .sessionWindowChanged(let sessionId, let windowId):
            guard Self.isValidTmuxId(sessionId), Self.isValidTmuxId(windowId) else { return }
            // tmux broadcasts this for EVERY session on the server, including
            // ones other clients are attached to — only follow window changes
            // of OUR session.
            guard sessionId == snapshot.activeSessionId else { return }
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
            if currentIsOurs, current != windowId { return }
            snapshot.activeWindowId = windowId
            // Window changed: land immersive — zoom the new window's active
            // pane if it has splits.
            ensureImmersiveZoom()
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
        case .exit:
            snapshot.isAttached = false
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

        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            self?.parseListSessions(response.lines)
        }
        sendCommand("list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} #{window_active} #{window_panes} #{window_name}'") { [weak self] response in
            self?.parseListWindows(response.lines)
        }
        sendCommand("list-panes -a -F \(Self.paneDiscoveryFormat)") { [weak self] response in
            guard let self else { return }
            self.parseListPanes(response.lines)
            self.ensureTerminalsForAllPanes(isFreshAttach: true)
            self.applyPendingRestoreIfReady()
        }
    }

    private func parseListSessions(_ lines: [String]) {
        var newSessions: [String: SessionInfo] = [:]
        var activeId: String? = snapshot.activeSessionId
        for line in lines {
            // Format: "#{session_id} #{session_attached} #{session_name}" —
            // the name comes LAST because it may contain spaces.
            let parts = line.split(separator: " ", maxSplits: 2)
            guard parts.count >= 3 else { continue }
            let id = String(parts[0])
            // Every id in `snapshot` ultimately gets spliced unquoted into a
            // `-t <id>` control-mode argument elsewhere in this file — reject
            // a malformed reply here rather than let it in.
            guard Self.isValidTmuxId(id) else { continue }
            let isAttached = parts[1] != "0"
            let name = String(parts[2])
            newSessions[id] = SessionInfo(id: id, name: name, isAttached: isAttached)
            if isAttached { activeId = id }
        }
        snapshot.sessions = newSessions
        if let activeId { snapshot.activeSessionId = activeId }
    }

    private func parseListWindows(_ lines: [String]) {
        var newWindows: [String: WindowInfo] = [:]
        var tmuxActiveWindowId: String?
        for line in lines {
            // Format: "#{session_id} #{window_id} #{window_index}
            // #{window_layout} #{window_active} #{window_panes} #{window_name}"
            // — name LAST because programs rename windows to titles with spaces.
            let parts = line.split(separator: " ", maxSplits: 6)
            guard parts.count >= 7 else { continue }
            let sessionId = String(parts[0])
            let id = String(parts[1])
            // See parseListSessions — reject ids that don't match tmux's
            // real shape before they can reach a spliced -t argument.
            guard Self.isValidTmuxId(sessionId), Self.isValidTmuxId(id) else { continue }
            let index = Int(parts[2]) ?? 0
            let layout = String(parts[3])
            let isActive = parts[4] == "1"
            let paneCount = Int(parts[5]) ?? 1
            let name = String(parts[6])
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
        if !currentIsOurs, let tmuxActiveWindowId {
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
            // spaces.
            let parts = line.split(separator: " ", maxSplits: 8)
            guard parts.count >= 9 else { continue }
            let id = String(parts[0])
            let windowId = String(parts[1])
            // See parseListSessions — reject ids that don't match tmux's
            // real shape before they can reach a spliced -t argument.
            guard Self.isValidTmuxId(id), Self.isValidTmuxId(windowId) else { continue }
            let index = Int(parts[2]) ?? 0
            let width = Int(parts[3]) ?? 80
            let height = Int(parts[4]) ?? 24
            let isActive = parts[5] == "1"
            paneScreenState[id] = ScreenState(isAlternate: parts[6] == "1",
                                              isInCopyMode: parts[7] == "1")
            let command = String(parts[8])
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
                fitWindowToClient(win)
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
        if pinsReleased {
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
        if var window = snapshot.windows[windowId] {
            window.layout = layout
            window.isZoomed = zoomed
            snapshot.windows[windowId] = window
        } else {
            // Live notifications are for the attached session.
            snapshot.windows[windowId] = WindowInfo(
                id: windowId, sessionId: snapshot.activeSessionId ?? "",
                layout: layout, isZoomed: zoomed)
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
                reclaimWindowOnDrift(windowId)
                armForeignQuietRepin(windowId)
            } else if activeWindowAtForeignWidth {
                foreignQuietRepinTask?.cancel()
                foreignQuietRepinTask = nil
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
            self.fitWindowToClient(windowId)
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
        let fields = layout.split(separator: ",")
        guard fields.count >= 2 else { return nil }
        return fields[1].split(separator: "x").first.flatMap { Int($0) }
    }

    private func handleWindowAdd(_ windowId: String) {
        if snapshot.windows[windowId] == nil {
            snapshot.windows[windowId] = WindowInfo(
                id: windowId, sessionId: snapshot.activeSessionId ?? "")
        }
        // Re-list so we pick up name, index, layout, and any new panes.
        sendCommand("list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} #{window_active} #{window_panes} #{window_name}'") { [weak self] response in
            self?.parseListWindows(response.lines)
        }
        sendCommand("list-panes -a -F \(Self.paneDiscoveryFormat)") { [weak self] response in
            guard let self else { return }
            self.parseListPanes(response.lines)
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
        // The window we were showing died (killed here or by another client).
        // Land on a surviving window LOCALLY, and point activePaneId at one of
        // its panes right away — leaving it on a dead pane blanks the screen
        // until the next discovery. Deliberately NO select-window: tmux is
        // already moving its own current window, and pushing ours would yank
        // desktop clients around the same way we refuse to be yanked.
        let fallback = snapshot.sortedWindows.first?.id
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

    private func handleWindowRenamed(_ windowId: String, name: String) {
        guard var window = snapshot.windows[windowId] else { return }
        window.name = name
        snapshot.windows[windowId] = window
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
        // Control clients have no size until they report one; without it
        // tmux suppresses %output entirely. Report a sane default before
        // discovery — the real size follows from the terminal view layout.
        // Control-plane-only mode (mosh sidecar) must NOT report a size:
        // staying sizeless keeps %output suppressed (mosh renders the
        // screen) and keeps this client out of tmux's window-size math.
        if rendersOutput {
            send(rawCommand: "refresh-client -C \(lastClientSize.cols)x\(lastClientSize.rows)")
        } else {
            // mosh renders tmux's full TUI, so hide the redundant status bar
            // for the whole connection (restored on session switch/disconnect).
            setStatusSuppressed(true)
        }
        // Both modes show ONE full-screen pane: zoom the active pane once the
        // window list lands (initial attach / session switch).
        pendingImmersiveZoom = true
        sendCommand("list-sessions -F '#{session_id} #{session_attached} #{session_name}'") { [weak self] response in
            self?.parseListSessions(response.lines)
        }
        refreshWindowsAndPanes(isFreshAttach: isFreshAttach)
    }

    private func handleCommandResponse(_ response: TmuxCommandResponse) {
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

    /// Total replies popped — the watchdog's progress probe. A wedged parser
    /// (a reply block whose terminator was lost in transit) keeps INCOMING
    /// bytes flowing while pops stop dead, which the silence check alone
    /// can't see.
    @ObservationIgnored private var popCounter: UInt64 = 0

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
    private func sendCommand(_ command: String,
                             callback: @escaping (TmuxCommandResponse) -> Void) {
        enqueue(command, callback: callback)
    }

    private func enqueue(_ command: String, callback: ((TmuxCommandResponse) -> Void)?) {
        // Before tmux is attached, anything we write lands in the SHELL, not
        // tmux — it would type garbage at the prompt AND leave an orphan
        // callback slot that desyncs the response FIFO for the entire
        // session. Drop commands until control mode is live. (Layout-driven
        // resizeClient calls are the usual pre-attach offender.)
        guard snapshot.isAttached else {
            ccTapLine("DROP  (pre-attach) [\(command.prefix(50))]")
            return
        }
        guard let data = (command + "\n").data(using: .utf8) else { return }
        pendingCallbacks.append((command: command, callback: callback))
        ccTapLine("SEND  depth=\(pendingCallbacks.count) [\(command.prefix(50))]")
        let session = sshSession
        let previous = writeChain
        writeChain = Task(priority: .userInitiated) {
            await previous?.value
            try? await session.write(data)
        }
    }

    // MARK: - Terminal minting

    /// Create the persistent ``TerminalView`` + ``Coordinator`` pair for
    /// `paneId`, wire input/resize back through the controller, and stash
    /// both in the non-observed pool.
    @discardableResult
    private func mintTerminal(for paneId: String) -> TerminalView {
        if let existing = paneTerminals[paneId] {
            return existing
        }
        let font = TerminalFont.font(id: currentFontName, size: CGFloat(currentFontSize))
        // Born at the client's grid, not at `.zero`.
        //
        // A pane terminal is minted during discovery — before SwiftUI has laid
        // it out even once — and `TerminalView(frame: .zero)` derives its grid
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
        let cell = TerminalCellGeometry.measuredCell(
            font: font, scale: UITraitCollection.current.displayScale)
        let birthFrame = CGRect(x: 0, y: 0,
                                width: cell.width * CGFloat(max(1, lastClientSize.cols)),
                                height: cell.height * CGFloat(max(1, lastClientSize.rows)))
        let terminal = TerminalView(frame: birthFrame, font: font)
        terminal.inputAccessoryView = nil   // we render our own shortcut bar
        TerminalKeyboard.enableComposingInput(on: terminal)
        TerminalScrollback.enlarge(terminal)
        // OSC-8 only — SwiftTerm's default `.implicit` mis-underlines file/
        // relative paths and truncates URLs (see SwiftTerminalView.makeUIView).
        terminal.linkReporting = .explicit
        terminal.linkHighlightMode = .always   // OSC-8 hyperlinks open on a plain tap
        // We own scrolling via gestures + the scroll thumb, so SwiftTerm must
        // NOT also report touches as mouse drags — those flow through onInput,
        // spam the remote during a scroll, and (over mosh) toggle copy-mode out
        // from under us, so keystrokes never leave copy-mode.
        terminal.allowMouseReporting = false
        currentTheme.apply(to: terminal)
        let desiredCursor = TerminalCursor.apply(
            shape: cursorShape, colorId: cursorColorId, blink: cursorBlink, to: terminal)

        let coordinator = SwiftTerminalView.Coordinator()
        // Enforced after every feed — pane output (vim, zsh plugins, coding
        // agents) carries DECSCUSR/OSC-12 that would otherwise override the
        // user's Settings choice.
        coordinator.enforcedCursor = desiredCursor
        coordinator.onInput = { [weak self, paneId] data in
            Task { @MainActor [weak self] in
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
        coordinator.onScrollBegin = { [weak self] in
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
        // none of the resize-driven healing runs. The coordinator repaints the
        // local screen; also converge with tmux's model in case composing
        // teardown left real cell damage.
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
        terminal.terminalDelegate = coordinator
        coordinator.attach(to: terminal)

        paneTerminals[paneId] = terminal
        paneCoordinators[paneId] = coordinator
        return terminal
    }
}
