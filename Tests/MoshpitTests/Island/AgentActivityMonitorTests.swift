import Foundation
import Testing
@testable import Moshpit
import MoshpitKit
#if canImport(ActivityKit)
import ActivityKit
#endif

/// What is left of the monitor in Swift: the platform half. Its rules —
/// announcing an episode once, the record across relaunches, reclassifying
/// fossils, telling a turn ending from an attach redraw — are the Rust
/// machine's now, tested in `crates/agents` (`src/tests.rs`, and against
/// the Swift monitor they replaced in `tests/swift_equivalence.rs`).
@Suite("AgentActivityMonitor")
@MainActor
struct AgentActivityMonitorTests {

    #if canImport(ActivityKit)
    /// iOS ends a Live Activity after eight hours, and the user can swipe one
    /// away. The monitor kept updating that handle — and adopted an ended one
    /// at launch — so the island never came back while any agent was listed.
    @Test("an island iOS ended or the user swiped away is let go, not updated")
    func endedIslandIsLetGo() {
        #expect(ActivityKitIsland.takesUpdates(.active))
        #expect(ActivityKitIsland.takesUpdates(.stale))
        #expect(!ActivityKitIsland.takesUpdates(.ended))
        #expect(!ActivityKitIsland.takesUpdates(.dismissed))
    }
    #endif

    /// The announced-episode record the machine keeps is the one earlier
    /// builds wrote: `"<uuid>|<pane>" -> epoch seconds` under the same key,
    /// so updating the app doesn't re-announce what is still standing.
    /// B538: a session that connected with notifications and the Live
    /// Activity both off was never watched: turning either on later did
    /// nothing for it until it reconnected. It is watched whatever they say;
    /// the machine decides, from the settings, what reaches the phone.
    @Test("a session is watched whatever the switches say (B538)")
    func watchedWithTheSwitchesOff() {
        let name = "test.agentmonitor.off.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let settings = AppSettings(defaults: defaults)
        settings.notificationsEnabled = false
        settings.liveActivityEnabled = false
        let monitor = AgentActivityMonitor(settings: settings, writeWidget: { _ in })
        let mux = AgentStandingTests.FakeMux()
        let connection = ServerConnection(name: "quiet", host: "192.0.2.2", port: 22,
                                          username: "tester", authMethod: .password)
        monitor.track(connection: connection, controller: mux)
        #expect(mux.onAgentHooksUpdated != nil, "connected with both off, never watched")
        monitor.untrack(connectionId: connection.id)
    }

    /// B594: the home-screen widget draws the most urgent agent (its first
    /// row) but opened the island's headline — the one the person had cycled
    /// to with Switch, which may be another agent: it showed one and opened
    /// another.
    @Test("the widget opens the agent it shows")
    func widgetOpensWhatItShows() {
        let row = { (id: String, state: String) in
            AgentRowValue(id: "C:\(id)", connectionId: "C", paneId: id, command: "claude",
                          location: "devbox", place: "devbox", detail: nil, state: state,
                          startedAt: 1_800_000_000, pausedAt: nil)
        }
        // %2 waits (first, most urgent); the pill was cycled to %1.
        let island = AgentIslandValue(agents: [row("%2", "attention"), row("%1", "working")],
                                      workingCount: 1, attentionCount: 1, headlineId: "C:%1")
        let widget = AgentActivityMonitor.widget(island, at: Date())
        #expect(widget.items.first?.id == "C:%2")
        #expect(widget.headlineDeepLink == widget.items.first?.deepLink,
                "the widget shows %2 and opened \(widget.headlineDeepLink ?? "-")")
    }

    /// 1.0.7 (1)'s note (B743): sharing images from Photos lists the agents
    /// to pick. The share extension lists the widget snapshot's items that
    /// address a pane (`Item.target`), so every agent the core reports has
    /// to: a working one, a waiting one, one whose connection dropped.
    @Test("every agent in the widget snapshot addresses its pane, so the share sheet lists it")
    func shareSheetListsTheAgents() {
        let connection = UUID()
        let row = { (pane: String, state: String, paused: Double?) in
            AgentRowValue(id: "\(connection.uuidString):\(pane)", connectionId: connection.uuidString,
                          paneId: pane, command: "claude", location: "devbox", place: "devbox",
                          detail: nil, state: state, startedAt: 1_800_000_000, pausedAt: paused)
        }
        let island = AgentIslandValue(
            agents: [row("%2", "attention", nil), row("%1", "working", nil), row("w1:p3", "idle", 1_800_000_100)],
            workingCount: 1, attentionCount: 1, headlineId: nil)
        let widget = AgentActivityMonitor.widget(island, at: Date())
        #expect(widget.items.count == 3)
        for item in widget.items {
            let target = item.target
            #expect(target?.connectionId == connection, "\(item.id): \(item.deepLink ?? "-")")
            #expect(target.map { "\(connection.uuidString):\($0.paneId)" } == item.id)
        }
    }

    /// B597: the compact pill's trailing "2" (two agents working) meant
    /// nothing to the owner. It says the headline's state now: needs you
    /// first — one "!", several the waiting hand and a count, never a bare
    /// number — else working its time, done a check, idle its dot.
    @Test("the compact pill says the headline's state")
    func compactSaysTheHeadlinesState() {
        let at = Date(timeIntervalSince1970: 1_800_000_000)
        let agent = { (pane: String, state: AgentActivityAttributes.AgentState) in
            AgentActivityAttributes.Agent(id: "C:\(pane)", connectionId: "C", paneId: pane,
                                          command: "claude", location: "mac", state: state,
                                          startedAt: at)
        }
        typealias State = AgentActivityAttributes.ContentState
        let two = State(agents: [agent("%1", .working), agent("%2", .working)],
                        workingCount: 2, attentionCount: 0)
        #expect(two.compactSignal == .working(since: at), "two working: the headline's time, not a 2")
        let done = State(agents: [agent("%1", .done), agent("%2", .working)],
                         workingCount: 1, attentionCount: 0, headlineId: "C:%1")
        #expect(done.compactSignal == .done)
        let one = State(agents: [agent("%1", .attention), agent("%2", .working)],
                        workingCount: 1, attentionCount: 1)
        #expect(one.compactSignal == .waiting(count: 1))
        let several = State(agents: [agent("%1", .attention), agent("%2", .attention)],
                            workingCount: 0, attentionCount: 2)
        #expect(several.compactSignal == .waiting(count: 2))
        #expect(State(agents: [agent("%1", .idle)], workingCount: 0, attentionCount: 0).compactSignal == .quiet)
        #expect(State(agents: [], workingCount: 0, attentionCount: 0).compactSignal == nil)
    }

    /// B709: B667 emptied the agents' snapshot at a launch that connected
    /// nowhere, and with it the share sheet's and the Shortcuts' targets,
    /// which read the same snapshot: they offered nothing where 1.0.8
    /// offered the last agents, saying how old the list is. A launch leaves
    /// it; the widget turns it stale on its own timeline.
    @Test("a launch leaves the last agents for the share sheet and Shortcuts",
          .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
    @MainActor
    func launchLeavesTheLastAgents() {
        let kept = AgentWidgetStore.read()
        defer { AgentWidgetStore.write(kept) }
        let then = Date(timeIntervalSince1970: 1_800_000_000)
        let id = UUID()
        let item = AgentWidgetState.Item(id: "\(id.uuidString):%1", command: "claude", location: "mac",
                                         state: "working", startedAt: then,
                                         deepLink: AgentDeepLink.string(connectionId: id, paneId: "%1"))
        AgentWidgetStore.write(AgentWidgetState(items: [item], attentionCount: 0, workingCount: 1,
                                                headlineDeepLink: item.deepLink, updatedAt: then))
        // A launch: the app's own monitor, writing the App Group's snapshot.
        let defaults = UserDefaults(suiteName: "test.launch.\(UUID().uuidString)")!
        let monitor = AgentActivityMonitor(
            settings: AppSettings(defaults: defaults), standing: nil, writeWidget: nil,
            island: AgentStandingTests.CountingIsland(), finishes: nil,
            notifier: .init(add: { _ in }, withdraw: { _ in }, requestAuthorization: {}))
        _ = monitor
        #expect(AgentWidgetStore.read().items.map(\.id) == [item.id], "the last agents are kept")
        #expect(AgentTargetEntity.fromSnapshot().map(\.paneId) == ["%1"], "Shortcuts offers them")
    }

    /// B662: a dropped connection's agents come from the machine paused,
    /// with when; the activity carries it (the island greys the row and
    /// stops its time there), the widget's item too, and the pill holds.
    @Test("a dropped connection's agents reach the island paused")
    func droppedConnectionPausesTheIsland() {
        let dropped = 1_800_000_060.0
        let row = { (pane: String, state: String, pausedAt: Double?) in
            AgentRowValue(id: "C:\(pane)", connectionId: "C", paneId: pane, command: "claude",
                          location: "mac", place: "mac", detail: nil, state: state,
                          startedAt: 1_800_000_000, pausedAt: pausedAt)
        }
        let island = AgentIslandValue(agents: [row("%1", "working", dropped)],
                                      workingCount: 0, attentionCount: 0, headlineId: "C:%1")
        let content = AgentActivityMonitor.content(island)
        #expect(content.agents[0].pausedAt == Date(timeIntervalSince1970: dropped))
        #expect(content.compactSignal == .paused)
        #expect(AgentActivityMonitor.widget(island, at: Date()).items[0].paused == true)
        // A live row elsewhere leads; the pill says its state.
        let both = AgentIslandValue(agents: [row("%2", "working", nil), row("%1", "attention", dropped)],
                                    workingCount: 1, attentionCount: 0, headlineId: "C:%2")
        #expect(AgentActivityMonitor.content(both).compactSignal
                == .working(since: Date(timeIntervalSince1970: 1_800_000_000)))
        #expect(AgentActivityMonitor.widget(both, at: Date()).items[0].paused == nil)
    }

    /// B701: the widget is written only when what it shows changes (each
    /// write reloads its timelines, from a daily budget), and its timeline
    /// turns it stale a horizon after the snapshot's date. Leaving the
    /// foreground writes the last one again, dated now: fresh while the app
    /// could still be watching. Not with no agents listed.
    @Test("leaving the foreground dates the widget's agents now, and nothing else")
    func leavingTheForegroundDatesTheWidgetNow() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let item = AgentWidgetState.Item(id: "C:%1", command: "claude", location: "mac",
                                         state: "working", startedAt: now.addingTimeInterval(-900))
        let last = AgentWidgetState(items: [item], attentionCount: 0, workingCount: 1,
                                    updatedAt: now.addingTimeInterval(-600))
        let again = AgentActivityMonitor.widgetRefreshedAtLeaving(last, at: now)
        #expect(again?.updatedAt == now)
        #expect(again?.items == last.items && again?.workingCount == 1)
        #expect(AgentActivityMonitor.widgetRefreshedAtLeaving(nil, at: now) == nil)
        #expect(AgentActivityMonitor.widgetRefreshedAtLeaving(.empty, at: now) == nil)
    }

    @Test("the announced record is read and written where earlier builds kept it")
    func announcedRecordKeepsItsKey() {
        #expect(AgentActivityMonitor.announcedKey == "island.announcedEpisodes")
    }

    /// A monitor made the way a prewarm before the first unlock makes it:
    /// the preferences can't be believed until `unlocked` says so. A
    /// question was announced before the reboot (`%9`).
    private final class Device: @unchecked Sendable {
        var unlocked = false
        var now = Date(timeIntervalSince1970: 1_790_000_000)
    }

    private func sealedMonitor(writeWidget: @escaping (AgentWidgetState) -> Void = { _ in })
        -> (AgentActivityMonitor, AgentStandingTests.FakeMux, ServerConnection, UserDefaults, String, Device) {
        let name = "test.agentmonitor.sealed.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        let settings = AppSettings(defaults: defaults)
        settings.notificationsEnabled = false
        settings.liveActivityEnabled = true
        // A sealed file holds no sentinel either.
        defaults.removeObject(forKey: DefaultsReadiness.sentinelKey)
        let connection = ServerConnection(name: "sealed", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        defaults.set(["\(connection.id.uuidString)|%9": 1_789_999_000.0],
                     forKey: AgentActivityMonitor.announcedKey)
        let device = Device()
        let monitor = AgentActivityMonitor(
            settings: settings, standing: nil, writeWidget: writeWidget,
            island: AgentStandingTests.CountingIsland(), finishes: nil,
            notifier: AgentStandingTests.Cards().notifier, clock: { device.now },
            readiness: DefaultsReadiness { device.unlocked })
        let mux = AgentStandingTests.FakeMux()
        monitor.track(connection: connection, controller: mux)
        return (monitor, mux, connection, defaults, name, device)
    }

    /// A question announced while the record can't be believed: the stored
    /// record is not written over (B348).
    @Test("a launch before the first unlock writes nothing over the announced record")
    func sealedWritesNothing() {
        let (monitor, mux, connection, defaults, name, device) = sealedMonitor()
        defer {
            monitor.untrack(connectionId: connection.id)
            defaults.removePersistentDomain(forName: name)
        }
        let before = defaults.dictionary(forKey: AgentActivityMonitor.announcedKey) as? [String: Double]
        mux.poll(["%1": AgentHook(state: "attention", agent: "claude", since: device.now,
                                  command: "claude", title: "Bash: npm test")])
        device.now += AgentActivityMonitor.announceGrace + 1
        monitor.sweep()
        let after = defaults.dictionary(forKey: AgentActivityMonitor.announcedKey) as? [String: Double]
        let unchanged = after == before
        #expect(unchanged, "the stored record was written over")
    }

    /// Once the device is unlocked and the app comes forward, the stored
    /// record joins what was announced meanwhile, and both are saved (B348).
    @Test("coming forward after the unlock keeps the stored episodes and the new one")
    func adoptionKeepsStoredEpisodes() {
        let (monitor, mux, connection, defaults, name, device) = sealedMonitor()
        defer {
            monitor.untrack(connectionId: connection.id)
            defaults.removePersistentDomain(forName: name)
        }
        let asked = device.now
        mux.poll(["%1": AgentHook(state: "attention", agent: "claude", since: asked,
                                  command: "claude", title: "Bash: npm test")])
        device.now += AgentActivityMonitor.announceGrace + 1
        monitor.sweep()
        monitor.reloadIfNeeded()
        let sealed = defaults.dictionary(forKey: AgentActivityMonitor.announcedKey) as? [String: Double]
        let untouched = sealed == ["\(connection.id.uuidString)|%9": 1_789_999_000.0]
        #expect(untouched, "nothing adopted while sealed")
        device.unlocked = true
        monitor.reloadIfNeeded()
        let stored = defaults.dictionary(forKey: AgentActivityMonitor.announcedKey) as? [String: Double]
        let record = stored ?? [:]
        let conn = connection.id.uuidString
        let before = record["\(conn)|%9"]
        let meanwhile = record["\(conn)|%1"]
        #expect(before == 1_789_999_000.0, "the stored episode")
        #expect(meanwhile == asked.timeIntervalSince1970, "the one announced meanwhile")
    }

    /// What the monitor wrote for the widget.
    private final class Snapshots {
        var written: [AgentWidgetState] = []
    }

    /// B603: the widget's snapshot lives in the App Group's preferences,
    /// which are sealed until the first unlock as the app's are, and a write
    /// into a domain that reads as empty can replace it with that one key —
    /// the push switches the notification service reads and the recorded
    /// language gone with it (``AppLanguage/recordIfTrusted``). Every other
    /// write there waits for the preferences to be believable; the snapshot
    /// was written on every sync. Now it is held, and the latest one written
    /// once they can be believed.
    @Test("a launch before the first unlock holds the widget's snapshot until it can be believed")
    func sealedHoldsTheWidget() {
        let snapshots = Snapshots()
        let (monitor, mux, connection, defaults, name, device) = sealedMonitor(
            writeWidget: { snapshots.written.append($0) })
        defer {
            monitor.untrack(connectionId: connection.id)
            defaults.removePersistentDomain(forName: name)
        }
        mux.poll(["%1": AgentHook(state: "working", agent: "claude", since: device.now,
                                  command: "claude", title: "Bash: npm test")])
        #expect(snapshots.written.isEmpty, "a snapshot written into sealed preferences")
        device.unlocked = true
        monitor.reloadIfNeeded()
        #expect(snapshots.written.count == 1, "the held snapshot, once")
        #expect(snapshots.written.last?.workingCount == 1)
        monitor.reloadIfNeeded()
        #expect(snapshots.written.count == 1, "written once")
    }
}
