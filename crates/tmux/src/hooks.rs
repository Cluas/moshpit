//! The agent stamps coding-agent hooks write into tmux `@moshpit_*` user
//! options, read for every pane in one `list-panes -a` (the 2 s poll behind
//! notifications and the Live Activity): `AgentHook` and
//! `parseAgentHooks` of `TmuxSessionController.swift`.

use std::collections::BTreeMap;

use crate::format::{fields, swift_double};

/// The `-F` argument, quoted: `pane_id|state|agent|since|command|title`. An
/// unset user option expands to "", so a never-stamped pane reads `%5||||x|`;
/// the title may hold `|` and is the rest of the line.
pub const AGENT_HOOK_FORMAT: &str = "'#{pane_id}|#{@moshpit_state}|#{@moshpit_agent}|#{@moshpit_since}|#{pane_current_command}|#{@moshpit_title}'";

/// One pane's stamp.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentHook {
    /// `working`, `attention` or `done`; `None` without precise hook data
    /// (unset or unrecognised: the monitor falls back to its heuristic).
    pub state: Option<String>,
    /// `@moshpit_agent`, e.g. `claude`.
    pub agent: Option<String>,
    /// `@moshpit_since` as Unix seconds.
    pub since: Option<f64>,
    /// What runs in the pane NOW (`#{pane_current_command}`), polled fresh
    /// every round: how the monitor spots a stamp whose agent died.
    pub command: Option<String>,
    /// `@moshpit_title`.
    pub title: Option<String>,
}

/// The poll's rows, by pane id. A pane with an empty or unknown state is
/// kept with `state: None` — "no hook data", not "pane gone".
pub fn parse_agent_hooks<S: AsRef<str>>(lines: &[S]) -> BTreeMap<String, AgentHook> {
    let mut parsed = BTreeMap::new();
    for line in lines {
        let parts = fields(line.as_ref(), b'|', 5, false);
        if parts.len() != 6 || parts[0].is_empty() {
            continue;
        }
        let state = match parts[1] {
            "working" | "attention" | "done" => Some(parts[1].to_owned()),
            _ => None,
        };
        let text = |field: &str| (!field.is_empty()).then(|| field.to_owned());
        parsed.insert(
            parts[0].to_owned(),
            AgentHook {
                state,
                agent: text(parts[2]),
                since: swift_double(parts[3]),
                command: text(parts[4]),
                title: text(parts[5]),
            },
        );
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_become_stamps() {
        let hooks = parse_agent_hooks(&[
            "%5|working|claude|1758000000|2.1.269|fixing | the scroll",
            "%6||||zsh|",
            "%7|bogus|codex|soon|node|",
            "|working|claude|1|x|",
            "%8|done",
        ]);
        assert_eq!(hooks.len(), 3);
        let five = &hooks["%5"];
        assert_eq!(five.state.as_deref(), Some("working"));
        assert_eq!(five.agent.as_deref(), Some("claude"));
        assert_eq!(five.since, Some(1_758_000_000.0));
        assert_eq!(five.title.as_deref(), Some("fixing | the scroll"));
        assert_eq!(hooks["%6"].state, None);
        assert_eq!(hooks["%6"].command.as_deref(), Some("zsh"));
        assert_eq!(hooks["%7"].state, None);
        assert_eq!(hooks["%7"].since, None);
    }
}
