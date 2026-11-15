//! A question with a line of text for its answer
//! (`ui/src/terminal/prompt.rs`), which a screen asks through its host:
//! NSH's alerts have no text field. The question comes in the event of the
//! screen that asks (Mux Picker's, Remote Folder's, the terminal's); the
//! answer is one message every one of them takes
//! ([`PageMessage::Prompted`](super::PageMessage::Prompted)).

use serde_json::{Value, json};

/// What a screen asks (`prompt::Prompt`, with the id it waits on), every
/// word already in the app's language.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TextPromptQuestion {
    pub id: u32,
    pub title: String,
    pub message: Option<String>,
    /// What the empty field says.
    pub placeholder: String,
    /// What the field starts with.
    pub text: String,
    /// The button that answers, and the one that does not.
    pub confirm: String,
    pub cancel: String,
}

impl TextPromptQuestion {
    /// The question in `told` (`{"ev": "prompt", …}`, as `prompt::event`
    /// writes it).
    pub(super) fn read(told: &Value) -> Option<Self> {
        let text = |key: &str| told[key].as_str().map(str::to_owned);
        Some(TextPromptQuestion {
            id: u32::try_from(told["id"].as_u64()?).ok()?,
            title: text("title")?,
            message: text("message"),
            placeholder: text("placeholder").unwrap_or_default(),
            text: text("text").unwrap_or_default(),
            confirm: text("confirm")?,
            cancel: text("cancel")?,
        })
    }
}

/// The answer to prompt `id`: the text, or none (Cancel).
pub(super) fn answer(id: u32, text: Option<String>) -> String {
    json!({"op": "prompted", "id": id, "text": text}).to_string()
}
