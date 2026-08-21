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
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if isLoading {
                    Text("Reading…")
                        .font(Face.mono(12)).foregroundStyle(Ink.meta)
                        .padding(16)
                } else if let loadError {
                    Text(loadError)
                        .font(Face.mono(12)).foregroundStyle(Ink.warn)
                        .padding(16)
                } else if entries.isEmpty {
                    Text("Nothing logged in the last 30 minutes.")
                        .font(Face.mono(12)).foregroundStyle(Ink.meta)
                        .padding(16)
                } else {
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
            do {
                let store = try OSLogStore(scope: .currentProcessIdentifier)
                let since = store.position(date: Date().addingTimeInterval(-Self.window))
                let matching = NSPredicate(format: "subsystem == %@", "com.cluas.moshpit")
                let rows = try store.getEntries(at: since, matching: matching)
                    .compactMap { $0 as? OSLogEntryLog }
                    .map { Entry(date: $0.date, category: $0.category, message: $0.composedMessage) }
                // Newest first: the thing that just happened is the thing
                // being looked for.
                return .success(Array(rows.suffix(400).reversed()))
            } catch {
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
