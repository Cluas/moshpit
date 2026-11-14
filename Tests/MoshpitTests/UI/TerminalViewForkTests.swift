import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// The SwiftTerm fork's iOS view, where the fork's own `swift test` (macOS)
/// cannot reach: what the view does with an erased history (patch 29), with
/// a full reset (patch 32) and with a bright-colours change (patch 30), and
/// what happens to its selection over an erased history (patch 37), a reset
/// (patch 38) and Select All (patch 39); round twenty-four's: a view that
/// leaves its window is freed (patch 46), its selection over a full history
/// (41), a switch of screens (42) and DECCOLM (43), autowrap above the bottom
/// row (40), and what it tells UIKit about the pointer (48), the keyboard
/// focus (50) and AutoFill (47); round twenty-nine's: a full history's
/// copies (54), the alternate screen on a short screen (53), a search match
/// in the last column (55), DECCOLM left to the program (58) and the text
/// input's predictions (60); round thirty-one's: a cancelled press released
/// for the kitty protocol (62), `clear` and the scroll-invariant numbers
/// (63) and the grid the app keeps its own (64); 65 (the image lines
/// `clear` takes with it) is the engine's, tested by the fork; round
/// thirty-two's: screen's title sequence consumed (66). The fork's
/// iOS tests that need no app run on Mac Catalyst
/// (its iOSTerminalViewTests).
@Suite("SwiftTerm fork: iOS view")
@MainActor
struct TerminalViewForkTests {

    private func makeView() -> TerminalView {
        TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 300),
                     font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
    }

    /// Two hundred lines of output: history above the screen.
    private func fillHistory(_ view: TerminalView) {
        for i in 0 ..< 200 {
            view.feed(text: "line \(i)\r\n")
        }
    }

    /// What `clear` prints (ncurses, with the E3 capability), and Claude
    /// Code's `/clear`, and Moshpit's own herdr pane switch.
    private let clear = "\u{1b}[H\u{1b}[2J\u{1b}[3J"

    /// The rows are drawn from the scroll offset. ESC [ 3 J trimmed the
    /// history and moved the viewport to the top without telling the view,
    /// so the offset stayed on a row that no longer existed and every draw
    /// came out blank — until something scrolled.
    @Test("clear with history puts the scroll offset back on the screen")
    func clearFromTheLiveBottom() throws {
        let view = makeView()
        fillHistory(view)
        let terminal = view.getTerminal()
        try #require(terminal.getTopVisibleRow() > 0, "history above the screen")
        #expect(view.contentOffset.y == CGFloat(terminal.getTopVisibleRow()) * view.cellHeight)

        view.feed(text: clear)

        #expect(terminal.getTopVisibleRow() == 0)
        #expect(view.contentOffset.y == 0, "the offset stayed on a row that is gone")
        #expect(view.contentSize.height == CGFloat(terminal.rows) * view.cellHeight,
                "the content still has room for the erased history")
        #expect(view.distanceToBottom == 0)
    }

    @Test("clear while reading history lands on the screen")
    func clearFromHistory() throws {
        let view = makeView()
        fillHistory(view)
        view.scrollUp(lines: 40)
        let terminal = view.getTerminal()
        try #require(view.distanceToBottom > 0, "reading history")

        view.feed(text: clear)

        #expect(terminal.getTopVisibleRow() == 0)
        #expect(view.contentOffset.y == 0)
        #expect(view.distanceToBottom == 0)
    }

    /// A full reset (`ESC c`, what `reset` and `tput reset` send) swaps in a
    /// fresh buffer with no history: the same viewport move as ESC [ 3 J,
    /// and the same blank screen, until fork patch 32 told the view.
    @Test("reset with history puts the scroll offset back on the screen")
    func resetFromTheLiveBottom() throws {
        let view = makeView()
        fillHistory(view)
        let terminal = view.getTerminal()
        try #require(terminal.getTopVisibleRow() > 0, "history above the screen")

        view.feed(text: "\u{1b}c")

        #expect(terminal.getTopVisibleRow() == 0)
        #expect(view.contentOffset.y == 0, "the offset stayed on a row that is gone")
        #expect(view.contentSize.height == CGFloat(terminal.rows) * view.cellHeight,
                "the content still has room for the history the reset dropped")
        #expect(view.distanceToBottom == 0)
    }

    @Test("reset while reading history lands on the screen")
    func resetFromHistory() throws {
        let view = makeView()
        fillHistory(view)
        view.scrollUp(lines: 40)
        let terminal = view.getTerminal()
        try #require(view.distanceToBottom > 0, "reading history")

        view.feed(text: "\u{1b}c")

        #expect(terminal.getTopVisibleRow() == 0)
        #expect(view.contentOffset.y == 0)
        #expect(view.distanceToBottom == 0)
    }

    /// The view caches each cell attribute it resolves; `useBrightColors` is
    /// read only while resolving, so a change has to drop the cache and
    /// repaint every row.
    @Test("Turning bright colours off repaints every row")
    func brightColorsChangeRepaints() {
        let view = makeView()
        let terminal = view.getTerminal()
        terminal.clearUpdateRange()

        view.useBrightColors = false

        let range = terminal.getUpdateRange()
        #expect(range.map { $0.startY <= 0 && $0.endY >= terminal.rows - 1 } == true,
                "nothing was marked for repaint, so nothing shows the change")
    }

    // MARK: - The selection

    /// The view with history, set up the way Moshpit sets it up for
    /// selecting: mouse reporting off (`TerminalMint`), so a selection stays
    /// while output arrives.
    private func makeSelectingView() -> TerminalView {
        let view = makeView()
        view.allowMouseReporting = false
        fillHistory(view)
        return view
    }

    /// A reset the remote sends (`reset` in the shell, `ESC c`) replaces the
    /// buffer, history and screen. The selection stayed, pointing into the
    /// fresh buffer — a highlight over blank rows, then over whatever came
    /// next, and Copy still offered — where the edit menu's own Reset
    /// dropped it (fork patch 38).
    @Test("a reset from the remote drops the selection")
    func remoteResetDropsTheSelection() throws {
        let view = makeSelectingView()
        try #require(view.findNext("line 198", scrollToResult: false))
        try #require(view.getSelection() == "line 198")

        view.feed(text: "\u{1b}c")

        #expect(!view.selectionActive, "the selection outlived the buffer it was made in")
        #expect(view.getSelection() == nil)
        #expect(!view.canPerformAction(#selector(UIResponder.copy(_:)), withSender: nil),
                "Copy is still offered for a selection that points nowhere")
    }

    /// `clear` ends with ESC [ 3 J, which drops the history and moves every
    /// row left up. The selection kept its old row numbers, so its highlight
    /// and its copy landed on other rows (fork patch 37).
    @Test("clear keeps a selection on the screen on its text")
    func eraseHistoryKeepsTheSelectionOnItsText() throws {
        let view = makeSelectingView()
        try #require(view.findNext("line 198", scrollToResult: false))
        try #require(view.getSelection() == "line 198")

        view.feed(text: "\u{1b}[3J")

        #expect(view.selectionActive, "the text it selected is still on the screen")
        #expect(view.getSelection() == "line 198", "the selection no longer covers its text")
    }

    @Test("clear drops a selection in the history it erased")
    func eraseHistoryDropsASelectionInIt() throws {
        let view = makeSelectingView()
        try #require(view.findNext("line 42", scrollToResult: false))
        try #require(view.getSelection() == "line 42")

        view.feed(text: "\u{1b}[3J")

        #expect(!view.selectionActive, "a selection of erased text survived")
        #expect(view.getSelection() == nil)
    }

    /// Select All ends on the last line with content, and its end is
    /// exclusive: it ended at the last column, so a last line that fills the
    /// row — the bottom row of a full-screen program — lost its last
    /// character in the copy (fork patch 39).
    @Test("Select All copies a full last line whole")
    func selectAllCopiesAFullLastLineWhole() throws {
        let view = makeView()
        let terminal = view.getTerminal()
        let last = String(repeating: "z", count: terminal.cols)
        for i in 0 ..< terminal.rows - 1 {
            view.feed(text: "row \(i)\r\n")
        }
        view.feed(text: last)

        view.selectAll(nil)

        let copied = try #require(view.getSelection())
        #expect(copied.hasPrefix("row 0\n"))
        #expect(copied.hasSuffix("\n" + last), "the last line copied as \(copied.split(separator: "\n").last ?? "")")
    }

    // MARK: - Round twenty-four

    /// A window to put views in: the test host's scene.
    private func makeWindow() -> UIWindow {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow()
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 300)
        return window
    }

    /// The display link held the view, and the run loop held the link, so
    /// every pane ever closed kept its view, its terminal and its history
    /// (about 19 MB at 10 000 lines) for the life of the app (fork patch 46).
    @Test("a view that leaves its window is freed with its terminal")
    func aViewThatLeavesItsWindowIsFreed() async throws {
        weak var released: TerminalView?
        weak var releasedTerminal: Terminal?
        let window = makeWindow()
        do {
            let view = makeView()
            window.addSubview(view)
            fillHistory(view)
            _ = view.tokenizer
            released = view
            releasedTerminal = view.getTerminal()
            view.removeFromSuperview()
        }
        // A repaint the view queued holds it until it has run.
        try await Task.sleep(nanoseconds: 200_000_000)

        #expect(released == nil, "the view outlived its window and its last reference")
        #expect(releasedTerminal == nil, "the terminal outlived its view")
        withExtendedLifetime(window) {}
    }

    /// Once the history is full every line of output pushes the oldest one
    /// out and moves every row up by one; the selection kept its rows and
    /// slid down the text (fork patch 41).
    @Test("a selection keeps to its text while lines scroll into a full history")
    func aSelectionKeepsToItsTextInAFullHistory() throws {
        let view = makeView()
        view.allowMouseReporting = false
        view.getTerminal().changeScrollback(50)
        fillHistory(view)
        try #require(view.findNext("line 190", scrollToResult: false))
        try #require(view.getSelection() == "line 190")

        for i in 0 ..< 5 {
            view.feed(text: "more \(i)\r\n")
        }

        #expect(view.getSelection() == "line 190", "the selection slid onto other text")
    }

    /// A program switching to the alternate screen puts other text where
    /// the selection was; Copy took it (fork patch 42).
    @Test("switching screens drops the selection")
    func switchingScreensDropsTheSelection() throws {
        let view = makeSelectingView()
        try #require(view.findNext("line 198", scrollToResult: false))

        view.feed(text: "\u{1b}[?1049h")

        #expect(!view.selectionActive, "the selection outlived the screen it was made on")
        #expect(view.getSelection() == nil)
    }

    /// DECCOLM was a full reset: the history went (fork patch 43). Now it
    /// clears the screen and changes the columns, as xterm does, for a
    /// program that allowed it with mode 40 (fork patch 58).
    @Test("DECCOLM keeps the history and drops the selection")
    func columnModeKeepsTheHistory() throws {
        let view = makeSelectingView()
        try #require(view.findNext("line 198", scrollToResult: false))
        let terminal = view.getTerminal()

        view.feed(text: "\u{1b}[?40h\u{1b}[?3h")

        #expect(terminal.cols == 132)
        #expect(view.contentSize.height > CGFloat(terminal.rows) * view.cellHeight, "the history went")
        #expect(!view.selectionActive)
    }

    /// Text that runs past the last column above the bottom row marks the
    /// row it runs on to; with history the mark went on a line of history,
    /// and the long line copied as two (fork patch 40).
    @Test("autowrap above the bottom row marks the row it runs on to")
    func autowrapAboveTheBottomRowMarksTheRightRow() {
        let view = makeView()
        fillHistory(view)
        let terminal = view.getTerminal()
        view.feed(text: "\u{1b}[H\u{1b}[2J" + String(repeating: "A", count: terminal.cols) + "BBB")

        #expect(terminal.isRowWrapped(0), "the row the text runs on to is not marked")
    }

    /// The view tells UIKit: the pointer over it is a beam (48), it is no
    /// stop of the keyboard focus, which took Tab away from it (50), and its
    /// text input is nothing AutoFill fills (47).
    @Test("the view's pointer, keyboard focus and AutoFill answers")
    func pointerFocusAndAutoFill() throws {
        let view = makeView()
        let interaction = try #require(view.interactions.compactMap { $0 as? UIPointerInteraction }.first)
        #expect(view.pointerInteraction(interaction, styleFor: UIPointerRegion(rect: view.bounds)) != nil)
        #expect(!view.canBecomeFocused)
        #expect(view.textContentType == TerminalView.terminalContentType)
    }

    // MARK: - Round twenty-nine

    /// Once the history is full, a line that scrolls in reuses the oldest
    /// line, and kept its wrap mark: after wrapped output, every other line
    /// of a long session copied joined to the line before it (fork patch
    /// 54).
    @Test("lines that scroll into a full history copy as lines")
    func linesIntoAFullHistoryCopyAsLines() throws {
        let view = makeView()
        let terminal = view.getTerminal()
        terminal.changeScrollback(20)
        let long = String(repeating: "A", count: terminal.cols) + "BBB"
        for _ in 0 ..< terminal.rows + 20 {
            view.feed(text: long + "\r\n")
        }
        for i in 0 ..< terminal.rows + 20 {
            view.feed(text: "line \(i)\r\n")
        }

        view.selectAll(nil)

        let copied = try #require(view.getSelection())
        let joined = copied.split(separator: "\n").filter { !$0.hasPrefix("line ") || Int($0.dropFirst(5)) == nil }
        #expect(joined.isEmpty, "lines copied joined: \(joined.prefix(3))")
        #expect(copied.hasSuffix("line \(terminal.rows + 19)"))
    }

    /// The alternate screen keeps no history; on a screen of fewer than 25
    /// rows (a phone's) it kept up to 25 - rows lines, and the view could
    /// scroll a full-screen program back into its old frames (fork patch 53).
    @Test("a full-screen program on a phone-sized screen keeps no history")
    func theAlternateScreenKeepsNoHistory() throws {
        let view = makeView()
        let terminal = view.getTerminal()
        try #require(terminal.rows < 25)

        view.feed(text: "\u{1b}[?1049h")
        for i in 0 ..< 60 {
            view.feed(text: "frame \(i)\r\n")
        }

        #expect(terminal.getTopVisibleRow() == 0, "the alternate screen kept history")
        #expect(view.contentSize.height == CGFloat(terminal.rows) * view.cellHeight)
    }

    /// A match that ends in the last column ended at the start of the next
    /// row, and the copy took the line break after it (fork patch 55).
    @Test("a search match in the last column copies as the match")
    func aSearchMatchInTheLastColumnCopiesAsItself() throws {
        let view = makeView()
        let cols = view.getTerminal().cols
        view.feed(text: String(repeating: "-", count: cols - 6) + "needle\r\nnext line")

        try #require(view.findNext("needle", scrollToResult: false))

        #expect(view.getSelection() == "needle")
    }

    /// DECCOLM alone made the grid 132 or 80 columns wide whatever the phone
    /// shows, and the app reports its grid to the remote as the window size;
    /// xterm honours it only once a program turns on mode 40 (fork patch 58).
    @Test("a remote's DECCOLM leaves the grid alone")
    func columnModeLeavesTheGridAlone() {
        let view = makeView()
        let terminal = view.getTerminal()
        let cols = terminal.cols
        view.feed(text: "$ prompt")

        view.feed(text: "\u{1b}[?3h")

        #expect(terminal.cols == cols)
        #expect(terminal.getLine(row: 0)?.translateToString(trimRight: true) == "$ prompt", "the screen was cleared")
    }

    /// The app keeps the grid its own (fork patch 64, B217): a minted
    /// terminal ignores DECCOLM even after a program turns mode 40 on, and
    /// says mode 40 is off. Without it a program could make the phone's grid
    /// 132 columns wide, and the remote is told the grid as the window size.
    @Test("a minted terminal keeps its grid through DECCOLM, mode 40 or not")
    func aMintedTerminalKeepsItsGrid() {
        let view = makeView()
        TerminalMint.configureInput(view)
        let terminal = view.getTerminal()
        let cols = terminal.cols
        view.feed(text: "$ prompt")

        view.feed(text: "\u{1b}[?40h\u{1b}[?3h")
        #expect(terminal.cols == cols, "the remote made the grid \(terminal.cols) columns wide")
        view.feed(text: "\u{1b}[?3l")
        #expect(terminal.cols == cols)
        #expect(terminal.getLine(row: 0)?.translateToString(trimRight: true) == "$ prompt", "the screen was cleared")
    }

    /// `clear` (ED 3) keeps the numbers of the lines it leaves (fork patch
    /// 63): they went back to 0, and a number read before `clear` found
    /// another line after it.
    @Test("clear keeps the scroll-invariant numbers of the lines it leaves")
    func clearKeepsTheScrollInvariantNumbers() throws {
        let view = makeView()
        fillHistory(view)
        let terminal = view.getTerminal()
        // The oldest line's number, then the top row of the screen's.
        let oldest = try #require((0 ..< 100_000).first { terminal.getScrollInvariantLine(row: $0) != nil })
        let row = oldest + terminal.buffer.yDisp
        let line = try #require(terminal.getScrollInvariantLine(row: row))
        let text = line.translateToString(trimRight: true)

        view.feed(text: "\u{1b}[3J")

        let after = try #require(terminal.getScrollInvariantLine(row: row), "the line lost its number")
        #expect(after.translateToString(trimRight: true) == text)
    }

    /// A press UIKit takes back (the key held while ⌘-Tab took the app
    /// away) is released for a program that asked for the kitty protocol's
    /// event reports (fork patch 62): it heard the key go down and never up.
    @Test("a cancelled press is released for the kitty keyboard protocol")
    func aCancelledPressIsReleased() {
        final class Key: UIKey {
            override var keyCode: UIKeyboardHIDUsage { .keyboardUpArrow }
            override var modifierFlags: UIKeyModifierFlags { [] }
            override var characters: String { "" }
            override var charactersIgnoringModifiers: String { "" }
        }
        final class Press: UIPress {
            private let pressed: UIKey
            init(_ key: UIKey) {
                pressed = key
                super.init()
            }
            override var key: UIKey? { pressed }
        }
        final class Host: NSObject, TerminalViewDelegate {
            var sent: [[UInt8]] = []
            func send(source: TerminalView, data: ArraySlice<UInt8>) { sent.append(Array(data)) }
            func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {}
            func setTerminalTitle(source: TerminalView, title: String) {}
            func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
            func scrolled(source: TerminalView, position: Double) {}
            func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {}
            func bell(source: TerminalView) {}
            func clipboardCopy(source: TerminalView, content: Data) {}
            func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
            func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
        }
        let view = makeView()
        let host = Host()
        view.terminalDelegate = host
        view.feed(text: "\u{1b}[>3u")
        let press = Press(Key())
        view.pressesBegan([press], with: nil)
        host.sent = []

        view.pressesCancelled([press], with: nil)

        #expect(host.sent == [Array("\u{1b}[1;1:3A".utf8)],
                "\(host.sent.map { String(decoding: $0, as: UTF8.self).debugDescription })")
    }

    /// A shell in tmux names its window with screen's title sequence,
    /// ESC k <title> ST (oh-my-zsh: before every command and at every
    /// prompt), and a tmux -CC client gets the pane's bytes raw: the view
    /// printed every title into the pane, so `echo hello` showed
    /// "echohello" (fork patch 66, B257).
    @Test("screen's title sequence is consumed, not printed")
    func screenTitlesAreNotPrinted() {
        let view = makeView()
        TerminalMint.configureInput(view)
        view.feed(text: "echo hello\r\n\u{1b}kecho\u{1b}\\hello\r\n\u{1b}k~/demo\u{1b}\\$ ")
        let terminal = view.getTerminal()
        let rows = (0 ..< 3).map { row in
            String((terminal.getLine(row: row)?.translateToString(trimRight: true) ?? "")
                .reversed().drop { $0 == " " }.reversed())
        }
        #expect(rows == ["echo hello", "hello", "$"], "\(rows)")
        #expect(terminal.getCursorLocation().x == 2)
    }

    /// Inline predictions, math results and Writing Tools each offer text
    /// that, taken, is typed into the remote; the view left them to the
    /// system's default, which offers them (fork patch 60).
    @Test("the text input takes no predictions, math results or Writing Tools")
    func theTextInputTakesNoPredictions() {
        let view = makeView()
        #expect(view.inlinePredictionType == UITextInlinePredictionType.no)
        #expect(view.mathExpressionCompletionType == UITextMathExpressionCompletionType.no)
        #expect(view.writingToolsBehavior == UIWritingToolsBehavior.none)
    }
}
