import OSLog
import SwiftUI
import MoshpitKit

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
///
/// Its accessibility identifiers (`recent-log-…`) are the NSH page's too
/// (`ui/src/recent_log.rs`, behind a switch: ``RecentLogPage``), so a UI test
/// finds either page the same way.
struct DiagnosticsLogView: View {
    @State private var entries: [Entry] = []
    @State private var loadError: String?
    @State private var isLoading = true

    typealias Entry = DiagnosticsLog.Entry

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
                        .accessibilityIdentifier("recent-log-loading")
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let loadError {
                Text(loadError)
                    .font(Face.mono(12)).foregroundStyle(Ink.warn)
                    .multilineTextAlignment(.center)
                    .accessibilityIdentifier("recent-log-error")
                    .padding(24)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if entries.isEmpty {
                Text("Nothing logged in the last 30 minutes.")
                    .font(Face.mono(12)).foregroundStyle(Ink.meta)
                    .accessibilityIdentifier("recent-log-empty")
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
                    .accessibilityIdentifier("recent-log-copy")
            }
        }
        .task { await load() }
    }

    private var logList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(entries) { entry in
                    VStack(alignment: .leading, spacing: 1) {
                        Text("\(DiagnosticsLog.clock.string(from: entry.date))  \(entry.category)")
                            .font(Face.mono(9)).foregroundStyle(Ink.meta)
                        Text(entry.message)
                            .font(Face.mono(11)).foregroundStyle(Ink.primary)
                            .textSelection(.enabled)
                            .accessibilityIdentifier("recent-log-entry")
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14).padding(.vertical, 6)
                    Divider().overlay(Ink.hairline)
                }
            }
        }
        .accessibilityIdentifier("recent-log-list")
    }

    private var plainText: String {
        entries.map(DiagnosticsLog.line).joined(separator: "\n")
    }

    /// The reading itself lives in `DiagnosticsLog` so the feedback mail can
    /// attach the same lines this screen shows.
    private func load() async {
        switch await DiagnosticsLog.recent() {
        case .success(let rows): entries = rows
        case .failure(let error): loadError = DiagnosticsLog.readFailure(error)
        }
        isLoading = false
    }
}
