import UserNotifications

// In the extension target the shared push types are compiled straight into this
// module, so nothing needs importing. This same file is also compiled into
// MoshpitTests — the only place `didReceive` can be driven without an APNs key
// and a phone — and there those types arrive from the app module instead.
//
// The condition is an explicit flag on the test target, NOT `canImport(Moshpit)`:
// canImport actually attempts to load the module, which drags the app's whole
// transitive C-module graph (CNIOPosix, CCryptoBoringSSL, _AtomicsShims…) into an
// extension that links none of it, and the build fails resolving them.
#if MOSHPIT_TESTS
@testable import Moshpit
#endif

/// Notification service extension — the only Moshpit code that runs when a push
/// arrives and the app does not.
///
/// Its whole job is to open the sealed envelope the relay could not read and put
/// the real text on screen. iOS invokes it for any push carrying
/// `mutable-content: 1`, gives it a few seconds, and shows the unmodified
/// payload if it does not answer in time — which is why the relay ships a
/// translated generic fallback rather than an empty alert.
///
/// Failure here is quiet by design. A push whose MAC matches none of this
/// device's pairing secrets is not an error to report: it means the host that
/// sent it was paired to a different install, and the honest response is to show
/// the generic line rather than a decryption complaint.
final class NotificationService: UNNotificationServiceExtension {
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(_ request: UNNotificationRequest,
                            withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        self.contentHandler = contentHandler
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
            contentHandler(request.content)
            return
        }
        bestAttempt = content

        // Recorded where the APP can read it: this process's own log is
        // invisible to the diagnostics screen (see PushDiagnostics).
        PushDiagnostics.record("woke for a push")

        guard let envelope = PushRemoteNotification.envelope(in: request.content.userInfo) else {
            Log.push.info("push carries no sealed envelope — delivering as received")
            PushDiagnostics.record("no sealed envelope — delivered as received")
            contentHandler(content)
            return
        }
        let secrets = PushPairingStore.secretsNewestFirst()
        guard let status = PushRemoteNotification.open(envelope, secrets: secrets) else {
            // The count is the whole diagnosis: 0 means the store did not reach
            // the extension (App Group or file protection), while n > 0 means the
            // sending host is paired to a different install of the app. Those
            // have completely different fixes and look identical on screen.
            Log.push.error("no key opened this envelope (\(secrets.count, privacy: .public) available) — showing the generic fallback")
            // The number is the diagnosis: 0 means this process could not reach
            // the pairing store at all, anything else means the sending host is
            // paired to a different install.
            PushDiagnostics.record("no key opened this envelope (\(secrets.count) available) — showed the fallback")
            // And WITHOUT its action buttons. The category the relay set is
            // Allow/Deny/Reply, but those actions need the connection and pane
            // ids that only live inside the envelope we just failed to open — so
            // the app would have nowhere to send the keystroke, and the tap
            // would do nothing at all. A dead Allow is the worst outcome this
            // feature has: the user walks away believing they approved and the
            // agent is still waiting. Strip the actions and let the body tap
            // open the app, which is what the fallback text already asks for.
            content.categoryIdentifier = ""
            contentHandler(content)
            return
        }
        Log.push.info("opened a \(status.state, privacy: .public) push from \(status.host, privacy: .public)")
        PushDiagnostics.record("opened a \(status.state) push from \(status.host)"
                              + (status.agent == PushRemoteNotification.selfTestAgent
                                 ? " (self-test \(status.title ?? "?"))" : ""))
        // The standing store is what lets this process and the app agree on the
        // 0→1 edge — whichever of the push and the local announcement lands
        // first rings; the other sees "already standing" and stays silent.
        if status.state == "attention" {
            let edge = PushStanding.noteStanding(
                conn: status.conn,
                entry: .init(pane: status.pane, agent: status.agent,
                             title: status.title, since: status.ts,
                             recordedAt: Date()))
            let count = PushStanding.standing(conn: status.conn).count
            PushRemoteNotification.apply(status, to: content,
                                         attentionEdge: edge, standingCount: count)
        } else {
            // A `done` closes its pane's turn — that pane is no longer waiting.
            PushStanding.clear(conn: status.conn, pane: status.pane)
            PushRemoteNotification.apply(status, to: content)
        }
        contentHandler(content)
    }

    override func serviceExtensionTimeWillExpire() {
        // The hardest failure for this process to account for, because from the
        // outside it looks identical to a push that simply showed the fallback.
        PushDiagnostics.record("ran out of time — delivered whatever was ready")
        if let contentHandler, let bestAttempt {
            contentHandler(bestAttempt)
        }
    }
}
