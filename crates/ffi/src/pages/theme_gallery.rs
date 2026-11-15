//! Settings › Theme (`ui/src/theme_gallery.rs`). Its themes carry colors (and
//! the screen holds the eight ANSI ones as a fixed array, which UniFFI
//! doesn't carry), so they cross as bridge records converted into the
//! screen's own.

use moshpit_ui::theme_gallery::{Palette, Themes};
use nsh_core::color::Rgba;
use serde_json::Value;

use super::{PageError, color};

/// What the Theme page shows (`Themes`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThemeGalleryPageState {
    /// The app's own themes.
    pub built_in: Vec<ThemePaletteEntry>,
    /// The user's.
    pub custom: Vec<ThemePaletteEntry>,
    /// The id of the theme in effect.
    pub selected: String,
    /// The custom theme the delete card asks about, while it shows.
    pub deleting: Option<String>,
}

/// One theme (`Palette`): its name and the colors its row previews, the
/// eight ANSI ones in order.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThemePaletteEntry {
    pub id: String,
    pub name: String,
    pub background_hex: String,
    pub foreground_hex: String,
    pub cursor_hex: String,
    pub ansi_hex: Vec<String>,
}

/// What the Theme page tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ThemeGalleryEvent {
    Back,
    /// A row picked: the theme applies.
    Select {
        id: String,
    },
    /// The add menu's New…: the editor, on a draft.
    New,
    /// The add menu's Import…
    Import,
    /// The delete card's Delete.
    Delete {
        id: String,
    },
    /// The delete card's Cancel.
    CancelDelete,
}

impl TryFrom<ThemeGalleryPageState> for Themes {
    type Error = PageError;

    fn try_from(s: ThemeGalleryPageState) -> Result<Self, PageError> {
        let palettes = |entries: Vec<ThemePaletteEntry>| {
            entries
                .into_iter()
                .map(Palette::try_from)
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Themes {
            built_in: palettes(s.built_in)?,
            custom: palettes(s.custom)?,
            selected: s.selected,
            deleting: s.deleting,
        })
    }
}

impl TryFrom<ThemePaletteEntry> for Palette {
    type Error = PageError;

    fn try_from(e: ThemePaletteEntry) -> Result<Self, PageError> {
        let ansi = e
            .ansi_hex
            .iter()
            .map(|hex| color("ansi", hex))
            .collect::<Result<Vec<Rgba>, _>>()?;
        let count = ansi.len();
        Ok(Palette {
            background: color("background", &e.background_hex)?,
            foreground: color("foreground", &e.foreground_hex)?,
            cursor: color("cursor", &e.cursor_hex)?,
            ansi: ansi.try_into().map_err(|_| PageError::Invalid {
                field: "ansi".to_owned(),
                value: format!("{count} colors, not 8"),
            })?,
            id: e.id,
            name: e.name,
        })
    }
}

impl ThemeGalleryEvent {
    /// What the Theme page told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let id = || value["id"].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "back" => ThemeGalleryEvent::Back,
            "select" => ThemeGalleryEvent::Select { id: id()? },
            "new" => ThemeGalleryEvent::New,
            "import" => ThemeGalleryEvent::Import,
            "delete" => ThemeGalleryEvent::Delete { id: id()? },
            "cancel-delete" => ThemeGalleryEvent::CancelDelete,
            _ => return None,
        })
    }
}
