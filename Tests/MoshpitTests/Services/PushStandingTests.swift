import Foundation
import Testing
import UserNotifications
@testable import Moshpit
import MoshpitKit

/// The 0→1 edge: the one attention event allowed to make a sound.
///
/// Everything in the quiet-notifications redesign hangs off this set agreeing,
/// across the app process and the notification service extension, about a
/// single question — is anyone already waiting? Get the edge wrong in one
/// direction and the phone rings for every agent that joins a wait (the
/// complaint that started this); wrong in the other and a real first prompt
/// arrives silently.
@Suite("Standing attention set", .serialized, .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
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

    /// Run `body` against the App Group's real standing file, put back byte
    /// for byte afterwards (it is the installed app's too), along with any
    /// copies the store made of it.
    private func withRawStore(_ body: (URL) throws -> Void) throws {
        let container = try #require(FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: PushPairingStore.appGroup))
        let url = container.appendingPathComponent("standing-attention.json")
        let manager = FileManager.default
        let siblings = Set((try? manager.contentsOfDirectory(atPath: container.path)) ?? [])
        let saved = try? Data(contentsOf: url)
        defer {
            if let saved { try? saved.write(to: url, options: .atomic) } else { try? manager.removeItem(at: url) }
            for name in (try? manager.contentsOfDirectory(atPath: container.path)) ?? []
            where !siblings.contains(name) && name.hasPrefix("standing-attention.json.") {
                try? manager.removeItem(at: container.appendingPathComponent(name))
            }
        }
        try body(url)
    }

    /// The set used to decode in one piece: one entry this build couldn't
    /// read made it "nobody is waiting", and the next prompt wrote that over
    /// every connection's standing prompts.
    @Test("an entry this build can't decode neither wipes the set nor fakes an edge")
    func undecodableEntryKeepsTheSet() throws {
        try withRawStore { url in
            let other = "66666666-7777-8888-9999-000000000000"
            let waiting = try PushStandingFileTests.stored(entry("%2"))
            let foreign = PushStandingFileTests.foreign(pane: "%3", at: Date())
            try JSONSerialization.data(withJSONObject: [Self.conn: [waiting, foreign], other: [waiting]])
                .write(to: url)

            #expect(!PushStanding.noteStanding(conn: Self.conn, entry: entry("%1")),
                    "%2 was already waiting: the next prompt is not the moment for a sound")

            #expect(PushStanding.standing(conn: other).map(\.pane) == ["%2"],
                    "recording a prompt on one connection wiped another's")
            #expect(Set(PushStanding.standing(conn: Self.conn).map(\.pane)) == ["%1", "%2"])
            #expect(try PushStandingFileTests.storedPanes(at: url, conn: Self.conn).contains("%3"),
                    "the entry this build can't read was dropped")
        }
    }
}

/// The standing file read an entry at a time — the B23 rule for the
/// pairings, applied to the set that decides which notification may ring.
@Suite("Standing file — what this build can't read, it keeps")
struct PushStandingFileTests {

    static let conn = "11111111-2222-3333-4444-555555555555"
    static let other = "66666666-7777-8888-9999-000000000000"

    private func makeFile() throws -> PushStandingFile {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("standing-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return PushStandingFile(url: dir.appendingPathComponent("standing-attention.json"))
    }

    private func cleanUp(_ file: PushStandingFile) {
        try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent())
    }

    private func entry(_ pane: String, since: Int = 1_756_000_000, at: Date = Date()) -> PushStanding.Entry {
        .init(pane: pane, agent: "claude", title: "may I", location: "m1 · work", since: since, recordedAt: at)
    }

    /// B727: an entry 1.0.8 recorded says nothing of how long it stands, and
    /// keeps that build's fifteen minutes: one twenty minutes old was
    /// answered at the desk long since and must not hold the edge for six
    /// hours. One this build records stands six hours (B660) and says so.
    /// The Rust set's `an_entry_1_0_8_recorded_keeps_its_fifteen_minutes`.
    @Test("an entry 1.0.8 recorded keeps its fifteen minutes")
    func olderEntryKeepsFifteenMinutes() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        // As 1.0.8's PushStanding wrote one (JSONEncoder: the date as
        // seconds since 2001, a nil field left out), `minutes` old.
        func older(_ pane: String, minutes: Double) -> String {
            let at = now.addingTimeInterval(-minutes * 60).timeIntervalSinceReferenceDate
            return #"{"pane":"\#(pane)","agent":"claude","title":"may I","since":1789990000,"recordedAt":\#(at)}"#
        }
        let text = #"{"\#(Self.conn)":[\#(older("%1", minutes: 20))],"\#(Self.other)":[\#(older("%2", minutes: 10))]}"#
        try Data(text.utf8).write(to: file.url)
        #expect(file.standing(conn: Self.conn, now: now).isEmpty, "20 minutes old: past 1.0.8's fifteen")
        #expect(file.standing(conn: Self.other, now: now).count == 1, "10 minutes old")
        #expect(file.noteStanding(conn: Self.conn, entry: entry("%3", since: 1, at: now), now: now),
                "nobody waits on conn: the next prompt rings")
        #expect(!file.noteStanding(conn: Self.other, entry: entry("%4", since: 1, at: now), now: now),
                "%2 still waits")
        let later = now.addingTimeInterval(PushStanding.lifetime - 60)
        #expect(file.standing(conn: Self.conn, now: later).map(\.pane) == ["%3"])
        #expect(file.standing(conn: Self.other, now: later).map(\.pane) == ["%4"], "1.0.8's %2 is gone")
        #expect(file.standing(conn: Self.conn, now: now.addingTimeInterval(PushStanding.lifetime + 1)).isEmpty)
        let stored = try JSONSerialization.jsonObject(with: Data(contentsOf: file.url)) as? [String: Any]
        let first = (stored?[Self.conn] as? [[String: Any]])?.first
        #expect(first?["lifetime"] as? Double == PushStanding.lifetime, "the file says how long it stands")
    }

    /// B347: the app and the notification service write the file at the same
    /// time. One writer notes and at once clears its prompts, the other notes
    /// its own, each through its own open: nothing either wrote is lost, and
    /// nothing cleared comes back. Without the lock the later writer wrote
    /// back what it had read (measured on a Mac: 56–91 of 150 notes lost, 3–5
    /// cleared prompts back).
    @Test("two writers at once lose nothing")
    func twoWritersAtOnce() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let url = file.url, conn = Self.conn, other = Self.other
        let rounds = 150
        let entry = { (pane: String) in
            PushStanding.Entry(pane: pane, agent: "claude", title: "may I", location: "m1 · work",
                               since: 1_756_000_000, recordedAt: Date())
        }
        let group = DispatchGroup()
        DispatchQueue.global().async(group: group) {
            let answering = PushStandingFile(url: url)
            for i in 0..<rounds {
                answering.noteStanding(conn: conn, entry: entry("%a\(i)"))
                answering.clear(conn: conn, pane: "%a\(i)")
            }
        }
        DispatchQueue.global().async(group: group) {
            let asking = PushStandingFile(url: url)
            for i in 0..<rounds {
                asking.noteStanding(conn: other, entry: entry("%b\(i)"))
            }
        }
        group.wait()
        let kept = file.standing(conn: other).count
        let back = file.standing(conn: conn).map(\.pane)
        #expect(kept == rounds, "a prompt noted at the same time as another write was lost")
        #expect(back.isEmpty, "cleared prompts came back: \(back)")
    }

    /// An entry as the store writes it.
    static func stored(_ entry: PushStanding.Entry) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(entry))
    }

    /// An entry in a shape this build can't decode — `since` as a string, as
    /// a build with another idea of it would write — with the `pane` and
    /// `recordedAt` this one writes.
    static func foreign(pane: String, at date: Date) -> [String: Any] {
        ["pane": pane, "since": "1756000000-2", "recordedAt": date.timeIntervalSinceReferenceDate]
    }

    /// The panes stored for `conn`, read as raw JSON — entries no build
    /// decodes are counted too.
    static func storedPanes(at url: URL, conn: String) throws -> [String] {
        let map = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any] ?? [:]
        return (map[conn] as? [[String: Any]] ?? []).compactMap { $0["pane"] as? String }
    }

    @Test("Recording a prompt keeps an entry this build can't decode, and every other connection's")
    func noteKeepsWhatItCantRead() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        try JSONSerialization.data(withJSONObject: [
            Self.conn: [try Self.stored(entry("%2")), Self.foreign(pane: "%3", at: Date())],
            Self.other: [try Self.stored(entry("%5"))],
        ]).write(to: file.url)
        #expect(file.read()[Self.conn]?.map(\.pane) == ["%2"], "one bad entry hid the whole set")

        #expect(!file.noteStanding(conn: Self.conn, entry: entry("%1")),
                "%2 was already waiting: not an edge")

        #expect(Set(try Self.storedPanes(at: file.url, conn: Self.conn)) == ["%1", "%2", "%3"])
        #expect(try Self.storedPanes(at: file.url, conn: Self.other) == ["%5"])
        #expect(file.read()[Self.other]?.map(\.pane) == ["%5"])
    }

    @Test("Clearing a pane removes its entry even in a shape this build can't decode")
    func clearRemovesTheCarriedPane() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        try JSONSerialization.data(withJSONObject: [
            Self.conn: [try Self.stored(entry("%2")), Self.foreign(pane: "%3", at: Date())],
        ]).write(to: file.url)

        #expect(file.clear(conn: Self.conn, pane: "%3").map(\.pane) == ["%2"])
        #expect(try Self.storedPanes(at: file.url, conn: Self.conn) == ["%2"])

        // And an entry nobody can read that has plainly expired goes too.
        try JSONSerialization.data(withJSONObject: [
            Self.conn: [Self.foreign(pane: "%4", at: Date().addingTimeInterval(-PushStanding.lifetime - 60))],
        ]).write(to: file.url)
        _ = file.noteStanding(conn: Self.conn, entry: entry("%1"))
        #expect(try Self.storedPanes(at: file.url, conn: Self.conn) == ["%1"])
    }

    @Test("A file that isn't a map at all is copied untouched before the first prompt is written")
    func garbageIsBackedUpFirst() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let garbage = Data(#"["standing", "in a shape nobody here knows"]"#.utf8)
        try garbage.write(to: file.url)
        #expect(file.read().isEmpty)

        _ = file.noteStanding(conn: Self.conn, entry: entry("%1"))

        #expect(try Data(contentsOf: file.backupURL) == garbage, "the only copy of the old set was written over")
        #expect(file.read()[Self.conn]?.map(\.pane) == ["%1"])
    }

    @Test("A file that is there but can't be read is never written over")
    func unreadableFileIsNotWritten() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        _ = file.noteStanding(conn: Self.conn, entry: entry("%1"))
        let before = try Data(contentsOf: file.url)
        // Stands in for a read before the first unlock, or an I/O error.
        try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: file.url.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.url.path) }
        try #require((try? Data(contentsOf: file.url)) == nil, "this filesystem still lets the owner read a 000 file")

        _ = file.noteStanding(conn: Self.other, entry: entry("%2"))
        _ = file.clear(conn: Self.conn, pane: "%1")
        file.clearAll(conn: Self.conn)

        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.url.path)
        #expect(try Data(contentsOf: file.url) == before)
    }

    /// B636: a `done` that comes again (a relay can send an envelope twice)
    /// clears only the question its pane asked up to its own time — the one
    /// asked after it stands. It used to clear that one too.
    @Test("a done clears what was asked up to it, not after")
    func clearEndedLeavesALaterQuestion() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        _ = file.noteStanding(conn: Self.conn, entry: entry("%3", since: 1_790_000_700))
        let left = file.clearEnded(conn: Self.conn, pane: "%3", endedAt: 1_790_000_600)
        #expect(left.map(\.pane) == ["%3"])
        #expect(file.clearEnded(conn: Self.conn, pane: "%3", endedAt: 1_790_000_800).isEmpty)
    }

    private func makeClosedFile() throws -> PushClosedFile {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("closed-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return PushClosedFile(url: dir.appendingPathComponent("closed-turns.json"))
    }

    private func storedKeys(_ file: PushClosedFile) throws -> [String] {
        let data = try Data(contentsOf: file.url)
        let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        return object.keys.sorted()
    }

    /// B636: the newest ending the host pushed for a pane is kept — an older
    /// one arriving after doesn't move it back — and forgotten with the
    /// standing set's lifetime.
    @Test("the newest ending of a pane is kept, and expires")
    func newestEndingKept() throws {
        let file = try makeClosedFile()
        defer { try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent()) }
        let now = Date()
        #expect(try file.closedAt(conn: Self.conn, pane: "%3", now: now) == nil)
        file.noteClosed(conn: Self.conn, pane: "%3", closedAt: 1_790_000_600, now: now)
        file.noteClosed(conn: Self.conn, pane: "%3", closedAt: 1_790_000_500, now: now)
        #expect(try file.closedAt(conn: Self.conn, pane: "%3", now: now) == 1_790_000_600)
        #expect(try file.closedAt(conn: Self.conn, pane: "%4", now: now) == nil)
        #expect(try file.closedAt(conn: Self.conn, pane: "%3",
                                  now: now.addingTimeInterval(PushStanding.lifetime + 1)) == nil)
    }

    /// B636: one entry per pane, and every write drops what is past the
    /// lifetime — the file can't grow with every pane ever seen.
    @Test("every write keeps one entry per pane and drops the expired")
    func endingsArePruned() throws {
        let file = try makeClosedFile()
        defer { try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent()) }
        let now = Date()
        file.noteClosed(conn: Self.conn, pane: "%1", closedAt: 100, now: now)
        file.noteClosed(conn: Self.conn, pane: "%1", closedAt: 200, now: now)
        #expect(try storedKeys(file) == ["\(Self.conn)|%1"])
        let later = now.addingTimeInterval(PushStanding.lifetime + 1)
        file.noteClosed(conn: Self.conn, pane: "%2", closedAt: 300, now: later)
        #expect(try storedKeys(file) == ["\(Self.conn)|%2"])
    }

    /// B636: a file a thousand stale entries long comes back to what is live
    /// on the next write.
    @Test("a thousand stale endings go on the next write")
    func thousandStaleEndings() throws {
        let file = try makeClosedFile()
        defer { try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent()) }
        let now = Date()
        let old = now.addingTimeInterval(-PushStanding.lifetime - 10)
        var stale: [String: PushClosedFile.Closed] = [:]
        for i in 0..<1000 { stale["\(Self.conn)|%\(i)"] = .init(closedAt: i, recordedAt: old) }
        try JSONEncoder().encode(stale).write(to: file.url)
        #expect(try file.closedAt(conn: Self.conn, pane: "%5", now: now) == nil)
        file.noteClosed(conn: Self.conn, pane: "%7", closedAt: 1_790_000_000, now: now)
        #expect(try storedKeys(file) == ["\(Self.conn)|%7"])
    }

    /// B636: the lock held by another past the budget fails open, in time:
    /// read as nothing ended, the ending not remembered — the notification
    /// never waits on it.
    @Test("a busy lock fails open within the budget")
    func busyLockFailsOpen() throws {
        let file = try makeClosedFile()
        defer { try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent()) }
        let fd = open(file.url.appendingPathExtension("lock").path, O_CREAT | O_RDWR, 0o600)
        #expect(fd >= 0)
        defer { close(fd) }
        #expect(flock(fd, LOCK_EX) == 0)
        defer { flock(fd, LOCK_UN) }
        let started = Date()
        #expect(throws: PushClosedFile.Busy.self) {
            try file.closedAt(conn: Self.conn, pane: "%3")
        }
        #expect(!file.noteClosed(conn: Self.conn, pane: "%3", closedAt: 1))
        #expect(Date().timeIntervalSince(started) < PushClosed.lockBudget * 4)
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
        #expect(update.title == "claude +2 needs you",
                "further prompts ride on the name as +N — one card for the whole wait")
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
