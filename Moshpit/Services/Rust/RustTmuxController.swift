import Foundation
import MoshpitKit
import MoshpitFFI
import Observation
import QuartzCore
import SwiftUI
import UIKit

/// tmux on the Rust core, as the terminal chrome reads tmux: an observable
/// surface — the tree, the notice, the refresh spinner, the size-lease
/// follower's veil, the agents' stamps — that mirrors the core's controller
/// (`TmuxReadModel`), pane terminals fed by their byte channels, and every
/// action an intent to the core. The breadcrumb, the sheets, the Home tree,
/// the follower's veil, swipes and scrolling read it.
///
/// It used to be a subclass of the Swift controller (`TmuxSessionController`)
/// that overrode its every action; that controller is gone (E3), and what
/// this one inherited from it lives here.
///
/// ### Contract
///   - The observation-tracked surface is ``snapshot`` and the chrome's
///     ``notice``, ``isRefreshing``, ``sizeFollow`` and ``agentHooksLoaded``.
///     The pane terminals are never observed: a window switch must not
///     destroy a terminal view.
///   - ``surface(for:)`` returns the same terminal for the same pane for the
///     pane's whole life.
///
/// One per attach: a reconnect's new attach gets a fresh one (fresh pane
/// terminals, as the core mints fresh channels), and the driver keeps the
/// previous one on screen, frozen, until the new one has painted.
///
/// Over mosh it is the `-CC` sidecar's surface (`sidecar`): the tree and
/// the actions only — mosh paints tmux's own client on the session's own
/// terminal, so no pane terminal is ever made here.
///
/// What a pane's view must do comes in its channel as marks, exactly where
/// the core's controller asked for them among the pane's bytes: a veil
/// before a switch's stale frame, the reveal after the repaint, a held
/// resize applied before the frame captured at the new size. The held
/// resize is applied as it is read (the terminal must take the new size
/// before the capture's bytes); every other mark goes into the pane's
/// terminal among its bytes and is acted on once the bytes before it are on
/// screen (``CoreTerminalView/feed(mark:)``) — with the frame that has them,
/// the core parsing on a thread of its own: a reveal never lifts the veil
/// off a repaint not yet drawn.
///
/// A pane's terminal is the core's view (docs/nsh/TERMINAL-VIEW.md).
@Observable
@MainActor
final class RustTmuxController: MultiplexerControlling {
    /// `MultiplexerControlling.multiplexer`: the vocabulary and key hints
    /// the sheets print.
    nonisolated var multiplexer: Multiplexer { .tmux }

    typealias Snapshot = TmuxSnapshot

    // MARK: What the chrome observes (mirrored from the core)

    private(set) var snapshot = Snapshot()
    /// tmux's last word, or what the core refused (B250).
    private(set) var notice: String?
    private(set) var isRefreshing = false
    /// The window another device holds the size lease on, while this one
    /// follows it (the follower's veil).
    private(set) var sizeFollow: SizeFollow?
    struct SizeFollow: Equatable {
        var windowId: String
        var ownerLabel: String
    }
    /// Each pane's agent stamps (`@moshpit_*`), as the core last read them.
    @ObservationIgnored private(set) var agentHooks: [String: AgentHook] = [:]
    private(set) var agentHooksLoaded = false

    // MARK: Who listens

    /// A pane wrote output (the agent monitor's heuristic).
    @ObservationIgnored var onPaneActivity: ((String) -> Void)?
    /// A pane rang the bell.
    @ObservationIgnored var onPaneBell: ((String) -> Void)?
    /// The agent stamps were read again.
    @ObservationIgnored var onAgentHooksUpdated: (() -> Void)?
    /// A pinch settled on a font size.
    @ObservationIgnored var onFontSizeCommitted: ((Double) -> Void)?
    /// A host path in a pane was tapped.
    @ObservationIgnored var onOpenRemotePath: ((String) -> Void)?

    /// The look new pane terminals take (and existing ones are moved to).
    struct Appearance: Equatable {
        var theme: TerminalTheme
        var fontSize: Double
        var fontName: String
        var cursorShape: CursorShape
        var cursorColorId: String
        var cursorBlink: Bool

        var terminalLook: TerminalLook {
            TerminalLook(theme: theme, cursorShape: cursorShape, cursorColorId: cursorColorId,
                         cursorBlink: cursorBlink)
        }
    }

    /// The core's attach this surface shows; nil until the attach lists its
    /// panes (the surface is up before tmux confirms, for the empty state).
    @ObservationIgnored var attach: UInt64?
    @ObservationIgnored private weak var driver: RustSessionDriver?
    @ObservationIgnored private var look: Appearance
    /// The grid pane terminals are born at, until their view reports one.
    @ObservationIgnored private let birthGrid: (cols: Int, rows: Int)
    @ObservationIgnored private var panes: [String: PaneTerminal] = [:]
    @ObservationIgnored private var model: TmuxReadModel?
    @ObservationIgnored private var rowCarry = ScrollRowCarry()
    /// Whether a pane of this attach has been revealed.
    @ObservationIgnored private(set) var paintedSinceAttach = false
    /// What the core refused, worded here (B250): the notice, for as long
    /// as one of tmux's own lasts, unless tmux has one of its own to say.
    @ObservationIgnored private var refusal: String?
    @ObservationIgnored private var refusalClear: Task<Void, Never>?

    /// A pane's persistent terminal and what feeds it.
    private final class PaneTerminal {
        let terminal: CoreTerminalView
        let coordinator: CoreTerminalHost.Coordinator
        var feeder: RustPaneFeeder?
        /// The grid its view last reported, sent when its channel arrives.
        var grid: (cols: Int, rows: Int)?
        var deferSafety: Task<Void, Never>?

        init(terminal: CoreTerminalView, coordinator: CoreTerminalHost.Coordinator) {
            self.terminal = terminal
            self.coordinator = coordinator
        }
    }

    /// The mosh sidecar's surface: no pane terminals.
    @ObservationIgnored let sidecar: Bool
    /// `driver` nil: a surface with no core behind it (the stage's tests),
    /// whose actions go nowhere.
    init(driver: RustSessionDriver?, appearance: Appearance, grid: (cols: Int, rows: Int),
         sidecar: Bool = false) {
        self.driver = driver
        look = appearance
        birthGrid = grid
        self.sidecar = sidecar
    }

    // MARK: Mirroring the core

    /// The core's view of tmux, where the chrome reads it.
    func mirror(_ model: TmuxReadModel) {
        self.model = model
        var snapshot = TmuxSnapshot()
        for session in model.sessions {
            snapshot.sessions[session.id] = SessionInfo(
                id: session.id, name: session.name, isAttached: session.isAttached)
        }
        for window in model.windows {
            snapshot.windows[window.id] = WindowInfo(
                id: window.id, sessionId: window.sessionId, name: window.name,
                index: Int(window.index), layout: window.layout, isActive: window.isActive,
                paneCount: Int(window.paneCount), isZoomed: window.isZoomed)
        }
        for pane in model.panes {
            snapshot.panes[pane.id] = PaneInfo(
                id: pane.id, windowId: pane.windowId, index: Int(pane.index),
                command: pane.command, width: Int(pane.width), height: Int(pane.height),
                isActive: pane.isActive, path: pane.path)
        }
        snapshot.activeSessionId = model.activeSessionId
        snapshot.activeWindowId = model.activeWindowId
        snapshot.activePaneId = model.activePaneId
        snapshot.isAttached = model.isAttached
        snapshot.everAttached = model.everAttached
        snapshot.lastSwitchForward = model.lastSwitchForward
        snapshot.hasSwipedSinceAttach = model.hasSwipedSinceAttach
        var hooks: [String: AgentHook] = [:]
        for hook in model.agentHooks {
            hooks[hook.pane] = AgentHook(
                state: hook.state, agent: hook.agent,
                since: hook.since.map { Date(timeIntervalSince1970: $0) },
                command: hook.command, title: hook.title)
        }
        let follow = model.sizeFollowWindow.map {
            SizeFollow(windowId: $0, ownerLabel: model.sizeFollowOwner ?? "")
        }
        publish(snapshot: snapshot, notice: model.notice ?? refusal, isRefreshing: model.refreshing,
                sizeFollow: follow, agentHooks: hooks, agentHooksLoaded: model.agentHooksLoaded)
    }

    /// What the chrome reads, set only where it changed (each set is a
    /// SwiftUI invalidation); a pane switch animates.
    private func publish(snapshot new: Snapshot, notice: String?, isRefreshing: Bool,
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

    /// An intent to the core; one it refused says why, as the notice.
    private func perform(_ intent: TmuxIntent) {
        guard let refused = driver?.perform(intent) else { return }
        noteRefused(RustCopy.tmuxRefusal(refused))
    }

    /// Shows `words` as the notice for three seconds, as tmux's own last.
    func noteRefused(_ words: String) {
        refusal = words
        showNotice()
        refusalClear?.cancel()
        refusalClear = Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled, let self else { return }
            self.refusal = nil
            self.showNotice()
        }
    }

    /// The notice as it stands: tmux's own, else the refusal.
    private func showNotice() {
        publish(snapshot: snapshot, notice: model?.notice ?? refusal, isRefreshing: isRefreshing,
                sizeFollow: sizeFollow, agentHooks: agentHooks, agentHooksLoaded: agentHooksLoaded)
    }

    /// The current attach's panes and their channels: each pane terminal
    /// (made now or already shown) starts reading its channel.
    func updatePanes(_ list: [PaneValue]) {
        guard !sidecar else { return }
        for info in list {
            let pane = paneTerminal(info.pane)
            guard pane.feeder?.stream != info.stream else { continue }
            pane.feeder?.detach()
            let feeder = RustPaneFeeder(stream: info.stream, coordinator: pane.coordinator)
            let paneId = info.pane
            feeder.onMark = { [weak self] mark in self?.read(mark, for: paneId) }
            pane.feeder = feeder
            if let grid = pane.grid {
                feeder.resize(cols: grid.cols, rows: grid.rows)
            }
        }
        // A pane whose window closed has no channel any more.
        let live = Set(list.map(\.pane))
        for (paneId, pane) in panes where !live.contains(paneId) && pane.feeder != nil {
            pane.feeder?.detach()
            pane.feeder = nil
        }
    }

    func notePainted() {
        paintedSinceAttach = true
    }

    /// Stops reading every channel (the surface is done: replaced, or the
    /// session closed).
    func detachAll() {
        for pane in panes.values {
            pane.feeder?.detach()
            pane.feeder = nil
            pane.deferSafety?.cancel()
        }
    }

    /// The pane input goes to when nobody says.
    var inputPaneId: String? {
        snapshot.activePaneId ?? snapshot.activePanes.first?.id
    }

    /// Keys for the pane on screen.
    func sendToActivePane(_ data: Data) {
        guard let paneId = inputPaneId else { return }
        _ = write(data, toPane: paneId)
    }

    /// The shortcut bar's keys for the pane on screen (B476: never kept for
    /// a reconnect to deliver later).
    func sendBarToActivePane(_ data: Data) {
        guard let paneId = inputPaneId, let feeder = panes[paneId]?.feeder else { return }
        feeder.writeBar(data)
    }

    /// Bytes into `paneId`'s channel; false when it has none.
    @discardableResult
    func write(_ data: Data, toPane paneId: String) -> Bool {
        guard let feeder = panes[paneId]?.feeder else {
            Log.input.info("rust tmux: \(data.count, privacy: .public) bytes for \(paneId, privacy: .public) before its channel — not sent")
            return false
        }
        return feeder.write(data)
    }

    /// A paste into `paneId`'s channel, in its place among the keys typed
    /// there (B361). A pane without a channel yet gets it as an intent, as
    /// before.
    func paste(_ text: String, toPane paneId: String) {
        if let feeder = panes[paneId]?.feeder, feeder.paste(text) { return }
        driver?.perform(.paste(pane: paneId, text: text))
    }

    // MARK: Marks

    /// The mark codes of crates/session's `view_marks` (tmux_leg.rs).
    typealias Mark = TerminalMark.Code

    /// A mark read from `paneId`'s channel, after the bytes before it were
    /// fed: a held resize now (the terminal takes the size before the
    /// capture's bytes come), any other among the bytes, for when they are
    /// on screen.
    func read(_ mark: MpStreamMark, for paneId: String) {
        guard let pane = panes[paneId] else { return }
        if mark.code == Mark.applyDeferredResize {
            pane.deferSafety?.cancel()
            pane.deferSafety = nil
            pane.coordinator.applyDeferredResize()
            return
        }
        pane.coordinator.feed(mark: TerminalMark(code: mark.code, arg: mark.arg))
    }

    /// A mark of `paneId`'s whose bytes are on screen.
    private func apply(_ mark: TerminalMark, to paneId: String) {
        guard let pane = panes[paneId] else { return }
        let coordinator = pane.coordinator
        switch mark.code {
        case Mark.veil:
            coordinator.veilForSwitch()
        case Mark.veilUnlessCovered:
            if !coordinator.isCoverPresented { coordinator.veilForSwitch() }
        case Mark.extendCover:
            coordinator.extendCoverTimeout(by: TimeInterval(mark.arg) / 1000)
        case Mark.reveal:
            coordinator.reveal()
        case Mark.releaseScrollHold:
            coordinator.releaseScrollHold()
        case Mark.scrollLocal:
            coordinator.scrollLocal(lines: Int(mark.arg))
        case Mark.settleLocalScroll:
            coordinator.settleLocalScroll()
        default:
            break
        }
    }

    // MARK: Pane terminals

    private func paneTerminal(_ paneId: String) -> PaneTerminal {
        if let pane = panes[paneId] { return pane }
        let font = TerminalFont.font(id: look.fontName, size: CGFloat(look.fontSize))
        // Born at the client's grid, not at `.zero` (see the Swift
        // controller's mintTerminal): bytes that land before the first layout
        // are parsed in the shape tmux drew them for.
        let terminal = TerminalSurfaces.make(font: font,
                                             grid: TerminalGrid(cols: birthGrid.cols, rows: birthGrid.rows))
        terminal.followTmuxClears()
        terminal.apply(look: look.terminalLook)
        let coordinator = CoreTerminalHost.Coordinator()
        let pane = PaneTerminal(terminal: terminal, coordinator: coordinator)
        wire(coordinator, for: paneId)
        coordinator.attach(terminal)
        panes[paneId] = pane
        return pane
    }

    private func wire(_ coordinator: CoreTerminalHost.Coordinator, for paneId: String) {
        // Keys go straight into the pane's channel, synchronously: no hop on
        // the byte path. The core drops what is typed while not live.
        coordinator.onInput = { [weak self] data in
            MainActor.assumeIsolated { _ = self?.write(data, toPane: paneId) }
        }
        coordinator.onReveal = { [weak self] in
            self?.driver?.perform(.viewRevealed(pane: paneId))
        }
        coordinator.onMark = { [weak self] mark in self?.apply(mark, to: paneId) }
        coordinator.onGridReport = { [weak self] cols, rows in
            self?.report(paneId, cols: cols, rows: rows)
        }
        coordinator.onSizeChange = { [weak self] cols, rows in
            MainActor.assumeIsolated { self?.report(paneId, cols: cols, rows: rows) }
        }
        coordinator.shouldDeferResize = { [weak self] cols, rows in
            self?.holdResize(paneId, cols: cols, rows: rows) ?? false
        }
        coordinator.onBell = { [weak self] in
            MainActor.assumeIsolated { self?.onPaneBell?(paneId) }
        }
        coordinator.onScroll = { [weak self] lines in self?.scroll(lines: lines) }
        coordinator.onScrollPixels = { [weak self] dy in self?.scroll(pixels: dy) ?? 0 }
        coordinator.onScrollEnd = { [weak self] in self?.scrollDidEnd() }
        coordinator.onScrollBegin = { [weak self] in
            self?.rowCarry.reset()
            self?.driver?.perform(.scrollBegan)
        }
        coordinator.onClick = { [weak self] col, row in self?.click(col: col, row: row) }
        coordinator.onSwitch = { [weak self] forward in self?.switchPaneOrWindow(forward: forward) }
        coordinator.onInputModeChange = { [weak self] in
            self?.driver?.perform(.inputModeChanged(pane: paneId))
        }
        coordinator.onFontSizeCommit = { [weak self] size in self?.onFontSizeCommitted?(size) }
        coordinator.onReturnSend = { [weak self] in
            self?.driver?.perform(.returnSent(pane: paneId))
        }
        // Host paths in the pane's output: confirmed over the session's own
        // connection, previewed on a tap (W14b).
        coordinator.pathLinks = pathLinksFactory?(paneId)
        coordinator.onOpenRemotePath = { [weak self] path in self?.onOpenRemotePath?(path) }
    }

    /// Host paths in a pane's output, confirmed for a link (W14b); panes
    /// made before the session supplied it get theirs when it does.
    @ObservationIgnored var pathLinksFactory: ((_ paneId: String) -> RemotePathLinks)? {
        didSet {
            for (paneId, pane) in panes where pane.coordinator.pathLinks == nil {
                pane.coordinator.pathLinks = pathLinksFactory?(paneId)
            }
        }
    }

    /// A pane view's grid: to its channel (the core sizes the client from
    /// the active pane's), or kept until the channel arrives.
    private func report(_ paneId: String, cols: Int, rows: Int) {
        guard let pane = panes[paneId] else { return }
        pane.grid = (cols, rows)
        pane.feeder?.resize(cols: cols, rows: rows)
    }

    /// The view is about to resize: hold it until the repaint that answers
    /// it (shrinking a buffer locally loses the rows below the cursor — an
    /// agent's footer — that tmux's capture still has). The core resizes the
    /// client and releases the hold with a mark when the capture lands, or
    /// at once when none is coming; a safety releases it if no mark ever
    /// does (a channel that died meanwhile).
    private func holdResize(_ paneId: String, cols: Int, rows: Int) -> Bool {
        guard snapshot.isAttached, let pane = panes[paneId], pane.feeder != nil,
              snapshot.activePaneId == paneId || snapshot.activePanes.count <= 1
        else { return false }
        if let grid = pane.grid, grid == (cols, rows) { return false }
        pane.grid = (cols, rows)
        driver?.perform(.deferResize(pane: paneId, cols: UInt16(clamping: cols),
                                     rows: UInt16(clamping: rows)))
        pane.deferSafety?.cancel()
        pane.deferSafety = Task { @MainActor [weak pane] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled, let pane else { return }
            pane.coordinator.applyDeferredResize()
        }
        return true
    }

    // MARK: The controller's surface, on the core

    func configureAppearance(theme: TerminalTheme, fontSize: Double,
                             fontName: String = "system",
                             cursorShape: CursorShape = .block,
                             cursorColorId: String = "teal",
                             cursorBlink: Bool = true) {
        configure(Appearance(theme: theme, fontSize: fontSize, fontName: fontName,
                             cursorShape: cursorShape, cursorColorId: cursorColorId,
                             cursorBlink: cursorBlink))
    }

    /// Only what changed is applied: a font or colour set again costs a
    /// pane a whole-screen repaint for nothing.
    func configure(_ appearance: Appearance) {
        guard appearance != look else { return }
        let old = look
        look = appearance
        let font = TerminalFont.font(id: appearance.fontName, size: CGFloat(appearance.fontSize))
        for pane in panes.values {
            if old.fontName != appearance.fontName || old.fontSize != appearance.fontSize {
                pane.terminal.apply(font: font)
            }
            // The theme only when it changed (each is a whole-screen
            // repaint); the cursor as the user has it.
            pane.terminal.apply(look: appearance.terminalLook)
        }
    }

    func surface(for paneId: String) -> CoreTerminalView {
        paneTerminal(paneId).terminal
    }

    func coordinator(for paneId: String) -> CoreTerminalHost.Coordinator? {
        panes[paneId]?.coordinator
    }

    var activeCoordinator: CoreTerminalHost.Coordinator? {
        inputPaneId.flatMap { panes[$0]?.coordinator }
    }

    func selectWindow(_ windowId: String) { driver?.perform(.selectWindow(id: windowId)) }
    func selectSession(_ sessionId: String) { driver?.perform(.selectSession(id: sessionId)) }
    func selectPane(_ paneId: String) { driver?.perform(.selectPane(id: paneId)) }
    func switchPaneOrWindow(forward: Bool) {
        driver?.perform(.switchPaneOrWindow(forward: forward))
    }
    func newWindow(named name: String? = nil) { perform(.newWindow(name: name)) }
    func newWindow(inSession sessionId: String, named name: String?) {
        perform(.newWindowInSession(session: sessionId, name: name))
    }
    func newPane() { driver?.perform(.newPane) }
    func newPane(inWindow windowId: String) {
        driver?.perform(.newPaneInWindow(window: windowId))
    }
    func newSession(named name: String? = nil) { perform(.newSession(name: name)) }
    func renameSession(_ sessionId: String, to name: String) {
        perform(.renameSession(session: sessionId, name: name))
    }
    func killSession(_ sessionId: String) { driver?.perform(.killSession(id: sessionId)) }
    func renameWindow(_ windowId: String, to name: String) {
        perform(.renameWindow(window: windowId, name: name))
    }
    func killWindow(_ windowId: String) { driver?.perform(.killWindow(id: windowId)) }
    func killPane(_ paneId: String) { driver?.perform(.killPane(id: paneId)) }
    func refresh() { driver?.perform(.refresh) }
    func pollAgentHooks() { driver?.perform(.pollAgentHooks) }
    func takeSizeOwnership() { driver?.perform(.takeSizeOwnership) }

    func sendInput(_ data: Data, paneId: String) {
        write(data, toPane: paneId)
    }

    func sendPaste(_ text: String, paneId: String) {
        paste(text, toPane: paneId)
    }

    // Scrolling: a tick goes where the core routes it; a drag's pixels go
    // where the active pane's mirrored flags say, the local scrollback moving
    // by the exact distance (the Swift controller's `scroll(pixels:)`).

    private enum ScrollRoute { case wheel, local, drop }

    /// Where a drag's pixels go: the core's rule (crates/tmux
    /// `scroll_route`, its verdicts mirrored), plus what only the view knows
    /// (B363, the 11-01 fixes):
    /// - a program that scrolls itself (an alternate-screen mouse app;
    ///   Claude with the mouse, wherever it draws) gets the wheel, even
    ///   while it repaints — its own history is the conversation, the
    ///   pane's local rows are leftovers of its repaints;
    /// - Claude on the primary screen whose flags haven't landed gets
    ///   nothing rather than a guess (the attach's first repaint brings
    ///   them);
    /// - a primary-screen pane already reading its local history keeps
    ///   reading there until it is back at the bottom (the streaming check
    ///   flips every half second for a ticking status line);
    /// - another mouse app gets the wheel while it isn't repainting (its
    ///   repaint would fight the wheel over the viewport);
    /// - an alternate-screen pane otherwise its rows go to the core, whose
    ///   controller presses the arrow keys once tmux has said the mouse is
    ///   off (B500: less, man, git log), else nothing; a shell its history.
    private func scrollRoute(_ pane: PaneTerminal) -> ScrollRoute {
        let wantsMouse = model?.wantsMouse ?? false
        let alternate = model?.altScreen ?? false
        if model?.scrollsItself == true { return .wheel }
        if model?.holdsTicks == true { return .drop }
        if !alternate, pane.coordinator.surface?.isReadingScrollback == true { return .local }
        let streaming = CACurrentMediaTime() - (pane.feeder?.lastFedAt ?? 0) < 0.5
        if wantsMouse, !streaming { return .wheel }
        return alternate ? .wheel : .local
    }

    func scroll(lines: Int) {
        guard lines != 0 else { return }
        driver?.perform(.scrollLines(lines: Int64(lines)))
    }

    @discardableResult
    func scroll(pixels dy: CGFloat) -> CGFloat {
        guard dy != 0, dy.isFinite, let paneId = inputPaneId, let pane = panes[paneId] else { return 0 }
        switch scrollRoute(pane) {
        case .wheel:
            // The program scrolls on the live screen: the view goes there
            // first (a drag begun in the local history before the flags
            // landed must not leave it up there).
            pane.coordinator.releaseScrollHold()
            let rows = rowCarry.take(pixels: dy, cellHeight: pane.coordinator.cellHeight)
            if rows != 0 { driver?.perform(.scrollLines(lines: Int64(rows))) }
            return dy
        case .drop:
            return 0
        case .local:
            return pane.coordinator.scrollLocal(pixels: dy)
        }
    }

    func scrollDidEnd() {
        guard let paneId = inputPaneId, let pane = panes[paneId] else { return }
        if scrollRoute(pane) == .local {
            pane.coordinator.settleLocalScroll()
            return
        }
        // An alternate-screen pane, or Claude drawing full-screen, has
        // nothing worth reading above its live screen: whatever left the
        // view up there, the drag's end puts it back.
        if model?.altScreen == true || model?.scrollsItself == true || model?.holdsTicks == true {
            pane.coordinator.releaseScrollHold()
        }
    }

    func click(col: Int, row: Int) {
        driver?.perform(.click(col: Int64(col), row: Int64(row)))
    }

    // A pane's working directory (path previews, the Files key) is asked of
    // tmux over the session's own connection. The window pins and the
    // writes are the core's: it hands the pins back on the screen's and the
    // app's comings and goings itself.

    func panePath(_ paneId: String) async -> String? {
        await driver?.side.panePath(paneId)
    }
}

