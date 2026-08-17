import Foundation

/// Decodes `herdr api snapshot` into the app's own model.
///
/// Pure and IO-free on purpose: everything here is exercised by unit tests
/// against a real captured snapshot (`MoshpitTests/Fixtures/herdr-snapshot.json`).
///
/// ### The mapping
///
/// herdr's three levels line up one-for-one with Moshpit's, so the sheets and
/// the home tree need no herdr-specific code:
///
/// | herdr     | Moshpit      | id form   |
/// |-----------|---------------|-----------|
/// | workspace | ``SessionInfo`` | `w1`      |
/// | tab       | ``WindowInfo``  | `w1:t1`   |
/// | pane      | ``PaneInfo``    | `w1:p1`   |
///
/// ### Why every field is optional
///
/// herdr's payload grows between releases and the app must run against
/// whatever the user has installed. Verified live: 0.7.3 (protocol 16) omits
/// `title`, `terminal_title`, `agent` and `label` from its pane objects
/// entirely, all of which exist in 0.8.0 (protocol 19). So this decodes
/// defensively — a missing field degrades that one detail, it never fails the
/// whole snapshot. The rule to keep: **never add a non-optional field here.**
enum HerdrSnapshot {

    /// Result of decoding: the tree plus the per-pane agent stamps that drive
    /// the Vibe Island dots. Kept together because both come from one payload.
    struct Decoded: Equatable {
        var snapshot: TmuxSnapshot
        var agentHooks: [String: AgentHook]
        /// Working directory per pane. Not part of `TmuxSnapshot` because tmux
        /// has no equivalent in ours — it's carried alongside so "which repos
        /// is this host working on" can be answered without another round trip
        /// (see `HerdrControlClient.gitRoots`).
        var paneCwds: [String: String] = [:]
        /// Workspaces that ARE a git worktree, mapped to the repo they came
        /// from. Only these can be removed as a task; a plain workspace has
        /// no checkout to delete.
        var worktreeRepos: [String: String] = [:]
    }

    /// Decode one `herdr api snapshot` stdout blob.
    ///
    /// Returns `nil` when the payload isn't a snapshot at all — no server
    /// running (`{"error":{"code":"server_not_running"}}`), a truncated read,
    /// or shell noise mixed into stdout. `nil` means "no new information";
    /// callers keep the snapshot they already had rather than blanking the UI.
    static func decode(_ raw: String) -> Decoded? {
        guard let json = firstJSONObject(in: raw) else { return nil }
        // Responses are envelopes — {"id":…,"result":{"snapshot":{…}}} — not
        // bare snapshots. Tolerate a bare one too in case a future CLI drops
        // the envelope.
        let body = ((json["result"] as? [String: Any])?["snapshot"] as? [String: Any])
            ?? (json["snapshot"] as? [String: Any])
            ?? json
        guard let workspaces = body["workspaces"] as? [[String: Any]] else { return nil }
        let tabs = body["tabs"] as? [[String: Any]] ?? []
        let panes = body["panes"] as? [[String: Any]] ?? []
        let layouts = body["layouts"] as? [[String: Any]] ?? []

        var snapshot = TmuxSnapshot()
        var worktreeRepos: [String: String] = [:]
        for ws in workspaces {
            guard let id = ws["workspace_id"] as? String else { continue }
            // 0.7.3 carries this; anything without it is an ordinary workspace.
            if let worktree = ws["worktree"] as? [String: Any],
               (worktree["is_linked_worktree"] as? Bool) == true {
                worktreeRepos[id] = (worktree["repo_name"] as? String)
                    ?? (worktree["checkout_path"] as? String) ?? id
            }
            snapshot.sessions[id] = SessionInfo(
                id: id,
                name: (ws["label"] as? String) ?? id,
                // herdr has exactly one attached UI per server, so "attached"
                // is the same question as "focused".
                isAttached: (ws["focused"] as? Bool) ?? false)
        }

        // Pane geometry and zoom live in `layouts`, one entry per tab, not on
        // the pane objects themselves.
        var rects: [String: (width: Int, height: Int)] = [:]
        var zoomedTabs: Set<String> = []
        for layout in layouts {
            if (layout["zoomed"] as? Bool) == true, let tabId = layout["tab_id"] as? String {
                zoomedTabs.insert(tabId)
            }
            for pane in layout["panes"] as? [[String: Any]] ?? [] {
                guard let id = pane["pane_id"] as? String,
                      let rect = pane["rect"] as? [String: Any] else { continue }
                rects[id] = (width: (rect["width"] as? Int) ?? 0,
                             height: (rect["height"] as? Int) ?? 0)
            }
        }

        for tab in tabs {
            guard let id = tab["tab_id"] as? String else { continue }
            snapshot.windows[id] = WindowInfo(
                id: id,
                sessionId: (tab["workspace_id"] as? String) ?? "",
                name: (tab["label"] as? String) ?? id,
                index: (tab["number"] as? Int) ?? 0,
                // herdr has no tmux-style layout string; `TmuxLayoutParser`
                // never sees a herdr window. Geometry comes from `layouts`.
                layout: "",
                isActive: (tab["focused"] as? Bool) ?? false,
                paneCount: (tab["pane_count"] as? Int) ?? 1,
                isZoomed: zoomedTabs.contains(id))
        }

        // herdr 0.8 moved the agent's IDENTITY off the pane objects: a pane
        // still carries `agent_status`, but `agent`/`display_agent` now live
        // only in a top-level `agents` array keyed by pane_id. Without this
        // overlay a running claude decoded to a hook with a state and no
        // name, and the Home agents tree rendered nothing (user report,
        // 2026-08-17, first device run against a 0.8 server).
        var agentsByPane: [String: [String: Any]] = [:]
        for agent in body["agents"] as? [[String: Any]] ?? [] {
            guard let paneId = agent["pane_id"] as? String else { continue }
            agentsByPane[paneId] = agent
        }

        var hooks: [String: AgentHook] = [:]
        var cwds: [String: String] = [:]
        for pane in panes {
            guard let id = pane["pane_id"] as? String else { continue }
            // `foreground_cwd` follows `cd` inside the shell; `cwd` is where
            // the pane started. Prefer the live one.
            if let cwd = (pane["foreground_cwd"] as? String) ?? (pane["cwd"] as? String),
               !cwd.isEmpty {
                cwds[id] = cwd
            }
            let size = rects[id]
            snapshot.panes[id] = PaneInfo(
                id: id,
                windowId: (pane["tab_id"] as? String) ?? "",
                index: paneIndex(id),
                command: command(from: pane),
                width: size?.width ?? 80,
                height: size?.height ?? 24,
                isActive: (pane["focused"] as? Bool) ?? false)
            if let hook = agentHook(from: pane, agentEntry: agentsByPane[id]) { hooks[id] = hook }
        }

        snapshot.activeSessionId = body["focused_workspace_id"] as? String
        snapshot.activeWindowId = body["focused_tab_id"] as? String
        snapshot.activePaneId = body["focused_pane_id"] as? String
        // A snapshot that names a workspace IS the attached state — there is
        // no separate attach handshake the way tmux's `%session-changed` is.
        snapshot.isAttached = !snapshot.sessions.isEmpty
        snapshot.everAttached = snapshot.isAttached

        return Decoded(snapshot: snapshot, agentHooks: hooks,
                       paneCwds: cwds, worktreeRepos: worktreeRepos)
    }

    // MARK: - Field mapping

    /// Pane index within its window, taken from the id's own numbering
    /// (`w1:p3` → 3). herdr panes carry no index field, and the app sorts panes
    /// by it — deriving it from the id keeps that order stable across polls,
    /// which array position would not.
    static func paneIndex(_ paneId: String) -> Int {
        guard let marker = paneId.range(of: ":p") else { return 0 }
        return Int(paneId[marker.upperBound...]) ?? 0
    }

    /// What to show as the pane's running command. herdr reports a detected
    /// agent name where it can (that's the whole point of it), so prefer that
    /// over the raw terminal title. Every one of these is absent on 0.7.3.
    private static func command(from pane: [String: Any]) -> String {
        for key in ["display_agent", "agent", "terminal_title_stripped", "title"] {
            if let value = pane[key] as? String, !value.isEmpty { return value }
        }
        return ""
    }

    /// Translate herdr's `agent_status` into the stamp vocabulary the sheets
    /// and the Vibe Island already speak (`working` / `attention` / `done`).
    ///
    /// This is the payoff of the whole herdr path: tmux needs host-side hooks
    /// writing `@moshpit_*` options to produce these, herdr just reports it.
    /// `unknown` deliberately produces no stamp — same as a tmux pane with no
    /// hook installed — so nothing lights up without cause. `idle` passes
    /// through by its own name: it never lights a dot or the island
    /// (`AgentSignal` and the monitor's `hookState` both map it to nothing),
    /// but the Agents section shows a named idle agent as a quiet row — an
    /// agent sitting at its prompt is one you can hand work to, and a list
    /// that hides it reads as "nothing here" to the person who just saw
    /// claude running in that pane.
    private static func agentHook(from pane: [String: Any],
                                  agentEntry: [String: Any]? = nil) -> AgentHook? {
        let state: String?
        // 0.8's agents-array entry carries its own agent_status too; prefer
        // it, since the pane-level one can lag a revision behind.
        switch (agentEntry?["agent_status"] as? String) ?? (pane["agent_status"] as? String) {
        case "working": state = "working"
        case "blocked": state = "attention"
        case "done":    state = "done"
        case "idle":    state = "idle"
        default:        state = nil
        }
        // Identity: pane fields first (0.7), then the 0.8 agents-array entry.
        let agent = (pane["display_agent"] as? String) ?? (pane["agent"] as? String)
            ?? (agentEntry?["display_agent"] as? String) ?? (agentEntry?["agent"] as? String)
        let title = (pane["terminal_title_stripped"] as? String) ?? (pane["title"] as? String)
        guard state != nil || agent != nil || title != nil else { return nil }
        // No `since`: herdr reports no transition timestamp, so the monitor
        // stamps "now" the first time it sees a state change.
        return AgentHook(state: state, agent: agent, since: nil, title: title)
    }

    // MARK: - Parsing

    /// Pull the first top-level JSON object out of stdout.
    ///
    /// A remote shell can prepend its own noise (a profile's `echo`, an MOTD
    /// fragment) before the CLI's output, so this scans for the first `{` and
    /// tracks brace depth — string-aware, so a `{` inside a cwd or a label
    /// can't throw the count off — instead of assuming the blob starts clean.
    static func firstJSONObject(in raw: String) -> [String: Any]? {
        let scalars = Array(raw.utf8)
        guard let start = scalars.firstIndex(of: UInt8(ascii: "{")) else { return nil }
        var depth = 0
        var inString = false
        var escaped = false
        for index in start..<scalars.count {
            let byte = scalars[index]
            if escaped { escaped = false; continue }
            if inString {
                if byte == UInt8(ascii: "\\") { escaped = true }
                else if byte == UInt8(ascii: "\"") { inString = false }
                continue
            }
            switch byte {
            case UInt8(ascii: "\""): inString = true
            case UInt8(ascii: "{"): depth += 1
            case UInt8(ascii: "}"):
                depth -= 1
                if depth == 0 {
                    let data = Data(scalars[start...index])
                    return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
                }
            default: break
            }
        }
        return nil
    }

    /// True when the payload is herdr telling us no server is listening —
    /// the "you have no sessions yet" case, not a failure.
    static func isServerNotRunning(_ raw: String) -> Bool {
        guard let json = firstJSONObject(in: raw),
              let error = json["error"] as? [String: Any] else { return false }
        return (error["code"] as? String) == "server_not_running"
    }

    /// herdr's client/server version-skew refusal, worth naming to the user
    /// because its remedy is theirs alone: restart (or upgrade) the herdr
    /// server on the HOST. Seen live when a host's herdr binary was upgraded
    /// under a still-running older server (protocol 16 vs 19, herdr 0.7→0.8):
    /// every `api` call answers `{"error":{"code":"protocol_mismatch",…}}`,
    /// which decode() rejects — and without this check the UI called it
    /// "no server running" and offered to create a session that could never
    /// attach.
    static func protocolMismatch(_ raw: String) -> String? {
        guard let json = firstJSONObject(in: raw),
              let error = json["error"] as? [String: Any],
              (error["code"] as? String) == "protocol_mismatch" else { return nil }
        // First sentence only — the CLI's full message includes multi-line
        // remediation prose sized for a terminal, not a banner.
        let message = (error["message"] as? String) ?? "herdr client/server protocol mismatch"
        return message.components(separatedBy: ";").first ?? message
    }

    /// Whether a surfaced herdr message describes client/server version skew
    /// — the case whose remedy (restart the server) the app can offer as an
    /// action. Matches both wire shapes: the API's "client protocol 19 is
    /// newer than server protocol 16" and the attach CLI's plain-text
    /// "server rejected handshake (version 16): …".
    static func looksLikeVersionSkew(_ message: String) -> Bool {
        message.contains("rejected handshake")
            || (message.contains("protocol") && message.contains("newer"))
    }
}
