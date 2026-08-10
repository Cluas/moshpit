import Foundation
import Testing
@testable import Moshpit

/// The frame channel's wire handling: what we parse out of the stream, and
/// what we put back into it.
@Suite("herdr frame channel")
struct HerdrFrameChannelTests {

    private func frameLine(seq: Int, width: Int = 60, height: Int = 20,
                           full: Bool = false, payload: String) -> String {
        let bytes = Data(payload.utf8).base64EncodedString()
        return #"{"type":"terminal.frame","seq":\#(seq),"encoding":"ansi","width":\#(width),"height":\#(height),"full":\#(full),"bytes":"\#(bytes)"}"#
    }

    // MARK: - Parsing

    @Test("A whole frame line decodes to ready-to-feed bytes")
    func parsesFrame() {
        var parser = HerdrFrameParser()
        let frames = parser.consume(Data((frameLine(seq: 1, full: true, payload: "\u{1b}[2Jhi") + "\n").utf8))
        #expect(frames == [.screen(seq: 1, width: 60, height: 20, full: true,
                                   bytes: Data("\u{1b}[2Jhi".utf8))])
    }

    @Test("Frames split across reads are reassembled")
    func handlesSplitReads() {
        var parser = HerdrFrameParser()
        let line = frameLine(seq: 7, payload: "abc") + "\n"
        let bytes = Array(line.utf8)
        let cut = bytes.count / 3

        #expect(parser.consume(Data(bytes[..<cut])).isEmpty)
        #expect(parser.consume(Data(bytes[cut..<(cut * 2)])).isEmpty)
        let frames = parser.consume(Data(bytes[(cut * 2)...]))
        #expect(frames.count == 1)
        if case .screen(let seq, _, _, _, let payload) = frames[0] {
            #expect(seq == 7)
            #expect(payload == Data("abc".utf8))
        } else {
            Issue.record("expected a screen frame")
        }
    }

    @Test("Several frames in one read all come back, in order")
    func multipleFramesPerRead() {
        var parser = HerdrFrameParser()
        let blob = frameLine(seq: 1, payload: "a") + "\n" + frameLine(seq: 2, payload: "b") + "\n"
        let frames = parser.consume(Data(blob.utf8))
        #expect(frames.count == 2)
    }

    @Test("terminal.closed is surfaced so the session can drop its target")
    func parsesClosed() {
        var parser = HerdrFrameParser()
        let frames = parser.consume(Data("{\"type\":\"terminal.closed\",\"reason\":\"pane gone\"}\n".utf8))
        #expect(frames == [.closed(reason: "pane gone")])
    }

    // MARK: - Tolerating the shell it runs inside

    /// The channel lives in a login shell on a PTY, so the stream carries
    /// things that aren't frames. Verified live: macOS prints "The default
    /// interactive shell is now zsh." and the command echoes once before
    /// `stty -echo` takes effect.
    @Test("Shell banners and prompts between frames are skipped")
    func skipsShellNoise() {
        var parser = HerdrFrameParser()
        let blob = "The default interactive shell is now zsh.\r\n"
            + "stty raw -echo; herdr terminal session control w1:p1\r\n"
            + frameLine(seq: 1, payload: "ok") + "\n"
        let frames = parser.consume(Data(blob.utf8))
        #expect(frames.count == 1)
    }

    @Test("A prompt glued to the front of a frame line still parses")
    func promptPrefix() {
        var parser = HerdrFrameParser()
        let blob = "cluas@host ~ % " + frameLine(seq: 3, payload: "z") + "\n"
        let frames = parser.consume(Data(blob.utf8))
        #expect(frames.count == 1)
    }

    /// This is what makes retargeting safe without resetting the parser.
    @Test("A truncated line costs one frame, then the stream recovers")
    func recoversFromTruncation() {
        var parser = HerdrFrameParser()
        _ = parser.consume(Data("{\"type\":\"terminal.fra".utf8))   // cut mid-write
        let frames = parser.consume(Data(("\n" + frameLine(seq: 9, payload: "back") + "\n").utf8))
        #expect(frames.count == 1)
        if case .screen(let seq, _, _, _, _) = frames[0] { #expect(seq == 9) }
    }

    @Test("Unknown message types are ignored rather than treated as errors")
    func ignoresUnknownTypes() {
        var parser = HerdrFrameParser()
        let frames = parser.consume(Data("{\"type\":\"terminal.future\",\"x\":1}\n".utf8))
        #expect(frames.isEmpty)
    }

    @Test("A runaway stream with no newline can't grow without bound")
    func bufferIsCapped() {
        var parser = HerdrFrameParser()
        let chunk = Data(repeating: UInt8(ascii: "x"), count: 1_000_000)
        for _ in 0..<20 { _ = parser.consume(chunk) }
        // Still functional after the cap kicked in.
        let frames = parser.consume(Data(("\n" + frameLine(seq: 1, payload: "alive") + "\n").utf8))
        #expect(frames.count == 1)
    }

    // MARK: - Commands we send

    @Test("The boot line disables echo and does not exec")
    func startCommand() {
        let command = HerdrFrameCommand.start(target: "w1:p2", cols: 52, rows: 30, customPath: nil)
        // Without `-echo` the PTY would feed our own JSON back into the frame
        // stream; without raw mode, canonical input truncates at 4096 bytes.
        #expect(command.hasPrefix("stty raw -echo; "))
        // `exec` would kill the shell with the command, and retargeting needs
        // the shell to survive.
        #expect(!command.contains("exec "))
        #expect(command.contains("terminal session control 'w1:p2'"))
        #expect(command.contains("--cols 52 --rows 30"))
        // Reconnecting must be able to evict our own stale attach.
        #expect(command.contains("--takeover"))
    }

    @Test("A custom herdr path is honored in the boot line")
    func startCommandCustomPath() {
        let command = HerdrFrameCommand.start(target: "w1:p1", cols: 40, rows: 20,
                                              customPath: "/opt/herdr/bin/herdr")
        #expect(command.contains("/opt/herdr/bin/herdr terminal session control"))
        #expect(!command.contains("$HOME/.local/bin"))
    }

    @Test("Input is base64 so arbitrary bytes survive the JSON round trip")
    func inputEncoding() throws {
        // An escape sequence plus a multi-byte character — exactly what plain
        // JSON string encoding would be lossy or awkward about.
        let data = Data([0x1b, 0x5b, 0x41]) + Data("é".utf8)
        let line = HerdrFrameCommand.input(data)
        let object = try JSONSerialization.jsonObject(with: Data(line.utf8))
        let json = try #require(object as? [String: Any])
        #expect(json["type"] as? String == "terminal.input")
        let encoded = try #require(json["bytes"] as? String)
        let decoded = try #require(Data(base64Encoded: encoded))
        #expect(decoded == data)
    }

    @Test("Resize is valid JSON and never sends a zero dimension")
    func resizeEncoding() throws {
        let object = try JSONSerialization.jsonObject(
            with: Data(HerdrFrameCommand.resize(cols: 0, rows: -4).utf8))
        let json = try #require(object as? [String: Any])
        #expect(json["cols"] as? Int == 1)
        #expect(json["rows"] as? Int == 1)
    }

    @Test("Scroll carries direction and the source herdr routes on")
    func scrollEncoding() throws {
        func decode(_ line: String) throws -> [String: Any] {
            let object = try JSONSerialization.jsonObject(with: Data(line.utf8))
            return try #require(object as? [String: Any])
        }
        let up = try decode(HerdrFrameCommand.scroll(up: true, lines: 3))
        #expect(up["direction"] as? String == "up")
        #expect(up["lines"] as? Int == 3)
        // `wheel` is what lets the SERVER decide mouse-report vs scrollback —
        // the decision tmux makes us compute ourselves.
        #expect(up["source"] as? String == "wheel")

        let down = try decode(HerdrFrameCommand.scroll(up: false, lines: 0, source: .pageKey))
        #expect(down["direction"] as? String == "down")
        #expect(down["lines"] as? Int == 1)   // clamped
        #expect(down["source"] as? String == "page_key")
    }

    @Test("Release is the detach message, not a kill")
    func releaseEncoding() throws {
        let object = try JSONSerialization.jsonObject(with: Data(HerdrFrameCommand.release.utf8))
        let json = try #require(object as? [String: Any])
        #expect(json["type"] as? String == "terminal.release")
    }
}
