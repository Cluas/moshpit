import MoshpitKit
import SwiftTerm
import UIKit

/// SwiftTerm's view behind the app's seam (``TerminalSurface``): the SwiftTerm
/// fork's `TerminalView` and everything the app does to it that is SwiftTerm's
/// alone — the input setup every terminal gets (``TerminalMint``), the
/// one-finger scroll and pinch (``TerminalScrollGesture``), holding output
/// while history is read (SwiftTerm follows the tail on every line), the
/// user's cursor re-asserted after every feed, the link passes that tag cells
/// (``PlainLinkDetector``, ``PathLinkDetector``), and mosh's predictive echo
/// written into its cells (``MoshPrediction``).
///
/// This folder (UI/Terminal/SwiftTerm/) is the only code that imports
/// SwiftTerm. Everything here was the session coordinator's until T3 put the
/// seam in; it does what it did there, call for call.
///
/// SwiftTerm parses as it is fed, on the caller's thread, so a mark fed
/// among the bytes is due at once: ``feed(mark:)`` hands it straight back.
@MainActor
final class SwiftTermSurface: NSObject, TerminalSurface {
    let terminalView: TerminalView
    weak var surfaceDelegate: TerminalSurfaceDelegate?
    var view: UIView { terminalView }

    /// A new terminal, born at `frame`, configured as every terminal of the
    /// app is.
    convenience init(frame: CGRect, font: UIFont) {
        let view = TerminalView(frame: frame, font: font)
        TerminalMint.configureInput(view)
        self.init(wrapping: view)
    }

    /// `view` as a surface: its delegate, its scroll and pinch.
    init(wrapping view: TerminalView) {
        terminalView = view
        super.init()
        view.terminalDelegate = self
        TerminalScrollGesture.attach(to: view, surface: self)
    }

    // MARK: Bytes

    func feed(_ data: Data) {
        // While the user reads scrollback, hold new output instead of
        // letting it yank the viewport to the bottom. Replay on release.
        if scrollHeld {
            TmuxSessionController.ccTap("cc-pairing.log", Data("FEED-HELD bytes=\(data.count)\n".utf8))
            heldFeed.append(data)
            heldBytes += data.count
            if heldBytes > heldByteLimit { releaseScrollHold() }
            return
        }
        liftingPredictions {
            terminalView.feed(byteArray: ArraySlice([UInt8](data)))
        }
        postFeedFixups()
    }

    func feed(mark: TerminalMark) {
        surfaceDelegate?.surface(self, reached: mark)
    }

    func discardPendingInput() {
        terminalView.getTerminal().discardPendingInput()
    }

    /// SwiftTerm's view keeps its own rule (the core's view mirrors tmux).
    func followTmuxClears() {}

    // MARK: The grid and the look

    var font: UIFont { terminalView.font }

    func apply(font: UIFont) {
        TerminalFont.apply(font, to: terminalView)
    }

    /// The look last applied (the theme is re-applied only when it changed,
    /// SwiftTerm's colour setters each repaint the whole screen).
    private(set) var appliedLook: TerminalLook?

    /// The user's cursor (Settings), re-asserted after every feed. Remote
    /// output carries DECSCUSR / OSC 12 (vim mode switches, zsh plugins,
    /// coding agents) and SwiftTerm applies them to the caret just like
    /// ours — without enforcement the user's choice only survives until the
    /// next such sequence.
    private(set) var enforcedCursor: (style: CursorStyle, color: UIColor)?

    func apply(look: TerminalLook) {
        guard look != appliedLook else { return }
        if appliedLook?.theme != look.theme {
            look.theme.apply(to: terminalView)
        }
        applyCursor(look)
    }

    func reassert(look: TerminalLook) {
        look.theme.apply(to: terminalView)
        applyCursor(look)
    }

    private func applyCursor(_ look: TerminalLook) {
        // After the theme (which sets caretColor too).
        enforcedCursor = TerminalCursor.apply(shape: look.cursorShape, colorId: look.cursorColorId,
                                              blink: look.cursorBlink, theme: look.appTheme,
                                              to: terminalView)
        appliedLook = look
    }

    var cellSize: CGSize { TerminalCellGeometry.cellSize(of: terminalView) }

    var grid: TerminalGrid {
        let t = terminalView.getTerminal()
        return TerminalGrid(cols: t.cols, rows: t.rows)
    }

    /// The grid SwiftTerm will lay `size` out to: its cell box from the font
    /// (measured as SwiftTerm measures it), truncating division.
    func grid(fitting size: CGSize) -> TerminalGrid {
        let scale = terminalView.window?.screen.scale ?? UITraitCollection.current.displayScale
        let cell = TerminalCellGeometry.measuredCell(font: terminalView.font, scale: scale)
        let g = TerminalCellGeometry.grid(for: size, cell: cell)
        return TerminalGrid(cols: g.cols, rows: g.rows)
    }

    func viewportCell(at point: CGPoint) -> TerminalCell {
        let c = TerminalCellGeometry.cell(at: point, in: terminalView)
        return TerminalCell(col: c.col, row: c.row)
    }

    var cursor: TerminalCell {
        let c = terminalView.getTerminal().getCursorLocation()
        return TerminalCell(col: c.x, row: c.y)
    }

    var modes: TerminalModes {
        let t = terminalView.getTerminal()
        return TerminalModes(applicationCursorKeys: t.applicationCursor, bracketedPaste: t.bracketedPasteMode,
                             mouseReporting: t.mouseMode != .off)
    }

    // MARK: History

    /// Output-hold for reading scrollback (see ``TerminalScrollGesture``).
    /// While the user is scrolled up, new output would otherwise snap the
    /// viewport to the bottom (SwiftTerm follows the tail on every line), so
    /// it is buffered here and replayed in order when they return to the
    /// bottom or type. Capped so a long read can't grow memory without
    /// bound — past the cap the hold gives up and live output resumes.
    private var scrollHeld = false
    private var heldFeed: [Data] = []
    private var heldBytes = 0
    private let heldByteLimit = 1_000_000

    var isReadingScrollback: Bool { terminalView.canScroll && terminalView.scrollPosition < 0.999 }

    /// The scroll view's content is every line SwiftTerm keeps, a cell high
    /// each; the screen's rows are not history.
    var historyLines: Int {
        let cell = terminalView.cellHeight
        guard cell > 0 else { return 0 }
        return max(0, Int((terminalView.contentSize.height / cell).rounded()) - terminalView.getTerminal().rows)
    }

    func scrollLocal(lines: Int) {
        guard lines != 0 else { return }
        // No buffer is 2^31 rows deep; bounded, `-lines` can't trap on
        // Int.min and SwiftTerm's `yDisp + lines` can't overflow (B149).
        let count = Int(min(lines.magnitude, UInt(Int32.max)))
        if lines > 0 {
            terminalView.scrollUp(lines: count)
        } else {
            terminalView.scrollDown(lines: count)
        }
        updateScrollHold()
    }

    func scrollLocal(pixels dy: CGFloat) -> CGFloat {
        guard dy != 0 else { return 0 }
        let moved = terminalView.scrollViewport(by: dy)
        updateScrollHold()
        return moved
    }

    func settleLocalScroll() {
        let gap = terminalView.distanceToBottom
        if gap > 0, gap < terminalView.cellHeight / 2 {
            terminalView.scrollViewport(by: -gap)
        }
        updateScrollHold()
    }

    /// Sub-row finger travel not yet turned into a wheel tick or a host row.
    private var rowCarry = ScrollRowCarry()

    func dragWillBegin() {
        rowCarry.reset()
    }

    /// V20 the SwiftTerm way: the host's rows, a mouse app's wheel, else the
    /// local history by the exact distance (sub-row offsets included, so the
    /// content tracks the finger); the row routes carry the remainder.
    func drag(_ dy: CGFloat, hostRows: Bool) -> (moved: CGFloat, rows: Int) {
        guard dy != 0, dy.isFinite else { return (0, 0) }
        let cellHeight = terminalView.cellHeight > 0 ? terminalView.cellHeight : 16
        if hostRows {
            return (dy, rowCarry.take(pixels: dy, cellHeight: cellHeight))
        }
        if modes.mouseReporting {
            let rows = rowCarry.take(pixels: dy, cellHeight: cellHeight)
            if rows != 0 { sendWheel(lines: rows) }
            return (dy, 0)
        }
        return (scrollLocal(pixels: dy), 0)
    }

    /// Hold live output exactly while history is on screen. `canScroll`
    /// first: `scrollPosition` is 0 both at the very top AND when there is
    /// nothing to scroll, so on its own a swipe over an empty pane would
    /// freeze its output until the next keystroke.
    private func updateScrollHold() {
        if terminalView.canScroll, terminalView.scrollPosition < 0.999 {
            engageScrollHold()
        } else {
            releaseScrollHold()
        }
    }

    /// Begin holding output — the user has scrolled up off the bottom.
    private func engageScrollHold() {
        if !scrollHeld, let prediction {
            // Guesses address the live screen's rows, which just scrolled
            // out from under them.
            if prediction.withdraw(from: terminalView.getTerminal()) { terminalView.feed(byteArray: []) }
            prediction.reset()
        }
        scrollHeld = true
    }

    /// Stop holding and replay everything buffered since the hold began, in
    /// order, so the viewport catches up to live output. Idempotent.
    func releaseScrollHold() {
        guard scrollHeld else { return }
        scrollHeld = false
        liftingPredictions {
            for chunk in heldFeed {
                terminalView.feed(byteArray: ArraySlice([UInt8](chunk)))
            }
        }
        heldFeed.removeAll()
        heldBytes = 0
        postFeedFixups()
    }

    func discardScrollHold() {
        scrollHeld = false
        heldFeed.removeAll()
        heldBytes = 0
    }

    // MARK: The program's mouse and paste

    /// SwiftTerm's encoder emits wheel events per the app's negotiated mouse
    /// protocol and routes them out through `send` — the same path as a
    /// typed key, so no copy-mode is involved. Button 64 = wheel-up (older),
    /// 65 = wheel-down (newer); clamped so a fast flick doesn't fire dozens.
    func sendWheel(lines: Int) {
        guard lines != 0 else { return }
        let terminal = terminalView.getTerminal()
        let button = lines > 0 ? 64 : 65
        // `magnitude`: `abs` traps on Int.min (B149).
        let count = Int(min(lines.magnitude, 6))
        let x = max(1, terminal.cols / 2), y = max(1, terminal.rows / 2)
        for _ in 0..<count {
            terminal.sendEvent(buttonFlags: button, x: x, y: y, pixelX: x, pixelY: y)
        }
    }

    func clickLocal(col: Int, row: Int) {
        guard modes.mouseReporting else { return }
        let terminal = terminalView.getTerminal()
        // 0 = left button press, 3 = release (SwiftTerm's `encodeButton`
        // convention, which its own tap handler uses).
        terminal.sendEvent(buttonFlags: 0, x: col, y: row, pixelX: col, pixelY: row)
        terminal.sendEvent(buttonFlags: 3, x: col, y: row, pixelX: col, pixelY: row)
    }

    /// Never SwiftTerm's own paste: it wraps the text as it is, and a marker
    /// inside it ends the paste early (``BracketedPaste``).
    func pastePayload(_ text: String) -> Data {
        BracketedPaste.payload(text, bracketed: terminalView.getTerminal().bracketedPasteMode)
    }

    func answerClipboard(_ text: String?) {
        terminalView.getTerminal().sendClipboardResponse(content: text?.data(using: .utf8))
    }

    // MARK: Text and links

    /// Positional, as the core's: a blank cell is a space (SwiftTerm's own
    /// text leaves its NULs out, which moves what follows left), the cell
    /// after a wide character is not a column of text.
    func rowText(_ row: Int) -> String {
        let terminal = terminalView.getTerminal()
        guard let line = terminal.getLine(row: row) else { return "" }
        return line.translateToString(trimRight: true, skipNullCellsFollowingWide: true) { cell in
            let c = terminal.getCharacter(for: cell)
            return c == "\u{0}" ? " " : c
        }
    }

    /// Which paths on this terminal the host has confirmed, so the link pass
    /// can make them tappable. An answer from the host schedules another
    /// pass.
    var pathLinks: RemotePathLinks? {
        didSet {
            pathLinks?.onResolved = { [weak self] in self?.scheduleLinkify(after: 0) }
        }
    }

    /// Everything that must re-run after any bytes land: re-assert the
    /// cursor style, and re-scan visible text for bare http(s) URLs to make
    /// tappable (``PlainLinkDetector``) — the remote often only wraps a URL
    /// in a real OSC-8 hyperlink when it's alone on its own line, not one
    /// mentioned inline or inside a summary box.
    ///
    /// The link scan is DEBOUNCED, not run per feed. It walks every visible
    /// cell (`viewportLineText` per row — a fresh String each), which was
    /// written off as "cheap to call after every feed" on a phone grid of
    /// ~2k cells. An iPad grid is ~9k cells, and a pane switch arrives as
    /// hundreds of separate `%output` events — paying O(rows×cols) on each
    /// was a visible slice of the "repaints line by line" stall. A link is
    /// tappable ~0.15s after output goes quiet, which no finger beats; the
    /// cursor fix stays immediate because it is O(1).
    private func postFeedFixups() {
        enforceCursor()
        scheduleLinkify()
    }

    /// Re-assert ``enforcedCursor`` if the fed bytes changed the caret.
    /// Cheap when nothing changed (two comparisons), so it runs after every
    /// feed.
    private func enforceCursor() {
        guard let enforcedCursor else { return }
        let terminal = terminalView.getTerminal()
        if terminal.options.cursorStyle != enforcedCursor.style {
            terminal.setCursorStyle(enforcedCursor.style)
        }
        if terminalView.caretColor != enforcedCursor.color {
            terminalView.caretColor = enforcedCursor.color
        }
    }

    private var linkifyWork: DispatchWorkItem?

    func scheduleLinkify(after delay: TimeInterval = 0.15) {
        linkifyWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                PlainLinkDetector.linkify(self.terminalView)
                // Path links cost the HOST a probe, so only a pane on
                // screen asks: a tmux session's other panes keep printing
                // while hidden, and nine agents' worth of re-asks every few
                // seconds is a `find` loop nobody is looking at. Showing the
                // pane lays it out, which runs a pass.
                if let links = self.pathLinks, self.terminalView.window != nil,
                   UIApplication.shared.applicationState != .background {
                    PathLinkDetector.linkify(terminal: self.terminalView.getTerminal(), resolver: links)
                }
            }
        }
        linkifyWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    /// The host gave the view its real bounds: a pane that just came on
    /// screen, whose path links were held back while it was hidden.
    func laidOut() {
        if pathLinks != nil { scheduleLinkify() }
    }

    // MARK: Pictures

    func snapshotView() -> UIView? {
        terminalView.snapshotView(afterScreenUpdates: false)
    }

    /// The app gives SwiftTerm's view no find bar.
    var isFinding: Bool { false }

    // MARK: mosh's predictive echo

    /// The session's engine, drawing into this view's cells.
    private(set) var prediction: MoshPrediction?

    func attachPrediction(_ engine: MoshPrediction?) {
        if prediction?.withdraw(from: terminalView.getTerminal()) == true {
            terminalView.feed(byteArray: [])
        }
        prediction = engine
    }

    /// The view predictions may draw into right now: output is live and the
    /// viewport sits on the live screen (guesses address its rows).
    private var predictionLive: Bool {
        prediction != nil && !scrollHeld && terminalView.distanceToBottom < 1
    }

    func predict(_ data: Data) -> Int? {
        guard let prediction else { return nil }
        guard predictionLive else {
            // Can't see the live screen (reading scrollback): guessing blind
            // would only be wrong — start over once it's back.
            prediction.reset()
            return nil
        }
        let terminal = terminalView.getTerminal()
        let lifted = prediction.withdraw(from: terminal)
        let batch = prediction.userInput(data, on: terminal)
        let drawn = prediction.apply(to: terminal)
        if lifted || drawn { terminalView.feed(byteArray: []) }   // queue the redraw + caret move
        return batch
    }

    func predictionSent(batch: Int, userState: UInt64?) {
        prediction?.sent(batch: batch, as: userState)
    }

    func feed(moshFrame frame: MoshHostFrame) {
        prediction?.noteEchoAck(frame.echoAck)
        if !frame.bytes.isEmpty {
            feed(frame.bytes)
        } else if let prediction, predictionLive {
            let terminal = terminalView.getTerminal()
            let lifted = prediction.withdraw(from: terminal)
            prediction.settle(on: terminal)
            let drawn = prediction.apply(to: terminal)
            if lifted || drawn { terminalView.feed(byteArray: []) }
        }
    }

    func dropPredictions() {
        guard let prediction else { return }
        if prediction.withdraw(from: terminalView.getTerminal()) {
            terminalView.feed(byteArray: [])
        }
        prediction.reset()
    }

    var predictionNeedsTick: Bool { prediction?.needsTick == true }

    /// mosh wakes every 50ms while a guess is pending, so one stuck past
    /// 250ms gets shown (and marked past 5s) without waiting for the next
    /// keystroke or frame. Clock-only, so it redraws only on a change.
    func predictionTick() -> Bool {
        guard let prediction, predictionLive else { return false }
        if prediction.tick() {
            let terminal = terminalView.getTerminal()
            let lifted = prediction.withdraw(from: terminal)
            let drawn = prediction.apply(to: terminal)
            if lifted || drawn { terminalView.feed(byteArray: []) }
        }
        return true
    }

    /// Run `body` (which feeds host bytes) with the guesses taken off the
    /// screen, then judge and redraw them. Never feed over them: the server
    /// diffs against the screen it thinks we have.
    private func liftingPredictions(_ body: () -> Void) {
        guard let prediction else { body(); return }
        let terminal = terminalView.getTerminal()
        prediction.withdraw(from: terminal)
        body()
        guard predictionLive else { return }
        prediction.settle(on: terminal)
        prediction.apply(to: terminal)
    }
}

// MARK: - SwiftTerm's delegate

extension SwiftTermSurface: TerminalViewDelegate {
    nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {
        MainActor.assumeIsolated { surfaceDelegate?.surface(self, send: Data(data)) }
    }

    nonisolated func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        MainActor.assumeIsolated { surfaceDelegate?.surface(self, sizeChanged: newCols, rows: newRows) }
    }

    nonisolated func setTerminalTitle(source: TerminalView, title: String) {
        MainActor.assumeIsolated { surfaceDelegate?.surface(self, titleChanged: title) }
    }

    nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        MainActor.assumeIsolated { surfaceDelegate?.surface(self, directoryChanged: directory) }
    }

    nonisolated func scrolled(source: TerminalView, position: Double) {
        // Scroll position is SwiftTerm's own.
    }

    nonisolated func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        MainActor.assumeIsolated { surfaceDelegate?.surface(self, openLink: link) }
    }

    nonisolated func bell(source: TerminalView) {
        MainActor.assumeIsolated { surfaceDelegate?.surfaceBell(self) }
    }

    nonisolated func clipboardCopy(source: TerminalView, content: Data) {
        // OSC 52 writes are off (an app that writes the phone's clipboard
        // behind the user's back).
    }

    nonisolated func clipboardRequest(source: TerminalView) {
        MainActor.assumeIsolated { surfaceDelegate?.surfaceClipboardRequest(self) }
    }

    nonisolated func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {
        // iTerm2's OSC 1337 is out of scope.
    }

    nonisolated func rangeChanged(source: TerminalView, startY: Int, endY: Int) {
        // Only fires when `notifyUpdateChanges` is on.
    }

    /// The edit menu's Paste and ⌘V (fork patch 31): the owner's paste path.
    nonisolated func pasteRequested(source: TerminalView, text: String) -> Bool {
        MainActor.assumeIsolated { surfaceDelegate?.surface(self, paste: text) }
        return true
    }
}

extension SwiftTerminalView.Coordinator {
    /// The coordinator's terminal as SwiftTerm's view, when SwiftTerm draws
    /// it: for the adapter's own debug probes and the tests of SwiftTerm's
    /// behaviour. App code asks the surface (``surface``).
    var swiftTermView: TerminalView? {
        withSurface { ($0 as? SwiftTermSurface)?.terminalView } ?? nil
    }
}
