//! What Settings shows in its tests: one state with every row's value set
//! (the Settings screen's tests and its Appearance choice's read it).

use moshpit_ui::{
    app_icon::Icon,
    settings::{Choice, Count, CursorShape, PredictMode, SettingsState, Swatch, TerminalTheme},
    voice_engine::Engine,
};

pub fn state() -> SettingsState {
    SettingsState {
        accent: Swatch {
            name: "Signal Room".into(),
            color: nsh_core::color::rgb(0x6C6BEF),
        },
        app_icon: Icon {
            id: "default".into(),
            name: "Default".into(),
            image: None,
        },
        fonts: vec![
            Choice {
                id: "system".into(),
                name: "SF Mono".into(),
            },
            Choice {
                id: "Menlo".into(),
                name: "Menlo".into(),
            },
        ],
        font: "Menlo".into(),
        font_size: 11.0,
        theme: TerminalTheme {
            name: "GitHub Dark".into(),
            background: nsh_core::color::rgb(0x0D1117),
            foreground: nsh_core::color::rgb(0xC9D1D9),
            swatches: (0..8).map(|i| nsh_core::color::rgb(0x202020 * i)).collect(),
        },
        cursor_shape: CursorShape::Bar,
        cursor_color: "white".into(),
        cursor_blink: false,
        trail_on_predict: true,
        keyboard_on_open: false,
        remote_clipboard_read: false,
        upload_retention_days: 7,
        mosh_by_default: true,
        predict_mode: PredictMode::Never,
        udp_range: [60000, 61000],
        shortcuts: Count {
            count: 9,
            limit: 12,
        },
        keys: 1,
        notifications: true,
        live_activity: false,
        alert_sound: true,
        lock_screen_detail: true,
        live_session: false,
        voice_input: true,
        voice_engine: Engine::Whisper,
        voice_model: None,
        voice_language: "Auto-detect".into(),
        version: "Moshpit 1.0.9 (1) · dev".into(),
        filing: None,
        app_theme: moshpit_ui::theme::AppTheme::SIGNAL_ROOM,
        appearance: moshpit_ui::screen::AppearanceChoice::System,
    }
}
