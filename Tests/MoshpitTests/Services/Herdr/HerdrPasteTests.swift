import Foundation
import Testing
@testable import Moshpit

/// herdr's frame channel takes a paste as ONE bracketed paste
/// (`ActiveSession.herdrPaste`), so a marker inside the pasted text must go:
/// herdr ends the paste at the first `ESC[201~`, and the rest is typed,
/// Return and all. Two ways one used to survive, both of which ran the
/// pasted command on live herdr 0.9.5 + zsh: a combining mark on the
/// marker's `~` (the removal compared `Character`s) and a marker assembled by
/// removing another (one pass).
@Suite("herdr paste", .serialized)
@MainActor
struct HerdrPasteTests {

    static let start = "\u{1B}[200~"
    static let end = "\u{1B}[201~"

    /// Crafted clipboard texts. `combining`: the end marker's `~` carries
    /// U+0301 (removal by `Character` missed it). `assembled`: removing the
    /// inner marker once leaves `ESC[20` + `1~`, a whole one. `bare`: a plain
    /// end marker. `opening`: a start marker, which stayed in the paste.
    static let combining = "ls\u{1B}[201~\u{301}\ntouch INJECTED\n"
    static let assembled = "\u{1B}[20\u{1B}[201~1~\ntouch INJECTED\n"
    static let bare = "ls\u{1B}[201~\ntouch INJECTED\n"
    static let opening = "ls\u{1B}[200~\ntouch INJECTED\n"

    /// What each must arrive as: the text with the markers gone.
    static let cleaned = [
        combining: "ls\u{301}\ntouch INJECTED\n",
        assembled: "\ntouch INJECTED\n",
        bare: "ls\ntouch INJECTED\n",
        opening: "ls\ntouch INJECTED\n",
    ]

    private func occurrences(of marker: String, in data: Data) -> Int {
        let bytes = [UInt8](data), pattern = [UInt8](marker.utf8)
        guard bytes.count >= pattern.count else { return 0 }
        return (0...(bytes.count - pattern.count))
            .filter { Array(bytes[$0..<($0 + pattern.count)]) == pattern }.count
    }

    private func waitUntil(timeout: Duration = .seconds(10), _ done: () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !(await done()) {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("A marker in the pasted text is dropped: one with a combining mark, one assembled, any",
          arguments: [combining, assembled, bare, opening])
    func herdrPaste(_ pasted: String) {
        let payload = SessionHub.ActiveSession.herdrPaste(pasted)
        #expect(payload == Data((Self.start + Self.cleaned[pasted]! + Self.end).utf8))
        #expect(occurrences(of: Self.end, in: payload) == 1, "the paste ends before its last bytes")
        #expect(occurrences(of: Self.start, in: payload) == 1)
    }

    @Test("What reaches the frame channel is one paste, markers gone")
    func herdrFrameChannel() async throws {
        var connection = ServerConnection(name: "paste-herdr", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        connection.connectionProtocol = .ssh
        connection.multiplexer = .herdr
        connection.herdrPath = "/usr/local/bin/herdr"
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let host = HerdrFrameHost(snapshot: HerdrFrameHost.onePane)
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        await session.start(theme: .githubDark, fontSize: 14)
        // The loop is the channel's program (B267): it says it reads, then
        // gets its first pane as one word.
        host.emit(Data((HerdrFrameCommand.readyMarker + "\n").utf8))
        #expect(await waitUntil { host.writtenText.contains("w1:p1 ") }, "the loop never got its first pane")
        let prompt = Data("$ ".utf8)
        host.emit(Data(HerdrFrameHost.frameLine(seq: 1, bytes: prompt, full: true).utf8))
        #expect(await waitUntil { session.coordinator.unattachedFeed.contains(prompt) })

        session.sendPaste(Self.combining)
        let expected = Data((Self.start + Self.cleaned[Self.combining]! + Self.end).utf8)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(expected)) },
                "the paste went out with its marker, or not at all")
        await session.stop()
    }
}
