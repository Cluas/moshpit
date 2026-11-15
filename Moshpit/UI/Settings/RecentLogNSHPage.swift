import SwiftUI

/// Settings › Recent Log drawn by NSH (`ui/src/recent_log.rs`): the same log,
/// read the same way (``DiagnosticsLog/recent()``), posted to the screen,
/// which shows it. Its back button pops the stack. If its screen can't run,
/// the page says so (``NSHPageFailed``).
struct RecentLogNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model = RecentLogModel()
    @State private var failed = false

    var body: some View {
        if failed {
            NSHPageFailed { failed = false }
        } else {
            NSHPage(
                screen: .recentLog,
                attach: { model.attach($0) },
                onMessage: { message in
                    switch message["ev"] as? String {
                    case "back": dismiss()
                    // A failed read's Retry (R16): read the log again. (Its
                    // own event: `retry` is the error boundary's, which runs
                    // the screen again on the state it keeps.)
                    case "reload": Task { await model.load() }
                    default: break
                    }
                },
                onFailure: { failed = true })
                .nshPage()
                .task { await model.load() }
        }
    }
}

/// What the NSH Recent Log shows, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule), and the messages that carry it
/// (`{"op": "log", "state": …}`, `ui/src/recent_log.rs`'s `Message`).
@MainActor
final class RecentLogModel {
    enum State: Equatable {
        case loading
        /// What to show: the error, in words.
        case failed(String)
        case loaded([DiagnosticsLog.Entry])

        static func == (lhs: State, rhs: State) -> Bool {
            switch (lhs, rhs) {
            case (.loading, .loading): true
            case let (.failed(a), .failed(b)): a == b
            case let (.loaded(a), .loaded(b)): a.map(\.id) == b.map(\.id)
            default: false
            }
        }
    }

    private(set) var state: State = .loading
    private weak var surface: NSHSurface?

    /// `surface` shows the state from now on, starting with the current one.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        surface.post(Self.message(for: state))
    }

    /// Reads the log and shows what came back.
    func load() async {
        switch await DiagnosticsLog.recent() {
        case .success(let rows): show(.loaded(rows))
        case .failure(let error): show(.failed(DiagnosticsLog.readFailure(error)))
        }
    }

    func show(_ state: State) {
        self.state = state
        surface?.post(Self.message(for: state))
    }

    /// The screen's message for `state`.
    nonisolated static func message(for state: State) -> [String: Any] {
        switch state {
        case .loading:
            return ["op": "log", "state": "loading"]
        case .failed(let message):
            return ["op": "log", "state": "failed", "message": message]
        case .loaded(let entries):
            return ["op": "log", "state": "loaded", "entries": entries.map(entry)]
        }
    }

    /// One entry as the screen takes it: the time formatted by the clock the
    /// SwiftUI page uses.
    nonisolated static func entry(_ entry: DiagnosticsLog.Entry) -> [String: Any] {
        [
            "time": DiagnosticsLog.clock.string(from: entry.date),
            "category": entry.category,
            "message": entry.message,
        ]
    }
}
