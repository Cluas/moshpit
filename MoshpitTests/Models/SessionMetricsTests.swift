import Foundation
import Testing
@testable import Moshpit

/// Pure-value coverage for `SessionMetrics`, its `SessionState` enum, the
/// `SessionGroup` model, and the query logic on `SessionMetricsRegistry`
/// (`liveConnections` / `heroConnection`). No persistence, no network — the
/// registry is fed an in-memory `ConnectionStore` backed by a throwaway
/// UserDefaults suite.
@Suite("SessionMetrics")
struct SessionMetricsTests {

    // MARK: - SessionState

    @Test("only .live and .roaming count as live")
    func stateIsLive() {
        #expect(SessionState.live.isLive)
        #expect(SessionState.roaming.isLive)
        #expect(!SessionState.saved.isLive)
        #expect(!SessionState.offline.isLive)
    }

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
        #expect(m.lossPct == nil)
        #expect(m.connectedAt == nil)
        #expect(m.srttHistory.isEmpty)
        #expect(m.lastOutputLine == nil)
        #expect(m.paneCount == 0)
        #expect(!m.isRoaming)
        #expect(!m.moshDegraded)
        #expect(m.moshDiagnostics == nil)
    }

    @Test("SessionMetrics is Hashable with structural equality")
    func metricsHashable() {
        let a = SessionMetrics(state: .live, srttMs: 42, srttHistory: [1, 2, 3], paneCount: 2)
        let b = SessionMetrics(state: .live, srttMs: 42, srttHistory: [1, 2, 3], paneCount: 2)
        let c = SessionMetrics(state: .live, srttMs: 43, srttHistory: [1, 2, 3], paneCount: 2)
        #expect(a == b)
        #expect(a != c)
        var set: Set<SessionMetrics> = []
        set.insert(a); set.insert(b); set.insert(c)
        #expect(set.count == 2, "equal metrics must collapse in a Set")
    }

    // MARK: - SessionGroup

    @Test("SessionGroup is Identifiable by its string id and carries its glyph color")
    func sessionGroup() {
        let g = SessionGroup(id: "pinned", name: "Pinned", glyph: "★",
                             glyphColor: .amber, connectionIDs: [])
        #expect(g.id == "pinned")
        #expect(g.glyphColor == .amber)
        #expect(SessionGroup.GlyphColor.cyan.rawValue == "cyan")
    }

    // MARK: - Registry queries

    /// Fresh registry + store pair over an isolated defaults suite. Returns the
    /// suite name so the caller can tear it down.
    private func makeRegistryAndStore() -> (SessionMetricsRegistry, ConnectionStore, UserDefaults, String) {
        let name = "test.sessionmetrics.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        return (SessionMetricsRegistry(), ConnectionStore(defaults: defaults), defaults, name)
    }

    @Test("liveConnections returns only servers whose metrics report a live state")
    func liveConnectionsFilters() {
        let (registry, store, defaults, name) = makeRegistryAndStore()
        defer { defaults.removePersistentDomain(forName: name) }

        let live = ServerConnection(name: "live", host: "h1", username: "u")
        let roaming = ServerConnection(name: "roaming", host: "h2", username: "u")
        let saved = ServerConnection(name: "saved", host: "h3", username: "u")
        let neverConnected = ServerConnection(name: "cold", host: "h4", username: "u")
        store.add(live); store.add(roaming); store.add(saved); store.add(neverConnected)

        registry.metrics[live.id] = SessionMetrics(state: .live)
        registry.metrics[roaming.id] = SessionMetrics(state: .roaming)
        registry.metrics[saved.id] = SessionMetrics(state: .saved)
        // neverConnected has NO metrics entry at all.

        let liveIDs = Set(registry.liveConnections(from: store).map(\.id))
        #expect(liveIDs == [live.id, roaming.id])
    }

    @Test("heroConnection is the most-recently-connected live session")
    func heroPicksMostRecentLive() {
        let (registry, store, defaults, name) = makeRegistryAndStore()
        defer { defaults.removePersistentDomain(forName: name) }

        let older = ServerConnection(name: "older", host: "h1", username: "u")
        let newer = ServerConnection(name: "newer", host: "h2", username: "u")
        store.add(older); store.add(newer)

        let now = Date()
        registry.metrics[older.id] = SessionMetrics(state: .live, connectedAt: now.addingTimeInterval(-600))
        registry.metrics[newer.id] = SessionMetrics(state: .live, connectedAt: now)

        #expect(registry.heroConnection(from: store)?.id == newer.id)
    }

    @Test("heroConnection is nil when nothing is live")
    func heroNilWhenNoneLive() {
        let (registry, store, defaults, name) = makeRegistryAndStore()
        defer { defaults.removePersistentDomain(forName: name) }

        let s = ServerConnection(name: "s", host: "h", username: "u")
        store.add(s)
        registry.metrics[s.id] = SessionMetrics(state: .offline)

        #expect(registry.heroConnection(from: store) == nil)
    }
}
