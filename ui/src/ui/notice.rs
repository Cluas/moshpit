//! Telling the user something without asking anything
//! (`docs/nsh/UI-DESIGN.md` §4.16): a [`Banner`] stays while what it says
//! holds (the connection is coming back, a host's key changed), and a
//! toast ([`Toasts`]) says briefly how something the user cannot see went
//! ("Copied"), then goes.
//!
//! A screen shows one toast at a time: a new one ends the one showing.
//! How many banners stand at once is the screen's business (the terminal's
//! chrome shows one).

use std::rc::Rc;

use nsh_base::{
    Anchor, SwipeDirection, ToastManager, ToastMotion, ToastOptions, ToastStackState,
    ToastTransitionStatus, toast, toast_stack,
};
use nsh_core::{prelude::*, time::now};

use crate::ui::{
    button::{IconButtonSize, Size, Variant, button, icon_button},
    icon::{IconSize, Tone, icon},
    motion::{Duration, ENTER, LEAVE, distance},
    tokens::{SHADOW_LG, number, token},
    typography::TextToken,
};

/// What a banner is about (§4.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BannerTone {
    /// Something to know (reconnecting, read-only).
    Info,
    /// Something that may need the user (a host's key changed).
    Warning,
    /// Something that failed and stays failed (the session ended).
    Destructive,
}

impl BannerTone {
    /// Its fill, its symbol's tone and its symbol when none is given.
    fn look(self) -> (&'static str, Tone, &'static str) {
        match self {
            BannerTone::Info => ("info-soft", Tone::Info, "info.circle.fill"),
            BannerTone::Warning => (
                "warning-soft",
                Tone::Warning,
                "exclamationmark.triangle.fill",
            ),
            BannerTone::Destructive => {
                ("destructive-soft", Tone::Destructive, "xmark.octagon.fill")
            }
        }
    }
}

/// A banner ([`banner`]).
pub struct Banner {
    tone: BannerTone,
    message: Rc<dyn Fn() -> SharedString>,
    symbol: Option<&'static str>,
    action: Option<(SharedString, Rc<dyn Fn()>)>,
    on_close: Option<Rc<dyn Fn()>>,
    shown: Rc<dyn Fn() -> bool>,
    still: Rc<dyn Fn() -> bool>,
    action_test_id: Option<SharedString>,
    close_test_id: Option<SharedString>,
    floating: bool,
    test_id: Option<SharedString>,
}

/// A banner of `tone` saying `message`: one sentence.
pub fn banner(tone: BannerTone, message: impl Fn() -> SharedString + 'static) -> Banner {
    Banner {
        tone,
        message: Rc::new(message),
        symbol: None,
        action: None,
        on_close: None,
        shown: Rc::new(|| true),
        still: Rc::new(|| false),
        action_test_id: None,
        close_test_id: None,
        floating: false,
        test_id: None,
    }
}

impl Banner {
    /// An SF Symbol of its own, in place of the tone's.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.symbol = Some(name);
        self
    }

    /// The one thing to do about it: a small ghost button saying `label`
    /// after the words.
    pub fn action(mut self, label: impl Into<SharedString>, on_press: impl Fn() + 'static) -> Self {
        self.action = Some((label.into(), Rc::new(on_press)));
        self
    }

    /// The action button's test id (and the host's identifier).
    pub fn action_test_id(mut self, id: &str) -> Self {
        self.action_test_id = Some(SharedString::from(id.to_owned()));
        self
    }

    /// Its close button's own test id.
    pub fn close_test_id(mut self, id: &str) -> Self {
        self.close_test_id = Some(SharedString::from(id.to_owned()));
        self
    }

    /// It floats over content that isn't a page's (the terminal): the
    /// tone's wash is laid on a `popover` surface with the large shadow,
    /// so what lies under it doesn't show through its words.
    pub fn floating(mut self) -> Self {
        self.floating = true;
        self
    }

    /// A close button at its end (screen readers: "Close").
    pub fn on_close(mut self, on_close: impl Fn() + 'static) -> Self {
        self.on_close = Some(Rc::new(on_close));
        self
    }

    /// While this holds the banner shows; when it stops, the banner fades
    /// out and then goes (NSH has no exit transition: it stays mounted for
    /// the fade). Without it, the banner shows for as long as it is
    /// mounted.
    pub fn shown(mut self, shown: impl Fn() -> bool + 'static) -> Self {
        self.shown = Rc::new(shown);
        self
    }

    /// While this holds the banner comes and goes at once, as under
    /// reduced motion (the terminal's protocol switch, which moves
    /// nothing). Read as it comes or goes, not tracked.
    pub fn still(mut self, still: impl Fn() -> bool + 'static) -> Self {
        self.still = Rc::new(still);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

/// A banner coming in: down from its edge as it fades in.
fn banner_enter() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(Duration::Slow.nominal_ms()).easing(ENTER))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [-distance::SLIDE, 0.0])
}

/// A banner going: it fades out where it is.
fn banner_leave() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(Duration::Base.nominal_ms()).easing(LEAVE))
        .values(MotionChannel::Opacity, [1.0, 0.0])
}

impl IntoElement for Banner {
    fn into_element(self) -> AnyElement {
        let Banner {
            tone,
            message,
            symbol,
            action,
            on_close,
            shown,
            still,
            action_test_id,
            close_test_id,
            floating,
            test_id,
        } = self;
        let reduced = reduce_motion();
        let shown = computed(move || shown());
        let mounted = signal(shown.get_untracked());
        // The motion of the banner mounted now, to play its leaving on.
        let moving = stored(None::<Motion>);
        let was = stored(shown.get_untracked());
        let pending = stored(None::<Spawned<()>>);
        let still_then = still.clone();
        effect(move || {
            let show = shown.get();
            if was.replace(show) == show {
                return;
            }
            if let Some(timer) = pending.replace(None) {
                timer.cancel();
            }
            let still_now = untracked(|| still_then());
            let playing = moving
                .get()
                .filter(|_| !reduced && !still_now && mounted.get_untracked());
            match (show, playing) {
                // Back while it faded: in again from where it is.
                (true, Some(m)) => drop(m.play(banner_enter())),
                (true, None) => mounted.set(true),
                (false, Some(m)) => {
                    drop(m.play(banner_leave()));
                    pending.replace(Some(set_timeout(
                        std::time::Duration::from_millis(Duration::Base.nominal_ms() as u64),
                        move || mounted.set(false),
                    )));
                }
                (false, None) => mounted.set(false),
            }
        });
        let (fill, ink, default_symbol) = tone.look();
        let make = move || {
            let message = message.clone();
            let mut row = view()
                .w_full()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .min_h(px(44.0))
                .ps(px(12.0))
                .pe(px(if on_close.is_some() { 4.0 } else { 12.0 }))
                .py(px(6.0))
                .rounded(px(number("radius")))
                .border_curve(BorderCurve::Continuous)
                .bg(move || token(fill))
                .child(
                    icon(symbol.unwrap_or(default_symbol))
                        .size(IconSize::Md)
                        .tone(ink),
                )
                .child(
                    TextToken::Footnote
                        .apply(text(move || message()))
                        .text_color(move || token("foreground"))
                        .flex_1(),
                );
            if let Some((label, on_press)) = action.clone() {
                let mut action = button(label)
                    .variant(Variant::Ghost)
                    .size(Size::Sm)
                    .on_press(move || on_press());
                if let Some(id) = &action_test_id {
                    action = action.test_id(id);
                }
                row = row.child(action);
            }
            if let Some(on_close) = on_close.clone() {
                let mut close = icon_button("xmark", t!("Close"))
                    .size(IconButtonSize::Sm)
                    .on_press(move || on_close());
                if let Some(id) = &close_test_id {
                    close = close.test_id(id);
                }
                row = row.child(close);
            }
            let mut banner = if floating {
                view()
                    .w_full()
                    .rounded(px(number("radius")))
                    .border_curve(BorderCurve::Continuous)
                    .bg(move || token("popover"))
                    // Read where it is drawn, as the toast's: the shadow
                    // follows the appearance it shows in.
                    .style_with(move |s| {
                        s.shadow([BoxShadow {
                            offset_x: px(0.0),
                            offset_y: px(SHADOW_LG.y),
                            blur: px(SHADOW_LG.blur),
                            spread: px(0.0),
                            color: color(token("shadow-lg")).into(),
                            inset: false,
                        }])
                    })
                    .child(row)
            } else {
                row
            };
            if !reduced && !untracked(|| still()) {
                let m = motion().entering(banner_enter());
                moving.replace(Some(m));
                banner = banner.bind_motion(&m);
            }
            if let Some(id) = &test_id {
                banner = banner.test_id(id);
            }
            banner
        };
        when(move || mounted.get(), make).into_element()
    }
}

/// How long a toast stays once it is in, in milliseconds (§4.16). The
/// user reads it in this time whatever the motion: reduced motion does not
/// change it.
pub const DWELL_MS: u64 = 4000;

/// How often the toasts' lifecycle moves on while one is mounted.
const TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// A mounted toast: its id, what it says, its phase.
type Shown = (u64, SharedString, ToastTransitionStatus);

/// A screen's toasts ([`toasts`]): [`Toasts::show`] from anywhere on the
/// screen, [`Toasts::host`] laid over it once. Built on `nsh_base`'s
/// toast manager and stack, which time a toast, run its enter and exit
/// transitions and keep it mounted until its exit has played. A `Copy`
/// handle.
#[derive(Clone, Copy)]
pub struct Toasts {
    manager: Stored<ToastManager<u64, SharedString>>,
    shown: Signal<Vec<Shown>>,
    state: ToastStackState,
    motion: ToastMotion,
    next: Stored<u64>,
    ticker: Stored<Option<Spawned<()>>>,
    owner: Owner,
}

/// The toast's motion on this surface: in over `slow`, out over `base`,
/// both instant while motion is reduced.
fn toast_motion() -> ToastMotion {
    let ms = |d: Duration| std::time::Duration::from_secs_f32(d.ms() / 1000.0);
    ToastMotion {
        duration: ms(Duration::Slow),
        exit_duration: ms(Duration::Base),
        ..ToastMotion::sonner()
    }
}

/// A screen's toasts, none showing. Made at the screen's root, it is its
/// views' context too: a view made under it finds it with
/// `use_context::<Toasts>()`.
pub fn toasts() -> Toasts {
    let motion = toast_motion();
    let toasts = Toasts {
        manager: stored(ToastManager::new(motion)),
        shown: signal(Vec::new()),
        state: ToastStackState::new(),
        motion,
        next: stored(0),
        ticker: stored(None),
        owner: Owner::current(),
    };
    provide_context(toasts);
    toasts
}

impl Toasts {
    /// Says `message` for [`DWELL_MS`], ending the toast showing, and has
    /// the screen reader read it.
    pub fn show(&self, message: impl Into<SharedString>) {
        let message = message.into();
        let id = self.next.with(|n| {
            *n += 1;
            *n
        });
        let at = now();
        self.manager.with(|m| {
            m.dismiss_all(at);
            m.push(
                id,
                message,
                ToastOptions::timeout(Some(std::time::Duration::from_millis(DWELL_MS))),
                at,
            );
        });
        self.sync();
        self.tick();
    }

    /// What the toast showing says, if one shows (an ending one does not).
    pub fn message(&self) -> Option<SharedString> {
        self.shown.with(|shown| {
            shown
                .iter()
                .rev()
                .find(|(.., status)| *status != ToastTransitionStatus::Ending)
                .map(|(_, message, _)| message.clone())
        })
    }

    /// Ends toast `id` (swiped away, Escape).
    fn dismiss(&self, id: u64) {
        if self.manager.with(|m| m.dismiss(&id, now())) {
            self.sync();
            self.tick();
        }
    }

    /// The mounted toasts, as the manager has them.
    fn sync(&self) {
        self.shown.set(self.manager.with(|m| {
            m.iter()
                .map(|(id, message, status)| (*id, message.clone(), status))
                .collect()
        }));
    }

    /// Moves the lifecycle on while a toast is mounted (paused while the
    /// user holds the stack), and stops once none is.
    fn tick(&self) {
        if self.ticker.with(|t| t.is_some()) {
            return;
        }
        let toasts = *self;
        let ticker = self.owner.run(|| {
            set_interval(TICK, move || {
                let paused = untracked(|| toasts.state.is_expanded());
                if toasts.manager.with(|m| m.advance(now(), paused).changed) {
                    toasts.sync();
                }
                if toasts.manager.with(|m| m.is_empty())
                    && let Some(ticker) = toasts.ticker.replace(None)
                {
                    ticker.cancel();
                }
            })
        });
        self.ticker.replace(Some(ticker));
    }

    /// Where the toasts show: over the screen, 8 pt above its bottom safe
    /// area (and [`ToastHost::clearance`]).
    pub fn host(&self) -> ToastHost {
        ToastHost {
            toasts: *self,
            clearance: Rc::new(|| 0.0),
            test_id: None,
        }
    }
}

/// Where a screen's toasts show ([`Toasts::host`]).
pub struct ToastHost {
    toasts: Toasts,
    clearance: Rc<dyn Fn() -> f32>,
    test_id: Option<SharedString>,
}

impl ToastHost {
    /// What stands at the screen's bottom above its safe area (the
    /// shortcut bar's height): the toast shows 8 pt above it.
    pub fn clearance(mut self, clearance: impl Fn() -> f32 + 'static) -> Self {
        self.clearance = Rc::new(clearance);
        self
    }

    /// The toast's own test id.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for ToastHost {
    fn into_element(self) -> AnyElement {
        let ToastHost {
            toasts,
            clearance,
            test_id,
        } = self;
        view()
            .absolute()
            .left(px(number("space-page")))
            .right(px(number("space-page")))
            .bottom(move || px(clearance() + 8.0))
            .pb_safe_area_plus(px(0.0))
            .pointer_events(PointerEvents::BoxNone)
            .child(
                toast_stack()
                    .placement(Anchor::BottomCenter)
                    .motion(toasts.motion)
                    .state(toasts.state)
                    .on_cancel(move || {
                        let newest = toasts.shown.with_untracked(|shown| {
                            shown
                                .iter()
                                .rev()
                                .find(|(.., s)| *s != ToastTransitionStatus::Ending)
                                .map(|(id, ..)| *id)
                        });
                        if let Some(id) = newest {
                            toasts.dismiss(id);
                        }
                    })
                    .child(each(
                        toasts.shown,
                        |(id, ..)| *id,
                        move |item, _| surface(toasts, item, test_id.clone()),
                    )),
            )
            .into_element()
    }
}

/// A toast (§4.16): a popup's surface — `popover`, `radius-lg`, a
/// `border-strong` hairline in both appearances, the large shadow — sized
/// to its words and centered. It rises in over `slow` and fades out over
/// `base`; swiping it down ends it.
fn surface(toasts: Toasts, item: ReadSignal<Shown>, test_id: Option<SharedString>) -> AnyElement {
    let id = item.get_untracked().0;
    let ending = move || item.get().2 == ToastTransitionStatus::Ending;
    let mut surface = toast()
        .transition_status(move || item.get().2)
        .accessibility_label(move || Some(item.get().1))
        .swipe_direction(Some(SwipeDirection::Down))
        .on_dismiss_request(move || toasts.dismiss(id))
        .starting_class(SharedString::from(format!(
            "opacity-0 translate-y-[{}px]",
            distance::ENTER
        )))
        .ending_class("opacity-0")
        .self_center()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .min_h(px(44.0))
        .px(px(16.0))
        .py(px(10.0))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("popover"))
        .border(px(0.5))
        .border_color(move || token("border-strong"))
        .transition()
        .style_with(move |s| {
            let s = s.shadow([BoxShadow {
                offset_x: px(0.0),
                offset_y: px(SHADOW_LG.y),
                blur: px(SHADOW_LG.blur),
                spread: px(0.0),
                color: color(token("shadow-lg")).into(),
                inset: false,
            }]);
            if ending() {
                s.duration(Duration::Base.ms()).ease_in()
            } else {
                s.duration(Duration::Slow.ms()).ease_out()
            }
        })
        .child(
            TextToken::Subhead
                .apply(text(move || item.get().1))
                .text_color(move || token("popover-foreground")),
        );
    if let Some(id) = &test_id {
        surface = surface.test_id(id);
    }
    surface.into_element()
}
