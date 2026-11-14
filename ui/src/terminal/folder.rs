//! A folder on the host (`RemoteFolderSheet`, `RemoteFolderView` in
//! `Moshpit/UI/Terminal/RemotePreview.swift`) in NSH: what a tapped folder
//! path or the Files key opens, in the sheet's own stack (a subfolder is
//! pushed as a page of its own). Images and videos in a grid of
//! thumbnails, the rest in rows; a tap opens a file in the preview (the
//! host's QuickLook or player) or a folder in place.
//!
//! The host owns the folder (`RemoteFolderModel`: the listing, the
//! thumbnails, the downloads, the uploads, the edits on the host), the
//! pickers and the preview: it posts what the page shows ([`Message`]),
//! `{"op": "folder", …}` ([`Folder`], each entry already said in the
//! system's words: its date and size), again whenever that changes. The
//! page tells it what the person asks:
//!
//! - `{"ev": "open", "path"}`: a tile or a row (a folder pushes its page).
//! - `{"ev": "back"}`: the bar's back button on a pushed folder.
//! - `{"ev": "climb", "path"}`: the title's menu, every folder above this
//!   one: the sheet starts over there.
//! - `{"ev": "add", "item": "photos" | "files"}`: the ＋ menu's uploads,
//!   which the host's pickers take; New Folder asks for its name first
//!   (the host's text prompt, [`super::prompt`]): `{"ev": "new-folder",
//!   "name"}`.
//! - A tile's or a row's long-press menu is the host's (`NSHContextMenus`
//!   on `remote-folder-item-<path>`: Open and Share for a file, Rename,
//!   Delete); its pick comes back as `{"op": "act", "action", "path"}` and
//!   the page goes on: `{"ev": "open" | "share", "path"}`, a rename asks for
//!   the new name (`{"ev": "rename", "path", "name"}`), a delete asks on
//!   NSH's action sheet (`{"ev": "delete", "path"}`).
//! - `{"ev": "refresh"}`: pulled down.
//! - A failure to open or to change something is NSH's alert; its OK tells
//!   `{"ev": "dismiss", "failure": "open" | "edit"}`.
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.9 (R13), on the `ui` layer: a
//! skeleton (six tiles, four rows) while the first listing comes, crossfading
//! into it; the media in a grid of `radius-sm` thumbnails (a video's length
//! in its corner), the rest in a section of rows (the kind's icon, the name
//! over its size and date, a folder's chevron); an empty folder or one that
//! would not open says so on the page, with Upload… or Retry.

use std::{cell::RefCell, rc::Rc};

use nsh_core::{EllipsizeMode, color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::prompt::{Answer, Prompt, Prompts};
use crate::{
    channel::{self, Channel},
    face,
    nav_bar::{self, BarMetrics},
    screen::Screen,
    theme::hex_string,
    ui::{
        alert::{AlertDialogOptions, alert_dialog, delete_title},
        button::{Variant, button},
        empty::{empty, loaded, skeleton_list},
        feedback::{self, SpinnerSize, spinner},
        icon::{IconSize, Tone, icon},
        menu::{MenuEntry, menu},
        row::row as list_row,
        section::section,
        tokens::{number, token},
        typography::TextToken,
    },
};

/// One file or folder, as the host says it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub dir: bool,
    /// A video (its tile has the play badge).
    pub video: bool,
    /// The SF Symbol of its kind (`RemoteFileIcon`).
    pub symbol: String,
    /// When it changed and how big it is, in the system's words.
    pub meta: String,
    /// The host has its thumbnail (`moshpit/remote-thumbnail` draws it).
    pub thumb: bool,
    /// A video's length (`1:05`).
    pub duration: Option<String>,
    /// It is downloading to open, this far.
    pub opening: Option<f64>,
}

/// A failure to say in an alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub title: String,
    pub message: String,
}

/// `{"op": "folder", …}`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Folder {
    pub name: String,
    pub path: String,
    /// The folders above this one, nearest first (the title's menu).
    pub parents: Vec<String>,
    /// The page was pushed (it has a back button).
    pub pushed: bool,
    pub loading: bool,
    /// Why the listing failed.
    pub failure: Option<String>,
    /// An upload is under way, this far.
    pub uploading: Option<f64>,
    /// Images and videos, then everything else.
    pub media: Vec<Entry>,
    pub rows: Vec<Entry>,
    /// A file that would not open (`CAN'T OPEN`), a change the host refused.
    pub open_failure: Option<String>,
    pub edit_failure: Option<Failure>,
    /// Each failure's number, which the host gives every failure it sets
    /// anew and never reuses: an alert is said once per number. The words
    /// can't tell a failure apart from the one before it (the same file
    /// failing again), and the host may never post the cleared state in
    /// between (it posts what it has when it next draws).
    pub open_failure_id: u64,
    pub edit_failure_id: u64,
}

/// What a tile's or a row's menu picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Act {
    Open,
    Share,
    Rename,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActFields {
    action: Act,
    path: String,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Folder(Box<Folder>),
    Act { action: Act, path: String },
    Prompted(Answer),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("remote-folder", json)?;
        let fields = serde_json::Value::Object(fields);
        let error = |e: serde_json::Error| format!("remote-folder: {op}: {e}");
        match op.as_str() {
            "folder" => serde_json::from_value(fields)
                .map(|f| Message::Folder(Box::new(f)))
                .map_err(error),
            "act" => serde_json::from_value::<ActFields>(fields)
                .map(|a| Message::Act {
                    action: a.action,
                    path: a.path,
                })
                .map_err(error),
            "prompted" => serde_json::from_value(fields)
                .map(Message::Prompted)
                .map_err(error),
            other => Err(format!("remote-folder: no op {other:?}")),
        }
    }
}

/// The name of the last component of `path` (`/` for the root).
pub fn last_component(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => "/".to_owned(),
    }
}

/// The delete's question about `entry`.
pub fn delete_question(entry: &Entry) -> AlertDialogOptions {
    let message = if entry.dir {
        t!(
            "“%@” and everything in it will be deleted from the host.",
            arg1 = entry.name
        )
    } else {
        t!("“%@” will be deleted from the host.", arg1 = entry.name)
    };
    alert_dialog()
        .title(delete_title(&entry.name))
        .description(message)
        .action_sheet()
        .destructive("delete", t!("Delete"))
        .cancel("cancel", t!("Cancel"))
}

/// The grid's columns in `width` (`GridItem(.adaptive(minimum: 100,
/// maximum: 180), spacing: 10)`): as many as fit at the least width, each
/// as wide as they share, no wider than the most.
pub fn columns(width: f32) -> (usize, f32) {
    let count = (((width + SPACING) / (TILE_MIN + SPACING)).floor() as usize).max(1);
    let each = ((width - SPACING * (count as f32 - 1.0)) / count as f32).min(TILE_MAX);
    (count, each)
}

/// What is drawn over a picture (a video's length, its play mark): white on
/// the scrim in either appearance, as the picture is no theme's.
const ON_MEDIA: Rgba = Rgba {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
/// The title menu's disc.
const TITLE_DISC: f32 = 17.0;
const TILE_MIN: f32 = 100.0;
const TILE_MAX: f32 = 180.0;
/// Between the grid's columns and its rows (`space-2`).
const SPACING: f32 = 8.0;
const ROW_SPACING: f32 = 8.0;
/// The skeleton's tiles while the first listing comes.
const SKELETON_TILES: usize = 6;
const SKELETON_ROWS: usize = 4;

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

type Tell = Rc<dyn Fn(serde_json::Value)>;

/// A folder of the host.
#[component]
pub fn RemoteFolderScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let bar = BarMetrics::or_standard(params.bar);
    let back_title = params.back.clone().unwrap_or_default();
    let folder = signal(Folder::default());
    let ready = signal(false);
    let tell: Tell = {
        let channel = channel.clone();
        Rc::new(move |event| {
            if let Some(channel) = &channel {
                channel.tell(&event);
            }
        })
    };
    let prompts = Prompts::default();
    // The failures said so far, by number: each is said once.
    let saying = Rc::new(RefCell::new(Said::default()));
    {
        let (tell, prompts, saying) = (tell.clone(), prompts.clone(), saying.clone());
        channel::hear(
            channel.as_ref(),
            Screen::RemoteFolder,
            Message::parse,
            move |message| match message {
                Message::Folder(next) => {
                    say_failures(&next, &saying, &tell);
                    if folder.with_untracked(|f| *f != *next) {
                        folder.set(*next);
                    }
                    ready.set(true);
                }
                Message::Act { action, path } => act(action, &path, folder, &tell, &prompts),
                Message::Prompted(answer) => prompts.answer(answer),
            },
        );
    }
    let scrolled = nav_bar::ScrollEdge::new();
    let width = signal(0.0_f32);
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            let refreshing = signal(false);
            scrolled
                .follow(scroll())
                .absolute()
                .inset_0()
                .test_id("remote-folder-list")
                .refreshing(move || refreshing.get())
                .on_refresh({
                    let tell = tell.clone();
                    move || {
                        // The host reloads; the spinner goes when the
                        // listing comes back (it posts again).
                        refreshing.set(true);
                        tell(json!({"ev": "refresh"}));
                        set_timeout(std::time::Duration::from_millis(600), move || {
                            refreshing.set(false)
                        });
                    }
                })
                .child(
                    view()
                        .w_full()
                        .child(view().h(move || nav_bar::bar_space(bar)))
                        .on_layout(move |b: &Bounds<Points>| {
                            if (width.get_untracked() - b.size.width.0).abs() > 0.25 {
                                width.set(b.size.width.0);
                            }
                        })
                        .child(loaded(
                            move || folder.with(|f| !(f.loading && is_empty(f))),
                            move || skeleton(width),
                            move || {
                                let tell = tell.clone();
                                dynamic(move || {
                                    let folder = folder.get();
                                    let tell = tell.clone();
                                    untracked(move || content(folder, width, tell))
                                })
                                .into_element()
                            },
                        ))
                        .child(view().h(move || px(safe_area().bottom.0))),
                )
                .into_element()
        }
    };
    let add = {
        let (tell, prompts) = (tell.clone(), prompts.clone());
        move |item: String| match item.as_str() {
            "folder" => {
                let answer_to = tell.clone();
                prompts.ask(
                    &*tell,
                    Prompt {
                        title: t!("New Folder"),
                        message: None,
                        placeholder: t!("Name"),
                        text: String::new(),
                        confirm: t!("Create"),
                        cancel: t!("Cancel"),
                    },
                    move |text| {
                        let Some(text) = text else { return };
                        let name = text.trim();
                        if !name.is_empty() {
                            answer_to(json!({"ev": "new-folder", "name": name}));
                        }
                    },
                );
            }
            item => tell(json!({"ev": "add", "item": item})),
        }
    };
    let uploading = computed(move || folder.with(|f| f.uploading.is_some()));
    let title = computed(move || folder.with(|f| f.name.clone()));
    let pushed = computed(move || folder.with(|f| f.pushed));
    let navigation = {
        let tell = tell.clone();
        dynamic(move || {
            let pushed = pushed.get();
            let (tell, add, back_title) = (tell.clone(), add.clone(), back_title.clone());
            untracked(move || {
                let bar = nav_bar::NavBar {
                    metrics: bar,
                    back_title,
                    // The title is drawn with its menu's chevron
                    // ([`title_menu`]).
                    title: Box::new(String::new),
                    leading: None,
                    trailing: vec![nav_bar::BarButton {
                        label: t!("Add"),
                        content: nav_bar::BarContent::Symbol("plus"),
                        enabled: computed(move || !uploading.get()),
                        emphasized: false,
                        test_id: "remote-folder-add",
                        on_press: Box::new(|| {}),
                        menu: Some(nav_bar::BarMenu {
                            entries: vec![
                                MenuEntry::new("photos", t!("Photos…"))
                                    .symbol("photo.on.rectangle"),
                                MenuEntry::new("files", t!("Files…")).symbol("doc.badge.plus"),
                                MenuEntry::new("folder", t!("New folder…"))
                                    .symbol("folder.badge.plus"),
                            ],
                            on_select: Box::new(add),
                        }),
                    }],
                    on_back: Box::new({
                        let tell = tell.clone();
                        move || tell(json!({"ev": "back"}))
                    }),
                    scrolled,
                };
                if pushed {
                    nav_bar::nav_bar(bar).into_element()
                } else {
                    nav_bar::root_bar(bar).into_element()
                }
            })
        })
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("remote-folder-sheet")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(navigation)
        .child(title_menu(bar, title, folder, tell))
}

/// The title, with the chevron of its menu (`toolbarTitleMenu`): the
/// folder's full path, and every folder above it — the way out of a working
/// directory when the files are somewhere else. The menu is the host's
/// (`moshpit/menu`), laid over the title.
fn title_menu(
    bar: BarMetrics,
    title: Memo<String>,
    folder: Signal<Folder>,
    tell: Tell,
) -> AnyElement {
    let entries = computed(move || {
        folder.with(|f| {
            f.parents
                .iter()
                .map(|path| MenuEntry::new(path.clone(), last_component(path)).symbol("folder"))
                .collect::<Vec<_>>()
        })
    });
    let placement = move || nav_bar::Placement::new(bar.height, nav_bar::text_size());
    view()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .pt_safe_area()
        .pointer_events(PointerEvents::BoxNone)
        .child(
            view()
                .h(px(bar.height))
                .items_center()
                .pointer_events(PointerEvents::BoxNone)
                .child(
                    view()
                        .relative()
                        .flex_row()
                        .items_start()
                        .gap(px(8.0))
                        .pt(move || px(placement().anchored_top))
                        .child(
                            text(move || title.get())
                                .text_size(move || px(nav_bar::text_size()))
                                .line_height(face::line_height())
                                .allow_font_scaling(false)
                                .number_of_lines(1u32)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || token("foreground"))
                                .role(AccessibilityRole::Header)
                                .test_id("nav-bar-title"),
                        )
                        .child(
                            view()
                                .h(move || px(face::line(nav_bar::text_size())))
                                .justify_center()
                                .child(
                                    // The title menu's mark: a chevron in a
                                    // 17 pt disc of the secondary fill.
                                    view()
                                        .w(px(TITLE_DISC))
                                        .h(px(TITLE_DISC))
                                        .rounded_full()
                                        .bg(move || token("secondary"))
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            native("moshpit/symbol")
                                                .prop("name", json!("chevron.down"))
                                                .prop("pointSize", json!(8.0))
                                                .prop("weight", json!("bold"))
                                                .prop("color", || hex("muted-foreground")),
                                        ),
                                ),
                        )
                        .child(dynamic(move || {
                            let entries = entries.get();
                            let tell = tell.clone();
                            untracked(move || {
                                menu(
                                    entries,
                                    Some("remote-folder-title-menu"),
                                    move || title.get_untracked(),
                                    move |path| tell(json!({"ev": "climb", "path": path})),
                                )
                            })
                        })),
                ),
        )
        .into_element()
}

/// Whether the folder shows nothing.
fn is_empty(folder: &Folder) -> bool {
    folder.media.is_empty() && folder.rows.is_empty()
}

/// The skeleton while the first listing comes: six tiles, four rows.
fn skeleton(width: Signal<f32>) -> AnyElement {
    let tile = move || {
        let (_, side) = columns(inner(width.get_untracked()));
        view()
            .w(px(side))
            .h(px(side))
            .rounded(px(number("radius-sm")))
            .border_curve(BorderCurve::Continuous)
            .bg(move || token("muted"))
            .into_element()
    };
    view()
        .w_full()
        .px(px(number("space-page")))
        .pt(px(16.0))
        .gap(px(24.0))
        .accessibility_elements_hidden(true)
        .test_id("remote-folder-skeleton")
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(SPACING))
                .children((0..SKELETON_TILES).map(|_| tile()).collect::<Vec<_>>()),
        )
        .child(
            view()
                .w_full()
                .rounded(px(number("radius-lg")))
                .border_curve(BorderCurve::Continuous)
                .overflow_hidden()
                .bg(move || token("card"))
                .child(skeleton_list(SKELETON_ROWS).leading(true)),
        )
        .into_element()
}

/// The grid's width in a page `width` wide.
fn inner(width: f32) -> f32 {
    (width - 2.0 * number("space-page")).max(TILE_MIN)
}

/// What the folder holds: the upload's strip, then why it is empty, or the
/// grid and the rows.
fn content(folder: Folder, width: Signal<f32>, tell: Tell) -> AnyElement {
    let mut column = view()
        .w_full()
        .px(px(number("space-page")))
        .pt(px(16.0))
        .gap(px(24.0));
    if let Some(fraction) = folder.uploading {
        column = column.child(upload_strip(fraction));
    }
    if let (Some(failure), true) = (&folder.failure, is_empty(&folder)) {
        let tell = tell.clone();
        column = column.child(
            empty(t!("Couldn't open this folder"))
                .icon("exclamationmark.triangle")
                .message(failure.clone())
                .action(
                    button(t!("Retry"))
                        .variant(Variant::Secondary)
                        .on_press(move || tell(json!({"ev": "refresh"})))
                        .test_id("remote-folder-retry"),
                )
                .test_id("remote-folder-failure"),
        );
    } else if is_empty(&folder) {
        let tell = tell.clone();
        column = column.child(
            empty(t!("This folder is empty"))
                .icon("folder")
                .message(t!(
                    "Only images, video, audio, PDFs and documents are shown here."
                ))
                .action(
                    button(t!("Upload…"))
                        .variant(Variant::Secondary)
                        .on_press(move || tell(json!({"ev": "add", "item": "files"})))
                        .test_id("remote-folder-upload"),
                )
                .test_id("remote-folder-empty"),
        );
    } else {
        if !folder.media.is_empty() {
            column = column.child(grid(folder.media.clone(), width, tell.clone()));
        }
        if !folder.rows.is_empty() {
            column = column.child(rows(folder.rows.clone(), tell));
        }
    }
    column.into_element()
}

/// `UPLOADING 42%` and its bar.
fn upload_strip(fraction: f64) -> AnyElement {
    let percent = (fraction * 100.0).floor() as i64;
    view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .test_id("remote-folder-uploading")
        .child(
            TextToken::MonoCaption
                .apply(text(t!("UPLOADING %lld%%", arg1 = percent.to_string())))
                .number_of_lines(1u32)
                .tabular_nums()
                .text_color(move || token("primary-text"))
                .flex_shrink_0(),
        )
        .child(view().flex_1().child(feedback::progress(
            move || fraction,
            move || format!("{percent}%"),
        )))
        .into_element()
}

/// The images and videos, in columns as wide as fit.
fn grid(media: Vec<Entry>, width: Signal<f32>, tell: Tell) -> AnyElement {
    dynamic(move || {
        let (count, each) = columns(inner(width.get()));
        let (media, tell) = (media.clone(), tell.clone());
        untracked(move || {
            let mut grid = view().gap(px(ROW_SPACING));
            for chunk in media.chunks(count) {
                let mut line = view().flex_row().gap(px(SPACING));
                for entry in chunk {
                    line = line.child(tile(entry.clone(), each, tell.clone()));
                }
                grid = grid.child(line);
            }
            grid.into_element()
        })
    })
    .into_element()
}

/// A pill over a picture: the scrim, white words or a white mark.
fn on_picture(child: impl IntoElement) -> View {
    view()
        .absolute()
        .bottom(px(6.0))
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(number("radius-xs")))
        .bg(move || token("scrim"))
        .child(child)
}

/// An image or a video: a square thumbnail cropped to fill (or its kind's
/// symbol), the play mark and its length on a video, the download's spinner
/// while it opens; its name under it.
fn tile(entry: Entry, side: f32, tell: Tell) -> AnyElement {
    let path = entry.path.clone();
    let mut square = view()
        .w(px(side))
        .h(px(side))
        .rounded(px(number("radius-sm")))
        .border_curve(BorderCurve::Continuous)
        .overflow_hidden()
        .bg(move || token("muted"))
        .items_center()
        .justify_center();
    square = if entry.thumb {
        square.child(
            native("moshpit/remote-thumbnail")
                .absolute()
                .inset_0()
                .prop("path", json!(entry.path)),
        )
    } else {
        square.child(
            icon(symbol(&entry.symbol))
                .size(IconSize::Lg)
                .tone(Tone::Muted),
        )
    };
    if entry.video {
        square = square
            .child(on_picture(super::symbol("play.fill", 10.0, "bold", ON_MEDIA)).left(px(6.0)));
    }
    if let Some(duration) = &entry.duration {
        square = square.child(
            on_picture(
                TextToken::MonoCaption
                    .apply(text(duration.clone()))
                    .number_of_lines(1u32)
                    .tabular_nums()
                    .text_color(ON_MEDIA),
            )
            .right(px(6.0)),
        );
    }
    if entry.opening.is_some() {
        square = square.child(
            view()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .bg(move || token("scrim"))
                .child(spinner(SpinnerSize::Md)),
        );
    }
    view()
        .w(px(side))
        .gap(px(4.0))
        .role(AccessibilityRole::Button)
        .label(format!("{}, {}", entry.name, entry.meta))
        .test_id(&format!("remote-folder-item-{path}"))
        .active(|s| s.opacity(0.6))
        .on_press(move || tell(json!({"ev": "open", "path": path})))
        .child(square)
        .child(
            TextToken::Caption
                .apply(text(entry.name.clone()))
                .number_of_lines(1u32)
                .text_overflow(EllipsizeMode::Middle)
                .text_color(move || token("muted-foreground"))
                .accessibility_elements_hidden(true),
        )
        .into_element()
}

/// An SF Symbol's name the host sent, kept for the life of the page (the
/// `ui` layer's icons take a `'static` name; a folder holds a few kinds).
pub(super) fn symbol(name: &str) -> &'static str {
    thread_local! {
        static NAMES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }
    NAMES.with(|names| {
        let mut names = names.borrow_mut();
        if let Some(known) = names.iter().find(|n| **n == name) {
            return *known;
        }
        let kept: &'static str = Box::leak(name.to_owned().into_boxed_str());
        names.push(kept);
        kept
    })
}

/// The folders and documents: a section of rows, each its kind's icon (a
/// folder's in the accent), its name over its size and date, a folder's
/// chevron; the download's spinner while one opens.
fn rows(entries: Vec<Entry>, tell: Tell) -> AnyElement {
    section()
        .rows(entries.into_iter().map(|entry| {
            let path = entry.path.clone();
            let tell = tell.clone();
            let opening = entry.opening.is_some();
            let row = list_row(entry.name.clone());
            let row = if entry.meta.is_empty() {
                row
            } else {
                row.subtitle(entry.meta.clone())
            };
            let row = row
                .leading(icon(symbol(&entry.symbol)).tone(if entry.dir {
                    Tone::Primary
                } else {
                    Tone::Muted
                }))
                .loading(move || opening)
                .on_press(move || tell(json!({"ev": "open", "path": path})))
                .test_id(&format!("remote-folder-item-{}", entry.path));
            if entry.dir { row.chevron() } else { row }
        }))
        .test_id("remote-folder-rows")
        .into_element()
}

/// A menu's pick on `path`: go on, asking where it asks.
fn act(action: Act, path: &str, folder: Signal<Folder>, tell: &Tell, prompts: &Prompts) {
    let Some(entry) = folder.with_untracked(|f| {
        f.media
            .iter()
            .chain(&f.rows)
            .find(|e| e.path == path)
            .cloned()
    }) else {
        return;
    };
    match action {
        Act::Open => tell(json!({"ev": "open", "path": entry.path})),
        Act::Share => tell(json!({"ev": "share", "path": entry.path})),
        Act::Rename => {
            let answer_to = tell.clone();
            let before = entry.name.clone();
            let path = entry.path.clone();
            prompts.ask(
                &**tell,
                Prompt {
                    title: t!("Rename"),
                    message: None,
                    placeholder: t!("Name"),
                    text: entry.name.clone(),
                    confirm: t!("Rename"),
                    cancel: t!("Cancel"),
                },
                move |text| {
                    let Some(name) = text else { return };
                    if name.trim().is_empty() || name == before {
                        return;
                    }
                    answer_to(json!({"ev": "rename", "path": path, "name": name}));
                },
            );
        }
        Act::Delete => {
            let answer = delete_question(&entry).show();
            let (tell, path) = (tell.clone(), entry.path.clone());
            drop(spawn(async move {
                if answer
                    .await
                    .is_ok_and(|r| r.action.as_deref() == Some("delete"))
                {
                    tell(json!({"ev": "delete", "path": path}));
                }
            }));
        }
    }
}

/// The number of the last failure of each kind said.
#[derive(Default)]
struct Said {
    open: Option<u64>,
    edit: Option<u64>,
}

/// Says a failure new in `folder` in NSH's alert, once; its OK tells the
/// host, which clears it. New means a number not said yet
/// ([`Folder::open_failure_id`]): a post the host sent before it heard the
/// OK (a thumbnail coming in) still carries the failure just answered, its
/// number the same, and is not said again; the same words failing again
/// come with a new number and are said, whether or not the host ever
/// posted the cleared state in between.
fn say_failures(folder: &Folder, saying: &Rc<RefCell<Said>>, tell: &Tell) {
    let (open_new, edit_new) = {
        let mut said = saying.borrow_mut();
        let open_new = folder.open_failure.is_some() && said.open != Some(folder.open_failure_id);
        let edit_new = folder.edit_failure.is_some() && said.edit != Some(folder.edit_failure_id);
        if open_new {
            said.open = Some(folder.open_failure_id);
        }
        if edit_new {
            said.edit = Some(folder.edit_failure_id);
        }
        (open_new, edit_new)
    };
    if let (Some(message), true) = (&folder.open_failure, open_new) {
        let answer = alert_dialog()
            .title(t!("CAN'T OPEN"))
            .description(message.clone())
            .cancel("ok", t!("OK"))
            .show();
        let tell = tell.clone();
        drop(spawn(async move {
            let _ = answer.await;
            tell(json!({"ev": "dismiss", "failure": "open"}));
        }));
    }
    if let (Some(failure), true) = (&folder.edit_failure, edit_new) {
        let answer = alert_dialog()
            .title(failure.title.clone())
            .description(failure.message.clone())
            .cancel("ok", t!("OK"))
            .show();
        let tell = tell.clone();
        drop(spawn(async move {
            let _ = answer.await;
            tell(json!({"ev": "dismiss", "failure": "edit"}));
        }));
    }
}

/// `name` as the host's props take a color: `#RRGGBB(AA)`.
fn hex(name: &'static str) -> serde_json::Value {
    json!(hex_string(Rgba::from(color(token(name)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_parse_and_bad_ones_are_refused() {
        let Ok(Message::Folder(folder)) = Message::parse(
            r#"{"op":"folder","name":"shots","path":"/tmp/shots","parents":["/tmp","/"],
                "media":[{"path":"/tmp/shots/a.png","name":"a.png","symbol":"photo","meta":"2 min. ago · 1 MB","thumb":true}],
                "rows":[{"path":"/tmp/shots/raw","name":"raw","dir":true,"symbol":"folder","meta":"1 hr. ago"}],
                "editFailure":{"title":"Couldn't rename","message":"Something with that name is already there."}}"#,
        ) else {
            panic!()
        };
        assert!(folder.media[0].thumb);
        assert!(folder.rows[0].dir);
        assert_eq!(folder.edit_failure.unwrap().title, "Couldn't rename");
        assert_eq!(
            Message::parse(r#"{"op":"act","action":"share","path":"/a"}"#),
            Ok(Message::Act {
                action: Act::Share,
                path: "/a".into()
            })
        );
        assert!(Message::parse(r#"{"op":"folder","colour":1}"#).is_err());
        assert!(Message::parse(r#"{"op":"act","action":"zip","path":"/a"}"#).is_err());
    }

    #[test]
    fn the_grid_fits_as_many_columns_as_it_can() {
        // 8 pt apart (space-2), at least 100 wide.
        assert_eq!(columns(552.0), (5, 104.0));
        assert_eq!(columns(100.0), (1, 100.0));
        // Wide: no column wider than 180.
        assert_eq!(columns(90.0).0, 1);
        assert_eq!(columns(1000.0), (9, 104.0));
        assert_eq!(columns(370.0), (3, 118.0));
    }

    #[test]
    fn a_path_names_its_last_folder() {
        assert_eq!(last_component("/Users/me/shots"), "shots");
        assert_eq!(last_component("/Users/me/shots/"), "shots");
        assert_eq!(last_component("/"), "/");
    }
}
