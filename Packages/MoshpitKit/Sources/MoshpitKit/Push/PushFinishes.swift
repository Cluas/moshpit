import Foundation

/// The finishes that have rung — per connection and pane, the last turn
/// whose `done` made a sound — shared through the App Group, so the app's
/// local finish card and the pushed one for the same turn make one sound
/// between them.
///
/// A long turn ending (``PushRemoteNotification/doneSoundThreshold``) is the
/// one finish worth a chime, and it reaches a phone twice when the app is
/// awake: the host pushes it the moment the turn stops, and the app's hook
/// poll sees it a second or two later. Both copies share one identifier, so
/// the second replaces the first on screen — but each rang, two chimes for
/// one build. ``PushStanding`` gives attention one edge between the two
/// processes; this gives a finish one.
///
/// A turn is told by when it ended, on the host's clock: the push carries its
/// send time (`ts`), the app the pane's `@moshpit_since` for `done` — the same
/// moment, a second or so apart. Two finishes of one pane within ``sameTurn``
/// are one turn: a turn worth a sound runs minutes, so two of them cannot
/// end that close together. Records expire after ``lifetime``.
///
/// What is lost when this store can't be read or written is a second chime,
/// never a notification: a failure answers "ring".
public enum PushFinishes {
    /// Finishes this close together, on one pane, are one turn's.
    public static let sameTurn: TimeInterval = 60
    /// How long a ring is remembered.
    public static let lifetime: TimeInterval = 15 * 60

    private static let filename = "rung-finishes.json"

    /// The file the app and the notification service extension share, in
    /// the App Group's folder kept out of backups (B785, ``DeviceLocalFiles``);
    /// nil with no App Group container (an unsigned build).
    public static var appGroupFile: PushFinishesFile? {
        DeviceLocalFiles.url(filename).map(PushFinishesFile.init(url:))
    }

    /// ``PushFinishesFile/noteRing(conn:pane:endedAt:now:)`` on the shared file.
    @discardableResult
    public static func noteRing(conn: String, pane: String, endedAt: Int, now: Date = Date()) -> Bool {
        appGroupFile?.noteRing(conn: conn, pane: pane, endedAt: endedAt, now: now) ?? true
    }
}

/// The rings file: a JSON object, `"<conn>|<pane>"` → the last ring.
public struct PushFinishesFile {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    struct Ring: Codable, Equatable {
        /// When the turn ended, on the host's clock (unix seconds).
        var endedAt: Int
        /// When the ring was recorded here, for expiry.
        var recordedAt: Date
    }

    /// The finish of `pane`'s turn ending at `endedAt` is about to ring:
    /// returns whether it is the first of its turn to — the caller's licence
    /// to make the sound — and remembers it either way.
    ///
    /// Read, decide and write under one lock, so the app and the extension
    /// can't both find the turn unrung.
    @discardableResult
    public func noteRing(conn: String, pane: String, endedAt: Int, now: Date = Date()) -> Bool {
        withLock {
            let key = "\(conn)|\(pane)"
            var rings: [String: Ring]
            switch read() {
            case .unreadable:
                // Can't tell, and must not write over what can't be read.
                return true
            case .rings(let stored):
                rings = stored.filter { live($0.value, now: now) }
            }
            if let last = rings[key],
               abs(TimeInterval(last.endedAt - endedAt)) <= PushFinishes.sameTurn {
                return false
            }
            rings[key] = Ring(endedAt: endedAt, recordedAt: now)
            write(rings)
            return true
        }
    }

    private func live(_ ring: Ring, now: Date) -> Bool {
        now.timeIntervalSince(ring.recordedAt) < PushFinishes.lifetime
            && ring.recordedAt < now.addingTimeInterval(60)   // clock jumped? drop it
    }

    private enum Contents {
        case rings([String: Ring])
        case unreadable
    }

    private func read() -> Contents {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            guard FileManager.default.fileExists(atPath: url.path) else { return .rings([:]) }
            Log.push.error("finished-turn rings present but unreadable: \(error.localizedDescription, privacy: .public)")
            return .unreadable
        }
        // A shape this build doesn't know is a memory of chimes, nothing
        // more: start it over.
        return .rings((try? JSONDecoder().decode([String: Ring].self, from: data)) ?? [:])
    }

    private func write(_ rings: [String: Ring]) {
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(),
                                                 withIntermediateDirectories: true)
        do {
            try JSONEncoder().encode(rings).write(to: url, options: .atomic)
        } catch {
            Log.push.error("could not save the finished-turn rings: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// The lock every read-modify-write of the App Group's files takes
    /// (``AppGroupFileLock``).
    private func withLock<T>(_ body: () -> T) -> T {
        AppGroupFileLock.around(url, body)
    }
}
