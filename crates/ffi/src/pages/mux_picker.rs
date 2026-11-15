//! A multiplexer's list (`ui/src/terminal/picker.rs`): its windows,
//! sessions or panes. The screen's types are named for its own module
//! (`Row`, `Kind`), so they cross as prefixed records converted into them;
//! its small enums (the list, the multiplexer, an agent's signal) as their
//! ids, the app's own enums' raw values.

use moshpit_ui::terminal::model::{Mux, Signal};
use moshpit_ui::terminal::picker::{Kind, Picker, Row};
use serde_json::{Value, json};

use super::{PageError, TextPromptQuestion, choice};

/// What the picker shows (`Picker`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MuxPickerPageState {
    /// `windows`, `sessions` or `panes`.
    pub kind: String,
    /// `tmux` or `herdr`.
    pub mux: String,
    /// In the order the list shows them.
    pub rows: Vec<MuxPickerRow>,
    /// The row the multiplexer is on.
    pub active: Option<String>,
    /// The list picks where a held share goes (B622): the share's agent,
    /// whose panes come first. A pick is a `Select` all the same.
    pub send: Option<String>,
}

/// One row, in the host's facts (`Row`; the page words them).
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct MuxPickerRow {
    /// What the multiplexer knows it by (`@3`, `$1`, `%7`, herdr's `w4`).
    pub id: String,
    /// A window's or a pane's number.
    pub index: Option<u32>,
    /// A window's or a session's name.
    pub name: String,
    /// A window's panes.
    pub panes: Option<u32>,
    pub command: String,
    pub agent: Option<String>,
    pub path: Option<String>,
    /// The agents' signal: `attention`, `working` or `done`.
    pub signal: Option<String>,
    /// A session another client is attached to.
    pub attached: bool,
    /// The active session's windows, and its active window (`index:name`).
    pub windows: Option<u32>,
    pub window: Option<String>,
}

/// What a row's long-press menu picked, handed back to the screen to ask
/// about (`picker::Act`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MuxPickerAction {
    Rename,
    Kill,
}

/// What the picker tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MuxPickerEvent {
    /// A row: the host switches and closes.
    Select {
        id: String,
    },
    /// The bar's +, with the name asked for (none: the multiplexer's own).
    New {
        name: Option<String>,
    },
    Rename {
        id: String,
        name: String,
    },
    Kill {
        id: String,
    },
    /// A name to ask for.
    Prompt {
        question: TextPromptQuestion,
    },
}

impl TryFrom<MuxPickerPageState> for Picker {
    type Error = PageError;

    fn try_from(s: MuxPickerPageState) -> Result<Self, PageError> {
        Ok(Picker {
            kind: choice::<Kind>("kind", &s.kind)?,
            mux: choice::<Mux>("mux", &s.mux)?,
            rows: s
                .rows
                .into_iter()
                .map(Row::try_from)
                .collect::<Result<_, _>>()?,
            active: s.active,
            send: s.send,
        })
    }
}

impl TryFrom<MuxPickerRow> for Row {
    type Error = PageError;

    fn try_from(r: MuxPickerRow) -> Result<Self, PageError> {
        Ok(Row {
            signal: r
                .signal
                .as_deref()
                .map(|id| choice::<Signal>("signal", id))
                .transpose()?,
            id: r.id,
            index: r.index,
            name: r.name,
            panes: r.panes,
            command: r.command,
            agent: r.agent,
            path: r.path,
            attached: r.attached,
            windows: r.windows,
            window: r.window,
        })
    }
}

/// `{"op": "picker", …}`, as the screen parses it.
pub(super) fn picker(picker: Picker) -> String {
    let mut value = serde_json::to_value(picker).expect("a picker serializes");
    value["op"] = json!("picker");
    value.to_string()
}

/// `{"op": "act", "action", "id"}`: row `id`'s menu picked `action`.
pub(super) fn act(action: MuxPickerAction, id: String) -> String {
    let action = match action {
        MuxPickerAction::Rename => "rename",
        MuxPickerAction::Kill => "kill",
    };
    json!({"op": "act", "action": action, "id": id}).to_string()
}

impl MuxPickerEvent {
    /// What the picker told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "select" => MuxPickerEvent::Select { id: text("id")? },
            "new" => MuxPickerEvent::New { name: text("name") },
            "rename" => MuxPickerEvent::Rename {
                id: text("id")?,
                name: text("name")?,
            },
            "kill" => MuxPickerEvent::Kill { id: text("id")? },
            "prompt" => MuxPickerEvent::Prompt {
                question: TextPromptQuestion::read(&value)?,
            },
            _ => return None,
        })
    }
}
