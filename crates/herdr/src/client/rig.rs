//! A virtual host for the client: a fake herdr that answers execs and runs
//! the frame channel's reader loop and attaches, on a virtual clock. The
//! tests drive the client against it, the equivalence harness serves the
//! same host to the Swift client, and nothing here is part of the app.
//!
//! The model is herdr 0.9.5 as recorded and probed (`tests/fixtures/
//! recorded/`, `harness/client/`): replies and errors in herdr's own shape,
//! `agent focus` that refuses a plain shell pane without moving the focus,
//! a release answered by one `terminal.closed` (`detached`) however many
//! were sent, a takeover's `terminal attach taken over`, `input ignored`
//! for a line a live attach can't parse, and a plain attach to a pane
//! another client holds refused (`… already has an attached client; retry
//! with --takeover`) where one with `--takeover` takes it.

use std::collections::{BTreeMap, VecDeque};

use super::{Client, Config, Event, ExecId, ExecResult, Output, StreamId};
use crate::base64;
use crate::control::EXIT_CODE_MARKER;
use crate::json;

/// When an exec answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    After(u64),
    Never,
    /// The exec couldn't run.
    Fail,
}

/// What happens to execs matching a pattern (the first rule that matches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub pattern: String,
    pub delay: Option<u64>,
    pub fate: Fate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fate {
    Normal,
    /// Never answers (a half-open connection, a wedged server).
    Hang,
    /// Can't run.
    Unreachable,
    /// Can't run the first time (the connection dropped under it); runs
    /// normally after.
    UnreachableOnce,
    /// Runs, but the answer stops before the exit-status trailer.
    CutTrailer,
    /// Runs, and prints exactly this.
    Output(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakePane {
    pub id: String,
    pub tab: String,
    pub workspace: String,
    pub cwd: String,
    pub agent: Option<String>,
}

/// herdr on the host, as the client sees it.
#[derive(Debug, Clone)]
pub struct FakeHerdr {
    pub version: String,
    /// `herdr status`'s client version (the `herdr` on PATH).
    pub client_version: String,
    pub running: bool,
    /// The server refuses this client's protocol with this message.
    pub mismatch: Option<String>,
    pub workspaces: Vec<(String, String)>,
    pub tabs: Vec<(String, String)>,
    pub panes: Vec<FakePane>,
    pub focused: Option<String>,
    /// Workspaces made by `worktree create`, and whether their checkout is
    /// dirty.
    pub worktrees: BTreeMap<String, bool>,
    pub manifests: Vec<String>,
    /// How long after `pane run` herdr detects the agent.
    pub detect_after: u64,
    pub default_delay: u64,
    pub rules: Vec<Rule>,
    /// Mutations in the order they landed on the server.
    pub landed: Vec<String>,
    /// What each pane received, by pane, in order: typed bytes, keys as
    /// `<up>`, prompts.
    pub typed: BTreeMap<String, Vec<u8>>,
    /// `pane run` that lands: when its agent becomes visible.
    pending_agents: Vec<(u64, String, String)>,
    next_workspace: u32,
    next_pane: u32,
    next_tab: u32,
    pub repos_output: String,
    pub scan_output: String,
    /// The snapshot names the server's version (herdr 0.9 does).
    pub snapshot_names_version: bool,
    /// Panes another client (another device) holds: a plain attach to one
    /// is refused, one with `--takeover` takes it.
    pub held_elsewhere: Vec<String>,
    /// Panes our `--takeover` took from that client, in order.
    pub taken_from_others: Vec<String>,
    /// Restarts of the server the client didn't make: each numbers every
    /// pane's terminal anew.
    pub terminal_epoch: u32,
}

impl Default for FakeHerdr {
    fn default() -> Self {
        Self {
            version: "0.9.5".to_owned(),
            client_version: "0.9.5".to_owned(),
            running: true,
            mismatch: None,
            workspaces: vec![("w1".to_owned(), "main".to_owned())],
            tabs: vec![("w1:t1".to_owned(), "w1".to_owned())],
            panes: vec![
                FakePane {
                    id: "w1:p1".to_owned(),
                    tab: "w1:t1".to_owned(),
                    workspace: "w1".to_owned(),
                    cwd: "/home/dev/code/app".to_owned(),
                    agent: None,
                },
                FakePane {
                    id: "w1:p2".to_owned(),
                    tab: "w1:t1".to_owned(),
                    workspace: "w1".to_owned(),
                    cwd: "/home/dev/code/app/sub".to_owned(),
                    agent: None,
                },
            ],
            focused: Some("w1:p1".to_owned()),
            worktrees: BTreeMap::new(),
            manifests: vec!["claude".to_owned(), "codex".to_owned(), "pi".to_owned()],
            detect_after: 1_500,
            default_delay: 40,
            rules: Vec::new(),
            landed: Vec::new(),
            typed: BTreeMap::new(),
            pending_agents: Vec::new(),
            next_workspace: 2,
            next_pane: 3,
            next_tab: 2,
            repos_output: "/home/dev/code/app\n".to_owned(),
            scan_output: "/home/dev/code/app\n/home/dev/code/other\n".to_owned(),
            snapshot_names_version: true,
            held_elsewhere: Vec::new(),
            taken_from_others: Vec::new(),
            terminal_epoch: 0,
        }
    }
}

/// The values a command names, in order: its single-quoted words.
pub fn quoted_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' {
            continue;
        }
        let mut word = String::new();
        loop {
            match chars.next() {
                None => break,
                Some('\'') => {
                    // `'\''` and `'\\'`: an apostrophe or a backslash
                    // written outside the quotes (`launch::quote`, B138),
                    // and the value goes on.
                    let rest: String = chars.clone().take(3).collect();
                    let escaped = match rest.as_str() {
                        "\\''" => Some('\''),
                        "\\\\'" => Some('\\'),
                        _ => None,
                    };
                    if let Some(escaped) = escaped {
                        word.push(escaped);
                        chars.next();
                        chars.next();
                        chars.next();
                        continue;
                    }
                    break;
                }
                Some(other) => word.push(other),
            }
        }
        words.push(word);
    }
    words
}

impl FakeHerdr {
    fn rule(&self, command: &str) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|rule| command.contains(rule.pattern.as_str()))
    }

    /// When the exec answers.
    pub fn plan(&self, command: &str) -> Plan {
        match self.rule(command) {
            Some(Rule {
                fate: Fate::Hang, ..
            }) => Plan::Never,
            Some(Rule {
                fate: Fate::Unreachable | Fate::UnreachableOnce,
                ..
            }) => Plan::Fail,
            Some(rule) => Plan::After(rule.delay.unwrap_or(self.default_delay)),
            None => Plan::After(self.default_delay),
        }
    }

    /// [`Self::plan`], spending a rule that fails once.
    pub fn plan_once(&mut self, command: &str) -> Plan {
        let plan = self.plan(command);
        if let Some(rule) = self
            .rules
            .iter_mut()
            .find(|rule| command.contains(&rule.pattern) && rule.fate == Fate::UnreachableOnce)
        {
            rule.fate = Fate::Normal;
        }
        plan
    }

    fn pane(&self, id: &str) -> Option<&FakePane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    fn json_error(id: &str, code: &str, message: &str) -> String {
        format!(
            "{{\"error\":{{\"code\":{},\"message\":{}}},\"id\":{}}}\n",
            json::encode_string(code),
            json::encode_string(message),
            json::encode_string(id)
        )
    }

    /// Advance agent detection to `now`.
    fn settle(&mut self, now: u64) {
        let due: Vec<(u64, String, String)> = self
            .pending_agents
            .iter()
            .filter(|(at, _, _)| *at <= now)
            .cloned()
            .collect();
        self.pending_agents.retain(|(at, _, _)| *at > now);
        for (_, pane, agent) in due {
            if let Some(found) = self.panes.iter_mut().find(|candidate| candidate.id == pane) {
                found.agent = Some(agent);
            }
        }
    }

    fn snapshot_json(&self) -> String {
        let workspaces: Vec<String> = self
            .workspaces
            .iter()
            .map(|(id, label)| {
                let focused = self
                    .focused
                    .as_deref()
                    .and_then(|pane| self.pane(pane))
                    .is_some_and(|pane| pane.workspace == *id);
                let worktree = if self.worktrees.contains_key(id) {
                    ",\"worktree\":{\"is_linked_worktree\":true,\"repo_name\":\"app\"}"
                } else {
                    ""
                };
                format!(
                    "{{\"workspace_id\":{},\"label\":{},\"focused\":{focused}{worktree}}}",
                    json::encode_string(id),
                    json::encode_string(label)
                )
            })
            .collect();
        let focused_pane = self.focused.as_deref().and_then(|pane| self.pane(pane));
        let tabs: Vec<String> = self
            .tabs
            .iter()
            .map(|(id, workspace)| {
                let count = self.panes.iter().filter(|pane| pane.tab == *id).count();
                let focused = focused_pane.is_some_and(|pane| pane.tab == *id);
                format!(
                    "{{\"tab_id\":{},\"workspace_id\":{},\"label\":{},\"number\":1,\"focused\":{focused},\"pane_count\":{count}}}",
                    json::encode_string(id),
                    json::encode_string(workspace),
                    json::encode_string(id)
                )
            })
            .collect();
        let panes: Vec<String> = self
            .panes
            .iter()
            .map(|pane| {
                let focused = self.focused.as_deref() == Some(pane.id.as_str());
                let status = if pane.agent.is_some() { "idle" } else { "unknown" };
                format!(
                    "{{\"pane_id\":{},\"tab_id\":{},\"workspace_id\":{},\"focused\":{focused},\"cwd\":{},\"agent_status\":\"{status}\",\"terminal_id\":\"term_{}\"}}",
                    json::encode_string(&pane.id),
                    json::encode_string(&pane.tab),
                    json::encode_string(&pane.workspace),
                    json::encode_string(&pane.cwd),
                    if self.terminal_epoch == 0 {
                        pane.id.replace(':', "_")
                    } else {
                        format!("{}_r{}", pane.id.replace(':', "_"), self.terminal_epoch)
                    }
                )
            })
            .collect();
        let agents: Vec<String> = self
            .panes
            .iter()
            .filter_map(|pane| {
                pane.agent.as_ref().map(|agent| {
                    format!(
                        "{{\"pane_id\":{},\"agent\":{}}}",
                        json::encode_string(&pane.id),
                        json::encode_string(agent)
                    )
                })
            })
            .collect();
        let focus = |value: Option<String>| {
            value.map_or_else(|| "null".to_owned(), |value| json::encode_string(&value))
        };
        format!(
            "{{\"id\":\"cli:api:snapshot\",\"result\":{{\"snapshot\":{{\"agents\":[{}],\"layouts\":[],\"panes\":[{}],\"protocol\":22,\"tabs\":[{}],\"version\":{},\"workspaces\":[{}],\"focused_workspace_id\":{},\"focused_tab_id\":{},\"focused_pane_id\":{}}},\"type\":\"session_snapshot\"}}}}\n",
            agents.join(","),
            panes.join(","),
            tabs.join(","),
            if self.snapshot_names_version {
                json::encode_string(&self.version)
            } else {
                "null".to_owned()
            },
            workspaces.join(","),
            focus(focused_pane.map(|pane| pane.workspace.clone())),
            focus(focused_pane.map(|pane| pane.tab.clone())),
            focus(self.focused.clone()),
        )
    }

    fn new_pane(&mut self, tab: &str, workspace: &str, cwd: &str) -> String {
        let id = format!("{workspace}:p{}", self.next_pane);
        self.next_pane += 1;
        self.panes.push(FakePane {
            id: id.clone(),
            tab: tab.to_owned(),
            workspace: workspace.to_owned(),
            cwd: cwd.to_owned(),
            agent: None,
        });
        id
    }

    fn new_workspace(&mut self, label: Option<&str>, cwd: &str) -> (String, String) {
        let id = format!("w{}", self.next_workspace);
        self.next_workspace += 1;
        self.workspaces
            .push((id.clone(), label.unwrap_or(&id).to_owned()));
        let tab = format!("{id}:t{}", self.next_tab);
        self.next_tab += 1;
        self.tabs.push((tab.clone(), id.clone()));
        let pane = self.new_pane(&tab, &id, cwd);
        (id, pane)
    }

    fn type_into(&mut self, pane: &str, bytes: &[u8]) {
        self.typed
            .entry(pane.to_owned())
            .or_default()
            .extend_from_slice(bytes);
    }

    /// What the exec printed, applying what it does on landing at `now`.
    pub fn land(&mut self, now: u64, command: &str) -> Vec<u8> {
        self.settle(now);
        let trailer = command.contains(EXIT_CODE_MARKER);
        let fate = self.rule(command).map(|rule| rule.fate.clone());
        if let Some(Fate::Output(output)) = fate {
            return output.into_bytes();
        }
        let (output, status) = self.answer(now, command);
        let mut out = output.into_bytes();
        if trailer && fate != Some(Fate::CutTrailer) {
            out.extend_from_slice(format!("{EXIT_CODE_MARKER}{status}\n").as_bytes());
        }
        out
    }

    fn answer(&mut self, now: u64, command: &str) -> (String, i32) {
        let words = quoted_words(command);
        let arg = |index: usize| words.get(index).cloned().unwrap_or_default();
        let not_running = || {
            (
                Self::json_error(
                    "cli:api:snapshot",
                    "server_not_running",
                    "no herdr server is running",
                ),
                1,
            )
        };
        if command.contains(" server >/dev/null") {
            self.running = true;
            self.landed.push("server start".to_owned());
            return (String::new(), 0);
        }
        if command.contains("for d in ") {
            return (self.repos_output.clone(), 0);
        }
        if command.starts_with("find -L") {
            return (self.scan_output.clone(), 0);
        }
        if command.contains("herdr status")
            || command.ends_with(" status")
            || command.contains(" status;")
        {
            let server = if self.running {
                format!("  status: running\n  version: {}\n", self.version)
            } else {
                "  status: not running\n".to_owned()
            };
            return (
                format!(
                    "client:\n  version: {}\n  channel: stable\n\nserver:\n{server}",
                    self.client_version
                ),
                0,
            );
        }
        if command.contains("server stop") {
            self.running = false;
            // Every client's attach ends with the server: another
            // device's hold on a pane too.
            self.held_elsewhere.clear();
            self.landed.push("server stop".to_owned());
            return ("{\"id\":\"cli:server:stop\",\"result\":{}}\n".to_owned(), 0);
        }
        if !self.running {
            return not_running();
        }
        if let Some(message) = &self.mismatch {
            return (
                Self::json_error("cli:api:snapshot", "protocol_mismatch", message),
                1,
            );
        }
        if command.contains("api snapshot") {
            return (self.snapshot_json(), 0);
        }
        if command.contains("server agent-manifests") {
            let items: Vec<String> = self
                .manifests
                .iter()
                .map(|agent| format!("{{\"agent\":{}}}", json::encode_string(agent)))
                .collect();
            return (
                format!(
                    "{{\"id\":\"cli:server:agent-manifests\",\"result\":{{\"manifests\":[{}]}}}}\n",
                    items.join(",")
                ),
                0,
            );
        }
        if command.contains("agent focus ") {
            // `agent focus` refuses a shell pane without moving the focus;
            // the zoom bounce behind it moves it.
            let pane = arg(0);
            if self.pane(&pane).is_none() {
                return (String::new(), 0);
            }
            self.focused = Some(pane.clone());
            self.landed.push(format!("focus {pane}"));
            return (String::new(), 0);
        }
        if command.contains("pane zoom --pane ") {
            let pane = arg(0);
            if self.pane(&pane).is_some() {
                self.focused = Some(pane.clone());
                self.landed.push(format!("zoom {pane}"));
            }
            return (String::new(), 0);
        }
        if command.contains("workspace focus ") {
            let workspace = arg(0);
            if let Some(pane) = self.panes.iter().find(|pane| pane.workspace == workspace) {
                self.focused = Some(pane.id.clone());
            }
            self.landed.push(format!("workspace focus {workspace}"));
            return (
                "{\"id\":\"cli:workspace:focus\",\"result\":{}}\n".to_owned(),
                0,
            );
        }
        if command.contains("tab focus ") {
            let tab = arg(0);
            if let Some(pane) = self.panes.iter().find(|pane| pane.tab == tab) {
                self.focused = Some(pane.id.clone());
            }
            self.landed.push(format!("tab focus {tab}"));
            return ("{\"id\":\"cli:tab:focus\",\"result\":{}}\n".to_owned(), 0);
        }
        if command.contains("workspace create") {
            let label = command.contains("--label").then(|| arg(0));
            let (_, pane) = self.new_workspace(label.as_deref(), "/home/dev");
            self.focused = Some(pane.clone());
            self.landed.push(format!(
                "workspace create{}",
                label.map(|label| format!(" {label}")).unwrap_or_default()
            ));
            return (
                format!(
                    "{{\"id\":\"cli:workspace:create\",\"result\":{{\"root_pane\":{{\"pane_id\":{}}}}}}}\n",
                    json::encode_string(&pane)
                ),
                0,
            );
        }
        if command.contains("tab create") {
            let Some(workspace) = self
                .focused
                .as_deref()
                .and_then(|pane| self.pane(pane))
                .map(|pane| pane.workspace.clone())
            else {
                return (
                    Self::json_error("cli:tab:create", "no_workspace", "no workspace"),
                    1,
                );
            };
            let tab = format!("{workspace}:t{}", self.next_tab);
            self.next_tab += 1;
            self.tabs.push((tab.clone(), workspace.clone()));
            let pane = self.new_pane(&tab, &workspace, "/home/dev");
            self.focused = Some(pane);
            self.landed.push("tab create".to_owned());
            return ("{\"id\":\"cli:tab:create\",\"result\":{}}\n".to_owned(), 0);
        }
        if command.contains("pane split ") {
            let target = arg(0);
            let Some(found) = self.pane(&target).cloned() else {
                return (
                    Self::json_error("cli:pane:split", "pane_not_found", "pane not found"),
                    1,
                );
            };
            let pane = self.new_pane(&found.tab, &found.workspace, &found.cwd);
            self.focused = Some(pane);
            self.landed.push(format!("split {target}"));
            return ("{\"id\":\"cli:pane:split\",\"result\":{}}\n".to_owned(), 0);
        }
        if command.contains("pane close ") {
            let pane = arg(0);
            if self.pane(&pane).is_none() {
                return (
                    Self::json_error(
                        "cli:pane:close",
                        "pane_not_found",
                        &format!("pane {pane} not found"),
                    ),
                    1,
                );
            }
            self.panes.retain(|candidate| candidate.id != pane);
            if self.focused.as_deref() == Some(pane.as_str()) {
                self.focused = self.panes.first().map(|pane| pane.id.clone());
            }
            self.landed.push(format!("close {pane}"));
            return ("{\"id\":\"cli:pane:close\",\"result\":{}}\n".to_owned(), 0);
        }
        if command.contains("workspace rename ") || command.contains("tab rename ") {
            self.landed.push(format!("rename {} {}", arg(0), arg(1)));
            return ("{\"id\":\"cli:rename\",\"result\":{}}\n".to_owned(), 0);
        }
        if command.contains("worktree create ") {
            let branch = words.get(1).cloned().unwrap_or_default();
            let (workspace, pane) = self.new_workspace(
                Some(&branch),
                &format!("/home/dev/.herdr/worktrees/app/{branch}"),
            );
            self.worktrees.insert(workspace.clone(), false);
            self.focused = Some(pane.clone());
            self.landed.push(format!("worktree create {branch}"));
            return (
                format!(
                    "{{\"id\":\"cli:worktree:create\",\"result\":{{\"type\":\"worktree_created\",\"workspace\":{{\"workspace_id\":{}}},\"root_pane\":{{\"pane_id\":{}}}}}}}\n",
                    json::encode_string(&workspace),
                    json::encode_string(&pane)
                ),
                0,
            );
        }
        if command.contains("worktree remove ") {
            let workspace = arg(0);
            let force = command.contains("--force");
            match self.worktrees.get(&workspace).copied() {
                None => {
                    return (
                        Self::json_error(
                            "cli:worktree:remove",
                            "workspace_not_found",
                            &format!("workspace {workspace} not found"),
                        ),
                        1,
                    );
                }
                Some(true) if !force => {
                    return (
                        Self::json_error(
                            "cli:worktree:remove",
                            "dirty_worktree_requires_force",
                            "contains modified or untracked files, use --force to delete it",
                        ),
                        1,
                    );
                }
                Some(_) => {}
            }
            self.worktrees.remove(&workspace);
            self.panes.retain(|pane| pane.workspace != workspace);
            self.workspaces.retain(|(id, _)| *id != workspace);
            self.landed.push(format!("worktree remove {workspace}"));
            return (
                format!(
                    "{{\"id\":\"cli:worktree:remove\",\"result\":{{\"type\":\"worktree_removed\",\"workspace_id\":{}}}}}\n",
                    json::encode_string(&workspace)
                ),
                0,
            );
        }
        if command.contains("pane run ") {
            let (pane, agent) = (arg(0), arg(1));
            if self.pane(&pane).is_none() {
                return (
                    Self::json_error(
                        "cli:pane:run",
                        "pane_not_found",
                        &format!("pane {pane} not found"),
                    ),
                    1,
                );
            }
            let mut typed = agent.clone().into_bytes();
            typed.push(b'\r');
            self.type_into(&pane, &typed);
            self.pending_agents
                .push((now + self.detect_after, pane.clone(), agent.clone()));
            self.landed.push(format!("run {pane} {agent}"));
            return (String::new(), 0);
        }
        if command.contains("agent prompt ") || command.contains("agent send ") {
            let (pane, text) = (arg(0), arg(1));
            if self
                .pane(&pane)
                .and_then(|pane| pane.agent.as_ref())
                .is_none()
            {
                return (
                    Self::json_error(
                        "cli:agent:prompt",
                        "agent_not_found",
                        &format!("agent target {pane} not found"),
                    ),
                    1,
                );
            }
            let mut typed = text.into_bytes();
            if command.contains("agent prompt ") {
                typed.push(b'\r');
            }
            self.type_into(&pane, &typed);
            self.landed.push(format!("prompt {pane}"));
            return (
                "{\"id\":\"cli:agent:prompt\",\"result\":{}}\n".to_owned(),
                0,
            );
        }
        if command.contains("pane send-keys ") {
            let pane = arg(0);
            for key in &words[1.min(words.len())..] {
                let bytes = if key == "Enter" {
                    b"\r".to_vec()
                } else {
                    format!("<{key}>").into_bytes()
                };
                self.type_into(&pane, &bytes);
            }
            self.landed
                .push(format!("keys {pane} {}", words[1..].join(" ")));
            return (String::new(), 0);
        }
        if command.contains("workspace close ") || command.contains("tab close ") {
            self.landed.push(format!("close {}", arg(0)));
            return ("{\"id\":\"cli:close\",\"result\":{}}\n".to_owned(), 0);
        }
        (String::new(), 0)
    }
}

// MARK: - The frame channel

/// One attach the fake frame channel runs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reader {
    /// The login shell at its prompt.
    Shell,
    /// The reader loop at its `read`.
    Loop,
    /// An attach reading the channel; what reaches it before `up` (its first
    /// frame) is lost, as herdr 0.9.5 loses it.
    Attach(String, u64),
}

/// The session's terminal channel running herdr's frame protocol.
#[derive(Debug, Clone)]
pub struct FakeFrames {
    reader: Reader,
    /// Lines on their way to the host, with when they arrive.
    inbound: VecDeque<(u64, String)>,
    /// What the host prints, with when it reaches the client.
    outbound: VecDeque<(u64, Vec<u8>)>,
    /// One way, either direction.
    pub latency: u64,
    /// From an attach line to the attach's first full frame.
    pub attach_ms: u64,
    /// The reader loop works (else the boot line's shape fails and only
    /// the one-shot line attaches).
    pub loop_works: bool,
    /// Lines the host's attaches received, by pane: `input`, `resize`, …
    pub received: BTreeMap<String, Vec<String>>,
    /// The attach lines the shell or the loop acted on, in order.
    pub attached: Vec<String>,
    /// Whether each of them asked for `--takeover`.
    pub took_over: Vec<bool>,
    /// The loop takes over whatever it attaches (the boot line before
    /// round twenty-eight).
    loop_takes_over: bool,
    /// Lines that reached an attach before it was up, and were lost.
    pub lost: Vec<String>,
    /// Lines a login shell got, in order (run, or chewed on).
    pub shell_lines: Vec<String>,
    seq: u64,
}

impl Default for FakeFrames {
    fn default() -> Self {
        Self {
            reader: Reader::Shell,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            latency: 20,
            attach_ms: 30,
            loop_works: true,
            received: BTreeMap::new(),
            attached: Vec::new(),
            took_over: Vec::new(),
            loop_takes_over: false,
            lost: Vec::new(),
            shell_lines: Vec::new(),
            seq: 0,
        }
    }
}

fn frame_line(pane: &str, seq: u64, full: bool) -> Vec<u8> {
    let screen = format!("[{pane} #{seq}]");
    format!(
        "{{\"bytes\":\"{}\",\"full\":{full},\"height\":24,\"seq\":{seq},\"type\":\"terminal.frame\",\"width\":80}}\n",
        base64::encode(screen.as_bytes())
    )
    .into_bytes()
}

impl FakeFrames {
    /// The channel's program is the reader loop itself
    /// ([`crate::frame::loop_program`]): no login shell under it; it says it
    /// reads (`ready_at`, when it works) and waits for a target.
    pub fn run_loop_program(&mut self, ready_at: u64) {
        self.reader = Reader::Loop;
        if self.loop_works {
            self.say(ready_at, b"{\"type\":\"moshpit.ready\"}\r\n".to_vec());
        }
    }

    /// The pane whose attach reads the channel.
    pub fn reading(&self) -> Option<&str> {
        match &self.reader {
            Reader::Attach(pane, _) => Some(pane),
            _ => None,
        }
    }

    fn say(&mut self, at: u64, bytes: Vec<u8>) {
        self.outbound.push_back((at + self.latency, bytes));
    }

    fn attach(&mut self, at: u64, pane: &str, takeover: bool, herdr: &mut FakeHerdr) {
        self.attached.push(pane.to_owned());
        self.took_over.push(takeover);
        if let Some(index) = herdr.held_elsewhere.iter().position(|held| held == pane) {
            if takeover {
                herdr.held_elsewhere.remove(index);
                herdr.taken_from_others.push(pane.to_owned());
            } else {
                self.reader = if self.loop_works && self.reader != Reader::Shell {
                    Reader::Loop
                } else {
                    self.reader.clone()
                };
                self.say(
                    at,
                    format!(
                        "{{\"reason\":\"terminal attach failed: terminal {pane} already has an attached client; retry with --takeover\",\"type\":\"terminal.closed\"}}\n"
                    )
                    .into_bytes(),
                );
                return;
            }
        }
        if herdr.panes.iter().any(|candidate| candidate.id == pane) {
            self.reader = Reader::Attach(pane.to_owned(), at + self.attach_ms);
            self.seq += 1;
            let seq = self.seq;
            self.say(at + self.attach_ms, frame_line(pane, seq, true));
        } else {
            self.reader = if self.loop_works && self.reader != Reader::Shell {
                Reader::Loop
            } else {
                self.reader.clone()
            };
            self.say(
                at,
                format!(
                    "{{\"reason\":\"terminal session control failed: terminal target {pane} not found\",\"type\":\"terminal.closed\"}}\n"
                )
                .into_bytes(),
            );
        }
    }

    /// The attach `pane` printed a frame (output in the pane).
    pub fn paint(&mut self, now: u64, full: bool) {
        if let Reader::Attach(pane, _) = self.reader.clone() {
            self.seq += 1;
            let seq = self.seq;
            self.say(now, frame_line(&pane, seq, full));
        }
    }

    /// Another client took the attach.
    pub fn take_over(&mut self, now: u64) {
        if let Reader::Attach(..) = self.reader {
            self.reader = if self.loop_works {
                Reader::Loop
            } else {
                Reader::Shell
            };
            self.say(
                now,
                b"{\"reason\":\"terminal attach taken over\",\"type\":\"terminal.closed\"}\n"
                    .to_vec(),
            );
        }
    }

    /// The attach reading the channel ended on the host's side and its CLI
    /// said its last words on the channel (`herdr: server shut down: …`):
    /// the server stopped, the pane closed, another client took it.
    pub fn attach_ends(&mut self, now: u64, said: &str) {
        if let Reader::Attach(..) = self.reader {
            self.reader = if self.loop_works {
                Reader::Loop
            } else {
                Reader::Shell
            };
            self.say(now, format!("{said}\r\n").into_bytes());
        }
    }

    /// One line reaching the host at `at`.
    fn line(&mut self, at: u64, line: &str, herdr: &mut FakeHerdr) {
        match self.reader.clone() {
            Reader::Attach(pane, up) => {
                if at < up {
                    self.lost.push(line.to_owned());
                    return;
                }
                if !line.starts_with('{') {
                    self.say(
                        at,
                        b"herdr: terminal session control input ignored: invalid json command: expected value at line 1 column 1\r\n".to_vec(),
                    );
                    return;
                }
                let Some(object) = json::parse_object(line.as_bytes()) else {
                    return;
                };
                let kind = object.str("type").unwrap_or("").to_owned();
                self.received
                    .entry(pane.clone())
                    .or_default()
                    .push(kind.clone());
                match kind.as_str() {
                    "terminal.input" => {
                        if let Some(bytes) = object.str("bytes").and_then(base64::decode) {
                            herdr.type_into(&pane, &bytes);
                        }
                    }
                    "terminal.release" => {
                        self.reader = if self.loop_works {
                            Reader::Loop
                        } else {
                            Reader::Shell
                        };
                        self.say(
                            at,
                            b"{\"reason\":\"detached\",\"type\":\"terminal.closed\"}\n".to_vec(),
                        );
                    }
                    "terminal.resize" if object.i64("cols").is_some_and(|cols| cols > 65_535) => {
                        self.say(at, b"herdr: terminal session control input ignored: invalid json command: invalid value\r\n".to_vec());
                    }
                    _ => {}
                }
            }
            Reader::Loop => {
                if line.starts_with('{') {
                    return;
                }
                // `<target> <cols> <rows> [take]`.
                let parts: Vec<&str> = line.split(' ').collect();
                if let Some(target) = parts.first() {
                    let takeover = self.loop_takes_over || parts.get(3) == Some(&"take");
                    self.attach(at, target, takeover, herdr);
                }
            }
            Reader::Shell => {
                self.shell_lines.push(line.to_owned());
                if line.contains("moshpit-herdr") {
                    let target = quoted_words(line).get(1).cloned().unwrap_or_default();
                    // The loop the app booted before round twenty-eight
                    // took over whatever it attached; now the first
                    // target's `take` rides in as `$4`.
                    let takeover = !line.contains("${4:+--takeover}") || line.ends_with(" take");
                    if self.loop_works {
                        self.say(at, b"{\"type\":\"moshpit.ready\"}\r\n".to_vec());
                        self.reader = Reader::Loop;
                        self.loop_takes_over = !line.contains("${4:+--takeover}");
                        self.attach(at, &target, takeover, herdr);
                    }
                } else if line.contains("terminal session control") {
                    let target = quoted_words(line).first().cloned().unwrap_or_default();
                    self.attach(at, &target, line.contains("--takeover"), herdr);
                } else {
                    self.say(at, b"zsh: command not found\r\n".to_vec());
                }
            }
        }
    }
}

// MARK: - The rig

/// An exec on its way.
#[derive(Debug, Clone)]
struct InFlight {
    at: u64,
    seq: u64,
    id: ExecId,
    command: String,
    fail: bool,
}

/// The client on the fake host, in virtual time.
#[derive(Debug)]
pub struct Rig {
    pub client: Client,
    pub host: FakeHerdr,
    pub frames: FakeFrames,
    pub now: u64,
    /// Every output, with when it came.
    pub log: Vec<(u64, Output)>,
    /// Bytes each stream received, in order.
    pub screens: BTreeMap<StreamId, Vec<u8>>,
    in_flight: Vec<InFlight>,
    /// Execs the host never answers.
    pub hung: Vec<(ExecId, String)>,
    seq: u64,
    /// Push channel: the host answers the subscribe (false: silence).
    pub push_answers: bool,
    pub push_open: bool,
    /// The push channel's program, as the client asked for it.
    pub push_program: Option<String>,
}

impl Rig {
    pub fn new(config: Config) -> Self {
        Self {
            client: Client::new(config),
            host: FakeHerdr::default(),
            frames: FakeFrames::default(),
            now: 1_000,
            log: Vec::new(),
            screens: BTreeMap::new(),
            in_flight: Vec::new(),
            hung: Vec::new(),
            seq: 0,
            push_answers: true,
            push_open: false,
            push_program: None,
        }
    }

    pub fn start(&mut self) {
        self.client.start(self.now);
        self.pump();
    }

    /// Carry out what the client asked.
    pub fn pump(&mut self) {
        while let Some(output) = self.client.poll_output() {
            match &output {
                Output::Exec { id, command } => match self.host.plan_once(command) {
                    Plan::After(delay) => {
                        self.seq += 1;
                        self.in_flight.push(InFlight {
                            at: self.now + delay,
                            seq: self.seq,
                            id: *id,
                            command: command.clone(),
                            fail: false,
                        });
                    }
                    Plan::Never => self.hung.push((*id, command.clone())),
                    Plan::Fail => {
                        self.seq += 1;
                        self.in_flight.push(InFlight {
                            at: self.now,
                            seq: self.seq,
                            id: *id,
                            command: command.clone(),
                            fail: true,
                        });
                    }
                },
                Output::FrameWrite(line) => {
                    let at = self.now + self.frames.latency;
                    self.frames.inbound.push_back((at, line.clone()));
                }
                Output::PaneBytes { stream, bytes, .. } => {
                    self.screens.entry(*stream).or_default().extend(bytes);
                }
                Output::OpenPush { program } => {
                    self.push_open = true;
                    self.push_program = Some(program.clone());
                }
                Output::ClosePush => self.push_open = false,
                _ => {}
            }
            self.log.push((self.now, output));
        }
    }

    fn next_event(&self) -> Option<u64> {
        let execs = self.in_flight.iter().map(|exec| exec.at).min();
        let inbound = self.frames.inbound.front().map(|(at, _)| *at);
        let outbound = self.frames.outbound.iter().map(|(at, _)| *at).min();
        [execs, inbound, outbound, self.client.next_deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Run until `until`, everything in time order.
    pub fn run_until(&mut self, until: u64) {
        while let Some(at) = self.next_event().filter(|&at| at <= until) {
            self.now = self.now.max(at);
            let now = self.now;
            if let Some(index) = self
                .in_flight
                .iter()
                .enumerate()
                .filter(|(_, exec)| exec.at <= now)
                .min_by_key(|(_, exec)| (exec.at, exec.seq))
                .map(|(index, _)| index)
            {
                let exec = self.in_flight.remove(index);
                let result = if exec.fail {
                    ExecResult::Failed("channel refused".to_owned())
                } else {
                    ExecResult::Output(self.host.land(now, &exec.command))
                };
                self.client.exec_finished(exec.id, result, now);
            } else if self
                .frames
                .inbound
                .front()
                .is_some_and(|(at, _)| *at <= now)
            {
                let (at, line) = self.frames.inbound.pop_front().unwrap_or_default();
                self.frames.line(at, &line, &mut self.host);
            } else if let Some(index) = self
                .frames
                .outbound
                .iter()
                .enumerate()
                .filter(|(_, (at, _))| *at <= now)
                .min_by_key(|(index, (at, _))| (*at, *index))
                .map(|(index, _)| index)
            {
                let (_, bytes) = self.frames.outbound.remove(index).unwrap_or_default();
                self.client.frame_bytes(&bytes, now);
            } else {
                self.client.tick(now);
            }
            self.pump();
        }
        self.now = self.now.max(until);
        self.client.tick(self.now);
        self.pump();
    }

    /// Run for `ms` more.
    pub fn run(&mut self, ms: u64) {
        let until = self.now + ms;
        self.run_until(until);
    }

    pub fn act(&mut self, action: super::Action) -> Option<super::RequestId> {
        let request = self.client.act(action, self.now);
        self.pump();
        request
    }

    pub fn type_text(&mut self, pane: &str, text: &str) {
        self.client.input(pane, text.as_bytes(), self.now);
        self.pump();
    }

    pub fn type_bytes(&mut self, pane: &str, bytes: &[u8]) {
        self.client.input(pane, bytes, self.now);
        self.pump();
    }

    /// The commands the client sent, in order (without the PATH prefix and
    /// the trailer), matching `filter`.
    pub fn commands(&self, filter: &str) -> Vec<String> {
        self.log
            .iter()
            .filter_map(|(_, output)| match output {
                Output::Exec { command, .. } => Some(short(command)),
                _ => None,
            })
            .filter(|command| command.contains(filter))
            .collect()
    }

    pub fn frame_writes(&self) -> Vec<String> {
        self.log
            .iter()
            .filter_map(|(_, output)| match output {
                Output::FrameWrite(line) => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn events(&self) -> Vec<Event> {
        self.log
            .iter()
            .filter_map(|(_, output)| match output {
                Output::Event(event) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    /// What `pane` received on the host, as text.
    pub fn typed(&self, pane: &str) -> String {
        String::from_utf8_lossy(self.host.typed.get(pane).map_or(&[][..], Vec::as_slice))
            .into_owned()
    }

    /// The screen bytes the client fed to `pane`'s terminal, as text.
    pub fn screen(&self, pane: &str) -> String {
        let Some(stream) = self.client.streams().get(pane) else {
            return String::new();
        };
        String::from_utf8_lossy(self.screens.get(stream).map_or(&[][..], Vec::as_slice))
            .into_owned()
    }

    /// Answer a hung exec now (the half-open connection came back).
    pub fn answer_hung(&mut self, filter: &str) {
        if let Some(index) = self
            .hung
            .iter()
            .position(|(_, command)| command.contains(filter))
        {
            let (id, command) = self.hung.remove(index);
            let output = self.host.land(self.now, &command);
            self.client
                .exec_finished(id, ExecResult::Output(output), self.now);
            self.pump();
        }
    }

    /// The push channel: open it, and answer the pump and the subscribe.
    pub fn open_push(&mut self) {
        self.client.push_opened(self.now);
        self.pump();
        if self.push_answers {
            self.client.push_bytes(b"MOSHPIT_PUMP_READY\n", self.now);
            self.pump();
        }
    }

    pub fn push_subscribed(&mut self) {
        self.client.push_bytes(
            b"{\"id\":\"mp1\",\"result\":{\"type\":\"subscription_started\"}}\n",
            self.now,
        );
        self.pump();
    }

    pub fn push_event(&mut self, kind: &str) {
        let line = format!("{{\"data\":{{}},\"event\":\"{kind}\"}}\n");
        self.client.push_bytes(line.as_bytes(), self.now);
        self.pump();
    }

    pub fn host_paint(&mut self, full: bool) {
        let now = self.now;
        self.frames.paint(now, full);
    }

    /// Another client (another device) takes the pane our attach is on
    /// over, and holds it from then on.
    pub fn host_take_over(&mut self) {
        let now = self.now;
        if let Some(pane) = self.frames.reading().map(str::to_owned) {
            self.host.held_elsewhere.push(pane);
        }
        self.frames.take_over(now);
    }

    /// The server restarted on the host, not by the client (the desktop
    /// upgraded herdr) and between two reads: every client's attach ended
    /// with it, another device's hold too, and every terminal is new.
    pub fn host_restarted_elsewhere(&mut self) {
        self.host.terminal_epoch += 1;
        self.host.held_elsewhere.clear();
        self.host_attach_ends("herdr: server shut down: server is shutting down");
    }

    /// The attach reading the channel ends with herdr's last words.
    pub fn host_attach_ends(&mut self, said: &str) {
        let now = self.now;
        self.frames.attach_ends(now, said);
    }

    /// Panes that received a release, an input, … in order.
    pub fn attach_lines(&self) -> &[String] {
        &self.frames.attached
    }
}

/// A command without the PATH prefix and the trailer.
pub fn short(command: &str) -> String {
    let start = command
        .find("\" herdr ")
        .map(|at| at + "\" herdr ".len())
        .unwrap_or(0);
    let end = command
        .find("; echo \"__moshpit_rc=$?\"")
        .unwrap_or(command.len());
    command[start..end.max(start)].to_owned()
}
