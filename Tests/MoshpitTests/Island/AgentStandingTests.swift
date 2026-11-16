import Foundation
import Observation
import Testing
import UserNotifications
@testable import Moshpit
import MoshpitKit

/// The standing set (crates/seal's `standing`) is what decides whether a waiting
/// prompt rings: only the 0→1 edge does. The app and the notification service
/// extension share it, and the app — the one of the two that can see a prompt
/// being answered — has to take answered prompts out of it, whoever put them
/// there. What it decides is the core's (crates/agents `tests.rs`); here, the
/// Swift side: the island asked for once, and a launch's leftovers ended.
@Suite("Agent standing set", .serialized)
@MainActor
struct AgentStandingTests {

    /// A multiplexer whose hook poll the test writes.
    @Observable
    final class FakeMux: MultiplexerControlling {
        var multiplexer: Multiplexer { .tmux }
        var snapshot = TmuxSnapshot()
        var agentHooks: [String: AgentHook] = [:]
        @ObservationIgnored var onAgentHooksUpdated: (() -> Void)?
        func pollAgentHooks() {}
        var isRefreshing: Bool { false }
        func selectWindow(_ windowId: String) {}
        func selectSession(_ sessionId: String) {}
        func selectPane(_ paneId: String) {}
        func newWindow(named name: String?) {}
        func newPane() {}
        func newSession(named name: String?) {}
        func newWindow(inSession sessionId: String, named name: String?) {}
        func newPane(inWindow windowId: String) {}
        func renameSession(_ sessionId: String, to name: String) {}
        func killSession(_ sessionId: String) {}
        func renameWindow(_ windowId: String, to name: String) {}
        func killWindow(_ windowId: String) {}
        func killPane(_ paneId: String) {}
        func refresh() {}

        /// One poll's answer, delivered as the controller delivers it.
        func poll(_ hooks: [String: AgentHook]) {
            agentHooks = hooks
            onAgentHooksUpdated?()
        }
    }

    /// The widget snapshots the monitor wrote, kept here instead of the App
    /// Group.
    final class Writes {
        var snapshots: [AgentWidgetState] = []
    }

    /// An island that is always up, and counts what it is sent.
    final class CountingIsland: AgentIsland {
        var requests: [AgentActivityAttributes.ContentState] = []
        var updates: [AgentActivityAttributes.ContentState] = []
        var up = false
        func live() -> String? { up ? "island" : nil }
        func request(_ state: AgentActivityAttributes.ContentState) -> String? {
            requests.append(state)
            up = true
            return "island"
        }
        func update(_ state: AgentActivityAttributes.ContentState) { updates.append(state) }
        func endAll() { up = false }
        var leftoversEnded = 0
        func endLeftovers() { leftoversEnded += 1 }
    }

    /// The cards the monitor posted and withdrew.
    final class Cards {
        var added: [UNNotificationRequest] = []
        var withdrawn: [String] = []
        var notifier: AgentActivityMonitor.Notifier {
            .init(add: { self.added.append($0) },
                  withdraw: { self.withdrawn.append($0) },
                  requestAuthorization: {})
        }
    }

    @MainActor
    private struct Bench {
        let monitor: AgentActivityMonitor
        let mux: FakeMux
        let file: URL
        let finishes: URL
        let connection: ServerConnection
        let directory: URL
        let defaults: String
        let writes: Writes
        let island: CountingIsland
        let cards: Cards

        var conn: String { connection.id.uuidString }

        func finish() {
            monitor.untrack(connectionId: connection.id)
            try? FileManager.default.removeItem(at: directory)
            UserDefaults().removePersistentDomain(forName: defaults)
        }
    }

    /// `notifications`: cards on (into ``Cards``, never the device's
    /// notification center). `announced`: panes whose prompt this app
    /// announced in an earlier session, pane → episode start, as the record
    /// a relaunch reads back.
    private func bench(notifications: Bool = false,
                       announced: [String: TimeInterval] = [:]) throws -> Bench {
        let writes = Writes()
        let island = CountingIsland()
        let cards = Cards()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("standing-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("standing-attention.json")
        let finishes = directory.appendingPathComponent("rung-finishes.json")
        let connection = ServerConnection(name: "standing", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        let name = "test.standing.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        if !announced.isEmpty {
            defaults.set(announced.reduce(into: [String: Double]()) {
                $0["\(connection.id.uuidString)|\($1.key)"] = $1.value
            }, forKey: "island.announcedEpisodes")
        }
        let settings = AppSettings(defaults: defaults)
        settings.notificationsEnabled = notifications
        settings.liveActivityEnabled = true
        let monitor = AgentActivityMonitor(settings: settings, standing: file,
                                           writeWidget: { writes.snapshots.append($0) },
                                           island: island, finishes: finishes,
                                           notifier: cards.notifier,
                                           // An unlocked phone, whatever the
                                           // test host says (B603).
                                           protectedData: { true })
        let mux = FakeMux()
        monitor.track(connection: connection, controller: mux)
        return Bench(monitor: monitor, mux: mux, file: file, finishes: finishes,
                     connection: connection, directory: directory, defaults: name,
                     writes: writes, island: island, cards: cards)
    }

    private func hook(_ state: String, since: TimeInterval) -> AgentHook {
        AgentHook(state: state, agent: "claude", since: Date(timeIntervalSince1970: since),
                  command: "claude", title: "Bash: npm test")
    }

    /// The flow the push exists for: away from the desk, the push rings; the
    /// user opens the app and answers inside the announce grace (so the app
    /// never announced it itself); the agent carries on and asks again a few
    /// minutes later. That second question used to arrive silent: the first
    /// one's entry still stood, so it was no edge.
    /// B441: a killed process leaves its island up, and nothing ended it
    /// until this process had something to show. The monitor ends what an
    /// earlier process left the moment it is made, before it tracks anything.
    @Test("a launch ends the islands an earlier process left up")
    func aLaunchEndsLeftoverIslands() throws {
        let bench = try bench()
        defer { bench.finish() }
        #expect(bench.island.leftoversEnded == 1)
    }

    /// Every sync sent the island and the widget what they already had, and
    /// a sync runs on every two-second hook poll: an owner's phone with one
    /// tmux session open logged an ActivityKit update every one to two
    /// seconds, continuously, each with a widget write and a timeline reload
    /// (B304).
    @Test("an unchanged state is not sent again on every poll (B304)")
    func unchangedStateIsNotResent() throws {
        let bench = try bench()
        defer { bench.finish() }
        let asked: TimeInterval = 1_790_000_000

        for _ in 0..<10 { bench.mux.poll(["%1": hook("working", since: asked)]) }
        #expect(bench.island.requests.count == 1, "the island is asked for once")
        #expect(bench.island.updates.isEmpty,
                "ten polls of the same state updated the island \(bench.island.updates.count) times")
        #expect(bench.writes.snapshots.count == 1,
                "ten polls of the same state wrote the widget \(bench.writes.snapshots.count) times")

        bench.mux.poll(["%1": hook("attention", since: asked + 60)])
        #expect(bench.island.updates.count == 1, "a change must still reach the island")
        #expect(bench.island.updates.last?.attentionCount == 1)
        #expect(bench.writes.snapshots.count == 2, "a change must still reach the widget")

        // And quiet again at the new state.
        for _ in 0..<10 { bench.mux.poll(["%1": hook("attention", since: asked + 60)]) }
        #expect(bench.island.updates.count == 1)
    }

}
