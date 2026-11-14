//! The app's look for the Swift host: the terminal cursor's color, resolved
//! the way the `ui` layer's swatches resolve it (`ui/src/ui/swatch.rs`), so
//! the cursor the terminal draws is the swatch Settings shows picked (B318).

use moshpit_ui::{
    theme::AppTheme,
    ui::swatch::{CursorColor, palette_of},
};

/// The terminal cursor's color for the stored id `id` (`cursorColorId`)
/// under the app theme `theme`, as `0xRRGGBBAA`, in the dark appearance or,
/// with `light`, the light one.
///
/// `theme` is the screens' `theme` param, JSON (`accent`, `accentPressed`,
/// `screenBG`, `terminalBG`); one that does not parse is Signal Room. Each
/// id is its own color (Green the success green, not the accent); a choice
/// hidden because it looks like the accent draws as Accent; an id that
/// names no color draws as the default (`teal`, Blue).
#[uniffi::export]
pub fn terminal_cursor_color(id: String, theme: String, light: bool) -> u32 {
    let app: AppTheme = serde_json::from_str(&theme).unwrap_or_default();
    let palette = palette_of(&app);
    let (on_light, on_dark) = CursorColor::drawn(&id, &palette).value(&palette);
    let c = if light { on_light } else { on_dark };
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(c.r) << 24 | byte(c.g) << 16 | byte(c.b) << 8 | byte(c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNAL_ROOM: &str = r##"{"accent":"#6C6BEF","accentPressed":"#5652D6","screenBG":"#06070A","terminalBG":"#030405"}"##;
    const TERMINAL_GREEN: &str = r##"{"accent":"#2FA871","accentPressed":"#25865A","screenBG":"#060B08","terminalBG":"#030503"}"##;

    fn drawn(id: &str, theme: &str) -> u32 {
        terminal_cursor_color(id.into(), theme.into(), false)
    }

    /// B318: under Signal Room the stored `green` draws the success green
    /// (`4CC38A` dark, `1F9D55` light), not the accent; `accent` draws the
    /// accent; White is white at 86%.
    #[test]
    fn green_is_green_and_not_the_accent() {
        assert_eq!(drawn("green", SIGNAL_ROOM), 0x4CC3_8AFF);
        assert_eq!(
            terminal_cursor_color("green".into(), SIGNAL_ROOM.into(), true),
            0x1F9D_55FF
        );
        assert_eq!(drawn("accent", SIGNAL_ROOM), 0x6C6B_EFFF);
        assert_ne!(drawn("green", SIGNAL_ROOM), drawn("accent", SIGNAL_ROOM));
        assert_eq!(
            drawn("teal", SIGNAL_ROOM),
            0x7AA7_FFFF,
            "Blue, the info blue"
        );
        assert_eq!(drawn("white", SIGNAL_ROOM), 0xFFFF_FFDB, "white at 86%");
    }

    /// A stored choice the swatches hide (Green under Terminal Green, whose
    /// accent is a green) draws as Accent; the others as themselves.
    #[test]
    fn a_hidden_choice_draws_as_the_accent() {
        assert_eq!(drawn("green", TERMINAL_GREEN), 0x2FA8_71FF);
        assert_eq!(
            drawn("green", TERMINAL_GREEN),
            drawn("accent", TERMINAL_GREEN)
        );
        assert_eq!(drawn("teal", TERMINAL_GREEN), 0x7AA7_FFFF);
    }

    #[test]
    fn an_unknown_id_or_theme_falls_back() {
        assert_eq!(drawn("magenta", SIGNAL_ROOM), drawn("teal", SIGNAL_ROOM));
        assert_eq!(drawn("accent", "not json"), drawn("accent", SIGNAL_ROOM));
    }
}
