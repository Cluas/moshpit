import Foundation
import Testing
@testable import Moshpit

@Suite("TmuxTypes value semantics")
struct TmuxTypesTests {

    // MARK: - PaneInfo

    @Test("PaneInfo Codable round-trip preserves every field")
    func paneInfoCodable() throws {
        let original = PaneInfo(
            id: "%4",
            windowId: "@0",
            command: "vim",
            width: 132,
            height: 40,
            isActive: true
        )
        let data = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(PaneInfo.self, from: data)
        #expect(decoded == original)
    }

    @Test("PaneInfo equality is structural, not identity-only")
    func paneInfoEquality() {
        let a = PaneInfo(id: "%0", windowId: "@0", command: "bash")
        let b = PaneInfo(id: "%0", windowId: "@0", command: "bash")
        let c = PaneInfo(id: "%0", windowId: "@0", command: "zsh")
        #expect(a == b)
        #expect(a != c, "different command must break equality")
    }

    @Test("PaneInfo.id is the tmux string identifier (%N), used directly as the Identifiable id")
    func paneInfoIdentifiableId() {
        let pane = PaneInfo(id: "%17", windowId: "@2")
        #expect(pane.id == "%17")
    }

    @Test("PaneInfo defaults: width=80, height=24, isActive=false, empty command")
    func paneInfoDefaults() {
        let pane = PaneInfo(id: "%0", windowId: "@0")
        #expect(pane.command == "")
        #expect(pane.width == 80)
        #expect(pane.height == 24)
        #expect(pane.isActive == false)
    }

    // MARK: - WindowInfo

    @Test("WindowInfo Codable round-trip preserves layout string verbatim")
    func windowInfoCodable() throws {
        let original = WindowInfo(
            id: "@5",
            name: "logs",
            index: 3,
            layout: "ab12,200x50,0,0{100x50,0,0,4,99x50,101,0,5}",
            isActive: true,
            paneCount: 2
        )
        let data = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(WindowInfo.self, from: data)
        #expect(decoded == original)
        // The layout descriptor is opaque to us; round-trip must not transform.
        #expect(decoded.layout == original.layout)
    }

    @Test("WindowInfo equality differentiates on every mutating field")
    func windowInfoEquality() {
        let base = WindowInfo(id: "@0", name: "main", index: 0, layout: "L", isActive: true, paneCount: 1)
        #expect(base == WindowInfo(id: "@0", name: "main", index: 0, layout: "L", isActive: true, paneCount: 1))
        #expect(base != WindowInfo(id: "@0", name: "edge", index: 0, layout: "L", isActive: true, paneCount: 1))
        #expect(base != WindowInfo(id: "@0", name: "main", index: 1, layout: "L", isActive: true, paneCount: 1))
        #expect(base != WindowInfo(id: "@0", name: "main", index: 0, layout: "X", isActive: true, paneCount: 1))
        #expect(base != WindowInfo(id: "@0", name: "main", index: 0, layout: "L", isActive: false, paneCount: 1))
        #expect(base != WindowInfo(id: "@0", name: "main", index: 0, layout: "L", isActive: true, paneCount: 2))
    }

    @Test("snapshot scopes windows by session; sortedWindows is the active session only")
    func windowsScopedBySession() {
        var snap = TmuxSnapshot()
        snap.windows = [
            "@0": WindowInfo(id: "@0", sessionId: "$0", index: 1),
            "@1": WindowInfo(id: "@1", sessionId: "$0", index: 0),
            "@2": WindowInfo(id: "@2", sessionId: "$1", index: 0),
        ]
        snap.activeSessionId = "$1"
        // Per-session lookup is sorted by index and isolated.
        #expect(snap.windows(inSession: "$0").map(\.id) == ["@1", "@0"])
        #expect(snap.windows(inSession: "$1").map(\.id) == ["@2"])
        // sortedWindows follows the attached session, not every window.
        #expect(snap.sortedWindows.map(\.id) == ["@2"])
    }

    // MARK: - SessionInfo

    @Test("SessionInfo Codable round-trip preserves attachment flag")
    func sessionInfoCodable() throws {
        let original = SessionInfo(id: "$0", name: "moshi", isAttached: true)
        let data = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(SessionInfo.self, from: data)
        #expect(decoded == original)
    }

    @Test("SessionInfo defaults: empty name, not attached")
    func sessionInfoDefaults() {
        let s = SessionInfo(id: "$0")
        #expect(s.name == "")
        #expect(s.isAttached == false)
    }

    // MARK: - Hashable / dictionary keys

    @Test("Equal PaneInfos hash to the same bucket (Hashable contract holds)")
    func paneInfoHashable() {
        var set: Set<PaneInfo> = []
        let a = PaneInfo(id: "%0", windowId: "@0", command: "bash")
        let b = PaneInfo(id: "%0", windowId: "@0", command: "bash")
        set.insert(a)
        set.insert(b)
        #expect(set.count == 1, "equal panes must collapse in a Set")
    }
}
