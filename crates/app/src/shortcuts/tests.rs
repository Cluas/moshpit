//! The shortcut pages' rules, ported from the Swift host's suite
//! (`KeysShortcutsSetupNSHTests`' Shortcuts parts) and Android's, run here
//! on the Rust store the hosts keep (`moshpit_store::shortcuts`).

use std::sync::Arc;

use super::*;
use moshpit_store::Readiness;
use moshpit_store::host::MemoryDefaults;
use serde_json::json;

fn store() -> ShortcutStore {
    ShortcutStore::new(Arc::new(MemoryDefaults::new()), Readiness::Always)
}

fn mine(label: &str) -> TerminalShortcut {
    let mut s = TerminalShortcut::new(Uuid::new_v4());
    s.kind = ShortcutKind::Text;
    s.payload = label.to_owned();
    s.chip_label = label.to_owned();
    s.summary = label.to_owned();
    s
}

fn plain(s: &TerminalShortcut) -> String {
    s.summary.clone()
}

/// What the host does with a list op (the iOS and Android hosts alike).
fn run(store: &mut ShortcutStore, op: ListOp) {
    match op {
        ListOp::Move { from, to } => store.move_in_toolbar(&BTreeSet::from([from]), to),
        ListOp::RemoveFromToolbar(id) => store.remove_from_toolbar(id),
        ListOp::AddToToolbar(id) => {
            store.add_to_toolbar(id);
        }
        ListOp::Delete(id) => store.remove(id),
        ListOp::Done | ListOp::Add | ListOp::Edit(_) => {}
    }
}

/// What the host does with Save: whether it went on the bar.
fn save(editor: &mut ShortcutEditor, store: &mut ShortcutStore) -> Option<bool> {
    let lookup = |id| store.shortcuts().iter().find(|s| s.id == id).cloned();
    match editor.save(lookup)? {
        EditorSave::Add(s) => Some(store.add(s)),
        EditorSave::Update(s) => {
            store.update(s);
            Some(true)
        }
    }
}

#[test]
fn the_page_is_a_message_the_screen_takes_and_available_has_every_shortcut_off_the_bar() {
    let mut store = store();
    let ls = mine("ls");
    store.add(ls.clone());
    store.remove_from_toolbar(ls.id);
    let json = page_message(store.shortcuts(), plain);
    let list_screen::Message::Shortcuts(shown) = list_screen::Message::parse(&json).unwrap();
    assert!(
        shown.available.iter().any(|s| s.id == ls.id.to_string()),
        "a custom shortcut off the bar can go back"
    );
    assert_eq!(shown.toolbar.len(), store.toolbar_count());
    assert!(
        shown.toolbar.iter().any(|s| s.kind == "files"),
        "the files chip, by its real kind"
    );
    assert_eq!(shown.custom.len(), 1);
    assert_eq!(shown.limit, 12);
}

/// B233: one past the twelfth that an older set marked as on the bar is not
/// on it, and is under AVAILABLE (Android listed only those not marked).
#[test]
fn one_marked_on_the_bar_past_the_twelfth_is_available() {
    let mut all: Vec<TerminalShortcut> = (0..13).map(|i| mine(&format!("s{i}"))).collect();
    for s in &mut all {
        s.in_toolbar = true;
    }
    let shown = page(&all, plain);
    assert_eq!(shown.toolbar.len(), 12);
    assert_eq!(shown.available.len(), 1);
    assert_eq!(shown.available[0].id, all[12].id.to_string());
}

/// B234: a built-in is called by the name the host gives it.
#[test]
fn a_built_in_is_named_as_the_host_names_it() {
    let store = store();
    let shown = page(store.shortcuts(), |s| {
        if s.is_builtin {
            format!("«{}»", s.summary)
        } else {
            s.summary.clone()
        }
    });
    assert!(shown.toolbar.iter().all(|s| s.summary.starts_with('«')));
}

#[test]
fn the_pages_changes_go_through_the_store() {
    let mut store = store();
    let first = store.toolbar()[0].clone();
    let second = store.toolbar()[1].clone();
    let op = list_op(
        &json!({"ev": "move", "from": 0, "to": 2}),
        store.shortcuts(),
    )
    .unwrap();
    run(&mut store, op);
    assert_eq!(
        store.toolbar()[..2]
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [second.id, first.id]
    );
    assert_eq!(
        list_op(
            &json!({"ev": "move", "from": 40, "to": 0}),
            store.shortcuts()
        ),
        None
    );
    let count = store.toolbar_count();
    assert_eq!(
        list_op(
            &json!({"ev": "move", "from": 0, "to": count + 1}),
            store.shortcuts()
        ),
        None,
        "a move off the bar"
    );
    let id = first.id.to_string();
    let op = list_op(
        &json!({"ev": "remove-from-toolbar", "id": id}),
        store.shortcuts(),
    )
    .unwrap();
    run(&mut store, op);
    assert!(!store.toolbar().iter().any(|s| s.id == first.id));
    assert!(
        store.shortcuts().iter().any(|s| s.id == first.id),
        "a built-in stays, off the bar"
    );
    let op = list_op(
        &json!({"ev": "add-to-toolbar", "id": id}),
        store.shortcuts(),
    )
    .unwrap();
    run(&mut store, op);
    assert!(store.toolbar().iter().any(|s| s.id == first.id));
    assert_eq!(
        list_op(&json!({"ev": "delete", "id": id}), store.shortcuts()),
        None,
        "a built-in is not deleted"
    );
    assert_eq!(
        list_op(&json!({"ev": "edit", "id": id}), store.shortcuts()),
        None,
        "a built-in is not edited"
    );
    let x = mine("x");
    store.add(x.clone());
    let xid = x.id.to_string();
    assert_eq!(
        list_op(&json!({"ev": "edit", "id": xid}), store.shortcuts()),
        Some(ListOp::Edit(x.id)),
        "the user's own opens its editor, wherever it is listed (B285)"
    );
    let op = list_op(&json!({"ev": "delete", "id": xid}), store.shortcuts()).unwrap();
    run(&mut store, op);
    assert!(!store.shortcuts().iter().any(|s| s.id == x.id));
    assert_eq!(
        list_op(&json!({"ev": "delete", "id": xid}), store.shortcuts()),
        None,
        "a shortcut it does not have"
    );
    assert_eq!(
        list_op(&json!({"ev": "done"}), store.shortcuts()),
        Some(ListOp::Done)
    );
    assert_eq!(
        list_op(&json!({"ev": "add"}), store.shortcuts()),
        Some(ListOp::Add)
    );
    assert_eq!(list_op(&json!({"ev": "jump"}), store.shortcuts()), None);
    assert_eq!(
        list_op(&json!({"ev": "delete", "id": "nope"}), store.shortcuts()),
        None
    );
}

#[test]
fn the_editors_draft_is_a_message_and_save_saves() {
    let mut store = store();
    let mut editor = ShortcutEditor::new(None);
    assert!(editor_screen::Message::parse(&editor.message(vec![], 0)).is_ok());
    assert_eq!(
        editor.draft.color_id, "accent",
        "a new shortcut is in the accent"
    );
    assert!(editor.draft.repeat_on_hold);
    assert_eq!(editor.draft.kind, "keyCombo");
    assert_eq!(save(&mut editor, &mut store), None, "nothing to save yet");
    editor.apply(&json!({
        "kind": "keyCombo", "modifiers": ["shift", "ctrl", "nonsense", "ctrl"], "key": "bbbbbbbb",
        "chipLabel": "C-b-long", "summary": "", "colorId": "mosh", "scopeHosts": ["nas", "m1"],
        "onlyInTmux": true, "payload": "", "appendReturn": false, "repeatOnHold": true
    }));
    assert_eq!(editor.draft.key, "bbbbbb", "six characters at most");
    assert_eq!(editor.draft.chip_label, "C-b-lo");
    assert_eq!(editor.draft.modifiers, ["ctrl", "shift"], "⌃⌥⇧⌘, once each");
    assert_eq!(editor.draft.scope_hosts, ["m1", "nas"]);
    let json = editor.message(vec!["m1".into(), "nas".into()], store.toolbar_count());
    assert!(editor_screen::Message::parse(&json).is_ok(), "{json}");
    let count = store.shortcuts().len();
    assert_eq!(save(&mut editor, &mut store), Some(true));
    let saved = store.shortcuts().last().unwrap().clone();
    assert_eq!(store.shortcuts().len(), count + 1);
    assert_eq!(
        saved.summary, "C-b-lo",
        "an empty summary is the chip label"
    );
    assert_eq!(saved.color_id, "mosh");
    assert_eq!(saved.scope_hosts.len(), 2);
    assert!(saved.only_in_tmux);
    assert_eq!(
        saved.modifiers.iter().copied().collect::<Vec<_>>(),
        [ShortcutModifier::Ctrl, ShortcutModifier::Shift]
    );

    // Editing keeps its id and updates in place.
    let mut edit = ShortcutEditor::new(Some(&saved));
    let shown: Value = serde_json::from_str(&edit.message(vec![], 0)).unwrap();
    assert_eq!(shown["editing"], true);
    edit.apply(&json!({"summary": "prefix"}));
    assert_eq!(save(&mut edit, &mut store), Some(true));
    assert_eq!(store.shortcuts().len(), count + 1);
    assert_eq!(store.shortcuts().last().unwrap().summary, "prefix");
    assert_eq!(store.shortcuts().last().unwrap().id, saved.id);
}

#[test]
fn what_the_draft_does_not_read_stays_as_it_was() {
    let mut editor = ShortcutEditor::new(None);
    let before = editor.clone();
    editor.apply(&json!({
        "kind": "rocket", "modifiers": "ctrl", "key": 7, "colorId": "neon", "scopeHosts": "nas",
        "onlyInTmux": "yes"
    }));
    assert_eq!(editor, before);
    editor.apply(&json!({}));
    assert_eq!(editor, before);
}

/// The chip's label holds six characters as a person counts them (Swift's
/// `prefix(6)`; Android's `chars().take(6)` cut a family emoji apart).
#[test]
fn six_characters_as_a_person_counts_them() {
    let mut editor = ShortcutEditor::new(None);
    editor.apply(&json!({"chipLabel": "👍🏽👍🏽👍🏽👍🏽👍🏽👍🏽👍🏽", "key": "👨‍👩‍👧x"}));
    assert_eq!(editor.draft.chip_label, "👍🏽".repeat(6));
    assert_eq!(editor.draft.key, "👨‍👩‍👧x");
}

/// B252: a new shortcut saved while the bar is full went under AVAILABLE
/// in silence. The store says it did not go on the bar; an edit of one
/// already saved never moves it.
#[test]
fn save_with_the_bar_full_says_it_is_under_available() {
    let mut store = store();
    let mut n = 0;
    while !store.toolbar_is_full() {
        store.add(mine(&format!("f{n}")));
        n += 1;
    }
    let mut editor = ShortcutEditor::new(None);
    editor.apply(&json!({"kind": "text", "payload": "git status", "chipLabel": "gs"}));
    assert_eq!(
        save(&mut editor, &mut store),
        Some(false),
        "saved off a full bar in silence"
    );
    let saved = store.shortcuts().last().unwrap().clone();
    assert!(saved.chip_label == "gs" && !saved.in_toolbar);
    assert!(store.available().iter().any(|s| s.id == saved.id));

    // A second Save of it updates it, off the bar as the store has it (the
    // copy iOS held said "on the bar").
    editor.apply(&json!({"summary": "status"}));
    assert_eq!(save(&mut editor, &mut store), Some(true));
    let again = store.shortcuts().last().unwrap();
    assert_eq!(again.id, saved.id);
    assert!(!again.in_toolbar, "a second Save put it on the bar");

    let mut edit = ShortcutEditor::new(Some(&saved));
    edit.apply(&json!({"summary": "st"}));
    assert_eq!(
        save(&mut edit, &mut store),
        Some(true),
        "an edit isn't a new place on the bar"
    );
    assert!(!store.shortcuts().last().unwrap().in_toolbar);
}

/// B288: a second Save of a new shortcut (a double tap while the sheet
/// goes away) added a second copy under a fresh id.
#[test]
fn a_new_shortcut_saved_twice_is_one_shortcut() {
    let mut store = store();
    let count = store.shortcuts().len();
    let mut editor = ShortcutEditor::new(None);
    editor.apply(&json!({"kind": "text", "payload": "git status", "chipLabel": "gs"}));
    assert_eq!(save(&mut editor, &mut store), Some(true));
    editor.apply(&json!({"summary": "status"}));
    assert_eq!(save(&mut editor, &mut store), Some(true));
    assert_eq!(
        store.shortcuts().len(),
        count + 1,
        "the second Save added another copy"
    );
    assert_eq!(store.shortcuts().last().unwrap().summary, "status");
}

/// An edit writes over the store's copy: where the shortcut sits on the bar
/// is the store's, even when it moved after the editor opened.
#[test]
fn an_edit_keeps_where_the_store_has_it() {
    let mut store = store();
    let x = mine("x");
    store.add(x.clone());
    let opened = store.shortcuts().last().unwrap().clone();
    let mut edit = ShortcutEditor::new(Some(&opened));
    store.remove_from_toolbar(x.id);
    edit.apply(&json!({"summary": "renamed"}));
    assert_eq!(save(&mut edit, &mut store), Some(true));
    let now = store.shortcuts().iter().find(|s| s.id == x.id).unwrap();
    assert_eq!(now.summary, "renamed");
    assert!(!now.in_toolbar, "the edit put it back on the bar");
}
