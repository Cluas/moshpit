//! herdr on the Rust session, as the hosts see it (UniFFI): the read model
//! the chrome draws from (the tree, the agents' stamps, the banner), the
//! person's intents, and what the ones with an answer come back with.
//!
//! The same shape the tmux read model has, so the host's herdr surface
//! mirrors it the way it mirrors tmux's: the tree as tmux's sessions,
//! windows and panes (herdr's workspaces, tabs and panes), and every notice
//! and outcome as a kind with its fields — the host words them (B202). What
//! herdr itself said (an error message, a refusal's text) crosses as herdr
//! said it.

use std::collections::HashMap;

use moshpit_herdr::client::{Action, Notice, NoticeAction, RestartOutcome};
use moshpit_herdr::control::{
    AgentTaskRequest, IDLE_POLL_INTERVAL, PUSHED_POLL_INTERVAL, Refusal, RemovalFailure,
    TaskFailure, TaskIncomplete, TaskInvalid, TaskOutcome, WorktreeRemoval,
};
use moshpit_herdr::frame::{MouseAction, MouseButton, ScrollSource};
use moshpit_herdr::version::VersionNotice;
use moshpit_session::{HerdrAction, HerdrAnswer, HerdrView};

use crate::session::{AgentHookValue, TmuxPaneValue, TmuxSessionValue, TmuxWindowValue};

/// What the person (or the UI) asks of herdr.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum HerdrIntent {
    SelectSession {
        id: String,
    },
    SelectWindow {
        id: String,
    },
    SelectPane {
        id: String,
    },
    /// The empty state's "create": a workspace, or a server when there is
    /// none.
    NewSession {
        name: Option<String>,
    },
    NewWindow {
        name: Option<String>,
    },
    NewPane,
    NewWindowInSession {
        session: String,
        name: Option<String>,
    },
    NewPaneInWindow {
        window: String,
    },
    RenameSession {
        session: String,
        name: String,
    },
    KillSession {
        id: String,
    },
    RenameWindow {
        window: String,
        name: String,
    },
    KillWindow {
        id: String,
    },
    KillPane {
        id: String,
    },
    /// The refresh control: read now, with the spinner.
    Refresh,
    /// The New agent task form's submit.
    StartAgentTask {
        repo_path: String,
        branch: String,
        agent: String,
        prompt: String,
    },
    RemoveWorktree {
        session: String,
        force: bool,
    },
    DiscoverRepos,
    AgentNames,
    /// The banner's restart button.
    RestartServer,
    /// The banner's button, whatever it offers.
    NoticeAction,
    /// The server's focus once the commands queued so far have landed.
    ConfirmFocus {
        budget_ms: u64,
    },
    /// A drag or wheel over `pane` (`page_keys`: the paging keys).
    Scroll {
        pane: String,
        lines: i64,
        page_keys: bool,
    },
    Mouse {
        pane: String,
        action: HerdrMouseAction,
        button: HerdrMouseButton,
        column: i64,
        row: i64,
        modifiers: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrMouseAction {
    Down,
    Up,
    Drag,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrMouseButton {
    Left,
    Right,
    Middle,
}

impl HerdrIntent {
    pub(crate) fn core(self) -> HerdrAction {
        match self {
            HerdrIntent::SelectSession { id } => Action::SelectSession(id),
            HerdrIntent::SelectWindow { id } => Action::SelectWindow(id),
            HerdrIntent::SelectPane { id } => Action::SelectPane(id),
            HerdrIntent::NewSession { name } => Action::NewSession { name },
            HerdrIntent::NewWindow { name } => Action::NewWindow { name },
            HerdrIntent::NewPane => Action::NewPane,
            HerdrIntent::NewWindowInSession { session, name } => {
                Action::NewWindowInSession { session, name }
            }
            HerdrIntent::NewPaneInWindow { window } => Action::NewPaneInWindow(window),
            HerdrIntent::RenameSession { session, name } => Action::RenameSession { session, name },
            HerdrIntent::KillSession { id } => Action::KillSession(id),
            HerdrIntent::RenameWindow { window, name } => Action::RenameWindow { window, name },
            HerdrIntent::KillWindow { id } => Action::KillWindow(id),
            HerdrIntent::KillPane { id } => Action::KillPane(id),
            HerdrIntent::Refresh => Action::Refresh,
            HerdrIntent::StartAgentTask {
                repo_path,
                branch,
                agent,
                prompt,
            } => Action::StartAgentTask(AgentTaskRequest {
                repo_path,
                branch,
                agent,
                prompt,
            }),
            HerdrIntent::RemoveWorktree { session, force } => {
                Action::RemoveWorktree { session, force }
            }
            HerdrIntent::DiscoverRepos => Action::DiscoverRepos,
            HerdrIntent::AgentNames => Action::AgentNames,
            HerdrIntent::RestartServer => Action::RestartServer,
            HerdrIntent::NoticeAction => Action::NoticeAction,
            HerdrIntent::ConfirmFocus { budget_ms } => Action::ConfirmFocus { budget_ms },
            HerdrIntent::Scroll {
                pane,
                lines,
                page_keys,
            } => Action::Scroll {
                pane,
                lines,
                source: if page_keys {
                    ScrollSource::PageKey
                } else {
                    ScrollSource::Wheel
                },
            },
            HerdrIntent::Mouse {
                pane,
                action,
                button,
                column,
                row,
                modifiers,
            } => Action::Mouse {
                pane,
                action: match action {
                    HerdrMouseAction::Down => MouseAction::Down,
                    HerdrMouseAction::Up => MouseAction::Up,
                    HerdrMouseAction::Drag => MouseAction::Drag,
                    HerdrMouseAction::Move => MouseAction::Move,
                },
                button: match button {
                    HerdrMouseButton::Left => MouseButton::Left,
                    HerdrMouseButton::Right => MouseButton::Right,
                    HerdrMouseButton::Middle => MouseButton::Middle,
                },
                column,
                row,
                modifiers,
            },
        }
    }
}

// MARK: - Answers

/// Why a command didn't do its job.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrRefusalValue {
    /// It never ran: the host couldn't be reached.
    Unreachable,
    /// herdr's own words.
    Said { text: String },
    /// It failed and printed nothing.
    ExitStatus { code: i64 },
    /// The answer was cut off before herdr said it was done.
    CutOff,
}

/// Why the New agent task form's request can't go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrTaskInvalidValue {
    PickRepository,
    PickAgent,
    NameBranch,
    BranchSpace,
    BranchEdge,
    BranchDotDot,
    BranchLock,
    BranchForbidden,
    BranchControl,
}

/// Nothing was made.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrTaskFailureValue {
    Invalid {
        invalid: HerdrTaskInvalidValue,
    },
    Unreachable,
    CreateFailed,
    /// herdr didn't confirm the worktree: look for this workspace.
    Unconfirmed {
        branch: String,
    },
    /// No answer in time: it may still land as this workspace.
    HostStoppedAnswering {
        branch: String,
    },
    /// herdr's own error text.
    Herdr {
        message: String,
    },
}

/// The worktree exists; what came after it didn't land.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrTaskIncompleteValue {
    NoPane,
    Cancelled,
    LaunchUnanswered {
        agent: String,
    },
    AgentDidNotStart {
        agent: String,
        reason: HerdrRefusalValue,
    },
    HostStoppedAnswering,
    MessageRefused {
        agent: String,
        reason: HerdrRefusalValue,
    },
    AgentNotReady {
        agent: String,
    },
    ReturnFailed {
        agent: String,
        reason: HerdrRefusalValue,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrTaskOutcomeValue {
    Started,
    Failed {
        failure: HerdrTaskFailureValue,
    },
    Incomplete {
        incomplete: HerdrTaskIncompleteValue,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrRemovalFailureValue {
    Unreachable,
    Herdr { message: String },
    Failed,
    CutOff,
    NoAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrRemovalValue {
    Removed,
    /// Modified or untracked files: confirm again to force it.
    NeedsForce,
    Failed {
        why: HerdrRemovalFailureValue,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrRestartValue {
    Restarted,
    Failed,
    /// Refused: restarting with the host's `herdr` would downgrade the
    /// server.
    WouldDowngrade {
        client: String,
        server: String,
    },
    /// Nothing done: the server already runs the host's `herdr` (B408).
    Unneeded,
}

/// What a herdr request came back with.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrAnswerValue {
    Task {
        outcome: HerdrTaskOutcomeValue,
    },
    Removal {
        outcome: HerdrRemovalValue,
    },
    Repos {
        repos: Vec<String>,
    },
    Agents {
        names: Vec<String>,
        answered: bool,
    },
    Restart {
        outcome: HerdrRestartValue,
    },
    Focus {
        pane: Option<String>,
    },
    Delivered {
        delivered: bool,
    },
    /// Nothing to report (a selection, a rename).
    Done,
}

fn refusal_value(refusal: Refusal) -> HerdrRefusalValue {
    match refusal {
        Refusal::Unreachable => HerdrRefusalValue::Unreachable,
        Refusal::Said(text) => HerdrRefusalValue::Said { text },
        Refusal::ExitStatus(code) => HerdrRefusalValue::ExitStatus { code },
        Refusal::CutOff => HerdrRefusalValue::CutOff,
    }
}

fn invalid_value(invalid: TaskInvalid) -> HerdrTaskInvalidValue {
    match invalid {
        TaskInvalid::PickRepository => HerdrTaskInvalidValue::PickRepository,
        TaskInvalid::PickAgent => HerdrTaskInvalidValue::PickAgent,
        TaskInvalid::NameBranch => HerdrTaskInvalidValue::NameBranch,
        TaskInvalid::BranchSpace => HerdrTaskInvalidValue::BranchSpace,
        TaskInvalid::BranchEdge => HerdrTaskInvalidValue::BranchEdge,
        TaskInvalid::BranchDotDot => HerdrTaskInvalidValue::BranchDotDot,
        TaskInvalid::BranchLock => HerdrTaskInvalidValue::BranchLock,
        TaskInvalid::BranchForbidden => HerdrTaskInvalidValue::BranchForbidden,
        TaskInvalid::BranchControl => HerdrTaskInvalidValue::BranchControl,
    }
}

fn task_value(outcome: TaskOutcome) -> HerdrTaskOutcomeValue {
    match outcome {
        TaskOutcome::Started => HerdrTaskOutcomeValue::Started,
        TaskOutcome::Failed(failure) => HerdrTaskOutcomeValue::Failed {
            failure: match failure {
                TaskFailure::Invalid(invalid) => HerdrTaskFailureValue::Invalid {
                    invalid: invalid_value(invalid),
                },
                TaskFailure::Unreachable => HerdrTaskFailureValue::Unreachable,
                TaskFailure::CreateFailed => HerdrTaskFailureValue::CreateFailed,
                TaskFailure::Unconfirmed { branch } => {
                    HerdrTaskFailureValue::Unconfirmed { branch }
                }
                TaskFailure::HostStoppedAnswering { branch } => {
                    HerdrTaskFailureValue::HostStoppedAnswering { branch }
                }
                TaskFailure::Herdr(message) => HerdrTaskFailureValue::Herdr { message },
            },
        },
        TaskOutcome::Incomplete(incomplete) => HerdrTaskOutcomeValue::Incomplete {
            incomplete: match incomplete {
                TaskIncomplete::NoPane => HerdrTaskIncompleteValue::NoPane,
                TaskIncomplete::Cancelled => HerdrTaskIncompleteValue::Cancelled,
                TaskIncomplete::LaunchUnanswered { agent } => {
                    HerdrTaskIncompleteValue::LaunchUnanswered { agent }
                }
                TaskIncomplete::AgentDidNotStart { agent, reason } => {
                    HerdrTaskIncompleteValue::AgentDidNotStart {
                        agent,
                        reason: refusal_value(reason),
                    }
                }
                TaskIncomplete::HostStoppedAnswering => {
                    HerdrTaskIncompleteValue::HostStoppedAnswering
                }
                TaskIncomplete::MessageRefused { agent, reason } => {
                    HerdrTaskIncompleteValue::MessageRefused {
                        agent,
                        reason: refusal_value(reason),
                    }
                }
                TaskIncomplete::AgentNotReady { agent } => {
                    HerdrTaskIncompleteValue::AgentNotReady { agent }
                }
                TaskIncomplete::ReturnFailed { agent, reason } => {
                    HerdrTaskIncompleteValue::ReturnFailed {
                        agent,
                        reason: refusal_value(reason),
                    }
                }
            },
        },
    }
}

fn removal_value(removal: WorktreeRemoval) -> HerdrRemovalValue {
    match removal {
        WorktreeRemoval::Removed => HerdrRemovalValue::Removed,
        WorktreeRemoval::NeedsForce => HerdrRemovalValue::NeedsForce,
        WorktreeRemoval::Failed(failure) => HerdrRemovalValue::Failed {
            why: match failure {
                RemovalFailure::Unreachable => HerdrRemovalFailureValue::Unreachable,
                RemovalFailure::Herdr(message) => HerdrRemovalFailureValue::Herdr { message },
                RemovalFailure::Failed => HerdrRemovalFailureValue::Failed,
                RemovalFailure::CutOff => HerdrRemovalFailureValue::CutOff,
                RemovalFailure::NoAnswer => HerdrRemovalFailureValue::NoAnswer,
            },
        },
    }
}

pub(crate) fn answer_value(answer: HerdrAnswer) -> HerdrAnswerValue {
    match answer {
        HerdrAnswer::Task(outcome) => HerdrAnswerValue::Task {
            outcome: task_value(outcome),
        },
        HerdrAnswer::Removal(outcome) => HerdrAnswerValue::Removal {
            outcome: removal_value(outcome),
        },
        HerdrAnswer::Repos(repos) => HerdrAnswerValue::Repos { repos },
        HerdrAnswer::Agents { names, answered } => HerdrAnswerValue::Agents { names, answered },
        HerdrAnswer::Restart(outcome) => HerdrAnswerValue::Restart {
            outcome: match outcome {
                RestartOutcome::Restarted => HerdrRestartValue::Restarted,
                RestartOutcome::Failed => HerdrRestartValue::Failed,
                RestartOutcome::WouldDowngrade(report) => HerdrRestartValue::WouldDowngrade {
                    client: report.client,
                    server: report.server,
                },
                RestartOutcome::Unneeded => HerdrRestartValue::Unneeded,
            },
        },
        HerdrAnswer::Focus(pane) => HerdrAnswerValue::Focus { pane },
        HerdrAnswer::Delivered(delivered) => HerdrAnswerValue::Delivered { delivered },
        HerdrAnswer::Done => HerdrAnswerValue::Done,
    }
}

// MARK: - The read model

/// The banner over a herdr terminal.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrNoticeValue {
    /// herdr refused the attach, in its own words.
    Refused {
        message: String,
    },
    /// The server refuses this client's protocol (herdr's words); which side
    /// is older isn't known.
    Mismatch {
        message: String,
    },
    /// The `herdr` on the host's PATH is older than its server: update it.
    ClientOlder {
        client: String,
        server: String,
    },
    /// The server is older than the host's `herdr`: restart it.
    ServerOlder {
        client: String,
        server: String,
    },
    /// Another device took the pane over, or holds the pane the focus went
    /// to ("This pane is open on another device"); never taken without the
    /// person ([`HerdrNoticeActionValue::OpenHere`]).
    OnAnotherDevice,
    /// Input was typed where no pane could take it.
    InputDropped,
    /// A reply (or a share) couldn't be put on its pane: nothing was sent.
    CouldntSwitch,
    Restarting,
    RestartFailed,
}

/// The banner's one-tap remedy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HerdrNoticeActionValue {
    RestartServer,
    Retry,
    /// "Open here": take the pane from the device that has it.
    OpenHere,
}

/// herdr as the chrome draws it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HerdrReadModel {
    /// Workspaces, tabs and panes, in tmux's shape.
    pub sessions: Vec<TmuxSessionValue>,
    pub windows: Vec<TmuxWindowValue>,
    pub panes: Vec<TmuxPaneValue>,
    pub active_session_id: Option<String>,
    pub active_window_id: Option<String>,
    pub active_pane_id: Option<String>,
    pub is_attached: bool,
    pub ever_attached: bool,
    pub agent_hooks: Vec<AgentHookValue>,
    pub agent_hooks_loaded: bool,
    pub notice: Option<HerdrNoticeValue>,
    pub notice_action: Option<HerdrNoticeActionValue>,
    pub refreshing: bool,
    pub restarting: bool,
    /// herdr is installed and its server isn't running: the empty state.
    pub server_not_running: bool,
    /// herdr's refusal while the server refuses this client.
    pub protocol_mismatch: Option<String>,
    /// The two versions while they disagree (`herdr status`).
    pub skew_client: Option<String>,
    pub skew_server: Option<String>,
    pub server_version: Option<String>,
    pub push_active: bool,
    pub pane_switch_in_flight: bool,
    /// Where each pane's program runs.
    pub pane_cwds: HashMap<String, String>,
    /// The repository each worktree workspace came from.
    pub worktree_repos: HashMap<String, String>,
    pub open_repo_guesses: Vec<String>,
    /// The pane the frame channel is pointed at, and the one taking input.
    pub frame_target: Option<String>,
    pub attached_pane: Option<String>,
    /// How long the backoff after a refusal is, while one runs: the
    /// banner counts it down from when it first sees it (B490).
    pub retry_wait_ms: Option<u64>,
}

pub(crate) fn read_model(view: &HerdrView) -> HerdrReadModel {
    let snapshot = &view.snapshot;
    let (notice, notice_action) = match &view.notice {
        Some((notice, action)) => (
            Some(match notice {
                Notice::Refused(message) => HerdrNoticeValue::Refused {
                    message: message.clone(),
                },
                Notice::Mismatch(message) => HerdrNoticeValue::Mismatch {
                    message: message.clone(),
                },
                Notice::Skew(VersionNotice::ClientOlder { client, server }) => {
                    HerdrNoticeValue::ClientOlder {
                        client: client.clone(),
                        server: server.clone(),
                    }
                }
                Notice::Skew(VersionNotice::ServerOlder { client, server }) => {
                    HerdrNoticeValue::ServerOlder {
                        client: client.clone(),
                        server: server.clone(),
                    }
                }
                Notice::OnAnotherDevice => HerdrNoticeValue::OnAnotherDevice,
                Notice::InputDropped => HerdrNoticeValue::InputDropped,
                Notice::CouldntSwitch => HerdrNoticeValue::CouldntSwitch,
                Notice::Restarting => HerdrNoticeValue::Restarting,
                Notice::RestartFailed => HerdrNoticeValue::RestartFailed,
            }),
            action.map(|action| match action {
                NoticeAction::RestartServer => HerdrNoticeActionValue::RestartServer,
                NoticeAction::Retry => HerdrNoticeActionValue::Retry,
                NoticeAction::OpenHere => HerdrNoticeActionValue::OpenHere,
            }),
        ),
        None => (None, None),
    };
    HerdrReadModel {
        sessions: snapshot
            .sessions
            .values()
            .map(|s| TmuxSessionValue {
                id: s.id.clone(),
                name: s.name.clone(),
                is_attached: s.is_attached,
            })
            .collect(),
        windows: snapshot
            .windows
            .values()
            .map(|w| TmuxWindowValue {
                id: w.id.clone(),
                session_id: w.session_id.clone(),
                name: w.name.clone(),
                index: w.index,
                layout: w.layout.clone(),
                is_active: w.is_active,
                pane_count: w.pane_count,
                is_zoomed: w.is_zoomed,
            })
            .collect(),
        panes: snapshot
            .panes
            .values()
            .map(|p| TmuxPaneValue {
                id: p.id.clone(),
                window_id: p.window_id.clone(),
                index: p.index,
                command: p.command.clone(),
                path: view.pane_cwds.get(&p.id).cloned(),
                width: p.width,
                height: p.height,
                is_active: p.is_active,
            })
            .collect(),
        active_session_id: snapshot.active_session_id.clone(),
        active_window_id: snapshot.active_window_id.clone(),
        active_pane_id: snapshot.active_pane_id.clone(),
        is_attached: snapshot.is_attached,
        ever_attached: snapshot.ever_attached,
        agent_hooks: view
            .agent_hooks
            .iter()
            .map(|(pane, hook)| AgentHookValue {
                pane: pane.clone(),
                state: hook.state.map(|state| state.as_str().to_owned()),
                agent: hook.agent.clone(),
                since: None,
                command: None,
                title: hook.title.clone(),
            })
            .collect(),
        agent_hooks_loaded: view.agent_hooks_loaded,
        notice,
        notice_action,
        refreshing: view.refreshing,
        restarting: view.restarting,
        server_not_running: view.server_not_running,
        protocol_mismatch: view.protocol_mismatch.clone(),
        skew_client: view.version_skew.as_ref().map(|skew| skew.client.clone()),
        skew_server: view.version_skew.as_ref().map(|skew| skew.server.clone()),
        server_version: view.server_version.clone(),
        push_active: view.push_active,
        pane_switch_in_flight: view.pane_switch_in_flight,
        pane_cwds: view.pane_cwds.clone().into_iter().collect(),
        worktree_repos: view.worktree_repos.clone().into_iter().collect(),
        open_repo_guesses: view.open_repo_guesses.clone(),
        frame_target: view.frame_target.clone(),
        attached_pane: view.attached_pane.clone(),
        retry_wait_ms: view.retry_wait_ms,
    }
}

/// How often herdr's agent stamps arrive on the core, in seconds: its
/// client's poll once nothing changes, or the poll that is only a safety
/// net while its push channel is up (`moshpit_herdr::control`). The agents
/// machine holds a stamp that long and a margin (B346).
#[uniffi::export]
pub fn herdr_stamp_interval(push_active: bool) -> f64 {
    let interval = if push_active {
        PUSHED_POLL_INTERVAL
    } else {
        IDLE_POLL_INTERVAL
    };
    interval.as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B346: herdr's stamps arrive at its poll's pace, and the machine is
    /// told so.
    #[test]
    fn herdrs_stamps_arrive_at_its_polls_pace() {
        assert_eq!(herdr_stamp_interval(false), 8.0);
        assert_eq!(herdr_stamp_interval(true), 45.0);
    }

    #[test]
    fn a_tasks_outcome_crosses_as_kinds_not_sentences() {
        let outcome = TaskOutcome::Incomplete(TaskIncomplete::ReturnFailed {
            agent: "claude".into(),
            reason: Refusal::Said("no such pane".into()),
        });
        assert_eq!(
            answer_value(HerdrAnswer::Task(outcome)),
            HerdrAnswerValue::Task {
                outcome: HerdrTaskOutcomeValue::Incomplete {
                    incomplete: HerdrTaskIncompleteValue::ReturnFailed {
                        agent: "claude".into(),
                        reason: HerdrRefusalValue::Said {
                            text: "no such pane".into()
                        }
                    }
                }
            }
        );
        assert_eq!(
            answer_value(HerdrAnswer::Removal(WorktreeRemoval::Failed(
                RemovalFailure::NoAnswer
            ))),
            HerdrAnswerValue::Removal {
                outcome: HerdrRemovalValue::Failed {
                    why: HerdrRemovalFailureValue::NoAnswer
                }
            }
        );
    }

    #[test]
    fn an_intent_is_the_clients_action() {
        assert_eq!(
            HerdrIntent::Scroll {
                pane: "p".into(),
                lines: -3,
                page_keys: true
            }
            .core(),
            Action::Scroll {
                pane: "p".into(),
                lines: -3,
                source: ScrollSource::PageKey
            }
        );
        assert_eq!(
            HerdrIntent::StartAgentTask {
                repo_path: "/r".into(),
                branch: "b".into(),
                agent: "claude".into(),
                prompt: "go".into()
            }
            .core(),
            Action::StartAgentTask(AgentTaskRequest {
                repo_path: "/r".into(),
                branch: "b".into(),
                agent: "claude".into(),
                prompt: "go".into()
            })
        );
    }
}
