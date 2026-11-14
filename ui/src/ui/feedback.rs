//! The quiet parts that say something is going on, or separate things
//! (`docs/nsh/UI-DESIGN.md` §4.15, §4.19): [`spinner`], [`progress`] and
//! [`separator`].

use nsh_core::{color::Rgba, prelude::*};
use serde_json::json;

use crate::{
    theme::hex_string,
    ui::tokens::{number, token},
};

/// A spinner's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpinnerSize {
    /// 16 pt: in a row's accessory lane, in a small button.
    Sm,
    /// 20 pt: on its own (a list loading).
    Md,
}

impl SpinnerSize {
    pub fn points(self) -> f32 {
        match self {
            SpinnerSize::Sm => 16.0,
            SpinnerSize::Md => 20.0,
        }
    }
}

/// The system's activity indicator (the host's
/// `moshpit/activity-indicator`, which spins on its own and stops under
/// reduced motion as the system's does), in `muted-foreground`. Only for
/// a real wait: a change that is already done shows no spinner.
pub fn spinner(size: SpinnerSize) -> AnyElement {
    let points = size.points();
    view()
        .w(px(points))
        .h(px(points))
        .flex_shrink_0()
        .label(t!("Loading"))
        .accessibility_state(AccessibilityState {
            busy: true,
            ..AccessibilityState::default()
        })
        .child(dynamic(move || {
            let tint = Rgba::from(color(token("muted-foreground")));
            untracked(move || {
                native("moshpit/activity-indicator")
                    .w(px(points))
                    .h(px(points))
                    .prop("color", json!(hex_string(tint)))
                    // The system's indicator is 20 pt.
                    .prop("scale", json!(points / 20.0))
            })
        }))
        .into_element()
}

/// The system's progress bar (the host's `moshpit/progress`, UIKit's
/// `UIProgressView`), as wide as its box, for a download or an upload:
/// `value()` from 0 to 1, the bar `primary` on a `muted` track. Screen
/// readers hear `label()`.
pub fn progress(
    value: impl Fn() -> f64 + 'static,
    label: impl Fn() -> String + 'static,
) -> AnyElement {
    let hex = |name: &'static str| move || json!(hex_string(Rgba::from(color(token(name)))));
    native("moshpit/progress")
        .w_full()
        .prop("value", move || json!(value()))
        .prop("tint", hex("primary"))
        .prop("track", hex("muted"))
        .prop("label", move || json!(label()))
        .into_element()
}

/// A hairline (`border`): one physical pixel, drawn once by the boundary's
/// owner. `inset` keeps it off the leading edge (a row's separator starts
/// at its content).
pub fn separator(inset: f32) -> AnyElement {
    view()
        .h(move || px(1.0 / pixel_ratio().max(1.0)))
        .ms(px(inset))
        .flex_shrink_0()
        .bg(move || token("border"))
        .accessibility_elements_hidden(true)
        .into_element()
}

/// The touch target's least side (`touch-target`, 44 pt on iOS).
pub fn touch_target() -> f32 {
    number("touch-target")
}
