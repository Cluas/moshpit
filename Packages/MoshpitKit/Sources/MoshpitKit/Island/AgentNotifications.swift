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
public enum AgentNotifications {
    /// `userInfo` keys identifying which pane a notification targets.
    public static let connectionKey = "connectionId"
    /// The agent, the status's state and the hook's title a card carries
    /// (crates/seal's `AGENT_KEY`, `STATE_KEY`, `DETAIL_KEY`): the push
    /// extension's cards and the app's alike.
    public static let agentKey = "moshpitAgent"
    public static let stateKey = "moshpitState"
    public static let detailKey = "moshpitDetail"
    /// A pairing self-test's agent (crates/seal's `SELF_TEST_AGENT`): plumbing,
    /// not news; the extension and the delegate both recognise it.
    public static let selfTestAgent = "moshpit-selftest"
    public static let paneKey = "paneId"

    /// Install the tap handler as the notification-center delegate. Called from
    /// `MoshpitApp.init()` — harmless for non-Pro users (no prompt; authorization
    /// is requested separately, only when a session is actually tracked).
    public static func configure(delegate: UNUserNotificationCenterDelegate) {
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
public final class AgentNotificationHandler: NSObject, UNUserNotificationCenterDelegate {
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
    public static func selfTestNonce(in info: [AnyHashable: Any]) -> String? {
        guard info[AgentNotifications.agentKey] as? String
                == AgentNotifications.selfTestAgent else { return nil }
        return info[AgentNotifications.detailKey] as? String ?? ""
    }

    public func userNotificationCenter(_ center: UNUserNotificationCenter,
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
        let content = notification.request.content
        let target = target(of: content.userInfo)
        let level = content.interruptionLevel
        let finished = content.userInfo[AgentNotifications.stateKey] as? String == "done"
        Task { @MainActor in
            let visible = target.map { AgentControlBridge.shared.isPaneVisible?($0.0, $0.1) == true } ?? false
            completionHandler(Self.foregroundPresentation(level: level, paneVisible: visible,
                                                          finished: finished))
        }
    }

    /// How a card shows while Moshpit is in the foreground.
    ///
    /// The moments worth an interruption — the first prompt of a wait, a
    /// long turn finishing — come `.timeSensitive` or `.active`, and banner
    /// even in the foreground: the user may be on another screen than the
    /// agent's pane. But not over that very pane: a banner and a chime over
    /// the prompt being read is noise, so it goes to the list only.
    ///
    /// A `.passive` card is an update to one the user already has — the
    /// waiting card redrawn because a second agent joined or one was
    /// answered, a short turn's finish. In the background it lands in the
    /// list without lighting the screen; in the foreground it bannered all
    /// the same, every redraw, because this answered every card alike (B315).
    ///
    /// A finish of the pane on screen isn't shown at all (B449): the person
    /// watched the turn end, and the pushed copy of it is not news — not
    /// even in the list. (The app posts no local one for it.)
    public static func foregroundPresentation(level: UNNotificationInterruptionLevel,
                                              paneVisible: Bool,
                                              finished: Bool = false) -> UNNotificationPresentationOptions {
        if paneVisible && finished { return [] }
        if paneVisible || level == .passive { return [.list] }
        return [.banner, .sound, .list]
    }

    public func userNotificationCenter(_ center: UNUserNotificationCenter,
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
            Log.island.info("land: notification tap conn=\(connectionId.uuidString, privacy: .public) pane=\(paneId, privacy: .public) action=\(actionIdentifier, privacy: .public)")
            // A body tap, or the system's "open". Nothing else can arrive: with
            // no categories registered there are no action identifiers to send.
            guard actionIdentifier == UNNotificationDefaultActionIdentifier else { return }
            AgentControlBridge.shared.open(connectionId: connectionId, paneId: paneId)
        }
    }
}
