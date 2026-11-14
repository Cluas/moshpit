//! The app's NSH theme: the semantic tokens ([`crate::ui::tokens`],
//! UI-DESIGN §2: shadcn's keys and the roles added beside them, light and
//! dark) made from the theme the user picks in Settings › Appearance
//! ([`AppTheme`]: the accent, its pressed shade, the screen's and the
//! terminal's backgrounds), which the host passes when it runs a screen,
//! and the navigation bar's scroll edge ([`bar_edge`]). ([`crate::face`] is
//! the typography.)
//!
//! R17 retired Ink and Metrics, the old dark-only colors and lengths
//! (`Moshpit/UI/DesignTokens.swift`): every screen draws from the tokens.
//! A test reads AppTheme.swift and fails when the default theme here
//! drifts from the Swift one.

use nsh_core::{
    color::Rgba,
    element::{AdaptiveColor, Tokens},
};
use serde::{Deserialize, Deserializer};

/// `0xRRGGBB` at `opacity`, as `Color(hex:).opacity(_:)` makes it.
const fn hex(rgb: u32, opacity: f32) -> Rgba {
    Rgba {
        a: opacity,
        ..nsh_core::color::rgb(rgb)
    }
}

/// How a navigation bar sets itself off from content scrolled under it
/// (`nav_bar::ScrollEdge`): every value of the look, here and in the theme's
/// tokens ([`tokens`] writes them, [`BarEdge::current`] and the colors below
/// read them). The bar holds the mechanism and none of these: a redesign
/// restyles the bar here.
///
/// On iOS 26 and later the bar is glass and has no background; iOS 26's
/// scroll edge effect (UIKit's soft style) blurs what is under it and lays
/// the page's color over it, whole under the bar and fading out past it.
/// Below iOS 26 the bar wears the system's chrome material and a hairline.
/// Either comes in over the first points of scrolling, following the list.
pub mod bar_edge {
    use nsh_core::prelude::BackdropMaterial;

    /// How much of the page's color the scroll edge effect lays over what
    /// it blurs (token `bar-edge`: the page's color at this).
    pub const TINT: f32 = 0.8;
    /// Its blur's radius, in points (`bar-edge-blur`).
    pub const BLUR: f32 = 20.0;
    /// How far past the bar's bottom it reaches, fading out
    /// (`bar-edge-reach`).
    pub const REACH: f32 = 20.0;
    /// How far above the bar's bottom it starts to fade (`bar-edge-fade`).
    pub const FADE: f32 = 12.0;
    /// Over how many points of scrolling either look comes in
    /// (`bar-edge-appear`): none at the list's top, whole this far down.
    pub const APPEAR: f32 = 8.0;
    /// The classic bar's material and its blur (UIKit's bar below iOS 26);
    /// its hairline is token `bar-hairline`.
    pub const MATERIAL: BackdropMaterial = BackdropMaterial::Chrome;
    pub const MATERIAL_BLUR: f32 = 20.0;

    /// A trace of colour under the classic bar's material, black in the
    /// dark appearance and white in the light one: NSH picks a material's
    /// variant by the view's background, not the appearance, and with
    /// nothing under it the dark app's bar turned light grey (and a light
    /// one would turn dark).
    pub fn material_trace() -> nsh_core::element::AdaptiveColor {
        let trace = |level: f32| super::Rgba {
            r: level,
            g: level,
            b: level,
            a: 0.01,
        };
        nsh_core::element::AdaptiveColor::new(trace(1.0), trace(0.0))
    }
}

/// [`bar_edge`]'s numbers as the surface's theme has them (its values
/// where the theme has none).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarEdge {
    /// [`bar_edge::BLUR`].
    pub blur: f32,
    /// [`bar_edge::REACH`].
    pub reach: f32,
    /// [`bar_edge::FADE`].
    pub fade: f32,
    /// [`bar_edge::APPEAR`].
    pub appear: f32,
}

impl BarEdge {
    /// The surface's.
    pub fn current() -> BarEdge {
        let tokens = nsh_core::prelude::tokens();
        let number = |name: &str, default: f32| tokens.number(name).unwrap_or(default);
        BarEdge {
            blur: number("bar-edge-blur", bar_edge::BLUR),
            reach: number("bar-edge-reach", bar_edge::REACH),
            fade: number("bar-edge-fade", bar_edge::FADE),
            appear: number("bar-edge-appear", bar_edge::APPEAR),
        }
    }
}

/// The page's color the scroll edge effect lays over what it blurs (token
/// `bar-edge`).
pub fn bar_edge_tint() -> AdaptiveColor {
    nsh_core::prelude::tokens()
        .color("bar-edge")
        .unwrap_or_else(|| bar_colors(&AppTheme::SIGNAL_ROOM).0)
}

/// The classic bar's hairline (token `bar-hairline`).
pub fn bar_hairline() -> AdaptiveColor {
    nsh_core::prelude::tokens()
        .color("bar-hairline")
        .unwrap_or_else(|| bar_colors(&AppTheme::SIGNAL_ROOM).1)
}

/// The bar's two colors from the semantic roles (UI-DESIGN §4.18):
/// `bar-edge` is `background` at [`bar_edge::TINT`], `bar-hairline` is
/// `border`, each in the light and the dark appearance.
fn bar_colors(app: &AppTheme) -> (AdaptiveColor, AdaptiveColor) {
    let colors = crate::ui::tokens::colors(app);
    let role = |name: &str| {
        colors
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, light, dark)| (*light, *dark))
            .unwrap_or_else(|| panic!("no color token {name}"))
    };
    let (light, dark) = role("background");
    let (line_light, line_dark) = role("border");
    (
        AdaptiveColor::new(edge_tint(light), edge_tint(dark)),
        AdaptiveColor::new(line_light, line_dark),
    )
}

/// `screen` at [`bar_edge::TINT`].
fn edge_tint(screen: Rgba) -> Rgba {
    Rgba {
        a: screen.a * bar_edge::TINT,
        ..screen
    }
}

/// The colors of the theme the user picked (`AppTheme`). JSON from the
/// host: `{"accent": "#6C6BEF", "accentPressed": "#5652D6", "screenBG":
/// "#06070A", "terminalBG": "#030405"}`; a color it leaves out is Signal
/// Room's.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppTheme {
    #[serde(deserialize_with = "color", serialize_with = "write_color")]
    pub accent: Rgba,
    #[serde(
        rename = "accentPressed",
        deserialize_with = "color",
        serialize_with = "write_color"
    )]
    pub accent_pressed: Rgba,
    #[serde(
        rename = "screenBG",
        deserialize_with = "color",
        serialize_with = "write_color"
    )]
    pub screen_bg: Rgba,
    #[serde(
        rename = "terminalBG",
        deserialize_with = "color",
        serialize_with = "write_color"
    )]
    pub terminal_bg: Rgba,
}

impl AppTheme {
    /// Signal Room, the default theme (`AppThemeCatalog.signalRoom`).
    pub const SIGNAL_ROOM: AppTheme = AppTheme {
        accent: hex(0x6C6BEF, 1.0),
        accent_pressed: hex(0x5652D6, 1.0),
        screen_bg: hex(0x06070A, 1.0),
        terminal_bg: hex(0x030405, 1.0),
    };
}

impl Default for AppTheme {
    fn default() -> Self {
        Self::SIGNAL_ROOM
    }
}

/// `#RRGGBB` or `#RRGGBBAA`.
pub fn parse_hex(text: &str) -> Option<Rgba> {
    let digits = text.strip_prefix('#')?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(nsh_core::color::rgb(value)),
        8 => Some(nsh_core::color::rgba(value)),
        _ => None,
    }
}

/// Reads a color written `#RRGGBB` or `#RRGGBBAA` (serde's
/// `deserialize_with`).
pub(crate) fn color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Rgba, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_hex(&text)
        .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not #RRGGBB or #RRGGBBAA")))
}

/// Writes a color as `#RRGGBBAA` (serde's `serialize_with`).
pub fn write_color<S: serde::Serializer>(color: &Rgba, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex_string(*color))
}

/// The NSH theme of the app's tokens with `app`'s colors (the module doc).
pub fn tokens(app: &AppTheme) -> Tokens {
    let mut theme = nsh_base::Theme::default();
    // The navigation bar's scroll edge numbers (`bar_edge`); its two
    // colors come from the semantic roles, below.
    for (name, value) in [
        ("bar-edge-blur", bar_edge::BLUR),
        ("bar-edge-reach", bar_edge::REACH),
        ("bar-edge-fade", bar_edge::FADE),
        ("bar-edge-appear", bar_edge::APPEAR),
    ] {
        theme.set_number(name, value);
    }
    // The semantic tokens: shadcn's keys (`background`, `primary`,
    // `radius`, …) and the roles beside them.
    crate::ui::tokens::install(&mut theme, app);
    let (edge, hairline) = bar_colors(app);
    theme.set_color("bar-edge", edge);
    theme.set_color("bar-hairline", hairline);
    theme.into_tokens()
}

/// `color` as `#RRGGBBAA`, for a native component's props.
pub fn hex_string(color: Rgba) -> String {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        byte(color.r),
        byte(color.g),
        byte(color.b),
        byte(color.a)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file of the app's, read from the checkout.
    fn app_file(path: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn the_default_theme_is_signal_room() {
        let source = app_file("Moshpit/UI/AppTheme.swift");
        let block = source
            .split("static let signalRoom = AppTheme(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .expect("AppTheme.swift defines signalRoom");
        let hex_of = |key: &str| {
            let after = block.split(&format!("{key}: \"")).nth(1).unwrap();
            parse_hex(&format!("#{}", &after[..6])).unwrap()
        };
        let theme = AppTheme::SIGNAL_ROOM;
        assert_eq!(theme.accent, hex_of("accentHex"));
        assert_eq!(theme.accent_pressed, hex_of("accentPressedHex"));
        assert_eq!(theme.screen_bg, hex_of("screenBGHex"));
        assert_eq!(theme.terminal_bg, hex_of("terminalBGHex"));
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#6C6BEF"), Some(nsh_core::color::rgb(0x6C6BEF)));
        assert_eq!(
            parse_hex("#6C6BEF80"),
            Some(nsh_core::color::rgba(0x6C6BEF80))
        );
        assert_eq!(parse_hex("6C6BEF"), None);
        assert_eq!(parse_hex("#6C6BE"), None);
        assert_eq!(parse_hex("#+6C6BE"), None);
        assert_eq!(hex_string(hex(0xFFB35C, 1.0)), "#FFB35CFF");
        assert_eq!(hex_string(hex(0xC7C1B5, 0.44)), "#C7C1B570");
    }

    #[test]
    fn a_theme_from_the_host() {
        let theme: AppTheme = serde_json::from_str(
            r##"{"accent":"#53DCC9","accentPressed":"#3EBFA9","screenBG":"#05090A","terminalBG":"#020505"}"##,
        )
        .unwrap();
        assert_eq!(theme.accent, nsh_core::color::rgb(0x53DCC9));
        assert_eq!(theme.screen_bg, nsh_core::color::rgb(0x05090A));
        let partial: AppTheme = serde_json::from_str(r##"{"accent":"#53DCC9"}"##).unwrap();
        assert_eq!(partial.screen_bg, AppTheme::SIGNAL_ROOM.screen_bg);
        assert!(serde_json::from_str::<AppTheme>(r#"{"accent":"teal"}"#).is_err());
    }

    #[test]
    fn the_nsh_theme_has_the_app_tokens() {
        let teal = AppTheme {
            accent: nsh_core::color::rgb(0x53DCC9),
            ..AppTheme::SIGNAL_ROOM
        };
        let tokens = tokens(&teal);
        let color = |name: &str| tokens.color(name).unwrap_or_else(|| panic!("no {name}"));
        assert_eq!(color("primary"), AdaptiveColor::from(teal.accent));
        // The theme's colors, in the dark appearance.
        assert_eq!(
            color("background").dark,
            AdaptiveColor::from(teal.screen_bg).dark
        );
        assert_eq!(tokens.number("radius"), Some(12.0));
        // Ink and Metrics are gone (R17).
        assert_eq!(tokens.color("ink-meta"), None);
        assert_eq!(tokens.number("metrics-card-radius"), None);
    }

    /// The bar's scroll edge look is the theme's (B316): the effect's tint
    /// is the theme's screen color at `bar_edge::TINT`, the hairline the
    /// system's separator, the numbers `bar_edge`'s; a theme's own values
    /// reach the bar through `BarEdge::current` and the color readers.
    #[test]
    fn the_bars_scroll_edge_look_is_the_themes() {
        let app = AppTheme::SIGNAL_ROOM;
        let tokens = tokens(&app);
        let roles = crate::ui::tokens::colors(&app);
        let role = |name: &str| {
            roles
                .iter()
                .find(|(n, _, _)| *n == name)
                .map(|(_, light, dark)| (*light, *dark))
                .unwrap()
        };
        // UI-DESIGN §4.18: `background` at TINT and `border`, both
        // appearances; the dark edge is the one the bar wore (the screen's
        // color at TINT).
        let tint = |c: Rgba| Rgba {
            a: c.a * bar_edge::TINT,
            ..c
        };
        let (light, dark) = role("background");
        assert_eq!(dark, app.screen_bg);
        assert_eq!(
            tokens.color("bar-edge"),
            Some(AdaptiveColor::new(tint(light), tint(dark)))
        );
        let (line_light, line_dark) = role("border");
        assert_eq!(
            tokens.color("bar-hairline"),
            Some(AdaptiveColor::new(line_light, line_dark))
        );
        for (name, value) in [
            ("bar-edge-blur", bar_edge::BLUR),
            ("bar-edge-reach", bar_edge::REACH),
            ("bar-edge-fade", bar_edge::FADE),
            ("bar-edge-appear", bar_edge::APPEAR),
        ] {
            assert_eq!(tokens.number(name), Some(value), "{name}");
        }
    }
}
