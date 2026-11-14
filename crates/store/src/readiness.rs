//! Whether a read of the preferences can be believed right now
//! (`DefaultsReadiness`, the sealed-launch rule).
//!
//! iOS launches the app with nobody watching — prewarming, a push, a Live
//! Activity intent. Before the first unlock after a reboot the preferences
//! file is still sealed, and every store that loads then reads NOTHING. A
//! store that believed it showed the first-run card over a phone full of
//! hosts, and the shortcut store seeded and wrote its built-ins over the
//! user's custom chips. Android has the same window: credential-encrypted
//! storage is unreadable until the user unlocks after boot.
//!
//! Two signals answer the question: the system's (protected data is
//! available: the device is unlocked) and ours — a sentinel written into
//! the same preferences on the first trusted load, so that a later read
//! that finds it found the file, unlocked or not. Absent both, a store
//! shows what it read, never writes over it, and reads again when the app
//! comes forward (`reload_if_needed` on every store).

use std::fmt;
use std::sync::Arc;

use crate::host::Defaults;
use crate::plist::{self, Plist};

/// The sentinel's key (`DefaultsReadiness.sentinelKey`).
pub const SENTINEL_KEY: &str = "moshpit.defaults.ready";

/// Where a store hears whether the preferences are readable.
#[derive(Clone)]
pub enum Readiness {
    /// The device's actual state, asked of the host at read time
    /// (`DefaultsReadiness.system`).
    System,
    /// Always readable: tests, and a store handed a scratch domain
    /// (`DefaultsReadiness.always`).
    Always,
    /// Asked of this function instead of the host (the tests' locked phone).
    Custom(Arc<dyn Fn() -> bool + Send + Sync>),
}

impl fmt::Debug for Readiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => f.write_str("System"),
            Self::Always => f.write_str("Always"),
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}

impl Readiness {
    /// A readiness that asks `available` whether protected data is.
    pub fn custom(available: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self::Custom(Arc::new(available))
    }

    fn protected_data_available(&self, defaults: &dyn Defaults) -> bool {
        match self {
            Self::System => defaults.protected_data_available(),
            Self::Always => true,
            Self::Custom(available) => available(),
        }
    }

    /// `isReadable`: the sentinel is there, or the system says protected
    /// data is available.
    pub fn is_readable(&self, defaults: &dyn Defaults) -> bool {
        plist::bool_value(defaults.get(SENTINEL_KEY).as_ref())
            || self.protected_data_available(defaults)
    }

    /// `markReadable`: record that a trusted load happened, so the next
    /// locked launch can tell "the file was there" from "the file was
    /// sealed". Written once.
    pub fn mark_readable(&self, defaults: &dyn Defaults) {
        if !plist::bool_value(defaults.get(SENTINEL_KEY).as_ref()) {
            defaults.set(SENTINEL_KEY, Some(Plist::Bool(true)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `DefaultsReadinessTests`.
    #[test]
    fn a_sealed_launch_is_not_readable_until_the_system_or_the_sentinel_says_so() {
        let defaults = MemoryDefaults::new();
        let unlocked = Arc::new(AtomicBool::new(false));
        let readiness = {
            let unlocked = unlocked.clone();
            Readiness::custom(move || unlocked.load(Ordering::SeqCst))
        };
        assert!(!readiness.is_readable(&defaults));

        unlocked.store(true, Ordering::SeqCst);
        assert!(readiness.is_readable(&defaults));
        readiness.mark_readable(&defaults);
        assert_eq!(defaults.get(SENTINEL_KEY), Some(Plist::Bool(true)));

        // Locked again, after a first unlock: the file was there.
        unlocked.store(false, Ordering::SeqCst);
        assert!(readiness.is_readable(&defaults));
    }

    #[test]
    fn the_sentinel_is_written_once() {
        let defaults = MemoryDefaults::new();
        defaults.set(SENTINEL_KEY, Some(Plist::String("YES".into())));
        Readiness::Always.mark_readable(&defaults);
        // Already true by `bool(forKey:)`'s rules: left as it is.
        assert_eq!(
            defaults.get(SENTINEL_KEY),
            Some(Plist::String("YES".into()))
        );
        defaults.set(SENTINEL_KEY, Some(Plist::Int(0)));
        Readiness::Always.mark_readable(&defaults);
        assert_eq!(defaults.get(SENTINEL_KEY), Some(Plist::Bool(true)));
    }

    #[test]
    fn system_asks_the_host() {
        let defaults = MemoryDefaults::new();
        assert!(Readiness::System.is_readable(&defaults));
        defaults.set_protected_data_available(false);
        assert!(!Readiness::System.is_readable(&defaults));
        assert!(Readiness::Always.is_readable(&defaults));
    }
}
