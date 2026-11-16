//! Sessions for Moshpit's Rust core (`docs/nsh/DESIGN.md` §3.2, §3.3): the
//! registry of connections the app has open, and each one's lifecycle —
//! dialling with generations, the state machine the UI reads, redial and
//! keepalive, the end of a session told apart from a drop, mosh with its
//! fall back to SSH, and the multiplexer attach onto the byte channel.
//!
//! - [`Sessions`]: the registry. `connect(spec) → SessionId`, the person's
//!   calls (disconnect, reconnect, switch to SSH, a host-key answer, tmux
//!   intents), the app's lifecycle and network changes, a read model per
//!   session ([`SessionView`]) and one stream of plain-data [`Event`]s.
//!   Every pane's terminal is a byte channel (`crates/stream`) named by the
//!   stream id in [`EventKind::Panes`].
//! - [`Lifecycle`]: the rules as a pure state machine on a virtual clock —
//!   generations, end versus drop, dial recovery on the foreground return
//!   and network changes, keepalive probes, the 20 s forced reconnect,
//!   failures that retrying can't fix left alone.
//! - The pane tasks: a plain shell on an SSH terminal channel; W8's tmux
//!   controller on `tmux -CC attach` ([`TmuxAction`]); W2b's herdr client
//!   on the frame channel, exec channels and its push channel ([`herdr`],
//!   [`HerdrAction`]); mosh's UDP loop over `crates/mosh`'s transport
//!   ([`mosh_udp`]: timers, flow rebuilds, roaming, the return-path
//!   watchdog, the echo acknowledgement predictive echo judges by), with
//!   tmux's own client as the mosh session's program when tmux is asked
//!   for, and W8's controller as its `-CC` sidecar beside it (the tree, the
//!   window pinned to the phone, session follow).
//!
//! The byte path is the design's: one task reads a transport and pushes
//! into the pane's channel as it reads (no per-chunk hops, nothing
//! dropped); input reaches the transport through one ordered queue per
//! session, held for its pane while that pane isn't attached.
//!
//! Borrowing the session's SSH ([`Sessions::borrow_ssh`]; for the hosts,
//! `CoreSessions::borrow_ssh` → `BorrowedSsh` in `crates/ffi`) is how what
//! runs beside a terminal reaches the host: the live SSH leg's connection,
//! mosh + tmux's control connection (a borrow in the round trip before its
//! terminal is up waits for it, rather than signing in beside it), or one
//! side connection the session dials and keeps for plain mosh. No second
//! sign-in or Face ID; its life
//! is the session's (closed on the reconnect or the end, which fails what
//! is in flight; the next borrow gets the new one). What the app's side
//! features need to switch to it — today each rides `ActiveSession.
//! acquireFileTransferSSH()`, a Swift SSH connection dialled beside the
//! Rust session:
//!
//! - **Remote preview** (`RemoteFileAccess.channel`): `sftp_stat`,
//!   `sftp_list`, `sftp_read` (offset and length: the text preview's
//!   ranges), `sftp_download` (to the preview cache, with progress),
//!   `sftp_upload_new` / `sftp_write` / `sftp_rename` / `sftp_remove_file`
//!   for edits; `BorrowFailure::Sftp { why: Unavailable }` is the host
//!   without SFTP, where the exec fallbacks (`RemoteFileCommands`) go over
//!   `run`.
//! - **Uploads** (image attachments, the share queue):
//!   `upload_to_uploads_directory(data, filename, progress)` (SFTP, exec on
//!   a host without it), and the uploads sweep over `run`.
//! - **File transfer and the path probes**: `run` (the probe's 8 s bound
//!   stays the caller's), the SFTP calls above.
//! - **HostAutoCare** (`HostChannel.run`): `run`, throwing unless
//!   `finished` and status 0 (what `executeCommand` does since B136).
//! - Each caller's liveness probe (`answersQuickly`) becomes `run("true")`
//!   on the borrowed connection, a failure borrowing again.
//! - **Notification replies and shares** (content for one pane, from the
//!   lock screen or the share sheet) need no connection: they go through
//!   [`Sessions::deliver`] (already wired for Rust sessions: the app's
//!   `deliverInput` / `deliverPaste`), which waits out a reconnect
//!   (bounded, [`DELIVERY_WAIT`]) instead of dropping what isn't typed live
//!   ([`Discarded::NotLive`], right for the keyboard and wrong for a reply
//!   the person sent from a notification), switches herdr to the pane and
//!   holds it for the pane's first frame (B185), and answers whether it
//!   went out.
//!
//! herdr over mosh ([`herdr_mosh`]): mosh-server runs the raw-attach loop
//! (or herdr's own TUI), and W2b's client runs beside it on the kept
//! bootstrap connection, steering the loop with execs, as tmux's sidecar
//! does.
//!
//! Tests: the lifecycle and the mosh loop on virtual clocks (unit tests);
//! the whole session against a private sshd, mosh-server and tmux server on
//! the loopback with faults injected (`tests/live.rs`, ignored by default).

mod attempt;
pub mod event;
pub mod herdr;
pub mod herdr_mosh;
pub mod lifecycle;
pub mod mosh_udp;
mod registry;
mod services;
mod session;
mod shell;
pub mod spec;
mod tmux_leg;

pub use attempt::{Route, mosh_falls_back, route};
pub use event::{
    Cause, CredentialsWhy, Degrade, Discarded, EndReason, Event, EventKind, Failure, FailureKind,
    KeyWhy, MoshWhy, PaneInfo, ProxyWhy, ResumeOutcome, SHELL_PANE, SessionId, SessionState,
    TunnelWhy, UnreachableWhy,
};
pub use herdr::{HerdrAction, HerdrAnswer, HerdrView, unanswered};
pub use lifecycle::{Lifecycle, Transport};
pub use mosh_udp::{SystemUdp, UdpFlow, UdpNetwork};
pub use registry::{Events, FLUSH_BOUND, Sessions};
pub use services::{CredentialError, Credentials, NoMemory, Services, SessionMemory};
pub use session::{DELIVERY_WAIT, SessionView};
pub use spec::{ConnectionSpec, Hop, HopBroken, Protocol, Timing, Via};
pub use tmux_leg::{TmuxAction, TmuxRefusal, TmuxView, mark as view_marks, view_action, view_mark};

pub use moshpit_install::Multiplexer;
pub use moshpit_mosh::{ChannelState as MoshChannelState, Diagnostics as MoshDiagnostics};
pub use moshpit_ssh::LinkFault;
pub use moshpit_stream::{Mark, StreamId};
