//! Keys and chips (`docs/nsh/UI-DESIGN.md` §4.17): [`Key`], a keycap of the
//! terminal's shortcut bar (and of Settings' preview of it), and [`Chip`], a
//! small choice in Settings (a shortcut, a host). Both are `nsh_base`'s
//! button (presses, the disabled and selected states, the role and name
//! screen readers hear, the keyboard's focus) in the app's look.
//!
//! - A key is drawn 36 pt tall and at least 44 pt wide, `radius-sm`, its
//!   label in `mono-key` (capped at xLarge, as the terminal's chrome is),
//!   a symbol in the same size and weight ([`IconSize::Key`]): a key that
//!   draws a symbol reads as heavy as one that spells its name. A face its
//!   caller draws ([`content_key`]) uses that size too. Its hit slop makes
//!   it 44 pt to the touch. Pressed, it takes the
//!   `key-pressed` fill and shrinks to 0.95 (only the fill under reduced
//!   motion). An armed modifier (a ⌃ waiting for its key) wears the
//!   accent's wash and ink, and screen readers hear it selected; the page
//!   says when it is armed and when it lets go (a second press, or a
//!   combination sent).
//! - A key reports its press going down and coming up
//!   ([`Key::on_press_down`], [`Key::on_press_up`]): a held key's repeat
//!   (after 0.3 s, every 70 ms) is the bar's to run.
//! - Screen readers say a key's words or its symbol's label; a key that
//!   means more than it shows (`ctrl`, which is "Control", and "Control
//!   armed" while armed) says so ([`Key::accessibility_label`]). A key whose
//!   face moves (the keyboard key's two glyphs swapping) is drawn by its
//!   caller ([`content_key`]).
//! - A key or a chip given no press handler is display only (Settings'
//!   preview of the bar, a shortcut's keycap in its row): drawn the same,
//!   it takes no press and no keyboard focus, has no pressed look, and is
//!   no element for screen readers, as the row or preview around it says
//!   what it shows.
//! - A chip is drawn 28 pt tall, `radius-sm`, and 44 pt to the touch.
//!   `Custom` is the user's own (the accent's wash); a `Toggle` chip, when
//!   selected, takes the wash and a check, and screen readers hear it
//!   selected.

use std::rc::Rc;

use nsh_core::{Points, prelude::*};

use crate::ui::{
    feedback::touch_target,
    icon::{IconSize, Tone, icon},
    motion::{Duration, SETTLE_CURVE, scale},
    tokens::{number, token},
    typography::TextToken,
};

/// How tall a key is drawn, and the least it is wide.
pub const KEY_HEIGHT: f32 = 36.0;
/// The least a key is wide.
pub const KEY_MIN_WIDTH: f32 = 44.0;
/// How tall a chip is drawn.
pub const CHIP_HEIGHT: f32 = 28.0;

/// What a key is (§4.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyVariant {
    /// A key that sends what it says (`esc`, `tab`, `^C`).
    Default,
    /// A modifier (⌃, ⌥): armed until its key is pressed.
    Modifier,
    /// A key that does something on the phone (an image, a folder, the
    /// mic): a symbol.
    Special,
    /// The keyboard's own key, pinned at the bar's trailing end: a symbol.
    Toggle,
}

/// What a key shows.
enum Face {
    Label(SharedString),
    Symbol {
        name: &'static str,
        label: SharedString,
    },
    /// Drawn by the caller; `label` is what screen readers say.
    Content {
        content: Box<AnyElement>,
        label: SharedString,
    },
}

/// A keycap ([`key`], [`symbol_key`], [`content_key`]).
pub struct Key {
    face: Face,
    variant: KeyVariant,
    armed: Rc<dyn Fn() -> bool>,
    disabled: Rc<dyn Fn() -> bool>,
    on_press: Option<Rc<dyn Fn()>>,
    on_press_down: Option<Rc<dyn Fn()>>,
    on_press_up: Option<Rc<dyn Fn()>>,
    spoken: Option<Rc<dyn Fn() -> SharedString>>,
    test_id: Option<SharedString>,
}

/// A key labeled `label`, which is also what screen readers say.
pub fn key(label: impl Into<SharedString>) -> Key {
    Key::new(Face::Label(label.into()), KeyVariant::Default)
}

/// A `Special` key showing the SF Symbol `name`; `label` is what screen
/// readers say for it, which every symbol key has.
pub fn symbol_key(name: &'static str, label: impl Into<SharedString>) -> Key {
    Key::new(
        Face::Symbol {
            name,
            label: label.into(),
        },
        KeyVariant::Special,
    )
}

/// A `Special` key showing `content`, which its caller draws (a face that
/// moves, such as the keyboard key's glyphs swapping); `label` is what
/// screen readers say for it.
pub fn content_key(content: impl IntoElement, label: impl Into<SharedString>) -> Key {
    Key::new(
        Face::Content {
            content: Box::new(content.into_element()),
            label: label.into(),
        },
        KeyVariant::Special,
    )
}

impl Key {
    fn new(face: Face, variant: KeyVariant) -> Key {
        Key {
            face,
            variant,
            armed: Rc::new(|| false),
            disabled: Rc::new(|| false),
            on_press: None,
            on_press_down: None,
            on_press_up: None,
            spoken: None,
            test_id: None,
        }
    }

    pub fn variant(mut self, variant: KeyVariant) -> Self {
        self.variant = variant;
        self
    }

    /// A modifier waiting for its key: the accent's wash and ink, and
    /// selected for screen readers.
    pub fn armed(mut self, armed: impl Fn() -> bool + 'static) -> Self {
        self.armed = Rc::new(armed);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// What a press does. A key given none of `on_press`,
    /// [`Self::on_press_down`] and [`Self::on_press_up`] is display only
    /// (the module's docs).
    pub fn on_press(mut self, on_press: impl Fn() + 'static) -> Self {
        self.on_press = Some(Rc::new(on_press));
        self
    }

    /// A press went down on the key (a held key's repeat starts here).
    pub fn on_press_down(mut self, on_press_down: impl Fn() + 'static) -> Self {
        self.on_press_down = Some(Rc::new(on_press_down));
        self
    }

    /// The press ended: released, moved away or cancelled.
    pub fn on_press_up(mut self, on_press_up: impl Fn() + 'static) -> Self {
        self.on_press_up = Some(Rc::new(on_press_up));
        self
    }

    /// What screen readers say for the key, in place of its words or its
    /// symbol's label; read as it changes (an armed `ctrl`). A display-only
    /// key says nothing: its row does.
    pub fn accessibility_label(mut self, label: impl Fn() -> SharedString + 'static) -> Self {
        self.spoken = Some(Rc::new(label));
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Key {
    fn into_element(self) -> AnyElement {
        let Key {
            face,
            variant,
            armed,
            disabled,
            on_press,
            on_press_down,
            on_press_up,
            spoken,
            test_id,
        } = self;
        let reduced = reduce_motion();
        let (content, label) = match face {
            Face::Label(words) => {
                let armed = armed.clone();
                let words = TextToken::MonoKey
                    .apply(text(words))
                    .text_color(move || {
                        token(if armed() {
                            "primary-text"
                        } else {
                            "key-foreground"
                        })
                    })
                    .number_of_lines(1u32)
                    .into_element();
                (words, None)
            }
            Face::Symbol { name, label } => (
                icon(name)
                    .size(IconSize::Key)
                    .tone(Tone::Key)
                    .into_element(),
                Some(label),
            ),
            Face::Content { content, label } => (*content, Some(label)),
        };
        // A `Toggle` key is drawn as any symbol key; its pinned place, the
        // hairline before it and the fade under it are the bar's (R12).
        let _ = variant;
        if on_press.is_none() && on_press_down.is_none() && on_press_up.is_none() {
            // Display only: drawn the same; no press, no focus, no pressed
            // look, nothing for screen readers.
            let mut cap = key_look(view(), armed, disabled)
                .accessibility_elements_hidden(true)
                .child(content);
            if let Some(id) = &test_id {
                cap = cap.test_id(id);
            }
            return cap.into_element();
        }
        let mut control = key_look(nsh_base::button(), armed.clone(), disabled.clone())
            .selected(move || armed())
            .disabled(move || disabled())
            .hit_slop(slop(KEY_HEIGHT, false))
            .on_press(fire(on_press))
            .on_press_down(fire(on_press_down))
            .on_press_up(fire(on_press_up))
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
                let s = if reduced {
                    s
                } else {
                    s.scale(scale::PRESS_ICON)
                };
                s.bg(move || token("key-pressed"))
            });
        match (spoken, label) {
            (Some(spoken), _) => control = control.accessibility_label(move || Some(spoken())),
            (None, Some(label)) => control = control.accessibility_label(label),
            (None, None) => {}
        }
        if let Some(id) = &test_id {
            control = control.test_id(id);
        }
        control.child(content).into_element()
    }
}

/// A keycap's look, pressable or not: 36 pt tall, at least 44 wide, the
/// key's fill (the accent's wash while armed), a stroke in light, dimmed
/// while disabled.
fn key_look<E: Styled>(cap: E, armed: Rc<dyn Fn() -> bool>, disabled: Rc<dyn Fn() -> bool>) -> E {
    cap.flex_row()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .min_h(px(KEY_HEIGHT))
        .min_w(px(KEY_MIN_WIDTH))
        .px(px(10.0))
        .rounded(px(number("radius-sm")))
        .border_curve(BorderCurve::Continuous)
        // In light, a white cap on the bar is set off by a stroke.
        .border(px(1.0))
        .border_color(move || {
            let stroke = token("border-strong");
            AdaptiveColor {
                light: stroke.light,
                dark: AdaptiveColor::TRANSPARENT.dark,
            }
        })
        .bg(move || token(if armed() { "primary-soft" } else { "key" }))
        .opacity(move || if disabled() { 0.4_f32 } else { 1.0 })
}

/// A press handler that may be missing (a key that reports only some of
/// its press).
fn fire(handler: Option<Rc<dyn Fn()>>) -> impl Fn() + 'static {
    move || {
        if let Some(handler) = &handler {
            handler();
        }
    }
}

/// What a chip is (§4.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipVariant {
    /// A built-in, neutral.
    Default,
    /// The user's own: the accent's wash.
    Custom,
    /// A choice that is on or off: the wash and a check when selected.
    Toggle,
}

/// A small choice ([`chip`]).
pub struct Chip {
    label: SharedString,
    variant: ChipVariant,
    selected: Rc<dyn Fn() -> bool>,
    mono: bool,
    symbol: Option<&'static str>,
    disabled: Rc<dyn Fn() -> bool>,
    on_press: Option<Rc<dyn Fn()>>,
    test_id: Option<SharedString>,
}

/// A neutral chip labeled `label`.
pub fn chip(label: impl Into<SharedString>) -> Chip {
    Chip {
        label: label.into(),
        variant: ChipVariant::Default,
        selected: Rc::new(|| false),
        mono: false,
        symbol: None,
        disabled: Rc::new(|| false),
        on_press: None,
        test_id: None,
    }
}

impl Chip {
    pub fn variant(mut self, variant: ChipVariant) -> Self {
        self.variant = variant;
        self
    }

    /// A `Toggle` chip that is on.
    pub fn selected(mut self, selected: impl Fn() -> bool + 'static) -> Self {
        self.selected = Rc::new(selected);
        self
    }

    /// Set in SF Mono: what a machine names (a key's label, a host).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Shows the SF Symbol `name` in place of its label (a shortcut the bar
    /// draws as a symbol: the mic, an image); screen readers say the label.
    pub fn symbol(mut self, name: &'static str) -> Self {
        self.symbol = Some(name);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// What a press does. A chip given none is display only (the
    /// module's docs).
    pub fn on_press(mut self, on_press: impl Fn() + 'static) -> Self {
        self.on_press = Some(Rc::new(on_press));
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Chip {
    fn into_element(self) -> AnyElement {
        let Chip {
            label,
            variant,
            selected,
            mono,
            symbol,
            disabled,
            on_press,
            test_id,
        } = self;
        let reduced = reduce_motion();
        // On: the accent's wash and ink (a `Custom` chip always is).
        let washed: Rc<dyn Fn() -> bool> = {
            let selected = selected.clone();
            Rc::new(move || match variant {
                ChipVariant::Default => false,
                ChipVariant::Custom => true,
                ChipVariant::Toggle => selected(),
            })
        };
        let words = match symbol {
            Some(name) => {
                let washed = washed.clone();
                dynamic(move || {
                    let tone = if washed() {
                        Tone::Primary
                    } else {
                        Tone::Foreground
                    };
                    untracked(move || icon(name).size(IconSize::Sm).tone(tone))
                })
                .into_element()
            }
            None => if mono {
                TextToken::MonoSubhead
            } else {
                TextToken::Subhead
            }
            .apply(text(label.clone()))
            .text_color({
                let washed = washed.clone();
                move || {
                    token(if washed() {
                        "primary-text"
                    } else {
                        "secondary-foreground"
                    })
                }
            })
            .number_of_lines(1u32)
            .into_element(),
        };
        let check = {
            let selected = selected.clone();
            when(
                move || variant == ChipVariant::Toggle && selected(),
                || icon("checkmark").size(IconSize::Sm).tone(Tone::Primary),
            )
        };
        let Some(on_press) = on_press else {
            // Display only: drawn the same; no press, no focus, no pressed
            // look, nothing for screen readers.
            let mut chip = chip_look(view(), washed, disabled)
                .accessibility_elements_hidden(true)
                .child(check)
                .child(words);
            if let Some(id) = &test_id {
                chip = chip.test_id(id);
            }
            return chip.into_element();
        };
        let mut control = chip_look(nsh_base::button(), washed, disabled.clone())
            .selected(move || variant == ChipVariant::Toggle && selected())
            .disabled(move || disabled())
            .hit_slop(slop(CHIP_HEIGHT, true))
            .on_press(move || on_press())
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
                let s = if reduced { s } else { s.scale(scale::PRESS) };
                s.opacity(0.75)
            });
        if symbol.is_some() {
            control = control.accessibility_label(label);
        }
        if let Some(id) = &test_id {
            control = control.test_id(id);
        }
        control.child(check).child(words).into_element()
    }
}

/// A chip's look, pressable or not: 28 pt tall, `secondary` (the accent's
/// wash while `washed`), dimmed while disabled.
fn chip_look<E: Styled>(
    chip: E,
    washed: Rc<dyn Fn() -> bool>,
    disabled: Rc<dyn Fn() -> bool>,
) -> E {
    chip.flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(4.0))
        .min_h(px(CHIP_HEIGHT))
        .px(px(10.0))
        .rounded(px(number("radius-sm")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || {
            token(if washed() {
                "primary-soft"
            } else {
                "secondary"
            })
        })
        .opacity(move || if disabled() { 0.4_f32 } else { 1.0 })
}

/// The hit slop that makes a control drawn `drawn` points tall take presses
/// over the touch target: above and below, and, for a control as narrow as
/// a chip of one character can be (`sides`), on its sides too. A key is
/// at least 44 pt wide already.
fn slop(drawn: f32, sides: bool) -> EdgeInsets {
    let v = ((touch_target() - drawn) / 2.0).max(0.0);
    let h = if sides { v } else { 0.0 };
    EdgeInsets {
        top: Points(v),
        right: Points(h),
        bottom: Points(v),
        left: Points(h),
    }
}
