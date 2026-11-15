import Foundation
import MoshpitFFI
import Testing
import UIKit
@testable import Moshpit

/// The core terminal view (T2, docs/nsh/TERMINAL-CORE.md): what it sends for
/// keys and input methods, driven through UITextInput directly as UIKit
/// would; and what it draws, rendered and compared against the core's own
/// frame and against a full repaint.
@Suite("core terminal view")
@MainActor
struct CoreTerminalViewTests {

    /// Collects what the view sends.
    final class Sink: CoreTerminalViewDelegate {
        var sent = Data()
        var sizes: [(Int, Int)] = []
        var links: [String] = []
        var pastes: [String] = []
        var clicks: [(Int, Int)] = []
        /// What the session's input path does with what it sends.
        var onSend: ((CoreTerminalView) -> Void)?
        var text: String { String(decoding: sent, as: UTF8.self) }
        func take() -> String { defer { sent = Data() }; return text }

        func surface(_ surface: CoreTerminalView, send data: Data) {
            sent.append(data)
            onSend?(surface)
        }
        func surface(_ surface: CoreTerminalView, sizeChanged cols: Int, rows: Int) { sizes.append((cols, rows)) }
        func surface(_ surface: CoreTerminalView, titleChanged title: String) {}
        func surfaceBell(_ surface: CoreTerminalView) {}
        func surface(_ surface: CoreTerminalView, openLink link: String) { links.append(link) }
        func surface(_ surface: CoreTerminalView, paste text: String) { pastes.append(text) }
        func surfaceClipboardRequest(_ surface: CoreTerminalView) {}
        func surfaceScrollWillBegin(_ surface: CoreTerminalView) {}
        func surface(_ surface: CoreTerminalView, scrollPixels dy: CGFloat) -> CGFloat { surface.scrollLocal(pixels: dy) }
        func surfaceScrollDidEnd(_ surface: CoreTerminalView) {}
        func surface(_ surface: CoreTerminalView, click col: Int, row: Int) { clicks.append((col, row)) }
        var surfaceScrollsElsewhere: Bool { false }
        var surfaceCanSwitch: Bool { false }
        func surface(_ surface: CoreTerminalView, switchForward forward: Bool) {}
        func surface(_ surface: CoreTerminalView, fontSizeCommitted size: Double) {}
    }

    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private func makeView(cols: Int = 40, rows: Int = 12) -> (CoreTerminalView, Sink) {
        let cell = TerminalCellGeometry.measuredCell(font: Self.font, scale: UITraitCollection.current.displayScale)
        let view = CoreTerminalView(frame: CGRect(x: 0, y: 0, width: cell.width * CGFloat(cols),
                                                  height: cell.height * CGFloat(rows)), font: Self.font)
        let sink = Sink()
        view.surfaceDelegate = sink
        // A steady cursor: two views drawn at different moments compare.
        var style = view.style
        style.cursorBlink = false
        view.style = style
        return (view, sink)
    }

    /// A window the view can take the keyboard in (the phantom and the
    /// input system's rules need a first responder).
    private func inWindow(_ view: CoreTerminalView) -> UIWindow {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 600, height: 800))
        window.frame = CGRect(x: 0, y: 0, width: 600, height: 800)
        let host = UIViewController()
        window.rootViewController = host
        host.view.addSubview(view)
        window.isHidden = false
        _ = view.becomeFirstResponder()
        return window
    }

    private func feed(_ view: CoreTerminalView, _ s: String, until done: @escaping (CoreTerminalView) -> Bool) {
        view.push(Data(s.utf8))
        #expect(view.settle { done($0) }, "the core never showed it")
    }

    private func row(_ view: CoreTerminalView, _ r: Int) -> String { view.rowText(r) }

    // MARK: Bytes and frames

    /// An image that scrolls off the screen (frames that shift the rows) is
    /// let go by the view: it kept the decoded pixels for good (B297).
    @Test func anImageScrolledAwayIsLetGo() {
        let (view, _) = makeView(cols: 40, rows: 12)
        // A 16 x 48 px kitty image: a few rows tall.
        let rgba = Data((0..<(16 * 48)).flatMap { _ in [UInt8(200), 0, 30, 255] })
        feed(view, "\u{1b}_Ga=T,f=32,s=16,v=48,q=2;\(rgba.base64EncodedString())\u{1b}\\\r\n",
             until: { !$0.rows.images.isEmpty })
        // Scrolled away a few lines at a time: each frame a shift.
        for i in 0..<8 {
            feed(view, "line \(i)\r\nline \(i)b\r\nline \(i)c\r\n",
                 until: { v in (0..<v.grid.rows).contains { v.rowText($0) == "line \(i)c" } })
        }
        #expect(view.rows.rows.allSatisfy { $0.images.isEmpty }, "no row shows it")
        #expect(view.rows.images.isEmpty, "its pixels let go")
    }

    @Test func pushedBytesShowInOrder() {
        let (view, _) = makeView()
        var text = ""
        for i in 0..<500 { text += "line \(i)\r\n" }
        feed(view, text + "end", until: { $0.rowText($0.rows.gridRows - 1) == "end" })
        #expect(row(view, view.rows.gridRows - 2) == "line 499")
        #expect(view.historyLines > 400)
    }

    @Test func aResizeIsReportedOnceAndReachesTheGrid() {
        let (view, sink) = makeView(cols: 40, rows: 12)
        view.frame.size.width = view.cellSize.width * 30 + 1
        view.layoutIfNeeded()
        #expect(sink.sizes.count == 1)
        #expect(sink.sizes.first?.0 == 30)
        #expect(view.settle { Int($0.rows.info.cols) == 30 })
    }

    // MARK: Keys

    /// B413: Option on a hardware keyboard is Meta, encoded by the core:
    /// ESC and the key, not the layout's ∫ ƒ ≥ or a dead key; Option-Return
    /// is Meta-Enter (Claude Code's newline), Option-←/→ readline's word
    /// moves.
    @Test func optionIsMeta() {
        let (view, sink) = makeView()
        func send(_ usage: UIKeyboardHIDUsage, _ chars: String) -> String {
            guard let m = CoreTerminalView.metaKey(usage, characters: chars, flags: .alternate, kitty: 0) else {
                return "nothing"
            }
            view.sendKey(m.key, scalar: m.scalar, mods: m.mods)
            return sink.take()
        }
        #expect(send(.keyboardB, "b") == "\u{1b}b")
        #expect(send(.keyboardE, "e") == "\u{1b}e", "not a dead key")
        #expect(send(.keyboardReturnOrEnter, "\r") == "\u{1b}\r", "Meta-Enter")
        #expect(send(.keyboardDeleteOrBackspace, "\u{8}") == "\u{1b}\u{7f}")
        #expect(send(.keyboardLeftArrow, "\u{F702}") == "\u{1b}b")
        #expect(send(.keyboardRightArrow, "\u{F703}") == "\u{1b}f")
        #expect(CoreTerminalView.metaKey(.keyboardUpArrow, characters: "\u{F700}", flags: .alternate, kitty: 0) == nil,
                "the other arrows keep their own encoding")
        #expect(CoreTerminalView.metaKey(.keyboardB, characters: "b", flags: [.alternate, .command], kitty: 0) == nil,
                "⌘ chords are the app's")
        #expect(CoreTerminalView.metaKey(.keyboardB, characters: "b", flags: [], kitty: 0) == nil)
        // Under the kitty protocol: its encoding of the arrow, with Alt.
        #expect(CoreTerminalView.metaKey(.keyboardLeftArrow, characters: "\u{F702}", flags: .alternate, kitty: 1) == nil)
    }

    // MARK: Input methods, through UITextInput

    @Test func typedTextGoesOutAsText() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        view.insertText("ls")
        view.insertText("\n")
        #expect(sink.take() == "ls\r")
    }

    @Test func compositionSendsNothingUntilCommitted() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        // Pinyin: n, ni, 你 picked.
        view.setMarkedText("n", selectedRange: NSRange(location: 1, length: 0))
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        #expect(sink.sent.isEmpty, "marked text never reaches the remote (patch 5)")
        #expect(view.markedLabel?.attributedText?.string == "ni", "it shows at the cursor (patch 5)")
        // Hold-to-delete while composing edits the composition (patch 7).
        view.deleteBackward()
        #expect(sink.sent.isEmpty)
        #expect(view.markedLabel?.attributedText?.string == "n")
        view.setMarkedText("你", selectedRange: NSRange(location: 1, length: 0))
        view.unmarkText()
        #expect(sink.take() == "你")
        #expect(view.markedLabel == nil)
        // The caret stays where the composition ended until the echo (9).
        #expect(view.pendingCaret != nil)
    }

    /// B573: a bar key pressed while composing. Esc takes the composition
    /// away unsent; any other key commits it first, so the remote gets the
    /// text, then the key. It used to stay composing: the key went out
    /// first, and the composition after it (vim left insert mode, then got
    /// the pinyin typed into it).
    @Test func aBarKeyEndsTheComposition() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.attach(view)
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        #expect(!coordinator.endComposition(cancel: false), "nothing composing: nothing to end")
        // Esc: away, unsent.
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        #expect(coordinator.endComposition(cancel: true))
        #expect(sent.isEmpty && sink.sent.isEmpty, "the cancelled composition went out")
        #expect(view.markedLabel == nil)
        #expect(view.markedTextRange == nil, "the input system still sees a composition")
        // Any other key: committed first.
        view.setMarkedText("日本", selectedRange: NSRange(location: 2, length: 0))
        #expect(coordinator.endComposition(cancel: false))
        #expect(sent == Data("日本".utf8))
        #expect(view.markedLabel == nil)
        #expect(view.markedTextRange == nil)
        #expect(TerminalScreen.cancelsComposition([Data([0x1B])]))
        #expect(!TerminalScreen.cancelsComposition([Data([0x09])]))
        #expect(!TerminalScreen.cancelsComposition([Data([0x1B, 0x5B, 0x41])]), "an arrow commits")
        #expect(!TerminalScreen.cancelsComposition([Data([0x1B]), Data([0x0D])]), "Esc then Return commits")
    }

    /// B694: the caret UIKit asks for (a hardware keyboard's candidate
    /// window and the system's caret popovers sit by it) was always the
    /// remote's cursor cell. Output that moves that cursor mid-composition
    /// leaves the composition where it is (V7), so the candidates went
    /// one way and the pinyin stayed the other.
    @Test func theCaretUIKitSeesIsTheCompositions() throws {
        let (view, _) = makeView(cols: 20, rows: 6)
        let window = inWindow(view)
        defer { window.isHidden = true }
        feed(view, "\u{1b}[4;6H", until: { $0.rows.info.cursor.row == 3 && $0.rows.info.cursor.col == 5 })
        #expect(view.caretRect(for: view.endOfDocument) == view.cursorCellRect(), "nothing composing: the cursor")
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        // A status line redrawn elsewhere: the cursor goes up and stays.
        feed(view, "\u{1b}[1;1Hworking", until: { $0.rows.info.cursor.row == 0 })
        let caret = view.caretRect(for: view.endOfDocument)
        let label = try #require(view.markedLabel)
        #expect(caret == view.compositionCaret.frame, "the caret is at \(caret), the composition's at \(view.compositionCaret.frame)")
        #expect(abs(caret.minY - label.frame.minY) < 0.001 && abs(caret.minX - label.frame.maxX) < 0.001)
        // Committed: the caret held at its end until the echo.
        view.unmarkText()
        let held = try #require(view.pendingCaret)
        #expect(view.caretRect(for: view.endOfDocument) == held.frame)
        // The echo moves the cursor: the cursor again.
        feed(view, "\u{1b}[4;8H", until: { $0.rows.info.cursor.col == 7 })
        #expect(view.pendingCaret == nil)
        #expect(view.caretRect(for: view.endOfDocument) == view.cursorCellRect())
    }

    /// B571: pinyin typed at the end of a long line ran past the view's
    /// right edge, which clips: the composition and its caret were cut off.
    @Test func aCompositionAtTheRightEdgeStaysOnScreen() throws {
        let (view, _) = makeView(cols: 20, rows: 4)
        let window = inWindow(view)
        defer { window.isHidden = true }
        feed(view, "\u{1b}[1;18H", until: { $0.rows.info.cursor.col == 17 })
        view.setMarkedText("zhongguoren", selectedRange: NSRange(location: 11, length: 0))
        let label = try #require(view.markedLabel)
        #expect(label.frame.maxX <= view.bounds.width, "the composition runs past the edge: \(label.frame)")
        #expect(view.compositionCaret.frame.maxX <= view.bounds.width + 0.5, "the caret is off screen")
        // Points, compared within a hair: a frame's origin comes back
        // through its centre and size, a few 1e-15 off (CI run 10:
        // 15.333333333333336 against 15.333333333333334).
        #expect(abs(label.frame.minY - view.y(ofRow: 0)) < 0.001, "it stays on the cursor's row")
        // Room enough: where the cursor is.
        view.unmarkText()
        feed(view, "\u{1b}[1;3H", until: { $0.rows.info.cursor.col == 2 })
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        let minX = try #require(view.markedLabel?.frame.minX)
        #expect(abs(minX - view.x(ofColumn: 2)) < 0.001, "\(minX) against \(view.x(ofColumn: 2))")
        #expect(CoreTerminalView.markedX(at: 300, width: 80, in: 320) == 240)
        #expect(CoreTerminalView.markedX(at: 10, width: 400, in: 320) == 0, "never past the left edge")
    }

    /// B692: the grid changed size under a composition (a rotation, the
    /// iPad's window, a keyboard's bar coming or going). The core reflows
    /// the lines and takes the cursor with them; the pinyin stayed on the
    /// cell it began at, numbered in the old grid: below the view once the
    /// rows were fewer, off its line once the columns were.
    @Test func aCompositionStaysOnItsLineThroughAResize() throws {
        let (view, sink) = makeView(cols: 20, rows: 12)
        let window = inWindow(view)
        defer { window.isHidden = true }
        feed(view, "\u{1b}[11;1H$ abcdefghijklm", until: {
            $0.rows.info.cursor.row == 10 && $0.rows.info.cursor.col == 15
        })
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        // Fewer rows: the cursor's line stays on the screen, higher up. The
        // composition is there at once, before any output.
        view.frame.size.height = view.cellSize.height * 6
        view.layoutIfNeeded()
        #expect(Int(view.rows.info.rows) == 6)
        var cursor = view.rows.info.cursor
        #expect(cursor.row < 6 && cursor.col == 15, "the core lost the cursor: \(cursor.col),\(cursor.row)")
        var label = try #require(view.markedLabel)
        #expect(label.frame.maxY <= view.bounds.height + 0.001, "the composition is below the view: \(label.frame)")
        #expect(abs(label.frame.minY - view.y(ofRow: Int(cursor.row))) < 0.001, "off the cursor's row")
        #expect(abs(label.frame.minX - view.x(ofColumn: 15)) < 0.001)
        // The remote's redraw for the new size takes the cursor to the
        // line's start on its way: the composition stays.
        feed(view, "\r", until: { $0.rows.info.cursor.col == 0 })
        #expect(abs(label.frame.minX - view.x(ofColumn: 15)) < 0.001, "the redraw moved the composition")
        feed(view, "\u{1b}[16G", until: { $0.rows.info.cursor.col == 15 })
        // Fewer columns: the line wraps; the composition goes where the
        // cursor went, on the wrapped line's second row.
        view.frame.size.width = view.cellSize.width * 10 + 1
        view.layoutIfNeeded()
        #expect(Int(view.rows.info.cols) == 10)
        cursor = view.rows.info.cursor
        #expect(cursor.col == 5, "the core didn't wrap the line: \(cursor.col),\(cursor.row)")
        label = try #require(view.markedLabel)
        #expect(abs(label.frame.minY - view.y(ofRow: Int(cursor.row))) < 0.001, "off the cursor's row")
        #expect(abs(label.frame.minX - view.x(ofColumn: 5)) < 0.001, "\(label.frame.minX) against \(view.x(ofColumn: 5))")
        // Output meanwhile still doesn't move it (V7).
        feed(view, "\u{1b}[1;1H", until: { $0.rows.info.cursor.row == 0 })
        #expect(abs(label.frame.minX - view.x(ofColumn: 5)) < 0.001, "output moved the composition")
        // Still the same composition: committed, it goes out as picked.
        view.setMarkedText("你", selectedRange: NSRange(location: 1, length: 0))
        view.unmarkText()
        #expect(sink.take() == "你")
    }

    /// B693: a composition and the history. Begun while the view showed the
    /// history (scrolled up to read what an agent printed), the pinyin sat
    /// at the cursor, below the view: nothing showed what was being typed
    /// until it was committed. A key brings the live screen back (B363);
    /// composing didn't. And the history scrolled mid-composition slid
    /// under it: the pinyin stayed put on the screen, over old lines.
    @Test func aCompositionLandsOnTheLiveScreenAndScrollsWithIt() throws {
        let (view, sink) = makeView(cols: 20, rows: 5)
        let window = inWindow(view)
        defer { window.isHidden = true }
        var text = ""
        for i in 0..<50 { text += "\(i)\r\n" }
        feed(view, text + "$ ", until: { $0.rowText(4) == "$" && $0.rows.info.cursor.col == 2 })
        view.scrollLines(10)
        #expect(view.isReadingScrollback)
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        #expect(!view.isReadingScrollback, "composing left the view in the history")
        let label = try #require(view.markedLabel)
        #expect(label.frame.maxY <= view.bounds.height + 0.001, "the composition is below the view: \(label.frame)")
        #expect(abs(label.frame.minY - view.y(ofRow: 4)) < 0.001)
        #expect(abs(label.frame.minX - view.x(ofColumn: 2)) < 0.001)
        // Scrolled mid-composition: it goes down with its row.
        view.scrollLines(2)
        #expect(view.rows.info.display_offset == 2)
        #expect(abs(label.frame.minY - view.y(ofRow: 6)) < 0.001, "the composition stayed over the history")
        view.scrollToBottom()
        #expect(abs(label.frame.minY - view.y(ofRow: 4)) < 0.001)
        #expect(sink.sent.isEmpty)
        view.unmarkText()
        #expect(sink.take() == "ni")
    }

    /// B575: herdr's frames are paintings of a pane herdr emulates and
    /// answers for. A query among them (OSC 4's palette question) got this
    /// terminal's answer into herdr's input, which reached the program as
    /// typing: `]4;191;rgb:d7d7/ffff/5f5f` in Claude Code's input box. A
    /// terminal that answers no queries drops its replies and its focus
    /// reports; typing still goes.
    @Test func aTerminalThatPaintsFramesAnswersNoQueries() {
        let (view, _) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.attach(view)
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        coordinator.answersQueries = false
        view.push(Data("\u{1b}]4;191;?\u{1b}\\\u{1b}[6n\u{1b}[?1004h".utf8))
        #expect(view.settle { $0.termModes.focus_reports })
        _ = view.becomeFirstResponder()
        _ = view.resignFirstResponder()
        #expect(view.settle(timeout: 0.3) { _ in !sent.isEmpty } == false, "answered: \(String(decoding: sent, as: UTF8.self))")
        view.insertText("a")
        #expect(sent == Data("a".utf8), "typing still goes")
        // Answering again: the palette's colour 191, the cursor.
        sent = Data()
        coordinator.answersQueries = true
        view.push(Data("\u{1b}]4;191;?\u{1b}\\\u{1b}[6n".utf8))
        #expect(view.settle { _ in String(decoding: sent, as: UTF8.self).contains("R") })
        let answer = String(decoding: sent, as: UTF8.self)
        #expect(answer.contains("]4;191;rgb:d7d7/ffff/5f5f"), "\(answer)")
    }

    /// B578: a program split in panes (vim's windows, lazygit's panels)
    /// scrolls the one under the finger: the wheel goes to the cell the
    /// drag began on, kept on the grid; the grid's centre when no drag
    /// aimed it. It went to the centre always.
    @Test func theWheelGoesWhereTheDragBegan() {
        let (view, sink) = makeView(cols: 20, rows: 6)
        let window = inWindow(view)
        defer { window.isHidden = true }
        feed(view, "\u{1b}[?1000h\u{1b}[?1006h", until: { $0.termModes.mouse != 0 })
        view.sendWheel(lines: 1)
        #expect(sink.take() == "\u{1b}[<64;11;4M", "unaimed: the centre")
        view.gestures.wheelAim = (col: 3, row: 1)
        view.sendWheel(lines: -2)
        #expect(sink.take() == String(repeating: "\u{1b}[<65;4;2M", count: 2), "where the drag began")
        #expect(CoreTerminalView.wheelCell(aim: (col: 99, row: -4), cols: 20, rows: 6) == (col: 19, row: 0))
        #expect(CoreTerminalView.wheelCell(aim: nil, cols: 20, rows: 6) == (col: 10, row: 3))
    }

    /// B691: with a Chinese keyboard up, the bar's Ctrl then `c` composed
    /// Pinyin `c`, sent the candidate picked later and spent the Ctrl on
    /// it: no Ctrl-C from the bar. The key the input method would compose is
    /// the chord: the composition goes unsent and ^C goes out. Unarmed, or a
    /// composition already under way, or kana, composing goes on as before.
    @Test func anArmedCtrlTakesTheKeyAnInputMethodWouldCompose() {
        let (view, _) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.attach(view)
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        coordinator.pendingCtrl = true
        view.setMarkedText("c", selectedRange: NSRange(location: 1, length: 0))
        #expect(view.settle(timeout: 2) { _ in sent == Data([0x03]) }, "sent: \(Array(sent))")
        #expect(view.markedLabel == nil, "the composition went")
        #expect(view.markedTextRange == nil)
        #expect(!coordinator.pendingCtrl, "the Ctrl is spent on its chord")
        // Not armed: Pinyin composes, nothing goes.
        sent = Data()
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        #expect(view.settle(timeout: 0.3) { _ in !sent.isEmpty } == false)
        #expect(view.markedLabel?.attributedText?.string == "ni")
        // Armed during a composition: the composition is the person's.
        coordinator.pendingCtrl = true
        view.setMarkedText("nih", selectedRange: NSRange(location: 3, length: 0))
        #expect(view.settle(timeout: 0.3) { _ in !sent.isEmpty } == false)
        #expect(view.markedLabel?.attributedText?.string == "nih")
        view.unmarkText()
        #expect(sent == Data("nih".utf8), "the commit is typed, unfolded (B570: text of more than one character)")
        #expect(!coordinator.pendingCtrl, "and takes the armed Ctrl, as typing does")
        // Kana is no chord.
        sent = Data()
        coordinator.pendingCtrl = true
        view.setMarkedText("に", selectedRange: NSRange(location: 1, length: 0))
        #expect(view.settle(timeout: 0.3) { _ in !sent.isEmpty } == false)
        #expect(view.markedLabel?.attributedText?.string == "に")
        #expect(CoreTerminalView.ctrlChordKey("c") == 0x63)
        #expect(CoreTerminalView.ctrlChordKey("你") == nil)
        #expect(CoreTerminalView.ctrlChordKey("ab") == nil)
    }

    @Test func japaneseKanaCommitsWhole() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        view.setMarkedText("に", selectedRange: NSRange(location: 1, length: 0))
        view.setMarkedText("にほ", selectedRange: NSRange(location: 2, length: 0))
        view.setMarkedText("日本", selectedRange: NSRange(location: 2, length: 0))
        #expect(sink.sent.isEmpty)
        view.insertText("日本")
        #expect(sink.take() == "日本")
    }

    @Test func holdingDeleteKeepsRepeatingOverRemoteText() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        // UIKit measures one character back before each repeat and stops on
        // an empty range: the phantom keeps it from ever being empty (12).
        for _ in 0..<8 {
            let caret = view.selectedTextRange!.start
            let back = view.position(from: caret, offset: -1)!
            #expect(view.compare(back, to: caret) == .orderedAscending, "something to delete")
            view.deleteBackward()
        }
        #expect(sink.take() == String(repeating: "\u{7f}", count: 8))
        #expect(view.hasText, "always (patch 10)")
    }

    /// B412: the space bar's trackpad dragged left past what was typed:
    /// the caret keeps something in front of it, so a held delete goes on
    /// repeating over the remote's text (UIKit checks one character back
    /// before each repeat).
    @Test func theTrackpadNeverStopsAHeldDelete() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        view.insertText("ab")
        _ = sink.take()
        view.beginFloatingCursor(at: CGPoint(x: 300, y: 10))
        for step in 1...5 { view.updateFloatingCursor(at: CGPoint(x: 300 - CGFloat(step) * 10, y: 10)) }
        view.endFloatingCursor()
        for _ in 0..<10 {
            guard let caret = view.selectedTextRange?.start, let back = view.position(from: caret, offset: -1) else {
                Issue.record("no caret")
                return
            }
            #expect(view.compare(back, to: caret) == .orderedAscending, "nothing in front of the caret: the hold stops")
            view.deleteBackward()
        }
        #expect(sink.sent.filter { $0 == 0x7f }.count == 10)
    }

    @Test func aThirdPartyKeyboardsDeleteIsNotAReturn() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        // 豆包 deletes the last character by selecting the two before the
        // caret, deleting them and inserting the first again (patch 23).
        view.insertText("a")
        _ = sink.take()
        view.selectedTextRange = view.textRange(from: view.beginningOfDocument, to: view.endOfDocument)
        view.deleteBackward()
        view.insertText("\n")
        #expect(!sink.text.contains("\r"), "putting the phantom back pressed Return")
        #expect(sink.take() == "\u{7f}", "only the real character costs a backspace")
    }

    /// B410: a third-party keyboard's delete into the remote's own text
    /// (it selects the one character before the caret: the phantom); the
    /// Return pressed in a later turn goes out. It was swallowed.
    @Test func aReturnAfterADeleteIntoTheRemotesTextGoesOut() async {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        view.selectedTextRange = view.textRange(from: view.beginningOfDocument, to: view.endOfDocument)
        view.deleteBackward()
        #expect(sink.take() == "\u{7f}")
        // The next turn of the main queue: the person presses Return.
        await withCheckedContinuation { done in DispatchQueue.main.async { done.resume() } }
        view.insertText("\n")
        #expect(sink.take() == "\r", "the Return after the delete was swallowed")
    }

    /// B414: lines a keyboard inserts at once (its clipboard panel, quick
    /// phrases) go the session's paste way, not typed line by line.
    @Test func linesInsertedAtOnceArePasted() {
        let (view, sink) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        view.insertText("echo one\necho two")
        #expect(sink.sent.isEmpty, "typed: each line ran as it arrived")
        #expect(sink.pastes == ["echo one\necho two"])
    }

    @Test func traitsOfferNothingToTypeIntoTheRemote() {
        let (view, _) = makeView()
        #expect(view.autocorrectionType == .no)
        #expect(view.smartQuotesType == .no)
        #expect(view.inlinePredictionType == .no)
        #expect(view.writingToolsBehavior == UIWritingToolsBehavior.none)
        #expect(view.textContentType == UITextContentType(rawValue: "terminal"))
        #expect(!view.canBecomeFocused)
    }

    // MARK: Scrolling, selection, links

    @Test func pixelScrollingShowsTheHistoryAndComesBack() {
        let (view, _) = makeView(cols: 20, rows: 5)
        var text = ""
        for i in 0..<50 { text += "\(i)\r\n" }
        feed(view, text + "$", until: { $0.rowText(4) == "$" })
        let h = view.cellSize.height
        #expect(abs(view.scrollViewport(by: h * 2.5) - h * 2.5) < 1, "on the pixel grid")
        #expect(view.isReadingScrollback)
        #expect(view.rows.rows.count == 6, "the row under the viewport rides along")
        // Output while reading does not move the view.
        let before = view.rowText(0)
        feed(view, "\r\nmore", until: { _ in true })
        _ = view.settle(timeout: 0.2) { _ in false }
        #expect(view.rowText(0) == before)
        view.scrollToBottom()
        #expect(!view.isReadingScrollback)
        #expect(view.settle { $0.rowText(4) == "more" })
    }

    /// B479: a pinch to another font size while reading history leaves the
    /// reader in the history, on the same line. It went to the bottom at
    /// every step of the pinch.
    @Test func aNewFontSizeLeavesAHistoryReaderReading() {
        let (view, _) = makeView(cols: 20, rows: 5)
        var text = ""
        for i in 0..<50 { text += "\(i)\r\n" }
        feed(view, text + "$", until: { $0.rowText(4) == "$" })
        view.scrollLines(10)
        let top = view.rowText(0)
        view.apply(font: Self.font.withSize(14))
        let reading = view.isReadingScrollback
        #expect(reading, "the new size sent the reader to the bottom")
        let still = view.rowText(0)
        #expect(still == top)
    }

    /// B426: the terminal's answer to a program's question goes out on the
    /// session's input path, which releases the scroll hold for what it
    /// sends; a history reader stays where it is all the same. A key still
    /// brings it back. It went to the bottom whenever a program asked.
    @Test func aReplyLeavesAHistoryReaderWhereItIs() {
        let (view, sink) = makeView(cols: 20, rows: 5)
        // As the session's coordinator: everything sent releases the hold.
        sink.onSend = { $0.releaseScrollHold() }
        var text = ""
        for i in 0..<50 { text += "\(i)\r\n" }
        feed(view, text + "$", until: { $0.rowText(4) == "$" })
        view.scrollLines(10)
        let reading = view.isReadingScrollback
        #expect(reading)
        view.push(Data("\u{1b}[6n".utf8))
        let answered = view.settle { _ in sink.text.contains("R") }
        #expect(answered)
        let stillReading = view.isReadingScrollback
        #expect(stillReading, "the reply sent the reader to the bottom")
        view.sendKey(UInt32(MP_KEY_CHAR), scalar: 0x78, mods: 0)
        let live = !view.isReadingScrollback
        #expect(live, "a key goes back to the live screen")
    }

    @Test func aSelectionEndingInTheLastColumnTakesIt() {
        let (view, _) = makeView(cols: 10, rows: 3)
        feed(view, "0123456789", until: { $0.rowText(0) == "0123456789" })
        // A drag to the edge after the last cell (patch 55).
        let e = view.edge(at: CGPoint(x: view.cellSize.width * 10 + 3, y: 2))
        #expect(e.after && e.col == 9)
        guard let term = view.term else { return }
        mp_term_select_start(term, 0, 0, 5, false)
        mp_term_select_extend(term, 0, UInt16(e.col), e.after)
        #expect(view.selectedText == "56789")
    }

    // MARK: Lifetime, handles, the pointer, predictions

    /// V4: a view its host lets go is freed, with its core (the display
    /// link, the blink timer and the core's waker hold it weakly).
    @Test func aViewLetGoIsFreed() {
        weak var weakView: CoreTerminalView?
        autoreleasepool {
            let (view, _) = makeView()
            view.push(Data("hello".utf8))
            _ = view.settle { $0.rowText(0) == "hello" }
            weakView = view
        }
        for _ in 0..<50 where weakView != nil {
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        #expect(weakView == nil, "the view outlived its last owner")
    }

    /// V16: a selection's handles are finger-sized and sit on its ends.
    @Test func handlesAreFingerSized() {
        let (view, _) = makeView()
        let window = inWindow(view)
        defer { window.isHidden = true }
        feed(view, "hello brave world", until: { $0.rowText(0) == "hello brave world" })
        view.select(kind: 1, row: 0, col: 7)
        let handles = try? #require(view.handles)
        #expect((handles?.start.frame.width ?? 0) >= 44 && (handles?.start.frame.height ?? 0) >= 44)
        #expect(abs((handles?.start.frame.midX ?? 0) - view.cellSize.width * 6) < 1, "before `brave`")
        #expect(abs((handles?.end.frame.midX ?? 0) - view.cellSize.width * 11) < 1, "after it")
        view.clearSelection()
        #expect(view.handles == nil)
    }

    /// V24: the pointer over the terminal is an I-beam a row tall.
    @Test func thePointerIsAnIBeam() {
        let (view, _) = makeView()
        let interaction = UIPointerInteraction(delegate: view)
        let style = view.pointerInteraction(interaction, styleFor: UIPointerRegion(rect: view.bounds))
        #expect(style != nil)
    }

    // MARK: Drawing

    /// The core's frame, drawn: every space cell shows its background, every
    /// printed cell has ink, a wide character inks both its columns, the
    /// block cursor shows the cursor's colour.
    @Test func whatIsDrawnIsTheCoresFrame() throws {
        let (view, _) = makeView(cols: 24, rows: 6)
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        feed(view, "\u{1b}[41m  \u{1b}[0m ab\u{1b}[44mC\u{1b}[0m 中文 😀\r\n\u{1b}[7mrev\u{1b}[0m",
             until: { $0.rowText(1) == "rev" })
        let shot = try #require(view.renderedPixels())
        let scale = view.metrics.scale
        let cw = Int((view.cellSize.width * scale).rounded()), ch = Int((view.cellSize.height * scale).rounded())
        func pixel(_ x: Int, _ y: Int) -> UInt32 {
            let i = (y * shot.width + x) * 4
            return UInt32(shot.pixels[i + 2]) << 16 | UInt32(shot.pixels[i + 1]) << 8 | UInt32(shot.pixels[i])
        }
        func ink(col: Int, row: Int, background: UInt32) -> Bool {
            for y in row * ch..<(row + 1) * ch { for x in col * cw..<(col + 1) * cw where pixel(x, y) != background { return true } }
            return false
        }
        let rows = view.rows
        for (c, cell) in rows.rows[0].cells.enumerated() {
            let bg = view.style.background(of: cell)
            let blank = cell.ch == 0x20 && !coreHas(cell, Int(MP_CELL_SPACER))
            if blank {
                #expect(pixel(c * cw + cw / 2, ch / 2) == bg, "cell \(c)'s background")
            } else if !coreHas(cell, Int(MP_CELL_SPACER)) {
                #expect(ink(col: c, row: 0, background: bg), "cell \(c) has ink")
            }
        }
        // 中 at column 7 (wide): ink in both of its columns.
        let wide = try #require(rows.rows[0].cells.firstIndex { $0.ch == 0x4E2D })
        #expect(coreHas(rows.rows[0].cells[wide], Int(MP_CELL_WIDE)))
        #expect(ink(col: wide + 1, row: 0, background: rows.rows[0].cells[wide].bg))
    }

    /// A frame that changes a few rows, drawn on top of what was there,
    /// equals a fresh view's full drawing of the same bytes (B20, the fork's
    /// 531-comparison check of patches 24–28): the damage the core reports
    /// is complete. Over the recorded mosh pane switch and seeded output.
    @Test func partialRedrawsEqualAFullRedraw() throws {
        let (view, _) = makeView(cols: 60, rows: 20)
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        var chunks: [Data] = []
        for name in ["phase0.bin", "phase1.bin", "phase2.bin", "phase3.bin"] {
            if let d = try? TestFixtures.data("mosh-switch/\(name)") {
                stride(from: 0, to: d.count, by: 4096).forEach { chunks.append(d.subdata(in: $0..<min($0 + 4096, d.count))) }
            }
        }
        var seed: UInt64 = 0x2026_1112
        func next() -> UInt64 { seed = seed &* 6364136223846793005 &+ 1442695040888963407; return seed >> 33 }
        let palette = ["\u{1b}[31m", "\u{1b}[1;32m", "\u{1b}[0m", "\u{1b}[7m", "\u{1b}[4m", "中", "é", "😀", "\r\n",
                       "\u{1b}[2K", "\u{1b}[5;5H", "\u{1b}[3X", "\u{1b}[2b", "abc", "\u{1b}[?25l", "\u{1b}[?25h"]
        for _ in 0..<40 {
            var s = ""
            for _ in 0..<12 { s += palette[Int(next() % UInt64(palette.count))] }
            chunks.append(Data(s.utf8))
        }
        var all = Data()
        var compared = 0
        for (i, chunk) in chunks.enumerated() {
            all.append(chunk)
            view.push(chunk)
            _ = view.settle(timeout: 0.3) { _ in false }
            guard i % 4 == 3 || i == chunks.count - 1 else { continue }
            let (fresh, _) = makeView(cols: 60, rows: 20)
            host(fresh, in: window)
            fresh.push(all)
            _ = fresh.settle(timeout: 0.5) { _ in false }
            let a = try #require(view.renderedPixels()), b = try #require(fresh.renderedPixels())
            #expect(a.pixels == b.pixels, "chunk \(i): a partial redraw differs from a full one")
            compared += 1
            fresh.removeFromSuperview()
        }
        #expect(compared > 5)
    }

    private func host(_ view: CoreTerminalView, in window: UIWindow) {
        window.rootViewController?.view.addSubview(view)
    }
}

/// B472: a glyph wider than its cells was narrowed on x alone, so a colour
/// emoji in one cell (Claude Code's "⏺" bullets, drawn from Apple Color
/// Emoji) kept its full height and came out a tall thin pill. Now it is
/// scaled uniformly to fit, its shape kept, centred, inside the line; box
/// drawing still fills the cell's height; a glyph whose ink fits is only
/// moved into its cells.
@Suite("glyphs wider than their cells keep their shape (B472)")
struct GlyphFitTests {
    private static let metrics = CoreTermMetrics(
        font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular), scale: 3)

    /// The ink's width over its height, drawn at its own size.
    private static func naturalAspect(_ text: String) -> CGFloat? {
        let m = metrics
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [
            .font: m.font, .foregroundColor: UIColor.white,
        ]))
        let w = 400, h = 200
        var data = [UInt8](repeating: 0, count: w * h * 4)
        var minX = w, minY = h, maxX = -1, maxY = -1
        data.withUnsafeMutableBytes { raw in
            guard let ctx = CGContext(data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8,
                                      bytesPerRow: w * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                                          | CGBitmapInfo.byteOrder32Little.rawValue) else { return }
            ctx.translateBy(x: 40, y: 60)
            ctx.scaleBy(x: m.scale, y: m.scale)
            ctx.textPosition = .zero
            CTLineDraw(line, ctx)
        }
        for y in 0..<h {
            for x in 0..<w where data[(y * w + x) * 4 + 3] != 0 {
                minX = min(minX, x); maxX = max(maxX, x); minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        guard maxX >= 0 else { return nil }
        return CGFloat(maxX - minX + 1) / CGFloat(maxY - minY + 1)
    }

    @Test("Emoji keep their shape in one cell and in two", arguments: ["⏺", "🍓"], [1, 2])
    func emojiKeepTheirShape(text: String, cells: Int) throws {
        let m = Self.metrics
        let natural = try #require(Self.naturalAspect(text))
        let ink = try #require(CoreTermAtlas.ink(text, style: cells == 2 ? 4 : 0, metrics: m))
        let aspect = CGFloat(ink.w) / CGFloat(ink.h)
        #expect(abs(aspect - natural) / natural <= 0.10,
                "\(text) in \(cells) cell(s): \(ink.w)×\(ink.h) px, aspect \(aspect) against \(natural)")
        // Inside its cells across.
        let span = cells * m.widthPx
        #expect(ink.dx >= -1 && ink.dx + ink.w <= span + 1, "\(ink.dx)+\(ink.w) in \(span)")
    }

    @Test("Wider than its cells: scaled uniformly, centred across and between baseline and ascent")
    func aWideInkIsScaledUniformly() throws {
        let fit = try #require(CoreTermGlyphFit.fit(
            advance: 16, ink: CGRect(x: 0, y: -2, width: 15, height: 15), span: 7.5,
            ascent: 9, descent: 3, scalar: 0x1F353))
        #expect(fit.scaleX == fit.scaleY, "its shape kept")
        #expect(abs(fit.scaleX - 0.5) < 0.0001)
        // Across: the 7.5 pt ink fills the 7.5 pt span from 0.
        #expect(abs(fit.dx - 0) < 0.0001)
        // Up: its middle at half the ascent.
        let middle = fit.dy + (-2 + 15.0 / 2) * fit.scaleY
        #expect(abs(middle - 4.5) < 0.0001)
    }

    @Test("Never taller than the line: kept between descent and ascent")
    func aTallInkStaysInsideTheLine() throws {
        let fit = try #require(CoreTermGlyphFit.fit(
            advance: 16, ink: CGRect(x: 0, y: -4, width: 15, height: 15), span: 15,
            ascent: 9, descent: 3, scalar: 0x1F353))
        #expect(fit.scaleY <= 12.0 / 15 + 0.0001, "the line is 12 pt")
        let top = fit.dy + 11 * fit.scaleY, bottom = fit.dy - 4 * fit.scaleY
        #expect(top <= 9.0001 && bottom >= -3.0001, "\(bottom)…\(top)")
    }

    @Test("A wide advance whose ink fits is moved into its cells, not shrunk")
    func anInkThatFitsIsOnlyMoved() throws {
        // "⎿" from a CJK fallback: 12 pt of advance, 6.2 pt of ink.
        let fit = try #require(CoreTermGlyphFit.fit(
            advance: 12, ink: CGRect(x: 4, y: -1.4, width: 6.2, height: 12), span: 7.2,
            ascent: 9, descent: 3, scalar: 0x23BF))
        #expect(fit.scaleX == 1 && fit.scaleY == 1)
        #expect(fit.dy == 0, "where its font put it")
        #expect(abs((fit.dx + 4) - 0.5) < 0.0001, "centred across")
    }

    @Test("Box drawing and blocks still fill the cell's height: narrowed on x")
    func boxDrawingKeepsTheCellsHeight() throws {
        for scalar: UInt32 in [0x2502, 0x2588, 0xE0B0] {
            let fit = try #require(CoreTermGlyphFit.fit(
                advance: 12, ink: CGRect(x: 0, y: -3, width: 12, height: 15), span: 7.2,
                ascent: 9, descent: 3, scalar: scalar))
            #expect(fit.scaleY == 1 && abs(fit.scaleX - 0.6) < 0.0001 && fit.dy == 0)
        }
    }

    @Test("A glyph that fits its cells is left alone")
    func aGlyphThatFitsIsLeftAlone() {
        #expect(CoreTermGlyphFit.fit(advance: 7.2, ink: CGRect(x: 0, y: 0, width: 7, height: 7), span: 7.2,
                                     ascent: 9, descent: 3, scalar: 0x2713) == nil)
    }
}

/// B473: the grid's leftover width all went to the right (columns are the
/// width's floor), so the content looked shoved left beside a strip; with
/// the page's near-black ground around a tmux pane's inset, the strip read
/// as black. The grid is centred now, the same origin for drawing and for
/// every tap.
@Suite("the grid sits centred across the terminal (B473)")
struct GridCentreTests {
    @Test("A width that isn't a whole number of cells leaves equal margins, to a pixel")
    func marginsAreEqual() {
        for (width, cell, scale) in [(402.0, 5.6, 3.0), (394.0, 7.2246, 3.0), (744.0, 8.4, 2.0), (320.0, 7.0, 2.0)] {
            let cols = Int(width / cell)
            let left = CoreTerminalView.gridOrigin(width: width, cols: cols, cellWidth: cell, scale: scale)
            let right = width - left - CGFloat(cols) * cell
            #expect(abs(left - right) <= 1 / scale + 0.0001, "\(width) / \(cell): \(left) against \(right)")
            #expect(left >= 0 && right >= -0.0001)
        }
    }

    @Test("A whole number of cells leaves no margin")
    func noMarginWhenItFits() {
        #expect(CoreTerminalView.gridOrigin(width: 560, cols: 100, cellWidth: 5.6, scale: 3) == 0)
    }

    @Test("A tap maps to the cell drawn under it: the first cell's centre is column 0")
    func tapsMapThroughTheSameOrigin() {
        let width: CGFloat = 402, cell: CGFloat = 5.6, scale: CGFloat = 3
        let cols = Int(width / cell)
        let origin = CoreTerminalView.gridOrigin(width: width, cols: cols, cellWidth: cell, scale: scale)
        #expect(origin > 0)
        #expect(CoreTerminalView.column(atX: origin + cell / 2, originX: origin, cellWidth: cell, cols: cols) == 0)
        #expect(CoreTerminalView.column(atX: origin + cell * 1.5, originX: origin, cellWidth: cell, cols: cols) == 1)
        let last = origin + CGFloat(cols - 1) * cell + cell / 2
        #expect(CoreTerminalView.column(atX: last, originX: origin, cellWidth: cell, cols: cols) == cols - 1)
        // The margins belong to the edge columns.
        #expect(CoreTerminalView.column(atX: 0.5, originX: origin, cellWidth: cell, cols: cols) == 0)
        #expect(CoreTerminalView.column(atX: width - 0.5, originX: origin, cellWidth: cell, cols: cols) == cols - 1)
        // Without the origin, the first cell's centre sat a margin to the
        // right of where it was drawn.
        #expect(CoreTerminalView.column(atX: cell / 2, originX: origin, cellWidth: cell, cols: cols) == 0)
    }

    @Test("A view's grid is centred, and its taps land on the cells drawn there")
    @MainActor
    func aViewCentresItsGrid() throws {
        let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        let view = CoreTerminalView(frame: CGRect(x: 0, y: 0, width: 402, height: 600), font: font)
        view.layoutIfNeeded()
        let cell = view.cellSize
        let cols = view.grid.cols
        let left = view.gridOriginX
        let right = view.bounds.width - view.x(ofColumn: cols)
        #expect(abs(left - right) <= 1 / view.metrics.scale + 0.0001, "\(left) against \(right)")
        let first = view.cell(at: CGPoint(x: view.x(ofColumn: 0) + cell.width / 2, y: cell.height / 2))
        #expect(first.col == 0 && first.row == 0)
        let next = view.cell(at: CGPoint(x: view.x(ofColumn: 1) + cell.width / 2, y: cell.height / 2))
        #expect(next.col == 1)
    }
}

/// B495: the margins the grid leaves (B473 centred them) were the theme's
/// background, the grid's own background could be another (a remote's
/// OSC 11, a program painting every cell): dark strips either side. They
/// take what the grid shows at its edges now; and the core's default
/// background is the theme's for every theme, so with nothing painted the
/// two are one colour.
@Suite("what lies outside the grid matches its edges (B495)")
struct GridGroundTests {
    @Test("The colour most edge cells have, if more than half do; else the fallback")
    func theMajorityOfTheEdges() {
        let grey: UInt32 = 0x282A36, black: UInt32 = 0x000000, blue: UInt32 = 0x0000AA
        #expect(CoreTerminalView.ground(edges: Array(repeating: grey, count: 40), fallback: black) == grey)
        #expect(CoreTerminalView.ground(edges: Array(repeating: grey, count: 38) + [blue, blue], fallback: black) == grey,
                "a status row of its own doesn't count")
        #expect(CoreTerminalView.ground(edges: [grey, blue, grey, blue], fallback: black) == black, "no majority")
        #expect(CoreTerminalView.ground(edges: [], fallback: black) == black)
    }

    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private static func look(_ theme: TerminalTheme) -> TerminalLook {
        TerminalLook(theme: theme, cursorShape: .block, cursorColorId: "teal", cursorBlink: false)
    }

    @Test("Every built-in theme: the core's default background is the one the view keeps")
    @MainActor
    func theCoreAndTheViewAgreeOnEveryTheme() {
        let view = CoreTerminalView(frame: CGRect(x: 0, y: 0, width: 360, height: 480), font: Self.font)
        for theme in TerminalTheme.builtIns {
            view.apply(look: Self.look(theme))
            #expect(view.settle { $0.rows.info.background == $0.style.background },
                    "\(theme.name): core \(String(view.rows.info.background, radix: 16)) against \(String(view.style.background, radix: 16))")
            #expect(view.style.outside == view.style.background, "\(theme.name): nothing painted, nothing else outside")
        }
    }

    @Test("A grid painted grey on a black theme leaves grey margins, not black ones")
    @MainActor
    func marginsFollowAPaintedGrid() {
        let view = CoreTerminalView(frame: CGRect(x: 0, y: 0, width: 362, height: 480), font: Self.font)
        let black = TerminalTheme.builtIns.first { $0.backgroundHex.lowercased().hasSuffix("000000") } ?? TerminalTheme.fallback
        view.apply(look: Self.look(black))
        // Erase in a set background: every cell takes it (background colour erase).
        view.feed(Data("\u{1b}[48;2;40;42;54m\u{1b}[2J".utf8))
        #expect(view.settle { $0.style.outside == 0x282A36 }, "outside \(String(view.style.outside, radix: 16))")
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        view.backgroundColor?.getRed(&r, green: &g, blue: &b, alpha: &a)
        #expect(abs(r * 255 - 40) < 1 && abs(g * 255 - 42) < 1 && abs(b * 255 - 54) < 1, "the gutter is the grid's grey")
    }
}
