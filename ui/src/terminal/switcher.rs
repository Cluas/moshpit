//! The sideways swipe between a session's panes, made visible (the pane
//! switching design's step 1, `docs/nsh/ui-mockups/pane-switch/` in the
//! design repo).
//!
//! The swipe walks every pane of the active session, window by window, and
//! stops at both ends: no wrap-around (tmux's `select-pane -t :.+` wraps,
//! which made the position meaningless). The page decides the order from
//! the tree the host posts ([`Switcher`]); the host does the moving: the
//! terminal follows the finger, rubber-bands where there is no neighbour,
//! and on a commit selects the neighbour the page named ([`Intent::
//! Neighbours`]).
//!
//! What the page draws for it:
//!
//! - The backstage ([`backstage`]): under the terminal's slot, the strip a
//!   moving terminal uncovers names the pane it would go to, its window and
//!   its state ("2 / 7 · trainer · claude · Needs you · 3m"). It never
//!   moves: the terminal moving away reveals it, so nothing crosses the
//!   bridge per frame.
//! - The dots ([`dots`]): under the top bar, one group per window, the
//!   current pane a wider pill, a pane that needs you amber. They show on
//!   entry, when the pane changes and while a swipe lasts, then go after
//!   1.2 s. Past [`MAX_DOTS`] panes, a count with the window's name.
//! - The hint ([`hint`]): once on this device ([`Switcher::hint_seen`]),
//!   the first time a session has a pane to swipe to, a toast over the
//!   shortcut bar for 4 s or until the first swipe. The host nudges the
//!   terminal toward the next pane as it shows ([`Intent::SwipeHint`]).
//!
//! A session with one pane shows none of it, and the host gets no
//! neighbour: no swipe, no rubber band hinting at more.

use std::{rc::Rc, time::Duration as Timer};

use nsh_core::prelude::*;

use super::{
    Ctx, Intent,
    cover::chrome_text,
    model::{Content, CoverKind},
};
use crate::{
    home::model::{
        Control, Mux, PaneInfo, Signal as AgentSignal, WindowInfo, elapsed_label, pane_title,
    },
    ui::{
        icon::{IconSize, Tone, icon},
        motion::{Duration, ENTER, LEAVE},
        status::{Status, status_dot},
        tokens::{SHADOW_LG, number, token},
        typography::TextToken,
    },
};

/// Past this many panes the dots become a count.
pub const MAX_DOTS: usize = 10;
/// How long the dots stay after the pane changed or a swipe ended.
const DOTS_LINGER: Timer = Timer::from_millis(1200);
/// How long the hint stays, unless a swipe comes first.
const HINT_FOR: Timer = Timer::from_secs(4);
/// The room between the hint and the shortcut bar.
const ABOVE_BAR: f32 = 8.0;

/// One pane the swipe stops at.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub pane: String,
    pub window: String,
    /// Its window's name by Home's rules (the breadcrumb's project): the
    /// window's own name, else the pane's folder, else the multiplexer's
    /// word and the window's number. Never tmux's automatic name.
    pub place: String,
    /// What runs in it: the agent, else the program, else "pane N".
    pub name: String,
    pub signal: Option<AgentSignal>,
    /// Since when it is in that state (Unix seconds).
    pub since: Option<f64>,
}

/// The words the order and its labels are said in: the app's language on
/// the page ([`Words::here`]), English in a test without an engine.
#[derive(Clone)]
pub struct Words {
    /// The multiplexer's word for a window: tmux's ("Window"), herdr's
    /// ("Tab").
    pub window: String,
    pub tab: String,
    /// What to call an agent no stamp names.
    pub agent: String,
    /// "pane 2".
    pub pane: Rc<dyn Fn(i64) -> String>,
    pub needs_you: String,
    pub working: String,
    pub finished: String,
    /// Under a minute.
    pub now: String,
}

impl Words {
    /// In the app's language.
    pub fn here() -> Words {
        Words {
            window: t!("Window"),
            tab: t!("Tab"),
            agent: t!("agent"),
            pane: Rc::new(|index| t!("pane %lld", arg1 = index.to_string())),
            needs_you: t!("Needs you"),
            working: t!("Working"),
            finished: t!("Finished"),
            now: t!("now"),
        }
    }
}

/// The active session's panes in the swipe's order, and which one shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Order {
    pub stops: Vec<Stop>,
    pub current: Option<usize>,
}

impl Order {
    /// `tree`'s active session: its windows by index, each window's panes
    /// by index.
    pub fn of(tree: &Control, words: &Words) -> Order {
        let Some(session) = tree.active_session.as_deref() else {
            return Order::default();
        };
        let stops: Vec<Stop> = tree
            .windows_in(session)
            .into_iter()
            .flat_map(|window| {
                tree.panes_in(&window.id)
                    .into_iter()
                    .map(move |pane| (window, pane))
            })
            .map(|(window, pane)| stop(tree, window, pane, words))
            .collect();
        let current = tree
            .active_pane
            .as_deref()
            .and_then(|id| stops.iter().position(|s| s.pane == id));
        Order { stops, current }
    }

    /// The pane before the one that shows; none at the first.
    pub fn previous(&self) -> Option<&Stop> {
        let at = self.current?;
        at.checked_sub(1).and_then(|i| self.stops.get(i))
    }

    /// The pane after the one that shows; none at the last.
    pub fn next(&self) -> Option<&Stop> {
        self.stops.get(self.current? + 1)
    }

    /// There is somewhere to swipe to.
    pub fn swipeable(&self) -> bool {
        self.previous().is_some() || self.next().is_some()
    }

    /// The pane that shows.
    pub fn shown(&self) -> Option<&Stop> {
        self.stops.get(self.current?)
    }

    /// The dots: a group per window, or a count past [`MAX_DOTS`].
    pub fn dots(&self) -> Dots {
        let Some(at) = self.current else {
            return Dots::Groups(Vec::new());
        };
        if self.stops.len() > MAX_DOTS {
            return Dots::Count {
                place: self.stops[at].place.clone(),
                at: at + 1,
                of: self.stops.len(),
            };
        }
        let mut groups: Vec<Vec<Dot>> = Vec::new();
        let mut window: Option<&str> = None;
        for (i, stop) in self.stops.iter().enumerate() {
            if window != Some(stop.window.as_str()) {
                groups.push(Vec::new());
                window = Some(stop.window.as_str());
            }
            let dot = if i == at {
                Dot::Current
            } else if stop.signal == Some(AgentSignal::Attention) {
                Dot::Attention
            } else {
                Dot::Other
            };
            groups.last_mut().expect("a group was pushed").push(dot);
        }
        Dots::Groups(groups)
    }
}

/// One dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Current,
    /// An agent there waits on you.
    Attention,
    Other,
}

/// What the position indicator shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dots {
    /// One group per window, in order.
    Groups(Vec<Vec<Dot>>),
    /// "search · 12 / 19".
    Count { place: String, at: usize, of: usize },
}

pub(crate) fn stop(tree: &Control, window: &WindowInfo, pane: &PaneInfo, words: &Words) -> Stop {
    let word = match tree.mux {
        Mux::Tmux => &words.window,
        Mux::Herdr => &words.tab,
    };
    // Never the session's name again (B650): every tab of a herdr
    // workspace named after its folder would say it.
    let session = tree
        .sessions
        .iter()
        .find(|s| s.id == window.session)
        .map_or("", |s| s.name.as_str());
    let place = crate::home::model::beside_session(
        super::model::project_name(&window.name, &pane.command, pane.path.as_deref()),
        session,
    )
    .unwrap_or_else(|| format!("{word} {}", window.index));
    let title = pane_title(tree, pane, &words.agent);
    let name = title
        .agent
        .or_else(|| (!title.program.is_empty()).then_some(title.program))
        .unwrap_or_else(|| (words.pane)(pane.index));
    let hook = tree.hooks.iter().find(|h| h.pane == pane.id);
    let signal = AgentSignal::of(hook.and_then(|h| h.state.as_deref()));
    Stop {
        pane: pane.id.clone(),
        window: window.id.clone(),
        place,
        name,
        signal,
        since: signal.and(hook.and_then(|h| h.since)),
    }
}

/// Whether the one-time hint shows now: never shown on this device, a pane
/// to swipe to, and the pane in view (no cover over it, the terminal in
/// the pane area).
pub fn hint_due(seen: bool, order: &Order, in_view: bool) -> bool {
    !seen && in_view && order.swipeable()
}

/// The panes the host may swipe to, by id: none either side while the pane
/// is not in view (a cover over it, an empty state) or the session has one
/// pane.
pub fn neighbours(order: &Order, in_view: bool) -> (Option<String>, Option<String>) {
    if !in_view {
        return (None, None);
    }
    (
        order.previous().map(|s| s.pane.clone()),
        order.next().map(|s| s.pane.clone()),
    )
}

/// What screen readers hear for the dots: "Pane 2 of 7, trainer".
pub fn position_label(order: &Order) -> String {
    match (order.current, order.shown()) {
        (Some(at), Some(stop)) => t!(
            "Pane %lld of %lld, %@",
            arg1 = (at + 1).to_string(),
            arg2 = order.stops.len().to_string(),
            arg3 = stop.place.clone()
        ),
        _ => String::new(),
    }
}

/// A stop's state in words with how long it has been in it ("Needs you ·
/// 3m"), its status, its tone; none for a pane with no agent signal.
pub fn state_line(stop: &Stop, now: f64, words: &Words) -> Option<(String, Status, &'static str)> {
    let signal = stop.signal?;
    let (word, status, tone) = match signal {
        AgentSignal::Attention => (&words.needs_you, Status::Attention, "warning-text"),
        AgentSignal::Working => (&words.working, Status::Working, "success-text"),
        AgentSignal::Done => (&words.finished, Status::Done, "info-text"),
    };
    let line = match elapsed_label(stop.since, now, &words.now) {
        Some(elapsed) => format!("{word} · {elapsed}"),
        None => word.clone(),
    };
    Some((line, status, tone))
}

// --- On the page ----------------------------------------------------------------

/// The time now, in Unix seconds, on the engine's clock.
pub(crate) fn unix_now() -> f64 {
    nsh_core::time::unix_ms() / 1000.0
}

/// The pane is in view: the terminal shows, no cover over it.
pub(crate) fn in_view(ctx: &Ctx) -> bool {
    ctx.chrome
        .with(|c| c.content == Content::Terminal && c.cover.kind == CoverKind::None)
}

/// The swipe's order, from the tree the host posts.
pub(crate) fn order(ctx: &Ctx) -> Memo<Order> {
    let switcher = ctx.switcher;
    computed(move || {
        switcher.with(|s| {
            s.tree
                .as_ref()
                .map(|tree| Order::of(tree, &Words::here()))
                .unwrap_or_default()
        })
    })
}

/// Tells the host the panes either side whenever they change: what it
/// rubber-bands against and switches to.
pub(crate) fn tell_neighbours(ctx: &Ctx, order: Memo<Order>) {
    let told = stored(None::<(Option<String>, Option<String>)>);
    let ctx = ctx.clone();
    effect(move || {
        let view = in_view(&ctx);
        let now = order.with(|o| neighbours(o, view));
        if told.with(|t| t.as_ref() == Some(&now)) {
            return;
        }
        told.replace(Some(now.clone()));
        let (previous, next) = now;
        ctx.ask(Intent::Neighbours { previous, next });
    });
}

/// `make()` while `visible()`, fading in and out (instant under reduced
/// motion), mounted only while it shows or fades out.
pub(crate) fn appearing(
    visible: Memo<bool>,
    (enter, leave): (Duration, Duration),
    make: impl Fn() -> AnyElement + 'static,
) -> AnyElement {
    let shown = visible.get_untracked();
    let mounted = signal(shown);
    let opacity = animated_value(if shown { 1.0 } else { 0.0 });
    let pending = stored(None::<Spawned<()>>);
    effect(move || {
        let show = visible.get();
        if let Some(timer) = pending.replace(None) {
            timer.cancel();
        }
        if show {
            mounted.set(true);
            let ms = enter.ms();
            if ms > 0.0 {
                drop(opacity.timing(1.0, ms, ENTER));
            } else {
                opacity.set(1.0);
            }
        } else {
            let ms = leave.ms();
            if ms > 0.0 {
                drop(opacity.timing(0.0, ms, LEAVE));
                pending.replace(Some(set_timeout(
                    Timer::from_millis(ms as u64),
                    move || mounted.set(false),
                )));
            } else {
                opacity.set(0.0);
                mounted.set(false);
            }
        }
    });
    view()
        .bind_opacity(&opacity)
        .pointer_events(PointerEvents::BoxNone)
        .child(when(move || mounted.get(), make))
        .into_element()
}

/// Under the terminal's slot: the panes either side, named, revealed as
/// the terminal moves away. Hidden from screen readers: they switch with
/// the terminal's own actions (Next pane, Previous pane).
pub(crate) fn backstage(ctx: Ctx, order: Memo<Order>) -> AnyElement {
    let swiping = ctx.swiping;
    view()
        .absolute()
        .inset_0()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(number("space-page")))
        .bg(move || token("background"))
        .accessibility_elements_hidden(true)
        .pointer_events(PointerEvents::None)
        .test_id("terminal-backstage")
        .child(dynamic(move || {
            // Its minutes read when a swipe starts.
            let _ = swiping.get();
            let (at, of, stop) =
                order.with(|o| (o.current.unwrap_or(0), o.stops.len(), o.previous().cloned()));
            untracked(move || side(stop, at, of, false))
        }))
        .child(dynamic(move || {
            let _ = swiping.get();
            let (at, of, stop) =
                order.with(|o| (o.current.unwrap_or(0) + 2, o.stops.len(), o.next().cloned()));
            untracked(move || side(stop, at, of, true))
        }))
        .into_element()
}

/// One side of the backstage: where the pane it names sits in the order
/// (1-based), its window, what runs in it, its state.
fn side(stop: Option<Stop>, at: usize, of: usize, next: bool) -> AnyElement {
    let Some(stop) = stop else {
        return view().into_element();
    };
    let mut column = view().w(px(140.0)).gap(px(2.0)).test_id(if next {
        "terminal-backstage-next"
    } else {
        "terminal-backstage-previous"
    });
    column = if next {
        column.items_end()
    } else {
        column.items_start()
    };
    let line = move |token_name: &'static str, words: String, style: TextToken| {
        let text = chrome_text(style, words)
            .number_of_lines(1u32)
            .text_color(move || token(token_name));
        if next {
            text.text_right()
        } else {
            text.text_left()
        }
    };
    column = column
        .child(
            line(
                "muted-foreground",
                format!("{at} / {of}"),
                TextToken::Caption,
            )
            .tabular_nums(),
        )
        .child(line("foreground", stop.place.clone(), TextToken::Headline))
        .child(line(
            "muted-foreground",
            stop.name.clone(),
            TextToken::Subhead,
        ));
    if let Some((words, status, tone)) = state_line(&stop, unix_now(), &Words::here()) {
        column = column.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(status_dot(status))
                .child(line(tone, words, TextToken::Footnote)),
        );
    }
    column.into_element()
}

/// The dots under the top bar while the pane changed or a swipe lasts.
pub(crate) fn dots(ctx: Ctx, order: Memo<Order>) -> AnyElement {
    let swiping = ctx.swiping;
    // The pane shown, when there is somewhere to swipe: a change shows the
    // dots (entering the screen is one).
    let position = computed(move || {
        order.with(|o| {
            o.swipeable()
                .then(|| o.shown().map(|s| s.pane.clone()))
                .flatten()
        })
    });
    let lingering = signal(false);
    let timer = stored(None::<Spawned<()>>);
    let linger = move || {
        if let Some(t) = timer.replace(None) {
            t.cancel();
        }
        lingering.set(true);
        timer.replace(Some(set_timeout(DOTS_LINGER, move || lingering.set(false))));
    };
    effect(move || {
        if position.get().is_some() {
            untracked(linger);
        }
    });
    let was_swiping = stored(false);
    effect(move || {
        let now = swiping.get();
        if was_swiping.replace(now) && !now {
            untracked(linger);
        }
    });
    let visible = {
        let ctx = ctx.clone();
        computed(move || {
            position.with(|p| p.is_some()) && in_view(&ctx) && (swiping.get() || lingering.get())
        })
    };
    view()
        .w_full()
        .items_center()
        .pointer_events(PointerEvents::BoxNone)
        .child(appearing(
            visible,
            (Duration::Fast, Duration::Base),
            move || {
                let label = computed(move || order.with(position_label));
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(number("radius-full")))
                    .bg(move || token("chrome"))
                    .border(px(0.5))
                    .border_color(move || token("border-strong"))
                    .backdrop_blur(px(20.0))
                    .label(move || label.get())
                    .test_id("terminal-pane-dots")
                    .child(dynamic(move || {
                        let dots = order.with(Order::dots);
                        untracked(move || dots_row(dots))
                    }))
                    .into_element()
            },
        ))
        .into_element()
}

fn dots_row(dots: Dots) -> AnyElement {
    match dots {
        Dots::Count { place, at, of } => view()
            .flex_row()
            .gap(px(4.0))
            .child(
                chrome_text(TextToken::Caption, place)
                    .number_of_lines(1u32)
                    .text_color(move || token("foreground")),
            )
            .child(
                chrome_text(TextToken::Caption, format!("· {at} / {of}"))
                    .tabular_nums()
                    .text_color(move || token("muted-foreground")),
            )
            .into_element(),
        Dots::Groups(groups) => {
            let mut row = view().flex_row().items_center().gap(px(10.0));
            for group in groups {
                let mut g = view().flex_row().items_center().gap(px(5.0));
                for dot in group {
                    let (width, color) = match dot {
                        Dot::Current => (14.0, "foreground"),
                        Dot::Attention => (6.0, "warning"),
                        Dot::Other => (6.0, "subtle-foreground"),
                    };
                    g = g.child(
                        view()
                            .w(px(width))
                            .h(px(6.0))
                            .rounded(px(number("radius-full")))
                            .bg(move || token(color)),
                    );
                }
                row = row.child(g);
            }
            row.into_element()
        }
    }
}

/// The one-time hint, over the shortcut bar.
pub(crate) fn hint(ctx: Ctx, order: Memo<Order>) -> AnyElement {
    // Shown once on this page, whatever the host says later; gone after
    // `HINT_FOR` or at the first swipe.
    let phase = signal(HintPhase::Waiting);
    let timer = stored(None::<Spawned<()>>);
    {
        let ctx = ctx.clone();
        effect(move || {
            let due = {
                let seen = ctx.switcher.with(|s| s.hint_seen);
                let view = in_view(&ctx);
                order.with(|o| hint_due(seen, o, view))
            };
            if !due || phase.get_untracked() != HintPhase::Waiting {
                return;
            }
            phase.set(HintPhase::Showing);
            let forward = order.with_untracked(|o| o.next().is_some());
            untracked(|| ctx.ask(Intent::SwipeHint { forward }));
            timer.replace(Some(set_timeout(HINT_FOR, move || {
                phase.set(HintPhase::Done)
            })));
        });
    }
    let swiping = ctx.swiping;
    effect(move || {
        if swiping.get() && phase.get_untracked() == HintPhase::Showing {
            if let Some(t) = timer.replace(None) {
                t.cancel();
            }
            phase.set(HintPhase::Done);
        }
    });
    let link = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.link.is_some()))
    };
    let visible = computed(move || phase.get() == HintPhase::Showing && !link.get());
    view()
        .absolute()
        .left(px(number("space-page")))
        .right(px(number("space-page")))
        .bottom(px(ABOVE_BAR))
        .items_center()
        .pointer_events(PointerEvents::BoxNone)
        .child(appearing(
            visible,
            (Duration::Slow, Duration::Base),
            move || {
                let words = t!("Swipe sideways to switch panes");
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .min_h(px(44.0))
                    .px(px(16.0))
                    .py(px(10.0))
                    .rounded(px(number("radius-lg")))
                    .border_curve(BorderCurve::Continuous)
                    .bg(move || token("popover"))
                    .border(px(0.5))
                    .border_color(move || token("border-strong"))
                    .shadow([BoxShadow {
                        offset_x: px(0.0),
                        offset_y: px(SHADOW_LG.y),
                        blur: px(SHADOW_LG.blur),
                        spread: px(0.0),
                        color: color(token("shadow-lg")).into(),
                        inset: false,
                    }])
                    .label(words.clone())
                    .test_id("terminal-swipe-hint")
                    .child(
                        icon("arrow.left.and.right")
                            .size(IconSize::Md)
                            .tone(Tone::Muted),
                    )
                    .child(
                        chrome_text(TextToken::Subhead, words)
                            .flex_shrink(1.0)
                            .text_color(move || token("popover-foreground")),
                    )
                    .into_element()
            },
        ))
        .into_element()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HintPhase {
    Waiting,
    Showing,
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::model::{Hook, SessionInfo};

    fn english() -> Words {
        Words {
            window: "Window".into(),
            tab: "Tab".into(),
            agent: "agent".into(),
            pane: Rc::new(|i| format!("pane {i}")),
            needs_you: "Needs you".into(),
            working: "Working".into(),
            finished: "Finished".into(),
            now: "now".into(),
        }
    }

    fn window(id: &str, session: &str, index: i64, name: &str, panes: i64) -> WindowInfo {
        WindowInfo {
            id: id.into(),
            session: session.into(),
            index,
            name: name.into(),
            pane_count: panes,
        }
    }

    fn pane(id: &str, window: &str, index: i64, command: &str, path: &str) -> PaneInfo {
        PaneInfo {
            id: id.into(),
            window: window.into(),
            index,
            command: command.into(),
            path: Some(path.into()),
        }
    }

    /// The mockup's session `work`: api (codex), trainer (claude waiting,
    /// python, zsh), web (zsh, node), docs (claude done); and a second
    /// session the swipe never reaches. The windows and panes arrive out
    /// of order, as a host's dictionary gives them.
    fn work(active: &str) -> Control {
        Control {
            mux: Mux::Tmux,
            attached: true,
            sessions: vec![
                SessionInfo {
                    id: "$0".into(),
                    name: "work".into(),
                },
                SessionInfo {
                    id: "$1".into(),
                    name: "play".into(),
                },
            ],
            windows: vec![
                window("@3", "$0", 3, "zsh", 2),
                window("@1", "$0", 1, "codex", 1),
                window("@9", "$1", 1, "zsh", 1),
                window("@2", "$0", 2, "2.1.322", 3),
                window("@4", "$0", 4, "docs", 1),
            ],
            panes: vec![
                pane("%6", "@3", 1, "node", "/home/dev/src/web"),
                pane("%5", "@3", 0, "zsh", "/home/dev/src/web"),
                pane("%1", "@1", 0, "codex", "/home/dev/src/api"),
                pane("%2", "@2", 0, "2.1.322", "/home/dev/src/trainer"),
                pane("%4", "@2", 2, "zsh", "/home/dev/src/trainer"),
                pane("%3", "@2", 1, "python", "/home/dev/src/trainer"),
                pane("%7", "@4", 0, "2.1.322", "/home/dev/src/docs"),
                pane("%9", "@9", 0, "zsh", "/home/dev/src/elsewhere"),
            ],
            active_session: Some("$0".into()),
            active_window: None,
            active_pane: Some(active.into()),
            hooks: vec![
                Hook {
                    pane: "%1".into(),
                    state: Some("working".into()),
                    agent: Some("codex".into()),
                    since: Some(1000.0),
                },
                Hook {
                    pane: "%2".into(),
                    state: Some("attention".into()),
                    agent: Some("claude".into()),
                    since: Some(1600.0),
                },
                Hook {
                    pane: "%7".into(),
                    state: Some("done".into()),
                    agent: None,
                    since: None,
                },
            ],
            ..Control::default()
        }
    }

    fn ids(order: &Order) -> Vec<&str> {
        order.stops.iter().map(|s| s.pane.as_str()).collect()
    }

    #[test]
    fn the_order_is_the_sessions_panes_window_by_window() {
        let order = Order::of(&work("%1"), &english());
        assert_eq!(ids(&order), ["%1", "%2", "%3", "%4", "%5", "%6", "%7"]);
        assert_eq!(order.current, Some(0));
        assert!(
            !ids(&order).contains(&"%9"),
            "another session's pane is never on the way"
        );
    }

    #[test]
    fn stops_are_named_the_home_way() {
        let order = Order::of(&work("%1"), &english());
        let names: Vec<(&str, &str)> = order
            .stops
            .iter()
            .map(|s| (s.place.as_str(), s.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("api", "codex"),
                ("trainer", "claude"),
                ("trainer", "python"),
                ("trainer", "zsh"),
                ("web", "zsh"),
                ("web", "node"),
                ("docs", "claude"),
            ],
            "folders and agents, never a version string"
        );
    }

    #[test]
    fn a_window_with_no_name_and_no_folder_says_its_number() {
        let mut tree = work("%1");
        tree.windows.iter_mut().for_each(|w| {
            if w.id == "@1" {
                w.name = "codex".into();
            }
        });
        tree.panes.iter_mut().for_each(|p| {
            if p.id == "%1" {
                p.path = Some("/home/dev".into());
            }
        });
        let order = Order::of(&tree, &english());
        assert_eq!(order.stops[0].place, "Window 1");
    }

    #[test]
    fn the_ends_do_not_wrap_around() {
        let first = Order::of(&work("%1"), &english());
        assert_eq!(first.previous(), None, "nothing before the first pane");
        assert_eq!(first.next().map(|s| s.pane.as_str()), Some("%2"));
        let last = Order::of(&work("%7"), &english());
        assert_eq!(last.next(), None, "nothing after the last pane");
        assert_eq!(last.previous().map(|s| s.pane.as_str()), Some("%6"));
        // Across a window's edge both ways.
        let middle = Order::of(&work("%4"), &english());
        assert_eq!(middle.previous().map(|s| s.pane.as_str()), Some("%3"));
        assert_eq!(middle.next().map(|s| s.pane.as_str()), Some("%5"));
    }

    #[test]
    fn neighbours_go_quiet_out_of_view_or_alone() {
        let order = Order::of(&work("%4"), &english());
        assert_eq!(
            neighbours(&order, true),
            (Some("%3".to_owned()), Some("%5".to_owned()))
        );
        assert_eq!(neighbours(&order, false), (None, None), "under a cover");
        let mut alone = work("%1");
        alone.windows.retain(|w| w.id == "@1" || w.session != "$0");
        alone.panes.retain(|p| p.window == "@1" || p.window == "@9");
        let alone = Order::of(&alone, &english());
        assert_eq!(alone.stops.len(), 1);
        assert_eq!(neighbours(&alone, true), (None, None), "one pane: no swipe");
        assert!(!alone.swipeable());
        // An active pane the tree doesn't hold yet: nowhere to go.
        assert_eq!(
            neighbours(&Order::of(&work("%42"), &english()), true),
            (None, None)
        );
    }

    #[test]
    fn dots_group_by_window_and_mark_who_waits() {
        let dots = Order::of(&work("%1"), &english()).dots();
        use Dot::*;
        assert_eq!(
            dots,
            Dots::Groups(vec![
                vec![Current],
                vec![Attention, Other, Other],
                vec![Other, Other],
                vec![Other],
            ])
        );
        // The current pane is the pill even when it waits.
        let on_waiting = Order::of(&work("%2"), &english()).dots();
        assert_eq!(
            on_waiting,
            Dots::Groups(vec![
                vec![Other],
                vec![Current, Other, Other],
                vec![Other, Other],
                vec![Other],
            ])
        );
    }

    #[test]
    fn many_panes_become_a_count() {
        let mut tree = work("%1");
        for i in 0..12 {
            let w = format!("@{}", 20 + i);
            tree.windows.push(window(&w, "$0", 10 + i, "", 1));
            tree.panes.push(pane(
                &format!("%{}", 20 + i),
                &w,
                0,
                "zsh",
                "/home/dev/src/x",
            ));
        }
        let order = Order::of(&work_with(tree, "%5"), &english());
        assert_eq!(order.stops.len(), 19);
        assert_eq!(
            order.dots(),
            Dots::Count {
                place: "web".into(),
                at: 5,
                of: 19
            }
        );
    }

    fn work_with(mut tree: Control, active: &str) -> Control {
        tree.active_pane = Some(active.into());
        tree
    }

    #[test]
    fn the_hint_is_due_once_there_is_somewhere_to_go() {
        let many = Order::of(&work("%1"), &english());
        assert!(hint_due(false, &many, true));
        assert!(!hint_due(true, &many, true), "seen on this device");
        assert!(!hint_due(false, &many, false), "under a cover");
        let mut alone = work("%1");
        alone.panes.retain(|p| p.id == "%1");
        alone.windows.retain(|w| w.id == "@1");
        assert!(
            !hint_due(false, &Order::of(&alone, &english()), true),
            "one pane"
        );
    }

    #[test]
    fn the_state_line_says_words_and_minutes() {
        let order = Order::of(&work("%1"), &english());
        let (words, status, tone) =
            state_line(&order.stops[1], 1600.0 + 180.0, &english()).unwrap();
        assert_eq!(words, "Needs you · 3m");
        assert_eq!((status, tone), (Status::Attention, "warning-text"));
        let (words, ..) = state_line(&order.stops[6], 5000.0, &english()).unwrap();
        assert_eq!(words, "Finished", "no time without a stamp's");
        assert_eq!(
            state_line(&order.stops[2], 0.0, &english()),
            None,
            "no agent: no state"
        );
    }

    #[test]
    fn a_stop_never_says_the_sessions_name_again() {
        // B650: a session named after the folder its windows work in.
        let mut tree = work("%5");
        tree.sessions[0].name = "Web".into();
        let order = Order::of(&tree, &english());
        let places: Vec<&str> = order.stops.iter().map(|s| s.place.as_str()).collect();
        assert_eq!(
            places,
            [
                "api", "trainer", "trainer", "trainer", "Window 3", "Window 3", "docs"
            ]
        );
    }

    #[test]
    fn herdr_tabs_walk_the_same_way() {
        let mut tree = work("%3");
        tree.mux = Mux::Herdr;
        let order = Order::of(&tree, &english());
        assert_eq!(order.previous().map(|s| s.pane.as_str()), Some("%2"));
        assert_eq!(order.next().map(|s| s.pane.as_str()), Some("%4"));
        tree.windows.iter_mut().for_each(|w| w.name = String::new());
        tree.panes.iter_mut().for_each(|p| p.path = None);
        assert_eq!(Order::of(&tree, &english()).stops[0].place, "Tab 1");
    }
}
