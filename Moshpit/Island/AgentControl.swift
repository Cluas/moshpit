import Foundation
import AppIntents

/// Vibe Island — T1 control surface.
///
/// The single process-wide hop from a lock-screen surface (a notification
/// action OR a Live Activity / Dynamic Island button) to the live tmux pane.
/// Both surfaces resolve in the APP's process: `LiveActivityIntent.perform()`
/// runs in-app (the system launches Moshpit to background if suspended), and the
/// `UNUserNotificationCenter` delegate is the app. The MoshpitIsland extension
/// links this file ONLY so `AgentApprovalIntent` exists to attach to a Button —
/// there `handler` is nil and `perform()` is never the copy that runs.
///
/// Honest iOS limit: delivery needs the SSH/tmux session still in the hub
/// (suspended is fine — it reconnects). A fully torn-down session can't be
/// revived from a background action; the user taps the notification to reopen.
@MainActor
final class AgentControlBridge {
    static let shared = AgentControlBridge()
    private init() {}

    /// Set by the app at launch. Maps an (action, connection, pane[, text]) to
    /// a real keystroke delivered over the live session — reconnecting first if
    /// iOS killed the socket while suspended. nil inside the widget process.
    var handler: ((AgentAction, _ connectionId: UUID, _ paneId: String, _ text: String?) async -> Void)?

    /// Set by the app at launch. Routes a notification body tap to the pane's
    /// terminal screen (mirrors the Live Activity's deep link).
    var opener: ((_ connectionId: UUID, _ paneId: String) -> Void)?

    /// Set by the app at launch. Cycles which agent the Live Activity shows as
    /// the headline (Dynamic Island), so multiple concurrent agents are all
    /// reachable from the island. nil inside the widget process.
    /// Answers "is this exact pane on screen right now?" — set by the app so
    /// foreground notification presentation can skip the banner + sound when
    /// the user is already looking at the prompt.
    var isPaneVisible: ((UUID, String) -> Bool)?
    var cycler: (() -> Void)?

    /// Set by the app at launch. Drains the share-extension image queue —
    /// the AttachImage intent calls it so a Shortcuts run delivers
    /// immediately when the target session is live. nil in the widget
    /// process (which never runs the intent's perform()).
    var drainShareQueue: (() async -> Void)?

    func dispatch(_ action: AgentAction, connectionId: UUID, paneId: String, text: String?) async {
        await handler?(action, connectionId, paneId, text)
    }

    func open(connectionId: UUID, paneId: String) {
        opener?(connectionId, paneId)
    }

    func cycleHeadline() {
        cycler?()
    }
}

/// The three control-surface verbs. Raw values double as the notification
/// action identifiers AND the intent's parameter, so the two surfaces stay in
/// lockstep.
enum AgentAction: String, Sendable {
    case allow
    case deny
    case reply
    /// Stop a running agent — Ctrl-C into the pane (universal SIGINT / cancel).
    case interrupt

    /// The keystroke this verb sends into the agent's tmux pane. Tuned for
    /// Claude Code's permission prompt — the affirmative option is the
    /// default-highlighted one (Enter selects it) and Esc cancels — which also
    /// fits the great majority of TUI yes/no menus.
    func bytes(text: String? = nil) -> Data {
        switch self {
        case .allow:
            return Data([0x0d])                 // Enter — accept the highlighted option
        case .deny:
            return Data([0x1b])                 // Esc — cancel / decline
        case .interrupt:
            return Data([0x03])                 // Ctrl-C — stop the running agent
        case .reply:
            var d = Data((text ?? "").utf8)     // the typed answer / quick-reply preset …
            d.append(0x0d)                      // … submitted with Enter
            return d
        }
    }
}

/// Tapping an Allow / Deny button inside the Dynamic Island or lock-screen Live
/// Activity. `perform()` runs in the app process and routes through the bridge;
/// in the widget process the bridge handler is nil, so the button merely needs
/// this type to compile.
struct AgentApprovalIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Respond to agent"
    static var isDiscoverable: Bool = false

    @Parameter(title: "Action") var action: String
    @Parameter(title: "Connection") var connectionId: String
    @Parameter(title: "Pane") var paneId: String
    /// Preset text for a quick-reply button (`.reply`); empty for the others.
    @Parameter(title: "Text") var text: String

    init() {}

    init(action: AgentAction, connectionId: String, paneId: String, text: String = "") {
        self.action = action.rawValue
        self.connectionId = connectionId
        self.paneId = paneId
        self.text = text
    }

    func perform() async throws -> some IntentResult {
        if let act = AgentAction(rawValue: action), let cid = UUID(uuidString: connectionId) {
            await AgentControlBridge.shared.dispatch(act, connectionId: cid, paneId: paneId,
                                                     text: text.isEmpty ? nil : text)
        }
        return .result()
    }
}

/// Tapping the "switch agent" affordance — cycles which agent the Dynamic Island
/// shows as the headline, so several concurrent agents are all reachable. Runs
/// in the app process (nil cycler in the widget process).
struct AgentCycleIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Switch agent"
    static var isDiscoverable: Bool = false

    init() {}

    func perform() async throws -> some IntentResult {
        await AgentControlBridge.shared.cycleHeadline()
        return .result()
    }
}
