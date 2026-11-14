//! The system's menu (`docs/nsh/UI-DESIGN.md` §4.8, Menu): what a ⋯, a bar
//! button or a page's title opens. The host draws it (`moshpit/menu`, a
//! button whose press opens UIKit's menu), so it feels as the system's
//! does. Lay it over its trigger, last: it fills the box it is in, a press
//! anywhere on that box opens it, and screen readers hear one button
//! called its label.
//!
//! An entry ([`MenuEntry`]) has an id and a title, and can have a symbol
//! beside the title, a line under it, the destructive role (drawn in red)
//! or be disabled. The same action says the same words with the same
//! symbol wherever it is offered.
//!
//! The menu's test id is the host's control's accessibility identifier
//! (as [`super::control`]'s), which tests find it by.

use nsh_core::prelude::*;
use serde_json::json;

/// One choice of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: String,
    pub title: String,
    /// The SF Symbol beside the title (SwiftUI's `Label(_:systemImage:)`).
    pub symbol: Option<&'static str>,
    /// Drawn in red (`Button(role: .destructive)`).
    pub destructive: bool,
    /// Greyed out, not pickable.
    pub disabled: bool,
    /// A line under the title (`UIAction.subtitle`).
    pub detail: Option<String>,
}

impl MenuEntry {
    /// A plain choice, picked by its `id`.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> MenuEntry {
        MenuEntry {
            id: id.into(),
            title: title.into(),
            symbol: None,
            destructive: false,
            disabled: false,
            detail: None,
        }
    }

    /// With `detail` under its title.
    pub fn detail(self, detail: impl Into<String>) -> MenuEntry {
        MenuEntry {
            detail: Some(detail.into()),
            ..self
        }
    }

    /// With `name` beside its title.
    pub fn symbol(self, name: &'static str) -> MenuEntry {
        MenuEntry {
            symbol: Some(name),
            ..self
        }
    }

    /// Drawn in red, for what deletes or removes.
    pub fn destructive(self) -> MenuEntry {
        MenuEntry {
            destructive: true,
            ..self
        }
    }

    /// Greyed out and not pickable while `disabled`.
    pub fn disabled(self, disabled: bool) -> MenuEntry {
        MenuEntry { disabled, ..self }
    }

    /// As the host's component takes it.
    fn json(&self) -> serde_json::Value {
        let mut item = json!({"id": self.id, "title": self.title});
        if let Some(symbol) = self.symbol {
            item["image"] = json!(symbol);
        }
        if self.destructive {
            item["destructive"] = json!(true);
        }
        if self.disabled {
            item["disabled"] = json!(true);
        }
        if let Some(detail) = &self.detail {
            item["detail"] = json!(detail);
        }
        item
    }
}

/// The system's menu of `entries`, over the box it fills: `on_select`
/// hears the id of the entry picked, and screen readers hear one button
/// called `label()`. `id` is its test id.
pub fn menu(
    entries: Vec<MenuEntry>,
    id: Option<&str>,
    label: impl Fn() -> String + 'static,
    on_select: impl Fn(String) + 'static,
) -> AnyElement {
    let items = json!(entries.iter().map(MenuEntry::json).collect::<Vec<_>>());
    over(
        native("moshpit/menu").prop("items", items),
        id,
        label,
        on_select,
    )
}

/// [`menu`] whose entries follow the state: `entries()` is read again when
/// what it reads changes (a tick that moves with the pick).
pub fn menu_following(
    entries: impl Fn() -> Vec<MenuEntry> + 'static,
    id: Option<&str>,
    label: impl Fn() -> String + 'static,
    on_select: impl Fn(String) + 'static,
) -> AnyElement {
    let items = move || json!(entries().iter().map(MenuEntry::json).collect::<Vec<_>>());
    over(
        native("moshpit/menu").prop("items", items),
        id,
        label,
        on_select,
    )
}

/// The host's menu laid over its trigger, labeled, telling `on_select`.
fn over(
    menu: NativeView,
    id: Option<&str>,
    label: impl Fn() -> String + 'static,
    on_select: impl Fn(String) + 'static,
) -> AnyElement {
    menu.absolute()
        .inset_0()
        .prop(
            "identifier",
            id.map_or(serde_json::Value::Null, |id| json!(id)),
        )
        .prop("label", move || json!(label()))
        .on_native_event(move |event: &NativeEvent| {
            if &*event.name == "select"
                && let Some(id) = event.data.as_str()
            {
                on_select(id.to_owned());
            }
        })
        .into_element()
}
