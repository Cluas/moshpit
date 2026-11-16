import Foundation
import Observation
import Testing
import UserNotifications
@testable import Moshpit
import MoshpitKit

/// The agent machine, end to end: the scripts the Rust machine runs (`crates/agents`,
/// `harness::run`) through the app's monitor — the Swift glue over
/// `AgentMachine`, across the FFI — must do exactly what the machine does
/// on its own (`Fixtures/agents/rust-transcripts.txt`, written by
/// `cargo run -p moshpit-agents --example agents_transcripts en`).
///
/// The same test, run against the Swift monitor before it was replaced,
/// recorded `crates/agents/tests/fixtures/swift/transcripts.txt`: what the
/// machine was held to (`swift_equivalence.rs`).
@Suite("Agents scripts (cross-check)", .serialized)
@MainActor
struct AgentsScriptTests {

    static let t0: TimeInterval = 1_790_000_000
    static let conn = "0F0E0D0C-0000-4000-8000-000000000002"

    struct Script: Decodable {
        let name: String
        let events: [Event]
    }

    struct Event: Decodable {
        let ev: String
        let t: Double
        var keep: Bool?
        var panes: PanesField?
        var pane: String?
        var visible: String?
        var ts: Double?
    }

    struct HookIn: Decodable {
        let pane: String
        let state: String?
        var agent: String?
        var since: Double?
        var command: String?
        var title: String?
    }

    /// `panes` is a tree (pane → command) or a poll (a list of hooks).
    enum PanesField: Decodable {
        case tree([String: String])
        case hooks([HookIn])
        init(from decoder: Decoder) throws {
            let single = try decoder.singleValueContainer()
            if let tree = try? single.decode([String: String].self) { self = .tree(tree) }
            else { self = .hooks(try single.decode([HookIn].self)) }
        }
    }

    final class Recorder: AgentIsland {
        var lines: [String] = []
        var up = false
        func live() -> String? { up ? "island" : nil }
        func request(_ state: AgentActivityAttributes.ContentState) -> String? {
            lines.append("island " + AgentsScriptTests.compact(state))
            up = true
            return "island"
        }
        func update(_ state: AgentActivityAttributes.ContentState) {
            lines.append("island " + AgentsScriptTests.compact(state))
        }
        func endAll() {
            lines.append("end-island")
            up = false
        }
    }

    final class Clock { var now = Date(timeIntervalSince1970: AgentsScriptTests.t0) }

    @Test("313 scripts through the app's monitor do what the Rust machine does")
    func transcripts() throws {
        let scripts = try JSONDecoder().decode([Script].self, from: try TestFixtures.data("agents/scripts.json"))
        var expected: [String: [String]] = [:]
        for line in try TestFixtures.string("agents/rust-transcripts.txt").split(separator: "\n") {
            let fields = line.split(separator: "\t", maxSplits: 2, omittingEmptySubsequences: false)
            guard fields.count == 3, fields[0] == "AGENTS" else { continue }
            expected[String(fields[1]), default: []].append(String(fields[2]))
        }
        #expect(expected.count == scripts.count)
        let saved = AgentControlBridge.shared.visiblePane
        defer { AgentControlBridge.shared.visiblePane = saved }
        var differing: [String] = []
        for script in scripts {
            let lines = try run(script)
            guard lines != expected[script.name] else { continue }
            let want = expected[script.name] ?? []
            let at = zip(lines, want).enumerated().first { $0.element.0 != $0.element.1 }?.offset
                ?? min(lines.count, want.count)
            differing.append("\(script.name) line \(at):\n  app  \(at < lines.count ? lines[at] : "<end>")"
                             + "\n  rust \(at < want.count ? want[at] : "<end>")")
        }
        #expect(differing.isEmpty, "\(differing.count) of \(scripts.count) differ:\n\(differing.prefix(5).joined(separator: "\n"))")
    }

    /// B407: with herdr pane w1:p1 on screen, w1:p2's waiting card is not
    /// the one being looked at: it stays posted and standing. With w1:p2
    /// itself on screen it is acknowledged, as a tmux pane's is.
    @Test
    func anotherHerdrPanesCardSurvivesTheSweep() throws {
        func script(onScreen pane: String) -> Script {
            Script(name: "herdr \(pane) on screen", events: [
                Event(ev: "tree", t: 0, panes: .tree(["w1:p1": "claude", "w1:p2": "claude"])),
                Event(ev: "track", t: 0),
                Event(ev: "hooks", t: 1, panes: .hooks([HookIn(
                    pane: "w1:p2", state: "attention", agent: "claude", since: 0,
                    command: "claude", title: "Bash: npm test")])),
                Event(ev: "sweep", t: 31),
                Event(ev: "sweepHerdr", t: 33, visible: pane),
            ])
        }
        let saved = AgentControlBridge.shared.visiblePane
        defer { AgentControlBridge.shared.visiblePane = saved }
        let other = try run(script(onScreen: "w1:p1"))
        #expect(other.contains { $0.hasPrefix("post ") && $0.contains(" w1:p2 ") }, "\(other)")
        #expect(other.last { $0.hasPrefix("standing ") } == "standing w1:p2@0", "\(other)")
        let same = try run(script(onScreen: "w1:p2"))
        #expect(same.last { $0.hasPrefix("standing ") } == "standing ", "\(same)")
    }

    /// B407: the pane on screen of a herdr session is its client's focused
    /// pane, not the whole connection; a session with no multiplexer is
    /// still the pane itself.
    @Test
    func aHerdrSessionsPaneIsTheOneOnScreen() throws {
        let connection = Self.connection()
        let session = Self.herdrSession(connection, focused: "w1:p1")
        let (conn, pane) = try #require(PaneOnScreen.of(session, foreground: true))
        #expect(conn == connection.id)
        #expect(pane == "w1:p1")
        #expect(PaneOnScreen.shows(session, foreground: true, connectionId: connection.id, paneId: "w1:p1"))
        #expect(!PaneOnScreen.shows(session, foreground: true, connectionId: connection.id, paneId: "w1:p2"))
        #expect(!PaneOnScreen.shows(session, foreground: true, connectionId: UUID(), paneId: "w1:p1"))
        let plain = SessionHub.ActiveSession(connection: connection)
        let (_, none) = try #require(PaneOnScreen.of(plain, foreground: true))
        #expect(none == nil)
        #expect(PaneOnScreen.shows(plain, foreground: true, connectionId: connection.id, paneId: "any"))
        #expect(PaneOnScreen.of(nil, foreground: true) == nil)
    }

    /// B438: the terminal stays presented when the phone locks; for the
    /// seconds the app runs on, its pane counted as looked at, and a prompt
    /// there was acknowledged — its card taken down — on a locked phone.
    @Test
    func noPaneIsOnScreenOutsideTheForeground() {
        let connection = Self.connection()
        let session = Self.herdrSession(connection, focused: "w1:p1")
        #expect(PaneOnScreen.of(session, foreground: false) == nil)
        #expect(!PaneOnScreen.shows(session, foreground: false, connectionId: connection.id, paneId: "w1:p1"))
        let plain = SessionHub.ActiveSession(connection: connection)
        #expect(!PaneOnScreen.shows(plain, foreground: false, connectionId: connection.id, paneId: "any"))
    }

    static func connection() -> ServerConnection {
        var connection = ServerConnection(name: "devbox", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        connection.id = UUID(uuidString: conn)!
        return connection
    }

    /// A session on `connection` running herdr on the core, two panes,
    /// `focused` the one its surface reads as focused (the core's mirror).
    static func herdrSession(_ connection: ServerConnection, focused: String) -> SessionHub.ActiveSession {
        let session = SessionHub.ActiveSession(connection: connection)
        let herdr = RustHerdrController(driver: nil)
        var model = RustSessionTests.herdrModel()
        model.panes = model.panes.map { pane in
            var pane = pane
            pane.command = "claude"
            pane.isActive = pane.id == focused
            return pane
        }
        model.activePaneId = focused
        herdr.mirror(model)
        session.rustSetHerdr(herdr)
        return session
    }

    private func run(_ script: Script) throws -> [String] {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("agents-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let standingURL = directory.appendingPathComponent("standing-attention.json")
        let finishesURL = directory.appendingPathComponent("rung-finishes.json")
        // What a push records, written as the notification service writes it.
        let pushed = PushFilesHandle(standingPath: standingURL.path, finishesPath: finishesURL.path)
        let suite = "test.agents.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let settings = AppSettings(defaults: defaults)
        settings.notificationsEnabled = true
        settings.liveActivityEnabled = true
        let recorder = Recorder()
        let clock = Clock()
        let notifier = AgentActivityMonitor.Notifier(
            add: { request in
                let content = request.content
                let pane = content.userInfo[AgentNotifications.paneKey] as? String ?? "?"
                recorder.lines.append("post \(request.identifier.replacingOccurrences(of: Self.conn, with: "C")) \(pane)"
                    + " | \(content.title) | \(content.body) | sound=\(content.sound != nil)"
                    + " | \(Self.level(content.interruptionLevel))")
            },
            withdraw: { recorder.lines.append("withdraw \($0.replacingOccurrences(of: Self.conn, with: "C"))") },
            requestAuthorization: { recorder.lines.append("auth") })
        let monitor = AgentActivityMonitor(
            settings: settings, standing: standingURL,
            writeWidget: { recorder.lines.append("widget " + Self.compact($0)) },
            island: recorder, finishes: finishesURL, notifier: notifier,
            clock: { clock.now },
            // A scratch suite and an unlocked phone, whatever the test host
            // says: the widget's snapshot waits for that (B603).
            protectedData: { true })
        let connection = Self.connection()
        let mux = AgentStandingTests.FakeMux()
        var lastT = 0.0
        for event in script.events {
            lastT = event.t
            clock.now = Date(timeIntervalSince1970: Self.t0 + event.t)
            switch event.ev {
            case "track":
                monitor.track(connection: connection, controller: mux)
            case "untrack":
                monitor.untrack(connectionId: connection.id, keepWaitingCards: event.keep ?? false)
            case "tree":
                guard case .tree(let tree) = event.panes else { break }
                var snapshot = TmuxSnapshot()
                snapshot.sessions["$0"] = SessionInfo(id: "$0", name: "work")
                snapshot.windows["@1"] = WindowInfo(id: "@1", sessionId: "$0", name: "api", index: 1)
                for (pane, command) in tree {
                    snapshot.panes[pane] = PaneInfo(id: pane, windowId: "@1", command: command)
                }
                mux.snapshot = snapshot
            case "hooks":
                guard case .hooks(let hooks) = event.panes else { break }
                mux.poll(Dictionary(hooks.map { hook in
                    (hook.pane, AgentHook(state: hook.state, agent: hook.agent,
                                          since: hook.since.map { Date(timeIntervalSince1970: Self.t0 + $0) },
                                          command: hook.command, title: hook.title))
                }, uniquingKeysWith: { _, last in last }))
            case "output":
                monitor.outputForScript(connectionId: connection.id, paneId: event.pane ?? "")
            case "bell":
                monitor.bellForScript(connectionId: connection.id, paneId: event.pane ?? "")
            case "sweep":
                let visible = event.visible
                AgentControlBridge.shared.visiblePane = {
                    visible.map { (connection.id, Optional($0)) }
                }
                monitor.sweep()
            case "sweepHerdr":
                // A herdr session on screen, its pane as the app reads it
                // (B407).
                let session = Self.herdrSession(connection, focused: event.visible ?? "")
                AgentControlBridge.shared.visiblePane = { PaneOnScreen.of(session, foreground: true) }
                monitor.sweep()
            case "push":
                _ = pushed.noteStanding(conn: Self.conn, entry: StandingEntryValue(
                    pane: event.pane ?? "", agent: "claude", title: "Bash: npm test",
                    since: Int64(Self.t0 + (event.ts ?? 0))), now: clock.now.timeIntervalSince1970)
            case "pushDone":
                _ = pushed.noteRing(conn: Self.conn, pane: event.pane ?? "",
                                    endedAt: Int64(Self.t0 + (event.ts ?? 0)),
                                    now: clock.now.timeIntervalSince1970)
            case "cycle":
                monitor.cycleHeadline()
            default:
                Issue.record("unknown event \(event.ev)")
            }
        }
        // What the record and the set hold at the end.
        let end = Date(timeIntervalSince1970: Self.t0 + lastT)
        var lines = recorder.lines
        let waiting = pushed.standing(conn: Self.conn, now: end.timeIntervalSince1970)
            .map { "\($0.pane)@\(Self.number(Double($0.since) - Self.t0))" }.sorted()
        lines.append("standing " + waiting.joined(separator: ","))
        let record = defaults.dictionary(forKey: "island.announcedEpisodes") as? [String: Double] ?? [:]
        let announced = record.map { key, at in
            "\(key.split(separator: "|", maxSplits: 1).last.map(String.init) ?? key)=\(Self.number(at - Self.t0))"
        }.sorted()
        lines.append("announced " + announced.joined(separator: ","))
        monitor.untrack(connectionId: connection.id)
        return lines
    }

    static func level(_ level: UNNotificationInterruptionLevel) -> String {
        switch level {
        case .passive: return "passive"
        case .active: return "active"
        case .timeSensitive: return "time-sensitive"
        case .critical: return "critical"
        @unknown default: return "?"
        }
    }

    /// As Rust's `{}` writes an f64: no ".0" on a whole number.
    static func number(_ value: Double) -> String {
        value == value.rounded() && abs(value) < 1e15 ? String(Int(value)) : String(value)
    }

    static func compact(_ state: AgentActivityAttributes.ContentState) -> String {
        let rows = state.agents.map { a in
            "\(a.paneId):\(a.state.rawValue):\(a.command):\(a.location):\(a.detail ?? "-"):\(number(a.startedAt.timeIntervalSince1970 - t0))"
        }
        let headline = state.headlineId.flatMap { $0.split(separator: ":").last.map(String.init) } ?? "-"
        return "[\(rows.joined(separator: ";"))] w=\(state.workingCount) a=\(state.attentionCount) h=\(headline)"
    }

    static func compact(_ snapshot: AgentWidgetState) -> String {
        let rows = snapshot.items.map { item in
            let pane = item.id.split(separator: ":").last.map(String.init) ?? "?"
            return "\(pane):\(item.state):\(item.command):\(item.location):\(item.detail ?? "-"):\(number(item.startedAt.timeIntervalSince1970 - t0))"
        }
        let headline = snapshot.headlineDeepLink.flatMap { AgentDeepLink.parse($0)?.paneId } ?? "-"
        return "[\(rows.joined(separator: ";"))] w=\(snapshot.workingCount) a=\(snapshot.attentionCount) h=\(headline)"
    }
}
