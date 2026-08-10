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
struct AgentActivityAttributes: ActivityAttributes {
    enum AgentState: String, Codable, Hashable {
        case working      // output flowing — agent is busy
        case attention    // BEL / prompt — agent waits for the human
        case done         // was working, now quiet — finished its turn
        case idle         // quiet, nothing notable

        var label: String {
            switch self {
            case .working:   return String(localized: "working")
            case .attention: return String(localized: "needs you")
            case .done:      return String(localized: "done")
            case .idle:      return String(localized: "idle")
            }
        }

        /// Urgency for sorting + which one the pill shows. Lower = more urgent.
        var rank: Int {
            switch self {
            case .attention: return 0
            case .working:   return 1
            case .done:      return 2
            case .idle:      return 3
            }
        }
    }

    /// One watched agent pane.
    struct Agent: Codable, Hashable, Identifiable {
        /// "<connectionUUID>:<paneId>" — stable across updates.
        var id: String
        var connectionId: String
        /// tmux pane id ("%N") — the target for the Live Activity Allow/Deny
        /// buttons (see ``AgentApprovalIntent``).
        var paneId: String
        /// Foreground command being watched, e.g. "claude", "cargo", "node".
        var command: String
        /// Where it lives: "work · 2:rednote" (host · window:pane).
        var location: String
        /// What the agent is actually doing / asking — the hook's `@moshpit_title`
        /// (e.g. "Bash: npm install", or the permission message). nil on the
        /// output heuristic (no precise content) and for older-build activities.
        var detail: String? = nil
        var state: AgentState
        /// When the pane entered its current state — drives the live timer.
        var startedAt: Date
    }

    struct ContentState: Codable, Hashable {
        /// Active agents, urgency-first, capped (see ``AgentActivityMonitor``).
        var agents: [Agent]
        var workingCount: Int
        var attentionCount: Int
        /// Deep link for a whole-activity tap → the headline agent's pane.
        var headlineDeepLink: String?
        /// Which agent the Dynamic Island pill shows, when the user has cycled
        /// past the most-urgent one (the "switch agent" control). nil = default
        /// (most urgent). Decodes to nil for activities from older builds.
        var headlineId: String? = nil

        /// The agent the Dynamic Island pill represents — the cycled-to one if
        /// set and still present, otherwise the most-urgent (first).
        var headline: Agent? {
            if let headlineId, let pick = agents.first(where: { $0.id == headlineId }) { return pick }
            return agents.first
        }
    }

    /// Bumped if the schema changes so stale activities can be ended.
    var schemaVersion: Int = 1
}
#endif
