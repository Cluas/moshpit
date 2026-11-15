//! New Agent Task (`Moshpit/UI/Herdr/NewAgentTaskSheet.swift` was) in NSH,
//! in its sheet: start an agent on its own git worktree, from a herdr
//! host's row on Home or its sessions page.
//!
//! The form is the page's: the repository (a pick from the host's, or a
//! path typed under Other…), the branch, the agent and the first message.
//! What the host knows comes in its post ([`Message`]), `{"op":
//! "agentTask", "repos", "agents", "scanning", "starting", "failure",
//! "taskExists"}`, again whenever it changes: the repositories (the panes'
//! first, then the host's scan), the agents herdr knows, whether the scan
//! is running, whether a start is in flight, why the last one failed, and
//! whether it left a worktree behind (Start would only collide with its
//! branch, so Done takes its place). The page tells it:
//!
//! - `{"ev": "start", "repo", "branch", "agent", "prompt"}`;
//! - `{"ev": "cancel"}` (live while starting too: a host that stopped
//!   answering used to hold the form with no way out) and `{"ev":
//!   "done"}`.
//!
//! Start waits until the request would pass the core's own check
//! ([`moshpit_herdr::control::AgentTaskRequest::validation_error`]); a
//! branch name git would refuse says why under its field once something
//! is typed. The words are the sheet's, key for key.
//!
//! Identifiers: `agent-task`, `agent-task-list`, `agent-task-repo`,
//! `agent-task-custom`, `agent-task-branch`, `agent-task-agent`,
//! `agent-task-prompt`, `agent-task-failure`, `start-agent-task`,
//! `agent-task-cancel`, `agent-task-done`.

use std::{rc::Rc, sync::Arc};

use moshpit_herdr::control::{AgentTaskRequest, TaskInvalid, branch_error};
use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    boundary::Tell,
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        field::{FieldLayout, field, text_input},
        icon::{IconSize, Tone, icon},
        menu::{MenuEntry, menu_following},
        notice::{BannerTone, banner},
        page,
        row::row,
        section::{Inset, section},
        tokens::token,
    },
};

/// What the host knows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct AgentTask {
    /// Repository roots on the host, absolute.
    pub repos: Vec<String>,
    /// The agents herdr knows, as it lists them.
    pub agents: Vec<String>,
    /// The host's scan for repositories is running.
    pub scanning: bool,
    /// A start is in flight.
    pub starting: bool,
    /// Why the last start didn't make it, or didn't finish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// The worktree exists but the run came back incomplete.
    pub task_exists: bool,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "agentTask", …}`.
    AgentTask(AgentTask),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("agent-task", json)?;
        match op.as_str() {
            "agentTask" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::AgentTask)
                .map_err(|e| format!("agent-task: agentTask: {e}")),
            other => Err(format!("agent-task: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::AgentTask(task) => {
                let mut value = serde_json::to_value(task).expect("a task serializes");
                value["op"] = json!("agentTask");
                value.to_string()
            }
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// The agent to start with: the ones this app is built around first
/// (herdr lists every agent it detects, alphabetically, which made the
/// default whatever sorts first), then herdr's first, then `claude`.
pub fn default_agent(agents: &[String]) -> String {
    ["claude", "codex"]
        .into_iter()
        .find(|preferred| agents.iter().any(|agent| agent == preferred))
        .map(str::to_owned)
        .or_else(|| agents.first().cloned())
        .unwrap_or_else(|| "claude".to_owned())
}

/// `/Users/dev/code/moshpit` → `~/code/moshpit`, for the menu's lines: a
/// list of nine repositories otherwise spells the account's prefix nine
/// times and wraps the part that differs. Only the home patterns git hosts
/// use; anything else stays as it is.
pub fn abbreviate(path: &str) -> String {
    for prefix in ["/Users/", "/home/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return match rest.find('/') {
                Some(slash) => format!("~{}", &rest[slash..]),
                None => "~".to_owned(),
            };
        }
    }
    if path == "/root" {
        return "~".to_owned();
    }
    if let Some(rest) = path.strip_prefix("/root/") {
        return format!("~/{rest}");
    }
    path.to_owned()
}

/// A repository's last path component, for the row.
pub fn repo_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
}

/// Why Start waits, in the app's words.
pub fn invalid_words(invalid: TaskInvalid) -> String {
    match invalid {
        TaskInvalid::PickRepository => t!("Pick a repository"),
        TaskInvalid::PickAgent => t!("Pick an agent"),
        TaskInvalid::NameBranch => t!("Name the branch"),
        TaskInvalid::BranchSpace => t!("No spaces in a branch name"),
        TaskInvalid::BranchEdge => t!("Can't start with “-” or “/”, or end with “/”"),
        TaskInvalid::BranchDotDot => t!("No “..” in a branch name"),
        TaskInvalid::BranchLock => t!("Can't end with “.lock”"),
        TaskInvalid::BranchForbidden => t!("No ~ ^ : ? * [ \\ in a branch name"),
        TaskInvalid::BranchControl => t!("No control characters in a branch name"),
    }
}

/// The id of the menu's Other…: no path starts with a NUL.
const OTHER: &str = "\u{0}other";

/// What the form holds.
#[derive(Clone, Copy)]
struct Form {
    /// The repository picked from the host's.
    repo: Signal<String>,
    /// Other… was picked: the typed path wins.
    custom: Signal<bool>,
    custom_path: Signal<String>,
    branch: Signal<String>,
    agent: Signal<String>,
    prompt: Signal<String>,
}

impl Form {
    fn new() -> Form {
        Form {
            repo: signal(String::new()),
            custom: signal(false),
            custom_path: signal(String::new()),
            branch: signal(String::new()),
            agent: signal(String::new()),
            prompt: signal(String::new()),
        }
    }

    /// The request as it would go: the typed path when Other… is open,
    /// otherwise the pick (a stale pick never overrides what was typed).
    fn request(self) -> AgentTaskRequest {
        AgentTaskRequest {
            repo_path: if self.custom.get() {
                self.custom_path.get().trim().to_owned()
            } else {
                self.repo.get()
            },
            branch: self.branch.get(),
            agent: self.agent.get(),
            prompt: self.prompt.get(),
        }
    }

    /// What the host's post fills in: the first repository while none is
    /// picked (or the pick was only a pane's guess the scan didn't find),
    /// the default agent once the scan is done and none is picked.
    fn take(self, task: &AgentTask) {
        let repo = self.repo.get_untracked();
        if (repo.is_empty() || !task.repos.contains(&repo))
            && let Some(first) = task.repos.first()
        {
            self.repo.set(first.clone());
        }
        if self.agent.get_untracked().is_empty() && !task.scanning {
            self.agent.set(default_agent(&task.agents));
        }
    }
}

/// New Agent Task.
#[component]
pub fn AgentTaskScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = signal(None::<AgentTask>);
    let form = Form::new();
    channel::hear(
        channel.as_ref(),
        Screen::AgentTask,
        Message::parse,
        move |Message::AgentTask(next)| {
            form.take(&next);
            if state.with_untracked(|s| s.as_ref() != Some(&next)) {
                state.set(Some(next));
            }
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.with(Option::is_some));
    let read = move |f: fn(&AgentTask) -> bool| state.with(|s| s.as_ref().is_some_and(f));
    let starting = computed(move || read(|s| s.starting));
    let exists = computed(move || read(|s| s.task_exists));
    let can_start =
        computed(move || !starting.get() && form.request().validation_error().is_none());
    let body = move || {
        page::page(
            bar,
            scrolled,
            "agent-task-list",
            [
                // Made again when Other… opens or closes its path's field:
                // the fields keep their text (the form holds it).
                dynamic(move || {
                    let custom = form.custom.get();
                    untracked(move || task_section(form, state, custom))
                }),
                message_section(form).into_element(),
                failure(state).into_element(),
            ],
        )
        .into_element()
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("agent-task")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(dynamic(move || {
            let exists = exists.get();
            let tell = tell.clone();
            untracked(move || sheet_bar(bar, scrolled, form, exists, starting, can_start, tell))
        }))
}

/// The bar: Cancel and Start (Starting… while it runs), or Done once a
/// start left a worktree behind.
fn sheet_bar(
    bar: BarMetrics,
    scrolled: nav_bar::ScrollEdge,
    form: Form,
    exists: bool,
    starting: Memo<bool>,
    can_start: Memo<bool>,
    tell: Tell,
) -> AnyElement {
    let (leading, trailing) = if exists {
        let tell = tell.clone();
        (
            None,
            nav_bar::BarButton {
                label: t!("Done"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: true,
                test_id: "agent-task-done",
                on_press: Box::new(move || tell(json!({"ev": "done"}))),
                menu: None,
            },
        )
    } else {
        let cancel = {
            let tell = tell.clone();
            move || tell(json!({"ev": "cancel"}))
        };
        let start = move || {
            if !can_start.get_untracked() {
                return;
            }
            dismiss_keyboard();
            let request = form.request();
            tell(json!({
                "ev": "start",
                "repo": request.repo_path,
                "branch": request.branch,
                "agent": request.agent,
                "prompt": request.prompt,
            }));
        };
        (
            Some(nav_bar::BarButton {
                label: t!("Cancel"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "agent-task-cancel",
                on_press: Box::new(cancel),
                menu: None,
            }),
            nav_bar::BarButton {
                label: t!("Start"),
                content: nav_bar::BarContent::Text,
                enabled: can_start,
                emphasized: true,
                test_id: "start-agent-task",
                on_press: Box::new(start),
                menu: None,
            }
            .busy(starting),
        )
    };
    let bar = nav_bar::NavBar {
        metrics: bar,
        back_title: String::new(),
        title: nav_bar::fixed(t!("New Agent Task")),
        leading,
        trailing: vec![trailing],
        on_back: Box::new(|| {}),
        scrolled,
    };
    if exists {
        nav_bar::root_bar(bar).into_element()
    } else {
        nav_bar::nav_bar(bar).into_element()
    }
}

/// The repository, its path when typed (`custom`), the branch and the
/// agent.
fn task_section(form: Form, state: Signal<Option<AgentTask>>, custom: bool) -> AnyElement {
    let repos = move || state.with(|s| s.as_ref().map(|s| s.repos.clone()).unwrap_or_default());
    let scanning = move || state.with(|s| s.as_ref().is_some_and(|s| s.scanning));
    let agents = move || state.with(|s| s.as_ref().map(|s| s.agents.clone()).unwrap_or_default());
    let repo_value = move || -> String {
        if form.custom.get() {
            t!("Custom")
        } else {
            let repo = form.repo.get();
            if repo.is_empty() {
                t!("Choose")
            } else {
                repo_name(&repo).to_owned()
            }
        }
    };
    let repo_entries = move || {
        let mut entries: Vec<MenuEntry> = repos()
            .iter()
            .map(|path| MenuEntry::new(path.clone(), repo_name(path)).detail(abbreviate(path)))
            .collect();
        if scanning() {
            entries.push(MenuEntry::new("", t!("Looking for repositories…")).disabled(true));
        } else if entries.is_empty() {
            // Say why the list is bare, or an empty menu reads as broken.
            entries.push(
                MenuEntry::new(
                    "",
                    t!("None found — no panes in repos, and nothing under ~"),
                )
                .disabled(true),
            );
        }
        entries.push(MenuEntry::new(OTHER, t!("Other…")));
        entries
    };
    let agent_entries = move || {
        agents()
            .into_iter()
            .map(|agent| MenuEntry::new(agent.clone(), agent))
            .collect::<Vec<_>>()
    };
    let agent_value = move || -> String {
        let agent = form.agent.get();
        if agent.is_empty() {
            t!("Choose")
        } else {
            agent
        }
    };
    let repo_label = move || format!("{}, {}", t!("Repo"), repo_value());
    let agent_label = move || format!("{}, {}", t!("Agent"), agent_value());
    let mut section = section()
        .header(t!("TASK"))
        .footer(t!(
            "Creates a git worktree on the host, then starts the agent inside it. Your working tree is untouched."
        ))
        .row(
            row(t!("Repo"))
                .value_with(move || repo_value().into())
                .loading(move || scanning() && repos().is_empty())
                .trailing(menu_mark())
                .test_id("agent-task-repo-row")
                .wrapped(move |built| {
                    over_menu(
                        built,
                        menu_following(
                            repo_entries,
                            Some("agent-task-repo"),
                            repo_label,
                            move |id| {
                                if id == OTHER {
                                    form.custom.set(true);
                                } else if !id.is_empty() {
                                    form.custom.set(false);
                                    form.repo.set(id);
                                }
                            },
                        ),
                    )
                }),
        );
    if custom {
        section = section.item(
            field(
                t!("Repository path"),
                text_input()
                    .mono()
                    .default_value(move || form.custom_path.get_untracked())
                    .placeholder(t!("Repository path"))
                    .auto_focus()
                    .on_change(move |text| form.custom_path.set(text)),
            )
            .layout(FieldLayout::Stacked)
            .test_id("agent-task-custom"),
            Inset::Content,
        );
    }
    section
        .item(
            field(
                t!("Branch"),
                text_input()
                    .mono()
                    .default_value(move || form.branch.get_untracked())
                    .placeholder(t!("Branch"))
                    .on_change(move |text| form.branch.set(text)),
            )
            // Only once something is typed: a field greyed out before
            // anyone touched it just reads as broken.
            .error(move || {
                let branch = form.branch.get();
                if branch.trim().is_empty() {
                    None
                } else {
                    branch_error(&branch).map(|invalid| invalid_words(invalid).into())
                }
            })
            .test_id("agent-task-branch"),
            Inset::Content,
        )
        .row(
            row(t!("Agent"))
                .value_with(move || agent_value().into())
                .trailing(menu_mark())
                .test_id("agent-task-agent-row")
                .wrapped(move |built| {
                    over_menu(
                        built,
                        menu_following(
                            agent_entries,
                            Some("agent-task-agent"),
                            agent_label,
                            move |id| form.agent.set(id),
                        ),
                    )
                }),
        )
        .into_element()
}

/// The first message, sent once the agent runs.
fn message_section(form: Form) -> impl IntoElement {
    section()
        .header(t!("FIRST MESSAGE"))
        .footer(t!(
            "Optional. Sent to the agent once it's running — leave blank to type it yourself."
        ))
        .item(
            field(
                t!("Prompt"),
                text_input()
                    .multiline(3, 8)
                    .default_value(move || form.prompt.get_untracked())
                    .placeholder(t!("Prompt"))
                    .on_change(move |text| form.prompt.set(text)),
            )
            .layout(FieldLayout::Stacked)
            .test_id("agent-task-prompt"),
            Inset::Content,
        )
}

/// Why the last start failed or stopped short, while it stands.
fn failure(state: Signal<Option<AgentTask>>) -> impl IntoElement {
    let failure = move || state.with(|s| s.as_ref().and_then(|s| s.failure.clone()));
    when(
        move || failure().is_some(),
        move || {
            banner(BannerTone::Warning, move || {
                failure().unwrap_or_default().into()
            })
            .icon("exclamationmark.circle.fill")
            .test_id("agent-task-failure")
        },
    )
}

/// The mark of a row that opens a menu: a select's (`ui::control`), whose
/// options are fixed where these follow the host.
fn menu_mark() -> AnyElement {
    icon("chevron.up.chevron.down")
        .size(IconSize::Sm)
        .tone(Tone::Subtle)
        .into_element()
}

/// `row` with the system's `menu` over it: a press anywhere on the row
/// opens it.
fn over_menu(row: AnyElement, menu: AnyElement) -> AnyElement {
    view().relative().child(row).child(menu).into_element()
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_default_agent_is_one_this_app_is_built_around() {
        assert_eq!(
            default_agent(&names(&["aider", "codex", "claude"])),
            "claude"
        );
        assert_eq!(default_agent(&names(&["aider", "codex"])), "codex");
        assert_eq!(default_agent(&names(&["aider", "goose"])), "aider");
        assert_eq!(default_agent(&[]), "claude");
    }

    #[test]
    fn a_home_prefix_collapses_to_a_tilde() {
        assert_eq!(abbreviate("/Users/dev/code/moshpit"), "~/code/moshpit");
        assert_eq!(abbreviate("/home/deploy/sample-app"), "~/sample-app");
        assert_eq!(abbreviate("/root/gotools"), "~/gotools");
        assert_eq!(abbreviate("/Users/dev"), "~");
        assert_eq!(abbreviate("/srv/repos/api"), "/srv/repos/api");
    }

    #[test]
    fn a_repository_is_named_by_its_last_component() {
        assert_eq!(repo_name("/Users/dev/code/moshpit"), "moshpit");
        assert_eq!(repo_name("/srv/api/"), "api");
        assert_eq!(repo_name("api"), "api");
    }

    #[test]
    fn a_message_round_trips() {
        let task = AgentTask {
            repos: names(&["/a"]),
            agents: names(&["claude"]),
            scanning: true,
            starting: false,
            failure: Some("no".into()),
            task_exists: true,
        };
        let json = Message::AgentTask(task.clone()).to_json();
        assert_eq!(Message::parse(&json), Ok(Message::AgentTask(task)));
        assert_eq!(
            Message::parse(r#"{"op":"agentTask"}"#),
            Ok(Message::AgentTask(AgentTask::default()))
        );
    }
}
