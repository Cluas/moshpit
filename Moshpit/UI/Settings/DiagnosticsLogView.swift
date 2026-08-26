import OSLog
import SwiftUI

/// The app's own recent log, on screen.
///
/// This exists because of how the last several bugs went. Every one of them
/// was reported from a phone, reproduced (or not) on a simulator against
/// localhost, and diagnosed by guesswork — twice from a test rig that lied.
/// The log has the answers in it (`layout:` names every terminal resize with
/// its before-and-after size, `health:` names the cause of every reconnect),
/// but reading it needed the phone plugged into a Mac, and the phone is never
/// plugged into a Mac when the bug happens.
///
/// `OSLogStore(scope: .currentProcessIdentifier)` lets the app read what it
/// wrote itself, so the answer travels as a screenshot instead. Nothing here
/// is collected, uploaded or persisted beyond what the system already keeps —
/// it is the same log `log stream` would show, shown in the place where the
/// problem was seen.
struct DiagnosticsLogView: View {
    @State private var entries: [Entry] = []
    @State private var loadError: String?
    @State private var isLoading = true

    struct Entry: Identifiable {
        let id = UUID()
        let date: Date
        let category: String
        let message: String
    }

    /// How far back to read. Long enough to cover "it just did the thing",
    /// short enough that the store answers quickly.
    private static let window: TimeInterval = 30 * 60

    var body: some View {
        // The message states live OUTSIDE the scroll view. Inside one they were
        // sized to their text and pinned to the top-left corner — a lone
        // "Reading…" floating in a dark screen read as a rendering bug, not as
        // a state, for however long the log-archive walk took.
        Group {
            if isLoading {
                VStack(spacing: 10) {
                    ProgressView()
                        .tint(Ink.meta)
                    Text("Reading…")
                        .font(Face.mono(12)).foregroundStyle(Ink.meta)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let loadError {
                Text(loadError)
                    .font(Face.mono(12)).foregroundStyle(Ink.warn)
                    .multilineTextAlignment(.center)
                    .padding(24)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if entries.isEmpty {
                Text("Nothing logged in the last 30 minutes.")
                    .font(Face.mono(12)).foregroundStyle(Ink.meta)
                    .padding(24)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                logList
            }
        }
        .background(Ink.screenBG)
        .navigationTitle("Recent Log")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Copy") { UIPasteboard.general.string = plainText }
                    .disabled(entries.isEmpty)
            }
        }
        .task { await load() }
    }

    private var logList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(entries) { entry in
                    VStack(alignment: .leading, spacing: 1) {
                        Text("\(Self.clock.string(from: entry.date))  \(entry.category)")
                            .font(Face.mono(9)).foregroundStyle(Ink.meta)
                        Text(entry.message)
                            .font(Face.mono(11)).foregroundStyle(Ink.primary)
                            .textSelection(.enabled)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14).padding(.vertical, 6)
                    Divider().overlay(Ink.hairline)
                }
            }
        }
    }

    private static let clock: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()

    private var plainText: String {
        entries.map { "\(Self.clock.string(from: $0.date))  \($0.category)  \($0.message)" }
            .joined(separator: "\n")
    }

    /// Off the main actor: the store walks the system's log archive and can
    /// take a beat on a device with a busy log.
    private func load() async {
        let found: Result<[Entry], Error> = await Task.detached(priority: .userInitiated) {
            let cutoff = Date().addingTimeInterval(-Self.window)
            // The notification service extension is a SEPARATE PROCESS, and
            // `.currentProcessIdentifier` cannot see it — iOS gives an app no way
            // to read another process's log. So the two lines that decide whether
            // a push was decrypted, or fell back and why, would never appear on
            // this screen no matter how long someone scrolled. The extension
            // leaves them in the App Group instead; they get merged in here.
            //
            // This is the whole point of the screen: a reviewer went looking for
            // exactly those lines here, found nothing, and reasoned from the
            // absence. A diagnostic tool that is blind to the riskiest component
            // misleads better than it helps.
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
                // trail is still worth showing — it is the half this screen
                // exists to surface.
                if !fromExtension.isEmpty {
                    return .success(fromExtension.sorted { $0.date > $1.date })
                }
                return .failure(error)
            }
        }.value
        switch found {
        case .success(let rows): entries = rows
        case .failure(let error): loadError = "Couldn't read the log: \(error.localizedDescription)"
        }
        isLoading = false
    }
}
