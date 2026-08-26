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
enum PushStanding {

    struct Entry: Codable, Equatable {
        var pane: String
        var agent: String?
        var title: String?
        /// Where the pane lives ("host · sess · 1: win"), for re-rendering the
        /// summary after another pane leaves the wait.
        var location: String? = nil
        /// Episode start (the host's `@moshpit_since`) — the identity of the
        /// question, same as the announce-once record uses.
        var since: Int
        /// When this entry was recorded on the phone, for expiry.
        var recordedAt: Date
    }

    /// How long an unrefreshed entry keeps counting. Matches the attention
    /// push's own APNs expiry: past 15 minutes the prompt is stale enough that
    /// re-ringing for a "new" one is better than staying silent for it.
    static let lifetime: TimeInterval = 15 * 60

    private static let filename = "standing-attention.json"

    private static var url: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: PushPairingStore.appGroup)?
            .appendingPathComponent(filename)
    }

    /// conn → entries. Pruned of expired entries on every read.
    static func read(now: Date = Date()) -> [String: [Entry]] {
        guard let url, let data = try? Data(contentsOf: url),
              let raw = try? JSONDecoder().decode([String: [Entry]].self, from: data)
        else { return [:] }
        return prune(raw, now: now)
    }

    static func prune(_ raw: [String: [Entry]], now: Date) -> [String: [Entry]] {
        raw.compactMapValues { entries in
            let live = entries.filter {
                now.timeIntervalSince($0.recordedAt) < lifetime
                    && $0.recordedAt < now.addingTimeInterval(60)   // clock jumped? drop it
            }
            return live.isEmpty ? nil : live
        }
    }

    private static func write(_ value: [String: [Entry]]) {
        guard let url, let data = try? JSONEncoder().encode(value) else { return }
        try? data.write(to: url, options: .atomic)
    }

    /// Record a prompt as standing. Returns whether this was the 0→1 edge for
    /// its connection — the caller's licence to make a sound.
    ///
    /// Re-recording the same episode (same pane, same `since`) refreshes its
    /// expiry and is NEVER an edge: the push and the local announcement for one
    /// prompt both land here, and exactly one of them may ring.
    @discardableResult
    static func noteStanding(conn: String, entry: Entry, now: Date = Date()) -> Bool {
        var all = read(now: now)
        var entries = all[conn] ?? []
        let wasEmpty = entries.isEmpty
        let isRepeat = entries.contains { $0.pane == entry.pane && $0.since == entry.since }
        entries.removeAll { $0.pane == entry.pane }
        entries.append(entry)
        all[conn] = entries
        write(all)
        return wasEmpty && !isRepeat
    }

    /// A pane stopped waiting (answered, died, moved on). Returns the entries
    /// still standing for that connection, so the caller can re-render or
    /// withdraw the summary.
    @discardableResult
    static func clear(conn: String, pane: String, now: Date = Date()) -> [Entry] {
        var all = read(now: now)
        var entries = all[conn] ?? []
        entries.removeAll { $0.pane == pane }
        if entries.isEmpty { all[conn] = nil } else { all[conn] = entries }
        write(all)
        return entries
    }

    /// Everything standing for a connection, newest first.
    static func standing(conn: String, now: Date = Date()) -> [Entry] {
        (read(now: now)[conn] ?? []).sorted { $0.recordedAt > $1.recordedAt }
    }

    static func clearAll(conn: String) {
        var all = read()
        all[conn] = nil
        write(all)
    }
}
