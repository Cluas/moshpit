//! A host's card on Home (`docs/nsh/UI-DESIGN.md` §6.1 structure 3): its
//! head over what its session has.
//!
//! The head: the host's mark (its state on the corner) · its name and
//! transport · `user@host · 3h` (a "1 waiting" badge at the end while one
//! of its agents waits) · ⋯ (Open, Edit…, Disconnect, Delete…). A tap on
//! the head opens the terminal where it was, or reconnects a host whose
//! session is gone. The body is R5's [`card_body`]: the agents by state
//! and the rest, or one row saying the host connects, is lost, failed, or
//! stalled, or herdr's version-skew banner.
//!
//! The head stays as it is while the host changes (its words follow); the
//! body is made again when its rows change, the layout animating (§6.1
//! motion), with a success tap when the host comes up.

use std::cell::Cell;

use nsh_core::{EllipsizeMode, prelude::*};
use serde_json::json;

use super::{
    Ctx, Intent, agent_word, animate_rows,
    model::{
        CardBody, Connection, FailureReason, Link, Mux, Unreachable, UnreachableReason, card_body,
    },
    rows,
    swipe::{self, Action, Tint},
};
use crate::ui::{
    feedback::{SpinnerSize, spinner},
    icon::{IconSize, Tone, icon},
    mark::host_mark,
    motion::Duration,
    notice::{BannerTone, banner},
    row::{PADDING_X, RowTone, row},
    section::{Inset, SectionVariant, section},
    status::{BadgeVariant, Status, badge},
    tokens::token,
    typography::TextToken,
};

/// The head's least height: a 40 pt mark and 12 pt above and below.
const HEAD_HEIGHT: f32 = 68.0;
/// ⋯'s square: the touch target.
const MORE: f32 = 44.0;

/// Connection `connection`'s card.
pub(crate) fn card(connection: ReadSignal<Connection>, ctx: Ctx) -> AnyElement {
    let (id, name) = connection.with_untracked(|c| (c.id.clone(), c.name.clone()));
    let body = {
        let (ctx, id) = (ctx.clone(), id.clone());
        computed(move || {
            let open = ctx.idle_open(&id);
            connection.with(|c| card_body(c, open, &agent_word()))
        })
    };
    let first = Cell::new(true);
    let was_connecting = Cell::new(false);
    let shown = {
        let ctx = ctx.clone();
        dynamic(move || {
            let current = body.get();
            let connecting = matches!(current, Some(CardBody::Connecting { .. }));
            if !first.replace(false) {
                animate_rows();
                // The host came up: its rows arrive with a success tap.
                if was_connecting.get()
                    && matches!(current, Some(CardBody::Rows(_) | CardBody::OpenTerminal))
                {
                    haptic(Haptic::Notification(HapticNotification::Success));
                }
            }
            was_connecting.set(connecting);
            let connection = connection.get_untracked();
            let ctx = ctx.clone();
            untracked(move || body_view(current, &connection, &ctx))
        })
    };
    section()
        .variant(SectionVariant::Card)
        .head(head(connection, ctx))
        .item(shown, Inset::Content)
        .test_id(&format!("home-card-{name}"))
        .into_element()
}

/// The dot a connection's state shows on its mark.
fn mark_status(connection: &Connection) -> Option<Status> {
    match connection.link {
        Link::Saved => None,
        Link::Live => Some(Status::Live),
        Link::Connecting | Link::Reconnecting | Link::Attaching => Some(Status::Connecting),
        Link::Stalled => Some(Status::Error),
        Link::Dead if connection.failure.is_some() => Some(Status::Error),
        Link::Dead => Some(Status::Offline),
    }
}

/// The state's word, for screen readers on the head.
fn state_word(link: Link) -> String {
    match link {
        Link::Live | Link::Stalled => t!("Online"),
        Link::Connecting | Link::Reconnecting | Link::Attaching => t!("Connecting…"),
        Link::Dead | Link::Saved => t!("Offline"),
    }
}

/// How long the session has been up: minutes under an hour, then hours
/// ("25m", "3h").
fn uptime(connected_at: f64, now: f64) -> String {
    let minutes = (((now - connected_at) / 60.0) as i64).max(1);
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h", minutes / 60)
    }
}

/// The transport's badge: `SSH` or `mosh`. mosh that fell back to SSH
/// (the host has no mosh-server) runs over SSH and doesn't roam: `SSH`
/// (B519), in the warning's colours ([`fell_back`]).
fn transport(connection: &Connection) -> &'static str {
    if connection.mosh && !connection.degraded {
        "mosh"
    } else {
        "SSH"
    }
}

/// The connection asked for mosh and got SSH.
fn fell_back(connection: &Connection) -> bool {
    connection.mosh && connection.degraded
}

/// The head: the target (mark, name, transport, the line under it), ⋯,
/// the swipe tray (Edit, Disconnect, Delete), and the identifier the
/// host's long press finds (`home-head-<connection>`).
fn head(connection: ReadSignal<Connection>, ctx: Ctx) -> AnyElement {
    let id = connection.with_untracked(|c| c.id.clone());
    // B512: the name and the mark follow the connection, so an edit or a
    // key trusted since shows at once.
    let name = computed(move || connection.with(|c| c.name.clone()));
    // The identifiers keep the name the card was made with.
    let named = name.get_untracked();
    let looks = computed(move || {
        connection.with(|c| {
            (
                c.initial.clone(),
                c.face.clone(),
                c.creature.clone(),
                c.emoji.clone(),
            )
        })
    });
    // With the pixel companion on, the mark is the key's creature.
    let creatures = ctx.creatures;
    let now = ctx.now;
    let link = computed(move || connection.with(|c| c.link));
    let waiting = computed(move || connection.with(Connection::waiting));
    let status = computed(move || connection.with(mark_status));
    let address = computed(move || connection.with(|c| format!("{}@{}", c.user, c.host)));
    let up = computed(move || {
        connection.with(|c| {
            (c.link == Link::Live)
                .then_some(c.connected_at)
                .flatten()
                .map(|at| uptime(at, now.get()))
        })
    });
    let mark = dynamic(move || {
        let status = status.get();
        let (initial, face, creature, emoji) = looks.get();
        // With Pixel Companion on, the key's creature stands for its face.
        let face = match creature {
            Some(creature) if creatures.get() => Some(creature),
            _ => face,
        };
        untracked(move || {
            let mark = host_mark(initial).face(face).emoji(emoji);
            match status {
                Some(status) => mark.status(status),
                None => mark,
            }
        })
    });
    let name_line = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .min_w(px(0.0))
        .child(
            TextToken::Headline
                .apply(text(move || name.get()))
                .number_of_lines(1u32)
                .flex_shrink(1.0)
                .text_color(move || token("foreground")),
        )
        .child(dynamic(move || {
            let (words, fell_back) = connection.with(|c| (transport(c), fell_back(c)));
            untracked(move || {
                let badge = badge(words).mono();
                if fell_back {
                    badge.variant(BadgeVariant::Warning)
                } else {
                    badge
                }
            })
        }));
    let second_line = view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .min_w(px(0.0))
        .child(
            TextToken::MonoSubhead
                .apply(text(move || address.get()))
                .number_of_lines(1u32)
                .text_overflow(EllipsizeMode::Middle)
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .text_color(move || token("muted-foreground")),
        )
        .child(when(
            move || waiting.get() == 0 && up.with(Option::is_some),
            move || {
                TextToken::Subhead
                    .apply(text(move || format!("· {}", up.get().unwrap_or_default())))
                    .number_of_lines(1u32)
                    .flex_shrink_0()
                    .tabular_nums()
                    .text_color(move || token("muted-foreground"))
            },
        ))
        .child(dynamic(move || {
            let n = waiting.get();
            untracked(move || {
                (n > 0).then(|| {
                    badge(crate::plural!("%lld waiting", n as i64))
                        .variant(BadgeVariant::Warning)
                        .into_element()
                })
            })
        }));
    let label = {
        move || -> SharedString {
            let state = if connection.with(|c| c.disconnecting) {
                t!("Disconnecting")
            } else {
                state_word(link.get())
            };
            let mut words = vec![
                name.get(),
                connection.with(transport).to_owned(),
                state,
                address.get(),
            ];
            let n = waiting.get();
            if n > 0 {
                words.push(crate::plural!("%lld waiting", n as i64));
            }
            words.join(", ").into()
        }
    };
    let tap = {
        let (ctx, id) = (ctx.clone(), id.clone());
        move || match link.get_untracked() {
            // Its teardown is under way: nothing to enter or connect until
            // it is over (B535); the card says so.
            _ if connection.with_untracked(|c| c.disconnecting) => {}
            Link::Dead => ctx.ask(Intent::Connect(id.clone())),
            Link::Saved => ctx.ask(Intent::Connect(id.clone())),
            _ => ctx.ask(Intent::Enter(id.clone())),
        }
    };
    let target = view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .min_h(px(HEAD_HEIGHT))
        .pl(px(PADDING_X))
        .py(px(12.0))
        .role(AccessibilityRole::Button)
        .label(label)
        .on_press(tap)
        .transition_colors()
        .duration(Duration::Instant.ms())
        .active(|s| s.bg(move || token("accent")))
        .focus_visible(|s| s.bg(move || token("accent")))
        .test_id(&format!("connection-card-{named}"))
        .child(mark)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(name_line)
                .child(second_line),
        );
    let row = view()
        .w_full()
        .flex_row()
        .items_center()
        .pr(px(6.0))
        .child(target)
        .child(more(connection, &named, ctx.clone()));
    let tray = vec![
        Action {
            title: t!("Edit…"),
            symbol: "pencil",
            tint: Tint::Accent,
            intent: Intent::Edit(id.clone()),
            row: None,
        },
        Action {
            title: t!("Disconnect"),
            symbol: "bolt.slash",
            tint: Tint::Warn,
            intent: Intent::Disconnect(id.clone()),
            row: None,
        },
        Action {
            title: t!("Delete…"),
            symbol: "trash",
            tint: Tint::Danger,
            intent: Intent::Delete(id.clone()),
            row: None,
        },
    ];
    view()
        .w_full()
        .test_id(&format!("home-head-{id}"))
        .child(swipe::tray_row(
            format!("head:{id}"),
            ctx.open,
            tray,
            ctx.teller(),
            row.into_element(),
        ))
        .into_element()
}

/// ⋯ (§6.1 decision 4: the head's only second button): the host's menu
/// (`moshpit/action-menu`) of Open (Connect for a host whose session is
/// gone), Edit…, Disconnect, and Delete… apart.
fn more(connection: ReadSignal<Connection>, name: &str, ctx: Ctx) -> AnyElement {
    let name = name.to_owned();
    let menu = dynamic(move || {
        // Its disconnect under way: a spinner in ⋯'s place, and no menu
        // (B535).
        if connection.with(|c| c.disconnecting) {
            return untracked(|| {
                view()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .child(spinner(SpinnerSize::Sm))
                    .test_id("card-disconnecting-spinner")
                    .into_element()
            });
        }
        let dead = connection.with(|c| c.link == Link::Dead);
        let id = connection.with_untracked(|c| c.id.clone());
        let (ctx, name) = (ctx.clone(), name.clone());
        untracked(move || {
            let item = |id: &str, title: String, image: &str, group: u8| {
                json!({
                    "id": id, "title": title, "image": image,
                    "destructive": id == "delete", "group": group,
                })
            };
            let items = vec![
                if dead {
                    item("connect", t!("Connect"), "terminal", 0)
                } else {
                    item("open", t!("Open"), "terminal", 0)
                },
                item("edit", t!("Edit…"), "pencil", 0),
                item("disconnect", t!("Disconnect"), "bolt.slash", 0),
                item("delete", t!("Delete…"), "trash", 1),
            ];
            let menu = native("moshpit/action-menu")
                .absolute()
                .inset_0()
                .prop("items", json!(items))
                .prop("identifier", json!(format!("card-menu-{name}")))
                .prop("label", json!(t!("More")))
                .on_native_event(move |event: &NativeEvent| {
                    let id = id.clone();
                    let intent = match event.data.as_str() {
                        _ if &*event.name != "select" => return,
                        Some("open") => Intent::Enter(id),
                        Some("connect") => Intent::Connect(id),
                        Some("edit") => Intent::Edit(id),
                        Some("disconnect") => Intent::Disconnect(id),
                        Some("delete") => Intent::Delete(id),
                        _ => return,
                    };
                    ctx.ask(intent);
                });
            view()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .child(
                    icon("ellipsis")
                        .size(IconSize::Md)
                        .tone(Tone::Muted)
                        .into_element(),
                )
                .child(menu)
                .into_element()
        })
    });
    view()
        .relative()
        .w(px(MORE))
        .h(px(MORE))
        .flex_shrink_0()
        .child(menu)
        .into_element()
}

/// Why a connection failed, in the screen's words (the closed set of
/// reasons the session layer gives; `Other` says nothing more).
fn reason_words(reason: FailureReason) -> Option<String> {
    Some(match reason {
        FailureReason::Auth => t!("Sign-in failed"),
        FailureReason::HostKey => t!("Host key not confirmed"),
        FailureReason::HostKeyChanged => t!("The host's key changed"),
        FailureReason::Unreachable => t!("Couldn't reach the host"),
        FailureReason::Timeout => t!("Timed out"),
        FailureReason::Proxy => t!("The proxy didn't connect"),
        FailureReason::Other => return None,
    })
}

/// What didn't answer and why, in full (B801, the alert's words from B740):
/// the reason with the host (and the port, for a refusal); where the Local
/// Network switch is, when the permission may be why.
fn unreachable_words(u: &Unreachable) -> String {
    let host = u.host.clone();
    match (u.why, u.local_network) {
        (UnreachableReason::Refused, _) => t!(
            "%@ refused the connection on port %@. Check the port, and that SSH is running there.",
            arg1 = host,
            arg2 = u.port.to_string()
        ),
        (UnreachableReason::NoAddress, true) => t!(
            "Couldn't find %@ on your local network. Check the name, and that Moshpit may use the local network: Settings → Privacy & Security → Local Network.",
            arg1 = host
        ),
        (UnreachableReason::NoAddress, false) => {
            t!("Couldn't find %@. Check the host name.", arg1 = host)
        }
        (_, true) => t!(
            "Couldn't reach %@ on your local network. Check that it's on, and that Moshpit may use the local network: Settings → Privacy & Security → Local Network.",
            arg1 = host
        ),
        (UnreachableReason::TimedOut, false) => t!(
            "%@ didn't answer. Check the address, that the host is on, and your VPN if it sits behind one.",
            arg1 = host
        ),
        (UnreachableReason::NoRoute, false) => t!(
            "There's no route to %@ from this network. Check your connection, and your VPN if the host sits behind one.",
            arg1 = host
        ),
        (UnreachableReason::Other, false) => {
            t!("Couldn't reach the server. Check the host, port, and that it's online.")
        }
    }
}

/// "Lost 25 minutes ago" (minutes, then hours, then days), "Lost just now"
/// under a minute.
fn lost_words(lost_at: f64, now: f64) -> String {
    let minutes = ((now - lost_at) / 60.0) as i64;
    if minutes < 1 {
        t!("Lost just now")
    } else if minutes < 60 {
        crate::plural!("Lost %lld minutes ago", minutes)
    } else if minutes < 24 * 60 {
        crate::plural!("Lost %lld hours ago", minutes / 60)
    } else {
        crate::plural!("Lost %lld days ago", minutes / (24 * 60))
    }
}

/// What a card shows under its head.
fn body_view(body: Option<CardBody>, connection: &Connection, ctx: &Ctx) -> AnyElement {
    let id = connection.id.clone();
    let name = connection.name.clone();
    let one = |r: crate::ui::row::Row| {
        section()
            .variant(SectionVariant::Plain)
            .row(r)
            .into_element()
    };
    match body {
        None => view().into_element(),
        Some(CardBody::Rows(host_rows)) => section()
            .variant(SectionVariant::Plain)
            .rows(
                host_rows
                    .into_iter()
                    .map(|r| rows::host_row(r, connection, ctx)),
            )
            .into_element(),
        Some(CardBody::Connecting { mux }) => one(row(match mux {
            None => t!("Connecting…"),
            Some(Mux::Tmux) => t!("Connecting to tmux…"),
            Some(Mux::Herdr) => t!("Connecting to herdr…"),
        })
        .leading(spinner(SpinnerSize::Sm))
        .test_id(&format!("card-connecting-{name}"))),
        Some(CardBody::Disconnecting) => one(row(t!("Disconnecting…"))
            .leading(spinner(SpinnerSize::Sm))
            .test_id(&format!("card-disconnecting-{name}"))),
        Some(CardBody::Reconnect {
            lost_at,
            reason,
            unreachable,
            leg,
        }) => {
            // When it was lost, and why the last try failed (its reason in
            // the screen's words: the failure's detail is the session
            // layer's log line, never shown). A host that didn't answer is
            // said in full, on as many lines as it takes (B801).
            let now = ctx.now;
            let full = unreachable.is_some();
            let why = match &unreachable {
                Some(u) => Some(unreachable_words(u)),
                None => reason.and_then(reason_words),
            };
            // A failure on a jump host's leg says which first (F1).
            let why = match (why, leg) {
                (Some(why), Some(leg)) => Some(t!("Jump host “%@”: %@", arg1 = leg, arg2 = why)),
                (why, _) => why,
            };
            let mut r = row(t!("Reconnect"))
                .tone(RowTone::Primary)
                .leading(icon("arrow.clockwise").tone(Tone::Primary));
            r = match (lost_at, why) {
                (Some(at), why) => r.subtitle_with(move || {
                    let lost = lost_words(at, now.get());
                    match &why {
                        Some(why) => format!("{lost} · {why}").into(),
                        None => lost.into(),
                    }
                }),
                (None, Some(why)) => r.subtitle(why),
                (None, None) => r,
            };
            let r = if full { r.wrap_subtitle() } else { r };
            one(r
                .on_press(ctx.asking(Intent::Connect(id)))
                .test_id(&format!("card-dead-{name}")))
        }
        Some(CardBody::EditConnection) => {
            let mut r = row(t!("Edit Connection"))
                .tone(RowTone::Primary)
                .leading(icon("key").tone(Tone::Primary));
            if let Some(why) = reason_words(FailureReason::Auth) {
                r = r.subtitle(why);
            }
            one(r
                .on_press(ctx.asking(Intent::Edit(id)))
                .test_id(&format!("card-edit-{name}")))
        }
        Some(CardBody::RetryAttach) => one(row(t!("Retry connecting tmux"))
            .tone(RowTone::Primary)
            .leading(icon("arrow.clockwise").tone(Tone::Primary))
            .subtitle(t!("tmux isn't answering"))
            .on_press(ctx.asking(Intent::RetryAttach(id)))
            .test_id(&format!("card-attach-stalled-retry-{name}"))),
        Some(CardBody::Mismatch(mismatch)) => {
            let notice = mismatch.notice.clone();
            let mut b = banner(BannerTone::Warning, move || notice.clone().into())
                .icon("exclamationmark.triangle")
                .test_id(&format!("card-mismatch-{name}"));
            if mismatch.restart && !mismatch.restarting {
                b = b.action(t!("Restart server"), ctx.asking(Intent::RestartServer(id)));
            }
            view().w_full().p(px(12.0)).child(b).into_element()
        }
        Some(CardBody::OpenTerminal) => one(row(t!("Open terminal"))
            .leading(icon("terminal").tone(Tone::Muted))
            .chevron()
            .on_press(ctx.asking(Intent::Enter(id)))
            .test_id(&format!("card-open-terminal-{name}"))),
    }
}
