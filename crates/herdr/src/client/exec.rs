//! One exec channel's answer, and how the client reads it.

use crate::control::{self, CommandResult};
use crate::text;

/// What an exec channel gave back for an [`super::Output::Exec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecResult {
    /// What the command printed, stdout and stderr merged in the order they
    /// arrived, as far as it got: whether it finished is for the exit-status
    /// trailer to say ([`read_result`]), never for how the channel ended.
    Output(Vec<u8>),
    /// It never ran: the channel couldn't be opened, the connection is gone.
    Failed(String),
}

impl ExecResult {
    /// The output as text (lossy UTF-8, as the app decodes it).
    pub fn text(&self) -> Option<String> {
        match self {
            ExecResult::Output(bytes) => Some(text::decode_utf8_lossy(bytes)),
            ExecResult::Failed(_) => None,
        }
    }
}

/// Split the exit-status trailer (`__moshpit_rc=N`, echoed after herdr
/// exits) off a command's output, read as UTF-8 (lossily): the trailer
/// counts only as the last thing printed ([`control::parse_result`]).
pub fn read_result(raw: &[u8]) -> CommandResult {
    control::parse_result(&text::decode_utf8_lossy(raw))
}
