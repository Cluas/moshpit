//! Settings › Set up a host (`docs/nsh/UI-DESIGN.md` §6.8, R16), in its
//! sheet: the agents' status scripts and the push pairing on the connected
//! host, each with the test that proves it works.
//!
//! Everything here is a state machine over a remote host, and it stays the
//! core's (`moshpit_hostcare`'s Host Setup page, `HostCare.swift` on iOS).
//! The host posts what the core says ([`Message`]): `{"op": "host-setup",
//! …}` ([`HostSetup`]: the phase, the host's facts, the hooks' and the
//! pairing's status, the tests, the errors), again whenever it changes;
//! the page tells it what the user asks for: `{"ev": "done" | "inspect" |
//! "install-hooks" | "install-hooks-anyway" | "dismiss" | "remove-hooks" |
//! "prove-hooks" | "pair" | "unpair" | "prove-push" | "remove-everything"}`
//! and `{"ev": "agent", "id": …}`.
//!
//! # The task
//!
//! Make this host tell this phone what its coding agents are doing, by
//! the status scripts in their configs and by push, and prove that it
//! does. The object is the connected host; what decides the next step is
//! whether it has the tools a push needs, whether the chosen agent's
//! status scripts are installed and current, whether the host is paired,
//! and what the last test found.
//!
//! # The page
//!
//! A sheet: its bar's title and Done (the spinner in Done's place while a
//! step runs; a test's own row holds its spinner). Then, on
//! [`page::page`]:
//!
//! - **Host**: the operating system and each tool, installed or not; a
//!   warning when a push needs a tool the host lacks. Under it, what the
//!   last step failed with, and "paired, but nothing will push".
//! - **Agent status**: the status scripts' row (the agent's config, a
//!   badge), the agent (a select whose menu says each agent's status, or
//!   that the host doesn't have it), Install (Update, Reinstall) and
//!   Remove, and the test. A step left on the host (Codex's trust) under
//!   it.
//! - **Push**: why this phone has no push service credential, if it has
//!   none; the pairing's row (where it pushes, a badge), Pair (Re-pair)
//!   and Unpair, and the test notification.
//! - **Remove Moshpit from this host**, once something of Moshpit's is
//!   there; it asks first, in the system's alert.
//!
//! Until the host is read, the Host section says it is looking; a host
//! that couldn't be read says so, the host's reason under it, with Retry
//! (`inspect`), and nothing else is offered (B382). While a step or a test
//! runs, every other control waits, the agent menu too (B383, B384).
//! Installing for an agent the host doesn't have asks first: the core's
//! question (`confirm`), in the system's alert (B381).
//!
//! # Motion
//!
//! | Change | Motion |
//! | --- | --- |
//! | A row pressed (a step, a test, the agent, Remove) | The row's wash, `instant` (the `ui` row) |
//! | Done while a step runs | The spinner takes Done's place at its width (the bar's busy button) |
//! | A step running | Its row's accessory cross-fades to the spinner and back, `base` |
//! | A status changes (installed, paired, out of date) | Its badge rises 4 pt and fades in, `base`, ease-out |
//! | A test's verdict | The spinner cross-fades away, `base`; its badge rises in, `base`, ease-out; the line under the title says the new state at once |
//! | A warning or an error appears or goes | The banner slides 8 pt in and fades, `slow`, ease-out; leaves fading, `base`, ease-in |
//! | The host read (or read again) | The looking row gives way: the page fades in over it, `base` |
//! | The agent menu | The system's menu |
//! | Remove, and the question before an install | The system's alert |
//! | Reduced motion | All of it at once; the spinners stay (they say work is going on) |
//!
//! The identifiers the app's tests use: `hostsetup`, `hostsetup-done`,
//! `hostsetup-agent` (the menu's UIKit control) and `hostsetup-*`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    boundary,
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        alert::{AlertDialogOptions, alert_dialog},
        button::{Variant, button},
        control::{option, select},
        empty::{empty, loaded},
        icon::Tone,
        motion::{Duration, ENTER, distance},
        notice::{BannerTone, banner},
        page,
        row::{Row, RowTone, row},
        section::{Inset, section},
        status::{BadgeVariant, badge},
        tokens::token,
    },
};

/// What the model is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    #[default]
    Idle,
    Inspecting,
    Working,
    /// A proof runs: the steps wait for its verdict, Done stays (its own
    /// row has the spinner).
    Proving,
}

/// An installed component's state on the host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    Absent,
    Current,
    Stale,
    /// The host hasn't been read (or couldn't be): nothing is said about
    /// it, and nothing is offered but reading it again (B382).
    Unknown,
}

/// A proof's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProofState {
    Untested,
    Proving,
    Proven,
    Failed,
    Unavailable,
}

/// Whether a path has been proven, and what was learnt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub state: ProofState,
    /// Why it failed or can't be tried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One tool the host has or lacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub present: bool,
}

/// What the host is, gathered before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Facts {
    pub uname: String,
    pub tools: Vec<Tool>,
    /// What pushes need that the host lacks (empty: it can push).
    pub missing_for_push: Vec<String>,
}

/// An agent the hooks are for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Where its hooks are configured (`~/.claude/settings.json`).
    #[serde(default)]
    pub config_hint: String,
    /// Its hooks on this host (B381).
    #[serde(default)]
    pub status: Status,
    /// Whether the host has it; absent when that isn't known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HostSetup {
    pub phase: Phase,
    /// What a running step says it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts: Option<Facts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Paired, with no hook script that will ever send.
    pub nothing_will_push: bool,
    /// The agent the hooks' buttons act on, and the others.
    pub agent: Agent,
    pub agents: Vec<Agent>,
    pub hooks: Status,
    /// A step the user must take on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_action: Option<String>,
    /// Asked before installing hooks for an agent the host doesn't have:
    /// why (B381). Yes is `{"ev": "install-hooks-anyway"}`, no
    /// `{"ev": "dismiss"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    pub hooks_proof: Proof,
    /// Why this phone has no relay credential, if it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_error: Option<String>,
    /// The relay the host is paired with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_relay: Option<String>,
    /// The relay this phone uses (`relayURL`, as the host writes it).
    #[serde(rename = "relayURL")]
    pub relay_url: String,
    pub pairing: Status,
    pub push_proof: Proof,
    /// Moshpit has put something on this host (its manifest records it):
    /// "Remove Moshpit from this host" is offered (B375).
    #[serde(default)]
    pub removable: bool,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "host-setup", …}`: what the page shows now.
    HostSetup(Box<HostSetup>),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("host-setup", json)?;
        match op.as_str() {
            "host-setup" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(|s| Message::HostSetup(Box::new(s)))
                .map_err(|e| format!("host-setup: host-setup: {e}")),
            other => Err(format!("host-setup: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::HostSetup(setup) => {
                let mut value = serde_json::to_value(setup).expect("a setup serializes");
                value["op"] = json!("host-setup");
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

/// What tells the host.
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The page's state: the host's last post.
type State = Signal<Option<HostSetup>>;

fn read<T: Default>(state: State, f: impl FnOnce(&HostSetup) -> T) -> T {
    state.with(|s| s.as_ref().map(f).unwrap_or_default())
}

/// A step or a test is running: every other control waits.
fn working(state: State) -> bool {
    read(state, |s| s.phase != Phase::Idle)
}

/// A step is running (not a test, whose own row says so): Done waits.
fn stepping(state: State) -> bool {
    read(state, |s| {
        matches!(s.phase, Phase::Inspecting | Phase::Working)
    })
}

/// The host has been read: its statuses mean something.
fn known(state: State) -> bool {
    state.with(|s| s.as_ref().is_some_and(|s| s.hooks != Status::Unknown))
}

/// The page has settled on what to show: the host read, or a read that
/// failed (nothing running, nothing known).
fn settled(state: State) -> bool {
    state.with(|s| {
        s.as_ref()
            .is_some_and(|s| s.hooks != Status::Unknown || s.phase == Phase::Idle)
    })
}

/// Set up a host (§6.8).
#[component]
pub fn HostSetupScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state: State = signal(None);
    channel::hear(
        channel.as_ref(),
        Screen::HostSetup,
        Message::parse,
        move |Message::HostSetup(setup)| state.set(Some(*setup)),
    );
    let tell = tell_host(channel);
    let scrolled = nav_bar::ScrollEdge::new();
    // The step the user started last: its row holds the spinner while the
    // core works on it.
    let pending = signal(None::<&'static str>);
    effect(move || {
        if !working(state) {
            pending.set(None);
        }
    });
    let steps = Steps {
        state,
        pending,
        tell: tell.clone(),
    };
    ask_before_installing(state, tell.clone());
    let body = {
        let steps = steps.clone();
        move || page::page(bar, scrolled, "hostsetup-list", [content(steps.clone())]).into_element()
    };
    let stepping = computed(move || stepping(state));
    let done = tell.clone();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("hostsetup")
        .child(boundary::guarded(body, tell.clone()))
        .child(nav_bar::root_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: nav_bar::fixed(t!("Set up a host")),
            leading: None,
            trailing: vec![
                nav_bar::BarButton {
                    label: t!("Done"),
                    content: nav_bar::BarContent::Text,
                    enabled: computed(|| true),
                    emphasized: true,
                    test_id: "hostsetup-done",
                    on_press: Box::new(move || done(json!({"ev": "done"}))),
                    menu: None,
                }
                .busy(stepping),
            ],
            on_back: Box::new(|| {}),
            scrolled,
        }))
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// The core's question before installing status scripts for an agent the
/// host doesn't have (`confirm`), asked once in the system's alert: Install
/// Anyway tells `install-hooks-anyway`, anything else `dismiss`.
fn ask_before_installing(state: State, tell: Tell) {
    let asked = stored(None::<String>);
    effect(move || {
        let question = read(state, |s| s.confirm.clone());
        let Some(why) = question else {
            asked.replace(None);
            return;
        };
        if asked.get().as_deref() == Some(why.as_str()) {
            return;
        }
        asked.replace(Some(why.clone()));
        let name =
            state.with_untracked(|s| s.as_ref().map(|s| s.agent.name.clone()).unwrap_or_default());
        let answer = alert_dialog()
            .title(t!("%@ isn't on this host", arg1 = name))
            .description(why)
            .action("install", t!("Install Anyway"))
            .cancel("cancel", t!("Cancel"))
            .show();
        let tell = tell.clone();
        drop(spawn(async move {
            let yes = answer
                .await
                .is_ok_and(|r| r.action.as_deref() == Some("install"));
            tell(json!({"ev": if yes { "install-hooks-anyway" } else { "dismiss" }}));
        }));
    });
}

/// What the page shows under its bar: that it is looking at the host, then
/// the host as read (or why it couldn't be), fading in over the looking.
fn content(steps: Steps) -> AnyElement {
    let state = steps.state;
    loaded(
        move || settled(state),
        move || looking(state),
        move || {
            let steps = steps.clone();
            dynamic(move || {
                let read_host = known(state);
                let steps = steps.clone();
                untracked(move || {
                    if read_host {
                        sections(steps)
                    } else {
                        unread(steps)
                    }
                })
            })
            .into_element()
        },
    )
}

/// The Host section while the page waits for the host: what it is doing.
fn looking(state: State) -> AnyElement {
    section()
        .header(t!("Host"))
        .row(
            row("")
                .title_with(move || {
                    read(state, |s| s.phase_label.clone())
                        .unwrap_or_else(|| t!("Looking at the host…"))
                        .into()
                })
                .loading(|| true)
                .test_id("hostsetup-looking"),
        )
        .into_element()
}

/// A host that couldn't be read: that, the host's reason, and Retry.
/// Nothing else is offered (B382).
fn unread(steps: Steps) -> AnyElement {
    let Steps { state, tell, .. } = steps;
    let mut said = empty(t!("Couldn't read this host"))
        .icon("exclamationmark.triangle")
        .action(
            button(t!("Retry"))
                .variant(Variant::Secondary)
                .disabled(move || working(state))
                .on_press(move || tell(json!({"ev": "inspect"})))
                .test_id("hostsetup-retry"),
        )
        .test_id("hostsetup-unread");
    if let Some(why) = state.with_untracked(|s| s.as_ref().and_then(|s| s.error.clone())) {
        said = said.message(why);
    }
    said.into_element()
}

/// The host as read: its sections, `space-6` apart as the page's are.
fn sections(steps: Steps) -> AnyElement {
    let state = steps.state;
    view()
        .w_full()
        .gap(px(page::BETWEEN))
        .child(host_group(state))
        .child(hooks_group(steps.clone()))
        .child(push_group(steps.clone()))
        .child(when(
            move || read(state, |s| s.removable),
            move || remove_section(steps.clone()),
        ))
        .into_element()
}

// --- Steps --------------------------------------------------------------------------------

/// What a step's row needs: the host's state, the step started last, and
/// what tells the host.
#[derive(Clone)]
struct Steps {
    state: State,
    pending: Signal<Option<&'static str>>,
    tell: Tell,
}

impl Steps {
    /// A step's row: `title`, in `tone`; a press starts step `ev`. While
    /// the core works on it the row holds the spinner; while anything
    /// else runs, or `enabled` says so, it waits.
    fn step(
        &self,
        title: impl Fn() -> String + 'static,
        tone: RowTone,
        ev: &'static str,
        enabled: impl Fn() -> bool + 'static,
        test_id: &str,
    ) -> Row {
        let Steps {
            state,
            pending,
            tell,
        } = self.clone();
        let running = Rc::new(move || {
            read(state, |s| s.phase == Phase::Working) && pending.get() == Some(ev)
        });
        let spinning = running.clone();
        row("")
            .title_with(move || title().into())
            .tone(tone)
            .loading(move || spinning())
            .disabled(move || !running() && (working(state) || !enabled()))
            .on_press(move || {
                pending.set(Some(ev));
                tell(json!({"ev": ev}));
            })
            .test_id(test_id)
    }
}

// --- Host --------------------------------------------------------------------------------------

/// Host: the system and the tools, what a push needs that the host lacks,
/// what the last step failed with, and "paired, but nothing will push".
fn host_group(state: State) -> AnyElement {
    let facts = computed(move || read(state, |s| s.facts.clone()));
    let missing = move || {
        read(state, |s| {
            s.facts
                .as_ref()
                .map(|f| f.missing_for_push.join(", "))
                .unwrap_or_default()
        })
    };
    view()
        .w_full()
        .gap_3()
        .child(dynamic(move || {
            let facts = facts.get();
            untracked(move || host_section(facts))
        }))
        .child(
            banner(
                BannerTone::Warning,
                kept(move || {
                    let missing = missing();
                    (!missing.is_empty())
                        .then(|| t!("Pushes need %@ on this host.", arg1 = missing))
                }),
            )
            .shown(move || !missing().is_empty())
            .test_id("hostsetup-missing"),
        )
        .child(
            banner(
                BannerTone::Destructive,
                kept(move || read(state, |s| s.error.clone())),
            )
            .shown(move || read(state, |s| s.error.is_some()))
            .test_id("hostsetup-error"),
        )
        .child(
            banner(BannerTone::Warning, || {
                t!("Paired, but nothing will push. Install the status scripts below.").into()
            })
            .icon("bell.slash.fill")
            .shown(move || read(state, |s| s.nothing_will_push))
            .test_id("hostsetup-nothing-will-push"),
        )
        .into_element()
}

/// The Host section for `facts`.
fn host_section(facts: Option<Facts>) -> AnyElement {
    let Some(facts) = facts else {
        return view().into_element();
    };
    let system = if facts.uname.is_empty() {
        "—".to_owned()
    } else {
        facts.uname
    };
    let tools = facts.tools.into_iter().map(|tool| {
        // jq is optional: without it the status still comes, untitled.
        let tone = if tool.present || tool.name == "jq" {
            Tone::Muted
        } else {
            Tone::Warning
        };
        row(tool.name.clone())
            .value(if tool.present {
                t!("Installed")
            } else {
                t!("Not installed")
            })
            .value_tone(tone)
            .test_id(&format!("hostsetup-tool-{}", tool.name))
    });
    section()
        .header(t!("Host"))
        .row(
            row(t!("Operating system"))
                .value(system)
                .mono_value()
                .test_id("hostsetup-uname"),
        )
        .rows(tools)
        .test_id("hostsetup-host")
        .into_element()
}

// --- Agent status --------------------------------------------------------------------------

/// Agent status: the status scripts' row, the agent, Install (Update,
/// Reinstall) and Remove, the test; the step left on the host under it.
fn hooks_group(steps: Steps) -> AnyElement {
    let state = steps.state;
    let shape = computed(move || read(state, |s| (s.hooks != Status::Absent, s.agents.clone())));
    view()
        .w_full()
        .gap_3()
        .child(dynamic(move || {
            let (installed, agents) = shape.get();
            let steps = steps.clone();
            untracked(move || hooks_section(steps, installed, agents))
        }))
        .child(
            banner(
                BannerTone::Warning,
                kept(move || read(state, |s| s.user_action.clone())),
            )
            .icon("hand.raised.fill")
            .shown(move || read(state, |s| s.user_action.is_some()))
            .test_id("hostsetup-user-action"),
        )
        .into_element()
}

fn hooks_section(steps: Steps, installed: bool, agents: Vec<Agent>) -> AnyElement {
    let state = steps.state;
    let hooks = move || read(state, |s| s.hooks);
    let tell = steps.tell.clone();
    let agent = select(
        t!("Agent"),
        agents
            .iter()
            .map(|a| option(a.id.clone(), a.name.clone()).detail(agent_detail(a))),
        move || read(state, |s| s.agent.id.clone()).into(),
        move |id| tell(json!({"ev": "agent", "id": id.to_string()})),
    )
    .disabled(move || working(state))
    .test_id("hostsetup-agent");
    let mut group = section()
        .header(t!("Agent status"))
        .row(
            row(t!("Status scripts"))
                .subtitle_with(move || read(state, |s| s.agent.config_hint.clone()).into())
                .mono_subtitle()
                .label_with(move || {
                    format!("{}, {}", t!("Status scripts"), hooks_words(hooks())).into()
                })
                .trailing(rises_in(hooks, |status| {
                    badge(hooks_words(status))
                        .variant(variant(status))
                        .into_element()
                }))
                .test_id("hostsetup-hooks"),
        )
        .item(agent, Inset::Content)
        .row(steps.step(
            move || match hooks() {
                Status::Absent | Status::Unknown => t!("Install"),
                Status::Current => t!("Reinstall"),
                Status::Stale => t!("Update"),
            },
            RowTone::Primary,
            "install-hooks",
            || true,
            "hostsetup-install",
        ));
    if installed {
        group = group.row(steps.step(
            || t!("Remove"),
            RowTone::Destructive,
            "remove-hooks",
            || true,
            "hostsetup-remove",
        ));
    }
    group
        .row(proof_row(
            &steps,
            |s| s.hooks_proof.clone(),
            hooks_proof_words(state),
            "prove-hooks",
            || true,
            "hostsetup-hooks-proof",
        ))
        .into_element()
}

/// What the agent menu says under an agent (B381): its status scripts
/// here, or that the host doesn't have it.
fn agent_detail(agent: &Agent) -> String {
    if agent.found == Some(false) && agent.status == Status::Absent {
        return t!("Not found on this host");
    }
    hooks_words(agent.status)
}

/// The status scripts' state, in words.
fn hooks_words(status: Status) -> String {
    match status {
        Status::Absent => t!("Not installed"),
        Status::Current => t!("Installed"),
        Status::Stale => t!("Out of date"),
        Status::Unknown => t!("Unknown"),
    }
}

/// The pairing's state, in words.
fn pairing_words(status: Status) -> String {
    match status {
        Status::Absent => t!("Not paired"),
        Status::Current => t!("Paired"),
        Status::Stale => t!("Out of date"),
        Status::Unknown => t!("Unknown"),
    }
}

/// A status's badge: green when current, amber when out of date, neutral
/// otherwise.
fn variant(status: Status) -> BadgeVariant {
    match status {
        Status::Current => BadgeVariant::Success,
        Status::Stale => BadgeVariant::Warning,
        Status::Absent => BadgeVariant::Neutral,
        Status::Unknown => BadgeVariant::Outline,
    }
}

// --- Push ------------------------------------------------------------------------------------

/// Push: why this phone has no push service credential, the pairing's
/// row, Pair (Re-pair) and Unpair, the test notification.
fn push_group(steps: Steps) -> AnyElement {
    let state = steps.state;
    let paired = computed(move || read(state, |s| s.pairing != Status::Absent));
    view()
        .w_full()
        .gap_3()
        .child(
            banner(
                BannerTone::Warning,
                kept(move || read(state, |s| s.relay_error.clone())),
            )
            .icon("antenna.radiowaves.left.and.right.slash")
            .shown(move || read(state, |s| s.relay_error.is_some()))
            .test_id("hostsetup-relay-error"),
        )
        .child(dynamic(move || {
            let paired = paired.get();
            let steps = steps.clone();
            untracked(move || push_section(steps, paired))
        }))
        .into_element()
}

fn push_section(steps: Steps, paired: bool) -> AnyElement {
    let state = steps.state;
    let pairing = move || read(state, |s| s.pairing);
    let mut group = section()
        .header(t!("Push"))
        .row(
            row(t!("Pairing"))
                // Where it pushes: the relay the host is paired with, else
                // the one a pairing would use.
                .subtitle_with(move || {
                    read(state, |s| {
                        s.paired_relay.clone().unwrap_or(s.relay_url.clone())
                    })
                    .into()
                })
                .mono_subtitle()
                .label_with(move || {
                    format!("{}, {}", t!("Pairing"), pairing_words(pairing())).into()
                })
                .trailing(rises_in(pairing, |status| {
                    badge(pairing_words(status))
                        .variant(variant(status))
                        .into_element()
                }))
                .test_id("hostsetup-pairing"),
        )
        .row(steps.step(
            move || {
                if pairing() == Status::Absent {
                    t!("Pair")
                } else {
                    t!("Re-pair")
                }
            },
            RowTone::Primary,
            "pair",
            || true,
            "hostsetup-pair",
        ));
    if paired {
        group = group.row(steps.step(
            || t!("Unpair"),
            RowTone::Destructive,
            "unpair",
            || true,
            "hostsetup-unpair",
        ));
    }
    group
        .row(proof_row(
            &steps,
            |s| s.push_proof.clone(),
            push_proof_words(),
            "prove-push",
            move || pairing() != Status::Absent,
            "hostsetup-push-proof",
        ))
        .footer(t!(
            "Reaches you when Moshpit isn't running. Your relay signs the push; it never sees what the agent said."
        ))
        .into_element()
}

// --- Remove ----------------------------------------------------------------------------------

/// Remove Moshpit from this host (B375): a destructive row that asks
/// first, in the system's alert, saying what goes.
fn remove_section(steps: Steps) -> AnyElement {
    let Steps { state, tell, .. } = steps;
    section()
        .row(
            row(t!("Remove Moshpit from this host"))
                .tone(RowTone::Destructive)
                .disabled(move || working(state))
                .on_press(move || {
                    let answer = remove_question().show();
                    let tell = tell.clone();
                    drop(spawn(async move {
                        if answer
                            .await
                            .is_ok_and(|r| r.action.as_deref() == Some("remove"))
                        {
                            tell(json!({"ev": "remove-everything"}));
                        }
                    }));
                })
                .test_id("hostsetup-remove-everything"),
        )
        .into_element()
}

/// What Remove Moshpit from this host asks first: what goes, and what
/// stays.
pub fn remove_question() -> AlertDialogOptions {
    alert_dialog()
        .title(t!("Remove Moshpit from this host?"))
        .description(t!(
            "Moshpit's hooks come out of every agent's config, and this phone's pairing is removed. If no other phone is paired with this host, ~/.moshpit goes too. Everything else in the agents' configs stays."
        ))
        .destructive("remove", t!("Remove"))
        .cancel("cancel", t!("Cancel"))
}

// --- Tests ---------------------------------------------------------------------------------

/// What a test's row says: its title, and the line under it for each
/// state (B380: the hooks' test and the push's are different tests and say
/// so).
struct ProofWords {
    action: String,
    explain: Rc<dyn Fn() -> String>,
    proving: String,
    proven: String,
}

/// The hooks' test: it runs the agent's registered command in this pane.
fn hooks_proof_words(state: State) -> ProofWords {
    ProofWords {
        action: t!("Test the status script"),
        explain: Rc::new(move || {
            t!(
                "Runs the command %@'s config registers for its hooks, in this tmux pane, and reads the pane back. The agent itself isn't started.",
                arg1 = read(state, |s| s.agent.name.clone())
            )
        }),
        proving: t!("Running it in this pane…"),
        proven: t!("Works — the pane shows the status it set."),
    }
}

/// The push's test: the host sends a real one to this phone.
fn push_proof_words() -> ProofWords {
    ProofWords {
        action: t!("Send a test notification"),
        explain: Rc::new(|| {
            t!("The host sends one for real. If it arrives here, the whole chain works.")
        }),
        proving: t!("Waiting for it to arrive…"),
        proven: t!("Proven — it arrived on this phone."),
    }
}

/// A test's row: its title, the line under it saying what it does or what
/// it found, the spinner while it runs and its verdict's badge after. A
/// press runs it (again); it waits while anything else runs, and can't
/// run when the core says why not.
fn proof_row(
    steps: &Steps,
    proof: fn(&HostSetup) -> Proof,
    words: ProofWords,
    ev: &'static str,
    enabled: impl Fn() -> bool + 'static,
    test_id: &str,
) -> Row {
    let Steps { state, tell, .. } = steps.clone();
    let ProofWords {
        action,
        explain,
        proving,
        proven,
    } = words;
    let current = move || {
        state.with(|s| {
            s.as_ref().map(proof).unwrap_or(Proof {
                state: ProofState::Untested,
                message: None,
            })
        })
    };
    let verdict = move || current().state;
    let title = action.clone();
    row(action)
        .subtitle_with(move || {
            let proof = current();
            match proof.state {
                ProofState::Untested => explain(),
                ProofState::Proving => proving.clone(),
                ProofState::Proven => proven.clone(),
                ProofState::Failed | ProofState::Unavailable => proof.message.unwrap_or_default(),
            }
            .into()
        })
        .label_with(move || {
            let said = match verdict() {
                ProofState::Proven => t!("Passed"),
                ProofState::Failed => t!("Failed"),
                _ => String::new(),
            };
            if said.is_empty() {
                title.clone().into()
            } else {
                format!("{title}, {said}").into()
            }
        })
        .tone(RowTone::Primary)
        .loading(move || verdict() == ProofState::Proving)
        .disabled(move || {
            let verdict = verdict();
            verdict != ProofState::Proving
                && (working(state) || verdict == ProofState::Unavailable || !enabled())
        })
        .trailing(rises_in(verdict, |verdict| match verdict {
            ProofState::Proven => badge(t!("Passed"))
                .variant(BadgeVariant::Success)
                .into_element(),
            ProofState::Failed => badge(t!("Failed"))
                .variant(BadgeVariant::Destructive)
                .into_element(),
            _ => view().into_element(),
        }))
        .on_press(move || tell(json!({"ev": ev})))
        .test_id(test_id)
}

// --- Helpers -------------------------------------------------------------------------------

/// What `make` draws for `key()`, again whenever it changes: what replaces
/// it rises 4 pt and fades in (`base`, ease-out). The first, and every one
/// while motion is reduced, shows at once.
fn rises_in<K: PartialEq + Clone + 'static>(
    key: impl Fn() -> K + 'static,
    make: impl Fn(K) -> AnyElement + 'static,
) -> AnyElement {
    let key = computed(key);
    let first = stored(true);
    let reduced = reduce_motion();
    dynamic(move || {
        let now = key.get();
        let animate = !first.replace(false) && !reduced;
        let drawn = untracked(|| make(now));
        if !animate {
            return drawn;
        }
        view()
            .bind_motion(
                &motion().entering(
                    MotionSpec::new(KeyframeTiming::new(Duration::Base.nominal_ms()).easing(ENTER))
                        .values(MotionChannel::Opacity, [0.0, 1.0])
                        .values(MotionChannel::TranslateY, [distance::RISE, 0.0]),
                ),
            )
            .child(drawn)
            .into_element()
    })
    .into_element()
}

/// A banner's words that stay what they last were once `words()` has none:
/// the banner says them while it fades out.
fn kept(words: impl Fn() -> Option<String> + 'static) -> impl Fn() -> SharedString + 'static {
    let last = stored(String::new());
    move || {
        if let Some(now) = words() {
            last.replace(now);
        }
        last.get().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn setup() -> HostSetup {
        HostSetup {
            phase: Phase::Idle,
            phase_label: None,
            facts: Some(Facts {
                uname: "Darwin".into(),
                tools: ["openssl", "curl", "jq", "tmux"]
                    .map(|name| Tool {
                        name: name.into(),
                        present: true,
                    })
                    .into(),
                missing_for_push: vec![],
            }),
            error: None,
            nothing_will_push: false,
            agent: Agent {
                id: "claude".into(),
                name: "Claude Code".into(),
                config_hint: "~/.claude/settings.json".into(),
                status: Status::Absent,
                found: None,
            },
            agents: vec![Agent {
                id: "claude".into(),
                name: "Claude Code".into(),
                config_hint: String::new(),
                status: Status::Absent,
                found: Some(true),
            }],
            hooks: Status::Absent,
            user_action: None,
            confirm: None,
            hooks_proof: Proof {
                state: ProofState::Untested,
                message: None,
            },
            relay_error: None,
            paired_relay: None,
            relay_url: "https://push.example.com".into(),
            pairing: Status::Absent,
            push_proof: Proof {
                state: ProofState::Untested,
                message: None,
            },
            removable: false,
        }
    }

    #[test]
    fn messages_round_trip() {
        let message = Message::HostSetup(Box::new(setup()));
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let busy = Message::HostSetup(Box::new(HostSetup {
            phase: Phase::Working,
            phase_label: Some("Installing hooks for Claude Code…".into()),
            facts: None,
            error: Some("permission denied".into()),
            hooks: Status::Stale,
            hooks_proof: Proof {
                state: ProofState::Failed,
                message: Some("The pane came back empty.".into()),
            },
            ..setup()
        }));
        assert_eq!(Message::parse(&busy.to_json()), Ok(busy));
    }

    #[test]
    fn the_host_message_as_written() {
        let json = r#"{"op":"host-setup","phase":"inspecting","phaseLabel":"Looking at the host…","nothingWillPush":false,"agent":{"id":"claude","name":"Claude Code","configHint":"~/.claude/settings.json"},"agents":[{"id":"claude","name":"Claude Code","configHint":"~/.claude/settings.json"}],"hooks":"absent","hooksProof":{"state":"untested"},"relayURL":"https://push.example.com","pairing":"absent","pushProof":{"state":"untested"}}"#;
        let Message::HostSetup(setup) = Message::parse(json).unwrap();
        assert_eq!(setup.relay_url, "https://push.example.com");
        assert_eq!(setup.phase, Phase::Inspecting);
        assert_eq!(setup.facts, None);
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        let mut bad = serde_json::to_value(setup()).unwrap();
        bad["op"] = json!("host-setup");
        bad["hooks"] = json!("installed");
        let error = Message::parse(&bad.to_string()).unwrap_err();
        assert!(error.contains("installed"), "{error}");
        let error = Message::parse(r#"{"op":"keys"}"#).unwrap_err();
        assert!(error.contains("no op"), "{error}");
    }
}
