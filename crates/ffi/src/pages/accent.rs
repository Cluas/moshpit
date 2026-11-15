//! Settings › Accent (`ui/src/accent.rs`). Its accents carry colors, so they
//! cross as bridge records converted into the screen's own (`#RRGGBB` in,
//! as the app writes them).

use moshpit_ui::accent::{Accent, Accents};
use serde_json::Value;

use super::{PageError, color};

/// What the Accent page shows (`Accents`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccentPageState {
    /// The app's own accents.
    pub built_in: Vec<AccentEntry>,
    /// The user's, newest last.
    pub custom: Vec<AccentEntry>,
    /// The id of the row to tick.
    pub selected: String,
    /// The custom accent the delete card asks about, while it shows.
    pub deleting: Option<String>,
}

/// One accent (`Accent`): its name and the two colors its row's swatch
/// shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccentEntry {
    pub id: String,
    pub name: String,
    pub accent_hex: String,
    /// The screen background it derives, the swatch's ground.
    pub screen_bg_hex: String,
}

/// What the Accent page tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AccentEvent {
    Back,
    /// A row picked.
    Select {
        id: String,
    },
    /// New: the editor, on a draft.
    Add,
    /// The delete card's Delete.
    Delete {
        id: String,
    },
    /// The delete card's Cancel.
    CancelDelete,
}

impl TryFrom<AccentPageState> for Accents {
    type Error = PageError;

    fn try_from(s: AccentPageState) -> Result<Self, PageError> {
        let accents = |entries: Vec<AccentEntry>| {
            entries
                .into_iter()
                .map(Accent::try_from)
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Accents {
            built_in: accents(s.built_in)?,
            custom: accents(s.custom)?,
            selected: s.selected,
            deleting: s.deleting,
        })
    }
}

impl TryFrom<AccentEntry> for Accent {
    type Error = PageError;

    fn try_from(e: AccentEntry) -> Result<Self, PageError> {
        Ok(Accent {
            accent: color("accent", &e.accent_hex)?,
            screen_bg: color("screenBG", &e.screen_bg_hex)?,
            id: e.id,
            name: e.name,
        })
    }
}

impl AccentEvent {
    /// What the Accent page told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let id = || value["id"].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "back" => AccentEvent::Back,
            "select" => AccentEvent::Select { id: id()? },
            "add" => AccentEvent::Add,
            "delete" => AccentEvent::Delete { id: id()? },
            "cancel-delete" => AccentEvent::CancelDelete,
            _ => return None,
        })
    }
}
