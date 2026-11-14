//! What Moshpit persists, and the rules for reading and writing it — shared
//! by iOS and Android (`docs/nsh/DESIGN.md` §3.2, §3.5).
//!
//! Every format is the one the app has written since 1.0.8, so nothing
//! migrates: on iOS the stores sit on `UserDefaults.standard` (through
//! [`host::Defaults`]) and the keychain ([`host::Keychain`]) exactly where
//! the Swift stores keep them.
//!
//! - [`connections`]: saved connections, with J1's rollback fields;
//! - [`ssh_keys`]: SSH key metadata — B32's two algorithm fields, B34's
//!   `bits`; the private half only ever as a keychain account name;
//! - [`known_hosts`]: the trusted host keys, the model `moshpit-ssh` decides
//!   from;
//! - [`push`]: push pairings (`moshpit-seal`'s file and model);
//! - [`pet`]: the companion pet's record and its egg's progress;
//! - [`shortcuts`]: the shortcut bar, its built-ins and their reconcile step;
//! - [`settings`]: every `AppSettings` key, type and default;
//! - [`themes`], [`app_themes`]: terminal themes and accent themes, the
//!   user's own among them; the icon catalog;
//! - [`readiness`]: the sealed-launch rule — a read made before the first
//!   unlock is shown, never written back;
//! - [`secrets`]: the keychain items the records name, each going with the
//!   last record naming it, and the orphans earlier builds left (B232);
//! - [`stored_list`]: round two's B8 rule — a list is read an entry at a
//!   time, and what can't be read is carried, not dropped.
//!
//! Where this port deliberately differs from the Swift code it is marked
//! `Fix (<id>)`.

pub mod app_themes;
pub mod codec;
pub mod connections;
#[doc(hidden)]
pub mod harness;
pub mod host;
pub mod known_hosts;
pub mod pet;
pub mod plist;
pub mod push;
pub mod readiness;
pub mod secrets;
pub mod session_memory;
pub mod settings;
pub mod shortcuts;
pub mod ssh_keys;
pub mod stored_list;
pub mod swift;
pub mod themes;

pub use host::{Defaults, Keychain, KeychainError, MemoryDefaults, MemoryKeychain};
pub use plist::Plist;
pub use readiness::Readiness;
