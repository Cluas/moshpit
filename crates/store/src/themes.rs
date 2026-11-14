//! Terminal colour schemes (`TerminalTheme`, `ThemeStore`): the built-ins,
//! and the user's own as one JSON array under
//! `moshpit.settings.customThemes`, read a theme at a time.
//!
//! A theme is stored flat and by name, so an exported theme is something a
//! person can read and edit and the importer takes palettes pasted from
//! other terminals: `{"id", "name", "background": "#0D1117", "foreground",
//! "cursor", "black", …, "white", "brightBlack", …}`. Colours are hex
//! strings ([`normalized_hex`]); the bright half of the palette is derived
//! unless a slot is overridden.

use std::sync::Arc;

use moshpit_seal::foundation::json::{self, Dialect, Object, Style, Value};
use moshpit_seal::foundation::text;
use moshpit_seal::foundation::uuid::Uuid;

use crate::codec;
use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;
use crate::stored_list::{self, Contents, Entry};
use crate::swift;

// MARK: - Hex colours

/// `Color.normalizedHex`: strip whitespace, `#` and `0x`, keep the hex
/// digits, expand `#RGB`, drop the alpha of `#RRGGBBAA`, upper case — or
/// black (`000000`) for anything that isn't six digits then.
///
/// Character by character, as Swift has it: a fullwidth digit is a hex digit
/// (and stays one), a `#` carrying a combining mark is no `#`.
pub fn normalized_hex(hex: &str) -> String {
    let mut s = swift::uppercased(text::trim_white_space(hex));
    if swift::has_prefix(&s, "#") {
        let first = swift::characters(&s).next().map_or(0, str::len);
        s.drain(..first);
    }
    if swift::has_prefix(&s, "0X") {
        let two: usize = swift::characters(&s).take(2).map(str::len).sum();
        s.drain(..two);
    }
    let mut digits: Vec<&str> = swift::characters(&s)
        .filter(|c| swift::is_hex_digit(c))
        .collect();
    if digits.len() == 3 {
        return digits.iter().map(|d| format!("{d}{d}")).collect();
    }
    if digits.len() == 8 {
        digits.truncate(6);
    }
    if digits.len() != 6 {
        return "000000".to_owned();
    }
    digits.concat()
}

/// `Color.rgbComponents(hex:)`: each channel 0–1. `Scanner.scanHexInt64`
/// reads the leading ASCII hex digits of the normalised form, so a
/// fullwidth digit ends the number there (and a theme whose colour is all
/// fullwidth digits is black).
pub fn rgb_components(hex: &str) -> (f64, f64, f64) {
    let cleaned = normalized_hex(hex);
    let digits: String = cleaned
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    let value = u64::from_str_radix(&digits, 16).unwrap_or(0);
    (
        ((value >> 16) & 0xFF) as f64 / 255.0,
        ((value >> 8) & 0xFF) as f64 / 255.0,
        (value & 0xFF) as f64 / 255.0,
    )
}

/// Swift's `min(a, b)`: `b < a ? b : a`.
fn swift_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Swift's `max(a, b)`: `b >= a ? b : a`.
fn swift_max(a: f64, b: f64) -> f64 {
    if b >= a { b } else { a }
}

/// `Color.hexString(r:g:b:)`: 0–1 components to bare upper-case hex.
pub fn hex_string(r: f64, g: f64, b: f64) -> String {
    let byte = |c: f64| (swift_max(0.0, swift_min(1.0, c)) * 255.0).round() as i64;
    format!("{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b))
}

/// `Color.blend(_:toward:by:)`: one colour a fraction of the way to another.
pub fn blend(hex: &str, target: &str, t: f64) -> String {
    let (r1, g1, b1) = rgb_components(hex);
    let (r2, g2, b2) = rgb_components(target);
    let k = swift_max(0.0, swift_min(1.0, t));
    hex_string(r1 + (r2 - r1) * k, g1 + (g2 - g1) * k, b1 + (b2 - b1) * k)
}

// MARK: - Terminal themes

/// The eight ANSI colours, in palette order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnsiSlot {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl AnsiSlot {
    pub const ALL: [Self; 8] = [
        Self::Black,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Magenta,
        Self::Cyan,
        Self::White,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The flat JSON key: `black` … `white`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
            Self::Cyan => "cyan",
            Self::White => "white",
        }
    }

    /// `brightBlack` … `brightWhite`.
    pub fn bright_key(self) -> String {
        let key = self.key();
        format!("bright{}{}", key[..1].to_uppercase(), &key[1..])
    }
}

/// A terminal colour scheme: background, foreground, cursor and the 16
/// ANSI colours (the bright eight derived unless overridden).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TerminalTheme {
    pub id: String,
    pub name: String,
    pub background_hex: String,
    pub foreground_hex: String,
    pub cursor_hex: String,
    /// ANSI 0–7: always eight once built by [`TerminalTheme::new`].
    pub ansi_hex: Vec<String>,
    /// ANSI 8–15, or `None` to derive them all; an empty entry derives that
    /// slot.
    pub bright_hex: Option<Vec<String>>,
    /// Shipped with the app: never stored, never edited in place.
    pub is_built_in: bool,
}

impl TerminalTheme {
    /// The normalising initializer: every colour through [`normalized_hex`],
    /// the base palette padded or cut to eight, the bright one too.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        name: &str,
        background: &str,
        foreground: &str,
        cursor: &str,
        ansi: &[&str],
        bright: Option<&[&str]>,
        is_built_in: bool,
    ) -> Self {
        let mut slots: Vec<String> = ansi.iter().map(|h| normalized_hex(h)).collect();
        while slots.len() < AnsiSlot::ALL.len() {
            slots.push("000000".to_owned());
        }
        slots.truncate(AnsiSlot::ALL.len());
        let bright_hex = bright.map(|list| {
            let mut normalized: Vec<String> = list
                .iter()
                .map(|h| {
                    if h.is_empty() {
                        String::new()
                    } else {
                        normalized_hex(h)
                    }
                })
                .collect();
            while normalized.len() < AnsiSlot::ALL.len() {
                normalized.push(String::new());
            }
            normalized.truncate(AnsiSlot::ALL.len());
            normalized
        });
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            background_hex: normalized_hex(background),
            foreground_hex: normalized_hex(foreground),
            cursor_hex: normalized_hex(cursor),
            ansi_hex: slots,
            bright_hex,
            is_built_in,
        }
    }

    /// One base colour (black past the end of a short palette).
    pub fn ansi(&self, slot: AnsiSlot) -> String {
        self.ansi_hex
            .get(slot.index())
            .cloned()
            .unwrap_or_else(|| "000000".to_owned())
    }

    /// One bright colour: the override when set, otherwise derived.
    pub fn bright(&self, slot: AnsiSlot) -> String {
        if let Some(value) = self
            .bright_hex
            .as_ref()
            .and_then(|overrides| overrides.get(slot.index()))
            .filter(|value| !value.is_empty())
        {
            return value.clone();
        }
        Self::derived_bright(&self.ansi(slot))
    }

    pub fn has_bright_override(&self, slot: AnsiSlot) -> bool {
        self.bright_hex
            .as_ref()
            .and_then(|overrides| overrides.get(slot.index()))
            .is_some_and(|value| !value.is_empty())
    }

    /// Set one base colour, normalised.
    pub fn set_ansi(&mut self, hex: &str, slot: AnsiSlot) {
        while self.ansi_hex.len() <= slot.index() {
            self.ansi_hex.push("000000".to_owned());
        }
        self.ansi_hex[slot.index()] = normalized_hex(hex);
    }

    /// Set or clear one bright override; a palette with none left is `None`.
    pub fn set_bright_override(&mut self, hex: Option<&str>, slot: AnsiSlot) {
        let mut overrides = self
            .bright_hex
            .clone()
            .unwrap_or_else(|| vec![String::new(); AnsiSlot::ALL.len()]);
        while overrides.len() < AnsiSlot::ALL.len() {
            overrides.push(String::new());
        }
        overrides[slot.index()] = hex.map(normalized_hex).unwrap_or_default();
        self.bright_hex = (!overrides.iter().all(String::is_empty)).then_some(overrides);
    }

    /// 32% of the way to white: keeps the hue, never darkens.
    pub fn derived_bright(hex: &str) -> String {
        let (r, g, b) = rgb_components(hex);
        let lift = |c: f64| c + (1.0 - c) * 0.32;
        hex_string(lift(r), lift(g), lift(b))
    }

    /// The flat encoding (`encode(to:)`); `isBuiltIn` is never written.
    pub fn to_json(&self) -> Value {
        let mut fields = Object::new();
        let hash = |hex: &str| Value::String(format!("#{hex}"));
        fields.insert("id", Value::String(self.id.clone()));
        fields.insert("name", Value::String(self.name.clone()));
        fields.insert("background", hash(&self.background_hex));
        fields.insert("foreground", hash(&self.foreground_hex));
        fields.insert("cursor", hash(&self.cursor_hex));
        for slot in AnsiSlot::ALL {
            fields.insert(slot.key(), hash(&self.ansi(slot)));
            if self.has_bright_override(slot) {
                fields.insert(slot.bright_key(), hash(&self.bright(slot)));
            }
        }
        Value::Object(fields)
    }

    /// The lenient decoding (`init(from:)`): any of the usual names for a
    /// colour, black or white for a missing one, a minted id for a theme
    /// pasted without one. Always custom.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let fields = value
            .as_object()
            .ok_or_else(|| "expected an object".to_owned())?;
        // `try? c.decode(String.self)`: only a non-empty string counts.
        let hex = |names: &[&str]| -> Option<String> {
            names.iter().find_map(|name| {
                fields
                    .get(name)
                    .and_then(Value::as_str)
                    .filter(|raw| !raw.is_empty())
                    .map(normalized_hex)
            })
        };
        let present_string = |key: &str| -> Result<Option<String>, String> {
            match fields.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(format!("{key}: expected a string")),
            }
        };
        let name = present_string("name")?.unwrap_or_else(|| "Imported".to_owned());
        let id = present_string("id")?.unwrap_or_else(mint_custom_id);
        let background = hex(&["background", "bg"]).unwrap_or_else(|| "000000".to_owned());
        let foreground = hex(&["foreground", "fg", "text"]).unwrap_or_else(|| "FFFFFF".to_owned());
        let cursor = hex(&["cursor", "cursorColor", "caret"]).unwrap_or_else(|| foreground.clone());
        let mut ansi = Vec::new();
        let mut bright = Vec::new();
        for slot in AnsiSlot::ALL {
            let index = slot.index();
            ansi.push(
                hex(&[slot.key(), &format!("ansi{index}")]).unwrap_or_else(|| "000000".to_owned()),
            );
            bright.push(
                hex(&[&slot.bright_key(), &format!("ansi{}", index + 8)]).unwrap_or_default(),
            );
        }
        let ansi: Vec<&str> = ansi.iter().map(String::as_str).collect();
        let bright: Vec<&str> = bright.iter().map(String::as_str).collect();
        let bright = (!bright.iter().all(|b| b.is_empty())).then_some(&bright[..]);
        Ok(Self::new(
            &id,
            &name,
            &background,
            &foreground,
            &cursor,
            &ansi,
            bright,
            false,
        ))
    }

    pub fn bioluminescent() -> Self {
        Self::new(
            "bioluminescent",
            "Bioluminescent",
            "0D1117",
            "C9D1D9",
            "97C459",
            &[
                "08080A", "F85149", "56D364", "EF9F27", "58A6FF", "C9B4F0", "58A6FF", "C9D1D9",
            ],
            None,
            true,
        )
    }

    pub fn dracula() -> Self {
        Self::new(
            "dracula",
            "Dracula",
            "282A36",
            "F8F8F2",
            "F8F8F2",
            &[
                "21222C", "FF5555", "50FA7B", "F1FA8C", "BD93F9", "FF79C6", "8BE9FD", "F8F8F2",
            ],
            None,
            true,
        )
    }

    pub fn nord() -> Self {
        Self::new(
            "nord",
            "Nord",
            "2E3440",
            "D8DEE9",
            "D8DEE9",
            &[
                "3B4252", "BF616A", "A3BE8C", "EBCB8B", "81A1C1", "B48EAD", "88C0D0", "E5E9F0",
            ],
            None,
            true,
        )
    }

    pub fn solarized_dark() -> Self {
        Self::new(
            "solarized-dark",
            "Solarized Dark",
            "002B36",
            "839496",
            "839496",
            &[
                "073642", "DC322F", "859900", "B58900", "268BD2", "D33682", "2AA198", "EEE8D5",
            ],
            None,
            true,
        )
    }

    pub fn monokai() -> Self {
        Self::new(
            "monokai",
            "Monokai",
            "272822",
            "F8F8F2",
            "F8F8F0",
            &[
                "272822", "F92672", "A6E22E", "F4BF75", "66D9EF", "AE81FF", "A1EFE4", "F8F8F2",
            ],
            None,
            true,
        )
    }

    pub fn tokyo_night() -> Self {
        Self::new(
            "tokyo-night",
            "Tokyo Night",
            "1A1B26",
            "C0CAF5",
            "C0CAF5",
            &[
                "15161E", "F7768E", "9ECE6A", "E0AF68", "7AA2F7", "BB9AF7", "7DCFFF", "A9B1D6",
            ],
            None,
            true,
        )
    }

    pub fn liquid_glass() -> Self {
        Self::new(
            "liquid-glass",
            "Liquid Glass",
            "0E1019",
            "E8EAF0",
            "5BD5F0",
            &[
                "0A0C13", "E68988", "7DD49E", "E4B656", "5BD5F0", "B289F5", "5BD5F0", "F8F8FB",
            ],
            None,
            true,
        )
    }

    /// GitHub Dark, the default.
    pub fn github_dark() -> Self {
        Self::new(
            "github-dark",
            "GitHub Dark",
            "050507",
            "C9D1D9",
            "5FE3D8",
            &[
                "484F58", "FF7B72", "3FB950", "D29922", "58A6FF", "BC8CFF", "39C5CF", "B1BAC4",
            ],
            None,
            true,
        )
    }

    /// The shipped themes, in the gallery's order.
    pub fn built_ins() -> Vec<Self> {
        vec![
            Self::github_dark(),
            Self::liquid_glass(),
            Self::bioluminescent(),
            Self::dracula(),
            Self::nord(),
            Self::solarized_dark(),
            Self::monokai(),
            Self::tokyo_night(),
        ]
    }

    /// What a stored id that resolves to nothing gets.
    pub fn fallback() -> Self {
        Self::github_dark()
    }
}

impl Entry for TerminalTheme {
    fn decode(value: &Value) -> Result<Self, String> {
        Self::from_json(value)
    }

    fn encode_list(items: &[Self]) -> String {
        codec::write(&Value::Array(items.iter().map(Self::to_json).collect()))
    }
}

/// Whether the stored list holds a theme this build reads that has no id
/// (missing or null) — one [`TerminalTheme::from_json`] mints an id for.
fn stored_without_id(stored: Option<&Plist>) -> bool {
    let Some(Value::Array(entries)) = stored
        .and_then(Plist::as_data)
        .and_then(stored_list::serialization_tree)
    else {
        return false;
    };
    entries.iter().any(|entry| {
        let unnamed = entry
            .as_object()
            .is_some_and(|fields| matches!(fields.get("id"), None | Some(Value::Null)));
        unnamed && TerminalTheme::from_json(entry).is_ok()
    })
}

/// `"<prefix>-" + UUID().uuidString.prefix(8).lowercased()`.
pub fn mint_id(prefix: &str) -> String {
    let uuid = Uuid::new_v4().to_string();
    format!("{prefix}-{}", uuid[..8].to_lowercase())
}

fn mint_custom_id() -> String {
    mint_id("custom")
}

/// `name.trimmingCharacters(in: .whitespacesAndNewlines)`, or `untitled`
/// when nothing is left, with " 2", " 3", … until no name in `taken` is
/// (canonically) the same.
pub fn unique_name(name: &str, untitled: &str, taken: &[&str]) -> String {
    let trimmed = text::trim_white_space(name);
    let base = if trimmed.is_empty() {
        untitled
    } else {
        trimmed
    };
    let is_taken = |candidate: &str| taken.iter().any(|t| swift::eq(t, candidate));
    if !is_taken(base) {
        return base.to_owned();
    }
    let mut suffix = 2;
    while is_taken(&format!("{base} {suffix}")) {
        suffix += 1;
    }
    format!("{base} {suffix}")
}

/// Why an import failed (`ThemeStore.ImportError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// Not theme JSON (or nothing at all).
    NotJson,
    /// An empty list.
    NoThemes,
}

/// The terminal themes: the built-ins and the user's own.
pub struct ThemeStore {
    defaults: Arc<dyn Defaults>,
    readiness: Readiness,
    custom_themes: Vec<TerminalTheme>,
    is_authoritative: bool,
    carried: Vec<Value>,
    /// The localized "Untitled" a nameless theme gets.
    untitled: String,
}

impl ThemeStore {
    pub const STORAGE_KEY: &'static str = "moshpit.settings.customThemes";

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

    /// The localized name a theme saved without one gets ("Untitled").
    pub fn set_untitled_name(&mut self, untitled: &str) {
        self.untitled = untitled.to_owned();
    }

    pub fn custom_themes(&self) -> &[TerminalTheme] {
        &self.custom_themes
    }

    pub fn is_authoritative(&self) -> bool {
        self.is_authoritative
    }

    /// Built-ins first, then the user's own.
    pub fn all_themes(&self) -> Vec<TerminalTheme> {
        let mut all = TerminalTheme::built_ins();
        all.extend(self.custom_themes.iter().cloned());
        all
    }

    /// The theme with this id, or the default for a deleted or unknown one.
    pub fn theme(&self, id: &str) -> TerminalTheme {
        self.all_themes()
            .into_iter()
            .find(|t| swift::eq(&t.id, id))
            .unwrap_or_else(TerminalTheme::fallback)
    }

    pub fn is_custom(&self, id: &str) -> bool {
        self.custom_themes.iter().any(|t| swift::eq(&t.id, id))
    }

    /// Insert or replace by id, with a name no other theme has. Built-ins
    /// are refused.
    pub fn save(&mut self, theme: TerminalTheme) {
        if theme.is_built_in {
            return;
        }
        self.reload_if_needed();
        let mut updated = theme;
        updated.name = self.unique_name(&updated.name, &updated.id);
        match self
            .custom_themes
            .iter()
            .position(|t| swift::eq(&t.id, &updated.id))
        {
            Some(index) => self.custom_themes[index] = updated,
            None => self.custom_themes.push(updated),
        }
        self.persist();
    }

    pub fn delete(&mut self, id: &str) {
        self.reload_if_needed();
        self.custom_themes.retain(|t| !swift::eq(&t.id, id));
        self.persist();
    }

    /// Drop every custom theme (the DEBUG reset seam).
    pub fn remove_all_custom(&mut self) {
        self.reload_if_needed();
        if self.custom_themes.is_empty() {
            return;
        }
        self.custom_themes.clear();
        self.persist();
    }

    /// An editable copy: a fresh id, custom, named "<name> Copy".
    pub fn duplicate(&self, theme: &TerminalTheme) -> TerminalTheme {
        let mut copy = theme.clone();
        copy.id = mint_custom_id();
        copy.is_built_in = false;
        copy.name = self.unique_name(&format!("{} Copy", theme.name), &copy.id);
        copy
    }

    /// A new theme starting from the default palette.
    pub fn make_draft(&self) -> TerminalTheme {
        let mut draft = TerminalTheme::fallback();
        draft.id = mint_custom_id();
        draft.is_built_in = false;
        draft.name = self.unique_name("My Theme", &draft.id);
        draft
    }

    /// One theme as pretty JSON with sorted keys, so a re-export diffs.
    pub fn export_json(theme: &TerminalTheme) -> String {
        json::write_encoder(&theme.to_json(), Style::PRETTY_SORTED)
    }

    /// Import one theme or an array of them, each under a fresh id, saved
    /// as custom themes; what was imported comes back.
    pub fn import_themes(&mut self, json_text: &str) -> Result<Vec<TerminalTheme>, ImportError> {
        if text::trim_white_space(json_text).is_empty() {
            return Err(ImportError::NotJson);
        }
        let parsed = json::parse(json_text.as_bytes(), Dialect::Decoder)
            .map_err(|_| ImportError::NotJson)?;
        let many = match &parsed {
            Value::Array(items) => items
                .iter()
                .map(TerminalTheme::from_json)
                .collect::<Result<Vec<_>, _>>()
                .ok(),
            _ => None,
        };
        let decoded = match many {
            Some(many) => many,
            None => vec![TerminalTheme::from_json(&parsed).map_err(|_| ImportError::NotJson)?],
        };
        if decoded.is_empty() {
            return Err(ImportError::NoThemes);
        }
        let mut imported = Vec::new();
        for mut theme in decoded {
            theme.id = mint_custom_id();
            theme.is_built_in = false;
            let fallback = theme.clone();
            self.save(theme);
            imported.push(self.custom_themes.last().cloned().unwrap_or(fallback));
        }
        Ok(imported)
    }

    pub fn reload_if_needed(&mut self) {
        if !self.is_authoritative {
            self.load();
        }
    }

    pub fn load(&mut self) {
        let defaults = self.defaults.as_ref();
        let readable = self.readiness.is_readable(defaults);
        let contents: Contents<TerminalTheme> = stored_list::read(defaults, Self::STORAGE_KEY);
        self.is_authoritative =
            readable && stored_list::secure(defaults, Self::STORAGE_KEY, &contents);
        // Decoded themes are custom by construction; a persisted `true`
        // would make one uneditable and undeletable.
        self.custom_themes = contents
            .elements
            .into_iter()
            .map(|mut theme| {
                theme.is_built_in = false;
                theme
            })
            .collect();
        self.carried = contents.carried;
        if readable {
            self.readiness.mark_readable(defaults);
        }
        // Fix (B160): a stored theme without an id is given one as it is
        // decoded, a fresh one on every read; the id the person selected
        // it by named nothing at the next launch. The one given now is
        // written, once the read can be trusted, so it is the theme's id
        // from then on — in every build.
        if self.is_authoritative && stored_without_id(contents.stored.as_ref()) {
            log::info!("{}: a theme stored without an id; its id written", Self::STORAGE_KEY);
            self.persist();
        }
    }

    fn unique_name(&self, name: &str, excluding_id: &str) -> String {
        let all = self.all_themes();
        let taken: Vec<&str> = all
            .iter()
            .filter(|t| !swift::eq(&t.id, excluding_id))
            .map(|t| t.name.as_str())
            .collect();
        unique_name(name, &self.untitled, &taken)
    }

    fn persist(&self) {
        if !self.is_authoritative {
            return;
        }
        if let Some(data) = stored_list::encode(&self.custom_themes, &self.carried) {
            self.defaults
                .set(Self::STORAGE_KEY, Some(Plist::Data(data)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;

    #[test]
    fn hex_normalisation() {
        for (input, expected) in [
            ("#0d1117", "0D1117"),
            ("0D1117", "0D1117"),
            ("0x0d1117", "0D1117"),
            (" #abc ", "AABBCC"),
            ("#0D1117FF", "0D1117"),
            ("#12", "000000"),
            ("zz", "000000"),
            ("", "000000"),
            (
                "#\u{FF21}\u{FF22}\u{FF23}",
                "\u{FF21}\u{FF21}\u{FF22}\u{FF22}\u{FF23}\u{FF23}",
            ),
            ("#\u{301}abc", "AABBCC"),
            ("ab-cd-ef", "ABCDEF"),
        ] {
            assert_eq!(normalized_hex(input), expected, "{input:?}");
        }
        assert_eq!(rgb_components("FF8000"), (1.0, 128.0 / 255.0, 0.0));
        assert_eq!(rgb_components("\u{FF21}BCDEF"), (0.0, 0.0, 0.0));
    }

    /// `AppThemeTests.blendStaysInRange`.
    #[test]
    fn blending_stays_in_range() {
        assert_eq!(blend("FF0000", "000000", 0.0), "FF0000");
        assert_eq!(blend("FF0000", "000000", 1.0), "000000");
        assert_eq!(blend("FF0000", "FFFFFF", 1.0), "FFFFFF");
        assert_eq!(blend("FF0000", "000000", 5.0), "000000");
        assert_eq!(blend("FF0000", "000000", -3.0), "FF0000");
    }

    #[test]
    fn a_theme_round_trips_through_its_flat_encoding() {
        let mut theme = TerminalTheme::dracula();
        theme.id = "custom-1".into();
        theme.is_built_in = false;
        theme.set_bright_override(Some("#123"), AnsiSlot::Red);
        let read = TerminalTheme::from_json(&theme.to_json()).unwrap();
        assert_eq!(read, theme);
        assert_eq!(read.bright(AnsiSlot::Red), "112233");
        assert!(!read.has_bright_override(AnsiSlot::Blue));
        theme.set_bright_override(None, AnsiSlot::Red);
        assert_eq!(theme.bright_hex, None);
    }

    #[test]
    fn a_pasted_palette_reads_with_any_of_the_usual_names() {
        let value = json::parse(
            br##"{"name":"Pasted","bg":"#101010","text":"eee","caret":"","ansi1":"#ff0000","brightGreen":"0x00ff00","ansi15":"fff"}"##,
            Dialect::Decoder,
        )
        .unwrap();
        let theme = TerminalTheme::from_json(&value).unwrap();
        assert!(theme.id.starts_with("custom-") && theme.id.len() == 15);
        assert_eq!(theme.background_hex, "101010");
        assert_eq!(theme.foreground_hex, "EEEEEE");
        assert_eq!(theme.cursor_hex, "EEEEEE");
        assert_eq!(theme.ansi(AnsiSlot::Red), "FF0000");
        assert_eq!(theme.ansi(AnsiSlot::Black), "000000");
        assert_eq!(theme.bright(AnsiSlot::Green), "00FF00");
        assert_eq!(theme.bright(AnsiSlot::White), "FFFFFF");
        assert_eq!(theme.bright_hex.as_ref().unwrap()[0], "");
        // A name that isn't a string is no theme.
        let bad = json::parse(br#"{"name":5}"#, Dialect::Decoder).unwrap();
        assert!(TerminalTheme::from_json(&bad).is_err());
    }

    #[test]
    fn export_is_pretty_and_sorted() {
        let text = ThemeStore::export_json(&TerminalTheme::nord());
        assert!(text.starts_with("{\n  \"background\" : \"#2E3440\",\n  \"black\" : \"#3B4252\","));
        assert!(text.ends_with("\n  \"yellow\" : \"#EBCB8B\"\n}"));
    }

    fn store() -> (Arc<MemoryDefaults>, ThemeStore) {
        let defaults = Arc::new(MemoryDefaults::new());
        let store = ThemeStore::new(defaults.clone(), Readiness::Always);
        (defaults, store)
    }

    /// B160: a theme stored without an id gets one once, written when the
    /// read can be trusted — the same id at every later launch, so a
    /// selection by it holds. A read before the first unlock writes nothing.
    #[test]
    fn a_theme_stored_without_an_id_keeps_the_one_it_is_given() {
        let stored = br##"[{"name":"Paper","background":"#FFFFFF","foreground":"#111111"},{"id":"custom-1","name":"Ink","background":"#000000"}]"##;
        let sealed = Arc::new(MemoryDefaults::with([(
            ThemeStore::STORAGE_KEY.to_owned(),
            Plist::Data(stored.to_vec()),
        )]));
        let unlocked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = unlocked.clone();
        let readiness = Readiness::custom(move || flag.load(std::sync::atomic::Ordering::SeqCst));
        let early = ThemeStore::new(sealed.clone(), readiness);
        assert_eq!(early.custom_themes().len(), 2);
        assert_eq!(
            sealed.get(ThemeStore::STORAGE_KEY),
            Some(Plist::Data(stored.to_vec())),
            "an untrusted read was written back"
        );

        let defaults = Arc::new(MemoryDefaults::with([(
            ThemeStore::STORAGE_KEY.to_owned(),
            Plist::Data(stored.to_vec()),
        )]));
        let first = ThemeStore::new(defaults.clone(), Readiness::Always);
        let paper = first.custom_themes()[0].id.clone();
        assert!(paper.starts_with("custom-"), "{paper}");
        assert_eq!(first.custom_themes()[1].id, "custom-1");
        for _ in 0..3 {
            let again = ThemeStore::new(defaults.clone(), Readiness::Always);
            assert_eq!(again.custom_themes()[0].id, paper, "a new id at the next launch");
            assert_eq!(again.theme(&paper).name, "Paper");
        }
    }

    #[test]
    fn saving_dedups_names_and_refuses_built_ins() {
        let (defaults, mut store) = store();
        store.save(TerminalTheme::dracula());
        assert!(store.custom_themes().is_empty());
        let mut draft = store.make_draft();
        assert_eq!(draft.name, "My Theme");
        draft.name = "  Dracula ".into();
        store.save(draft.clone());
        assert_eq!(store.custom_themes()[0].name, "Dracula 2");
        let copy = store.duplicate(&TerminalTheme::nord());
        assert_eq!(copy.name, "Nord Copy");
        store.save(copy);
        let mut untitled = store.make_draft();
        untitled.name = " \n".into();
        store.save(untitled);
        assert_eq!(store.custom_themes()[2].name, "Untitled");
        let reread = ThemeStore::new(defaults, Readiness::Always);
        assert_eq!(reread.custom_themes(), store.custom_themes());
        assert_eq!(reread.theme("nope").id, "github-dark");
    }

    #[test]
    fn import_takes_one_or_many_and_mints_ids() {
        let (_, mut store) = store();
        let one = ThemeStore::export_json(&TerminalTheme::monokai());
        let imported = store.import_themes(&one).unwrap();
        assert_eq!(imported.len(), 1);
        assert_ne!(imported[0].id, "monokai");
        assert_eq!(imported[0].name, "Monokai 2");
        assert_eq!(store.import_themes("[]"), Err(ImportError::NoThemes));
        assert_eq!(store.import_themes("  "), Err(ImportError::NotJson));
        assert_eq!(store.import_themes("[1]"), Err(ImportError::NotJson));
        assert_eq!(store.import_themes("not json"), Err(ImportError::NotJson));
        let many = format!(
            "[{},{}]",
            codec::write(&TerminalTheme::nord().to_json()),
            codec::write(&TerminalTheme::dracula().to_json())
        );
        assert_eq!(store.import_themes(&many).unwrap().len(), 2);
        assert_eq!(store.custom_themes().len(), 3);
    }
}
