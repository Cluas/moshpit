//! Moshpit on Android, its Rust half (`docs/nsh/DESIGN.md` §3.2): the one
//! library the APK carries, `lib/arm64-v8a/libnsh.so` — NSH's Android
//! runtime and the shared screens (`moshpit-ui`), the core (`moshpit-store`,
//! `moshpit-session`), the controller that hosts the screens, and the JNI
//! the Java host (`android/app/src/main/java`) calls.
//!
//! - [`app`]: the controller — the stores, the sessions, the page stack and
//!   each screen's host side (what the Swift pages do on iOS). Platform-
//!   neutral: `cargo test` runs it on the Mac against a fake platform.
//! - [`platform`]: what the Java host gives it ([`platform::Platform`]):
//!   `SharedPreferences`, the Keystore-wrapped keychain, the Keystore's
//!   hardware keys, the page stack, the system's dialogs and intents.
//! - [`prefs`]: the property list values the store keeps, as
//!   `SharedPreferences` strings.
//! - `android` (Android only): the JNI — NSH's entry (`Native.runRust`),
//!   `Core`'s entry points, the byte channel's hot path, the Java platform.
//!
//! # Why hand-written JNI, not UniFFI
//!
//! UniFFI writes Kotlin bindings (and Swift); it has no Java backend, and
//! this machine can't build Kotlin (no JDK 17+, so no Kotlin compiler, no
//! Gradle). More to the point, the controller lives in Rust, so what
//! crosses is small: about twenty calls, most of them a string or two (the
//! screens' JSON already is the control path). The byte channel's calls are
//! the hot path and want a pinned `byte[]`, which a generated layer would
//! copy through a buffer anyway (crates/ffi measured 0.074 µs against
//! 1.9 µs a call on iOS). So: plain JNI with the `jni` crate NSH re-exports,
//! one Java class per side (`Core`, `ByteChannel` in, `Platform` out).

pub mod app;
pub mod platform;
pub mod prefs;

#[cfg(target_os = "android")]
pub mod android;
