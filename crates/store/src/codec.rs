//! How the stores' models become JSON and back, on top of
//! `moshpit_seal::foundation::json` (Foundation's parser and writers).
//!
//! `JSONEncoder` without `.sortedKeys` writes an object's members in the
//! order of a Swift `Dictionary`, which is seeded per process: the app's own
//! output differs from launch to launch. So no member order is "the app's";
//! Rust writes members sorted by code point (what `.sortedKeys` would give),
//! the same on every platform and every run. Everything else about the
//! output — numbers, escapes, dates, what is left out — is `JSONEncoder`'s.

use moshpit_seal::foundation::json::{self, Style, Value};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Compact, members sorted, `/` escaped.
pub const SORTED: Style = Style {
    pretty: false,
    sorted_keys: true,
    escape_slashes: true,
};

/// `JSONEncoder().encode(value)`, members sorted.
pub fn encode<T: Serialize + ?Sized>(value: &T) -> String {
    json::write_encoder(&json::to_value(&value), SORTED)
}

/// A tree as `JSONEncoder` writes it, members sorted.
pub fn write(value: &Value) -> String {
    json::write_encoder(value, SORTED)
}

/// Decode an already parsed entry with `JSONDecoder`'s rules.
pub fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    json::decode_value(value).map_err(|error| error.to_string())
}
