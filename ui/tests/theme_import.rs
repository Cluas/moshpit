//! Import Theme on a headless engine: Import waits for text, Paste from
//! Clipboard replaces the text with the clipboard's, what Import and Cancel
//! tell the host, the host's error, and Chinese.

mod common;

use common::{hear, next, post, start, steady, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use serde_json::json;

#[test]
fn what_is_typed_or_pasted_is_imported() {
    let (mut app, channel) = start(Screen::ThemeImport, Params::default(), "theme-import");
    let heard = hear(&channel);
    post(&mut app, &channel, r#"{"op":"themeImport"}"#);
    app.wait_for("id:theme-import-editor").unwrap();
    for text in [
        "Import Theme",
        "Cancel",
        "Import",
        "JSON",
        "Paste from Clipboard",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert_eq!(app.screen().enabled("id:theme-import-submit"), Ok(false));
    app.type_text("id:theme-import-editor-input", "  ").unwrap();
    assert_eq!(
        app.screen().enabled("id:theme-import-submit"),
        Ok(false),
        "blank is nothing"
    );

    // A paste replaces the text.
    let theme = r##"{"name":"Ember","background":"#101010"}"##;
    app.set_clipboard(Some(theme.to_owned()));
    app.press("id:theme-import-paste").unwrap();
    steady(&mut app);
    app.wait_for(&format!("text:{theme}")).unwrap();
    assert_eq!(app.screen().enabled("id:theme-import-submit"), Ok(true));
    app.press("id:theme-import-submit").unwrap();
    assert_eq!(next(&heard), json!({"ev": "import", "text": theme}));

    // An empty clipboard leaves the text as it is.
    app.set_clipboard(None);
    app.press("id:theme-import-paste").unwrap();
    steady(&mut app);
    app.press("id:theme-import-submit").unwrap();
    assert_eq!(next(&heard), json!({"ev": "import", "text": theme}));

    // The host couldn't: it says why.
    post(
        &mut app,
        &channel,
        r#"{"op":"themeImport","error":"That JSON has no theme in it."}"#,
    );
    app.wait_for("id:theme-import-error").unwrap();
    assert!(app.screen().shows("That JSON has no theme in it."));
    post(&mut app, &channel, r#"{"op":"themeImport"}"#);
    assert!(!app.screen().matches("id:theme-import-error").unwrap());

    app.press("id:theme-import-cancel").unwrap();
    assert_eq!(next(&heard), json!({"ev": "cancel"}));
}

#[test]
fn in_chinese() {
    let (mut app, channel) = start(Screen::ThemeImport, zh_hans(), "theme-import");
    post(&mut app, &channel, r#"{"op":"themeImport"}"#);
    app.wait_for("id:theme-import-editor").unwrap();
    for text in ["导入主题", "取消", "导入", "从剪贴板粘贴"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
}

#[test]
fn it_follows_the_system() {
    assert!(Screen::ThemeImport.follows_system());
    assert!(Screen::ThemeImport.draws_light());
}
