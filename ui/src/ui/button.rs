//! Buttons (`docs/nsh/UI-DESIGN.md` §4.2, §4.3): [`Button`] for a labeled
//! action, [`IconButton`] for a well-known symbol. Both are `nsh_base`'s
//! button (presses, the disabled state, the role and name screen readers
//! hear, the keyboard's focus) in the app's look.
//!
//! A variant says what the action means, a size sets height, padding, text,
//! icon and corner together, and the states are the component's own:
//!
//! - pressed: the whole control shrinks a little (0.97, an icon button
//!   0.95) on the press's settle and its fill deepens; under reduced motion
//!   only the fill changes;
//! - disabled: dimmed, no press, no press feedback;
//! - loading (a [`Button`]): its content fades out and a spinner takes its
//!   place, the width unchanged, and it takes no press (no double submit);
//! - keyboard focus: a 2 pt `ring` outline outside the control.
//!
//! A small control is drawn small but takes presses over the full touch
//! target (`touch-target`, 44 pt): its hit slop makes up the rest.

use std::rc::Rc;

use nsh_core::{GlassEffect, Points, prelude::*};

use crate::ui::{
    feedback::{SpinnerSize, spinner, touch_target},
    icon::{IconSize, Tone, icon},
    motion::{Duration, SETTLE_CURVE, scale},
    tokens::{number, token},
    typography::TextToken,
};

/// What a button's action means (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// The one default commitment of a decision area (a form's Save, a
    /// panel's Send).
    Primary,
    /// An ordinary visible action.
    Secondary,
    /// An action that needs a clear edge with less weight.
    Outline,
    /// A quiet, familiar action.
    Ghost,
    /// A destructive action outside a confirmation (which is the system
    /// alert's).
    Destructive,
    /// A text button in a bar (Cancel).
    Bar,
    /// A bar's commit (Done, Save).
    BarCommit,
}

impl Variant {
    /// Its fill at rest, a color token, if it has one.
    fn fill(self) -> Option<&'static str> {
        match self {
            Variant::Primary => Some("primary"),
            Variant::Secondary => Some("secondary"),
            Variant::Destructive => Some("destructive-soft"),
            _ => None,
        }
    }

    /// Its fill while pressed; `None`: it dims instead.
    fn pressed_fill(self) -> Option<&'static str> {
        match self {
            Variant::Primary => Some("primary-active"),
            Variant::Outline | Variant::Ghost => Some("accent"),
            _ => None,
        }
    }

    fn ink(self) -> Tone {
        match self {
            Variant::Primary => Tone::OnPrimary,
            Variant::Destructive => Tone::Destructive,
            Variant::Bar | Variant::BarCommit => Tone::Primary,
            _ => Tone::Foreground,
        }
    }

    fn weight(self) -> FontWeight {
        match self {
            Variant::Bar => FontWeight::NORMAL,
            _ => FontWeight::SEMIBOLD,
        }
    }

    fn in_bar(self) -> bool {
        matches!(self, Variant::Bar | Variant::BarCommit)
    }
}

/// A button's size (§4.2). The touch target is 44 pt at every size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    /// Drawn 32 pt tall.
    Sm,
    /// 44 pt.
    Md,
    /// 50 pt: an empty state's action, a page's commit.
    Lg,
}

impl Size {
    /// Its drawn height (its least: it grows with its text).
    pub fn height(self) -> f32 {
        match self {
            Size::Sm => 32.0,
            Size::Md => 44.0,
            Size::Lg => 50.0,
        }
    }

    fn padding(self) -> f32 {
        match self {
            Size::Sm => 12.0,
            Size::Md => 16.0,
            Size::Lg => 24.0,
        }
    }

    fn radius(self) -> &'static str {
        match self {
            Size::Sm => "radius-sm",
            Size::Md | Size::Lg => "radius",
        }
    }

    fn text(self) -> TextToken {
        match self {
            Size::Sm => TextToken::Subhead,
            Size::Md | Size::Lg => TextToken::Body,
        }
    }

    fn icon(self) -> IconSize {
        match self {
            Size::Sm => IconSize::Sm,
            Size::Md | Size::Lg => IconSize::Md,
        }
    }

    fn gap(self) -> f32 {
        match self {
            Size::Sm => 4.0,
            Size::Md | Size::Lg => 8.0,
        }
    }
}

/// The hit slop that makes a control `drawn` points tall (and `wide` wide)
/// take presses over the touch target.
fn slop(drawn: f32, wide: f32) -> EdgeInsets {
    let target = touch_target();
    let (v, h) = (
        ((target - drawn) / 2.0).max(0.0),
        ((target - wide) / 2.0).max(0.0),
    );
    EdgeInsets {
        top: Points(v),
        right: Points(h),
        bottom: Points(v),
        left: Points(h),
    }
}

/// A labeled button ([`button`]).
pub struct Button {
    label: SharedString,
    variant: Variant,
    size: Size,
    icon: Option<&'static str>,
    disabled: Rc<dyn Fn() -> bool>,
    loading: Rc<dyn Fn() -> bool>,
    on_press: Rc<dyn Fn()>,
    test_id: Option<SharedString>,
}

/// A secondary, middle-sized button saying `label`.
pub fn button(label: impl Into<SharedString>) -> Button {
    Button {
        label: label.into(),
        variant: Variant::Secondary,
        size: Size::Md,
        icon: None,
        disabled: Rc::new(|| false),
        loading: Rc::new(|| false),
        on_press: Rc::new(|| {}),
        test_id: None,
    }
}

impl Button {
    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// An SF Symbol before the label.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// While this holds, the button shows a spinner and takes no press.
    pub fn loading(mut self, loading: impl Fn() -> bool + 'static) -> Self {
        self.loading = Rc::new(loading);
        self
    }

    pub fn on_press(mut self, on_press: impl Fn() + 'static) -> Self {
        self.on_press = Rc::new(on_press);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Button {
    fn into_element(self) -> AnyElement {
        let Button {
            label,
            variant,
            size,
            icon: symbol,
            disabled,
            loading,
            on_press,
            test_id,
        } = self;
        let reduced = reduce_motion();
        let ink = variant.ink();
        let inert = {
            let (disabled, loading) = (disabled.clone(), loading.clone());
            move || disabled() || loading()
        };
        let padding = if variant.in_bar() {
            8.0
        } else {
            size.padding()
        };
        let height = if variant.in_bar() {
            44.0
        } else {
            size.height()
        };
        let mut content = view()
            .flex_row()
            .items_center()
            .gap(px(size.gap()))
            .opacity({
                let loading = loading.clone();
                move || if loading() { 0.0_f32 } else { 1.0 }
            });
        if let Some(name) = symbol {
            content = content.child(icon(name).size(size.icon()).tone(ink));
        }
        content = content.child(
            size.text()
                .apply(text(label.clone()))
                .font_weight(variant.weight())
                .text_color(move || token(ink.token())),
        );
        let mut control = nsh_base::button()
            // What it says is its name (a back button names the page under it).
            .accessibility_label(label)
            .disabled(inert)
            .hit_slop(slop(height, 0.0))
            .on_press(move || on_press())
            .relative()
            .flex_row()
            .items_center()
            .justify_center()
            .min_h(px(height))
            .px(px(padding))
            .rounded(px(number(size.radius())))
            .border_curve(BorderCurve::Continuous)
            .opacity({
                let disabled = disabled.clone();
                move || if disabled() { 0.4_f32 } else { 1.0 }
            })
            .transition_all()
            .duration(Duration::Fast.ms())
            .ease_bezier(
                SETTLE_CURVE[0],
                SETTLE_CURVE[1],
                SETTLE_CURVE[2],
                SETTLE_CURVE[3],
            )
            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || token("ring")))
            .active(move |s| {
                // A bar's text button dims as the system bar's do (§4.18).
                if variant.in_bar() {
                    return s.opacity(BAR_PRESSED);
                }
                let s = if reduced { s } else { s.scale(scale::PRESS) };
                match variant.pressed_fill() {
                    Some(fill) => s.bg(move || token(fill)),
                    None => s.opacity(0.75),
                }
            });
        if let Some(fill) = variant.fill() {
            control = control.bg(move || token(fill));
        }
        if variant == Variant::Outline {
            control = control
                .border(px(1.0))
                .border_color(move || token("border-strong"));
        }
        if let Some(id) = &test_id {
            control = control.test_id(id);
        }
        control
            .child(content)
            .child(when(
                move || loading(),
                || {
                    view()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .child(spinner(SpinnerSize::Sm))
                },
            ))
            .into_element()
    }
}

/// An icon button's look (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconVariant {
    /// On glass, floating over content (a bar's buttons).
    Glass,
    /// No fill until pressed (a card's ⋯).
    Ghost,
    /// The accent's wash under the accent's symbol.
    Tinted,
    /// In a navigation bar (§4.18): on glass a glass circle, as
    /// [`Self::Glass`]; without glass no base (the bar's material is its
    /// ground) and the symbol in the accent, dimming when pressed as the
    /// system bar's buttons do.
    Bar,
}

/// What a bar's button dims to while pressed (§4.18: UIKit's bar).
const BAR_PRESSED: f32 = 0.2;

/// An icon button's size: drawn 32 or 44 pt square, 44 pt to the touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconButtonSize {
    Sm,
    Md,
}

impl IconButtonSize {
    pub fn side(self) -> f32 {
        match self {
            IconButtonSize::Sm => 32.0,
            IconButtonSize::Md => 44.0,
        }
    }

    fn icon(self) -> IconSize {
        match self {
            IconButtonSize::Sm => IconSize::Md,
            IconButtonSize::Md => IconSize::Lg,
        }
    }
}

/// A button that is a symbol ([`icon_button`]).
pub struct IconButton {
    symbol: &'static str,
    label: SharedString,
    variant: IconVariant,
    size: IconButtonSize,
    tone: Option<Tone>,
    disabled: Rc<dyn Fn() -> bool>,
    on_press: Rc<dyn Fn()>,
    test_id: Option<SharedString>,
}

/// A ghost, middle-sized button showing the SF Symbol `symbol`; `label` is
/// what screen readers say for it, which every icon button has.
pub fn icon_button(symbol: &'static str, label: impl Into<SharedString>) -> IconButton {
    IconButton {
        symbol,
        label: label.into(),
        variant: IconVariant::Ghost,
        size: IconButtonSize::Md,
        tone: None,
        disabled: Rc::new(|| false),
        on_press: Rc::new(|| {}),
        test_id: None,
    }
}

impl IconButton {
    pub fn variant(mut self, variant: IconVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: IconButtonSize) -> Self {
        self.size = size;
        self
    }

    /// The symbol in `tone` rather than its look's: what the symbol says
    /// (a red − takes a shortcut off the bar, a green ＋ puts it on).
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    pub fn on_press(mut self, on_press: impl Fn() + 'static) -> Self {
        self.on_press = Rc::new(on_press);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for IconButton {
    fn into_element(self) -> AnyElement {
        let IconButton {
            symbol,
            label,
            variant,
            size,
            tone: toned,
            disabled,
            on_press,
            test_id,
        } = self;
        let reduced = reduce_motion();
        let side = size.side();
        let glass = crate::nav_bar::glass();
        let tone = toned.unwrap_or(match variant {
            IconVariant::Glass => Tone::Foreground,
            IconVariant::Bar if glass => Tone::Foreground,
            IconVariant::Ghost => Tone::Muted,
            IconVariant::Tinted | IconVariant::Bar => Tone::Primary,
        });
        let mut control = nsh_base::button()
            .accessibility_label(label)
            .disabled({
                let disabled = disabled.clone();
                move || disabled()
            })
            .hit_slop(slop(side, side))
            .on_press(move || on_press())
            .w(px(side))
            .h(px(side))
            .flex_shrink_0()
            .rounded_full()
            .items_center()
            .justify_center()
            .opacity(move || if disabled() { 0.4_f32 } else { 1.0 })
            .transition_all()
            .duration(Duration::Fast.ms())
            .ease_bezier(
                SETTLE_CURVE[0],
                SETTLE_CURVE[1],
                SETTLE_CURVE[2],
                SETTLE_CURVE[3],
            )
            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || token("ring")))
            .active(move |s| {
                if variant == IconVariant::Bar && !glass {
                    return s.opacity(BAR_PRESSED);
                }
                let s = if reduced {
                    s
                } else {
                    s.scale(scale::PRESS_ICON)
                };
                match variant {
                    IconVariant::Ghost => s.bg(move || token("accent")),
                    // A glass circle only scales (§4.18).
                    IconVariant::Bar => s,
                    _ => s.opacity(0.75),
                }
            });
        control = match variant {
            IconVariant::Glass | IconVariant::Bar if glass => {
                control.glass_effect(GlassEffect::Regular)
            }
            IconVariant::Bar => control,
            IconVariant::Glass => control
                .bg(move || token("chrome"))
                .backdrop_blur(px(20.0))
                .border(px(0.5))
                .border_color(move || token("border-strong")),
            IconVariant::Ghost => control,
            IconVariant::Tinted => control.bg(move || token("primary-soft")),
        };
        if let Some(id) = &test_id {
            control = control.test_id(id);
        }
        control
            .child(icon(symbol).size(size.icon()).tone(tone))
            .into_element()
    }
}
