//! Settings › App Icon (`Moshpit/UI/Settings/AppIconGalleryView.swift`) in
//! NSH: the home-screen icons bundled with the app, in a grid, the one in
//! use ticked; picking one asks the host to switch to it.
//!
//! The host owns the icons and the switch (iOS's `setAlternateIconName`):
//! it posts what the page shows ([`Message`]), `{"op": "icons", "icons":
//! [{"id", "name", "image"}…], "selected": "pit", "failure": "…"}`, again
//! whenever that changes. The page tells it `{"ev": "select", "id": …}` when
//! an icon is picked and `{"ev": "back"}`; the tick moves only once iOS has
//! switched the icon (the host posts the new pick), or the failure shows
//! what iOS said.
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.10 (R14): a refusal in a
//! destructive banner over the grid; the adaptive grid (columns at least
//! 96 pt, 14 pt apart, rows 16 pt apart) of 68 pt thumbnails (their corners
//! the picture's own), each with its name under it and the one in use
//! ticked in the accent (selected for screen readers); the note under the
//! grid. The accessibility identifiers are `app-icon-<id>`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        icon::{IconSize, Tone, icon},
        notice::{BannerTone, banner},
        page::page,
        tokens::{number, token},
        typography::TextToken,
    },
};

/// One icon, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icon {
    /// The setting's id (`AppIconOption.id`).
    pub id: String,
    pub name: String,
    /// Its preview, an image URI (`asset:/IconPreview-Pit@3x.png`); none: a
    /// placeholder, as the SwiftUI page draws for a missing preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icons {
    /// Every icon, in the host's order.
    pub icons: Vec<Icon>,
    /// The id of the icon in use (one no icon has: none ticked).
    pub selected: String,
    /// Why iOS refused the last change (its description), until one works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "icons", …}`: what the page shows now.
    Icons(Icons),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("app-icon", json)?;
        match op.as_str() {
            "icons" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Icons)
                .map_err(|e| format!("app-icon: icons: {e}")),
            other => Err(format!("app-icon: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Icons(icons) => {
                let mut value = serde_json::to_value(icons).expect("icons serialize");
                value["op"] = json!("icons");
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

/// The grid: columns at least this wide (`GridItem(.adaptive(minimum: 96),
/// spacing: 14)`), this far apart, rows `ROW_SPACING` apart.
const COLUMN_MIN: f32 = 96.0;
const COLUMN_SPACING: f32 = 14.0;
const ROW_SPACING: f32 = 16.0;
/// A thumbnail's side, and the room between it and the icon's name.
const THUMB: f32 = 68.0;
const NAME_GAP: f32 = 8.0;
/// The tick's disc over a thumbnail's corner.
const TICK: f32 = 22.0;

/// The grid's columns in `width` (the content's): how many fit at least
/// [`COLUMN_MIN`] wide, and how wide each is then, as SwiftUI's adaptive
/// grid lays them out.
pub fn columns(width: f32) -> (usize, f32) {
    let count = ((width + COLUMN_SPACING) / (COLUMN_MIN + COLUMN_SPACING))
        .floor()
        .max(1.0);
    let column = (width - COLUMN_SPACING * (count - 1.0)) / count;
    (count as usize, column.max(0.0))
}

/// Settings › App Icon.
#[component]
pub fn AppIconScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let icons = signal(Vec::<Icon>::new());
    // None until the host's first post: then a later change of the pick
    // is felt.
    let selected = signal(None::<String>);
    let failure = signal(None::<String>);
    channel::hear(
        channel.as_ref(),
        Screen::AppIcon,
        Message::parse,
        move |Message::Icons(state)| {
            let before = selected.get_untracked();
            if before
                .as_ref()
                .is_some_and(|before| *before != state.selected)
            {
                haptic(Haptic::Selection);
            }
            icons.set(state.icons);
            selected.set(Some(state.selected));
            failure.set(state.failure);
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel.clone());
    let pick = {
        let tell = tell.clone();
        move |id: String| tell(json!({"ev": "select", "id": id}))
    };
    let refusal = when(
        move || failure.with(Option::is_some),
        move || {
            banner(BannerTone::Destructive, move || {
                let detail = failure.get().unwrap_or_default();
                t!("Couldn't change the icon: %@", arg1 = detail).into()
            })
            .test_id("app-icon-failure")
        },
    )
    .into_element();
    let note = TextToken::Footnote
        .apply(text(t!(
            "The icon is separate from the accent color, because iOS only allows switching between icons bundled with the app — a custom accent can't have matching artwork generated for it."
        )))
        .text_color(move || token("muted-foreground"))
        .test_id("app-icon-note")
        .into_element();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("app-icon")
        .child(page(
            bar,
            scrolled,
            "app-icon-grid",
            [refusal, grid(icons, selected, pick).into_element(), note],
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: t!("Settings"),
            title: nav_bar::fixed(t!("App Icon")),
            leading: None,
            trailing: Vec::new(),
            on_back: Box::new(move || tell(json!({"ev": "back"}))),
            scrolled,
        }))
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Rc<dyn Fn(serde_json::Value)> {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// The icons in the adaptive grid, as wide as the page's content.
fn grid(
    icons: Signal<Vec<Icon>>,
    selected: Signal<Option<String>>,
    pick: impl Fn(String) + Clone + 'static,
) -> impl IntoElement {
    // A hair under the exact width, so the last column never wraps on a
    // rounding error.
    let column = move || px(columns(window().width.0 - 2.0 * number("space-page")).1 - 0.01);
    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap_x(px(COLUMN_SPACING))
        .gap_y(px(ROW_SPACING))
        .child(each(
            icons,
            |icon: &Icon| icon.id.clone(),
            move |icon, _| {
                let pick = pick.clone();
                cell(icon.get_untracked(), selected, column, pick)
            },
        ))
}

/// One icon: its thumbnail (ticked when in use) over its name, the whole
/// column its target.
fn cell(
    icon: Icon,
    selected: Signal<Option<String>>,
    width: impl Fn() -> Points + 'static,
    pick: impl Fn(String) + 'static,
) -> impl IntoElement {
    let id = icon.id.clone();
    let is_selected = {
        let id = id.clone();
        computed(move || selected.with(|s| s.as_deref() == Some(id.as_str())))
    };
    view()
        .w(width)
        .items_center()
        .gap(px(NAME_GAP))
        .role(AccessibilityRole::Button)
        .label(icon.name.clone())
        .accessibility_state(move || AccessibilityState {
            selected: is_selected.get(),
            ..AccessibilityState::default()
        })
        .test_id(&format!("app-icon-{id}"))
        .active(|s| s.opacity(0.6))
        .on_press(move || pick(id.clone()))
        .child(
            view()
                .relative()
                .child(thumbnail(&icon, THUMB))
                .child(when(move || is_selected.get(), tick)),
        )
        .child(
            TextToken::Caption
                .apply(text(icon.name.clone()))
                .number_of_lines(1u32)
                .text_color(move || {
                    token(if is_selected.get() {
                        "foreground"
                    } else {
                        "muted-foreground"
                    })
                }),
        )
}

/// The picked icon's tick: the accent's filled check on a disc of the card,
/// at the thumbnail's bottom trailing corner, half out of it.
fn tick() -> AnyElement {
    view()
        .absolute()
        .right(px(-TICK / 3.0))
        .bottom(px(-TICK / 3.0))
        .w(px(TICK))
        .h(px(TICK))
        .rounded_full()
        .bg(move || token("card"))
        .items_center()
        .justify_center()
        .child(
            icon("checkmark.circle.fill")
                .size(IconSize::Md)
                .tone(Tone::Primary),
        )
        .into_element()
}

/// `AppIconThumb`: the icon as the home screen draws it, `side` points
/// square, in its continuous rounded corners (22% of the side), a faint
/// white border, a soft shadow under it.
pub fn thumbnail(icon: &Icon, side: f32) -> impl IntoElement {
    let radius = side * 0.22;
    let rounded = move |v: View| v.rounded(px(radius)).border_curve(BorderCurve::Continuous);
    let picture = match &icon.image {
        Some(uri) => image(nsh_core::element::ImageSrc::from(uri.clone()))
            .w(px(side))
            .h(px(side))
            .into_element(),
        // Only if a preview was renamed without regenerating: something
        // inert rather than a hole, as the SwiftUI page draws.
        None => view()
            .w(px(side))
            .h(px(side))
            .bg(move || token("muted"))
            .items_center()
            .justify_center()
            .child(crate::ui::icon::icon("questionmark").tone(Tone::Muted))
            .into_element(),
    };
    rounded(view())
        .w(px(side))
        .h(px(side))
        .shadow([nsh_core::BoxShadow {
            offset_x: px(0.0),
            offset_y: px(side * 0.03),
            // SwiftUI's shadow radius is Core Animation's, half CSS's blur.
            blur: px(2.0 * side * 0.08),
            spread: px(0.0),
            color: nsh_core::Color::from(nsh_core::color::Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.3,
            }),
            inset: false,
        }])
        .child(
            rounded(view())
                .absolute()
                .inset_0()
                .overflow_hidden()
                .child(picture),
        )
        .child(
            rounded(view())
                .absolute()
                .inset_0()
                .border(px(1.0))
                .border_color(move || token("border"))
                .pointer_events(PointerEvents::None),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(id: &str, name: &str) -> Icon {
        Icon {
            id: id.into(),
            name: name.into(),
            image: Some(format!("asset:/IconPreview-{name}@3x.png")),
        }
    }

    #[test]
    fn messages_round_trip() {
        for icons in [
            Icons {
                icons: vec![icon("default", "Moshpit"), icon("pit", "Pit")],
                selected: "default".into(),
                failure: None,
            },
            Icons {
                icons: vec![Icon {
                    id: "wq".into(),
                    name: ":wq".into(),
                    image: None,
                }],
                selected: "gone".into(),
                failure: Some("The operation couldn’t be completed.".into()),
            },
        ] {
            let message = Message::Icons(icons);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_message_as_written() {
        assert_eq!(
            Message::parse(
                r#"{"op":"icons","selected":"pit","icons":[{"id":"pit","name":"Pit","image":"asset:/IconPreview-Pit@3x.png"}]}"#
            ),
            Ok(Message::Icons(Icons {
                icons: vec![icon("pit", "Pit")],
                selected: "pit".into(),
                failure: None,
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            ("[]", "JSON object"),
            (r#"{"icons":[]}"#, "\"op\""),
            (r#"{"op":"log"}"#, "no op \"log\""),
            (r#"{"op":"icons","icons":[]}"#, "selected"),
            (
                r#"{"op":"icons","icons":[{"id":"a"}],"selected":"a"}"#,
                "name",
            ),
            (
                r#"{"op":"icons","icons":[],"selected":"a","colour":1}"#,
                "colour",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }

    #[test]
    fn columns_are_swiftuis_adaptive_grid() {
        // Settings' sheet on the iPad mini: 540 pt wide, 508 of content.
        assert_eq!(columns(508.0), (4, (508.0 - 3.0 * 14.0) / 4.0));
        // An iPhone 16 Pro in portrait: 402 pt, 370 of content.
        assert_eq!(columns(370.0), (3, (370.0 - 2.0 * 14.0) / 3.0));
        // Exactly room for two.
        assert_eq!(columns(206.0), (2, 96.0));
        assert_eq!(columns(205.9).0, 1);
        assert_eq!(columns(20.0), (1, 20.0));
    }
}
