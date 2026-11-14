//! Moshpit's UI in NSH (`docs/nsh/DESIGN.md` §3.2, 界面): the screens moved
//! from SwiftUI one at a time, each behind its own switch in the iOS app
//! (§3.6), and later the same screens on Android and macOS.
//!
//! - [`screen`]: the screens by name, and the NSH root a surface runs for
//!   one, with what the host says before it runs ([`screen::Params`]: the
//!   app's language and theme).
//! - [`channel`]: one surface's messages with its host, in order: what the
//!   host posts to the screen (the data it owns, such as the log the Recent
//!   Log screen shows) and what the screen tells the host (go back).
//! - [`theme`], [`face`]: the app's tokens (`Moshpit/UI/DesignTokens.swift`:
//!   Ink, Face, Metrics) as an NSH theme, and its typography, which follows
//!   Dynamic Type through NSH's font scale.
//! - [`strings`]: the screens' text in the app's languages, generated from
//!   the app's string catalog (`ui/locales/strings.json`).
//! - [`nav_bar`]: the top bar of a page pushed in the app's navigation.
//! - [`recent_log`]: Settings › Recent Log.
//! - [`app_icon`], [`accent`]: Settings › App Icon and Accent.
//! - [`voice_engine`], [`voice_language`]: Settings › Voice Input ›
//!   Recognition and Language.
//! - [`settings`]: Settings, the root of its stack.
//! - [`feedback`]: Settings › Send Feedback.
//! - [`whisper_model`]: Settings › Voice Input › Model.
//! - [`theme_gallery`], [`theme_editor`]: Settings › Theme, and a custom
//!   theme's editor.
//! - [`accent_editor`]: a custom accent's editor (Settings › Accent).
//! - [`boundary`]: a screen's body behind an error boundary: a body that
//!   fails says so, tells the host and offers Retry (B366).
//! - [`host_log`]: where the screens' log goes: stderr, and the host's
//!   handler for warnings and errors.
//! - [`home`]: Home, the app's first screen: the connections and their
//!   sessions' trees.
//! - [`connection_form`]: Home › Add Connection and Edit Connection.
//! - [`ssh_keys`]: Settings › SSH Keys.
//! - [`key_picker`]: Add Connection › Select Key.
//! - [`add_key`]: SSH Keys › Add Key.
//! - [`shortcuts`]: Settings › Shortcuts.
//! - [`add_shortcut`]: Shortcuts › Add Shortcut and a shortcut's editor.
//! - [`host_setup`]: Settings › Set up this host.
//! - [`os`]: the platform a screen runs on, where its words or rows differ.
//! - [`ui`]: the app's `ui` layer (UI-DESIGN §4), on which the redesign
//!   rebuilds the screens: its tokens (R0), type and motion (R1), and its first components
//!   with their gallery (R2).
//! - [`terminal`]: a connection's terminal: the chrome NSH draws around the
//!   host's terminal view.
//! - `ios` (iOS only): the NSH view as a `UIView` for the Swift host, behind
//!   a C ABI (`include/moshpit_ui.h`): create a surface, register the host's
//!   native components, run a screen, post to it, hear it, destroy it.
//!
//! The crate is linked into the app through `crates/ffi`, the one Rust
//! static library; NSH runs the screens' async tasks on the core's runtime
//! (`moshpit-runtime`).

pub mod accent;
pub mod accent_editor;
pub mod add_key;
pub mod add_shortcut;
pub mod app_icon;
pub mod boundary;
pub mod channel;
pub mod connection_form;
pub mod face;
pub mod feedback;
pub mod home;
pub mod host_care;
pub mod host_log;
pub mod host_setup;
pub mod key_picker;
pub mod nav_bar;
pub mod os;
pub mod recent_log;
pub mod screen;
pub mod sessions;
pub mod settings;
pub mod shortcuts;
pub mod ssh_keys;
pub mod strings;
pub mod terminal;
pub mod theme;
pub mod theme_editor;
pub mod theme_gallery;
pub mod ui;
pub mod voice_engine;
pub mod voice_language;
pub mod whisper_model;

#[cfg(target_os = "ios")]
mod ios;
