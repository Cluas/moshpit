import Foundation
import Testing
@testable import Moshpit

/// The breadcrumb's third segment doubles as the agent crumb, and the bar is
/// 402pt wide — so what each segment says, and which one gives up its text
/// first, is decided in a pure function these tests can hold still.
@Suite("Breadcrumb plan")
@MainActor
struct BreadcrumbPlanTests {

    private func snapshot(windowName: String = "fix-scroll") -> TmuxSnapshot {
        var snap = TmuxSnapshot()
        snap.sessions["w1"] = SessionInfo(id: "w1", name: "moshi", isAttached: true)
        snap.windows["w1:t2"] = WindowInfo(id: "w1:t2", sessionId: "w1", name: windowName, index: 2)
        snap.panes["w1:p1"] = PaneInfo(id: "w1:p1", windowId: "w1:t2", index: 1, command: "zsh")
        snap.panes["w1:p2"] = PaneInfo(id: "w1:p2", windowId: "w1:t2", index: 2)
        snap.activeSessionId = "w1"
        snap.activeWindowId = "w1:t2"
        snap.activePaneId = "w1:p1"
        snap.isAttached = true
        return snap
    }

    @Test("No agent: three full segments, exactly what always shipped")
    func plainPane() {
        let plan = BreadcrumbPlan.make(snapshot: snapshot(), hooks: [:])
        #expect(plan == BreadcrumbPlan(
            sessionTitle: "moshi", sessionIconOnly: false,
            windowTitle: "2:fix-scroll", paneTitle: "zsh", paneSignal: nil))
    }

    @Test("Agent in the pane: its name takes the crumb, the session yields its text")
    func agentPane() {
        let plan = BreadcrumbPlan.make(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "attention", agent: "Claude Code")])
        #expect(plan?.paneTitle == "Claude Code")
        #expect(plan?.paneSignal == .attention)
        #expect(plan?.sessionIconOnly == true)
        // Still there for the tap target / VoiceOver, just not spelled out.
        #expect(plan?.sessionTitle == "moshi")
    }

    @Test("A signal without a name still lights the dot on the command")
    func signalWithoutName() {
        let plan = BreadcrumbPlan.make(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "working")])
        #expect(plan?.paneTitle == "zsh")
        #expect(plan?.paneSignal == .working)
        #expect(plan?.sessionIconOnly == true)
    }

    @Test("A hook on some OTHER pane changes nothing here")
    func hookElsewhere() {
        let plan = BreadcrumbPlan.make(
            snapshot: snapshot(),
            hooks: ["w1:p2": AgentHook(state: "attention", agent: "Codex")])
        #expect(plan?.paneTitle == "zsh")
        #expect(plan?.paneSignal == nil)
        #expect(plan?.sessionIconOnly == false)
    }

    /// herdr 0.7.3: no command field on panes — the number keeps the crumb
    /// (and with it the Select Pane sheet's only entry point) alive.
    @Test("No command, no agent: the pane number floor")
    func paneNumberFloor() {
        var snap = snapshot()
        snap.activePaneId = "w1:p2"
        let plan = BreadcrumbPlan.make(snapshot: snap, hooks: [:])
        #expect(plan?.paneTitle == "pane 2")
    }

    @Test("herdr's default tab names say the number once, not twice")
    func windowNameDeduped() {
        let plan = BreadcrumbPlan.make(snapshot: snapshot(windowName: "2"), hooks: [:])
        #expect(plan?.windowTitle == "2")
    }

    @Test("Not attached: no plan, the bar shows connection identity instead")
    func detached() {
        var snap = snapshot()
        snap.isAttached = false
        #expect(BreadcrumbPlan.make(snapshot: snap, hooks: [:]) == nil)
    }

    @Test("An empty agent string is not a name")
    func emptyAgentName() {
        let plan = BreadcrumbPlan.make(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "working", agent: "")])
        #expect(plan?.paneTitle == "zsh")
        #expect(plan?.paneSignal == .working)
    }
}
