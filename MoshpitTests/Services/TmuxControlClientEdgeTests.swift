import Foundation
import Testing
@testable import Moshpit

// MARK: - Recorder

/// Identical recorder shape to `TmuxControlClientParserTests` so each suite
/// owns its own copy and they can run interleaved without sharing state.
private final class EdgeRecorder: @unchecked Sendable {
    private let lock = NSLock()
    enum Event: Equatable {
        case output(paneId: String, data: Data)
        case windowAdd(String)
        case windowClose(String)
        case windowRenamed(String, String)
        case layoutChange(windowId: String, layout: String, zoomed: Bool)
        case sessionChanged(String, String)
        case sessionWindowChanged(String, String)
        case activePaneChanged(String, String)
        case pause(String)
        case `continue`(String)
        case clientDetached(String)
        case exit(String?)
        case commandResponse(commandId: Int, commandNum: Int, isError: Bool, lineCount: Int)
        case protocolError
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
            onLayoutChange: { [self] id, layout, zoomed in append(.layoutChange(windowId: id, layout: layout, zoomed: zoomed)) },
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
            onCommandResponse: { [self] r in
                append(.commandResponse(commandId: r.commandId, commandNum: r.commandNum, isError: r.isError, lineCount: r.lines.count))
            },
            onProtocolError: { [self] _ in append(.protocolError) }
        )
    }
}

private func bytes(_ s: String) -> Data { Data(s.utf8) }

// MARK: - Edge tests

@Suite("TmuxControlClient parser — edge cases")
struct TmuxControlClientEdgeTests {

    // MARK: - Octal escapes inside %output

    @Test("Octal-escaped newline (\\012) inside %output stays inside the payload, not a line break")
    func octalNewlineDoesNotSplitLines() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        // tmux escapes any byte < 0x20 or > 0x7E. Embedded newline is `\012`.
        await client.feed(bytes(#"%output %0 first\012second"# + "\n"))

        #expect(rec.events == [.output(paneId: "%0", data: bytes("first\nsecond"))])
    }

    @Test("Octal-escaped CR + LF in payload do not get treated as transport CRLF")
    func octalCRLFDoesNotSplitLines() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        await client.feed(bytes(#"%output %0 alpha\015\012beta"# + "\n"))

        #expect(rec.events == [.output(paneId: "%0", data: bytes("alpha\r\nbeta"))])
    }

    // MARK: - Aggressive fragmentation

    @Test("Feeding one byte at a time still produces exactly one notification")
    func feedSingleByteChunks() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        let full = bytes("%window-add @42\n")
        for byte in full {
            await client.feed(Data([byte]))
        }

        #expect(rec.events == [.windowAdd("@42")])
    }

    @Test("Multiple complete lines in a single feed all fire in source order")
    func manyLinesInOneFeed() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        let payload = """
        %window-add @1
        %window-add @2
        %window-add @3
        %window-close @2

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .windowAdd("@1"),
            .windowAdd("@2"),
            .windowAdd("@3"),
            .windowClose("@2"),
        ])
    }

    // MARK: - Empty / minimal payloads

    @Test("%output with empty payload still fires onPaneOutput with empty Data")
    func emptyOutputPayload() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%output %0 \n"))

        #expect(rec.events == [.output(paneId: "%0", data: Data())])
    }

    @Test("%session-changed without a name string emits an empty-name event")
    func sessionChangedWithoutName() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%session-changed $7\n"))

        #expect(rec.events == [.sessionChanged("$7", "")])
    }

    // MARK: - Block framing pathologies

    @Test("A `%begin`-looking line inside an open block is captured as content, not parsed as a nested block")
    func nestedBeginCapturedAsContent() async throws {
        // Contract: once we're inside `%begin … %end`, only `%end`/`%error`
        // close the block. Any other line — even one starting with `%begin` —
        // is part of the response payload. tmux's protocol forbids nesting,
        // so this also protects us from response text that happens to look
        // like a directive.
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        let payload = """
        %begin 1 1 0
        first content line
        %begin 2 2 0
        second content line
        %end 2 2 0
        %end 1 1 0

        """
        await client.feed(bytes(payload))

        // Only the terminator echoing the OPEN block's ids closes it: tmux
        // guarantees `%end` repeats its `%begin`'s ids, so `%end 2 2 0` here
        // is pane CONTENT (a capture of a pane that displays control-mode
        // text — e.g. developing against tmux -CC), not a terminator. The
        // old contract closed the block on ANY `%end`-shaped line, which
        // truncated such frames and shifted command↔response pairing for
        // every later reply (the 2026-08-19 weak-network garble evidence).
        let responses = rec.events.compactMap { event -> EdgeRecorder.Event? in
            if case .commandResponse = event { return event }
            return nil
        }
        #expect(responses.count == 1, "exactly one block should close")
        #expect(responses.first == .commandResponse(commandId: 1, commandNum: 1, isError: false, lineCount: 4),
                "all four payload lines — including both %-shaped ones — belong to block 1")

        let errors = rec.events.filter { if case .protocolError = $0 { return true } else { return false } }
        #expect(errors.isEmpty, "nothing about this frame is a protocol violation under id-matched termination")
    }

    @Test("%end without a matching %begin surfaces as a protocol error")
    func endWithoutBegin() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%end 0 0 0\n"))

        #expect(rec.events == [.protocolError])
    }

    @Test("Response lines starting with %output / %window-add are captured verbatim, not parsed")
    func responseLineWithNotificationPrefix() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        // list-windows can return rows that incidentally start with %.
        let payload = """
        %begin 1 1 0
        %output not actually a notification
        %window-add looks-like-add
        %end 1 1 0

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [
            .commandResponse(commandId: 1, commandNum: 1, isError: false, lineCount: 2)
        ])
    }

    // MARK: - Layout change permutations

    @Test("%layout-change with the optional visible-layout and *Z flags sets zoomed=true")
    func layoutChangeWithVisibleLayoutAndZoom() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        // Real tmux: %layout-change <window-id> <window-layout> [visible_layout] [<flags>]
        await client.feed(bytes("%layout-change @5 81x24,0,0,8 81x24,0,0,8 *Z\n"))

        #expect(rec.events == [.layoutChange(windowId: "@5", layout: "81x24,0,0,8", zoomed: true)])
    }

    @Test("%layout-change without the zoom flag sets zoomed=false")
    func layoutChangeWithoutZoom() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%layout-change @5 81x24,0,0,8\n"))

        #expect(rec.events == [.layoutChange(windowId: "@5", layout: "81x24,0,0,8", zoomed: false)])
    }

    // MARK: - DCS prologue

    @Test("Leading DCS prologue before the first %begin is skipped, not treated as garbage")
    func dcsPrologueSkipped() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        // Some transports prepend `\eP1000p` before the first tmux notification.
        let dcs = Data([0x1B, 0x50, 0x31, 0x30, 0x30, 0x30, 0x70])  // ESC P 1000 p
        var line = dcs
        line.append(contentsOf: bytes("%window-add @0\n"))
        await client.feed(line)

        #expect(rec.events == [.windowAdd("@0")])
    }

    // MARK: - Large payloads

    @Test("Single %output line carrying ~16 KB of escape-free text routes once")
    func largeOutputPayload() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        let big = String(repeating: "x", count: 16 * 1024)
        await client.feed(bytes("%output %0 \(big)\n"))

        #expect(rec.events == [.output(paneId: "%0", data: bytes(big))])
    }

    // MARK: - %client-detached variants

    @Test("Bare %client-detached and %client-detached <id> both fire onClientDetached")
    func clientDetachedBareAndPayload() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        await client.feed(bytes("%client-detached\n%client-detached client-7\n"))

        #expect(rec.events == [.clientDetached(""), .clientDetached("client-7")])
    }

    // MARK: - Forward-compat noise mixed with real events

    @Test("Unknown directives between real events do not desync the parser")
    func unknownDirectivesDoNotDesync() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        let payload = """
        %window-add @1
        %future-directive @1 some payload
        %not-a-real-thing
        %window-close @1

        """
        await client.feed(bytes(payload))

        #expect(rec.events == [.windowAdd("@1"), .windowClose("@1")])
    }

    // MARK: - reset() during open block

    @Test("reset() abandons an in-flight %begin block without emitting onCommandResponse")
    func resetDuringOpenBlock() async throws {
        let client = TmuxControlClient()
        let rec = EdgeRecorder()
        await rec.install(on: client)

        // Open a block but never close it.
        await client.feed(bytes("%begin 5 5 0\nline-one\n"))
        await client.reset()
        // Subsequent fresh notification should fire normally — no leftover
        // command-response from the aborted block.
        await client.feed(bytes("%window-add @9\n"))

        #expect(rec.events == [.windowAdd("@9")])
    }
}
