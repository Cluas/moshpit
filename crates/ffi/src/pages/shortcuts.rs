//! Settings › Shortcuts (`ui/src/shortcuts.rs`) and its editor
//! (`ui/src/add_shortcut.rs`). Both pages' messages are the shared model's
//! (`moshpit_app::shortcuts`: `shortcuts_page_message`,
//! `ShortcutEditorHandle::message`), posted as it writes them; what the
//! pages tell crosses here. A list page's event is only what was asked:
//! `shortcuts_list_op` checks it against the store.

use serde_json::{Value, json};

/// What Settings › Shortcuts tells the host, as told (unchecked).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ShortcutsEvent {
    Done,
    /// The bar's +.
    Add,
    /// A custom shortcut pressed: its editor.
    Edit {
        id: String,
    },
    /// A custom shortcut's Delete.
    Delete {
        id: String,
    },
    /// A bar row's −: it goes to Available.
    RemoveFromToolbar {
        id: String,
    },
    /// An Available row's +.
    AddToToolbar {
        id: String,
    },
    /// A bar shortcut dragged from place `from` to before place `to`.
    Move {
        from: u32,
        to: u32,
    },
}

/// What the shortcut editor tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AddShortcutEvent {
    /// The whole draft as it changed, as JSON: the shared editor's to read
    /// (`ShortcutEditorHandle::apply`).
    Change {
        draft_json: String,
    },
    Save,
    Cancel,
}

impl ShortcutsEvent {
    /// What the list page told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let id = || value["id"].as_str().map(str::to_owned);
        let place = |key: &str| u32::try_from(value[key].as_u64()?).ok();
        Some(match value["ev"].as_str()? {
            "done" => ShortcutsEvent::Done,
            "add" => ShortcutsEvent::Add,
            "edit" => ShortcutsEvent::Edit { id: id()? },
            "delete" => ShortcutsEvent::Delete { id: id()? },
            "remove-from-toolbar" => ShortcutsEvent::RemoveFromToolbar { id: id()? },
            "add-to-toolbar" => ShortcutsEvent::AddToToolbar { id: id()? },
            "move" => ShortcutsEvent::Move {
                from: place("from")?,
                to: place("to")?,
            },
            _ => return None,
        })
    }

    /// The event as the page told it, for the shared model's check
    /// (`moshpit_app::shortcuts::list_op`).
    pub(crate) fn told(&self) -> Value {
        match self {
            ShortcutsEvent::Done => json!({"ev": "done"}),
            ShortcutsEvent::Add => json!({"ev": "add"}),
            ShortcutsEvent::Edit { id } => json!({"ev": "edit", "id": id}),
            ShortcutsEvent::Delete { id } => json!({"ev": "delete", "id": id}),
            ShortcutsEvent::RemoveFromToolbar { id } => {
                json!({"ev": "remove-from-toolbar", "id": id})
            }
            ShortcutsEvent::AddToToolbar { id } => json!({"ev": "add-to-toolbar", "id": id}),
            ShortcutsEvent::Move { from, to } => json!({"ev": "move", "from": from, "to": to}),
        }
    }
}

impl AddShortcutEvent {
    /// What the editor told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        Some(match value["ev"].as_str()? {
            "change" => AddShortcutEvent::Change {
                draft_json: value.get("draft").filter(|d| d.is_object())?.to_string(),
            },
            "save" => AddShortcutEvent::Save,
            "cancel" => AddShortcutEvent::Cancel,
            _ => return None,
        })
    }
}
