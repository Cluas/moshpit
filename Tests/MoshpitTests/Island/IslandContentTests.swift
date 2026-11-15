import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// Vibe Island content + multi-agent redesign:
///   - the stamp script's title derivation
///   - the `@moshpit_title` → `Agent.detail` schema (backward-compatible decode)
@Suite("Vibe Island content + agents")
struct IslandContentTests {

    typealias Agent = AgentActivityAttributes.Agent

    // The stamp script's title derivation is pinned with the script, in the
    // core: crates/install/tests/stamp.rs.

    // MARK: - A swiped island comes back for a new question (B665)

    /// B665: an island the user swiped away stayed away while any agent was
    /// listed — a new "needs you" included. Work, a finish, a question it
    /// already showed keep it away; a question it did not show brings it
    /// back, and so does the same agent asking again.
    @Test("a swiped island comes back for a new question, and only for one")
    func swipedIslandReturnsForANewQuestion() {
        typealias State = AgentActivityAttributes.ContentState
        let at = { (seconds: Double) in Date(timeIntervalSince1970: 1_800_000_000 + seconds) }
        let agent = { (pane: String, state: AgentActivityAttributes.AgentState, since: Double) in
            Agent(id: "C:\(pane)", connectionId: "C", paneId: pane, command: "claude",
                  location: "mac", state: state, startedAt: at(since))
        }
        let swiped = State(agents: [agent("%1", .attention, 0), agent("%2", .working, 0)],
                           workingCount: 1, attentionCount: 1)
        let shown = IslandReturn.questions(swiped)
        // The same question, work going on, a finish: it stays away.
        #expect(!IslandReturn.bringsBack(swiped, swipedWith: shown))
        let done = State(agents: [agent("%1", .attention, 0), agent("%2", .done, 60)],
                         workingCount: 0, attentionCount: 1)
        #expect(!IslandReturn.bringsBack(done, swipedWith: shown))
        // Another agent asks: back.
        let another = State(agents: [agent("%1", .attention, 0), agent("%2", .attention, 90)],
                            workingCount: 0, attentionCount: 2)
        #expect(IslandReturn.bringsBack(another, swipedWith: shown))
        // The same agent asks again, later: back.
        let again = State(agents: [agent("%1", .attention, 120)], workingCount: 0, attentionCount: 1)
        #expect(IslandReturn.bringsBack(again, swipedWith: shown))
        // Swiped with nothing asking: the first question brings it back.
        #expect(IslandReturn.bringsBack(swiped, swipedWith: []))
    }

    // MARK: - Agent.detail schema (backward compatible)

    @Test("Agent round-trips its detail")
    func agentDetailRoundTrip() throws {
        let agent = Agent(id: "c:%1", connectionId: "c", paneId: "%1",
                          command: "claude", location: "work · 1:dev",
                          detail: "Bash: npm install", state: .attention,
                          startedAt: Date(timeIntervalSince1970: 1_700_000_000))
        let data = try JSONEncoder().encode(agent)
        let back = try JSONDecoder().decode(Agent.self, from: data)
        #expect(back.detail == "Bash: npm install")
        #expect(back.state == .attention)
    }

    @Test("an activity payload from an older build (no detail key) still decodes")
    func agentDetailBackwardCompat() throws {
        // Encode a full agent, then strip the `detail` key to mimic a pre-redesign
        // ContentState, and confirm it decodes with detail == nil (no schema bump).
        let agent = Agent(id: "c:%2", connectionId: "c", paneId: "%2",
                          command: "cargo", location: "work · 2:build",
                          detail: "Bash: cargo test", state: .working,
                          startedAt: Date(timeIntervalSince1970: 1_700_000_000))
        let data = try JSONEncoder().encode(agent)
        var obj = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        obj.removeValue(forKey: "detail")
        let stripped = try JSONSerialization.data(withJSONObject: obj)
        let back = try JSONDecoder().decode(Agent.self, from: stripped)
        #expect(back.detail == nil)
        #expect(back.command == "cargo")
    }
}
