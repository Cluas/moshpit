import Foundation
import UserNotifications

/// Turning a sealed push into the notification the user sees.
///
/// Shared by the app and the notification service extension, which is the only
/// process awake when a push lands. The words themselves come from
/// ``AgentNotificationCopy`` — the same renderer the app uses for its local
/// cards, so a pushed finish and a local one read identically. Its sentences
/// are translated in BOTH processes because the extension ships a catalog of
/// its own (`Extensions/MoshpitPush/Localizable.xcstrings`, written by
/// `scripts/gen/gen_xcstrings.py` from the app's table); `String(localized:)`
/// resolves against `Bundle.main`, which inside an extension is the extension.
/// The relay's `title-loc-key`/`loc-key` fallback — what shows when no key
/// opens the envelope — is resolved by iOS against the app's catalog instead.
public enum PushRemoteNotification {
    /// Payload key holding the sealed envelope. Matches the relay's `mp`.
    public static let envelopeKey = "mp"

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
    public static let selfTestAgent = "moshpit-selftest"

    public static let agentKey = "moshpitAgent"
    public static let stateKey = "moshpitState"
    public static let detailKey = "moshpitDetail"

    /// Pull the envelope out of an APNs payload. Returns nil for any
    /// notification that is not one of ours — including a local one.
    public static func envelope(in userInfo: [AnyHashable: Any]) -> PushSealedBox.Envelope? {
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
    public static func open(_ envelope: PushSealedBox.Envelope,
                            secrets: [String]) -> PushSealedBox.Status? {
        for secret in secrets {
            if let status = try? PushSealedBox.open(envelope, secretHex: secret) {
                return status
            }
        }
        return nil
    }

    /// Where this agent is, in the form the local notifications use
    /// (``AgentNotificationCopy/place(label:session:)``). `label` is the name
    /// the user gave this connection on the phone, when the caller knows it —
    /// the extension reads it off the pairing the envelope's `conn` points at;
    /// the host's own name is the fallback. The envelope cannot carry the label:
    /// the host never learns what the phone calls it.
    public static func location(_ status: PushSealedBox.Status, label: String? = nil) -> String {
        let label = label?.trimmingCharacters(in: .whitespaces) ?? ""
        return AgentNotificationCopy.place(label: label.isEmpty ? status.host : label,
                                           session: status.sess)
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
    public static func lifetime(forState state: String) -> TimeInterval {
        state == "done" ? 2 * 60 * 60 : 15 * 60
    }

    /// How long a finished turn must have RUN for its completion to make a
    /// sound. A three-minute build ending is worth a chime; a twenty-second
    /// answer is list-only. The turn length rides in `Status.dur`, computed on
    /// the host where both ends of the turn were stamped.
    public static let doneSoundThreshold = 180

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
    /// `standingCount` is how many prompts are waiting INCLUDING this one; a
    /// count above one renders as "+N" on the agent's name, one card for the
    /// whole wait. `label` is what the phone calls this connection (see
    /// ``location(_:label:)``).
    public static func apply(_ status: PushSealedBox.Status,
                             to content: UNMutableNotificationContent,
                             attentionEdge: Bool = true,
                             standingCount: Int = 1,
                             prefs: PushPrefs.Values = .default,
                             label: String? = nil,
                             now: Date = Date()) {
        let who = status.agent?.isEmpty == false ? status.agent! : status.host
        let place = location(status, label: label)
        // "Show detail on lock screen" is a promise this renderer has to keep
        // for PUSHED notifications too: with it off, the body names where —
        // never what the agent is running, asking, or was asked to do. The
        // title (agent name) and the userInfo copy of the detail stay: the name
        // is the card's job, and userInfo is never rendered — the app reads it
        // to acknowledge prompts and match self-tests, so it carries the REAL
        // detail either way.
        let rawDetail = status.title?.trimmingCharacters(in: .whitespacesAndNewlines)
        let detail = prefs.showDetail ? rawDetail : nil

        if status.state == "done" {
            // On a `done` the hook's title is the prompt the turn answered
            // (`@moshpit_prompt` on the host), so the card says what finished.
            // `dur` is absent from older senders; absent reads as short, because
            // "quieter than intended" is the recoverable direction.
            AgentNotificationCopy.done(agent: who, detail: detail, place: place,
                                       duration: status.dur ?? 0, sound: prefs.sound,
                                       threshold: Self.doneSoundThreshold, into: content)
        } else {
            // No hook title — no jq on the host, a bell-only signal, or detail
            // hidden by the switch — leaves the body to the location alone,
            // which is still more use than the relay's generic fallback line.
            AgentNotificationCopy.attention(agent: who, standingCount: standingCount,
                                            detail: detail, place: place,
                                            edge: attentionEdge, sound: prefs.sound,
                                            into: content)
        }

        var info = content.userInfo
        info[AgentNotifications.connectionKey] = status.conn
        info[AgentNotifications.paneKey] = status.pane
        info[agentKey] = status.agent ?? ""
        info[stateKey] = status.state
        // The hook's title survives here even when the rendered body drops it
        // (Show detail off). A pairing self-test carries its nonce in exactly
        // this field, and matching it is the only way the phone can prove that
        // THIS push — not a stale one from an earlier attempt — arrived.
        info[detailKey] = rawDetail ?? ""
        content.userInfo = info

        // No category, ever. A pushed notification carries no buttons because
        // Moshpit's notifications carry none at all any more — see
        // AgentNotifications. What it does carry is the two ids below, already
        // set above, which is what lets a tap land on the right pane.
        content.categoryIdentifier = ""
    }
}
