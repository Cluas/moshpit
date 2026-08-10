import Foundation
import Observation

/// In-memory snapshot of "live session" telemetry — what the Mosaic prototype's
/// home/terminal screens use to render latency / loss / sparkline values.
///
/// This intentionally does NOT persist. Real values come from a future Mosh /
/// SSH SRTT collector; until then, the registry seeds plausible values for the
/// servers in `ConnectionStore` so the UI renders the way the design intends.
@Observable
final class SessionMetricsRegistry {
    /// Keyed by `ServerConnection.id`. A missing entry means "saved but never
    /// connected" — UI shows `—` for latency and no live indicators.
    var metrics: [UUID: SessionMetrics] = [:]

    /// Group → ordered server IDs. Drives the "Pinned", "Homelab" etc.
    /// sections on the home screen. The first group is treated as primary
    /// and pinned to the top.
    var groups: [SessionGroup] = []

    init() {}

    /// Pull out the connections that are currently "live" (running session).
    /// Used by the home screen hero strip.
    func liveConnections(from store: ConnectionStore) -> [ServerConnection] {
        store.connections.filter { metrics[$0.id]?.state.isLive == true }
    }

    /// Find the hero connection: prefer the highest-priority live session,
    /// otherwise return `nil` so the home screen falls back to an empty hero.
    func heroConnection(from store: ConnectionStore) -> ServerConnection? {
        let live = liveConnections(from: store)
        return live.sorted { lhs, rhs in
            let lhsAge = metrics[lhs.id]?.connectedAt ?? .distantPast
            let rhsAge = metrics[rhs.id]?.connectedAt ?? .distantPast
            return lhsAge > rhsAge
        }.first
    }
}

struct SessionMetrics: Hashable {
    var state: SessionState
    /// SRTT in milliseconds; nil = no live data.
    var srttMs: Double?
    /// Packet loss percentage 0…100.
    var lossPct: Double?
    /// When the session connected — used for "connected 42 min" displays.
    var connectedAt: Date?
    /// Last 24 SRTT samples for the sparkline; oldest first.
    var srttHistory: [Double]
    /// Optional last terminal line (for the home hero preview).
    var lastOutputLine: String?
    /// Pane count surfaced from tmux cc, if attached.
    var paneCount: Int
    /// Notes whether the session is currently roaming between networks.
    var isRoaming: Bool
    /// Trailing "now/14h/3d" style timestamp marker for the server-list row.
    var lastActiveLabel: String?
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
        lossPct: Double? = nil,
        connectedAt: Date? = nil,
        srttHistory: [Double] = [],
        lastOutputLine: String? = nil,
        paneCount: Int = 0,
        isRoaming: Bool = false,
        lastActiveLabel: String? = nil,
        moshDegraded: Bool = false,
        moshDiagnostics: MoshDiagnostics? = nil
    ) {
        self.state = state
        self.srttMs = srttMs
        self.lossPct = lossPct
        self.connectedAt = connectedAt
        self.srttHistory = srttHistory
        self.lastOutputLine = lastOutputLine
        self.paneCount = paneCount
        self.isRoaming = isRoaming
        self.lastActiveLabel = lastActiveLabel
        self.moshDegraded = moshDegraded
        self.moshDiagnostics = moshDiagnostics
    }
}

enum SessionState: String, Hashable {
    case saved      // configured, never connected
    case live       // currently attached
    case roaming    // mosh resume in progress
    case offline    // last known but no longer reachable

    var isLive: Bool { self == .live || self == .roaming }
}

struct SessionGroup: Identifiable, Hashable {
    let id: String
    let name: String
    let glyph: String          // ★, ⌂, etc.
    let glyphColor: GlyphColor
    var connectionIDs: [UUID]

    enum GlyphColor: String, Hashable {
        case amber, cyan, violet, lime
    }
}
