//! Settings › Voice Input › Model (`Moshpit/UI/Settings/WhisperModelView.swift`)
//! in NSH: the local Whisper models this device can run, their sizes before
//! anything is tapped, downloads with their progress, the one in use ticked,
//! removal.
//!
//! The host owns the models (`WhisperModelStore`, `WhisperDownloadCenter`):
//! it posts what the page shows ([`Message`]), `{"op": "models", …}`
//! ([`Models`]), again whenever that changes (a download's progress, one
//! finishing, a removal). Sizes come formatted as the system formats bytes
//! for this locale; the page says what they mean. What a model's row shows
//! is the host's answer, never the page's guess: a model removed while it was
//! loading is the store's to drop (B124), not something the page tracks. The
//! page tells it:
//!
//! - `{"ev": "select", "id": …}` for a downloaded model's row, not in use;
//! - `{"ev": "download", "id": …}` (Download or Resume), `{"ev": "pause",
//!   "id": …}`, `{"ev": "discard", "id": …}` (a paused download's trash);
//! - `{"ev": "remove", "id": …}` (a downloaded model's trash: the host asks
//!   first, with the SwiftUI page's alert);
//! - `{"ev": "back"}`.
//!
//! The page is the redesign's (`docs/nsh/UI-DESIGN.md` §6.11, R15): a row
//! per model, its name over its size (shown before anything is downloaded)
//! and what it is for; at its trailing end the In use badge, an outline
//! Download (Resume for a paused one), or while it downloads the system's
//! progress bar, the percent and Pause; the ⋯ menu beside a downloaded or
//! paused model removes it or discards the part kept; the last failure (a
//! full disk) as a `destructive` banner; what the models take. Test ids:
//! `whisper-model-<id>` (the row, selected while in use),
//! `whisper-model-<download|pause|in-use|more>-<id>`, `whisper-model-error`,
//! `whisper-model-storage`; the menu's items are `remove` and `discard`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    boundary::Tell,
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        button::{Size, Variant, button},
        feedback::{self, separator, touch_target},
        icon::{Tone, icon},
        menu::{MenuEntry, menu},
        notice::{BannerTone, banner},
        page,
        row::row,
        section::{Inset, section},
        status::{BadgeVariant, badge},
        tokens::{number, token},
        typography::TextToken,
    },
};

/// One model, as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Model {
    /// The variant (`WhisperModelOption.id`).
    pub id: String,
    pub name: String,
    /// One line of guidance for choosing.
    pub summary: String,
    /// Its download's approximate size, formatted.
    pub approximate: String,
    /// Its size on the device, formatted, once downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
    /// How much of an interrupted download is kept, formatted (not
    /// downloaded yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<String>,
    /// How far its download is, 0 to 1, while one runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// It is the model a dictation would use.
    pub selected: bool,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    /// The models this device can run, best first.
    pub models: Vec<Model>,
    /// The last download failure, in the system's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// What the downloaded models take, formatted (none downloaded: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "models", …}`: what the page shows now.
    Models(Models),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("whisper-model", json)?;
        match op.as_str() {
            "models" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Models)
                .map_err(|e| format!("whisper-model: models: {e}")),
            other => Err(format!("whisper-model: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Models(models) => {
                let mut value = serde_json::to_value(models).expect("models serialize");
                value["op"] = json!("models");
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

/// Settings › Voice Input › Model (§6.11).
#[component]
pub fn WhisperModelScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let bar = BarMetrics::or_standard(params.bar);
    let back_title = params.back.clone().unwrap_or_else(|| t!("Settings"));
    let models = signal(Vec::<Model>::new());
    let failure = signal(None::<String>);
    let storage = signal(None::<String>);
    // The body waits for the host's first post (B366's `ready_body`).
    let ready = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::WhisperModel,
        Message::parse,
        move |Message::Models(next)| {
            models.set(next.models);
            failure.set(next.failure);
            storage.set(next.storage);
            ready.set(true);
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let back = {
        let tell = tell.clone();
        move || tell(json!({"ev": "back"}))
    };
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            let list = each(
                models,
                |model: &Model| model.id.clone(),
                move |model, index| model_item(model, index, models, tell.clone()),
            );
            page::page(
                bar,
                scrolled,
                "whisper-model-list",
                [
                    when(
                        move || failure.with(Option::is_some),
                        move || {
                            banner(BannerTone::Destructive, move || {
                                failure.get().unwrap_or_default().into()
                            })
                            .test_id("whisper-model-error")
                        },
                    )
                    .into_element(),
                    section()
                        .item(view().w_full().child(list), Inset::Content)
                        .footer(t!(
                            "Downloads once from Hugging Face, then runs on this device. Bigger models handle mixed speech better, but slower."
                        ))
                        .into_element(),
                    when(
                        move || storage.with(Option::is_some),
                        move || {
                            section().row(
                                row(t!("Models on this device"))
                                    .value_with(move || storage.get().unwrap_or_default().into())
                                    .test_id("whisper-model-storage"),
                            )
                        },
                    )
                    .into_element(),
                ],
            )
            .into_element()
        }
    };
    let failed = tell.clone();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("whisper-model")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            failed,
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title,
            title: nav_bar::fixed(t!("Model")),
            leading: None,
            trailing: Vec::new(),
            on_back: Box::new(back),
            scrolled,
        }))
}

/// What a model's row says under its name: its size (what it takes on the
/// device, how much of a paused download is kept, or what a download
/// takes), then what it is for.
pub fn subtitle(model: &Model) -> String {
    let size = if let Some(installed) = &model.installed {
        installed.clone()
    } else if let Some(partial) = &model.partial {
        t!(
            "%@ of ≈%@",
            arg1 = partial.clone(),
            arg2 = model.approximate.clone()
        )
    } else {
        format!("≈{}", model.approximate)
    };
    if model.summary.is_empty() {
        size
    } else {
        format!("{size} · {}", model.summary)
    }
}

/// How far a download is, as the row says it: a whole percent.
pub fn percent(progress: f64) -> String {
    format!("{}%", (progress.clamp(0.0, 1.0) * 100.0).floor() as u32)
}

/// What a model's row shows at its trailing end, by where the model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trailing {
    /// Its download's progress and Pause.
    Downloading,
    /// The one in use: the In use badge.
    InUse,
    /// Downloaded, not in use: nothing (a press uses it).
    Ready,
    /// Download, or Resume when part of it is kept.
    Get { partial: bool },
}

impl Trailing {
    fn of(model: &Model) -> Self {
        if model.progress.is_some() {
            Trailing::Downloading
        } else if model.installed.is_some() {
            if model.selected {
                Trailing::InUse
            } else {
                Trailing::Ready
            }
        } else {
            Trailing::Get {
                partial: model.partial.is_some(),
            }
        }
    }
}

/// One model: its row (a press uses a downloaded model), and beside it the
/// ⋯ menu that removes it or discards a paused download; the separator
/// above all but the first.
fn model_item(
    model: ReadSignal<Model>,
    index: ReadSignal<usize>,
    models: Signal<Vec<Model>>,
    tell: Tell,
) -> impl IntoElement {
    let id = model.with_untracked(|m| m.id.clone());
    let event = {
        let (tell, id) = (tell.clone(), id.clone());
        move |ev: &'static str| {
            let (tell, id) = (tell.clone(), id.clone());
            move || tell(json!({"ev": ev, "id": id}))
        }
    };
    let pick = {
        let tell = tell.clone();
        let id = id.clone();
        move || {
            let (installed, selected) =
                model.with_untracked(|m| (m.installed.is_some(), m.selected));
            if installed && !selected {
                haptic(Haptic::Selection);
                tell(json!({"ev": "select", "id": id}));
            }
        }
    };
    let state = computed(move || model.with(Trailing::of));
    let trailing = {
        let id = id.clone();
        dynamic(move || {
            let state = state.get();
            let (id, event) = (id.clone(), event.clone());
            untracked(move || match state {
                Trailing::Downloading => view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(progress(model))
                    .child(
                        TextToken::Subhead
                            .apply(text(move || {
                                percent(model.with(|m| m.progress.unwrap_or(0.0)))
                            }))
                            .text_color(move || token("muted-foreground"))
                            .tabular_nums()
                            .number_of_lines(1u32),
                    )
                    .child(
                        button(t!("Pause"))
                            .variant(Variant::Ghost)
                            .size(Size::Sm)
                            .on_press(event("pause"))
                            .test_id(&format!("whisper-model-pause-{id}")),
                    )
                    .into_element(),
                Trailing::InUse => badge(t!("In use"))
                    .variant(BadgeVariant::Success)
                    .test_id(&format!("whisper-model-in-use-{id}"))
                    .into_element(),
                Trailing::Ready => view().into_element(),
                Trailing::Get { partial } => button(if partial {
                    t!("Resume")
                } else {
                    t!("Download")
                })
                .variant(Variant::Outline)
                .size(Size::Sm)
                .on_press(event("download"))
                .test_id(&format!("whisper-model-download-{id}"))
                .into_element(),
            })
        })
    };
    let shown = row("")
        .title_with(move || model.with(|m| m.name.clone()).into())
        .subtitle_with(move || model.with(subtitle).into())
        .selected(move || model.with(|m| m.selected))
        .trailing(trailing)
        .on_press(pick)
        .test_id(&format!("whisper-model-{id}"));
    view()
        .w_full()
        .child(when(
            move || index.get() > 0,
            || separator(Inset::Content.points()),
        ))
        .child(
            // The row and its ⋯ lane as one: the card's corners clip both
            // (its first row's top, its last row's bottom), and the lane
            // takes the selected row's tint.
            view()
                .w_full()
                .flex_row()
                .items_center()
                .overflow_hidden()
                .border_curve(BorderCurve::Continuous)
                .rounded_t(move || {
                    px(if index.get() == 0 {
                        inner_radius()
                    } else {
                        0.0
                    })
                })
                .rounded_b(move || {
                    let last = index.get() + 1 == models.with(Vec::len);
                    px(if last { inner_radius() } else { 0.0 })
                })
                .child(view().flex_1().min_w(px(0.0)).child(shown))
                .child(more(model, state, id, tell)),
        )
}

/// The card's rounding inside its edge, which a row's corners follow.
fn inner_radius() -> f32 {
    (number("radius-lg") - number("card-border-width")).max(0.0)
}

/// The download's progress, [`PROGRESS_WIDTH`] wide.
fn progress(model: ReadSignal<Model>) -> AnyElement {
    view()
        .w(px(PROGRESS_WIDTH))
        .child(feedback::progress(
            move || model.with(|m| m.progress.unwrap_or(0.0)),
            move || t!("Downloading %@", arg1 = model.with(|m| m.name.clone())),
        ))
        .into_element()
}

/// The progress bar's width in a row.
const PROGRESS_WIDTH: f32 = 56.0;

/// The ⋯ menu: Remove (the host asks first) for a downloaded model,
/// Discard download for a paused one; nothing while it downloads or before
/// any of it is here. Its lane, a touch target wide, is kept on every row
/// so the rows' trailing ends line up.
fn more(model: ReadSignal<Model>, state: Memo<Trailing>, id: String, tell: Tell) -> AnyElement {
    let menu_id = format!("whisper-model-more-{id}");
    view()
        .w(px(touch_target()))
        .self_stretch()
        .flex_shrink_0()
        .bg(move || {
            if model.with(|m| m.selected) {
                token("selection")
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .items_center()
        .justify_center()
        .child(dynamic(move || {
            let state = state.get();
            let (tell, menu_id, id) = (tell.clone(), menu_id.clone(), id.clone());
            untracked(move || {
                let entry = match state {
                    Trailing::InUse | Trailing::Ready => {
                        MenuEntry::new("remove", t!("Remove")).symbol("trash")
                    }
                    Trailing::Get { partial: true } => {
                        MenuEntry::new("discard", t!("Discard download")).symbol("trash")
                    }
                    Trailing::Downloading | Trailing::Get { partial: false } => {
                        return view().into_element();
                    }
                };
                let name = move || model.with(|m| m.name.clone());
                view()
                    .relative()
                    .w(px(touch_target()))
                    .h(px(touch_target()))
                    .items_center()
                    .justify_center()
                    .test_id(&menu_id)
                    .child(
                        view()
                            .accessibility_elements_hidden(true)
                            .child(icon("ellipsis").tone(Tone::Muted)),
                    )
                    .child(menu(
                        vec![entry.destructive()],
                        Some(&menu_id),
                        move || t!("More for %@", arg1 = name()),
                        move |picked| tell(json!({"ev": picked, "id": id})),
                    ))
                    .into_element()
            })
        }))
        .into_element()
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        Model {
            id: id.into(),
            name: "Small".into(),
            summary: "Good balance of accuracy and speed.".into(),
            approximate: "470 MB".into(),
            installed: None,
            partial: None,
            progress: None,
            selected: false,
        }
    }

    #[test]
    fn messages_round_trip() {
        let message = Message::Models(Models {
            models: vec![
                model("openai_whisper-small"),
                Model {
                    installed: Some("482 MB".into()),
                    selected: true,
                    ..model("openai_whisper-base")
                },
                Model {
                    partial: Some("120 MB".into()),
                    progress: Some(0.25),
                    ..model("openai_whisper-tiny")
                },
            ],
            failure: Some("The network connection was lost.".into()),
            storage: Some("482 MB".into()),
        });
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let empty = Message::Models(Models {
            models: vec![],
            failure: None,
            storage: None,
        });
        assert_eq!(Message::parse(&empty.to_json()), Ok(empty));
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            (r#"{"op":"engine"}"#, "no op \"engine\""),
            (r#"{"op":"models"}"#, "models"),
            (
                r#"{"op":"models","models":[{"id":"a","name":"A","summary":"","approximate":"1 MB"}]}"#,
                "selected",
            ),
            (
                r#"{"op":"models","models":[],"storage":482}"#,
                "expected a string",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }
}
