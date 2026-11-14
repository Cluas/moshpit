//! The system's own controls, in the app's colors
//! (`docs/nsh/UI-DESIGN.md` §4.7, §4.8): [`Switch`] (`UISwitch`),
//! [`Segmented`] (`UISegmentedControl`) and [`Select`], a row that opens the
//! system's menu of options. The host draws them (`moshpit/switch`,
//! `moshpit/segmented`, `moshpit/menu`, NSH-GAPS #3), so they feel as the
//! system's do; their colors come from the tokens.
//!
//! A control's test id is its UIKit control's accessibility identifier
//! (the host's `identifier` prop), and nothing else's: NSH would put a
//! view's test id on the wrapper around a host's control (NSH-GAPS #13),
//! and two views of one identifier send a finder (a device test, a UI
//! test, VoiceOver's rotor) to the wrapper rather than the control.
//! Headless tests find these controls by their `identifier` prop.

use std::rc::Rc;

use nsh_core::{color::Rgba, prelude::*};
use serde_json::json;

use crate::{
    theme::hex_string,
    ui::{
        icon::{IconSize, Tone, icon},
        row::row,
        tokens::token,
    },
};

/// Color token `name` now, as the host's components take a color.
fn hex(name: &str) -> serde_json::Value {
    json!(hex_string(Rgba::from(color(token(name)))))
}

/// A native control's accessibility identifier: its test id, which a UI
/// test finds it by.
fn identifier(id: Option<&str>) -> serde_json::Value {
    id.map_or(serde_json::Value::Null, |id| json!(id))
}

/// A switch ([`switch`]).
pub struct Switch {
    on: Rc<dyn Fn() -> bool>,
    on_change: Rc<dyn Fn(bool)>,
    label: Option<SharedString>,
    disabled: Rc<dyn Fn() -> bool>,
    test_id: Option<SharedString>,
}

/// The system switch, on while `on()` holds; it moves when the user flips
/// it, as UIKit's does, and `on_change` hears it (save at once; if saving
/// fails, `on()` puts it back). On is `primary`, the off track `muted`.
pub fn switch(on: impl Fn() -> bool + 'static, on_change: impl Fn(bool) + 'static) -> Switch {
    Switch {
        on: Rc::new(on),
        on_change: Rc::new(on_change),
        label: None,
        disabled: Rc::new(|| false),
        test_id: None,
    }
}

impl Switch {
    /// What screen readers call it: its row's title.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// The UIKit control's accessibility identifier, which tests find it by.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Switch {
    fn into_element(self) -> AnyElement {
        let Switch {
            on,
            on_change,
            label,
            disabled,
            test_id,
        } = self;
        let mut control = native("moshpit/switch")
            .flex_shrink_0()
            .prop("on", move || json!(on()))
            .prop("tint", || hex("primary"))
            .prop("offTint", || hex("muted"))
            .prop("enabled", move || json!(!disabled()))
            .prop("identifier", identifier(test_id.as_deref()))
            .on_native_event(move |event: &NativeEvent| {
                if &*event.name == "change"
                    && let Some(on) = event.data.as_bool()
                {
                    on_change(on);
                }
            });
        if let Some(label) = label {
            control = control.prop("label", json!(label.to_string()));
        }
        control.into_element()
    }
}

/// A segmented control ([`segmented`]).
pub struct Segmented {
    items: Vec<SharedString>,
    selected: Rc<dyn Fn() -> Option<usize>>,
    on_change: Rc<dyn Fn(usize)>,
    label: Option<SharedString>,
    test_id: Option<SharedString>,
}

/// The system segmented control of `items` (two to four), segment
/// `selected()` picked (`None`: none); `on_change` hears the user pick one.
/// It is as wide as its box. Its selected segment is `thumb`, its words
/// `foreground`. Its words do not grow with the text size (the system's
/// rule): where long labels would not fit at accessibility sizes, a screen
/// uses rows with checks instead.
pub fn segmented(
    items: impl IntoIterator<Item = impl Into<SharedString>>,
    selected: impl Fn() -> Option<usize> + 'static,
    on_change: impl Fn(usize) + 'static,
) -> Segmented {
    Segmented {
        items: items.into_iter().map(Into::into).collect(),
        selected: Rc::new(selected),
        on_change: Rc::new(on_change),
        label: None,
        test_id: None,
    }
}

impl Segmented {
    /// What screen readers call it.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The UIKit control's accessibility identifier, which tests find it by.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Segmented {
    fn into_element(self) -> AnyElement {
        let Segmented {
            items,
            selected,
            on_change,
            label,
            test_id,
        } = self;
        let titles: Vec<String> = items.iter().map(ToString::to_string).collect();
        let mut control = native("moshpit/segmented")
            .w_full()
            .prop("items", json!(titles))
            .prop("selected", move || {
                json!(selected().map_or(-1, |i| i as i64))
            })
            .prop("thumb", || hex("thumb"))
            .prop("text", || hex("muted-foreground"))
            .prop("selectedText", || hex("foreground"))
            .prop("identifier", identifier(test_id.as_deref()))
            .on_native_event(move |event: &NativeEvent| {
                if &*event.name == "change"
                    && let Some(index) = event.data.as_u64()
                {
                    on_change(index as usize);
                }
            });
        if let Some(label) = label {
            control = control.prop("label", json!(label.to_string()));
        }
        control.into_element()
    }
}

/// One option of a [`Select`] ([`option`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    id: SharedString,
    title: SharedString,
    detail: Option<SharedString>,
}

/// An option called `title`, picked by its `id`.
pub fn option(id: impl Into<SharedString>, title: impl Into<SharedString>) -> SelectOption {
    SelectOption {
        id: id.into(),
        title: title.into(),
        detail: None,
    }
}

impl SelectOption {
    /// A line under its title in the menu, saying what it is for ("tmux ·
    /// on almost every host").
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// A select ([`select`]).
pub struct Select {
    label: SharedString,
    options: Rc<Vec<SelectOption>>,
    selected: Rc<dyn Fn() -> SharedString>,
    on_select: Rc<dyn Fn(SharedString)>,
    disabled: Rc<dyn Fn() -> bool>,
    test_id: Option<SharedString>,
}

/// A row labeled `label` showing the option whose id is `selected()`, with
/// the up-down chevron; pressed, the system's menu of `options` opens, the
/// option in use checked, and `on_select` hears the id picked. Screen
/// readers hear one button: the label and the option in use.
pub fn select(
    label: impl Into<SharedString>,
    options: impl IntoIterator<Item = SelectOption>,
    selected: impl Fn() -> SharedString + 'static,
    on_select: impl Fn(SharedString) + 'static,
) -> Select {
    Select {
        label: label.into(),
        options: Rc::new(options.into_iter().collect()),
        selected: Rc::new(selected),
        on_select: Rc::new(on_select),
        disabled: Rc::new(|| false),
        test_id: None,
    }
}

impl Select {
    /// While this holds the select waits: its row dims and its menu is not
    /// there to open (a step about the option in use is running).
    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// The UIKit control's accessibility identifier, which tests find it by.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Select {
    fn into_element(self) -> AnyElement {
        let Select {
            label,
            options,
            selected,
            on_select,
            disabled,
            test_id,
        } = self;
        let disabled = computed(move || disabled());
        let current = {
            let (options, selected) = (options.clone(), selected.clone());
            move || -> SharedString {
                let id = selected();
                options
                    .iter()
                    .find(|o| o.id == id)
                    .map_or(id, |o| o.title.clone())
            }
        };
        let items = {
            let (options, selected) = (options.clone(), selected.clone());
            move || {
                let id = selected();
                json!(
                    options
                        .iter()
                        .map(|o| {
                            let mut item =
                                json!({"id": o.id.to_string(), "title": o.title.to_string()});
                            if o.id == id {
                                item["checked"] = json!(true);
                            }
                            if let Some(detail) = &o.detail {
                                item["detail"] = json!(detail.to_string());
                            }
                            item
                        })
                        .collect::<Vec<_>>()
                )
            }
        };
        let spoken = {
            let (label, current) = (label.clone(), current.clone());
            move || json!(format!("{label}, {}", current()))
        };
        // An option's name is read whole: the row stacks rather than
        // shorten it.
        let shown = row(label)
            .value_with(current)
            .whole_value()
            .disabled(move || disabled.get())
            .trailing(
                icon("chevron.up.chevron.down")
                    .size(IconSize::Sm)
                    .tone(Tone::Subtle),
            );
        let (items, spoken) = (Rc::new(items), Rc::new(spoken));
        let menu = move || {
            let (items, spoken, on_select) = (items.clone(), spoken.clone(), on_select.clone());
            native("moshpit/menu")
                .absolute()
                .inset_0()
                .prop("items", move || items())
                .prop("identifier", identifier(test_id.as_deref()))
                .prop("label", move || spoken())
                .on_native_event(move |event: &NativeEvent| {
                    if &*event.name == "select"
                        && let Some(id) = event.data.as_str()
                    {
                        on_select(SharedString::from(id.to_owned()));
                    }
                })
        };
        view()
            .w_full()
            .relative()
            // The menu over the row is what screen readers and presses
            // reach, the row under it only shows; while the select waits
            // there is no menu, and screen readers hear the dimmed row.
            .child(
                view()
                    .accessibility_elements_hidden(move || !disabled.get())
                    .child(shown),
            )
            .child(when(move || !disabled.get(), menu))
            .into_element()
    }
}
