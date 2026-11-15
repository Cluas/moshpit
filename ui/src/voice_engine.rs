//! Settings › Voice Input › Recognition
//! (`Moshpit/UI/Settings/VoiceEngineView.swift`) in NSH: which speech engine
//! dictation runs on, Apple's or a local model (Qwen3-ASR or Whisper, the one
//! picked under Model).
//!
//! The host owns the setting: it posts what the page shows ([`Message`]),
//! `{"op": "engine", "engine": "apple" | "whisper", "needsModel": false}`,
//! again whenever that changes (`needsModel`: Whisper is picked and no model
//! is on the device, the SwiftUI page's SETUP group). The page tells it
//! `{"ev": "select", "engine": …}` when an engine is picked (the host applies
//! it as the SwiftUI page does), `{"ev": "models"}` from Download a model
//! (the host pushes the model page) and `{"ev": "back"}`.
//!
//! The page is the redesign's (`docs/nsh/UI-DESIGN.md` §6.11, R15): the
//! engines as a choice of one (each its name over what it is, the one in use
//! checked and selected for screen readers), a footer on where the voice
//! goes, and while Whisper has no model an `info` banner whose Download
//! opens the models. Test ids: `voice-engine-<engine>` for the rows,
//! `voice-engine-setup` for the banner and `voice-engine-models` for its
//! button.

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
        notice::{BannerTone, banner},
        page,
        row::row,
        section::section,
        tokens::token,
    },
};

/// A speech engine (`VoiceEngineKind`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Apple's on-device transcribers (the setting's default).
    #[default]
    Apple,
    /// A local model: Qwen3-ASR or Whisper, the one picked under Model. The
    /// id stays "whisper", what the setting stored before there were two.
    Whisper,
}

impl Engine {
    /// Both, in the SwiftUI page's order.
    pub const ALL: [Engine; 2] = [Engine::Apple, Engine::Whisper];

    /// Its id, as the host and the setting know it.
    pub fn id(self) -> &'static str {
        match self {
            Engine::Apple => "apple",
            Engine::Whisper => "whisper",
        }
    }

    /// What the page calls it (`VoiceEngineKind.displayName`).
    pub fn name(self) -> String {
        match self {
            Engine::Apple => t!("Apple (built in)"),
            Engine::Whisper => t!("Local model"),
        }
    }

    /// Its row's title: the engine's own name.
    pub fn title(self) -> String {
        match self {
            Engine::Apple => "Apple".into(),
            Engine::Whisper => t!("Local model"),
        }
    }

    /// What it is and what it costs, in a line (its row's subtitle).
    pub fn cost(self) -> String {
        match self {
            Engine::Apple => t!("Built in and quick; one language at a time"),
            Engine::Whisper => t!("A model on this device; follows speech that mixes languages"),
        }
    }

    /// What picking it costs (`VoiceEngineKind.summary`).
    pub fn summary(self) -> String {
        match self {
            Engine::Apple => t!("No download. One language per session."),
            Engine::Whisper => t!(
                "Downloads a model. Around 100 languages, and copes with a sentence that mixes two."
            ),
        }
    }
}

/// What the page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineState {
    /// The engine in use.
    pub engine: Engine,
    /// Whisper is in use and has no model on the device yet.
    #[serde(rename = "needsModel")]
    pub needs_model: bool,
}

/// A message from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "engine", …}`: what the page shows now.
    Engine(EngineState),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("voice-engine", json)?;
        match op.as_str() {
            "engine" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Engine)
                .map_err(|e| format!("voice-engine: engine: {e}")),
            other => Err(format!("voice-engine: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Engine(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("engine");
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

/// Settings › Voice Input › Recognition (§6.11): the two engines as a
/// choice of one, each saying what it costs, and while Whisper has no model
/// a banner that takes the user to the models.
#[component]
pub fn VoiceEngineScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    // None until the host's first post: no engine ticked yet.
    let engine = signal(None::<Engine>);
    let needs_model = signal(false);
    // The body waits for the host's first post (B366's `ready_body`).
    let ready = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::VoiceEngine,
        Message::parse,
        move |Message::Engine(state)| {
            engine.set(Some(state.engine));
            needs_model.set(state.needs_model);
            ready.set(true);
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let select: Rc<dyn Fn(Engine)> = {
        let tell = tell.clone();
        Rc::new(move |picked: Engine| {
            if engine.get_untracked() == Some(picked) {
                return;
            }
            haptic(Haptic::Selection);
            tell(json!({"ev": "select", "engine": picked.id()}));
        })
    };
    let models: Rc<dyn Fn()> = {
        let tell = tell.clone();
        Rc::new(move || tell(json!({"ev": "models"})))
    };
    let body = move || {
        let engines = section()
            .rows(Engine::ALL.into_iter().map(|kind| {
                let select = select.clone();
                row(kind.title())
                    .subtitle(kind.cost())
                    .check(move || engine.get() == Some(kind))
                    .on_press(move || select(kind))
                    .test_id(&format!("voice-engine-{}", kind.id()))
            }))
            .footer(t!("Both run on this device. Your voice is never uploaded."));
        let models = models.clone();
        page::page(
            bar,
            scrolled,
            "voice-engine-list",
            [
                when(
                    move || needs_model.get(),
                    move || {
                        let models = models.clone();
                        banner(BannerTone::Info, || t!("Download a model first").into())
                            .icon("arrow.down.circle")
                            .action(t!("Download"), move || models())
                            .action_test_id("voice-engine-models")
                            .test_id("voice-engine-setup")
                    },
                )
                .into_element(),
                engines.into_element(),
            ],
        )
        .into_element()
    };
    let failed = tell.clone();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("voice-engine")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            failed,
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: t!("Settings"),
            title: nav_bar::fixed(t!("Recognition")),
            leading: None,
            trailing: Vec::new(),
            on_back: Box::new(move || tell(json!({"ev": "back"}))),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        for engine in Engine::ALL {
            for needs_model in [false, true] {
                let message = Message::Engine(EngineState {
                    engine,
                    needs_model,
                });
                assert_eq!(Message::parse(&message.to_json()), Ok(message));
            }
        }
    }

    #[test]
    fn the_host_message_as_written() {
        assert_eq!(
            Message::parse(r#"{"op":"engine","engine":"whisper","needsModel":true}"#),
            Ok(Message::Engine(EngineState {
                engine: Engine::Whisper,
                needs_model: true,
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            (r#"{"op":"languages"}"#, "no op \"languages\""),
            (
                r#"{"op":"engine","engine":"siri","needsModel":false}"#,
                "siri",
            ),
            (r#"{"op":"engine","engine":"apple"}"#, "needsModel"),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }
}
