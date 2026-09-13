import Foundation
import OSLog
import MoshpitKit

/// The app's own recent log, read back through `OSLogStore` — what the
/// Recent Log screen shows and what a feedback mail can attach. Nothing is
/// collected or persisted beyond what the system already keeps; see
/// `DiagnosticsLogView` for why reading it back on the phone matters.
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

    static let clock: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()

    static func line(_ entry: Entry) -> String {
        "\(clock.string(from: entry.date))  \(entry.category)  \(entry.message)"
    }

    /// Newest first. Off the main actor: the store walks the system's log
    /// archive and can take a beat on a device with a busy log.
    static func recent() async -> Result<[Entry], Error> {
        await Task.detached(priority: .userInitiated) {
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
            let fromExtension = PushDiagnostics.recent(since: cutoff).map {
                Entry(date: $0.at, category: PushDiagnostics.source, message: $0.text)
            }
            do {
                let store = try OSLogStore(scope: .currentProcessIdentifier)
                let since = store.position(date: cutoff)
                let matching = NSPredicate(format: "subsystem == %@", "com.cluas.moshpit")
                let rows = try store.getEntries(at: since, matching: matching)
                    .compactMap { $0 as? OSLogEntryLog }
                    .map { Entry(date: $0.date, category: $0.category, message: $0.composedMessage) }
                // Newest first: the thing that just happened is the thing
                // being looked for.
                let merged = (Array(rows.suffix(400)) + fromExtension)
                    .sorted { $0.date > $1.date }
                return .success(merged)
            } catch {
                // Even when the system store is unavailable, the extension's own
                // trail is still worth showing — it is the half this exists to
                // surface.
                if !fromExtension.isEmpty {
                    return .success(fromExtension.sorted { $0.date > $1.date })
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
