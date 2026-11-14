import Foundation
import Observation

/// In-memory "live session" telemetry, keyed by connection — what the home
/// card and the terminal's mosh pill read for latency, roaming and the
/// sparkline. Written by `SessionHub` (the mosh transport's SRTT/roaming
/// callbacks and each session's connect/disconnect). Never persisted.
@Observable
final class SessionMetricsRegistry {
    /// Keyed by `ServerConnection.id`. A missing entry means "saved but never
    /// connected" — UI shows `—` for latency and no live indicators.
    var metrics: [UUID: SessionMetrics] = [:]

    init() {}
}

struct SessionMetrics: Hashable {
    var state: SessionState
    /// SRTT in milliseconds; nil = no live data.
    var srttMs: Double?
    /// When the session connected — used for "connected 42 min" displays.
    var connectedAt: Date?
    /// Last 24 SRTT samples for the sparkline; oldest first.
    var srttHistory: [Double]
    /// Notes whether the session is currently roaming between networks.
    var isRoaming: Bool
    /// True when a mosh-configured connection fell back to plain SSH because
    /// the host had no mosh-server. The home card's MOSH pill shows a degraded
    /// (grey + ⚠) state so the user knows roaming isn't active this session.
    var moshDegraded: Bool
    /// Live mosh protocol counters, for the pill's long-press diagnostic
    /// overlay. nil until the first datagram of a mosh session arrives.
    var moshDiagnostics: MoshDiagnostics?

    init(
        state: SessionState = .saved,
        srttMs: Double? = nil,
        connectedAt: Date? = nil,
        srttHistory: [Double] = [],
        isRoaming: Bool = false,
        moshDegraded: Bool = false,
        moshDiagnostics: MoshDiagnostics? = nil
    ) {
        self.state = state
        self.srttMs = srttMs
        self.connectedAt = connectedAt
        self.srttHistory = srttHistory
        self.isRoaming = isRoaming
        self.moshDegraded = moshDegraded
        self.moshDiagnostics = moshDiagnostics
    }
}

enum SessionState: String, Hashable {
    case saved      // configured, never connected
    case live       // currently attached
    case roaming    // mosh resume in progress
    case offline    // last known but no longer reachable
}
