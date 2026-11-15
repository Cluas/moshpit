//! The pane area (`TerminalScreen.terminalBody`, UI-DESIGN §6.3's 窗格区):
//! the terminal, what stands in for it, and what covers it, on the `ui`
//! layer's tokens and components.
//!
//! - The terminal is the host's (`moshpit/terminal`): a slot NSH lays out,
//!   in which the host shows the pane's persistent SwiftTerm view, swaps
//!   panes (sliding after a swipe, fading otherwise) and cross-fades a
//!   reconnect's kept frame to the new one. The element is made once for
//!   the page; a rekey makes it again, after telling the host
//!   ([`super::Intent::Handover`]).
//! - The multiplexer's empty state, when there is no session to show: an
//!   inline [`crate::ui::empty::Empty`], centered.
//! - Over them, one of, as the host posts it (never on a timer of the
//!   page's: the host lifts a cover on the first frame it has painted).
//!   A first connect has none (B657, the owner: 「这种动画可以删掉了 不符合
//!   我们的产品了」): the pane is the terminal's background until the
//!   first frame, and the bar's transport pill says what is under way.
//!   - a reconnect's veil (`ReconnectVeil`): the kept frame dimmed under
//!     one status capsule, never a full-screen cover (the owner's rule: a
//!     reconnect keeps the last frame, dimmed 45%, and the host
//!     cross-fades it to the new one); a protocol switch's veil
//!     ([`super::model::Chrome::still`]) comes and goes at once, its
//!     capsule too, and the host swaps the frames at once;
//!   - the size-lease follower's veil (`SizeFollowVeil`): the frame dimmed
//!     under a card naming the device whose size the window has, with the
//!     button that takes it back.

use std::time::Duration as Timer;

use nsh_core::{color::Hsla, platform::Idiom, prelude::*};
use serde_json::json;

use super::{
    Ctx, Intent,
    model::{Content, Cover, CoverKind, Empty, LinkState, Mux},
    styled,
};
use crate::{
    os::Os,
    ui::{
        button::{Size, Variant, button},
        empty::{EmptyVariant, empty},
        icon::{IconSize, Tone, icon},
        motion::{Duration, ENTER, LEAVE, distance},
        status::{Status, status_dot},
        tokens::{SHADOW_LG, number, token},
        typography::TextToken,
    },
};

/// How dim a veil keeps the frame under it: the owner's 45% (§6.3, "scrim
/// 盖上（45%）"), in both appearances, of the `scrim` token's color.
pub(crate) const VEIL_DIM: f32 = 0.45;

/// A status capsule's height (§4.19, StatusCapsule).
const CAPSULE_HEIGHT: f32 = 32.0;

/// How far under the pane's top the veils' capsule and card sit.
const VEIL_TOP: f32 = 12.0;

/// The pane area.
pub(crate) fn pane_area(ctx: Ctx, order: Memo<super::switcher::Order>) -> AnyElement {
    let cover = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.cover.clone()))
    };
    let content = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.content))
    };
    let veil = computed(move || cover.with(|c| c.kind == CoverKind::Veil));
    let follow = computed(move || cover.with(|c| c.kind == CoverKind::Follow));
    let pane_follow = computed(move || cover.with(|c| c.kind == CoverKind::PaneFollow));
    let still = {
        let ctx = ctx.clone();
        computed(move || ctx.still())
    };
    let generation = ctx.generation;
    view()
        .flex_1()
        .relative()
        .overflow_hidden()
        .bg(move || token("terminal-bg"))
        .test_id("terminal-pane-area")
        // Under the terminal: the panes a swipe would go to, uncovered as
        // the host moves the terminal aside (its slot is clear then).
        .child(super::switcher::backstage(ctx.clone(), order))
        // A new element per generation (a rekey); the host moves the same
        // terminal into it.
        .child(dynamic(move || {
            let generation = generation.get();
            untracked(move || slot(generation))
        }))
        .child(when(move || content.get() == Content::Empty, {
            let ctx = ctx.clone();
            move || empty_state(ctx.clone())
        }))
        .child(fading(
            move || veil.get(),
            move || still.get(),
            || PointerEvents::None,
            (Duration::Base, Duration::Base),
            {
                let ctx = ctx.clone();
                move || veil_view(ctx.clone())
            },
        ))
        .child(fading(
            move || follow.get(),
            move || still.get(),
            || PointerEvents::Auto,
            (Duration::Base, Duration::Base),
            {
                let ctx = ctx.clone();
                move || follow_view(ctx.clone())
            },
        ))
        .child(fading(
            move || pane_follow.get(),
            move || still.get(),
            || PointerEvents::Auto,
            (Duration::Base, Duration::Base),
            {
                let ctx = ctx.clone();
                move || pane_follow_view(ctx.clone())
            },
        ))
        .child(super::switcher::hint(ctx.clone(), order))
        .child(super::chrome::link_chip(ctx))
        .into_element()
}

/// The cover the host last posted while it was `kind`: what a cover keeps
/// saying while it fades out (the host's cover is already gone then, and
/// its state is a default the fading cover must not read).
fn while_shown(ctx: &Ctx, kind: CoverKind) -> Memo<Option<Cover>> {
    let chrome = ctx.chrome;
    let last = stored(None::<Cover>);
    computed(move || {
        let cover = chrome.with(|c| c.cover.clone());
        if cover.kind == kind {
            last.replace(Some(cover.clone()));
            Some(cover)
        } else {
            last.with(|c| c.clone())
        }
    })
}

/// How long a cover fades (whole milliseconds) over `duration` ms (a
/// motion token's, none under reduced motion): none at all during a
/// protocol switch (`still`).
pub(crate) fn fade_ms(duration: f32, still: bool) -> u64 {
    if still {
        0
    } else {
        duration.max(0.0).round() as u64
    }
}

/// The terminal's slot, keyed by `generation`.
fn slot(generation: u32) -> AnyElement {
    native("moshpit/terminal")
        .key(format!("terminal-{generation}"))
        .absolute()
        .inset_0()
        .prop("generation", json!(generation))
        .into_element()
}

/// `make()` over the pane while `visible()`: fading in over the first
/// duration and out over the second (both instant under reduced motion),
/// mounted only while it shows or fades out, so a hidden cover's
/// animations do not run. When it shows is the host's to say.
///
/// While `still()` (a protocol switch) it comes at once, and a cover that
/// was up during the switch goes at once, even when the post that lifts it
/// no longer says so: it leaves as it stood.
fn fading(
    visible: impl Fn() -> bool + 'static,
    still: impl Fn() -> bool + 'static,
    pointer: impl Fn() -> PointerEvents + 'static,
    (enter, leave): (Duration, Duration),
    make: impl Fn() -> AnyElement + 'static,
) -> AnyElement {
    let visible = computed(visible);
    let still = computed(still);
    let shown = visible.get_untracked();
    let mounted = signal(shown);
    let opacity = animated_value(if shown { 1.0 } else { 0.0 });
    let pending = stored(None::<Spawned<()>>);
    // Whether the cover up now stood through a switch.
    let stood_still = stored(false);
    effect(move || {
        if visible.get() && still.get() {
            stood_still.replace(true);
        }
    });
    effect(move || {
        let show = visible.get();
        if let Some(timer) = pending.replace(None) {
            timer.cancel();
        }
        let instant = still.get_untracked() || stood_still.with(|s| *s);
        if show {
            mounted.set(true);
            match fade_ms(enter.ms(), instant) {
                0 => opacity.set(1.0),
                ms => drop(opacity.timing(1.0, ms as f32, ENTER)),
            }
        } else {
            stood_still.replace(false);
            match fade_ms(leave.ms(), instant) {
                0 => {
                    opacity.set(0.0);
                    mounted.set(false);
                }
                ms => {
                    drop(opacity.timing(0.0, ms as f32, LEAVE));
                    pending.replace(Some(set_timeout(Timer::from_millis(ms), move || {
                        mounted.set(false)
                    })));
                }
            }
        }
    });
    view()
        .absolute()
        .inset_0()
        .bind_opacity(&opacity)
        .pointer_events(move || {
            if visible.get() {
                pointer()
            } else {
                PointerEvents::None
            }
        })
        .child(when(move || mounted.get(), make))
        .into_element()
}

/// `content` in text token `token`, no larger than at extra large: the
/// terminal's chrome stops growing there (§6.3, 外框封顶 xLarge).
pub(crate) fn chrome_text(token: TextToken, content: impl IntoReactive<SharedString>) -> Text {
    styled(
        text(content),
        token.design(),
        token.size(1.0),
        token.weight(),
    )
}

/// What a status capsule says over a cover (`ReconnectVeil.label`, in the
/// design's words): an ended session says so; with nothing being
/// attempted the phone waits for a network; otherwise it reconnects.
pub(crate) fn capsule_label(state: LinkState, ended: bool) -> String {
    if ended {
        t!("Session ended")
    } else if state == LinkState::Offline {
        t!("Waiting for network…")
    } else {
        t!("Reconnecting…")
    }
}

/// What leads a status capsule's words: the spinner while something is
/// being attempted, the hollow ring when nothing is.
fn capsule_status(state: LinkState, ended: bool) -> Status {
    if ended || state == LinkState::Offline {
        Status::Offline
    } else {
        Status::Connecting
    }
}

/// A status capsule (§4.19): glass, 32 tall, the status's dot (or the
/// spinner) and the words in `text-subhead`. A change of words rises 4 pt
/// into place as it fades in (`fast`).
fn status_capsule(lead: Memo<Status>, label: Memo<String>, still: Memo<bool>) -> View {
    let reduced = reduce_motion();
    let first = stored(true);
    view()
        .relative()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(CAPSULE_HEIGHT))
        .px(px(14.0))
        .rounded_full()
        .border_curve(BorderCurve::Continuous)
        // The bar's capsules' look (UI-DESIGN §6.3, the mockup's
        // `.capsule`): `chrome` over a blur, a `border-strong` hairline, so
        // it reads on the kept frame in either appearance.
        .backdrop_blur(px(24.0))
        .bg(move || token("chrome"))
        .border(px(0.5))
        .border_color(move || token("border-strong"))
        .child(dynamic(move || {
            let status = lead.get();
            untracked(move || status_dot(status).into_element())
        }))
        .child(dynamic(move || {
            let label = label.get();
            let changed = !first.replace(false);
            untracked(move || {
                let mut words = view().child(
                    chrome_text(TextToken::Subhead, label)
                        .number_of_lines(1u32)
                        .text_color(move || token("foreground")),
                );
                // A switch's capsule changes its words at once.
                if changed && !reduced && !still.get_untracked() {
                    let rise = MotionSpec::new(
                        KeyframeTiming::new(Duration::Fast.nominal_ms()).easing(ENTER),
                    )
                    .values(MotionChannel::Opacity, [0.0, 1.0])
                    .values(MotionChannel::TranslateY, [distance::RISE, 0.0]);
                    words = words.bind_motion(&motion().entering(rise));
                }
                words.into_element()
            })
        }))
}

/// The reconnect's veil (`ReconnectVeil`): the kept frame dimmed 45%
/// under one status capsule, centered at the pane's top. It never covers
/// the whole pane: the frame under it is the person's last view of it, and
/// the host cross-fades it to the new one. The capsule drops 8 pt into
/// place as it fades in (`slow`) once the veil is up; a protocol switch's
/// is in place at once, and the host swaps its frames at once.
fn veil_view(ctx: Ctx) -> AnyElement {
    let cover = while_shown(&ctx, CoverKind::Veil);
    let said = computed(move || {
        cover.with(|c| {
            c.as_ref()
                .map_or((LinkState::Reconnecting, false), |c| (c.state, c.ended))
        })
    });
    let lead = computed(move || {
        let (state, ended) = said.get();
        capsule_status(state, ended)
    });
    let label = computed(move || {
        let (state, ended) = said.get();
        capsule_label(state, ended)
    });
    let still = {
        let ctx = ctx.clone();
        computed(move || ctx.still())
    };
    let mut capsule = status_capsule(lead, label, still)
        .mt(px(VEIL_TOP))
        .test_id("terminal-reconnect-capsule");
    // A switch's capsule is there at once, as its veil is.
    if !reduce_motion() && !still.get_untracked() {
        let drop_in =
            MotionSpec::new(KeyframeTiming::new(Duration::Slow.nominal_ms()).easing(ENTER))
                .values(MotionChannel::Opacity, [0.0, 1.0])
                .values(MotionChannel::TranslateY, [-distance::SLIDE, 0.0]);
        capsule = capsule.bind_motion(&motion().entering(drop_in));
    }
    view()
        .absolute()
        .inset_0()
        .items_center()
        .bg(veil_scrim)
        .test_id("terminal-reconnect-veil")
        .child(capsule)
        .into_element()
}

/// The veils' scrim: the `scrim` token's color at [`VEIL_DIM`].
fn veil_scrim() -> AdaptiveColor {
    token("scrim").map(|c: Hsla| Hsla { a: VEIL_DIM, ..c })
}

/// The device this is, as the follower's button names it.
fn this_device() -> Option<&'static str> {
    match (Os::current(), idiom()) {
        (Os::Android, _) => None,
        (_, Idiom::Pad) => Some("iPad"),
        _ => Some("iPhone"),
    }
}

/// The follower's veil (`SizeFollowVeil`): the window is sized for another
/// of the person's devices, so the pane keeps its frame, dimmed, under a
/// card that names the holder and offers this device's size back.
/// B587: the desktop picked another pane in the window shown (tmux keeps
/// one active pane per window, shared); the phone keeps its own, frozen on
/// its last frame, and the button takes it back — the last touch wins, as
/// with the size lease. A keystroke takes it back too.
fn pane_follow_view(ctx: Ctx) -> AnyElement {
    let take = ctx.asking(Intent::TakeOver);
    view()
        .absolute()
        .inset_0()
        .items_center()
        .bg(veil_scrim)
        .test_id("terminal-pane-follow")
        .child(
            view()
                .mt(px(VEIL_TOP))
                .mx(px(number("space-page")))
                .max_w(px(360.0))
                .items_center()
                .gap(px(12.0))
                .p(px(16.0))
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
                .test_id("terminal-pane-follow-card")
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon("desktopcomputer").size(IconSize::Md).tone(Tone::Muted))
                        .child(
                            chrome_text(
                                TextToken::Subhead,
                                t!("The desktop switched to another pane."),
                            )
                            .flex_shrink(1.0)
                            .text_color(move || token("popover-foreground")),
                        ),
                )
                .child(
                    // `Size::Md`: 44 pt high, the tap target's minimum.
                    button(t!("Take it back"))
                        .variant(Variant::Primary)
                        .size(Size::Md)
                        .on_press(take)
                        .test_id("terminal-pane-follow-take"),
                ),
        )
        .into_element()
}

fn follow_view(ctx: Ctx) -> AnyElement {
    let owner = ctx
        .chrome
        .with_untracked(|c| c.cover.owner.clone().unwrap_or_default());
    let take = ctx.asking(Intent::TakeOver);
    let symbol = if owner == "iPad" { "ipad" } else { "iphone" };
    let says = t!("This window is sized for %@.", arg1 = owner);
    let take_label = match this_device() {
        Some(device) => t!("Use this %@'s size", arg1 = device),
        None => t!("Use this device's size"),
    };
    view()
        .absolute()
        .inset_0()
        .items_center()
        .bg(veil_scrim)
        .test_id("terminal-size-follow")
        .child(
            view()
                .mt(px(VEIL_TOP))
                .mx(px(number("space-page")))
                .max_w(px(360.0))
                .items_center()
                .gap(px(12.0))
                .p(px(16.0))
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
                .test_id("terminal-size-follow-card")
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon(symbol).size(IconSize::Md).tone(Tone::Muted))
                        .child(
                            chrome_text(TextToken::Subhead, says)
                                .flex_shrink(1.0)
                                .text_color(move || token("popover-foreground")),
                        ),
                )
                .child(
                    button(take_label)
                        .variant(Variant::Primary)
                        .size(Size::Md)
                        .on_press(take)
                        .test_id("terminal-size-follow-take"),
                ),
        )
        .into_element()
}

/// How long the empty state shows the connection's wait before it says
/// what it found: the attach handshake's normal seconds.
const SETTLE: Timer = Timer::from_secs(4);

/// The multiplexer's empty state (`MultiplexerEmptyStateView`): the wait
/// first (the bare background: no poster, B657), then
/// one of two honest answers: not installed (Install…), or no sessions
/// (New Session; Moshpit never makes one behind the person's back). When
/// the host has the answer already ([`Empty::settled`]) there is no wait:
/// the answer shows at once, and the moment the host says so mid-wait.
fn empty_state(ctx: Ctx) -> AnyElement {
    let waited = signal(false);
    drop(set_timeout(SETTLE, move || waited.set(true)));
    let known = computed(move || {
        ctx.chrome
            .with(|c| c.empty.as_ref().is_some_and(|e| e.settled))
    });
    let settled = computed(move || waited.get() || known.get());
    let creating = signal(false);
    // A creating that failed says so in the controller's notice: New
    // Session is back (SwiftUI's `.onChange(of: failure)`).
    let failure = computed(move || {
        ctx.chrome
            .with(|c| c.notice.as_ref().map(|n| n.text.clone()))
    });
    effect(move || {
        if failure.get().is_some() {
            creating.set(false);
        }
    });
    view()
        .absolute()
        .inset_0()
        .bg(move || token("terminal-bg"))
        .test_id("terminal-empty-state")
        .child(dynamic(move || {
            let settled = settled.get();
            // While it creates, the button says so and is off.
            let busy = creating.get();
            let empty = ctx.chrome.with(|c| c.empty.clone()).unwrap_or_default();
            let ctx = ctx.clone();
            untracked(move || {
                if !settled {
                    return view().into_element();
                }
                answer(&empty, busy, creating, ctx)
            })
        }))
        .into_element()
}

/// The empty state's answer, once it has one: an inline empty state,
/// centered in the pane.
fn answer(empty_state: &Empty, busy: bool, creating: Signal<bool>, ctx: Ctx) -> AnyElement {
    let mux = match empty_state.mux {
        Mux::Herdr => "herdr",
        _ => "tmux",
    };
    let host = ctx.chrome.with_untracked(|c| c.name.clone());
    let state = if empty_state.missing {
        empty(t!("%@ isn't installed on this host", arg1 = mux))
            .icon("shippingbox")
            .action(
                button(t!("Install…"))
                    .variant(Variant::Primary)
                    .on_press(ctx.asking(Intent::Install))
                    .test_id("multiplexer-install"),
            )
    } else {
        let label = if busy {
            t!("Creating…")
        } else {
            t!("New %@", arg1 = empty_state.session.clone())
        };
        empty(t!(
            "No %@ %@ on %@",
            arg1 = mux,
            arg2 = empty_state.sessions_lower.clone(),
            arg3 = host
        ))
        .icon("square.stack.3d.up.slash")
        .action(
            button(label)
                .variant(Variant::Primary)
                .disabled(move || busy)
                .on_press(move || {
                    // `new-session` is not idempotent: one at a time.
                    if creating.get_untracked() {
                        return;
                    }
                    creating.set(true);
                    ctx.ask(Intent::Create);
                })
                .test_id("multiplexer-create-first-session"),
        )
    };
    view()
        .absolute()
        .inset_0()
        .items_center()
        .justify_center()
        .px(px(number("space-page")))
        .child(
            view()
                .w_full()
                .max_w(px(360.0))
                .test_id("terminal-empty-answer")
                .child(state.variant(EmptyVariant::Inline)),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{theme::AppTheme, ui::tokens::colors};

    #[test]
    fn a_veil_spins_while_something_is_attempted_and_rings_hollow_when_not() {
        assert_eq!(
            capsule_status(LinkState::Reconnecting, false),
            Status::Connecting
        );
        assert_eq!(
            capsule_status(LinkState::Connecting, false),
            Status::Connecting
        );
        assert_eq!(capsule_status(LinkState::Offline, false), Status::Offline);
        assert_eq!(
            capsule_status(LinkState::Reconnecting, true),
            Status::Offline
        );
    }

    #[test]
    fn the_veil_dims_the_frame_45_percent_in_both_appearances() {
        let (_, light, dark) = colors(&AppTheme::SIGNAL_ROOM)
            .into_iter()
            .find(|(name, ..)| *name == "scrim")
            .unwrap();
        let scrim = AdaptiveColor::new(light, dark).map(|c: Hsla| Hsla { a: VEIL_DIM, ..c });
        assert_eq!((scrim.light.a, scrim.dark.a), (0.45, 0.45));
        assert_eq!(Rgba::from(scrim.dark).r, 0.0, "the scrim's black");
    }

    #[test]
    fn a_switch_fades_nothing_and_anything_else_fades_its_token() {
        assert_eq!(fade_ms(240.0, true), 0, "a protocol switch moves nothing");
        assert_eq!(fade_ms(400.0, true), 0);
        assert_eq!(fade_ms(240.0, false), 240);
        assert_eq!(fade_ms(0.0, false), 0, "reduced motion");
    }
}
