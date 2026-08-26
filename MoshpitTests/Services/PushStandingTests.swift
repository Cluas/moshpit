import Foundation
import Testing
import UserNotifications
@testable import Moshpit

/// The 0→1 edge: the one attention event allowed to make a sound.
///
/// Everything in the quiet-notifications redesign hangs off this set agreeing,
/// across the app process and the notification service extension, about a
/// single question — is anyone already waiting? Get the edge wrong in one
/// direction and the phone rings for every agent that joins a wait (the
/// complaint that started this); wrong in the other and a real first prompt
/// arrives silently.
@Suite("Standing attention set", .serialized)
struct PushStandingTests {

    static let conn = "11111111-2222-3333-4444-555555555555"

    private func fresh() { PushStanding.clearAll(conn: Self.conn) }
    private func entry(_ pane: String, since: Int = 1_756_000_000,
                       at: Date = Date()) -> PushStanding.Entry {
        .init(pane: pane, agent: "claude", title: "may I", location: "m1 · work",
              since: since, recordedAt: at)
    }

    @Test("the first waiting prompt is the edge; the second is not")
    func edgeSemantics() {
        fresh()
        #expect(PushStanding.noteStanding(conn: Self.conn, entry: entry("%1")),
                "an empty set gaining its first prompt IS the moment worth a sound")
        #expect(!PushStanding.noteStanding(conn: Self.conn, entry: entry("%2")),
                "a second agent joining the wait changes a count, not the user's situation")
        #expect(PushStanding.standing(conn: Self.conn).count == 2)
        fresh()
    }

    @Test("the push and the local path cannot both ring for one prompt")
    func sameEpisodeIsNeverASecondEdge() {
        fresh()
        // Same pane, same episode — the local announcement and the pushed copy
        // of the very same prompt, landing in either order.
        #expect(PushStanding.noteStanding(conn: Self.conn, entry: entry("%1", since: 100)))
        #expect(!PushStanding.noteStanding(conn: Self.conn, entry: entry("%1", since: 100)),
                "the second arrival of the SAME question must see 'already standing'")
        fresh()
    }

    @Test("answering everyone re-arms the edge")
    func edgeReArmsWhenEmpty() {
        fresh()
        _ = PushStanding.noteStanding(conn: Self.conn, entry: entry("%1"))
        _ = PushStanding.clear(conn: Self.conn, pane: "%1")
        #expect(PushStanding.noteStanding(conn: Self.conn, entry: entry("%1", since: 200)),
                "once nobody is waiting, the next prompt is a fresh interruption again")
        fresh()
    }

    @Test("clearing one pane reports who is still waiting")
    func clearReportsRemainder() {
        fresh()
        _ = PushStanding.noteStanding(conn: Self.conn, entry: entry("%1"))
        _ = PushStanding.noteStanding(conn: Self.conn, entry: entry("%2"))
        let left = PushStanding.clear(conn: Self.conn, pane: "%1")
        #expect(left.map(\.pane) == ["%2"])
        fresh()
    }

    @Test("stale entries stop counting, so an unanswered hour-old prompt cannot mute a new one")
    func expiry() {
        // Nothing on the phone learns that a prompt was answered at the desk
        // while the app was dead — the host pushes on attention and done, never
        // on "answered". Expiry is what keeps that blindness from accumulating.
        let old = Date().addingTimeInterval(-PushStanding.lifetime - 60)
        let raw = [Self.conn: [entry("%9", at: old)]]
        #expect(PushStanding.prune(raw, now: Date()).isEmpty)
        // And an entry stamped in the future is a moved clock, not a wait.
        let future = [Self.conn: [entry("%9", at: Date().addingTimeInterval(3600))]]
        #expect(PushStanding.prune(future, now: Date()).isEmpty)
    }
}

/// The rendering half: what edge and duration turn into on the lock screen.
@Suite("Quiet-notification rendering")
struct QuietRenderingTests {

    private func status(state: String, dur: Int? = nil) -> PushSealedBox.Status {
        PushSealedBox.Status(conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301",
                             host: "m1-pro", sess: "work", pane: "%3",
                             agent: "claude", state: state, title: "Bash: make",
                             ts: Int(Date().timeIntervalSince1970), dur: dur)
    }

    @Test("only the edge rings and pierces Focus; updates are passive and silent")
    func attentionTiers() {
        let edge = UNMutableNotificationContent()
        PushRemoteNotification.apply(status(state: "attention"), to: edge,
                                     attentionEdge: true, standingCount: 1)
        #expect(edge.sound != nil)
        #expect(edge.interruptionLevel == .timeSensitive)

        let update = UNMutableNotificationContent()
        PushRemoteNotification.apply(status(state: "attention"), to: update,
                                     attentionEdge: false, standingCount: 3)
        #expect(update.sound == nil, "a count changing is not worth a sound")
        #expect(update.interruptionLevel == .passive)
        #expect(update.title == "claude +2",
                "the summary is language-free (+N) because the extension has no catalog")
    }

    @Test("a finished short turn is information, not an interruption")
    func doneTiers() {
        let short = UNMutableNotificationContent()
        PushRemoteNotification.apply(status(state: "done", dur: 25), to: short)
        #expect(short.sound == nil)
        #expect(short.interruptionLevel == .passive)

        let long = UNMutableNotificationContent()
        PushRemoteNotification.apply(status(state: "done", dur: 240), to: long)
        #expect(long.sound != nil, "walking away from a 4-minute build deserves the chime")
        #expect(long.interruptionLevel == .active)

        // Older senders send no duration; quieter is the recoverable direction.
        let unknown = UNMutableNotificationContent()
        PushRemoteNotification.apply(status(state: "done"), to: unknown)
        #expect(unknown.sound == nil)
        #expect(unknown.interruptionLevel == .passive)
    }
}
