//! Install Assist (`ui/src/terminal/install.rs`). Its message is the core's
//! (`hostcare::install_assist_message`, which Android posts too); what it
//! tells comes back here.

use serde_json::Value;

/// What Install Assist tells the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InstallAssistEvent {
    /// Done: the sheet goes.
    Done,
    /// Run in Terminal: the command typed into the live shell.
    Run,
    /// Copy: the command on the pasteboard.
    Copy,
    /// Re-check: the host probed again.
    Recheck,
    /// Reconnect, once a re-check found everything (or had no channel).
    Reconnect,
}

impl InstallAssistEvent {
    /// What Install Assist told, read as it writes it; `None` for anything
    /// else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        Some(match value["ev"].as_str()? {
            "done" => InstallAssistEvent::Done,
            "run" => InstallAssistEvent::Run,
            "copy" => InstallAssistEvent::Copy,
            "recheck" => InstallAssistEvent::Recheck,
            "reconnect" => InstallAssistEvent::Reconnect,
            _ => return None,
        })
    }
}
