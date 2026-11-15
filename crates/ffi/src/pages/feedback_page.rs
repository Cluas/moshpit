//! Feedback (`ui/src/feedback.rs`): its state and events cross as they are.

use moshpit_ui::feedback::{FeedbackEvent, FeedbackState, FeedbackSwitch};

/// What the Feedback page shows.
#[uniffi::remote(Record)]
pub struct FeedbackState {
    pub message: Option<String>,
    pub environment: bool,
    pub log: bool,
    pub preparing: bool,
    pub show_address: bool,
    pub copied: bool,
    pub recipient: String,
}

/// What the Feedback page tells the host.
#[uniffi::remote(Enum)]
pub enum FeedbackEvent {
    Cancel,
    Text { value: String },
    Set { key: FeedbackSwitch, value: bool },
    Send { message: String },
    CopyAddress,
    GithubIssue,
}

/// The Feedback page's two switches.
#[uniffi::remote(Enum)]
pub enum FeedbackSwitch {
    Environment,
    Log,
}
