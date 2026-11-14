//! Push pairings: one per paired connection, in `push-pairings.json` in a
//! directory the notification service can read (iOS: the App Group; Android:
//! the app's private files). The model and the file are `moshpit-seal`'s —
//! one copy, with round five's B23 rule: read an entry at a time, carry what
//! can't be read, copy a file that isn't a list aside before writing over
//! it, never write over a file that can't be read.

pub use moshpit_seal::pairing::{PushPairing, normalise_relay_url};
pub use moshpit_seal::pairing_file::{PushPairingFile, newest_first};
