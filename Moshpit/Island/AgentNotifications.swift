import Foundation
import UserNotifications

/// Vibe Island — T1 control surface, notification half.
///
/// Defines the two interactive notification categories and the keys their
/// `userInfo` carries. `AgentActivityMonitor` posts with these categories;
/// `AgentNotificationHandler` consumes the action the user taps and routes it
/// through `AgentControlBridge` to the live pane.
enum AgentNotifications {
    enum Category {
        /// "agent needs you" — Allow · Deny · Reply.
        static let attention = "moshpit.category.attention"
        /// "agent finished" — Reply (send the next instruction).
        static let done = "moshpit.category.done"
    }

    /// `userInfo` keys identifying which pane a notification targets.
    static let connectionKey = "connectionId"
    static let paneKey = "paneId"

    /// Register the interactive categories and install the action handler as the
    /// notification-center delegate. Called once at launch — harmless for
    /// non-Pro users (no prompt; authorization is requested separately, only
    /// when a session is actually tracked).
    static func configure(delegate: UNUserNotificationCenterDelegate) {
        let center = UNUserNotificationCenter.current()
        center.delegate = delegate
        center.setNotificationCategories([attentionCategory(), doneCategory()])
    }

    /// A lock-screen / island control action failed to reach the agent's pane
    /// (transport dead and force-resume failed). Fired by the app-side handler
    /// so the user is TOLD instead of walking away believing they approved.
    static func postDeliveryFailure() {
        let content = UNMutableNotificationContent()
        content.title = String(localized: "Not delivered")
        content.body = String(localized: "Your tap didn't reach the agent — open Moshpit and answer there.")
        content.sound = .default
        UNUserNotificationCenter.current().add(UNNotificationRequest(
            identifier: "moshpit.delivery.failure",
            content: content, trigger: nil))
    }

    /// The user tapped Allow/Deny on a notification whose prompt has already
    /// been answered or superseded — the keystroke was NOT sent.
    static func postPromptExpired() {
        let content = UNMutableNotificationContent()
        content.title = String(localized: "Prompt already gone")
        content.body = String(localized: "That request was already answered or has changed — nothing was sent. Open Moshpit to see the current state.")
        content.sound = nil
        UNUserNotificationCenter.current().add(UNNotificationRequest(
            identifier: "moshpit.delivery.expired",
            content: content, trigger: nil))
    }

    private static func attentionCategory() -> UNNotificationCategory {
        let allow = UNNotificationAction(
            identifier: AgentAction.allow.rawValue,
            title: String(localized: "Allow"),
            options: [])
        let deny = UNNotificationAction(
            identifier: AgentAction.deny.rawValue,
            title: String(localized: "Deny"),
            options: [.destructive])
        let reply = UNTextInputNotificationAction(
            identifier: AgentAction.reply.rawValue,
            title: String(localized: "Reply"),
            options: [],
            textInputButtonTitle: String(localized: "Send"),
            textInputPlaceholder: String(localized: "Type a response…"))
        let interrupt = UNNotificationAction(
            identifier: AgentAction.interrupt.rawValue,
            title: String(localized: "Stop"),
            options: [.destructive])
        return UNNotificationCategory(
            identifier: Category.attention,
            actions: [allow, deny, reply, interrupt],
            intentIdentifiers: [],
            options: [.customDismissAction])
    }

    private static func doneCategory() -> UNNotificationCategory {
        let reply = UNTextInputNotificationAction(
            identifier: AgentAction.reply.rawValue,
            title: String(localized: "Reply"),
            options: [],
            textInputButtonTitle: String(localized: "Send"),
            textInputPlaceholder: String(localized: "Next instruction…"))
        return UNNotificationCategory(
            identifier: Category.done,
            actions: [reply],
            intentIdentifiers: [],
            options: [])
    }
}

/// Notification-center delegate for the control surface. Each tapped action
/// (Allow / Deny / Reply) or a body tap resolves the target pane from
/// `userInfo` and hands off to `AgentControlBridge` (which lives in the app
/// process and owns the live session).
///
/// Deliberately the completion-HANDLER delegate variants, not the async ones.
/// The async variants read cleaner, but the ObjC bridge invokes their
/// auto-generated completion on the concurrency pool's thread — and
/// UNUserNotificationCenter's own completion for `didReceive` runs UIKit
/// state-restoration/snapshot work (`_updateStateRestorationArchive…`) that
/// NSAsserts the main thread. Tapping a notification while Moshpit was
/// backgrounded crashed on exactly that assert (TestFlight 344, thread 17
/// SIGABRT). Here every completion is invoked from a `@MainActor` task, so
/// UIKit's continuation runs where it insists on running.
final class AgentNotificationHandler: NSObject, UNUserNotificationCenterDelegate {
    /// (connectionId, paneId) out of a notification's userInfo, or nil for
    /// notifications that aren't ours.
    private func target(of info: [AnyHashable: Any]) -> (UUID, String)? {
        guard let cidString = info[AgentNotifications.connectionKey] as? String,
              let connectionId = UUID(uuidString: cidString),
              let paneId = info[AgentNotifications.paneKey] as? String
        else { return nil }
        return (connectionId, paneId)
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                willPresent notification: UNNotification,
                                withCompletionHandler completionHandler:
                                    @escaping (UNNotificationPresentationOptions) -> Void) {
        // Surface attention even while Moshpit is foreground (the user may be on
        // a different screen than the agent's pane) — EXCEPT when the user is
        // looking at that exact pane: a banner + chime over the prompt you are
        // already reading is triple noise. Keep it in the list only.
        let target = target(of: notification.request.content.userInfo)
        Task { @MainActor in
            if let (connectionId, paneId) = target,
               AgentControlBridge.shared.isPaneVisible?(connectionId, paneId) == true {
                completionHandler([.list])
            } else {
                completionHandler([.banner, .sound, .list])
            }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        // Everything needed is extracted HERE, on the delegate's queue —
        // UNNotificationResponse isn't Sendable and shouldn't cross into the
        // main-actor task.
        let target = target(of: response.notification.request.content.userInfo)
        let actionIdentifier = response.actionIdentifier
        let text = (response as? UNTextInputNotificationResponse)?.userText

        Task { @MainActor in
            defer { completionHandler() }
            guard let (connectionId, paneId) = target else { return }

            // Body tap (or "open") → jump to the pane, same as the island
            // deep link.
            if actionIdentifier == UNNotificationDefaultActionIdentifier {
                AgentControlBridge.shared.open(connectionId: connectionId, paneId: paneId)
                return
            }
            guard let action = AgentAction(rawValue: actionIdentifier) else { return }
            await AgentControlBridge.shared.dispatch(action, connectionId: connectionId,
                                                     paneId: paneId, text: text)
        }
    }
}
