//! The shortcut bar (`TerminalShortcut`, `ShortcutStore`): one JSON array
//! under `moshpit.shortcuts.v1`, in toolbar order.
//!
//! This store needed the B8 rule most: a set that would not decode in one
//! piece read as "nothing stored", and nothing stored is what seeds the
//! built-ins and WRITES them — over a sealed file (a launch before the first
//! unlock) or over a set with one chip kind the build didn't know, that
//! erased every custom chip. Now the set is read a shortcut at a time, the
//! built-ins are written only over a read that was trusted and complete,
//! and a built-in a newer build added reaches an older set through the
//! summary-keyed reconcile step — which counts the carried entries too, so a
//! built-in stored in a shape this build can't read isn't added twice.
//!
//! Fix (files-downgrade): `kind` is what every build reads, so it holds a
//! kind 1.0.8 decodes — the real one goes in `shortcutKind`, as B32 did for a
//! key's algorithm. The files chip (newer than 1.0.8) was stored as
//! `"kind":"files"` (`TerminalShortcut.swift:57`, the builtin at :566), and
//! 1.0.8 decodes the set in one piece: it read every set the current app
//! saves as nothing, seeded its built-ins and saved them over every custom
//! chip — a downgrade wiped the bar. Here the chip is stored as an inert key
//! combo in `kind` (no key, no payload: a tap in 1.0.8 sends nothing) and as
//! `files` in `shortcutKind`; a set an older build saved back without that
//! field still names the built-in by its summary.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use moshpit_seal::foundation::json::Value;
use moshpit_seal::foundation::uuid::Uuid;
use serde::{Deserialize, Serialize};

use crate::codec;
use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;
use crate::stored_list::{self, Contents, Entry};
use crate::swift;

/// What a chip does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutKind {
    KeyCombo,
    Text,
    Command,
    /// The drag joystick (one built-in).
    Dpad,
    /// Four tappable arrow zones (one built-in).
    Arrows,
    /// Pastes the clipboard (resolved by the caller).
    Paste,
    /// Drags the local scrollback (never sent).
    Scroll,
    /// Arms sticky Ctrl.
    Ctrl,
    /// Starts and stops voice input.
    Mic,
    /// Attaches an image.
    Image,
    /// Opens the folder sheet at the pane's directory. Newer than 1.0.8,
    /// which can't decode it.
    Files,
}

impl ShortcutKind {
    pub const ALL: [Self; 11] = [
        Self::KeyCombo,
        Self::Text,
        Self::Command,
        Self::Dpad,
        Self::Arrows,
        Self::Paste,
        Self::Scroll,
        Self::Ctrl,
        Self::Mic,
        Self::Image,
        Self::Files,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::KeyCombo => "keyCombo",
            Self::Text => "text",
            Self::Command => "command",
            Self::Dpad => "dpad",
            Self::Arrows => "arrows",
            Self::Paste => "paste",
            Self::Scroll => "scroll",
            Self::Ctrl => "ctrl",
            Self::Mic => "mic",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.raw_value() == raw)
    }

    /// What a stored shortcut says in `kind`: a kind every build since
    /// 1.0.8 decodes. A kind newer than that is an inert key combo there.
    pub fn older_builds_name(self) -> Self {
        match self {
            Self::Files => Self::KeyCombo,
            kind => kind,
        }
    }
}

/// The summary that names the files built-in for good.
const FILES_SUMMARY: &str = "Browse files";

/// A modifier, ordered as macOS writes them: ⌃⌥⇧⌘.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutModifier {
    Ctrl,
    Alt,
    Shift,
    Cmd,
}

impl ShortcutModifier {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Ctrl => "⌃",
            Self::Alt => "⌥",
            Self::Shift => "⇧",
            Self::Cmd => "⌘",
        }
    }
}

/// One entry of the shortcut bar and library.
#[derive(Debug, Clone)]
pub struct TerminalShortcut {
    pub id: Uuid,
    pub kind: ShortcutKind,
    /// A set: order means nothing.
    pub modifiers: BTreeSet<ShortcutModifier>,
    /// The main key of a key combo: one character or a named key (`esc`).
    pub key: String,
    /// What a text or command chip types; a key combo's escape-sequence
    /// override. `\r` `\n` `\t` `\e` are unescaped when sent.
    pub payload: String,
    pub append_return: bool,
    pub repeat_on_hold: bool,
    /// At most six characters, on the chip.
    pub chip_label: String,
    /// The editor's description — and a built-in's identity, for good.
    pub summary: String,
    /// `gray`, `accent`, `mosh` or `amber`.
    pub color_id: String,
    /// A set (canonically, as Swift's `Set<String>`): empty is every host,
    /// otherwise only connections with these names.
    pub scope_hosts: Vec<String>,
    /// Only inside a multiplexer (tmux or herdr; the name is historical).
    pub only_in_tmux: bool,
    pub is_builtin: bool,
    /// Whether it takes a toolbar slot.
    pub in_toolbar: bool,
}

impl PartialEq for TerminalShortcut {
    fn eq(&self, other: &Self) -> bool {
        let hosts = |s: &Self| -> BTreeSet<String> {
            s.scope_hosts.iter().map(|h| swift::canonical(h)).collect()
        };
        self.id == other.id
            && self.kind == other.kind
            && self.modifiers == other.modifiers
            && swift::eq(&self.key, &other.key)
            && swift::eq(&self.payload, &other.payload)
            && self.append_return == other.append_return
            && self.repeat_on_hold == other.repeat_on_hold
            && swift::eq(&self.chip_label, &other.chip_label)
            && swift::eq(&self.summary, &other.summary)
            && swift::eq(&self.color_id, &other.color_id)
            && hosts(self) == hosts(other)
            && self.only_in_tmux == other.only_in_tmux
            && self.is_builtin == other.is_builtin
            && self.in_toolbar == other.in_toolbar
    }
}

/// The stored shape: every field required, as the synthesized `Codable`
/// has it, and the real kind beside `kind` (Fix (files-downgrade)).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    id: Uuid,
    /// The kind every build reads.
    kind: ShortcutKind,
    /// The real kind; read on its own (any value at all may be there).
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    shortcut_kind: Option<ShortcutKind>,
    modifiers: Vec<ShortcutModifier>,
    key: String,
    payload: String,
    append_return: bool,
    repeat_on_hold: bool,
    chip_label: String,
    summary: String,
    color_id: String,
    scope_hosts: Vec<String>,
    only_in_tmux: bool,
    is_builtin: bool,
    in_toolbar: bool,
}

impl TerminalShortcut {
    /// A blank shortcut: a key combo, gray, in the toolbar.
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            kind: ShortcutKind::KeyCombo,
            modifiers: BTreeSet::new(),
            key: String::new(),
            payload: String::new(),
            append_return: false,
            repeat_on_hold: false,
            chip_label: String::new(),
            summary: String::new(),
            color_id: "gray".to_owned(),
            scope_hosts: Vec::new(),
            only_in_tmux: false,
            is_builtin: false,
            in_toolbar: true,
        }
    }

    pub fn is_custom_styled(&self) -> bool {
        !self.is_builtin || self.color_id != "gray"
    }

    /// Set the host scope, a set: canonically equal names count once.
    pub fn set_scope_hosts(&mut self, hosts: impl IntoIterator<Item = String>) {
        self.scope_hosts = swift::dedup(hosts);
    }

    fn from_wire(wire: Wire, real_kind: Option<ShortcutKind>) -> Self {
        // A set an older build saved back has no real kind: its files
        // built-in is the inert key combo it was given, named by its summary.
        let recovered = wire.kind == ShortcutKind::KeyCombo
            && wire.is_builtin
            && wire.key.is_empty()
            && wire.payload.is_empty()
            && swift::eq(&wire.summary, FILES_SUMMARY);
        let kind = real_kind.unwrap_or(if recovered {
            ShortcutKind::Files
        } else {
            wire.kind
        });
        Self {
            id: wire.id,
            kind,
            modifiers: wire.modifiers.into_iter().collect(),
            key: wire.key,
            payload: wire.payload,
            append_return: wire.append_return,
            repeat_on_hold: wire.repeat_on_hold,
            chip_label: wire.chip_label,
            summary: wire.summary,
            color_id: wire.color_id,
            scope_hosts: swift::dedup(wire.scope_hosts),
            only_in_tmux: wire.only_in_tmux,
            is_builtin: wire.is_builtin,
            in_toolbar: wire.in_toolbar,
        }
    }

    fn to_wire(&self) -> Wire {
        Wire {
            id: self.id,
            kind: self.kind.older_builds_name(),
            shortcut_kind: Some(self.kind),
            modifiers: self.modifiers.iter().copied().collect(),
            key: self.key.clone(),
            payload: self.payload.clone(),
            append_return: self.append_return,
            repeat_on_hold: self.repeat_on_hold,
            chip_label: self.chip_label.clone(),
            summary: self.summary.clone(),
            color_id: self.color_id.clone(),
            scope_hosts: self.scope_hosts.clone(),
            only_in_tmux: self.only_in_tmux,
            is_builtin: self.is_builtin,
            in_toolbar: self.in_toolbar,
        }
    }

    /// The shortcut as `JSONEncoder` writes it.
    pub fn to_json(&self) -> String {
        codec::encode(&self.to_wire())
    }

    /// One shortcut as `JSONDecoder` reads it.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let wire = codec::decode::<Wire>(value)?;
        // `try? decodeIfPresent(String.self)`, then a raw value this build
        // knows: anything else falls back to `kind`.
        let real_kind = value
            .as_object()
            .and_then(|fields| fields.get("shortcutKind"))
            .and_then(Value::as_str)
            .and_then(ShortcutKind::from_raw);
        Ok(Self::from_wire(wire, real_kind))
    }

    /// The bytes a tap sends; `None` for the chips the caller resolves (the
    /// joystick, the arrows, paste, scroll, ctrl, mic, image, files).
    pub fn encoded_bytes(&self) -> Option<Vec<u8>> {
        match self.kind {
            ShortcutKind::Text => {
                let mut text = unescape(&self.payload);
                if self.append_return {
                    text.push('\r');
                }
                Some(text.into_bytes())
            }
            ShortcutKind::Command => Some(format!("{}\r", unescape(&self.payload)).into_bytes()),
            ShortcutKind::KeyCombo => {
                if !self.payload.is_empty() {
                    let mut text = unescape(&self.payload);
                    if self.append_return {
                        text.push('\r');
                    }
                    return Some(text.into_bytes());
                }
                self.combo_bytes()
            }
            ShortcutKind::Dpad
            | ShortcutKind::Scroll
            | ShortcutKind::Arrows
            | ShortcutKind::Ctrl
            | ShortcutKind::Paste
            | ShortcutKind::Mic
            | ShortcutKind::Image
            | ShortcutKind::Files => None,
        }
    }

    /// xterm's CSI modifier parameter: 1 plain, +1 shift, +2 alt, +4 ctrl
    /// (⌘ counts as ctrl on a phone).
    fn csi_modifier(&self) -> u32 {
        let mut m = 1;
        if self.modifiers.contains(&ShortcutModifier::Shift) {
            m += 1;
        }
        if self.modifiers.contains(&ShortcutModifier::Alt) {
            m += 2;
        }
        if self.modifiers.contains(&ShortcutModifier::Ctrl)
            || self.modifiers.contains(&ShortcutModifier::Cmd)
        {
            m += 4;
        }
        m
    }

    fn csi_letter(&self, letter: char) -> Vec<u8> {
        match self.csi_modifier() {
            1 => format!("\u{1B}[{letter}"),
            m => format!("\u{1B}[1;{m}{letter}"),
        }
        .into_bytes()
    }

    fn csi_tilde(&self, number: u32) -> Vec<u8> {
        match self.csi_modifier() {
            1 => format!("\u{1B}[{number}~"),
            m => format!("\u{1B}[{number};{m}~"),
        }
        .into_bytes()
    }

    fn combo_bytes(&self) -> Option<Vec<u8>> {
        let key = swift::lowercased(&self.key);
        let is = |name: &str| swift::eq(&key, name);
        // Named CSI keys carry their modifiers in the sequence.
        for (names, letter) in [
            (&["up"][..], 'A'),
            (&["down"], 'B'),
            (&["right"], 'C'),
            (&["left"], 'D'),
            (&["home"], 'H'),
            (&["end"], 'F'),
        ] {
            if names.iter().any(|n| is(n)) {
                return Some(self.csi_letter(letter));
            }
        }
        if is("pgup") || is("pageup") {
            return Some(self.csi_tilde(5));
        }
        if is("pgdn") || is("pagedown") {
            return Some(self.csi_tilde(6));
        }
        let mut data = Vec::new();
        if self.modifiers.contains(&ShortcutModifier::Alt) {
            data.push(0x1B);
        }
        if is("esc") || is("escape") {
            data.push(0x1B);
        } else if is("tab") {
            // ⇧Tab is its own sequence (back-tab).
            if self.modifiers.contains(&ShortcutModifier::Shift) {
                return Some(b"\x1B[Z".to_vec());
            }
            data.push(0x09);
        } else if is("return") || is("enter") {
            data.push(0x0D);
        } else if is("space") {
            data.push(0x20);
        } else {
            let scalar = key.chars().next()?;
            if self.modifiers.contains(&ShortcutModifier::Ctrl)
                || self.modifiers.contains(&ShortcutModifier::Cmd)
            {
                data.push((u32::from(scalar) & 0x1F) as u8);
            } else if self.modifiers.contains(&ShortcutModifier::Shift) {
                data.extend_from_slice(swift::uppercased(&self.key).as_bytes());
            } else {
                data.extend_from_slice(self.key.as_bytes());
            }
        }
        (!data.is_empty()).then_some(data)
    }

    /// A text chip whose payload is a run of key presses (C0 controls and
    /// DEL), not text to type.
    pub fn sends_discrete_keys(&self) -> bool {
        if self.kind != ShortcutKind::Text {
            return false;
        }
        let bytes = unescape(&self.payload).into_bytes();
        bytes.len() > 1 && bytes.iter().all(|&b| b < 0x20 || b == 0x7F)
    }

    /// The writes a tap makes, in order: one, or one per key press for a run
    /// of keys (a TUI takes content and a Return in one read as a paste).
    pub fn encoded_stages(&self) -> Vec<Vec<u8>> {
        let Some(data) = self.encoded_bytes().filter(|d| !d.is_empty()) else {
            return Vec::new();
        };
        if !self.sends_discrete_keys() {
            return vec![data];
        }
        data.into_iter().map(|byte| vec![byte]).collect()
    }
}

/// Interpret the `\r` `\n` `\t` `\e` typed in the payload editor
/// (`TerminalShortcut.unescape`). Foundation's search matches whole
/// characters: `\r` followed by a combining mark is left alone.
pub fn unescape(text: &str) -> String {
    let text = replace_characters(text, "\\r", "\r");
    let text = replace_characters(&text, "\\n", "\n");
    let text = replace_characters(&text, "\\t", "\t");
    replace_characters(&text, "\\e", "\u{1B}")
}

/// `replacingOccurrences(of:with:)`: non-overlapping matches, left to right,
/// that begin and end on character boundaries.
fn replace_characters(text: &str, needle: &str, replacement: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let boundaries: HashSet<usize> = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = text[at..].find(needle) {
        let start = at + found;
        let end = start + needle.len();
        if boundaries.contains(&start) && boundaries.contains(&end) {
            out.push_str(&text[at..start]);
            out.push_str(replacement);
            at = end;
        } else {
            let next = start + text[start..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[at..next]);
            at = next;
        }
    }
    out.push_str(&text[at..]);
    out
}

impl Entry for TerminalShortcut {
    fn decode(value: &Value) -> Result<Self, String> {
        Self::from_json(value)
    }

    fn encode_list(items: &[Self]) -> String {
        codec::encode(&items.iter().map(Self::to_wire).collect::<Vec<_>>())
    }
}

/// The shortcut bar (`ShortcutStore`). Order is toolbar order.
pub struct ShortcutStore {
    defaults: Arc<dyn Defaults>,
    readiness: Readiness,
    shortcuts: Vec<TerminalShortcut>,
    is_authoritative: bool,
    carried: Vec<Value>,
}

impl ShortcutStore {
    pub const TOOLBAR_LIMIT: usize = 12;
    pub const STORAGE_KEY: &'static str = "moshpit.shortcuts.v1";

    pub fn new(defaults: Arc<dyn Defaults>, readiness: Readiness) -> Self {
        let mut store = Self {
            defaults,
            readiness,
            shortcuts: Vec::new(),
            is_authoritative: false,
            carried: Vec::new(),
        };
        store.load();
        store
    }

    pub fn shortcuts(&self) -> &[TerminalShortcut] {
        &self.shortcuts
    }

    pub fn is_authoritative(&self) -> bool {
        self.is_authoritative
    }

    /// The toolbar: the first twelve marked for it.
    pub fn toolbar(&self) -> Vec<&TerminalShortcut> {
        self.shortcuts
            .iter()
            .filter(|s| s.in_toolbar)
            .take(Self::TOOLBAR_LIMIT)
            .collect()
    }

    pub fn custom(&self) -> Vec<&TerminalShortcut> {
        self.shortcuts.iter().filter(|s| !s.is_builtin).collect()
    }

    /// Every shortcut not on the toolbar, the user's own too — each one's
    /// way back with ＋.
    ///
    /// Fix (B233): `ShortcutStore.available` listed built-ins alone, so a
    /// custom shortcut taken off the bar (or saved while it was full) could
    /// never come back, while the editor's footnote said it stays under
    /// AVAILABLE. A shortcut past the twelfth that an older set marked as on
    /// the bar is not on it either, and shows here too.
    pub fn available(&self) -> Vec<&TerminalShortcut> {
        let shown: Vec<Uuid> = self.toolbar().iter().map(|s| s.id).collect();
        self.shortcuts
            .iter()
            .filter(|s| !shown.contains(&s.id))
            .collect()
    }

    pub fn toolbar_count(&self) -> usize {
        self.toolbar().len()
    }

    /// The bar holds [`Self::TOOLBAR_LIMIT`] shortcuts: ＋ adds none until
    /// one comes off, and the page says so (B233).
    pub fn toolbar_is_full(&self) -> bool {
        self.toolbar_count() >= Self::TOOLBAR_LIMIT
    }

    /// The toolbar for one connection: host-scoped chips only on their hosts
    /// (every host when the name isn't known), multiplexer-only chips only
    /// inside tmux or herdr.
    pub fn toolbar_for(
        &self,
        host_name: Option<&str>,
        in_multiplexer: bool,
    ) -> Vec<&TerminalShortcut> {
        self.toolbar()
            .into_iter()
            .filter(|s| {
                if s.only_in_tmux && !in_multiplexer {
                    return false;
                }
                if s.scope_hosts.is_empty() {
                    return true;
                }
                let Some(host) = host_name else {
                    return true;
                };
                s.scope_hosts.iter().any(|h| swift::eq(h, host))
            })
            .collect()
    }

    /// Add a shortcut; it takes a toolbar slot if one is free.
    /// Adds `shortcut`, on the bar while it has room; whether it went on
    /// the bar. One saved while the bar is full is under AVAILABLE, and the
    /// editor says so as it saves (B252) — it went there in silence.
    pub fn add(&mut self, shortcut: TerminalShortcut) -> bool {
        self.reload_if_needed();
        let mut shortcut = shortcut;
        let on_bar = !self.toolbar_is_full();
        shortcut.in_toolbar = on_bar;
        self.shortcuts.push(shortcut);
        self.persist();
        on_bar
    }

    pub fn update(&mut self, shortcut: TerminalShortcut) {
        self.reload_if_needed();
        let Some(index) = self.shortcuts.iter().position(|s| s.id == shortcut.id) else {
            return;
        };
        self.shortcuts[index] = shortcut;
        self.persist();
    }

    /// A built-in leaves the toolbar; a custom shortcut is deleted.
    pub fn remove(&mut self, id: Uuid) {
        self.reload_if_needed();
        let Some(index) = self.shortcuts.iter().position(|s| s.id == id) else {
            return;
        };
        if self.shortcuts[index].is_builtin {
            self.shortcuts[index].in_toolbar = false;
        } else {
            self.shortcuts.remove(index);
        }
        self.persist();
    }

    /// Put shortcut `id` on the bar — a built-in or the user's own; false
    /// when it can't go there (the bar is full, or no such shortcut), so
    /// the page can say why.
    pub fn add_to_toolbar(&mut self, id: Uuid) -> bool {
        self.reload_if_needed();
        if self.toolbar_is_full() {
            return false;
        }
        let Some(index) = self.shortcuts.iter().position(|s| s.id == id) else {
            return false;
        };
        self.shortcuts[index].in_toolbar = true;
        self.persist();
        true
    }

    pub fn remove_from_toolbar(&mut self, id: Uuid) {
        self.reload_if_needed();
        let Some(index) = self.shortcuts.iter().position(|s| s.id == id) else {
            return;
        };
        self.shortcuts[index].in_toolbar = false;
        self.persist();
    }

    /// Back to the shipped set (the DEBUG reset seam). Carried shortcuts stay.
    pub fn restore_defaults(&mut self) {
        self.reload_if_needed();
        self.shortcuts = Self::builtins();
        self.persist();
    }

    /// Move the toolbar entries at `from_offsets` (toolbar positions) to
    /// before toolbar position `to_offset`, as SwiftUI's `move` does; the
    /// rest follows the toolbar.
    ///
    /// Fix (toolbar-overflow): `ShortcutStore.moveInToolbar`
    /// (`TerminalShortcut.swift:390-399`) rebuilds the set from the toolbar
    /// (the first twelve marked for it) and the unmarked rest, so a set with
    /// more than twelve marked loses the extras on a drag. Here they follow
    /// the toolbar, in their order.
    ///
    /// Offsets past the toolbar (one that changed under the drag, when a
    /// re-read replaced an untrusted set) are left out and the destination
    /// is clamped, where Swift's `move` traps.
    pub fn move_in_toolbar(&mut self, from_offsets: &BTreeSet<usize>, to_offset: usize) {
        self.reload_if_needed();
        let bar_positions: Vec<usize> = self
            .shortcuts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.in_toolbar)
            .map(|(index, _)| index)
            .take(Self::TOOLBAR_LIMIT)
            .collect();
        let mut bar: Vec<TerminalShortcut> = bar_positions
            .iter()
            .map(|&index| self.shortcuts[index].clone())
            .collect();
        move_offsets(&mut bar, from_offsets, to_offset);
        let rest: Vec<TerminalShortcut> = self
            .shortcuts
            .iter()
            .enumerate()
            .filter(|(index, _)| !bar_positions.contains(index))
            .map(|(_, s)| s.clone())
            .collect();
        self.shortcuts = bar.into_iter().chain(rest).collect();
        self.persist();
    }

    pub fn reload_if_needed(&mut self) {
        if !self.is_authoritative {
            self.load();
        }
    }

    pub fn load(&mut self) {
        // Trust is decided first: `persist` — below and inside the reconcile
        // step — refuses to write over a read it cannot trust.
        let defaults = self.defaults.as_ref();
        let readable = self.readiness.is_readable(defaults);
        let contents: Contents<TerminalShortcut> = stored_list::read(defaults, Self::STORAGE_KEY);
        self.carried = contents.carried.clone();
        self.is_authoritative =
            readable && stored_list::secure(defaults, Self::STORAGE_KEY, &contents);
        if readable {
            self.readiness.mark_readable(defaults);
        }
        if !contents.elements.is_empty() {
            self.shortcuts = contents.elements;
            self.reconcile_builtins();
            return;
        }
        // Nothing this build can read: the shipped set, so the bar works —
        // written only over a read that found nothing it couldn't read, and
        // only when trusted.
        self.shortcuts = Self::builtins();
        if contents.is_complete() {
            self.persist();
        }
    }

    /// Built-in summaries among the carried entries (`isBuiltin` true by
    /// Swift's bridging, a string `summary`).
    fn carried_builtin_summaries(&self) -> Vec<String> {
        self.carried
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                if stored_list::json_bool(object.get("isBuiltin")?) != Some(true) {
                    return None;
                }
                object.get("summary")?.as_str().map(str::to_owned)
            })
            .collect()
    }

    /// Give a persisted set every built-in added since it was saved.
    /// Identity is the summary; the cap holds (a new chip lands outside a
    /// full toolbar); new chips are appended so nothing in the bar moves.
    fn reconcile_builtins(&mut self) {
        let mut known: HashSet<String> = self
            .shortcuts
            .iter()
            .filter(|s| s.is_builtin)
            .map(|s| swift::canonical(&s.summary))
            .collect();
        known.extend(
            self.carried_builtin_summaries()
                .iter()
                .map(|s| swift::canonical(s)),
        );
        let mut changed = false;
        for mut builtin in Self::builtins() {
            if known.contains(&swift::canonical(&builtin.summary)) {
                continue;
            }
            if builtin.in_toolbar
                && self.shortcuts.iter().filter(|s| s.in_toolbar).count() >= Self::TOOLBAR_LIMIT
            {
                builtin.in_toolbar = false;
            }
            self.shortcuts.push(builtin);
            changed = true;
        }
        if changed {
            self.persist();
        }
    }

    fn persist(&self) {
        if !self.is_authoritative {
            return;
        }
        if let Some(data) = stored_list::encode(&self.shortcuts, &self.carried) {
            self.defaults
                .set(Self::STORAGE_KEY, Some(Plist::Data(data)));
        }
    }

    /// The shipped set, each with a fresh id: the toolbar esc, tab, ^C,
    /// paste, the joystick, mic, img, files; the rest available.
    pub fn builtins() -> Vec<TerminalShortcut> {
        use ShortcutModifier::{Ctrl, Shift};
        fn base(kind: ShortcutKind, chip: &str, summary: &str, in_bar: bool) -> TerminalShortcut {
            let mut sc = TerminalShortcut::new(Uuid::new_v4());
            sc.kind = kind;
            sc.chip_label = chip.to_owned();
            sc.summary = summary.to_owned();
            sc.is_builtin = true;
            sc.in_toolbar = in_bar;
            sc
        }
        fn combo(
            chip: &str,
            summary: &str,
            mods: &[ShortcutModifier],
            key: &str,
            repeats: bool,
            in_bar: bool,
        ) -> TerminalShortcut {
            let mut sc = base(ShortcutKind::KeyCombo, chip, summary, in_bar);
            sc.modifiers = mods.iter().copied().collect();
            sc.key = key.to_owned();
            sc.repeat_on_hold = repeats;
            sc
        }
        fn text(chip: &str, summary: &str, payload: &str) -> TerminalShortcut {
            let mut sc = base(ShortcutKind::Text, chip, summary, false);
            sc.payload = payload.to_owned();
            sc
        }
        vec![
            combo("esc", "Escape", &[], "esc", false, true),
            combo("tab", "Tab", &[], "tab", false, true),
            combo("^C", "Interrupt (SIGINT)", &[Ctrl], "c", false, true),
            combo("^L", "Clear screen", &[Ctrl], "l", false, false),
            base(ShortcutKind::Paste, "paste", "Paste clipboard", true),
            base(ShortcutKind::Dpad, "✛", "Arrow keys", true),
            base(ShortcutKind::Arrows, "←↑↓→", "Tap arrows", false),
            base(ShortcutKind::Ctrl, "ctrl", "Control", false),
            base(ShortcutKind::Scroll, "⇅", "Scroll history", false),
            base(ShortcutKind::Mic, "mic", "Voice input", true),
            base(ShortcutKind::Image, "img", "Attach image", true),
            base(ShortcutKind::Files, "files", "Browse files", true),
            combo("⏎", "Return", &[], "return", false, false),
            text("⇥⏎", "Accept suggestion and send", "\\t\\r"),
            combo("^D", "End of file", &[Ctrl], "d", false, false),
            combo("^R", "Reverse search", &[Ctrl], "r", false, false),
            combo("⌃End", "Jump to end", &[Ctrl], "end", false, false),
            combo(
                "⇧Tab",
                "Back-tab / toggle mode",
                &[Shift],
                "tab",
                false,
                false,
            ),
            combo("↑", "History prev", &[], "up", true, false),
            combo("↓", "History next", &[], "down", true, false),
            combo("←", "Cursor left", &[], "left", true, false),
            combo("→", "Cursor right", &[], "right", true, false),
            combo("^U", "Clear line before cursor", &[Ctrl], "u", false, false),
            combo("^K", "Clear line after cursor", &[Ctrl], "k", false, false),
            combo("^A", "Beginning of line", &[Ctrl], "a", false, false),
            combo("^E", "End of line", &[Ctrl], "e", false, false),
            combo("^W", "Delete word", &[Ctrl], "w", false, false),
            combo("^Z", "Suspend", &[Ctrl], "z", false, false),
            combo("End", "End", &[], "end", false, false),
            combo("Home", "Home", &[], "home", false, false),
            combo("PgUp", "Page up", &[], "pgup", true, false),
            combo("PgDn", "Page down", &[], "pgdn", true, false),
        ]
    }
}

/// SwiftUI's `move(fromOffsets:toOffset:)`: the elements at `from` (in
/// their order) go to before `to`, an offset into the array as it was. An
/// offset past the end is left out and `to` clamped to the end, where
/// SwiftUI traps.
pub fn move_offsets<T>(items: &mut Vec<T>, from: &BTreeSet<usize>, to: usize) {
    let len = items.len();
    let from: BTreeSet<usize> = from.iter().copied().filter(|&i| i < len).collect();
    let to = to.min(len);
    let below = from.iter().filter(|&&i| i < to).count();
    let mut moving = Vec::with_capacity(from.len());
    for &index in from.iter().rev() {
        moving.push(items.remove(index));
    }
    moving.reverse();
    let at = to - below;
    for (offset, item) in moving.into_iter().enumerate() {
        items.insert(at + offset, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;
    use moshpit_seal::foundation::json;
    use serde::{Deserialize, Serialize};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn combo(mods: &[ShortcutModifier], key: &str) -> TerminalShortcut {
        let mut sc = TerminalShortcut::new(Uuid::new_v4());
        sc.modifiers = mods.iter().copied().collect();
        sc.key = key.into();
        sc
    }

    fn text(payload: &str) -> TerminalShortcut {
        let mut sc = TerminalShortcut::new(Uuid::new_v4());
        sc.kind = ShortcutKind::Text;
        sc.payload = payload.into();
        sc
    }

    fn fresh() -> (Arc<MemoryDefaults>, ShortcutStore) {
        let defaults = Arc::new(MemoryDefaults::new());
        let store = ShortcutStore::new(defaults.clone(), Readiness::Always);
        (defaults, store)
    }

    fn stored_value(defaults: &MemoryDefaults) -> Vec<Value> {
        let Some(Plist::Data(data)) = defaults.get(ShortcutStore::STORAGE_KEY) else {
            panic!("nothing stored")
        };
        let Value::Array(entries) = json::parse(&data, json::Dialect::Decoder).unwrap() else {
            panic!()
        };
        entries
    }

    /// `TerminalShortcutEncodingTests`.
    #[test]
    fn encoding() {
        use ShortcutModifier::*;
        assert_eq!(combo(&[], "esc").encoded_bytes(), Some(vec![0x1B]));
        assert_eq!(combo(&[], "tab").encoded_bytes(), Some(vec![0x09]));
        assert_eq!(combo(&[], "return").encoded_bytes(), Some(vec![0x0D]));
        assert_eq!(combo(&[], "enter").encoded_bytes(), Some(vec![0x0D]));
        assert_eq!(combo(&[], "space").encoded_bytes(), Some(vec![0x20]));
        assert_eq!(combo(&[], "up").encoded_bytes(), Some(b"\x1B[A".to_vec()));
        assert_eq!(combo(&[], "left").encoded_bytes(), Some(b"\x1B[D".to_vec()));
        assert_eq!(combo(&[Ctrl], "c").encoded_bytes(), Some(vec![0x03]));
        assert_eq!(combo(&[Ctrl], "u").encoded_bytes(), Some(vec![0x15]));
        assert_eq!(combo(&[Alt], "b").encoded_bytes(), Some(vec![0x1B, 0x62]));
        assert_eq!(
            combo(&[Ctrl], "end").encoded_bytes(),
            Some(b"\x1B[1;5F".to_vec())
        );
        assert_eq!(
            combo(&[Shift], "tab").encoded_bytes(),
            Some(b"\x1B[Z".to_vec())
        );
        assert_eq!(
            combo(&[], "pgup").encoded_bytes(),
            Some(b"\x1B[5~".to_vec())
        );
        assert_eq!(
            combo(&[Ctrl], "pgdn").encoded_bytes(),
            Some(b"\x1B[6;5~".to_vec())
        );
        assert_eq!(
            combo(&[Ctrl], "left").encoded_bytes(),
            Some(b"\x1B[1;5D".to_vec())
        );
        assert_eq!(combo(&[Shift], "a").encoded_bytes(), Some(b"A".to_vec()));
        assert_eq!(combo(&[], "").encoded_bytes(), None);
        let mut sc = text("git status");
        assert_eq!(sc.encoded_bytes(), Some(b"git status".to_vec()));
        sc.append_return = true;
        assert_eq!(sc.encoded_bytes(), Some(b"git status\r".to_vec()));
        sc.kind = ShortcutKind::Command;
        sc.append_return = false;
        assert_eq!(sc.encoded_bytes(), Some(b"git status\r".to_vec()));
        assert_eq!(
            text("a\\tb\\r\\e").encoded_bytes(),
            Some(vec![0x61, 0x09, 0x62, 0x0D, 0x1B])
        );
        for kind in [
            ShortcutKind::Ctrl,
            ShortcutKind::Arrows,
            ShortcutKind::Files,
        ] {
            let mut sc = text("x");
            sc.kind = kind;
            assert_eq!(sc.encoded_bytes(), None);
        }
    }

    #[test]
    fn only_key_runs_split_into_stages() {
        let two_step = ShortcutStore::builtins()
            .into_iter()
            .find(|s| s.chip_label == "⇥⏎")
            .unwrap();
        assert_eq!(two_step.encoded_stages(), [vec![0x09], vec![0x0D]]);
        assert!(!two_step.append_return);
        assert_eq!(
            text("git status").encoded_stages(),
            [b"git status".to_vec()]
        );
        assert!(!text("git status").sends_discrete_keys());
        assert_eq!(text("\\r").encoded_stages(), [vec![0x0D]]);
        assert!(!text("deploy\\r").sends_discrete_keys());
        assert_eq!(text("\\e\\r").encoded_stages(), [vec![0x1B], vec![0x0D]]);
        assert!(text("").encoded_stages().is_empty());
    }

    #[test]
    fn an_escape_only_counts_on_character_boundaries() {
        assert_eq!(unescape("a\\r\u{301}b"), "a\\r\u{301}b");
        assert_eq!(unescape("\\\\r"), "\\\r");
        assert_eq!(unescape("\\r\\n"), "\r\n");
    }

    #[test]
    fn modifiers_sort_as_macos_writes_them() {
        use ShortcutModifier::*;
        let set: BTreeSet<_> = [Cmd, Shift, Alt, Ctrl].into_iter().collect();
        assert_eq!(set.into_iter().collect::<Vec<_>>(), [Ctrl, Alt, Shift, Cmd]);
    }

    #[test]
    fn seeds_a_lean_default_toolbar() {
        let (defaults, store) = fresh();
        let labels: Vec<&str> = store
            .toolbar()
            .iter()
            .map(|s| s.chip_label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["esc", "tab", "^C", "paste", "✛", "mic", "img", "files"]
        );
        assert!(
            store
                .available()
                .iter()
                .any(|s| s.kind == ShortcutKind::Ctrl)
        );
        assert!(
            store
                .available()
                .iter()
                .any(|s| s.kind == ShortcutKind::Scroll)
        );
        assert!(store.available().iter().all(|s| s.is_builtin));
        let arrows = store
            .shortcuts()
            .iter()
            .find(|s| s.kind == ShortcutKind::Arrows)
            .unwrap();
        assert!(!arrows.in_toolbar);
        for key in ["up", "down", "left", "right"] {
            let sc = store
                .shortcuts()
                .iter()
                .find(|s| s.kind == ShortcutKind::KeyCombo && s.key == key)
                .unwrap();
            assert!(sc.repeat_on_hold && !sc.in_toolbar);
        }
        // Seeded and written on a trusted, empty read.
        assert_eq!(
            stored_value(&defaults).len(),
            ShortcutStore::builtins().len()
        );
    }

    #[test]
    fn a_builtin_put_in_the_bar_survives_a_relaunch() {
        let (defaults, mut store) = fresh();
        let up = store
            .available()
            .into_iter()
            .find(|s| s.key == "up")
            .unwrap()
            .id;
        store.add_to_toolbar(up);
        let relaunched = ShortcutStore::new(defaults, Readiness::Always);
        assert!(relaunched.toolbar().iter().any(|s| s.id == up));
    }

    /// Fix (B233): a custom shortcut off the bar was in no list a ＋ could
    /// reach — the app's AVAILABLE held built-ins only.
    #[test]
    fn a_custom_shortcut_off_the_bar_is_available_and_comes_back() {
        let (defaults, mut store) = fresh();
        let mut mine = TerminalShortcut::new(Uuid::new_v4());
        mine.kind = ShortcutKind::Text;
        mine.chip_label = "gs".into();
        mine.payload = "git status".into();
        store.add(mine.clone());
        store.remove_from_toolbar(mine.id);
        assert!(!store.toolbar().iter().any(|s| s.id == mine.id));
        assert!(store.available().iter().any(|s| s.id == mine.id));
        assert!(store.add_to_toolbar(mine.id));
        assert!(store.toolbar().iter().any(|s| s.id == mine.id));
        assert!(!store.available().iter().any(|s| s.id == mine.id));
        let relaunched = ShortcutStore::new(defaults, Readiness::Always);
        assert!(relaunched.toolbar().iter().any(|s| s.id == mine.id));
    }

    /// Fix (B233): with twelve on the bar ＋ did nothing and said nothing;
    /// now the store says it can't, and one saved meanwhile waits under
    /// AVAILABLE.
    #[test]
    fn a_full_bar_says_it_cant_and_keeps_the_rest_available() {
        let (_, mut store) = fresh();
        while !store.toolbar_is_full() {
            let mut filler = TerminalShortcut::new(Uuid::new_v4());
            filler.chip_label = format!("f{}", store.toolbar_count());
            store.add(filler);
        }
        let late = TerminalShortcut::new(Uuid::new_v4());
        store.add(late.clone());
        assert!(store.available().iter().any(|s| s.id == late.id));
        assert!(!store.add_to_toolbar(late.id));
        assert_eq!(store.toolbar_count(), ShortcutStore::TOOLBAR_LIMIT);
        let first = store.toolbar()[0].id;
        store.remove_from_toolbar(first);
        assert!(store.add_to_toolbar(late.id));
    }

    /// Fix (B252): a shortcut saved while the bar is full went under
    /// AVAILABLE in silence; the store says where it went, so the editor
    /// can say so as it saves.
    #[test]
    fn adding_says_whether_the_shortcut_went_on_the_bar() {
        let (_, mut store) = fresh();
        while !store.toolbar_is_full() {
            let mut filler = TerminalShortcut::new(Uuid::new_v4());
            filler.chip_label = format!("f{}", store.toolbar_count());
            assert!(store.add(filler), "a shortcut added with room on the bar");
        }
        let late = TerminalShortcut::new(Uuid::new_v4());
        assert!(
            !store.add(late.clone()),
            "saved off a full bar, and said it went on"
        );
        let saved = store.shortcuts().iter().find(|s| s.id == late.id).unwrap();
        assert!(!saved.in_toolbar);
        let first = store.toolbar()[0].id;
        store.remove_from_toolbar(first);
        assert!(store.add(TerminalShortcut::new(Uuid::new_v4())));
    }

    #[test]
    fn a_full_toolbar_keeps_new_builtins_out_of_the_bar() {
        let defaults = Arc::new(MemoryDefaults::new());
        let full: Vec<TerminalShortcut> = (0..ShortcutStore::TOOLBAR_LIMIT)
            .map(|i| {
                let mut sc = TerminalShortcut::new(Uuid::new_v4());
                sc.key = i.to_string();
                sc.chip_label = format!("k{i}");
                sc
            })
            .collect();
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(
                TerminalShortcut::encode_list(&full).into_bytes(),
            )),
        );
        let store = ShortcutStore::new(defaults, Readiness::Always);
        assert_eq!(store.toolbar_count(), ShortcutStore::TOOLBAR_LIMIT);
        assert!(!store.toolbar().iter().any(|s| s.kind == ShortcutKind::Mic));
        assert!(
            store
                .shortcuts()
                .iter()
                .any(|s| s.kind == ShortcutKind::Mic)
        );
    }

    #[test]
    fn the_toolbar_is_capped_at_twelve_and_filters_by_scope() {
        let (_, mut store) = fresh();
        for i in 0..10 {
            let mut sc = text("x");
            sc.chip_label = format!("c{i}");
            store.add(sc);
        }
        assert_eq!(store.toolbar_count(), ShortcutStore::TOOLBAR_LIMIT);
        assert!(store.custom().iter().any(|s| !s.in_toolbar));

        let (_, mut store) = fresh();
        let mut scoped = text("tmux");
        scoped.kind = ShortcutKind::Command;
        scoped.chip_label = "⌘B".into();
        scoped.set_scope_hosts(["work".to_owned()]);
        store.add(scoped);
        let has = |store: &ShortcutStore, host: Option<&str>, mux: bool| {
            store
                .toolbar_for(host, mux)
                .iter()
                .any(|s| s.chip_label == "⌘B")
        };
        assert!(has(&store, Some("work"), false));
        assert!(!has(&store, Some("rednote"), false));
        assert!(has(&store, None, false));
        let mut tmux_only = text("claude");
        tmux_only.chip_label = "clm".into();
        tmux_only.only_in_tmux = true;
        let (_, mut store) = fresh();
        store.add(tmux_only);
        assert!(
            store
                .toolbar_for(None, true)
                .iter()
                .any(|s| s.chip_label == "clm")
        );
        assert!(
            !store
                .toolbar_for(None, false)
                .iter()
                .any(|s| s.chip_label == "clm")
        );
    }

    #[test]
    fn removing_a_builtin_keeps_it_available() {
        let (_, mut store) = fresh();
        let esc = store.toolbar()[0].id;
        store.remove_from_toolbar(esc);
        assert!(!store.toolbar().iter().any(|s| s.chip_label == "esc"));
        assert!(store.available().iter().any(|s| s.chip_label == "esc"));
        store.remove(esc);
        assert!(store.shortcuts().iter().any(|s| s.id == esc));
    }

    #[test]
    fn new_builtins_are_injected_into_an_older_set() {
        let defaults = Arc::new(MemoryDefaults::new());
        let pruned: Vec<TerminalShortcut> = ShortcutStore::builtins()
            .into_iter()
            .filter(|s| {
                ![
                    "Jump to end",
                    "Back-tab / toggle mode",
                    "End",
                    "Home",
                    "Page up",
                    "Page down",
                ]
                .contains(&s.summary.as_str())
            })
            .collect();
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(
                TerminalShortcut::encode_list(&pruned).into_bytes(),
            )),
        );
        let migrated = ShortcutStore::new(defaults.clone(), Readiness::Always);
        assert!(
            migrated
                .shortcuts()
                .iter()
                .any(|s| s.summary == "Jump to end" && !s.in_toolbar)
        );
        assert!(
            migrated
                .shortcuts()
                .iter()
                .any(|s| s.summary == "Page down")
        );

        let without_ctrl: Vec<TerminalShortcut> = ShortcutStore::builtins()
            .into_iter()
            .filter(|s| s.kind != ShortcutKind::Ctrl)
            .collect();
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(
                TerminalShortcut::encode_list(&without_ctrl).into_bytes(),
            )),
        );
        let migrated = ShortcutStore::new(defaults, Readiness::Always);
        assert!(
            migrated
                .available()
                .iter()
                .any(|s| s.kind == ShortcutKind::Ctrl)
        );
        assert!(
            !migrated
                .toolbar()
                .iter()
                .any(|s| s.kind == ShortcutKind::Ctrl)
        );
    }

    #[test]
    fn custom_shortcuts_persist() {
        let (defaults, mut store) = fresh();
        let mut sc = text("git push");
        sc.kind = ShortcutKind::Command;
        sc.chip_label = "GP".into();
        store.add(sc);
        let reader = ShortcutStore::new(defaults, Readiness::Always);
        assert!(reader.custom().iter().any(|s| s.chip_label == "GP"));
    }

    #[test]
    fn built_ins_are_shown_not_written_over_a_file_that_could_not_be_read() {
        let scratch = Arc::new(MemoryDefaults::new());
        let mut deploy = text("make deploy");
        deploy.chip_label = "deploy".into();
        deploy.summary = "Deploy".into();
        ShortcutStore::new(scratch.clone(), Readiness::Always).add(deploy);
        let payload = scratch.get(ShortcutStore::STORAGE_KEY);

        let defaults = Arc::new(MemoryDefaults::new());
        let unlocked = Arc::new(AtomicBool::new(false));
        let flag = unlocked.clone();
        let mut store = ShortcutStore::new(
            defaults.clone(),
            Readiness::custom(move || flag.load(Ordering::SeqCst)),
        );
        assert!(!store.toolbar().is_empty());
        assert!(!store.is_authoritative());
        assert_eq!(defaults.get(ShortcutStore::STORAGE_KEY), None);

        defaults.set(ShortcutStore::STORAGE_KEY, payload);
        unlocked.store(true, Ordering::SeqCst);
        store.reload_if_needed();
        assert_eq!(
            store
                .custom()
                .iter()
                .map(|s| s.summary.as_str())
                .collect::<Vec<_>>(),
            ["Deploy"]
        );
        assert!(store.is_authoritative());
    }

    #[test]
    fn a_chip_kind_this_build_doesnt_know_no_longer_reseeds_the_bar() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mut deploy = text("make deploy");
        deploy.summary = "Deploy".into();
        let mut macro_chip =
            json::parse(deploy.to_json().as_bytes(), json::Dialect::Decoder).unwrap();
        if let Value::Object(fields) = &mut macro_chip {
            fields.insert("kind", Value::String("macro".into()));
            fields.insert("id", Value::String(Uuid::new_v4().to_string()));
        }
        let list = format!("[{},{}]", deploy.to_json(), codec::write(&macro_chip));
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(list.into_bytes())),
        );
        let store = ShortcutStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(
            store
                .custom()
                .iter()
                .map(|s| s.summary.as_str())
                .collect::<Vec<_>>(),
            ["Deploy"]
        );
        let kinds: Vec<String> = stored_value(&defaults)
            .iter()
            .filter_map(|e| Some(e.as_object()?.get("kind")?.as_str()?.to_owned()))
            .collect();
        assert!(kinds.contains(&"macro".to_owned()));
        assert_eq!(kinds.last().map(String::as_str), Some("macro"));
    }

    #[test]
    fn a_set_this_build_cant_read_at_all_shows_the_builtins_and_writes_nothing() {
        let defaults = Arc::new(MemoryDefaults::new());
        let unreadable =
            br#"[{"kind":"macro","summary":"x"},{"kind":"macro","summary":"y"}]"#.to_vec();
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(unreadable.clone())),
        );
        let mut store = ShortcutStore::new(defaults.clone(), Readiness::Always);
        assert!(!store.toolbar().is_empty());
        assert_eq!(
            defaults.get(ShortcutStore::STORAGE_KEY),
            Some(Plist::Data(unreadable))
        );
        let mut sc = text("x");
        sc.summary = "z".into();
        store.add(sc);
        let summaries: Vec<String> = stored_value(&defaults)
            .iter()
            .filter_map(|e| Some(e.as_object()?.get("summary")?.as_str()?.to_owned()))
            .collect();
        assert!(summaries.contains(&"x".to_owned()) && summaries.contains(&"y".to_owned()));
    }

    #[test]
    fn a_builtin_stored_in_a_shape_this_build_cant_read_isnt_injected_twice() {
        let defaults = Arc::new(MemoryDefaults::new());
        let builtins = ShortcutStore::builtins();
        let mut entries: Vec<String> = builtins
            .iter()
            .filter(|s| s.kind != ShortcutKind::Paste)
            .map(TerminalShortcut::to_json)
            .collect();
        entries
            .push(r#"{"kind":"pasteV2","summary":"Paste clipboard","isBuiltin":true}"#.to_owned());
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(format!("[{}]", entries.join(",")).into_bytes())),
        );
        let store = ShortcutStore::new(defaults, Readiness::Always);
        assert_eq!(
            store
                .shortcuts()
                .iter()
                .filter(|s| s.summary == "Paste clipboard")
                .count(),
            0
        );
    }

    /// `TerminalShortcut` as 1.0.8 declares it: its ten kinds.
    #[derive(Debug, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct OlderBuildShortcut {
        id: Uuid,
        kind: OlderKind,
        modifiers: Vec<ShortcutModifier>,
        key: String,
        payload: String,
        append_return: bool,
        repeat_on_hold: bool,
        chip_label: String,
        summary: String,
        color_id: String,
        scope_hosts: Vec<String>,
        only_in_tmux: bool,
        is_builtin: bool,
        in_toolbar: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    enum OlderKind {
        KeyCombo,
        Text,
        Command,
        Dpad,
        Arrows,
        Paste,
        Scroll,
        Ctrl,
        Mic,
        Image,
    }

    /// Fix (files-downgrade): 1.0.8 decoded the old `"kind":"files"` as no
    /// set at all and seeded its built-ins over every custom chip.
    #[test]
    fn version_1_0_8_reads_every_shortcut_this_build_saves_and_this_build_reads_back_what_it_saves()
    {
        let (defaults, mut store) = fresh();
        let mut deploy = text("make deploy");
        deploy.summary = "Deploy".into();
        store.add(deploy);
        let Some(Plist::Data(data)) = defaults.get(ShortcutStore::STORAGE_KEY) else {
            panic!()
        };
        let mut older: Vec<OlderBuildShortcut> = json::decode(&data).unwrap();
        let files = older.iter().find(|s| s.summary == "Browse files").unwrap();
        assert_eq!(files.kind, OlderKind::KeyCombo);
        assert!(files.key.is_empty() && files.payload.is_empty());
        assert!(older.iter().any(|s| s.summary == "Deploy"));

        // 1.0.8 saves it back without the field it doesn't know.
        older.retain(|s| s.summary != "Tab");
        defaults.set(
            ShortcutStore::STORAGE_KEY,
            Some(Plist::Data(json::encode(&older).into_bytes())),
        );
        let reread = ShortcutStore::new(defaults, Readiness::Always);
        let files = reread
            .shortcuts()
            .iter()
            .find(|s| s.summary == "Browse files")
            .unwrap();
        assert_eq!(files.kind, ShortcutKind::Files);
        assert_eq!(
            reread
                .shortcuts()
                .iter()
                .filter(|s| s.summary == "Browse files")
                .count(),
            1
        );
        assert!(reread.custom().iter().any(|s| s.summary == "Deploy"));
    }

    #[test]
    fn the_files_chip_the_app_stored_reads_and_an_unknown_real_kind_falls_back() {
        let id = Uuid::new_v4();
        let stored = format!(
            r#"{{"id":"{id}","kind":"files","modifiers":[],"key":"","payload":"","appendReturn":false,"repeatOnHold":false,"chipLabel":"files","summary":"Browse files","colorId":"gray","scopeHosts":[],"onlyInTmux":false,"isBuiltin":true,"inToolbar":true}}"#
        );
        let value = json::parse(stored.as_bytes(), json::Dialect::Decoder).unwrap();
        assert_eq!(
            TerminalShortcut::from_json(&value).unwrap().kind,
            ShortcutKind::Files
        );
        let mut custom = text("x");
        custom.kind = ShortcutKind::Image;
        let mut value = json::parse(custom.to_json().as_bytes(), json::Dialect::Decoder).unwrap();
        if let Value::Object(fields) = &mut value {
            fields.insert("shortcutKind", Value::String("macro".into()));
        }
        assert_eq!(
            TerminalShortcut::from_json(&value).unwrap().kind,
            ShortcutKind::Image
        );
        // Not the built-in: a custom key combo stays one.
        let mut lookalike = TerminalShortcut::new(Uuid::new_v4());
        lookalike.summary = "Browse files".into();
        let mut value =
            json::parse(lookalike.to_json().as_bytes(), json::Dialect::Decoder).unwrap();
        if let Value::Object(fields) = &mut value {
            fields.remove("shortcutKind");
        }
        assert_eq!(
            TerminalShortcut::from_json(&value).unwrap().kind,
            ShortcutKind::KeyCombo
        );
    }

    #[test]
    fn moving_follows_swiftuis_offsets() {
        let mut items = vec!['a', 'b', 'c', 'd', 'e'];
        move_offsets(&mut items, &[1].into_iter().collect(), 4);
        assert_eq!(items, ['a', 'c', 'd', 'b', 'e']);
        let mut items = vec!['a', 'b', 'c', 'd', 'e'];
        move_offsets(&mut items, &[3, 4].into_iter().collect(), 0);
        assert_eq!(items, ['d', 'e', 'a', 'b', 'c']);
        let mut items = vec!['a', 'b', 'c', 'd', 'e'];
        move_offsets(&mut items, &[0, 2].into_iter().collect(), 5);
        assert_eq!(items, ['b', 'd', 'e', 'a', 'c']);
        let mut items = vec!['a', 'b', 'c'];
        move_offsets(&mut items, &[1, 7].into_iter().collect(), 9);
        assert_eq!(items, ['a', 'c', 'b']);
    }

    #[test]
    fn a_toolbar_move_keeps_every_shortcut() {
        let (_, mut store) = fresh();
        for shortcut in store.shortcuts.iter_mut().take(14) {
            shortcut.in_toolbar = true;
        }
        let before = store.shortcuts().len();
        store.move_in_toolbar(&[0].into_iter().collect(), 2);
        assert_eq!(store.shortcuts().len(), before);
        assert_eq!(store.toolbar()[1].chip_label, "esc");
    }
}
