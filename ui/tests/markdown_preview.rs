//! A Markdown file on a headless engine: laid out, not shown as its source;
//! what Done, Share and a link tell the host; Chinese.

mod common;

use common::{hear, next, post, start, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use serde_json::json;

const README: &str = r##"{"op":"markdown","name":"README.md","text":"# Moshpit\n\nA **terminal** for the phone.\n\n- [x] mosh\n- [ ] more\n\n| Key | Does |\n| --- | --- |\n| Esc | leaves |\n\n```sh\ncargo test\n```\n\nSee [the site](https://example.com/docs)."}"##;

#[test]
fn the_document_is_laid_out_not_shown_as_source() {
    let (mut app, channel) = start(
        Screen::MarkdownPreview,
        Params::default(),
        "markdown-preview",
    );
    let heard = hear(&channel);
    post(&mut app, &channel, README);
    app.wait_for("id:markdown-preview-document").unwrap();
    let screen = app.screen();
    for text in ["README.md", "Moshpit", "Esc", "cargo test"] {
        assert!(screen.shows(text), "{text}: {:?}", screen.texts());
    }
    // The syntax is gone: a heading, not "# Moshpit"; bold, not "**".
    for source in ["# Moshpit", "**terminal**", "| --- |", "```", "[x]"] {
        assert!(
            !screen.texts().iter().any(|t| t.contains(source)),
            "{source} shown as written: {:?}",
            screen.texts()
        );
    }

    app.press("text:the site").unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "link", "url": "https://example.com/docs"})
    );
    app.press("id:markdown-preview-share").unwrap();
    assert_eq!(next(&heard), json!({"ev": "share"}));
    app.press("id:markdown-preview-close").unwrap();
    assert_eq!(next(&heard), json!({"ev": "close"}));
}

#[test]
fn in_chinese() {
    let (mut app, channel) = start(Screen::MarkdownPreview, zh_hans(), "markdown-preview");
    post(&mut app, &channel, README);
    app.wait_for("id:markdown-preview-document").unwrap();
    assert!(app.screen().shows("README.md"));
}

#[test]
fn it_follows_the_system() {
    assert!(Screen::MarkdownPreview.follows_system());
    assert!(Screen::MarkdownPreview.draws_light());
}
