import Foundation

/// A slim, ActivityKit-free snapshot of agent status, written by the app into the
/// App Group container and read by the home/lock-screen Widget's TimelineProvider.
///
/// Unlike the Live Activity (push-updated via ActivityKit), a WidgetKit timeline
/// widget PULLS on the OS's schedule, so it can't read the ActivityKit state — it
/// needs shared storage. The app writes this on every monitor sync and nudges
/// `WidgetCenter.reloadAllTimelines()`; the widget reads it in its provider.
struct AgentWidgetState: Codable, Equatable {
    struct Item: Codable, Equatable, Identifiable {
        var id: String
        var command: String
        var location: String
        /// Hook `@moshpit_title` — what the agent is doing/asking; nil on the
        /// heuristic path. Optional so older snapshots still decode.
        var detail: String?
        var state: String        // AgentActivityAttributes.AgentState rawValue
        var startedAt: Date
    }
    var items: [Item]
    var attentionCount: Int
    var workingCount: Int
    /// Deep link to the headline agent's pane (moshpit://…), for the widget tap.
    var headlineDeepLink: String?
    var updatedAt: Date

    static let empty = AgentWidgetState(items: [], attentionCount: 0, workingCount: 0,
                                        headlineDeepLink: nil, updatedAt: .distantPast)
}

/// App Group bridge for the agent-status widget. The suite is shared between the
/// app (writer) and the MoshpitIsland extension (reader). On the simulator the
/// suite works without provisioning; on device the App Group capability must be
/// enabled for both the app and the extension.
enum AgentWidgetStore {
    static let appGroup = "group.com.cluas.moshpit"
    private static let key = "moshpit.widget.agentState"

    private static var defaults: UserDefaults? { UserDefaults(suiteName: appGroup) }

    static func write(_ state: AgentWidgetState) {
        guard let defaults, let data = try? JSONEncoder().encode(state) else { return }
        defaults.set(data, forKey: key)
    }

    static func read() -> AgentWidgetState {
        guard let defaults, let data = defaults.data(forKey: key),
              let state = try? JSONDecoder().decode(AgentWidgetState.self, from: data)
        else { return .empty }
        return state
    }
}
