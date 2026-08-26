import Foundation
import UserNotifications

/// Vibe Island — the notification half.
///
/// Two `userInfo` keys and a delegate. That is deliberately all of it.
///
/// This file used to define interactive categories — Allow, Deny, Reply, Stop —
/// and they are gone. Not because they were hard to deliver (they were, in three
/// separate ways) but because the operation did not belong: each one sent a blind
/// keystroke into a pane, so answering an agent's permission request meant
/// approving something you had not read, from an app whose whole purpose is that
/// you can read it. A notification's job here is to wake you, name the agent, say
/// what it is asking, and take you to the pane. `AgentNotificationHandler` routes
/// that tap on the keys below.
enum AgentNotifications {
    /// `userInfo` keys identifying which pane a notification targets.
    static let connectionKey = "connectionId"
    static let paneKey = "paneId"

    /// Install the tap handler as the notification-center delegate. Called from
    /// `MoshpitApp.init()` — harmless for non-Pro users (no prompt; authorization
    /// is requested separately, only when a session is actually tracked).
    static func configure(delegate: UNUserNotificationCenterDelegate) {
        // Only the delegate. There are no categories any more: a category exists
        // to carry ACTION BUTTONS, and Moshpit's notifications no longer have
        // any. Allow/Deny sent a blind Enter or Esc into a pane — approving a
        // coding agent's permission request from a lock screen without reading
        // what it asked, in an app whose entire value is that you CAN read it.
        // The honest notification wakes you, says which agent wants what, and
        // opens the pane when tapped; that tap routes on `userInfo`, never on a
        // category identifier.
        UNUserNotificationCenter.current().delegate = delegate
    }
}

/// Notification-center delegate. A tapped notification resolves its target pane
/// from `userInfo` and asks `AgentControlBridge` to open it. That is the only
/// interaction — the tapped-ACTION branch that used to sit here, turning Allow /
/// Deny / Reply into keystrokes, went with the buttons.
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

    /// A pairing self-test's nonce, when this notification is one — nil for
    /// everything else. Pure and static so a test can pin it: this recognition
    /// was DESTROYED once, silently, in an unrelated rewrite of the delegate,
    /// and from that day every "send a test notification" timed out with "the
    /// host sent one, but this screen never saw it arrive" — the push chain was
    /// perfect and the last inch was gone. Nothing failed loudly, because the
    /// closure it feeds is optional and nobody was left to call it.
    static func selfTestNonce(in info: [AnyHashable: Any]) -> String? {
        guard info[PushRemoteNotification.agentKey] as? String
                == PushRemoteNotification.selfTestAgent else { return nil }
        return info[PushRemoteNotification.detailKey] as? String ?? ""
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                willPresent notification: UNNotification,
                                withCompletionHandler completionHandler:
                                    @escaping (UNNotificationPresentationOptions) -> Void) {
        // A pairing self-test is plumbing, not news: hand its nonce to the
        // sheet that is waiting on it and show nothing.
        if let nonce = Self.selfTestNonce(in: notification.request.content.userInfo) {
            Task { @MainActor in
                AgentControlBridge.shared.pushSelfTest?(nonce)
                completionHandler([])
            }
            return
        }
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

        Task { @MainActor in
            defer { completionHandler() }
            guard let (connectionId, paneId) = target else { return }
            // A body tap, or the system's "open". Nothing else can arrive: with
            // no categories registered there are no action identifiers to send.
            guard actionIdentifier == UNNotificationDefaultActionIdentifier else { return }
            AgentControlBridge.shared.open(connectionId: connectionId, paneId: paneId)
        }
    }
}
