import Foundation

/// A slim, ActivityKit-free snapshot of agent status, written by the app into the
/// App Group container and read by the home/lock-screen Widget's TimelineProvider.
///
/// Unlike the Live Activity (push-updated via ActivityKit), a WidgetKit timeline
/// widget PULLS on the OS's schedule, so it can't read the ActivityKit state — it
/// needs shared storage. The app writes this on every monitor sync and nudges
/// `WidgetCenter.reloadAllTimelines()`; the widget reads it in its provider.
public struct AgentWidgetState: Codable, Equatable {
    public struct Item: Codable, Equatable, Identifiable {
        public var id: String
        public var command: String
        public var location: String
        /// Hook `@moshpit_title` — what the agent is doing/asking; nil on the
        /// heuristic path. Optional so older snapshots still decode.
        public var detail: String?
        public var state: String        // AgentActivityAttributes.AgentState rawValue
        public var startedAt: Date
        /// Deep link to THIS agent's pane (`moshpit://connection/<uuid>?pane=%N`)
        /// — how the share extension addresses a queued image at a specific
        /// pane. Optional so older snapshots still decode.
        public var deepLink: String?
        /// Its connection dropped (B662): greyed, "offline". Optional so
        /// older snapshots still decode.
        public var paused: Bool?

        public init(id: String, command: String, location: String, detail: String? = nil,
                    state: String, startedAt: Date, deepLink: String? = nil, paused: Bool? = nil) {
            self.id = id; self.command = command; self.location = location; self.detail = detail
            self.state = state; self.startedAt = startedAt; self.deepLink = deepLink
            self.paused = paused
        }
    }
    public var items: [Item]
    public var attentionCount: Int
    public var workingCount: Int
    /// Deep link to the headline agent's pane (moshpit://…), for the widget tap.
    public var headlineDeepLink: String?
    public var updatedAt: Date

    public static let empty = AgentWidgetState(items: [], attentionCount: 0, workingCount: 0,
                                               headlineDeepLink: nil, updatedAt: .distantPast)

    public init(items: [Item], attentionCount: Int, workingCount: Int,
                headlineDeepLink: String? = nil, updatedAt: Date) {
        self.items = items; self.attentionCount = attentionCount; self.workingCount = workingCount
        self.headlineDeepLink = headlineDeepLink; self.updatedAt = updatedAt
    }
}

/// App Group bridge for the agent-status widget. The suite is shared between the
/// app (writer) and the MoshpitIsland extension (reader). On the simulator the
/// suite works without provisioning; on device the App Group capability must be
/// enabled for both the app and the extension.
///
/// The snapshot is a file in the App Group's folder kept out of backups
/// (B785, ``DeviceLocalFiles``): it was a key in the suite, which rides a
/// backup, and a phone restored from one showed the old phone's agents. The
/// key an earlier build wrote is never read, and goes at the first write.
public enum AgentWidgetStore {
    public static let appGroup = "group.com.cluas.moshpit"
    private static let key = "moshpit.widget.agentState"
    private static let filename = "agent-widget-state.json"

    private static var file: URL? { DeviceLocalFiles.url(filename, appGroup: appGroup) }

    public static func write(_ state: AgentWidgetState) {
        guard let file, let data = try? JSONEncoder().encode(state) else { return }
        try? data.write(to: file, options: .atomic)
        UserDefaults(suiteName: appGroup)?.removeObject(forKey: key)
    }

    public static func read() -> AgentWidgetState {
        guard let file, let data = try? Data(contentsOf: file),
              let state = try? JSONDecoder().decode(AgentWidgetState.self, from: data)
        else { return .empty }
        return state
    }
}
