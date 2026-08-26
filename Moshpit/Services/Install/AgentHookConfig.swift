import Foundation

/// A coding agent Moshpit can register hooks for.
///
/// Moved out of the install sheet because the engine needs it and the sheet is
/// being replaced. The stamp script is SHARED — agents differ only in where and
/// how their lifecycle hooks are registered — and the read side is
/// agent-agnostic: anything that stamps `@moshpit_*` shows up on the island, so
/// adding an agent is one row here plus, for a new config format, a branch in
/// ``AgentHookConfig``.
struct HookAgent: Identifiable, Hashable, Sendable {
    enum Format: Sendable { case claudeJSON, codexTOML }

    /// Stamp argument and `@moshpit_agent` label, e.g. "claude".
    let id: String
    let displayName: String
    /// Shell expression, expanded on the host.
    let configPath: String
    let format: Format

    /// Human-readable config location, for UI.
    var configHint: String { configPath.replacingOccurrences(of: "$HOME", with: "~") }

    /// A step the USER has to take before these hooks will ever run, if there is
    /// one.
    ///
    /// Codex has this and Claude Code does not. Verified against codex-cli
    /// 0.143.0: it reads our `[[hooks.EventName]]` tables and lists all four
    /// events back with `handlerType: "command"` and the commands intact — the
    /// shape is right, and with trust satisfied they really do execute. But a
    /// newly written hook arrives `trustStatus: "untrusted"`, and Codex will not
    /// run an untrusted hook. Without saying so, installing for Codex produces
    /// exactly the failure this whole engine exists to remove: config present,
    /// script present, sheet reporting "Current", and nothing that will ever
    /// fire.
    ///
    /// The trust is recorded against a hash of the hook entry as written in the
    /// config, so updating the stamp SCRIPT does not un-trust it; changing the
    /// command line would.
    var trustStep: String? {
        switch format {
        case .claudeJSON: return nil
        case .codexTOML:
            return String(localized: "Codex won't run a newly added hook until you trust it. Run /hooks inside Codex on this host and trust the four Moshpit entries — until you do, nothing will be sent.")
        }
    }

    /// JSON-settings agents share Claude Code's hook schema AND its stdin
    /// payload, so one merge covers them all — only the path and the label
    /// change. Codex uses TOML array-of-tables with the same stdin payload, so
    /// the stamp script is reused as-is.
    static let all: [HookAgent] = [
        .init(id: "claude",    displayName: "Claude Code", configPath: "$HOME/.claude/settings.json",    format: .claudeJSON),
        .init(id: "codex",     displayName: "Codex",       configPath: "$HOME/.codex/config.toml",       format: .codexTOML),
        .init(id: "gemini",    displayName: "Gemini CLI",  configPath: "$HOME/.gemini/settings.json",    format: .claudeJSON),
        .init(id: "qwen",      displayName: "Qwen Code",   configPath: "$HOME/.qwen/settings.json",      format: .claudeJSON),
        .init(id: "qoder",     displayName: "Qoder",       configPath: "$HOME/.qoder/settings.json",     format: .claudeJSON),
        .init(id: "factory",   displayName: "Factory",     configPath: "$HOME/.factory/settings.json",   format: .claudeJSON),
        .init(id: "codebuddy", displayName: "CodeBuddy",   configPath: "$HOME/.codebuddy/settings.json", format: .claudeJSON),
    ]

    static func named(_ id: String) -> HookAgent? { all.first { $0.id == id } }
}

/// Editing an agent's own config to register (or unregister) Moshpit's hooks.
///
/// Every function here is pure: config text in, config text out. That is the
/// whole point of moving this off the host. The flow it replaces did the same
/// edit with a jq program, a python3 fallback for hosts without jq, and a
/// heredoc to carry both — roughly half of the 6.5 KB it pasted into a live
/// shell, and untestable except by running it on someone's machine. Here the
/// merge is a unit test, and the host only needs `cat` and `base64 -d`.
enum AgentHookConfig {

    /// The four events Moshpit registers, and the state each stamps.
    ///
    /// `matcher: "*"` exists only on the two tool-scoped events; the other two
    /// are session-scoped and take no matcher. Gemini has no `Notification`
    /// event at all — that group is simply inert there, and attention falls back
    /// to the bell heuristic.
    static let events: [(event: String, state: String, matched: Bool)] = [
        ("UserPromptSubmit", "working",   false),
        ("PreToolUse",       "working",   true),
        ("Notification",     "attention", true),
        ("Stop",             "done",      false),
    ]

    /// How a registered hook invokes the stamp script. The substring the
    /// de-duplicator recognises as ours.
    static let marker = "moshpit-stamp.sh"

    static func command(state: String, agent: String) -> String {
        "sh ~/.moshpit/moshpit-stamp.sh \(state) \(agent)"
    }

    enum Failure: LocalizedError, Equatable {
        case unparseableJSON(String)
        case notAnObject

        var errorDescription: String? {
            switch self {
            case .unparseableJSON(let detail):
                return String(localized: "That config file isn't valid JSON (\(detail)). Fix it on the host, or move it aside and Moshpit will write a fresh one.")
            case .notAnObject:
                return String(localized: "That config file's top level isn't a JSON object, so Moshpit can't merge hooks into it.")
            }
        }
    }

    // MARK: - JSON family

    /// Merge Moshpit's hook groups into a Claude-format settings file.
    ///
    /// Idempotent: any group whose commands mention the stamp script is dropped
    /// before ours is appended, so re-running replaces rather than stacks. Every
    /// other key — and every hook the user registered themselves — is preserved.
    ///
    /// One honest trade-off: the file is re-serialised, so object keys come back
    /// SORTED rather than in the order the user had them. `JSONSerialization`
    /// exposes no order-preserving path, and a hand-rolled order-preserving JSON
    /// parser is a worse thing to have in the blast radius of somebody's agent
    /// config than a stable reordering. Output is deterministic, so this costs
    /// one diff on first install and none afterwards — and the pre-Moshpit
    /// original is kept beside it (see ``HostCommands/backupOnce``).
    static func mergedJSON(existing: String, agent: String) throws -> String {
        var root: [String: Any] = [:]
        let trimmed = existing.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            let data = Data(trimmed.utf8)
            let parsed: Any
            do {
                parsed = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
            } catch {
                // Never silently overwrite: a syntax error here is a file the
                // user cares about, and the old installer's jq simply failed and
                // left the sheet saying what it always said.
                throw Failure.unparseableJSON(error.localizedDescription)
            }
            guard let object = parsed as? [String: Any] else { throw Failure.notAnObject }
            root = object
        }

        var hooks = root["hooks"] as? [String: Any] ?? [:]
        for (event, state, matched) in events {
            var groups = (hooks[event] as? [[String: Any]] ?? []).filter { !isOurs($0) }
            var group: [String: Any] = [
                "hooks": [["type": "command", "command": command(state: state, agent: agent)]],
            ]
            if matched { group["matcher"] = "*" }
            groups.append(group)
            hooks[event] = groups
        }
        root["hooks"] = hooks
        return try serialise(root)
    }

    /// Remove Moshpit's hook groups, leaving everything else — including hooks
    /// the user added — untouched.
    ///
    /// Surgical rather than "restore the backup", because a config the user has
    /// edited since installing must not lose those edits to an uninstall.
    static func withoutMoshpitJSON(existing: String) throws -> String {
        let trimmed = existing.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return existing }
        guard let root = try? JSONSerialization.jsonObject(with: Data(trimmed.utf8)) as? [String: Any]
        else { throw Failure.unparseableJSON("could not be read for removal") }

        var out = root
        guard var hooks = root["hooks"] as? [String: Any] else { return try serialise(out) }
        for event in hooks.keys {
            guard let groups = hooks[event] as? [[String: Any]] else { continue }
            let kept = groups.filter { !isOurs($0) }
            // Drop the event entirely when we were its only entry, rather than
            // leaving an empty array behind as litter.
            if kept.isEmpty { hooks.removeValue(forKey: event) } else { hooks[event] = kept }
        }
        if hooks.isEmpty { out.removeValue(forKey: "hooks") } else { out["hooks"] = hooks }
        return try serialise(out)
    }

    /// True when this hook group is one Moshpit registered.
    private static func isOurs(_ group: [String: Any]) -> Bool {
        guard let entries = group["hooks"] as? [[String: Any]] else { return false }
        return entries.contains { ($0["command"] as? String)?.contains(marker) == true }
    }

    private static func serialise(_ root: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(
            withJSONObject: root, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        return (String(data: data, encoding: .utf8) ?? "{}") + "\n"
    }

    // MARK: - Codex TOML

    static let tomlBegin = "# >>> moshpit vibe island >>>"
    static let tomlEnd = "# <<< moshpit vibe island <<<"

    /// Append (or replace) Moshpit's block in a Codex config.
    ///
    /// TOML array-of-tables cannot be merged structurally without a TOML
    /// library, and pulling one in to append four fixed tables would be the
    /// wrong trade. A fenced block is honest instead: the user can see exactly
    /// what Moshpit owns, and removal is exact rather than a guess.
    static func mergedTOML(existing: String, agent: String) -> String {
        var body = withoutMoshpitTOML(existing: existing)
        if !body.isEmpty && !body.hasSuffix("\n") { body += "\n" }
        if !body.isEmpty { body += "\n" }

        var block = "\(tomlBegin)\n"
        for (event, state, _) in events where event != "Notification" {
            block += """
            [[hooks.\(event)]]
            [[hooks.\(event).hooks]]
            type = "command"
            command = "\(command(state: state, agent: agent))"

            """
        }
        // Codex's attention signal is PermissionRequest, not Notification.
        block += """
        [[hooks.PermissionRequest]]
        [[hooks.PermissionRequest.hooks]]
        type = "command"
        command = "\(command(state: "attention", agent: agent))"

        """
        block += "\(tomlEnd)\n"
        return body + block
    }

    /// Strip Moshpit's fenced block, and nothing else.
    static func withoutMoshpitTOML(existing: String) -> String {
        var kept: [String] = []
        var inside = false
        for line in existing.split(separator: "\n", omittingEmptySubsequences: false) {
            let text = String(line)
            if text.trimmingCharacters(in: .whitespaces) == tomlBegin { inside = true; continue }
            if text.trimmingCharacters(in: .whitespaces) == tomlEnd { inside = false; continue }
            if !inside { kept.append(text) }
        }
        while kept.last?.trimmingCharacters(in: .whitespaces).isEmpty == true { kept.removeLast() }
        let body = kept.joined(separator: "\n")
        return body.isEmpty ? "" : body + "\n"
    }

    /// Merge for whichever format this agent uses.
    static func merged(existing: String, agent: HookAgent) throws -> String {
        switch agent.format {
        case .claudeJSON: return try mergedJSON(existing: existing, agent: agent.id)
        case .codexTOML:  return mergedTOML(existing: existing, agent: agent.id)
        }
    }

    static func removed(existing: String, agent: HookAgent) throws -> String {
        switch agent.format {
        case .claudeJSON: return try withoutMoshpitJSON(existing: existing)
        case .codexTOML:  return withoutMoshpitTOML(existing: existing)
        }
    }
}
