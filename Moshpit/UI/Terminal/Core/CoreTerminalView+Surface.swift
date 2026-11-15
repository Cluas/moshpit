import MoshpitFFI
import UIKit

/// The core's view as the app's terminal (``CoreTerminalView``): what the
/// session's coordinator, the tmux controllers and the hub ask of a
/// terminal, answered by the core.
extension CoreTerminalView {
    var view: UIView { self }

    // MARK: Bytes

    func feed(_ data: Data) { push(data) }

    // `feed(mark:)` and `discardPendingInput()`: CoreTerminalView.

    // MARK: The grid and the look

    func apply(look: TerminalLook) {
        guard look != appliedLook else { return }
        appliedLook = look
        apply(theme: look.theme, cursorShape: look.cursorShape, cursorColorId: look.cursorColorId,
              cursorBlink: look.cursorBlink, appTheme: look.appTheme)
    }

    /// The core's palette is the theme's whatever the remote set: the same
    /// as ``apply(look:)``.
    func reassert(look: TerminalLook) {
        appliedLook = nil
        apply(look: look)
    }

    func grid(fitting size: CGSize) -> TerminalGrid {
        let cell = cellSize
        guard cell.width > 0, cell.height > 0 else { return TerminalGrid(cols: 0, rows: 0) }
        return TerminalGrid(cols: Int(size.width / cell.width), rows: Int(size.height / cell.height))
    }

    /// The host gave the view its real bounds: a pane that just came on
    /// screen.
    func laidOut() { schedulePathPass() }

    func viewportCell(at point: CGPoint) -> TerminalCell {
        let c = cell(at: point)
        return TerminalCell(col: c.col, row: c.row)
    }

    var cursor: TerminalCell {
        TerminalCell(col: Int(rows.info.cursor.col), row: Int(rows.info.cursor.row))
    }

    var modes: TerminalModes {
        let m = termModes
        return TerminalModes(applicationCursorKeys: m.app_cursor, bracketedPaste: m.bracketed_paste,
                             mouseReporting: m.mouse != 0)
    }

    // MARK: History

    func scrollLocal(lines: Int) { scrollLines(lines) }

    func scrollLocal(pixels dy: CGFloat) -> CGFloat { scrollViewport(by: dy) }

    func settleLocalScroll() { settleScroll() }

    /// A drag's start settles what a previous one left (the scroll model's
    /// own carry).
    func dragWillBegin() { settleScroll() }

    // `drag(_:hostRows:)`: CoreTerminalView. The core never holds output:
    // its view stays where it was scrolled while output lands (V21).
    /// Back on the live screen (B363): what typing, a paste and a wheel
    /// forwarded to the program all mean. Left empty, a wheel that followed
    /// a drag begun in the local history (the flags not known yet) went to
    /// the program while the view stayed up there: old rows on top, the
    /// program's repaints underneath, until a keystroke — which didn't
    /// bring it down either. Not for the terminal's own answers (B426).
    func releaseScrollHold() {
        guard !sendingReply else { return }
        scrollToBottom()
    }
    func discardScrollHold() {}

    /// The terminal's own answers (to a cursor-position or colour query, a
    /// focus report) out on the session's input path, which releases the
    /// scroll hold for what it sends: they are not the person's, and leave
    /// a history reader where it is (B426). They sent it to the bottom
    /// whenever a program asked the terminal something.
    func sendReply(_ data: Data) {
        sendingReply = true
        defer { sendingReply = false }
        surfaceDelegate?.surface(self, reply: data)
    }

    // MARK: The program's mouse and paste

    func pastePayload(_ text: String) -> Data { pasteBytes(text) ?? Data() }

    // MARK: Pictures

    func snapshotView() -> UIView? { snapshot() }

    // `redrawAll()` and `framesSinceCanvas`: CoreTerminalView.

    /// What lies outside the grid when it isn't the theme's (B495).
    var groundColor: UInt32? { style.ground }

    // MARK: mosh's predictive echo (crates/term/src/predict.rs)

    func attachPrediction(_ engine: MoshPrediction?) {
        prediction = engine
        predictAttach(engine)
    }

    func predict(_ data: Data) -> Int? {
        guard let prediction else { return nil }
        return predictInput(data, srtt: prediction.srttMs)
    }

    func predictionSent(batch: Int, userState: UInt64?) {
        predictSent(batch: batch, userState: userState)
    }

    /// The bytes, then the ack, in the core's queue: judged after the bytes
    /// it came with.
    func feed(moshFrame frame: MoshHostFrame) {
        if !frame.bytes.isEmpty { push(frame.bytes) }
        if prediction != nil { predictEchoAck(frame.echoAck) }
    }

    func dropPredictions() {
        guard prediction != nil else { return }
        predictReset()
    }

    var predictionNeedsTick: Bool { prediction != nil && predictNeedsTick }

    func predictionTick() -> Bool {
        guard let prediction else { return false }
        predictTick(srtt: prediction.srttMs)
        return true
    }
}
