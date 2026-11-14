import Foundation
import Testing
@testable import Moshpit

/// Pure-value coverage for `SessionMetrics` and its `SessionState` enum.
@Suite("SessionMetrics")
struct SessionMetricsTests {

    // MARK: - SessionState

    @Test("SessionState raw values are stable wire strings")
    func stateRawValues() {
        #expect(SessionState.saved.rawValue == "saved")
        #expect(SessionState.live.rawValue == "live")
        #expect(SessionState.roaming.rawValue == "roaming")
        #expect(SessionState.offline.rawValue == "offline")
    }

    // MARK: - SessionMetrics value semantics

    @Test("SessionMetrics defaults: saved, no telemetry, empty history")
    func metricsDefaults() {
        let m = SessionMetrics()
        #expect(m.state == .saved)
        #expect(m.srttMs == nil)
        #expect(m.connectedAt == nil)
        #expect(m.srttHistory.isEmpty)
        #expect(!m.isRoaming)
        #expect(!m.moshDegraded)
        #expect(m.moshDiagnostics == nil)
    }

    @Test("SessionMetrics is Hashable with structural equality")
    func metricsHashable() {
        let a = SessionMetrics(state: .live, srttMs: 42, srttHistory: [1, 2, 3])
        let b = SessionMetrics(state: .live, srttMs: 42, srttHistory: [1, 2, 3])
        let c = SessionMetrics(state: .live, srttMs: 43, srttHistory: [1, 2, 3])
        #expect(a == b)
        #expect(a != c)
        var set: Set<SessionMetrics> = []
        set.insert(a); set.insert(b); set.insert(c)
        #expect(set.count == 2, "equal metrics must collapse in a Set")
    }
}
