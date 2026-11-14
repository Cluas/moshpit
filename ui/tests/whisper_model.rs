//! Settings › Voice Input › Model on a headless engine (R15, §6.11): the
//! models the host posts, each its name over its size and what it is for;
//! what its row offers at its trailing end as the model moves from not
//! downloaded to downloading to paused to downloaded (Download, the
//! progress with its percent and Pause, Resume, In use); the ⋯ menu that
//! removes or discards; what the user does reaching the host; the failure
//! banner and what the models take; the back title the host names; and the
//! app's language.

mod common;

use common::{emit, hear, next, post, quiet, start, start_observing, zh_hans};
use moshpit_ui::{
    screen::{Params, Screen},
    whisper_model::{Message, Model, Models, percent, subtitle},
};
use serde_json::json;

fn model(id: &str, name: &str) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        summary: format!("{name}'s summary."),
        approximate: "470 MB".into(),
        installed: None,
        partial: None,
        progress: None,
        selected: false,
    }
}

fn models() -> Models {
    Models {
        models: vec![
            Model {
                installed: Some("482 MB".into()),
                selected: true,
                ..model("base", "Base")
            },
            Model {
                installed: Some("1.1 GB".into()),
                ..model("turbo", "Large v3 Turbo")
            },
            Model {
                partial: Some("120 MB".into()),
                ..model("small", "Small")
            },
            model("tiny", "Tiny"),
        ],
        failure: None,
        storage: Some("1.6 GB".into()),
    }
}

fn show(app: &mut nsh_testing::Headless, channel: &moshpit_ui::channel::Channel, models: Models) {
    post(app, channel, &Message::Models(models).to_json());
}

#[test]
fn a_rows_subtitle_is_its_size_then_what_it_is_for() {
    let m = models();
    // (A paused download's line is the app's words, read on a screen:
    // each_model_says_its_size_and_what_can_be_done.)
    assert_eq!(subtitle(&m.models[0]), "482 MB · Base's summary.");
    assert_eq!(subtitle(&m.models[3]), "≈470 MB · Tiny's summary.");
    let bare = Model {
        summary: String::new(),
        ..model("x", "X")
    };
    assert_eq!(subtitle(&bare), "≈470 MB");
    assert_eq!(percent(0.0), "0%");
    assert_eq!(percent(0.426), "42%");
    assert_eq!(percent(1.2), "100%");
}

#[test]
fn each_model_says_its_size_and_what_can_be_done() {
    let (mut app, channel) = start(Screen::WhisperModel, Params::default(), "whisper-model");
    show(&mut app, &channel, models());
    app.wait_for("id:whisper-model-tiny").unwrap();
    for text in [
        "Model",
        "Settings",
        "Base",
        "482 MB · Base's summary.",
        "1.1 GB · Large v3 Turbo's summary.",
        "120 MB of ≈470 MB · Small's summary.",
        "≈470 MB · Tiny's summary.",
        "In use",
        "Resume",
        "Download",
        "Models on this device",
        "1.6 GB",
        "Downloads once from Hugging Face, then runs on this device. Bigger models handle mixed speech better, but slower.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(app.screen().selected("id:whisper-model-base").unwrap());
    assert!(!app.screen().selected("id:whisper-model-turbo").unwrap());
    for id in [
        "whisper-model-in-use-base",
        "whisper-model-more-base",
        "whisper-model-more-turbo",
        "whisper-model-more-small",
        "whisper-model-download-small",
        "whisper-model-download-tiny",
    ] {
        assert!(app.screen().matches(&format!("id:{id}")).unwrap(), "{id}");
    }
    for id in [
        "whisper-model-in-use-turbo",
        "whisper-model-more-tiny",
        "whisper-model-download-base",
        "whisper-model-error",
    ] {
        assert!(!app.screen().matches(&format!("id:{id}")).unwrap(), "{id}");
    }
    channel.close();
}

#[test]
fn the_menu_removes_a_model_or_discards_a_paused_download() {
    let (mut app, channel, natives) =
        start_observing(Screen::WhisperModel, Params::default(), "whisper-model");
    let told = hear(&channel);
    show(&mut app, &channel, models());
    app.wait_for("id:whisper-model-more-small").unwrap();
    let base = natives
        .find("moshpit/menu", "whisper-model-more-base")
        .unwrap();
    assert_eq!(
        natives.prop(base, "items"),
        Some(json!([{"id": "remove", "title": "Remove", "image": "trash", "destructive": true}]))
    );
    assert_eq!(natives.prop(base, "label"), Some(json!("More for Base")));
    emit(&mut app, base, "select", json!("remove"));
    assert_eq!(next(&told), json!({"ev": "remove", "id": "base"}));
    let small = natives
        .find("moshpit/menu", "whisper-model-more-small")
        .unwrap();
    assert_eq!(
        natives.prop(small, "items"),
        Some(
            json!([{"id": "discard", "title": "Discard download", "image": "trash", "destructive": true}])
        )
    );
    emit(&mut app, small, "select", json!("discard"));
    assert_eq!(next(&told), json!({"ev": "discard", "id": "small"}));
    channel.close();
}

#[test]
fn a_download_shows_its_progress_percent_and_pause() {
    let (mut app, channel, natives) =
        start_observing(Screen::WhisperModel, Params::default(), "whisper-model");
    let mut state = models();
    state.models[3].progress = Some(0.4);
    show(&mut app, &channel, state.clone());
    app.wait_for("id:whisper-model-pause-tiny").unwrap();
    assert!(app.screen().shows("40%"));
    assert!(
        !app.screen()
            .matches("id:whisper-model-download-tiny")
            .unwrap()
    );
    assert_eq!(natives.count("moshpit/progress"), 1);
    state.models[3].progress = Some(0.6);
    show(&mut app, &channel, state.clone());
    let bar = natives.only("moshpit/progress").unwrap();
    assert_eq!(natives.prop(bar, "value"), Some(json!(0.6)));
    assert_eq!(natives.prop(bar, "label"), Some(json!("Downloading Tiny")));
    assert!(app.screen().shows("60%"));
    // Done: the bar goes; the row offers its menu.
    state.models[3].progress = None;
    state.models[3].installed = Some("75 MB".into());
    show(&mut app, &channel, state);
    app.wait_for("id:whisper-model-more-tiny").unwrap();
    assert_eq!(natives.count("moshpit/progress"), 0);
    channel.close();
}

#[test]
fn what_the_user_does_reaches_the_host() {
    let (mut app, channel) = start(Screen::WhisperModel, Params::default(), "whisper-model");
    let told = hear(&channel);
    let mut state = models();
    state.models[3].progress = Some(0.4);
    show(&mut app, &channel, state);
    app.wait_for("id:whisper-model-pause-tiny").unwrap();
    for (selector, event) in [
        (
            "id:whisper-model-turbo",
            json!({"ev": "select", "id": "turbo"}),
        ),
        (
            "id:whisper-model-download-small",
            json!({"ev": "download", "id": "small"}),
        ),
        (
            "id:whisper-model-pause-tiny",
            json!({"ev": "pause", "id": "tiny"}),
        ),
        ("id:nav-bar-back", json!({"ev": "back"})),
    ] {
        app.press(selector).unwrap();
        assert_eq!(next(&told), event, "{selector}");
    }
    // The model in use, or one not downloaded: its row picks nothing.
    app.press("id:whisper-model-base").unwrap();
    app.press("id:whisper-model-small").unwrap();
    quiet(&told);
    channel.close();
}

#[test]
fn a_failure_shows_as_a_banner() {
    let (mut app, channel) = start(Screen::WhisperModel, Params::default(), "whisper-model");
    let mut state = models();
    state.failure = Some("There isn’t enough space on this iPhone.".into());
    state.storage = None;
    show(&mut app, &channel, state);
    app.wait_for("id:whisper-model-error").unwrap();
    assert!(
        app.screen()
            .shows("There isn’t enough space on this iPhone.")
    );
    assert!(!app.screen().shows("Models on this device"));
    channel.close();
}

#[test]
fn the_back_button_names_the_page_under_it() {
    let params = Params {
        back: Some("Recognition".into()),
        ..Params::default()
    };
    let (app, channel) = start(Screen::WhisperModel, params, "whisper-model");
    assert!(app.screen().matches("label:Recognition").unwrap());
    channel.close();
}

#[test]
fn the_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::WhisperModel, zh_hans(), "whisper-model");
    show(&mut app, &channel, models());
    app.wait_for("id:whisper-model-tiny").unwrap();
    for text in [
        "模型",
        "本机已下载",
        "继续",
        "下载",
        "使用中",
        "已下载 120 MB，共约 470 MB",
    ] {
        assert!(
            app.screen().texts().iter().any(|t| t.contains(text)),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

#[test]
fn the_more_menu_is_a_whole_touch_target() {
    let (mut app, channel) = start(Screen::WhisperModel, Params::default(), "whisper-model");
    show(&mut app, &channel, models());
    app.wait_for("id:whisper-model-more-small").unwrap();
    for id in ["whisper-model-more-base", "whisper-model-more-small"] {
        let target = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|node| node.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no view {id}"))
            .bounds;
        assert!(
            target.size.width.0 >= 44.0 && target.size.height.0 >= 44.0,
            "{id}: {target:?}"
        );
    }
    channel.close();
}

/// The selected row's tint covers the row and its ⋯ lane together, and
/// the card's corners clip it: the row and its lane are in one view that
/// clips, as wide as the card.
#[test]
fn a_models_row_and_its_menu_lane_are_one_clipped_row() {
    let (mut app, channel) = start(Screen::WhisperModel, Params::default(), "whisper-model");
    show(&mut app, &channel, models());
    app.wait_for("id:whisper-model-more-base").unwrap();
    let nodes = app.screen().layout_nodes();
    let by_id = |id: &str| {
        nodes
            .iter()
            .find(|n| n.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no view {id}"))
    };
    // A view and the views around it, innermost first.
    let ancestors = |id| {
        let mut chain = vec![id];
        while let Some(parent) = nodes
            .iter()
            .find(|n| Some(&n.id) == chain.last())
            .and_then(|n| nodes.iter().find(|p| p.id == n.parent))
        {
            chain.push(parent.id);
        }
        chain
    };
    let (row, lane) = (
        ancestors(by_id("whisper-model-base").id),
        ancestors(by_id("whisper-model-more-base").id),
    );
    let shared = row.iter().find(|id| lane.contains(id)).unwrap();
    let shared = nodes.iter().find(|n| n.id == *shared).unwrap();
    assert!(
        shared.clips,
        "the row and its lane clip together: {shared:?}"
    );
    // The card: the nearest view around it that clips.
    let card = ancestors(shared.id)
        .into_iter()
        .skip(1)
        .filter_map(|id| nodes.iter().find(|n| n.id == id))
        .find(|n| n.clips)
        .map(|n| n.bounds.size.width.0);
    assert_eq!(
        Some(shared.bounds.size.width.0),
        card,
        "as wide as the card it is on"
    );
    channel.close();
}
