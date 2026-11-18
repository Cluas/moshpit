//! The app's semantic color tokens (`docs/nsh/UI-DESIGN.md` §2): roles, not
//! uses, each with a light and a dark value. The `ui` layer and the screens
//! built on it read these; a raw color appears only here.
//!
//! - Most values are fixed. A few follow the theme the user picked
//!   ([`AppTheme`], §2.3): `background` is its screen color; the dark
//!   surfaces (`card`, `popover`, `muted`, …) keep Signal Room's steps above
//!   it, in its hue ([`follow`]); the `primary` family is its accent, with
//!   `primary-text` moved in lightness until it reads on every surface it
//!   sits on ([`readable_on`]) and `primary-foreground` whichever of white
//!   and ink reads on the accent ([`on_accent`]).
//! - Names follow NSH's default theme (shadcn's keys: `background`, `card`,
//!   `primary`, …, the fields of [`nsh_base::Theme`]); the roles it lacks
//!   are added beside them (`primary-text`, `warning-soft`, …). Careful with
//!   `accent`: here, as in shadcn, it is the quiet wash under a pressed row,
//!   not the user's accent color, which feeds `primary`.
//! - [`install`] writes them into a surface's theme. Until every screen is
//!   rebuilt on them (UI-DESIGN §8, R17) the old Ink colors stay beside them
//!   as `ink-*` ([`crate::theme::tokens`]); screens not yet rebuilt read
//!   only `primary` and `background` of these, whose dark values are the
//!   ones they always had (`theme`'s tests hold both).
//! - Every page is dark today (the host pins it); the light values are
//!   designed and tested here, and show once the host lets a page follow
//!   the system (R18).
//!
//! Owner picks that set a value here are named constants ([`SURFACE`]).

use nsh_core::{
    color::{Hsla, Rgba},
    element::AdaptiveColor,
};

use crate::theme::AppTheme;

// ---------------------------------------------------------------------------
// Owner picks (UI-DESIGN §0) that change a value here.

/// How cards set themselves off from the page (UI-DESIGN §0 #5,
/// `surfaces`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Borderless: the card's fill against the page is its edge.
    Plain,
    /// A hairline of `border-strong` around each card.
    Outline,
    /// A translucent fill (`glass`) over the blurred page.
    Glass,
}

/// The owner's pick for §0 #5; the recommendation is [`Surface::Plain`]. It
/// sets `card-fill`, `card-border`, `card-border-width` and `card-blur`,
/// all a card reads for its edge, so another pick is this one line.
pub const SURFACE: Surface = Surface::Plain;

// ---------------------------------------------------------------------------
// Literal values (§2.2).

const fn rgb(hex: u32) -> Rgba {
    nsh_core::color::rgb(hex)
}

const fn alpha(hex: u32, a: f32) -> Rgba {
    Rgba {
        a,
        ..nsh_core::color::rgb(hex)
    }
}

const WHITE: u32 = 0xFFFFFF;
const BLACK: u32 = 0x000000;
/// The light appearance's ink: its foreground, and what its washes and
/// lines are made of.
const INK: u32 = 0x16171B;

/// Signal Room's screen color, and the dark surfaces above it: the steps
/// every theme's dark surfaces keep ([`follow`]).
const SIGNAL_ROOM_SCREEN: Rgba = rgb(0x06070A);
const DARK_CARD: Rgba = rgb(0x13161C);
const DARK_POPOVER: Rgba = rgb(0x1B1E25);
const DARK_MUTED: Rgba = rgb(0x1E222A);
const DARK_SECONDARY: Rgba = rgb(0x262A33);
const DARK_THUMB: Rgba = rgb(0x3A3F4A);

/// The light page: warm paper, the same for every theme.
const LIGHT_BACKGROUND: Rgba = rgb(0xF3F2EE);
const LIGHT_SURFACE: Rgba = rgb(WHITE);
const LIGHT_MUTED: Rgba = rgb(0xECEBE6);
const LIGHT_SECONDARY: Rgba = rgb(0xE7E6E1);
/// The terminal's background in light chrome (a light terminal theme's
/// chrome, R12); dark chrome uses the theme's own terminal color.
const LIGHT_TERMINAL: Rgba = rgb(0xF7F6F2);

/// The fixed tokens: name, light, dark.
fn fixed() -> Vec<(&'static str, Rgba, Rgba)> {
    let mut tokens = vec![
        // Surfaces (those that follow the theme are in [`colors`]).
        ("accent", alpha(INK, 0.05), alpha(WHITE, 0.07)),
        ("chrome", alpha(0xF9F8F5, 0.80), alpha(0x101218, 0.74)),
        ("scrim", alpha(BLACK, 0.32), alpha(BLACK, 0.50)),
        // Foregrounds.
        ("foreground", rgb(INK), rgb(0xF2EFE8)),
        ("muted-foreground", rgb(0x5E5C56), rgb(0xA7A398)),
        ("subtle-foreground", rgb(0x8C8A83), rgb(0x77736A)),
        // Structure.
        ("border", alpha(INK, 0.10), alpha(WHITE, 0.09)),
        ("border-strong", alpha(INK, 0.18), alpha(WHITE, 0.16)),
        ("input", alpha(INK, 0.04), alpha(WHITE, 0.06)),
        // Status: solid, text, soft.
        ("success", rgb(0x1F9D55), rgb(0x4CC38A)),
        ("success-text", rgb(0x177346), rgb(0x7FDB95)),
        ("success-soft", alpha(0x1F9D55, 0.12), alpha(0x4CC38A, 0.16)),
        ("warning", rgb(0xD97706), rgb(0xFFB35C)),
        ("warning-text", rgb(0x935200), rgb(0xFFC27A)),
        ("warning-soft", alpha(0xD97706, 0.14), alpha(0xFFB35C, 0.16)),
        ("warning-foreground", rgb(0x2A1700), rgb(0x2A1700)),
        ("info", rgb(0x2F6FEB), rgb(0x7AA7FF)),
        ("info-text", rgb(0x2558C9), rgb(0x9DBDFF)),
        ("info-soft", alpha(0x2F6FEB, 0.12), alpha(0x7AA7FF, 0.16)),
        ("destructive", rgb(0xD93A3F), rgb(0xE5484D)),
        ("destructive-text", rgb(0xBC262C), rgb(0xFF7B7B)),
        (
            "destructive-soft",
            alpha(0xD93A3F, 0.12),
            alpha(0xE5484D, 0.16),
        ),
        // Terminal chrome (its appearance follows the terminal theme's, R12).
        ("key", rgb(WHITE), rgb(0x22262E)),
        ("key-foreground", rgb(INK), rgb(0xECE8DF)),
        ("key-pressed", rgb(0xE4E3DE), rgb(0x353A45)),
        ("grid-line", alpha(INK, 0.06), alpha(0x26303C, 0.24)),
        // A host key's face (the host mark, §4.13): eight hues, each a
        // colour for its pattern and a wash for its disc, the pattern
        // reading on the wash and on the card at 3:1 and better in both
        // appearances ([`FACES`]).
        ("face-red", rgb(0xC2362F), rgb(0xF27A72)),
        (
            "face-red-soft",
            alpha(0xC2362F, 0.14),
            alpha(0xF27A72, 0.18),
        ),
        ("face-orange", rgb(0xC2570C), rgb(0xF79A55)),
        (
            "face-orange-soft",
            alpha(0xC2570C, 0.14),
            alpha(0xF79A55, 0.18),
        ),
        ("face-amber", rgb(0x9A6A00), rgb(0xE8B53C)),
        (
            "face-amber-soft",
            alpha(0x9A6A00, 0.14),
            alpha(0xE8B53C, 0.18),
        ),
        ("face-green", rgb(0x1E8A47), rgb(0x5CC98A)),
        (
            "face-green-soft",
            alpha(0x1E8A47, 0.14),
            alpha(0x5CC98A, 0.18),
        ),
        ("face-teal", rgb(0x0B7F84), rgb(0x4CC9C9)),
        (
            "face-teal-soft",
            alpha(0x0B7F84, 0.14),
            alpha(0x4CC9C9, 0.18),
        ),
        ("face-blue", rgb(0x2E62D9), rgb(0x7AA7FF)),
        (
            "face-blue-soft",
            alpha(0x2E62D9, 0.14),
            alpha(0x7AA7FF, 0.18),
        ),
        ("face-violet", rgb(0x7046D6), rgb(0xB49AFB)),
        (
            "face-violet-soft",
            alpha(0x7046D6, 0.14),
            alpha(0xB49AFB, 0.18),
        ),
        ("face-pink", rgb(0xBE2F78), rgb(0xF283B8)),
        (
            "face-pink-soft",
            alpha(0xBE2F78, 0.14),
            alpha(0xF283B8, 0.18),
        ),
        // Shadows (their geometry: [`SHADOW_LG`], [`SHADOW_SM`]).
        ("shadow-lg", alpha(BLACK, 0.12), alpha(BLACK, 0.50)),
        ("shadow-sm", alpha(BLACK, 0.16), alpha(BLACK, 0.35)),
    ];
    tokens.extend(creatures());
    tokens
}

/// A host's pixel creature (the pixel companion's host mark,
/// moshpit-design `PET-DESIGN.md` §6.3), its colours by its cells' kinds
/// ([`creature_ink`]). Per face hue, from the hue's two `face-*` values:
/// in light, the body is the strong value 28 % toward white, the shade the
/// strong value, the light 72 % toward white, the line 55 % toward ink; in
/// dark, the body is the dark value, the shade 22 % toward black, the light
/// 50 % toward white, the line 60 % toward `#06070A`. The rest are the
/// same for every hue, and a muted creature (a host offline or failed)
/// takes a grey set (`creature-down-*`).
fn creatures() -> Vec<(&'static str, Rgba, Rgba)> {
    vec![
        ("creature-red-body", rgb(0xD36E69), rgb(0xF27A72)),
        ("creature-red-shade", rgb(0xC2362F), rgb(0xBD5F59)),
        ("creature-red-light", rgb(0xEEC7C5), rgb(0xF8BCB8)),
        ("creature-red-line", rgb(0x632524), rgb(0x643534)),
        ("creature-orange-body", rgb(0xD38650), rgb(0xF79A55)),
        ("creature-orange-shade", rgb(0xC2570C), rgb(0xC17842)),
        ("creature-orange-light", rgb(0xEED0BB), rgb(0xFBCCAA)),
        ("creature-orange-line", rgb(0x633414), rgb(0x664228)),
        ("creature-amber-body", rgb(0xB69447), rgb(0xE8B53C)),
        ("creature-amber-shade", rgb(0x9A6A00), rgb(0xB58D2F)),
        ("creature-amber-light", rgb(0xE3D5B8), rgb(0xF4DA9E)),
        ("creature-amber-line", rgb(0x513C0F), rgb(0x604D1E)),
        ("creature-green-body", rgb(0x5DAB7B), rgb(0x5CC98A)),
        ("creature-green-shade", rgb(0x1E8A47), rgb(0x489D6C)),
        ("creature-green-light", rgb(0xC0DECB), rgb(0xAEE4C4)),
        ("creature-green-line", rgb(0x1A4B2F), rgb(0x28553D)),
        ("creature-teal-body", rgb(0x4FA3A6), rgb(0x4CC9C9)),
        ("creature-teal-shade", rgb(0x0B7F84), rgb(0x3B9D9D)),
        ("creature-teal-light", rgb(0xBBDBDD), rgb(0xA6E4E4)),
        ("creature-teal-line", rgb(0x11464A), rgb(0x225556)),
        ("creature-blue-body", rgb(0x698EE4), rgb(0x7AA7FF)),
        ("creature-blue-shade", rgb(0x2E62D9), rgb(0x5F82C7)),
        ("creature-blue-light", rgb(0xC4D3F4), rgb(0xBCD3FF)),
        ("creature-blue-line", rgb(0x213970), rgb(0x34476C)),
        ("creature-violet-body", rgb(0x987AE1), rgb(0xB49AFB)),
        ("creature-violet-shade", rgb(0x7046D6), rgb(0x8C78C4)),
        ("creature-violet-light", rgb(0xD7CBF4), rgb(0xDACCFD)),
        ("creature-violet-line", rgb(0x3E2C6F), rgb(0x4C426A)),
        ("creature-pink-body", rgb(0xD0699E), rgb(0xF283B8)),
        ("creature-pink-shade", rgb(0xBE2F78), rgb(0xBD6690)),
        ("creature-pink-light", rgb(0xEDC5D9), rgb(0xF8C1DC)),
        ("creature-pink-line", rgb(0x622245), rgb(0x643950)),
        // The same for every hue.
        ("creature-horn", rgb(0xFBF5EA), rgb(0xFBF5EA)),
        ("creature-eye", rgb(INK), rgb(INK)),
        ("creature-white", rgb(WHITE), rgb(WHITE)),
        ("creature-mouth", rgb(0x7A2E3A), rgb(0x7A2E3A)),
        ("creature-blush", rgb(0xF7A3B8), rgb(0xFFB3C6)),
        ("creature-yellow", rgb(0xF5B700), rgb(0xFFD36E)),
        ("creature-green", rgb(0x2FA866), rgb(0x5CC98A)),
        ("creature-pink", rgb(0xE83E8C), rgb(0xFF5FA2)),
        // Muted: grey, its eyes still ink and white.
        ("creature-down-body", rgb(0xB9B6AE), rgb(0x5A5D66)),
        ("creature-down-shade", rgb(0x98958F), rgb(0x4A4C54)),
        ("creature-down-light", rgb(0xD8D7D2), rgb(0xA4A6AB)),
        ("creature-down-horn", rgb(0xE3E2DF), rgb(0xBDBEC2)),
        ("creature-down-line", rgb(0x5F5F5D), rgb(0x42444C)),
        ("creature-down-mouth", rgb(0x6B3A44), rgb(0x6B3A44)),
        ("creature-down-blush", rgb(0xD0A2B2), rgb(0x976C87)),
        ("creature-down-yellow", rgb(0xC9C3B6), rgb(0xC9C3B6)),
        ("creature-down-green", rgb(0x9FA3A0), rgb(0x9FA3A0)),
        ("creature-down-pink", rgb(0xB3AFB8), rgb(0xB3AFB8)),
    ]
}

/// A creature's hue tokens, in [`FACES`]' order: its body, shade, light
/// and line.
const CREATURE_HUES: [[&str; 4]; 8] = [
    [
        "creature-red-body",
        "creature-red-shade",
        "creature-red-light",
        "creature-red-line",
    ],
    [
        "creature-orange-body",
        "creature-orange-shade",
        "creature-orange-light",
        "creature-orange-line",
    ],
    [
        "creature-amber-body",
        "creature-amber-shade",
        "creature-amber-light",
        "creature-amber-line",
    ],
    [
        "creature-green-body",
        "creature-green-shade",
        "creature-green-light",
        "creature-green-line",
    ],
    [
        "creature-teal-body",
        "creature-teal-shade",
        "creature-teal-light",
        "creature-teal-line",
    ],
    [
        "creature-blue-body",
        "creature-blue-shade",
        "creature-blue-light",
        "creature-blue-line",
    ],
    [
        "creature-violet-body",
        "creature-violet-shade",
        "creature-violet-light",
        "creature-violet-line",
    ],
    [
        "creature-pink-body",
        "creature-pink-shade",
        "creature-pink-light",
        "creature-pink-line",
    ],
];

/// The colour token of a creature's cell: its kind (`moshpit_pet`'s
/// `creature::Kind`, by number: 1 body, 2 shade, 3 light, 4 horn, 5
/// outline, 6 eye, 7 white, 8 mouth, 9 blush, 10–12 a top's yellow, green
/// and pink) in face hue `hue`, or muted. None for an empty cell (0) or a
/// kind it doesn't know.
pub fn creature_ink(kind: u8, hue: u8, muted: bool) -> Option<&'static str> {
    let [body, shade, light, line] = if muted {
        [
            "creature-down-body",
            "creature-down-shade",
            "creature-down-light",
            "creature-down-line",
        ]
    } else {
        CREATURE_HUES[usize::from(hue) % CREATURE_HUES.len()]
    };
    let either = |awake: &'static str, down: &'static str| if muted { down } else { awake };
    Some(match kind {
        1 => body,
        2 => shade,
        3 => light,
        4 => either("creature-horn", "creature-down-horn"),
        5 => line,
        6 => "creature-eye",
        7 => "creature-white",
        8 => either("creature-mouth", "creature-down-mouth"),
        9 => either("creature-blush", "creature-down-blush"),
        10 => either("creature-yellow", "creature-down-yellow"),
        11 => either("creature-green", "creature-down-green"),
        12 => either("creature-pink", "creature-down-pink"),
        _ => return None,
    })
}

/// A shadow's geometry, in points; its color is a token of the same name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub y: f32,
    pub blur: f32,
}

/// What floats over content: menus, popovers, panels, toasts.
pub const SHADOW_LG: Shadow = Shadow {
    y: 12.0,
    blur: 32.0,
};
/// A control's thumb: a switch's, a segmented control's selected segment.
pub const SHADOW_SM: Shadow = Shadow { y: 1.0, blur: 3.0 };

/// The number tokens (§3): radii, the page's inset, widths, targets.
pub const NUMBERS: &[(&str, f32)] = &[
    ("radius-xs", 4.0),
    ("radius-sm", 8.0),
    ("radius", 12.0),
    ("radius-lg", 20.0),
    // A circle or a pill: a theme that squares corners squares these too.
    ("radius-full", 9999.0),
    ("space-page", 16.0),
    // The page's inset at a regular width (an iPad's).
    ("space-page-regular", 20.0),
    ("content-max-width", 640.0),
    // The least a pressable target is, on iOS (Android's 48 dp is its own).
    ("touch-target", 44.0),
];

// ---------------------------------------------------------------------------
// Derivations (§2.3).

/// `color` laid over `under` (both straight alpha): the opaque color seen.
pub fn over(color: Rgba, under: Rgba) -> Rgba {
    let a = color.a;
    let mix = |c: f32, u: f32| c * a + u * (1.0 - a);
    Rgba {
        r: mix(color.r, under.r),
        g: mix(color.g, under.g),
        b: mix(color.b, under.b),
        a: 1.0,
    }
}

/// `a` moved toward `b` by `t` (0: `a`, 1: `b`), alpha kept.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let m = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: a.a,
    }
}

/// WCAG's relative luminance of an opaque color.
pub fn luminance(color: Rgba) -> f32 {
    let channel = |c: f32| {
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}

/// WCAG's contrast ratio of two opaque colors (1 to 21).
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The least contrast text must have on what it sits on (WCAG AA, body
/// text).
pub const READABLE: f32 = 4.5;

/// A dark surface of the theme whose screen color is `screen`: Signal Room's
/// surface `signal_room` moved by what separates `screen` from Signal
/// Room's screen, in hue and lightness (its saturation kept). On Signal
/// Room it is the surface itself; on a warmer theme it is as far above that
/// theme's screen, in that theme's hue.
pub fn follow(screen: Rgba, signal_room: Rgba) -> Rgba {
    let base: Hsla = SIGNAL_ROOM_SCREEN.into();
    let target: Hsla = signal_room.into();
    let page: Hsla = screen.into();
    // A grey screen has no hue to follow: keep the surface's.
    let hue = if page.s < 0.02 {
        target.h
    } else {
        (page.h + target.h - base.h).rem_euclid(1.0)
    };
    Hsla {
        h: hue,
        s: target.s,
        l: (page.l + target.l - base.l).clamp(0.0, 1.0),
        a: 1.0,
    }
    .into()
}

/// `accent` moved in lightness (lighter for a dark page, darker for a light
/// one), its hue and saturation kept, by the least that makes it readable
/// ([`READABLE`]) on every color of `surfaces` (opaque): what the accent
/// is as text and icons.
pub fn readable_on(accent: Rgba, surfaces: &[Rgba], dark: bool) -> Rgba {
    let reads = |c: Rgba| surfaces.iter().all(|s| contrast(c, *s) >= READABLE);
    let mut hsl: Hsla = Rgba { a: 1.0, ..accent }.into();
    for _ in 0..=200 {
        let color: Rgba = hsl.into();
        if reads(color) {
            return color;
        }
        hsl.l = (hsl.l + if dark { 0.005 } else { -0.005 }).clamp(0.0, 1.0);
    }
    // At white or black every surface here reads; not reached.
    hsl.into()
}

/// What is drawn on the accent, a filled button's label: white while white
/// reads on it ([`READABLE`], 4.5:1, as body text: the label is 17 pt
/// semibold, but a pale or mid-tone accent under white is what users
/// reported unreadable), else the light appearance's ink (a pale accent: a
/// yellow, a light teal), else black (a mid-tone accent, such as Signal
/// Room's periwinkle, where neither white nor ink reaches 4.5:1). One of
/// white and black always does: every accent gets a readable label.
pub fn on_accent(accent: Rgba) -> Rgba {
    let accent = Rgba { a: 1.0, ..accent };
    [rgb(WHITE), rgb(INK), rgb(BLACK)]
        .into_iter()
        .find(|label| contrast(*label, accent) >= READABLE)
        .unwrap_or(rgb(BLACK))
}

/// A host key's face's hues (§4.13), in the order a face's `hue` picks
/// them: the pattern's colour token and its disc's wash.
pub const FACES: [(&str, &str); 8] = [
    ("face-red", "face-red-soft"),
    ("face-orange", "face-orange-soft"),
    ("face-amber", "face-amber-soft"),
    ("face-green", "face-green-soft"),
    ("face-teal", "face-teal-soft"),
    ("face-blue", "face-blue-soft"),
    ("face-violet", "face-violet-soft"),
    ("face-pink", "face-pink-soft"),
];

/// The light and the dark value of every color token for `app`.
pub fn colors(app: &AppTheme) -> Vec<(&'static str, Rgba, Rgba)> {
    let accent = Rgba {
        a: 1.0,
        ..app.accent
    };
    let screen = app.screen_bg;
    let dark_card = follow(screen, DARK_CARD);
    let dark_popover = follow(screen, DARK_POPOVER);
    let soft = |a: f32| Rgba { a, ..accent };
    let (light_soft, dark_soft) = (soft(0.12), soft(0.16));
    // Where the accent is text: the page, a card, a popup, and a chip of the
    // accent's own wash on a card or a popup.
    let light_text = readable_on(
        accent,
        &[
            LIGHT_BACKGROUND,
            LIGHT_SURFACE,
            over(light_soft, LIGHT_SURFACE),
            over(light_soft, LIGHT_BACKGROUND),
        ],
        false,
    );
    let dark_text = readable_on(
        accent,
        &[
            screen,
            dark_card,
            dark_popover,
            over(dark_soft, dark_card),
            over(dark_soft, dark_popover),
        ],
        true,
    );
    let foreground = (rgb(INK), rgb(0xF2EFE8));
    let (fill, border, _, _) = card_edge(dark_card);
    let mut colors = vec![
        // Surfaces.
        ("background", LIGHT_BACKGROUND, screen),
        ("card", LIGHT_SURFACE, dark_card),
        ("card-foreground", foreground.0, foreground.1),
        ("popover", LIGHT_SURFACE, dark_popover),
        ("popover-foreground", foreground.0, foreground.1),
        ("muted", LIGHT_MUTED, follow(screen, DARK_MUTED)),
        ("secondary", LIGHT_SECONDARY, follow(screen, DARK_SECONDARY)),
        ("secondary-foreground", foreground.0, foreground.1),
        ("thumb", LIGHT_SURFACE, follow(screen, DARK_THUMB)),
        ("accent-foreground", foreground.0, foreground.1),
        (
            "glass",
            alpha(WHITE, 0.60),
            Rgba {
                a: 0.58,
                ..dark_card
            },
        ),
        ("card-fill", fill.0, fill.1),
        ("card-border", border.0, border.1),
        // Actions and selection, from the accent.
        ("primary", accent, accent),
        (
            "primary-hover",
            mix(accent, rgb(BLACK), 0.08),
            mix(accent, rgb(WHITE), 0.08),
        ),
        ("primary-active", app.accent_pressed, app.accent_pressed),
        ("primary-foreground", on_accent(accent), on_accent(accent)),
        ("primary-text", light_text, dark_text),
        ("primary-soft", light_soft, dark_soft),
        ("selection", soft(0.22), soft(0.32)),
        ("ring", light_text, dark_text),
        // The terminal's own color: the theme's, dark.
        ("terminal-bg", LIGHT_TERMINAL, app.terminal_bg),
    ];
    colors.extend(fixed());
    colors
}

/// A card's edge under [`SURFACE`]: its fill, its border color, the
/// border's width and the blur behind it, each as (light, dark) where it
/// is a color.
fn card_edge(dark_card: Rgba) -> ((Rgba, Rgba), (Rgba, Rgba), f32, f32) {
    let none = alpha(BLACK, 0.0);
    let strong = (alpha(INK, 0.18), alpha(WHITE, 0.16));
    let glass = (
        alpha(WHITE, 0.60),
        Rgba {
            a: 0.58,
            ..dark_card
        },
    );
    match SURFACE {
        Surface::Plain => ((LIGHT_SURFACE, dark_card), (none, none), 0.0, 0.0),
        Surface::Outline => ((LIGHT_SURFACE, dark_card), strong, 1.0, 0.0),
        Surface::Glass => (glass, strong, 0.5, 24.0),
    }
}

/// The surface's color token `name` (`"primary-text"`), for the `ui`
/// layer's components; Signal Room's value where the surface's theme lacks
/// it (a surface run without the app's theme, such as a test's probe).
pub fn token(name: &str) -> AdaptiveColor {
    nsh_core::prelude::tokens().color(name).unwrap_or_else(|| {
        colors(&AppTheme::SIGNAL_ROOM)
            .into_iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, light, dark)| AdaptiveColor::new(light, dark))
            .unwrap_or_else(|| panic!("no color token {name}"))
    })
}

/// The surface's number token `name` (`"radius"`), with the same fallback
/// as [`token`].
pub fn number(name: &str) -> f32 {
    nsh_core::prelude::tokens().number(name).unwrap_or_else(|| {
        let (_, _, width, blur) = card_edge(follow(AppTheme::SIGNAL_ROOM.screen_bg, DARK_CARD));
        match name {
            "card-border-width" => width,
            "card-blur" => blur,
            _ => NUMBERS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, value)| *value)
                .unwrap_or_else(|| panic!("no number token {name}")),
        }
    })
}

/// Writes every token for `app` into `theme`: the colors ([`colors`]), the
/// numbers ([`NUMBERS`]) and the card edge's ([`SURFACE`]).
pub fn install(theme: &mut nsh_base::Theme, app: &AppTheme) {
    for (name, light, dark) in colors(app) {
        theme.set_color(name, AdaptiveColor::new(light, dark));
    }
    for (name, value) in NUMBERS {
        theme.set_number(name, *value);
    }
    let (_, _, width, blur) = card_edge(follow(app.screen_bg, DARK_CARD));
    theme.set_number("card-border-width", width);
    theme.set_number("card-blur", blur);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{hex_string, parse_hex};

    /// A file of the app's, read from the checkout.
    fn app_file(path: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn hex(text: &str) -> Rgba {
        parse_hex(&format!("#{text}")).unwrap()
    }

    /// The app's built-in themes (`AppThemeCatalog.builtIns`), read from
    /// AppTheme.swift: name and theme.
    fn built_ins() -> Vec<(String, AppTheme)> {
        let source = app_file("Moshpit/UI/AppTheme.swift");
        let mut themes = Vec::new();
        for block in source.split("static let ").skip(1) {
            let Some((name, rest)) = block.split_once(" = AppTheme(") else {
                continue;
            };
            let Some(args) = rest.split(')').next() else {
                continue;
            };
            let value = |key: &str| {
                let after = args.split(&format!("{key}: \"")).nth(1)?;
                Some(hex(&after[..6]))
            };
            let (Some(accent), Some(pressed), Some(screen), Some(terminal)) = (
                value("accentHex"),
                value("accentPressedHex"),
                value("screenBGHex"),
                value("terminalBGHex"),
            ) else {
                continue;
            };
            themes.push((
                name.trim().to_owned(),
                AppTheme {
                    accent,
                    accent_pressed: pressed,
                    screen_bg: screen,
                    terminal_bg: terminal,
                },
            ));
        }
        themes
    }

    /// `AppTheme.custom` for `accent`, as the Swift app derives it
    /// (`Color.blend` toward black, to whole bytes).
    fn custom(accent: &str) -> AppTheme {
        let accent = hex(accent);
        let toward_black = |t: f32| {
            let byte = |c: f32| ((c * (1.0 - t)).clamp(0.0, 1.0) * 255.0).round() / 255.0;
            Rgba {
                r: byte(accent.r),
                g: byte(accent.g),
                b: byte(accent.b),
                a: 1.0,
            }
        };
        AppTheme {
            accent,
            accent_pressed: toward_black(0.18),
            screen_bg: toward_black(0.955),
            terminal_bg: toward_black(0.982),
        }
    }

    /// Every theme the tests hold to the rules: the built-ins and custom
    /// accents at the edges (pale, near white, very dark, vivid).
    fn themes() -> Vec<(String, AppTheme)> {
        let mut themes = built_ins();
        assert_eq!(
            themes.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            [
                "signalRoom",
                "moshpitClassic",
                "terminalGreen",
                "amberConsole"
            ],
            "the parser still reads AppTheme.swift's built-ins"
        );
        for accent in ["F5E663", "EDEDED", "1A237E", "FF2D55", "8E8E93"] {
            themes.push((format!("custom {accent}"), custom(accent)));
        }
        themes
    }

    fn token(colors: &[(&'static str, Rgba, Rgba)], name: &str) -> (Rgba, Rgba) {
        colors
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, l, d)| (*l, *d))
            .unwrap_or_else(|| panic!("no token {name}"))
    }

    /// The opaque color a token shows over `under`, in each appearance.
    fn seen(
        colors: &[(&'static str, Rgba, Rgba)],
        name: &str,
        under: (Rgba, Rgba),
    ) -> (Rgba, Rgba) {
        let (l, d) = token(colors, name);
        (over(l, under.0), over(d, under.1))
    }

    #[test]
    fn every_role_has_one_token_and_both_appearances() {
        let colors = colors(&AppTheme::SIGNAL_ROOM);
        let mut names: Vec<&str> = colors.iter().map(|(n, _, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "a token is defined twice");
        // UI-DESIGN §2.2's roles, and the card edge (§0 #5) and shadows.
        let mut expected = vec![
            "background",
            "card",
            "card-foreground",
            "popover",
            "popover-foreground",
            "muted",
            "secondary",
            "secondary-foreground",
            "thumb",
            "accent",
            "accent-foreground",
            "chrome",
            "scrim",
            "foreground",
            "muted-foreground",
            "subtle-foreground",
            "border",
            "border-strong",
            "input",
            "ring",
            "primary",
            "primary-hover",
            "primary-active",
            "primary-foreground",
            "primary-text",
            "primary-soft",
            "selection",
            "success",
            "success-text",
            "success-soft",
            "warning",
            "warning-text",
            "warning-soft",
            "warning-foreground",
            "info",
            "info-text",
            "info-soft",
            "destructive",
            "destructive-text",
            "destructive-soft",
            "terminal-bg",
            "key",
            "key-foreground",
            "key-pressed",
            "grid-line",
            "glass",
            "card-fill",
            "card-border",
            "shadow-lg",
            "shadow-sm",
        ];
        // A host key's face's hues (§4.13), each a pattern and a wash.
        expected.extend(FACES.iter().flat_map(|(strong, soft)| [*strong, *soft]));
        // A host's pixel creature's (PET-DESIGN §6.3): every kind's, in
        // every hue and muted.
        expected.extend(creatures().iter().map(|(name, _, _)| *name));
        expected.sort_unstable();
        assert_eq!(names, expected);
        // The light appearance is designed, not a copy of the dark: every
        // surface and text role differs between the two.
        for name in [
            "background",
            "card",
            "foreground",
            "muted-foreground",
            "border",
            "primary-text",
        ] {
            let (l, d) = token(&colors, name);
            assert_ne!(l, d, "{name} has one value for both appearances");
        }
    }

    #[test]
    fn signal_room_is_the_design() {
        let colors = colors(&AppTheme::SIGNAL_ROOM);
        let close = |a: Rgba, b: Rgba| {
            [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
                .iter()
                .all(|(x, y)| (x - y).abs() <= 1.0 / 255.0 + 1e-4)
        };
        for (name, light, dark) in [
            ("background", "F3F2EE", "06070A"),
            ("card", "FFFFFF", "13161C"),
            ("popover", "FFFFFF", "1B1E25"),
            ("muted", "ECEBE6", "1E222A"),
            ("secondary", "E7E6E1", "262A33"),
            ("thumb", "FFFFFF", "3A3F4A"),
            ("foreground", "16171B", "F2EFE8"),
            ("muted-foreground", "5E5C56", "A7A398"),
            ("primary", "6C6BEF", "6C6BEF"),
            ("primary-active", "5652D6", "5652D6"),
            // Mid-tone: white reads at 4.2:1, black at 5:1 (4.5:1 for labels).
            ("primary-foreground", "000000", "000000"),
            // Derived (UI-DESIGN §2.3), as the spec's table has them.
            ("primary-text", "4E4DEC", "8A89F2"),
            ("primary-hover", "6362DC", "7877F0"),
            ("success-text", "177346", "7FDB95"),
            ("destructive-text", "BC262C", "FF7B7B"),
        ] {
            let (l, d) = token(&colors, name);
            assert!(close(l, hex(light)), "{name} light: {}", hex_string(l));
            assert!(close(d, hex(dark)), "{name} dark: {}", hex_string(d));
        }
    }

    /// A host key's face reads in both appearances under every theme: its
    /// pattern on its disc's wash and on the card (3:1, WCAG's for a
    /// graphic), and the eight hues are eight colours.
    #[test]
    fn a_host_faces_pattern_reads_on_its_disc_and_the_card() {
        for (theme_name, theme) in themes() {
            let colors = colors(&theme);
            let card = token(&colors, "card");
            for (strong, soft) in FACES {
                let (pattern, wash) = (token(&colors, strong), token(&colors, soft));
                for (dark, p, w, c) in [
                    (false, pattern.0, wash.0, card.0),
                    (true, pattern.1, wash.1, card.1),
                ] {
                    let disc = over(w, c);
                    let look = if dark { "dark" } else { "light" };
                    assert!(
                        contrast(p, c) >= 3.0,
                        "{theme_name}: {strong} on the card in {look}"
                    );
                    assert!(
                        contrast(p, disc) >= 3.0,
                        "{theme_name}: {strong} on its disc in {look}"
                    );
                }
            }
        }
        let names: std::collections::BTreeSet<_> =
            FACES.iter().map(|(strong, _)| *strong).collect();
        assert_eq!(names.len(), FACES.len());
    }

    /// A creature's colours are its face hue's, by PET-DESIGN §6.3's rule
    /// (the design's `build.py`), and every kind has one in every hue,
    /// awake and muted.
    #[test]
    fn a_creatures_colours_come_from_its_face_hue() {
        let colors = colors(&AppTheme::SIGNAL_ROOM);
        let mix = |a: Rgba, b: Rgba, t: f32| Rgba {
            r: a.r + (b.r - a.r) * t,
            g: a.g + (b.g - a.g) * t,
            b: a.b + (b.b - a.b) * t,
            a: 1.0,
        };
        let close = |a: Rgba, b: Rgba| {
            [(a.r, b.r), (a.g, b.g), (a.b, b.b)]
                .iter()
                .all(|(x, y)| (x - y).abs() <= 1.0 / 255.0 + 1e-4)
        };
        let (white, black) = (rgb(WHITE), rgb(BLACK));
        for (hue, (strong, _)) in FACES.iter().enumerate() {
            let (light, dark) = token(&colors, strong);
            let [body, shade, lit, line] = CREATURE_HUES[hue];
            for (name, want) in [
                (body, (mix(light, white, 0.28), dark)),
                (shade, (light, mix(dark, black, 0.22))),
                (lit, (mix(light, white, 0.72), mix(dark, white, 0.5))),
                (
                    line,
                    (
                        mix(light, rgb(INK), 0.55),
                        mix(dark, SIGNAL_ROOM_SCREEN, 0.6),
                    ),
                ),
            ] {
                let (l, d) = token(&colors, name);
                assert!(close(l, want.0), "{name} light: {}", hex_string(l));
                assert!(close(d, want.1), "{name} dark: {}", hex_string(d));
            }
            for muted in [false, true] {
                for kind in 1..=12 {
                    let ink = creature_ink(kind, hue as u8, muted).unwrap();
                    token(&colors, ink);
                }
                assert_eq!(creature_ink(0, hue as u8, muted), None, "empty");
                assert_eq!(creature_ink(13, hue as u8, muted), None);
            }
        }
    }

    #[test]
    fn text_reads_on_every_surface_it_sits_on() {
        for (theme_name, theme) in themes() {
            let colors = colors(&theme);
            let surfaces: Vec<(&str, (Rgba, Rgba))> = ["background", "card", "popover"]
                .into_iter()
                .map(|n| (n, token(&colors, n)))
                .collect();
            let check = |text: &str, on: &str, under: (Rgba, Rgba), least: f32| {
                let (tl, td) = token(&colors, text);
                for (dark, (t, u)) in [(false, (tl, under.0)), (true, (td, under.1))] {
                    let ratio = contrast(t, u);
                    assert!(
                        ratio >= least,
                        "{theme_name}: {text} on {on} ({}) is {ratio:.2}:1 in {}",
                        hex_string(u),
                        if dark { "dark" } else { "light" }
                    );
                }
            };
            for (on, under) in &surfaces {
                for text in ["foreground", "muted-foreground", "primary-text"] {
                    check(text, on, *under, READABLE);
                }
                for status in ["success", "warning", "info", "destructive"] {
                    let text = format!("{status}-text");
                    check(&text, on, *under, READABLE);
                    // A status's text on its own wash, laid on this surface.
                    let wash = seen(&colors, &format!("{status}-soft"), *under);
                    check(&text, &format!("{status}-soft on {on}"), wash, READABLE);
                }
                // The accent's text on a chip of the accent's wash.
                let wash = seen(&colors, "primary-soft", *under);
                check(
                    "primary-text",
                    &format!("primary-soft on {on}"),
                    wash,
                    READABLE,
                );
            }
            // Secondary text on the tracks and fills it labels.
            for fill in ["muted", "secondary"] {
                check("muted-foreground", fill, token(&colors, fill), READABLE);
            }
            check("key-foreground", "key", token(&colors, "key"), READABLE);
            check(
                "key-foreground",
                "key-pressed",
                token(&colors, "key-pressed"),
                READABLE,
            );
            check(
                "warning-foreground",
                "warning",
                token(&colors, "warning"),
                READABLE,
            );
            // A filled button's label (large text).
            check(
                "primary-foreground",
                "primary",
                token(&colors, "primary"),
                READABLE,
            );
        }
    }

    #[test]
    fn a_pale_accent_takes_ink_and_a_mid_tone_one_black() {
        let ink = rgb(INK);
        let pale = colors(&custom("F5E663"));
        assert_eq!(token(&pale, "primary-foreground"), (ink, ink));
        let teal = colors(&built_ins()[1].1);
        assert_eq!(
            token(&teal, "primary-foreground"),
            (ink, ink),
            "Moshpit Classic's teal is pale"
        );
        // Signal Room's periwinkle is mid-tone: white reads at 4.2:1 and
        // ink at 4.3:1 on it, black at 5:1.
        let signal = colors(&AppTheme::SIGNAL_ROOM);
        assert_eq!(
            token(&signal, "primary-foreground"),
            (rgb(BLACK), rgb(BLACK))
        );
        // A deep accent keeps white.
        let deep = colors(&custom("1A237E"));
        assert_eq!(token(&deep, "primary-foreground"), (rgb(WHITE), rgb(WHITE)));
    }

    /// A filled button's label reads at 4.5:1 on its fill, in both
    /// appearances, for every built-in accent, pale and mid-tone picks, and
    /// any accent at all (a sweep plus a seeded random walk).
    #[test]
    fn a_filled_buttons_label_reads_on_any_accent() {
        let mut accents: Vec<(String, Rgba)> = built_ins()
            .into_iter()
            .map(|(name, theme)| (name, theme.accent))
            .collect();
        for pick in [
            "53DCC9", "F5E663", "EDEDED", "FFFFFF", "000000", "8E8E93", "777777", "6C6BEF",
            "FF2D55", "34C759", "FF9500", "5AC8FA", "AF52DE", "1A237E",
        ] {
            accents.push((pick.to_owned(), hex(pick)));
        }
        let mut seed: u32 = 0x9E37_79B9;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let byte = |shift: u32| ((seed >> shift) & 0xFF) as f32 / 255.0;
            let color = Rgba {
                r: byte(0),
                g: byte(8),
                b: byte(16),
                a: 1.0,
            };
            accents.push((hex_string(color), color));
        }
        for grey in 0..=255u32 {
            let c = grey as f32 / 255.0;
            accents.push((
                format!("grey {grey}"),
                Rgba {
                    r: c,
                    g: c,
                    b: c,
                    a: 1.0,
                },
            ));
        }
        for (name, accent) in accents {
            let colors = colors(&custom(&hex_string(accent).trim_start_matches('#')[..6]));
            let (fill, label) = (
                token(&colors, "primary"),
                token(&colors, "primary-foreground"),
            );
            for (side, (f, l)) in [("light", (fill.0, label.0)), ("dark", (fill.1, label.1))] {
                let ratio = contrast(l, f);
                assert!(
                    ratio >= READABLE,
                    "{name}: label {} on {} is {ratio:.2}:1 ({side})",
                    hex_string(l),
                    hex_string(f)
                );
            }
        }
    }

    #[test]
    fn dark_surfaces_follow_the_theme() {
        // Amber Console's screen is warm: its cards are warm too, as far
        // above it as Signal Room's are above Signal Room's screen.
        let amber = &built_ins()[3].1;
        let card = token(&colors(amber), "card").1;
        assert!(
            card.r > card.b,
            "a warm theme's card is warm: {}",
            hex_string(card)
        );
        let step = luminance(card) - luminance(amber.screen_bg);
        let signal_step = luminance(DARK_CARD) - luminance(SIGNAL_ROOM_SCREEN);
        assert!(
            (step - signal_step).abs() < 0.004,
            "{step} vs {signal_step}"
        );
    }

    #[test]
    fn the_card_edge_is_the_surface_pick() {
        let colors = colors(&AppTheme::SIGNAL_ROOM);
        let (fill, border) = (token(&colors, "card-fill"), token(&colors, "card-border"));
        match SURFACE {
            Surface::Plain => {
                assert_eq!(fill, token(&colors, "card"));
                assert_eq!((border.0.a, border.1.a), (0.0, 0.0));
            }
            Surface::Outline => assert_eq!(border, token(&colors, "border-strong")),
            Surface::Glass => assert_eq!(fill, token(&colors, "glass")),
        }
    }

    #[test]
    fn the_theme_carries_every_token() {
        let mut theme = nsh_base::Theme::default();
        install(&mut theme, &AppTheme::SIGNAL_ROOM);
        let tokens = theme.into_tokens();
        for (name, light, dark) in colors(&AppTheme::SIGNAL_ROOM) {
            assert_eq!(
                tokens.color(name),
                Some(AdaptiveColor::new(light, dark)),
                "{name}"
            );
        }
        for (name, value) in NUMBERS {
            assert_eq!(tokens.number(name), Some(*value), "{name}");
        }
        assert!(tokens.number("card-border-width").is_some());
        assert!(tokens.number("card-blur").is_some());
    }
}
