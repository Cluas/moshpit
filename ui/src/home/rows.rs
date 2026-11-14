//! Home's rows (`docs/nsh/UI-DESIGN.md` §6.1, decision 4): R3's rows on
//! their fixed lanes, each one target.
//!
//! ```text
//! [mark 28] [project                    ] [state / time] [›]
//!           [agent · ~/path (mono)      ]
//! ```
//!
//! An agent's row is called by its project (R5's names), its agent and
//! where it works under it; its value is its state: waiting `2m` in amber,
//! working `● 12m` (the dot breathes), done in blue, idle nothing. Screen
//! readers hear "api, claude, working 12 minutes, ~/src/api".

use nsh_core::prelude::*;

use super::{
    Ctx, Intent,
    model::{AgentRow, AgentState, Connection, HostRow, Mux, OtherPane, WaitingRow, pane_icon},
    swipe::{self, Action, Tint},
};
use crate::ui::{
    icon::{Tone, icon},
    mark::{MarkSize, agent_mark, host_mark},
    row::{Row, row},
    section::{Inset, section},
    status::Status,
};

/// The dot an agent's state shows as.
pub(crate) fn status_of(state: AgentState) -> Status {
    match state {
        AgentState::NeedsYou => Status::Attention,
        AgentState::Working => Status::Working,
        AgentState::Done => Status::Done,
        AgentState::Idle => Status::Idle,
    }
}

/// The state's word, as screen readers hear it within the row's sentence
/// ("api, claude, working 12 minutes, ~/src/api"). The lower-case keys are
/// the agent's states in the word list (`crates/copy`: "done" is 已完成,
/// "Done" the button's 完成).
fn state_word(state: AgentState) -> String {
    match state {
        AgentState::NeedsYou => t!("needs you"),
        AgentState::Working => t!("working"),
        AgentState::Done => t!("done"),
        AgentState::Idle => t!("idle"),
    }
}

/// `path` as the user reads it: under the login's home, `~` for it
/// (`/home/dev/src/api` for `dev` is `~/src/api`).
pub(crate) fn short_path(path: &str, user: &str) -> String {
    let homes = [
        format!("/home/{user}"),
        format!("/Users/{user}"),
        if user == "root" {
            "/root".to_owned()
        } else {
            String::new()
        },
    ];
    for home in homes.iter().filter(|h| !h.is_empty()) {
        if path == home {
            return "~".to_owned();
        }
        if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    path.to_owned()
}

/// Whole minutes from `since` to `now`: what a row's time says ("2m",
/// "1h 12m"; "now" under a minute).
fn elapsed(since: Option<f64>, now: f64) -> String {
    super::model::elapsed_label(since, now, &t!("now")).unwrap_or_default()
}

/// The same time in words, for screen readers ("12 minutes", "1 hour 12
/// minutes").
fn spoken(since: Option<f64>, now: f64) -> String {
    let Some(since) = since else {
        return String::new();
    };
    let minutes = (((now - since) as i64) / 60).max(0);
    if minutes < 60 {
        return crate::plural!("%lld minutes", minutes);
    }
    let hours = crate::plural!("%lld hours", minutes / 60);
    match minutes % 60 {
        0 => hours,
        rest => format!("{hours} {}", crate::plural!("%lld minutes", rest)),
    }
}

/// An agent's row; `host` named in its subtitle (NEEDS YOU's rows).
fn agent(agent_row: AgentRow, user: String, host: Option<String>, ctx: &Ctx) -> Row {
    let AgentRow {
        connection,
        pane,
        title,
        agent,
        path,
        state,
        since,
    } = agent_row;
    let now = ctx.now;
    let words = match &host {
        Some(host) => format!("{agent} · {host}"),
        None => agent.clone(),
    };
    let shown_path = path.as_deref().map(|p| short_path(p, &user));
    let mut r = row(title.clone()).leading(agent_mark(status_of(state)));
    r = match shown_path.clone() {
        Some(path) => {
            let words = words.clone();
            r.subtitle_parts(move || words.clone().into(), move || path.clone().into())
        }
        None => r.subtitle(words.clone()),
    };
    r = match state {
        AgentState::NeedsYou => r
            .value_with(move || elapsed(since, now.get()).into())
            .value_tone(Tone::Warning),
        AgentState::Working => r
            .value_with(move || elapsed(since, now.get()).into())
            .value_dot(Status::Working)
            .value_tone(Tone::Foreground),
        AgentState::Done => r.value(t!("done")).value_tone(Tone::Info),
        AgentState::Idle => r,
    };
    let said = {
        let (title, agent, host, path) = (title, agent, host, shown_path);
        move || -> SharedString {
            let mut words = vec![title.clone(), agent.clone()];
            words.extend(host.clone());
            let state_now = match state {
                AgentState::Idle | AgentState::Done => state_word(state),
                _ => format!("{} {}", state_word(state), spoken(since, now.get())),
            };
            words.push(state_now);
            words.extend(path.clone());
            words.join(", ").into()
        }
    };
    let ctx = ctx.clone();
    let id = format!("home-pane-{connection}-{pane}");
    r.label_with(said)
        .chevron()
        .on_press(move || {
            ctx.ask(Intent::Pane {
                connection: connection.clone(),
                pane: pane.clone(),
            })
        })
        .test_id(&id)
}

/// A waiting agent in NEEDS YOU: its host in the middle of its subtitle
/// (`claude · gpu-01 · ~/src/trainer`).
pub(crate) fn waiting_row(waiting: WaitingRow, user: String, ctx: &Ctx) -> Row {
    let WaitingRow {
        row: agent_row,
        host,
    } = waiting;
    agent(agent_row, user, Some(host), ctx)
}

/// A row of a live host's card (R5's [`HostRow`]): an agent, the working
/// agents past the first six, the idle group, another pane, all sessions.
pub(crate) fn host_row(host_row: HostRow, connection: &Connection, ctx: &Ctx) -> Row {
    let id = connection.id.clone();
    let user = connection.user.clone();
    match host_row {
        HostRow::Agent(a) => agent(a, user, None, ctx),
        HostRow::MoreWorking(n) => row(crate::plural!("%lld more working", n as i64))
            .leading(agent_mark(Status::Working))
            .chevron()
            .on_press(ctx.asking(Intent::Sessions(id.clone())))
            .test_id(&format!("home-more-working-{id}")),
        HostRow::Idle {
            count,
            titles,
            more,
            expanded,
        } => {
            let mut names = titles.join(", ");
            if more > 0 {
                names.push_str(&format!(" +{more}"));
            }
            let ctx = ctx.clone();
            let toggle = id.clone();
            row(crate::plural!("%lld idle", count as i64))
                .leading(icon("square.stack").tone(Tone::Muted))
                .subtitle(names)
                .disclosure(move || expanded)
                .on_press(move || ctx.toggle_idle(&toggle))
                .test_id(&format!("home-idle-{id}"))
        }
        HostRow::Other(other) => other_pane(other, &id, &user, ctx),
        HostRow::All { mux, count } => row(match mux {
            Mux::Tmux => t!("All sessions"),
            Mux::Herdr => t!("All workspaces"),
        })
        .value(count.to_string())
        .chevron()
        .on_press(ctx.asking(Intent::Sessions(id.clone())))
        .test_id(&format!("home-all-{id}")),
    }
}

/// A pane with no agent: called by its folder, `program · ~/path` under
/// it, the program's symbol before it.
fn other_pane(other: OtherPane, connection: &str, user: &str, ctx: &Ctx) -> Row {
    let OtherPane {
        pane,
        program,
        title,
        path,
    } = other;
    let mut r = row(title).leading(icon(pane_icon(&program)).tone(Tone::Muted));
    r = match path.map(|p| short_path(&p, user)) {
        Some(path) => {
            let program = program.clone();
            r.subtitle_parts(move || program.clone().into(), move || path.clone().into())
        }
        None => r.subtitle(program),
    };
    let connection = connection.to_owned();
    let id = format!("home-pane-{connection}-{pane}");
    r.chevron()
        .on_press(ctx.asking(Intent::Pane { connection, pane }))
        .test_id(&id)
}

/// SAVED's card: a row per host not connected (its mark, its name, `user@host`,
/// a chevron), connecting it on a tap; Edit and Delete in its swipe tray
/// and the host's long press (`home-head-<connection>`).
pub(crate) fn saved_card(hosts: Vec<Connection>, ctx: &Ctx) -> AnyElement {
    let mut card = section();
    let creatures = ctx.creatures.get_untracked();
    for host in hosts {
        let id = host.id.clone();
        let face = match &host.creature {
            Some(creature) if creatures => Some(creature.clone()),
            _ => host.face.clone(),
        };
        let saved = row(host.name.clone())
            .leading(
                host_mark(host.initial.clone())
                    .face(face)
                    .emoji(host.emoji.clone())
                    .size(MarkSize::Sm),
            )
            .subtitle(format!("{}@{}", host.user, host.host))
            .mono_subtitle()
            .chevron()
            .on_press(ctx.asking(Intent::Connect(id.clone())))
            .test_id(&format!("connection-card-{}", host.name));
        let tray = vec![
            Action {
                title: t!("Edit…"),
                symbol: "pencil",
                tint: Tint::Accent,
                intent: Intent::Edit(id.clone()),
            },
            Action {
                title: t!("Delete…"),
                symbol: "trash",
                tint: Tint::Danger,
                intent: Intent::Delete(id.clone()),
            },
        ];
        let element = view()
            .w_full()
            .test_id(&format!("home-head-{id}"))
            .child(swipe::tray_row(
                format!("saved:{id}"),
                ctx.open,
                tray,
                ctx.clone(),
                saved.into_element(),
            ));
        card = card.item(element, Inset::Lead);
    }
    card.into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_under_the_login_reads_from_home() {
        assert_eq!(short_path("/home/dev/src/api", "dev"), "~/src/api");
        assert_eq!(short_path("/home/dev", "dev"), "~");
        assert_eq!(short_path("/Users/me/code/x", "me"), "~/code/x");
        assert_eq!(short_path("/root/work", "root"), "~/work");
        assert_eq!(short_path("/home/devops/x", "dev"), "/home/devops/x");
        assert_eq!(short_path("/srv/app", "dev"), "/srv/app");
    }
}
