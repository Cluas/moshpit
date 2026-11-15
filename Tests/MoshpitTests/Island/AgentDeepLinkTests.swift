import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// `moshpit://connection/<uuid>?pane=<paneId>` is written by the app (Live
/// Activity, widget snapshot) and read back by the app, the share extension and
/// the Shortcuts intent. tmux pane ids start with `%`, which is exactly the
/// percent-escape character — the link has to survive that round trip.
@Suite("AgentDeepLink")
struct AgentDeepLinkTests {
    let id = UUID(uuidString: "3F2504E0-4F89-11D3-9A0C-0305E82C3301")!

    @Test("pane ids round-trip, including two-digit ones that look like escapes",
          arguments: ["%1", "%5", "%12", "%40", "%123", "w1 p2"])
    func roundTrip(pane: String) throws {
        let link = AgentDeepLink.string(connectionId: id, paneId: pane)
        let parsed = try #require(AgentDeepLink.parse(link))
        #expect(parsed.connectionId == id)
        #expect(parsed.paneId == pane)
    }

    /// B702: a tap on the lock-screen card only opened the app — the
    /// `widgetURL` was the Dynamic Island's alone. The card opens its first
    /// row's pane, the most urgent agent's, as the Home Screen widget does.
    @Test("the lock-screen card opens its first row's pane")
    func lockScreenCardOpensItsFirstRow() throws {
        typealias Agent = AgentActivityAttributes.Agent
        let agents = [
            Agent(id: "\(id.uuidString):%12", connectionId: id.uuidString, paneId: "%12",
                  command: "claude", location: "mac", state: .attention, startedAt: Date()),
            Agent(id: "\(id.uuidString):%3", connectionId: id.uuidString, paneId: "%3",
                  command: "codex", location: "mac", state: .working, startedAt: Date()),
        ]
        // The headline cycled to the second: the card still opens the first.
        let state = AgentActivityAttributes.ContentState(agents: agents, workingCount: 1, attentionCount: 1,
                                                         headlineId: agents[1].id)
        let url = try #require(LockScreenView.deepLink(for: state))
        let parsed = try #require(AgentDeepLink.parse(url.absoluteString))
        #expect(parsed.connectionId == id && parsed.paneId == "%12")
        let none = AgentActivityAttributes.ContentState(agents: [], workingCount: 0, attentionCount: 0)
        #expect(LockScreenView.deepLink(for: none) == nil)
    }

    @Test("the uuid is the path, not the host")
    func uuidIsThePath() throws {
        let url = try #require(URL(string: AgentDeepLink.string(connectionId: id, paneId: "%3")))
        #expect(url.host() == "connection")
        #expect(url.lastPathComponent == id.uuidString)
        // Not a connection link: the uuid in the host slot is rejected.
        #expect(AgentDeepLink.parse("moshpit://\(id.uuidString)?pane=%253") == nil)
    }

    @Test("links written before the encoder existed still land on their pane",
          arguments: ["%1", "%12", "%123"])
    func legacyUnencodedPane(pane: String) throws {
        let parsed = try #require(AgentDeepLink.parse("moshpit://connection/\(id.uuidString)?pane=\(pane)"))
        #expect(parsed.paneId == pane)
    }

    @Test("a link without a pane still names the connection")
    func noPane() throws {
        let parsed = try #require(AgentDeepLink.parse("moshpit://connection/\(id.uuidString)"))
        #expect(parsed.connectionId == id)
        #expect(parsed.paneId == nil)
    }

    @Test("other schemes are refused unless the caller allows them")
    func schemes() throws {
        let url = try #require(URL(string: "moshpitdemo://connection/\(id.uuidString)?pane=%2512"))
        #expect(AgentDeepLink.parse(url) == nil)
        #expect(AgentDeepLink.parse(url, schemes: ["moshpit", "moshpitdemo"])?.paneId == "%12")
    }

    @Test("a widget row's target is what the share sheet and Shortcuts address")
    func widgetItemTarget() throws {
        let item = AgentWidgetState.Item(
            id: "x", command: "claude", location: "work · 4:rugisland",
            state: "attention", startedAt: .now,
            deepLink: AgentDeepLink.string(connectionId: id, paneId: "%12"))
        let target = try #require(item.target)
        #expect(target.connectionId == id)
        #expect(target.paneId == "%12")

        let noLink = AgentWidgetState.Item(id: "y", command: "claude", location: "",
                                           state: "working", startedAt: .now)
        #expect(noLink.target == nil)
    }

    @Test("the app's URL handler requests the exact pane")
    @MainActor
    func routerRequestsPane() throws {
        let router = DeepLinkRouter()
        let url = try #require(URL(string: AgentDeepLink.string(connectionId: id, paneId: "%12")))
        router.handle(url)
        #expect(router.paneRequest?.connectionId == id)
        #expect(router.paneRequest?.paneId == "%12")
    }
}

/// B468: a notification for a pane that was closed since (or a stale id)
/// left its landing waiting for the screen's whole life, and the first pane
/// to take that id later (herdr numbers panes anew after a restart) pulled
/// the screen there out of nowhere. A pane missing from an attached
/// snapshot for `landingPatience` is dropped.
@Suite("landing on a pane (B468)")
struct LandingStepTests {
    @Test("Not attached yet: wait, however long it has been")
    func waitsForTheAttach() {
        #expect(TerminalScreen.landingStep(present: false, attached: false, attachedFor: nil) == .wait)
        #expect(TerminalScreen.landingStep(present: true, attached: false, attachedFor: nil) == .wait)
    }

    @Test("The pane is there: go")
    func selectsAPaneThatIsThere() {
        #expect(TerminalScreen.landingStep(present: true, attached: true, attachedFor: 0) == .select)
        #expect(TerminalScreen.landingStep(present: true, attached: true, attachedFor: 60) == .select)
    }

    @Test("Missing: wait while the pane list may still fill, then drop it")
    func dropsAPaneThatNeverComes() {
        let patience = TerminalScreen.landingPatience
        #expect(TerminalScreen.landingStep(present: false, attached: true, attachedFor: 0) == .wait)
        #expect(TerminalScreen.landingStep(present: false, attached: true, attachedFor: patience - 1) == .wait)
        #expect(TerminalScreen.landingStep(present: false, attached: true, attachedFor: patience) == .drop)
        #expect(TerminalScreen.landingStep(present: false, attached: true, attachedFor: nil) == .wait,
                "attached just now: its patience starts")
    }
}

/// B492: a landing whose pane is gone was dropped without a word: the
/// person landed on another pane and took the notification for wrong.
/// Now it says so for a moment; the multiplexer's own notice wins.
@Suite("a gone pane's landing is said (B492)")
struct LandingSaidTests {
    @Test("A one-off shows when nothing else is said, and gives way to the multiplexer's notice")
    func aOneOffGivesWay() {
        #expect(TerminalScreen.shownNotice(multiplexer: nil, flashed: "That pane is closed") == "That pane is closed")
        #expect(TerminalScreen.shownNotice(multiplexer: "Refused", flashed: "That pane is closed") == "Refused")
        #expect(TerminalScreen.shownNotice(multiplexer: nil, flashed: nil) == nil)
        #expect(TerminalScreen.flashSeconds > 0 && TerminalScreen.flashSeconds <= 5, "a moment")
    }
}

/// B661: the island's "+N more" opens Home's "Needs you", where every agent
/// waiting is listed: the link is told apart from a pane's, and the router
/// asks Home for it, not for a pane.
@Suite("Needs-you link")
@MainActor
struct NeedsYouLinkTests {
    @Test("the needs-you link routes to Home, not to a pane")
    func needsYouRoutesHome() throws {
        let url = try #require(URL(string: AgentDeepLink.needsYou))
        #expect(AgentDeepLink.isNeedsYou(url))
        #expect(AgentDeepLink.parse(url) == nil, "not a pane's link")
        let pane = try #require(URL(string: AgentDeepLink.string(connectionId: UUID(), paneId: "%1")))
        #expect(!AgentDeepLink.isNeedsYou(pane))
        let router = DeepLinkRouter()
        router.handle(url)
        #expect(router.needsYouRequest == 1)
        #expect(router.paneRequest == nil)
        router.handle(url)
        #expect(router.needsYouRequest == 2, "a second tap scrolls again")
    }
}
