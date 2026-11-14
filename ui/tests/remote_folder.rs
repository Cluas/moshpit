//! A host folder on a headless engine (R13, `docs/nsh/UI-DESIGN.md` §6.9):
//! the grid and the rows the host posts, a tap opens, the skeleton while
//! the first listing comes, an empty or a failed folder said on the page
//! with Upload… or Retry, the title's menu climbs, the ＋ menu's New
//! folder… asks first, a menu's pick renames after asking and deletes after
//! NSH's action sheet, a failure is said once in an alert.

mod common;

use common::{Natives, hear, next, post, quiet, start_observing, wait_dialog, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use nsh_testing::{DialogAnswer, Headless};
use serde_json::{Value, json};

fn folder(changes: Value) -> String {
    let mut base = json!({
        "op": "folder", "name": "work", "path": "/srv/work", "parents": ["/srv", "/"],
        "pushed": false, "loading": false,
        "media": [
            {"path": "/srv/work/shot.png", "name": "shot.png", "symbol": "photo", "meta": "2 min. ago · 120 KB", "thumb": true},
            {"path": "/srv/work/demo.mov", "name": "demo.mov", "video": true, "symbol": "film", "meta": "1 hr. ago · 2 MB", "duration": "0:42"}
        ],
        "rows": [
            {"path": "/srv/work/logs", "name": "logs", "dir": true, "symbol": "folder", "meta": "3 hr. ago"},
            {"path": "/srv/work/notes.md", "name": "notes.md", "symbol": "doc.text", "meta": "5 days ago · 2 KB"}
        ]
    });
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base.to_string()
}

fn open(
    params: Params,
) -> (
    common::App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    Natives,
) {
    let (mut app, channel, natives) =
        start_observing(Screen::RemoteFolder, params, "remote-folder-sheet");
    app.answer_dialogs_myself();
    (app, channel, natives)
}

#[test]
fn the_folder_shows_its_media_and_rows_and_a_tap_opens() {
    let (mut app, channel, natives) = open(Params::default());
    let heard = hear(&channel);
    post(&mut app, &channel, &folder(json!({})));
    app.wait_for("id:remote-folder-item-/srv/work/notes.md")
        .unwrap();
    for text in [
        "work",
        "shot.png",
        "demo.mov",
        "0:42",
        "logs",
        "notes.md",
        "5 days ago · 2 KB",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    // The thumbnail is the host's; a tile without one shows its symbol.
    assert_eq!(natives.count("moshpit/remote-thumbnail"), 1);
    assert!(natives.find("moshpit/menu", "remote-folder-add").is_some());
    app.press("id:remote-folder-item-/srv/work/logs").unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "open", "path": "/srv/work/logs"})
    );
    app.press("id:remote-folder-item-/srv/work/shot.png")
        .unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "open", "path": "/srv/work/shot.png"})
    );
    quiet(&heard);
}

#[test]
fn empty_failed_and_loading_folders_say_so() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    post(
        &mut app,
        &channel,
        &folder(json!({"media": [], "rows": []})),
    );
    app.wait_for("text:This folder is empty").unwrap();
    assert!(
        app.screen()
            .shows("Only images, video, audio, PDFs and documents are shown here.")
    );
    app.press("id:remote-folder-upload").unwrap();
    assert_eq!(next(&heard), json!({"ev": "add", "item": "files"}));
    post(
        &mut app,
        &channel,
        &folder(json!({"media": [], "rows": [], "failure": "Permission denied"})),
    );
    app.wait_for("text:Couldn't open this folder").unwrap();
    assert!(app.screen().shows("Permission denied"));
    app.press("id:remote-folder-retry").unwrap();
    assert_eq!(next(&heard), json!({"ev": "refresh"}));
    // Loading with nothing to show yet: the skeleton, six tiles and four
    // rows, hidden from screen readers but for its one busy element.
    post(
        &mut app,
        &channel,
        &folder(json!({"media": [], "rows": [], "loading": true})),
    );
    app.wait_for("id:remote-folder-skeleton").unwrap();
    assert!(app.screen().matches("label:Loading").unwrap());
    // The listing comes: the skeleton goes.
    post(&mut app, &channel, &folder(json!({})));
    app.wait_for("id:remote-folder-item-/srv/work/logs")
        .unwrap();
    app.wait_for_gone("id:remote-folder-skeleton", common::WAIT)
        .unwrap();
    post(&mut app, &channel, &folder(json!({"uploading": 0.42})));
    app.wait_for("text:UPLOADING 42%")
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    quiet(&heard);
}

#[test]
fn the_add_menu_says_its_choices_in_the_redesigns_words() {
    let (mut app, channel, natives) = open(Params::default());
    post(&mut app, &channel, &folder(json!({})));
    app.wait_for("id:remote-folder-item-/srv/work/logs")
        .unwrap();
    let add = natives.find("moshpit/menu", "remote-folder-add").unwrap();
    assert_eq!(
        natives.prop(add, "items"),
        Some(json!([
            {"id": "photos", "title": "Photos…", "image": "photo.on.rectangle"},
            {"id": "files", "title": "Files…", "image": "doc.badge.plus"},
            {"id": "folder", "title": "New folder…", "image": "folder.badge.plus"},
        ]))
    );
    // The rows are one section's, each a 44 pt target at least, a folder's
    // with its chevron.
    let row = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some("remote-folder-item-/srv/work/logs"))
        .unwrap()
        .bounds;
    assert!(row.size.height.0 >= 44.0, "{row:?}");
    assert!(app.screen().matches("id:remote-folder-rows").unwrap());
}

#[test]
fn a_menu_renames_after_asking_and_deletes_after_the_sheet() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    post(&mut app, &channel, &folder(json!({})));
    app.wait_for("id:remote-folder-item-/srv/work/logs")
        .unwrap();
    post(
        &mut app,
        &channel,
        r#"{"op":"act","action":"rename","path":"/srv/work/notes.md"}"#,
    );
    let prompt = next(&heard);
    assert_eq!(prompt["title"], "Rename");
    assert_eq!(prompt["text"], "notes.md");
    let id = prompt["id"].as_u64().unwrap();
    post(
        &mut app,
        &channel,
        &json!({"op": "prompted", "id": id, "text": "todo.md"}).to_string(),
    );
    assert_eq!(
        next(&heard),
        json!({"ev": "rename", "path": "/srv/work/notes.md", "name": "todo.md"})
    );
    post(
        &mut app,
        &channel,
        r#"{"op":"act","action":"delete","path":"/srv/work/logs"}"#,
    );
    assert!(wait_dialog(&mut app), "the sheet never asked");
    let dialog = app.pending_dialog().cloned().expect("the sheet asks");
    assert!(dialog.sheet);
    assert_eq!(dialog.title.as_deref(), Some("Delete logs?"));
    assert_eq!(
        dialog.message.as_deref(),
        Some("“logs” and everything in it will be deleted from the host.")
    );
    app.answer_dialog(DialogAnswer::Press("Delete".into()))
        .unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "delete", "path": "/srv/work/logs"})
    );
    post(
        &mut app,
        &channel,
        r#"{"op":"act","action":"share","path":"/srv/work/notes.md"}"#,
    );
    assert_eq!(
        next(&heard),
        json!({"ev": "share", "path": "/srv/work/notes.md"})
    );
}

/// The folder named `name` with the open failure `message` numbered `id`:
/// the name is what a step waits for to know the screen took the post.
fn failing(name: &str, message: Option<&str>, id: u64) -> String {
    let mut changes = json!({"name": name});
    if let Some(message) = message {
        changes["openFailure"] = json!(message);
        changes["openFailureId"] = json!(id);
    }
    folder(changes)
}

/// Posts `json` and waits until the screen shows `name` (what it took).
fn post_and_see(
    app: &mut Headless,
    channel: &moshpit_ui::channel::Channel,
    json: &str,
    name: &str,
) {
    post(app, channel, json);
    app.wait_for(&format!("text:{name}"))
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
}

#[test]
fn a_failure_is_said_once_and_its_ok_clears_it() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    post(
        &mut app,
        &channel,
        &failing("work", Some("The file is gone."), 1),
    );
    assert!(wait_dialog(&mut app), "the failure went unsaid");
    let dialog = app.pending_dialog().cloned().expect("an alert");
    assert!(!dialog.sheet);
    assert_eq!(dialog.title.as_deref(), Some("CAN'T OPEN"));
    // Posted again before it is answered (a thumbnail came in): still one
    // alert.
    post_and_see(
        &mut app,
        &channel,
        &failing("work-2", Some("The file is gone."), 1),
        "work-2",
    );
    app.answer_dialog(DialogAnswer::Press("OK".into())).unwrap();
    assert!(app.pending_dialog().is_none(), "the failure was said twice");
    assert_eq!(next(&heard), json!({"ev": "dismiss", "failure": "open"}));
    // A post the host sent before it heard the dismiss still carries it: the
    // same number, already said.
    post_and_see(
        &mut app,
        &channel,
        &failing("work-3", Some("The file is gone."), 1),
        "work-3",
    );
    assert!(
        app.pending_dialog().is_none(),
        "the failure answered with OK was said again"
    );
    // The same file failing again, the host never having posted the cleared
    // state (it posts what it has when it next draws): a new number, said.
    post(
        &mut app,
        &channel,
        &failing("work-4", Some("The file is gone."), 2),
    );
    assert!(
        wait_dialog(&mut app),
        "a new failure in the same words went unsaid"
    );
    app.answer_dialog(DialogAnswer::Press("OK".into())).unwrap();
    assert_eq!(next(&heard), json!({"ev": "dismiss", "failure": "open"}));
    // Cleared, then failing again: said again.
    post_and_see(&mut app, &channel, &failing("work-5", None, 0), "work-5");
    assert!(app.pending_dialog().is_none());
    post(
        &mut app,
        &channel,
        &failing("work-6", Some("The file is gone."), 3),
    );
    assert!(wait_dialog(&mut app), "a new failure went unsaid");
    app.answer_dialog(DialogAnswer::Press("OK".into())).unwrap();
    assert_eq!(next(&heard), json!({"ev": "dismiss", "failure": "open"}));
    // A refused change: its own numbers, its own alert.
    post(
        &mut app,
        &channel,
        &folder(
            json!({"name": "work-7", "editFailure": {"title": "Couldn't rename", "message": "taken"}, "editFailureId": 1}),
        ),
    );
    assert!(wait_dialog(&mut app), "the refused change went unsaid");
    assert_eq!(
        app.pending_dialog()
            .and_then(|d| d.title.clone())
            .as_deref(),
        Some("Couldn't rename")
    );
    app.answer_dialog(DialogAnswer::Press("OK".into())).unwrap();
    assert_eq!(next(&heard), json!({"ev": "dismiss", "failure": "edit"}));
    post_and_see(
        &mut app,
        &channel,
        &folder(
            json!({"name": "work-8", "editFailure": {"title": "Couldn't rename", "message": "taken"}, "editFailureId": 1}),
        ),
        "work-8",
    );
    assert!(
        app.pending_dialog().is_none(),
        "the refused change was said again"
    );
}

#[test]
fn a_pushed_folder_goes_back_and_its_words_are_chinese() {
    let (mut app, channel, _) = open(zh_hans());
    let heard = hear(&channel);
    post(
        &mut app,
        &channel,
        &folder(json!({"pushed": true, "media": [], "rows": []})),
    );
    app.wait_for("text:这个文件夹是空的").unwrap();
    assert!(app.screen().shows("上传…"));
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&heard), json!({"ev": "back"}));
}

/// B302: a row is pressed where a finger lands, as on iOS. The iOS surface
/// makes a scroll view's content as it comes near its viewport (content
/// culling, off in headless runs); a view made that way never gets the
/// `pointer_events(None)` its style says (NSH-GAPS #24), so the rows card's
/// border, once drawn over the rows, took every tap there. With culling on,
/// the step's own check (`covered`) sees what a finger would reach.
#[test]
fn rows_and_tiles_take_a_press_where_a_finger_lands_as_on_ios() {
    let (mut app, channel, _) = open(Params::default());
    app.engine().control().set_content_culling(true);
    let heard = hear(&channel);
    post(&mut app, &channel, &folder(json!({})));
    for path in [
        "/srv/work/logs",
        "/srv/work/notes.md",
        "/srv/work/shot.png",
        "/srv/work/demo.mov",
    ] {
        app.press(&format!("id:remote-folder-item-{path}"))
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(next(&heard), json!({"ev": "open", "path": path}));
    }
}

#[test]
fn the_upload_bar_is_the_systems_on_its_track() {
    let (mut app, channel, natives) = open(Params::default());
    post(&mut app, &channel, &folder(json!({"uploading": 0.42})));
    app.wait_for("text:UPLOADING 42%")
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    let bar = natives.only("moshpit/progress").unwrap();
    assert_eq!(natives.prop(bar, "value"), Some(json!(0.42)));
    assert_eq!(natives.prop(bar, "label"), Some(json!("42%")));
    assert!(
        natives.prop(bar, "track").is_some(),
        "the bar lies on the muted track"
    );
    channel.close();
}
