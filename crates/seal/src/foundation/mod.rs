//! The Foundation behaviour the app's push and install code leans on,
//! reproduced byte for byte where the bytes reach a file, a host or a
//! notification: how `JSONDecoder` reads and `JSONEncoder` /
//! `JSONSerialization` write JSON, what a Swift `Date` and `UUID` look like
//! on the wire, how Swift prints a `Double`, which characters Foundation
//! trims and how Swift's `Character` sees a string.
//!
//! Everything here was measured against the Foundation of macOS 27 (Swift
//! 6.4) by `harness/swift/main.swift`; each module says what it reproduces
//! and where it deliberately does not. `moshpit-install` uses the same
//! pieces for the host's manifest and the agents' configs.

pub mod base64;
pub mod collation;
pub mod date;
pub mod decode;
pub mod double;
pub mod hex;
pub mod json;
pub mod text;
pub mod uuid;

mod numeric_table;
