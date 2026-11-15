//! A folder on the host (`ui/src/terminal/folder.rs`): its listing, each
//! entry in the system's words. The screen's types are named for its own
//! module (`Entry`, `Folder`, `Failure`), so they cross as prefixed records
//! converted into them.

use moshpit_ui::terminal::folder::{Entry, Failure, Folder};
use serde_json::{Value, json};

use super::TextPromptQuestion;

/// What the folder page shows (`Folder`).
#[derive(Debug, Clone, PartialEq, Default, uniffi::Record)]
pub struct RemoteFolderPageState {
    pub name: String,
    pub path: String,
    /// The folders above this one, nearest first (the title's menu).
    pub parents: Vec<String>,
    /// The page was pushed (it has a back button).
    pub pushed: bool,
    pub loading: bool,
    /// Why the listing failed.
    pub failure: Option<String>,
    /// An upload is under way, this far.
    pub uploading: Option<f64>,
    /// Images and videos, then everything else.
    pub media: Vec<RemoteFolderItem>,
    pub rows: Vec<RemoteFolderItem>,
    /// A file that would not open, a change the host refused.
    pub open_failure: Option<String>,
    pub edit_failure: Option<RemoteFolderFailure>,
    /// Each failure's number: an alert is said once per number.
    pub open_failure_id: u64,
    pub edit_failure_id: u64,
}

/// One file or folder (`Entry`).
#[derive(Debug, Clone, PartialEq, Default, uniffi::Record)]
pub struct RemoteFolderItem {
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub video: bool,
    /// The SF Symbol of its kind.
    pub symbol: String,
    /// When it changed and how big it is.
    pub meta: String,
    /// The host has its thumbnail.
    pub thumb: bool,
    /// A video's length (`1:05`).
    pub duration: Option<String>,
    /// It is downloading to open, this far.
    pub opening: Option<f64>,
}

/// A failure to say in an alert (`Failure`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteFolderFailure {
    pub title: String,
    pub message: String,
}

/// What a tile's or a row's menu picked, handed back to the page
/// (`folder::Act`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteFolderAction {
    Open,
    Share,
    Rename,
    Delete,
}

/// The ＋ menu's uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteFolderUpload {
    Photos,
    Files,
}

/// Which failure's alert went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteFolderAlert {
    Open,
    Edit,
}

/// What the folder page tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteFolderEvent {
    /// A tile or a row (a folder pushes its page).
    Open {
        path: String,
    },
    Share {
        path: String,
    },
    /// The bar's back button on a pushed folder.
    Back,
    /// The title's menu: start over at a folder above.
    Climb {
        path: String,
    },
    Add {
        item: RemoteFolderUpload,
    },
    NewFolder {
        name: String,
    },
    Rename {
        path: String,
        name: String,
    },
    Delete {
        path: String,
    },
    /// Pulled down.
    Refresh,
    /// A failure's alert was read.
    Dismiss {
        failure: RemoteFolderAlert,
    },
    /// A name to ask for.
    Prompt {
        question: TextPromptQuestion,
    },
}

impl From<RemoteFolderPageState> for Folder {
    fn from(s: RemoteFolderPageState) -> Self {
        Folder {
            name: s.name,
            path: s.path,
            parents: s.parents,
            pushed: s.pushed,
            loading: s.loading,
            failure: s.failure,
            uploading: s.uploading,
            media: s.media.into_iter().map(Into::into).collect(),
            rows: s.rows.into_iter().map(Into::into).collect(),
            open_failure: s.open_failure,
            edit_failure: s.edit_failure.map(|f| Failure {
                title: f.title,
                message: f.message,
            }),
            open_failure_id: s.open_failure_id,
            edit_failure_id: s.edit_failure_id,
        }
    }
}

impl From<RemoteFolderItem> for Entry {
    fn from(e: RemoteFolderItem) -> Self {
        Entry {
            path: e.path,
            name: e.name,
            dir: e.dir,
            video: e.video,
            symbol: e.symbol,
            meta: e.meta,
            thumb: e.thumb,
            duration: e.duration,
            opening: e.opening,
        }
    }
}

/// `{"op": "folder", …}`, as the screen parses it.
pub(super) fn folder(folder: Folder) -> String {
    let mut value = serde_json::to_value(folder).expect("a folder serializes");
    value["op"] = json!("folder");
    value.to_string()
}

/// `{"op": "act", "action", "path"}`: `path`'s menu picked `action`.
pub(super) fn act(action: RemoteFolderAction, path: String) -> String {
    let action = match action {
        RemoteFolderAction::Open => "open",
        RemoteFolderAction::Share => "share",
        RemoteFolderAction::Rename => "rename",
        RemoteFolderAction::Delete => "delete",
    };
    json!({"op": "act", "action": action, "path": path}).to_string()
}

impl RemoteFolderEvent {
    /// What the folder page told, read as it writes it; `None` for
    /// anything else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "open" => RemoteFolderEvent::Open {
                path: text("path")?,
            },
            "share" => RemoteFolderEvent::Share {
                path: text("path")?,
            },
            "back" => RemoteFolderEvent::Back,
            "climb" => RemoteFolderEvent::Climb {
                path: text("path")?,
            },
            "add" => RemoteFolderEvent::Add {
                item: match value["item"].as_str()? {
                    "photos" => RemoteFolderUpload::Photos,
                    "files" => RemoteFolderUpload::Files,
                    _ => return None,
                },
            },
            "new-folder" => RemoteFolderEvent::NewFolder {
                name: text("name")?,
            },
            "rename" => RemoteFolderEvent::Rename {
                path: text("path")?,
                name: text("name")?,
            },
            "delete" => RemoteFolderEvent::Delete {
                path: text("path")?,
            },
            "refresh" => RemoteFolderEvent::Refresh,
            "dismiss" => RemoteFolderEvent::Dismiss {
                failure: match value["failure"].as_str()? {
                    "open" => RemoteFolderAlert::Open,
                    "edit" => RemoteFolderAlert::Edit,
                    _ => return None,
                },
            },
            "prompt" => RemoteFolderEvent::Prompt {
                question: TextPromptQuestion::read(&value)?,
            },
            _ => return None,
        })
    }
}
