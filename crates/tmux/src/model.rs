//! The tmux world as the controller sees it: sessions, windows and panes by
//! tmux's own ids (`$0`, `@0`, `%0`), and the selection a reconnect lands
//! back on. A port of `TmuxTypes.swift` and `TmuxSnapshot`
//! (`MultiplexerControlling.swift`).
//!
//! The maps are keyed by id and iterate in string order, which is
//! deterministic; wherever the Swift code walked a `Dictionary` (whose order
//! is unspecified, and different in every process) the port walks the ids in
//! the order tmux made them ([`id_cmp`]), and the equivalence harness pins
//! the Swift side to the same order.

use std::cmp::Ordering;
use std::collections::BTreeMap;

/// A tmux pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    /// `%<n>`.
    pub id: String,
    /// The window it is in, `@<n>`; empty for a pane known only from its
    /// `%output` (it streamed before discovery placed it).
    pub window_id: String,
    /// `#{pane_index}`.
    pub index: i64,
    /// `#{pane_current_command}`.
    pub command: String,
    /// `#{pane_current_path}`: the working directory of what runs in the
    /// pane, as of the last pane listing; `None` when tmux had none to give
    /// (or answered a listing of an older format).
    pub path: Option<String>,
    pub width: i64,
    pub height: i64,
    /// Whether it is its window's active pane.
    pub is_active: bool,
}

impl PaneInfo {
    /// What `PaneInfo(id:windowId:)` makes: the defaults of the Swift init.
    pub fn placeholder(id: &str, window_id: &str) -> Self {
        Self {
            id: id.to_owned(),
            window_id: window_id.to_owned(),
            index: 0,
            command: String::new(),
            path: None,
            width: 80,
            height: 24,
            is_active: false,
        }
    }

    /// What to call the foreground process (`PaneInfo.displayCommand`):
    /// Claude Code's native install runs a binary named after its version,
    /// so a bare `N.N.N` comm is claude too.
    pub fn display_command(&self) -> &str {
        display_name_for_command(&self.command)
    }
}

/// `PaneInfo.displayCommand` for any comm name. The Swift regex's `\d` is
/// ICU's (any decimal digit); tmux reports comm names, which are ASCII in
/// every case that matters here, so ASCII digits are matched.
pub fn display_name_for_command(command: &str) -> &str {
    if command.to_lowercase().contains("claude") {
        return "claude";
    }
    let mut groups = 0;
    let mut all_digits = true;
    for group in command.split('.') {
        groups += 1;
        all_digits &= !group.is_empty() && group.bytes().all(|b| b.is_ascii_digit());
    }
    if groups == 3 && all_digits {
        return "claude";
    }
    command
}

/// A tmux window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// `@<n>`.
    pub id: String,
    /// The session it is linked into, `$<n>`; empty when unknown.
    pub session_id: String,
    pub name: String,
    pub index: i64,
    /// `#{window_layout}` verbatim.
    pub layout: String,
    /// Whether it is its session's current window.
    pub is_active: bool,
    pub pane_count: i64,
    /// Whether its active pane is zoomed (from `%layout-change` flags).
    pub is_zoomed: bool,
}

impl WindowInfo {
    /// `WindowInfo(id:sessionId:)` with the Swift init's defaults.
    pub fn new(id: &str, session_id: &str) -> Self {
        Self {
            id: id.to_owned(),
            session_id: session_id.to_owned(),
            name: String::new(),
            index: 0,
            layout: String::new(),
            is_active: false,
            pane_count: 1,
            is_zoomed: false,
        }
    }
}

/// A tmux session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// `$<n>`.
    pub id: String,
    pub name: String,
    /// Whether any client is attached to it (`#{session_attached}`).
    pub is_attached: bool,
}

/// A session/window/pane selection, replayed after a reconnect
/// (`TmuxSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub session: String,
    pub window: Option<String>,
    pub pane: Option<String>,
}

/// The state the UI draws from (`TmuxSnapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub sessions: BTreeMap<String, SessionInfo>,
    pub windows: BTreeMap<String, WindowInfo>,
    pub panes: BTreeMap<String, PaneInfo>,
    pub active_session_id: Option<String>,
    pub active_window_id: Option<String>,
    pub active_pane_id: Option<String>,
    pub is_attached: bool,
    /// Latches once attached this connection.
    pub ever_attached: bool,
    /// Direction of the last swipe switch (`true` = forward).
    pub last_switch_forward: bool,
    /// Whether a swipe has switched panes since attach.
    pub has_swiped_since_attach: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            sessions: BTreeMap::new(),
            windows: BTreeMap::new(),
            panes: BTreeMap::new(),
            active_session_id: None,
            active_window_id: None,
            active_pane_id: None,
            is_attached: false,
            ever_attached: false,
            last_switch_forward: true,
            has_swiped_since_attach: false,
        }
    }
}

impl Snapshot {
    /// The panes in id order (the order tmux made them).
    pub fn panes_in_id_order(&self) -> Vec<&PaneInfo> {
        let mut panes: Vec<&PaneInfo> = self.panes.values().collect();
        panes.sort_by(|a, b| id_cmp(&a.id, &b.id));
        panes
    }

    /// The windows in id order.
    pub fn windows_in_id_order(&self) -> Vec<&WindowInfo> {
        let mut windows: Vec<&WindowInfo> = self.windows.values().collect();
        windows.sort_by(|a, b| id_cmp(&a.id, &b.id));
        windows
    }

    /// The active window's panes, in id order (`activePanes`).
    pub fn active_panes(&self) -> Vec<&PaneInfo> {
        let Some(window) = self.active_window_id.as_deref() else {
            return Vec::new();
        };
        self.panes_in_id_order()
            .into_iter()
            .filter(|pane| pane.window_id == window)
            .collect()
    }

    /// The sessions in the order they were made (`sortedSessions`).
    pub fn sorted_sessions(&self) -> Vec<&SessionInfo> {
        let mut sessions: Vec<&SessionInfo> = self.sessions.values().collect();
        sessions.sort_by(|a, b| id_cmp(&a.id, &b.id));
        sessions
    }

    /// The active session's windows by index; every window by index when
    /// no session is active (`sortedWindows`).
    pub fn sorted_windows(&self) -> Vec<&WindowInfo> {
        match self.active_session_id.as_deref() {
            Some(session) => self.windows_in_session(session),
            None => {
                let mut windows = self.windows_in_id_order();
                windows.sort_by_key(|window| window.index);
                windows
            }
        }
    }

    /// A session's windows by index.
    pub fn windows_in_session(&self, session: &str) -> Vec<&WindowInfo> {
        let mut windows: Vec<&WindowInfo> = self
            .windows_in_id_order()
            .into_iter()
            .filter(|window| window.session_id == session)
            .collect();
        windows.sort_by_key(|window| window.index);
        windows
    }

    /// A window's panes by index (`panes(inWindow:)`).
    pub fn panes_in_window(&self, window: &str) -> Vec<&PaneInfo> {
        let mut panes: Vec<&PaneInfo> = self
            .panes_in_id_order()
            .into_iter()
            .filter(|pane| pane.window_id == window)
            .collect();
        panes.sort_by_key(|pane| pane.index);
        panes
    }

    /// The active pane, or the active window's first: the pane input and
    /// repaints go to (`activePaneId ?? activePanes.first?.id`).
    pub fn input_pane(&self) -> Option<String> {
        self.active_pane_id
            .clone()
            .or_else(|| self.active_panes().first().map(|pane| pane.id.clone()))
    }
}

/// The order tmux mints ids in: `$2` before `$10`, `%9` before `%11`
/// (`TmuxSnapshot.idPrecedes`, Foundation's `.numeric` comparison): runs of
/// ASCII digits compare by value, everything else by Unicode scalar; a
/// prefix comes first. Equal values that differ in leading zeros fall back
/// to plain string order, so this is a total order.
pub fn id_cmp(a: &str, b: &str) -> Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        if x[i].is_ascii_digit() && y[j].is_ascii_digit() {
            let start_i = i;
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            let start_j = j;
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }
            let digits_x = trim_leading_zeros(&x[start_i..i]);
            let digits_y = trim_leading_zeros(&y[start_j..j]);
            let by_value = digits_x
                .len()
                .cmp(&digits_y.len())
                .then_with(|| digits_x.cmp(digits_y));
            if by_value != Ordering::Equal {
                return by_value;
            }
            continue;
        }
        // Compare one scalar from each side.
        let cx = a[i..].chars().next().expect("char boundary");
        let cy = b[j..].chars().next().expect("char boundary");
        if cx != cy {
            return cx.cmp(&cy);
        }
        i += cx.len_utf8();
        j += cy.len_utf8();
    }
    (x.len() - i).cmp(&(y.len() - j)).then_with(|| a.cmp(b))
}

/// `TmuxSnapshot.idPrecedes`.
pub fn id_precedes(a: &str, b: &str) -> bool {
    id_cmp(a, b) == Ordering::Less
}

fn trim_leading_zeros(digits: &[u8]) -> &[u8] {
    let zeros = digits.iter().take_while(|&&b| b == b'0').count();
    &digits[zeros.min(digits.len().saturating_sub(1))..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_number() {
        assert!(id_precedes("$2", "$10"));
        assert!(id_precedes("%9", "%11"));
        assert!(id_precedes("w1:p2", "w1:p10"));
        assert!(!id_precedes("$2", "$2"));
        assert!(id_precedes("$1", "$1x"));
        assert_eq!(id_cmp("@1", "@01"), "@1".cmp("@01"));
    }

    #[test]
    fn claude_by_comm_name() {
        assert_eq!(display_name_for_command("2.1.269"), "claude");
        assert_eq!(display_name_for_command("Claude"), "claude");
        assert_eq!(display_name_for_command("2.1"), "2.1");
        assert_eq!(display_name_for_command("zsh"), "zsh");
    }
}
