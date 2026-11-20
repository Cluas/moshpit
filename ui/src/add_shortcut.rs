//! Shortcuts › Add Shortcut, and a custom shortcut's editor
//! (`docs/nsh/UI-DESIGN.md` §6.7), in NSH, in its sheet: a live preview of
//! the key, its type, a key combo's quick keys and trigger, its action, how
//! its chip looks, and where it shows.
//!
//! The host posts the shortcut to edit ([`Message`]): `{"op": "shortcut",
//! "draft": {…}, "editing": bool, "hosts": [...], "toolbarCount": n,
//! "limit": 12}` ([`Editor`]). The page keeps its own copy for the preview
//! and tells the host the whole draft as it changes (`{"ev": "change",
//! "draft": {…}}`), then `{"ev": "save"}` (enabled when the draft can be
//! saved: a chip label, and a key or payload) or `{"ev": "cancel"}`.
//! Saving is the host's: its store keeps the format every build since 1.0.8
//! reads.
//!
//! Built from the ui layer (R11): the preview as the bar's own key on the
//! terminal's ground; Key Combo, Text or Command in a segmented control;
//! the quick keys as chips that wrap (none cut short at any text size);
//! the modifiers as chips that are on or off and the main key's field; the
//! payload in a stacked field, Append Return and Repeat on hold; the chip
//! label (six characters at most, the field says so), the description and
//! the color's swatches; the hosts as chips and Only in a multiplexer.
//! Cancel and Save, the title Add Shortcut or Edit Shortcut. The
//! identifiers: `shortcut-cancel`, `shortcut-save`, `shortcut-type`,
//! `shortcut-preset-<label>`, `shortcut-modifier-<id>`, `shortcut-key`
//! (its input `shortcut-key-input`), `shortcut-combo`, `shortcut-payload`,
//! `shortcut-append-return`, `shortcut-repeat`, `shortcut-chip-label`,
//! `shortcut-description`, `shortcut-color-<id>`, `shortcut-host-<name>`,
//! `shortcut-only-multiplexer`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        control::{segmented, switch},
        feedback::separator,
        field::{ControlState, FieldControl, FieldLayout, field, remount, text_input},
        key::{ChipVariant, chip, key},
        page,
        row::{GAP, PADDING_X, PADDING_Y, Row, row},
        section::{Inset, section},
        swatch::swatch,
        tokens::{number, token},
        typography::TextToken,
    },
};

/// A shortcut as the editor edits it (the store's fields, by their names).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Draft {
    /// `keyCombo`, `text` or `command` (a built-in's other kinds are kept
    /// as they are).
    pub kind: String,
    /// `ctrl`, `alt`, `shift`, `cmd`.
    pub modifiers: Vec<String>,
    pub key: String,
    pub payload: String,
    pub append_return: bool,
    pub repeat_on_hold: bool,
    pub chip_label: String,
    pub summary: String,
    /// `gray`, `accent`, `mosh`, `amber`.
    pub color_id: String,
    pub scope_hosts: Vec<String>,
    pub only_in_tmux: bool,
}

/// The modifiers in the order a chip writes them (⌃⌥⇧⌘), with their
/// symbols.
pub const MODIFIERS: [(&str, &str); 4] =
    [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("cmd", "⌘")];

impl Draft {
    /// What a key combo's chip shows: its modifiers' symbols and its key,
    /// in capitals (`comboCap`).
    pub fn combo_cap(&self) -> String {
        MODIFIERS
            .iter()
            .filter(|(id, _)| self.modifiers.iter().any(|m| m == id))
            .map(|(_, symbol)| (*symbol).to_owned())
            .chain(std::iter::once(self.key.to_uppercase()))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Whether Save can be pressed (`canSave`): a chip label, and a key or
    /// payload for a key combo, a payload otherwise.
    pub fn can_save(&self) -> bool {
        !self.chip_label.is_empty()
            && if self.kind == "keyCombo" {
                !self.key.is_empty() || !self.payload.is_empty()
            } else {
                !self.payload.is_empty()
            }
    }

    /// The preview's line after the chip (`previewSummary`).
    pub fn preview_summary(&self) -> String {
        let name = if self.summary.is_empty() {
            t!("unnamed")
        } else {
            self.summary.clone()
        };
        match self.kind.as_str() {
            "keyCombo" => {
                let cap = self.combo_cap();
                if cap.is_empty() {
                    name
                } else {
                    t!("%@ · sends %@", arg1 = name, arg2 = cap)
                }
            }
            "text" if !self.payload.is_empty() => {
                t!(
                    "%@ · types \"%@\"",
                    arg1 = name,
                    arg2 = self.payload.clone()
                )
            }
            "command" if !self.payload.is_empty() => {
                t!("%@ · runs %@", arg1 = name, arg2 = self.payload.clone())
            }
            _ => name,
        }
    }

    /// `preset` filled in (`apply(_:)`): the trigger, the payload and the
    /// chip label are the preset's; a summary the user wrote stays.
    pub fn apply(&mut self, preset: &Preset) {
        self.apply_described(preset, preset.summary);
    }

    /// [`Self::apply`], the preset's description worded as `summary` — the
    /// page passes it in the app's language ([`describe`], B234): what the
    /// person sees is what they keep.
    pub fn apply_described(&mut self, preset: &Preset, summary: &str) {
        self.modifiers = preset.mods.iter().map(|m| (*m).to_owned()).collect();
        self.key = preset.key.to_owned();
        self.payload = preset.payload.to_owned();
        self.chip_label = preset.chip.to_owned();
        if self.summary.is_empty() {
            self.summary = summary.to_owned();
        }
    }
}

/// A quick key's description (or a built-in shortcut's name) in the app's
/// language; anything else as it is. B234: they were English in every
/// language — on both pages.
pub fn describe(english: &str) -> String {
    describe_in(english, &crate::strings::shown_language())
}

/// [`describe`] in `locale`, for a host outside a surface (the Shortcuts
/// page's names: a built-in's stored summary is its identity, so it stays
/// English in storage and is put into words only here).
pub fn describe_in(english: &str, locale: &str) -> String {
    match english {
        "multiplexer prefix" => crate::tl!("multiplexer prefix", locale),
        "Interrupt" => crate::tl!("Interrupt", locale),
        "EOF" => crate::tl!("EOF", locale),
        "Suspend" => crate::tl!("Suspend", locale),
        "Quit" => crate::tl!("Quit", locale),
        "Clear" => crate::tl!("Clear", locale),
        "Line start" => crate::tl!("Line start", locale),
        "Line end" => crate::tl!("Line end", locale),
        "Search" => crate::tl!("Search", locale),
        "Del word" => crate::tl!("Del word", locale),
        "Del line" => crate::tl!("Del line", locale),
        "Kill EOL" => crate::tl!("Kill EOL", locale),
        "Yank" => crate::tl!("Yank", locale),
        "Prev" => crate::tl!("Prev", locale),
        "Next" => crate::tl!("Next", locale),
        "Tab" => crate::tl!("Tab key", locale),
        "Back-tab" => crate::tl!("Back-tab", locale),
        "Return" => crate::tl!("Return", locale),
        "Escape" => crate::tl!("Escape", locale),
        "Forward delete" => crate::tl!("Forward delete", locale),
        "Backspace" => crate::tl!("Backspace", locale),
        "Left" => crate::tl!("Left", locale),
        "Down" => crate::tl!("Down", locale),
        "Up" => crate::tl!("Up", locale),
        "Right" => crate::tl!("Right", locale),
        "Home" => crate::tl!("Home", locale),
        "End" => crate::tl!("End", locale),
        "Page up" => crate::tl!("Page up", locale),
        "Page down" => crate::tl!("Page down", locale),
        // The built-ins' names (the store's `ShortcutStore::builtins`) the
        // quick keys do not share.
        "Interrupt (SIGINT)" => crate::tl!("Interrupt (SIGINT)", locale),
        "Clear screen" => crate::tl!("Clear screen", locale),
        "Paste clipboard" => crate::tl!("Paste clipboard", locale),
        "Arrow keys" => crate::tl!("Arrow keys", locale),
        "Tap arrows" => crate::tl!("Tap arrows", locale),
        "Control" => crate::tl!("Control", locale),
        "Scroll history" => crate::tl!("Scroll history", locale),
        "Voice input" => crate::tl!("Voice input", locale),
        "Attach image" => crate::tl!("Attach image", locale),
        "Browse files" => crate::tl!("Browse files", locale),
        "Accept suggestion and send" => crate::tl!("Accept suggestion and send", locale),
        "End of file" => crate::tl!("End of file", locale),
        "Reverse search" => crate::tl!("Reverse search", locale),
        "Jump to end" => crate::tl!("Jump to end", locale),
        "Back-tab / toggle mode" => crate::tl!("Back-tab / toggle mode", locale),
        "History prev" => crate::tl!("History prev", locale),
        "History next" => crate::tl!("History next", locale),
        "Cursor left" => crate::tl!("Cursor left", locale),
        "Cursor right" => crate::tl!("Cursor right", locale),
        "Clear line before cursor" => crate::tl!("Clear line before cursor", locale),
        "Clear line after cursor" => crate::tl!("Clear line after cursor", locale),
        "Beginning of line" => crate::tl!("Beginning of line", locale),
        "End of line" => crate::tl!("End of line", locale),
        "Delete word" => crate::tl!("Delete word", locale),
        other => other.to_owned(),
    }
}

/// What the Shortcuts page calls `shortcut`: a built-in's name in
/// `locale`, the user's own words as they wrote them.
pub fn shortcut_name(shortcut: &moshpit_store::shortcuts::TerminalShortcut, locale: &str) -> String {
    if shortcut.is_builtin {
        describe_in(&shortcut.summary, locale)
    } else {
        shortcut.summary.clone()
    }
}

/// A quick keys group's name in the app's language (B234).
pub fn group_name(english: &str) -> String {
    match english {
        "PREFIX" => t!("PREFIX"),
        "CONTROL" => t!("CONTROL"),
        "SPECIAL" => t!("SPECIAL"),
        "NAVIGATION" => t!("NAVIGATION"),
        "F-KEYS" => t!("F-KEYS"),
        other => other.to_owned(),
    }
}

/// The editor, as the host posts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Editor {
    pub draft: Draft,
    /// A custom shortcut's editor (Edit Shortcut), not a new one.
    pub editing: bool,
    /// Every host the user has, for SCOPE.
    pub hosts: Vec<String>,
    /// How many shortcuts the bar holds now, and at most.
    pub toolbar_count: usize,
    pub limit: usize,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "shortcut", …}`: the shortcut to edit.
    Shortcut(Editor),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("add-shortcut", json)?;
        match op.as_str() {
            "shortcut" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Shortcut)
                .map_err(|e| format!("add-shortcut: shortcut: {e}")),
            other => Err(format!("add-shortcut: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Shortcut(editor) => {
                let mut value = serde_json::to_value(editor).expect("an editor serializes");
                value["op"] = json!("shortcut");
                value.to_string()
            }
        }
    }
}

/// A quick key: its button, and what it fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub label: &'static str,
    pub chip: &'static str,
    pub mods: &'static [&'static str],
    pub key: &'static str,
    pub payload: &'static str,
    pub summary: &'static str,
}

const fn combo(
    label: &'static str,
    chip: &'static str,
    mods: &'static [&'static str],
    key: &'static str,
    summary: &'static str,
) -> Preset {
    Preset {
        label,
        chip,
        mods,
        key,
        payload: "",
        summary,
    }
}

const fn sequence(
    label: &'static str,
    chip: &'static str,
    payload: &'static str,
    summary: &'static str,
) -> Preset {
    Preset {
        label,
        chip,
        mods: &[],
        key: "",
        payload,
        summary,
    }
}

const CTRL: &[&str] = &["ctrl"];

/// The quick keys, by group (`presetGroups`): tmux prefixes, control
/// chords, special and navigation keys, F-keys.
pub const PRESETS: [(&str, &[Preset]); 5] = [
    (
        "PREFIX",
        &[
            combo("PREFIX", "C-b", CTRL, "b", "multiplexer prefix"),
            combo("C-c", "^C", CTRL, "c", "Interrupt"),
            combo("C-d", "^D", CTRL, "d", "EOF"),
            combo("C-z", "^Z", CTRL, "z", "Suspend"),
            combo("C-\\", "^\\", CTRL, "\\", "Quit"),
            combo("CLEAR", "^L", CTRL, "l", "Clear"),
        ],
    ),
    (
        "CONTROL",
        &[
            combo("C-a", "^A", CTRL, "a", "Line start"),
            combo("C-e", "^E", CTRL, "e", "Line end"),
            combo("C-r", "^R", CTRL, "r", "Search"),
            combo("C-w", "^W", CTRL, "w", "Del word"),
            combo("C-u", "^U", CTRL, "u", "Del line"),
            combo("C-k", "^K", CTRL, "k", "Kill EOL"),
            combo("C-y", "^Y", CTRL, "y", "Yank"),
            combo("C-p", "^P", CTRL, "p", "Prev"),
            combo("C-n", "^N", CTRL, "n", "Next"),
        ],
    ),
    (
        "SPECIAL",
        &[
            combo("TAB", "tab", &[], "tab", "Tab"),
            sequence("S-Tab", "⇤", "\\e[Z", "Back-tab"),
            combo("Enter", "⏎", &[], "return", "Return"),
            combo("Esc", "esc", &[], "esc", "Escape"),
            sequence("Del", "⌦", "\\e[3~", "Forward delete"),
            sequence("BSpace", "⌫", "\u{7f}", "Backspace"),
        ],
    ),
    (
        "NAVIGATION",
        &[
            combo("←", "←", &[], "left", "Left"),
            combo("↓", "↓", &[], "down", "Down"),
            combo("↑", "↑", &[], "up", "Up"),
            combo("→", "→", &[], "right", "Right"),
            sequence("Home", "Home", "\\e[H", "Home"),
            sequence("End", "End", "\\e[F", "End"),
            sequence("PgUp", "PgUp", "\\e[5~", "Page up"),
            sequence("PgDn", "PgDn", "\\e[6~", "Page down"),
        ],
    ),
    (
        "F-KEYS",
        &[
            sequence("F1", "F1", "\\eOP", ""),
            sequence("F2", "F2", "\\eOQ", ""),
            sequence("F3", "F3", "\\eOR", ""),
            sequence("F4", "F4", "\\eOS", ""),
            sequence("F5", "F5", "\\e[15~", ""),
            sequence("F6", "F6", "\\e[17~", ""),
            sequence("F7", "F7", "\\e[18~", ""),
            sequence("F8", "F8", "\\e[19~", ""),
            sequence("F9", "F9", "\\e[20~", ""),
            sequence("F10", "F10", "\\e[21~", ""),
            sequence("F11", "F11", "\\e[23~", ""),
            sequence("F12", "F12", "\\e[24~", ""),
        ],
    ),
];

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// What tells the host.
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The page's state.
#[derive(Clone, Copy)]
struct State {
    draft: Signal<Option<Draft>>,
    editor: Signal<Option<Editor>>,
    /// Moves when the page fills a field itself (a quick key): the fields
    /// start again from the draft.
    fields: Signal<u64>,
}

impl State {
    fn read<T: Default>(self, f: impl FnOnce(&Draft) -> T) -> T {
        self.draft.with(|d| d.as_ref().map(f).unwrap_or_default())
    }
}

/// The chip label's and a key's most characters (`prefix(6)`).
const MAX_CHARS: usize = 6;

/// Add Shortcut, or a custom shortcut's editor.
#[component]
pub fn AddShortcutScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = State {
        draft: signal(None),
        editor: signal(None),
        fields: signal(0),
    };
    channel::hear(
        channel.as_ref(),
        Screen::AddShortcut,
        Message::parse,
        move |Message::Shortcut(editor)| {
            state.draft.set(Some(editor.draft.clone()));
            state.editor.set(Some(editor));
            state.fields.update(|f| *f += 1);
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.draft.with(Option::is_some));
    let editing = computed(move || state.editor.with(|e| e.as_ref().is_some_and(|e| e.editing)));
    let can_save = computed(move || state.read(Draft::can_save));
    // Each change: the preview takes it, the host hears the whole draft.
    let change: Change = {
        let tell = tell.clone();
        Rc::new(move |f: &dyn Fn(&mut Draft)| {
            state.draft.update(|d| {
                if let Some(d) = d {
                    f(d);
                }
            });
            let draft = state.draft.get_untracked();
            if let Some(draft) = draft {
                tell(json!({"ev": "change", "draft": draft}));
            }
        })
    };
    let kind = computed(move || state.read(|d| d.kind.clone()));
    let body = {
        let change = change.clone();
        move || {
            let change = change.clone();
            page::page(
                bar,
                scrolled,
                "add-shortcut-list",
                [
                    preview_section(state),
                    type_section(state, change.clone()),
                    dynamic({
                        let change = change.clone();
                        move || {
                            let combo = kind.get() == "keyCombo";
                            let change = change.clone();
                            untracked(move || {
                                combo.then(|| {
                                    view()
                                        .w_full()
                                        .gap(px(page::BETWEEN))
                                        .child(quick_keys_section(state, change.clone()))
                                        .child(trigger_section(state, change))
                                })
                            })
                        }
                    })
                    .into_element(),
                    action_section(state, change.clone()),
                    display_section(state, change.clone()),
                    scope_section(state, change.clone()),
                    // Room for the keyboard while it is up, so the last
                    // fields scroll above it.
                    view().w_full().pb_keyboard().into_element(),
                ],
            )
            .into_element()
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("add-shortcut")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: Box::new(move || {
                if editing.get() {
                    t!("Edit Shortcut")
                } else {
                    t!("Add Shortcut")
                }
            }),
            leading: Some(nav_bar::BarButton {
                label: t!("Cancel"),
                content: nav_bar::BarContent::Close,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "shortcut-cancel",
                on_press: Box::new({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "cancel"}))
                }),
                menu: None,
            }),
            trailing: vec![nav_bar::BarButton {
                label: t!("Save"),
                content: nav_bar::BarContent::Confirm,
                enabled: can_save,
                emphasized: true,
                test_id: "shortcut-save",
                on_press: Box::new(move || {
                    dismiss_keyboard();
                    tell(json!({"ev": "save"}));
                }),
                menu: None,
            }],
            on_back: Box::new(|| {}),
            scrolled,
        }))
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// What applies a change to the draft and tells the host.
type Change = Rc<dyn Fn(&dyn Fn(&mut Draft))>;

/// `text` cut to its first six characters.
fn capped(text: &str) -> String {
    text.chars().take(MAX_CHARS).collect()
}

/// A footnote under a section's card, on the row's inset.
fn note(words: String) -> AnyElement {
    view()
        .px(px(PADDING_X))
        .child(
            TextToken::Footnote
                .apply(text(words))
                .text_color(move || token("muted-foreground")),
        )
        .into_element()
}

// --- Preview ------------------------------------------------------------------

/// PREVIEW: the key as the bar will show it, what it does, and the slot it
/// takes, on the terminal's ground.
fn preview_section(state: State) -> AnyElement {
    let label = move || -> SharedString {
        let label = state.read(|d| d.chip_label.clone());
        if label.is_empty() {
            "—".into()
        } else {
            label.into()
        }
    };
    let slot = move || {
        state.editor.with(|e| {
            e.as_ref()
                .map(|e| t!("SLOT %lld/%lld", arg1 = e.toolbar_count, arg2 = e.limit))
                .unwrap_or_default()
        })
    };
    let shown = view()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .p(px(12.0))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("terminal-bg"))
        .test_id("shortcut-preview")
        // The key as the bar will show it, display only (no press handler):
        // the preview's words say what it does.
        .child(dynamic(move || {
            let words = label();
            untracked(move || key(words).armed(|| true))
        }))
        .child(
            TextToken::MonoSubhead
                .apply(text(move || state.read(Draft::preview_summary)))
                .text_color(move || token("key-foreground"))
                .number_of_lines(1u32)
                .flex_shrink(1.0),
        )
        .child(view().flex_1())
        .child(
            TextToken::MonoCaption
                .apply(text(slot))
                .text_color(move || token("muted-foreground"))
                .number_of_lines(1u32)
                .flex_shrink_0(),
        );
    view()
        .w_full()
        .gap(px(8.0))
        .child(shown)
        .child(note(t!(
            "Live preview of your input below. Saved shortcuts land in the Custom group of the Shortcuts toolbar."
        )))
        .into_element()
}

// --- Type -----------------------------------------------------------------------

/// The kinds a user makes, in TYPE's order.
const KINDS: [&str; 3] = ["keyCombo", "text", "command"];

/// TYPE: Key Combo, Text or Command.
fn type_section(state: State, change: Change) -> AnyElement {
    view()
        .w_full()
        .gap(px(8.0))
        .child(
            segmented(
                [t!("Key Combo"), t!("Text"), t!("Command")],
                move || state.read(|d| KINDS.iter().position(|k| *k == d.kind)),
                move |index| {
                    if let Some(kind) = KINDS.get(index) {
                        change(&|d: &mut Draft| d.kind = (*kind).to_owned());
                    }
                },
            )
            .label(t!("TYPE"))
            .test_id("shortcut-type"),
        )
        .child(note(t!(
            "Key Combo: sends modifiers + key. Text: types a string as-is. Command: submits one command with Return."
        )))
        .into_element()
}

// --- Quick keys -------------------------------------------------------------------

/// QUICK KEYS: the presets' groups, each its chips, wrapping (a chip is as
/// wide as its words, at every text size: none is cut short).
fn quick_keys_section(state: State, change: Change) -> AnyElement {
    let groups = view()
        .w_full()
        .gap(px(12.0))
        .px(px(PADDING_X))
        .py(px(12.0))
        .children(PRESETS.iter().map(|(title, presets)| {
            let change = change.clone();
            view()
                .gap(px(8.0))
                .child(
                    TextToken::Caption
                        .apply(text(group_name(title)))
                        .text_color(move || token("muted-foreground")),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(8.0))
                        .children(presets.iter().map(move |preset| {
                            let change = change.clone();
                            // Worded while the page draws, in its language
                            // (B234).
                            let summary = describe(preset.summary);
                            chip(preset.label)
                                .mono()
                                .on_press(move || {
                                    change(&|d: &mut Draft| d.apply_described(preset, &summary));
                                    state.fields.update(|f| *f += 1);
                                })
                                .test_id(&format!("shortcut-preset-{}", preset.label))
                                .into_element()
                        })),
                )
        }));
    section()
        .header(t!("QUICK KEYS"))
        .footer(t!(
            "Tap a preset to fill the trigger — tmux prefixes, control chords, special & navigation keys, F-keys. Then tweak the chip label / color below."
        ))
        .item(groups.into_element(), Inset::Content)
        .into_element()
}

// --- Trigger ------------------------------------------------------------------------

/// TRIGGER: the four modifiers, each a chip that is on or off, and the main
/// key; what the combo sends, once it sends something.
fn trigger_section(state: State, change: Change) -> AnyElement {
    let modifiers = view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap(px(8.0))
        .px(px(PADDING_X))
        .py(px(PADDING_Y))
        .children(MODIFIERS.iter().map(|(id, symbol)| {
            let change = change.clone();
            let on = move || state.read(|d| d.modifiers.iter().any(|m| m == id));
            chip(modifier_name(id, symbol))
                .variant(ChipVariant::Toggle)
                .selected(on)
                .on_press(move || {
                    change(&|d: &mut Draft| {
                        if let Some(i) = d.modifiers.iter().position(|m| m == id) {
                            d.modifiers.remove(i);
                        } else {
                            d.modifiers.push((*id).to_owned());
                        }
                    })
                })
                .test_id(&format!("shortcut-modifier-{id}"))
                .into_element()
        }));
    let main_key = remount(move || state.fields.get(), {
        let change = change.clone();
        move || {
            let change = change.clone();
            field(
                t!("Main key"),
                text_input()
                    .mono()
                    .placeholder("b")
                    .max_length(MAX_CHARS as u32)
                    .default_value(move || state.read(|d| d.key.clone()))
                    .on_change(move |text| {
                        let text = capped(&text);
                        change(&|d: &mut Draft| d.key = text.clone());
                    }),
            )
            .test_id("shortcut-key")
            .into_element()
        }
    });
    let cap = computed(move || state.read(Draft::combo_cap));
    section()
        .header(t!("TRIGGER"))
        .footer(t!(
            "Tap modifiers to toggle them and enter a single character as the main key; the mobile keyboard triggers the raw scancode automatically."
        ))
        .item(modifiers.into_element(), Inset::Content)
        .item(main_key, Inset::Content)
        .row(
            row(t!("Sends"))
                .value_with(move || {
                    let cap = cap.get();
                    if cap.is_empty() { "—".into() } else { cap.into() }
                })
                .mono_value()
                .test_id("shortcut-combo"),
        )
        .into_element()
}

/// A modifier's chip: its symbol and its name, as the system's keyboard
/// settings name it.
fn modifier_name(id: &str, symbol: &str) -> String {
    match id {
        "ctrl" => t!("⌃ Control"),
        "alt" => t!("⌥ Option"),
        "shift" => t!("⇧ Shift"),
        "cmd" => t!("⌘ Command"),
        _ => symbol.to_owned(),
    }
}

// --- Action -----------------------------------------------------------------------

/// The payload's least height: a few lines of text.
const PAYLOAD_MIN_HEIGHT: f32 = 64.0;

/// ACTION: the payload (what a text types, the command, a combo's escape
/// sequence), Append Return and Repeat on hold.
fn action_section(state: State, change: Change) -> AnyElement {
    let set = |f: fn(&mut Draft, bool)| {
        let change = change.clone();
        move |on: bool| change(&move |d: &mut Draft| f(d, on))
    };
    let payload = remount(move || state.fields.get(), {
        let change = change.clone();
        move || {
            let change = change.clone();
            field(
                t!("Payload"),
                Payload {
                    start: state.read(|d| d.payload.clone()),
                    on_change: Rc::new(move |text: String| {
                        change(&|d: &mut Draft| d.payload = text.clone())
                    }),
                },
            )
            .layout(FieldLayout::Stacked)
            .test_id("shortcut-payload-field")
            .into_element()
        }
    });
    section()
        .header(t!("ACTION"))
        .footer(t!(
            "In Key Combo mode the payload is the escape sequence sent to the PTY, honoring the transport transcription rules."
        ))
        .item(payload, Inset::Content)
        .row(switch_row(
            t!("Append Return"),
            t!("Automatically appends ⏎ to submit"),
            move || state.read(|d| d.append_return),
            set(|d, on| d.append_return = on),
            "shortcut-append-return",
        ))
        .row(switch_row(
            t!("Repeat on hold"),
            t!("Repeats at 30/s after a 0.4s hold"),
            move || state.read(|d| d.repeat_on_hold),
            set(|d, on| d.repeat_on_hold = on),
            "shortcut-repeat",
        ))
        .into_element()
}

/// A switch row: `title` over `subtitle`, on while `on()`.
fn switch_row(
    title: String,
    subtitle: String,
    on: impl Fn() -> bool + Clone + 'static,
    on_change: impl Fn(bool) + 'static,
    test_id: &'static str,
) -> Row {
    row(title.clone())
        .subtitle(subtitle)
        .trailing(switch(on, on_change).label(title).test_id(test_id))
}

/// The payload's editor: several lines, mono, no autocorrection; Return is
/// a new line.
struct Payload {
    start: String,
    on_change: Rc<dyn Fn(String)>,
}

impl FieldControl for Payload {
    fn handle(&self) -> ElementRef {
        element_ref()
    }

    fn build(&self, control: &ControlState) -> AnyElement {
        let on_change = self.on_change.clone();
        let spoken = control.spoken.clone();
        let focused = control.focused;
        let input = nsh_core::element::input()
            .multiline(true)
            .w_full()
            .min_h(px(PAYLOAD_MIN_HEIGHT))
            .default_value(self.start.clone())
            .placeholder(t!("Payload (text or command)"))
            .placeholder_color(move || color(token("subtle-foreground")))
            // An input follows NSH's font scale on its own: it is given the
            // style's size at the default text size.
            .text_size(px(TextToken::MonoSubhead.size(1.0)))
            .font_family("monospace")
            .text_color(move || token("foreground"))
            .caret_color(move || color(token("primary")))
            .keyboard_appearance(move || {
                if dark() {
                    KeyboardAppearance::Dark
                } else {
                    KeyboardAppearance::Light
                }
            })
            .auto_correct(false)
            .auto_capitalize(AutoCapitalize::None)
            .scrollbar(false)
            .label(move || spoken())
            .element_ref(&control.handle)
            .on_focus(move || focused.set(true))
            .on_blur(move || focused.set(false))
            .on_change(move |text: &str| on_change(text.to_owned()));
        input.test_id("shortcut-payload").into_element()
    }
}

// --- Display ----------------------------------------------------------------------

/// The chip colors: their ids, as the store keeps them, and their names.
const COLORS: [&str; 4] = ["gray", "accent", "mosh", "amber"];

/// A chip color's swatch: its name and color token.
fn color_of(id: &'static str) -> (String, &'static str) {
    match id {
        "gray" => (t!("Gray"), "muted-foreground"),
        "accent" => (t!("Accent"), "primary"),
        "mosh" => (t!("Blue"), "info"),
        _ => (t!("Amber"), "warning"),
    }
}

/// DISPLAY: the chip's label (six characters at most), the description,
/// the color.
fn display_section(state: State, change: Change) -> AnyElement {
    let fields = remount(move || state.fields.get(), {
        let change = change.clone();
        move || {
            let (label_change, summary_change) = (change.clone(), change.clone());
            view()
                .w_full()
                .child(
                    field(
                        t!("Chip Label"),
                        text_input()
                            .mono()
                            .placeholder("⌘B")
                            .max_length(MAX_CHARS as u32)
                            .default_value(move || state.read(|d| d.chip_label.clone()))
                            .on_change(move |text| {
                                let text = capped(&text);
                                label_change(&|d: &mut Draft| d.chip_label = text.clone());
                            }),
                    )
                    .description(t!("At most 6 characters."))
                    .test_id("shortcut-chip-label"),
                )
                .child(separator(Inset::Content.points()))
                .child(
                    field(
                        t!("Description"),
                        text_input()
                            .placeholder(t!("multiplexer prefix"))
                            .default_value(move || state.read(|d| d.summary.clone()))
                            .on_change(move |text| {
                                summary_change(&|d: &mut Draft| d.summary = text.clone());
                            }),
                    )
                    .test_id("shortcut-description"),
                )
                .into_element()
        }
    });
    let colors = view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap(px(16.0))
        .px(px(PADDING_X))
        .py(px(12.0))
        .children(COLORS.map(|id| {
            let (name, source) = color_of(id);
            let change = change.clone();
            swatch(name, move || token(source))
                .selected(move || state.read(|d| d.color_id == id))
                .on_press(move || change(&|d: &mut Draft| d.color_id = id.to_owned()))
                .test_id(&format!("shortcut-color-{id}"))
                .into_element()
        }));
    section()
        .header(t!("DISPLAY"))
        .footer(t!(
            "Chip Label is capped at 6 characters; Description appears in the edit list to keep shortcuts recognizable."
        ))
        .item(fields, Inset::Content)
        .item(colors.into_element(), Inset::Content)
        .into_element()
}

// --- Scope --------------------------------------------------------------------------

/// SCOPE (optional): a chip for each host, on for those the shortcut shows
/// on, and Only in a multiplexer.
fn scope_section(state: State, change: Change) -> AnyElement {
    // The user's hosts, then any the shortcut is bound to that is gone since
    // (renamed or deleted): shown on, so the binding can come off — bound
    // only to hosts that are gone, the shortcut is in no toolbar at all,
    // which "available everywhere" would hide.
    let hosts = computed(move || {
        let mut hosts = state
            .editor
            .with(|e| e.as_ref().map(|e| e.hosts.clone()).unwrap_or_default());
        let gone: Vec<String> = state.read(|d| {
            d.scope_hosts
                .iter()
                .filter(|h| !hosts.contains(h))
                .cloned()
                .collect()
        });
        hosts.extend(gone);
        hosts
    });
    let chips = dynamic({
        let change = change.clone();
        move || {
            let hosts = hosts.get();
            let change = change.clone();
            untracked(move || {
                view()
                    .w_full()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .px(px(PADDING_X))
                    .py(px(PADDING_Y))
                    .children(hosts.into_iter().map(move |host| {
                        let change = change.clone();
                        let on = {
                            let host = host.clone();
                            move || state.read(|d| d.scope_hosts.contains(&host))
                        };
                        let id = format!("shortcut-host-{host}");
                        chip(host.clone())
                            .variant(ChipVariant::Toggle)
                            .mono()
                            .selected(on)
                            .on_press(move || {
                                change(&|d: &mut Draft| {
                                    if let Some(i) = d.scope_hosts.iter().position(|h| *h == host) {
                                        d.scope_hosts.remove(i);
                                    } else {
                                        d.scope_hosts.push(host.clone());
                                    }
                                })
                            })
                            .test_id(&id)
                            .into_element()
                    }))
            })
        }
    });
    section()
        .header(t!("SCOPE"))
        .footer(t!(
            "Available everywhere when nothing is selected. When bound, it appears only in the toolbar of the selected hosts, saving slots in the 12-slot budget."
        ))
        .item(chips.into_element(), Inset::Content)
        .row(switch_row(
            t!("Only in a multiplexer"),
            t!("Show only while the session runs tmux or herdr"),
            move || state.read(|d| d.only_in_tmux),
            move |on| change(&move |d: &mut Draft| d.only_in_tmux = on),
            "shortcut-only-multiplexer",
        ))
        .into_element()
}

/// How many columns of at least `min` points, `GRID_SPACING` apart, fit
/// `width` (SwiftUI's adaptive grid item).
pub fn grid_columns(width: f32, min: f32) -> usize {
    (((width + GRID_SPACING) / (min + GRID_SPACING)).floor() as usize).max(1)
}

/// The room between quick keys.
const GRID_SPACING: f32 = 8.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// B234, from the app's Swift suite: every built-in shortcut's name is
    /// in each language the app ships, read from the shared catalog.
    #[test]
    fn every_built_in_is_named_in_each_language() {
        use moshpit_store::shortcuts::ShortcutStore;
        // Proper names of keys a language may keep as they are.
        let kept = ["Home", "End", "Escape", "Return"];
        // Words a language spells exactly as English does, by its catalog
        // entry (the key name Control in several).
        let alike = |language: &str| -> &'static [&'static str] {
            match language {
                "de" | "es" | "ko" | "pt-BR" => &["Control"],
                _ => &[],
            }
        };
        let english: Vec<String> = ShortcutStore::builtins().into_iter().map(|s| s.summary).collect();
        assert!(english.len() >= 30, "{english:?}");
        for language in ["zh-Hans", "zh-Hant", "ja", "ko", "de", "es", "fr", "pt-BR"] {
            for text in &english {
                let shown = describe_in(text, language);
                assert!(!shown.is_empty(), "{language}: {text}");
                if !kept.contains(&text.as_str()) && !alike(language).contains(&text.as_str()) {
                    assert_ne!(&shown, text, "{language}: {text:?} is still English");
                }
            }
        }
        // And in English, as stored.
        for text in &english {
            let shown = describe_in(text, "en");
            assert!(shown == *text || text == "Tab", "{text} → {shown}");
        }
    }

    #[test]
    fn chinese_reads_as_chinese() {
        assert_eq!(describe_in("Interrupt (SIGINT)", "zh-Hans"), "中断（SIGINT）");
        assert_eq!(describe_in("Tab", "zh-Hans"), "Tab 键", "the key, not the multiplexer's tab");
        assert_eq!(describe_in("Clear line before cursor", "zh-Hans"), "删掉光标前的内容");
    }

    /// A built-in is shown translated and stored as it was; the user's own
    /// words are their own, even when they are a built-in's.
    #[test]
    fn only_a_built_in_is_put_into_words() {
        use moshpit_store::shortcuts::ShortcutStore;
        let builtin = ShortcutStore::builtins()
            .into_iter()
            .find(|s| s.summary == "Interrupt (SIGINT)")
            .expect("a built-in ^C");
        assert!(builtin.is_builtin);
        assert_eq!(shortcut_name(&builtin, "zh-Hans"), "中断（SIGINT）");
        assert_eq!(builtin.summary, "Interrupt (SIGINT)");
        let mut mine = builtin.clone();
        mine.is_builtin = false;
        assert_eq!(shortcut_name(&mine, "zh-Hans"), "Interrupt (SIGINT)");
    }

    fn draft() -> Draft {
        Draft {
            kind: "keyCombo".into(),
            color_id: "accent".into(),
            ..Draft::default()
        }
    }

    #[test]
    fn messages_round_trip() {
        let message = Message::Shortcut(Editor {
            draft: Draft {
                modifiers: vec!["ctrl".into()],
                key: "b".into(),
                chip_label: "C-b".into(),
                scope_hosts: vec!["nas".into()],
                ..draft()
            },
            editing: true,
            hosts: vec!["nas".into(), "m1-pro".into()],
            toolbar_count: 7,
            limit: 12,
        });
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let error = Message::parse(r#"{"op":"shortcut","draft":{"kind":"text"},"editing":false,"hosts":[],"toolbarCount":0,"limit":12}"#).unwrap_err();
        assert!(error.contains("modifiers"), "{error}");
    }

    #[test]
    fn the_combo_cap_writes_modifiers_in_macos_order() {
        let d = Draft {
            modifiers: vec!["cmd".into(), "ctrl".into(), "shift".into()],
            key: "b".into(),
            ..draft()
        };
        assert_eq!(d.combo_cap(), "⌃ ⇧ ⌘ B");
        assert_eq!(draft().combo_cap(), "");
        assert_eq!(
            Draft {
                key: "esc".into(),
                ..draft()
            }
            .combo_cap(),
            "ESC"
        );
    }

    #[test]
    fn save_needs_a_label_and_something_to_send() {
        assert!(!draft().can_save());
        assert!(
            !Draft {
                chip_label: "x".into(),
                ..draft()
            }
            .can_save()
        );
        assert!(
            Draft {
                chip_label: "x".into(),
                key: "a".into(),
                ..draft()
            }
            .can_save()
        );
        assert!(
            Draft {
                chip_label: "x".into(),
                payload: "\\e[Z".into(),
                ..draft()
            }
            .can_save()
        );
        let text = Draft {
            kind: "text".into(),
            chip_label: "x".into(),
            key: "a".into(),
            ..draft()
        };
        assert!(!text.can_save(), "a text needs its payload");
        assert!(
            Draft {
                payload: "ls".into(),
                ..text
            }
            .can_save()
        );
    }

    #[test]
    fn a_preset_fills_the_trigger_and_keeps_a_written_summary() {
        let prefix = &PRESETS[0].1[0];
        let mut d = draft();
        d.apply(prefix);
        assert_eq!(d.modifiers, ["ctrl"]);
        assert_eq!(d.key, "b");
        assert_eq!(d.chip_label, "C-b");
        assert_eq!(d.summary, "multiplexer prefix");
        let back_tab = &PRESETS[2].1[1];
        d.apply(back_tab);
        assert_eq!(d.modifiers, Vec::<String>::new());
        assert_eq!(d.payload, "\\e[Z");
        assert_eq!(d.summary, "multiplexer prefix", "kept");
        assert_eq!(PRESETS.iter().map(|g| g.1.len()).sum::<usize>(), 41);
    }

    #[test]
    fn the_grid_fits_columns_as_swiftuis_adaptive_items() {
        assert_eq!(grid_columns(516.0, 72.0), 6);
        assert_eq!(grid_columns(300.0, 72.0), 3);
        assert_eq!(grid_columns(516.0, 104.0), 4);
        assert_eq!(grid_columns(10.0, 72.0), 1);
    }

    #[test]
    fn six_characters_at_most() {
        assert_eq!(capped("abcdefgh"), "abcdef");
        assert_eq!(capped("⌃⇧⌘B"), "⌃⇧⌘B");
    }
}
