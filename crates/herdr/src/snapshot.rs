//! `herdr api snapshot`, decoded into Moshpit's own tree: a port of
//! `Moshpit/Services/Herdr/HerdrSnapshot.swift`.
//!
//! herdr's three levels line up one for one with the app's, so nothing above
//! this needs to know which multiplexer it is looking at:
//!
//! | herdr     | Moshpit  | id form |
//! |-----------|----------|---------|
//! | workspace | session  | `w1`    |
//! | tab       | window   | `w1:t1` |
//! | pane      | pane     | `w1:p1` |
//!
//! Every field is optional on purpose. herdr's payload grows between
//! releases and the app runs against whatever the user installed: 0.7.3
//! (protocol 16) omits `title`, `terminal_title`, `agent` and `label` from
//! its panes, all present in 0.8.0. A missing field degrades that one
//! detail. (A field of the wrong type does too; only `workspaces` that is
//! not an array of objects fails the whole snapshot, as the Swift cast
//! does.)

use std::collections::BTreeMap;

use crate::json::{self, Object};

/// A session (herdr workspace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub name: String,
    /// herdr has one attached UI per server, so attached = focused.
    pub is_attached: bool,
}

/// A window (herdr tab).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub index: i64,
    /// tmux's layout string; herdr has none (geometry is in `layouts`).
    pub layout: String,
    pub is_active: bool,
    pub pane_count: i64,
    pub is_zoomed: bool,
}

/// A pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    pub window_id: String,
    pub index: i64,
    pub command: String,
    pub width: i64,
    pub height: i64,
    pub is_active: bool,
}

/// The tree, in the app's vocabulary (`TmuxSnapshot` as herdr fills it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub sessions: BTreeMap<String, Session>,
    pub windows: BTreeMap<String, Window>,
    pub panes: BTreeMap<String, Pane>,
    pub active_session_id: Option<String>,
    pub active_window_id: Option<String>,
    pub active_pane_id: Option<String>,
    /// A snapshot that names a workspace IS the attached state: there is no
    /// separate attach handshake as with tmux's `%session-changed`.
    pub is_attached: bool,
    /// Latches once attached, so an emptied tree reads as "the last session
    /// ended", not as a first attach still in progress.
    pub ever_attached: bool,
}

impl Snapshot {
    /// Panes in the active window, by id.
    pub fn active_panes(&self) -> Vec<&Pane> {
        let Some(window) = &self.active_window_id else {
            return Vec::new();
        };
        self.panes
            .values()
            .filter(|pane| &pane.window_id == window)
            .collect()
    }

    /// A session's windows, by index (ties by id).
    pub fn windows_in_session(&self, session_id: &str) -> Vec<&Window> {
        let mut windows: Vec<&Window> = self
            .windows
            .values()
            .filter(|window| window.session_id == session_id)
            .collect();
        windows.sort_by_key(|window| window.index);
        windows
    }

    /// A window's panes, by index (ties by id).
    pub fn panes_in_window(&self, window_id: &str) -> Vec<&Pane> {
        let mut panes: Vec<&Pane> = self
            .panes
            .values()
            .filter(|pane| pane.window_id == window_id)
            .collect();
        panes.sort_by_key(|pane| pane.index);
        panes
    }
}

/// What an agent in a pane is doing, in the stamp vocabulary the sheets and
/// the island already speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Working,
    /// herdr's `blocked`: waiting on the user.
    Attention,
    Done,
    /// Lights nothing, but the Agents section lists a named idle agent as a
    /// quiet row: one you can hand work to.
    Idle,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Attention => "attention",
            AgentState::Done => "done",
            AgentState::Idle => "idle",
        }
    }
}

/// The per-pane agent stamp (`AgentHook`). herdr reports no transition
/// time, so there is no `since`: the monitor stamps the first sighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHook {
    pub state: Option<AgentState>,
    pub agent: Option<String>,
    pub title: Option<String>,
}

/// One decoded snapshot: the tree and what rides alongside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decoded {
    pub snapshot: Snapshot,
    pub agent_hooks: BTreeMap<String, AgentHook>,
    /// Working directory per pane (the live `foreground_cwd`, else `cwd`).
    pub pane_cwds: BTreeMap<String, String>,
    /// herdr terminal id per pane: what `herdr terminal attach` (the mosh
    /// renderer) resolves; it refuses pane ids.
    pub terminal_ids: BTreeMap<String, String>,
    /// Workspaces that are a git worktree, to the repo they came from: only
    /// these can be removed as a task.
    pub worktree_repos: BTreeMap<String, String>,
}

/// Decode one `herdr api snapshot` stdout blob. `None` when it isn't a
/// snapshot at all (no server, a truncated read, shell noise): "no new
/// information", never an empty tree.
pub fn decode(raw: &str) -> Option<Decoded> {
    let json = json::first_object(raw)?;
    // Replies are envelopes, {"id":…,"result":{"snapshot":{…}}}; a bare
    // snapshot is taken too, in case a future CLI drops the envelope.
    let body = json
        .object("result")
        .and_then(|result| result.object("snapshot"))
        .or_else(|| json.object("snapshot"))
        .unwrap_or(&json);
    let workspaces = body.objects("workspaces")?;
    let tabs = body.objects("tabs").unwrap_or_default();
    let panes = body.objects("panes").unwrap_or_default();
    let layouts = body.objects("layouts").unwrap_or_default();

    let mut decoded = Decoded::default();
    let snapshot = &mut decoded.snapshot;
    for workspace in workspaces {
        let Some(id) = workspace.str("workspace_id") else {
            continue;
        };
        if let Some(worktree) = workspace.object("worktree")
            && worktree.bool("is_linked_worktree") == Some(true)
        {
            let repo = worktree
                .str("repo_name")
                .or_else(|| worktree.str("checkout_path"))
                .unwrap_or(id);
            decoded
                .worktree_repos
                .insert(id.to_owned(), repo.to_owned());
        }
        snapshot.sessions.insert(
            id.to_owned(),
            Session {
                id: id.to_owned(),
                name: workspace.str("label").unwrap_or(id).to_owned(),
                is_attached: workspace.bool("focused").unwrap_or(false),
            },
        );
    }

    // Geometry and zoom live in `layouts`, one per tab.
    let mut rects: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    let mut zoomed_tabs: Vec<&str> = Vec::new();
    for layout in layouts {
        if layout.bool("zoomed") == Some(true)
            && let Some(tab) = layout.str("tab_id")
        {
            zoomed_tabs.push(tab);
        }
        for pane in layout.objects("panes").unwrap_or_default() {
            let (Some(id), Some(rect)) = (pane.str("pane_id"), pane.object("rect")) else {
                continue;
            };
            rects.insert(
                id,
                (
                    rect.i64("width").unwrap_or(0),
                    rect.i64("height").unwrap_or(0),
                ),
            );
        }
    }

    for tab in tabs {
        let Some(id) = tab.str("tab_id") else {
            continue;
        };
        snapshot.windows.insert(
            id.to_owned(),
            Window {
                id: id.to_owned(),
                session_id: tab.str("workspace_id").unwrap_or("").to_owned(),
                name: tab.str("label").unwrap_or(id).to_owned(),
                index: tab.i64("number").unwrap_or(0),
                layout: String::new(),
                is_active: tab.bool("focused").unwrap_or(false),
                pane_count: tab.i64("pane_count").unwrap_or(1),
                is_zoomed: zoomed_tabs.contains(&id),
            },
        );
    }

    // herdr 0.8 moved the agent's identity off the panes into a top-level
    // `agents` array keyed by pane id.
    let mut agents: BTreeMap<&str, &Object> = BTreeMap::new();
    for agent in body.objects("agents").unwrap_or_default() {
        if let Some(pane) = agent.str("pane_id") {
            agents.insert(pane, agent);
        }
    }

    for pane in panes {
        let Some(id) = pane.str("pane_id") else {
            continue;
        };
        // `foreground_cwd` follows `cd`; `cwd` is where the pane started.
        if let Some(cwd) = pane.str("foreground_cwd").or_else(|| pane.str("cwd"))
            && !cwd.is_empty()
        {
            decoded.pane_cwds.insert(id.to_owned(), cwd.to_owned());
        }
        if let Some(terminal) = pane.str("terminal_id")
            && !terminal.is_empty()
        {
            decoded
                .terminal_ids
                .insert(id.to_owned(), terminal.to_owned());
        }
        let size = rects.get(id);
        snapshot.panes.insert(
            id.to_owned(),
            Pane {
                id: id.to_owned(),
                window_id: pane.str("tab_id").unwrap_or("").to_owned(),
                index: pane_index(id),
                command: command(pane),
                width: size.map_or(80, |size| size.0),
                height: size.map_or(24, |size| size.1),
                is_active: pane.bool("focused").unwrap_or(false),
            },
        );
        if let Some(hook) = agent_hook(pane, agents.get(id).copied()) {
            decoded.agent_hooks.insert(id.to_owned(), hook);
        }
    }

    snapshot.active_session_id = body.str("focused_workspace_id").map(str::to_owned);
    snapshot.active_window_id = body.str("focused_tab_id").map(str::to_owned);
    snapshot.active_pane_id = body.str("focused_pane_id").map(str::to_owned);
    snapshot.is_attached = !snapshot.sessions.is_empty();
    snapshot.ever_attached = snapshot.is_attached;
    Some(decoded)
}

/// A pane's index within its window, from its id's own numbering
/// (`w1:p3` → 3): herdr panes carry no index, and array position would
/// reshuffle panes between polls. 0 when the id has none.
pub fn pane_index(pane_id: &str) -> i64 {
    let Some(marker) = pane_id.find(":p") else {
        return 0;
    };
    crate::text::swift_int(&pane_id[marker + 2..]).unwrap_or(0)
}

/// What to show as the pane's command: the detected agent where herdr has
/// one, else the terminal title. Every one of these is absent on 0.7.3.
fn command(pane: &Object) -> String {
    ["display_agent", "agent", "terminal_title_stripped", "title"]
        .iter()
        .find_map(|key| pane.str(key).filter(|value| !value.is_empty()))
        .unwrap_or("")
        .to_owned()
}

/// herdr's `agent_status` in the stamp vocabulary. `unknown` gives no state,
/// like a tmux pane with no hook installed, so nothing lights up without
/// cause.
fn agent_hook(pane: &Object, entry: Option<&Object>) -> Option<AgentHook> {
    // The agents-array entry carries its own status; the pane's can lag a
    // revision behind.
    let status = entry
        .and_then(|entry| entry.str("agent_status"))
        .or_else(|| pane.str("agent_status"));
    let state = match status {
        Some("working") => Some(AgentState::Working),
        Some("blocked") => Some(AgentState::Attention),
        Some("done") => Some(AgentState::Done),
        Some("idle") => Some(AgentState::Idle),
        _ => None,
    };
    // Identity: the pane's own fields first (0.7), then the 0.8 entry's.
    let agent = pane
        .str("display_agent")
        .or_else(|| pane.str("agent"))
        .or_else(|| entry.and_then(|entry| entry.str("display_agent")))
        .or_else(|| entry.and_then(|entry| entry.str("agent")));
    let title = pane
        .str("terminal_title_stripped")
        .or_else(|| pane.str("title"));
    if state.is_none() && agent.is_none() && title.is_none() {
        return None;
    }
    Some(AgentHook {
        state,
        agent: agent.map(str::to_owned),
        title: title.map(str::to_owned),
    })
}

/// The error code of the first JSON object in `raw`, if it is an error.
fn error_code(raw: &str) -> Option<(String, Option<String>)> {
    let json = json::first_object(raw)?;
    let error = json.object("error")?;
    Some((
        error.str("code")?.to_owned(),
        error.str("message").map(str::to_owned),
    ))
}

/// herdr saying no server is listening: "no sessions yet", not a failure.
pub fn is_server_not_running(raw: &str) -> bool {
    error_code(raw).is_some_and(|(code, _)| code == "server_not_running")
}

/// herdr 0.7.3 with no server listening: no structured error, the socket's
/// own (`Error: Os { code: 2, kind: NotFound, … }`, or `ConnectionRefused`
/// where a dead server left its socket file).
pub fn is_no_socket(raw: &str) -> bool {
    let Some(at) = raw.find("Os {") else {
        return false;
    };
    let error = &raw[at..];
    if error.contains("kind: NotFound") || error.contains("kind: ConnectionRefused") {
        return true;
    }
    // ENOENT; ECONNREFUSED on macOS and on Linux.
    let code = error.split_once("code: ").map(|(_, rest)| {
        rest.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
    });
    matches!(code.as_deref(), Some("2" | "61" | "111"))
}

/// herdr's client/server version-skew refusal (`protocol_mismatch`), whose
/// remedy is the user's alone. Its first sentence only: the CLI's message
/// goes on with remediation prose sized for a terminal.
pub fn protocol_mismatch(raw: &str) -> Option<String> {
    let (code, message) = error_code(raw)?;
    if code != "protocol_mismatch" {
        return None;
    }
    let message = message.unwrap_or_else(|| "herdr client/server protocol mismatch".to_owned());
    Some(message.split(';').next().unwrap_or(&message).to_owned())
}

/// Whether a herdr message describes client/server version skew, in either
/// wire shape: the API's "client protocol 19 is newer than server protocol
/// 16" and the attach CLI's "server rejected handshake (version 16): …".
pub fn looks_like_version_skew(message: &str) -> bool {
    message.contains("rejected handshake")
        || (message.contains("protocol") && message.contains("newer"))
}

/// The skew in the other direction: the `herdr` the app runs is older than
/// the server, so restarting the server with it would downgrade it.
pub fn client_is_older(message: &str) -> bool {
    message.contains("older than server")
}
