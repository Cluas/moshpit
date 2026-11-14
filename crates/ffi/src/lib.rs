//! The one Rust static library the iOS app links (`libmoshpit_ffi.a`).
//!
//! An app can link only one Rust static library: two would each carry the
//! Rust runtime and collide at link time (`docs/nsh/DESIGN.md` §3.2). So
//! every core crate the app uses is linked through here, re-exported, and
//! whatever C ABI the app calls is defined or re-exported here too:
//!
//! - [`abi`]: the version of the library's hand-written C ABI
//!   (`mp_ffi_abi_version`, `include/moshpit_ffi.h`, imported into Swift as
//!   the `MoshpitFFI` module with the other C headers below).
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
//! - [`moshpit_pet`]: the companion pet, through UniFFI ([`pet`]): its
//!   record and rules, the screens' `{"op": "pet"}` message and the Live
//!   Activity's sprites; the seed, the clock and iCloud are the app's.
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

pub mod abi;
pub mod agents;
pub mod app;
pub mod borrow;
pub mod herdr;
pub mod home;
pub mod hostcare;
pub mod keys;
pub mod look;
pub mod pet;
pub mod session;
pub mod store;

uniffi::setup_scaffolding!();

pub use moshpit_session;
pub use moshpit_store;
pub use moshpit_stream;
// Named, so it is linked (its C ABI with it).
pub use moshpit_term;
// The session core's tmux, named for the hosts that read its types.
pub use moshpit_tmux;
// Named, so it is linked: an unused dependency would not be, and with it
// the screens' C ABI.
pub use moshpit_ui;
