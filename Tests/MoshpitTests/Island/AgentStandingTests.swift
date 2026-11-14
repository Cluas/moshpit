import Foundation
import Observation
import Testing
import UserNotifications
@testable import Moshpit
import MoshpitKit

/// The standing set (``PushStanding``) is what decides whether a waiting
/// prompt rings: only the 0→1 edge does. The app and the notification service
/// extension share it, and the app — the one of the two that can see a prompt
/// being answered — has to take answered prompts out of it, whoever put them
/// there.
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
        let file: PushStandingFile
        let finishes: PushFinishesFile
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
        let file = PushStandingFile(url: directory.appendingPathComponent("standing-attention.json"))
        let finishes = PushFinishesFile(url: directory.appendingPathComponent("rung-finishes.json"))
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
                                           notifier: cards.notifier)
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

    /// What the notification service extension records when a push opens.
    private func pushed(_ bench: Bench, pane: String, sentAt: Int) -> Bool {
        bench.file.noteStanding(conn: bench.conn, entry: .init(
            pane: pane, agent: "claude", title: "Bash: npm test",
            since: sentAt, recordedAt: Date()))
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

    @Test("a pushed prompt answered while the app polls leaves the set: the next question rings")
    func answeredPushedPromptLeaves() throws {
        let bench = try bench()
        defer { bench.finish() }
        let asked: TimeInterval = 1_790_000_000

        #expect(pushed(bench, pane: "%1", sentAt: Int(asked) + 31), "the first push is the edge")
        bench.mux.poll(["%1": hook("attention", since: asked)])
        #expect(bench.file.standing(conn: bench.conn).map(\.pane) == ["%1"])

        // Answered in the app.
        bench.mux.poll(["%1": hook("working", since: asked + 40)])
        #expect(bench.file.standing(conn: bench.conn).isEmpty,
                "the answered prompt still stands: \(bench.file.standing(conn: bench.conn).map(\.pane))")

        // The same turn's next question, pushed while the app is away again.
        #expect(pushed(bench, pane: "%1", sentAt: Int(asked) + 240),
                "the next question was no edge: it would ring nobody")
    }

    @Test("answering one of two waiting prompts leaves the other standing")
    func onlyTheAnsweredOneLeaves() throws {
        let bench = try bench()
        defer { bench.finish() }
        let asked: TimeInterval = 1_790_000_000

        #expect(pushed(bench, pane: "%1", sentAt: Int(asked) + 31))
        #expect(!pushed(bench, pane: "%2", sentAt: Int(asked) + 35))
        bench.mux.poll(["%1": hook("working", since: asked + 50),
                        "%2": hook("attention", since: asked + 4)])
        #expect(bench.file.standing(conn: bench.conn).map(\.pane) == ["%2"])
    }

    /// A poll reply that sat in the socket while the app was suspended says
    /// what the pane was doing BEFORE the question was asked; it answers
    /// nothing.
    @Test("a poll older than the question leaves it standing")
    func stalePollAnswersNothing() throws {
        let bench = try bench()
        defer { bench.finish() }
        let asked: TimeInterval = 1_790_000_000

        #expect(pushed(bench, pane: "%1", sentAt: Int(asked) + 31))
        bench.mux.poll(["%1": hook("working", since: asked - 120)])
        #expect(bench.file.standing(conn: bench.conn).map(\.pane) == ["%1"])
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

    // MARK: - A long finish rings once

    /// A long turn's finish reaches a phone twice when the app is awake: the
    /// host pushes it the moment the turn stops, and the app's poll sees it a
    /// second later. Both rang — two chimes for one build.
    @Test("a long turn's finish the push already rang is a silent card here")
    func pushedFinishRingsOnce() throws {
        let bench = try bench(notifications: true)
        defer { bench.finish() }
        let ended: TimeInterval = 1_790_000_600

        // The push landed first; the extension rang it.
        #expect(bench.finishes.noteRing(conn: bench.conn, pane: "%1", endedAt: Int(ended) + 1))

        bench.mux.poll(["%1": hook("working", since: ended - 600)])
        bench.mux.poll(["%1": hook("done", since: ended)])
        let done = try #require(bench.cards.added.last { $0.identifier.hasPrefix("moshpit.done.") })
        #expect(done.content.sound == nil, "one turn's finish rang twice")
        #expect(done.content.interruptionLevel == .passive)
        #expect(done.content.title.contains("10"), "the card still says how long it ran: \(done.content.title)")
    }

    @Test("a long finish nobody rang rings here, and the push after it doesn't")
    func localFinishRingsFirst() throws {
        let bench = try bench(notifications: true)
        defer { bench.finish() }
        let ended: TimeInterval = 1_790_000_600

        bench.mux.poll(["%1": hook("working", since: ended - 600)])
        bench.mux.poll(["%1": hook("done", since: ended)])
        let done = try #require(bench.cards.added.last { $0.identifier.hasPrefix("moshpit.done.") })
        #expect(done.content.sound != nil)
        #expect(done.content.interruptionLevel == .active)
        // The extension, a moment later, for the same turn.
        #expect(!bench.finishes.noteRing(conn: bench.conn, pane: "%1", endedAt: Int(ended) + 1))
        // A later long turn on the same pane rings again.
        #expect(bench.finishes.noteRing(conn: bench.conn, pane: "%1", endedAt: Int(ended) + 900))
    }

    // MARK: - Disconnecting

    /// The agents on a host the user disconnects from are still waiting. The
    /// card saying so was withdrawn with the panes, as if every prompt had
    /// been answered — the lock screen's only reminder, gone.
    @Test("disconnecting leaves a waiting prompt's card up; answering it, seen later, takes it down")
    func disconnectKeepsTheCard() throws {
        // Recent: an announced record older than a day is not read back.
        let asked = Date().timeIntervalSince1970.rounded() - 120
        let bench = try bench(notifications: true, announced: ["%1": asked])
        defer { bench.finish() }
        // Announced in an earlier session, still standing.
        _ = bench.file.noteStanding(conn: bench.conn, entry: .init(
            pane: "%1", agent: "claude", title: nil, location: "devbox",
            since: Int(asked), recordedAt: Date()))
        bench.mux.poll(["%1": hook("attention", since: asked)])
        let card = "moshpit.attention.\(bench.conn)"

        bench.monitor.untrack(connectionId: bench.connection.id, keepWaitingCards: true)
        #expect(!bench.cards.withdrawn.contains(card), "disconnecting withdrew a prompt still waiting")
        #expect(bench.file.standing(conn: bench.conn).map(\.pane) == ["%1"])

        // Connected again, the prompt answered meanwhile: now it goes.
        bench.monitor.track(connection: bench.connection, controller: bench.mux)
        bench.mux.poll(["%1": hook("attention", since: asked)])
        bench.mux.poll(["%1": hook("working", since: asked + 300)])
        #expect(bench.cards.withdrawn.contains(card))
        #expect(bench.file.standing(conn: bench.conn).isEmpty)
    }

    /// Past the standing set's fifteen minutes only the app remembers it put
    /// the card up; after a relaunch or a disconnect it forgot, and an
    /// answered prompt's card stayed on the lock screen for good.
    @Test("a card announced before a relaunch is withdrawn once its prompt is answered")
    func relaunchedCardIsWithdrawn() throws {
        // Recent: an announced record older than a day is not read back.
        let asked = Date().timeIntervalSince1970.rounded() - 120
        let bench = try bench(notifications: true, announced: ["%1": asked])
        defer { bench.finish() }
        bench.mux.poll(["%1": hook("attention", since: asked)])
        bench.mux.poll(["%1": hook("working", since: asked + 3600)])
        #expect(bench.cards.withdrawn.contains("moshpit.attention.\(bench.conn)"))
    }

    @Test("deleting a connection takes its cards with it")
    func deleteWithdraws() throws {
        // Recent: an announced record older than a day is not read back.
        let asked = Date().timeIntervalSince1970.rounded() - 120
        let bench = try bench(notifications: true, announced: ["%1": asked])
        defer { bench.finish() }
        _ = bench.file.noteStanding(conn: bench.conn, entry: .init(
            pane: "%1", agent: "claude", since: Int(asked), recordedAt: Date()))
        bench.mux.poll(["%1": hook("attention", since: asked)])
        bench.monitor.untrack(connectionId: bench.connection.id)
        #expect(bench.cards.withdrawn.contains("moshpit.attention.\(bench.conn)"))
    }
}
