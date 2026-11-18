//! Form fields (`docs/nsh/UI-DESIGN.md` §4.4, used as §6.5 shows):
//! [`Field`], a label people can see and its control — a [`TextInput`] or
//! a [`NumberInput`] — with an optional one-line description and an error
//! that takes the description's place; and [`SecretField`], a password or
//! passphrase whose text never leaves the host's own secure field.
//!
//! The label leads its control in a row ([`FieldLayout::Inline`]: the
//! label's column at least 88 pt and as wide as the label needs) or stands
//! over it ([`FieldLayout::Stacked`]: paths, long values, and every inline
//! field from the first accessibility text size on). A field is at least
//! the touch target tall and grows with its text; a press anywhere on it
//! puts the keyboard in its control.
//!
//! States, the component's own:
//!
//! - focused: a 2 pt `ring` around the control (the row's fill stays);
//! - invalid ([`Field::error`]): the value in `destructive-text`, an
//!   `exclamationmark.circle` at the row's end and the error right under
//!   the control, rising 4 pt as it fades in (`fast`; at once under reduced
//!   motion). The pinned NSH has no `invalid` state for screen readers, so
//!   the error is said with the field's name;
//! - disabled: dimmed, it takes no focus and no edit; read only: no edit.
//!
//! Return moves to the next field ([`TextInput::next`]); a field with none
//! after it says Done and puts the keyboard away.
//!
//! The label, the description and the error are said through the control
//! (its name and description), not again on their own.
//!
//! A field whose text the page or the host sets (a picked file, a preset)
//! is made again when it does ([`remount`]).

use std::rc::Rc;

use nsh_core::{color::Rgba, prelude::*};
use serde_json::json;

use crate::{
    face,
    theme::hex_string,
    ui::{
        feedback::touch_target,
        icon::{Tone, icon},
        motion::{Duration, ENTER, distance},
        row::{GAP as ROW_GAP, PADDING_X, PADDING_Y},
        status::{BadgeVariant, badge},
        tokens::{number, token},
        typography::TextToken,
    },
};

/// The label column's least width in an inline field (§4.4).
pub const LABEL_COLUMN: f32 = 88.0;
/// A field's leading and trailing inset: a row's (§3.2).
const INSET: f32 = PADDING_X;
/// A field's top and bottom inset: a row's. With a body line (22 pt) it
/// makes the touch target (44 pt) at the default text size, a medium Row's
/// rhythm, and the field grows from there with its text.
const INSET_V: f32 = PADDING_Y;
/// Between a control's parts: a stacked label and its control, a control
/// and its note (`space-1`).
const PART_GAP: f32 = 4.0;
/// Between the label column and the control, and the control's lane
/// parts: a row's gap between lanes.
const GAP: f32 = ROW_GAP;
/// A number input's width in an inline field.
const NUMBER_WIDTH: f32 = 100.0;

/// Where a field's label goes (§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FieldLayout {
    /// The label leading, the control trailing: a field in a grouped row.
    /// From the first accessibility text size on it stands stacked.
    #[default]
    Inline,
    /// The label over the control: paths and long values.
    Stacked,
}

impl FieldLayout {
    /// Whether it stands stacked at `font_scale` (NSH's).
    pub fn stacked_at(self, font_scale: f32) -> bool {
        self == FieldLayout::Stacked || face::category(font_scale) >= face::AX1
    }
}

/// What a field hands its control when it lays it out.
#[derive(Clone)]
pub struct ControlState {
    /// What screen readers say for the control: the field's label, then its
    /// error while it has one.
    pub spoken: Rc<dyn Fn() -> SharedString>,
    /// The field's description, said after the name.
    pub description: Option<SharedString>,
    /// The field has an error: the value shows it.
    pub invalid: Rc<dyn Fn() -> bool>,
    /// The field is disabled: no focus, no edit.
    pub disabled: Rc<dyn Fn() -> bool>,
    /// The field is read only: no edit.
    pub readonly: Rc<dyn Fn() -> bool>,
    /// The field's focus: the control sets it as it gains and loses the
    /// keyboard, and the field draws the ring from it.
    pub focused: Signal<bool>,
    /// The ref a press on the field focuses: the control carries it.
    pub handle: ElementRef,
    /// The control's test id (`{field}-input`).
    pub test_id: Option<String>,
    /// The field stands stacked.
    pub stacked: bool,
}

/// A field's control: [`TextInput`] or [`NumberInput`].
pub trait FieldControl {
    /// The ref a press on the field focuses: the control's own when it was
    /// given one ([`TextInput::element_ref`]), else a new one.
    fn handle(&self) -> ElementRef;

    /// The control in the field's state. Called again when the field
    /// changes layout (crossing into or out of the accessibility sizes): a
    /// control starts from its [`TextInput::default_value`] each time.
    fn build(&self, state: &ControlState) -> AnyElement;
}

// MARK: - Text input

/// A one-line text field in the app's look ([`text_input`]).
#[derive(Clone)]
pub struct TextInput {
    start: Option<Rc<dyn Fn() -> String>>,
    placeholder: SharedString,
    mono: bool,
    number: bool,
    keyboard: KeyboardType,
    handle: Option<ElementRef>,
    next: Option<ElementRef>,
    on_change: Rc<dyn Fn(String)>,
    on_blur: Rc<dyn Fn()>,
    on_submit: Rc<dyn Fn()>,
    max_length: Option<u32>,
    /// A textarea's least and most lines ([`TextInput::multiline`]).
    lines: Option<(u32, u32)>,
    auto_focus: bool,
}

/// A text input, empty and in the body style.
pub fn text_input() -> TextInput {
    TextInput {
        start: None,
        placeholder: SharedString::default(),
        mono: false,
        number: false,
        keyboard: KeyboardType::Default,
        handle: None,
        next: None,
        on_change: Rc::new(|_| {}),
        on_blur: Rc::new(|| {}),
        on_submit: Rc::new(|| {}),
        max_length: None,
        lines: None,
        auto_focus: false,
    }
}

impl TextInput {
    /// The text it starts with, read each time it is laid out anew: the
    /// page's draft then, not the opening one.
    pub fn default_value(mut self, start: impl Fn() -> String + 'static) -> Self {
        self.start = Some(Rc::new(start));
        self
    }

    /// What it shows while empty: an example (`my-server`).
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set in SF Mono: what a machine names (a host, a user, a path).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn keyboard(mut self, keyboard: KeyboardType) -> Self {
        self.keyboard = keyboard;
        self
    }

    /// Puts `handle` on the input: what a press on its field focuses, and
    /// what the field before it moves to on Return.
    pub fn element_ref(mut self, handle: &ElementRef) -> Self {
        self.handle = Some(*handle);
        self
    }

    /// Return moves the keyboard to the field `next` names; without one the
    /// Return key says Done and puts the keyboard away.
    pub fn next(mut self, next: &ElementRef) -> Self {
        self.next = Some(*next);
        self
    }

    pub fn max_length(mut self, length: u32) -> Self {
        self.max_length = Some(length);
        self
    }

    /// Hears each edit.
    pub fn on_change(mut self, on_change: impl Fn(String) + 'static) -> Self {
        self.on_change = Rc::new(on_change);
        self
    }

    /// Hears the keyboard leave it (a page checks a value then, §6.5).
    pub fn on_blur(mut self, on_blur: impl Fn() + 'static) -> Self {
        self.on_blur = Rc::new(on_blur);
        self
    }

    /// A textarea (§4.4): prose over several lines, from `least` lines
    /// tall to `most` (then it scrolls), with the keyboard's corrections
    /// and capitals; Return starts a new line. Lay it out stacked.
    pub fn multiline(mut self, least: u32, most: u32) -> Self {
        self.lines = Some((least.max(1), most.max(least.max(1))));
        self
    }

    /// The keyboard comes up in it as the page opens.
    pub fn auto_focus(mut self) -> Self {
        self.auto_focus = true;
        self
    }

    /// Hears Return.
    pub fn on_submit(mut self, on_submit: impl Fn() + 'static) -> Self {
        self.on_submit = Rc::new(on_submit);
        self
    }

    fn style(&self) -> TextToken {
        if self.mono {
            TextToken::MonoSubhead
        } else {
            TextToken::Body
        }
    }
}

/// An input's height: its text's line. The field around it takes the
/// presses (over at least the touch target) and puts them in the input.
fn input_height(style: TextToken) -> f32 {
    face::frame(style.size(font_scale()), pixel_ratio())
}

/// The least height inside a field's insets: the touch target less them.
fn lane_height() -> f32 {
    touch_target() - 2.0 * INSET_V
}

impl FieldControl for TextInput {
    fn handle(&self) -> ElementRef {
        self.handle.unwrap_or_else(element_ref)
    }

    fn build(&self, state: &ControlState) -> AnyElement {
        let style = self.style();
        let ControlState {
            spoken,
            description,
            invalid,
            disabled,
            readonly,
            focused,
            handle,
            test_id,
            stacked,
        } = state.clone();
        let start = self
            .start
            .as_ref()
            .map(|start| untracked(|| start()))
            .unwrap_or_default();
        let (on_change, on_blur, on_submit) = (
            self.on_change.clone(),
            self.on_blur.clone(),
            self.on_submit.clone(),
        );
        let next = self.next;
        let lines = self.lines;
        let mut field = nsh_core::element::input()
            .min_w(px(0.0))
            .min_h(move || px(input_height(style)))
            .default_value(start)
            .placeholder(self.placeholder.to_string())
            .placeholder_color(move || color(token("subtle-foreground")))
            // An input follows NSH's font scale on its own (it cannot opt
            // out): it is given the style's size at the default text size.
            .text_size(px(style.size(1.0)))
            .line_height(face::line_height())
            .text_color(move || {
                token(if invalid() {
                    "destructive-text"
                } else {
                    "foreground"
                })
            })
            .caret_color(move || color(token("primary")))
            .keyboard_appearance(move || {
                if dark() {
                    KeyboardAppearance::Dark
                } else {
                    KeyboardAppearance::Light
                }
            })
            .keyboard_type(self.keyboard)
            .auto_correct(false)
            .auto_capitalize(AutoCapitalize::None)
            .disabled({
                let disabled = disabled.clone();
                move || disabled()
            })
            // Disabled takes no edit wherever the platform would still
            // let one through.
            .readonly(move || readonly() || disabled())
            .label(move || spoken())
            .element_ref(&handle)
            .on_focus(move || focused.set(true))
            .on_blur(move || {
                focused.set(false);
                on_blur();
            })
            .on_change(move |text: &str| on_change(text.to_owned()));
        if self.auto_focus {
            field = field.auto_focus(true);
        }
        if let Some((least, most)) = lines {
            // A textarea: its lines' height, the first one's frame and the
            // leading of the rest; Return is a new line.
            let tall = move |n: u32| {
                let scale = font_scale();
                let size = style.size(scale);
                (n.saturating_sub(1)) as f32 * face::leading(style.size(1.0), scale)
                    + face::frame(size, pixel_ratio())
            };
            let mut area = field
                .flex_1()
                .multiline(true)
                .scrollbar(false)
                .auto_correct(true)
                .auto_capitalize(AutoCapitalize::Sentences)
                .min_h(move || px(tall(least)))
                .max_h(move || px(tall(most)));
            if let Some(description) = description {
                area = area.accessibility_description(description);
            }
            if let Some(id) = test_id {
                area = area.test_id(&id);
            }
            return area.into_element();
        }
        field = match next {
            Some(next) => field
                .return_key_type(ReturnKeyType::Next)
                .submit_behavior(SubmitBehavior::Submit)
                .on_submit(move |_: &str| {
                    on_submit();
                    next.focus();
                }),
            None => field
                .return_key_type(ReturnKeyType::Done)
                .on_submit(move |_: &str| on_submit()),
        };
        if self.mono {
            field = field.font_family("monospace");
        }
        if self.number {
            field = field.tabular_nums().text_right();
            field = if stacked {
                field.flex_1()
            } else {
                field.w(px(NUMBER_WIDTH))
            };
        } else {
            field = field.flex_1();
        }
        if let Some(length) = self.max_length {
            field = field.max_length(length);
        }
        if let Some(description) = description {
            field = field.accessibility_description(description);
        }
        if let Some(id) = &test_id {
            field = field.test_id(id);
        }
        field.into_element()
    }
}

// MARK: - Number input

/// A number in a text field ([`number_input`]): a port, a count.
#[derive(Clone)]
pub struct NumberInput(TextInput);

/// A number input: the number keyboard, the digits in SF Mono, of one
/// width and at the trailing end; 100 pt wide in an inline field. Whether
/// the number is a good one is the page's ([`Field::error`]).
pub fn number_input() -> NumberInput {
    let mut input = text_input().mono().keyboard(KeyboardType::Number);
    input.number = true;
    NumberInput(input)
}

impl NumberInput {
    /// As [`TextInput::default_value`].
    pub fn default_value(self, start: impl Fn() -> String + 'static) -> Self {
        NumberInput(self.0.default_value(start))
    }

    pub fn placeholder(self, placeholder: impl Into<SharedString>) -> Self {
        NumberInput(self.0.placeholder(placeholder))
    }

    pub fn element_ref(self, handle: &ElementRef) -> Self {
        NumberInput(self.0.element_ref(handle))
    }

    pub fn next(self, next: &ElementRef) -> Self {
        NumberInput(self.0.next(next))
    }

    pub fn max_length(self, length: u32) -> Self {
        NumberInput(self.0.max_length(length))
    }

    pub fn on_change(self, on_change: impl Fn(String) + 'static) -> Self {
        NumberInput(self.0.on_change(on_change))
    }

    pub fn on_blur(self, on_blur: impl Fn() + 'static) -> Self {
        NumberInput(self.0.on_blur(on_blur))
    }

    pub fn on_submit(self, on_submit: impl Fn() + 'static) -> Self {
        NumberInput(self.0.on_submit(on_submit))
    }
}

impl FieldControl for NumberInput {
    fn handle(&self) -> ElementRef {
        self.0.handle()
    }

    fn build(&self, state: &ControlState) -> AnyElement {
        self.0.build(state)
    }
}

// MARK: - Field

/// A label and its control ([`field`]).
pub struct Field<C> {
    label: SharedString,
    control: C,
    layout: FieldLayout,
    description: Option<SharedString>,
    error: Rc<dyn Fn() -> Option<SharedString>>,
    disabled: Rc<dyn Fn() -> bool>,
    readonly: Rc<dyn Fn() -> bool>,
    test_id: Option<SharedString>,
    show_focus: bool,
}

/// An inline field labeled `label` (always shown) around `control`.
pub fn field<C: FieldControl>(label: impl Into<SharedString>, control: C) -> Field<C> {
    Field {
        label: label.into(),
        control,
        layout: FieldLayout::Inline,
        description: None,
        error: Rc::new(|| None),
        disabled: Rc::new(|| false),
        readonly: Rc::new(|| false),
        test_id: None,
        show_focus: false,
    }
}

impl<C> Field<C> {
    pub fn layout(mut self, layout: FieldLayout) -> Self {
        self.layout = layout;
        self
    }

    /// One line under the control saying what the field takes.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// While this says something, the field is invalid and shows it in the
    /// description's place. When to say it is the page's (§6.5: on leaving
    /// the field or on Save, never while typing).
    pub fn error(mut self, error: impl Fn() -> Option<SharedString> + 'static) -> Self {
        self.error = Rc::new(error);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    pub fn readonly(mut self, readonly: impl Fn() -> bool + 'static) -> Self {
        self.readonly = Rc::new(readonly);
        self
    }

    /// The field is `id`; its control `{id}-input`, its label
    /// `{id}-label`, its error `{id}-error`, its focus ring `{id}-ring`.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }

    /// Draws the focus ring without the focus: the gallery's still picture
    /// of the focused state (a real focus would raise the keyboard there).
    pub(crate) fn show_focus(mut self) -> Self {
        self.show_focus = true;
        self
    }
}

/// `label`, then `error` while there is one: what screen readers say for a
/// field's control.
fn spoken(
    label: SharedString,
    error: Rc<dyn Fn() -> Option<SharedString>>,
) -> Rc<dyn Fn() -> SharedString> {
    Rc::new(move || match error() {
        Some(error) => t!("%@, %@", arg1 = label, arg2 = error).into(),
        None => label.clone(),
    })
}

impl<C: FieldControl + 'static> IntoElement for Field<C> {
    fn into_element(self) -> AnyElement {
        let Field {
            label,
            control,
            layout,
            description,
            error,
            disabled,
            readonly,
            test_id,
            show_focus,
        } = self;
        let handle = control.handle();
        let invalid: Rc<dyn Fn() -> bool> = {
            let error = error.clone();
            Rc::new(move || error().is_some())
        };
        let state = ControlState {
            spoken: spoken(label.clone(), error.clone()),
            description: description.clone(),
            invalid,
            disabled: disabled.clone(),
            readonly,
            focused: signal(show_focus),
            handle,
            test_id: test_id.as_ref().map(|id| format!("{id}-input")),
            stacked: false,
        };
        frame(Frame {
            label,
            layout,
            description,
            error,
            disabled,
            test_id,
            state,
            control: Rc::new(move |state: &ControlState| control.build(state)),
            badge: None,
        })
    }
}

// MARK: - The frame both fields share

struct Frame {
    label: SharedString,
    layout: FieldLayout,
    description: Option<SharedString>,
    error: Rc<dyn Fn() -> Option<SharedString>>,
    disabled: Rc<dyn Fn() -> bool>,
    test_id: Option<SharedString>,
    state: ControlState,
    control: Rc<dyn Fn(&ControlState) -> AnyElement>,
    /// What stands after the control while it shows something (a secret
    /// field's "Saved"): nothing at all otherwise, not even the gap.
    badge: Option<Trailer>,
}

/// Something after a field's control, while `shows` holds.
#[derive(Clone)]
struct Trailer {
    shows: Rc<dyn Fn() -> bool>,
    element: Rc<dyn Fn() -> AnyElement>,
}

/// The id `{base}-{part}`, when the field has one.
fn part(base: Option<&str>, part: &str) -> Option<String> {
    base.map(|base| format!("{base}-{part}"))
}

fn frame(frame: Frame) -> AnyElement {
    let Frame {
        label,
        layout,
        description,
        error,
        disabled,
        test_id,
        state,
        control,
        badge,
    } = frame;
    let stacked = computed(move || layout.stacked_at(font_scale()));
    let handle = state.handle;
    let focused = state.focused;
    let row_id = test_id.clone();
    let body = dynamic(move || {
        let stacked = stacked.get();
        let test_id = test_id.clone();
        let (label, description, error, state, control, badge) = (
            label.clone(),
            description.clone(),
            error.clone(),
            state.clone(),
            control.clone(),
            badge.clone(),
        );
        untracked(move || {
            let state = ControlState { stacked, ..state };
            let invalid = state.invalid.clone();
            let mut lane = view()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap(px(GAP))
                .child(control(&state));
            if !stacked {
                // A control narrower than the lane (a number) keeps to its
                // trailing end.
                lane = lane.justify_end();
            }
            if let Some(Trailer { shows, element }) = badge {
                lane = lane.child(when(move || shows(), move || element()));
            }
            let lane = lane
                .child(when(
                    move || invalid(),
                    || {
                        let fade = motion().entering(
                            MotionSpec::new(KeyframeTiming::new(Duration::Fast.ms()).easing(ENTER))
                                .values(MotionChannel::Opacity, [0.0, 1.0]),
                        );
                        view()
                            .bind_motion(&fade)
                            .child(icon("exclamationmark.circle").tone(Tone::Destructive))
                    },
                ))
                .child({
                    let ring_id = part(test_id.as_deref(), "ring");
                    when(move || focused.get(), move || ring(ring_id.clone()))
                });
            let note = note(
                description,
                error,
                part(test_id.as_deref(), "error"),
                part(test_id.as_deref(), "description"),
            );
            let mut name = TextToken::Body
                .apply(text(label))
                .text_color(move || token("foreground"))
                .accessibility_elements_hidden(true);
            if let Some(id) = part(test_id.as_deref(), "label") {
                name = name.test_id(&id);
            }
            if stacked {
                view()
                    .w_full()
                    .child(name)
                    .child(lane.mt(px(PART_GAP)))
                    .child(note)
                    .into_element()
            } else {
                view()
                    .w_full()
                    .flex_row()
                    .items_start()
                    .gap(px(GAP))
                    .child(
                        view()
                            .min_w(px(LABEL_COLUMN))
                            .flex_shrink_0()
                            .min_h(move || px(lane_height()))
                            .justify_center()
                            .child(name),
                    )
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                view()
                                    .w_full()
                                    .flex_row()
                                    .min_h(move || px(lane_height()))
                                    .child(lane),
                            )
                            .child(note),
                    )
                    .into_element()
            }
        })
    });
    let mut row = view()
        .w_full()
        .min_h(move || px(touch_target()))
        .px(px(INSET))
        .py(px(INSET_V))
        .justify_center()
        .accessible(false)
        .opacity({
            let disabled = disabled.clone();
            move || if disabled() { 0.4_f32 } else { 1.0 }
        })
        .on_press(move || {
            if !disabled() {
                handle.focus();
            }
        })
        .child(body);
    if let Some(id) = &row_id {
        row = row.test_id(id);
    }
    row.into_element()
}

/// The focus ring: 2 pt of `ring` just outside the control's lane.
fn ring(test_id: Option<String>) -> AnyElement {
    let mut ring = view()
        .absolute()
        .top(px(-2.0))
        .bottom(px(-2.0))
        .left(px(-6.0))
        .right(px(-6.0))
        .border(px(2.0))
        .border_color(move || token("ring"))
        .rounded(px(number("radius-sm")))
        .border_curve(BorderCurve::Continuous)
        // It lies over the control while the control has the keyboard:
        // touches go through to it (a tap moves the caret, a long press
        // opens the edit menu), never to the ring, which would take a tap
        // for one outside the input and put the keyboard away.
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true);
    if let Some(id) = &test_id {
        ring = ring.test_id(id);
    }
    ring.into_element()
}

/// Under the control: the error while there is one, rising 4 pt as it
/// fades in; else the description, if any. Neither is said on its own
/// (the control says them).
fn note(
    description: Option<SharedString>,
    error: Rc<dyn Fn() -> Option<SharedString>>,
    error_id: Option<String>,
    description_id: Option<String>,
) -> AnyElement {
    let shown = computed({
        let error = error.clone();
        move || error()
    });
    dynamic(move || {
        let (description, error_id, description_id) = (
            description.clone(),
            error_id.clone(),
            description_id.clone(),
        );
        match shown.get() {
            Some(error) => untracked(move || {
                let rise = motion().entering(
                    MotionSpec::new(KeyframeTiming::new(Duration::Fast.ms()).easing(ENTER))
                        .values(MotionChannel::Opacity, [0.0, 1.0])
                        .values(MotionChannel::TranslateY, [distance::RISE, 0.0]),
                );
                let mut words = TextToken::Footnote
                    .apply(text(error))
                    .text_color(move || token("destructive-text"));
                if let Some(id) = error_id {
                    words = words.test_id(&id);
                }
                view()
                    .pt(px(PART_GAP))
                    .bind_motion(&rise)
                    .accessibility_elements_hidden(true)
                    .child(words)
                    .into_element()
            }),
            None => match description {
                Some(description) => {
                    let mut words = TextToken::Footnote
                        .apply(text(description))
                        .text_color(move || token("muted-foreground"));
                    if let Some(id) = description_id {
                        words = words.test_id(&id);
                    }
                    view()
                        .pt(px(PART_GAP))
                        .accessibility_elements_hidden(true)
                        .child(words)
                        .into_element()
                }
                None => view().into_element(),
            },
        }
    })
}

// MARK: - Secret field

/// What a secret field shows of the secret it never sees (§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretShown {
    /// Nothing kept (or a new one being typed where none was kept): the
    /// page's placeholder ("Password").
    Empty,
    /// The connection keeps one and nothing is typed: "Saved", and "Type to
    /// replace" in the field.
    Saved,
    /// Something is typed over a kept one: "Replaces saved", a warning that
    /// Save overwrites it.
    Replacing,
}

impl SecretShown {
    /// From whether the host's field holds text (`typed`) and whether the
    /// connection keeps a secret (`kept`). The host's `SecretState` maps as
    /// Empty → Empty, Saved → Saved, Edited → this with `kept` its
    /// `stored_password` (or `stored_key`).
    pub fn of(typed: bool, kept: bool) -> Self {
        match (typed, kept) {
            (true, true) => SecretShown::Replacing,
            (false, true) => SecretShown::Saved,
            (_, false) => SecretShown::Empty,
        }
    }
}

/// A secret in the host's secure field ([`secret_field`]).
pub struct SecretField {
    label: SharedString,
    host: &'static str,
    identifier: &'static str,
    placeholder: SharedString,
    shown: Rc<dyn Fn() -> SecretShown>,
    on_filled: Rc<dyn Fn(bool)>,
    handle: Option<ElementRef>,
    error: Rc<dyn Fn() -> Option<SharedString>>,
    disabled: Rc<dyn Fn() -> bool>,
    test_id: Option<SharedString>,
    description: Option<SharedString>,
    auto_focus: bool,
}

/// A field labeled `label` whose control is the host's secure field `host`
/// (`moshpit/connection-password`): what is typed stays in the host, and
/// the page hears only whether there is something ([`SecretField::on_filled`]).
/// `identifier` is what the host knows the field by.
pub fn secret_field(
    label: impl Into<SharedString>,
    host: &'static str,
    identifier: &'static str,
) -> SecretField {
    let label = label.into();
    SecretField {
        placeholder: label.clone(),
        label,
        host,
        identifier,
        shown: Rc::new(|| SecretShown::Empty),
        on_filled: Rc::new(|_| {}),
        handle: None,
        error: Rc::new(|| None),
        disabled: Rc::new(|| false),
        test_id: None,
        description: None,
        auto_focus: false,
    }
}

impl SecretField {
    /// What it shows while empty with nothing kept (the label by default).
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// What it shows of the secret: empty, saved, or replacing a saved one.
    pub fn shown(mut self, shown: impl Fn() -> SecretShown + 'static) -> Self {
        self.shown = Rc::new(shown);
        self
    }

    /// Hears whether the host's field holds text, each time that changes.
    pub fn on_filled(mut self, on_filled: impl Fn(bool) + 'static) -> Self {
        self.on_filled = Rc::new(on_filled);
        self
    }

    pub fn element_ref(mut self, handle: &ElementRef) -> Self {
        self.handle = Some(*handle);
        self
    }

    /// As [`Field::error`].
    pub fn error(mut self, error: impl Fn() -> Option<SharedString> + 'static) -> Self {
        self.error = Rc::new(error);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// As [`Field::test_id`]; its badge is `{id}-badge`.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }

    /// As [`Field::description`]: one line under the field, said after
    /// its name (`{id}-description`).
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The host's field takes the keyboard once it is on screen (its
    /// `autoFocus` prop): the one thing the page is open for.
    pub fn auto_focus(mut self) -> Self {
        self.auto_focus = true;
        self
    }
}

/// `name` as the host's props take a color: `#RRGGBB(AA)`.
fn hex(name: &'static str) -> serde_json::Value {
    json!(hex_string(Rgba::from(color(token(name)))))
}

impl IntoElement for SecretField {
    fn into_element(self) -> AnyElement {
        let SecretField {
            label,
            host,
            identifier,
            placeholder,
            shown,
            on_filled,
            handle,
            error,
            disabled,
            test_id,
            description,
            auto_focus,
        } = self;
        let shown = computed(move || shown());
        let handle = handle.unwrap_or_else(element_ref);
        let invalid: Rc<dyn Fn() -> bool> = {
            let error = error.clone();
            Rc::new(move || error().is_some())
        };
        // The name, what the field shows of the secret, then the error.
        let said: Rc<dyn Fn() -> SharedString> = {
            let (label, error) = (label.clone(), error.clone());
            Rc::new(move || {
                let name: SharedString = match shown.get() {
                    SecretShown::Empty => label.clone(),
                    SecretShown::Saved => t!("%@, saved, type to replace", arg1 = label).into(),
                    SecretShown::Replacing => t!("%@, replaces saved", arg1 = label).into(),
                };
                match error() {
                    Some(error) => t!("%@, %@", arg1 = name, arg2 = error).into(),
                    None => name,
                }
            })
        };
        let state = ControlState {
            spoken: said,
            description: description.clone(),
            invalid,
            disabled: disabled.clone(),
            readonly: Rc::new(|| false),
            focused: signal(false),
            handle,
            test_id: test_id.as_ref().map(|id| format!("{id}-input")),
            stacked: false,
        };
        let control = Rc::new(move |state: &ControlState| {
            let ControlState {
                spoken,
                description,
                invalid,
                focused,
                handle,
                test_id,
                ..
            } = state.clone();
            let placeholder = placeholder.clone();
            let on_filled = on_filled.clone();
            let mut host = native(host)
                .flex_1()
                .min_w(px(0.0))
                .min_h(move || px(input_height(TextToken::Body)))
                .prop("identifier", json!(identifier))
                .prop("placeholder", move || {
                    json!(match shown.get() {
                        SecretShown::Empty => placeholder.to_string(),
                        SecretShown::Saved | SecretShown::Replacing => t!("Type to replace"),
                    })
                })
                .prop("placeholderColor", move || hex("subtle-foreground"))
                .prop("fontSize", move || {
                    json!(TextToken::Body.size(font_scale()))
                })
                .prop("textColor", move || {
                    hex(if invalid() {
                        "destructive-text"
                    } else {
                        "foreground"
                    })
                })
                .prop("tint", move || hex("primary"))
                .label(move || spoken())
                .element_ref(&handle)
                // The host says whether it holds text ("filled"), and — when
                // it tells NSH its focus — "focus" and "blur".
                .on_native_event(move |event: &NativeEvent| match &*event.name {
                    "filled" => {
                        if let Some(filled) = event.data.as_bool() {
                            on_filled(filled);
                        }
                    }
                    "focus" => focused.set(true),
                    "blur" => focused.set(false),
                    _ => {}
                });
            if let Some(description) = description {
                host = host.accessibility_description(description);
            }
            if auto_focus {
                host = host.prop("autoFocus", json!(true));
            }
            if let Some(id) = &test_id {
                host = host.test_id(id);
            }
            host.into_element()
        });
        let badge_id = part(test_id.as_deref(), "badge");
        let shows: Rc<dyn Fn() -> bool> = Rc::new(move || shown.get() != SecretShown::Empty);
        // Made again as what it shows changes: Saved and Replaces saved are
        // both shown, so the field's own `when` does not remake it between
        // them.
        let badge: Rc<dyn Fn() -> AnyElement> = Rc::new(move || {
            let badge_id = badge_id.clone();
            dynamic(move || {
                let shown = shown.get();
                let badge_id = badge_id.clone();
                untracked(move || {
                    let words = match shown {
                        SecretShown::Empty => return view().into_element(),
                        SecretShown::Saved => badge(t!("Saved"))
                            .variant(BadgeVariant::Success)
                            .icon("checkmark"),
                        SecretShown::Replacing => {
                            badge(t!("Replaces saved")).variant(BadgeVariant::Warning)
                        }
                    };
                    let fade = motion().entering(
                        MotionSpec::new(KeyframeTiming::new(Duration::Fast.ms()).easing(ENTER))
                            .values(MotionChannel::Opacity, [0.0, 1.0]),
                    );
                    let mut slot = view()
                        .flex_shrink_0()
                        .bind_motion(&fade)
                        .accessibility_elements_hidden(true)
                        .child(words);
                    if let Some(id) = badge_id {
                        slot = slot.test_id(&id);
                    }
                    slot.into_element()
                })
            })
            .into_element()
        });
        frame(Frame {
            label,
            layout: FieldLayout::Inline,
            description,
            error,
            disabled,
            test_id,
            state,
            control,
            badge: Some(Trailer {
                shows,
                element: badge,
            }),
        })
    }
}

// MARK: - Search field

/// How far a search field's focus ring stands outside its box.
const RING_GAP: f32 = 3.0;

/// A search field (a list's filter, §6.11's language list): the
/// `magnifyingglass` and an input on the `input` fill, `radius`, at least the
/// touch target tall; a press anywhere on it puts the keyboard in it. It has
/// no visible label: `placeholder` says what it searches, and screen readers
/// hear it as the field's name. `on_change` hears the text as it is typed;
/// Return puts the keyboard away.
pub fn search_field(
    placeholder: impl Into<SharedString>,
    on_change: impl Fn(String) + 'static,
) -> SearchField {
    SearchField {
        placeholder: placeholder.into(),
        on_change: Rc::new(on_change),
        test_id: None,
    }
}

/// A search field ([`search_field`]).
pub struct SearchField {
    placeholder: SharedString,
    on_change: Rc<dyn Fn(String)>,
    test_id: Option<SharedString>,
}

impl SearchField {
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for SearchField {
    fn into_element(self) -> AnyElement {
        let SearchField {
            placeholder,
            on_change,
            test_id,
        } = self;
        let handle = element_ref();
        let focused = signal(false);
        let style = TextToken::Body;
        let mut input = nsh_core::element::input()
            .flex_1()
            .min_w(px(0.0))
            .min_h(move || px(input_height(style)))
            .placeholder(placeholder.to_string())
            .placeholder_color(move || color(token("subtle-foreground")))
            .text_size(px(style.size(1.0)))
            .line_height(face::line_height())
            .text_color(move || token("foreground"))
            .caret_color(move || color(token("primary")))
            .keyboard_appearance(move || {
                if dark() {
                    KeyboardAppearance::Dark
                } else {
                    KeyboardAppearance::Light
                }
            })
            .auto_correct(false)
            .auto_capitalize(AutoCapitalize::None)
            .return_key_type(ReturnKeyType::Search)
            .label(placeholder.to_string())
            .element_ref(&handle)
            .on_focus(move || focused.set(true))
            .on_blur(move || focused.set(false))
            .on_change(move |text: &str| on_change(text.to_owned()))
            .on_submit(|_: &str| dismiss_keyboard());
        if let Some(id) = &test_id {
            input = input.test_id(id);
        }
        let ring_id = test_id.as_ref().map(|id| format!("{id}-ring"));
        view()
            .w_full()
            .relative()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .min_h(px(touch_target()))
            .px(px(12.0))
            .rounded(px(number("radius")))
            .border_curve(BorderCurve::Continuous)
            .bg(move || token("input"))
            .on_press(move || handle.focus())
            .child(icon("magnifyingglass").tone(Tone::Muted))
            .child(input)
            // Focused: a 2 pt `ring` outside the box, as every field's
            // control has (§4.4), its corner concentric with the box's.
            .child(when(
                move || focused.get(),
                move || {
                    let mut ring = view()
                        .absolute()
                        .top(px(-RING_GAP))
                        .bottom(px(-RING_GAP))
                        .left(px(-RING_GAP))
                        .right(px(-RING_GAP))
                        .border(px(2.0))
                        .border_color(move || token("ring"))
                        .rounded(px(number("radius") + RING_GAP))
                        .border_curve(BorderCurve::Continuous)
                        .pointer_events(PointerEvents::None)
                        .accessibility_elements_hidden(true);
                    if let Some(id) = &ring_id {
                        ring = ring.test_id(id);
                    }
                    ring
                },
            ))
            .into_element()
    }
}

/// `make()`'s element, made again (new nodes, not the old ones updated)
/// each time `revision()` moves: a field whose text the page or the host
/// sets (a picked file, a preset) starts from it, as a field's default
/// value applies only to a field made with it.
pub fn remount(
    revision: impl Fn() -> u64 + 'static,
    make: impl Fn() -> AnyElement + 'static,
) -> AnyElement {
    each(
        move || vec![revision()],
        |revision: &u64| *revision,
        move |_, _| make(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_secret_shows_saved_until_something_is_typed_over_it() {
        assert_eq!(SecretShown::of(false, false), SecretShown::Empty);
        assert_eq!(SecretShown::of(false, true), SecretShown::Saved);
        assert_eq!(SecretShown::of(true, true), SecretShown::Replacing);
        // A new one where none was kept says nothing.
        assert_eq!(SecretShown::of(true, false), SecretShown::Empty);
    }

    #[test]
    fn an_inline_field_stands_stacked_from_the_first_accessibility_size() {
        let ax1 = face::point_size(face::TextStyle::Body, 1.0);
        assert!(!FieldLayout::Inline.stacked_at(1.0));
        assert!(FieldLayout::Stacked.stacked_at(1.0));
        // AX1's body is 28 pt over the default 17.
        assert!(FieldLayout::Inline.stacked_at(28.0 / ax1));
        assert!(!FieldLayout::Inline.stacked_at(23.0 / ax1));
    }
}
