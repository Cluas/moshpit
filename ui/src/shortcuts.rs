//! Settings › Shortcuts (`docs/nsh/UI-DESIGN.md` §6.7) in NSH, in its
//! sheet: the terminal's shortcut bar, edited. A preview of the bar, the
//! shortcuts on it (reordered by dragging their handles, taken off with −),
//! the user's own, and the rest, each put on with ＋.
//!
//! The host owns the shortcuts (`ShortcutStore`, which saves them in the
//! format every build since 1.0.8 reads) and posts them ([`Message`]):
//! `{"op": "shortcuts", "toolbar": [Shortcut…], "custom": […],
//! "available": […], "limit": 12}`, again whenever they change. The page
//! tells it what to change:
//!
//! - `{"ev": "move", "from": i, "to": j}`: a bar shortcut dragged from `i`
//!   to before the one at `j` of the bar as it was (SwiftUI's `onMove`);
//!   the page shows the new order at once;
//! - `{"ev": "remove-from-toolbar", "id"}` (a bar row's −: it goes to
//!   Available, so nothing asks), `{"ev": "delete", "id"}` (a custom
//!   shortcut's −, once the system's alert has asked),
//!   `{"ev": "add-to-toolbar", "id"}`;
//! - `{"ev": "edit", "id"}` (a custom shortcut pressed), `{"ev": "add"}`,
//!   `{"ev": "done"}`.
//!
//! Built from the ui layer (R11): the preview, a strip of the shortcut
//! bar's own keys on the terminal's ground (their text no larger than at
//! xLarge, as the bar's); IN THE BAR · n OF 12 with each row's red −, its chip
//! and what it does, and its handle (`line.3.horizontal`, 44 pt to the
//! touch); YOURS, a press on a row editing it; AVAILABLE with the green ＋,
//! the whole group disabled while the bar is full and a line under its
//! title saying how many it holds. The identifiers: `shortcuts-done`,
//! `shortcuts-add`, `shortcuts-preview`, `shortcut-row-<id>`,
//! `shortcut-remove-<id>`, `shortcut-grip-<id>`, `shortcut-custom-<id>`,
//! `shortcut-delete-<id>`, `shortcut-available-<id>`, `shortcut-add-<id>`,
//! `shortcuts-no-custom`.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        alert::{alert_dialog, delete_title},
        button::{IconButtonSize, icon_button},
        empty::{EmptyVariant, empty},
        feedback::{separator, touch_target},
        icon::{IconSize, Tone, icon},
        key::{ChipVariant, chip, key, symbol_key},
        motion::Duration,
        page,
        row::{GAP, HEIGHT_MD, LEADING, PADDING_X},
        section::{Inset, Section, SectionVariant, section},
        tokens::{number, token},
        typography::{TextToken, eyebrow},
    },
};

/// One shortcut, as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Shortcut {
    pub id: String,
    /// Its kind, as the store names it (`keyCombo`, `text`, `command`,
    /// `dpad`, `arrows`, `paste`, `scroll`, `ctrl`, `mic`, `image`,
    /// `files`).
    pub kind: String,
    /// Its chip's label (`esc`, `^C`).
    pub label: String,
    /// What the editor calls it.
    pub summary: String,
    /// Built in (its chip neutral), not the user's own (in the accent).
    pub builtin: bool,
}

impl Shortcut {
    /// The SF Symbol of a kind the bar draws as an icon
    /// (`ShortcutEditRow.glyphSymbol`).
    pub fn glyph(&self) -> Option<&'static str> {
        glyph(&self.kind)
    }
}

/// The SF Symbol the bar draws for shortcuts of `kind`, if it draws one.
pub fn glyph(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "dpad" => "arrow.up.and.down.and.arrow.left.and.right",
        "scroll" => "arrow.up.and.down",
        "mic" => "mic",
        "image" => "photo",
        "files" => "folder",
        _ => return None,
    })
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shortcuts {
    /// The bar, in order.
    pub toolbar: Vec<Shortcut>,
    /// The user's own, in or out of the bar.
    pub custom: Vec<Shortcut>,
    /// Those off the bar, which ＋ puts back.
    pub available: Vec<Shortcut>,
    /// How many the bar holds.
    pub limit: usize,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "shortcuts", …}`: what the page shows now.
    Shortcuts(Shortcuts),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("shortcuts", json)?;
        match op.as_str() {
            "shortcuts" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Shortcuts)
                .map_err(|e| format!("shortcuts: shortcuts: {e}")),
            other => Err(format!("shortcuts: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Shortcuts(shortcuts) => {
                let mut value = serde_json::to_value(shortcuts).expect("shortcuts serialize");
                value["op"] = json!("shortcuts");
                value.to_string()
            }
        }
    }
}

/// Where a shortcut dragged from `from` lands when it is dropped at `to`
/// (both positions in the bar): the `to` of SwiftUI's `onMove`, the
/// position before which it goes in the bar as it was.
pub fn move_destination(from: usize, to: usize) -> usize {
    if to > from { to + 1 } else { to }
}

/// `items` with the one at `from` moved as SwiftUI's `onMove(from, to)`
/// moves it.
pub fn moved<T: Clone>(items: &[T], from: usize, to: usize) -> Vec<T> {
    let mut items = items.to_vec();
    if from >= items.len() {
        return items;
    }
    let item = items.remove(from);
    let at = if to > from { to - 1 } else { to };
    items.insert(at.min(items.len()), item);
    items
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// What tells the host.
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The page's state: the host's last post.
#[derive(Clone, Copy)]
struct State {
    toolbar: Signal<Vec<Shortcut>>,
    custom: Signal<Vec<Shortcut>>,
    available: Signal<Vec<Shortcut>>,
    limit: Signal<usize>,
}

impl State {
    /// The bar holds all it can: nothing more goes on it.
    fn full(self) -> bool {
        self.toolbar.with(Vec::len) >= self.limit.get()
    }
}

/// Settings › Shortcuts.
#[component]
pub fn ShortcutsScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = State {
        toolbar: signal(Vec::new()),
        custom: signal(Vec::new()),
        available: signal(Vec::new()),
        limit: signal(12),
    };
    let ready = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::Shortcuts,
        Message::parse,
        move |Message::Shortcuts(shortcuts)| {
            state.toolbar.set(shortcuts.toolbar);
            state.custom.set(shortcuts.custom);
            state.available.set(shortcuts.available);
            state.limit.set(shortcuts.limit);
            ready.set(true);
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let body = {
        let tell = tell.clone();
        move || {
            page::page(
                bar,
                scrolled,
                "shortcuts-list",
                [
                    preview_section(state),
                    toolbar_section(state, tell.clone()),
                    custom_section(state, tell.clone()),
                    available_section(state, tell.clone()),
                ],
            )
            .into_element()
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("shortcuts")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: nav_bar::fixed(t!("Shortcuts")),
            leading: Some(nav_bar::BarButton {
                label: t!("Done"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "shortcuts-done",
                on_press: Box::new({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "done"}))
                }),
                menu: None,
            }),
            trailing: vec![nav_bar::BarButton {
                label: t!("Add Shortcut"),
                content: nav_bar::BarContent::Circled("plus"),
                enabled: computed(|| true),
                emphasized: false,
                test_id: "shortcuts-add",
                on_press: Box::new(move || tell(json!({"ev": "add"}))),
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

// --- The preview --------------------------------------------------------------

/// The bar as the terminal shows it: its keys (the shortcut bar's own key)
/// on the terminal's ground, in a strip that scrolls sideways. A key comes
/// and goes with a fade.
fn preview_section(state: State) -> AnyElement {
    let strip = scroll()
        .horizontal()
        .scrollbar(false)
        .w_full()
        .test_id("shortcuts-preview")
        .child(view().flex_row().gap(px(6.0)).p(px(8.0)).child(each(
            state.toolbar,
            |s: &Shortcut| s.id.clone(),
            |shortcut, _| preview_key(shortcut),
        )));
    section()
        .variant(SectionVariant::Plain)
        .item(
            view()
                .w_full()
                .rounded(px(number("radius-lg")))
                .border_curve(BorderCurve::Continuous)
                .overflow_hidden()
                .bg(move || token("terminal-bg"))
                .accessibility_elements_hidden(true)
                .child(strip)
                .into_element(),
            Inset::Content,
        )
        .test_id("shortcuts-preview-section")
        .into_element()
}

/// One key of the preview: its symbol or its label, as the bar draws it;
/// display only (no press handler), the strip hidden from screen readers.
fn preview_key(shortcut: ReadSignal<Shortcut>) -> AnyElement {
    let (glyph, label, builtin) =
        shortcut.with_untracked(|s| (s.glyph(), s.label.clone(), s.builtin));
    let key = match glyph {
        Some(name) => symbol_key(name, label),
        None => key(label),
    };
    let key = if builtin { key } else { key.armed(|| true) };
    let fade = motion().entering(
        MotionSpec::new(KeyframeTiming::new(Duration::Fast.ms()))
            .values(MotionChannel::Opacity, [0.0, 1.0]),
    );
    view().bind_motion(&fade).child(key).into_element()
}

// --- In the bar -----------------------------------------------------------------

/// A drag of a bar row by its handle.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    /// The row's position when the drag began.
    from: usize,
    /// How far the finger moved, down positive.
    dy: f32,
}

/// IN THE BAR · n OF 12: the bar's rows, each with its − and its handle.
fn toolbar_section(state: State, tell: Tell) -> AnyElement {
    let drag = signal(None::<Drag>);
    let heights: Rc<RefCell<HashMap<String, f32>>> = Rc::default();
    let header = computed(move || {
        t!(
            "In the bar · %lld of %lld",
            arg1 = state.toolbar.with(Vec::len),
            arg2 = state.limit.get()
        )
    });
    let rows = each(
        state.toolbar,
        |s: &Shortcut| s.id.clone(),
        move |shortcut, index| {
            toolbar_row(shortcut, index, state, drag, heights.clone(), tell.clone())
        },
    );
    titled(
        move || header.get(),
        None,
        section()
            .item(rows, Inset::Lead)
            .test_id("shortcuts-toolbar"),
    )
}

/// A section under a title of the page's own (`title()` follows the state)
/// and, while `subtitle()` says one, a line under it.
fn titled(
    title: impl Fn() -> String + 'static,
    subtitle: Option<Rc<dyn Fn() -> Option<SharedString>>>,
    section: Section,
) -> AnyElement {
    let mut head = view()
        .ps(px(PADDING_X))
        .gap(px(2.0))
        .child(eyebrow(text(title)).role(AccessibilityRole::Header));
    if let Some(subtitle) = subtitle {
        head = head.child(dynamic(move || {
            let line = subtitle();
            untracked(move || {
                line.map(|line| {
                    TextToken::Footnote
                        .apply(text(line))
                        .text_color(move || token("muted-foreground"))
                })
            })
        }));
    }
    view()
        .w_full()
        .gap(px(8.0))
        .child(head)
        .child(section)
        .into_element()
}

/// A bar row: its −, which takes it off the bar (it goes to Available; no
/// question), its chip and what it does, and its handle; a custom shortcut
/// opens its editor on a press. Dragged by its handle, it follows the
/// finger and the others make room; let go, it lands where it is and the
/// host hears the move (`{"ev": "move", "from", "to"}`).
fn toolbar_row(
    shortcut: ReadSignal<Shortcut>,
    index: ReadSignal<usize>,
    state: State,
    drag: Signal<Option<Drag>>,
    heights: Rc<RefCell<HashMap<String, f32>>>,
    tell: Tell,
) -> AnyElement {
    let id = shortcut.with_untracked(|s| s.id.clone());
    let order = move || {
        state
            .toolbar
            .with(|t| t.iter().map(|s| s.id.clone()).collect::<Vec<_>>())
    };
    let heights_in_order = {
        let heights = heights.clone();
        move || {
            let heights = heights.borrow();
            order()
                .iter()
                .map(|id| heights.get(id).copied().unwrap_or(HEIGHT_MD))
                .collect::<Vec<_>>()
        }
    };
    let offset = {
        let heights_in_order = heights_in_order.clone();
        move || {
            let Some(drag) = drag.get() else { return 0.0 };
            let me = index.get();
            if me == drag.from {
                return drag.dy;
            }
            let all = heights_in_order();
            let to = drop_position(&all, drag.from, drag.dy);
            let dragged = all.get(drag.from).copied().unwrap_or(HEIGHT_MD);
            shift(me, drag.from, to, dragged)
        }
    };
    let dragging = computed(move || drag.with(|d| d.is_some_and(|d| d.from == index.get())));
    let reduced = reduce_motion();
    let edit = shortcut.with_untracked(|s| !s.builtin).then(|| {
        let tell = tell.clone();
        let id = id.clone();
        Rc::new(move || tell(json!({"ev": "edit", "id": id}))) as Rc<dyn Fn()>
    });
    let remove = icon_button(
        "minus.circle.fill",
        t!(
            "Remove %@ from toolbar",
            arg1 = shortcut.with_untracked(|s| s.summary.clone())
        ),
    )
    .size(IconButtonSize::Sm)
    .tone(Tone::Destructive)
    .on_press({
        let tell = tell.clone();
        let id = id.clone();
        move || tell(json!({"ev": "remove-from-toolbar", "id": id}))
    })
    .test_id(&format!("shortcut-remove-{id}"));
    let handle = view()
        .w(px(touch_target()))
        .h(px(touch_target()))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .role(AccessibilityRole::Adjustable)
        .label(move || t!("Reorder %@", arg1 = shortcut.with(|s| s.summary.clone())))
        // VoiceOver's swipe up moves the shortcut a place earlier in the bar,
        // down a place later (the drag's move, a step at a time).
        .accessibility_actions(vec![
            nsh_core::AccessibilityActionDef {
                name: "increment".into(),
                label: None,
            },
            nsh_core::AccessibilityActionDef {
                name: "decrement".into(),
                label: None,
            },
        ])
        .on_accessibility_action({
            let tell = tell.clone();
            move |action: &str| {
                let from = index.get_untracked();
                let count = state.toolbar.with_untracked(Vec::len);
                let to = match action {
                    "increment" if from > 0 => from - 1,
                    "decrement" if from + 1 < count => from + 1,
                    _ => return,
                };
                let destination = move_destination(from, to);
                state.toolbar.update(|t| *t = moved(t, from, destination));
                tell(json!({"ev": "move", "from": from, "to": destination}));
            }
        })
        .test_id(&format!("shortcut-grip-{id}"))
        .claim_touches(TouchClaim::on_start())
        .on_responder_grant(move |_: &ResponderEvent| {
            drag.set(Some(Drag {
                from: index.get_untracked(),
                dy: 0.0,
            }));
            haptic(Haptic::Selection);
        })
        .on_responder_move(move |event: &ResponderEvent| {
            drag.update(|d| {
                if let Some(d) = d {
                    d.dy = event.gesture.delta.y.0;
                }
            });
        })
        .on_responder_release({
            let heights_in_order = heights_in_order.clone();
            move |_: &ResponderEvent| {
                let Some(done) = drag.get_untracked() else {
                    return;
                };
                drag.set(None);
                let to = drop_position(&heights_in_order(), done.from, done.dy);
                if to != done.from {
                    let destination = move_destination(done.from, to);
                    state
                        .toolbar
                        .update(|t| *t = moved(t, done.from, destination));
                    tell(json!({"ev": "move", "from": done.from, "to": destination}));
                }
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| drag.set(None))
        .child(
            icon("line.3.horizontal")
                .size(IconSize::Md)
                .tone(Tone::Subtle),
        );
    let id_for_layout = id.clone();
    view()
        .w_full()
        .relative()
        // Its own ground only while it is lifted: the card's is under it
        // otherwise.
        .bg(move || {
            if dragging.get() {
                token("card")
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .opacity(move || if dragging.get() { 0.92_f32 } else { 1.0 })
        .translate_y(move || px(offset()))
        .z_index(move || if dragging.get() { 10 } else { 0 })
        // While a drag is under way the others make room over `base`; the
        // dragged row follows the finger with none, and the drop lands at
        // once (the new order and the offsets' end come together).
        .transition_transform()
        .duration(move || {
            if reduced || dragging.get() || drag.with(Option::is_none) {
                0.0
            } else {
                Duration::Base.ms()
            }
        })
        .ease_in_out()
        .on_layout(move |bounds: &Bounds<Points>| {
            heights
                .borrow_mut()
                .insert(id_for_layout.clone(), bounds.size.height.0);
        })
        .child(edit_row(
            shortcut,
            move || index.get() > 0 && !dragging.get(),
            remove.into_element(),
            edit,
            Some(handle.into_element()),
            format!("shortcut-row-{id}"),
            || false,
        ))
        .into_element()
}

// --- Yours ------------------------------------------------------------------------

/// YOURS: the user's own, each with its − (which deletes it, asking in the
/// system's alert) and a press to edit; or a word that there are none.
fn custom_section(state: State, tell: Tell) -> AnyElement {
    let has_custom = computed(move || state.custom.with(|c| !c.is_empty()));
    // Made again only when the first is made or the last deleted.
    let body = dynamic(move || {
        let any = has_custom.get();
        let tell = tell.clone();
        untracked(move || {
            let section = section()
                .footer(t!(
                    "Tap ＋ in the top right to create a custom shortcut — key combos, text snippets, and command chains. Tap one to edit it."
                ))
                .test_id("shortcuts-custom");
            let section = if any {
                section.item(
                    each(
                        state.custom,
                        |s: &Shortcut| s.id.clone(),
                        move |shortcut, index| custom_row(shortcut, index, tell.clone()),
                    ),
                    Inset::Lead,
                )
            } else {
                section.item(
                    empty(t!("No custom shortcuts yet"))
                        .variant(EmptyVariant::Inline)
                        .icon("keyboard")
                        .test_id("shortcuts-no-custom")
                        .into_element(),
                    Inset::Content,
                )
            };
            section.into_element()
        })
    });
    view()
        .w_full()
        .gap(px(8.0))
        .child(
            view()
                .ps(px(PADDING_X))
                .child(eyebrow(text(t!("Yours"))).role(AccessibilityRole::Header)),
        )
        .child(body)
        .into_element()
}

/// One of the user's own: − asks the system's alert, whose Delete tells
/// the host `{"ev": "delete", "id"}`; a press on the row edits it.
fn custom_row(shortcut: ReadSignal<Shortcut>, index: ReadSignal<usize>, tell: Tell) -> AnyElement {
    let id = shortcut.with_untracked(|s| s.id.clone());
    let summary = shortcut.with_untracked(|s| s.summary.clone());
    let delete = icon_button("minus.circle.fill", t!("Delete %@", arg1 = summary))
        .size(IconButtonSize::Sm)
        .tone(Tone::Destructive)
        .on_press({
            let tell = tell.clone();
            let id = id.clone();
            move || {
                let label = shortcut.with_untracked(|s| s.label.clone());
                let answer = alert_dialog()
                    .title(delete_title(&label))
                    .description(t!("It comes off the toolbar and out of your shortcuts."))
                    .destructive("delete", t!("Delete"))
                    .cancel("cancel", t!("Cancel"))
                    .show();
                let (tell, id) = (tell.clone(), id.clone());
                drop(spawn(async move {
                    if answer
                        .await
                        .is_ok_and(|r| r.action.as_deref() == Some("delete"))
                    {
                        tell(json!({"ev": "delete", "id": id}));
                    }
                }));
            }
        })
        .test_id(&format!("shortcut-delete-{id}"));
    let edit = {
        let id = id.clone();
        Rc::new(move || tell(json!({"ev": "edit", "id": id}))) as Rc<dyn Fn()>
    };
    edit_row(
        shortcut,
        move || index.get() > 0,
        delete.into_element(),
        Some(edit),
        None,
        format!("shortcut-custom-{id}"),
        || false,
    )
}

// --- Available --------------------------------------------------------------------

/// AVAILABLE: every shortcut off the bar, each with its green ＋. While the
/// bar is full the whole group is disabled, and a line under its title says
/// why (B233: a ＋ that does nothing says why).
fn available_section(state: State, tell: Tell) -> AnyElement {
    let full = computed(move || state.full());
    let rows = each(
        state.available,
        |s: &Shortcut| s.id.clone(),
        move |shortcut, index| available_row(shortcut, index, full, tell.clone()),
    );
    titled(
        || t!("Available"),
        Some(Rc::new(move || {
            full.get().then(|| {
                SharedString::from(t!(
                    "The bar holds up to %lld.",
                    arg1 = state.limit.get_untracked()
                ))
            })
        }) as Rc<dyn Fn() -> Option<SharedString>>),
        section()
            .item(rows, Inset::Lead)
            .test_id("shortcuts-available"),
    )
}

/// A shortcut off the bar: ＋ puts it on (`{"ev": "add-to-toolbar", "id"}`);
/// a custom one opens its editor on a press. While the bar is full the row
/// is disabled: ＋ and the press both, the row dimmed.
fn available_row(
    shortcut: ReadSignal<Shortcut>,
    index: ReadSignal<usize>,
    full: Memo<bool>,
    tell: Tell,
) -> AnyElement {
    let id = shortcut.with_untracked(|s| s.id.clone());
    let add = icon_button(
        "plus.circle.fill",
        t!(
            "Add %@ to toolbar",
            arg1 = shortcut.with_untracked(|s| s.summary.clone())
        ),
    )
    .size(IconButtonSize::Sm)
    .tone(Tone::Success)
    .disabled(move || full.get())
    .on_press({
        let tell = tell.clone();
        let id = id.clone();
        move || tell(json!({"ev": "add-to-toolbar", "id": id}))
    })
    .test_id(&format!("shortcut-add-{id}"));
    // A custom shortcut opens its editor from here too.
    let edit = shortcut.with_untracked(|s| !s.builtin).then(|| {
        let id = id.clone();
        Rc::new(move || tell(json!({"ev": "edit", "id": id}))) as Rc<dyn Fn()>
    });
    edit_row(
        shortcut,
        move || index.get() > 0,
        add.into_element(),
        edit,
        None,
        format!("shortcut-available-{id}"),
        move || full.get(),
    )
}

// --- A row ------------------------------------------------------------------------

/// A shortcut's row, on the row's lanes: its control (−, ＋) in the leading
/// lane, its chip and what it does in the content lane (a press there
/// edits it, when `edit` says how), and the handle (`trailing`) at its end.
/// A hairline above it while `separated()` (every row but the first);
/// dimmed, and its press off, while `disabled()`.
fn edit_row(
    shortcut: ReadSignal<Shortcut>,
    separated: impl Fn() -> bool + 'static,
    control: AnyElement,
    edit: Option<Rc<dyn Fn()>>,
    trailing: Option<AnyElement>,
    content_id: String,
    disabled: impl Fn() -> bool + Clone + 'static,
) -> AnyElement {
    let (builtin, glyph, label) =
        shortcut.with_untracked(|s| (s.builtin, s.glyph(), s.label.clone()));
    let mut face = chip(label).mono().variant(if builtin {
        ChipVariant::Default
    } else {
        ChipVariant::Custom
    });
    if let Some(name) = glyph {
        face = face.symbol(name);
    }
    let summary = move || shortcut.with(|s| s.summary.clone());
    let mut content = view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(HEIGHT_MD))
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .label(summary)
        .opacity({
            let disabled = disabled.clone();
            move || if disabled() { 0.4_f32 } else { 1.0 }
        })
        // The chip shows (display only: no press, no element of its own),
        // the row speaks: one target.
        .child(face)
        .child(
            TextToken::Body
                .apply(text(summary))
                .text_color(move || token("foreground"))
                .number_of_lines(1u32)
                .flex_shrink(1.0),
        )
        .test_id(&content_id);
    if let Some(edit) = edit {
        content = content
            .role(AccessibilityRole::Button)
            .disabled(disabled)
            .active(|s| s.opacity(0.6))
            .on_press(move || edit());
    }
    let mut row = view()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .ps(px(PADDING_X))
        .pe(px(if trailing.is_some() { 0.0 } else { PADDING_X }))
        .child(
            view()
                .w(px(LEADING))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .child(control),
        )
        .child(content);
    if let Some(trailing) = trailing {
        row = row.child(trailing);
    }
    view()
        .w_full()
        .child(when(separated, || separator(Inset::Lead.points())))
        .child(row)
        .into_element()
}

/// The position a row dragged from `from` by `dy` takes, given the rows'
/// heights in order: past the middle of a neighbour, it takes its place.
pub fn drop_position(heights: &[f32], from: usize, dy: f32) -> usize {
    if heights.is_empty() || from >= heights.len() {
        return from;
    }
    let mut position = from;
    if dy > 0.0 {
        let mut travelled = 0.0;
        for (i, height) in heights.iter().enumerate().skip(from + 1) {
            if dy > travelled + height / 2.0 {
                position = i;
                travelled += height;
            } else {
                break;
            }
        }
    } else if dy < 0.0 {
        let mut travelled = 0.0;
        for i in (0..from).rev() {
            if -dy > travelled + heights[i] / 2.0 {
                position = i;
                travelled += heights[i];
            } else {
                break;
            }
        }
    }
    position
}

/// How far the row at `index` moves aside while the row dragged from
/// `drag.from` is over `to` (`dragged` its height).
fn shift(index: usize, from: usize, to: usize, dragged: f32) -> f32 {
    if to > from && index > from && index <= to {
        -dragged
    } else if to < from && index >= to && index < from {
        dragged
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(id: &str, kind: &str, builtin: bool) -> Shortcut {
        Shortcut {
            id: id.into(),
            kind: kind.into(),
            label: id.into(),
            summary: format!("{id} summary"),
            builtin,
        }
    }

    #[test]
    fn messages_round_trip() {
        let message = Message::Shortcuts(Shortcuts {
            toolbar: vec![
                shortcut("esc", "keyCombo", true),
                shortcut("files", "files", true),
            ],
            custom: vec![shortcut("mine", "text", false)],
            available: vec![shortcut("^L", "keyCombo", true)],
            limit: 12,
        });
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let error = Message::parse(r#"{"op":"shortcuts","toolbar":[],"custom":[]}"#).unwrap_err();
        assert!(error.contains("available"), "{error}");
        let error = Message::parse(r#"{"op":"keys"}"#).unwrap_err();
        assert!(error.contains("no op"), "{error}");
    }

    #[test]
    fn the_kinds_drawn_as_icons() {
        assert_eq!(glyph("files"), Some("folder"));
        assert_eq!(glyph("mic"), Some("mic"));
        assert_eq!(
            glyph("dpad"),
            Some("arrow.up.and.down.and.arrow.left.and.right")
        );
        assert_eq!(glyph("keyCombo"), None);
        assert_eq!(glyph("arrows"), None, "the cluster's chip is its label");
    }

    #[test]
    fn a_drag_lands_past_the_middle_of_a_neighbour() {
        let heights = [44.0, 44.0, 60.0, 44.0];
        assert_eq!(drop_position(&heights, 0, 0.0), 0);
        assert_eq!(drop_position(&heights, 0, 21.0), 0);
        assert_eq!(drop_position(&heights, 0, 23.0), 1);
        assert_eq!(drop_position(&heights, 0, 44.0 + 31.0), 2);
        assert_eq!(drop_position(&heights, 0, 1000.0), 3);
        assert_eq!(
            drop_position(&heights, 3, -29.0),
            3,
            "half of the 60 pt row above"
        );
        assert_eq!(drop_position(&heights, 3, -31.0), 2);
        assert_eq!(drop_position(&heights, 3, -1000.0), 0);
        assert_eq!(drop_position(&[], 0, 50.0), 0);
    }

    #[test]
    fn moves_follow_swiftuis_on_move() {
        let bar = ["a", "b", "c", "d"];
        // Down: a to where c is → before d.
        assert_eq!(move_destination(0, 2), 3);
        assert_eq!(moved(&bar, 0, 3), ["b", "c", "a", "d"]);
        // Up: d to where b is → before b.
        assert_eq!(move_destination(3, 1), 1);
        assert_eq!(moved(&bar, 3, 1), ["a", "d", "b", "c"]);
        assert_eq!(moved(&bar, 1, 1), bar);
        assert_eq!(moved(&bar, 9, 0), bar);
    }

    #[test]
    fn rows_make_room_for_the_dragged_one() {
        assert_eq!(shift(1, 0, 2, 44.0), -44.0);
        assert_eq!(shift(2, 0, 2, 44.0), -44.0);
        assert_eq!(shift(3, 0, 2, 44.0), 0.0);
        assert_eq!(shift(1, 3, 1, 44.0), 44.0);
        assert_eq!(shift(0, 3, 1, 44.0), 0.0);
    }
}
