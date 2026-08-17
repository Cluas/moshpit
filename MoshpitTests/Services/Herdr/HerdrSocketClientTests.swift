import Foundation
import Testing
@testable import Moshpit

/// The socket client against REAL herdr 0.8.0 (protocol 19) captures:
/// `herdr-socket-snapshot.json` is one `session.snapshot` reply line, and
/// `herdr-socket-events.jsonl` is a live `events.subscribe` stream — the
/// synthetic bootstrap replay (existing workspace/tab/pane arrive as
/// created/focused events) followed by a real `tab.create` mutation.
@Suite("herdr socket client")
struct HerdrSocketClientTests {

    private func fixture(_ name: String) throws -> String {
        let path = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // …/MoshpitTests/Services/Herdr
            .deletingLastPathComponent()   // …/MoshpitTests/Services
            .deletingLastPathComponent()   // …/MoshpitTests
            .appendingPathComponent("Fixtures/\(name)")
        return try String(contentsOf: path, encoding: .utf8)
    }

    /// A scripted far end: captures each outgoing line, extracts the request
    /// id, and answers with a canned body under that id — so the test drives
    /// the real encode → correlate → decode round trip, not internals.
    private final class ScriptedServer: @unchecked Sendable {
        private let lock = NSLock()
        private var sentLines: [String] = []
        var sent: [String] { lock.withLock { sentLines } }

        func record(_ data: Data) -> String? {
            let line = String(decoding: data, as: UTF8.self)
                .trimmingCharacters(in: .newlines)
            lock.withLock { sentLines.append(line) }
            guard let object = HerdrSnapshot.firstJSONObject(in: line) else { return nil }
            return object["id"] as? String
        }
    }

    // MARK: - Request round trip

    @Test("Request envelope carries string id, method, and mandatory params")
    func requestEnvelopeShape() async throws {
        let server = ScriptedServer()
        var respond: (@Sendable (String) async -> Void)!
        let client = HerdrSocketClient { data in
            if let id = server.record(data) {
                await respond(#"{"id":"\#(id)","result":{"type":"ok"}}"#)
            }
        }
        respond = { @Sendable line in await client.feed(Data((line + "\n").utf8)) }

        _ = try await client.request("ping")
        let sent = try #require(server.sent.first)
        let object = try #require(HerdrSnapshot.firstJSONObject(in: sent))
        // id must be a STRING — an integer id is silently ignored by herdr
        // (verified live: the server never replies), so this shape is
        // load-bearing, not style.
        #expect(object["id"] as? String != nil)
        #expect(object["method"] as? String == "ping")
        // params must exist even when empty — same silent-ignore failure.
        #expect(object["params"] as? [String: Any] != nil)
    }

    @Test("The real snapshot reply resolves the request and feeds the existing decoder")
    func snapshotRoundTrip() async throws {
        let reply = try fixture("herdr-socket-snapshot.json")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let server = ScriptedServer()
        var respond: (@Sendable (String) async -> Void)!
        let client = HerdrSocketClient { data in
            if let id = server.record(data) {
                // Re-stamp the capture's id ("s1") with the live request's.
                let line = reply.replacingOccurrences(of: #""id": "s1""#, with: #""id": "\#(id)""#)
                    .replacingOccurrences(of: #""id":"s1""#, with: #""id":"\#(id)""#)
                await respond(line)
            }
        }
        respond = { @Sendable line in await client.feed(Data((line + "\n").utf8)) }

        let raw = try await client.request("session.snapshot")
        let decoded = try #require(HerdrSnapshot.decode(raw))
        #expect(decoded.snapshot.sessions.count == 1)
        #expect(decoded.snapshot.panes.count == 1)
    }

    // MARK: - Event stream

    @Test("The real capture yields the bootstrap replay then the live mutation")
    func realEventStream() async throws {
        let client = HerdrSocketClient { _ in }
        let lines = try fixture("herdr-socket-events.jsonl")
            .split(separator: "\n").map(String.init)

        var received: [String] = []
        let collector = Task {
            for await event in client.events { received.append(event.kind) }
        }
        for line in lines { await client.feed(Data((line + "\n").utf8)) }
        await client.finishInput()
        _ = await collector.value

        #expect(received.count == lines.count)
        // The bootstrap replay: current state arrives as synthetic events.
        #expect(received.prefix(5) == [
            "workspace_created", "workspace_focused", "tab_created",
            "tab_focused", "pane_created",
        ])
        // The live tab.create mutation lands after the replay.
        #expect(received.filter { $0 == "tab_created" }.count == 2)
        #expect(received.contains("pane_agent_detected"))
    }

    @Test("Partial chunks reassemble; responses and events interleave on one pipe")
    func chunkedInterleaving() async throws {
        let client = HerdrSocketClient { _ in }
        var kinds: [String] = []
        let collector = Task {
            for await event in client.events { kinds.append(event.kind) }
        }
        let event = #"{"event":"pane_focused","data":{"pane_id":"w1:p1"}}"#
        // Split mid-line: no event until the newline completes it.
        let bytes = Data((event + "\n").utf8)
        await client.feed(bytes.prefix(17))
        await client.feed(bytes.dropFirst(17))
        // Shell noise between JSON lines is skipped, not fatal.
        await client.feed(Data("Last login: Sun Aug 17 on ttys002\n".utf8))
        await client.feed(Data((event + "\n").utf8))
        await client.finishInput()
        _ = await collector.value
        #expect(kinds == ["pane_focused", "pane_focused"])
    }

    // MARK: - Failure shapes

    @Test("A server error envelope throws with its code")
    func serverErrorThrows() async throws {
        var respond: (@Sendable (String) async -> Void)!
        let server = ScriptedServer()
        let client = HerdrSocketClient { data in
            if let id = server.record(data) {
                await respond(#"{"id":"\#(id)","error":{"code":"pane_not_found","message":"no such pane"}}"#)
            }
        }
        respond = { @Sendable line in await client.feed(Data((line + "\n").utf8)) }

        await #expect(throws: HerdrSocketError.server(code: "pane_not_found", message: "no such pane")) {
            _ = try await client.request("pane.focus")
        }
    }

    @Test("An empty-id error fails every in-flight request (server hangs up next)")
    func emptyIdFailsAllPending() async throws {
        let client = HerdrSocketClient { _ in /* never answered individually */ }
        let first = Task { try await client.request("session.snapshot") }
        let second = Task { try await client.request("ping") }
        // Give both requests time to register before the poison line lands.
        try await Task.sleep(for: .milliseconds(50))
        await client.feed(Data(#"{"id":"","error":{"code":"invalid_request","message":"missing field"}}"#.utf8 + [UInt8(ascii: "\n")]))

        await #expect(throws: HerdrSocketError.self) { _ = try await first.value }
        await #expect(throws: HerdrSocketError.self) { _ = try await second.value }
    }

    @Test("EOF fails pending requests instead of hanging them")
    func eofFailsPending() async throws {
        let client = HerdrSocketClient { _ in }
        let reply = Task { try await client.request("session.snapshot") }
        try await Task.sleep(for: .milliseconds(50))
        await client.finishInput()
        await #expect(throws: HerdrSocketError.connectionClosed) { _ = try await reply.value }
    }

    @Test("No reply times out rather than waiting forever")
    func requestTimesOut() async throws {
        let client = HerdrSocketClient(requestTimeout: .milliseconds(80)) { _ in }
        await #expect(throws: HerdrSocketError.timeout) {
            _ = try await client.request("ping")
        }
    }

    // MARK: - Subscription encoding

    @Test("Pane-scoped kinds carry pane_id; global kinds omit it entirely")
    func subscriptionEncoding() async throws {
        let server = ScriptedServer()
        var respond: (@Sendable (String) async -> Void)!
        let client = HerdrSocketClient { data in
            if let id = server.record(data) {
                await respond(#"{"id":"\#(id)","result":{"type":"subscription_started"}}"#)
            }
        }
        respond = { @Sendable line in await client.feed(Data((line + "\n").utf8)) }

        try await client.subscribe(
            ["tab.created"],
            paneScoped: [(kind: "pane.agent_status_changed", paneId: "w1:p1")])

        let sent = try #require(server.sent.first)
        let object = try #require(HerdrSnapshot.firstJSONObject(in: sent))
        let params = try #require(object["params"] as? [String: Any])
        let subs = try #require(params["subscriptions"] as? [[String: Any]])
        #expect(subs.count == 2)
        let global = try #require(subs.first { $0["type"] as? String == "tab.created" })
        // Absent, not null — the schema validates per-variant.
        #expect(global["pane_id"] == nil)
        let scoped = try #require(subs.first { $0["type"] as? String == "pane.agent_status_changed" })
        #expect(scoped["pane_id"] as? String == "w1:p1")
    }
}
