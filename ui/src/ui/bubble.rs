//! A speech bubble (moshpit-design `docs/nsh/PET-DESIGN.md` §17.3): what
//! the companion says, beside it, its tail pointing at it. It grows from
//! its tail and fades in (`fast`, the settle curve; reduced motion: the
//! fade alone), stays 3 s (4.5 s for a long line), then fades out (`base`,
//! ease-in). It takes no touches, so what's under it keeps them; screen
//! readers hear the line as an announcement, not from the bubble.

use std::time::Duration as Wait;

use nsh_core::prelude::*;

use crate::ui::{
    motion::{Duration, LEAVE, SETTLE},
    tokens::{SHADOW_SM, number, token},
    typography::TextToken,
};

/// How wide a bubble may grow; past it the line wraps, to two lines.
pub const MAX_WIDTH: f32 = 200.0;

/// A line longer than this (in characters) stays longer.
const LONG: usize = 24;

/// How long a line stays: 3 s, 4.5 s for a long one.
pub fn hold(line: &str) -> Wait {
    if line.chars().count() > LONG {
        Wait::from_millis(4500)
    } else {
        Wait::from_millis(3000)
    }
}

/// Which edge of the bubble its tail is on: the side of the companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tail {
    /// The companion is past its trailing edge (the bubble on its leading
    /// side, as it sits at a card's trailing end).
    #[default]
    Trailing,
    /// The companion is past its leading edge (moved toward a card's
    /// leading end, it speaks into the room on its trailing side).
    Leading,
}

/// The bubble for what `saying` holds (`(n, line)`: each line its number),
/// its trailing edge `gap` points from the companion it points at: placed
/// by the caller (absolute), its bottom level with the companion's middle.
/// It clears `saying` once the line has gone.
pub fn speech(saying: Signal<Option<(u64, String)>>) -> AnyElement {
    speech_toward(saying, Tail::Trailing)
}

/// [`speech`] with its tail on `tail`'s edge.
pub fn speech_toward(saying: Signal<Option<(u64, String)>>, tail: Tail) -> AnyElement {
    dynamic(move || match saying.get() {
        Some((n, line)) => untracked(move || bubble(saying, n, line, tail)),
        None => untracked(|| view().into_element()),
    })
    .into_element()
}

fn bubble(saying: Signal<Option<(u64, String)>>, n: u64, line: String, tail: Tail) -> AnyElement {
    let shown = animated_value(0.0);
    drop(shown.timing_with(1.0, Duration::Fast.timing(SETTLE)));
    // The line goes after its time, unless a newer one replaced it.
    drop(set_timeout(hold(&line), move || {
        if saying.with_untracked(|s| s.as_ref().map(|(m, _)| *m)) != Some(n) {
            return;
        }
        drop(shown.timing_with(0.0, Duration::Base.timing(LEAVE)));
        let gone = Wait::from_millis(Duration::Base.ms() as u64);
        drop(set_timeout(gone, move || {
            if saying.with_untracked(|s| s.as_ref().map(|(m, _)| *m)) == Some(n) {
                saying.set(None);
            }
        }));
    }));
    let mut bubble = view()
        .relative()
        .max_w(px(MAX_WIDTH))
        .px(px(11.0))
        .py(px(6.0))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("popover"))
        .border(px(0.5))
        .border_color(move || token("border-strong"))
        .shadow([BoxShadow {
            offset_x: px(0.0),
            offset_y: px(SHADOW_SM.y),
            blur: px(SHADOW_SM.blur),
            spread: px(0.0),
            color: color(token("shadow-sm")).into(),
            inset: false,
        }])
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true);
    // It grows from its tail.
    bubble = match tail {
        Tail::Trailing => bubble.origin_right(),
        Tail::Leading => bubble.origin_left(),
    };
    let tip = view().absolute();
    let tip = match tail {
        Tail::Trailing => tip.right(px(-4.0)),
        Tail::Leading => tip.left(px(-4.0)),
    };
    bubble = bubble
        .bind_opacity(&shown)
        .test_id("home-pet-says")
        // The tail: a small square turned 45°, half under the bubble's
        // edge on the companion's side.
        .child(
            tip.top(px(0.0)).bottom(px(0.0)).justify_center().child(
                view()
                    .w(px(8.0))
                    .h(px(8.0))
                    .rotate(Radians(std::f32::consts::FRAC_PI_4))
                    .bg(move || token("popover")),
            ),
        )
        .child(
            TextToken::Footnote
                .apply(text(line))
                .number_of_lines(2u32)
                .text_color(move || token("popover-foreground")),
        );
    if !reduce_motion() {
        bubble = bubble.bind_scale(&shown.interpolate(&[0.0, 1.0], &[0.9, 1.0]));
    }
    bubble.into_element()
}
