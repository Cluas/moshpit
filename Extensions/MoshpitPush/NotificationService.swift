import UserNotifications
import MoshpitKit
import MoshpitPushFFI

/// Notification service extension — the only Moshpit code that runs when a push
/// arrives and the app does not.
///
/// Its whole job is to open the sealed envelope the relay could not read and put
/// the real text on screen. iOS invokes it for any push carrying
/// `mutable-content: 1`, gives it a few seconds, and shows the unmodified
/// payload if it does not answer in time — which is why the relay ships a
/// translated generic fallback rather than an empty alert.
///
/// The rules are the app's, once (E7b): one call into crates/seal
/// (`moshpit_push_receive`, crates/seal-ffi) opens the envelope with this
/// device's pairings, keeps the standing set, the rung finishes and the
/// closed turns, leaves the diagnostic lines, and says what to show. This
/// file reads what only it can (the switches and the language from the App
/// Group's defaults, the files' places) and maps the answer onto the
/// notification.
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
        let answer = Self.receive(request.content.userInfo, now: Date())
        Self.apply(answer, to: content)
        if let waiting = answer?["waiting"] as? [String: Any] { Self.update(waiting) }
        contentHandler(content)
    }

    override func serviceExtensionTimeWillExpire() {
        // The hardest failure for this process to account for, because from the
        // outside it looks identical to a push that simply showed the fallback.
        if let diagnostics = Self.paths()?.diagnostics {
            _ = Self.call(["note": "ran out of time — delivered whatever was ready",
                           "diagnostics": diagnostics.path])
        }
        if let contentHandler, let bestAttempt {
            contentHandler(bestAttempt)
        }
    }

    // MARK: - The call

    static let appGroup = AppGroup.id

    /// The files the call keeps: the pairings and the diagnostics ring at the
    /// container's root, the standing set and the rung finishes in its
    /// device-only folder (B785, out of backups; the closed turns go beside
    /// the standing set). Nil with no container (an unsigned build).
    static func paths() -> (pairings: URL, diagnostics: URL, standing: URL, finishes: URL)? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup),
              let standing = DeviceLocalFiles.url("standing-attention.json", appGroup: appGroup),
              let finishes = DeviceLocalFiles.url("rung-finishes.json", appGroup: appGroup)
        else { return nil }
        // The closed turns' folder is the standing set's: made ready the same way.
        _ = DeviceLocalFiles.url("closed-turns.json", appGroup: appGroup)
        return (container.appendingPathComponent("push-pairings.json"),
                container.appendingPathComponent("push-diagnostics.json"),
                standing, finishes)
    }

    /// One push through crates/seal: its answer (crates/seal-ffi's table), or
    /// nil when the call can't be made (no container) — then the push is
    /// shown as it came, with the user's sound switch kept.
    static func receive(_ userInfo: [AnyHashable: Any], now: Date,
                        defaults: UserDefaults? = UserDefaults(suiteName: appGroup)) -> [String: Any]? {
        guard let paths = paths() else { return nil }
        let info = userInfo.reduce(into: [String: Any]()) { out, pair in
            if let key = pair.key as? String { out[key] = pair.value }
        }
        return call([
            "userInfo": info,
            "pairings": paths.pairings.path,
            "diagnostics": paths.diagnostics.path,
            "standing": paths.standing.path,
            "finishes": paths.finishes.path,
            "now": now.timeIntervalSince1970,
            "sound": defaults?.object(forKey: "moshpit.push.sound") as? Bool ?? true,
            "showDetail": defaults?.object(forKey: "moshpit.push.showDetail") as? Bool ?? true,
            "language": AppLanguage.current(suiteName: appGroup),
        ])
    }

    /// `request` through `moshpit_push_receive`, its JSON answer decoded.
    static func call(_ request: [String: Any]) -> [String: Any]? {
        guard JSONSerialization.isValidJSONObject(request),
              let data = try? JSONSerialization.data(withJSONObject: request),
              let text = String(data: data, encoding: .utf8),
              let answer = text.withCString({ moshpit_push_receive($0) })
        else { return nil }
        defer { moshpit_push_free(answer) }
        guard let object = try? JSONSerialization.jsonObject(with: Data(String(cString: answer).utf8)),
              let dictionary = object as? [String: Any]
        else { return nil }
        return dictionary
    }

    // MARK: - The answer on screen

    /// `answer` onto `content`: a card's words, sound, interruption and
    /// routing keys; no category on a card or the fallback (Moshpit's
    /// notifications carry no action buttons, and the fallback's would need
    /// the pane the envelope hides). The user's sound switch holds on every
    /// path, the one that couldn't call included.
    static func apply(_ answer: [String: Any]?, to content: UNMutableNotificationContent) {
        let soundAllowed = answer?["soundAllowed"] as? Bool ?? true
        if !soundAllowed { content.sound = nil }
        switch answer?["delivery"] as? String {
        case "card":
            guard let card = answer?["card"] as? [String: Any] else { return }
            fill(content, with: card)
        case "fallback":
            content.categoryIdentifier = ""
        default:
            break
        }
    }

    /// `card` (crates/seal-ffi's) into `content`, its keys added to what the
    /// push carried.
    static func fill(_ content: UNMutableNotificationContent, with card: [String: Any]) {
        content.title = card["title"] as? String ?? content.title
        content.subtitle = card["subtitle"] as? String ?? content.subtitle
        content.body = card["body"] as? String ?? content.body
        content.sound = card["sound"] as? Bool == true ? .default : nil
        switch card["level"] as? String {
        case "timeSensitive": content.interruptionLevel = .timeSensitive
        case "active": content.interruptionLevel = .active
        default: content.interruptionLevel = .passive
        }
        var info = content.userInfo
        for (key, value) in card["userInfo"] as? [String: Any] ?? [:] { info[key] = value }
        content.userInfo = info
        content.categoryIdentifier = ""
    }

    /// The connection's waiting card after a finish left the set (B669):
    /// replaced in place, silently, under the identifier it was posted with,
    /// or withdrawn when nothing waits.
    static func update(_ waiting: [String: Any], center: UNUserNotificationCenter = .current()) {
        if let redraw = waiting["redraw"] as? [String: Any], let id = redraw["id"] as? String,
           let card = redraw["card"] as? [String: Any] {
            let content = UNMutableNotificationContent()
            fill(content, with: card)
            center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
        } else if let withdraw = waiting["withdraw"] as? [String: Any], let id = withdraw["id"] as? String {
            center.removeDeliveredNotifications(withIdentifiers: [id])
        }
    }
}
