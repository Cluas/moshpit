import Foundation
import AppIntents
import OSLog

/// Vibe Island — the process-wide hop from a lock-screen surface into the app.
///
/// What is left here is navigation and status: open a pane, cycle which agent the
/// island shows, notice a self-test push, drain the share queue. The MoshpitIsland
/// extension links this file so `AgentCycleIntent` exists to attach to a Button;
/// there the closures are nil and `perform()` is never the copy that runs.
///
/// What used to be here was the CONTROL surface — `AgentAction`,
/// `AgentApprovalIntent`, and a `handler` that turned a lock-screen Allow or Deny
/// into a keystroke. All of it is gone, and the reason is worth keeping: those
/// buttons sent a BLIND key into a pane. Answering an agent's permission request
/// that way means approving something you have not read, in an app whose entire
/// value is that you can read it. The delivery problems were real too — a live
/// session is exactly what a lock screen does not have — but they were the second
/// reason, not the first.
@MainActor
final class AgentControlBridge {
    static let shared = AgentControlBridge()
    private init() {}

    /// Declared here rather than taken from `Log`, because this file is compiled
    /// into the widget extension too and `Moshpit/Services/Log.swift` is not.
    static let log = Logger(subsystem: "com.cluas.moshpit", category: "island")

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

    /// Set by the app at launch. Called when a pairing self-test push arrives,
    /// with the nonce it carried.
    ///
    /// Lives on the bridge for the same reason everything else here does: this
    /// file is linked into the extensions, and the notification-center delegate
    /// that spots the push must not reach for an app-only type. nil in every
    /// process but the app.
    var pushSelfTest: ((String) -> Void)?

    /// Set by the app at launch. Drains the share-extension image queue —
    /// the AttachImage intent calls it so a Shortcuts run delivers
    /// immediately when the target session is live. nil in the widget
    /// process (which never runs the intent's perform()).
    var drainShareQueue: (() async -> Void)?

    func open(connectionId: UUID, paneId: String) {
        guard let opener else {
            // Same trap, quieter consequence: a notification body tap that opens
            // nothing. Worth a log rather than an alert — the user is looking at
            // the app by then and can navigate.
            Self.log.error("deep link to pane \(paneId, privacy: .public) dropped: no opener")
            return
        }
        opener(connectionId, paneId)
    }

    func cycleHeadline() {
        cycler?()
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
