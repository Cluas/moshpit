import Foundation
#if canImport(ActivityKit)
import ActivityKit

/// Live Activity payload for the Vibe Island. ONE aggregate activity for all of
/// Moshpit: its ``ContentState`` carries the list of currently-active agent panes
/// (most-urgent first). The Dynamic Island pill shows the single headline agent
/// (iOS only renders one activity in the pill); the lock screen / expanded view
/// shows the list. Shared between the app target and the MoshpitIsland extension.
///
/// A "row only when a pane is actually doing something" is what keeps the lock
/// screen clean — idle shells never appear (the old design minted a card per
/// *connection* and stacked identical "work" cards).
public struct AgentActivityAttributes: ActivityAttributes {
    public enum AgentState: String, Codable, Hashable {
        case working      // output flowing — agent is busy
        case attention    // BEL / prompt — agent waits for the human
        case done         // was working, now quiet — finished its turn
        case idle         // quiet, nothing notable

        /// Resolved against `Bundle.main` — the app's or the widget's own
        /// catalog, both of which carry these four keys.
        public var label: String {
            switch self {
            case .working:   return String(localized: "working")
            case .attention: return String(localized: "needs you")
            case .done:      return String(localized: "done")
            case .idle:      return String(localized: "idle")
            }
        }

        /// Urgency for sorting + which one the pill shows. Lower = more urgent.
        public var rank: Int {
            switch self {
            case .attention: return 0
            case .working:   return 1
            case .done:      return 2
            case .idle:      return 3
            }
        }
    }

    /// One watched agent pane.
    public struct Agent: Codable, Hashable, Identifiable {
        /// "<connectionUUID>:<paneId>" — stable across updates.
        public var id: String
        public var connectionId: String
        /// tmux pane id ("%N") — the target for the Live Activity Allow/Deny
        /// buttons (see ``AgentApprovalIntent``).
        public var paneId: String
        /// Foreground command being watched, e.g. "claude", "cargo", "node".
        public var command: String
        /// Where it lives: "work · 2:rednote" (host · window:pane).
        public var location: String
        /// What the agent is actually doing / asking — the hook's `@moshpit_title`
        /// (e.g. "Bash: npm install", or the permission message). nil on the
        /// output heuristic (no precise content) and for older-build activities.
        public var detail: String?
        public var state: AgentState
        /// When the pane entered its current state — drives the live timer.
        public var startedAt: Date

        public init(id: String, connectionId: String, paneId: String, command: String,
                    location: String, detail: String? = nil, state: AgentState, startedAt: Date) {
            self.id = id; self.connectionId = connectionId; self.paneId = paneId
            self.command = command; self.location = location; self.detail = detail
            self.state = state; self.startedAt = startedAt
        }
    }

    public struct ContentState: Codable, Hashable {
        /// Active agents, urgency-first, capped (see ``AgentActivityMonitor``).
        public var agents: [Agent]
        public var workingCount: Int
        public var attentionCount: Int
        /// Deep link for a whole-activity tap → the headline agent's pane.
        public var headlineDeepLink: String?
        /// Which agent the Dynamic Island pill shows, when the user has cycled
        /// past the most-urgent one (the "switch agent" control). nil = default
        /// (most urgent). Decodes to nil for activities from older builds.
        public var headlineId: String?

        /// The agent the Dynamic Island pill represents — the cycled-to one if
        /// set and still present, otherwise the most-urgent (first).
        public var headline: Agent? {
            if let headlineId, let pick = agents.first(where: { $0.id == headlineId }) { return pick }
            return agents.first
        }

        public init(agents: [Agent], workingCount: Int, attentionCount: Int,
                    headlineDeepLink: String? = nil, headlineId: String? = nil) {
            self.agents = agents; self.workingCount = workingCount; self.attentionCount = attentionCount
            self.headlineDeepLink = headlineDeepLink; self.headlineId = headlineId
        }
    }

    /// Bumped if the schema changes so stale activities can be ended.
    public var schemaVersion: Int = 1

    public init(schemaVersion: Int = 1) {
        self.schemaVersion = schemaVersion
    }
}
#endif
