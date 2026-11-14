//! Spike G1 (`docs/nsh/DESIGN.md`, the gate table): the byte channel
//! between a Rust core and SwiftTerm inside an NSH-hosted iOS app, on the
//! device.
//!
//! - [`core`]: one tokio runtime (NSH reuses it), the terminal's stream, a
//!   task echoing its input back, the burst producer.
//! - [`burst`]: the burst's deterministic terminal output and its hash.
//! - `app` and `ui` (iOS only): the NSH app view behind a C ABI
//!   (`include/moshpit_spike.h`), the root with the status line and the
//!   terminal. The Swift host links this static library; the terminal
//!   component talks to `moshpit_stream`'s C ABI directly.
//!
//! Spike G4 (the same app under Mac Catalyst) adds [`probe`]: what each
//! stream's core end took from the view (bytes, resizes), for the host to
//! check against what it typed and the window's size.
//!
//! Spike G5 (the iOS terminal screen) adds:
//!
//! - [`panes`]: the shell and the full-screen app its panes run.
//! - `screen` (iOS only): Moshpit's terminal screen with its chrome drawn by
//!   NSH over the host's terminal component, in an NSH navigation stack or
//!   alone (for a SwiftUI navigation stack to host).

pub mod burst;
pub mod core;
pub mod panes;
pub mod probe;

#[cfg(target_os = "ios")]
mod app;
#[cfg(target_os = "ios")]
mod screen;
#[cfg(target_os = "ios")]
mod ui;

// The byte channel's C ABI, linked into the app from here.
pub use moshpit_stream::ffi as stream_ffi;
// Spike T1: the shared terminal core's C ABI (moshpit_term.h), for
// CoreTermView (Spike/CoreFrameRenderer.swift).
pub use moshpit_term::ffi as term_ffi;
