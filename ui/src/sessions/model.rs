//! What the sessions page draws, derived from the host's tree
//! ([`Control`]) and the disclosure: pure, so the rules are tested without
//! a surface.

use std::collections::HashSet;

use nsh_core::prelude::*;

use crate::home::{
    TreeState,
    model::{
        AgentState, Connection, Control, Mux, beside_session, display_command, is_unnamed_window,
        pane_icon, pane_title, split_display_name, window_title,
    },
};

/// Past this many sessions, only the one in use starts open.
pub const OPEN_AT_MOST: usize = 8;

/// The multiplexer's words: tmux's sessions and windows, herdr's
/// workspaces and tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    /// A window mid-sentence, numbered ("Window 2"): the title of one with
    /// no name of its own.
    pub window: String,
    /// The empty page: no sessions on this host.
    pub none: String,
    /// Making one: the empty page's button.
    pub new: String,
    /// Making a window (herdr: tab) in a session: the row at the end of an
    /// open session's card.
    pub new_window: String,
    /// A pane whose program the host did not say.
    pub shell: String,
    /// An agent no stamp names.
    pub agent: String,
}

impl Words {
    pub fn of(mux: Mux) -> Words {
        match mux {
            Mux::Tmux => Words {
                window: t!("Window"),
                none: t!("No tmux sessions on this host"),
                new: t!("New Session"),
                new_window: t!("New window"),
                shell: t!("shell"),
                agent: t!("agent"),
            },
            Mux::Herdr => Words {
                window: t!("Tab"),
                none: t!("No herdr workspaces on this host"),
                new: t!("New Workspace"),
                new_window: t!("New tab"),
                shell: t!("shell"),
                agent: t!("agent"),
            },
        }
    }
}

/// The tree's disclosure: the sessions folded away, the windows whose panes
/// show, and whether the host has kept any for this host yet (until it
/// has, a host of many sessions opens only the one in use).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Disclosure {
    pub collapsed: HashSet<String>,
    pub expanded: HashSet<String>,
    pub told: bool,
}

impl Disclosure {
    /// The disclosure the host kept.
    pub fn told(tree: &TreeState) -> Disclosure {
        Disclosure {
            collapsed: tree.collapsed.iter().cloned().collect(),
            expanded: tree.expanded.iter().cloned().collect(),
            told: true,
        }
    }

    /// Whether session `session` shows its windows, among `sessions`,
    /// `active` the one in use.
    pub fn open(&self, session: &str, sessions: &[String], active: Option<&str>) -> bool {
        if self.told || sessions.len() <= OPEN_AT_MOST {
            !self.collapsed.contains(session)
        } else {
            active == Some(session)
        }
    }

    /// The disclosure as the page shows it, written out: a host of many
    /// sessions that kept none has every session but the one in use folded.
    pub fn settled(self, sessions: &[String], active: Option<&str>) -> Disclosure {
        if self.told || sessions.len() <= OPEN_AT_MOST {
            return Disclosure { told: true, ..self };
        }
        Disclosure {
            collapsed: sessions
                .iter()
                .filter(|s| Some(s.as_str()) != active)
                .cloned()
                .collect(),
            expanded: self.expanded,
            told: true,
        }
    }
}

/// A session's card.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    /// Its name (with its id when another has the same).
    pub title: String,
    /// The session in use.
    pub active: bool,
    /// The repository a worktree's session belongs to.
    pub repo: Option<String>,
    /// How many agents in it wait on a human.
    pub waiting: usize,
    pub expanded: bool,
    /// Folded: its first three windows' titles, and how many more.
    pub summary: Vec<String>,
    pub more: usize,
    pub windows: Vec<WindowItem>,
    /// Every session's id, and the one in use: what a fold writes out.
    pub all_sessions: Vec<String>,
    pub active_session: Option<String>,
}

impl Group {
    /// The head's second line: in use, the repository, and (folded) the
    /// windows it holds.
    pub fn subtitle(&self) -> String {
        let mut parts = Vec::new();
        if self.active {
            parts.push(t!("Attached"));
        }
        if let Some(repo) = &self.repo {
            parts.push(repo.clone());
        }
        if !self.expanded && !self.summary.is_empty() {
            let mut names = self.summary.join(", ");
            if self.more > 0 {
                names.push_str(", ");
                names.push_str(&crate::plural!("%lld more", self.more as i64));
            }
            parts.push(names);
        }
        parts.join(" · ")
    }
}

/// A window's row.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowItem {
    pub id: String,
    /// What it is called: a name a person gave it, else its pane's project
    /// (§6.1, the naming rules).
    pub title: String,
    /// Its panes when several, and whether it is the one in use.
    pub subtitle: String,
    /// What runs in its pane (an SF Symbol).
    pub icon: &'static str,
    /// The agent of a window of one pane, and its state.
    pub agent: Option<AgentState>,
    pub expanded: bool,
    pub panes: Vec<PaneItem>,
}

/// A pane's row.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneItem {
    pub id: String,
    /// The agent's name, else the program; empty: the host said nothing.
    pub title: String,
    pub path: Option<String>,
    pub icon: &'static str,
    pub agent: Option<AgentState>,
    pub active: bool,
}

/// The sessions' cards, in the order they were made.
pub fn groups(control: &Control, disclosure: &Disclosure, words: &Words) -> Vec<Group> {
    let sessions: Vec<String> = control
        .sorted_sessions()
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let active = control.active_session.clone();
    control
        .sorted_sessions()
        .into_iter()
        .map(|session| {
            let windows = control.windows_in(&session.id);
            // Twins are told apart by their ids, the name kept: "api ·
            // w4", as the terminal's breadcrumb says (B555: the name was
            // dropped). A name of "~" says nothing beside the id.
            let display = control.session_display_name(session);
            let title = match split_display_name(&display) {
                (id, Some(name)) => format!("{name} · {id}"),
                (id, None) => id,
            };
            let open = disclosure.open(&session.id, &sessions, active.as_deref());
            let titles: Vec<String> = windows
                .iter()
                .map(|w| window_name(control, w.id.as_str(), words))
                .collect();
            let waiting = control
                .panes
                .iter()
                .filter(|p| windows.iter().any(|w| w.id == p.window))
                .filter(|p| agent_state(control, &p.id, words) == Some(AgentState::NeedsYou))
                .count();
            Group {
                id: session.id.clone(),
                title,
                active: active.as_deref() == Some(session.id.as_str()),
                repo: control.worktree(&session.id).map(str::to_owned),
                waiting,
                expanded: open,
                summary: titles.iter().take(3).cloned().collect(),
                more: titles.len().saturating_sub(3),
                windows: windows
                    .iter()
                    .map(|w| window_item(control, &w.id, disclosure, words))
                    .collect(),
                all_sessions: sessions.clone(),
                active_session: active.clone(),
            }
        })
        .collect()
}

/// A window's title: the name a person gave it, else what its pane is
/// about (§6.1), else the multiplexer's word and its index. Never its
/// session's name again (B650): under a herdr workspace named after its
/// folder every tab said it (a pane's title is its workspace there), so a
/// window that would is called by its word and index ("Tab 1").
fn window_name(control: &Control, window: &str, words: &Words) -> String {
    let Some(info) = control.windows.iter().find(|w| w.id == window) else {
        return String::new();
    };
    let session = control
        .sessions
        .iter()
        .find(|s| s.id == info.session)
        .map_or("", |s| s.name.as_str());
    let pane = control.window_pane(window);
    let command = pane.map_or("", |p| p.command.as_str());
    let named = if !is_unnamed_window(&info.name, command) {
        Some(info.name.trim().to_owned())
    } else {
        pane.map(|pane| pane_title(control, pane, &words.agent).title)
    };
    match named {
        // No name of its own at all: the multiplexer's way ("Window 2").
        None => window_title(info, &words.window),
        // Its name, unless the session said it already: then its number.
        Some(named) => beside_session(Some(named), session)
            .unwrap_or_else(|| format!("{} {}", words.window, info.index)),
    }
}

fn window_item(
    control: &Control,
    window: &str,
    disclosure: &Disclosure,
    words: &Words,
) -> WindowItem {
    let panes: Vec<PaneItem> = control
        .panes_in(window)
        .into_iter()
        .map(|pane| {
            let title = pane_title(control, pane, &words.agent);
            let program = if pane.command.is_empty() {
                String::new()
            } else {
                display_command(&pane.command)
            };
            let agent = agent_state(control, &pane.id, words);
            PaneItem {
                id: pane.id.clone(),
                title: if agent.is_some() {
                    title.agent.unwrap_or_else(|| words.agent.clone())
                } else {
                    program.clone()
                },
                path: pane.path.clone(),
                icon: pane_icon(&program),
                agent,
                active: control.active_pane.as_deref() == Some(pane.id.as_str()),
            }
        })
        .collect();
    let active = control.active_window.as_deref() == Some(window);
    let mut subtitle = Vec::new();
    if panes.len() > 1 {
        subtitle.push(crate::plural!("%lld panes", panes.len() as i64));
    }
    if active {
        subtitle.push(t!("current"));
    }
    let single = (panes.len() == 1).then(|| panes[0].clone());
    WindowItem {
        id: window.to_owned(),
        title: window_name(control, window, words),
        subtitle: subtitle.join(" · "),
        icon: single.as_ref().map_or("rectangle.split.2x1", |p| p.icon),
        agent: single.and_then(|p| p.agent),
        expanded: panes.len() > 1 && disclosure.expanded.contains(window),
        panes,
    }
}

/// The state of the agent in pane `pane`, if an agent runs there (a stamp
/// names one or says it works, waits or is done, or its program is a
/// known agent).
fn agent_state(control: &Control, pane: &str, words: &Words) -> Option<AgentState> {
    let info = control.panes.iter().find(|p| p.id == pane)?;
    pane_title(control, info, &words.agent).agent?;
    let hook = control.hooks.iter().find(|h| h.pane == pane);
    Some(AgentState::of(hook))
}

/// The tree's shape: its sessions, windows and panes. A change of shape
/// (one made or closed) moves the rows; a change of state does not.
pub fn shape(connection: &Connection) -> Vec<String> {
    let Some(control) = &connection.control else {
        return Vec::new();
    };
    control
        .sessions
        .iter()
        .map(|s| format!("s{}", s.id))
        .chain(control.windows.iter().map(|w| format!("w{}", w.id)))
        .chain(control.panes.iter().map(|p| format!("p{}", p.id)))
        .collect()
}
