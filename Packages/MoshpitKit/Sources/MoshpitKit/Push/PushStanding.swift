import Foundation

/// The set of prompts currently standing — who is waiting, per connection —
/// shared through the App Group so the APP and the NOTIFICATION SERVICE
/// EXTENSION make the same call on the one question that decides whether a
/// notification makes a sound:
///
///     is this the moment "nobody is waiting on you" became
///     "someone is waiting on you"?
///
/// That 0→1 edge is the only attention event worth interrupting a person for.
/// The second agent joining the wait changes a COUNT the user will discover
/// when they come back anyway; it rings no bell and pierces no Focus. Both
/// processes must agree on the edge or the user hears double — the push for a
/// prompt the local path already announced (or vice versa) has to see "already
/// standing" and stay silent.
///
/// Staleness is handled by expiry, not by trust. While the app runs, hook
/// polling removes entries the moment a pane leaves `attention`. While it does
/// NOT run, nothing on the phone learns that a prompt was answered at the desk
/// — the host only pushes on attention and done, and `done` closes a turn, not
/// a question. So entries expire after ``lifetime``: past it they neither count
/// toward the edge nor render in a summary. The worst staleness costs is a
/// silent update that should have rung; the worst trust would cost is a phone
/// that rings for a prompt answered an hour ago.
public enum PushStanding {

    public struct Entry: Codable, Equatable {
        public var pane: String
        public var agent: String?
        public var title: String?
        /// Where the pane lives ("host · sess · 1: win"), for re-rendering the
        /// summary after another pane leaves the wait.
        public var location: String?
        /// Episode start (the host's `@moshpit_since`) — the identity of the
        /// question, same as the announce-once record uses.
        public var since: Int
        /// When this entry was recorded on the phone, for expiry.
        public var recordedAt: Date

        public init(pane: String, agent: String? = nil, title: String? = nil, location: String? = nil,
                    since: Int, recordedAt: Date) {
            self.pane = pane; self.agent = agent; self.title = title; self.location = location
            self.since = since; self.recordedAt = recordedAt
        }
    }

    /// How long an unrefreshed entry keeps counting. Matches the attention
    /// push's own APNs expiry: past 15 minutes the prompt is stale enough that
    /// re-ringing for a "new" one is better than staying silent for it.
    public static let lifetime: TimeInterval = 15 * 60

    private static let filename = "standing-attention.json"

    /// The store's file — nil with no App Group container (an unsigned
    /// build), when nothing is stored and every read is empty.
    private static var file: PushStandingFile? { appGroupFile }

    /// The file the app and the notification service extension share, for
    /// a caller that holds the file itself (a test hands it another one).
    public static var appGroupFile: PushStandingFile? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: PushPairingStore.appGroup)
            .map { PushStandingFile(url: $0.appendingPathComponent(filename)) }
    }

    /// conn → entries. Pruned of expired entries on every read.
    public static func read(now: Date = Date()) -> [String: [Entry]] {
        file?.read(now: now) ?? [:]
    }

    public static func prune(_ raw: [String: [Entry]], now: Date) -> [String: [Entry]] {
        raw.compactMapValues { entries in
            let live = entries.filter { isLive(recordedAt: $0.recordedAt, now: now) }
            return live.isEmpty ? nil : live
        }
    }

    /// Whether something recorded at `recordedAt` still counts.
    static func isLive(recordedAt: Date, now: Date) -> Bool {
        now.timeIntervalSince(recordedAt) < lifetime
            && recordedAt < now.addingTimeInterval(60)   // clock jumped? drop it
    }

    /// Record a prompt as standing. Returns whether this was the 0→1 edge for
    /// its connection — the caller's licence to make a sound.
    ///
    /// Re-recording the same episode (same pane, same `since`) refreshes its
    /// expiry and is NEVER an edge: the push and the local announcement for one
    /// prompt both land here, and exactly one of them may ring.
    @discardableResult
    public static func noteStanding(conn: String, entry: Entry, now: Date = Date()) -> Bool {
        guard let file else { return true }   // nothing can be standing: an edge
        return file.noteStanding(conn: conn, entry: entry, now: now)
    }

    /// A pane stopped waiting (answered, died, moved on). Returns the entries
    /// still standing for that connection, so the caller can re-render or
    /// withdraw the summary.
    @discardableResult
    public static func clear(conn: String, pane: String, now: Date = Date()) -> [Entry] {
        file?.clear(conn: conn, pane: pane, now: now) ?? []
    }

    /// Everything standing for a connection, newest first.
    public static func standing(conn: String, now: Date = Date()) -> [Entry] {
        file?.standing(conn: conn, now: now) ?? []
    }

    public static func clearAll(conn: String) {
        file?.clearAll(conn: conn)
    }
}

/// The standing set's file: a JSON object, connection id → array of
/// ``PushStanding/Entry`` — the format every build and the notification
/// service extension read. ``PushStanding`` is this file at the App Group
/// path.
///
/// Read an entry at a time, and nothing it can't read is ever dropped — the
/// rule ``PushPairingFile`` keeps for the pairings. The map used to decode in
/// one piece, and anything that failed — one entry in a shape this build
/// doesn't know (a rollback to an older TestFlight build), one damaged byte,
/// a file that couldn't be opened — read as "nobody is waiting". The next
/// prompt or answer wrote that over the file: every other standing prompt,
/// on every connection, was gone, and the sound decisions after it were
/// wrong until the entries would have expired anyway. Now:
///  * entries this build decodes are the standing set;
///  * entries it can't are written back exactly as stored on every save —
///    unless the save replaces or clears that very pane, or they have
///    plainly expired; they never count toward the edge, since nothing here
///    can tell what they say;
///  * a file that holds anything this build couldn't decode is copied to
///    ``backupURL`` before the first write over it;
///  * a file that is there but can't be read is never written over.
/// Every read-modify-write holds the lock the finishes take
/// (``AppGroupFileLock``, B347): the app and the notification service both
/// write this file, and without it the later writer dropped the earlier
/// one's change.
public struct PushStandingFile {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    /// Where a file this build couldn't fully read is kept, untouched, before
    /// anything is written over it.
    public var backupURL: URL { url.appendingPathExtension("backup") }

    /// What one read of the file found.
    struct Contents {
        /// The entries this build decoded, unpruned.
        var entries: [String: [PushStanding.Entry]] = [:]
        /// Per connection, the entries it couldn't decode, as stored.
        var carried: [String: [Any]] = [:]
        /// Connections whose value isn't a list at all, as stored.
        var odd: [String: Any] = [:]
        /// The file's bytes; nil when there is no file.
        var raw: Data?
        /// False when the bytes aren't a JSON object — then none of them can
        /// be carried entry by entry.
        var isMap = true
        /// The file is there but couldn't be read. Nothing is known about
        /// what it holds.
        var unreadable = false

        /// Everything stored was read.
        var isComplete: Bool { carried.isEmpty && odd.isEmpty && isMap && !unreadable }
    }

    /// conn → the entries this build can read, pruned of expired ones.
    public func read(now: Date = Date()) -> [String: [PushStanding.Entry]] {
        PushStanding.prune(contents().entries, now: now)
    }

    /// ``PushStanding/standing(conn:now:)``, on this file.
    public func standing(conn: String, now: Date = Date()) -> [PushStanding.Entry] {
        (read(now: now)[conn] ?? []).sorted { $0.recordedAt > $1.recordedAt }
    }

    /// ``PushStanding/noteStanding(conn:entry:now:)``, on this file.
    @discardableResult
    public func noteStanding(conn: String, entry: PushStanding.Entry, now: Date = Date()) -> Bool {
        AppGroupFileLock.around(url) {
            let contents = contents()
            var all = PushStanding.prune(contents.entries, now: now)
            var entries = all[conn] ?? []
            // Nobody else waiting (B450): the pane's own earlier entry is the
            // question this one replaces, not another wait. The same
            // question — the pushed and the local copy name one `since` (the
            // host's `@moshpit_since`) — is a repeat.
            let wasEmpty = entries.allSatisfy { $0.pane == entry.pane }
            let isRepeat = entries.contains { $0.pane == entry.pane && $0.since == entry.since }
            entries.removeAll { $0.pane == entry.pane }
            entries.append(entry)
            all[conn] = entries
            var carried = contents.carried
            carried[conn]?.removeAll { Self.pane(of: $0) == entry.pane }
            var odd = contents.odd
            odd[conn] = nil
            save(all, carrying: carried, odd: odd, over: contents, now: now)
            return wasEmpty && !isRepeat
        }
    }

    /// ``PushStanding/clear(conn:pane:now:)``, on this file.
    @discardableResult
    public func clear(conn: String, pane: String, now: Date = Date()) -> [PushStanding.Entry] {
        AppGroupFileLock.around(url) {
            let contents = contents()
            var all = PushStanding.prune(contents.entries, now: now)
            var entries = all[conn] ?? []
            entries.removeAll { $0.pane == pane }
            all[conn] = entries.isEmpty ? nil : entries
            var carried = contents.carried
            carried[conn]?.removeAll { Self.pane(of: $0) == pane }
            save(all, carrying: carried, odd: contents.odd, over: contents, now: now)
            return entries
        }
    }

    /// ``PushStanding/clearAll(conn:)``, on this file.
    public func clearAll(conn: String, now: Date = Date()) {
        AppGroupFileLock.around(url) {
            let contents = contents()
            var all = PushStanding.prune(contents.entries, now: now)
            all[conn] = nil
            var carried = contents.carried
            carried[conn] = nil
            var odd = contents.odd
            odd[conn] = nil
            save(all, carrying: carried, odd: odd, over: contents, now: now)
        }
    }

    func contents() -> Contents {
        var contents = Contents()
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            guard FileManager.default.fileExists(atPath: url.path) else { return contents }  // nobody waiting yet
            contents.unreadable = true
            Log.push.error("standing store present but unreadable: \(error.localizedDescription, privacy: .public)")
            return contents
        }
        contents.raw = data
        guard let map = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            contents.isMap = false
            Log.push.error("standing store present but not a map (\(data.count, privacy: .public) bytes) — nothing read, file kept")
            return contents
        }
        let decoder = JSONDecoder()
        for (conn, value) in map {
            guard let list = value as? [Any] else {
                contents.odd[conn] = value
                Log.push.error("standing entries for a connection aren't a list — kept as stored")
                continue
            }
            for item in list {
                // Wrapped in a one-entry array: JSONSerialization won't write
                // a bare fragment.
                if let single = try? JSONSerialization.data(withJSONObject: [item]),
                   let entry = try? decoder.decode([PushStanding.Entry].self, from: single).first {
                    contents.entries[conn, default: []].append(entry)
                } else {
                    contents.carried[conn, default: []].append(item)
                    Log.push.error("standing entry undecodable — kept as stored")
                }
            }
        }
        return contents
    }

    /// The pane a carried entry names, if it names one in the shape this
    /// build writes.
    private static func pane(of item: Any) -> String? {
        (item as? [String: Any])?["pane"] as? String
    }

    /// A carried entry whose `recordedAt` reads, in the shape this build
    /// writes it, as past its lifetime — stale under any build's rule.
    private static func hasExpired(_ item: Any, now: Date) -> Bool {
        guard let seconds = (item as? [String: Any])?["recordedAt"] as? Double else { return false }
        return !PushStanding.isLive(recordedAt: Date(timeIntervalSinceReferenceDate: seconds), now: now)
    }

    /// Write `all`, then `carried` and `odd` as read, over the file
    /// `contents` was read from.
    @discardableResult
    private func save(_ all: [String: [PushStanding.Entry]],
                      carrying carried: [String: [Any]], odd: [String: Any],
                      over contents: Contents, now: Date) -> Bool {
        guard !contents.unreadable else {
            Log.push.error("not saving the standing set: the store couldn't be read, and a save now would replace what it holds")
            return false
        }
        guard secure(contents) else { return false }
        let carried = carried
            .mapValues { $0.filter { !Self.hasExpired($0, now: now) } }
            .filter { !$0.value.isEmpty }
        let data: Data
        do {
            let encoded = try JSONEncoder().encode(all)
            if carried.isEmpty && odd.isEmpty {
                data = encoded
            } else {
                var map = (try JSONSerialization.jsonObject(with: encoded)) as? [String: Any] ?? [:]
                for (conn, items) in carried {
                    map[conn] = ((map[conn] as? [Any]) ?? []) + items
                }
                for (conn, value) in odd where map[conn] == nil {
                    map[conn] = value
                }
                data = try JSONSerialization.data(withJSONObject: map)
            }
        } catch {
            Log.push.error("could not encode the standing set: \(error.localizedDescription, privacy: .public)")
            return false
        }
        return store(data, at: url, what: "the standing set")
    }

    /// Keep an untouched copy of a file this build couldn't fully read, and
    /// say whether a write over it is now safe. A map with entries skipped
    /// carries them through every save, so its first copy is all it needs; a
    /// file that is no map at all is the only copy of whatever it holds, so
    /// each distinct one is kept.
    private func secure(_ contents: Contents) -> Bool {
        guard !contents.isComplete, let raw = contents.raw else { return true }
        if (try? Data(contentsOf: backupURL)) == raw { return true }
        var target = backupURL
        if FileManager.default.fileExists(atPath: target.path) {
            if contents.isMap { return true }
            target = url.appendingPathExtension("backup-\(Int(Date().timeIntervalSince1970))")
        }
        guard store(raw, at: target, what: "a copy of the standing set"),
              (try? Data(contentsOf: target)) == raw else {
            Log.push.error("could not keep a copy of the standing set; not writing over it")
            return false
        }
        Log.push.notice("standing set as found copied to \(target.lastPathComponent, privacy: .public)")
        return true
    }

    private func store(_ data: Data, at target: URL, what: String) -> Bool {
        // The container directory is normally created by containerURL(...) —
        // but not on every path (an unsigned simulator test host reaches here
        // with no directory at all), and an atomic write into a missing
        // directory fails without a word. A store whose whole job is agreeing
        // on the 0→1 edge must not "write and hope".
        try? FileManager.default.createDirectory(at: target.deletingLastPathComponent(),
                                                 withIntermediateDirectories: true)
        do {
            try data.write(to: target, options: .atomic)
            return true
        } catch {
            Log.push.error("could not save \(what, privacy: .public): \(error.localizedDescription, privacy: .public)")
            return false
        }
    }
}
