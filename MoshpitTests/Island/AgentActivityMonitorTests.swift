import Foundation
import Testing
@testable import Moshpit

/// `AgentActivityMonitor` is a `@MainActor` state machine whose interesting
/// logic (output-heuristic promotion, the hook-owned precedence rules, the
/// idle/done/linger sweep) is all `private` and driven exclusively by live
/// `TmuxSessionController` callbacks and timers. The only connection-free,
/// pure surface it exposes is the `attentionState` query and the tracking
/// lifecycle, so that is what these tests pin down. See the report notes for
/// what could not be reached without a live tmux fixture or source changes.
@Suite("AgentActivityMonitor")
@MainActor
struct AgentActivityMonitorTests {

    private func makeMonitor() -> (AgentActivityMonitor, UserDefaults, String) {
        let name = "test.agentmonitor.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        let settings = AppSettings(defaults: defaults)
        return (AgentActivityMonitor(settings: settings), defaults, name)
    }

    @Test("attentionState is nil for a pane the monitor has no record of")
    func attentionStateNilWhenUntracked() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        // nil (not false) is the documented contract: callers must not block on
        // an absence of data, only on an explicit "no longer attention".
        #expect(monitor.attentionState(connectionId: UUID(), paneId: "%0") == nil)
    }

    @Test("untracking an unknown connection is a harmless no-op")
    func untrackUnknownIsSafe() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        monitor.untrack(connectionId: UUID())
        #expect(monitor.attentionState(connectionId: UUID(), paneId: "%1") == nil)
    }

    @Test("cycleHeadline is safe to call with no agents present")
    func cycleHeadlineOnEmptyStateIsSafe() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        // No tracked panes → buildState() yields an empty agent list; cycling the
        // headline offset must wrap harmlessly rather than divide/index by zero.
        monitor.cycleHeadline()
        monitor.cycleHeadline()
        #expect(monitor.attentionState(connectionId: UUID(), paneId: "%0") == nil)
    }
}
