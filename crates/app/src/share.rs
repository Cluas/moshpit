//! Where an image shared to an agent while its session was away may go
//! when the session is live again (B622).
//!
//! A share is queued for a pane (its id on the connection) and the agent it
//! showed there (`agentName`: the pane's stamped agent, else its foreground
//! command, as the island listed it). By the time it is delivered, up to 48
//! hours later, that pane may be gone, run a shell (the agent exited), or
//! run something else under the same id (tmux numbers panes again from %0
//! after its server restarts). The path is never typed into such a pane:
//! the share is held back and the person is told, never dropped silently.
//! A pane that moved to another window keeps its id in tmux, so it is
//! still found.

/// A pane as the session shows it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneNow {
    pub id: String,
    /// The pane's stamped agent (`@moshpit_agent`), if any.
    pub agent: Option<String>,
    /// What runs in its foreground now.
    pub command: String,
}

/// Where a queued share goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Into this pane: it still runs the agent the share was queued for.
    Deliver { pane: String },
    /// Nowhere: the pane is gone.
    PaneGone,
    /// Nowhere: the agent exited, its pane runs this shell now.
    AgentExited { shell: String },
    /// Nowhere: the pane runs another program now.
    AnotherProgram { now: String },
}

/// What an agent's process may be named instead of the agent: the runtime
/// it runs on (Gemini and Qwen run on node, Aider on python).
const RUNTIMES: &[&str] = &["node", "bun", "deno", "python", "python3"];

/// Where a share queued for `pane`, to `agent`, goes among `panes`.
pub fn target(pane: &str, agent: &str, panes: &[PaneNow]) -> Target {
    let Some(now) = panes.iter().find(|p| p.id == pane) else {
        return Target::PaneGone;
    };
    if moshpit_agents::is_shell(&now.command) {
        return Target::AgentExited {
            shell: now.command.clone(),
        };
    }
    // The process says who runs there now; a stamp says who ran there (it
    // outlives its agent), so it names the pane only while the process is
    // a runtime an agent runs on.
    let running = match now.agent.as_deref() {
        Some(stamped) if RUNTIMES.contains(&now.command.as_str()) => stamped,
        _ => now.command.as_str(),
    };
    if same_agent(running, agent) {
        Target::Deliver {
            pane: now.id.clone(),
        }
    } else {
        Target::AnotherProgram {
            now: running.to_owned(),
        }
    }
}

/// One agent named two ways: case aside, and Claude Code, whose process is
/// named by its version ("2.0.31"), is `claude`.
fn same_agent(a: &str, b: &str) -> bool {
    canonical(a) == canonical(b)
}

fn canonical(name: &str) -> String {
    let name = name.trim().to_lowercase();
    if is_semver(&name) {
        "claude".to_owned()
    } else {
        name
    }
}

/// `^\d+\.\d+\.\d+$`, digits being ASCII.
fn is_semver(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, agent: Option<&str>, command: &str) -> PaneNow {
        PaneNow {
            id: id.into(),
            agent: agent.map(str::to_owned),
            command: command.into(),
        }
    }

    #[test]
    fn the_pane_still_running_its_agent_takes_the_share() {
        let panes = [
            pane("%2", None, "zsh"),
            pane("%3", Some("claude"), "2.0.31"),
        ];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::Deliver { pane: "%3".into() }
        );
        // Named by its process alone, a version number is Claude Code.
        let panes = [pane("%3", None, "2.0.31")];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::Deliver { pane: "%3".into() }
        );
        assert_eq!(
            target("%3", "Claude", &[pane("%3", Some("claude"), "node")]),
            Target::Deliver { pane: "%3".into() }
        );
    }

    #[test]
    fn a_tmux_restart_that_gave_the_id_to_another_program_holds_the_share_back() {
        // The server restarted: %3 is vim now, and nothing stamps it.
        let panes = [pane("%0", None, "zsh"), pane("%3", None, "vim")];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::AnotherProgram { now: "vim".into() }
        );
    }

    #[test]
    fn an_agent_back_at_its_shell_holds_the_share_back() {
        // Its stamp stayed (the Stop hook never ran): the shell decides.
        let panes = [pane("%3", Some("claude"), "zsh")];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::AgentExited {
                shell: "zsh".into()
            }
        );
    }

    #[test]
    fn a_pane_moved_to_another_window_keeps_its_id_and_its_share() {
        // tmux's move-pane/join-pane keep the %id: the pane is found by it
        // wherever it went (its window is not part of the question).
        let panes = [pane("%7", None, "zsh"), pane("%3", Some("codex"), "codex")];
        assert_eq!(
            target("%3", "codex", &panes),
            Target::Deliver { pane: "%3".into() }
        );
    }

    #[test]
    fn a_stale_stamp_over_another_program_holds_the_share_back() {
        // Claude exited, its stamp stayed, and vim runs in the pane now:
        // the stamp names who ran there, the process who runs there now.
        let panes = [pane("%3", Some("claude"), "vim")];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::AnotherProgram { now: "vim".into() }
        );
        // Another agent took the pane: its own stamp and process say so.
        let panes = [pane("%3", Some("codex"), "codex")];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::AnotherProgram { now: "codex".into() }
        );
        // An agent that runs on node is stamped: the stamp names it.
        let panes = [pane("%3", Some("gemini"), "node")];
        assert_eq!(
            target("%3", "claude", &panes),
            Target::AnotherProgram { now: "gemini".into() }
        );
        assert_eq!(
            target("%3", "gemini", &panes),
            Target::Deliver { pane: "%3".into() }
        );
    }

    #[test]
    fn a_pane_that_is_gone_holds_the_share_back() {
        assert_eq!(
            target("%3", "claude", &[pane("%1", Some("claude"), "claude")]),
            Target::PaneGone
        );
    }
}
