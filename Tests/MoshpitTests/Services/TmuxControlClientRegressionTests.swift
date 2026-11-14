import Foundation
import Testing
@testable import Moshpit

// MARK: - Recorder

/// Records every callback at full fidelity — pane bytes, every response
/// line, the text of each protocol error — because the captures below pin
/// all of it. Own copy, like the other parser suites, so suites never share
/// state.
private final class CaptureRecorder: @unchecked Sendable {
    private let lock = NSLock()

    enum Event: Equatable {
        case output(paneId: String, data: Data)
        case layoutChange(windowId: String, layout: String, zoomed: Bool)
        case windowAdd(String)
        case windowClose(String)
        case windowRenamed(String, String)
        case sessionChanged(String, String)
        case sessionWindowChanged(String, String)
        case activePaneChanged(String, String)
        case pause(String)
        case `continue`(String)
        case clientDetached(String)
        case exit(String?)
        case commandResponse(commandId: Int, commandNum: Int, isError: Bool, lines: [String])
        case protocolError(TmuxControlError)
    }

    private var _events: [Event] = []

    var events: [Event] {
        lock.lock(); defer { lock.unlock() }
        return _events
    }

    func append(_ event: Event) {
        lock.lock(); defer { lock.unlock() }
        _events.append(event)
    }

    func install(on client: TmuxControlClient) async {
        await client.setCallbacks(
            onPaneOutput: { [self] id, data in append(.output(paneId: id, data: data)) },
            onLayoutChange: { [self] id, layout, zoomed in
                append(.layoutChange(windowId: id, layout: layout, zoomed: zoomed))
            },
            onWindowAdd: { [self] id in append(.windowAdd(id)) },
            onWindowClose: { [self] id in append(.windowClose(id)) },
            onWindowRenamed: { [self] id, name in append(.windowRenamed(id, name)) },
            onSessionChanged: { [self] id, name in append(.sessionChanged(id, name)) },
            onSessionWindowChanged: { [self] s, w in append(.sessionWindowChanged(s, w)) },
            onActivePaneChanged: { [self] w, p in append(.activePaneChanged(w, p)) },
            onPause: { [self] id in append(.pause(id)) },
            onContinue: { [self] id in append(.continue(id)) },
            onClientDetached: { [self] payload in append(.clientDetached(payload)) },
            onExit: { [self] reason in append(.exit(reason)) },
            onCommandResponse: { [self] response in
                append(.commandResponse(commandId: response.commandId,
                                        commandNum: response.commandNum,
                                        isError: response.isError,
                                        lines: response.lines))
            },
            onProtocolError: { [self] error in append(.protocolError(error)) }
        )
    }
}

// MARK: - Captures

/// The minimal captures in `crates/tmux/tests/fixtures/divergences/`, byte
/// for byte (`crates/tmux/harness/synth.py` writes them): one per bug the
/// Rust port of this parser found when both were diffed on the same streams.
/// Embedded, not read from the repo, because this suite runs on devices.
/// Each test expects that capture's `.rust.jsonl` — now this parser's
/// output too.
private enum Capture {
    /// tmux's `%begin` time field in every capture.
    static let time = 1_700_000_000

    /// The boot block a capture opens with: the attach line's own reply,
    /// swallowed, and the last real block the recovery window counts from.
    static func boot(_ num: Int) -> String {
        "%begin \(time) \(num) 0\r\n%end \(time) \(num) 0\r\n"
    }

    /// CRLF-terminated lines, the way tmux writes them on a pty.
    static func lines(_ lines: String...) -> String {
        lines.map { $0 + "\r\n" }.joined()
    }

    /// `overflow-trap.bin`: pane output that ends in a `%begin` whose id is
    /// `Int.min` — the recovery window's distance overflowed and trapped.
    static let overflowTrap = Data((boot(100) + lines(
        "%output %0 x%begin -9223372036854775808 101 1",
        "%output %0 after"
    )).utf8)

    /// `block-cap-terminator.bin`: a reply of exactly 5000 (empty) lines —
    /// the parser's runaway-block cap — closed by its own `%end`.
    static let blockCapTerminator = Data((boot(100)
        + "%begin \(time) 101 1\r\n"
        + String(repeating: "\r\n", count: 5000)
        + lines("%end \(time) 101 1", "%window-add @1")).utf8)

    /// `terminator-number-boundary.bin`: inside block 10, a content line
    /// quoting the terminator of block 105 of the same second.
    static let terminatorNumberBoundary = Data((boot(9) + lines(
        "%begin \(time) 10 1",
        "seen in a log: %end \(time) 105 1",
        "bar",
        "%end \(time) 10 1",
        "%window-add @1"
    )).utf8)

    /// `unicode-delimiter.bin`: a combining acute accent (U+0301) right
    /// after a protocol space or `%`.
    static let unicodeDelimiter = Data((boot(100) + lines(
        "%window-renamed @1 \u{301}x",
        "%session-changed $1 \u{301}s",
        "junk%\u{301}window-add @1 %window-add @2"
    )).utf8)

    /// `unicode-flag.bin`: pane output ending in a %begin whose flag is the
    /// CJK numeral 五, then the real block with that number.
    static let unicodeFlag = Data((boot(100) + lines(
        "%output %0 x%begin \(time) 101 \u{4E94}",
        "%begin \(time) 101 1",
        "real reply",
        "%end \(time) 101 1"
    )).utf8)

    /// `unicode-tail.bin`: inside block 10, a content line ending in the
    /// block's own terminator with an Arabic-Indic three (U+0663) as flag.
    static let unicodeTail = Data((boot(9) + lines(
        "%begin \(time) 10 1",
        "foo%end \(time) 10 \u{663}",
        "bar",
        "%end \(time) 10 1"
    )).utf8)

    /// The window layout `zoom-flags.bin` repeats.
    static let zoomLayout = "9bb2,80x24,0,0{26x24,0,0,0,26x24,27,0,1,26x24,54,0,2}"

    /// `zoom-flags.bin`: one three-pane window's layout change under seven
    /// window-flags fields tmux 3.6a sends; the first five are zoomed.
    static let zoomFlags = Data((boot(100) + ["-Z", "*MZ", "Z", "#*Z", "*Z", "*", "-"].map {
        "%layout-change @1 \(zoomLayout) b25d,80x24,0,0,0 \($0)\r\n"
    }.joined()).utf8)
}

/// Feed `stream` to a fresh parser in `chunk`-byte feeds and return every
/// event it reported.
private func events(_ stream: Data, chunk: Int = .max) async -> [CaptureRecorder.Event] {
    let client = TmuxControlClient()
    let recorder = CaptureRecorder()
    await recorder.install(on: client)
    var start = stream.startIndex
    while start < stream.endIndex {
        let end = stream.index(start, offsetBy: min(chunk, stream.endIndex - start))
        await client.feed(stream[start..<end])
        start = end
    }
    return recorder.events
}

private func bytes(_ string: String) -> Data { Data(string.utf8) }

// MARK: - Regression tests

@Suite("TmuxControlClient parser — regression captures", .tmuxParserUnderTest)
struct TmuxControlClientRegressionTests {

    @Test("pane output ending in a %begin with id Int.min stays pane output instead of crashing")
    func overflowTrapCapture() async {
        // Pane content can print anything, and a `%begin` at the end of an
        // %output payload is checked as a possibly glued protocol line. Its
        // id was subtracted from the last block's in plain Int arithmetic:
        // `abs(Int.min - 1700000000)` trapped, and the app died (SIGTRAP).
        let expected: [CaptureRecorder.Event] = [
            .output(paneId: "%0", data: bytes("x%begin -9223372036854775808 101 1")),
            .output(paneId: "%0", data: bytes("after")),
        ]
        #expect(await events(Capture.overflowTrap) == expected)
        #expect(await events(Capture.overflowTrap, chunk: 1) == expected)
    }

    @Test("ids at the Int limits in a glued %begin never trap",
          arguments: [
              // No real block yet, so the window counts from (0, 0):
              // `abs(Int.min - 0)` itself overflows.
              ("", "x%begin -9223372036854775808 1 1"),
              // The last block's number is Int.min: `Int.max - Int.min`.
              ("%begin 1700000000 -9223372036854775808 0\r\n%end 1700000000 -9223372036854775808 0\r\n",
               "x%begin 1700000000 9223372036854775807 1"),
              // Far from the last block's id, but no overflow: stays output.
              (Capture.boot(100), "x%begin 9223372036854775807 101 1"),
          ])
    func extremeGluedIds(prefix: String, payload: String) async {
        let stream = bytes(prefix + "%output %0 \(payload)\r\n")
        #expect(await events(stream) == [.output(paneId: "%0", data: bytes(payload))])
    }

    @Test("a reply of exactly 5000 lines closes on its own %end, not as a runaway block")
    func blockCapTerminatorCapture() async {
        // The runaway cap ran before the terminator check, so the 5000-line
        // reply's real %end tripped it: the reply came back as an ERROR and
        // the %end was then reported as a terminator without a %begin.
        let expected: [CaptureRecorder.Event] = [
            .commandResponse(commandId: Capture.time, commandNum: 101, isError: false,
                             lines: Array(repeating: "", count: 5000)),
            .windowAdd("@1"),
        ]
        #expect(await events(Capture.blockCapTerminator) == expected)
        #expect(await events(Capture.blockCapTerminator, chunk: 1) == expected)
    }

    @Test("a block one line past the cap is still force-closed, and the line re-dispatched")
    func blockCapStillCatchesARunawayBlock() async {
        // The other side of the same boundary: line 5001 that is NOT the
        // block's terminator means the terminator was lost.
        let stream = bytes(Capture.boot(100)
            + "%begin \(Capture.time) 101 1\r\n"
            + String(repeating: "\r\n", count: 5000)
            + Capture.lines("%window-add @9"))
        let expected: [CaptureRecorder.Event] = [
            .protocolError(.malformedLine("block #\(Capture.time) exceeded 5000 lines — forcing close")),
            .commandResponse(commandId: Capture.time, commandNum: 101, isError: true,
                             lines: Array(repeating: "", count: 5000)),
            .windowAdd("@9"),
        ]
        #expect(await events(stream) == expected)
    }

    @Test("a content line quoting %end of a longer command number stays in the block")
    func terminatorNumberBoundaryCapture() async {
        // The glued-terminator recovery looked for "%end <cid> 10" at the
        // end of the line and found it inside "%end <cid> 105 1": the reply
        // was cut at that line, "bar" spilled out as a stray line, and the
        // real %end became a protocol error.
        let expected: [CaptureRecorder.Event] = [
            .commandResponse(commandId: Capture.time, commandNum: 10, isError: false,
                             lines: ["seen in a log: %end \(Capture.time) 105 1", "bar"]),
            .windowAdd("@1"),
        ]
        #expect(await events(Capture.terminatorNumberBoundary) == expected)
        #expect(await events(Capture.terminatorNumberBoundary, chunk: 1) == expected)
    }

    @Test("the number boundary holds for %error and a bare number too, and a real glued terminator still closes")
    func terminatorNumberBoundaryVariants() async {
        let stream = bytes(Capture.boot(9) + Capture.lines(
            "%begin \(Capture.time) 10 1",
            "log: %error \(Capture.time) 107",
            "log: %end \(Capture.time) 1000 1",
            "cut short%end \(Capture.time) 10 1"
        ))
        #expect(await events(stream) == [
            .commandResponse(commandId: Capture.time, commandNum: 10, isError: false,
                             lines: ["log: %error \(Capture.time) 107",
                                     "log: %end \(Capture.time) 1000 1",
                                     "cut short"]),
        ])
    }

    @Test("a combining mark after a protocol space or % does not hide it")
    func unicodeDelimiterCapture() async {
        // Matched by Character, " \u{301}" is ONE Character, not a space:
        // the rename was reported as malformed, the session id swallowed
        // its name ("$1 \u{301}s"), and the junk strip skipped the `%`
        // before the combining mark to emit a window-add from mid-line.
        let expected: [CaptureRecorder.Event] = [
            .windowRenamed("@1", "\u{301}x"),
            .sessionChanged("$1", "\u{301}s"),
        ]
        #expect(await events(Capture.unicodeDelimiter) == expected)
        #expect(await events(Capture.unicodeDelimiter, chunk: 1) == expected)
    }

    @Test("a non-ASCII numeral is no flag digit: no phantom block from pane output")
    func unicodeFlagCapture() async {
        // `Character.isNumber` took 五 for the flag of a glued %begin, so
        // the recovery opened block 101 out of pane text, and the REAL
        // block 101 became content of the phantom one.
        let expected: [CaptureRecorder.Event] = [
            .output(paneId: "%0", data: bytes("x%begin \(Capture.time) 101 \u{4E94}")),
            .commandResponse(commandId: Capture.time, commandNum: 101, isError: false,
                             lines: ["real reply"]),
        ]
        #expect(await events(Capture.unicodeFlag) == expected)
        #expect(await events(Capture.unicodeFlag, chunk: 1) == expected)
    }

    @Test("a non-ASCII digit after a glued terminator keeps the line in the block")
    func unicodeTailCapture() async {
        // The glued-terminator tail accepted any Character with a numeric
        // value: "foo%end <cid> 10 ٣" closed block 10, "bar" spilled out
        // and the real %end became a protocol error.
        let expected: [CaptureRecorder.Event] = [
            .commandResponse(commandId: Capture.time, commandNum: 10, isError: false,
                             lines: ["foo%end \(Capture.time) 10 \u{663}", "bar"]),
        ]
        #expect(await events(Capture.unicodeTail) == expected)
        #expect(await events(Capture.unicodeTail, chunk: 1) == expected)
    }

    @Test("%begin, %end and %layout-change split their fields at every space byte")
    func combiningMarkAfterSpaceInFields() async {
        // The other Character splits: a %begin whose flags field starts
        // with a combining mark was malformed (no block opened, its reply
        // lost), and the visible layout of a %layout-change merged into
        // the layout string.
        let stream = bytes(Capture.boot(100) + Capture.lines(
            "%begin \(Capture.time) 101 \u{301}",
            "reply",
            "%end \(Capture.time) 101 \u{301}",
            "%layout-change @1 b25d,80x24,0,0,0 \u{301}b25d,80x24,0,0,0 *Z"
        ))
        #expect(await events(stream) == [
            .commandResponse(commandId: Capture.time, commandNum: 101, isError: false,
                             lines: ["reply"]),
            .layoutChange(windowId: "@1", layout: "b25d,80x24,0,0,0", zoomed: true),
        ])
    }

    @Test("a zoomed window reads as zoomed whatever other window flags it carries")
    func zoomFlagsCapture() async {
        // tmux 3.6a writes `*Z` only for the current window with no other
        // flag; a zoomed window that is not current arrives as `-Z` or
        // `Z`, with a marked pane as `*MZ`, with activity as `#*Z`. Only
        // the exact `*Z` used to count.
        let expected = [true, true, true, true, true, false, false].map {
            CaptureRecorder.Event.layoutChange(windowId: "@1", layout: Capture.zoomLayout, zoomed: $0)
        }
        #expect(await events(Capture.zoomFlags) == expected)
        #expect(await events(Capture.zoomFlags, chunk: 1) == expected)
    }

    @Test("zoomed comes from a flags field holding Z and nothing but window flags",
          arguments: [
              ("*Z", true), ("-Z", true), ("Z", true), ("*MZ", true), ("#*Z", true), ("!-Z", true),
              ("~Z", true), ("*", false), ("-", false), ("", false), ("*Zx", false), ("*z", false),
          ])
    func zoomFlagsField(flags: String, zoomed: Bool) async {
        let layout = "b25d,80x24,0,0,0"
        // With the visible layout (tmux 3.x) and without it (older tmux).
        let stream = bytes(Capture.boot(100) + Capture.lines(
            "%layout-change @1 \(layout) \(layout) \(flags)",
            "%layout-change @1 \(layout) \(flags)"
        ))
        let event = CaptureRecorder.Event.layoutChange(windowId: "@1", layout: layout, zoomed: zoomed)
        #expect(await events(stream) == [event, event])
    }

    // MARK: - Line extraction

    @Test("one big feed parses as fast as the same bytes in 4 KB feeds")
    func bigFeedIsLinear() async {
        // A `seq`-like burst, about 2 MB of short %output lines in ONE feed
        // — what a reconnect's backlog or a fast producer hands the parser.
        // Line extraction re-copied the rest of its buffer after every
        // line, so one feed cost O(lines × bytes): in a debug build on an
        // M2 this feed took 2.6 s whole and 0.17 s in 4 KB pieces, a gap
        // that grows with the feed. Linear extraction takes the same time
        // either way.
        var text = Capture.boot(100)
        for n in 0..<80_000 { text += "%output %0 \(n)\\015\\012\r\n" }
        let stream = bytes(text)
        func fastest(chunk: Int) async -> (Duration, [CaptureRecorder.Event]) {
            var best = Duration.seconds(1_000)
            var seen: [CaptureRecorder.Event] = []
            for _ in 0..<3 {
                let start = ContinuousClock.now
                seen = await events(stream, chunk: chunk)
                best = min(best, ContinuousClock.now - start)
            }
            return (best, seen)
        }
        let (pieces, piecesEvents) = await fastest(chunk: 4096)
        let (whole, wholeEvents) = await fastest(chunk: .max)
        #expect(wholeEvents.count == 80_000)
        #expect(wholeEvents == piecesEvents)
        #expect(whole < pieces * 4, "one \(stream.count)-byte feed took \(whole), 4 KB feeds \(pieces)")
    }

    @Test("a reply block's long lines parse no slower than the same lines as pane output")
    func blockLinesAreNotSearchedWhole() async {
        // B102: every line of an open %begin block (a backfill's
        // capture-pane reply is thousands of them) was searched whole,
        // twice, for the block's own terminator glued to its end — with
        // needles formatted again for every line. Only the end of a line
        // can hold one. The same 16 lines of 16 KB as %output lines (which
        // are octal-decoded byte by byte) are the yardstick: with the old
        // search the block took 2.3 times as long as the output in a debug
        // build on an M2 (and parsed at 12 MB/s in a release one), with
        // the search confined to the line's end it takes a fifth.
        let line = String(repeating: "x", count: 16 * 1024 - 2)
        var reply = Capture.boot(100) + "%begin \(Capture.time) 101 1\r\n"
        var output = Capture.boot(100)
        for _ in 0..<16 {
            reply += line + "\r\n"
            output += "%output %0 " + line + "\r\n"
        }
        reply += "%end \(Capture.time) 101 1\r\n"
        func fastest(_ stream: Data) async -> (Duration, [CaptureRecorder.Event]) {
            var best = Duration.seconds(1_000)
            var seen: [CaptureRecorder.Event] = []
            for _ in 0..<3 {
                let start = ContinuousClock.now
                seen = await events(stream)
                best = min(best, ContinuousClock.now - start)
            }
            return (best, seen)
        }
        let (replyTime, replyEvents) = await fastest(bytes(reply))
        let (outputTime, outputEvents) = await fastest(bytes(output))
        #expect(replyEvents == [.commandResponse(commandId: Capture.time, commandNum: 101,
                                                 isError: false,
                                                 lines: Array(repeating: line, count: 16))])
        #expect(outputEvents.count == 16)
        // The Rust parser (this suite runs on both) has its own count of the
        // work per block line (crates/tmux/tests/block_cost.rs); behind the
        // FFI its two paths cost about the same, which is all asked of it.
        let swift = await TmuxControlClient().backend == .swift
        #expect(replyTime < outputTime * (swift ? 1 : 3),
                "the reply's 16 lines of 16 KB took \(replyTime), the same lines as output \(outputTime)")
    }

    @Test("where the feeds cut the stream never changes the events")
    func chunkingNeverChangesTheEvents() async {
        // Every line shape — CRLF, LF, a stray CR, CJK and a UTF-8 sequence
        // split across two %output events, blocks with look-alike and glued
        // terminators, a glued %begin, junk before a keyword, a session
        // ending and a new one booting — in a seeded random order, fed
        // whole and in pieces from 1 byte up. The pieces are slices of the
        // stream, so a feed's indices do not start at zero either.
        let shapes = [
            "%output %1 plain \\033[1mbold\\033[0m \\134 测试\r\n",
            "%output %2 \u{2500}\r\n%output %2 x\u{1F600}\r\n",
            "%begin \(Capture.time) 101 1\r\n你好 ┌─┐\r\n%end 9 9 9\r\ncut%end \(Capture.time) 101 1\r\n",
            "%begin \(Capture.time) 102 1\r\nfoo%end \(Capture.time) 1023 1\r\n%error \(Capture.time) 102 1\r\n",
            "%layout-change @1 b25d,80x24,0,0,0 b25d,80x24,0,0,0 -Z\r\n",
            "%window-renamed @2 汉字 窗口\r\n",
            "%session-changed $1 main\n",
            "\u{1B}P1000p%window-add @3\r\n",
            "%exit\r\n%begin 1 1 0\r\n%end 1 1 0\r\n",
            "%output %0 x%begin \(Capture.time) 103 1\r\nglued\r\n%end \(Capture.time) 103 1\r\n",
            "garbage line\r\n\r\n%pause %1\r\n%continue %1\r\n",
            "%end 5 5 5\r\n%client-detached /dev/ttys1\r\n%window-add @4\r\r\n",
        ]
        var state: UInt64 = 7
        var text = Capture.boot(100)
        for _ in 0..<300 {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            text += shapes[Int(state >> 33) % shapes.count]
        }
        // A split UTF-8 sequence, byte-exact: `─` is e2 94 80.
        var stream = bytes(text)
        stream.append(contentsOf: bytes("%output %5 ") + [0xE2, 0x94] + bytes("\r\n%output %5 ")
                      + [0x80] + bytes("\r\n%output %5 unterminated"))
        let whole = await events(stream)
        #expect(whole.count > 300)
        for chunk in [1, 2, 3, 7, 64, 1500] {
            #expect(await events(stream, chunk: chunk) == whole, "feeds of \(chunk) bytes")
        }
    }
}
