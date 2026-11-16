import Foundation
import OSLog
import MoshpitKit

/// The app's own recent log, read back through `OSLogStore` — what the
/// Recent Log screen shows and what a feedback mail can attach.
///
/// It exists because of how bugs get reported: from a phone, never plugged
/// into a Mac when the bug happens. The log has the answers in it (`layout:`
/// names every terminal resize with its before-and-after size, `health:` the
/// cause of every reconnect), and `OSLogStore(scope: .currentProcessIdentifier)`
/// lets the app read what it wrote itself, so the answer travels as a
/// screenshot instead. Nothing is collected, uploaded or persisted beyond
/// what the system already keeps — it is the same log `log stream` would
/// show, shown where the problem was seen.
enum DiagnosticsLog {
    struct Entry: Identifiable {
        let id = UUID()
        let date: Date
        let category: String
        let message: String
    }

    /// How far back to read. Long enough to cover "it just did the thing",
    /// short enough that the store answers quickly.
    static let window: TimeInterval = 30 * 60

    /// A fixed-format clock, so it is fixed: in `en_US_POSIX`. In the
    /// user's locale the system may rewrite the format — the 12/24-hour
    /// setting turns `HH` into `h a` in some regions, and some languages
    /// give other digits — so a copied line read differently on every
    /// phone (QA1480; B128). The time zone stays the phone's own.
    static let clock: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()

    /// What the Recent Log says when the log can't be read, in the user's
    /// language — the SwiftUI page and the NSH one both show it. It was an
    /// English string built in each page, shown as is (B127).
    static func readFailure(_ error: Error, bundle: Bundle = .main) -> String {
        String(format: bundle.localizedString(forKey: readFailureKey, value: nil, table: nil),
               error.localizedDescription)
    }

    /// ``readFailure(_:bundle:)``'s catalog key.
    static let readFailureKey = "Couldn't read the log: %@"

    static func line(_ entry: Entry) -> String {
        "\(clock.string(from: entry.date))  \(entry.category)  \(entry.message)"
    }

    /// Newest first. Off the main actor: the store walks the system's log
    /// archive and can take a beat on a device with a busy log.
    static func recent() async -> Result<[Entry], Error> {
        #if DEBUG
        if let seeded = await Seed.requested?.result() { return seeded }
        #endif
        return await Task.detached(priority: .userInitiated) {
            let cutoff = Date().addingTimeInterval(-window)
            // The notification service extension is a SEPARATE PROCESS, and
            // `.currentProcessIdentifier` cannot see it — iOS gives an app no way
            // to read another process's log. So the two lines that decide whether
            // a push was decrypted, or fell back and why, would never appear here
            // no matter how long someone scrolled. The extension leaves them in
            // the App Group instead; they get merged in.
            //
            // This is the whole point of the screen: a reviewer went looking for
            // exactly those lines, found nothing, and reasoned from the absence.
            // A diagnostic tool that is blind to the riskiest component misleads
            // better than it helps.
            // Its ring (crates/seal's diagnostics, E7b) at the container's root.
            let ring = FileManager.default
                .containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
                .appendingPathComponent("push-diagnostics.json")
            let source = pushDiagnosticsSource()
            let fromExtension = ring.map {
                pushDiagnosticsRecent(path: $0.path, since: cutoff.timeIntervalSince1970).map {
                    Entry(date: Date(timeIntervalSince1970: $0.at), category: source, message: $0.text)
                }
            } ?? []
            // The NSH failures of the launches before this one (B366): a page
            // that came up as its bar alone, then a restart, took them out
            // of this process's log. This launch's own are in it already.
            let fromEarlierRuns = NSHDiagnostics.trail
                .earlier(since: Date().addingTimeInterval(-NSHDiagnostics.earlierWindow))
                .map { Entry(date: $0.at, category: NSHDiagnostics.source, message: $0.text) }
            let kept = fromExtension + fromEarlierRuns
            do {
                let store = try OSLogStore(scope: .currentProcessIdentifier)
                let since = store.position(date: cutoff)
                let matching = NSPredicate(format: "subsystem == %@", "com.cluas.moshpit")
                let rows = try store.getEntries(at: since, matching: matching)
                    .compactMap { $0 as? OSLogEntryLog }
                    .map { Entry(date: $0.date, category: $0.category, message: $0.composedMessage) }
                // Newest first: the thing that just happened is the thing
                // being looked for.
                let merged = (Array(rows.suffix(400)) + kept)
                    .sorted { $0.date > $1.date }
                return .success(merged)
            } catch {
                // Even when the system store is unavailable, the extension's own
                // trail is still worth showing — it is the half this exists to
                // surface.
                if !kept.isEmpty {
                    return .success(kept.sorted { $0.date > $1.date })
                }
                return .failure(error)
            }
        }.value
    }

    /// Oldest first, one line per entry — the shape a mail attachment wants.
    /// nil when there is nothing to attach.
    static func transcript() async -> String? {
        guard case .success(let entries) = await recent(), !entries.isEmpty else { return nil }
        return entries.reversed().map(line).joined(separator: "\n")
    }
}

#if DEBUG
extension DiagnosticsLog {
    /// What a capture run has the log read as (`-MOSHPIT_SEED_RECENT_LOG
    /// <name>`), so the Recent Log's SwiftUI and NSH pages can be shot with
    /// the same content and compared pixel for pixel: `entries` (a fixed
    /// log), `empty`, `failed`, `loading` (never finishes). DEBUG only, as
    /// the other capture seeds are.
    enum Seed: String {
        case entries, empty, failed, loading

        static var requested: Seed? {
            let args = ProcessInfo.processInfo.arguments
            guard let i = args.firstIndex(of: "-MOSHPIT_SEED_RECENT_LOG"), i + 1 < args.count else { return nil }
            return Seed(rawValue: args[i + 1])
        }

        func result() async -> Result<[Entry], Error> {
            switch self {
            case .entries: return .success(Self.fixture)
            case .empty: return .success([])
            case .failed: return .failure(SeedError())
            case .loading:
                try? await Task.sleep(for: .seconds(3600))
                return .success([])
            }
        }

        private struct SeedError: LocalizedError {
            var errorDescription: String? { "The log store is not available (seeded)." }
        }

        /// Newest first, as `recent()` returns them; fixed times (local
        /// clock) so both pages show the same text.
        static let fixture: [Entry] = {
            let rows: [(String, String, String)] = [
                ("09:41:07.912", "health", "reconnect: network path changed (wifi → cellular), redialing dev"),
                ("09:41:07.388", "layout", "terminal 80x24 → 70x40 (keyboard up, iPad portrait)"),
                ("09:41:06.205", "push-extension", "decrypted attention envelope for dev · claude needs you"),
                ("09:41:05.770", "ssh", "channel 2 opened: tmux -CC attach -t =main"),
                ("09:41:05.104", "tmux", "%session-changed $1 main; following the phone's client"),
                ("09:41:04.551", "layout", "pin window @3 to 70x40; desktop client keeps 199x58 until it detaches"),
                ("09:41:03.902", "input", "backspace hold: 14 repeats in 1.40 s"),
                ("09:41:03.117", "voice", "dictation engine: SpeechTranscriber (zh-Hans), on device"),
                ("09:41:02.640", "health", "keepalive answered in 38 ms"),
                ("09:41:01.003", "push", "relay sync: 2 pairings, token a1b2c3…"),
            ]
            let day = Calendar.current.startOfDay(for: Date())
            return rows.map { time, category, message in
                let parts = time.split(whereSeparator: { $0 == ":" || $0 == "." }).compactMap { Double($0) }
                let seconds = parts[0] * 3600 + parts[1] * 60 + parts[2] + parts[3] / 1000
                return Entry(date: day.addingTimeInterval(seconds), category: category, message: message)
            }
        }()
    }
}
#endif
