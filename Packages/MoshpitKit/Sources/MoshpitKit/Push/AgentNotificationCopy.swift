import Foundation
import UserNotifications

/// The words on an agent notification — ONE renderer for both copies of every
/// card: the pushed one the notification service extension writes when a sealed
/// push lands, and the local one `AgentActivityMonitor` posts while the app is
/// awake.
///
/// Before this, each path composed its own strings, and the same event read
/// differently depending on how it reached the phone: a pushed finish was
/// "✓ claude / mac-mini.lan", the local twin "✓ claude finished / work · 0 ·
/// 14: bl". Neither said what had finished. Now both say the same thing:
///
///     ✓ claude finished · 12 min
///     Upload the build with the new Xcode — mac-mini · work
///
///     claude needs you
///     Bash: rm -rf build — mac-mini · work
///
/// Localization: `String(localized:)` resolves against `Bundle.main`, which is
/// the app inside the app and the extension inside the extension. Both bundles
/// carry the keys used here — `scripts/gen/gen_xcstrings.py` writes the
/// extension's catalog (`Extensions/MoshpitPush/Localizable.xcstrings`) from the
/// same table as the app's — so a sentence composed here is translated in either
/// process. Durations go through Foundation's own unit formatter, which needs no
/// catalog at all.
public enum AgentNotificationCopy {

    /// The longest detail (a prompt, a question, a tool line) a card shows.
    ///
    /// A lock screen shows about two lines of body, and the location has to fit
    /// after the detail. A prompt is the user's own text and can be a paragraph;
    /// the host already cuts it to 80 bytes before it travels, this is the
    /// character-aware cut that decides what is actually read.
    public static let detailLimit = 72

    /// A finished turn shorter than this shows no duration: "finished · 0 min"
    /// says nothing a bare "finished" does not.
    public static let durationFloor = 60

    /// The detail as a card shows it: whitespace runs (including the newlines a
    /// multi-line prompt has) collapsed to one space, trimmed, and cut to
    /// ``detailLimit`` characters with an ellipsis. nil when nothing is left.
    public static func detail(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let collapsed = raw.split(whereSeparator: { $0.isWhitespace || $0.isNewline })
            .joined(separator: " ")
        guard !collapsed.isEmpty else { return nil }
        guard collapsed.count > detailLimit else { return collapsed }
        return String(collapsed.prefix(detailLimit - 1)) + "…"
    }

    /// Where an agent is, as a card names it: `label · session`.
    ///
    /// The label is the name the user gave the connection when the phone knows
    /// it, the host name otherwise. The window is deliberately absent — on a
    /// lock screen it read as "0 · 14: bl", and the body now says WHAT instead
    /// of narrowing down WHERE.
    ///
    /// A session name that is only digits is treated as no name. tmux calls its
    /// first session `0`, so the honest-looking `host · session` rendering came
    /// out as "mac-mini.lan · 0" on a real lock screen — which reads as a broken
    /// counter, not a location. Technically correct, which is why no test caught
    /// it and why it took looking at a real notification to see. The trade:
    /// someone who deliberately names a session `7` loses that from the card. A
    /// bare number carries no information about where you are either way.
    public static func place(label: String, session: String?) -> String {
        let session = session?.trimmingCharacters(in: .whitespaces) ?? ""
        guard !session.isEmpty, !session.allSatisfy(\.isNumber) else { return label }
        return "\(label) · \(session)"
    }

    /// How long a turn ran, as a card shows it — "12 min", "1 hr, 5 min" — in
    /// the device's language via Foundation. nil under ``durationFloor``.
    /// Whole minutes only: a card is not a stopwatch.
    public static func duration(_ seconds: Int, locale: Locale = .current) -> String? {
        guard seconds >= durationFloor else { return nil }
        let minutes = Duration.seconds((seconds / 60) * 60)
        return minutes.formatted(
            .units(allowed: [.hours, .minutes], width: .abbreviated, maximumUnitCount: 2)
                .locale(locale))
    }

    /// The finished card. `duration` is how long the closing turn ran, in
    /// seconds, and decides two things: whether the title carries it, and — at
    /// `threshold` and above — whether the finish is worth a sound. A
    /// three-minute build ending is worth a chime; a twenty-second answer is
    /// information, not an interruption: silent, `.passive`, in the list for
    /// whenever the user next looks.
    public static func done(agent: String, detail: String?, place: String,
                            duration: Int, sound: Bool, threshold: Int,
                            into content: UNMutableNotificationContent) {
        var title = String(localized: "✓ \(agent) finished",
                           comment: "Notification title: an agent's turn ended. %@ is the agent's name.")
        if let ran = self.duration(duration) { title += " · \(ran)" }
        content.title = title
        content.body = body(detail: detail, place: place)
        let isLong = duration >= threshold
        content.sound = (isLong && sound) ? .default : nil
        content.interruptionLevel = isLong ? .active : .passive
    }

    /// The needs-you card. `standingCount` is how many prompts are waiting
    /// INCLUDING this one; above one it renders as a "+N" on the name, so one
    /// card carries the whole wait ("claude +2 needs you"). `edge` is whether
    /// this is the moment "nobody is waiting" became "someone is" — only that
    /// edge rings and may pierce Focus; everything else updates the card
    /// silently at `.passive`.
    public static func attention(agent: String, standingCount: Int, detail: String?,
                                 place: String, edge: Bool, sound: Bool,
                                 into content: UNMutableNotificationContent) {
        let who = standingCount > 1 ? "\(agent) +\(standingCount - 1)" : agent
        content.title = String(localized: "\(who) needs you",
                               comment: "Notification title: an agent stopped for the user. %@ is the agent's name, possibly with a +N count.")
        content.body = body(detail: detail, place: place)
        content.sound = (edge && sound) ? .default : nil
        content.interruptionLevel = edge ? .timeSensitive : .passive
    }

    /// "what — where", or just "where" when there is no what (no hook title,
    /// no jq on the host, or the Show detail switch is off — the caller passes
    /// nil for that, so this renderer never has to know about the setting).
    static func body(detail: String?, place: String) -> String {
        if let detail = self.detail(detail) { return "\(detail) — \(place)" }
        return place
    }
}
