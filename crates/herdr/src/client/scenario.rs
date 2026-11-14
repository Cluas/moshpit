//! Scenarios for the equivalence runs against the app's Swift control
//! client (`harness/client/`): one script, run by this crate's client on
//! the rig and by `HerdrControlClient.swift` on the same fake host (served
//! by `examples/herdr_host.rs`), each printing the same log.
//!
//! A script is lines; `#` starts a comment.
//!
//! ```text
//! host delay 40                 the host's default exec delay (ms)
//! host rule <pattern> <delay|-> <normal|hang|unreachable|cut|empty>
//! host version 0.9.5            host client-version 0.8.0
//! host running false            host mismatch <message…>
//! host detect 1500              host focus w1:p2
//! host dirty w7                 (a worktree workspace with a dirty checkout)
//! host clean w7                 (a worktree workspace, clean)
//! host no-version               (snapshots don't name the version)
//! at <ms> <action>              at that time after the start
//! end <ms>
//! ```
//!
//! Actions: `start`, `select-pane P`, `select-session S`, `select-window W`,
//! `new-session NAME|-`, `new-window NAME|-`, `new-pane`,
//! `new-window-in S NAME|-`, `new-pane-in W`, `rename-session S NAME`,
//! `kill-session S`, `rename-window W NAME`, `kill-window W`, `kill-pane P`,
//! `refresh`, `task REPO BRANCH AGENT PROMPT…` (`-` for an empty prompt),
//! `cancel-task`, `remove S 0|1`, `repos`, `agents`, `restart`,
//! `confirm-focus MS`, `host-focus P`, `host-running true|false`,
//! `host-mismatch MESSAGE…|-`, `host-rule …` (as above), `host-clear` (no
//! rules any more).
//!
//! The log: `t=<ms> exec <command without the PATH prefix and trailer>`,
//! `t=<ms> focus <pane>` (a focused-pane report), `t=<ms> mismatch <msg>`,
//! `t=<ms> outcome <what> <text>`, then at the end the host's `landed`
//! mutations in order and a `state` line.

use super::rig::{Fate, Rig, Rule, short};
use super::{Action, Config, Event, Output, RestartOutcome};
use crate::control::{AgentTaskRequest, TaskOutcome, WorktreeRemoval};

/// One parsed script.
#[derive(Debug, Clone, Default)]
pub struct Script {
    pub host: Vec<Vec<String>>,
    pub steps: Vec<(u64, Vec<String>)>,
    pub end: u64,
}

pub fn parse(text: &str) -> Result<Script, String> {
    let mut script = Script::default();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        match words[0].as_str() {
            "host" => script.host.push(words[1..].to_vec()),
            "at" => {
                let at = words
                    .get(1)
                    .and_then(|at| at.parse().ok())
                    .ok_or_else(|| format!("line {}: bad time", number + 1))?;
                script.steps.push((at, words[2..].to_vec()));
            }
            "end" => {
                script.end = words
                    .get(1)
                    .and_then(|at| at.parse().ok())
                    .ok_or_else(|| format!("line {}: bad end", number + 1))?;
            }
            other => return Err(format!("line {}: unknown {other:?}", number + 1)),
        }
    }
    script.steps.sort_by_key(|(at, _)| *at);
    Ok(script)
}

/// Apply one `host …` setting to the fake host.
pub fn apply_host(host: &mut super::rig::FakeHerdr, words: &[String]) -> Result<(), String> {
    let word = |index: usize| words.get(index).cloned().unwrap_or_default();
    let rest = |index: usize| words.get(index..).map(|w| w.join(" ")).unwrap_or_default();
    match word(0).as_str() {
        "delay" => host.default_delay = word(1).parse().map_err(|_| "bad delay")?,
        "rule" => {
            let fate = match word(3).as_str() {
                "normal" => Fate::Normal,
                "hang" => Fate::Hang,
                "unreachable" => Fate::Unreachable,
                "cut" => Fate::CutTrailer,
                "empty" => Fate::Output(String::new()),
                other => return Err(format!("bad fate {other:?}")),
            };
            host.rules.push(Rule {
                pattern: word(1).replace('_', " "),
                delay: word(2).parse().ok(),
                fate,
            });
        }
        "version" => host.version = word(1),
        "client-version" => host.client_version = word(1),
        "running" => host.running = word(1) == "true",
        "mismatch" => {
            host.mismatch = (word(1) != "-").then(|| rest(1));
        }
        "detect" => host.detect_after = word(1).parse().map_err(|_| "bad detect")?,
        "focus" => host.focused = Some(word(1)),
        "dirty" | "clean" => {
            let workspace = word(1);
            if !host.workspaces.iter().any(|(id, _)| *id == workspace) {
                host.workspaces.push((workspace.clone(), workspace.clone()));
            }
            host.worktrees.insert(workspace, word(0) == "dirty");
        }
        "no-version" => host.snapshot_names_version = false,
        "clear" => host.rules.clear(),
        "empty" => {
            host.workspaces.clear();
            host.tabs.clear();
            host.panes.clear();
            host.focused = None;
        }
        other => return Err(format!("unknown host setting {other:?}")),
    }
    Ok(())
}

/// The client's action for a script step (`None` for the host's own).
pub fn action(words: &[String]) -> Result<Option<Action>, String> {
    let word = |index: usize| words.get(index).cloned().unwrap_or_default();
    let name = |index: usize| Some(word(index)).filter(|name| name != "-" && !name.is_empty());
    Ok(Some(match word(0).as_str() {
        "select-pane" => Action::SelectPane(word(1)),
        "select-session" => Action::SelectSession(word(1)),
        "select-window" => Action::SelectWindow(word(1)),
        "new-session" => Action::NewSession { name: name(1) },
        "new-window" => Action::NewWindow { name: name(1) },
        "new-pane" => Action::NewPane,
        "new-window-in" => Action::NewWindowInSession {
            session: word(1),
            name: name(2),
        },
        "new-pane-in" => Action::NewPaneInWindow(word(1)),
        "rename-session" => Action::RenameSession {
            session: word(1),
            name: word(2),
        },
        "kill-session" => Action::KillSession(word(1)),
        "rename-window" => Action::RenameWindow {
            window: word(1),
            name: word(2),
        },
        "kill-window" => Action::KillWindow(word(1)),
        "kill-pane" => Action::KillPane(word(1)),
        "refresh" => Action::Refresh,
        "task" => {
            let prompt = words.get(4..).map(|w| w.join(" ")).unwrap_or_default();
            Action::StartAgentTask(AgentTaskRequest {
                repo_path: word(1),
                branch: word(2),
                agent: word(3),
                prompt: if prompt == "-" { String::new() } else { prompt },
            })
        }
        "remove" => Action::RemoveWorktree {
            session: word(1),
            force: word(2) == "1",
        },
        "repos" => Action::DiscoverRepos,
        "agents" => Action::AgentNames,
        "restart" => Action::RestartServer,
        "confirm-focus" => Action::ConfirmFocus {
            budget_ms: word(1).parse().map_err(|_| "bad budget")?,
        },
        "start" | "cancel-task" | "host-focus" | "host-running" | "host-mismatch" | "host-rule"
        | "host-clear" => {
            return Ok(None);
        }
        other => return Err(format!("unknown action {other:?}")),
    }))
}

/// The outcome texts both sides print.
pub fn task_text(outcome: &TaskOutcome) -> String {
    match outcome {
        TaskOutcome::Started => "started".to_owned(),
        TaskOutcome::Failed(_) => format!("failed {}", outcome.english().unwrap_or_default()),
        TaskOutcome::Incomplete(_) => {
            format!("incomplete {}", outcome.english().unwrap_or_default())
        }
    }
}

pub fn removal_text(outcome: &WorktreeRemoval) -> String {
    match outcome {
        WorktreeRemoval::Removed => "removed".to_owned(),
        WorktreeRemoval::NeedsForce => "needs-force".to_owned(),
        WorktreeRemoval::Failed(failure) => format!("failed {}", failure.english()),
    }
}

pub fn restart_text(outcome: &RestartOutcome) -> String {
    match outcome {
        RestartOutcome::Restarted => "restarted".to_owned(),
        RestartOutcome::Failed => "failed".to_owned(),
        RestartOutcome::WouldDowngrade(report) => {
            format!("would-downgrade {} {}", report.client, report.server)
        }
        RestartOutcome::Unneeded => "unneeded".to_owned(),
    }
}

/// Run a script with this crate's client on the rig (control plane only,
/// as the Swift harness runs the app's: no frame channel), returning the
/// log.
pub fn run(text: &str) -> Result<String, String> {
    let script = parse(text)?;
    let config = Config {
        frame_channel: false,
        ..Config::default()
    };
    let mut rig = Rig::new(config);
    let origin = rig.now;
    for words in &script.host {
        apply_host(&mut rig.host, words)?;
    }
    let mut log = Vec::new();
    let mut seen = 0;
    let mut last_task = None;
    let collect = |rig: &Rig, seen: &mut usize, log: &mut Vec<String>| {
        for (at, output) in &rig.log[*seen..] {
            let t = at - origin;
            match output {
                Output::Exec { command, .. } => log.push(format!("t={t} exec {}", short(command))),
                Output::Event(event) => match event {
                    Event::FocusedPane(pane) => log.push(format!("t={t} focus {pane}")),
                    Event::Task { outcome, .. } => {
                        log.push(format!("t={t} outcome task {}", task_text(outcome)))
                    }
                    Event::Removal { outcome, .. } => {
                        log.push(format!("t={t} outcome removal {}", removal_text(outcome)))
                    }
                    Event::Repos { repos, .. } => {
                        log.push(format!("t={t} outcome repos [{}]", repos.join(" ")))
                    }
                    Event::Agents { names, .. } => {
                        log.push(format!("t={t} outcome agents [{}]", names.join(" ")))
                    }
                    Event::Restart { outcome, .. } => {
                        log.push(format!("t={t} outcome restart {}", restart_text(outcome)))
                    }
                    Event::FocusConfirmed { pane, .. } => log.push(format!(
                        "t={t} outcome confirm {}",
                        pane.as_deref().unwrap_or("nil")
                    )),
                    _ => {}
                },
                _ => {}
            }
        }
        *seen = rig.log.len();
    };
    for (at, words) in &script.steps {
        rig.run_until(origin + at);
        match words.first().map(String::as_str) {
            Some("start") => rig.start(),
            Some("cancel-task") => {
                if let Some(request) = last_task {
                    rig.act(Action::CancelAgentTask(request));
                }
            }
            Some("host-focus") => rig.host.focused = words.get(1).cloned(),
            Some("host-running") => {
                rig.host.running = words.get(1).map(String::as_str) == Some("true")
            }
            Some("host-mismatch") => {
                let mut setting = vec!["mismatch".to_owned()];
                setting.extend(words[1..].iter().cloned());
                apply_host(&mut rig.host, &setting)?;
            }
            Some("host-rule") => {
                let mut setting = vec!["rule".to_owned()];
                setting.extend(words[1..].iter().cloned());
                apply_host(&mut rig.host, &setting)?;
            }
            Some("host-clear") => rig.host.rules.clear(),
            _ => {
                if let Some(action) = action(words)? {
                    let task = matches!(action, Action::StartAgentTask(_));
                    let request = rig.act(action);
                    if task {
                        last_task = request;
                    }
                }
            }
        }
        collect(&rig, &mut seen, &mut log);
    }
    rig.run_until(origin + script.end);
    collect(&rig, &mut seen, &mut log);
    for landed in &rig.host.landed {
        log.push(format!("landed {landed}"));
    }
    let snapshot = rig.client.snapshot();
    log.push(format!(
        "state focus={} server={} sessions={} notrunning={} mismatch={}",
        snapshot.active_pane_id.as_deref().unwrap_or("nil"),
        rig.host.focused.as_deref().unwrap_or("nil"),
        snapshot.sessions.len(),
        rig.client.server_not_running(),
        rig.client.protocol_mismatch().is_some(),
    ));
    Ok(log.join("\n") + "\n")
}
