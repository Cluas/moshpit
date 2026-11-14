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

    private func makeMonitor() -> (AgentActivityMonitor, UserDefaults, String) {
        let name = "test.agentmonitor.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        let settings = AppSettings(defaults: defaults)
        return (AgentActivityMonitor(settings: settings, writeWidget: { _ in }), defaults, name)
    }

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

    @Test("untracking an unknown connection is a harmless no-op")
    func untrackUnknownIsSafe() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        monitor.untrack(connectionId: UUID())
    }

    @Test("cycleHeadline is safe to call with no agents present")
    func cycleHeadlineOnEmptyStateIsSafe() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        monitor.cycleHeadline()
        monitor.cycleHeadline()
    }

    /// The announced-episode record the machine keeps is the one earlier
    /// builds wrote: `"<uuid>|<pane>" -> epoch seconds` under the same key,
    /// so updating the app doesn't re-announce what is still standing.
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

    private func sealedMonitor() -> (AgentActivityMonitor, AgentStandingTests.FakeMux, ServerConnection,
                                     UserDefaults, String, Device) {
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
            settings: settings, standing: nil, writeWidget: { _ in },
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
}
