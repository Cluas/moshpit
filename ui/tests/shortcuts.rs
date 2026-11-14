//! Settings › Shortcuts and the shortcut editor on a headless engine: the
//! bar, the user's own and the rest as the host posts them, the edit
//! controls (−, a delete asking the system's alert, ＋, a drag of a handle
//! reordering the bar) and what each tells the host, the editor's preview,
//! quick keys, trigger, display and scope, the app's language and the
//! identifiers the device tests find.

mod common;

use common::{emit, hear, next, post, quiet, start, start_observing, wait_dialog, zh_hans};
use moshpit_ui::{
    add_shortcut::{self, Draft, Editor},
    screen::{Params, Screen},
    shortcuts::{self, Shortcut, Shortcuts},
};
use nsh_core::geometry::{point, px};
use nsh_testing::DialogAnswer;
use serde_json::json;

fn shortcut(id: &str, kind: &str, label: &str, summary: &str, builtin: bool) -> Shortcut {
    Shortcut {
        id: id.into(),
        kind: kind.into(),
        label: label.into(),
        summary: summary.into(),
        builtin,
    }
}

fn bar() -> Vec<Shortcut> {
    vec![
        shortcut("esc", "keyCombo", "esc", "Escape", true),
        shortcut("tab", "keyCombo", "tab", "Tab", true),
        shortcut("ctrlc", "keyCombo", "^C", "Interrupt (SIGINT)", true),
        shortcut("files", "files", "files", "Browse files", true),
        shortcut("mine", "text", "gs", "git status", false),
    ]
}

fn shortcuts(toolbar: Vec<Shortcut>, custom: Vec<Shortcut>, available: Vec<Shortcut>) -> String {
    shortcuts::Message::Shortcuts(Shortcuts {
        toolbar,
        custom,
        available,
        limit: 12,
    })
    .to_json()
}

/// The top and the height of the view `selector` names, as laid out.
fn frame(app: &nsh_testing::Headless, selector: &str) -> (f32, f32) {
    let id = app.screen().id(selector).unwrap();
    let bounds = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("{selector} is laid out"))
        .bounds;
    (bounds.origin.y.0, bounds.size.height.0)
}

fn standard() -> String {
    shortcuts(
        bar(),
        vec![shortcut("mine", "text", "gs", "git status", false)],
        vec![shortcut("ctrll", "keyCombo", "^L", "Clear screen", true)],
    )
}

#[test]
fn the_groups_show_what_the_host_posts() {
    let (mut app, channel, natives) =
        start_observing(Screen::Shortcuts, Params::default(), "shortcuts");
    post(&mut app, &channel, &standard());
    app.wait_for("id:shortcut-row-esc").unwrap();
    for text in [
        "Shortcuts",
        "IN THE BAR · 5 OF 12",
        "Escape",
        "Interrupt (SIGINT)",
        "Browse files",
        "YOURS",
        "git status",
        "AVAILABLE",
        "Clear screen",
        "^L",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(!app.screen().matches("id:shortcuts-no-custom").unwrap());
    assert!(app.screen().matches("id:shortcuts-preview").unwrap());
    // The files shortcut is drawn as its folder, in the strip and in its row.
    let folders = natives.count("moshpit/symbol");
    assert!(folders >= 2, "{folders}");
    assert!(app.screen().matches("id:shortcut-grip-esc").unwrap());
    assert!(app.screen().matches("id:shortcut-remove-esc").unwrap());
    assert!(app.screen().matches("id:shortcut-add-ctrll").unwrap());
    assert!(
        app.screen()
            .matches("label:Add Clear screen to toolbar")
            .unwrap()
    );
    assert!(app.screen().matches("label:Reorder Escape").unwrap());
    channel.close();
}

#[test]
fn no_custom_shortcut_says_so() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    post(&mut app, &channel, &shortcuts(bar(), vec![], vec![]));
    app.wait_for("id:shortcuts-no-custom").unwrap();
    assert!(app.screen().shows("No custom shortcuts yet"));
    channel.close();
}

/// − on a bar row takes the shortcut off at once (it goes to Available);
/// − on one of the user's own deletes it, once the system's alert asked.
#[test]
fn minus_takes_off_at_once_and_a_delete_asks_the_system_alert() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    app.answer_dialogs_myself();
    let told = hear(&channel);
    post(&mut app, &channel, &standard());
    app.wait_for("id:shortcut-remove-esc").unwrap();
    assert!(
        app.screen()
            .matches("label:Remove Escape from toolbar")
            .unwrap()
    );
    app.press("id:shortcut-remove-esc").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "remove-from-toolbar", "id": "esc"})
    );
    assert!(app.pending_dialog().is_none(), "taking off asks nothing");
    app.wait_for("id:shortcut-delete-mine").unwrap();
    assert!(app.screen().matches("label:Delete git status").unwrap());
    app.press("id:shortcut-delete-mine").unwrap();
    assert!(wait_dialog(&mut app), "the delete never asked");
    let alert = app.pending_dialog().cloned().unwrap();
    assert_eq!(alert.title.as_deref(), Some("Delete gs?"));
    assert_eq!(common::dialog_buttons(&alert), ["Delete", "Cancel"]);
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    quiet(&told);
    app.press("id:shortcut-delete-mine").unwrap();
    assert!(wait_dialog(&mut app));
    app.answer_dialog(DialogAnswer::Press("Delete".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "delete", "id": "mine"}));
    channel.close();
}

#[test]
fn plus_edit_add_and_done_reach_the_host() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    let told = hear(&channel);
    post(
        &mut app,
        &channel,
        &shortcuts(
            bar()[..4].to_vec(),
            vec![shortcut("mine", "text", "gs", "git status", false)],
            vec![
                shortcut("ctrll", "keyCombo", "^L", "Clear screen", true),
                shortcut("mine", "text", "gs", "git status", false),
            ],
        ),
    );
    app.wait_for("id:shortcut-add-ctrll").unwrap();
    app.press("id:shortcut-add-ctrll").unwrap();
    assert_eq!(next(&told), json!({"ev": "add-to-toolbar", "id": "ctrll"}));
    app.press("id:shortcuts-add").unwrap();
    assert_eq!(next(&told), json!({"ev": "add"}));
    app.press("id:shortcuts-done").unwrap();
    assert_eq!(next(&told), json!({"ev": "done"}));
    // A built-in's row opens nothing; a custom shortcut's opens its editor,
    // in Yours and in Available.
    let _ = app.press("id:shortcut-row-esc");
    quiet(&told);
    let _ = app.press("id:shortcut-available-ctrll");
    quiet(&told);
    app.press("id:shortcut-custom-mine").unwrap();
    assert_eq!(next(&told), json!({"ev": "edit", "id": "mine"}));
    app.press("id:shortcut-available-mine").unwrap();
    assert_eq!(next(&told), json!({"ev": "edit", "id": "mine"}));
    channel.close();
}

/// R11 (§8): with 12 on the bar, Available is disabled — every ＋ in it —
/// and a line under its title says how many the bar holds (B233).
#[test]
fn a_full_bar_disables_available() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    let told = hear(&channel);
    let full: Vec<Shortcut> = (0..12)
        .map(|i| {
            shortcut(
                &format!("s{i}"),
                "keyCombo",
                &format!("k{i}"),
                &format!("key {i}"),
                true,
            )
        })
        .collect();
    let available = vec![
        shortcut("ctrll", "keyCombo", "^L", "Clear screen", true),
        shortcut("ctrlz", "keyCombo", "^Z", "Suspend", true),
    ];
    post(
        &mut app,
        &channel,
        &shortcuts(full, vec![], available.clone()),
    );
    app.wait_for("id:shortcut-add-ctrll").unwrap();
    assert!(app.screen().shows("IN THE BAR · 12 OF 12"));
    for id in ["id:shortcut-add-ctrll", "id:shortcut-add-ctrlz"] {
        assert!(!app.screen().enabled(id).unwrap(), "{id}");
    }
    let why = "The bar holds up to 12.";
    assert!(app.screen().shows(why), "{:?}", app.screen().texts());
    let _ = app.press("id:shortcut-add-ctrll");
    quiet(&told);
    // A slot frees: the ＋ works and the line goes.
    post(&mut app, &channel, &shortcuts(bar(), vec![], available));
    app.wait_for("id:shortcut-row-esc").unwrap();
    assert!(app.screen().enabled("id:shortcut-add-ctrll").unwrap());
    assert!(!app.screen().shows(why));
    channel.close();
}

/// R11 (§8): a row dragged by its handle tells the host the new order
/// (`move`), and the page shows it at once.
#[test]
fn dragging_a_handle_tells_the_host_the_order() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    let told = hear(&channel);
    post(&mut app, &channel, &standard());
    app.wait_for("id:shortcut-grip-esc").unwrap();
    let (_, height) = frame(&app, "id:shortcut-row-esc");
    // esc, two rows and a bit down: after ^C, before files.
    let down = 2.0 * height + 8.0;
    app.drag("id:shortcut-grip-esc", point(px(0.0), px(down)))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "move", "from": 0, "to": 3}));
    let order: Vec<f32> = ["esc", "tab", "ctrlc"]
        .iter()
        .map(|id| frame(&app, &format!("id:shortcut-row-{id}")).0)
        .collect();
    assert!(
        order[1] < order[2] && order[2] < order[0],
        "tab, ^C, then esc: {order:?}"
    );
    quiet(&told);
    channel.close();
}

#[test]
fn the_shortcuts_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::Shortcuts, zh_hans(), "shortcuts");
    post(&mut app, &channel, &standard());
    app.wait_for("id:shortcut-row-esc").unwrap();
    for text in ["快捷键", "完成", "在栏上 · 5/12", "自己做的", "可以加的"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

// --- The editor --------------------------------------------------------------

mod editor {
    use super::*;

    fn editor(draft: Draft, editing: bool) -> String {
        add_shortcut::Message::Shortcut(Editor {
            draft,
            editing,
            hosts: vec!["m1-pro".into(), "nas".into()],
            toolbar_count: 7,
            limit: 12,
        })
        .to_json()
    }

    fn new_draft() -> Draft {
        Draft {
            kind: "keyCombo".into(),
            repeat_on_hold: true,
            color_id: "accent".into(),
            ..Draft::default()
        }
    }

    /// The last draft the page told the host, after whatever else it told.
    fn last_draft(told: &std::sync::mpsc::Receiver<String>) -> serde_json::Value {
        let mut draft = serde_json::Value::Null;
        while let Ok(json) = told.recv_timeout(std::time::Duration::from_millis(300)) {
            let event: serde_json::Value = serde_json::from_str(&json).unwrap();
            if event["ev"] == json!("change") {
                draft = event["draft"].clone();
            }
        }
        draft
    }

    #[test]
    fn a_new_key_combo_shows_every_group() {
        let (mut app, channel, natives) =
            start_observing(Screen::AddShortcut, Params::default(), "add-shortcut");
        post(&mut app, &channel, &editor(new_draft(), false));
        app.wait_for("id:shortcut-key").unwrap();
        for text in [
            "Add Shortcut",
            "SLOT 7/12",
            "unnamed",
            "QUICK KEYS",
            "PREFIX",
            "F-KEYS",
            "TRIGGER",
            "⌃ Control",
            "⌘ Command",
            "Main key",
            "Sends",
            "ACTION",
            "Payload",
            "Append Return",
            "Repeat on hold",
            "DISPLAY",
            "Chip Label",
            "At most 6 characters.",
            "Description",
            "Gray",
            "Amber",
            "SCOPE",
            "Only in a multiplexer",
            "m1-pro",
        ] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        let kind = natives.find("moshpit/segmented", "shortcut-type").unwrap();
        assert_eq!(natives.prop(kind, "selected"), Some(json!(0)));
        assert!(app.screen().selected("id:shortcut-color-accent").unwrap());
        assert!(
            !app.screen().enabled("id:shortcut-save").unwrap(),
            "nothing to save yet"
        );
        channel.close();
    }

    #[test]
    fn a_quick_key_fills_the_trigger_and_save_follows() {
        let (mut app, channel) = start(Screen::AddShortcut, Params::default(), "add-shortcut");
        let told = hear(&channel);
        post(&mut app, &channel, &editor(new_draft(), false));
        app.wait_for("id:shortcut-preset-PREFIX").unwrap();
        app.press("id:shortcut-preset-PREFIX").unwrap();
        let draft = last_draft(&told);
        assert_eq!(draft["modifiers"], json!(["ctrl"]));
        assert_eq!(draft["key"], json!("b"));
        assert_eq!(draft["chipLabel"], json!("C-b"));
        assert_eq!(draft["summary"], json!("multiplexer prefix"));
        assert_eq!(
            app.screen().input_text("id:shortcut-key-input").unwrap(),
            Some("b")
        );
        assert_eq!(
            app.screen()
                .input_text("id:shortcut-chip-label-input")
                .unwrap(),
            Some("C-b")
        );
        assert!(app.screen().shows("multiplexer prefix · sends ⌃ B"));
        assert!(app.screen().enabled("id:shortcut-save").unwrap());
        assert!(app.screen().selected("id:shortcut-modifier-ctrl").unwrap());
        app.press("id:shortcut-modifier-shift").unwrap();
        assert_eq!(last_draft(&told)["modifiers"], json!(["ctrl", "shift"]));
        assert!(app.screen().shows("⌃ ⇧ B"));
        app.press("id:shortcut-save").unwrap();
        assert_eq!(next(&told), json!({"ev": "save"}));
        app.press("id:shortcut-cancel").unwrap();
        assert_eq!(next(&told), json!({"ev": "cancel"}));
        channel.close();
    }

    /// A shortcut bound to a host the user no longer has (renamed or
    /// deleted since): its editor shows that binding, picked, so it can be
    /// taken off; otherwise the shortcut is in no toolbar at all while the
    /// editor says it is everywhere.
    #[test]
    fn a_binding_to_a_host_gone_since_shows_and_comes_off() {
        let (mut app, channel) = start(Screen::AddShortcut, Params::default(), "add-shortcut");
        let told = hear(&channel);
        let mut draft = new_draft();
        draft.kind = "text".into();
        draft.scope_hosts = vec!["nas".into(), "old-box".into()];
        post(&mut app, &channel, &editor(draft, true));
        app.wait_for("id:shortcut-host-nas").unwrap();
        assert!(
            app.screen().matches("id:shortcut-host-old-box").unwrap(),
            "the gone host's binding is not shown: {:?}",
            app.screen().texts()
        );
        assert!(app.screen().selected("id:shortcut-host-old-box").unwrap());
        app.press("id:shortcut-host-old-box").unwrap();
        assert_eq!(last_draft(&told)["scopeHosts"], json!(["nas"]));
        channel.close();
    }

    #[test]
    fn text_and_command_hide_the_trigger_and_tell_the_host() {
        let (mut app, channel, natives) =
            start_observing(Screen::AddShortcut, Params::default(), "add-shortcut");
        let told = hear(&channel);
        post(&mut app, &channel, &editor(new_draft(), false));
        app.wait_for("id:shortcut-key").unwrap();
        let kind = natives.find("moshpit/segmented", "shortcut-type").unwrap();
        emit(&mut app, kind, "change", json!(2));
        assert_eq!(last_draft(&told)["kind"], json!("command"));
        assert!(!app.screen().matches("id:shortcut-key").unwrap());
        assert!(!app.screen().shows("QUICK KEYS"));
        app.press("id:shortcut-color-amber").unwrap();
        assert_eq!(last_draft(&told)["colorId"], json!("amber"));
        app.press("id:shortcut-host-nas").unwrap();
        assert_eq!(last_draft(&told)["scopeHosts"], json!(["nas"]));
        let only = natives
            .find("moshpit/switch", "shortcut-only-multiplexer")
            .unwrap();
        emit(&mut app, only, "change", json!(true));
        assert_eq!(last_draft(&told)["onlyInTmux"], json!(true));
        channel.close();
    }

    #[test]
    fn an_existing_shortcut_is_edited() {
        let (mut app, channel) = start(Screen::AddShortcut, Params::default(), "add-shortcut");
        post(
            &mut app,
            &channel,
            &editor(
                Draft {
                    kind: "text".into(),
                    payload: "git status".into(),
                    chip_label: "gs".into(),
                    summary: "status".into(),
                    ..new_draft()
                },
                true,
            ),
        );
        app.wait_for("id:shortcut-payload").unwrap();
        assert!(app.screen().shows("Edit Shortcut"));
        assert!(app.screen().shows("status · types \"git status\""));
        assert_eq!(
            app.screen().input_text("id:shortcut-payload").unwrap(),
            Some("git status")
        );
        assert!(app.screen().enabled("id:shortcut-save").unwrap());
        channel.close();
    }

    #[test]
    fn the_editor_speaks_the_apps_language() {
        let (mut app, channel) = start(Screen::AddShortcut, zh_hans(), "add-shortcut");
        post(&mut app, &channel, &editor(new_draft(), false));
        app.wait_for("id:shortcut-key").unwrap();
        for text in ["添加快捷键", "取消", "保存", "发送", "最多 6 个字符。"] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        channel.close();
    }

    /// B234: the quick keys' groups and what a quick key fills in were
    /// English in every language.
    #[test]
    fn the_quick_keys_speak_the_apps_language() {
        let (mut app, channel) = start(Screen::AddShortcut, zh_hans(), "add-shortcut");
        let told = hear(&channel);
        post(&mut app, &channel, &editor(new_draft(), false));
        app.wait_for("id:shortcut-preset-C-c").unwrap();
        for text in ["前缀键", "控制键", "特殊键", "移动", "功能键"] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        // (PREFIX is also a quick key's own label, which stays.)
        for english in ["CONTROL", "SPECIAL", "NAVIGATION", "F-KEYS"] {
            assert!(!app.screen().shows(english), "{english} untranslated");
        }
        app.press("id:shortcut-preset-C-c").unwrap();
        assert_eq!(last_draft(&told)["summary"], json!("中断"));
        channel.close();
    }
}

#[test]
fn the_shortcuts_page_follows_dynamic_type() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    post(&mut app, &channel, &standard());
    app.wait_for("id:shortcut-row-esc").unwrap();
    let size = |app: &nsh_testing::Headless, selector: &str| {
        app.screen().span_style(selector).unwrap().font_size.0
    };
    assert_eq!(size(&app, "text:Escape"), 17.0, "the summary is the body");
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Escape"), 23.0);
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
    assert!(app.screen().matches("id:shortcut-row-esc").unwrap());
    channel.close();
}

#[test]
fn the_grips_are_labelled_for_voiceover() {
    let (mut app, channel) = start(Screen::Shortcuts, Params::default(), "shortcuts");
    let told = hear(&channel);
    post(&mut app, &channel, &standard());
    app.wait_for("id:shortcut-grip-tab").unwrap();
    // The grip is an adjustable element called after its shortcut (its
    // increment and decrement are run on the device: VoiceOver's swipes).
    assert!(app.screen().matches("label:Reorder Tab").unwrap());
    quiet(&told);
    channel.close();
}
