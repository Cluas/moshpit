import Foundation
import Testing
@testable import Moshpit

/// Decoding `herdr api snapshot`. The fixture is a REAL capture from herdr
/// 0.7.3 (protocol 16) driven by Moshpit itself: two workspaces, the first
/// with two tabs, the first tab split into two panes.
@Suite("herdr snapshot decoding")
struct HerdrSnapshotTests {

    /// Read the capture from the source tree rather than the test bundle:
    /// whether a `.json` gets copied as a bundle resource depends on how the
    /// target was generated, and this file is checked in next to the tests.
    private func fixture() throws -> String {
        let path = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // …/MoshpitTests/Services/Herdr
            .deletingLastPathComponent()   // …/MoshpitTests/Services
            .deletingLastPathComponent()   // …/MoshpitTests
            .appendingPathComponent("Fixtures/herdr-snapshot.json")
        return try String(contentsOf: path, encoding: .utf8)
    }

    // MARK: - The real payload

    @Test("Real 0.7.3 capture decodes into the app's three levels")
    func decodesRealSnapshot() throws {
        let decoded = try #require(HerdrSnapshot.decode(fixture()))
        let snap = decoded.snapshot

        // workspace → session
        #expect(snap.sessions.count == 2)
        #expect(snap.sessions["w1"]?.name == "~")
        #expect(snap.sessions["w2"]?.name == "api")
        #expect(snap.sessions["w1"]?.isAttached == true)
        #expect(snap.sessions["w2"]?.isAttached == false)

        // tab → window, grouped under its workspace
        #expect(snap.windows.count == 3)
        #expect(snap.windows(inSession: "w1").map(\.id) == ["w1:t1", "w1:t2"])
        #expect(snap.windows["w1:t2"]?.name == "logs")
        #expect(snap.windows["w1:t2"]?.index == 2)
        #expect(snap.windows["w1:t1"]?.paneCount == 2)

        // pane
        #expect(snap.panes.count == 4)
        #expect(snap.panes(inWindow: "w1:t1").map(\.id) == ["w1:p1", "w1:p3"])
        #expect(snap.activePaneId == "w1:p3")
        #expect(snap.activeWindowId == "w1:t1")
        #expect(snap.activeSessionId == "w1")
        #expect(snap.panes["w1:p3"]?.isActive == true)
    }

    @Test("Pane geometry comes from the layouts array, not the pane objects")
    func geometryFromLayouts() throws {
        let decoded = try #require(HerdrSnapshot.decode(fixture()))
        // The split tab: two panes side by side, each half of the 44-col area.
        #expect(decoded.snapshot.panes["w1:p1"]?.width == 22)
        #expect(decoded.snapshot.panes["w1:p3"]?.width == 22)
        #expect(decoded.snapshot.panes["w1:p1"]?.height == 32)
    }

    @Test("A snapshot naming a workspace counts as attached")
    func attachedWhenPopulated() throws {
        let decoded = try #require(HerdrSnapshot.decode(fixture()))
        #expect(decoded.snapshot.isAttached)
        #expect(decoded.snapshot.everAttached)
    }

    // MARK: - Index derivation

    @Test("Pane index comes from the id, so poll order can't reshuffle panes",
          arguments: [("w1:p1", 1), ("w1:p3", 3), ("w12:p47", 47), ("nonsense", 0)])
    func paneIndexFromId(id: String, expected: Int) {
        #expect(HerdrSnapshot.paneIndex(id) == expected)
    }

    // MARK: - Version tolerance

    @Test("0.8.0-style panes carry agent + title through; 0.7.3-style don't break")
    func newerFieldsDecode() throws {
        // 0.7.3 omits every one of these keys. This is the same payload with
        // the 0.8.0 additions present.
        let raw = """
        {"id":"x","result":{"snapshot":{
          "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
          "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],
          "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true,
                    "agent_status":"working","agent":"claude","display_agent":"Claude Code",
                    "terminal_title_stripped":"claude — moshi"}],
          "layouts":[],
          "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}
        """
        let decoded = try #require(HerdrSnapshot.decode(raw))
        #expect(decoded.snapshot.panes["w1:p1"]?.command == "Claude Code")
        #expect(decoded.agentHooks["w1:p1"]?.state == "working")
        #expect(decoded.agentHooks["w1:p1"]?.agent == "Claude Code")
        #expect(decoded.agentHooks["w1:p1"]?.title == "claude — moshi")
        // No geometry reported → the model's own defaults, not a crash.
        #expect(decoded.snapshot.panes["w1:p1"]?.width == 80)
    }

    @Test("A bare snapshot (no response envelope) still decodes")
    func bareSnapshotDecodes() throws {
        let raw = """
        {"workspaces":[{"workspace_id":"w1","label":"solo","focused":true}],
         "tabs":[],"panes":[],"layouts":[]}
        """
        let decoded = try #require(HerdrSnapshot.decode(raw))
        #expect(decoded.snapshot.sessions["w1"]?.name == "solo")
    }

    // MARK: - Agent status → the app's stamp vocabulary

    @Test("agent_status maps onto the stamps the Island and sheets already speak",
          arguments: [("working", "working"), ("blocked", "attention"), ("done", "done")])
    func agentStatusMapping(status: String, stamp: String) throws {
        let decoded = try #require(HerdrSnapshot.decode(paneWith(status: status)))
        #expect(decoded.agentHooks["w1:p1"]?.state == stamp)
    }

    @Test("unknown produces no stamp — nothing lights up without cause")
    func unknownStatusIsQuiet() throws {
        let decoded = try #require(HerdrSnapshot.decode(paneWith(status: "unknown")))
        #expect(decoded.agentHooks["w1:p1"]?.state == nil)
    }

    /// `idle` passes through by its own name so the Agents section can show a
    /// NAMED idle agent as a quiet row — but it must never light anything:
    /// `AgentSignal` and the island's `hookState` both map it to nothing.
    @Test("idle passes through by name, and by name only")
    func idleStatusIsCarriedButUnlit() throws {
        let decoded = try #require(HerdrSnapshot.decode(paneWith(status: "idle")))
        #expect(decoded.agentHooks["w1:p1"]?.state == "idle")
        #expect(AgentSignal("idle") == nil)
    }

    private func paneWith(status: String) -> String {
        """
        {"result":{"snapshot":{
          "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
          "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],
          "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"\(status)"}],
          "layouts":[]}}}
        """
    }

    // MARK: - Junk in, no crash out

    @Test("Shell noise before the JSON is skipped, not fatal")
    func leadingNoise() throws {
        let raw = "Welcome to Ubuntu 24.04\n" + """
        {"result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"~"}],"tabs":[],"panes":[],"layouts":[]}}}
        """
        #expect(HerdrSnapshot.decode(raw) != nil)
    }

    @Test("A brace inside a label can't confuse the object scanner")
    func braceInsideString() throws {
        let raw = """
        {"result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"weird{name}"}],
         "tabs":[],"panes":[],"layouts":[]}}}
        """
        let decoded = try #require(HerdrSnapshot.decode(raw))
        #expect(decoded.snapshot.sessions["w1"]?.name == "weird{name}")
    }

    @Test("Payloads that aren't snapshots decode to nil rather than an empty tree",
          arguments: ["", "not json at all", "{}", "{\"result\":{}}"])
    func nonSnapshots(raw: String) {
        #expect(HerdrSnapshot.decode(raw) == nil)
    }

    @Test("server_not_running is recognized as its own state")
    func serverNotRunning() {
        let raw = """
        {"error":{"code":"server_not_running","message":"no herdr server is running at /x/herdr.sock"},"id":"cli:api:snapshot"}
        """
        #expect(HerdrSnapshot.decode(raw) == nil)
        #expect(HerdrSnapshot.isServerNotRunning(raw))
        // A different error must NOT read as "no server".
        #expect(!HerdrSnapshot.isServerNotRunning("{\"error\":{\"code\":\"not_found\"}}"))
    }
}
