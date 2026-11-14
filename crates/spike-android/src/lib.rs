//! Spike gate G3 (docs/nsh/DESIGN.md): Moshpit's Android app as an NSH Rust
//! root whose terminal is Termux's view, registered by the Java host as the
//! native component `moshpit/terminal` and fed by the pane's byte channel
//! (crates/stream) through JNI.
//!
//! Built for `aarch64-linux-android` this is the app's `libnsh.so`
//! (spike/android/build.sh): NSH's Android runtime, the root ([`ui`]), the
//! JNI glue ([`bridge`]) and a core that echoes and bursts ([`core`]).
//! Elsewhere only the pure parts are here, for `cargo test`.

pub mod burst;
pub mod echo;

#[cfg(target_os = "android")]
mod alog;
#[cfg(target_os = "android")]
mod bridge;
#[cfg(target_os = "android")]
mod core;
#[cfg(target_os = "android")]
mod ui;
