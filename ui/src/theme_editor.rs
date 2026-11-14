//! Settings › Theme › the editor of a custom terminal theme
//! (`Moshpit/UI/Settings/ThemeEditorView.swift`) in NSH, in its sheet.
//!
//! The preview at the top is the point of the page: a fake session in the
//! draft's colors, so every change is judged in context. The host owns the
//! draft and the store: it posts the theme being edited ([`Message`]),
//! `{"op": "theme", "theme": {"name", "background", "foreground",
//! "cursor", "ansi": [8], "bright": [8, null where derived]},
//! "overrideBright": false}`; the page then keeps its own copy for the
//! preview and tells the host each change, which applies it to its draft
//! (and saves that on Save):
//!
//! - `{"ev": "name", "value": …}` as the name changes;
//! - `{"ev": "color", "slot": "background" | "foreground" | "cursor" |
//!   "black" … "white" | "brightBlack" … "brightWhite", "value":
//!   "#RRGGBB"}` for a color picked in the system's color picker (each row's
//!   color well is UIKit's, the host's component);
//! - `{"ev": "override-bright", "value": …}`: the switch; off drops the
//!   bright overrides, which derive from the eight again;
//! - `{"ev": "copy-json"}` (the bar's menu), `{"ev": "save"}`,
//!   `{"ev": "cancel"}`.
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.10 (R14), on the `ui` layer: the
//! preview, the name's field, then Terminal, ANSI colors and Bright colors,
//! each color a row with its code and the host's color well; the bright
//! rows under the switch while it is on. Cancel, the menu and Save in the
//! bar, the title the name (New Theme while it is empty). The accessibility
//! identifiers name every control (`theme-editor-name`, `theme-color-<slot>`,
//! `theme-editor-bright`, `theme-editor-more`, `theme-editor-save`,
//! `theme-editor-cancel`).

use std::{rc::Rc, sync::Arc};

use moshpit_store::themes::TerminalTheme;
use nsh_core::{color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    face,
    nav_bar::{self, BarMetrics},
    screen::Screen,
    theme,
    theme_gallery::eight,
    ui::{
        control::switch,
        field::{field, text_input},
        menu::MenuEntry,
        page::page,
        row::{Row, row as list_row},
        section::{Inset, SectionVariant, section},
        swatch::color_well,
        tokens::{number, token},
    },
};

/// The theme being edited, as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub background: Rgba,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub foreground: Rgba,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub cursor: Rgba,
    /// ANSI 0–7, black to white.
    #[serde(with = "eight")]
    pub ansi: [Rgba; 8],
    /// ANSI 8–15's overrides, black to white: `None` derives the slot from
    /// its base color.
    #[serde(with = "overrides")]
    pub bright: [Option<Rgba>; 8],
}

/// Eight optional colors: `#RRGGBB` or `null`.
mod overrides {
    use nsh_core::color::Rgba;
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};

    pub fn serialize<S: Serializer>(
        colors: &[Option<Rgba>; 8],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(colors.len()))?;
        for color in colors {
            seq.serialize_element(&color.map(crate::theme::hex_string))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[Option<Rgba>; 8], D::Error> {
        let texts = Vec::<Option<String>>::deserialize(deserializer)?;
        let colors = texts
            .iter()
            .map(|text| match text {
                None => Ok(None),
                Some(text) => crate::theme::parse_hex(text).map(Some).ok_or_else(|| {
                    serde::de::Error::custom(format!("{text:?} is not #RRGGBB or #RRGGBBAA"))
                }),
            })
            .collect::<Result<Vec<_>, D::Error>>()?;
        colors.try_into().map_err(|c: Vec<Option<Rgba>>| {
            serde::de::Error::custom(format!("{} bright colors, not 8", c.len()))
        })
    }
}

/// A color of a theme the page edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Background,
    Foreground,
    Cursor,
    /// ANSI 0–7.
    Ansi(usize),
    /// ANSI 8–15.
    Bright(usize),
}

/// The ANSI colors' names, as the theme's JSON names them.
const ANSI: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

impl Slot {
    /// The terminal's three, in the TERMINAL group's order.
    pub const TERMINAL: [Slot; 3] = [Slot::Background, Slot::Foreground, Slot::Cursor];

    /// What the page and the host call it: the key of the theme's JSON
    /// (`background`, `red`, `brightRed`).
    pub fn id(self) -> String {
        match self {
            Slot::Background => "background".into(),
            Slot::Foreground => "foreground".into(),
            Slot::Cursor => "cursor".into(),
            Slot::Ansi(i) => ANSI[i].into(),
            Slot::Bright(i) => {
                let name = ANSI[i];
                format!("bright{}{}", name[..1].to_uppercase(), &name[1..])
            }
        }
    }

    /// The slot `id` names.
    pub fn named(id: &str) -> Option<Slot> {
        Self::TERMINAL
            .into_iter()
            .chain((0..8).map(Slot::Ansi))
            .chain((0..8).map(Slot::Bright))
            .find(|slot| slot.id() == id)
    }

    /// Its name on the page, in the app's language.
    pub fn label(self) -> String {
        match self {
            Slot::Background => t!("Background"),
            Slot::Foreground => t!("Foreground"),
            Slot::Cursor => t!("Cursor"),
            Slot::Ansi(0) => t!("Black"),
            Slot::Ansi(1) => t!("Red"),
            Slot::Ansi(2) => t!("Green"),
            Slot::Ansi(3) => t!("Yellow"),
            Slot::Ansi(4) => t!("Blue"),
            Slot::Ansi(5) => t!("Magenta"),
            Slot::Ansi(6) => t!("Cyan"),
            Slot::Ansi(_) => t!("White"),
            Slot::Bright(0) => t!("Bright Black"),
            Slot::Bright(1) => t!("Bright Red"),
            Slot::Bright(2) => t!("Bright Green"),
            Slot::Bright(3) => t!("Bright Yellow"),
            Slot::Bright(4) => t!("Bright Blue"),
            Slot::Bright(5) => t!("Bright Magenta"),
            Slot::Bright(6) => t!("Bright Cyan"),
            Slot::Bright(_) => t!("Bright White"),
        }
    }

    /// The accessibility identifier of its color well.
    pub fn test_id(self) -> String {
        format!("theme-color-{}", self.id())
    }
}

impl Draft {
    /// The color of `slot`: a bright one is its override, else derived.
    pub fn color(&self, slot: Slot) -> Rgba {
        match slot {
            Slot::Background => self.background,
            Slot::Foreground => self.foreground,
            Slot::Cursor => self.cursor,
            Slot::Ansi(i) => self.ansi[i],
            Slot::Bright(i) => self.bright[i].unwrap_or_else(|| derived_bright(self.ansi[i])),
        }
    }

    /// `slot` is `color` now (a bright one, an override).
    pub fn set(&mut self, slot: Slot, color: Rgba) {
        match slot {
            Slot::Background => self.background = color,
            Slot::Foreground => self.foreground = color,
            Slot::Cursor => self.cursor = color,
            Slot::Ansi(i) => self.ansi[i] = color,
            Slot::Bright(i) => self.bright[i] = Some(color),
        }
    }

    /// Whether any bright slot is overridden.
    pub fn overrides_bright(&self) -> bool {
        self.bright.iter().any(Option::is_some)
    }
}

/// A bright slot derived from its base color: 32% of the way to white
/// (`TerminalTheme.derivedBright`, the store's rule).
pub fn derived_bright(base: Rgba) -> Rgba {
    let hex = TerminalTheme::derived_bright(&theme::hex_string(base)[1..7]);
    theme::parse_hex(&format!("#{hex}")).unwrap_or(base)
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EditorState {
    /// The theme as the host's draft has it.
    pub theme: Draft,
    /// Whether the bright rows show (the switch).
    pub override_bright: bool,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "theme", …}`: the theme to edit.
    Theme(EditorState),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("theme-editor", json)?;
        match op.as_str() {
            "theme" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Theme)
                .map_err(|e| format!("theme-editor: theme: {e}")),
            other => Err(format!("theme-editor: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Theme(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("theme");
                value.to_string()
            }
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// What tells the host.
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The theme editor.
#[component]
pub fn ThemeEditorScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let draft = signal(None::<Draft>);
    let overriding = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::ThemeEditor,
        Message::parse,
        move |Message::Theme(state)| {
            overriding.set(state.override_bright);
            draft.set(Some(state.theme));
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || draft.with(Option::is_some));
    let name = computed(move || draft.with(|d| d.as_ref().map(|d| d.name.clone())));
    let title = move || match name.get() {
        Some(name) if !name.is_empty() => name,
        _ => t!("New Theme"),
    };
    let body = {
        let tell = tell.clone();
        move || {
            // The field starts with the name the host posted.
            let start = name.get_untracked().unwrap_or_default();
            page(
                bar,
                scrolled,
                "theme-editor-list",
                groups(draft, overriding, start, tell.clone()),
            )
            .into_element()
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("theme-editor")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: Box::new(title),
            leading: Some(nav_bar::BarButton {
                label: t!("Cancel"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "theme-editor-cancel",
                on_press: Box::new({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "cancel"}))
                }),
                menu: None,
            }),
            trailing: vec![
                nav_bar::BarButton {
                    label: t!("More"),
                    content: nav_bar::BarContent::Symbol("ellipsis.circle"),
                    enabled: computed(|| true),
                    emphasized: false,
                    test_id: "theme-editor-more",
                    on_press: Box::new(|| {}),
                    menu: Some(nav_bar::BarMenu {
                        entries: vec![
                            MenuEntry::new("copy-json", t!("Copy as JSON")).symbol("doc.on.doc"),
                        ],
                        on_select: Box::new({
                            let tell = tell.clone();
                            move |choice: String| tell(json!({"ev": choice}))
                        }),
                    }),
                },
                nav_bar::BarButton {
                    label: t!("Save"),
                    content: nav_bar::BarContent::Text,
                    enabled: computed(|| true),
                    emphasized: true,
                    test_id: "theme-editor-save",
                    on_press: Box::new({
                        let tell = tell.clone();
                        move || {
                            dismiss_keyboard();
                            tell(json!({"ev": "save"}));
                        }
                    }),
                    menu: None,
                },
            ],
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

/// The draft's color of `slot`, black before the host's first post.
fn color_of(draft: Signal<Option<Draft>>, slot: Slot) -> impl Fn() -> Rgba + Clone + 'static {
    move || draft.with(|d| d.as_ref().map_or(Rgba::default(), |d| d.color(slot)))
}

/// What a color row does with a color picked for `slot`: the preview
/// takes it, the host hears it.
fn change(draft: Signal<Option<Draft>>, slot: Slot, tell: &Tell) -> impl Fn(Rgba) + 'static {
    let tell = tell.clone();
    move |color: Rgba| {
        draft.update(|d| {
            if let Some(d) = d {
                d.set(slot, color);
            }
        });
        tell(json!({
            "ev": "color",
            "slot": slot.id(),
            "value": &theme::hex_string(color)[..7],
        }));
    }
}

/// A color's row: its name, its code, and the host's color well (which
/// takes the presses).
fn color_row(draft: Signal<Option<Draft>>, slot: Slot, tell: &Tell) -> Row {
    let color = color_of(draft, slot);
    let code = {
        let color = color.clone();
        move || format!("#{}", &theme::hex_string(color())[1..7]).into()
    };
    list_row(slot.label())
        .value_with(code)
        .mono_value()
        .trailing(color_well(
            color,
            Some(&slot.test_id()),
            slot.label(),
            change(draft, slot, tell),
        ))
}

/// Every section, in order.
fn groups(
    draft: Signal<Option<Draft>>,
    overriding: Signal<bool>,
    start: String,
    tell: Tell,
) -> Vec<AnyElement> {
    let on_name = {
        let tell = tell.clone();
        move |text: String| {
            draft.update(|d| {
                if let Some(d) = d {
                    d.name = text.clone();
                }
            });
            tell(json!({"ev": "name", "value": text}));
        }
    };
    let name = field(
        t!("Name"),
        text_input()
            .placeholder(t!("Theme name"))
            .default_value(move || start.clone())
            .on_change(on_name),
    )
    .test_id("theme-editor-name");
    let colors = |slots: Vec<Slot>| -> Vec<Row> {
        slots
            .into_iter()
            .map(|slot| color_row(draft, slot, &tell))
            .collect()
    };
    let bright = {
        let tell = tell.clone();
        when(
            move || overriding.get(),
            move || {
                section()
                    .rows((0..8).map(|i| color_row(draft, Slot::Bright(i), &tell)))
                    .test_id("theme-editor-bright-rows")
            },
        )
        .into_element()
    };
    vec![
        section()
            .variant(SectionVariant::Plain)
            .item(preview(draft), Inset::Content)
            .into_element(),
        section().item(name, Inset::Content).into_element(),
        section()
            .header(t!("Terminal"))
            .rows(colors(Slot::TERMINAL.to_vec()))
            .into_element(),
        section()
            .header(t!("ANSI colors"))
            .footer(t!("These eight are what shells, diffs and TUIs paint with."))
            .rows(colors((0..8).map(Slot::Ansi).collect()))
            .into_element(),
        section()
            .header(t!("Bright colors"))
            .footer(t!(
                "Bright colors follow the eight above. Many tools draw dim text in bright black, so keep it apart from black."
            ))
            .row(bright_switch(draft, overriding, &tell))
            .into_element(),
        bright,
    ]
}

/// Override bright colors: turning it off drops the overrides, which derive
/// from the eight again.
fn bright_switch(draft: Signal<Option<Draft>>, overriding: Signal<bool>, tell: &Tell) -> Row {
    let label = t!("Override bright colors");
    let tell = tell.clone();
    list_row(label.clone()).trailing(
        switch(
            move || overriding.get(),
            move |on: bool| {
                overriding.set(on);
                if !on {
                    draft.update(|d| {
                        if let Some(d) = d {
                            d.bright = [None; 8];
                        }
                    });
                }
                tell(json!({"ev": "override-bright", "value": on}));
            },
        )
        .label(label)
        .test_id("theme-editor-bright"),
    )
}

/// A run of the preview's text and what colors it.
type Span = (&'static str, Box<dyn Fn() -> Rgba>);

/// How far SwiftUI sets the preview's lines below where a one-line text
/// lands here (measured on the iPad: half a point lower, at every text
/// size, the size being fixed).
const PREVIEW_DROP: f32 = 0.5;

/// The preview's border, a hairline, inside its box.
const BORDER: f32 = 1.0;
/// The preview's padding (`space-4`).
const PREVIEW_PADDING: f32 = 16.0;

/// The size of the preview's text (`.system(size: 12, design:
/// .monospaced)`: a fixed size, which Dynamic Type leaves alone).
const PREVIEW_TEXT: f32 = 12.0;

/// The live preview (`ThemeLivePreview`): a fake session in the draft's
/// colors, its lines chosen for the colors that decide whether a theme is
/// usable (prompt green, path blue, a diff's red and green, a warning's
/// yellow, dim text in bright black). Its words are a terminal's, the same
/// in every language (the app's catalog keeps them verbatim).
fn preview(draft: Signal<Option<Draft>>) -> AnyElement {
    let color = move |slot: Slot| color_of(draft, slot)();
    let fg = move || color(Slot::Foreground);
    let line = |spans: Vec<Span>| {
        let mut line = text("")
            .font_family("monospace")
            .text_size(px(PREVIEW_TEXT))
            .line_height(face::line_height())
            .allow_font_scaling(false)
            .number_of_lines(1u32)
            .h(move || px(face::frame(PREVIEW_TEXT, pixel_ratio())))
            .pt(move || {
                px(
                    face::frame(PREVIEW_TEXT, pixel_ratio()) - face::line(PREVIEW_TEXT)
                        + PREVIEW_DROP,
                )
            });
        for (words, tint) in spans {
            line = line.child(text(words).text_color(move || AdaptiveColor::from(tint())));
        }
        line.into_element()
    };
    let green = move || color(Slot::Ansi(2));
    let red = move || color(Slot::Ansi(1));
    // SwiftUI strokes the border over the padded box; NSH's border is part
    // of the box: the padding gives it its point.
    view()
        .w_full()
        .py(px(PREVIEW_PADDING - BORDER))
        .px(px(PREVIEW_PADDING - BORDER))
        .gap(px(3.0))
        .items_start()
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || AdaptiveColor::from(color(Slot::Background)))
        .border(px(1.0))
        .border_color(move || token("border"))
        .accessible(true)
        .label(t!("Theme preview"))
        .test_id("theme-editor-preview")
        .child(line(vec![
            ("❯ ", Box::new(green)),
            ("git ", Box::new(fg)),
            ("status", Box::new(move || color(Slot::Ansi(6)))),
        ]))
        .child(line(vec![
            ("On branch ", Box::new(fg)),
            ("main", Box::new(move || color(Slot::Ansi(4)))),
        ]))
        .child(line(vec![
            ("+ ", Box::new(green)),
            ("MoshTransport.swift", Box::new(green)),
        ]))
        .child(line(vec![
            ("- ", Box::new(red)),
            ("MoshpitMark.swift", Box::new(red)),
        ]))
        .child(line(vec![
            ("warning: ", Box::new(move || color(Slot::Ansi(3)))),
            ("2 files changed", Box::new(fg)),
        ]))
        .child(line(vec![(
            "hint: use --staged",
            Box::new(move || color(Slot::Bright(0))),
        )]))
        .child(line(vec![
            ("❯ ", Box::new(green)),
            ("█", Box::new(move || color(Slot::Cursor))),
        ]))
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn draft() -> Draft {
        Draft {
            name: "Harbor".into(),
            background: nsh_core::color::rgb(0x050507),
            foreground: nsh_core::color::rgb(0xC9D1D9),
            cursor: nsh_core::color::rgb(0x5FE3D8),
            ansi: [
                0x484F58, 0xFF7B72, 0x3FB950, 0xD29922, 0x58A6FF, 0xBC8CFF, 0x39C5CF, 0xB1BAC4,
            ]
            .map(nsh_core::color::rgb),
            bright: [None; 8],
        }
    }

    #[test]
    fn messages_round_trip() {
        let mut overridden = draft();
        overridden.bright[0] = Some(nsh_core::color::rgb(0x7A7468));
        for state in [
            EditorState {
                theme: draft(),
                override_bright: false,
            },
            EditorState {
                theme: overridden,
                override_bright: true,
            },
        ] {
            let message = Message::Theme(state);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_message_as_written() {
        let message = Message::parse(
            r##"{"op":"theme","theme":{"name":"Harbor","background":"#050507","foreground":"#C9D1D9","cursor":"#5FE3D8","ansi":["#484F58","#FF7B72","#3FB950","#D29922","#58A6FF","#BC8CFF","#39C5CF","#B1BAC4"],"bright":[null,null,null,null,null,null,null,null]},"overrideBright":false}"##,
        );
        assert_eq!(
            message,
            Ok(Message::Theme(EditorState {
                theme: draft(),
                override_bright: false,
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        let with = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut value = serde_json::to_value(EditorState {
                theme: draft(),
                override_bright: false,
            })
            .unwrap();
            value["op"] = json!("theme");
            f(&mut value);
            value.to_string()
        };
        for (json, says) in [
            ("{".to_owned(), "not JSON"),
            (r#"{"op":"themes"}"#.to_owned(), "no op \"themes\""),
            (
                with(&|v| {
                    v.as_object_mut().unwrap().remove("overrideBright");
                }),
                "overrideBright",
            ),
            (
                with(&|v| v["theme"]["bright"] = json!([null])),
                "1 bright colors, not 8",
            ),
            (with(&|v| v["theme"]["cursor"] = json!("teal")), "teal"),
            (with(&|v| v["theme"]["id"] = json!("custom-1")), "id"),
        ] {
            let error = Message::parse(&json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }

    #[test]
    fn slots_are_named_as_the_themes_json_names_them() {
        assert_eq!(Slot::Background.id(), "background");
        assert_eq!(Slot::Ansi(1).id(), "red");
        assert_eq!(Slot::Bright(0).id(), "brightBlack");
        assert_eq!(Slot::Bright(7).id(), "brightWhite");
        for slot in Slot::TERMINAL
            .into_iter()
            .chain((0..8).map(Slot::Ansi))
            .chain((0..8).map(Slot::Bright))
        {
            assert_eq!(Slot::named(&slot.id()), Some(slot));
        }
        assert_eq!(Slot::named("bright"), None);
        assert_eq!(Slot::Ansi(4).test_id(), "theme-color-blue");
    }

    #[test]
    fn a_bright_slot_is_derived_until_overridden() {
        let mut theme = draft();
        // GitHub Dark's black, 32% of the way to white, as the store has it.
        assert_eq!(theme.color(Slot::Bright(0)), nsh_core::color::rgb(0x83878D));
        assert!(!theme.overrides_bright());
        theme.set(Slot::Bright(0), nsh_core::color::rgb(0x112233));
        assert_eq!(theme.color(Slot::Bright(0)), nsh_core::color::rgb(0x112233));
        assert!(theme.overrides_bright());
        // A new base color moves the derived ones with it.
        theme.set(Slot::Ansi(1), nsh_core::color::rgb(0x000000));
        assert_eq!(theme.color(Slot::Bright(1)), nsh_core::color::rgb(0x525252));
    }
}
