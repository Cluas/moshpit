//! What lies over the pane under the top bar (UI-DESIGN §6.3's 横幅区 and
//! 多路复用器提示) and at the pane's foot (the chip that opens a link just
//! copied in the terminal), on the `ui` layer's [`Banner`] and the toast's
//! surface.
//!
//! - One banner at a time, 8 pt under the top bar, 16 pt in from the
//!   sides: mosh's dead return path (`warning`), roaming (`info`), a
//!   degraded session (`warning`) — whichever the host posts in its one
//!   slot. A new kind replaces the one up at once; the last one fades out
//!   where it is. The terminal stays usable behind it.
//! - The multiplexer's notice under it (`info`): a failed command's words,
//!   or herdr's about its pane ("This pane is open on another device",
//!   with "Open here"), with the remedy the person may take — never taken
//!   for them.
//! - The link chip, 8 pt above the shortcut bar, in a toast's form, for as
//!   long as the host shows it (it times it).

use std::time::Duration as Timer;

use nsh_core::prelude::*;

use super::{
    Ctx, Intent,
    cover::chrome_text,
    model::{Banner as BannerModel, Missing, Notice},
};
use crate::ui::{
    icon::{IconSize, Tone, icon},
    motion::{Duration, ENTER, LEAVE, distance},
    notice::{BannerTone, banner},
    tokens::{SHADOW_LG, number, token},
    typography::TextToken,
};

/// How far under the top bar the banner area starts.
const UNDER_BAR: f32 = 8.0;
/// How far above the shortcut bar the link chip sits.
const ABOVE_BAR: f32 = 8.0;

/// A banner's kind: what decides which banner is up (its words may
/// change under it, as the round trip does while roaming).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    ReturnPath,
    Roam,
    Degrade(Missing),
}

impl Kind {
    pub(crate) fn of(banner: &BannerModel) -> Kind {
        match banner {
            BannerModel::ReturnPath => Kind::ReturnPath,
            BannerModel::Roam { .. } => Kind::Roam,
            BannerModel::Degrade { missing } => Kind::Degrade(*missing),
        }
    }
}

/// What `kind`'s banner says; `srtt`: the smoothed round trip a roam
/// banner reads in small words after its sentence.
pub(crate) fn banner_words(kind: Kind, srtt: Option<f64>) -> String {
    match kind {
        Kind::ReturnPath => t!("No UDP reply from mosh-server."),
        Kind::Roam => {
            let said = t!("Network changed. mosh is reconnecting.");
            match srtt {
                Some(srtt) => format!("{said} · {} ms", srtt.round() as i64),
                None => said,
            }
        }
        Kind::Degrade(Missing::Tmux) => {
            t!(
                "%@ isn't installed here, so this is a plain shell.",
                arg1 = "tmux"
            )
        }
        Kind::Degrade(Missing::Herdr) => {
            t!(
                "%@ isn't installed here, so this is a plain shell.",
                arg1 = "herdr"
            )
        }
        Kind::Degrade(Missing::MoshServer) => {
            t!("mosh-server isn't installed here, so this is SSH.")
        }
    }
}

/// The value `read()` last had while it was something: what a piece
/// keeps saying while it fades out (the host's value is already gone).
fn kept<T: Clone + PartialEq + 'static>(read: impl Fn() -> Option<T> + 'static) -> Memo<Option<T>> {
    let last = stored(None::<T>);
    computed(move || match read() {
        Some(value) => {
            last.replace(Some(value.clone()));
            Some(value)
        }
        None => last.with(|v| v.clone()),
    })
}

/// The banner and the notice, laid over the screen under the top bar.
pub(crate) fn over_pane(ctx: Ctx, order: Memo<super::switcher::Order>) -> AnyElement {
    let compact = {
        let ctx = ctx.clone();
        computed(move || ctx.compact())
    };
    view()
        .absolute()
        .inset_0()
        .pointer_events(PointerEvents::BoxNone)
        .child(
            view()
                .absolute()
                .left(px(number("space-page")))
                .right(px(number("space-page")))
                .top(move || px(safe_area().top.0 + super::bar::height(compact.get()) + UNDER_BAR))
                .gap(px(8.0))
                .pointer_events(PointerEvents::BoxNone)
                .test_id("terminal-banners")
                .child(banner_slot(ctx.clone()))
                .child(notice_slot(ctx.clone()))
                // The swipe's dots, under whatever banner is up.
                .child(super::switcher::dots(ctx, order)),
        )
        .into_element()
}

/// The one banner (§6.3): the kind the host posts, made again only when
/// the kind changes, so a roam's round trip updates its words in place.
fn banner_slot(ctx: Ctx) -> AnyElement {
    let current = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.banner.as_ref().map(Kind::of)))
    };
    let last = kept(move || current.get());
    dynamic(move || {
        let kind = last.get();
        let ctx = ctx.clone();
        untracked(move || match kind {
            Some(kind) => banner_view(kind, current, ctx),
            None => view().into_element(),
        })
    })
    .into_element()
}

fn banner_view(kind: Kind, current: Memo<Option<Kind>>, ctx: Ctx) -> AnyElement {
    let chrome = ctx.chrome;
    let words = move || -> SharedString {
        let srtt = chrome.with(|c| match &c.banner {
            Some(BannerModel::Roam { srtt, .. }) => *srtt,
            _ => None,
        });
        banner_words(kind, srtt).into()
    };
    let shown = move || current.get() == Some(kind);
    match kind {
        Kind::ReturnPath => banner(BannerTone::Warning, words)
            .icon("wifi.exclamationmark")
            .action(t!("Use SSH"), ctx.asking(Intent::BannerAction))
            .action_test_id("mosh-returnpath-switch-ssh")
            .on_close(ctx.asking(Intent::BannerDismiss))
            .close_test_id("mosh-returnpath-dismiss")
            .test_id("mosh-returnpath-banner"),
        Kind::Degrade(_) => banner(BannerTone::Warning, words)
            .action(t!("Install…"), ctx.asking(Intent::BannerAction))
            .action_test_id("host-banner-install")
            .on_close(ctx.asking(Intent::BannerDismiss))
            .close_test_id("host-banner-dismiss")
            .test_id("host-banner"),
        Kind::Roam => banner(BannerTone::Info, words)
            .icon("antenna.radiowaves.left.and.right")
            .test_id("roam-banner"),
    }
    .floating()
    .shown(shown)
    .still(move || chrome.with_untracked(|c| c.still))
    .into_element()
}

/// The multiplexer's notice (§6.3): its words, and its remedy, if it has
/// one.
fn notice_slot(ctx: Ctx) -> AnyElement {
    let current = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.notice.clone()))
    };
    let last = kept(move || current.get());
    dynamic(move || {
        let notice = last.get();
        let ctx = ctx.clone();
        untracked(move || match notice {
            Some(Notice { text, action }) => {
                let said = text.clone();
                let mut notice = banner(BannerTone::Info, move || said.clone().into())
                    .floating()
                    .shown(move || current.with(|n| n.as_ref().is_some_and(|n| n.text == text)))
                    .test_id("terminal-notice");
                if let Some(action) = action {
                    notice = notice
                        .action(action, ctx.asking(Intent::NoticeAction))
                        .action_test_id("terminal-notice-action");
                }
                notice.into_element()
            }
            None => view().into_element(),
        })
    })
    .into_element()
}

/// A link chip coming in: up from below as it fades in (a toast's).
fn chip_enter() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(Duration::Slow.nominal_ms()).easing(ENTER))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [distance::ENTER, 0.0])
}

/// A link chip going: it fades out where it is.
fn chip_leave() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(Duration::Base.nominal_ms()).easing(LEAVE))
        .values(MotionChannel::Opacity, [1.0, 0.0])
}

/// The chip that opens a link just copied (§6.3, 复制链接芯片): a toast's
/// surface — `popover`, `radius-lg`, a `border-strong` hairline, the large
/// shadow — with the link symbol and "Open <host>", 8 pt above the
/// shortcut bar. It shows while the host posts the link, and fades out
/// where it is when the host takes it away.
pub(crate) fn link_chip(ctx: Ctx) -> AnyElement {
    let current = {
        let ctx = ctx.clone();
        computed(move || ctx.chrome.with(|c| c.link.clone()))
    };
    let last = kept(move || current.get());
    let reduced = reduce_motion();
    view()
        .absolute()
        .left(px(number("space-page")))
        .right(px(number("space-page")))
        .bottom(px(ABOVE_BAR))
        .items_center()
        .pointer_events(PointerEvents::BoxNone)
        .child(dynamic(move || {
            let host = last.get();
            let ctx = ctx.clone();
            untracked(move || {
                let Some(host) = host else {
                    return view().into_element();
                };
                let mounted = signal(true);
                let moving = stored(None::<Motion>);
                let pending = stored(None::<Spawned<()>>);
                let this = host.clone();
                let showing =
                    computed(move || current.with(|l| l.as_deref() == Some(this.as_str())));
                effect(move || {
                    let show = showing.get();
                    if let Some(timer) = pending.replace(None) {
                        timer.cancel();
                    }
                    let m = moving.get().filter(|_| !reduced);
                    match (show, m) {
                        (true, Some(m)) => {
                            mounted.set(true);
                            drop(m.play(chip_enter()));
                        }
                        (true, None) => mounted.set(true),
                        (false, Some(m)) => {
                            drop(m.play(chip_leave()));
                            pending.replace(Some(set_timeout(
                                Timer::from_millis(Duration::Base.nominal_ms() as u64),
                                move || mounted.set(false),
                            )));
                        }
                        (false, None) => mounted.set(false),
                    }
                });
                let label = t!("Open %@", arg1 = host);
                when(
                    move || mounted.get(),
                    move || {
                        let mut chip = view()
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
                            .shadow([BoxShadow {
                                offset_x: px(0.0),
                                offset_y: px(SHADOW_LG.y),
                                blur: px(SHADOW_LG.blur),
                                spread: px(0.0),
                                color: color(token("shadow-lg")).into(),
                                inset: false,
                            }])
                            .role(AccessibilityRole::Link)
                            .label(label.clone())
                            .test_id("terminal-open-link")
                            .active(|s| s.bg(move || token("accent")))
                            .on_press(ctx.asking(Intent::OpenLink))
                            .child(icon("link").size(IconSize::Sm).tone(Tone::Primary))
                            .child(
                                chrome_text(TextToken::Subhead, label.clone())
                                    .number_of_lines(1u32)
                                    .flex_shrink(1.0)
                                    .text_color(move || token("popover-foreground")),
                            );
                        if !reduced {
                            let m = motion().entering(chip_enter());
                            moving.replace(Some(m));
                            chip = chip.bind_motion(&m);
                        }
                        chip
                    },
                )
                .into_element()
            })
        }))
        .into_element()
}
