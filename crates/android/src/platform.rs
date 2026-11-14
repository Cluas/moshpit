//! What the Java host gives the app (`docs/nsh/DESIGN.md` §3.2, 平台模块):
//! its preferences and keychain, the Android Keystore's hardware keys, the
//! page stack the screens are pushed on, and the system's clipboard,
//! sharing, notifications and dialogs.
//!
//! [`Platform`] is the whole of it, one trait so the controller
//! ([`crate::app`]) runs the same against the JNI host (`android::host`)
//! and against [`testing::FakePlatform`] in `cargo test`. Every call
//! returns at once: what shows something posts to the host's main thread,
//! what asks the person answers later through `Core` (a dialog's token).
//!
//! The store's traits sit on top: [`PlatformDefaults`] keeps the property
//! list values in `SharedPreferences` strings ([`crate::prefs`]),
//! [`PlatformKeychain`] keeps secrets wrapped by an AES key of the Android
//! Keystore (the host's `Secrets`).

use std::sync::Arc;

use moshpit_store::{Defaults, Keychain, KeychainError, Plist};

use crate::prefs;

/// A notification the app posts (the host's notification channels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The channel: `agents` (an agent needs the person) or `sessions` (a
    /// session's state).
    pub channel: String,
    /// One per connection and pane: a newer one replaces it.
    pub tag: String,
    pub title: String,
    pub body: String,
    /// Where a tap lands: `moshpit://connection/<id>?pane=<pane>`.
    pub link: String,
}

/// A question the host asks with a system dialog; the answer comes back
/// through `Core.answer(token, …)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// OK / Cancel; `destructive` paints OK red.
    Confirm {
        token: u64,
        title: String,
        message: String,
        ok: String,
        destructive: bool,
    },
    /// A text field, prefilled; `multiline` for a paste of several lines
    /// (a theme's JSON).
    Prompt {
        token: u64,
        title: String,
        text: String,
        ok: String,
        multiline: bool,
    },
}

/// Everything the controller asks of the host.
pub trait Platform: Send + Sync {
    // --- Preferences (`SharedPreferences`), as strings (crate::prefs) ---
    fn pref_get(&self, key: &str) -> Option<String>;
    fn pref_set(&self, key: &str, value: Option<&str>);
    /// The person has unlocked since boot (`UserManager.isUserUnlocked`).
    fn user_unlocked(&self) -> bool;

    // --- The keychain: secrets wrapped by a Keystore AES key ---
    fn secret_save(&self, account: &str, secret: &[u8], biometry: bool) -> Result<(), String>;
    /// `Ok(None)`: no item.
    fn secret_load(&self, account: &str) -> Result<Option<Vec<u8>>, String>;
    fn secret_delete(&self, account: &str) -> Result<(), String>;
    fn secret_exists(&self, account: &str) -> bool;

    // --- Hardware keys: EC P-256 in the Keystore, never leaving it ---
    /// Makes key `alias`; its public half as X.509 SubjectPublicKeyInfo DER.
    fn hardware_generate(&self, alias: &str) -> Result<Vec<u8>, String>;
    /// `data` signed with SHA256withECDSA by key `alias`: ASN.1 DER.
    fn hardware_sign(&self, alias: &str, data: &[u8]) -> Result<Vec<u8>, String>;
    fn hardware_delete(&self, alias: &str);

    // --- The page stack ---
    /// Shows a new page on top, which runs surface `surface` (the host
    /// makes an NSH view and runs it through `Core.prepare`).
    fn push(&self, surface: u64, animated: bool);
    /// Takes page `surface` away (wherever it is in the stack).
    fn pop(&self, surface: u64, animated: bool);
    /// The person backed out of the first page: leave the app.
    fn finish(&self);

    // --- The system ---
    fn copy_text(&self, text: &str);
    fn share_text(&self, text: &str);
    fn open_url(&self, url: &str);
    fn toast(&self, text: &str);
    fn notify(&self, notification: &Notification);
    fn ask(&self, ask: &Ask);
    /// A pane's terminal is done for good: the host drops its view and
    /// emulator (the scrollback with them).
    fn release_stream(&self, stream: u64);

    // --- The terminal page's slot ---
    /// The terminal slot of page `surface` shows stream `stream`'s view (0:
    /// none yet).
    fn show_stream(&self, surface: u64, stream: u64);
    /// Every terminal's look: background, foreground, cursor and ANSI 0–15
    /// (`#RRGGBB`), and the text size in points.
    fn terminal_look(&self, colors: &[String], font_size: f64);
    /// Raise (`true`) or put away the keyboard of the terminal on screen.
    fn keyboard(&self, show: bool);
    /// Arm the sticky ctrl: the next key the terminal types is a control
    /// key (the host tells `Core.ctrlUsed` once it was).
    fn set_ctrl(&self, armed: bool);
    /// Paste the clipboard into the terminal on screen.
    fn paste(&self);

    // --- Files ---
    /// The system's document picker (the Storage Access Framework): the
    /// file picked comes back through `Core.picked(token, bytes)`, null
    /// when the person picked none or it could not be read.
    fn pick_file(&self, token: u64);
    /// The private key's field (`pem`) or its passphrase's on the page on
    /// top shows `text` (a key read from a file): the host's own field, whose
    /// text goes to the core and never to a screen.
    fn fill_key_field(&self, kind: &str, text: &str);
}

/// The store's preferences over the host's.
pub struct PlatformDefaults(pub Arc<dyn Platform>);

impl Defaults for PlatformDefaults {
    fn get(&self, key: &str) -> Option<Plist> {
        self.0.pref_get(key).as_deref().and_then(prefs::decode)
    }

    fn set(&self, key: &str, value: Option<Plist>) {
        let stored = value.as_ref().map(prefs::encode);
        self.0.pref_set(key, stored.as_deref());
    }

    fn protected_data_available(&self) -> bool {
        self.0.user_unlocked()
    }
}

/// The store's keychain over the host's.
pub struct PlatformKeychain(pub Arc<dyn Platform>);

impl Keychain for PlatformKeychain {
    fn save(
        &self,
        account: &str,
        secret: &[u8],
        require_biometry: bool,
    ) -> Result<(), KeychainError> {
        self.0
            .secret_save(account, secret, require_biometry)
            .map_err(KeychainError::Other)
    }

    fn load(&self, account: &str, _prompt: &str) -> Result<Vec<u8>, KeychainError> {
        match self.0.secret_load(account) {
            Ok(Some(secret)) => Ok(secret),
            Ok(None) => Err(KeychainError::NotFound),
            Err(why) => Err(KeychainError::Other(why)),
        }
    }

    fn delete(&self, account: &str) -> Result<(), KeychainError> {
        self.0.secret_delete(account).map_err(KeychainError::Other)
    }

    fn exists(&self, account: &str) -> bool {
        self.0.secret_exists(account)
    }
}

pub mod testing {
    //! A platform in memory that records what the controller asked of it.

    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    /// The P-256 generator point as a SubjectPublicKeyInfo: a hardware
    /// key's public half, as the Keystore hands it over.
    pub const P256_SPKI: [u8; 91] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04, 0x6b, 0x17, 0xd1,
        0xf2, 0xe1, 0x2c, 0x42, 0x47, 0xf8, 0xbc, 0xe6, 0xe5, 0x63, 0xa4, 0x40, 0xf2, 0x77, 0x03,
        0x7d, 0x81, 0x2d, 0xeb, 0x33, 0xa0, 0xf4, 0xa1, 0x39, 0x45, 0xd8, 0x98, 0xc2, 0x96, 0x4f,
        0xe3, 0x42, 0xe2, 0xfe, 0x1a, 0x7f, 0x9b, 0x8e, 0xe7, 0xeb, 0x4a, 0x7c, 0x0f, 0x9e, 0x16,
        0x2b, 0xce, 0x33, 0x57, 0x6b, 0x31, 0x5e, 0xce, 0xcb, 0xb6, 0x40, 0x68, 0x37, 0xbf, 0x51,
        0xf5,
    ];

    /// One thing the controller asked the host to do.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Call {
        Push(u64),
        Pop(u64),
        Finish,
        Copy(String),
        Share(String),
        OpenUrl(String),
        Toast(String),
        Notify(Notification),
        Ask(Ask),
        Release(u64),
        Show(u64, u64),
        Look(usize, u64),
        Keyboard(bool),
        Ctrl(bool),
        Paste,
        PickFile(u64),
        FillKeyField(String, usize),
    }

    #[derive(Default)]
    pub struct FakePlatform {
        pub prefs: Mutex<BTreeMap<String, String>>,
        pub secrets: Mutex<BTreeMap<String, Vec<u8>>>,
        pub hardware: Mutex<BTreeMap<String, Vec<u8>>>,
        pub calls: Mutex<Vec<Call>>,
    }

    impl FakePlatform {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// What was asked since the last call, and forget it.
        pub fn take_calls(&self) -> Vec<Call> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }

        fn record(&self, call: Call) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl Platform for FakePlatform {
        fn pref_get(&self, key: &str) -> Option<String> {
            self.prefs.lock().unwrap().get(key).cloned()
        }
        fn pref_set(&self, key: &str, value: Option<&str>) {
            let mut prefs = self.prefs.lock().unwrap();
            match value {
                Some(v) => prefs.insert(key.to_owned(), v.to_owned()),
                None => prefs.remove(key),
            };
        }
        fn user_unlocked(&self) -> bool {
            true
        }
        fn secret_save(&self, account: &str, secret: &[u8], _: bool) -> Result<(), String> {
            self.secrets
                .lock()
                .unwrap()
                .insert(account.to_owned(), secret.to_vec());
            Ok(())
        }
        fn secret_load(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.secrets.lock().unwrap().get(account).cloned())
        }
        fn secret_delete(&self, account: &str) -> Result<(), String> {
            self.secrets.lock().unwrap().remove(account);
            Ok(())
        }
        fn secret_exists(&self, account: &str) -> bool {
            self.secrets.lock().unwrap().contains_key(account)
        }
        fn hardware_generate(&self, alias: &str) -> Result<Vec<u8>, String> {
            // A fixed P-256 SubjectPublicKeyInfo (the generator point).
            let spki = P256_SPKI.to_vec();
            self.hardware
                .lock()
                .unwrap()
                .insert(alias.to_owned(), spki.clone());
            Ok(spki)
        }
        fn hardware_sign(&self, alias: &str, _: &[u8]) -> Result<Vec<u8>, String> {
            if self.hardware.lock().unwrap().contains_key(alias) {
                Err("the fake platform signs nothing".to_owned())
            } else {
                Err(format!("no key {alias}"))
            }
        }
        fn hardware_delete(&self, alias: &str) {
            self.hardware.lock().unwrap().remove(alias);
        }
        fn push(&self, surface: u64, _: bool) {
            self.record(Call::Push(surface));
        }
        fn pop(&self, surface: u64, _: bool) {
            self.record(Call::Pop(surface));
        }
        fn finish(&self) {
            self.record(Call::Finish);
        }
        fn copy_text(&self, text: &str) {
            self.record(Call::Copy(text.to_owned()));
        }
        fn share_text(&self, text: &str) {
            self.record(Call::Share(text.to_owned()));
        }
        fn open_url(&self, url: &str) {
            self.record(Call::OpenUrl(url.to_owned()));
        }
        fn toast(&self, text: &str) {
            self.record(Call::Toast(text.to_owned()));
        }
        fn notify(&self, notification: &Notification) {
            self.record(Call::Notify(notification.clone()));
        }
        fn ask(&self, ask: &Ask) {
            self.record(Call::Ask(ask.clone()));
        }
        fn release_stream(&self, stream: u64) {
            self.record(Call::Release(stream));
        }
        fn show_stream(&self, surface: u64, stream: u64) {
            self.record(Call::Show(surface, stream));
        }
        fn keyboard(&self, show: bool) {
            self.record(Call::Keyboard(show));
        }
        fn terminal_look(&self, colors: &[String], font_size: f64) {
            self.record(Call::Look(colors.len(), font_size as u64));
        }
        fn set_ctrl(&self, armed: bool) {
            self.record(Call::Ctrl(armed));
        }
        fn paste(&self) {
            self.record(Call::Paste);
        }
        fn pick_file(&self, token: u64) {
            self.record(Call::PickFile(token));
        }
        fn fill_key_field(&self, kind: &str, text: &str) {
            // The length only: a key's text is not kept in the record.
            self.record(Call::FillKeyField(kind.to_owned(), text.len()));
        }
    }
}
