//! Color choices (`docs/nsh/UI-DESIGN.md` §4.19, §6.4): [`Swatch`], a
//! 28 pt circle with its name under it, [`CursorColor`] with
//! [`cursor_colors`], the terminal cursor's four, and [`color_well`], the
//! system's well for picking any color.
//!
//! Every swatch has a visible name and is a button screen readers hear by
//! it (pain point 6: two cursor colors looked the same and nothing said
//! which was which). The one picked is ringed: a 2 pt `foreground` ring
//! with 2 pt of `card` between it and the color, the ring scaling up from
//! 0.8 as it fades in (at once under reduced motion); screen readers hear
//! it selected. A color that can vanish into the card (white on a white
//! card) is drawn with a thin edge ([`Swatch::outlined`]).
//!
//! B318: the cursor's Green and Accent were both the accent, so two of the
//! four circles were one color. Now each is its own: Accent `primary`, Blue
//! `info`, Green `success`, White white at 86%. An accent can still be one
//! of the others (Terminal Green's is a green): a fixed swatch that looks
//! like the accent (ΔE under [`HIDE_BELOW`]) is not shown, since Accent
//! already offers that color ([`CursorColor::visible`]).

use std::rc::Rc;

use nsh_core::{color::Rgba, prelude::*};
use serde_json::json;

use crate::{
    theme::{AppTheme, hex_string, parse_hex},
    ui::{
        feedback::touch_target,
        motion::{Duration, SETTLE_CURVE},
        tokens::{self, token},
        typography::TextToken,
    },
};

/// A swatch's circle.
pub const SWATCH: f32 = 28.0;
/// The ring around the one picked, and the room between them.
const RING: f32 = 2.0;
const GAP: f32 = 2.0;
/// The ring's scale as it starts to show.
const RING_FROM: f32 = 0.8;
/// The thin edge of an outlined swatch.
const EDGE: f32 = 1.0;

/// A color choice ([`swatch`]).
pub struct Swatch {
    name: SharedString,
    color: Rc<dyn Fn() -> AdaptiveColor>,
    outlined: bool,
    size: f32,
    selected: Rc<dyn Fn() -> bool>,
    on_press: Rc<dyn Fn()>,
    test_id: Option<SharedString>,
}

/// A swatch of `color`, named `name` under it and to screen readers.
pub fn swatch(
    name: impl Into<SharedString>,
    color: impl Fn() -> AdaptiveColor + 'static,
) -> Swatch {
    Swatch {
        name: name.into(),
        color: Rc::new(color),
        outlined: false,
        size: SWATCH,
        selected: Rc::new(|| false),
        on_press: Rc::new(|| {}),
        test_id: None,
    }
}

impl Swatch {
    /// A color that can vanish into the card it sits on (white on a white
    /// card): drawn with a thin `border-strong` edge, in both appearances.
    pub fn outlined(mut self) -> Self {
        self.outlined = true;
        self
    }

    /// The circle `points` across instead of [`SWATCH`] (the accent grid's
    /// 44, §6.10); the ring and its room stay 2 pt.
    pub fn size(mut self, points: f32) -> Self {
        self.size = points;
        self
    }

    /// The one picked: ringed, and selected for screen readers.
    pub fn selected(mut self, selected: impl Fn() -> bool + 'static) -> Self {
        self.selected = Rc::new(selected);
        self
    }

    pub fn on_press(mut self, on_press: impl Fn() + 'static) -> Self {
        self.on_press = Rc::new(on_press);
        self
    }

    /// The swatch is `id`; its circle `{id}-color`.
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Swatch {
    fn into_element(self) -> AnyElement {
        let Swatch {
            name,
            color,
            outlined,
            size,
            selected,
            on_press,
            test_id,
        } = self;
        let reduced = reduce_motion();
        let outer = size + 2.0 * (GAP + RING);
        let shown = selected.clone();
        let ring = view()
            .absolute()
            .inset_0()
            .rounded_full()
            .border(px(RING))
            .border_color(move || token("foreground"))
            .opacity(move || if shown() { 1.0_f32 } else { 0.0 })
            .scale({
                let selected = selected.clone();
                move || {
                    if reduced || selected() {
                        1.0_f32
                    } else {
                        RING_FROM
                    }
                }
            })
            .transition_all()
            .duration(Duration::Fast.ms())
            .ease_bezier(
                SETTLE_CURVE[0],
                SETTLE_CURVE[1],
                SETTLE_CURVE[2],
                SETTLE_CURVE[3],
            );
        // The color, the swatch's size, on `card`: the room between it and
        // the ring.
        let mut paint = view()
            .w(px(size))
            .h(px(size))
            .rounded_full()
            .bg(move || color());
        if outlined {
            paint = paint
                .border(px(EDGE))
                .border_color(move || token("border-strong"));
        }
        if let Some(id) = &test_id {
            paint = paint.test_id(&format!("{id}-color"));
        }
        let dot = view()
            .w(px(size + 2.0 * GAP))
            .h(px(size + 2.0 * GAP))
            .rounded_full()
            .items_center()
            .justify_center()
            .bg(move || token("card"))
            .child(paint);
        let circle = view()
            .relative()
            .w(px(outer))
            .h(px(outer))
            .items_center()
            .justify_center()
            .child(dot)
            .child(ring);
        let words = TextToken::Caption
            .apply(text(name.clone()))
            .text_color({
                let selected = selected.clone();
                move || {
                    token(if selected() {
                        "foreground"
                    } else {
                        "muted-foreground"
                    })
                }
            })
            // One line up to the largest standard size, as many as it
            // needs from AX1, centred under the circle (B840): French
            // "Couleur d'accentuation" was cut at AX5.
            .number_of_lines(name_lines)
            .text_center()
            .max_w(relative(1.0));
        let mut control = nsh_base::button()
            .accessibility_label(name)
            .selected(move || selected())
            .on_press(move || on_press())
            .flex_shrink_0()
            .items_center()
            .gap(px(4.0))
            .min_w(px(touch_target()))
            .max_w(relative(1.0))
            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || token("ring")));
        if let Some(id) = &test_id {
            control = control.test_id(id);
        }
        control.child(circle).child(words).into_element()
    }
}

/// The lines a swatch's name takes at the user's text size: one up to the
/// largest standard size, as many as it needs from AX1, as a row's words
/// do (B753).
fn name_lines() -> u32 {
    u32::from(crate::face::category(font_scale()) < crate::face::AX1)
}

// MARK: - The cursor's colors

/// A fixed swatch closer than this to the accent (CIE76 ΔE, in either
/// appearance, on the card) is not shown: it would look like Accent.
pub const HIDE_BELOW: f32 = 12.0;

/// The White cursor: white, at 86%, in both appearances.
pub const WHITE: Rgba = Rgba {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.86,
};

/// The terminal cursor's colors, in the swatches' order (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorColor {
    Accent,
    Blue,
    Green,
    White,
}

/// A color token's value in each appearance, (light, dark).
pub type Palette<'a> = &'a dyn Fn(&str) -> (Rgba, Rgba);

impl CursorColor {
    pub const ALL: [CursorColor; 4] = [
        CursorColor::Accent,
        CursorColor::Blue,
        CursorColor::Green,
        CursorColor::White,
    ];

    /// The one stored before any is picked (`teal`, the store's default).
    pub const DEFAULT: CursorColor = CursorColor::Blue;

    /// Its id, as the setting stores it (`cursorColorId`). Unchanged from
    /// every build before (1.0.8 reads them): the blue swatch is stored
    /// as `teal`, the default.
    pub fn id(self) -> &'static str {
        match self {
            CursorColor::Accent => "accent",
            CursorColor::Blue => "teal",
            CursorColor::Green => "green",
            CursorColor::White => "white",
        }
    }

    /// The color stored as `id`; another id names none.
    pub fn from_id(id: &str) -> Option<CursorColor> {
        CursorColor::ALL.into_iter().find(|c| c.id() == id)
    }

    /// Its name, in the app's language.
    pub fn name(self) -> String {
        match self {
            CursorColor::Accent => t!("Accent"),
            CursorColor::Blue => t!("Blue"),
            CursorColor::Green => t!("Green"),
            CursorColor::White => t!("White"),
        }
    }

    /// Its color token; `None`: White's own.
    fn source(self) -> Option<&'static str> {
        match self {
            CursorColor::Accent => Some("primary"),
            CursorColor::Blue => Some("info"),
            CursorColor::Green => Some("success"),
            CursorColor::White => None,
        }
    }

    /// Drawn with a thin edge: White vanishes into a white card.
    pub fn outlined(self) -> bool {
        self == CursorColor::White
    }

    /// Its color, from the surface's tokens.
    pub fn color(self) -> AdaptiveColor {
        match self.source() {
            Some(name) => token(name),
            None => AdaptiveColor::from(WHITE),
        }
    }

    /// Its color with `palette`'s tokens, light and dark ([`color`]'s).
    ///
    /// [`color`]: CursorColor::color
    pub fn value(self, palette: Palette<'_>) -> (Rgba, Rgba) {
        match self.source() {
            Some(name) => palette(name),
            None => (WHITE, WHITE),
        }
    }

    /// What the person sees of it, light and dark, laid on the `card` it
    /// sits on, with `palette`'s tokens.
    pub fn seen(self, palette: Palette<'_>) -> (Rgba, Rgba) {
        let (light, dark) = self.value(palette);
        let (card_light, card_dark) = palette("card");
        (
            tokens::over(light, card_light),
            tokens::over(dark, card_dark),
        )
    }

    /// The swatches shown with `palette`'s accent, in order: Accent, and
    /// each fixed color that differs from it in both appearances (ΔE at
    /// least [`HIDE_BELOW`]).
    pub fn visible(palette: Palette<'_>) -> Vec<CursorColor> {
        let accent = CursorColor::Accent.seen(palette);
        CursorColor::ALL
            .into_iter()
            .filter(|c| {
                *c == CursorColor::Accent || {
                    let seen = c.seen(palette);
                    delta_e(seen.0, accent.0) >= HIDE_BELOW
                        && delta_e(seen.1, accent.1) >= HIDE_BELOW
                }
            })
            .collect()
    }

    /// The swatch shown picked for the stored id `stored`: its own while it
    /// is shown, Accent while it is not (the accent is that color then; the
    /// stored id is left as it is); none for an id that names no color.
    pub fn shown_picked(stored: &str, visible: &[CursorColor]) -> Option<CursorColor> {
        let color = CursorColor::from_id(stored)?;
        Some(if visible.contains(&color) {
            color
        } else {
            CursorColor::Accent
        })
    }

    /// The color the cursor is drawn in for the stored id `stored` with
    /// `palette`'s accent: the one the swatches show picked
    /// ([`shown_picked`]), so a hidden choice draws as Accent; an id that
    /// names no color draws as [`DEFAULT`].
    ///
    /// [`shown_picked`]: CursorColor::shown_picked
    /// [`DEFAULT`]: CursorColor::DEFAULT
    pub fn drawn(stored: &str, palette: Palette<'_>) -> CursorColor {
        let visible = CursorColor::visible(palette);
        CursorColor::shown_picked(stored, &visible)
            .or_else(|| CursorColor::shown_picked(CursorColor::DEFAULT.id(), &visible))
            .unwrap_or(CursorColor::Accent)
    }
}

/// `app`'s tokens, light and dark.
pub fn palette_of(app: &AppTheme) -> impl Fn(&str) -> (Rgba, Rgba) + use<> {
    let colors = tokens::colors(app);
    move |name: &str| {
        colors
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, light, dark)| (*light, *dark))
            .unwrap_or_else(|| panic!("no color token {name}"))
    }
}

/// The surface's tokens, light and dark (read where it runs: a reactive
/// read of the theme).
pub(crate) fn live_palette(name: &str) -> (Rgba, Rgba) {
    let color = token(name);
    (Rgba::from(color.light), Rgba::from(color.dark))
}

/// The CIE76 color difference of two opaque sRGB colors.
pub fn delta_e(a: Rgba, b: Rgba) -> f32 {
    let (a, b) = (lab(a), lab(b));
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// CIELAB (D65) of an opaque sRGB color.
fn lab(c: Rgba) -> (f32, f32, f32) {
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(c.r), linear(c.g), linear(c.b));
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f32| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// The cursor's swatches ([`cursor_colors`]).
pub struct CursorColors {
    stored: Rc<dyn Fn() -> String>,
    on_pick: Rc<dyn Fn(&'static str)>,
    test_id: Option<SharedString>,
}

/// The cursor's swatches for the stored id `stored()`: the ones that differ
/// from the surface's accent ([`CursorColor::visible`]), the one shown
/// picked as [`CursorColor::shown_picked`] says; `on_pick` hears the id a
/// press picks, to store.
pub fn cursor_colors(
    stored: impl Fn() -> String + 'static,
    on_pick: impl Fn(&'static str) + 'static,
) -> CursorColors {
    CursorColors {
        stored: Rc::new(stored),
        on_pick: Rc::new(on_pick),
        test_id: None,
    }
}

impl CursorColors {
    /// The row is `id`; each swatch `{id}-{stored id}` (`{id}-teal`).
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for CursorColors {
    fn into_element(self) -> AnyElement {
        let CursorColors {
            stored,
            on_pick,
            test_id,
        } = self;
        let visible = computed(move || CursorColor::visible(&live_palette));
        let picked = computed(move || {
            let stored = stored();
            visible.with(|v| CursorColor::shown_picked(&stored, v))
        });
        let row_id = test_id.clone();
        let mut row = view()
            .flex_row()
            .flex_wrap()
            .gap(px(16.0))
            .child(dynamic(move || {
                let on_pick = on_pick.clone();
                visible
                    .get()
                    .into_iter()
                    .map(|color| {
                        let on_pick = on_pick.clone();
                        let mut swatch = swatch(color.name(), move || color.color())
                            .selected(move || picked.get() == Some(color))
                            .on_press(move || on_pick(color.id()));
                        if color.outlined() {
                            swatch = swatch.outlined();
                        }
                        if let Some(id) = &test_id {
                            swatch = swatch.test_id(&format!("{id}-{}", color.id()));
                        }
                        swatch.into_element()
                    })
                    .collect::<Vec<_>>()
            }));
        if let Some(id) = &row_id {
            row = row.test_id(id);
        }
        row.into_element()
    }
}

/// The box a color well sits in, in a row's trailing lane; the well is
/// UIKit's size, centered in it.
const COLOR_WELL_WIDTH: f32 = 34.0;
/// UIKit's color well's height.
const COLOR_WELL_HEIGHT: f32 = 28.0;

/// The system's color well (the host's `moshpit/color-well`, UIKit's
/// `UIColorWell`): it shows `color()`; a tap presents the system's color
/// picker (without opacity: a palette entry has none), and `on_change`
/// hears each color picked there. Screen readers hear `label`; `id` is the
/// well's accessibility identifier, which tests find it by.
///
/// A touch target whole (44 pt): the box stays UIKit's well's size, so the
/// row keeps its height and the well its look, and its hit slop takes the
/// rest, which the host's component hands to the well.
pub fn color_well(
    color: impl Fn() -> Rgba + 'static,
    id: Option<&str>,
    label: String,
    on_change: impl Fn(Rgba) + 'static,
) -> AnyElement {
    let (x, y) = (
        ((touch_target() - COLOR_WELL_WIDTH) / 2.0).max(0.0),
        ((touch_target() - COLOR_WELL_HEIGHT) / 2.0).max(0.0),
    );
    native("moshpit/color-well")
        .w(px(COLOR_WELL_WIDTH))
        .h(px(COLOR_WELL_HEIGHT))
        .flex_shrink_0()
        .hit_slop(EdgeInsets {
            top: Points(y),
            right: Points(x),
            bottom: Points(y),
            left: Points(x),
        })
        .prop("color", move || json!(hex_string(color())))
        .prop(
            "identifier",
            id.map_or(serde_json::Value::Null, |id| json!(id)),
        )
        .prop("label", json!(label))
        .on_native_event(move |event: &NativeEvent| {
            if &*event.name == "change"
                && let Some(color) = event.data.as_str().and_then(parse_hex)
            {
                on_change(color);
            }
        })
        .into_element()
}

#[cfg(test)]
mod tests {
    use moshpit_store::app_themes::{AppTheme as Stored, catalog};
    use serde_json::json;

    use super::*;

    /// The app's theme as the host posts it from the store's.
    fn theme(stored: &Stored) -> AppTheme {
        serde_json::from_value(json!({
            "accent": format!("#{}", stored.accent_hex),
            "accentPressed": format!("#{}", stored.accent_pressed_hex),
            "screenBG": format!("#{}", stored.screen_bg_hex),
            "terminalBG": format!("#{}", stored.terminal_bg_hex),
        }))
        .unwrap_or_else(|e| panic!("{}: {e}", stored.name))
    }

    /// The built-in themes, and custom accents at the edges: R0's (pale,
    /// near white, very dark, vivid, grey) and ones at the fixed colors
    /// themselves (the info and success blues and greens, pure white).
    fn themes() -> Vec<(String, AppTheme)> {
        let mut themes: Vec<(String, AppTheme)> = catalog::built_ins()
            .iter()
            .map(|t| (t.name.clone(), theme(t)))
            .collect();
        assert_eq!(themes.len(), 4);
        for accent in [
            "F5E663", "EDEDED", "1A237E", "FF2D55", "8E8E93", "2F6FEB", "7AA7FF", "1F9D55",
            "4CC38A", "FFFFFF", "5FE3D8",
        ] {
            let custom = Stored::custom("c", &format!("custom {accent}"), accent);
            themes.push((custom.name.clone(), theme(&custom)));
        }
        themes
    }

    fn shown(app: &AppTheme) -> Vec<CursorColor> {
        CursorColor::visible(&palette_of(app))
    }

    /// B318: the swatches shown are distinct — every pair, both
    /// appearances, on the card — under every built-in theme and every
    /// edge accent.
    #[test]
    fn every_two_swatches_shown_look_different() {
        for (name, app) in themes() {
            let palette = palette_of(&app);
            let shown = CursorColor::visible(&palette);
            assert_eq!(shown[0], CursorColor::Accent, "{name}: Accent always shows");
            assert!(shown.len() >= 2, "{name}: {shown:?}");
            for (i, a) in shown.iter().enumerate() {
                for b in &shown[i + 1..] {
                    let (x, y) = (a.seen(&palette), b.seen(&palette));
                    for (look, x, y) in [("light", x.0, y.0), ("dark", x.1, y.1)] {
                        let d = delta_e(x, y);
                        assert!(d >= HIDE_BELOW, "{name}: {a:?} and {b:?} in {look}: ΔE {d}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_fixed_swatch_like_the_accent_hides() {
        let built_in = |id: &str| {
            let stored = catalog::built_ins()
                .into_iter()
                .find(|t| t.id == id)
                .unwrap();
            shown(&theme(&stored))
        };
        use CursorColor::*;
        assert_eq!(built_in("signal-room"), [Accent, Blue, Green, White]);
        assert_eq!(
            built_in("terminal-green"),
            [Accent, Blue, White],
            "Terminal Green's accent is a green"
        );
        let custom = |accent: &str| shown(&theme(&Stored::custom("c", "c", accent)));
        assert!(!custom("2F6FEB").contains(&Blue), "the info blue");
        assert!(!custom("4CC38A").contains(&Green), "the success green");
        assert!(!custom("FFFFFF").contains(&White), "white");
    }

    #[test]
    fn a_hidden_choice_shows_accent_picked_and_stays_stored() {
        use CursorColor::*;
        let all = [Accent, Blue, Green, White];
        let terminal_green = [Accent, Blue, White];
        assert_eq!(CursorColor::shown_picked("green", &all), Some(Green));
        assert_eq!(
            CursorColor::shown_picked("green", &terminal_green),
            Some(Accent)
        );
        assert_eq!(
            CursorColor::shown_picked("teal", &terminal_green),
            Some(Blue)
        );
        assert_eq!(CursorColor::shown_picked("magenta", &all), None);
    }

    /// B318 where the cursor is drawn: each stored id draws its own color,
    /// `green` the success green and not the accent; a hidden choice draws
    /// as Accent; an unknown id as the default.
    #[test]
    fn the_cursor_draws_the_color_the_swatches_show_picked() {
        let built_in = |id: &str| {
            theme(
                &catalog::built_ins()
                    .into_iter()
                    .find(|t| t.id == id)
                    .unwrap(),
            )
        };
        use CursorColor::*;
        let signal_room = palette_of(&built_in("signal-room"));
        for color in CursorColor::ALL {
            assert_eq!(CursorColor::drawn(color.id(), &signal_room), color);
        }
        let (green, accent) = (Green.value(&signal_room), Accent.value(&signal_room));
        assert_ne!(green, accent, "green is not the accent");
        assert!(delta_e(green.1, accent.1) >= HIDE_BELOW);
        assert_eq!(green, signal_room("success"));
        assert_eq!(Blue.value(&signal_room), signal_room("info"));
        assert_eq!(White.value(&signal_room), (WHITE, WHITE));
        let terminal_green = palette_of(&built_in("terminal-green"));
        assert_eq!(CursorColor::drawn("green", &terminal_green), Accent);
        assert_eq!(CursorColor::drawn("teal", &terminal_green), Blue);
        assert_eq!(CursorColor::drawn("magenta", &signal_room), Blue);
        assert_eq!(CursorColor::drawn("", &signal_room), CursorColor::DEFAULT);
        // Under every theme the id drawn is the one shown picked.
        for (name, app) in themes() {
            let palette = palette_of(&app);
            let visible = CursorColor::visible(&palette);
            for color in CursorColor::ALL {
                assert_eq!(
                    Some(CursorColor::drawn(color.id(), &palette)),
                    CursorColor::shown_picked(color.id(), &visible),
                    "{name}: {color:?}"
                );
            }
        }
    }

    /// B729: an upgrade from 1.0.8 keeps the stored `teal`, the store's
    /// default. 1.0.8 filled its swatch with the signal blue `7AA7FF` and
    /// gave it no name, though its cursor drew `5FE3D8`. 1.0.9 ticks the
    /// swatch named Blue and draws that same blue, so the tick, the name
    /// and the cursor agree under every theme; where an accent like the
    /// blue hides Blue, the tick and the cursor are both Accent.
    #[test]
    fn a_stored_teal_from_1_0_8_is_ticked_and_drawn_as_one_blue() {
        use CursorColor::*;
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        let rgb = |c: Rgba| byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b);
        let mut blue_shown = 0;
        for (name, app) in themes() {
            let palette = palette_of(&app);
            let visible = CursorColor::visible(&palette);
            let ticked = CursorColor::shown_picked("teal", &visible);
            let drawn = CursorColor::drawn("teal", &palette);
            assert_eq!(ticked, Some(drawn), "{name}: the tick is what's drawn");
            if visible.contains(&Blue) {
                blue_shown += 1;
                assert_eq!(drawn, Blue, "{name}");
                assert_eq!(
                    rgb(drawn.value(&palette).1),
                    0x7AA7FF,
                    "{name}: 1.0.8's swatch blue"
                );
            } else {
                assert_eq!(drawn, Accent, "{name}: Blue hidden");
            }
        }
        assert!(blue_shown >= 4, "every built-in theme shows Blue");
    }

    #[test]
    fn the_stored_ids_are_the_ones_every_build_reads() {
        let ids: Vec<&str> = CursorColor::ALL.iter().map(|c| c.id()).collect();
        assert_eq!(ids, ["accent", "teal", "green", "white"]);
        for color in CursorColor::ALL {
            assert_eq!(CursorColor::from_id(color.id()), Some(color));
        }
        assert_eq!(CursorColor::from_id("blue"), None, "never stored");
        assert_eq!(CursorColor::DEFAULT.id(), "teal", "the store's default");
    }

    #[test]
    fn each_is_its_own_color_and_white_is_white() {
        let sources: Vec<Option<&str>> = CursorColor::ALL.iter().map(|c| c.source()).collect();
        assert_eq!(
            sources,
            [Some("primary"), Some("info"), Some("success"), None]
        );
        assert_eq!(WHITE.a, 0.86);
        assert_eq!((WHITE.r, WHITE.g, WHITE.b), (1.0, 1.0, 1.0));
        assert!(CursorColor::White.outlined());
        assert!(!CursorColor::Blue.outlined());
    }

    #[test]
    fn delta_e_matches_known_values() {
        let black = Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let white = Rgba { a: 1.0, ..WHITE };
        assert!((delta_e(black, white) - 100.0).abs() < 0.1);
        assert_eq!(delta_e(white, white), 0.0);
    }
}
