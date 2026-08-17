import Foundation
import SwiftTerm
import Testing
@testable import Moshpit

/// The "white cursor-sized blocks over mosh" bug, reproduced headlessly.
///
/// Field report: the first pane view after connecting is clean; the blocks
/// appear only after SWITCHING panes, and never go away. The app's switch is
/// a `select-pane -Z` issued out of band on the SSH `-CC` control lane
/// (``TmuxSessionController/selectPane``), so the mosh screen simply receives
/// tmux's zoom-relayout repaint burst.
///
/// THE MECHANISM (see `orphanedWideCharContinuationRendersAsABlock`):
///
///   * mosh-server's framebuffer stores a double-width char in ONE cell and
///     leaves the following cell an ordinary blank.
///   * SwiftTerm stores it as a width-2 cell plus a width-0 CONTINUATION cell
///     whose background is `.defaultInvertedColor` (Buffer.insertCharacter).
///   * When a repaint writes a NARROW char over the wide char's first half
///     and the new row ends there, mosh sees blank→blank at the second half
///     and emits nothing at all for that column.
///   * SwiftTerm is left with a width-0 cell that no longer has a wide char
///     in front of it. `AppleTerminalView.buildAttributedString` only skips
///     continuations by advancing `col += 2` past a real wide char, so this
///     orphan is visited with `max(1, width) == 1` and drawn as a space with
///     `bg == .defaultInvertedColor`, which `mapColor` resolves to
///     `nativeBackgroundColor.inverseColor()` — a near-white filled cell.
///   * mosh believes that cell is already correct and never resends it, so
///     the block is permanent. Exactly the report.
///
/// The fixtures under `Fixtures/mosh-switch/` are a REAL capture (loopback,
/// no packet loss) produced by `scripts/capture-mosh-switch-bytes.sh`, which
/// drives the app's own ``MoshTransport`` against a real mosh-server + tmux.
@Suite("mosh pane-switch render divergence")
struct MoshSwitchReplayTests {

    // MARK: - Fixtures

    /// Read from the source tree rather than the bundle — same reason as
    /// ``HerdrSnapshotTests``: whether a fixture gets copied as a bundle
    /// resource depends on how the target was generated.
    private static func fixture(_ name: String) -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // …/MoshpitTests/Services
            .deletingLastPathComponent()   // …/MoshpitTests
            .appendingPathComponent("Fixtures/mosh-switch/\(name)")
    }

    private static func bytes(_ name: String) throws -> [UInt8] {
        [UInt8](try Data(contentsOf: fixture(name)))
    }

    private struct Meta: Decodable {
        struct Phase: Decodable {
            let index: Int
            let bytes: String
            let truth: String
            let pane: String
            let what: String
        }
        let cols: Int
        let rows: Int
        let reference: String
        let phases: [Phase]
    }

    private static func meta() throws -> Meta {
        try JSONDecoder().decode(Meta.self, from: Data(contentsOf: fixture("meta.json")))
    }

    // MARK: - Headless terminal

    /// `Terminal.tdel` is weak, so the delegate has to outlive the terminal.
    private final class Sink: TerminalDelegate {
        func send(source: Terminal, data: ArraySlice<UInt8>) {}
    }

    private static func terminal(_ sink: Sink, cols: Int, rows: Int) -> Terminal {
        Terminal(delegate: sink, options: TerminalOptions(cols: cols, rows: rows, scrollback: 500))
    }

    /// One screen cell, reduced to what a divergence report needs.
    private struct Cell: Equatable {
        var text: String
        var width: Int
        var inverse: Bool
        var fg: String
        var bg: String

        var render: String {
            let glyph = text == " " ? "␠" : (text.isEmpty ? "·" : text)
            var tags: [String] = []
            if inverse { tags.append("INVERSE") }
            if fg != "default" { tags.append("fg=\(fg)") }
            if bg != "default" { tags.append("bg=\(bg)") }
            if width != 1 { tags.append("width=\(width)") }
            return tags.isEmpty ? glyph : "\(glyph)[\(tags.joined(separator: ","))]"
        }
    }

    private static func name(_ color: Attribute.Color) -> String {
        switch color {
        case .defaultColor: return "default"
        case .defaultInvertedColor: return "defaultInverted"
        case .ansi256(let code): return "ansi\(code)"
        case .trueColor(let r, let g, let b): return "rgb(\(r),\(g),\(b))"
        }
    }

    private static func grid(_ terminal: Terminal, cols: Int, rows: Int) -> [[Cell]] {
        (0..<rows).map { row in
            (0..<cols).map { col in
                guard let cd = terminal.getCharData(col: col, row: row) else {
                    return Cell(text: "?", width: 1, inverse: false, fg: "default", bg: "default")
                }
                let ch = cd.getCharacter()
                return Cell(text: ch == "\0" ? "" : String(ch),
                            width: Int(cd.width),
                            inverse: cd.attribute.style.contains(.inverse),
                            fg: name(cd.attribute.fg),
                            bg: name(cd.attribute.bg))
            }
        }
    }

    /// tmux's `capture-pane -p -e` frame, replayed the way the app's own
    /// `TmuxSessionController.resyncPane` replays one: home, erase-display,
    /// the rows, SGR reset.
    private static func truthFrame(_ text: String) -> String {
        var lines = text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        return "\u{1b}[H\u{1b}[2J" + lines.joined(separator: "\r\n") + "\u{1b}[0m"
    }

    /// Display width of a capture-pane line, ignoring its SGR escapes. Past
    /// this column tmux told us nothing: `grid_line_length` trims trailing
    /// single-space cells REGARDLESS of their attributes, so a reverse-video
    /// space parked at a line end is simply absent from the capture.
    private static func truthWidth(_ line: String) -> Int {
        var total = 0, i = line.startIndex
        while i < line.endIndex {
            if line[i] == "\u{1b}" {
                var j = line.index(after: i)
                if j < line.endIndex { j = line.index(after: j) }
                while j < line.endIndex, !("@"..."~" ~= line[j]) { j = line.index(after: j) }
                i = j < line.endIndex ? line.index(after: j) : line.endIndex
                continue
            }
            total += (line[i].unicodeScalars.first.map(isWide) ?? false) ? 2 : 1
            i = line.index(after: i)
        }
        return total
    }

    private static func isWide(_ s: Unicode.Scalar) -> Bool {
        switch s.value {
        case 0x1100...0x115F, 0x2E80...0x303E, 0x3041...0x33FF, 0x3400...0x4DBF,
             0x4E00...0x9FFF, 0xA000...0xA4CF, 0xAC00...0xD7A3, 0xF900...0xFAFF,
             0xFE30...0xFE6F, 0xFF00...0xFF60, 0xFFE0...0xFFE6,
             0x1F300...0x1F64F, 0x1F900...0x1F9FF, 0x20000...0x2FFFD, 0x30000...0x3FFFD:
            return true
        default: return false
        }
    }

    // MARK: - The green half: characters agree with tmux

    /// Guards the half of the pipeline that is NOT broken: replaying mosh's
    /// bytes puts the right CHARACTERS in the right columns, through the
    /// initial sync and all three pane switches — CJK wraps, box drawing,
    /// emoji, inverse runs and all. Whatever else changes, this must hold, or
    /// the divergence below has a second, much bigger cause.
    ///
    /// Only the columns tmux actually reported are compared (see
    /// ``truthWidth``).
    @Test("mosh's incremental repaint puts the right glyphs in the right columns")
    func charactersAgreeWithTmux() throws {
        let meta = try Self.meta()
        let sink = Sink()
        let incremental = Self.terminal(sink, cols: meta.cols, rows: meta.rows)

        for phase in meta.phases {
            incremental.feed(byteArray: try Self.bytes(phase.bytes))

            let truthText = try String(contentsOf: Self.fixture(phase.truth), encoding: .utf8)
            var truthLines = truthText.components(separatedBy: "\n")
            if truthLines.last == "" { truthLines.removeLast() }
            let refSink = Sink()
            let reference = Self.terminal(refSink, cols: meta.cols, rows: meta.rows)
            reference.feed(text: Self.truthFrame(truthText))

            let got = Self.grid(incremental, cols: meta.cols, rows: meta.rows)
            let want = Self.grid(reference, cols: meta.cols, rows: meta.rows)

            var mismatches: [String] = []
            for row in 0..<meta.rows {
                let limit = row < truthLines.count ? min(meta.cols, Self.truthWidth(truthLines[row])) : 0
                for col in 0..<limit where got[row][col].text != want[row][col].text
                    || got[row][col].width != want[row][col].width {
                    mismatches.append("r\(row + 1)c\(col + 1): tmux=\(want[row][col].render) mosh=\(got[row][col].render)")
                }
            }
            #expect(mismatches.isEmpty, Comment(rawValue:
                "phase \(phase.index) (\(phase.what)) diverged from tmux:\n" + mismatches.joined(separator: "\n")))
        }
    }

    // MARK: - The repro

    /// THE BUG, minimal and fixture-free.
    ///
    /// This is the exact byte sequence mosh emitted in the captured switch
    /// burst — `ESC [ 34 ; 88 H a`, i.e. "put one narrow char at row 34
    /// column 88" — landing on a row that previously held a wide char at
    /// columns 88-89. mosh emits nothing for column 89 because in ITS
    /// framebuffer that cell was blank before and is blank after.
    ///
    /// The surviving cell is width 0 with `bg == .defaultInvertedColor` and no
    /// wide char in front of it, which is what the renderer paints as a
    /// near-white block.
    @Test("orphaned wide-char continuation survives a mosh minimal repaint")
    func orphanedWideCharContinuationRendersAsABlock() {
        let sink = Sink()
        let terminal = Self.terminal(sink, cols: 10, rows: 3)

        // A CJK char at columns 3-4 (any wide char — inverse video is NOT
        // required, which is why the blocks cluster wherever CJK does).
        terminal.feed(text: "\u{1b}[1;1Hab汉")
        #expect(terminal.getCharData(col: 2, row: 0)?.width == 2)
        #expect(terminal.getCharData(col: 3, row: 0)?.width == 0)

        // mosh's minimal repaint of the new row: one narrow char at column 3,
        // nothing at all for column 4.
        terminal.feed(text: "\u{1b}[1;3H\u{1b}[0mx")

        let orphan = terminal.getCharData(col: 3, row: 0)
        let before = terminal.getCharData(col: 2, row: 0)
        #expect(before?.width == 1, "column 3 is now a narrow char")
        // The renderer skips a continuation only by stepping col += 2 past a
        // real wide char. With a narrow char in front, this cell is visited
        // with max(1, width) == 1 and drawn as a space using its own
        // attribute — and `.defaultInvertedColor` maps to
        // nativeBackgroundColor.inverseColor(), i.e. a white block.
        #expect(orphan?.width == 1,
                "column 4 kept a width-0 continuation with no wide char in front of it: the renderer paints it as a space with bg=.defaultInvertedColor — the white block")
        #expect(orphan?.attribute.bg == .defaultColor, Comment(rawValue:
                "column 4's background is \(Self.name(orphan?.attribute.bg ?? .defaultColor)), not the default — this is the cell that renders white"))
    }

    /// The same divergence at full scale, from the real capture: replay every
    /// host byte of the initial sync plus three pane switches, then diff the
    /// resulting screen against mosh-server's OWN framebuffer.
    ///
    /// The reference is a second, FRESH mosh-server attached to the identical
    /// (static) tmux session, so its first sync paints the whole screen from
    /// a genuinely blank state. It is the only attribute-accurate reference
    /// available:
    ///   * `capture-pane` trims trailing spaces regardless of attributes, so
    ///     it cannot see a block parked at a line end;
    ///   * ``MoshTransport/requestFullRedraw()`` does NOT make a stock
    ///     mosh-server repaint — transportsender.cc culls sent states older
    ///     than the last ack and IGNORES an ack naming a culled state. The
    ///     capture's `redraw.bin` came back empty, so the diagnostics
    ///     popover's repaint button is a no-op against a real server.
    ///
    /// Every mismatch is a cell mosh believes is already correct and will
    /// never resend.
    @Test("incremental screen matches mosh-server's own framebuffer")
    func incrementalScreenMatchesMoshFramebuffer() throws {
        let meta = try Self.meta()
        let sink = Sink(), refSink = Sink()
        let incremental = Self.terminal(sink, cols: meta.cols, rows: meta.rows)
        for phase in meta.phases { incremental.feed(byteArray: try Self.bytes(phase.bytes)) }

        let reference = Self.terminal(refSink, cols: meta.cols, rows: meta.rows)
        reference.feed(byteArray: try Self.bytes(meta.reference))

        let got = Self.grid(incremental, cols: meta.cols, rows: meta.rows)
        let want = Self.grid(reference, cols: meta.cols, rows: meta.rows)

        var mismatches: [String] = []
        for row in 0..<meta.rows {
            for col in 0..<meta.cols where got[row][col] != want[row][col] {
                mismatches.append("r\(row + 1)c\(col + 1): mosh-framebuffer=\(want[row][col].render) "
                    + "incremental=\(got[row][col].render)")
            }
        }
        #expect(mismatches.isEmpty, Comment(rawValue:
            "\(mismatches.count) cells will never be repainted:\n" + mismatches.joined(separator: "\n")))
    }
}
