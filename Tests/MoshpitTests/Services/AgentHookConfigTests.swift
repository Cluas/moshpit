import Foundation
import Testing
@testable import Moshpit

/// Editing somebody else's agent config.
///
/// This is the riskiest code in the install engine: it rewrites a file the user
/// depends on, on a machine we cannot see. The old implementation did the same
/// edit as a jq program with a python fallback, pasted into a live shell, and was
/// untestable except by running it on a real host. These tests are the reason it
/// is worth having moved.
@Suite("Agent hook config")
struct AgentHookConfigTests {

    private func object(_ json: String) throws -> [String: Any] {
        try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    private func groups(_ json: String, _ event: String) throws -> [[String: Any]] {
        let hooks = try #require(try object(json)["hooks"] as? [String: Any])
        return (hooks[event] as? [[String: Any]]) ?? []
    }

    private func commands(_ json: String, _ event: String) throws -> [String] {
        try groups(json, event).flatMap { group in
            (group["hooks"] as? [[String: Any]] ?? []).compactMap { $0["command"] as? String }
        }
    }

    // MARK: - JSON family

    @Test("a fresh config gets all four events with the right states")
    func freshJSON() throws {
        let out = try AgentHookConfig.mergedJSON(existing: "", agent: "claude")
        #expect(try commands(out, "UserPromptSubmit") == ["sh ~/.moshpit/moshpit-stamp.sh working claude"])
        #expect(try commands(out, "PreToolUse") == ["sh ~/.moshpit/moshpit-stamp.sh working claude"])
        #expect(try commands(out, "Notification") == ["sh ~/.moshpit/moshpit-stamp.sh attention claude"])
        #expect(try commands(out, "Stop") == ["sh ~/.moshpit/moshpit-stamp.sh done claude"])
    }

    @Test("only the tool-scoped events carry a matcher")
    func matchers() throws {
        let out = try AgentHookConfig.mergedJSON(existing: "{}", agent: "claude")
        #expect(try groups(out, "PreToolUse").first?["matcher"] as? String == "*")
        #expect(try groups(out, "Notification").first?["matcher"] as? String == "*")
        #expect(try groups(out, "UserPromptSubmit").first?["matcher"] == nil)
        #expect(try groups(out, "Stop").first?["matcher"] == nil)
    }

    @Test("everything else in the file survives, including the user's own hooks")
    func preservesUserContent() throws {
        let existing = """
        {
          "model": "opus",
          "permissions": {"allow": ["Bash(git*)"]},
          "hooks": {
            "PreToolUse": [
              {"matcher": "Bash", "hooks": [{"type": "command", "command": "my-own-audit.sh"}]}
            ],
            "SessionStart": [
              {"hooks": [{"type": "command", "command": "greet.sh"}]}
            ]
          }
        }
        """
        let out = try AgentHookConfig.mergedJSON(existing: existing, agent: "claude")
        let root = try object(out)
        #expect(root["model"] as? String == "opus")
        #expect(root["permissions"] != nil)
        #expect(try commands(out, "PreToolUse").contains("my-own-audit.sh"))
        #expect(try commands(out, "PreToolUse").contains("sh ~/.moshpit/moshpit-stamp.sh working claude"))
        // An event Moshpit does not use must be left exactly alone.
        #expect(try commands(out, "SessionStart") == ["greet.sh"])
    }

    @Test("re-running replaces our hooks instead of stacking them")
    func idempotent() throws {
        let once = try AgentHookConfig.mergedJSON(existing: "{}", agent: "claude")
        let twice = try AgentHookConfig.mergedJSON(existing: once, agent: "claude")
        #expect(once == twice)
        #expect(try commands(twice, "PreToolUse").count == 1)

        // Re-registering for a DIFFERENT agent replaces ours too — one stamp per
        // pane is the contract, and two would fight over the same options.
        let switched = try AgentHookConfig.mergedJSON(existing: once, agent: "codex")
        #expect(try commands(switched, "Stop") == ["sh ~/.moshpit/moshpit-stamp.sh done codex"])
    }

    @Test("a config that will not parse is refused, never overwritten")
    func refusesBrokenJSON() {
        #expect(throws: (any Error).self) {
            try AgentHookConfig.mergedJSON(existing: "{\"model\": }", agent: "claude")
        }
        // The message has to tell the user what to do about it.
        do {
            _ = try AgentHookConfig.mergedJSON(existing: "{oops", agent: "claude")
            Issue.record("expected a failure")
        } catch {
            #expect(error.localizedDescription.contains("valid JSON"))
        }
    }

    @Test("a top level that is not an object is refused")
    func refusesNonObject() {
        #expect(throws: AgentHookConfig.Failure.notAnObject) {
            try AgentHookConfig.mergedJSON(existing: "[1,2,3]", agent: "claude")
        }
    }

    @Test("removal takes ours out and leaves theirs alone")
    func removalJSON() throws {
        let installed = try AgentHookConfig.mergedJSON(existing: """
        {"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "mine.sh"}]}]}}
        """, agent: "claude")
        let out = try AgentHookConfig.withoutMoshpitJSON(existing: installed)

        #expect(try commands(out, "PreToolUse") == ["mine.sh"])
        // Events where we were the only entry go away entirely rather than
        // leaving an empty array as litter.
        let hooks = try #require(try object(out)["hooks"] as? [String: Any])
        #expect(hooks["Stop"] == nil)
        #expect(hooks["Notification"] == nil)
    }

    @Test("removing from a config that only ever had our hooks leaves no hooks key")
    func removalEmptiesCleanly() throws {
        let installed = try AgentHookConfig.mergedJSON(existing: #"{"model":"opus"}"#, agent: "claude")
        let out = try AgentHookConfig.withoutMoshpitJSON(existing: installed)
        let root = try object(out)
        #expect(root["hooks"] == nil)
        #expect(root["model"] as? String == "opus")
    }

    @Test("output is deterministic, so a re-install produces no diff")
    func deterministic() throws {
        let a = try AgentHookConfig.mergedJSON(existing: #"{"b":1,"a":2}"#, agent: "claude")
        let b = try AgentHookConfig.mergedJSON(existing: #"{"a":2,"b":1}"#, agent: "claude")
        #expect(a == b, "key order in the input must not change the output")
        #expect(a.hasSuffix("\n"), "a config file should end with a newline")
        #expect(!a.contains("\\/"), "escaped slashes would mangle every path in the file")
    }

    // MARK: - Codex TOML

    @Test("the Codex block is fenced and uses PermissionRequest for attention")
    func freshTOML() {
        let out = AgentHookConfig.mergedTOML(existing: "", agent: "codex")
        #expect(out.contains(AgentHookConfig.tomlBegin))
        #expect(out.contains(AgentHookConfig.tomlEnd))
        #expect(out.contains("[[hooks.PermissionRequest]]"))
        #expect(out.contains(#"command = "sh ~/.moshpit/moshpit-stamp.sh attention codex""#))
        #expect(out.contains(#"command = "sh ~/.moshpit/moshpit-stamp.sh done codex""#))
        // Codex has no Notification event — registering one would be dead config.
        #expect(!out.contains("[[hooks.Notification]]"))
    }

    @Test("the user's TOML is preserved above our block, and re-running replaces it")
    func tomlPreservesAndReplaces() {
        let existing = """
        model = "gpt-5"

        [tools]
        web_search = true
        """
        let once = AgentHookConfig.mergedTOML(existing: existing, agent: "codex")
        #expect(once.hasPrefix("model = \"gpt-5\""))
        #expect(once.contains("web_search = true"))

        let twice = AgentHookConfig.mergedTOML(existing: once, agent: "codex")
        #expect(twice == once)
        #expect(twice.components(separatedBy: AgentHookConfig.tomlBegin).count == 2,
                "the fenced block must not stack")
    }

    @Test("removing the block restores the file to what the user had")
    func tomlRemoval() {
        let existing = "model = \"gpt-5\"\n"
        let installed = AgentHookConfig.mergedTOML(existing: existing, agent: "codex")
        #expect(AgentHookConfig.withoutMoshpitTOML(existing: installed) == existing)
        // And a file that never had a block is untouched.
        #expect(AgentHookConfig.withoutMoshpitTOML(existing: existing) == existing)
    }

    @Test("Codex declares its trust step, Claude has none")
    func trustStep() throws {
        // Verified against codex-cli 0.143.0 by asking it what it sees: our four
        // `[[hooks.X]]` tables come back with handlerType "command" and the
        // commands intact, so the SHAPE is right — and with trust satisfied they
        // really execute. But a fresh hook arrives untrusted and Codex will not
        // run an untrusted hook, which without saying so is "installed, shown as
        // Current, and nothing will ever fire".
        let codex = try #require(HookAgent.named("codex"))
        let step = try #require(codex.trustStep)
        #expect(step.contains("/hooks"), "the user needs the command, not just the news")
        #expect(HookAgent.named("claude")?.trustStep == nil)
        #expect(HookAgent.named("gemini")?.trustStep == nil)
    }

    @Test("the agent roster covers the formats and keeps Claude first")
    func roster() {
        #expect(HookAgent.all.first?.id == "claude")
        #expect(HookAgent.named("codex")?.format == .codexTOML)
        #expect(HookAgent.named("gemini")?.format == .claudeJSON)
        #expect(HookAgent.named("nope") == nil)
        #expect(HookAgent.named("claude")?.configHint == "~/.claude/settings.json")
    }
}
