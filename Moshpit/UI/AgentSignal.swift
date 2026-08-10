import SwiftUI

/// The one place that decides what an agent's state looks like on screen.
///
/// Three surfaces read the same `AgentHook.state` string — the window/session
/// dots in the sheets, the Agents section on Home, and the Vibe Island. Before
/// this existed, the sheets carried their own private `if states.contains(…)`
/// ladder; a second copy in a third surface is exactly how "amber means needs
/// you" quietly becomes true in two places and false in the third.
///
/// The state strings themselves come from two very different sources and mean
/// the same thing by the time they arrive here: tmux reads `@moshpit_*`
/// options that host-side hooks write, herdr reports `agent_status` natively
/// (see `HerdrSnapshot`).
enum AgentSignal: Equatable, CaseIterable {
    /// Waiting on a human — an approval, a question, a permission prompt.
    case attention
    /// Actively doing something.
    case working
    /// Finished, not yet looked at.
    case done

    init?(_ state: String?) {
        switch state {
        case "attention": self = .attention
        case "working":   self = .working
        case "done":      self = .done
        default:          return nil   // idle / unknown / no hook at all
        }
    }

    /// Vibe Island colours, so a dot means the same thing wherever it appears.
    ///
    /// Through `AgentPalette` — the shared, FIXED values — and pointedly not
    /// `Ink.accent`: the accent follows the user's theme, and state colours
    /// that move with taste stop being state colours. (Found as: working =
    /// teal on the lock screen, indigo — or whatever the theme said — in-app.)
    var color: Color {
        switch self {
        case .attention: return AgentPalette.attention
        case .working:   return AgentPalette.working
        case .done:      return AgentPalette.done
        }
    }

    /// Sort weight — what needs a human comes first. Used to order the Agents
    /// section, which exists to answer "who is waiting on me" at a glance.
    var rank: Int {
        switch self {
        case .attention: return 0
        case .working:   return 1
        case .done:      return 2
        }
    }

    var label: String {
        switch self {
        case .attention: return String(localized: "needs you")
        case .working:   return String(localized: "working")
        case .done:      return String(localized: "done")
        }
    }

    /// The signal a window or session row shows for everything inside it.
    ///
    /// Deliberately ignores `.done`: a finished agent is worth a ROW in the
    /// Agents list ("review me"), but lighting up a whole window dot for it
    /// would change what the tmux sheets have always shown. Aggregate dots
    /// stay exactly as they shipped — attention beats working, nothing else
    /// registers.
    static func aggregate(_ signals: [AgentSignal?]) -> AgentSignal? {
        let present = signals.compactMap { $0 }
        if present.contains(.attention) { return .attention }
        if present.contains(.working) { return .working }
        return nil
    }
}
