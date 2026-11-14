import Foundation
import MoshpitKit
#if canImport(ActivityKit)
import ActivityKit
#endif

/// Where `AgentActivityMonitor` puts its Live Activity — the one island it
/// keeps up while agents are listed.
///
/// The monitor decides WHAT to send and WHEN (only a state that changed,
/// or a refresh before the island would read as stale); this decides which
/// island takes it. A protocol so that decision can be counted by a test:
/// ActivityKit itself does not run on the iPad that runs the tests.
@MainActor
protocol AgentIsland: AnyObject {
    /// The island the next update goes to — after letting go of one that
    /// is over, and adopting one an earlier process left up. nil when there
    /// is none to update: none is up, islands are off, or the user swiped
    /// this one away.
    func live() -> String?
    /// Ask for a new island showing `state`. Its id, or nil when none may
    /// be shown or the system refused.
    func request(_ state: AgentActivityAttributes.ContentState) -> String?
    /// Send `state` to the island ``live()`` named.
    func update(_ state: AgentActivityAttributes.ContentState)
    /// No agents are listed: end every island this app has up.
    func endAll()
}

/// The island as ActivityKit shows it.
@MainActor
final class ActivityKitIsland: AgentIsland {
    private let settings: AppSettings

    init(settings: AppSettings) {
        self.settings = settings
    }

    /// How long a state stays fresh on the island. The monitor's poll stops
    /// the moment iOS suspends the app, and the island would go on showing
    /// "working" forever; past this the views render an honest "paused"
    /// hint (`context.isStale`) instead. The monitor sends again well
    /// before it runs out while it is watching.
    static let freshFor: TimeInterval = 120

    #if canImport(ActivityKit)
    private var activity: Activity<AgentActivityAttributes>?
    /// The user swiped the island away while agents were listed.
    private var dismissed = false
    /// The update on its way, which the next one waits for.
    private var pending: Task<Void, Never>?

    /// Whether an island in `state` still shows what it is sent. An ended
    /// one can linger on the lock screen for hours, frozen; a dismissed one
    /// is gone.
    nonisolated static func takesUpdates(_ state: ActivityState) -> Bool {
        state == .active || state == .stale
    }

    private var enabled: Bool {
        settings.liveActivityEnabled && ActivityAuthorizationInfo().areActivitiesEnabled
    }

    func live() -> String? {
        guard enabled else { return nil }
        // A handle to an island that is over takes updates and shows none.
        // iOS ends a Live Activity after eight hours whatever the app does,
        // and the user can swipe one away; the monitor kept updating the old
        // handle regardless, and the island never came back while any agent
        // was still listed — for a long-running agent, the rest of the day.
        if let current = activity, !Self.takesUpdates(current.activityState) {
            Log.island.info("island: \(current.id, privacy: .public) is \(String(describing: current.activityState), privacy: .public) — letting it go")
            if current.activityState == .dismissed { dismissed = true }
            activity = nil
        }
        // Swiped away: not back on the next sweep, two seconds later. It
        // returns once the agents it showed have all gone and new ones come.
        guard !dismissed else { return nil }
        if activity == nil { adoptOrphans() }
        return activity?.id
    }

    func request(_ state: AgentActivityAttributes.ContentState) -> String? {
        guard enabled, !dismissed else { return nil }
        activity = try? Activity.request(attributes: AgentActivityAttributes(),
                                         content: content(state))
        return activity?.id
    }

    func update(_ state: AgentActivityAttributes.ContentState) {
        guard let activity else { return }
        let content = content(state)
        // One at a time, in order: an update still on its way is not
        // overtaken by the next one.
        let previous = pending
        pending = Task {
            await previous?.value
            await activity.update(content)
        }
    }

    func endAll() {
        let live = Activity<AgentActivityAttributes>.activities
        if !live.isEmpty {
            Log.island.info("island: no agents, ending \(live.count) activity(ies)")
        }
        for a in live {
            Task { await a.end(nil, dismissalPolicy: .immediate) }
        }
        activity = nil
        dismissed = false
    }

    private func content(_ state: AgentActivityAttributes.ContentState)
        -> ActivityContent<AgentActivityAttributes.ContentState> {
        ActivityContent(state: state, staleDate: Date().addingTimeInterval(Self.freshFor))
    }

    /// Reclaim Live Activities left behind by earlier processes.
    ///
    /// `activity` is an in-memory handle, but the activity itself belongs to
    /// the system and outlives us (up to 8h). Every relaunch — jetsam, a
    /// crash, a swipe-kill, a TestFlight install — used to find the handle
    /// nil and request a fresh island, so they multiplied: one frozen
    /// "paused" pill per dead process, all showing the same agents. Keep one
    /// (it gets fresh content on this very sync) and end the rest.
    private func adoptOrphans() {
        let live = Activity<AgentActivityAttributes>.activities
        guard !live.isEmpty else { return }
        // Only one that still takes updates: falling back to any island at
        // all adopted an ended one, and every update after went nowhere.
        // The ended ones are ended for good, off the lock screen too, where
        // they'd otherwise sit showing hours-old agents.
        let keep = live.first { Self.takesUpdates($0.activityState) }
        activity = keep
        let orphans = live.filter { $0.id != keep?.id }
        Log.island.info("island: adopted \(keep?.id ?? "none", privacy: .public), ending \(orphans.count) orphan(s)")
        for orphan in orphans {
            Task { await orphan.end(nil, dismissalPolicy: .immediate) }
        }
    }
    #else
    func live() -> String? { nil }
    func request(_ state: AgentActivityAttributes.ContentState) -> String? { nil }
    func update(_ state: AgentActivityAttributes.ContentState) {}
    func endAll() {}
    #endif
}
