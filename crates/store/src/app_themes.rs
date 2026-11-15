//! The app's accent themes (`AppTheme`, `AppThemeCatalog`, `AppThemeStore`)
//! and its icons (`AppIconCatalog`).
//!
//! A custom accent theme stores only its accent — one JSON array of
//! `{"id", "name", "accent"}` under `moshpit.settings.customAppThemes` —
//! because everything else is derived from it; storing the derived values
//! would freeze an old derivation into themes already saved. The icon is a
//! separate choice: a platform can only switch between icons bundled when
//! it was built.

use std::sync::Arc;

use moshpit_seal::foundation::json::Value;
use serde::{Deserialize, Serialize};

use crate::codec;
use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;
use crate::stored_list::{self, Contents, Entry};
use crate::swift;
use crate::themes::{self, blend, normalized_hex};

/// An app-chrome scheme: the accent, its pressed state and two faint
/// background tints, as bare hex.
#[derive(Debug, Clone)]
pub struct AppTheme {
    pub id: String,
    pub name: String,
    /// Normalised (`normalizedHex`): what a custom theme stores.
    pub accent_hex: String,
    pub accent_pressed_hex: String,
    pub screen_bg_hex: String,
    pub terminal_bg_hex: String,
    /// False for the user's own, which can be edited and deleted.
    pub is_built_in: bool,
}

/// Equal when the id and the accent are: a custom theme keeps its id while
/// its accent is edited.
impl PartialEq for AppTheme {
    fn eq(&self, other: &Self) -> bool {
        swift::eq(&self.id, &other.id) && swift::eq(&self.accent_hex, &other.accent_hex)
    }
}

impl AppTheme {
    fn built_in(
        id: &str,
        name: &str,
        accent: &str,
        pressed: &str,
        screen: &str,
        terminal: &str,
    ) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            accent_hex: normalized_hex(accent),
            accent_pressed_hex: pressed.to_owned(),
            screen_bg_hex: screen.to_owned(),
            terminal_bg_hex: terminal.to_owned(),
            is_built_in: true,
        }
    }

    /// A theme from one accent (`AppTheme.custom`): pressed is 18% toward
    /// black, the backgrounds near-black with a wash of the accent.
    pub fn custom(id: &str, name: &str, accent_hex: &str) -> Self {
        let accent = normalized_hex(accent_hex);
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            accent_pressed_hex: blend(&accent, "000000", 0.18),
            screen_bg_hex: blend(&accent, "000000", 0.955),
            terminal_bg_hex: blend(&accent, "000000", 0.982),
            accent_hex: accent,
            is_built_in: false,
        }
    }
}

/// The shipped accent themes.
pub mod catalog {
    use super::AppTheme;

    pub fn signal_room() -> AppTheme {
        AppTheme::built_in(
            "signal-room",
            "Signal Room",
            "6C6BEF",
            "5652D6",
            "06070A",
            "030405",
        )
    }

    pub fn moshpit_classic() -> AppTheme {
        AppTheme::built_in(
            "moshpit-classic",
            "Teal Line",
            "53DCC9",
            "3EBFA9",
            "05090A",
            "020505",
        )
    }

    pub fn terminal_green() -> AppTheme {
        AppTheme::built_in(
            "terminal-green",
            "Terminal Green",
            "2FA871",
            "25865A",
            "060B08",
            "030503",
        )
    }

    pub fn amber_console() -> AppTheme {
        AppTheme::built_in(
            "amber-console",
            "Amber Console",
            "C98A2E",
            "A66F1F",
            "0A0806",
            "050403",
        )
    }

    pub fn built_ins() -> Vec<AppTheme> {
        vec![
            signal_room(),
            moshpit_classic(),
            terminal_green(),
            amber_console(),
        ]
    }
}

/// One bundled app icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppIconOption {
    /// The id the setting stores; `default` is the primary icon.
    pub id: &'static str,
    pub name: &'static str,
    /// What iOS's `setAlternateIconName` wants; `None` for the primary.
    pub alternate_name: Option<&'static str>,
    pub preview_asset: &'static str,
}

impl AppIconOption {
    pub fn is_primary(&self) -> bool {
        self.alternate_name.is_none()
    }
}

/// The icon gallery (`AppIconCatalog`).
pub mod icons {
    use super::AppIconOption;

    pub const ALL: [AppIconOption; 22] = [
        AppIconOption {
            id: "default",
            name: "Moshpit",
            alternate_name: None,
            preview_asset: "IconPreview-Moshpit",
        },
        // The pet peeking over a pane, a species each and its shiny
        // (B668): the hatched pet offers its own; then the three panes that
        // were the primary until the egg took over.
        AppIconOption {
            id: "tun",
            name: "Tun",
            alternate_name: Some("AppIcon-Tun"),
            preview_asset: "IconPreview-Tun",
        },
        AppIconOption {
            id: "tun-shiny",
            name: "Shiny Tun",
            alternate_name: Some("AppIcon-TunShiny"),
            preview_asset: "IconPreview-TunShiny",
        },
        AppIconOption {
            id: "coo",
            name: "Coo",
            alternate_name: Some("AppIcon-Coo"),
            preview_asset: "IconPreview-Coo",
        },
        AppIconOption {
            id: "coo-shiny",
            name: "Shiny Coo",
            alternate_name: Some("AppIcon-CooShiny"),
            preview_asset: "IconPreview-CooShiny",
        },
        AppIconOption {
            id: "dig",
            name: "Dig",
            alternate_name: Some("AppIcon-Dig"),
            preview_asset: "IconPreview-Dig",
        },
        AppIconOption {
            id: "dig-shiny",
            name: "Shiny Dig",
            alternate_name: Some("AppIcon-DigShiny"),
            preview_asset: "IconPreview-DigShiny",
        },
        AppIconOption {
            id: "hex",
            name: "Hex",
            alternate_name: Some("AppIcon-Hex"),
            preview_asset: "IconPreview-Hex",
        },
        AppIconOption {
            id: "hex-shiny",
            name: "Shiny Hex",
            alternate_name: Some("AppIcon-HexShiny"),
            preview_asset: "IconPreview-HexShiny",
        },
        AppIconOption {
            id: "ether",
            name: "Ether",
            alternate_name: Some("AppIcon-Ether"),
            preview_asset: "IconPreview-Ether",
        },
        AppIconOption {
            id: "ether-shiny",
            name: "Shiny Ether",
            alternate_name: Some("AppIcon-EtherShiny"),
            preview_asset: "IconPreview-EtherShiny",
        },
        AppIconOption {
            id: "moth",
            name: "Lumen",
            alternate_name: Some("AppIcon-Moth"),
            preview_asset: "IconPreview-Moth",
        },
        AppIconOption {
            id: "moth-shiny",
            name: "Shiny Lumen",
            alternate_name: Some("AppIcon-MothShiny"),
            preview_asset: "IconPreview-MothShiny",
        },
        AppIconOption {
            id: "classic",
            name: "Classic",
            alternate_name: Some("AppIcon-Classic"),
            preview_asset: "IconPreview-Classic",
        },
        AppIconOption {
            id: "pit",
            name: "Pit",
            alternate_name: Some("AppIcon-Pit"),
            preview_asset: "IconPreview-Pit",
        },
        AppIconOption {
            id: "island",
            name: "Island",
            alternate_name: Some("AppIcon-Island"),
            preview_asset: "IconPreview-Island",
        },
        AppIconOption {
            id: "wq",
            name: ":wq",
            alternate_name: Some("AppIcon-Wq"),
            preview_asset: "IconPreview-Wq",
        },
        AppIconOption {
            id: "prefix",
            name: "Prefix",
            alternate_name: Some("AppIcon-Prefix"),
            preview_asset: "IconPreview-Prefix",
        },
        AppIconOption {
            id: "localhost",
            name: "Localhost",
            alternate_name: Some("AppIcon-Localhost"),
            preview_asset: "IconPreview-Localhost",
        },
        AppIconOption {
            id: "prompt",
            name: "Prompt",
            alternate_name: Some("AppIcon-Prompt"),
            preview_asset: "IconPreview-Prompt",
        },
        AppIconOption {
            id: "cursor",
            name: "Cursor",
            alternate_name: Some("AppIcon-Cursor"),
            preview_asset: "IconPreview-Cursor",
        },
        AppIconOption {
            id: "hail",
            name: "Hail",
            alternate_name: Some("AppIcon-Hail"),
            preview_asset: "IconPreview-Hail",
        },
    ];

    pub const PRIMARY: AppIconOption = ALL[0];

    /// The option with this id, or the primary for any other (B13: no
    /// stored id, or one from a theme, reads as the primary).
    pub fn option(id: &str) -> AppIconOption {
        ALL.into_iter()
            .find(|option| crate::swift::eq(option.id, id))
            .unwrap_or(PRIMARY)
    }
}

/// One custom accent as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Stored {
    id: String,
    name: String,
    accent: String,
}

impl Entry for Stored {
    fn decode(value: &Value) -> Result<Self, String> {
        codec::decode(value)
    }

    fn encode_list(items: &[Self]) -> String {
        codec::encode(items)
    }
}

/// The user's accent themes (`AppThemeStore`).
pub struct AppThemeStore {
    defaults: Arc<dyn Defaults>,
    readiness: Readiness,
    custom_themes: Vec<AppTheme>,
    is_authoritative: bool,
    carried: Vec<Value>,
    untitled: String,
}

impl AppThemeStore {
    pub const STORAGE_KEY: &'static str = "moshpit.settings.customAppThemes";

    pub fn new(defaults: Arc<dyn Defaults>, readiness: Readiness) -> Self {
        let mut store = Self {
            defaults,
            readiness,
            custom_themes: Vec::new(),
            is_authoritative: false,
            carried: Vec::new(),
            untitled: "Untitled".to_owned(),
        };
        store.load();
        store
    }

    /// The localized name an accent saved without one gets ("Untitled").
    pub fn set_untitled_name(&mut self, untitled: &str) {
        self.untitled = untitled.to_owned();
    }

    pub fn custom_themes(&self) -> &[AppTheme] {
        &self.custom_themes
    }

    pub fn is_authoritative(&self) -> bool {
        self.is_authoritative
    }

    /// Built-ins, then the user's own.
    pub fn all(&self) -> Vec<AppTheme> {
        let mut all = catalog::built_ins();
        all.extend(self.custom_themes.iter().cloned());
        all
    }

    /// The theme with this id, or Signal Room.
    pub fn theme(&self, id: &str) -> AppTheme {
        self.all()
            .into_iter()
            .find(|t| swift::eq(&t.id, id))
            .unwrap_or_else(catalog::signal_room)
    }

    pub fn is_custom(&self, id: &str) -> bool {
        self.custom_themes.iter().any(|t| swift::eq(&t.id, id))
    }

    /// Insert or replace by id; the derived values are rebuilt from the
    /// accent. Built-ins are refused.
    pub fn save(&mut self, theme: &AppTheme) {
        if theme.is_built_in {
            return;
        }
        self.reload_if_needed();
        let rebuilt = AppTheme::custom(
            &theme.id,
            &self.unique_name(&theme.name, &theme.id),
            &theme.accent_hex,
        );
        match self
            .custom_themes
            .iter()
            .position(|t| swift::eq(&t.id, &rebuilt.id))
        {
            Some(index) => self.custom_themes[index] = rebuilt,
            None => self.custom_themes.push(rebuilt),
        }
        self.persist();
    }

    pub fn delete(&mut self, id: &str) {
        self.reload_if_needed();
        self.custom_themes.retain(|t| !swift::eq(&t.id, id));
        self.persist();
    }

    pub fn remove_all_custom(&mut self) {
        self.reload_if_needed();
        if self.custom_themes.is_empty() {
            return;
        }
        self.custom_themes.clear();
        self.persist();
    }

    /// A draft from the current accent, under a fresh id.
    pub fn make_draft(&self, based_on: &AppTheme) -> AppTheme {
        AppTheme::custom(
            &themes::mint_id("app"),
            &self.unique_name("My Accent", ""),
            &based_on.accent_hex,
        )
    }

    pub fn reload_if_needed(&mut self) {
        if !self.is_authoritative {
            self.load();
        }
    }

    pub fn load(&mut self) {
        let defaults = self.defaults.as_ref();
        let readable = self.readiness.is_readable(defaults);
        let contents: Contents<Stored> = stored_list::read(defaults, Self::STORAGE_KEY);
        self.is_authoritative =
            readable && stored_list::secure(defaults, Self::STORAGE_KEY, &contents);
        self.custom_themes = contents
            .elements
            .iter()
            .map(|s| AppTheme::custom(&s.id, &s.name, &s.accent))
            .collect();
        self.carried = contents.carried;
        if readable {
            self.readiness.mark_readable(defaults);
        }
    }

    fn unique_name(&self, name: &str, excluding_id: &str) -> String {
        let mut themes = catalog::built_ins();
        themes.extend(self.custom_themes.iter().cloned());
        let taken: Vec<&str> = themes
            .iter()
            .filter(|t| !swift::eq(&t.id, excluding_id))
            .map(|t| t.name.as_str())
            .collect();
        themes::unique_name(name, &self.untitled, &taken)
    }

    fn persist(&self) {
        if !self.is_authoritative {
            return;
        }
        let stored: Vec<Stored> = self
            .custom_themes
            .iter()
            .map(|t| Stored {
                id: t.id.clone(),
                name: t.name.clone(),
                accent: t.accent_hex.clone(),
            })
            .collect();
        if let Some(data) = stored_list::encode(&stored, &self.carried) {
            self.defaults
                .set(Self::STORAGE_KEY, Some(Plist::Data(data)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;

    /// `AppThemeTests`.
    #[test]
    fn a_custom_accent_derives_its_pressed_state_and_background_wash() {
        let theme = AppTheme::custom("app-1", "Mine", "#ff8800");
        assert_eq!(theme.accent_hex, "FF8800");
        assert!(!theme.is_built_in);
        assert_ne!(theme.accent_pressed_hex, "FF8800");
        assert!(themes::rgb_components(&theme.accent_pressed_hex).0 < 1.0);
        for hex in [&theme.screen_bg_hex, &theme.terminal_bg_hex] {
            let (r, g, b) = themes::rgb_components(hex);
            assert!(r < 0.09 && g < 0.09 && b < 0.09);
        }
        assert_eq!(theme, AppTheme::custom("app-1", "Renamed", "FF8800"));
        assert_ne!(theme, AppTheme::custom("app-1", "Mine", "FF8801"));
    }

    #[test]
    fn saving_stores_the_accent_and_refuses_built_ins() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mut store = AppThemeStore::new(defaults.clone(), Readiness::Always);
        store.save(&catalog::signal_room());
        assert!(store.custom_themes().is_empty());
        store.save(&AppTheme::custom("app-1", "Signal Room", "3366ff"));
        assert_eq!(store.custom_themes()[0].name, "Signal Room 2");
        store.save(&AppTheme::custom("app-1", "Signal Room", "3366ff"));
        assert_eq!(store.custom_themes().len(), 1);
        let reloaded = AppThemeStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(reloaded.custom_themes()[0].accent_hex, "3366FF");
        assert_eq!(
            defaults.get(AppThemeStore::STORAGE_KEY),
            Some(Plist::Data(
                br#"[{"accent":"3366FF","id":"app-1","name":"Signal Room 2"}]"#.to_vec()
            ))
        );
        assert_eq!(reloaded.theme("does-not-exist").id, "signal-room");
        let draft = reloaded.make_draft(&catalog::amber_console());
        assert!(draft.id.starts_with("app-") && draft.id.len() == 12);
        assert_eq!(draft.name, "My Accent");
    }

    #[test]
    fn an_unknown_icon_id_resolves_to_the_primary_icon() {
        assert_eq!(icons::option("nope").id, "default");
        assert_eq!(icons::option("hail").alternate_name, Some("AppIcon-Hail"));
        assert_eq!(icons::ALL.iter().filter(|o| o.is_primary()).count(), 1);
    }
}
