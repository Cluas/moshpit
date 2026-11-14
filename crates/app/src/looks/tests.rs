//! The editors' rules, ported from `ThemeNSHTests`' editor parts.

use super::*;
use serde_json::json;

/// A custom theme with a bright override (Swift's `harbor()`).
fn harbor() -> TerminalTheme {
    TerminalTheme::new(
        "custom-1a2b3c4d",
        "Harbor Night",
        "0B1420",
        "D6E2F0",
        "F2C14E",
        &[
            "1B2633", "F07178", "9BD67E", "F2C14E", "6CB6FF", "C39BFF", "5FD7D7", "E6EDF3",
        ],
        Some(&["7A7468", "", "", "", "", "", "", ""]),
        false,
    )
}

#[test]
fn every_editor_state_is_a_message_the_screen_takes() {
    let message = ThemeEditor::new(harbor()).message();
    let theme_screen::Message::Theme(state) = theme_screen::Message::parse(&message).unwrap();
    assert!(state.override_bright);
    assert!(state.theme.bright[0].is_some());
    assert!(
        state.theme.bright[1..].iter().all(Option::is_none),
        "derived slots are none"
    );
    let value: Value = serde_json::from_str(&message).unwrap();
    assert_eq!(value["theme"]["bright"][1], Value::Null);
}

#[test]
fn the_pages_changes_apply_to_the_draft() {
    assert!(
        ThemeEditor::new(harbor()).override_bright,
        "an override opens the bright rows"
    );
    let mut plain = harbor();
    plain.set_bright_override(None, AnsiSlot::Black);
    let mut editor = ThemeEditor::new(plain);
    assert!(!editor.override_bright);
    assert_eq!(
        editor.handle(&json!({"ev": "name", "value": "Dusk"})),
        Handled::Changed
    );
    assert_eq!(editor.draft.name, "Dusk");
    editor.handle(&json!({"ev": "color", "slot": "background", "value": "#112233"}));
    assert_eq!(editor.draft.background_hex, "112233");
    editor.handle(&json!({"ev": "color", "slot": "cursor", "value": "#abcdef"}));
    assert_eq!(editor.draft.cursor_hex, "ABCDEF");
    editor.handle(&json!({"ev": "color", "slot": "red", "value": "#FF0000"}));
    assert_eq!(editor.draft.ansi(AnsiSlot::Red), "FF0000");
    editor.handle(&json!({"ev": "override-bright", "value": true}));
    editor.handle(&json!({"ev": "color", "slot": "brightBlack", "value": "#7A7468"}));
    assert!(editor.draft.has_bright_override(AnsiSlot::Black));
    assert_eq!(editor.draft.bright(AnsiSlot::Black), "7A7468");
    editor.handle(&json!({"ev": "override-bright", "value": false}));
    assert!(!editor.override_bright);
    assert!(
        !editor.draft.has_bright_override(AnsiSlot::Black),
        "off drops the overrides"
    );
    assert_eq!(editor.draft.bright_hex, None);

    let before = editor.clone();
    for unknown in [
        json!({"ev": "color", "slot": "brightness", "value": "#000000"}),
        json!({"ev": "color", "slot": "red"}),
        json!({"ev": "name"}),
        json!({"ev": "override-bright", "value": "yes"}),
        json!({"ev": "jump"}),
        json!({}),
    ] {
        assert_eq!(editor.handle(&unknown), Handled::Unknown, "{unknown}");
    }
    assert_eq!(
        editor, before,
        "what it does not read changes nothing (Android cleared the name)"
    );

    assert_eq!(
        editor.handle(&json!({"ev": "save"})),
        Handled::Intent(EditorIntent::Save)
    );
    assert_eq!(
        editor.handle(&json!({"ev": "cancel"})),
        Handled::Intent(EditorIntent::Cancel)
    );
    assert_eq!(
        editor.handle(&json!({"ev": "back"})),
        Handled::Intent(EditorIntent::Cancel)
    );
    assert_eq!(
        editor.handle(&json!({"ev": "copy-json"})),
        Handled::Intent(EditorIntent::CopyJson)
    );
}

#[test]
fn the_accent_editors_state_is_a_message_and_its_changes_apply() {
    let mut editor = AccentEditor::new(&AppTheme::custom("app-1a2b3c4d", "Mint", "2FD8A8"));
    assert!(accent_screen::Message::parse(&editor.message()).is_ok());
    assert_eq!(
        editor.handle(&json!({"ev": "color", "value": "#ff8800"})),
        Handled::Changed
    );
    assert_eq!(
        editor.handle(&json!({"ev": "name", "value": "Ember"})),
        Handled::Changed
    );
    let before = editor.clone();
    assert_eq!(editor.handle(&json!({"ev": "color"})), Handled::Unknown);
    assert_eq!(editor.handle(&json!({"ev": "name"})), Handled::Unknown);
    assert_eq!(editor, before);
    assert_eq!(
        editor.accent(),
        AppTheme::custom("app-1a2b3c4d", "Ember", "FF8800")
    );
    editor.name = String::new();
    assert!(
        accent_screen::Message::parse(&editor.message()).is_ok(),
        "an empty name"
    );
    assert_eq!(
        editor.handle(&json!({"ev": "save"})),
        Handled::Intent(EditorIntent::Save)
    );
    assert_eq!(
        editor.handle(&json!({"ev": "back"})),
        Handled::Intent(EditorIntent::Cancel)
    );
}
