//! Settings › Accent › the editor of a custom accent (`AccentEditorView`,
//! `Moshpit/UI/Settings/AccentGalleryView.swift`) in NSH, in its sheet: one
//! color and a name, previewed on a mock of the app's own chrome (a
//! button, a pill, a selected row), so the color is judged where it will
//! appear.
//!
//! The host owns the draft and the store: it posts the accent being edited
//! ([`Message`]), `{"op": "accent", "name": …, "accent": "#RRGGBB"}`; the
//! page keeps its own copy for the preview and tells the host each change,
//! which applies it to its draft (and saves that on Save):
//!
//! - `{"ev": "name", "value": …}` as the name changes;
//! - `{"ev": "color", "value": "#RRGGBB"}` for a color picked in the
//!   system's color picker (the row's color well is UIKit's, the host's
//!   component);
//! - `{"ev": "save"}`, `{"ev": "cancel"}`.
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.10 (R14), on the `ui` layer. The
//! preview draws a button, a badge and a selected row with the tokens the
//! accent derives (§2.3: on the card of the background it derives, the
//! store's `AppTheme.custom`, 95.5% of the way to black), and the accent as
//! text in both appearances (`primary-text`): when it is too pale to read
//! as text in the light one, a note says the app darkens it there. Then the
//! name's field and the color's row with its well; Cancel and Save in the
//! bar, the title the name (New Accent while it is empty). The
//! accessibility identifiers name every control (`accent-editor-name`,
//! `accent-editor-color`, `accent-editor-save`, `accent-editor-cancel`).

use std::{rc::Rc, sync::Arc};

use nsh_core::{color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    theme::{self, AppTheme},
    ui::{
        field::{field, text_input},
        notice::{BannerTone, banner},
        page::page,
        row::{GAP, HEIGHT_MD, PADDING_X, row as list_row},
        section::{Inset, SectionVariant, section},
        swatch::color_well,
        tokens::{self, READABLE, contrast, number, over, token},
        typography::TextToken,
    },
};

/// The accent being edited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccentDraft {
    pub name: String,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub accent: Rgba,
}

/// The screen background an accent derives (`AppTheme.custom`'s
/// `screenBG`: the accent 95.5% of the way to black, the store's rule).
pub fn screen_bg(accent: Rgba) -> Rgba {
    let hex = moshpit_store::themes::blend(&theme::hex_string(accent)[1..7], "000000", 0.955);
    theme::parse_hex(&format!("#{hex}")).unwrap_or(accent)
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "accent", …}`: the accent to edit.
    Accent(AccentDraft),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("accent-editor", json)?;
        match op.as_str() {
            "accent" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Accent)
                .map_err(|e| format!("accent-editor: accent: {e}")),
            other => Err(format!("accent-editor: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Accent(draft) => {
                let mut value = serde_json::to_value(draft).expect("a draft serializes");
                value["op"] = json!("accent");
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

/// The accent editor.
#[component]
pub fn AccentEditorScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let draft = signal(None::<AccentDraft>);
    channel::hear(
        channel.as_ref(),
        Screen::AccentEditor,
        Message::parse,
        move |Message::Accent(next)| draft.set(Some(next)),
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || draft.with(Option::is_some));
    let name = computed(move || draft.with(|d| d.as_ref().map(|d| d.name.clone())));
    let accent = move || {
        draft.with(|d| {
            d.as_ref()
                .map_or(AppTheme::SIGNAL_ROOM.accent, |d| d.accent)
        })
    };
    let title = move || match name.get() {
        Some(name) if !name.is_empty() => name,
        _ => t!("New Accent"),
    };
    let body = {
        let tell = tell.clone();
        move || {
            let start = name.get_untracked().unwrap_or_default();
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
            let on_color = {
                let tell = tell.clone();
                move |color: Rgba| {
                    draft.update(|d| {
                        if let Some(d) = d {
                            d.accent = color;
                        }
                    });
                    tell(json!({"ev": "color", "value": &theme::hex_string(color)[..7]}));
                }
            };
            let code = move || format!("#{}", &theme::hex_string(accent())[1..7]).into();
            page(
                bar,
                scrolled,
                "accent-editor-list",
                [
                    section()
                        .variant(SectionVariant::Plain)
                        .item(preview(accent), Inset::Content)
                        .into_element(),
                    banner(BannerTone::Info, || {
                        t!("As text in the light appearance this color is too pale, so the app darkens it there.")
                            .into()
                    })
                    .shown(move || pale_as_light_text(accent()))
                    .test_id("accent-editor-pale")
                    .into_element(),
                    section()
                        .item(
                            field(
                                t!("Name"),
                                text_input()
                                    .placeholder(t!("Accent name"))
                                    .default_value(move || start.clone())
                                    .on_change(on_name),
                            )
                            .test_id("accent-editor-name"),
                            Inset::Content,
                        )
                        .into_element(),
                    section()
                        .footer(t!(
                            "The pressed state and the background wash are derived from this one color."
                        ))
                        .row(
                            list_row(t!("Color"))
                                .value_with(code)
                                .mono_value()
                                .trailing(color_well(
                                    accent,
                                    Some("accent-editor-color"),
                                    t!("Accent"),
                                    on_color,
                                )),
                        )
                        .into_element(),
                ],
            )
            .into_element()
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("accent-editor")
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
                test_id: "accent-editor-cancel",
                on_press: Box::new({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "cancel"}))
                }),
                menu: None,
            }),
            trailing: vec![nav_bar::BarButton {
                label: t!("Save"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: true,
                test_id: "accent-editor-save",
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

/// The tokens `accent` derives (§2.3), as (light, dark) by name: the
/// theme a custom accent makes (`AppTheme.custom`'s background).
fn derived(accent: Rgba) -> impl Fn(&str) -> (Rgba, Rgba) {
    let accent = Rgba { a: 1.0, ..accent };
    let colors = tokens::colors(&AppTheme {
        accent,
        accent_pressed: accent,
        screen_bg: screen_bg(accent),
        terminal_bg: screen_bg(accent),
    });
    move |name: &str| {
        colors
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, light, dark)| (*light, *dark))
            .unwrap_or_else(|| panic!("no color token {name}"))
    }
}

/// Whether `accent` as it is reads too pale as text in the light
/// appearance (under [`READABLE`] on the page, a card, or its own wash on
/// them), so that `primary-text` there is a darker one (§2.3).
pub fn pale_as_light_text(accent: Rgba) -> bool {
    let accent = Rgba { a: 1.0, ..accent };
    let token = derived(accent);
    let (page, card, soft) = (
        token("background").0,
        token("card").0,
        token("primary-soft").0,
    );
    ![page, card, over(soft, card), over(soft, page)]
        .iter()
        .all(|surface| contrast(accent, *surface) >= READABLE)
}

/// The preview: a button, a badge and a selected row in the accent's
/// tokens on the card of the background it derives, the status colors
/// beside them (fixed on purpose: a clash shows before saving); under them
/// the accent as text in the dark and the light appearance.
fn preview(accent: impl Fn() -> Rgba + Copy + 'static) -> AnyElement {
    let dark = move |name: &'static str| move || AdaptiveColor::from(derived(accent())(name).1);
    let light = move |name: &'static str| move || AdaptiveColor::from(derived(accent())(name).0);
    let connect = view()
        .min_h(px(36.0))
        .px(px(16.0))
        .justify_center()
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .bg(dark("primary"))
        .child(
            TextToken::Body
                .apply(text(t!("Connect")))
                .font_weight(FontWeight::SEMIBOLD)
                .number_of_lines(1u32)
                .text_color(dark("primary-foreground")),
        );
    let badge = view()
        .min_h(px(20.0))
        .px(px(6.0))
        .justify_center()
        .rounded(px(number("radius-xs")))
        .bg(dark("primary-soft"))
        .child(
            TextToken::MonoCaption
                .apply(text("mosh"))
                .number_of_lines(1u32)
                .text_color(dark("primary-text")),
        );
    let status = |word: String, name: &'static str| {
        TextToken::MonoCaption
            .apply(text(word))
            .number_of_lines(1u32)
            .text_color(dark(name))
    };
    let selected = view()
        .w_full()
        .min_h(px(HEIGHT_MD))
        .px(px(PADDING_X))
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .bg(dark("selection"))
        .child(
            TextToken::Body
                .apply(text(t!("selected row")))
                .number_of_lines(1u32)
                .flex_1()
                .text_color(dark("foreground")),
        )
        .child(status(t!("warn"), "warning-text"))
        .child(status(t!("ok"), "success-text"))
        .child(status(t!("err"), "destructive-text"))
        .child(dynamic(move || {
            // The check in the accent's own text color, not the app's.
            let tint = derived(accent())("primary-text").1;
            untracked(move || {
                native("moshpit/symbol")
                    .flex_shrink_0()
                    .prop("name", json!("checkmark"))
                    .prop("pointSize", json!(13.0))
                    .prop("weight", json!("semibold"))
                    .prop("color", json!(theme::hex_string(tint)))
            })
        }));
    let card = view()
        .w_full()
        .pt(px(PADDING_X))
        .gap(px(12.0))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .overflow_hidden()
        .bg(dark("card"))
        .child(
            view()
                .px(px(PADDING_X))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(connect)
                .child(badge),
        )
        .child(selected);
    let samples = view()
        .w_full()
        .flex_row()
        .gap(px(8.0))
        .child(sample(
            t!("Dark"),
            dark("card"),
            dark("primary-text"),
            "accent-editor-dark-text",
        ))
        .child(sample(
            t!("Light"),
            light("card"),
            light("primary-text"),
            "accent-editor-light-text",
        ));
    view()
        .w_full()
        .gap(px(8.0))
        .accessible(true)
        .label(t!("Accent preview"))
        .test_id("accent-editor-preview")
        .child(
            view()
                .w_full()
                .p(px(PADDING_X))
                .rounded(px(number("radius-lg")))
                .border_curve(BorderCurve::Continuous)
                .bg(move || AdaptiveColor::from(screen_bg(accent())))
                .child(card),
        )
        .child(samples)
        .into_element()
}

/// The accent as text where an appearance puts it: `ink` on `ground` (the
/// appearance's card), named `label` over it.
fn sample(
    label: String,
    ground: impl Fn() -> AdaptiveColor + 'static,
    ink: impl Fn() -> AdaptiveColor + 'static,
    id: &'static str,
) -> View {
    view()
        .flex_1()
        .min_h(px(HEIGHT_MD))
        .px(px(PADDING_X))
        .py(px(8.0))
        .gap(px(2.0))
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .bg(ground)
        .border(px(1.0))
        .border_color(move || token("border"))
        .test_id(id)
        .child(
            TextToken::Caption
                .apply(text(label))
                .number_of_lines(1u32)
                .text_color(move || token("muted-foreground")),
        )
        .child(
            TextToken::Headline
                .apply(text(t!("Accent")))
                .number_of_lines(1u32)
                .text_color(ink),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mint() -> AccentDraft {
        AccentDraft {
            name: "Mint".into(),
            accent: nsh_core::color::rgb(0x2FD8A8),
        }
    }

    #[test]
    fn messages_round_trip() {
        for draft in [
            mint(),
            AccentDraft {
                name: String::new(),
                ..mint()
            },
        ] {
            let message = Message::Accent(draft);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_message_as_written() {
        assert_eq!(
            Message::parse(r##"{"op":"accent","name":"Mint","accent":"#2FD8A8"}"##),
            Ok(Message::Accent(mint()))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            (r#"{"op":"accents"}"#, "no op \"accents\""),
            (r##"{"op":"accent","name":"A"}"##, "accent"),
            (r##"{"op":"accent","name":"A","accent":"mint"}"##, "mint"),
            (
                r##"{"op":"accent","name":"A","accent":"#2FD8A8","id":"x"}"##,
                "id",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }

    #[test]
    fn a_pale_accent_reads_too_pale_as_light_text_a_deep_one_does_not() {
        // Signal Room's light text is darker than its accent (§2.3: #4E4DEC);
        // a pale yellow and mint more so; a deep indigo reads as it is.
        assert!(pale_as_light_text(nsh_core::color::rgb(0x6C6BEF)));
        assert!(pale_as_light_text(nsh_core::color::rgb(0xFFE066)));
        assert!(pale_as_light_text(nsh_core::color::rgb(0x2FD8A8)));
        assert!(!pale_as_light_text(nsh_core::color::rgb(0x3433B8)));
        assert!(!pale_as_light_text(nsh_core::color::rgb(0x1F1F1F)));
    }

    #[test]
    fn the_preview_wears_the_background_the_accent_derives() {
        // As `AppTheme.custom` derives it (Signal Room's own background is
        // fixed in the catalog, not derived).
        assert_eq!(
            screen_bg(nsh_core::color::rgb(0x6C6BEF)),
            nsh_core::color::rgb(0x05050B)
        );
        assert_eq!(
            screen_bg(nsh_core::color::rgb(0xFFFFFF)),
            nsh_core::color::rgb(0x0B0B0B)
        );
    }
}
