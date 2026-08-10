import Foundation
import Testing
@testable import Moshpit

/// Vibe Island content + multi-agent redesign:
///   - the per-agent hook installer (`IslandHooksInstallView.installCommand`)
///   - the `@moshpit_title` → `Agent.detail` schema (backward-compatible decode)
@Suite("Vibe Island content + agents")
struct IslandContentTests {

    typealias Agent = AgentActivityAttributes.Agent

    // MARK: - Per-agent installer

    @Test("the agent roster covers Claude, Codex and the JSON family")
    func roster() {
        let ids = IslandHooksInstallView.IslandAgent.all.map(\.id)
        #expect(ids.first == "claude")                 // default selection
        #expect(ids.contains("codex"))
        #expect(ids.contains("gemini"))
        // The four Claude-format forks ride the same jq merge for free.
        for fork in ["qwen", "qoder", "factory", "codebuddy"] {
            #expect(ids.contains(fork))
        }
        // Exactly one Codex (TOML); everything else is JSON-settings.
        let codex = IslandHooksInstallView.IslandAgent.all.filter { $0.format == .codexTOML }
        #expect(codex.count == 1 && codex.first?.id == "codex")
    }

    @Test("configHint shows ~ instead of the raw $HOME path")
    func configHint() {
        let claude = IslandHooksInstallView.IslandAgent.all.first { $0.id == "claude" }!
        #expect(claude.configHint == "~/.claude/settings.json")
        #expect(!claude.configHint.contains("$HOME"))
    }

    @Test("JSON-family command targets the agent's config and stamps its name")
    func jsonCommand() {
        let gemini = IslandHooksInstallView.IslandAgent.all.first { $0.id == "gemini" }!
        let cmd = IslandHooksInstallView.installCommand(for: gemini)
        // wrapped as a single pasteable line
        #expect(cmd.hasPrefix("sh -c '"))
        // writes the shared stamp script + reads stdin titles via jq
        #expect(cmd.contains("moshpit-stamp.sh"))
        #expect(cmd.contains("@moshpit_title"))
        #expect(cmd.contains("hook_event_name"))
        // merges into the gemini settings path, not claude's
        #expect(cmd.contains("$HOME/.gemini/settings.json"))
        #expect(!cmd.contains(".claude/settings.json"))
        // hooks stamp the gemini agent name
        #expect(cmd.contains("working gemini"))
        #expect(cmd.contains("attention gemini"))
        #expect(cmd.contains("done gemini"))
        // backs up before editing + has a jq path and a python3 fallback
        #expect(cmd.contains(".moshpit.bak."))
        #expect(cmd.contains("command -v jq"))
        #expect(cmd.contains("python3"))
    }

    @Test("Codex command appends an idempotent TOML block with PermissionRequest")
    func codexCommand() {
        let codex = IslandHooksInstallView.IslandAgent.all.first { $0.id == "codex" }!
        let cmd = IslandHooksInstallView.installCommand(for: codex)
        #expect(cmd.contains("$HOME/.codex/config.toml"))
        // TOML array-of-tables, not jq
        #expect(cmd.contains("[[hooks.PreToolUse]]"))
        #expect(cmd.contains("[[hooks.PermissionRequest]]"))  // Codex's attention event
        #expect(cmd.contains("[[hooks.Stop]]"))
        #expect(cmd.contains("attention codex"))
        // idempotent: strips a prior marked block before re-appending
        #expect(cmd.contains("# >>> moshpit vibe island >>>"))
        #expect(cmd.contains("# <<< moshpit vibe island <<<"))
        #expect(cmd.contains("sed -i"))
    }

    @Test("the stamp script derives a title from each hook event")
    func stampTitleEvents() {
        // The shared script is embedded in every install command; assert it
        // extracts the fields we display for each event.
        let cmd = IslandHooksInstallView.installCommand(
            for: IslandHooksInstallView.IslandAgent.all[0])
        #expect(cmd.contains("PreToolUse|PermissionRequest"))  // tool + arg title
        #expect(cmd.contains(".tool_input.command"))
        #expect(cmd.contains(".message"))                      // Notification title
        #expect(cmd.contains(".prompt"))                       // UserPromptSubmit title
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
