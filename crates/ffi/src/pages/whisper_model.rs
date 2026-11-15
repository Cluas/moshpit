//! Settings › Voice Input › Model (`ui/src/whisper_model.rs`). The screen's
//! types are named for its own module (`Model`, `Models`), too generic for
//! the app's, so they cross as prefixed records converted into them.

use moshpit_ui::whisper_model::{Model, Models};
use serde_json::Value;

/// What the Model page shows (`Models`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WhisperModelsState {
    /// The models this device can run, best first.
    pub models: Vec<WhisperModelRow>,
    /// The last download failure, in the system's words.
    pub failure: Option<String>,
    /// What the downloaded models take, formatted.
    pub storage: Option<String>,
}

/// One model as the page shows it (`Model`): its sizes formatted.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WhisperModelRow {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub approximate: String,
    pub installed: Option<String>,
    pub partial: Option<String>,
    /// How far its download is, 0 to 1, while one runs.
    pub progress: Option<f64>,
    pub selected: bool,
}

/// What the Model page tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WhisperModelEvent {
    Back,
    /// Use downloaded model `id`.
    Select {
        id: String,
    },
    /// Download, or Resume.
    Download {
        id: String,
    },
    Pause {
        id: String,
    },
    /// A paused download's trash.
    Discard {
        id: String,
    },
    /// A downloaded model's trash (the host asks first).
    Remove {
        id: String,
    },
}

impl From<WhisperModelsState> for Models {
    fn from(s: WhisperModelsState) -> Self {
        Models {
            models: s
                .models
                .into_iter()
                .map(|m| Model {
                    id: m.id,
                    name: m.name,
                    summary: m.summary,
                    approximate: m.approximate,
                    installed: m.installed,
                    partial: m.partial,
                    progress: m.progress,
                    selected: m.selected,
                })
                .collect(),
            failure: s.failure,
            storage: s.storage,
        }
    }
}

impl WhisperModelEvent {
    /// What the Model page told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let id = || value["id"].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "back" => WhisperModelEvent::Back,
            "select" => WhisperModelEvent::Select { id: id()? },
            "download" => WhisperModelEvent::Download { id: id()? },
            "pause" => WhisperModelEvent::Pause { id: id()? },
            "discard" => WhisperModelEvent::Discard { id: id()? },
            "remove" => WhisperModelEvent::Remove { id: id()? },
            _ => return None,
        })
    }
}
