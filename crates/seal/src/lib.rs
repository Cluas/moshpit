//! Moshpit's push envelope and everything the phone's side of a push needs,
//! for the Rust core: a port of the app's `MoshpitKit/Push` and the push half
//! of `PushService` without the parts that need a platform.
//!
//! - [`envelope`]: format v1 — AES-256-CBC + HMAC-SHA256, encrypt-then-MAC —
//!   sealing and opening, byte-compatible with the host's openssl pipeline
//!   (`moshpit-push.sh`), the Go relay's `sealbox` and Swift's
//!   `PushSealedBox` (`harness/interop.py` and `tests/interop.rs` hold all
//!   four to shared vectors).
//! - [`status`]: the sealed plaintext.
//! - [`pairing`], [`pairing_file`]: a paired host's secrets and the file
//!   that keeps them, read an entry at a time (round five's B23 rule).
//! - [`token`]: the relay's send token and the `/v1/mint` exchange.
//! - [`standing`]: the prompts standing per connection — the 0→1 edge that
//!   alone may ring.
//! - [`copy`], [`notification`]: the words on a card in the app's nine
//!   languages, and a status rendered into one.
//! - [`receive`]: the notification service's flow, for Android's FCM
//!   service; [`prefs`] and [`diagnostics`] are what it reads and leaves.
//! - [`service`]: the phone's mint decisions.
//! - [`foundation`]: the Foundation behaviour all of this has to match byte
//!   for byte (`JSONDecoder`, `JSONEncoder`, `JSONSerialization`, `Date`,
//!   `UUID`, Swift's `Character`).
//!
//! Where this port deliberately differs from the Swift code it is marked
//! `Fix (<id>)` in the source; `harness/divergences.json` lists each with
//! the Swift location, the symptom, the trigger and a minimal input.
//!
//! Not here: the iOS stores' App Group paths, file protection classes and
//! backup exclusion; registering for a device token; the HTTP calls to the
//! relay; posting the notification. Those belong to each platform.

pub mod closed;
pub mod copy;
pub mod diagnostics;
pub mod envelope;
pub mod finishes;
pub mod foundation;
#[doc(hidden)]
pub mod harness;
pub mod notification;
pub mod pairing;
pub mod pairing_file;
pub mod prefs;
pub mod random;
pub mod receive;
pub mod service;
pub mod standing;
pub mod status;
pub mod token;

mod store;

pub use copy::{InterruptionLevel, Language};
pub use finishes::PushFinishesFile;
pub use envelope::{Envelope, Failure, Keys, derive_keys, open, open_raw, seal};
pub use foundation::date::Date;
pub use foundation::uuid::Uuid;
pub use notification::{Card, Render, render};
pub use pairing::PushPairing;
pub use pairing_file::PushPairingFile;
pub use prefs::Prefs;
pub use receive::{Delivery, Received, receive};
pub use standing::PushStandingFile;
pub use status::Status;
