//! herdr's control plane, the pure half of
//! `Moshpit/Services/Herdr/HerdrControlClient.swift`: every command line it
//! runs, how it reads what comes back, and the decisions it makes from that.
//!
//! What stays with the transport (a later work package): the SSH exec
//! channel, the poll timer, the command lane that runs one command at a time
//! in the order issued (with its budgets), the epoch that drops a read taken
//! while a command was in flight, and the sleeps of the first-message retry.
//! Each of those is a loop around a function here.

use std::time::Duration;

use crate::json::{self, Object};
use crate::launch::{self, quote};
use crate::snapshot::{self, Decoded, Snapshot};
use crate::text;
use crate::version::{self, VersionReport};

// MARK: Running a command

/// Marker carrying the remote exit status back: the SSH layer merges stderr
/// into stdout and drops the status, and herdr's failure output is not
/// stable enough to match on (0.7.3 prints a bare `Error: Os { code: 2, … }`
/// with no server, 0.8.0 a structured `server_not_running`); the exit code
/// is 1 in both.
pub const EXIT_CODE_MARKER: &str = "__moshpit_rc=";

/// One `herdr` invocation and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub output: String,
    /// The exit status, or `None` if the trailer went missing.
    pub exit_code: Option<i64>,
}

impl CommandResult {
    /// Exited nonzero. A missing status is not a failure here: blanking the
    /// tree on a truncated read would be worse than keeping it (but see
    /// [`refusal`]).
    pub fn failed(&self) -> bool {
        self.exit_code.unwrap_or(0) != 0
    }
}

/// The command line `run(_:)` sends for a herdr subcommand: the binary (with
/// the PATH treatment), the subcommand, the exit-status trailer.
pub fn command_line(subcommand: &str, custom_path: Option<&str>) -> String {
    format!(
        "{}; echo \"{EXIT_CODE_MARKER}$?\"",
        launch::command(subcommand, custom_path)
    )
}

/// Split the exit-status trailer (`__moshpit_rc=N`, echoed after herdr
/// exits) off a command's output.
///
/// Fix (trailer-at-end): the trailer counts only as the last thing printed:
/// the marker, its ASCII digits, then nothing but whitespace. Anything else
/// is no trailer: the output is kept whole and there is no status. Output
/// cut off before the real trailer but carrying the marker inside herdr's
/// own text (a pane title, a working directory, a label: anything a
/// program on the host sets) read as that command's status — a truncated
/// answer taken for a finished one, the B90 class this crate refuses
/// everywhere else; and `007z` read as 7, an empty `__moshpit_rc=` as no
/// status with the marker cut off. The app's rule too since round 36
/// (B193).
pub fn parse_result(raw: &str) -> CommandResult {
    let unread = || CommandResult {
        output: raw.to_owned(),
        exit_code: None,
    };
    let Some(at) = raw.rfind(EXIT_CODE_MARKER) else {
        return unread();
    };
    let tail = &raw[at + EXIT_CODE_MARKER.len()..];
    let digits = text::ascii_digits(tail);
    if digits.is_empty() || !text::trim_whitespaces_and_newlines(&tail[digits.len()..]).is_empty() {
        return unread();
    }
    let Some(code) = text::swift_int(digits) else {
        return unread();
    };
    CommandResult {
        output: raw[..at].to_owned(),
        exit_code: Some(code),
    }
}

/// herdr's structured error text, if the payload carries one: its message,
/// else its code.
pub fn error_message(object: &Object) -> Option<String> {
    let error = object.object("error")?;
    error
        .str("message")
        .or_else(|| error.str("code"))
        .map(str::to_owned)
}

/// Why a command didn't do its job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// It never ran: the channel couldn't run it.
    Unreachable,
    /// herdr's own words: its error message, or the first line it printed
    /// (a usage error is plain text), cut at 200 characters.
    Said(String),
    /// It failed and printed nothing.
    ExitStatus(i64),
    /// No exit status came back and herdr's own result wasn't printed
    /// either: the answer was cut off (the channel dropped before the
    /// trailer), so nothing says the command ran (B90).
    CutOff,
}

impl Refusal {
    /// The app's English text (its string catalog's source).
    pub fn english(&self) -> String {
        match self {
            Refusal::Unreachable => "Couldn't reach the host".to_owned(),
            Refusal::Said(text) => text.clone(),
            Refusal::ExitStatus(code) => format!("herdr exited with status {code}"),
            Refusal::CutOff => "herdr's answer was cut off".to_owned(),
        }
    }
}

/// Longest printed line a refusal quotes.
const REFUSAL_CHARS: usize = 200;

/// Why a command didn't do its job, or `None` when it did: herdr's own
/// error text when it gave one, else the first thing it printed, else that
/// it couldn't run at all.
///
/// Done takes evidence: exit status 0, or herdr's own `result` printed. A
/// result whose exit status never came back (the trailer lost, the channel
/// dropped before it) and that carries no `result` is [`Refusal::CutOff`]:
/// the app's rule since round eighteen (B90, `a22454d7`), which this port
/// had answered differently before (any JSON object counted, and a printed
/// line was quoted as herdr's reason).
///
/// The printed line is cut at 200 Unicode scalars; Swift cuts at 200
/// grapheme clusters (divergence `refusal-cut`, display only).
pub fn refusal(result: Option<&CommandResult>) -> Option<Refusal> {
    let Some(result) = result else {
        return Some(Refusal::Unreachable);
    };
    let object = json::first_object(&result.output);
    if let Some(message) = object.as_ref().and_then(error_message) {
        return Some(Refusal::Said(message));
    }
    let Some(code) = result.exit_code else {
        return if object
            .as_ref()
            .is_some_and(|object| object.get("result").is_some())
        {
            None
        } else {
            Some(Refusal::CutOff)
        };
    };
    if code == 0 {
        return None;
    }
    let printed = text::lines(&result.output)
        .map(text::trim_whitespaces)
        .find(|line| !line.is_empty());
    Some(match printed {
        Some(line) => Refusal::Said(line.chars().take(REFUSAL_CHARS).collect()),
        None => Refusal::ExitStatus(code),
    })
}

/// herdr turned a message away because it doesn't see the agent in that
/// pane yet: `agent_not_found` until detection names it, `agent_not_ready`
/// while it starts. Anything else won't change by asking again.
pub fn agent_not_up_yet(result: &CommandResult) -> bool {
    let Some(object) = json::first_object(&result.output) else {
        return false;
    };
    matches!(
        object.object("error").and_then(|error| error.str("code")),
        Some("agent_not_found" | "agent_not_ready")
    )
}

// MARK: Commands

/// Subcommands the client runs, each through [`command_line`].
pub const SNAPSHOT: &str = "api snapshot";
/// `herdr status`: which `herdr` the commands run, which server is up.
pub const STATUS: &str = "status";

pub fn select_session(session_id: &str) -> String {
    format!("workspace focus {}", quote(session_id))
}

pub fn select_window(window_id: &str) -> String {
    format!("tab focus {}", quote(window_id))
}

/// Focus one pane. `agent focus` is the only CLI command that takes a pane
/// id (`pane focus` is directional only), and current herdr fails it on a
/// plain shell pane without moving the focus; so the same exec carries a
/// second rung, a zoom bounce: `pane zoom --on` moves the focus, `--off`
/// restores the layout and keeps it. `--on` before `--off`, always together.
pub fn select_pane(pane_id: &str, custom_path: Option<&str>) -> String {
    let target = quote(pane_id);
    let herdr = launch::attach_command(custom_path);
    format!(
        "agent focus {target} 2>/dev/null || {{ \
         {herdr} pane zoom --pane {target} --on >/dev/null 2>&1; \
         {herdr} pane zoom --pane {target} --off >/dev/null 2>&1; }}"
    )
}

/// mosh + herdr's immersive zoom: focus and fill the shared screen in one
/// idempotent command. A pane gone mid-flight is a stale tap, not a broken
/// channel, hence stderr dropped and the status forced.
pub fn immersive_zoom(pane_id: &str) -> String {
    format!(
        "pane zoom --pane {} --on 2>/dev/null || true",
        quote(pane_id)
    )
}

fn with_label(command: &str, name: Option<&str>) -> String {
    match name.filter(|name| !name.is_empty()) {
        Some(name) => format!("{command} --label {}", quote(name)),
        None => command.to_owned(),
    }
}

/// A new workspace. On a host with no server the client starts one instead
/// ([`launch::daemon_command`]), which brings its own first workspace only
/// when it restores a saved session; otherwise this follows it.
pub fn new_session(name: Option<&str>) -> String {
    with_label("workspace create --focus", name)
}

pub fn new_window(name: Option<&str>) -> String {
    with_label("tab create --focus", name)
}

fn split(pane_id: &str) -> String {
    format!("pane split {} --direction right --focus", quote(pane_id))
}

/// Split the focused pane: herdr needs an explicit target (`--current` is
/// the pane the CLI runs in, none of them). `None` with no pane to split.
pub fn new_pane(snapshot: &Snapshot) -> Option<String> {
    let target = snapshot
        .active_pane_id
        .as_deref()
        .or_else(|| snapshot.active_panes().first().map(|pane| pane.id.as_str()))?;
    Some(split(target))
}

/// Home's "New tab" on a session row: focus that workspace if it isn't the
/// focused one, then create. One lane step; a failed focus stops the create.
pub fn new_window_in_session(
    snapshot: &Snapshot,
    session_id: &str,
    name: Option<&str>,
) -> Vec<String> {
    let mut steps = Vec::new();
    if snapshot.active_session_id.as_deref() != Some(session_id) {
        steps.push(select_session(session_id));
    }
    steps.push(new_window(name));
    steps
}

/// Home's "New pane" on a window row: the window's own active pane is split
/// (or its first), after focusing its workspace and tab as needed. `None`
/// when the window or its panes are unknown.
pub fn new_pane_in_window(snapshot: &Snapshot, window_id: &str) -> Option<Vec<String>> {
    let window = snapshot.windows.get(window_id)?;
    let panes = snapshot.panes_in_window(window_id);
    let target = panes
        .iter()
        .find(|pane| pane.is_active)
        .or_else(|| panes.first())?;
    let mut steps = Vec::new();
    if snapshot.active_session_id.as_deref() != Some(window.session_id.as_str()) {
        steps.push(select_session(&window.session_id));
    }
    if snapshot.active_window_id.as_deref() != Some(window_id) {
        steps.push(select_window(window_id));
    }
    steps.push(split(&target.id));
    Some(steps)
}

pub fn rename_session(session_id: &str, name: &str) -> String {
    format!("workspace rename {} {}", quote(session_id), quote(name))
}

pub fn kill_session(session_id: &str) -> String {
    format!("workspace close {}", quote(session_id))
}

pub fn rename_window(window_id: &str, name: &str) -> String {
    format!("tab rename {} {}", quote(window_id), quote(name))
}

pub fn kill_window(window_id: &str) -> String {
    format!("tab close {}", quote(window_id))
}

pub fn kill_pane(pane_id: &str) -> String {
    format!("pane close {}", quote(pane_id))
}

/// What a pane's screen shows, with its colours (`pane read --format
/// ansi`, SGR in the text): the overview's picture of a pane the phone
/// doesn't show (B751), stripped there, and a swipe's peek, drawn as the
/// terminal draws. Takes no pane, moves no focus.
pub fn pane_read(pane_id: &str) -> String {
    format!("pane read {} --format ansi", quote(pane_id))
}

/// Semantic keys for `pane id`, encoded by herdr against the pane's real
/// terminal modes (the local emulator never learns DECCKM over herdr, so
/// `ESC[A` vs `ESC OA` can't be chosen here). One exec for a run of keys;
/// `None` for none.
pub fn send_keys(keys: &[&str], pane_id: &str) -> Option<String> {
    if keys.is_empty() {
        return None;
    }
    let names: Vec<String> = keys.iter().map(|key| quote(key)).collect();
    Some(format!(
        "pane send-keys {} {}",
        quote(pane_id),
        names.join(" ")
    ))
}

// MARK: Agent tasks

/// What the New agent task form collects (`AgentTaskRequest`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentTaskRequest {
    /// Absolute path to the repository root on the host.
    pub repo_path: String,
    /// The worktree's branch, also its workspace label.
    pub branch: String,
    /// The agent command to run in the new pane, e.g. `claude`.
    pub agent: String,
    /// Sent to the agent once it is up; empty sends nothing.
    pub prompt: String,
}

/// Why a request can't be submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskInvalid {
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

impl TaskInvalid {
    pub fn english(self) -> &'static str {
        match self {
            TaskInvalid::PickRepository => "Pick a repository",
            TaskInvalid::PickAgent => "Pick an agent",
            TaskInvalid::NameBranch => "Name the branch",
            TaskInvalid::BranchSpace => "No spaces in a branch name",
            TaskInvalid::BranchEdge => "Can't start with “-” or “/”, or end with “/”",
            TaskInvalid::BranchDotDot => "No “..” in a branch name",
            TaskInvalid::BranchLock => "Can't end with “.lock”",
            TaskInvalid::BranchForbidden => "No ~ ^ : ? * [ \\ in a branch name",
            TaskInvalid::BranchControl => "No control characters in a branch name",
        }
    }
}

impl AgentTaskRequest {
    /// Why this can't be submitted, or `None` when it can.
    pub fn validation_error(&self) -> Option<TaskInvalid> {
        if text::trim_whitespaces(&self.repo_path).is_empty() {
            return Some(TaskInvalid::PickRepository);
        }
        if text::trim_whitespaces(&self.agent).is_empty() {
            return Some(TaskInvalid::PickAgent);
        }
        branch_error(&self.branch)
    }

    /// The branch as the commands use it.
    pub fn branch(&self) -> &str {
        text::trim_whitespaces(&self.branch)
    }
}

/// The part of `git check-ref-format` worth enforcing on a phone keyboard,
/// so a typo is caught before a round trip.
pub fn branch_error(raw: &str) -> Option<TaskInvalid> {
    let branch = text::trim_whitespaces(raw);
    if branch.is_empty() {
        return Some(TaskInvalid::NameBranch);
    }
    if branch.contains(' ') {
        return Some(TaskInvalid::BranchSpace);
    }
    if branch.starts_with('-') || branch.starts_with('/') || branch.ends_with('/') {
        return Some(TaskInvalid::BranchEdge);
    }
    if branch.contains("..") {
        return Some(TaskInvalid::BranchDotDot);
    }
    if branch.ends_with(".lock") {
        return Some(TaskInvalid::BranchLock);
    }
    if branch.contains(['~', '^', ':', '?', '*', '[', '\\', '\u{7f}']) {
        return Some(TaskInvalid::BranchForbidden);
    }
    if branch.chars().any(|scalar| u32::from(scalar) < 0x20) {
        return Some(TaskInvalid::BranchControl);
    }
    None
}

/// How a New agent task went: the form acts on the difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    /// The worktree, its agent, and the first message if there was one.
    Started,
    /// Nothing was made; the form stays open for another try.
    Failed(TaskFailure),
    /// The worktree exists but what came after it didn't land; starting
    /// again would only collide with its branch.
    Incomplete(TaskIncomplete),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskFailure {
    Invalid(TaskInvalid),
    Unreachable,
    CreateFailed,
    /// No answer to read, yet no failure either: no pane was named, so no
    /// agent started. (B72.)
    Unconfirmed {
        branch: String,
    },
    /// `worktree create` got no answer within its budget (B89): it may
    /// still land, so the message names the workspace to look for.
    HostStoppedAnswering {
        branch: String,
    },
    /// herdr's own error text.
    Herdr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskIncomplete {
    NoPane,
    /// The form was cancelled while the worktree was being made: it is
    /// there, and nothing more is done in it.
    Cancelled,
    /// `pane run` got no answer within its budget (B89).
    LaunchUnanswered {
        agent: String,
    },
    AgentDidNotStart {
        agent: String,
        reason: Refusal,
    },
    HostStoppedAnswering,
    MessageRefused {
        agent: String,
        reason: Refusal,
    },
    AgentNotReady {
        agent: String,
    },
    ReturnFailed {
        agent: String,
        reason: Refusal,
    },
}

impl TaskOutcome {
    /// The app's English text, `None` for [`TaskOutcome::Started`].
    pub fn english(&self) -> Option<String> {
        Some(match self {
            TaskOutcome::Started => return None,
            TaskOutcome::Failed(failure) => match failure {
                TaskFailure::Invalid(invalid) => invalid.english().to_owned(),
                TaskFailure::Unreachable => "Couldn't reach the host".to_owned(),
                TaskFailure::CreateFailed => "Creating the worktree failed".to_owned(),
                TaskFailure::Unconfirmed { branch } => format!(
                    "herdr didn't confirm the worktree, so no agent was started. Look for a “{branch}” workspace before trying again."
                ),
                TaskFailure::HostStoppedAnswering { branch } => format!(
                    "The host stopped answering while the worktree was being made, so no agent was started. Look for a “{branch}” workspace before trying again."
                ),
                TaskFailure::Herdr(message) => message.clone(),
            },
            TaskOutcome::Incomplete(incomplete) => match incomplete {
                TaskIncomplete::NoPane => "The worktree was created but has no pane".to_owned(),
                TaskIncomplete::Cancelled => {
                    "Cancelled. The worktree is ready, but no agent was started in it.".to_owned()
                }
                TaskIncomplete::LaunchUnanswered { agent } => format!(
                    "The worktree is ready, but the host stopped answering before {agent} started."
                ),
                TaskIncomplete::AgentDidNotStart { agent, reason } => format!(
                    "The worktree is ready, but {agent} didn't start: {}",
                    reason.english()
                ),
                TaskIncomplete::HostStoppedAnswering => {
                    "The task is running, but the host stopped answering before the first message was sent.".to_owned()
                }
                TaskIncomplete::MessageRefused { agent, reason } => format!(
                    "The task is running, but {agent} didn't get the first message: {}",
                    reason.english()
                ),
                TaskIncomplete::AgentNotReady { agent } => format!(
                    "The task is running, but {agent} wasn't ready for the first message in time."
                ),
                TaskIncomplete::ReturnFailed { agent, reason } => format!(
                    "The task is running, but the first message is still in {agent}'s input: pressing Return failed ({}). Press Return in the pane to send it.",
                    reason.english()
                ),
            },
        })
    }
}

/// Step one of a task: a worktree whose workspace's root pane already sits
/// in the new checkout. `--focus` moves the server's focus like a tap, so
/// it takes its turn on the command lane.
pub fn worktree_create(request: &AgentTaskRequest) -> String {
    let branch = quote(request.branch());
    format!(
        "worktree create --cwd {} --branch {branch} --label {branch} --focus --json",
        quote(&request.repo_path)
    )
}

/// What `worktree create` answered: the root pane the agent goes into, or
/// the outcome that ends the task here. `None` for "no answer at all".
pub fn worktree_created(
    result: Option<&CommandResult>,
    branch: &str,
) -> Result<String, TaskOutcome> {
    let Some(result) = result else {
        return Err(TaskOutcome::Failed(TaskFailure::Unreachable));
    };
    let Some(object) = json::first_object(&result.output) else {
        return Err(TaskOutcome::Failed(if result.failed() {
            TaskFailure::CreateFailed
        } else {
            TaskFailure::Unconfirmed {
                branch: branch.to_owned(),
            }
        }));
    };
    if let Some(message) = error_message(&object) {
        return Err(TaskOutcome::Failed(TaskFailure::Herdr(message)));
    }
    object
        .object("result")
        .and_then(|payload| payload.object("root_pane"))
        .and_then(|pane| pane.str("pane_id"))
        .map(str::to_owned)
        .ok_or(TaskOutcome::Incomplete(TaskIncomplete::NoPane))
}

/// Step two: type the agent into its pane and press return, as the user
/// would (`agent start` would land in the CLI's own directory instead).
pub fn pane_run(pane_id: &str, agent: &str) -> String {
    format!("pane run {} {}", quote(pane_id), quote(agent))
}

/// What `pane run` answered: `None` when the agent is starting.
pub fn agent_launched(result: Option<&CommandResult>, agent: &str) -> Option<TaskOutcome> {
    refusal(result).map(|reason| {
        TaskOutcome::Incomplete(TaskIncomplete::AgentDidNotStart {
            agent: agent.to_owned(),
            reason,
        })
    })
}

/// The first herdr whose CLI has `agent prompt`; the same release took
/// `agent send` away and moved the protocol from 16 to 17, so a client and
/// server that accept each other are on the same side of it.
pub const AGENT_PROMPT_VERSION: &str = "0.7.5";

/// Whether a server running `server_version` takes the first message as
/// `agent send` (typed, no Enter). `None` (unreadable) means today's herdr.
pub fn types_without_submitting(server_version: Option<&str>) -> bool {
    server_version.is_some_and(|version| {
        version::compare(version, AGENT_PROMPT_VERSION) == std::cmp::Ordering::Less
    })
}

/// The command that hands `text` to the agent in `pane_id`: `agent prompt`
/// from 0.7.5 (types, waits a beat, presses Enter), `agent send` before
/// (types only). Sending `agent send` to a 0.7.5+ herdr is a usage error
/// that never reaches the server (B58).
pub fn first_message(text: &str, pane_id: &str, server_version: Option<&str>) -> String {
    let verb = if types_without_submitting(server_version) {
        "agent send"
    } else {
        "agent prompt"
    };
    format!("{verb} {} {}", quote(pane_id), quote(text))
}

/// The Enter under a first message `agent send` typed (B74): `pane
/// send-keys`, which herdr encodes against the pane's own modes.
pub fn submit(pane_id: &str) -> String {
    format!("pane send-keys {} {}", quote(pane_id), quote("Enter"))
}

/// What to do after one try at the first message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstMessageStep {
    /// Delivered: the task has started.
    Started,
    /// herdr doesn't see the agent yet; try again after
    /// [`FIRST_MESSAGE_RETRY_INTERVAL`].
    Retry,
    /// Typed by `agent send`: press Enter ([`submit`]) after
    /// [`FIRST_MESSAGE_SUBMIT_DELAY`].
    Submit,
    Incomplete(TaskIncomplete),
}

/// One try's answer (`None`: no answer within the budget). `past_deadline`:
/// the patience ([`FIRST_MESSAGE_PATIENCE`]) has run out.
pub fn after_first_message(
    answer: Option<&CommandResult>,
    agent: &str,
    past_deadline: bool,
    server_version: Option<&str>,
) -> FirstMessageStep {
    let Some(answer) = answer else {
        return FirstMessageStep::Incomplete(TaskIncomplete::HostStoppedAnswering);
    };
    if let Some(reason) = refusal(Some(answer)) {
        if !agent_not_up_yet(answer) {
            return FirstMessageStep::Incomplete(TaskIncomplete::MessageRefused {
                agent: agent.to_owned(),
                reason,
            });
        }
        if past_deadline {
            return FirstMessageStep::Incomplete(TaskIncomplete::AgentNotReady {
                agent: agent.to_owned(),
            });
        }
        return FirstMessageStep::Retry;
    }
    if types_without_submitting(server_version) {
        FirstMessageStep::Submit
    } else {
        FirstMessageStep::Started
    }
}

/// The Enter's answer (`None`: none within the budget).
pub fn after_submit(pressed: Option<&CommandResult>, agent: &str) -> FirstMessageStep {
    match refusal(pressed) {
        Some(reason) => FirstMessageStep::Incomplete(TaskIncomplete::ReturnFailed {
            agent: agent.to_owned(),
            reason,
        }),
        None => FirstMessageStep::Started,
    }
}

/// The pause before the first try: `pane run` has only typed the command.
pub const FIRST_MESSAGE_SETTLE: Duration = Duration::from_secs(2);
/// How long to keep trying while herdr can't see the agent: what herdr's
/// own `agent start` gives an agent to be ready.
pub const FIRST_MESSAGE_PATIENCE: Duration = Duration::from_secs(30);
pub const FIRST_MESSAGE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// Between `agent send` and the Enter: the beat `agent prompt` waits, so the
/// text isn't swallowed with the Enter as one paste.
pub const FIRST_MESSAGE_SUBMIT_DELAY: Duration = Duration::from_millis(300);

/// How long one command may hold up the commands queued behind it.
pub const COMMAND_BUDGET: Duration = Duration::from_secs(5);
/// How long one semantic key may hold up the input queued behind it.
pub const KEY_SEND_BUDGET: Duration = Duration::from_secs(3);

// MARK: Worktree removal

/// What happened when a task's worktree was to be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeRemoval {
    Removed,
    /// Modified or untracked files: deleting destroys them, so this comes
    /// back for a second, much sharper confirmation.
    NeedsForce,
    Failed(RemovalFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalFailure {
    Unreachable,
    Herdr(String),
    Failed,
    /// Fix (B119): no exit status and no result from herdr, so nothing
    /// says the checkout went.
    CutOff,
    /// Fix (B119): no answer within the budget (a half-open connection);
    /// the removal may still land.
    NoAnswer,
}

impl RemovalFailure {
    pub fn english(&self) -> String {
        match self {
            RemovalFailure::Unreachable => "Couldn't reach the host".to_owned(),
            RemovalFailure::Herdr(message) => message.clone(),
            RemovalFailure::Failed => "Removing the worktree failed".to_owned(),
            RemovalFailure::CutOff => "herdr's answer was cut off".to_owned(),
            RemovalFailure::NoAnswer => {
                "The host stopped answering while the worktree was being removed.".to_owned()
            }
        }
    }
}

/// Never forced on the first try: herdr refuses a dirty checkout with
/// `dirty_worktree_requires_force` and leaves every file in place.
pub fn worktree_remove(session_id: &str, force: bool) -> String {
    let mut command = format!("worktree remove --workspace {}", quote(session_id));
    if force {
        command.push_str(" --force");
    }
    command
}

/// What `worktree remove` answered.
///
/// Fix (B119): removed takes evidence, as every other answer does here
/// ([`refusal`]): herdr's `result`, or exit status 0. Swift reported a
/// removal whose trailer was lost (and that printed no JSON) as removed.
pub fn worktree_removal(result: Option<&CommandResult>) -> WorktreeRemoval {
    let Some(result) = result else {
        return WorktreeRemoval::Failed(RemovalFailure::Unreachable);
    };
    let object = json::first_object(&result.output);
    if let Some(error) = object.as_ref().and_then(|object| object.object("error")) {
        if error.str("code") == Some("dirty_worktree_requires_force") {
            return WorktreeRemoval::NeedsForce;
        }
        return WorktreeRemoval::Failed(match error.str("message") {
            Some(message) => RemovalFailure::Herdr(message.to_owned()),
            None => RemovalFailure::Failed,
        });
    }
    if object
        .as_ref()
        .is_some_and(|object| object.get("result").is_some())
    {
        return WorktreeRemoval::Removed;
    }
    match result.exit_code {
        Some(0) => WorktreeRemoval::Removed,
        Some(_) => WorktreeRemoval::Failed(RemovalFailure::Failed),
        None => WorktreeRemoval::Failed(RemovalFailure::CutOff),
    }
}

// MARK: Repositories and agents

/// Resolve the panes' working directories to their repo roots: one small
/// round trip that survives bad links. `None` with no directory to ask
/// about. The directories are deduplicated and sorted.
pub fn pane_repos(cwds: &[&str]) -> Option<String> {
    let mut dirs: Vec<&str> = cwds.to_vec();
    dirs.sort_unstable();
    dirs.dedup();
    if dirs.is_empty() {
        return None;
    }
    let quoted: Vec<String> = dirs.iter().map(|dir| quote(dir)).collect();
    Some(format!(
        "for d in {}; do git -C \"$d\" rev-parse --show-toplevel 2>/dev/null; done",
        quoted.join(" ")
    ))
}

/// The panes' raw directories, sorted, each once: what the form offers
/// before (or instead of) the resolved repos.
pub fn open_repo_guesses(cwds: &[&str]) -> Vec<String> {
    let mut dirs: Vec<&str> = cwds.to_vec();
    dirs.sort_unstable();
    dirs.dedup();
    dirs.into_iter().map(str::to_owned).collect()
}

/// The `$HOME` walk for repositories, most recently touched first: shallow,
/// pruning the expensive and useless places (`Library`, `node_modules`, the
/// media libraries that can block on iCloud materialisation, every hidden
/// directory but `.git`), following symlinks. Run under a client-side
/// budget; missing it costs the discovery list, not the form.
pub const HOME_SCAN: &str = r#"find -L "$HOME" -maxdepth 4 \( -type d \( -name Library -o -name node_modules -o -name .Trash -o -name Applications -o -name Music -o -name Movies -o -name Pictures -o -name .build -o -name vendor \) -prune \) -o \( -type d -name '.?*' ! -name .git -prune \) -o \( -type d -name .git -print \) 2>/dev/null | while read -r g; do d="${g%/.git}"; printf '%s\t%s\n' "$(stat -f %m "$d" 2>/dev/null || stat -c %Y "$d" 2>/dev/null)" "$d"; done | sort -rn | cut -f2 | head -40"#;

/// Absolute paths, first occurrence wins; everything else in the merged
/// stdout and stderr (login noise, git's complaints) drops.
pub fn repo_lines(output: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    text::lines(output)
        .map(text::trim_whitespaces)
        .filter(|line| line.starts_with('/') && seen.insert(*line))
        .map(str::to_owned)
        .collect()
}

/// The panes' repos first ("resume what I was doing"), then the scan's,
/// each once.
pub fn merge_repos(from_panes: &[String], scanned: &[String]) -> Vec<String> {
    let mut seen: std::collections::BTreeSet<&str> =
        from_panes.iter().map(String::as_str).collect();
    let mut repos = from_panes.to_vec();
    repos.extend(
        scanned
            .iter()
            .filter(|repo| seen.insert(repo.as_str()))
            .cloned(),
    );
    repos
}

/// `herdr server agent-manifests --json`: run through
/// [`launch::command`], without the exit-status trailer.
pub const AGENT_MANIFESTS: &str = "server agent-manifests --json";

/// The agents herdr knows how to detect (`claude`, `codex`, …), sorted.
///
/// herdr 0.9 wraps the list in its reply envelope (`{"id",
/// "result":{"manifests":[…]}}`, B118); older ones printed the bare array.
/// The envelope is read when there is one; otherwise the list is taken from
/// the first `[` to the last `]` (what the app does for both: the slice
/// happens to work on the envelope as long as nothing after the list holds
/// a `]`).
///
/// Fix (manifest-range-trap): when the last `]` comes before the first `[`
/// (`"a]b["`), Swift's `raw[start...end]` traps and the app crashes; here
/// that is no list.
pub fn agent_names(raw: &str) -> Vec<String> {
    if let Some(manifests) = json::first_object(raw)
        .as_ref()
        .and_then(|reply| reply.object("result"))
        .and_then(|result| result.objects("manifests"))
    {
        let mut names: Vec<String> = manifests
            .iter()
            .filter_map(|item| item.str("agent").map(str::to_owned))
            .collect();
        names.sort();
        return names;
    }
    agent_names_in_brackets(raw)
}

/// The bracket slice of [`agent_names`], as the app reads every reply.
pub fn agent_names_in_brackets(raw: &str) -> Vec<String> {
    let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) else {
        return Vec::new();
    };
    if end < start {
        return Vec::new();
    }
    let Some(list) = json::parse(&raw.as_bytes()[start..=end]).and_then(|value| {
        value
            .as_objects()
            .map(|items| items.into_iter().cloned().collect::<Vec<Object>>())
    }) else {
        return Vec::new();
    };
    let mut names: Vec<String> = list
        .iter()
        .filter_map(|item| item.str("agent").map(str::to_owned))
        .collect();
    names.sort();
    names
}

// MARK: Reading the tree

/// What one `api snapshot` read says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollReading {
    Tree(Box<Decoded>),
    /// The server is running but refuses this client's protocol: version
    /// skew, whose remedy (restart or upgrade on the host) is the user's.
    Mismatch(String),
    /// No server (the exit status says so, whatever the wording): an empty
    /// state, not a failure.
    NoServer,
    /// Shell noise, a truncated read: keep the last good tree.
    NoNews,
}

/// Classify one read: a tree, a refusal over skew, no server, or nothing
/// new — in that order.
///
/// No server is what says so: herdr's structured `server_not_running`,
/// 0.7.3's bare socket error, or no herdr at all (127). Any other failed
/// read — a request that timed out, a CLI that crashed — is nothing new
/// (B445): taken for no server, it emptied the tree and dropped the
/// terminal out of its slot for the beat until the next good read.
pub fn read_poll(result: &CommandResult) -> PollReading {
    if let Some(decoded) = snapshot::decode(&result.output) {
        return PollReading::Tree(Box::new(decoded));
    }
    if let Some(mismatch) = snapshot::protocol_mismatch(&result.output) {
        return PollReading::Mismatch(mismatch);
    }
    if snapshot::is_server_not_running(&result.output)
        || (result.failed()
            && (result.exit_code == Some(127) || snapshot::is_no_socket(&result.output)))
    {
        return PollReading::NoServer;
    }
    PollReading::NoNews
}

/// Cadence while anything is happening: each poll is a whole SSH exec.
pub const FAST_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Once the tree has been identical for a while: the most staleness that
/// felt honest for a "needs you" prompt.
pub const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(8);
/// Once the push channel carries events: the poll is only a safety net.
pub const PUSHED_POLL_INTERVAL: Duration = Duration::from_secs(45);
/// Identical polls before easing off.
pub const IDLE_POLL_THRESHOLD: u32 = 3;

/// What the client must do after a read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollEffect {
    /// Ask `herdr status` which side is behind (once per mismatch episode),
    /// before the mismatch is reported, then [`PollState::note_versions`].
    DiagnoseSkew,
    /// The server refuses this client: report it (the terminal banner).
    Mismatch(String),
    /// The server's focused pane, after EVERY tree, not only on change: the
    /// frame channel can lose its attach (another client's `--takeover`)
    /// with the focus unmoved, and this repeat is how it recovers.
    FocusedPane(String),
    /// Agent stamps were rebuilt.
    HooksUpdated,
    /// The mosh renderer's immersive zoom follows a switch: zoom this pane
    /// ([`immersive_zoom`]).
    Zoom(String),
}

/// The control client's state as reads change it: the tree, the agent
/// stamps, what the empty state and the banners show, and the cadence.
#[derive(Debug, Clone)]
pub struct PollState {
    pub snapshot: Snapshot,
    pub agent_hooks: std::collections::BTreeMap<String, snapshot::AgentHook>,
    pub agent_hooks_loaded: bool,
    pub pane_cwds: std::collections::BTreeMap<String, String>,
    pub terminal_ids: std::collections::BTreeMap<String, String>,
    pub worktree_repos: std::collections::BTreeMap<String, String>,
    /// A read came back with no server.
    pub server_not_running: bool,
    /// The refusal while the server refuses this client.
    pub protocol_mismatch: Option<String>,
    /// Which side is behind, kept only while the two disagree.
    pub version_skew: Option<VersionReport>,
    /// The interval the next tick waits.
    pub interval: Duration,
    /// The mosh renderer zooms the focused pane (never on SSH).
    pub immersive_zoom: bool,
    push_active: bool,
    last_decoded: Option<Decoded>,
    unchanged_polls: u32,
    last_poll_mismatched: bool,
    zoom_after_next_apply: bool,
}

impl Default for PollState {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            agent_hooks: Default::default(),
            agent_hooks_loaded: false,
            pane_cwds: Default::default(),
            terminal_ids: Default::default(),
            worktree_repos: Default::default(),
            server_not_running: false,
            protocol_mismatch: None,
            version_skew: None,
            interval: FAST_POLL_INTERVAL,
            immersive_zoom: false,
            push_active: false,
            last_decoded: None,
            unchanged_polls: 0,
            last_poll_mismatched: false,
            zoom_after_next_apply: false,
        }
    }
}

impl PollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one read.
    pub fn apply(&mut self, reading: PollReading) -> Vec<PollEffect> {
        let mut effects = Vec::new();
        match reading {
            PollReading::Tree(decoded) => {
                self.server_not_running = false;
                self.last_poll_mismatched = false;
                self.protocol_mismatch = None;
                self.version_skew = None;
                // Ease off only while the answer keeps coming back
                // identical; snap back the moment anything moves.
                if self.last_decoded.as_ref() == Some(&*decoded) {
                    self.unchanged_polls += 1;
                    if self.unchanged_polls >= IDLE_POLL_THRESHOLD {
                        self.interval = if self.push_active {
                            PUSHED_POLL_INTERVAL
                        } else {
                            IDLE_POLL_INTERVAL
                        };
                    }
                } else {
                    self.unchanged_polls = 0;
                    self.interval = FAST_POLL_INTERVAL;
                }
                self.last_decoded = Some((*decoded).clone());
                let Decoded {
                    snapshot: mut next,
                    agent_hooks,
                    pane_cwds,
                    terminal_ids,
                    worktree_repos,
                } = *decoded;
                next.ever_attached = self.snapshot.ever_attached || next.is_attached;
                self.snapshot = next;
                self.agent_hooks = agent_hooks;
                self.agent_hooks_loaded = true;
                self.pane_cwds = pane_cwds;
                self.terminal_ids = terminal_ids;
                self.worktree_repos = worktree_repos;
                if let Some(pane) = &self.snapshot.active_pane_id {
                    effects.push(PollEffect::FocusedPane(pane.clone()));
                }
                effects.push(PollEffect::HooksUpdated);
                // A tab or workspace switch (or the first attach) names its
                // focused pane only now.
                if self.zoom_after_next_apply
                    && self.immersive_zoom
                    && let Some(pane) = &self.snapshot.active_pane_id
                {
                    self.zoom_after_next_apply = false;
                    effects.push(PollEffect::Zoom(pane.clone()));
                }
            }
            PollReading::Mismatch(message) => {
                // Keep the last good tree: the panes still exist server-side.
                self.last_poll_mismatched = true;
                if self.protocol_mismatch.is_none() {
                    effects.push(PollEffect::DiagnoseSkew);
                }
                self.protocol_mismatch = Some(message.clone());
                effects.push(PollEffect::Mismatch(message));
                self.unchanged_polls = 0;
                // It won't heal until a human restarts the server.
                self.interval = IDLE_POLL_INTERVAL;
            }
            PollReading::NoServer => {
                self.server_not_running = true;
                // A host that just lost its server is about to get a new one.
                self.unchanged_polls = 0;
                self.interval = FAST_POLL_INTERVAL;
                self.last_decoded = None;
                self.snapshot = Snapshot {
                    ever_attached: self.snapshot.ever_attached,
                    ..Snapshot::default()
                };
                self.agent_hooks.clear();
                self.agent_hooks_loaded = true;
                // What the tree said of its panes goes with it: a terminal id
                // kept from the gone tree pointed the mosh renderer at a
                // terminal no server has (a restarted one numbers them anew).
                self.terminal_ids.clear();
                self.pane_cwds.clear();
                self.worktree_repos.clear();
            }
            PollReading::NoNews => {}
        }
        effects
    }

    /// `herdr status`'s report after [`PollEffect::DiagnoseSkew`]; `None`
    /// when it couldn't be read. Returns the skew it keeps.
    pub fn note_versions(&mut self, report: Option<VersionReport>) -> Option<&VersionReport> {
        if let Some(report) = report {
            self.version_skew = report.mismatched().then_some(report);
        }
        self.version_skew.as_ref()
    }

    /// Back to the fast cadence: something happened the backoff can't know
    /// about (the frame channel lost its attach, the user asked for a
    /// refresh). The caller reads now.
    pub fn quicken(&mut self) {
        self.unchanged_polls = 0;
        self.interval = FAST_POLL_INTERVAL;
    }

    /// The push channel came up (true) or died (false).
    pub fn set_push_active(&mut self, active: bool) {
        if active == self.push_active {
            return;
        }
        self.push_active = active;
        if !active {
            self.quicken();
        }
    }

    /// Zoom whatever pane the next tree names focused (mosh renderer):
    /// after a tab or workspace switch, and on the first attach.
    pub fn zoom_after_next_tree(&mut self) {
        self.zoom_after_next_apply = true;
    }

    /// Whether the most recent read was a protocol mismatch: a restart's
    /// success check, with [`PollState::server_not_running`].
    pub fn last_read_mismatched(&self) -> bool {
        self.last_poll_mismatched
    }

    /// A restart on the user's tap — refused when the `herdr` the commands
    /// run is the older side, since restarting with it downgrades the server
    /// and exits every pane.
    pub fn restart_would_downgrade(report: Option<&VersionReport>) -> bool {
        report.is_some_and(VersionReport::client_is_older)
    }
}

/// `herdr server stop`: exempt from the version gate (the mismatch message
/// prescribes it). Run through [`launch::command`], no trailer.
pub const SERVER_STOP: &str = "server stop";
