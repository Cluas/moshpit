//! herdr support for Moshpit's Rust core: the pure logic of
//! `Moshpit/Services/Herdr/*`, ported from the app's Swift and proven
//! equivalent on the same inputs by the harness in `harness/`.
//!
//! - [`frame`]: the frame channel (`herdr terminal session control`): its
//!   newline-delimited JSON frames, and every line the app writes into it
//!   (boot and retarget lines, input, resize, scroll, mouse, release),
//!   bracketed pastes and semantic arrow keys.
//! - [`snapshot`]: `herdr api snapshot` decoded into sessions, windows and
//!   panes, agent stamps, working directories, terminal ids, worktrees; the
//!   server-not-running and protocol-mismatch replies.
//! - [`socket`]: the JSON socket API: requests, the reply and event routing
//!   (the subscribe's bootstrap replay and live mutations), the pump boot
//!   line, the push driver's subscription kinds and debounce.
//! - [`launch`]: the attach, daemon and mosh raw-attach command lines.
//! - [`renderer`]: mosh's raw-attach renderer around them, as a sans-I/O
//!   state machine: which pane the loop shows, when a switch counts, where
//!   typing and deliveries may go.
//! - [`version`]: `herdr status` and version skew.
//! - [`control`]: the control plane's commands and how their results are
//!   read (`refusal` and friends), the agent-task steps, the poll's
//!   classification and cadence.
//! - [`client`]: the control client around all of it, as a sans-I/O state
//!   machine (W2b): the command lane and the polls, the frame channel's
//!   attach and its write lane, the push channel, agent tasks, removal,
//!   discovery and restarts, every wait bounded.
//! - [`json`], [`text`], [`base64`]: Foundation's rules as the Swift code
//!   relies on them, written out so the port reads every line the same way.
//!
//! No SSH, no tasks, no UI: every type here is fed bytes or results and
//! hands back values; [`client::Client`] adds the timers and the ordering,
//! still without I/O, so a host only carries out what it asks. Where the Swift code misbehaves the
//! port is fixed and says so (`Fix (<id>)` in the source, the entries of
//! `harness/divergences.json`).

pub mod base64;
pub mod client;
pub mod control;
pub mod frame;
#[doc(hidden)]
pub mod harness;
pub mod json;
pub mod launch;
pub mod renderer;
pub mod snapshot;
pub mod socket;
pub mod text;
pub mod version;

pub use client::Client;
pub use frame::{Frame, FrameParser};
pub use snapshot::{AgentHook, AgentState, Decoded, Pane, Session, Snapshot, Window};
pub use socket::{SocketClient, SocketError, SocketEvent};
pub use version::VersionReport;
