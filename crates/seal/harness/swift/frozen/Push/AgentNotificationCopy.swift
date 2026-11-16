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
/// Localization: in the language Moshpit shows (``AppLanguage``), looked up in
/// `Bundle.main` — the app inside the app, the extension inside the extension.
/// Both bundles carry the keys used here — `scripts/gen/gen_xcstrings.py`
/// writes the extension's catalog (`Extensions/MoshpitPush/Localizable.xcstrings`)
/// from the same table as the app's — so a card reads the same, in the same
/// language, whichever process wrote it. Durations go through Foundation's own
/// unit formatter, in that language's locale.
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

    /// Whether no card shows `scalar` (B639): the bidi controls — embeddings,
    /// overrides, isolates and the three marks — and the control characters
    /// that aren't whitespace. What a pane says is untrusted text, and a
    /// right-to-left override in a detail turned everything after it around on
    /// screen, the place too: a detail could end in a host name of its choosing
    /// and show it where the card names the host. Whitespace controls stay, to
    /// collapse to a space like any other. Listed by value, the same list as
    /// the Rust port's (`crates/seal/src/copy.rs`).
    static func isHidden(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x00...0x08, 0x0E...0x1F, 0x7F...0x84, 0x86...0x9F,
             0x061C, 0x200E, 0x200F, 0x202A...0x202E, 0x2066...0x2069:
            return true
        default:
            return false
        }
    }

    /// `raw` on one line as a card shows it: what no card shows dropped
    /// (``isHidden(_:)``), and whitespace runs (including the newlines a
    /// multi-line prompt has) collapsed to one space, and trimmed.
    static func oneLine(_ raw: String) -> String {
        var shown = String.UnicodeScalarView()
        shown.append(contentsOf: raw.unicodeScalars.lazy.filter { !isHidden($0) })
        return String(shown).split(whereSeparator: { $0.isWhitespace || $0.isNewline })
            .joined(separator: " ")
    }

    /// The detail as a card shows it: on one line (``oneLine(_:)``), and cut to
    /// ``detailLimit`` characters with an ellipsis. nil when nothing is left.
    public static func detail(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let collapsed = oneLine(raw)
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
    public static func duration(_ seconds: Int, locale: Locale = AppLanguage.locale()) -> String? {
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
    ///
    /// `rings` false: this turn's finish already rang — the other copy of it,
    /// pushed or local (``PushFinishes``) — and this one only updates the
    /// card, silent and `.passive`, however long the turn ran.
    public static func done(agent: String, detail: String?, place: String,
                            duration: Int, sound: Bool, threshold: Int,
                            rings: Bool = true,
                            into content: UNMutableNotificationContent) {
        let agent = oneLine(agent)
        var title = AppLanguage.string("✓ \(agent) finished",
                                       comment: "Notification title: an agent's turn ended. %@ is the agent's name.")
        if let ran = self.duration(duration) { title += " · \(ran)" }
        content.title = title
        content.body = body(detail: detail, place: place)
        let isLong = duration >= threshold && rings
        content.sound = (isLong && sound) ? .default : nil
        content.interruptionLevel = isLong ? .active : .passive
    }

    /// Whether a finish of a turn that ran `duration` seconds would wake the
    /// screen — `.active`, with the sound if the user allows one: the
    /// finishes ``PushFinishes`` shares between the pushed copy and the
    /// local one. It used to ask the sound switch too (B442): with the sound
    /// off the record was never consulted, and one turn woke the screen twice.
    public static func doneWakes(duration: Int, threshold: Int) -> Bool {
        duration >= threshold
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
        let agent = oneLine(agent)
        let who = standingCount > 1 ? "\(agent) +\(standingCount - 1)" : agent
        content.title = AppLanguage.string("\(who) needs you",
                                           comment: "Notification title: an agent stopped for the user. %@ is the agent's name, possibly with a +N count.")
        content.body = body(detail: detail, place: place)
        content.sound = (edge && sound) ? .default : nil
        content.interruptionLevel = edge ? .timeSensitive : .passive
    }

    /// "what — where", or just "where" when there is no what (no hook title,
    /// no jq on the host, or the Show detail switch is off — the caller passes
    /// nil for that, so this renderer never has to know about the setting).
    /// The place on one line too: a stored one may come from a session the
    /// host named.
    static func body(detail: String?, place: String) -> String {
        let place = oneLine(place)
        if let detail = self.detail(detail) { return "\(detail) — \(place)" }
        return place
    }
}
