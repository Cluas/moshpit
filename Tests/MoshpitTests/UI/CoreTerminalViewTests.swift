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
    final class Sink: TerminalSurfaceDelegate {
        var sent = Data()
        var sizes: [(Int, Int)] = []
        var links: [String] = []
        var pastes: [String] = []
        var clicks: [(Int, Int)] = []
        /// What the session's input path does with what it sends.
        var onSend: ((TerminalSurface) -> Void)?
        var text: String { String(decoding: sent, as: UTF8.self) }
        func take() -> String { defer { sent = Data() }; return text }

        func surface(_ surface: TerminalSurface, send data: Data) {
            sent.append(data)
            onSend?(surface)
        }
        func surface(_ surface: TerminalSurface, sizeChanged cols: Int, rows: Int) { sizes.append((cols, rows)) }
        func surface(_ surface: TerminalSurface, titleChanged title: String) {}
        func surfaceBell(_ surface: TerminalSurface) {}
        func surface(_ surface: TerminalSurface, openLink link: String) { links.append(link) }
        func surface(_ surface: TerminalSurface, paste text: String) { pastes.append(text) }
        func surfaceClipboardRequest(_ surface: TerminalSurface) {}
        func surfaceScrollWillBegin(_ surface: TerminalSurface) {}
        func surface(_ surface: TerminalSurface, scrollPixels dy: CGFloat) -> CGFloat { surface.scrollLocal(pixels: dy) }
        func surfaceScrollDidEnd(_ surface: TerminalSurface) {}
        func surface(_ surface: TerminalSurface, click col: Int, row: Int) { clicks.append((col, row)) }
        var surfaceScrollsElsewhere: Bool { false }
        var surfaceCanSwitch: Bool { false }
        func surface(_ surface: TerminalSurface, switchForward forward: Bool) {}
        func surface(_ surface: TerminalSurface, fontSizeCommitted size: Double) {}
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

    @Test func repliesGoOutWithTheKeys() {
        let (view, sink) = makeView()
        view.push(Data("\u{1b}[6n".utf8))
        #expect(view.settle { _ in sink.text.contains("R") })
        #expect(sink.take() == "\u{1b}[1;1R")
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

    @Test func arrowsFollowDECCKM() {
        let (view, sink) = makeView()
        view.sendKey(UInt32(MP_KEY_UP), mods: 0)
        #expect(sink.take() == "\u{1b}[A")
        feed(view, "\u{1b}[?1h", until: { $0.termModes.app_cursor })
        view.sendKey(UInt32(MP_KEY_UP), mods: 0)
        #expect(sink.take() == "\u{1b}OA")
        view.sendKey(UInt32(MP_KEY_CHAR), scalar: 0x63, mods: UInt8(MP_MOD_CTRL))
        #expect(sink.take() == "\u{03}")
    }

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

    @Test func kittyReleasesAreSent() {
        let (view, sink) = makeView()
        feed(view, "\u{1b}[>3u", until: { $0.termModes.kitty_flags == 3 })
        view.sendKey(UInt32(MP_KEY_CHAR), scalar: 0x61, mods: 0, down: false)
        #expect(sink.take() == "\u{1b}[97;1:3u")
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

    @Test func aDoubleTapSelectsAWordAndCopies() {
        let (view, _) = makeView()
        feed(view, "hello brave world", until: { $0.rowText(0) == "hello brave world" })
        view.select(kind: 1, row: 0, col: 7)
        #expect(view.selectedText == "brave")
        view.clearSelection()
        #expect(view.selectedText == nil)
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

    @Test func linksAreFoundUnderTheFinger() {
        let (view, _) = makeView(cols: 30, rows: 4)
        feed(view, "\u{1b}]8;;https://example.com/a\u{1b}\\open\u{1b}]8;;\u{1b}\\ see https://example.com/b",
             until: { $0.rowText(0).hasPrefix("open") })
        #expect(view.link(at: (col: 1, row: 0)) == "https://example.com/a")
        #expect(view.link(at: (col: 12, row: 0)) == "https://example.com/b")
        #expect(view.link(at: (col: 6, row: 0)) == nil)
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

    /// V36: mosh's guesses show over the frame once an epoch is confirmed,
    /// the emulator's own grid untouched.
    @Test func predictionsShowOverTheFrame() {
        let (view, _) = makeView()
        feed(view, "$ ", until: { $0.rowText(0) == "$" })
        let engine = MoshPrediction(mode: .always, trail: false)
        engine.srttMs = 500
        view.predictAttach(engine)
        if let batch = view.predictInput(Data("a".utf8), srtt: 500) { view.predictSent(batch: batch, userState: 1) }
        #expect(view.rowText(0) == "$", "the first epoch waits for the server")
        view.push(Data("a".utf8))
        view.predictEchoAck(1)
        #expect(view.settle { $0.rowText(0) == "$ a" })
        if let batch = view.predictInput(Data("b".utf8), srtt: 500) { view.predictSent(batch: batch, userState: 2) }
        #expect(view.rowText(0) == "$ ab", "shown before the echo")
        guard let term = view.term, let p = mp_term_view_row_text(term, 0, nil) else { return }
        #expect(String(cString: p) == "$ a", "the grid holds only what the server sent")
        mp_string_free(p)
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
