//! The app's screen models: what each NSH screen's host keeps and decides,
//! in one Rust copy that iOS (through `crates/ffi`) and Android (through
//! `crates/android`) both run.
//!
//! A screen (`moshpit-ui`) draws and tells; its host keeps the draft and
//! decides what an edit, a tap or a save does. Until W15d each platform had
//! its own host model: Swift on iOS (`ConnectionDraft`, `TerminalShortcut`,
//! `RemoteFolderModel`, …) and Rust on Android (`crates/android/src/app`).
//! They live here now, with the stricter of the two rules wherever they
//! differed, and the hosts keep only what is theirs: the keychain and the
//! secrets in it, the pickers, QuickLook, the share sheet, Photos, the
//! system's file importer.
//!
//! Nothing here does I/O or calls a platform API, so every rule runs in
//! `cargo test`. Secrets never enter it: of a password or a pasted key, a
//! model hears only whether one was given (round 40's rule; the host's own
//! fields hold the text).
//!
//! - [`home`]: Home's record — the saved connections and what each one's
//!   session has, read from the session layer's views and the agents
//!   monitor's clock.
//! - [`connection_form`]: Add Connection and Edit Connection — the draft,
//!   its ports, Save's rules and the credential a save points the
//!   connection at (B294, B298).
//! - [`shortcuts`]: Settings › Shortcuts and the shortcut editor — what the
//!   page shows of the store, what its taps may do, the editor's draft and
//!   Save (B233, B252, B285, B288).
//! - [`remote_paths`] (W6): what a host path is worth, the probe that asks
//!   the host about the paths a terminal shows, the link a confirmed path
//!   carries, and the exec forms of reading and changing files.
//! - [`remote_folder`]: one folder in the folder sheet — its listing,
//!   thumbnails, fetches, uploads, names and failures (B286).
//! - [`image_attachment`]: images picked to the text pasted at the prompt,
//!   and the session's numbered upload log.
//! - [`looks`]: Settings › Accent's and › Theme's editors.

pub mod connection_form;
pub mod home;
pub mod image_attachment;
pub mod looks;
pub mod remote_folder;
pub mod remote_paths;
pub mod shortcuts;
