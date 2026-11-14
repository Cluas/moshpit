//! mosh's live counters (`MoshDiagnosticsView`), in a popover hung from
//! the transport pill while the host shows them (the pill held, over
//! mosh): what came in, what was applied, what went out and whether the
//! server acknowledged it, and a button that has the server repaint from
//! blank. It is meant to be screenshotted and sent back, so its counter
//! names stay as they are in every language.
//!
//! The popover is drawn here (NSH's overlays place it under the pill and
//! keep it on screen), in UIKit's shape: a rounded box on the popover's
//! material with its arrow at the pill's middle, fading in and out. A
//! press anywhere outside it closes it and goes no further, as a popover's
//! does.

use std::time::Duration;

use nsh_core::prelude::*;

use super::{Ctx, Intent, model::Counters, styled};
use crate::{face::Design, ui::tokens::token};

/// UIKit's popover: its corner radius, its arrow's height and base, the
/// room it keeps from the screen's edges, and its fade.
const RADIUS: f32 = 13.0;
const ARROW_HEIGHT: f32 = 13.0;
const ARROW_BASE: f32 = 36.0;
const MARGIN: f32 = 10.0;
const FADE: Duration = Duration::from_millis(200);
/// The popover's background: the `popover` surface, in the appearance
/// the screen draws (UIKit's popover was #1C1C1E over a dark terminal).
fn material() -> AdaptiveColor {
    token("popover")
}

/// The popover, while the host shows it.
pub(crate) fn popover(ctx: Ctx) -> AnyElement {
    let shown = {
        let ctx = ctx.clone();
        computed(move || ctx.diagnostics.with(|d| d.shown))
    };
    // It stays while it fades out.
    let present = signal(false);
    let opacity = animated_value(0.0);
    let pending = stored(None::<Spawned<()>>);
    effect(move || {
        let shown = shown.get();
        if let Some(timer) = pending.replace(None) {
            timer.cancel();
        }
        if shown {
            present.set(true);
            drop(
                Animation::timing(
                    &opacity,
                    1.0,
                    Timing::new(FADE.as_millis() as f32, Easing::EaseOut),
                )
                .start(),
            );
        } else if present.get_untracked() {
            drop(
                Animation::timing(
                    &opacity,
                    0.0,
                    Timing::new(FADE.as_millis() as f32, Easing::EaseIn),
                )
                .start(),
            );
            pending.replace(Some(set_timeout(FADE, move || present.set(false))));
        }
    });
    when(
        move || present.get(),
        move || shown_popover(ctx.clone(), shown, opacity),
    )
    .into_element()
}

fn shown_popover(ctx: Ctx, shown: Memo<bool>, opacity: AnimatedValue) -> AnyElement {
    let dismiss = {
        let ctx = ctx.clone();
        move || {
            if shown.get_untracked() {
                ctx.ask(Intent::DiagnosticsDismiss);
            }
        }
    };
    // Where the box went: its arrow points at the pill's middle.
    let placed = signal(None::<Bounds<Points>>);
    let arrow_x = {
        let ctx = ctx.clone();
        computed(move || {
            let (Some(pill), Some(placed)) = (ctx.pill_rect.get(), placed.get()) else {
                return 0.0;
            };
            let middle = pill.origin.x.0 + pill.size.width.0 / 2.0;
            (middle - placed.origin.x.0 - ARROW_BASE / 2.0).clamp(
                RADIUS,
                (placed.size.width.0 - RADIUS - ARROW_BASE).max(RADIUS),
            )
        })
    };
    let counters = {
        let ctx = ctx.clone();
        computed(move || ctx.diagnostics.with(|d| d.counters.clone()))
    };
    let content = {
        let ctx = ctx.clone();
        dynamic(move || {
            let counters = counters.get();
            let ctx = ctx.clone();
            untracked(move || body(counters, ctx))
        })
    };
    view()
        .absolute()
        .inset_0()
        .child(
            // Takes the press outside the popover: it closes the popover
            // and reaches nothing under it.
            view()
                .absolute()
                .inset_0()
                .accessibility_elements_hidden(true)
                .on_press(dismiss.clone()),
        )
        .child(
            overlay()
                .anchor(&ctx.pill)
                .placement(Placement::Bottom)
                .align(Align::Center)
                .margin(px(MARGIN))
                .on_position(move |p: &OverlayPosition| {
                    if placed.get_untracked() != Some(p.bounds) {
                        placed.set(Some(p.bounds));
                    }
                })
                .on_cancel(dismiss)
                .bind_opacity(&opacity)
                .test_id("mosh-diagnostics-overlay")
                .child(
                    view().child(arrow(arrow_x)).child(
                        view()
                            .rounded(px(RADIUS))
                            .border_curve(BorderCurve::Continuous)
                            .overflow_hidden()
                            .bg(material)
                            .child(content),
                    ),
                ),
        )
        .into_element()
}

/// The arrow over the box, at `x`: a square turned a quarter, its lower
/// half hidden under the box.
fn arrow(x: Memo<f32>) -> AnyElement {
    let side = ARROW_BASE / std::f32::consts::SQRT_2;
    view()
        .h(px(ARROW_HEIGHT))
        .w_full()
        .overflow_hidden()
        .child(
            view()
                .absolute()
                .left(move || px(x.get() + (ARROW_BASE - side) / 2.0))
                .top(px(ARROW_HEIGHT - side / 2.0))
                .w(px(side))
                .h(px(side))
                .rounded(px(3.0))
                .bg(material)
                .rotate(deg(45.0)),
        )
        .into_element()
}

/// A counter's row: its name muted, its value in bold, in `value_color`
/// (a token's name).
fn row(label: &str, value: String, value_color: &'static str) -> AnyElement {
    view()
        .flex_row()
        .items_center()
        .child(mono(
            label.to_owned(),
            FontWeight::NORMAL,
            "muted-foreground",
        ))
        .child(view().flex_1().min_w(px(20.0)))
        .child(mono(value, FontWeight::BOLD, value_color))
        .into_element()
}

/// `Face.mono(12)` at `weight` in the token `color`.
fn mono(content: String, weight: FontWeight, color: &'static str) -> Text {
    super::line(content, Design::Monospaced, 12.0, weight).text_color(move || token(color))
}

/// The counters and the Repaint button.
fn body(counters: Option<Counters>, ctx: Ctx) -> AnyElement {
    let mut rows = view().gap(px(6.0)).p(px(14.0)).child(
        super::line(
            t!("MOSH DIAGNOSTICS"),
            Design::Monospaced,
            10.0,
            FontWeight::BOLD,
        )
        .letter_spacing(px(0.6))
        .text_color(move || token("muted-foreground")),
    );
    match counters {
        Some(c) => {
            for (label, value) in [
                ("datagrams", c.datagrams),
                ("applied", c.applied),
                ("parse fails", c.parse_fails),
                ("gaps", c.gaps),
                ("sent", c.sent),
                ("user", c.user),
                ("acked", c.acked),
            ] {
                rows = rows.child(row(label, value.to_string(), "foreground"));
            }
            let ready = c.channel == "ready";
            rows = rows.child(row(
                "channel",
                c.channel,
                if ready { "foreground" } else { "warning-text" },
            ));
            if c.held {
                rows = rows.child(mono(
                    t!("input held until the server answers"),
                    FontWeight::BOLD,
                    "warning-text",
                ));
            }
        }
        None => {
            rows = rows.child(mono(
                t!("No datagrams yet."),
                FontWeight::NORMAL,
                "foreground",
            ));
        }
    }
    view().child(rows).child(repaint(ctx)).into_element()
}

/// Repaint screen: the server rediffs from blank (a complete redraw).
fn repaint(ctx: Ctx) -> AnyElement {
    // `.safeAreaInset(edge: .bottom)`'s spacing above it: the system's 8.
    view()
        .px(px(14.0))
        .pt(px(8.0))
        .pb(px(12.0))
        .child(
            view()
                .w_full()
                .items_center()
                .py(px(8.0))
                .rounded(px(8.0))
                .border_curve(BorderCurve::Continuous)
                .bg(move || token("primary-soft"))
                .role(AccessibilityRole::Button)
                .test_id("mosh-diagnostics-repaint")
                .active(|s| s.opacity(0.6))
                .on_press(ctx.asking(Intent::Repaint))
                .child(
                    styled(
                        text(t!("Repaint screen")),
                        Design::Monospaced,
                        12.0,
                        FontWeight::BOLD,
                    )
                    .text_color(move || token("primary-text")),
                ),
        )
        .into_element()
}
