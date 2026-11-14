//! The two switches a pushed notification must honour (`PushPrefs`).
//!
//! The app writes them where its notification service can read them (on iOS
//! the App Group's `UserDefaults` suite under these keys); the service only
//! reads. Never written means both on — what the settings themselves default
//! to.

/// The App Group `UserDefaults` key of "Show detail on lock screen".
pub const DETAIL_KEY: &str = "moshpit.push.showDetail";
/// The App Group `UserDefaults` key of "Alert sound".
pub const SOUND_KEY: &str = "moshpit.push.sound";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    /// Render what the agent is running or asking in the body. Off: the body
    /// still names where, never what.
    pub show_detail: bool,
    /// Whether a card that earned a sound may play one.
    pub sound: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            show_detail: true,
            sound: true,
        }
    }
}

impl Prefs {
    /// From what is stored under [`DETAIL_KEY`] and [`SOUND_KEY`]: an
    /// unwritten switch reads as on.
    pub fn from_stored(show_detail: Option<bool>, sound: Option<bool>) -> Self {
        Self {
            show_detail: show_detail.unwrap_or(true),
            sound: sound.unwrap_or(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PushPrefsTests`.
    #[test]
    fn an_unwritten_mirror_reads_as_both_switches_on() {
        assert_eq!(
            Prefs::from_stored(None, None),
            Prefs {
                show_detail: true,
                sound: true
            }
        );
        assert_eq!(Prefs::default(), Prefs::from_stored(None, None));
    }

    #[test]
    fn what_the_app_writes_is_what_the_service_reads() {
        assert_eq!(
            Prefs::from_stored(Some(false), Some(true)),
            Prefs {
                show_detail: false,
                sound: true
            }
        );
        assert_eq!(
            Prefs::from_stored(Some(true), Some(false)),
            Prefs {
                show_detail: true,
                sound: false
            }
        );
    }
}
