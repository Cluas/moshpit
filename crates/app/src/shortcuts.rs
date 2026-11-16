//! Settings › Shortcuts and the shortcut editor, the host's model: what the
//! list page shows of the store and what its taps may do, and the editor's
//! draft and Save. One copy for iOS (`ShortcutsNSHPage.swift`'s
//! `ShortcutsModel` and `AddShortcutModel`, before this crate) and Android
//! (`crates/android/src/app/shortcuts.rs`), with the stricter rule of the
//! two wherever they differed:
//!
//! - AVAILABLE is every shortcut off the bar as shown, the user's own too,
//!   and one past the twelfth an older set marked as on it (B233; Android
//!   listed only those not marked).
//! - A built-in is never edited or deleted from the page (Android let a
//!   crafted event do both).
//! - A move stays within the bar.
//! - A second Save of a new shortcut (a double tap, or a tap while the
//!   "saved off the bar" note comes up) updates the one the first added,
//!   never adds another (B288; Android added two).
//! - A new shortcut saved while the bar is full says so (B252; Android was
//!   silent).
//! - A new shortcut starts as the editor shows it: a key combo in the
//!   accent, repeating on hold (Android started from the store's blank: gray,
//!   no repeat).
//! - The chip's label and key hold six characters as a person counts them
//!   (grapheme clusters, as Swift's `prefix(6)`; Android counted scalars,
//!   so an emoji with a skin tone took two of the six).
//! - Save writes over the store's current copy of the shortcut, so where it
//!   sits on the bar stays as the store has it (iOS wrote back the copy the
//!   editor opened with: a second Save of a shortcut saved off a full bar
//!   marked it as on the bar).
//!
//! The model never touches a store: it says what to do ([`ListOp`],
//! [`EditorSave`]) and the host does it, on whichever store it runs.

use std::collections::BTreeSet;

use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::shortcuts::{ShortcutKind, ShortcutModifier, ShortcutStore, TerminalShortcut};
use moshpit_ui::add_shortcut::{self as editor_screen, Draft};
use moshpit_ui::shortcuts::{self as list_screen, Shortcut, Shortcuts};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

/// How many shortcuts the bar holds.
pub const TOOLBAR_LIMIT: usize = ShortcutStore::TOOLBAR_LIMIT;

/// The colours a custom chip may take.
pub const COLORS: [&str; 4] = ["gray", "accent", "mosh", "amber"];

/// How many characters a chip's label and a combo's key hold.
pub const CHIP_LENGTH: usize = 6;

/// The bar as it shows: the shortcuts marked as on it, the first twelve.
pub fn toolbar(all: &[TerminalShortcut]) -> Vec<&TerminalShortcut> {
    all.iter()
        .filter(|s| s.in_toolbar)
        .take(TOOLBAR_LIMIT)
        .collect()
}

/// What the list page shows of `all` (in the store's order): the bar, the
/// user's own, and every shortcut off the bar as shown. `name` is what a
/// shortcut is called on the page: a built-in's name in the app's language
/// (B234, the host's to say), the user's own summary as it is.
pub fn page(all: &[TerminalShortcut], name: impl Fn(&TerminalShortcut) -> String) -> Shortcuts {
    let entry = |s: &TerminalShortcut| Shortcut {
        id: s.id.to_string(),
        kind: s.kind.raw_value().to_owned(),
        label: s.chip_label.clone(),
        summary: name(s),
        builtin: s.is_builtin,
    };
    let bar = toolbar(all);
    let shown: BTreeSet<Uuid> = bar.iter().map(|s| s.id).collect();
    Shortcuts {
        toolbar: bar.iter().map(|s| entry(s)).collect(),
        custom: all.iter().filter(|s| !s.is_builtin).map(entry).collect(),
        available: all
            .iter()
            .filter(|s| !shown.contains(&s.id))
            .map(entry)
            .collect(),
        limit: TOOLBAR_LIMIT,
    }
}

/// `{"op": "shortcuts", …}` of [`page`].
pub fn page_message(
    all: &[TerminalShortcut],
    name: impl Fn(&TerminalShortcut) -> String,
) -> String {
    list_screen::Message::Shortcuts(page(all, name)).to_json()
}

/// What a tap on the list page does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOp {
    /// Close the page.
    Done,
    /// Open the editor on a new shortcut.
    Add,
    /// Open the editor on the user's shortcut `id`.
    Edit(Uuid),
    /// Move bar position `from` to before bar position `to`.
    Move {
        from: usize,
        to: usize,
    },
    RemoveFromToolbar(Uuid),
    /// Put `id` on the bar; the host says so when the bar is full.
    AddToToolbar(Uuid),
    /// Delete the user's shortcut `id`.
    Delete(Uuid),
}

/// What the list page told (`{"ev": …}`), checked against `all`: `None` for
/// an event it does not know, a shortcut it does not have, a built-in's
/// edit or delete, or a move off the bar.
pub fn list_op(message: &Value, all: &[TerminalShortcut]) -> Option<ListOp> {
    let named = || {
        let id = Uuid::parse(message["id"].as_str()?)?;
        all.iter().find(|s| s.id == id)
    };
    let custom = || named().filter(|s| !s.is_builtin).map(|s| s.id);
    Some(match message["ev"].as_str()? {
        "done" => ListOp::Done,
        "add" => ListOp::Add,
        "edit" => ListOp::Edit(custom()?),
        "delete" => ListOp::Delete(custom()?),
        "remove-from-toolbar" => ListOp::RemoveFromToolbar(named()?.id),
        "add-to-toolbar" => ListOp::AddToToolbar(named()?.id),
        "move" => {
            let count = toolbar(all).len();
            let from = usize::try_from(message["from"].as_u64()?).ok()?;
            let to = usize::try_from(message["to"].as_u64()?).ok()?;
            if from >= count || to > count {
                return None;
            }
            ListOp::Move { from, to }
        }
        _ => return None,
    })
}

/// What the editor's Save does.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorSave {
    /// Add a new shortcut (on the bar while it has room: the store says
    /// whether it went there, and the host says so when it did not, B252).
    Add(TerminalShortcut),
    /// Write over a shortcut the store has.
    Update(TerminalShortcut),
}

/// The shortcut editor's model: the shortcut edited (none: a new one), the
/// draft as the page has it, and the new shortcut a first Save added.
#[derive(Debug, Clone, PartialEq)]
pub struct ShortcutEditor {
    /// The user's shortcut edited, as the editor opened it.
    pub existing: Option<TerminalShortcut>,
    pub draft: Draft,
    /// The new shortcut a first Save added: a second Save updates it.
    pub added: Option<Uuid>,
}

impl ShortcutEditor {
    /// The editor on `existing`, or on a new shortcut: a key combo in the
    /// accent that repeats on hold.
    pub fn new(existing: Option<&TerminalShortcut>) -> Self {
        let draft = match existing {
            Some(s) => Draft {
                kind: s.kind.raw_value().to_owned(),
                modifiers: s
                    .modifiers
                    .iter()
                    .map(|m| modifier_name(*m).to_owned())
                    .collect(),
                key: s.key.clone(),
                payload: s.payload.clone(),
                append_return: s.append_return,
                repeat_on_hold: s.repeat_on_hold,
                chip_label: s.chip_label.clone(),
                summary: s.summary.clone(),
                color_id: s.color_id.clone(),
                scope_hosts: sorted(s.scope_hosts.iter().cloned()),
                only_in_tmux: s.only_in_tmux,
            },
            None => Draft {
                kind: ShortcutKind::KeyCombo.raw_value().to_owned(),
                repeat_on_hold: true,
                color_id: "accent".to_owned(),
                ..Draft::default()
            },
        };
        ShortcutEditor {
            existing: existing.cloned(),
            draft,
            added: None,
        }
    }

    /// The page's draft (`{"kind", "modifiers", …}`, its `change` event's
    /// `draft`), as it told it: a field it does not send, or sends in a
    /// shape this does not read, stays as it was; a label or key longer
    /// than [`CHIP_LENGTH`] is cut there; a colour it does not know is not
    /// taken.
    pub fn apply(&mut self, draft: &Value) {
        let d = &mut self.draft;
        if let Some(kind) = draft["kind"].as_str().and_then(ShortcutKind::from_raw) {
            d.kind = kind.raw_value().to_owned();
        }
        if let Some(mods) = draft["modifiers"].as_array() {
            let set: BTreeSet<ShortcutModifier> =
                mods.iter().filter_map(|m| modifier(m.as_str()?)).collect();
            d.modifiers = set
                .into_iter()
                .map(|m| modifier_name(m).to_owned())
                .collect();
        }
        if let Some(text) = draft["key"].as_str() {
            d.key = prefix(text, CHIP_LENGTH);
        }
        if let Some(text) = draft["payload"].as_str() {
            d.payload = text.to_owned();
        }
        if let Some(on) = draft["appendReturn"].as_bool() {
            d.append_return = on;
        }
        if let Some(on) = draft["repeatOnHold"].as_bool() {
            d.repeat_on_hold = on;
        }
        if let Some(text) = draft["chipLabel"].as_str() {
            d.chip_label = prefix(text, CHIP_LENGTH);
        }
        if let Some(text) = draft["summary"].as_str() {
            d.summary = text.to_owned();
        }
        if let Some(id) = draft["colorId"].as_str().filter(|id| COLORS.contains(id)) {
            d.color_id = id.to_owned();
        }
        if let Some(names) = draft["scopeHosts"].as_array() {
            d.scope_hosts = sorted(names.iter().filter_map(|n| n.as_str().map(str::to_owned)));
        }
        if let Some(on) = draft["onlyInTmux"].as_bool() {
            d.only_in_tmux = on;
        }
    }

    /// Whether Save may save: a chip label, and a key or a payload for a
    /// key combo, a payload otherwise.
    pub fn can_save(&self) -> bool {
        self.draft.can_save()
    }

    /// `{"op": "shortcut", …}`: the draft, with the user's `hosts` for SCOPE
    /// and how many shortcuts the bar holds now.
    pub fn message(&self, hosts: Vec<String>, toolbar_count: usize) -> String {
        editor_screen::Message::Shortcut(editor_screen::Editor {
            draft: self.draft.clone(),
            editing: self.existing.is_some(),
            hosts,
            toolbar_count,
            limit: TOOLBAR_LIMIT,
        })
        .to_json()
    }

    /// Save: the shortcut to write, over the store's current copy of it
    /// (`current` looks one up by id), or `None` when it can't be saved
    /// yet. An empty summary is the chip's label. A new shortcut is added
    /// once; a second Save updates it (B288).
    pub fn save(
        &mut self,
        current: impl Fn(Uuid) -> Option<TerminalShortcut>,
    ) -> Option<EditorSave> {
        if !self.can_save() {
            return None;
        }
        let id = self.existing.as_ref().map(|s| s.id).or(self.added);
        let base = id
            .and_then(&current)
            .or_else(|| self.existing.clone())
            .unwrap_or_else(|| TerminalShortcut::new(id.unwrap_or_else(Uuid::new_v4)));
        let d = &self.draft;
        let mut s = base;
        if let Some(kind) = ShortcutKind::from_raw(&d.kind) {
            s.kind = kind;
        }
        s.modifiers = d.modifiers.iter().filter_map(|m| modifier(m)).collect();
        s.key = d.key.clone();
        s.payload = d.payload.clone();
        s.append_return = d.append_return;
        s.repeat_on_hold = d.repeat_on_hold;
        s.chip_label = d.chip_label.clone();
        s.summary = if d.summary.is_empty() {
            d.chip_label.clone()
        } else {
            d.summary.clone()
        };
        s.color_id = d.color_id.clone();
        s.set_scope_hosts(d.scope_hosts.iter().cloned());
        s.only_in_tmux = d.only_in_tmux;
        if id.is_none() {
            self.added = Some(s.id);
            return Some(EditorSave::Add(s));
        }
        Some(EditorSave::Update(s))
    }
}

fn modifier(raw: &str) -> Option<ShortcutModifier> {
    Some(match raw {
        "ctrl" => ShortcutModifier::Ctrl,
        "alt" => ShortcutModifier::Alt,
        "shift" => ShortcutModifier::Shift,
        "cmd" => ShortcutModifier::Cmd,
        _ => return None,
    })
}

fn modifier_name(m: ShortcutModifier) -> &'static str {
    match m {
        ShortcutModifier::Ctrl => "ctrl",
        ShortcutModifier::Alt => "alt",
        ShortcutModifier::Shift => "shift",
        ShortcutModifier::Cmd => "cmd",
    }
}

/// The first `n` characters of `text`, as a person counts them.
fn prefix(text: &str, n: usize) -> String {
    text.graphemes(true).take(n).collect()
}

fn sorted(names: impl Iterator<Item = String>) -> Vec<String> {
    names.collect::<BTreeSet<_>>().into_iter().collect()
}

#[cfg(test)]
mod tests;
