//! What Home shows, as the host posts it, and the pure derivations the page
//! draws from it: each card's state, its tree as rows in display order, the
//! agents a herdr card lists, the status line.
//!
//! The host owns the data — the saved connections (`ConnectionStore`), each
//! one's live session (`SessionHub`: its state, its multiplexer's snapshot
//! and agent stamps), the session metrics and the agent monitor's clock —
//! and posts it whenever any of it changes ([`HomeState`]). Everything a
//! card derives for display (the order of sessions, a window's title, which
//! rows a collapsed session hides, who is waiting) is worked out here, from
//! that data, the way `HomeView.swift`'s `ConnectionCard` works it out.

use std::{cmp::Ordering, collections::HashMap};

use serde::{Deserialize, Serialize};

use crate::ui::mark::Face;

/// Everything Home shows (`{"op": "home", …}`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HomeState {
    /// The saved connections, in the store's order.
    pub connections: Vec<Connection>,
    /// The build's version line under the last card (`HomeFooter`).
    pub version: String,
    /// Home is the sidebar of the iPad's split view: the bar carries the
    /// sidebar button UIKit puts there.
    #[serde(default)]
    pub sidebar: bool,
}

/// A share the host held back instead of delivering it (B622): the pane
/// it was queued for no longer runs the agent it was queued for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldShare {
    /// The share's id (the host's queue entry): what the page's intents
    /// name.
    pub id: String,
    /// The connection it was queued on (its id).
    pub connection: String,
    /// The agent it was queued for ("claude").
    pub agent: String,
    pub reason: HeldReason,
}

/// Why a share was held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeldReason {
    /// The pane is gone.
    PaneGone,
    /// The agent exited: its pane runs a shell now.
    AgentExited,
    /// The pane runs another program now.
    AnotherProgram,
}

/// Where a connection's session is (the host's reading of the session
/// hub: `ConnectionCard`'s `isDead`, `isConnecting`, `isAttachStalled`,
/// `isLive`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    /// No session: a saved host.
    #[default]
    Saved,
    /// The SSH handshake is in flight.
    Connecting,
    /// A dropped session is being dialled again.
    Reconnecting,
    /// Connected; tmux's first attach has not confirmed yet.
    Attaching,
    /// tmux's attach never confirmed (`attachStalled`).
    Stalled,
    /// The session's transport is gone: it failed, ended or disconnected.
    Dead,
    /// Connected and ready.
    Live,
}

impl Link {
    /// The session is still completing its handshake or tmux attach (the
    /// card spins in place).
    pub fn is_connecting(self) -> bool {
        matches!(
            self,
            Link::Connecting | Link::Reconnecting | Link::Attaching
        )
    }

    /// There is a session at all.
    pub fn has_session(self) -> bool {
        self != Link::Saved
    }

    /// There is a session and its transport is not gone: what the status
    /// line counts as live. (The SwiftUI line counts any session, a dead
    /// one too: "1 live connection" over a card saying OFFLINE.)
    pub fn is_up(self) -> bool {
        self.has_session() && self != Link::Dead
    }

    /// Up and settled: what Home's top line counts as online. A host still
    /// dialling or attaching is not, while its card spins (B552: "1
    /// connection online" over "Connecting…"); a stalled attach is
    /// connected, and its card says why.
    pub fn is_online(self) -> bool {
        self.is_up() && !self.is_connecting()
    }
}

/// The transport's state as the terminal's pill reads it
/// (`TransportConnState`): what tints a card's connecting row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Live,
    Connecting,
    Reconnecting,
    Offline,
}

/// A multiplexer whose control plane a session has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mux {
    #[default]
    Tmux,
    Herdr,
}

/// One saved connection.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Connection {
    /// Its id (a UUID): what the page's intents name.
    pub id: String,
    /// What it is called (`displayName`).
    pub name: String,
    /// The avatar's letter: the name's first character, upper case. What
    /// the mark shows before any host key is trusted.
    pub initial: String,
    /// Its trusted host key's face (§4.13), which the mark shows in place
    /// of the initial; none before a key is trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<Face>,
    /// The same key's pixel creature (moshpit-design `PET-DESIGN.md`
    /// §6.3), which the mark shows in the face's place while the pixel
    /// companion is on; none before a key is trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creature: Option<Face>,
    /// The emoji picked for it in its form, which the mark shows in place
    /// of the face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub user: String,
    pub host: String,
    /// It runs over mosh (else SSH).
    pub mosh: bool,
    /// The first port of its UDP range (mosh).
    pub udp_port: u32,
    pub link: Link,
    #[serde(default)]
    pub transport: Transport,
    /// mosh fell back to plain SSH on this host (no mosh-server).
    #[serde(default)]
    pub degraded: bool,
    /// When the session connected (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<f64>,
    /// The session has roamed (mosh moved networks under it).
    #[serde(default)]
    pub roaming: bool,
    /// Its disconnect is under way: a spinner in place of its ⋯ menu.
    #[serde(default)]
    pub disconnecting: bool,
    /// The multiplexer's control plane, when the session has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<Control>,
    /// When its session stopped being live, in Unix seconds ("Lost 25m
    /// ago"): a cut, an end or a failure; kept through the redials of one
    /// reconnect. `None` while live, or for a session that never went live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost_at: Option<f64>,
    /// Why its session failed, when it did (a session that ended has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<ConnectionFailure>,
}

/// Why a session failed: the reason the screen words, and the session's own
/// account of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionFailure {
    pub reason: FailureReason,
    /// What the session said, in English: for the log and diagnostics, not
    /// for the screen's words.
    #[serde(default)]
    pub detail: String,
    /// What didn't answer and why, when nothing did (B801): the card says
    /// it in full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreachable: Option<Unreachable>,
    /// The jump host whose leg failed, by its saved name (the jump-host
    /// feature's `Failure.leg`, F1b): the card says it first ("Jump host
    /// “bastion”: …"). None: the target's own leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leg: Option<String>,
}

/// A host that didn't answer (B801, the core's `Cause::Unreachable`): as
/// typed, the reason, and whether the Local Network permission may be why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Unreachable {
    pub host: String,
    pub port: u16,
    pub why: UnreachableReason,
    /// The host is on the local network as iOS's permission counts it
    /// (or a `.local` name that didn't resolve), and it didn't answer: the
    /// card names where the Local Network switch is.
    pub local_network: bool,
}

/// Why nothing answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnreachableReason {
    /// The port turned the connection away (the host answered).
    Refused,
    /// The name resolves to no address.
    NoAddress,
    /// Nothing answered within the dial's time.
    TimedOut,
    /// No route to the network or the host.
    NoRoute,
    Other,
}

/// What a session's failure was, as the screen words it (a closed set: the
/// screen says each in its own words, R19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureReason {
    /// The server turned the credentials down: the connection needs editing.
    Auth,
    /// A new host's key the person didn't trust.
    HostKey,
    /// A host's key that changed, refused.
    HostKeyChanged,
    /// Nothing answered.
    Unreachable,
    /// The dial or the handshake ran out of time.
    Timeout,
    /// The SOCKS5 proxy wouldn't carry the connection.
    Proxy,
    /// Anything else (a locked keychain, a channel, mosh's bootstrap, …).
    Other,
}

/// A session's multiplexer, as its control plane reports it
/// (`MultiplexerControlling`: the snapshot and the agent stamps).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Control {
    pub mux: Mux,
    /// tmux's attach confirmed (herdr: its snapshot came).
    pub attached: bool,
    /// A refresh the user asked for is in flight.
    #[serde(default)]
    pub refreshing: bool,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub windows: Vec<WindowInfo>,
    #[serde(default)]
    pub panes: Vec<PaneInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_pane: Option<String>,
    /// The agent stamps, by pane.
    #[serde(default)]
    pub hooks: Vec<Hook>,
    /// herdr refuses every command over a version skew: what to say, and
    /// whether restarting the server fixes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mismatch: Option<Mismatch>,
    /// The sessions that are git worktrees (herdr), and their repositories.
    #[serde(default)]
    pub worktrees: Vec<Worktree>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: String,
    pub session: String,
    pub index: i64,
    pub name: String,
    /// How many panes the window has (tmux's `window_panes`).
    pub pane_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneInfo {
    pub id: String,
    pub window: String,
    pub index: i64,
    /// The foreground process (`pane_current_command`); empty: unknown.
    pub command: String,
    /// Its working directory (tmux's `pane_current_path`, herdr's pane
    /// cwd); `None` when unknown (a host that sends none, a failed read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One pane's agent stamp (`AgentHook`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub pane: String,
    /// `attention`, `working`, `done`, `idle`, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// The agent's name, if the hook gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Since when it is in this state (Unix seconds): the stamp's own, else
    /// when the app first saw it there (the island's clock).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mismatch {
    /// What the row says (herdr's versions, in the host's words).
    pub notice: String,
    /// Restarting the server fixes it (the server is the older side).
    pub restart: bool,
    /// A restart is under way.
    pub restarting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worktree {
    pub session: String,
    pub repo: String,
}

/// An agent's signal (`AgentSignal`): only these three light anything up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    /// Waiting on a human.
    Attention,
    /// Doing something.
    Working,
    /// Finished, not yet looked at.
    Done,
}

impl Signal {
    /// The signal of a hook's state; none for `idle`, unknown or nothing.
    pub fn of(state: Option<&str>) -> Option<Signal> {
        match state? {
            "attention" => Some(Signal::Attention),
            "working" => Some(Signal::Working),
            "done" => Some(Signal::Done),
            _ => None,
        }
    }

    /// What needs a human comes first.
    pub fn rank(self) -> u8 {
        match self {
            Signal::Attention => 0,
            Signal::Working => 1,
            Signal::Done => 2,
        }
    }
}

/// Whether id `a` comes before id `b` by the numbers in them, as tmux mints
/// its ids (`$2` before `$10`) and herdr its (`w1:p2` before `w1:p10`):
/// Foundation's numeric comparison (`TmuxSnapshot.idPrecedes`), runs of
/// digits compared as numbers, the rest character by character.
pub fn numeric_order(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (x, y) = (run(&mut a), run(&mut b));
                let (tx, ty) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

/// What to call a pane's foreground process (`PaneInfo.displayCommand`):
/// Claude Code's native install runs a binary named after its version, so
/// a bare version (`2.1.269`) or anything with "claude" in it is claude.
pub fn display_command(command: &str) -> String {
    let lowered = command.to_lowercase();
    if lowered.contains("claude") {
        return "claude".to_owned();
    }
    let parts: Vec<&str> = lowered.split('.').collect();
    if parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return "claude".to_owned();
    }
    command.to_owned()
}

/// A pane's icon: what is running, at a glance (`paneIcon(for:)`).
pub fn pane_icon(command: &str) -> &'static str {
    match command.to_lowercase().as_str() {
        "claude" | "codex" | "aider" | "agy" => "sparkle",
        "vim" | "nvim" | "vi" | "nano" | "emacs" | "hx" => "square.and.pencil",
        "git" | "lazygit" | "tig" => "arrow.triangle.branch",
        "ssh" | "mosh" | "mosh-client" => "network",
        "top" | "htop" | "btop" => "chart.bar",
        "docker" | "kubectl" | "podman" => "shippingbox",
        "tail" | "less" | "bat" | "more" => "doc.text",
        "node" | "python" | "python3" | "ruby" | "cargo" | "go" | "swift" | "make" | "npm"
        | "pnpm" | "bun" | "yarn" => "hammer",
        _ => "terminal",
    }
}

/// A session row's title and label (`splitDisplayName`): a name made
/// unique with its id ("~ · w9") shows the id as the title and the name as
/// a chip, and herdr's placeholder `~` not at all.
pub fn split_display_name(display: &str) -> (String, Option<String>) {
    let Some(at) = display.rfind(" · ") else {
        return (display.to_owned(), None);
    };
    let (name, id) = (&display[..at], &display[at + " · ".len()..]);
    if id.is_empty() {
        return (display.to_owned(), None);
    }
    let label = (!name.is_empty() && name != "~").then(|| name.to_owned());
    (id.to_owned(), label)
}

/// "2m", "1h 12m", "now" under a minute (`elapsedLabel(since:)`): minutes at
/// the finest, as the stamps behind it are seconds coarse. `now_label` is
/// "now" in the shown language.
pub fn elapsed_label(since: Option<f64>, now: f64, now_label: &str) -> Option<String> {
    let since = since?;
    let minutes = ((now - since) as i64) / 60;
    if minutes < 1 {
        return Some(now_label.to_owned());
    }
    if minutes < 60 {
        return Some(format!("{minutes}m"));
    }
    Some(format!("{}h {}m", minutes / 60, minutes % 60))
}

/// Whole hours a session has been up, at least one (`subtitle`'s "2h up").
pub fn hours_up(connected_at: f64, now: f64) -> i64 {
    (((now - connected_at) / 3600.0) as i64).max(1)
}

impl Control {
    /// The sessions in the order they were made.
    pub fn sorted_sessions(&self) -> Vec<&SessionInfo> {
        let mut sessions: Vec<&SessionInfo> = self.sessions.iter().collect();
        sessions.sort_by(|a, b| numeric_order(&a.id, &b.id));
        sessions
    }

    /// Session `session`'s windows, by index.
    pub fn windows_in(&self, session: &str) -> Vec<&WindowInfo> {
        let mut windows: Vec<&WindowInfo> = self
            .windows
            .iter()
            .filter(|w| w.session == session)
            .collect();
        windows.sort_by_key(|w| w.index);
        windows
    }

    /// Window `window`'s panes, by index.
    pub fn panes_in(&self, window: &str) -> Vec<&PaneInfo> {
        let mut panes: Vec<&PaneInfo> = self.panes.iter().filter(|p| p.window == window).collect();
        panes.sort_by_key(|p| p.index);
        panes
    }

    /// A session row's label, its id appended when another session has
    /// the same name (`sessionDisplayName`).
    pub fn session_display_name(&self, session: &SessionInfo) -> String {
        let twins = self
            .sessions
            .iter()
            .filter(|s| s.name == session.name)
            .count();
        if twins > 1 {
            format!("{} · {}", session.name, session.id)
        } else {
            session.name.clone()
        }
    }

    fn pane(&self, id: &str) -> Option<&PaneInfo> {
        self.panes.iter().find(|p| p.id == id)
    }

    fn window(&self, id: &str) -> Option<&WindowInfo> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// How many agents wait on a human (every stamp, as the status line
    /// counts them).
    pub fn waiting(&self) -> usize {
        self.hooks
            .iter()
            .filter(|h| Signal::of(h.state.as_deref()) == Some(Signal::Attention))
            .count()
    }

    /// The strongest signal of the panes in `session` (a collapsed
    /// session's summary leads with it).
    pub fn strongest_in_session(&self, session: &str) -> Option<Signal> {
        self.hooks
            .iter()
            .filter_map(|hook| {
                let pane = self.pane(&hook.pane)?;
                let window = self.window(&pane.window)?;
                (window.session == session).then_some(())?;
                Signal::of(hook.state.as_deref())
            })
            .min_by_key(|s| s.rank())
    }

    /// Some pane of `window` has an agent waiting on a human.
    pub fn attention_in_window(&self, window: &str) -> bool {
        self.hooks.iter().any(|hook| {
            Signal::of(hook.state.as_deref()) == Some(Signal::Attention)
                && self.pane(&hook.pane).is_some_and(|p| p.window == window)
        })
    }

    /// The repository session `session` is a worktree of, if it is one.
    pub fn worktree(&self, session: &str) -> Option<&str> {
        self.worktrees
            .iter()
            .find(|w| w.session == session)
            .map(|w| w.repo.as_str())
    }

    /// The active pane of `window`, else its lowest-indexed one: the pane
    /// a window row's menu offers to kill.
    pub fn window_pane(&self, window: &str) -> Option<&PaneInfo> {
        let panes = self.panes_in(window);
        let active = self.active_pane.as_deref();
        panes
            .iter()
            .find(|p| Some(p.id.as_str()) == active)
            .or(panes.first())
            .copied()
    }
}

/// An agent the AGENTS section lists (`AgentEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEntry {
    pub pane: String,
    pub name: String,
    /// "work · api": its session and window.
    pub location: String,
    /// None: idle, an agent herdr knows by name sitting at its prompt.
    pub signal: Option<Signal>,
    /// When it entered this state (none for an idle one).
    pub since: Option<f64>,
}

/// The agents a card lists, in order (`ConnectionCard.agentEntries`): a row
/// needs a reason — a live signal, or an explicit `idle` with an agent's
/// name — whoever waits on a human first, idle ones last, ties by name and
/// pane. `fallback` is what to call an agent whose pane runs nothing known
/// ("agent", in the shown language).
pub fn agent_entries(control: &Control, fallback: &str) -> Vec<AgentEntry> {
    let mut entries: Vec<AgentEntry> = control
        .hooks
        .iter()
        .filter_map(|hook| {
            let pane = control.pane(&hook.pane)?;
            let signal = Signal::of(hook.state.as_deref());
            let named = hook.agent.as_deref().filter(|a| !a.is_empty());
            if signal.is_none() && !(hook.state.as_deref() == Some("idle") && named.is_some()) {
                return None;
            }
            let window = control.window(&pane.window);
            let session = window.and_then(|w| control.sessions.iter().find(|s| s.id == w.session));
            let name = match named {
                Some(name) => name.to_owned(),
                None if pane.command.is_empty() => fallback.to_owned(),
                None => display_command(&pane.command),
            };
            let location = [
                session.map(|s| s.name.as_str()),
                window.map(|w| w.name.as_str()),
            ]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            Some(AgentEntry {
                pane: hook.pane.clone(),
                name,
                location,
                signal,
                since: signal.and(hook.since),
            })
        })
        .collect();
    let rank = |e: &AgentEntry| e.signal.map_or(u8::MAX, Signal::rank);
    entries.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.pane.cmp(&b.pane))
    });
    entries
}

/// A row of a card's tree, in display order (`ConnectionCard.TreeRow`).
#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Session(SessionRow),
    Window(WindowRow),
    Pane(PaneRow),
}

impl TreeRow {
    /// Its key: the level's letter and the id (`s$0`, `w@1`, `p%2`).
    pub fn key(&self) -> String {
        match self {
            TreeRow::Session(s) => format!("s{}", s.id),
            TreeRow::Window(w) => format!("w{}", w.id),
            TreeRow::Pane(p) => format!("p{}", p.id),
        }
    }
}

/// A session's row.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: String,
    /// Its name as the row titles it (`splitDisplayName`'s title).
    pub title: String,
    /// The name, when the title is the id.
    pub label: Option<String>,
    /// The name to confirm a kill by (`sessionDisplayName`).
    pub display: String,
    /// Its name as stored (a rename starts from it).
    pub name: String,
    pub windows: usize,
    pub active: bool,
    pub expanded: bool,
    /// Collapsed: up to three window titles, the rest as a count, and
    /// whether an agent inside waits on a human.
    pub summary: Vec<String>,
    pub more: usize,
    pub attention: bool,
    pub last: bool,
}

/// A window's row.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRow {
    pub id: String,
    pub session: String,
    pub title: String,
    pub name: String,
    pub panes: i64,
    pub active: bool,
    /// Its pane level shows (only windows of more than one pane expand).
    pub expanded: bool,
    pub attention: bool,
    /// The pane its menu offers to kill: id and index.
    pub pane: Option<(String, i64)>,
    pub last: bool,
}

/// A pane's row.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneRow {
    pub id: String,
    pub index: i64,
    /// What runs in it ("shell" when tmux says nothing).
    pub label: String,
    pub icon: &'static str,
    pub active: bool,
    pub last: bool,
}

/// A window's title (`WindowInfo.displayTitle`): `index: name`, or the
/// multiplexer's word for a window and its index when it has no name of its
/// own.
pub fn window_title(window: &WindowInfo, word: &str) -> String {
    if window.name.is_empty() || window.name == window.index.to_string() {
        format!("{word} {}", window.index)
    } else {
        format!("{}: {}", window.index, window.name)
    }
}

/// The tree's rows (`ConnectionCard.treeRows`): every session, the windows
/// of those not `collapsed`, the panes of windows `expanded`. `window_word`
/// is the multiplexer's word for a window.
pub fn tree_rows(
    control: &Control,
    collapsed: &[String],
    expanded: &[String],
    window_word: &str,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for session in control.sorted_sessions() {
        let windows = control.windows_in(&session.id);
        let display = control.session_display_name(session);
        let (title, label) = split_display_name(&display);
        let open = !collapsed.contains(&session.id);
        let titles: Vec<String> = windows
            .iter()
            .map(|w| window_title(w, window_word))
            .collect();
        rows.push(TreeRow::Session(SessionRow {
            id: session.id.clone(),
            title,
            label,
            display,
            name: session.name.clone(),
            windows: windows.len(),
            active: control.active_session.as_deref() == Some(session.id.as_str()),
            expanded: open,
            summary: titles.iter().take(3).cloned().collect(),
            more: titles.len().saturating_sub(3),
            attention: control.strongest_in_session(&session.id) == Some(Signal::Attention),
            last: false,
        }));
        if !open {
            continue;
        }
        for window in windows {
            let shows_panes = expanded.contains(&window.id);
            rows.push(TreeRow::Window(WindowRow {
                id: window.id.clone(),
                session: session.id.clone(),
                title: window_title(window, window_word),
                name: window.name.clone(),
                panes: window.pane_count,
                active: control.active_window.as_deref() == Some(window.id.as_str()),
                expanded: shows_panes,
                attention: control.attention_in_window(&window.id),
                pane: control
                    .window_pane(&window.id)
                    .map(|p| (p.id.clone(), p.index)),
                last: false,
            }));
            if !shows_panes {
                continue;
            }
            for pane in control.panes_in(&window.id) {
                let label = if pane.command.is_empty() {
                    "shell".to_owned()
                } else {
                    display_command(&pane.command)
                };
                rows.push(TreeRow::Pane(PaneRow {
                    id: pane.id.clone(),
                    index: pane.index,
                    icon: pane_icon(&label),
                    label,
                    active: control.active_pane.as_deref() == Some(pane.id.as_str()),
                    last: false,
                }));
            }
        }
    }
    if let Some(row) = rows.last_mut() {
        match row {
            TreeRow::Session(s) => s.last = true,
            TreeRow::Window(w) => w.last = true,
            TreeRow::Pane(p) => p.last = true,
        }
    }
    rows
}

/// What a card shows under its head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    /// Nothing: a saved host.
    Nothing,
    /// The in-place spinner while the session connects or attaches.
    Connecting,
    /// The transport is gone: tap to reconnect.
    Dead,
    /// tmux's attach never confirmed: retry or dismiss.
    Stalled,
    /// tmux: the SESSIONS tree.
    Tmux,
    /// herdr: AGENTS, then the SESSIONS tree.
    Herdr,
    /// Live with no multiplexer control plane: one row into the terminal.
    Terminal,
}

impl Connection {
    /// What shows under the head (`ConnectionCard.body`'s branches).
    pub fn body(&self) -> Body {
        match self.link {
            Link::Saved => Body::Nothing,
            link if link.is_connecting() => Body::Connecting,
            Link::Dead => Body::Dead,
            Link::Stalled => Body::Stalled,
            _ => match &self.control {
                Some(control) if control.mux == Mux::Tmux => Body::Tmux,
                Some(_) => Body::Herdr,
                None => Body::Terminal,
            },
        }
    }

    /// How many agents wait on this connection's human: its rows in NEEDS
    /// YOU ([`needs_you`]), so the card's badge and the status line never
    /// count one the inbox doesn't list (B518): a hook left for a pane that
    /// has gone, a stalled attach, a herdr over a version skew.
    pub fn waiting(&self) -> usize {
        if !lists_agents(self) {
            return 0;
        }
        agent_rows(self, "")
            .iter()
            .filter(|a| a.state == AgentState::NeedsYou)
            .count()
    }
}

/// What the status line says and where its tap goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Agents waiting on a human, across every live connection.
    pub waiting: usize,
    /// Connections with a session.
    pub live: usize,
    /// Saved connections.
    pub saved: usize,
    /// Where a tap goes: the first connection with someone waiting, else
    /// the first live one; none when all is quiet.
    pub target: Option<String>,
}

/// The status line of `connections` (`HomeView.statusRow`).
pub fn status(connections: &[Connection]) -> Status {
    let waiting = connections.iter().map(Connection::waiting).sum();
    let live = connections.iter().filter(|c| c.link.is_up()).count();
    let first_waiting = connections.iter().find(|c| c.waiting() > 0);
    let first_live = connections.iter().find(|c| c.link.is_up());
    Status {
        waiting,
        live,
        saved: connections.len(),
        target: first_waiting.or(first_live).map(|c| c.id.clone()),
    }
}

/// The connections whose sessions are `Live` with a multiplexer, by id: for
/// lookups a row's intent needs (a session's name, a window's).
pub fn index(connections: &[Connection]) -> HashMap<&str, &Connection> {
    connections.iter().map(|c| (c.id.as_str(), c)).collect()
}

// MARK: - Home's lists (UI-DESIGN §6.1, R5)

/// The agents Home knows by their program (UI-DESIGN §6.1, the naming
/// rules' first), as [`display_command`] names them: `2.1.322` is claude.
pub const KNOWN_AGENTS: [&str; 6] = ["claude", "codex", "aider", "agy", "gemini", "opencode"];

/// Shells: a window named after one was named by tmux, not by a person.
const SHELLS: [&str; 7] = ["zsh", "bash", "fish", "sh", "dash", "ksh", "tmux"];

/// How many working agents a host's card lists before it folds the rest
/// into one row (§6.1: "from the 7th on").
pub const WORKING_SHOWN: usize = 6;

/// How many panes without an agent a host's card lists.
pub const OTHER_PANES_SHOWN: usize = 3;

/// How many project names the idle group's subtitle names.
pub const IDLE_TITLES_SHOWN: usize = 4;

/// Where an agent stands, as Home groups it (§6.1, decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgentState {
    /// Waiting on a human: Home's NEEDS YOU section, not its host's card.
    NeedsYou,
    Working,
    /// Finished, not yet looked at.
    Done,
    /// At its prompt, or nothing says otherwise.
    Idle,
}

impl AgentState {
    /// The state a hook's stamp says (none or an unknown one: idle).
    pub fn of(hook: Option<&Hook>) -> AgentState {
        match Signal::of(hook.and_then(|h| h.state.as_deref())) {
            Some(Signal::Attention) => AgentState::NeedsYou,
            Some(Signal::Working) => AgentState::Working,
            Some(Signal::Done) => AgentState::Done,
            None => AgentState::Idle,
        }
    }
}

/// What a pane is called, on Home and in the terminal's breadcrumb (§6.1,
/// the naming rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTitle {
    /// The title: the project. herdr's workspace (not `~`), else a window
    /// a person named, else the last folder of the pane's directory; with
    /// none of them (a host that sends no path), "agent · window".
    pub title: String,
    /// The agent the pane runs (`claude`), for the subtitle.
    pub agent: Option<String>,
    /// What runs in the pane, as [`display_command`] names it; empty when
    /// the host gave nothing.
    pub program: String,
}

/// An agent pane, as Home lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRow {
    /// The connection's id.
    pub connection: String,
    /// The pane's id.
    pub pane: String,
    /// [`PaneTitle::title`].
    pub title: String,
    /// The agent's name (`claude`; the caller's word for an unknown one).
    pub agent: String,
    /// The pane's working directory, as the host gave it.
    pub path: Option<String>,
    pub state: AgentState,
    /// Since when it is in this state (Unix seconds); none for an idle one.
    pub since: Option<f64>,
}

/// An agent waiting on a human, in Home's NEEDS YOU section: its host named
/// in its subtitle (`claude · gpu-01 · ~/src/trainer`).
#[derive(Debug, Clone, PartialEq)]
pub struct WaitingRow {
    pub row: AgentRow,
    /// The connection's name.
    pub host: String,
}

/// A pane with no agent (a shell, an editor, a log), on a host's card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherPane {
    pub pane: String,
    /// What runs in it (`nvim`; `zsh`).
    pub program: String,
    /// [`PaneTitle::title`]: the row says `program · title`.
    pub title: String,
    pub path: Option<String>,
}

/// A row of a live host's card, top to bottom (§6.1, structure 3): what
/// works, what is done, the idle ones as one row, a few other panes, and
/// the way to the full tree. An agent that waits is not here: it is in
/// [`needs_you`]. There is no footer.
#[derive(Debug, Clone, PartialEq)]
pub enum HostRow {
    Agent(AgentRow),
    /// "3 more working": the working agents past [`WORKING_SHOWN`].
    MoreWorking(usize),
    /// "9 idle": every idle agent as one row, the first
    /// [`IDLE_TITLES_SHOWN`] titles in its subtitle and how many more;
    /// `expanded` (the host keeps it, per connection), its agents follow it.
    Idle {
        count: usize,
        titles: Vec<String>,
        more: usize,
        expanded: bool,
    },
    Other(OtherPane),
    /// "All sessions · 3" (tmux) / "All workspaces · 12" (herdr): the
    /// session page.
    All {
        mux: Mux,
        count: usize,
    },
}

/// The line at the top of Home when nobody waits (§6.1, structure 2, and
/// decision 2): only facts, never "quiet" beside something working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Summary {
    /// Someone waits: the NEEDS YOU section shows instead of a line.
    NeedsYou { waiting: usize },
    /// Nobody waits, agents run: "Nothing needs you · 2 working · 9 idle".
    Agents {
        working: usize,
        done: usize,
        idle: usize,
    },
    /// Nobody waits and no agents: "2 connections online".
    Online { connections: usize },
    /// No connection is up: nothing at the top.
    Nothing,
}

/// The agent pane `pane` runs: its hook's name, else its program when that
/// is a known agent ([`KNOWN_AGENTS`]).
fn agent_name(pane: &PaneInfo, hook: Option<&Hook>) -> Option<String> {
    if let Some(name) = hook
        .and_then(|h| h.agent.as_deref())
        .filter(|a| !a.is_empty())
    {
        return Some(name.to_owned());
    }
    let program = display_command(&pane.command).to_lowercase();
    KNOWN_AGENTS.contains(&program.as_str()).then_some(program)
}

/// A version as a process names itself after one (`2.1.322`, `1.0`).
fn is_version(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether a window's name is one nobody chose (§6.1, the naming rules'
/// second and fourth): empty, a bare number (an index, not a name), the
/// pane's program (as tmux's automatic rename sets it: the command, its
/// display name, a version), a shell or a known agent's name, or herdr's
/// `Tab N`.
pub fn is_unnamed_window(name: &str, command: &str) -> bool {
    let name = name.trim();
    let lowered = name.to_lowercase();
    if name.is_empty()
        || name.bytes().all(|b| b.is_ascii_digit())
        || is_version(name)
        || lowered == command.to_lowercase()
        || lowered == display_command(command).to_lowercase()
        || SHELLS.contains(&lowered.as_str())
        || KNOWN_AGENTS.contains(&lowered.as_str())
    {
        return true;
    }
    name.strip_prefix("Tab ")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether two names say the same thing (B650): case and the spaces at
/// their ends aside.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// A window's project beside its session's name (B650): none when it only
/// says the session's name again (a herdr workspace named after its
/// folder, its tab working there; a tmux session named after its repo).
/// The session says it already; the caller names the window itself then:
/// its number (with the window's word or icon).
pub fn beside_session(project: Option<String>, session: &str) -> Option<String> {
    project.filter(|p| !same_name(p, session))
}

/// The project a directory names: its last folder (`~/src/api` → `api`);
/// none for the root, a home directory or nothing.
pub fn folder_name(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let home = matches!(parts.as_slice(), ["Users" | "home", _] | ["root"] | ["~"]);
    if home {
        return None;
    }
    parts.last().map(|last| (*last).to_owned())
}

/// Where a pane is, as Home names it (§6.1, the naming rules): its herdr
/// workspace (`workspace`, none under tmux), else its window's name when
/// someone chose it (not the program's own, a version, a shell or `Tab N`:
/// [`is_unnamed_window`]), else its folder. None when nothing names it:
/// Home then says the agent and the window's index; the island, the host
/// alone.
pub fn pane_place(
    command: &str,
    window_name: &str,
    workspace: Option<&str>,
    path: Option<&str>,
) -> Option<String> {
    let window_name = window_name.trim();
    workspace
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "~")
        .map(str::to_owned)
        .or_else(|| (!is_unnamed_window(window_name, command)).then(|| window_name.to_owned()))
        .or_else(|| path.and_then(folder_name))
}

/// What pane `pane` of `control` is called (§6.1, the naming rules);
/// `fallback` is the word for an agent no stamp names ("agent").
pub fn pane_title(control: &Control, pane: &PaneInfo, fallback: &str) -> PaneTitle {
    let hook = control.hooks.iter().find(|h| h.pane == pane.id);
    let agent = agent_name(pane, hook)
        .or_else(|| Signal::of(hook.and_then(|h| h.state.as_deref())).map(|_| fallback.to_owned()));
    let program = if pane.command.is_empty() {
        String::new()
    } else {
        display_command(&pane.command)
    };
    let window = control.window(&pane.window);
    let window_name = window.map_or("", |w| w.name.trim());
    let workspace = window
        .filter(|_| control.mux == Mux::Herdr)
        .and_then(|w| control.sessions.iter().find(|s| s.id == w.session))
        .map(|s| s.name.as_str());
    let title = pane_place(&pane.command, window_name, workspace, pane.path.as_deref())
        .unwrap_or_else(|| {
            let who = agent
                .clone()
                .or_else(|| (!program.is_empty()).then(|| program.clone()))
                .unwrap_or_else(|| fallback.to_owned());
            // A window the program named after itself says nothing more:
            // its index does ("claude · 2").
            let place = if is_unnamed_window(window_name, &pane.command)
                && !window_name.starts_with("Tab ")
            {
                window.map(|w| w.index.to_string()).unwrap_or_default()
            } else {
                window_name.to_owned()
            };
            if place.is_empty() {
                who
            } else {
                format!("{who} · {place}")
            }
        });
    PaneTitle {
        title,
        agent,
        program,
    }
}

impl Control {
    /// Every pane in the tree's order (sessions by id, their windows by
    /// index, their panes by index), then any the tree doesn't reach, by
    /// id: Home's rows keep their places as states change.
    pub fn panes_in_tree_order(&self) -> Vec<&PaneInfo> {
        let mut ordered: Vec<&PaneInfo> = Vec::new();
        for session in self.sorted_sessions() {
            for window in self.windows_in(&session.id) {
                ordered.extend(self.panes_in(&window.id));
            }
        }
        let mut rest: Vec<&PaneInfo> = self
            .panes
            .iter()
            .filter(|p| !ordered.iter().any(|o| o.id == p.id))
            .collect();
        rest.sort_by(|a, b| numeric_order(&a.id, &b.id));
        ordered.extend(rest);
        ordered
    }
}

/// The agent panes of `connection`'s multiplexer, in the tree's order: a
/// pane runs an agent when a stamp names one or says it works, waits or is
/// done, or when its program is a known agent.
pub fn agent_rows(connection: &Connection, fallback: &str) -> Vec<AgentRow> {
    let Some(control) = &connection.control else {
        return Vec::new();
    };
    control
        .panes_in_tree_order()
        .into_iter()
        .filter_map(|pane| {
            let hook = control.hooks.iter().find(|h| h.pane == pane.id);
            let state = AgentState::of(hook);
            let named = pane_title(control, pane, fallback);
            let agent = named.agent?;
            Some(AgentRow {
                connection: connection.id.clone(),
                pane: pane.id.clone(),
                title: named.title,
                agent,
                path: pane.path.clone(),
                state,
                since: (state != AgentState::Idle)
                    .then(|| hook.and_then(|h| h.since))
                    .flatten(),
            })
        })
        .collect()
}

/// Whether Home lists `connection`'s agents: it is live with a multiplexer
/// ([`Body::Tmux`] or [`Body::Herdr`]) that answers (herdr over a version
/// skew refuses every command: its card is the warning, [`CardBody::Mismatch`]).
fn lists_agents(connection: &Connection) -> bool {
    matches!(connection.body(), Body::Tmux | Body::Herdr)
        && connection
            .control
            .as_ref()
            .is_some_and(|c| c.mismatch.is_none())
}

/// Home's NEEDS YOU section (§6.1, decision 2): every agent waiting on a
/// human across the connections that list agents, the one waiting longest
/// first (a newcomer joins at the end; an unknown wait after every known
/// one), ties in the connections' and the tree's order.
pub fn needs_you(connections: &[Connection], fallback: &str) -> Vec<WaitingRow> {
    let mut waiting: Vec<(usize, usize, WaitingRow)> = connections
        .iter()
        .enumerate()
        .filter(|(_, c)| lists_agents(c))
        .flat_map(|(at, connection)| {
            agent_rows(connection, fallback)
                .into_iter()
                .enumerate()
                .filter(|(_, row)| row.state == AgentState::NeedsYou)
                .map(move |(order, row)| {
                    (
                        at,
                        order,
                        WaitingRow {
                            row,
                            host: connection.name.clone(),
                        },
                    )
                })
        })
        .collect();
    waiting.sort_by(|a, b| {
        let since = |w: &WaitingRow| w.row.since.unwrap_or(f64::INFINITY);
        since(&a.2)
            .total_cmp(&since(&b.2))
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });
    waiting.into_iter().map(|(_, _, row)| row).collect()
}

/// A live host's card body (§6.1, structure 3): its working agents (past
/// [`WORKING_SHOWN`], one row for the rest), its done ones, its idle ones
/// as one row (`idle_expanded`: their rows after it), at most
/// [`OTHER_PANES_SHOWN`] panes without an agent, then the way to every
/// session or workspace. Empty for a connection that lists no agents
/// (saved, connecting, stalled, gone, without a multiplexer, or herdr over
/// a version skew: their bodies are [`card_body`]'s). `fallback` names an
/// unnamed agent.
pub fn host_rows(connection: &Connection, idle_expanded: bool, fallback: &str) -> Vec<HostRow> {
    let Some(control) = connection
        .control
        .as_ref()
        .filter(|_| lists_agents(connection))
    else {
        return Vec::new();
    };
    let agents = agent_rows(connection, fallback);
    let of = |state: AgentState| agents.iter().filter(move |a| a.state == state).cloned();
    let mut rows: Vec<HostRow> = Vec::new();
    let working: Vec<AgentRow> = of(AgentState::Working).collect();
    let shown = working.len().min(WORKING_SHOWN);
    rows.extend(working[..shown].iter().cloned().map(HostRow::Agent));
    if working.len() > shown {
        rows.push(HostRow::MoreWorking(working.len() - shown));
    }
    rows.extend(of(AgentState::Done).map(HostRow::Agent));
    let idle: Vec<AgentRow> = of(AgentState::Idle).collect();
    if !idle.is_empty() {
        rows.push(HostRow::Idle {
            count: idle.len(),
            titles: idle
                .iter()
                .take(IDLE_TITLES_SHOWN)
                .map(|a| a.title.clone())
                .collect(),
            more: idle.len().saturating_sub(IDLE_TITLES_SHOWN),
            expanded: idle_expanded,
        });
        if idle_expanded {
            rows.extend(idle.into_iter().map(HostRow::Agent));
        }
    }
    let others = control
        .panes_in_tree_order()
        .into_iter()
        .filter(|pane| !agents.iter().any(|a| a.pane == pane.id))
        .take(OTHER_PANES_SHOWN)
        .map(|pane| {
            let named = pane_title(control, pane, fallback);
            HostRow::Other(OtherPane {
                pane: pane.id.clone(),
                program: named.program,
                title: named.title,
                path: pane.path.clone(),
            })
        });
    rows.extend(others);
    rows.push(HostRow::All {
        mux: control.mux,
        count: control.sessions.len(),
    });
    rows
}

/// What the top of Home says (§6.1, structure 2): the NEEDS YOU section
/// when anyone waits; else, with a connection up, how many agents work,
/// are done and idle, or how many connections are online when none runs
/// an agent; nothing with no connection up.
pub fn summary(connections: &[Connection], fallback: &str) -> Summary {
    let waiting = needs_you(connections, fallback).len();
    if waiting > 0 {
        return Summary::NeedsYou { waiting };
    }
    let online = connections.iter().filter(|c| c.link.is_online()).count();
    if online == 0 {
        return Summary::Nothing;
    }
    let agents: Vec<AgentRow> = connections
        .iter()
        .filter(|c| lists_agents(c))
        .flat_map(|c| agent_rows(c, fallback))
        .collect();
    let count = |state: AgentState| agents.iter().filter(|a| a.state == state).count();
    if agents.is_empty() {
        Summary::Online {
            connections: online,
        }
    } else {
        Summary::Agents {
            working: count(AgentState::Working),
            done: count(AgentState::Done),
            idle: count(AgentState::Idle),
        }
    }
}

/// What a host's card shows under its head (§6.1, structure 3 and its
/// states): the one row a state calls for, or a live host's rows.
#[derive(Debug, Clone, PartialEq)]
pub enum CardBody {
    /// The session dials, redials or attaches its multiplexer: a spinner
    /// row, "Connecting…", or "Connecting to tmux…" with `mux` attaching.
    Connecting { mux: Option<Mux> },
    /// Its disconnect is under way (the teardown flushes for up to ~2 s): a
    /// spinner row, "Disconnecting…", and nothing to tap (B535).
    Disconnecting,
    /// The session is gone: "Reconnect", "Lost 25m ago" from `lost_at`.
    /// `reason`: why it failed, when it did (R19 may word a host key's
    /// differently); none for a session that ended.
    Reconnect {
        lost_at: Option<f64>,
        reason: Option<FailureReason>,
        /// What didn't answer and why, said in full (B801).
        unreachable: Option<Unreachable>,
        /// The jump host whose leg failed, said first.
        leg: Option<String>,
    },
    /// The server turned the credentials down: "Edit connection…".
    EditConnection,
    /// tmux's attach never confirmed: "Retry connecting tmux" (⋯ also has
    /// "Open a plain shell").
    RetryAttach,
    /// herdr refuses every command over a version skew: the warning, and
    /// "Restart server" when that fixes it.
    Mismatch(Mismatch),
    /// Live without a multiplexer: one "Open terminal" row.
    OpenTerminal,
    /// Live with a multiplexer: [`host_rows`].
    Rows(Vec<HostRow>),
}

/// `connection`'s card body; none for a saved host (it has no card, a row
/// in SAVED). `idle_expanded` and `fallback` as [`host_rows`] takes them.
pub fn card_body(connection: &Connection, idle_expanded: bool, fallback: &str) -> Option<CardBody> {
    let body = match connection.link {
        Link::Saved => return None,
        _ if connection.disconnecting => CardBody::Disconnecting,
        Link::Connecting | Link::Reconnecting => CardBody::Connecting { mux: None },
        Link::Attaching => CardBody::Connecting {
            mux: Some(connection.control.as_ref().map_or(Mux::Tmux, |c| c.mux)),
        },
        Link::Stalled => CardBody::RetryAttach,
        Link::Dead => match connection.failure.as_ref().map(|f| f.reason) {
            Some(FailureReason::Auth) => CardBody::EditConnection,
            reason => CardBody::Reconnect {
                lost_at: connection.lost_at,
                reason,
                unreachable: connection
                    .failure
                    .as_ref()
                    .and_then(|f| f.unreachable.clone()),
                leg: connection.failure.as_ref().and_then(|f| f.leg.clone()),
            },
        },
        Link::Live => match &connection.control {
            None => CardBody::OpenTerminal,
            Some(control) => match &control.mismatch {
                Some(mismatch) => CardBody::Mismatch(mismatch.clone()),
                None => CardBody::Rows(host_rows(connection, idle_expanded, fallback)),
            },
        },
    };
    Some(body)
}

#[cfg(test)]
mod tests {

    /// UI-DESIGN §10.5: a pane's working directory comes with the host's
    /// pane; a host that sends none (an older one, a read that failed)
    /// leaves it unknown, and it is not written back unknown.
    #[test]
    fn a_panes_path_is_read_when_the_host_sends_one() {
        let with: PaneInfo = serde_json::from_value(serde_json::json!({
            "id": "%1", "window": "@1", "index": 0, "command": "zsh", "path": "/srv/my app",
        }))
        .unwrap();
        assert_eq!(with.path.as_deref(), Some("/srv/my app"));
        let without: PaneInfo = serde_json::from_value(serde_json::json!({
            "id": "%1", "window": "@1", "index": 0, "command": "zsh",
        }))
        .unwrap();
        assert_eq!(without.path, None);
        assert_eq!(serde_json::to_value(&without).unwrap().get("path"), None);
    }

    use super::*;

    /// `lostAt` and `failure` (R5's): read when a host sends them, and
    /// left out of the JSON when absent, so a host that doesn't is
    /// unchanged.
    #[test]
    fn a_connection_says_when_it_was_lost_and_why_it_failed() {
        let failed: Connection = serde_json::from_value(serde_json::json!({
            "id": "a", "name": "n", "initial": "N", "user": "u", "host": "h", "mosh": false,
            "udpPort": 60001, "link": "dead", "lostAt": 1_762_800_000.5,
            "failure": {"reason": "auth", "detail": "the server refused the password"},
        }))
        .unwrap();
        assert_eq!(failed.lost_at, Some(1_762_800_000.5));
        assert_eq!(
            failed.failure,
            Some(ConnectionFailure {
                reason: FailureReason::Auth,
                detail: "the server refused the password".into(),
                unreachable: None,
                leg: None,
            })
        );
        for reason in [
            "auth",
            "hostKey",
            "hostKeyChanged",
            "unreachable",
            "timeout",
            "proxy",
            "other",
        ] {
            let failure: ConnectionFailure =
                serde_json::from_value(serde_json::json!({"reason": reason})).unwrap();
            assert_eq!(serde_json::to_value(failure.reason).unwrap(), reason);
        }
        assert!(
            serde_json::from_value::<ConnectionFailure>(serde_json::json!({"reason": "cosmic"}))
                .is_err(),
            "a closed set"
        );
        let quiet = serde_json::to_value(Connection::default()).unwrap();
        assert_eq!(quiet.get("lostAt"), None);
        assert_eq!(quiet.get("failure"), None);
    }

    fn control() -> Control {
        Control {
            mux: Mux::Tmux,
            attached: true,
            sessions: vec![
                SessionInfo {
                    id: "$10".into(),
                    name: "ops".into(),
                },
                SessionInfo {
                    id: "$2".into(),
                    name: "work".into(),
                },
            ],
            windows: vec![
                WindowInfo {
                    id: "@3".into(),
                    session: "$2".into(),
                    index: 1,
                    name: "api".into(),
                    pane_count: 2,
                },
                WindowInfo {
                    id: "@1".into(),
                    session: "$2".into(),
                    index: 0,
                    name: String::new(),
                    pane_count: 1,
                },
                WindowInfo {
                    id: "@7".into(),
                    session: "$10".into(),
                    index: 0,
                    name: "logs".into(),
                    pane_count: 1,
                },
            ],
            panes: vec![
                PaneInfo {
                    id: "%4".into(),
                    window: "@3".into(),
                    index: 1,
                    command: "2.1.269".into(),
                    path: None,
                },
                PaneInfo {
                    id: "%3".into(),
                    window: "@3".into(),
                    index: 0,
                    command: "nvim".into(),
                    path: None,
                },
                PaneInfo {
                    id: "%1".into(),
                    window: "@1".into(),
                    index: 0,
                    command: String::new(),
                    path: None,
                },
                PaneInfo {
                    id: "%9".into(),
                    window: "@7".into(),
                    index: 0,
                    command: "tail".into(),
                    path: None,
                },
            ],
            active_session: Some("$2".into()),
            active_window: Some("@3".into()),
            active_pane: Some("%4".into()),
            hooks: vec![
                Hook {
                    pane: "%4".into(),
                    state: Some("attention".into()),
                    agent: Some("claude".into()),
                    since: Some(1000.0),
                },
                Hook {
                    pane: "%9".into(),
                    state: Some("working".into()),
                    agent: None,
                    since: Some(2000.0),
                },
            ],
            ..Control::default()
        }
    }

    #[test]
    fn ids_sort_by_their_numbers() {
        assert_eq!(numeric_order("$2", "$10"), Ordering::Less);
        assert_eq!(numeric_order("$10", "$2"), Ordering::Greater);
        assert_eq!(numeric_order("w1:p2", "w1:p10"), Ordering::Less);
        assert_eq!(numeric_order("%9", "%9"), Ordering::Equal);
        assert_eq!(numeric_order("$02", "$2"), Ordering::Equal);
        assert_eq!(numeric_order("a", "b"), Ordering::Less);
        assert_eq!(numeric_order("$1", "$1a"), Ordering::Less);
    }

    #[test]
    fn the_tree_flattens_as_the_swiftui_card_does() {
        let control = control();
        let keys = |rows: &[TreeRow]| rows.iter().map(TreeRow::key).collect::<Vec<_>>();
        let rows = tree_rows(&control, &[], &[], "Window");
        assert_eq!(keys(&rows), ["s$2", "w@1", "w@3", "s$10", "w@7"]);
        let TreeRow::Window(first) = &rows[1] else {
            panic!()
        };
        assert_eq!(first.title, "Window 0", "an unnamed window");
        let TreeRow::Window(api) = &rows[2] else {
            panic!()
        };
        assert_eq!(api.title, "1: api");
        assert!(api.active && api.attention);
        assert_eq!(api.pane, Some(("%4".into(), 1)), "the active pane");
        let TreeRow::Window(logs) = &rows[4] else {
            panic!()
        };
        assert!(logs.last);

        // A window's panes, and a collapsed session's summary.
        let rows = tree_rows(&control, &["$10".into()], &["@3".into()], "Window");
        assert_eq!(keys(&rows), ["s$2", "w@1", "w@3", "p%3", "p%4", "s$10"]);
        let TreeRow::Pane(nvim) = &rows[3] else {
            panic!()
        };
        assert_eq!(
            (nvim.label.as_str(), nvim.icon),
            ("nvim", "square.and.pencil")
        );
        let TreeRow::Pane(claude) = &rows[4] else {
            panic!()
        };
        assert_eq!((claude.label.as_str(), claude.icon), ("claude", "sparkle"));
        assert!(claude.active);
        let TreeRow::Session(ops) = &rows[5] else {
            panic!()
        };
        assert!(!ops.expanded && ops.last);
        assert_eq!(ops.summary, ["0: logs"]);
        assert!(!ops.attention, "working is not attention");
        let TreeRow::Session(work) = &rows[0] else {
            panic!()
        };
        assert!(work.attention && work.active);
    }

    #[test]
    fn sessions_with_one_name_show_their_ids() {
        let mut control = control();
        control.sessions = vec![
            SessionInfo {
                id: "w1".into(),
                name: "~".into(),
            },
            SessionInfo {
                id: "w2".into(),
                name: "~".into(),
            },
            SessionInfo {
                id: "w3".into(),
                name: "api".into(),
            },
        ];
        let first = &control.sessions[0];
        assert_eq!(control.session_display_name(first), "~ · w1");
        assert_eq!(split_display_name("~ · w1"), ("w1".into(), None));
        assert_eq!(
            split_display_name("api · w4"),
            ("w4".into(), Some("api".into()))
        );
        assert_eq!(split_display_name("api"), ("api".into(), None));
        assert_eq!(control.session_display_name(&control.sessions[2]), "api");
    }

    #[test]
    fn agents_need_a_reason_and_waiting_ones_lead() {
        let mut control = control();
        control.mux = Mux::Herdr;
        control.hooks.push(Hook {
            pane: "%1".into(),
            state: Some("idle".into()),
            agent: Some("codex".into()),
            since: Some(5.0),
        });
        control.hooks.push(Hook {
            pane: "%3".into(),
            state: Some("idle".into()),
            agent: None,
            since: None,
        });
        control.hooks.push(Hook {
            pane: "%gone".into(),
            state: Some("attention".into()),
            agent: None,
            since: None,
        });
        let entries = agent_entries(&control, "agent");
        let panes: Vec<&str> = entries.iter().map(|e| e.pane.as_str()).collect();
        assert_eq!(panes, ["%4", "%9", "%1"], "attention, working, idle");
        assert_eq!(entries[0].location, "work · api");
        assert_eq!(entries[1].name, "tail", "the command, without a name");
        assert_eq!(entries[2].signal, None);
        assert_eq!(entries[2].since, None, "idle shows no duration");
        assert_eq!(entries[0].since, Some(1000.0));
    }

    /// What the SwiftUI card's own tests pinned (AgentSectionTests,
    /// HomeTreeRowsTests), kept when the card went (D8): agents in one
    /// state keep their order poll after poll (by name, then pane), one
    /// with no name and no command says the generic word, and a level's
    /// key never meets another level's however alike the raw ids.
    #[test]
    fn ties_keep_their_order_and_keys_their_level() {
        let hook = |pane: &str, agent: Option<&str>| Hook {
            pane: pane.into(),
            state: Some("working".into()),
            agent: agent.map(Into::into),
            since: None,
        };
        let mut control = control();
        control.hooks = vec![
            hook("%9", Some("codex")),
            hook("%4", Some("codex")),
            hook("%1", None),
        ];
        let panes = |control: &Control| {
            agent_entries(control, "agent")
                .into_iter()
                .map(|e| (e.pane, e.name))
                .collect::<Vec<_>>()
        };
        let first = panes(&control);
        assert_eq!(
            first,
            [
                ("%1".to_owned(), "agent".to_owned()),
                ("%4".to_owned(), "codex".to_owned()),
                ("%9".to_owned(), "codex".to_owned()),
            ]
        );
        control.hooks.reverse();
        assert_eq!(panes(&control), first, "the hooks' order changes nothing");

        let alike = Control {
            mux: Mux::Tmux,
            attached: true,
            sessions: vec![SessionInfo {
                id: "1".into(),
                name: "one".into(),
            }],
            windows: vec![WindowInfo {
                id: "1".into(),
                session: "1".into(),
                index: 1,
                name: "one".into(),
                pane_count: 1,
            }],
            panes: vec![PaneInfo {
                id: "1".into(),
                window: "1".into(),
                index: 0,
                command: String::new(),
                path: None,
            }],
            ..Control::default()
        };
        let keys: Vec<String> = tree_rows(&alike, &[], &["1".into()], "Window")
            .iter()
            .map(TreeRow::key)
            .collect();
        assert_eq!(keys, ["s1", "w1", "p1"]);
    }

    #[test]
    fn elapsed_and_uptime_labels() {
        assert_eq!(elapsed_label(None, 100.0, "now"), None);
        assert_eq!(
            elapsed_label(Some(100.0), 130.0, "now").as_deref(),
            Some("now")
        );
        assert_eq!(
            elapsed_label(Some(0.0), 125.0, "now").as_deref(),
            Some("2m")
        );
        assert_eq!(
            elapsed_label(Some(0.0), 72.0 * 60.0, "now").as_deref(),
            Some("1h 12m")
        );
        assert_eq!(hours_up(0.0, 60.0), 1, "at least one");
        assert_eq!(hours_up(0.0, 2.5 * 3600.0), 2);
    }

    #[test]
    fn display_commands_and_icons() {
        assert_eq!(display_command("2.1.269"), "claude");
        assert_eq!(display_command("Claude-Code"), "claude");
        assert_eq!(display_command("1.2"), "1.2");
        assert_eq!(display_command("zsh"), "zsh");
        assert_eq!(pane_icon("zsh"), "terminal");
        assert_eq!(pane_icon("Cargo"), "hammer");
    }

    #[test]
    fn bodies_and_the_status_line() {
        let saved = Connection {
            id: "a".into(),
            ..Connection::default()
        };
        assert_eq!(saved.body(), Body::Nothing);
        let attaching = Connection {
            id: "b".into(),
            link: Link::Attaching,
            control: Some(control()),
            ..Connection::default()
        };
        assert_eq!(attaching.body(), Body::Connecting);
        let live = Connection {
            id: "c".into(),
            link: Link::Live,
            control: Some(control()),
            ..Connection::default()
        };
        assert_eq!(live.body(), Body::Tmux);
        let plain = Connection {
            id: "d".into(),
            link: Link::Live,
            ..Connection::default()
        };
        assert_eq!(plain.body(), Body::Terminal);
        let dead = Connection {
            id: "e".into(),
            link: Link::Dead,
            control: Some(control()),
            ..Connection::default()
        };
        assert_eq!(dead.body(), Body::Dead);
        assert_eq!(
            Connection {
                link: Link::Stalled,
                ..Connection::default()
            }
            .body(),
            Body::Stalled
        );

        let all = vec![saved.clone(), plain.clone(), live.clone()];
        let status = status(&all);
        assert_eq!(status.waiting, 1);
        assert_eq!(status.live, 2);
        assert_eq!(status.saved, 3);
        assert_eq!(status.target.as_deref(), Some("c"), "where someone waits");
        let quiet = super::status(&[saved.clone(), plain]);
        assert_eq!(quiet.target.as_deref(), Some("d"), "else the first live");
        assert_eq!(super::status(std::slice::from_ref(&saved)).target, None);
        // A dead session is not live, and its stale stamps wait on no one.
        let gone = super::status(&[saved, dead]);
        assert_eq!((gone.live, gone.waiting, gone.target), (0, 0, None));
    }
}
