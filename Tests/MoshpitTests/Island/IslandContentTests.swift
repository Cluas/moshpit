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
