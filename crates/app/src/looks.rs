//! Settings › Accent's and › Theme's editors, the host's model: the draft
//! the page edits and what its changes do. One copy for iOS
//! (`AccentEditorNSHPage.swift`'s `AccentEditorModel`,
//! `ThemeEditorNSHPage.swift`'s `ThemeEditorModel`, before this crate) and Android
//! (`crates/android/src/app/looks.rs`). The colour wells are the hosts'
//! (UIKit's `UIColorWell`, Android's picker); saving goes through the
//! stores.
//!
//! Merged rules: a change the page tells in a shape the editor does not
//! read changes nothing (Android cleared the name on a `name` event with
//! no text); a colour is stored normalized (`6C6BEF`); turning the bright
//! rows off drops every bright override; the bright rows show from the
//! start when the theme overrides one.

use moshpit_store::app_themes::AppTheme;
use moshpit_store::themes::{AnsiSlot, TerminalTheme, normalized_hex};
use moshpit_ui::accent_editor::{self as accent_screen, AccentDraft};
use moshpit_ui::theme::parse_hex;
use moshpit_ui::theme_editor::{self as theme_screen, Draft, EditorState, Slot};
use nsh_core::color::Rgba;
use serde_json::Value;

/// What a message from an editor's page did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// A change, made to the draft.
    Changed,
    /// Nothing it reads: nothing changed.
    Unknown,
    /// Something beyond a change.
    Intent(EditorIntent),
}

/// What an editor's page asked beyond a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorIntent {
    /// Save the draft, and close.
    Save,
    /// Close without saving (Cancel, or the back button).
    Cancel,
    /// Copy the theme's JSON (the theme editor's).
    CopyJson,
}

const BLACK: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// A stored hex colour (`6C6BEF`, `#6C6BEF`) as a colour; black when it is
/// none.
pub fn rgba(hex: &str) -> Rgba {
    parse_hex(&format!("#{}", hex.trim_start_matches('#'))).unwrap_or(BLACK)
}

/// The intent of `ev`, if it is one.
fn intent(ev: &str) -> Handled {
    match ev {
        "save" => Handled::Intent(EditorIntent::Save),
        "cancel" | "back" => Handled::Intent(EditorIntent::Cancel),
        _ => Handled::Unknown,
    }
}

/// A custom accent's editor.
#[derive(Debug, Clone, PartialEq)]
pub struct AccentEditor {
    pub id: String,
    pub name: String,
    /// Normalized (`6C6BEF`).
    pub accent_hex: String,
}

impl AccentEditor {
    pub fn new(accent: &AppTheme) -> Self {
        AccentEditor {
            id: accent.id.clone(),
            name: accent.name.clone(),
            accent_hex: normalized_hex(&accent.accent_hex),
        }
    }

    /// The draft as an accent, its pressed shade and washes derived.
    pub fn accent(&self) -> AppTheme {
        AppTheme::custom(&self.id, &self.name, &self.accent_hex)
    }

    /// What the page told (`{"ev": …}`): a change, or an intent. One it
    /// does not read changes nothing.
    pub fn handle(&mut self, message: &Value) -> Handled {
        let Some(ev) = message["ev"].as_str() else {
            return Handled::Unknown;
        };
        let value = message["value"].as_str();
        match (ev, value) {
            ("name", Some(name)) => self.name = name.to_owned(),
            ("color", Some(hex)) => self.accent_hex = normalized_hex(hex),
            ("name" | "color", None) => return Handled::Unknown,
            _ => return intent(ev),
        }
        Handled::Changed
    }

    /// `{"op": "accent", …}`.
    pub fn message(&self) -> String {
        accent_screen::Message::Accent(AccentDraft {
            name: self.name.clone(),
            accent: rgba(&self.accent_hex),
        })
        .to_json()
    }
}

/// A custom terminal theme's editor.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeEditor {
    pub draft: TerminalTheme,
    /// The bright rows show.
    pub override_bright: bool,
}

impl ThemeEditor {
    /// The editor on `theme`: the bright rows show when it overrides one.
    pub fn new(theme: TerminalTheme) -> Self {
        let override_bright = AnsiSlot::ALL.iter().any(|s| theme.has_bright_override(*s));
        ThemeEditor {
            draft: theme,
            override_bright,
        }
    }

    /// What the page told: a change, or an intent. One it does not read
    /// changes nothing.
    pub fn handle(&mut self, message: &Value) -> Handled {
        let Some(ev) = message["ev"].as_str() else {
            return Handled::Unknown;
        };
        match ev {
            "name" => match message["value"].as_str() {
                Some(name) => self.draft.name = name.to_owned(),
                None => return Handled::Unknown,
            },
            "color" => {
                let (Some(slot), Some(hex)) = (message["slot"].as_str(), message["value"].as_str())
                else {
                    return Handled::Unknown;
                };
                if !set_color(&mut self.draft, slot, hex) {
                    return Handled::Unknown;
                }
            }
            "override-bright" => {
                let Some(on) = message["value"].as_bool() else {
                    return Handled::Unknown;
                };
                self.override_bright = on;
                if !on {
                    for slot in AnsiSlot::ALL {
                        self.draft.set_bright_override(None, slot);
                    }
                }
            }
            "copy-json" => return Handled::Intent(EditorIntent::CopyJson),
            _ => return intent(ev),
        }
        Handled::Changed
    }

    /// `{"op": "theme", …}`: the theme, its bright overrides (none where
    /// derived), and whether the bright rows show.
    pub fn message(&self) -> String {
        let theme = &self.draft;
        let mut ansi = [BLACK; 8];
        let mut bright = [None; 8];
        for (i, slot) in AnsiSlot::ALL.iter().enumerate() {
            ansi[i] = rgba(&theme.ansi(*slot));
            bright[i] = theme
                .has_bright_override(*slot)
                .then(|| rgba(&theme.bright(*slot)));
        }
        theme_screen::Message::Theme(EditorState {
            theme: Draft {
                name: theme.name.clone(),
                background: rgba(&theme.background_hex),
                foreground: rgba(&theme.foreground_hex),
                cursor: rgba(&theme.cursor_hex),
                ansi,
                bright,
            },
            override_bright: self.override_bright,
        })
        .to_json()
    }
}

/// Colour `slot` (the theme's JSON key: `background`, `red`, `brightRed`)
/// of `theme` set to `hex`; false for a slot it does not know.
pub fn set_color(theme: &mut TerminalTheme, slot: &str, hex: &str) -> bool {
    match Slot::named(slot) {
        Some(Slot::Background) => theme.background_hex = normalized_hex(hex),
        Some(Slot::Foreground) => theme.foreground_hex = normalized_hex(hex),
        Some(Slot::Cursor) => theme.cursor_hex = normalized_hex(hex),
        Some(Slot::Ansi(i)) => theme.set_ansi(hex, AnsiSlot::ALL[i]),
        Some(Slot::Bright(i)) => theme.set_bright_override(Some(hex), AnsiSlot::ALL[i]),
        None => return false,
    }
    true
}

#[cfg(test)]
mod tests;
