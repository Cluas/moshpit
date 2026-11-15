import Foundation

/// The turns the host said ended — per connection and pane, when the newest
/// closing status the host pushed was sent, on the host's clock (B636).
///
/// A pushed `attention` and the `done` that ends its question can arrive in
/// either order: APNs reorders, and a relay can send an envelope it carried
/// once again (a sealed envelope is opened, not checked for freshness). An
/// `attention` that came after its own `done` was taken for a new question:
/// it rang "needs you" and stood as waiting for hours on a pane nobody
/// waited on. A `done` that came again cleared the question the pane asked
/// after it. Both are told by the host's clock: a question is its pane's
/// `since` (the episode), a closing status its send time (`ts`).
///
/// Shared with the Rust notification service (`crates/seal/src/closed.rs`):
/// `closed-turns.json` beside the standing set, `"<conn>|<pane>"` →
/// `{"closedAt": <unix seconds>, "recordedAt": <Date>}`, under the App
/// Group's file lock. What can't be read answers "nothing ended" and is
/// never written over; records expire with ``PushStanding/lifetime``.
public enum PushClosed {
    private static let filename = "closed-turns.json"

    /// The file the app and the notification service extension share; nil
    /// with no App Group container (an unsigned build).
    public static var appGroupFile: PushClosedFile? {
        // Beside the standing set, in the App Group's folder kept out of
        // backups (B785, ``DeviceLocalFiles``): the Rust receive path finds
        // it beside the standing file (`PushClosedFile::beside`), and a
        // phone restored from a backup starts with none of the old phone's
        // pane endings. A root copy an earlier build kept is deleted.
        DeviceLocalFiles.url(filename).map(PushClosedFile.init(url:))
    }

    /// How long the lock is waited for: what the file says refines a card,
    /// and the service has seconds in all. Past this it goes on as if
    /// nothing had ended — today's behaviour.
    public static let lockBudget: TimeInterval = 0.2

    /// ``PushClosedFile/noteClosed(conn:pane:closedAt:now:)`` on the shared
    /// file; false when its lock stayed busy past the budget.
    @discardableResult
    public static func noteClosed(conn: String, pane: String, closedAt: Int, now: Date = Date()) -> Bool {
        appGroupFile?.noteClosed(conn: conn, pane: pane, closedAt: closedAt, now: now) ?? true
    }

    /// ``PushClosedFile/closedAt(conn:pane:now:)`` on the shared file.
    public static func closedAt(conn: String, pane: String, now: Date = Date()) throws -> Int? {
        try appGroupFile?.closedAt(conn: conn, pane: pane, now: now)
    }
}

/// The endings file: a JSON object, `"<conn>|<pane>"` → the newest ending.
public struct PushClosedFile {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    public struct Closed: Codable, Equatable {
        /// When the host said the turn ended, on its clock (unix seconds).
        public var closedAt: Int
        /// When it was recorded here, for expiry.
        public var recordedAt: Date

        public init(closedAt: Int, recordedAt: Date) {
            self.closedAt = closedAt
            self.recordedAt = recordedAt
        }
    }

    /// The lock stayed taken past ``PushClosed/lockBudget``.
    public struct Busy: Error {}

    /// The host said `pane`'s turn ended at `closedAt`: remembered, unless a
    /// later ending already is. The file keeps one entry per pane, and every
    /// call drops what is past the lifetime, so it holds only the panes that
    /// ended lately. False when the lock stayed busy past the budget: not
    /// remembered.
    @discardableResult
    public func noteClosed(conn: String, pane: String, closedAt: Int, now: Date = Date()) -> Bool {
        let done: Void? = AppGroupFileLock.attempt(url, budget: PushClosed.lockBudget) {
            let key = "\(conn)|\(pane)"
            guard case .closed(let stored) = read() else { return }
            var all = stored.filter { live($0.value, now: now) }
            if (all[key]?.closedAt ?? .min) < closedAt {
                all[key] = Closed(closedAt: closedAt, recordedAt: now)
            }
            if all != stored { write(all) }
        }
        return done != nil
    }

    /// When the host last said `pane`'s turn ended, within the lifetime.
    /// Throws ``Busy`` when the lock stayed taken past the budget: the caller
    /// goes on as if nothing had ended — today's behaviour.
    public func closedAt(conn: String, pane: String, now: Date = Date()) throws -> Int? {
        let found: Int?? = AppGroupFileLock.attempt(url, budget: PushClosed.lockBudget) {
            guard case .closed(let all) = read(), let closed = all["\(conn)|\(pane)"],
                  live(closed, now: now) else { return nil }
            return closed.closedAt
        }
        guard let found else { throw Busy() }
        return found
    }

    private func live(_ closed: Closed, now: Date) -> Bool {
        now.timeIntervalSince(closed.recordedAt) < PushStanding.lifetime
            && closed.recordedAt < now.addingTimeInterval(60)   // clock jumped? drop it
    }

    private enum Contents {
        case closed([String: Closed])
        case unreadable
    }

    private func read() -> Contents {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            guard FileManager.default.fileExists(atPath: url.path) else { return .closed([:]) }
            Log.push.error("turn endings present but unreadable: \(error.localizedDescription, privacy: .public)")
            return .unreadable
        }
        // A shape this build doesn't know is a memory of endings, nothing
        // more: start it over.
        return .closed((try? JSONDecoder().decode([String: Closed].self, from: data)) ?? [:])
    }

    private func write(_ all: [String: Closed]) {
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(),
                                                 withIntermediateDirectories: true)
        do {
            try JSONEncoder().encode(all).write(to: url, options: .atomic)
        } catch {
            Log.push.error("could not save the turn endings: \(error.localizedDescription, privacy: .public)")
        }
    }
}
