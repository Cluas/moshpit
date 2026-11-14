//! tmux support for Moshpit's Rust core.
//!
//! - [`control`]: the control-mode (`-CC`) parser, a port of the app's Swift
//!   `TmuxControlClient` (G2), proven equivalent on recorded tmux streams by
//!   the harness in `harness/`.
//! - [`controller`]: the control-mode controller, a port of the app's Swift
//!   `TmuxSessionController` as a sans-I/O state machine (W8): attach and
//!   discovery, backfill and repair frames, keys and paste, the size lease
//!   and the layout pin, the mosh sidecar's session follow, the agent-hook
//!   poll and scroll routing.
//! - [`model`], [`format`](mod@format), [`lease`], [`clients`], [`bell`], [`hooks`]: the
//!   tmux world as the controller sees it and the text formats it reads and
//!   writes.
//! - [`boot`]: the command lines that attach the control client, the mosh
//!   sidecar and the mosh renderer.

pub mod bell;
pub mod boot;
pub mod clients;
pub mod control;
pub mod controller;
pub mod format;
#[doc(hidden)]
pub mod harness;
pub mod hooks;
pub mod lease;
pub mod model;
#[doc(hidden)]
pub mod rig;
#[doc(hidden)]
pub mod scenario;

pub use control::{
    CommandResponse, ControlError, ControlParser, Event, MAX_BLOCK_LINES, decode_octal_escapes,
};
