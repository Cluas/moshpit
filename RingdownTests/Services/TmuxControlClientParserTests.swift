import Foundation
import Testing
@testable import Ringdown

// MARK: - CallbackRecorder

/// Test harness that captures every callback the parser fires so each `@Test`
/// can assert against a deterministic event log. The recorder isolates mutable
/// state behind its own lock so the parser actor can post callbacks from any
/// task without test-side data races.
private final class CallbackRecorder: @unchecked Sendable {
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

    /// Install every callback on the parser so any event lands in `events`.
    func install(on client: TmuxControlClient) async {
        await client.setCallbacks(
            onPaneOutput: { [self] id, data in
                append(.output(paneId: id, data: data))
            },
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
                append(.commandResponse(
                    commandId: response.commandId,
                    commandNum: response.commandNum,
                    isError: response.isError,
                    lines: response.lines
                ))
            },
            onProtocolError: { [self] error in append(.protocolError(error)) }
        )
    }
}

// MARK: - Helpers

private func bytes(_ string: String) -> Data {
    Data(string.utf8)
}

// MARK: - TmuxControlClientParserTests

@Suite("TmuxControlClient parser")
struct TmuxControlClientParserTests {

    // MARK: - Output

    @Test("%output without escapes routes raw bytes to onPaneOutput")
    func plainOutput() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%output %0 hello world\n"))

        #expect(rec.events == [.output(paneId: "%0", data: bytes("hello world"))])
    }

    @Test("%output decodes \\NNN octal escapes")
    func octalEscapeDecoding() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        // `\033` = ESC (0x1B), `\007` = BEL (0x07).
        await client.feed(bytes(#"%output %1 \033[31mred\033[0m\007"# + "\n"))

        let expected: Data = {
            var d = Data()
            d.append(0x1B); d.append(contentsOf: bytes("[31mred"))
            d.append(0x1B); d.append(contentsOf: bytes("[0m"))
            d.append(0x07)
            return d
        }()
        #expect(rec.events == [.output(paneId: "%1", data: expected)])
    }

    @Test("%output handles \\\\ as a literal backslash")
    func escapedBackslash() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes(#"%output %2 path\\to\\file"# + "\n"))

        #expect(rec.events == [.output(paneId: "%2", data: bytes(#"path\to\file"#))])
    }

    @Test("a UTF-8 char split across two %output events survives byte-exact")
    func splitUTF8AcrossOutputEvents() async throws {
        // tmux flushes pane output mid-character: a 3-byte `─` (e2 94 80)
        // can end one %output event after `e2 94` and start the next with
        // `80`. Each line alone is invalid UTF-8 — any String round-trip
        // would bake U+FFFD (ef bf bd) into the stream, the "？？" garble
        // smeared over Claude Code's borders. The payloads must come out
        // byte-exact so the terminal's own stateful decoder can reassemble.
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        var first = Data("%output %5 ".utf8)
        first.append(contentsOf: [0xE2, 0x94, 0x80, 0xE2, 0x94])  // ─ + partial ─
        first.append(UInt8(ascii: "\n"))
        var second = Data("%output %5 ".utf8)
        second.append(contentsOf: [0x80, 0xE2, 0x94, 0x80])       // completes it + one more ─
        second.append(UInt8(ascii: "\n"))

        await client.feed(first)
        await client.feed(second)

        #expect(rec.events == [
            .output(paneId: "%5", data: Data([0xE2, 0x94, 0x80, 0xE2, 0x94])),
            .output(paneId: "%5", data: Data([0x80, 0xE2, 0x94, 0x80])),
        ])
        // The joined stream must be three intact box-drawing chars: one
        // complete in the first event, one split across the boundary, one
        // complete in the second event.
        if case let .output(_, d1) = rec.events[0], case let .output(_, d2) = rec.events[1] {
            #expect(String(data: d1 + d2, encoding: .utf8) == "───")
        }
    }

    @Test("raw CJK bytes in %output pass through without String mangling")
    func rawCJKOutput() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        var line = Data("%output %6 ".utf8)
        line.append(contentsOf: Data("测试中文".utf8))
        line.append(UInt8(ascii: "\n"))
        await client.feed(line)

        #expect(rec.events == [.output(paneId: "%6", data: Data("测试中文".utf8))])
    }

    // MARK: - Block framing

    @Test("%begin / response lines / %end collapse to a single onCommandResponse")
    func happyPathCommandResponse() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        let payload = """
        %begin 1234 7 0
        @0 main 0 81x24,0,0,0 1 1
        @1 logs 1 81x24,0,0,1 0 1
        %end 1234 7 0

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .commandResponse(
                commandId: 1234,
                commandNum: 7,
                isError: false,
                lines: [
                    "@0 main 0 81x24,0,0,0 1 1",
                    "@1 logs 1 81x24,0,0,1 0 1"
                ]
            )
        ])
    }

    @Test("block content lines containing a %keyword substring are kept verbatim")
    func blockLinesWithPercentKeywordNotTruncated() async throws {
        // capture-pane scrollback can contain text that merely *includes* a
        // tmux %keyword (e.g. "80%done", "x%window"). These must round-trip
        // untouched — decodeLine's leading-junk strip would otherwise truncate
        // them to the %keyword suffix and garble the history.
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        let payload = """
        %begin 5 2 0
        build is 80%done now
        cpu x%window load
        plain ascii line
        100%end of run
        %end 5 2 0

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .commandResponse(
                commandId: 5,
                commandNum: 2,
                isError: false,
                lines: [
                    "build is 80%done now",
                    "cpu x%window load",
                    "plain ascii line",
                    "100%end of run"
                ]
            )
        ])
    }

    @Test("CJK + box-drawing content survives a block round-trip byte-for-byte")
    func blockLinesPreserveWideChars() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        let payload = """
        %begin 6 3 0
        你好世界 ┌─┐ │x│ └─┘
        %end 6 3 0

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .commandResponse(
                commandId: 6,
                commandNum: 3,
                isError: false,
                lines: ["你好世界 ┌─┐ │x│ └─┘"]
            )
        ])
    }

    @Test("%error closes the block with isError=true")
    func errorBlock() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        let payload = """
        %begin 99 1 0
        no server running
        %error 99 1 0

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .commandResponse(
                commandId: 99,
                commandNum: 1,
                isError: true,
                lines: ["no server running"]
            )
        ])
    }

    @Test("response lines that themselves start with % are captured, not parsed as notifications")
    func responseLinesStartingWithPercent() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        // list-panes returns lines like `%160 @84 …`.
        let payload = """
        %begin 1 1 0
        %160 @84 bash 80 24 1
        %161 @84 vim 80 24 0
        %end 1 1 0

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .commandResponse(
                commandId: 1,
                commandNum: 1,
                isError: false,
                lines: [
                    "%160 @84 bash 80 24 1",
                    "%161 @84 vim 80 24 0"
                ]
            )
        ])
    }

    // MARK: - Notifications

    @Test("Window + session + layout notifications fire in order")
    func notificationStream() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        let payload = """
        %session-changed $0 main
        %window-add @5
        %window-renamed @5 logs
        %layout-change @5 81x24,0,0,8 *Z
        %window-pane-changed @5 %12
        %session-window-changed $0 @5
        %window-close @5

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .sessionChanged("$0", "main"),
            .windowAdd("@5"),
            .windowRenamed("@5", "logs"),
            .layoutChange(windowId: "@5", layout: "81x24,0,0,8", zoomed: true),
            .activePaneChanged("@5", "%12"),
            .sessionWindowChanged("$0", "@5"),
            .windowClose("@5")
        ])
    }

    @Test("%pause and %continue route to onPause / onContinue")
    func pauseContinue() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%pause %4\n%continue %4\n"))

        #expect(rec.events == [.pause("%4"), .continue("%4")])
    }

    @Test("%exit with and without reason")
    func exitDirective() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%exit\n%exit detached\n"))

        #expect(rec.events == [.exit(nil), .exit("detached")])
    }

    // MARK: - Forward-compat + errors

    @Test("Unknown %foo directives are silently ignored")
    func unknownDirectiveIgnored() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%this-is-from-the-future @0 some payload\n"))

        // Empty: no callbacks at all.
        #expect(rec.events.isEmpty)
    }

    @Test("Malformed %output (no pane id) raises onProtocolError")
    func malformedOutput() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%output without-pane-id\n"))

        // The handler reports the malformed line. The exact body text isn't
        // contract — what matters is that *some* protocol error fires.
        switch rec.events.first {
        case .protocolError(.malformedLine(_)):
            break
        default:
            Issue.record("Expected .protocolError(.malformedLine), got \(String(describing: rec.events.first))")
        }
    }

    // MARK: - Buffer across feeds

    @Test("Bytes split across feed() calls still produce one whole-line event")
    func chunkedAcrossFeeds() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        // Split in the middle of the payload — first chunk has no newline.
        await client.feed(bytes("%output %0 hel"))
        #expect(rec.events.isEmpty, "no newline yet, callback must wait")

        await client.feed(bytes("lo world\n"))
        #expect(rec.events == [.output(paneId: "%0", data: bytes("hello world"))])
    }

    @Test("CRLF line endings are stripped just like LF")
    func crlfHandling() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%window-add @9\r\n"))

        #expect(rec.events == [.windowAdd("@9")])
    }

    // MARK: - Octal decoder primitive

    @Test("decodeOctalEscapes passes plain bytes through")
    func decodeOctalIdentity() {
        let raw = bytes("hello")
        #expect(TmuxControlClient.decodeOctalEscapes(raw) == raw)
    }

    @Test("decodeOctalEscapes converts every valid \\NNN sequence")
    func decodeOctalAllValues() {
        // Build inputs for a representative selection of byte values.
        let cases: [(escape: String, value: UInt8)] = [
            ("\\000", 0x00),
            ("\\007", 0x07),
            ("\\033", 0x1B),
            ("\\177", 0x7F),
            ("\\377", 0xFF)
        ]
        for (escape, value) in cases {
            let decoded = TmuxControlClient.decodeOctalEscapes(bytes(escape))
            #expect(decoded == Data([value]), "\(escape) should decode to 0x\(String(value, radix: 16))")
        }
    }

    @Test("reset() drops a buffered partial line so subsequent feed reparses")
    func resetClearsBuffer() async throws {
        let client = TmuxControlClient()
        let rec = CallbackRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%output %0 nev"))
        await client.reset()
        await client.feed(bytes("er-mind\n%window-add @1\n"))

        // The dangling "nev" + "er-mind\n" no longer form a recognised line
        // because "er-mind" doesn't start with %; it should be silently
        // ignored. The subsequent %window-add must still fire.
        #expect(rec.events == [.windowAdd("@1")])
    }
}
