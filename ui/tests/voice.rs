//! Settings › Voice Input › Recognition and Language on a headless engine
//! (R15, §6.11): the engines and languages the host posts, the picked one
//! checked (and selected for screen readers), picks reaching the host,
//! Whisper's banner while it has no model, Apple's languages while they are
//! read, the search, and the app's language.

mod common;

use common::{hear, next, post, quiet, start, zh_hans};
use moshpit_ui::{
    screen::{Params, Screen},
    voice_engine::{self, Engine, EngineState},
    voice_language::{self, Language, Languages},
};
use serde_json::json;

fn engine(engine: Engine, needs_model: bool) -> String {
    voice_engine::Message::Engine(EngineState {
        engine,
        needs_model,
    })
    .to_json()
}

fn language(id: &str, name: &str) -> Language {
    Language {
        id: id.into(),
        name: name.into(),
    }
}

fn apple(options: Option<Vec<Language>>, selected: &str) -> String {
    voice_language::Message::Languages(Languages {
        engine: Engine::Apple,
        automatic: Some("English (United States)".into()),
        suggested: vec![],
        options,
        selected: selected.into(),
    })
    .to_json()
}

fn whisper(selected: &str) -> String {
    voice_language::Message::Languages(Languages {
        engine: Engine::Whisper,
        automatic: None,
        suggested: vec![language("zh", "Chinese"), language("en", "English")],
        options: Some(vec![
            language("af", "Afrikaans"),
            language("zh", "Chinese"),
            language("en", "English"),
        ]),
        selected: selected.into(),
    })
    .to_json()
}

#[test]
fn the_engines_show_the_one_in_use_selected() {
    let (mut app, channel) = start(Screen::VoiceEngine, Params::default(), "voice-engine");
    post(&mut app, &channel, &engine(Engine::Apple, false));
    app.wait_for("id:voice-engine-apple").unwrap();
    for text in [
        "Recognition",
        "Apple",
        "Built in and quick; one language at a time",
        "Local model",
        "A model on this device; follows speech that mixes languages",
        "Both run on this device. Your voice is never uploaded.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(app.screen().selected("id:voice-engine-apple").unwrap());
    assert!(!app.screen().selected("id:voice-engine-whisper").unwrap());
    assert!(!app.screen().matches("id:voice-engine-models").unwrap());
    channel.close();
}

#[test]
fn picking_an_engine_asks_the_host_once() {
    let (mut app, channel) = start(Screen::VoiceEngine, Params::default(), "voice-engine");
    let told = hear(&channel);
    post(&mut app, &channel, &engine(Engine::Apple, false));
    app.wait_for("id:voice-engine-whisper").unwrap();
    // The engine in use: nothing to do, as on the SwiftUI page.
    app.press("id:voice-engine-apple").unwrap();
    quiet(&told);
    app.press("id:voice-engine-whisper").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "engine": "whisper"}));
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    channel.close();
}

#[test]
fn whisper_without_a_model_offers_the_download() {
    let (mut app, channel) = start(Screen::VoiceEngine, Params::default(), "voice-engine");
    let told = hear(&channel);
    post(&mut app, &channel, &engine(Engine::Whisper, true));
    app.wait_for("id:voice-engine-models").unwrap();
    assert!(app.screen().matches("id:voice-engine-setup").unwrap());
    assert!(app.screen().shows("Download a model first"));
    assert!(app.screen().shows("Download"));
    assert!(app.screen().selected("id:voice-engine-whisper").unwrap());
    app.press("id:voice-engine-models").unwrap();
    assert_eq!(next(&told), json!({"ev": "models"}));
    // A model arrived: the group goes.
    post(&mut app, &channel, &engine(Engine::Whisper, false));
    app.wait_for_gone("id:voice-engine-setup", common::WAIT)
        .unwrap();
    channel.close();
}

#[test]
fn the_engine_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::VoiceEngine, zh_hans(), "voice-engine");
    post(&mut app, &channel, &engine(Engine::Whisper, true));
    app.wait_for("id:voice-engine-models").unwrap();
    for text in [
        "识别引擎",
        "系统自带，快；一次听一种语言",
        "本地模型，中英混说也能听",
        "先下载一个模型",
        "下载",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

#[test]
fn apples_languages_show_once_read_with_automatic_spelled_out() {
    let (mut app, channel) = start(Screen::VoiceLanguage, Params::default(), "voice-language");
    post(&mut app, &channel, &apple(None, ""));
    app.wait_for("id:voice-language-loading").unwrap();
    // Nothing to search until the list is read.
    assert!(!app.screen().matches("id:voice-language-search").unwrap());
    for text in [
        "Language",
        "Automatic",
        "English (United States)",
        "Only languages this device can transcribe. Each downloads once, then works offline.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(
        app.screen()
            .selected("id:voice-language-automatic")
            .unwrap()
    );
    post(
        &mut app,
        &channel,
        &apple(
            Some(vec![
                language("zh-CN", "Chinese (China mainland)"),
                language("en-US", "English (United States)"),
            ]),
            "en-US",
        ),
    );
    app.wait_for("id:voice-language-en-US").unwrap();
    assert!(!app.screen().matches("id:voice-language-loading").unwrap());
    assert!(app.screen().selected("id:voice-language-en-US").unwrap());
    assert!(
        !app.screen()
            .selected("id:voice-language-automatic")
            .unwrap()
    );
    assert!(app.screen().matches("id:voice-language-search").unwrap());
    assert!(!app.screen().shows("ALL LANGUAGES"));
    channel.close();
}

#[test]
fn whispers_languages_put_the_users_own_first() {
    let (mut app, channel) = start(Screen::VoiceLanguage, Params::default(), "voice-language");
    post(&mut app, &channel, &whisper("zh"));
    app.wait_for("id:voice-language-af").unwrap();
    let texts = app.screen().texts();
    let at = |t: &str| texts.iter().position(|s| *s == t).unwrap();
    assert!(at("Automatic") < at("Whisper decides from what it hears"));
    // Section titles are eyebrows: capitals in English.
    assert!(at("Whisper decides from what it hears") < at("COMMON"));
    assert!(at("COMMON") < at("ALL LANGUAGES"));
    assert!(at("ALL LANGUAGES") < at("Afrikaans"));
    // A suggested language is listed again among all of them.
    assert_eq!(texts.iter().filter(|t| **t == "Chinese").count(), 2);
    assert_eq!(app.screen().count("id:voice-language-zh").unwrap(), 2);
    channel.close();
}

#[test]
fn picking_a_language_asks_the_host_every_time() {
    let (mut app, channel) = start(Screen::VoiceLanguage, Params::default(), "voice-language");
    let told = hear(&channel);
    post(&mut app, &channel, &whisper(""));
    app.wait_for("id:voice-language-af").unwrap();
    app.press("id:voice-language-af").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": "af"}));
    // The picked row too, as the SwiftUI page sets it again.
    app.press("id:voice-language-automatic").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": ""}));
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    channel.close();
}

#[test]
fn the_language_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::VoiceLanguage, zh_hans(), "voice-language");
    post(&mut app, &channel, &whisper("zh"));
    app.wait_for("id:voice-language-af").unwrap();
    for text in [
        "语言",
        "常用",
        "全部语言",
        "自动",
        "让 Whisper 自己听出是哪种语言",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

#[test]
fn rows_follow_dynamic_type() {
    let (mut app, channel) = start(Screen::VoiceLanguage, Params::default(), "voice-language");
    post(&mut app, &channel, &apple(None, ""));
    app.wait_for("id:voice-language-loading").unwrap();
    let size =
        |app: &nsh_testing::Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    // A row's title is the body style, its subtitle the subheadline.
    assert_eq!(size(&app, "text:Automatic"), 17.0);
    assert_eq!(size(&app, "text:English (United States)"), 15.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Automatic"), 23.0);
    assert_eq!(size(&app, "text:English (United States)"), 21.0);
    channel.close();
}

#[test]
fn the_search_finds_languages_by_name_or_id() {
    let (mut app, channel) = start(Screen::VoiceLanguage, Params::default(), "voice-language");
    let told = hear(&channel);
    post(&mut app, &channel, &whisper("zh"));
    app.wait_for("id:voice-language-search").unwrap();
    app.type_text("id:voice-language-search", "afr").unwrap();
    app.wait_for_gone("id:voice-language-automatic", common::WAIT)
        .unwrap();
    assert_eq!(app.screen().count("id:voice-language-af").unwrap(), 1);
    assert!(!app.screen().matches("id:voice-language-en").unwrap());
    assert!(!app.screen().shows("COMMON"));
    app.press("id:voice-language-af").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": "af"}));
    // By id, case aside.
    app.clear("id:voice-language-search").unwrap();
    app.type_text("id:voice-language-search", "EN").unwrap();
    app.wait_for("id:voice-language-en").unwrap();
    assert!(!app.screen().matches("id:voice-language-af").unwrap());
    // Nothing: it says so.
    app.clear("id:voice-language-search").unwrap();
    app.type_text("id:voice-language-search", "klingon")
        .unwrap();
    app.wait_for("id:voice-language-none").unwrap();
    assert!(app.screen().shows("No language matches"));
    // Cleared: the groups are back.
    app.clear("id:voice-language-search").unwrap();
    app.wait_for("id:voice-language-automatic").unwrap();
    assert!(app.screen().shows("COMMON"));
    channel.close();
}

#[test]
fn a_language_matches_its_name_or_its_id() {
    let l = language("pt-BR", "Portuguese (Brazil)");
    for query in ["", "  ", "port", "BRAZIL", "pt-br", "PT"] {
        assert!(voice_language::matches(&l, query), "{query}");
    }
    for query in ["spanish", "fr"] {
        assert!(!voice_language::matches(&l, query), "{query}");
    }
}
