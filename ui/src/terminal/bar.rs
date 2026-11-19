//! The terminal's top bar (`TerminalScreen.topBar`, UI-DESIGN §6.3): the
//! leading button, the transport capsule, the breadcrumb.
//!
//! A system navigation bar's 44 pt (40 in iPad landscape) on the `chrome`
//! material, from the top of the screen down to its edge, a `border`
//! hairline under it. Its three pieces are one kind of capsule
//! ([`capsule`]: 36 pt tall, `chrome` under a `border-strong` hairline),
//! 8 pt apart on one center line, each as wide as what it shows: back (or
//! the iPad's sidebar toggle) a circle, an [`IconButton`]; the transport
//! capsule a status dot and `mosh` or `SSH`; the breadcrumb, hugging its
//! segments up to the room left (then they shrink), says `session › project › agent●`, named by
//! Home's rules ([`Crumbs::names`]), each segment a button, the agent's
//! segment amber while it waits on you. A segment opens its level
//! (`Intent::Crumb`): the host's quick list of that level (sessions,
//! windows, the window's panes), each ending in its New row. The bar has
//! no ＋ of its own. The colors are tokens: the page is light on a light
//! terminal theme ([`crate::screen::Params::terminal`]).

use std::time::Duration;

use nsh_core::prelude::*;

use super::{
    Crumb, Ctx, Intent, line,
    model::{Crumbs, LinkState, Mux, PillKind, Signal as AgentSignal},
};
use crate::{
    connection_form::JumpRoute,
    face::Design,
    ui::{
        alert::alert_dialog,
        button::{CAPSULE_PRESSED, IconButtonSize, IconVariant, icon_button},
        icon::{IconSize, Tone, icon},
        motion::Duration as Motion,
        status::{Status, status_dot},
        tokens::token,
    },
};

/// The bar's height below the status bar: a system bar's, iPad landscape's
/// trimmed one.
pub(crate) fn height(compact: bool) -> f32 {
    if compact { 40.0 } else { 44.0 }
}

/// The height of the back button, the capsule and the breadcrumb, within
/// the bar's row.
const PIECE: f32 = 36.0;

/// The bar's capsule: [`PIECE`] tall, its words 12 pt in from its ends,
/// `chrome` under a `border-strong` hairline (back's circle, an
/// [`IconVariant::Capsule`] button, is drawn the same).
fn capsule(view: View) -> View {
    view.h(px(PIECE))
        .flex_row()
        .items_center()
        .px(px(12.0))
        .rounded_full()
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("chrome"))
        .border(px(0.5))
        .border_color(move || token("border-strong"))
}

/// The bar.
pub(crate) fn top_bar(ctx: Ctx) -> AnyElement {
    let compact = {
        let ctx = ctx.clone();
        computed(move || ctx.compact())
    };
    view()
        .relative()
        .w_full()
        .flex_shrink_0()
        .test_id("terminal-top-bar")
        // The `chrome` material, from the top of the screen.
        .backdrop_blur(px(24.0))
        .bg(move || token("chrome"))
        .child(
            view().pt_safe_area().child(
                view()
                    .h(move || px(height(compact.get())))
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .child(leading_button(ctx.clone()))
                    .child(pill(ctx.clone()))
                    .child(breadcrumb(ctx.clone())),
            ),
        )
        // The hairline at the bar's bottom, over it.
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .h(px(0.5))
                .bg(move || token("border"))
                .pointer_events(PointerEvents::None),
        )
        .into_element()
}

/// A view that tells the host where it is.
trait Anchored {
    fn anchored(self, ctx: Ctx, name: &'static str, target: ElementRef) -> Self;
}

impl Anchored for View {
    fn anchored(self, ctx: Ctx, name: &'static str, target: ElementRef) -> View {
        anchored(self, ctx, name, target)
    }
}

/// `view` (carrying `target`), telling the host where it is (in the
/// surface's points) when it moves: what the host hangs a popover from.
/// The pill's place is kept for its own popover too.
fn anchored(view: View, ctx: Ctx, name: &'static str, target: ElementRef) -> View {
    let measure = move || {
        let ctx = ctx.clone();
        drop(spawn(async move {
            if let Some(m) = target.measure().await {
                if name == "pill" {
                    let bounds = Bounds {
                        origin: point(m.page.x, m.page.y),
                        size: m.frame.size,
                    };
                    if ctx.pill_rect.get_untracked() != Some(bounds) {
                        ctx.pill_rect.set(Some(bounds));
                    }
                }
                ctx.ask(Intent::Anchor {
                    name,
                    rect: [
                        m.page.x.0,
                        m.page.y.0,
                        m.frame.size.width.0,
                        m.frame.size.height.0,
                    ],
                });
            }
        }));
    };
    // The bar moves without the piece's own frame changing (the status bar
    // on a rotation): measured again then too.
    let again = measure.clone();
    effect(move || {
        let _ = (safe_area(), window());
        again();
    });
    view.element_ref(&target)
        .on_layout(move |_: &Bounds<Points>| measure())
}

/// Back, or the iPad's sidebar toggle: a circle of the capsules' height
/// ([`PIECE`]) on their surface, its symbol their words' size, its target
/// the system's 44 pt (the button's own slop).
fn leading_button(ctx: Ctx) -> AnyElement {
    let sidebar = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.sidebar))
    };
    dynamic(move || {
        let sidebar = sidebar.get();
        let ctx = ctx.clone();
        untracked(move || {
            let (symbol, label, id, intent) = if sidebar {
                (
                    "sidebar.leading",
                    t!("Toggle Sidebar"),
                    "terminal-sidebar",
                    Intent::Sidebar,
                )
            } else {
                ("chevron.left", t!("Back"), "terminal-back", Intent::Back)
            };
            icon_button(symbol, label)
                .variant(IconVariant::Capsule)
                .size(IconButtonSize::Chrome)
                .test_id(id)
                .on_press(ctx.asking(intent))
                .into_element()
        })
    })
    .into_element()
}

// --- The transport capsule -------------------------------------------------

/// What the capsule says: the transport.
fn pill_word(kind: PillKind) -> &'static str {
    match kind {
        PillKind::Mosh | PillKind::MoshRoaming => "mosh",
        PillKind::Ssh => "SSH",
    }
}

/// The capsule's dot: the link's state, amber while mosh roams.
fn pill_status(kind: PillKind, state: LinkState) -> Status {
    match state {
        LinkState::Connecting | LinkState::Reconnecting => Status::Connecting,
        LinkState::Offline => Status::Offline,
        LinkState::Live if kind == PillKind::MoshRoaming => Status::Attention,
        LinkState::Live => Status::Live,
    }
}

/// How long a dropped state must stand before the capsule's word says so:
/// a reconnect that lasts under a second never shoves the bar.
const LABEL_DEBOUNCE: Duration = Duration::from_millis(800);

/// The transport capsule (`TransportPill`, StatusCapsule): the dot follows
/// the link at once; the word changes only for `offline`, and only once it
/// has stood for a beat. A press asks to switch SSH ⇄ mosh; held, mosh's
/// live counters.
fn pill(ctx: Ctx) -> AnyElement {
    let kind = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.pill))
    };
    let state = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.state))
    };
    // The state the word shows, lagging a drop by LABEL_DEBOUNCE.
    let shown = signal(LinkState::Live);
    let pending = stored(None::<Spawned<()>>);
    effect(move || {
        let state = state.get();
        if let Some(timer) = pending.replace(None) {
            timer.cancel();
        }
        match state {
            LinkState::Live | LinkState::Connecting => shown.set(state),
            LinkState::Reconnecting | LinkState::Offline => {
                pending.replace(Some(set_timeout(LABEL_DEBOUNCE, move || shown.set(state))));
            }
        }
    });
    let word = computed(move || match shown.get() {
        LinkState::Offline => t!("offline"),
        _ => pill_word(kind.get()).to_owned(),
    });
    let mosh = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.mosh))
    };
    let dot = dynamic(move || {
        let status = pill_status(kind.get(), state.get());
        untracked(move || {
            // The spinner takes 16 pt; a dot 8: one slot for both.
            view()
                .w(px(16.0))
                .h(px(16.0))
                .items_center()
                .justify_center()
                .child(status_dot(status))
                .into_element()
        })
    });
    capsule(view())
        .flex_shrink_0()
        .hit_slop(EdgeInsets {
            top: Points((TARGET - PIECE) / 2.0),
            right: Points(0.0),
            bottom: Points((TARGET - PIECE) / 2.0),
            left: Points(0.0),
        })
        .gap(px(8.0))
        .role(AccessibilityRole::Button)
        .label(move || word.get())
        .hint(t!("Double tap to switch between SSH and Mosh"))
        .test_id("terminal-transport-switch")
        .anchored(ctx.clone(), "pill", ctx.pill)
        .active(|s| s.opacity(CAPSULE_PRESSED))
        .on_press({
            let ctx = ctx.clone();
            move || confirm_switch(&ctx)
        })
        .on_long_press({
            let ctx = ctx.clone();
            move || {
                if mosh.get_untracked() {
                    ctx.ask(Intent::Diagnostics)
                }
            }
        })
        .child(dot)
        .child(
            line(
                move || word.get(),
                Design::Monospaced,
                13.0,
                FontWeight::NORMAL,
            )
            .text_color(move || token("foreground"))
            .flex_shrink_0(),
        )
        .into_element()
}

/// Through a jump host over SSH, mosh asks how its UDP gets to the host:
/// straight there, or forwarded by the jump host. The pick is saved as the
/// connection's route and the session reconnects over mosh; the form's
/// MOSH section has the same choice (and the forwarding rule to copy).
///
/// `answered` runs once the sheet is answered (the jump-host notice goes
/// then: gone sooner, it would take the pending answer with it).
pub(crate) fn pick_udp_route(ctx: &Ctx, answered: impl FnOnce() + 'static) {
    ctx.ask(Intent::DialogAnchor("pill"));
    let answer = alert_dialog()
        .title(t!("Use mosh over UDP?"))
        .description(t!(
            "Pick how mosh's UDP gets to this host. You can change it later in the connection's settings, under MOSH."
        ))
        .action_sheet()
        .action(JumpRoute::Direct.id(), JumpRoute::Direct.title())
        .action(JumpRoute::ViaJump.id(), JumpRoute::ViaJump.title())
        .cancel("cancel", t!("Cancel"))
        .show();
    let ctx = ctx.clone();
    drop(spawn(async move {
        let picked = answer
            .await
            .ok()
            .and_then(|r| r.action)
            .and_then(|id| JumpRoute::named(&id));
        if let Some(route) = picked {
            ctx.ask(Intent::JumpRoute(route));
        }
        answered();
    }));
}

/// The banner's "Use SSH" through a jump host, mosh's UDP on a route of
/// its own: this time only (the banner's action, a one-shot reconnect), or
/// from now on (the route goes back to SSH, as the pill's switch does).
pub(crate) fn confirm_ssh_for_now_or_good(ctx: &Ctx) {
    ctx.ask(Intent::DialogAnchor("pill"));
    let answer = alert_dialog()
        .title(t!("Use SSH?"))
        .description(t!(
            "Reconnects this session over SSH, through the jump host."
        ))
        .action_sheet()
        .action("once", t!("This time only"))
        .action("always", t!("From now on"))
        .cancel("cancel", t!("Cancel"))
        .show();
    let ctx = ctx.clone();
    drop(spawn(async move {
        match answer.await.ok().and_then(|r| r.action).as_deref() {
            Some("once") => ctx.ask(Intent::BannerAction),
            Some("always") => ctx.ask(Intent::Transport),
            _ => {}
        }
    }));
}

/// The pill's question (`confirmationDialog`): switch to the other
/// transport? A tap alone never reconnects. On the iPad the host hangs the
/// sheet's popover from the pill.
pub(crate) fn confirm_switch(ctx: &Ctx) {
    // Behind a jump host there is nothing to switch to: say why, and leave
    // the saved protocol alone.
    if ctx.chrome.with_untracked(|c| c.jump) {
        pick_udp_route(ctx, || {});
        return;
    }
    let target = if ctx.chrome.with_untracked(|c| c.mosh) {
        "SSH"
    } else {
        "Mosh"
    };
    ctx.ask(Intent::DialogAnchor("pill"));
    let answer = alert_dialog()
        .title(t!("Switch to %@?", arg1 = target))
        .description(t!("Reconnects this session over %@.", arg1 = target))
        .action_sheet()
        .action("switch", t!("Switch"))
        .cancel("cancel", t!("Cancel"))
        .show();
    let ctx = ctx.clone();
    drop(spawn(async move {
        if answer
            .await
            .is_ok_and(|r| r.action.as_deref() == Some("switch"))
        {
            ctx.ask(Intent::Transport);
        }
    }));
}

// --- The breadcrumb ------------------------------------------------------------

/// The breadcrumb's capsule: `session › project › agent●`, or the
/// connection's identity while there is no tree. The remembered plan of a
/// connection coming back is dimmed and inert.
///
/// The capsule is as wide as its segments, not the bar's rest: a capsule
/// stretched past its last word read as an empty field. It sits in a slot
/// that takes the room left; when the segments need more than that, the
/// capsule stops at the slot's end and they shrink.
fn breadcrumb(ctx: Ctx) -> AnyElement {
    let crumbs = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.crumbs.clone()))
    };
    let retained = computed(move || crumbs.with(|c| c.as_ref().is_some_and(|c| !c.live)));
    // The session's name first, its icon alone only when the capsule with
    // the three full segments is wider than the slot. Both are measured
    // outside the capsule shown, so a squeezed (narrower) capsule never
    // decides it stays squeezed.
    let room = signal(0.0_f32);
    let natural = signal(0.0_f32);
    let squeezed = computed(move || natural.get() > room.get() + 0.5);
    let still = {
        let ctx = ctx.clone();
        computed(move || ctx.still())
    };
    // The capsule with the full segments, unseen, unbounded (the measuring
    // copy: the same boxes, no buttons).
    let measured = dynamic(move || {
        let crumbs = crumbs.get();
        untracked(move || match crumbs {
            Some(plan) => {
                let names = plan.names(&t!("agent"));
                capsule(view())
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .opacity(0.0)
                    .pointer_events(PointerEvents::None)
                    .accessibility_elements_hidden(true)
                    .on_layout(move |b: &Bounds<Points>| {
                        if (natural.get_untracked() - b.size.width.0).abs() > 0.25 {
                            natural.set(b.size.width.0);
                        }
                    })
                    .child(row(&plan, &names, true, None))
                    .into_element()
            }
            None => view().into_element(),
        })
    });
    let content = {
        let ctx = ctx.clone();
        dynamic(move || {
            let crumbs = crumbs.get();
            // Without a tree the capsule names the connection: its words
            // are read here, so a change redraws them.
            if crumbs.is_none() {
                ctx.chrome.with(|c| (c.user.len(), c.host.len(), c.mux));
            }
            let ctx = ctx.clone();
            untracked(move || match crumbs {
                Some(plan) => {
                    let names = plan.names(&t!("agent"));
                    dynamic(move || {
                        let squeeze = plan.squeezable && squeezed.get();
                        let (plan, names, ctx) = (plan.clone(), names.clone(), ctx.clone());
                        untracked(move || row(&plan, &names, !squeeze, Some(ctx)))
                    })
                    .into_element()
                }
                None => identity(&ctx),
            })
        })
    };
    let shown = capsule(view())
        .flex_shrink(1.0)
        .min_w(px(0.0))
        .relative()
        // No clip: the segments' targets reach past the capsule's top and
        // bottom (a view that clips hands its children no touch outside
        // it); the words truncate inside it on their own.
        .test_id("terminal-breadcrumb")
        .anchored(ctx.clone(), "breadcrumb", element_ref())
        .child(
            view()
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .opacity(move || if retained.get() { 0.45_f32 } else { 1.0 })
                .transition_opacity()
                // At once through a protocol switch (nothing moves then).
                .duration(move || if still.get() { 0.0 } else { Motion::Base.ms() })
                .disabled(move || retained.get())
                .child(content),
        );
    // The room left in the bar, the capsule at its start.
    view()
        .flex_1()
        .min_w(px(0.0))
        .relative()
        .flex_row()
        .items_center()
        .on_layout(move |b: &Bounds<Points>| {
            if (room.get_untracked() - b.size.width.0).abs() > 0.25 {
                room.set(b.size.width.0);
            }
        })
        .child(measured)
        .child(shown)
        .into_element()
}

/// A breadcrumb segment's target, at least: the system's 44 pt square.
const TARGET: f32 = 44.0;
/// The room between a segment and a `›`.
const GAP: f32 = 6.0;

/// A segment's box: its target. It reaches past `content` on each side by
/// the row's gap and half a `›` (`separator`: a `›`'s width, as laid out),
/// to the separator's middle, so a tap between two segments goes to the
/// nearer one (at the row's ends, that far into the capsule's padding);
/// and 4 pt above and below the capsule. Negative margins give the room
/// back, so the words and the separators stay where they were; a segment
/// shorter than the target (a window called `1`) widens just enough for
/// its box to be [`TARGET`] wide. When the capsule is short of room it
/// shrinks by `shrink` (NSH's items don't by default), down to its target,
/// its words truncating.
fn segment_box(content: AnyElement, separator: Signal<f32>, shrink: f32) -> View {
    let reach = move || GAP + separator.get() / 2.0;
    view()
        .flex_shrink(shrink)
        .flex_row()
        .items_center()
        .justify_center()
        .min_w(px(TARGET))
        .h(px(TARGET))
        .px(move || px(reach()))
        .mx(move || px(-reach()))
        .my(px(-(TARGET - PIECE) / 2.0))
        .child(content)
}

/// The segments in a row; `named`: the session segment spells its name
/// (else its icon alone). With `ctx`, the segments are buttons (the
/// measuring copy has none, but the same boxes, so it measures what
/// shows).
fn row(plan: &Crumbs, names: &super::model::Names, named: bool, ctx: Option<Ctx>) -> AnyElement {
    // The remembered plan's segments are inert: the controller they would
    // navigate is being rebuilt.
    let live = plan.live;
    // A `›`'s width, as laid out: how far the targets reach.
    let separator_width = signal(0.0_f32);
    // The session and the window give up room first; the pane, the name
    // that matters most, last. Its factor is 1, not a fraction: a lone
    // item still shrinking with a factor under 1 takes only that fraction
    // of what is short (CSS's rule), and the pane ran past the capsule.
    let button = |crumb: Crumb, label: String, content: AnyElement| -> AnyElement {
        let shrink = if crumb == Crumb::Pane { 1.0 } else { 1000.0 };
        let target = segment_box(content, separator_width, shrink);
        match &ctx {
            Some(ctx) => {
                let target = target
                    .role(AccessibilityRole::Button)
                    .label(label)
                    .test_id(match crumb {
                        Crumb::Session => "terminal-crumb-session",
                        Crumb::Window => "terminal-crumb-window",
                        Crumb::Pane => "terminal-crumb-pane",
                    })
                    .disabled(!live)
                    .active(|s| s.opacity(0.5));
                let ctx = ctx.clone();
                target
                    .on_press(move || {
                        if live {
                            press_segment(&ctx, crumb)
                        }
                    })
                    .into_element()
            }
            None => target.into_element(),
        }
    };
    let mut session_content = view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .min_w(px(0.0))
        .flex_shrink(if named { 1.0 } else { 0.0 })
        .child(
            icon(icon_name(&plan.session_icon))
                .size(IconSize::Sm)
                .tone(Tone::Muted),
        );
    if named {
        session_content = session_content.child(segment_text(names.session.clone(), "foreground"));
    }
    let session = button(
        Crumb::Session,
        names.session.clone(),
        session_content.into_element(),
    );
    // A window told by its number (its name would say the session's again,
    // B650, or it has none): the window's symbol before it, the
    // multiplexer's word for screen readers ("Tab 1").
    let window = if names.project_numbered {
        let word = super::picker::Vocab::of(match plan.tree.as_ref().map(|t| t.mux) {
            Some(crate::home::model::Mux::Herdr) => super::model::Mux::Herdr,
            _ => super::model::Mux::Tmux,
        })
        .window;
        button(
            Crumb::Window,
            format!("{word} {}", names.project),
            view()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .min_w(px(0.0))
                .test_id("terminal-crumb-window-number")
                .child(
                    icon(icon_name(&plan.window_icon))
                        .size(IconSize::Sm)
                        .tone(Tone::Muted),
                )
                .child(segment_text(names.project.clone(), "foreground"))
                .into_element(),
        )
    } else {
        button(
            Crumb::Window,
            names.project.clone(),
            segment_text(names.project.clone(), "foreground").into_element(),
        )
    };
    // As wide as its segments, or as the capsule's room when they need
    // more: then they shrink (the measuring copy, unbounded, never does).
    let mut row = view()
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .child(session)
        .child(separator(Some(separator_width)))
        .child(window);
    if let Some(pane) = &names.pane {
        let label = names.pane_label.clone().unwrap_or_else(|| pane.clone());
        row = row.child(separator(Some(separator_width))).child(button(
            Crumb::Pane,
            label,
            agent_segment(pane, plan.signal),
        ));
    }
    row.into_element()
}

/// A segment's press: the host's quick picker of its level, hung from the
/// breadcrumb: the sessions (herdr: workspaces), the windows (herdr: tabs)
/// or the window's panes, the current one ticked, each list ending in its
/// "＋ New…" row (New pane splits the window).
fn press_segment(ctx: &Ctx, crumb: Crumb) {
    ctx.ask(Intent::Crumb(crumb));
}

/// The multiplexer's symbol, as the `ui` layer's icons take it (a
/// `'static` name: the vocabulary's few).
fn icon_name(name: &str) -> &'static str {
    match name {
        "square.stack.3d.up" => "square.stack.3d.up",
        "macwindow" => "macwindow",
        "rectangle.grid.2x2" => "rectangle.grid.2x2",
        "square.on.square" => "square.on.square",
        _ => "square.stack.3d.up",
    }
}

/// A segment's words: `text-subhead`, one line, truncating.
fn segment_text(words: String, color: &'static str) -> Text {
    line(words, Design::Default, 15.0, FontWeight::NORMAL)
        .truncate()
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .text_color(move || token(color))
}

/// The agent's segment: its name and its state's dot; amber while the
/// agent waits on you (`warning-soft` under `warning-text`), the change
/// a `base` color transition.
fn agent_segment(name: &str, signal: Option<AgentSignal>) -> AnyElement {
    let waiting = signal == Some(AgentSignal::Attention);
    let mut segment = view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .min_w(px(0.0))
        // Narrower than its words when its box is: the name truncates.
        .flex_shrink(1.0)
        .h(px(24.0))
        .px(px(if waiting { 8.0 } else { 0.0 }))
        .rounded_full()
        .border_curve(BorderCurve::Continuous)
        .bg(move || {
            if waiting {
                token("warning-soft")
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .transition_colors()
        .duration(Motion::Base.ms())
        .child(segment_text(
            name.to_owned(),
            if waiting {
                "warning-text"
            } else {
                "foreground"
            },
        ));
    if let Some(signal) = signal {
        segment = segment.child(status_dot(match signal {
            AgentSignal::Attention => Status::Attention,
            AgentSignal::Working => Status::Working,
            AgentSignal::Done => Status::Done,
        }));
    }
    segment.into_element()
}

/// `›` between segments: under the targets of the segments either side
/// of it, which split it at its middle; its width goes to `width`, for
/// the breadcrumb's targets (the connection's identity has none).
fn separator(width: Option<Signal<f32>>) -> AnyElement {
    let mut mark = view()
        .flex_shrink_0()
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true);
    if let Some(width) = width {
        mark = mark.on_layout(move |b: &Bounds<Points>| {
            if (width.get_untracked() - b.size.width.0).abs() > 0.25 {
                width.set(b.size.width.0);
            }
        });
    }
    mark.child(
        line("›", Design::Default, 15.0, FontWeight::NORMAL)
            .text_color(move || token("subtle-foreground")),
    )
    .into_element()
}

/// No tree to show (mosh, plain SSH, herdr before its tree): the
/// connection's identity, and the multiplexer inside it.
fn identity(ctx: &Ctx) -> AnyElement {
    let (user, host, mux) = ctx
        .chrome
        .with_untracked(|c| (c.user.clone(), c.host.clone(), c.mux));
    let mut row = view()
        .flex_shrink(1.0)
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            line(
                format!("{user}@{host}"),
                Design::Monospaced,
                13.0,
                FontWeight::NORMAL,
            )
            .truncate()
            .min_w(px(0.0))
            .flex_shrink(1.0)
            .text_color(move || token("muted-foreground")),
        );
    if mux != Mux::None {
        row = row.child(separator(None)).child(
            line(mux.label(), Design::Monospaced, 13.0, FontWeight::NORMAL)
                .flex_shrink_0()
                .text_color(move || token("foreground")),
        );
    }
    row.into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pill's dot (what `TransportConnState.transientTint` pinned for
    /// the SwiftUI pill, gone in D8): offline, with nothing being attempted,
    /// is its own state and no other borrows it; connecting and
    /// reconnecting are both an attempt; live is the transport's own, amber
    /// only while mosh roams.
    #[test]
    fn the_pills_dot_says_offline_only_when_nothing_is_attempted() {
        for kind in [PillKind::Ssh, PillKind::Mosh, PillKind::MoshRoaming] {
            assert_eq!(pill_status(kind, LinkState::Offline), Status::Offline);
            for attempt in [LinkState::Connecting, LinkState::Reconnecting] {
                assert_eq!(pill_status(kind, attempt), Status::Connecting);
            }
        }
        assert_eq!(pill_status(PillKind::Ssh, LinkState::Live), Status::Live);
        assert_eq!(pill_status(PillKind::Mosh, LinkState::Live), Status::Live);
        assert_eq!(
            pill_status(PillKind::MoshRoaming, LinkState::Live),
            Status::Attention
        );
    }
}
