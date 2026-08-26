import Foundation
import UserNotifications

/// Turning a sealed push into the notification the user sees.
///
/// Shared by the app and the notification service extension, which is the only
/// process awake when a push lands. Kept free of any English sentence on
/// purpose: `String(localized:)` resolves against `Bundle.main`, which inside an
/// extension is the EXTENSION's bundle — so a sentence composed here would
/// silently ship untranslated. Everything shown is either data the agent sent or
/// a glyph, and the two generic lines that need real prose stay where they can be
/// translated: the relay's `title-loc-key`/`loc-key` fallback, resolved by iOS
/// against the app's own catalog.
enum PushRemoteNotification {
    /// Payload key holding the sealed envelope. Matches the relay's `mp`.
    static let envelopeKey = "mp"

    /// Decrypted fields copied into `userInfo` alongside the two routing keys.
    ///
    /// The app cannot re-open the envelope — by the time a notification reaches
    /// it, only the extension had the key in hand — so anything the app needs to
    /// reason about has to be carried across in the clear, inside the
    /// notification, on the device. Two things need it: matching a pairing
    /// self-test against the nonce it asked for, and knowing which agent and
    /// state a tapped notification came from without guessing from the title.
    /// Agent label a pairing self-test carries.
    ///
    /// Lives here rather than with the installer because it is a PUSH concept:
    /// it travels inside the sealed status, and both the extension and the
    /// notification delegate — neither of which links the install engine — have
    /// to recognise it. `AgentActivityMonitor` skips it so proving an install
    /// leaves no phantom agent on the island.
    static let selfTestAgent = "moshpit-selftest"

    static let agentKey = "moshpitAgent"
    static let stateKey = "moshpitState"
    static let detailKey = "moshpitDetail"

    /// Pull the envelope out of an APNs payload. Returns nil for any
    /// notification that is not one of ours — including a local one.
    static func envelope(in userInfo: [AnyHashable: Any]) -> PushSealedBox.Envelope? {
        guard let raw = userInfo[envelopeKey] as? [String: Any],
              let v = raw["v"] as? Int,
              let iv = raw["iv"] as? String,
              let ct = raw["ct"] as? String,
              let mac = raw["mac"] as? String
        else { return nil }
        return PushSealedBox.Envelope(v: v, iv: iv, ct: ct, mac: mac)
    }

    /// Try every secret this device holds until one authenticates.
    ///
    /// The extension cannot know which host sealed the envelope — that fact is
    /// inside it — so trying each is not a fallback but the design. Order is
    /// newest-first because a freshly paired host is the likeliest sender.
    /// A MAC failure is entirely normal here (it means "not this key"), so it is
    /// never surfaced; only running out of keys is a real failure.
    static func open(_ envelope: PushSealedBox.Envelope,
                     secrets: [String]) -> PushSealedBox.Status? {
        for secret in secrets {
            if let status = try? PushSealedBox.open(envelope, secretHex: secret) {
                return status
            }
        }
        return nil
    }

    /// Where this agent is, in the form the local notifications already use.
    ///
    /// A session name that is only digits is treated as no name. tmux calls its
    /// first session `0`, so the honest-looking `host · session` rendering came
    /// out as "mac-mini.lan · 0" on a real lock screen — which reads as a broken
    /// counter, not a location. Technically correct, which is why no test caught
    /// it and why it took looking at a real notification to see.
    ///
    /// The trade: someone who deliberately names a session `7` loses that from
    /// the notification. A bare number carries no information about where you
    /// are either way, so this is the better of the two.
    static func location(_ status: PushSealedBox.Status) -> String {
        let session = status.sess?.trimmingCharacters(in: .whitespaces) ?? ""
        guard !session.isEmpty, !session.allSatisfy(\.isNumber) else { return status.host }
        return "\(status.host) · \(session)"
    }

    /// Rewrite a notification with the decrypted content.
    ///
    /// The `userInfo` merge is what makes a pushed notification interchangeable
    /// with a local one: `AgentNotificationHandler` reads exactly these two keys
    /// to route a lock-screen Allow/Deny to a live pane, so once they are present
    /// the entire T1 control surface works on the push path with no new code.
    /// How old a status may be and still be worth acting on.
    ///
    /// Currently unused for gating, because a pushed notification carries no
    /// actions at all (see `apply`). Kept, with its reasoning, because it is the
    /// rule a real reply path will need on day one: `ts` is the only timestamp
    /// inside the sealed envelope, so it is the only one a compromised relay
    /// cannot move.
    ///
    /// The relay sets `apns-expiration` for this — 10 minutes for an attention,
    /// an hour for a done — but the relay is the party this design does not
    /// trust: a compromised one can hold an envelope and replay it whenever it
    /// likes, and a phone that was off has no record of whether the prompt still
    /// exists. `ts` is inside the sealed envelope, so it is the one timestamp an
    /// attacker cannot move; checking it here puts the guarantee back on the
    /// device.
    ///
    /// Both states are checked, not just attention. `done`'s only action is
    /// Reply, which types the user's text into the pane — less dangerous than a
    /// blind Enter, because they are actively composing, but a replayed done
    /// still aims that text at whatever the pane holds NOW, and the hour that
    /// bounds it was otherwise enforced only by the untrusted party. An
    /// exemption here would have been an implicit one, which is worse than either
    /// answer.
    ///
    /// Both are generous against the relay's own limits, because host and phone
    /// clocks disagree and a false "stale" costs a real prompt its buttons.
    static func lifetime(forState state: String) -> TimeInterval {
        state == "done" ? 2 * 60 * 60 : 15 * 60
    }

    /// How long a finished turn must have RUN for its completion to make a
    /// sound. A three-minute build ending is worth a chime; a twenty-second
    /// answer is list-only. The turn length rides in `Status.dur`, computed on
    /// the host where both ends of the turn were stamped.
    static let doneSoundThreshold = 180

    /// Render a status into notification content.
    ///
    /// `attentionEdge` is the caller's answer to the one question that decides
    /// interruption: is this the moment "nobody is waiting" became "someone is
    /// waiting"? Only that edge rings and pierces Focus. Everything else — a
    /// second agent joining the wait, a re-render after one leaves — updates
    /// the summary silently at `.passive`. The caller reads the edge from
    /// ``PushStanding``, which both the app and the extension share precisely
    /// so the two paths cannot both claim it for the same prompt.
    ///
    /// `standingCount` is how many prompts are waiting AFTER this one; a count
    /// above one renders as a language-free "+N" suffix, because this function
    /// also runs in the notification service extension, whose bundle has no
    /// localization catalog — a sentence composed there ships untranslated.
    static func apply(_ status: PushSealedBox.Status,
                      to content: UNMutableNotificationContent,
                      attentionEdge: Bool = true,
                      standingCount: Int = 1,
                      now: Date = Date()) {
        let who = status.agent?.isEmpty == false ? status.agent! : status.host
        let place = location(status)
        let detail = status.title?.trimmingCharacters(in: .whitespacesAndNewlines)

        if status.state == "done" {
            content.title = "✓ \(who)"
            content.body = place
            // A finished SHORT turn is information, not an interruption: no
            // sound, `.passive` (the list, but no lit screen, no Focus breach).
            // A long turn — the user walked away from a build — earns `.active`
            // and the chime. `dur` is absent from older senders; absent reads
            // as short, because "quieter than intended" is the recoverable
            // direction.
            let isLong = (status.dur ?? 0) >= Self.doneSoundThreshold
            content.sound = isLong ? .default : nil
            content.interruptionLevel = isLong ? .active : .passive
        } else {
            // The newest question leads; further standing prompts are a count.
            content.title = standingCount > 1 ? "\(who) +\(standingCount - 1)" : who
            if let detail, !detail.isEmpty {
                content.body = "\(detail) — \(place)"
            } else {
                // No hook title — no jq on the host, or a bell-only signal. The
                // agent name and the location are still more use than the
                // relay's generic fallback line, so both slots take data.
                content.body = place
            }
            content.sound = attentionEdge ? .default : nil
            content.interruptionLevel = attentionEdge ? .timeSensitive : .passive
        }

        var info = content.userInfo
        info[AgentNotifications.connectionKey] = status.conn
        info[AgentNotifications.paneKey] = status.pane
        info[agentKey] = status.agent ?? ""
        info[stateKey] = status.state
        // The hook's title survives here even when the rendered body drops it
        // (a `done` shows the location instead). A pairing self-test carries its
        // nonce in exactly this field, and matching it is the only way the phone
        // can prove that THIS push — not a stale one from an earlier attempt —
        // arrived.
        info[detailKey] = detail ?? ""
        content.userInfo = info


        // No category, ever. A pushed notification carries no buttons because
        // Moshpit's notifications carry none at all any more — see
        // AgentNotifications. What it does carry is the two ids below, already
        // set above, which is what lets a tap land on the right pane.
        content.categoryIdentifier = ""
    }
}
