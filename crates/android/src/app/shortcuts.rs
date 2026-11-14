//! Shortcuts' host side on Android: the shared model
//! (`moshpit_app::shortcuts`: what the page shows of the store, what its
//! taps may do, the editor's draft and Save) over the store that keeps the
//! format every build since 1.0.8 reads.

use std::collections::BTreeSet;

use moshpit_app::shortcuts::{self as model, EditorSave, ListOp, ShortcutEditor};
use serde_json::Value;

use crate::app::{App, Core, PageKind};

/// The editor's model: the shortcut edited, its draft, the one a first Save
/// added.
pub type ShortcutDraft = ShortcutEditor;

/// `{"op": "shortcuts", …}`.
pub fn message(app: &App) -> String {
    model::page_message(app.shortcuts.shortcuts(), |s| s.summary.clone())
}

pub fn on_event(app: &mut App, core: &Core, surface: u64, _ev: &str, event: &Value) {
    let Some(op) = model::list_op(event, app.shortcuts.shortcuts()) else {
        return;
    };
    match op {
        ListOp::Done => {
            app.close(surface);
            return;
        }
        ListOp::Add => {
            app.open(
                core,
                PageKind::AddShortcut(Box::new(ShortcutEditor::new(None))),
                true,
            );
            return;
        }
        ListOp::Edit(id) => {
            let existing = app
                .shortcuts
                .shortcuts()
                .iter()
                .find(|s| s.id == id)
                .cloned();
            let editor = ShortcutEditor::new(existing.as_ref());
            app.open(core, PageKind::AddShortcut(Box::new(editor)), true);
            return;
        }
        ListOp::Move { from, to } => app.shortcuts.move_in_toolbar(&BTreeSet::from([from]), to),
        ListOp::RemoveFromToolbar(id) => app.shortcuts.remove_from_toolbar(id),
        ListOp::AddToToolbar(id) => {
            if !app.shortcuts.add_to_toolbar(id) {
                app.platform
                    .toast("The bar is full: take a shortcut off it first");
            }
        }
        ListOp::Delete(id) => app.shortcuts.remove(id),
    }
    app.refresh();
}

/// `{"op": "shortcut", …}`.
pub fn editor_message(app: &App, editor: &ShortcutDraft) -> String {
    let hosts = app
        .connections
        .connections()
        .iter()
        .map(|c| c.display_name())
        .collect();
    editor.message(hosts, app.shortcuts.toolbar_count())
}

pub fn on_editor_event(app: &mut App, _core: &Core, surface: u64, ev: &str, event: &Value) {
    match ev {
        "change" => {
            if let Some(PageKind::AddShortcut(editor)) = app.page_mut(surface).map(|p| &mut p.kind)
            {
                editor.apply(&event["draft"]);
            }
        }
        "save" => {
            let shortcuts = app.shortcuts.shortcuts().to_vec();
            let Some(PageKind::AddShortcut(editor)) = app.page_mut(surface).map(|p| &mut p.kind)
            else {
                return;
            };
            let Some(save) = editor.save(|id| shortcuts.iter().find(|s| s.id == id).cloned())
            else {
                return;
            };
            match save {
                EditorSave::Add(shortcut) => {
                    if !app.shortcuts.add(shortcut) {
                        // B252: under AVAILABLE, said before the page goes.
                        app.platform.toast("Saved under AVAILABLE: the bar is full");
                    }
                }
                EditorSave::Update(shortcut) => app.shortcuts.update(shortcut),
            }
            app.close(surface);
            app.refresh();
        }
        "cancel" => app.close(surface),
        _ => {}
    }
}
