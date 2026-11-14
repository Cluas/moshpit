//! What a person starts and waits on: a New agent task, a worktree's
//! removal, repository and agent discovery, a server restart (and the
//! bootstrap of a server that wasn't running). Each answers exactly once,
//! in bounded time.

use super::timers::Slot;
use super::{
    AfterPoll, Client, Event, ExecResult, Notice, NoticeAction, Owner, Purpose, RequestId,
    StatusFor,
};
use crate::control::{
    self, AgentTaskRequest, CommandResult, FirstMessageStep, RemovalFailure, TaskFailure,
    TaskIncomplete, TaskOutcome, WorktreeRemoval,
};
use crate::launch;
use crate::text;
use crate::version::VersionReport;

/// What an owned command's owner hears.
#[derive(Debug, Clone)]
pub(super) enum Answer {
    /// It ran; what it printed.
    Printed(CommandResult),
    /// It couldn't run.
    Unreachable,
    /// No answer within the owner's patience.
    Unanswered,
}

impl Answer {
    fn printed(&self) -> Option<&CommandResult> {
        match self {
            Answer::Printed(result) => Some(result),
            _ => None,
        }
    }
}

/// How a restart went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartOutcome {
    Restarted,
    Failed,
    /// Refused: the `herdr` the commands run is older than the server, so a
    /// restart with it would downgrade the server and exit every pane.
    WouldDowngrade(VersionReport),
    /// Nothing done: the server already runs the `herdr` the commands run
    /// (B408). A banner up from before the skew was fixed elsewhere offered
    /// the restart; stopping a healthy server would exit every pane.
    Unneeded,
}

#[derive(Debug)]
pub(super) struct AgentTask {
    request: AgentTaskRequest,
    branch: String,
    /// The first message, trimmed; empty sends nothing.
    prompt: String,
    stage: Stage,
    cancelled: bool,
    pane: Option<String>,
    /// The server's version once known (`Some(None)`: unreadable).
    version: Option<Option<String>>,
    settled: bool,
    deadline: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Creating,
    Launching,
    /// `pane run` only typed the agent: give it a moment.
    Settling,
    Prompting,
    /// herdr doesn't see the agent yet: try again.
    Retrying,
    /// Typed by `agent send`: Enter after a beat.
    Submitting,
    PressingReturn,
}

impl AgentTask {
    /// The outcome of a task cut short where it stands (the client closed):
    /// what the person can rely on is what had landed.
    pub fn interrupted(&self) -> TaskOutcome {
        let agent = self.request.agent.clone();
        match self.stage {
            // The create may still land.
            Stage::Creating => TaskOutcome::Failed(TaskFailure::HostStoppedAnswering {
                branch: self.branch.clone(),
            }),
            Stage::Launching => TaskOutcome::Incomplete(TaskIncomplete::LaunchUnanswered { agent }),
            Stage::Settling | Stage::Prompting | Stage::Retrying => {
                TaskOutcome::Incomplete(TaskIncomplete::HostStoppedAnswering)
            }
            Stage::Submitting | Stage::PressingReturn => {
                TaskOutcome::Incomplete(TaskIncomplete::ReturnFailed {
                    agent,
                    reason: control::Refusal::Unreachable,
                })
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Discovery {
    from_panes: Option<Vec<String>>,
    scanned: Option<Vec<String>>,
    guesses: Vec<String>,
}

impl Discovery {
    /// What is known so far, merged.
    pub fn so_far(&self) -> Vec<String> {
        control::merge_repos(
            self.from_panes.as_deref().unwrap_or(&self.guesses),
            self.scanned.as_deref().unwrap_or_default(),
        )
    }
}

#[derive(Debug)]
pub(super) struct Restart {
    /// `None` for the bootstrap of the empty state's create.
    pub request: Option<RequestId>,
    stage: RestartStage,
    /// The first workspace's label (the empty state's name).
    label: Option<String>,
    rounds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestartStage {
    CheckingVersions,
    Stopping,
    Pausing,
    Starting,
    /// Between the bootstrap's reads.
    Waiting,
    Reading,
    Creating,
    /// The read after the first workspace.
    Confirming,
}

impl Client {
    // MARK: - New agent task

    /// Make an isolated worktree and start an agent in it
    /// (`startAgentTask`): `worktree create`, then `pane run` into its root
    /// pane, then the first message — every step on the command lane, every
    /// wait bounded, every answer read for evidence.
    pub(super) fn start_task(&mut self, request: AgentTaskRequest) -> RequestId {
        let id = self.request_id();
        if let Some(invalid) = request.validation_error() {
            self.event(Event::Task {
                request: id,
                outcome: TaskOutcome::Failed(TaskFailure::Invalid(invalid)),
            });
            return id;
        }
        let branch = request.branch().to_owned();
        let prompt = text::trim_whitespaces_and_newlines(&request.prompt).to_owned();
        let command = self.herdr(&control::worktree_create(&request));
        self.tasks.insert(
            id.0,
            AgentTask {
                request,
                branch,
                prompt,
                stage: Stage::Creating,
                cancelled: false,
                pane: None,
                version: None,
                settled: false,
                deadline: None,
            },
        );
        let patience = self.config.budgets.worktree_create;
        self.enqueue_owned(command, Owner::TaskCreate(id.0), true, patience);
        id
    }

    /// The form went away: nothing more is started once the worktree's
    /// answer is in.
    pub(super) fn cancel_task(&mut self, request: RequestId) {
        if let Some(task) = self.tasks.get_mut(&request.0) {
            task.cancelled = true;
        }
    }

    fn finish_task(&mut self, id: u64, outcome: TaskOutcome) {
        if self.tasks.remove(&id).is_some() {
            self.timers.cancel(&Slot::Task(id));
            self.event(Event::Task {
                request: RequestId(id),
                outcome,
            });
        }
    }

    pub(super) fn task_created(&mut self, id: u64, answer: Answer) {
        let Some(task) = self.tasks.get(&id) else {
            return;
        };
        let branch = task.branch.clone();
        let pane = match answer {
            Answer::Unanswered => Err(TaskOutcome::Failed(TaskFailure::HostStoppedAnswering {
                branch,
            })),
            Answer::Unreachable => Err(TaskOutcome::Failed(TaskFailure::Unreachable)),
            Answer::Printed(result) => control::worktree_created(Some(&result), &branch),
        };
        let pane = match pane {
            Ok(pane) => pane,
            Err(outcome) => return self.finish_task(id, outcome),
        };
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        if task.cancelled {
            return self.finish_task(id, TaskOutcome::Incomplete(TaskIncomplete::Cancelled));
        }
        task.stage = Stage::Launching;
        task.pane = Some(pane.clone());
        let subcommand = control::pane_run(&pane, &task.request.agent);
        let command = self.herdr(&subcommand);
        let patience = self.config.budgets.agent_launch;
        self.enqueue_owned(command, Owner::TaskLaunch(id), true, patience);
    }

    pub(super) fn task_launched(&mut self, id: u64, answer: Answer) {
        let Some(task) = self.tasks.get(&id) else {
            return;
        };
        let agent = task.request.agent.clone();
        let refused = match &answer {
            Answer::Unanswered => Some(TaskOutcome::Incomplete(TaskIncomplete::LaunchUnanswered {
                agent,
            })),
            Answer::Unreachable => control::agent_launched(None, &agent),
            Answer::Printed(result) => control::agent_launched(Some(result), &agent),
        };
        if let Some(outcome) = refused {
            return self.finish_task(id, outcome);
        }
        if task.prompt.is_empty() {
            return self.finish_task(id, TaskOutcome::Started);
        }
        // Which command takes the first message depends on the server's
        // herdr: ask while the agent comes up, unless a read already said.
        let known = self.server_version.clone();
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        task.stage = Stage::Settling;
        match known {
            Some(version) => task.version = Some(Some(version)),
            None => {
                let command = self.herdr(control::STATUS);
                self.exec(
                    command,
                    Purpose::Status(StatusFor::Task(id)),
                    Some(self.config.budgets.status),
                );
            }
        }
        self.arm(Slot::Task(id), self.config.budgets.first_message_settle);
    }

    pub(super) fn task_versions(&mut self, id: u64, report: Option<VersionReport>) {
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        task.version = Some(report.map(|report| report.server));
        if task.stage == Stage::Settling && task.settled {
            self.task_prompt(id);
        }
    }

    pub(super) fn task_timer(&mut self, id: u64) {
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        match task.stage {
            Stage::Settling => {
                task.settled = true;
                if task.version.is_some() {
                    self.task_prompt(id);
                }
            }
            Stage::Retrying => self.task_prompt(id),
            Stage::Submitting => {
                task.stage = Stage::PressingReturn;
                let pane = task.pane.clone().unwrap_or_default();
                let command = self.herdr(&control::submit(&pane));
                let patience = self.config.budgets.command_hold;
                self.enqueue_owned(command, Owner::TaskSubmit(id), true, patience);
            }
            _ => {}
        }
    }

    /// One try at the first message, on the lane.
    fn task_prompt(&mut self, id: u64) {
        let now = self.now;
        let patience = self.config.budgets.first_message_patience;
        let hold = self.config.budgets.command_hold;
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        task.stage = Stage::Prompting;
        let deadline = *task.deadline.get_or_insert(now + patience);
        let version = task.version.clone().flatten();
        let pane = task.pane.clone().unwrap_or_default();
        let subcommand = control::first_message(&task.prompt, &pane, version.as_deref());
        let command = self.herdr(&subcommand);
        // Bounded, so a host that stops answering can't leave the form
        // spinning; the try that ran out may still land.
        let wait = deadline.saturating_sub(now).max(hold);
        self.enqueue_owned(command, Owner::TaskPrompt(id), true, wait);
    }

    pub(super) fn task_prompted(&mut self, id: u64, answer: Answer) {
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        let past_deadline = task.deadline.is_some_and(|deadline| self.now >= deadline);
        let version = task.version.clone().flatten();
        let step = control::after_first_message(
            answer.printed(),
            &task.request.agent,
            past_deadline,
            version.as_deref(),
        );
        match step {
            FirstMessageStep::Started => self.finish_task(id, TaskOutcome::Started),
            FirstMessageStep::Retry => {
                task.stage = Stage::Retrying;
                self.arm(Slot::Task(id), self.config.budgets.first_message_retry);
            }
            FirstMessageStep::Submit => {
                task.stage = Stage::Submitting;
                self.arm(
                    Slot::Task(id),
                    self.config.budgets.first_message_submit_delay,
                );
            }
            FirstMessageStep::Incomplete(incomplete) => {
                self.finish_task(id, TaskOutcome::Incomplete(incomplete))
            }
        }
    }

    pub(super) fn task_submitted(&mut self, id: u64, answer: Answer) {
        let Some(task) = self.tasks.get(&id) else {
            return;
        };
        let step = control::after_submit(answer.printed(), &task.request.agent);
        let outcome = match step {
            FirstMessageStep::Incomplete(incomplete) => TaskOutcome::Incomplete(incomplete),
            _ => TaskOutcome::Started,
        };
        self.finish_task(id, outcome);
    }

    // MARK: - Worktree removal

    /// Remove a task's worktree, checkout and workspace (`removeWorktree`):
    /// on the lane like any command that changes the tree, never forced
    /// unless asked, bounded (B119).
    pub(super) fn remove_worktree(&mut self, session: &str, force: bool) -> RequestId {
        let id = self.request_id();
        self.removals.insert(id.0);
        let command = self.herdr(&control::worktree_remove(session, force));
        let patience = self.config.budgets.worktree_remove;
        self.enqueue_owned(command, Owner::Removal(id.0), true, patience);
        id
    }

    pub(super) fn removal_answered(&mut self, id: u64, answer: Answer) {
        if !self.removals.remove(&id) {
            return;
        }
        let outcome = match answer {
            Answer::Printed(result) => control::worktree_removal(Some(&result)),
            Answer::Unreachable => WorktreeRemoval::Failed(RemovalFailure::Unreachable),
            Answer::Unanswered => WorktreeRemoval::Failed(RemovalFailure::NoAnswer),
        };
        self.event(Event::Removal {
            request: RequestId(id),
            outcome,
        });
    }

    // MARK: - Discovery

    /// Repositories for the New agent task form (`gitRepos`): the panes'
    /// own, resolved through git (falling back to their raw directories),
    /// then a bounded `$HOME` scan. Two commands, so a scan that hangs costs
    /// the scan, not the form.
    pub(super) fn discover_repos(&mut self) -> RequestId {
        let id = self.request_id();
        let cwds: Vec<String> = self.poll.pane_cwds.values().cloned().collect();
        let cwds: Vec<&str> = cwds.iter().map(String::as_str).collect();
        let mut discovery = Discovery {
            guesses: control::open_repo_guesses(&cwds),
            ..Discovery::default()
        };
        let resolve = control::pane_repos(&cwds);
        if resolve.is_none() {
            discovery.from_panes = Some(Vec::new());
        }
        self.discoveries.insert(id.0, discovery);
        let budgets = self.config.budgets.clone();
        if let Some(command) = resolve {
            self.exec(command, Purpose::PaneRepos(id.0), Some(budgets.discovery));
        }
        self.exec(
            control::HOME_SCAN.to_owned(),
            Purpose::HomeScan(id.0),
            Some(budgets.home_scan),
        );
        id
    }

    pub(super) fn pane_repos_answered(&mut self, id: u64, result: Option<ExecResult>) {
        let Some(discovery) = self.discoveries.get_mut(&id) else {
            return;
        };
        // A link that dies, or a resolve that doesn't answer (B120), falls
        // back to the raw directories: the server checks the path anyway.
        discovery.from_panes = Some(match result.as_ref().and_then(ExecResult::text) {
            Some(output) => control::repo_lines(&output),
            None => discovery.guesses.clone(),
        });
        self.maybe_finish_discovery(id);
    }

    pub(super) fn home_scan_answered(&mut self, id: u64, result: Option<ExecResult>) {
        let Some(discovery) = self.discoveries.get_mut(&id) else {
            return;
        };
        discovery.scanned = Some(match result.as_ref().and_then(ExecResult::text) {
            Some(output) => control::repo_lines(&output),
            None => Vec::new(),
        });
        self.maybe_finish_discovery(id);
    }

    fn maybe_finish_discovery(&mut self, id: u64) {
        let done = self
            .discoveries
            .get(&id)
            .is_some_and(|discovery| discovery.from_panes.is_some() && discovery.scanned.is_some());
        if done && let Some(discovery) = self.discoveries.remove(&id) {
            self.event(Event::Repos {
                request: RequestId(id),
                repos: discovery.so_far(),
            });
        }
    }

    /// The agents herdr detects (`agentNames`), bounded (B120).
    pub(super) fn list_agents(&mut self) -> RequestId {
        let id = self.request_id();
        self.agent_lists.insert(id.0);
        let command = self.herdr_bare(control::AGENT_MANIFESTS);
        self.exec(
            command,
            Purpose::Agents(id.0),
            Some(self.config.budgets.discovery),
        );
        id
    }

    pub(super) fn agents_answered(&mut self, id: u64, result: Option<ExecResult>) {
        if !self.agent_lists.remove(&id) {
            return;
        }
        let (names, answered) = match result.as_ref().and_then(ExecResult::text) {
            Some(output) => (control::agent_names(&output), true),
            None => (Vec::new(), false),
        };
        self.event(Event::Agents {
            request: RequestId(id),
            names,
            answered,
        });
    }

    // MARK: - Restart and bootstrap

    /// Restart the host's herdr server on the person's tap
    /// (`restartServer`): never when the `herdr` the commands run is the
    /// older side (that would downgrade the server and exit every pane).
    pub(super) fn restart_server(&mut self) -> Option<RequestId> {
        if self.restart.is_some() {
            return None;
        }
        let id = self.request_id();
        self.restart = Some(Restart {
            request: Some(id),
            stage: RestartStage::CheckingVersions,
            label: None,
            rounds: 0,
        });
        self.set_notice(Notice::Restarting, None);
        self.dirty = true;
        self.heard_since_restart = false;
        let command = self.herdr(control::STATUS);
        self.exec(
            command,
            Purpose::Status(StatusFor::Restart),
            Some(self.config.budgets.status),
        );
        Some(id)
    }

    /// The banner's button.
    pub(super) fn notice_action(&mut self) -> Option<RequestId> {
        if self.retry_stuck_channel() {
            return None;
        }
        match self.notice.as_ref().and_then(|(_, action)| *action) {
            Some(NoticeAction::RestartServer | NoticeAction::Retry) => self.restart_server(),
            Some(NoticeAction::OpenHere) => {
                self.open_here();
                None
            }
            None => None,
        }
    }

    pub(super) fn restart_versions(&mut self, report: Option<VersionReport>) {
        if !self
            .restart
            .as_ref()
            .is_some_and(|restart| restart.stage == RestartStage::CheckingVersions)
        {
            return;
        }
        // Read just now, right before `server stop`: not the versions some
        // earlier read left. An unreadable report (an old herdr without
        // `status`, or a stopped server, which names no version) doesn't
        // block: that was the behaviour before the check existed.
        if let Some(report) = report {
            self.poll.note_versions(Some(report.clone()));
            if report.client_is_older() {
                return self.finish_restart(RestartOutcome::WouldDowngrade(report));
            }
            if !report.mismatched() {
                return self.finish_restart(RestartOutcome::Unneeded);
            }
        }
        if let Some(restart) = self.restart.as_mut() {
            restart.stage = RestartStage::Stopping;
        }
        let command = self.herdr_bare(control::SERVER_STOP);
        self.exec(
            command,
            Purpose::ServerStop,
            Some(self.config.budgets.server_command),
        );
    }

    pub(super) fn server_stop_answered(&mut self) {
        if let Some(restart) = self.restart.as_mut()
            && restart.stage == RestartStage::Stopping
        {
            restart.stage = RestartStage::Pausing;
            self.arm(Slot::Restart, self.config.budgets.restart_pause);
        }
    }

    /// Start a server and make sure it has a workspace (`bootstrapServer`):
    /// a server with nothing to restore comes up empty.
    pub(super) fn bootstrap(&mut self, request: Option<RequestId>, label: Option<String>) {
        if self.restart.is_some() {
            return;
        }
        self.restart = Some(Restart {
            request,
            stage: RestartStage::Starting,
            label,
            rounds: 0,
        });
        self.dirty = true;
        self.heard_since_restart = false;
        self.start_daemon();
    }

    fn start_daemon(&mut self) {
        if let Some(restart) = self.restart.as_mut() {
            restart.stage = RestartStage::Starting;
        }
        let command = launch::daemon_command(self.config.custom_path.as_deref());
        self.exec(
            command,
            Purpose::Daemon,
            Some(self.config.budgets.server_command),
        );
    }

    pub(super) fn daemon_answered(&mut self) {
        if let Some(restart) = self.restart.as_mut()
            && restart.stage == RestartStage::Starting
        {
            restart.stage = RestartStage::Waiting;
            self.arm(Slot::Restart, self.config.budgets.bootstrap_interval);
        }
    }

    pub(super) fn restart_timer(&mut self) {
        let Some(restart) = self.restart.as_mut() else {
            return;
        };
        match restart.stage {
            RestartStage::Pausing => self.start_daemon(),
            RestartStage::Waiting => {
                restart.stage = RestartStage::Reading;
                self.after_next_poll(AfterPoll::Bootstrap);
            }
            _ => {}
        }
    }

    /// A read after the server start.
    pub(super) fn bootstrap_read(&mut self) {
        let rounds = self.config.budgets.bootstrap_rounds;
        let empty = self.poll.snapshot.sessions.is_empty();
        // A read that never answered says nothing about the new server.
        let no_server = self.poll.server_not_running || !self.heard_since_restart;
        let Some(restart) = self.restart.as_mut() else {
            return;
        };
        match restart.stage {
            RestartStage::Reading => {
                if no_server {
                    restart.rounds += 1;
                    if restart.rounds >= rounds {
                        return self.finish_bootstrap();
                    }
                    restart.stage = RestartStage::Waiting;
                    self.arm(Slot::Restart, self.config.budgets.bootstrap_interval);
                } else if empty {
                    restart.stage = RestartStage::Creating;
                    let label = restart.label.clone();
                    let command = self.herdr(&control::new_session(label.as_deref()));
                    let patience = self.config.budgets.bootstrap_create;
                    self.enqueue_owned(command, Owner::Bootstrap, true, patience);
                } else {
                    self.finish_bootstrap();
                }
            }
            RestartStage::Confirming => self.finish_bootstrap(),
            _ => {}
        }
    }

    /// The first workspace answered (or not): read once the lane let go of
    /// it, so the outcome is judged on the server's answer.
    pub(super) fn bootstrap_created(&mut self) {
        if let Some(restart) = self.restart.as_mut()
            && restart.stage == RestartStage::Creating
        {
            restart.stage = RestartStage::Confirming;
            self.after_next_poll(AfterPoll::Bootstrap);
        }
    }

    fn finish_bootstrap(&mut self) {
        let outcome = if self.heard_since_restart
            && !self.poll.server_not_running
            && !self.poll.last_read_mismatched()
        {
            RestartOutcome::Restarted
        } else {
            RestartOutcome::Failed
        };
        self.finish_restart(outcome);
    }

    fn finish_restart(&mut self, outcome: RestartOutcome) {
        let Some(restart) = self.restart.take() else {
            return;
        };
        self.timers.cancel(&Slot::Restart);
        self.dirty = true;
        let Some(request) = restart.request else {
            // The empty state's create: the tree says how it went.
            return;
        };
        match &outcome {
            RestartOutcome::Restarted => {
                // Stopping the server ended every client's attach, another
                // device's too: no pane is anyone else's now (B418).
                self.attach.held_elsewhere = None;
                self.clear_notice();
                self.attach.retry_after = None;
                self.quicken();
            }
            RestartOutcome::Unneeded => {
                // Nothing restarted: a pane another device holds is still
                // theirs, said again with its take-back (B418). The skew's
                // banner had covered it, and clearing that left the screen
                // quiet, typing said not sent with no button.
                if self.attach.held_elsewhere.is_some() {
                    self.set_notice(Notice::OnAnotherDevice, Some(NoticeAction::OpenHere));
                } else {
                    self.clear_notice();
                }
                self.attach.retry_after = None;
                self.quicken();
            }
            RestartOutcome::WouldDowngrade(report) => {
                let report = report.clone();
                self.show_skew(&report);
            }
            RestartOutcome::Failed => {
                self.set_notice(Notice::RestartFailed, Some(NoticeAction::Retry));
            }
        }
        self.event(Event::Restart { request, outcome });
    }
}
