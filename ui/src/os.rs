//! The platform the screens run on, where what they say or show differs
//! (B258): iOS's words (the Secure Enclave, Face ID, the Keychain, the
//! Dynamic Island, the iPhone) are not Android's (the Android Keystore,
//! this device), and a row for what a platform doesn't have is left out
//! there (Android's launcher keeps the one icon; it has no Live Activities;
//! a key behind a biometric prompt isn't made there yet).
//!
//! A screen asks [`Os::current`] and says each platform's words through
//! `t!`, so both are in the string catalog and translated; its tests ask
//! for the other platform's by name.

/// A platform the screens run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    /// iOS, iPadOS and the Mac (Catalyst).
    Apple,
    Android,
}

impl Os {
    /// The platform this build is for.
    pub const fn current() -> Os {
        if cfg!(target_os = "android") {
            Os::Android
        } else {
            Os::Apple
        }
    }

    pub const fn is_android(self) -> bool {
        matches!(self, Os::Android)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_is_apple_off_android() {
        assert_eq!(Os::current().is_android(), cfg!(target_os = "android"));
        assert!(Os::Android.is_android());
        assert!(!Os::Apple.is_android());
    }
}
