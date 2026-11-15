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
        /// How long it stands, in seconds: ``PushStanding/lifetime``, set
        /// when this build records it (``PushStanding/noteStanding(conn:entry:now:)``).
        /// None on one 1.0.8 recorded, which keeps that build's
        /// ``PushStanding/olderLifetime`` (B727).
        public var lifetime: TimeInterval?

        public init(pane: String, agent: String? = nil, title: String? = nil, location: String? = nil,
                    since: Int, recordedAt: Date, lifetime: TimeInterval? = PushStanding.lifetime) {
            self.pane = pane; self.agent = agent; self.title = title; self.location = location
            self.since = since; self.recordedAt = recordedAt; self.lifetime = lifetime
        }

        /// Whether it still counts at `now`: younger than its own lifetime
        /// (never past this build's), and not from more than a minute in
        /// the future.
        public func isLive(now: Date) -> Bool {
            let lasts = min(lifetime ?? PushStanding.olderLifetime, PushStanding.lifetime)
            return now.timeIntervalSince(recordedAt) < lasts
                && recordedAt < now.addingTimeInterval(60)   // clock jumped? drop it
        }
    }

    /// How long an unrefreshed entry keeps counting: six hours (B660), the
    /// Rust set's `LIFETIME_SECONDS`. An entry stands until its question is
    /// answered or resolved, or its pane or agent ends; it used to expire at
    /// fifteen minutes, taking down a card for a question still waiting.
    public static let lifetime: TimeInterval = 6 * 60 * 60

    /// How long an entry 1.0.8 recorded stands — one without a
    /// ``Entry/lifetime``: that build's fifteen minutes (B727). It took a
    /// question unanswered that long for answered at the desk; read with
    /// six hours, one answered there while no app ran held the edge, and
    /// the next prompt came in silent.
    public static let olderLifetime: TimeInterval = 15 * 60

    private static let filename = "standing-attention.json"

    /// The store's file — nil with no App Group container (an unsigned
    /// build), when nothing is stored and every read is empty.
    private static var file: PushStandingFile? { appGroupFile }

    /// The file the app and the notification service extension share, for
    /// a caller that holds the file itself (a test hands it another one).
    /// In the App Group's folder kept out of backups (B785): who was
    /// waiting on the old phone is not who waits on a phone restored from
    /// its backup.
    public static var appGroupFile: PushStandingFile? {
        DeviceLocalFiles.url(filename).map(PushStandingFile.init(url:))
    }

    /// conn → entries. Pruned of expired entries on every read.
    public static func read(now: Date = Date()) -> [String: [Entry]] {
        file?.read(now: now) ?? [:]
    }

    public static func prune(_ raw: [String: [Entry]], now: Date) -> [String: [Entry]] {
        raw.compactMapValues { entries in
            let live = entries.filter { $0.isLive(now: now) }
            return live.isEmpty ? nil : live
        }
    }

    /// Whether something recorded at `recordedAt` still counts under the
    /// longest lifetime any build gives it: what an entry this build can't
    /// read is held to.
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

    /// The host said `pane`'s turn ended at `endedAt` (its clock, B636): the
    /// question it asked up to then stops waiting — not one the pane asked
    /// after, which a `done` that came again used to clear.
    @discardableResult
    public static func clearEnded(conn: String, pane: String, endedAt: Int, now: Date = Date()) -> [Entry] {
        file?.clearEnded(conn: conn, pane: pane, endedAt: endedAt, now: now) ?? []
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
        // Recorded by this build: it stands this build's lifetime (B727).
        var entry = entry
        entry.lifetime = PushStanding.lifetime
        return AppGroupFileLock.around(url) {
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

    /// ``PushStanding/clearEnded(conn:pane:endedAt:now:)``, on this file.
    @discardableResult
    public func clearEnded(conn: String, pane: String, endedAt: Int, now: Date = Date()) -> [PushStanding.Entry] {
        AppGroupFileLock.around(url) {
            let contents = contents()
            var all = PushStanding.prune(contents.entries, now: now)
            var entries = all[conn] ?? []
            let before = entries.count
            entries.removeAll { $0.pane == pane && $0.since <= endedAt }
            all[conn] = entries.isEmpty ? nil : entries
            var carried = contents.carried
            if entries.count != before {
                carried[conn]?.removeAll { Self.pane(of: $0) == pane }
            }
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

/// The App Group's folder for what holds of this device alone — the
/// standing set (``PushStanding``), the pane endings beside it
/// (``PushClosed``, B636), the finish records (``PushFinishes``), the
/// widget's snapshot (``AgentWidgetStore``) — kept out of backups
/// (B785). They rode a backup to a new phone: after a Quick Start, minutes
/// old, the old phone's standing entries held the 0→1 edge there for hours
/// (B727's six), and the widget showed the old phone's agents. A folder, not
/// each file: every save replaces its file (an atomic rename), and a file's
/// own exclusion went with the file it replaced.
///
/// What an earlier build kept at the container's root is deleted, not
/// carried in: on a phone restored from that build's backup it is the old
/// phone's, and on any other it is minutes of waiting state.
public enum DeviceLocalFiles {
    static let folderName = "device-only"

    private static let lock = NSLock()
    /// The files made ready this process (`<container>/<filename>`): the
    /// folder is set up and the root cleared once, not at every read.
    nonisolated(unsafe) private static var ready: Set<String> = []

    /// `filename` in the folder of the App Group's container; nil with no
    /// container (an unsigned build).
    public static func url(_ filename: String, appGroup: String = PushPairingStore.appGroup) -> URL? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
        else { return nil }
        let key = container.path + "/" + filename
        lock.lock()
        defer { lock.unlock() }
        if ready.contains(key) {
            return container.appendingPathComponent(folderName, isDirectory: true)
                .appendingPathComponent(filename)
        }
        guard let url = url(filename, in: container) else { return nil }
        ready.insert(key)
        return url
    }

    /// `filename` in the folder of `container`: the folder made if it isn't
    /// there and kept out of backups, the file of that name at the root
    /// removed. Nil when the folder can't be made.
    public static func url(_ filename: String, in container: URL) -> URL? {
        let manager = FileManager.default
        var folder = container.appendingPathComponent(folderName, isDirectory: true)
        do {
            try manager.createDirectory(at: folder, withIntermediateDirectories: true)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try folder.setResourceValues(values)
        } catch {
            Log.push.error("device-only folder: \(error.localizedDescription, privacy: .public)")
            return nil
        }
        // An earlier build's copy, and what it kept beside it (`.lock`,
        // `.backup`): the old phone's state on a restored one.
        let siblings = (try? manager.contentsOfDirectory(atPath: container.path)) ?? []
        for name in siblings where name == filename || name.hasPrefix(filename + ".") {
            try? manager.removeItem(at: container.appendingPathComponent(name))
        }
        return folder.appendingPathComponent(filename)
    }
}
