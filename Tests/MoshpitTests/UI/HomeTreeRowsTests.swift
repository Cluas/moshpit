import Foundation
import Testing
@testable import Moshpit

/// The Home tree is flattened into List rows so each level is a row of its
/// own (swipe actions, long-press menu, insert/remove animation). What a
/// collapsed session and an unexpanded window hide is the whole contract, so
/// it is pinned here against the pure builder rather than the view.
@Suite("Home tree rows")
@MainActor
struct HomeTreeRowsTests {

    private func snapshot() -> TmuxSnapshot {
        var snap = TmuxSnapshot()
        snap.sessions["$0"] = SessionInfo(id: "$0", name: "main", isAttached: true)
        snap.sessions["$1"] = SessionInfo(id: "$1", name: "dev")
        snap.windows["@0"] = WindowInfo(id: "@0", sessionId: "$0", name: "alpha", index: 1)
        snap.windows["@1"] = WindowInfo(id: "@1", sessionId: "$0", name: "bravo", index: 2)
        snap.windows["@2"] = WindowInfo(id: "@2", sessionId: "$1", name: "work", index: 1)
        snap.panes["%0"] = PaneInfo(id: "%0", windowId: "@0", index: 0, command: "zsh")
        snap.panes["%1"] = PaneInfo(id: "%1", windowId: "@0", index: 1, command: "claude")
        snap.panes["%2"] = PaneInfo(id: "%2", windowId: "@1", index: 0)
        snap.panes["%3"] = PaneInfo(id: "%3", windowId: "@2", index: 0)
        return snap
    }

    private func sessions(_ snap: TmuxSnapshot) -> [SessionInfo] {
        snap.sessions.values.sorted { $0.id < $1.id }
    }

    @Test("every session lists its windows; panes only under an expanded window")
    func expandedWindowShowsItsPanes() {
        let snap = snapshot()
        let rows = ConnectionCard.treeRows(sessions: sessions(snap), snapshot: snap,
                                           collapsed: [], expanded: ["@0"])
        #expect(rows.map(\.id) == ["s$0", "w@0", "p%0", "p%1", "w@1", "s$1", "w@2"])
    }

    @Test("a collapsed session hides its windows and their panes, expanded or not")
    func collapsedSessionHidesItsSubtree() {
        let snap = snapshot()
        let rows = ConnectionCard.treeRows(sessions: sessions(snap), snapshot: snap,
                                           collapsed: ["$0"], expanded: ["@0"])
        #expect(rows.map(\.id) == ["s$0", "s$1", "w@2"])
    }

    @Test("row ids never collide across levels, even with look-alike raw ids")
    func rowIdsAreLevelScoped() {
        var snap = TmuxSnapshot()
        snap.sessions["1"] = SessionInfo(id: "1", name: "one")
        snap.windows["1"] = WindowInfo(id: "1", sessionId: "1", name: "one", index: 1)
        snap.panes["1"] = PaneInfo(id: "1", windowId: "1", index: 0)
        let rows = ConnectionCard.treeRows(sessions: sessions(snap), snapshot: snap,
                                           collapsed: [], expanded: ["1"])
        #expect(Set(rows.map(\.id)).count == rows.count)
    }
}
