//! The one Rust static library the iOS app links (`libmoshpit_ffi.a`).
//!
//! An app can link only one Rust static library: two would each carry the
//! Rust runtime and collide at link time (`docs/nsh/DESIGN.md` §3.2). So
//! every core crate the app uses is linked through here, re-exported, and
//! whatever C ABI the app calls is defined or re-exported here too:
//!
//! - [`moshpit_tmux`]: the tmux control-mode parser, called by
//!   `TmuxControlClient` when the Rust parser switch is on, through
//!   [`tmux`]'s `mp_tmux_parser_*` (`include/moshpit_ffi.h`, imported into
//!   Swift as the `MoshpitFFI` module).
//! - [`moshpit_stream`]: the byte channel (`mp_stream_*`,
//!   `crates/stream/include/moshpit_stream.h`): a pane's bytes, both ways,
//!   and the marks its view acts on, when a session runs on the Rust core.
//! - [`moshpit_term`]: the shared terminal core (`mp_term_*`,
//!   `crates/term/include/moshpit_term.h`): the emulator the app's new
//!   terminal view (`CoreTerminalView`, behind its `MOSHPIT_TERM_CORE`
//!   switch) draws frames of and encodes keys and the mouse with, parsing
//!   off the main thread (docs/nsh/TERMINAL-CORE.md).
//! - [`moshpit_session`]: the session layer (SSH, mosh, tmux's controller,
//!   herdr's client), through UniFFI ([`session`], [`herdr`], and
//!   [`borrow`] for the session's SSH lent to what runs beside a terminal):
//!   what the app runs a terminal on behind its `MOSHPIT_RUST_SESSION`
//!   switch.
//! - [`moshpit_ssh::keyfile`]: SSH keys — import in every form, generation,
//!   what a stored key is and what the keychain keeps — through UniFFI
//!   ([`keys`]); the keychain, the Secure Enclave and the file picker stay
//!   the app's.
//! - [`moshpit_store`]: what the app persists and the rules for reading and
//!   writing it, through UniFFI ([`store`]; Swift and Kotlin bindings from
//!   `examples/uniffi-bindgen.rs`) — not wired to the app's stores yet.
//! - [`moshpit_app`]: the screens' host models, shared with Android, through
//!   UniFFI ([`app`]): the connection form, the shortcut pages, the remote
//!   paths and folder, image attachments, the accent and theme editors; and
//!   Home's record, read from the session registry ([`home`]).
//! - [`moshpit_hostcare`]: host care, the Host Setup screen, Install
//!   Assist and the phone's side of push pairing, through UniFFI
//!   ([`hostcare`]): the core's machine, with Swift as its platform (a
//!   command on a session's host, the relay's HTTPS, the device token, the
//!   App Group's pairings file).
//! - [`moshpit_ui`]: the NSH screens, with NSH itself, and on iOS their
//!   host's C ABI (`mp_ui_*`, `ui/include/moshpit_ui.h`, in the
//!   `MoshpitFFI` module too): the app runs a screen in an NSH view behind
//!   its switch. NSH runs the screens' tasks on the core's runtime
//!   (`moshpit-runtime`).
//!
//! Built by `scripts/rust/build-ios.sh` from the app target's "Build Rust"
//! phase (`project.yml`), with the `app` profile of the workspace manifest
//! for a Release build and `app-test` (no whole-graph LTO) for a Debug one.
//!
//! # Why a hand-written C ABI and not UniFFI
//!
//! The parser is fed every chunk of the `-CC` stream, so its call is on the
//! hot path of every keystroke's echo. UniFFI lowers a `Vec<u8>` argument
//! and a `Vec<Event>` result through serialised `RustBuffer`s: the chunk is
//! copied into a buffer and out again, every event is written field by
//! field and read back, and the buffers are allocated and freed through
//! their own calls. Here a feed is one call with the chunk borrowed, and
//! each event one callback with its fields borrowed; Swift copies each field
//! once, into the `String`s and `Data` its callbacks already take.
//!
//! Measured on an M2 (2026-11-11; this crate's `app` profile against the
//! same parser behind UniFFI 0.31 with LTO, Swift `-O -wmo`, every event
//! turned into the `String`s and `Data` the app's callbacks take), CPU per
//! feed, median of 7–21 rounds:
//!
//! | input | C ABI | UniFFI |
//! | --- | --- | --- |
//! | one keystroke's echo, `%output %0 a` (14 B, 1 event) | 0.074 µs | 1.9 µs |
//! | `seq 1 200000` burst, 4 KB feeds (~88 events each) | 14 µs | 125 µs |
//! | the same, 32 KB feeds | 112–133 µs | 962–1014 µs |
//! | the committed tmux captures, 512 B feeds | 25–28 µs | 55–59 µs |
//!
//! Through UniFFI the burst parses at 32–34 MB/s — the Swift parser's own
//! speed — against 245–292 MB/s here: the serialisation would cost what the
//! port gained. UniFFI stays the plan for the core's API (`Core`, `Session`,
//! …), which is called per user action, not per chunk.
//!
//! (Measured at opt-level 3. Since NSH joined the library the `app` profile
//! optimises for size, `"z"`, which keeps the app within the size gate; the
//! parser gives up about a fifth of its speed for it: the workspace
//! manifest has the numbers.)

pub mod agents;
pub mod app;
pub mod borrow;
pub mod herdr;
pub mod home;
pub mod hostcare;
pub mod keys;
pub mod look;
pub mod session;
pub mod store;
pub mod tmux;
#[cfg(test)]
mod tmux_tests;

uniffi::setup_scaffolding!();

pub use moshpit_session;
pub use moshpit_store;
pub use moshpit_stream;
// Named, so it is linked (its C ABI with it).
pub use moshpit_term;
pub use moshpit_tmux;
// Named, so it is linked: an unused dependency would not be, and with it
// the screens' C ABI.
pub use moshpit_ui;
