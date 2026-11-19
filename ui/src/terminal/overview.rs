//! The pane page (the pane-switching design's steps 2 and 3: names,
//! states and pictures): the panes of the window shown, a card each, over
//! the terminal screen. It is the one full-screen grid: the session and the
//! window (herdr: workspace and tab) are picked from the host's quick
//! pickers, hung from the breadcrumb.
//!
//! - Opened from the breadcrumb's pane segment, ⌘K, and VoiceOver's Show
//!   all panes on the terminal (the host's `{"op": "overview", "open":
//!   true}`); closed by Done, ⌘K again, Escape, or a pane picked.
//! - A card per pane of the window shown: its picture, then what runs in
//!   it and its state (the state's dot first), and its ⋯ (the pane's
//!   menu); the current one has the accent's ring. Last, "＋ New pane",
//!   which splits the window at once.
//! - A pane picked switches to it ([`Intent::SelectPane`]) and the page
//!   goes.
//! - Its long press is the host's menu (`overview-pane-<id>`: Open, New
//!   Pane, Rename window…, Kill pane…), its pick coming back as `{"op":
//!   "overview-act"}`; a rename asks for the name (the host's prompt); a
//!   kill asks on NSH's action sheet first.
//! - The bar's capsule names the window and opens the windows' quick
//!   picker (herdr: tabs), as the breadcrumb's window segment does.
//! - Pictures (B658): a pane's cell shows the end of its screen as tiny
//!   text, read from the terminals the session already keeps (every pane of
//!   an SSH tmux attach has one; a mosh or herdr session only the pane it
//!   shows): asked for once on opening and for panes that come while it is
//!   open, never drawn live. A pane without one shows the terminal's
//!   symbol. Decorative: hidden from screen readers.
//! - Its coming and going (B658): the page fades (`base` in, `fast` out;
//!   a cross-fade of `fast` both ways under reduced motion), and tells the
//!   host where the shown pane's cell is (`overview-anchor`), so the live
//!   terminal shrinks into it and grows back out of it, and where a picked
//!   pane's cell is, so its terminal grows from there. The motion is the
//!   host's, under the page; nothing waits for it.
//!
//! The page reads the tree the switcher already has (`{"op":
//! "switcher"}`): the active session, whole; it shows the window of the
//! pane in view.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use nsh_core::{EllipsizeMode, LineHeight, prelude::*};

use super::{
    Crumb, Ctx, Intent,
    cover::chrome_text,
    picker::{self, Kind, Row, Vocab},
    prompt::Prompts,
    switcher::{self, Words, stop},
};
use crate::{
    home::model::{Control, Mux as HomeMux, Signal as AgentSignal},
    nav_bar,
    ui::{
        alert::alert_dialog,
        icon::{IconSize, Tone, icon},
        menu::{MenuEntry, menu},
        motion::{Duration, ENTER},
        status::{Status, status_dot},
        tokens::{number, token},
        typography::TextToken,
    },
};

/// The narrowest a card may be: the grid takes as many columns as fit,
/// two at least, four at most.
pub const CARD_MIN: f32 = 160.0;
/// Between cards, across and down.
pub const GAP: f32 = 12.0;
pub const ROW_GAP: f32 = 20.0;
/// A card's box: its pane's picture.
pub const BOX: f32 = 112.0;
/// A picture's text and its line (points): a texture to recognise a
/// screen by, not to read.
pub const PICTURE_TEXT: f32 = 4.5;
pub const PICTURE_LINE: f32 = 5.5;
/// The most rows a picture keeps (the screen's end).
pub const PICTURE_ROWS: usize = 24;
/// Wide enough for any row at [`PICTURE_TEXT`]: a row is cut by its cell,
/// never wrapped.
const PICTURE_WIDTH: f32 = 1200.0;

/// One pane of a window.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub pane: String,
    /// What runs in it: the agent, else the program.
    pub name: String,
    pub signal: Option<AgentSignal>,
    pub since: Option<f64>,
    pub current: bool,
}

/// One window.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub window: String,
    pub index: i64,
    /// Its name by Home's rules.
    pub place: String,
    /// Its own name (what a rename starts from).
    pub name: String,
    pub cells: Vec<Cell>,
}

impl Card {
    /// The most urgent agent state among its panes: who waits on you, then
    /// who works, then who is done.
    pub fn strongest(&self) -> Option<AgentSignal> {
        self.cells
            .iter()
            .filter_map(|c| c.signal)
            .min_by_key(|s| s.rank())
    }

    /// The card holds the pane shown.
    pub fn current(&self) -> bool {
        self.cells.iter().any(|c| c.current)
    }
}

/// The session's cards.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Overview {
    pub mux: HomeMux,
    /// The session's name, as the breadcrumb says it.
    pub session: String,
    pub cards: Vec<Card>,
}

impl Overview {
    /// `tree`'s active session: its windows by index, their panes by index.
    pub fn of(tree: &Control, words: &Words) -> Overview {
        let Some(session) = tree.active_session.as_deref() else {
            return Overview::default();
        };
        let session_name = tree
            .sessions
            .iter()
            .find(|s| s.id == session)
            .map(|s| tree.session_display_name(s))
            .unwrap_or_default();
        let cards = tree
            .windows_in(session)
            .into_iter()
            .map(|window| {
                let cells: Vec<Cell> = tree
                    .panes_in(&window.id)
                    .into_iter()
                    .map(|pane| {
                        let s = stop(tree, window, pane, words);
                        Cell {
                            current: tree.active_pane.as_deref() == Some(pane.id.as_str()),
                            pane: s.pane,
                            name: s.name,
                            signal: s.signal,
                            since: s.since,
                        }
                    })
                    .collect();
                // The place is the window's, named from the pane it shows
                // (the first, when none is current).
                let named = tree
                    .panes_in(&window.id)
                    .into_iter()
                    .find(|p| tree.active_pane.as_deref() == Some(p.id.as_str()))
                    .or_else(|| tree.panes_in(&window.id).into_iter().next());
                let place = match named {
                    Some(pane) => stop(tree, window, pane, words).place,
                    None => super::model::project_name(&window.name, "", None)
                        .unwrap_or_else(|| window.index.to_string()),
                };
                Card {
                    window: window.id.clone(),
                    index: window.index,
                    place,
                    name: window.name.clone(),
                    cells,
                }
            })
            .collect();
        Overview {
            mux: tree.mux,
            session: session_name,
            cards,
        }
    }

    pub fn panes(&self) -> usize {
        self.cards.iter().map(|c| c.cells.len()).sum()
    }

    /// Panes whose agent waits on you.
    pub fn waiting(&self) -> usize {
        self.cards
            .iter()
            .flat_map(|c| &c.cells)
            .filter(|c| c.signal == Some(AgentSignal::Attention))
            .count()
    }

    pub fn cell(&self, pane: &str) -> Option<(&Card, &Cell)> {
        self.cards.iter().find_map(|card| {
            card.cells
                .iter()
                .find(|c| c.pane == pane)
                .map(|c| (card, c))
        })
    }

    pub fn card(&self, window: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.window == window)
    }

    /// The window the page shows: the one holding the pane in view (the
    /// first, when none is).
    pub fn shown(&self) -> Option<&Card> {
        self.cards
            .iter()
            .find(|c| c.current())
            .or_else(|| self.cards.first())
    }
}

/// How many columns of cards fit `width`.
pub fn columns(width: f32) -> usize {
    (((width + GAP) / (CARD_MIN + GAP)).floor() as usize).clamp(2, 4)
}

/// What a pane's picture shows of its screen's `rows`, `fits` lines of it:
/// the screen up to its last row with text (where the newest output and
/// the prompt are), its last `fits` rows when it has more, from the top
/// when it has fewer; trailing blanks cut. Nothing for a blank screen.
pub fn picture_rows(rows: &[String], fits: usize) -> Vec<String> {
    let Some(last) = rows.iter().rposition(|r| !r.trim().is_empty()) else {
        return Vec::new();
    };
    let shown = &rows[..=last];
    let keep = fits.min(PICTURE_ROWS).min(shown.len());
    shown[shown.len() - keep..]
        .iter()
        .map(|r| r.trim_end().to_owned())
        .collect()
}

/// How long the overview's fade is, `showing` or going (ms): `base` in,
/// `fast` out; under reduced motion a plain cross-fade, `fast` both ways
/// (the host's shrink and growth don't play then).
pub fn fade_ms(reduced: bool, showing: bool) -> f32 {
    match (reduced, showing) {
        (false, true) => Duration::Base.nominal_ms(),
        _ => Duration::Fast.nominal_ms(),
    }
}

fn stop_of(cell: &Cell, card: &Card) -> switcher::Stop {
    switcher::Stop {
        pane: cell.pane.clone(),
        window: card.window.clone(),
        place: card.place.clone(),
        name: cell.name.clone(),
        signal: cell.signal,
        since: cell.since,
    }
}

/// A state in words for screen readers: "working 13 minutes", as Home's
/// rows say it.
fn spoken_state(signal: AgentSignal, since: Option<f64>, now: f64) -> String {
    // The agents' states in the word list (`picker::signal_label`'s).
    let word = match signal {
        AgentSignal::Attention => t!("needs you"),
        AgentSignal::Working => t!("working"),
        AgentSignal::Done => t!("done"),
    };
    let time = crate::home::rows::spoken(since, now);
    if time.is_empty() {
        word
    } else {
        format!("{word} {time}")
    }
}

/// The tone a status is said in.
fn tone(status: Status) -> &'static str {
    match status {
        Status::Attention => "warning-text",
        Status::Working => "success-text",
        Status::Done => "info-text",
        _ => "muted-foreground",
    }
}

/// The multiplexer's words for this tree's.
fn vocab(mux: HomeMux) -> Vocab {
    Vocab::of(match mux {
        HomeMux::Tmux => super::model::Mux::Tmux,
        HomeMux::Herdr => super::model::Mux::Herdr,
    })
}

/// What a pane's menu offers (the host's long press draws the same, by
/// these ids): open it, split its window, rename its window, kill it.
pub fn pane_entries(mux: HomeMux) -> Vec<MenuEntry> {
    let herdr = mux == HomeMux::Herdr;
    vec![
        MenuEntry::new("open", t!("Open")).symbol("arrow.up.right.square"),
        MenuEntry::new("new-pane", t!("New Pane")).symbol("rectangle.split.2x1"),
        MenuEntry::new(
            "rename",
            if herdr {
                t!("Rename tab…")
            } else {
                t!("Rename window…")
            },
        )
        .symbol("pencil"),
        MenuEntry::new(
            "kill",
            if herdr {
                t!("Close pane…")
            } else {
                t!("Kill pane…")
            },
        )
        .symbol("xmark.square")
        .destructive(),
    ]
}

// --- Acting ---------------------------------------------------------------------

/// What the page does for a menu's pick on pane `pane` (`open`, `new-pane`,
/// `rename`, `kill`): switches, splits, asks for the window's new name,
/// asks before the kill.
pub(crate) fn act_on_pane(
    ctx: &Ctx,
    prompts: &Prompts,
    overview: &Overview,
    action: &str,
    pane: &str,
) {
    let Some((card, cell)) = overview.cell(pane) else {
        return;
    };
    match action {
        "open" => pick(ctx, &cell.pane),
        "new-pane" => ctx.ask(Intent::NewPane(card.window.clone())),
        "rename" => rename(ctx, prompts, overview.mux, card),
        "kill" => {
            let herdr = overview.mux == HomeMux::Herdr;
            let (question, button) = if herdr {
                (
                    t!(
                        "Close pane “%1$@” in %2$@? What runs in it stops.",
                        arg1 = cell.name.clone(),
                        arg2 = card.place.clone()
                    ),
                    t!("Close pane"),
                )
            } else {
                (
                    t!(
                        "Kill pane “%1$@” in %2$@? What runs in it stops.",
                        arg1 = cell.name.clone(),
                        arg2 = card.place.clone()
                    ),
                    t!("Kill pane"),
                )
            };
            let answer = alert_dialog()
                .title(question)
                .action_sheet()
                .destructive("kill", button)
                .cancel("cancel", t!("Cancel"))
                .show();
            let ctx = ctx.clone();
            let pane = cell.pane.clone();
            drop(spawn(async move {
                if answer
                    .await
                    .is_ok_and(|r| r.action.as_deref() == Some("kill"))
                {
                    ctx.ask(Intent::KillPane(pane));
                }
            }));
        }
        _ => {}
    }
}

/// A card as the picker's window row: what its questions name.
fn window_row(card: &Card) -> Row {
    Row {
        id: card.window.clone(),
        index: u32::try_from(card.index).ok(),
        name: card.name.clone(),
        panes: u32::try_from(card.cells.len()).ok(),
        command: card
            .cells
            .iter()
            .find(|c| c.current)
            .or(card.cells.first())
            .map(|c| c.name.clone())
            .unwrap_or_default(),
        ..Row::default()
    }
}

/// Asks for window `card`'s new name, then renames it: nothing for a name
/// cleared or left as it was.
fn rename(ctx: &Ctx, prompts: &Prompts, mux: HomeMux, card: &Card) {
    let vocab = vocab(mux);
    let before = card.name.clone();
    let id = card.window.clone();
    let tell = ctx.tell.clone();
    let ask = ctx.tell.clone();
    prompts.ask(
        &move |event| ask(Intent::Prompt(event)),
        picker::rename_prompt(Kind::Windows, &window_row(card), &vocab),
        move |text| {
            let Some(text) = text else { return };
            let name = text.trim();
            if name.is_empty() || name == before {
                return;
            }
            tell(Intent::RenameWindow {
                id,
                name: name.to_owned(),
            });
        },
    );
}

/// Pane `pane` picked: the host switches, the overview goes.
fn pick(ctx: &Ctx, pane: &str) {
    ctx.ask(Intent::SelectPane(pane.to_owned()));
    ctx.overview.set(false);
}

/// Pane `pane` picked from its cell at `rect` (the surface's points): the
/// host hears where first, so the pane's terminal grows from there, then
/// switches at once (B658).
fn pick_from(ctx: &Ctx, pane: &str, rect: Option<[f32; 4]>) {
    if let Some(rect) = rect {
        ctx.ask(Intent::OverviewAnchor {
            pane: pane.to_owned(),
            rect,
            picked: true,
        });
    }
    pick(ctx, pane);
}

/// Tells the host where the shown pane's cell `target` is (`pane`), once
/// per place (`told`): its live terminal shrinks into it, and grows back
/// out of it on closing.
fn tell_anchor(
    ctx: &Ctx,
    target: ElementRef,
    pane: String,
    told: Stored<Option<(String, [f32; 4])>>,
) {
    let ctx = ctx.clone();
    drop(spawn(async move {
        let Some(m) = target.measure().await else {
            return;
        };
        let rect = [
            m.page.x.0,
            m.page.y.0,
            m.frame.size.width.0,
            m.frame.size.height.0,
        ];
        if rect[2] <= 0.0 || rect[3] <= 0.0 || !ctx.overview.get_untracked() {
            return;
        }
        let place = Some((pane.clone(), rect));
        if told.with(|t| *t == place) {
            return;
        }
        told.replace(place);
        ctx.ask(Intent::OverviewAnchor {
            pane,
            rect,
            picked: false,
        });
    }));
}

// --- On the page ----------------------------------------------------------------

/// The overview's model, from the tree the host posts.
pub(crate) fn model(ctx: &Ctx) -> Memo<Overview> {
    let switcher = ctx.switcher;
    computed(move || {
        switcher.with(|s| {
            s.tree
                .as_ref()
                .map(|tree| Overview::of(tree, &Words::here()))
                .unwrap_or_default()
        })
    })
}

/// Tells the host when the overview opens or closes (its keyboard goes
/// down while it is up, and comes back).
pub(crate) fn tell_open(ctx: &Ctx) {
    let open = ctx.overview;
    // The session's tree gone (a reconnect, the multiplexer gone): the
    // overview goes too, never left on "0 windows · 0 panes" and a New
    // window that would land nowhere (B655).
    let switcher = ctx.switcher;
    effect(move || {
        if switcher.with(|s| s.tree.is_none()) && open.get_untracked() {
            open.set(false);
        }
    });
    let told = stored(false);
    let ctx = ctx.clone();
    effect(move || {
        let now = open.get();
        if told.replace(now) != now {
            untracked(|| {
                if !now {
                    ctx.pictures.set(HashMap::new());
                }
                ctx.ask(Intent::Overview(now));
            });
        }
    });
}

/// Asks the host for the pictures of the session's panes when the overview
/// opens, and for the panes that come while it is open: once each, never
/// on a tick (B658).
pub(crate) fn ask_pictures(ctx: &Ctx, model: Memo<Overview>) {
    let open = ctx.overview;
    let asked = stored(HashSet::<String>::new());
    let ctx = ctx.clone();
    effect(move || {
        if !open.get() {
            asked.replace(HashSet::new());
            return;
        }
        let (panes, current) = model.with(|o| {
            let panes: Vec<String> = o
                .shown()
                .iter()
                .flat_map(|c| c.cells.iter().map(|c| c.pane.clone()))
                .collect();
            let current = o
                .cards
                .iter()
                .flat_map(|c| &c.cells)
                .find(|c| c.current)
                .map(|c| c.pane.clone());
            (panes, current)
        });
        let new: Vec<String> =
            asked.with(|a| panes.into_iter().filter(|p| !a.contains(p)).collect());
        if new.is_empty() {
            return;
        }
        asked.with(|a| a.extend(new.iter().cloned()));
        untracked(|| {
            ctx.ask(Intent::Pictures {
                panes: new,
                current,
            })
        });
    });
}

/// The overview, over the whole screen while it is open.
pub(crate) fn overview(ctx: Ctx, model: Memo<Overview>, prompts: Prompts) -> AnyElement {
    let open = ctx.overview;
    let visible = computed(move || open.get());
    // Escape closes it (a hardware keyboard; the terminal has given up the
    // keyboard while it is up).
    on_key("escape", move |_: &str| {
        if open.get_untracked() {
            open.set(false);
        }
    });
    view()
        .absolute()
        .inset_0()
        .pointer_events(PointerEvents::BoxNone)
        .child(switcher::appearing_ms(
            visible,
            |showing| fade_ms(reduce_motion(), showing),
            move || page(ctx.clone(), model, prompts.clone()),
        ))
        .into_element()
}

/// The shown pane's cell, for the host's shrink and growth: the element
/// and the place last told.
#[derive(Clone, Copy)]
struct Anchor {
    target: ElementRef,
    told: Stored<Option<(String, [f32; 4])>>,
}

/// The page: its bar, the summary, the shown window's pane cards.
fn page(ctx: Ctx, model: Memo<Overview>, prompts: Prompts) -> AnyElement {
    let anchor = Anchor {
        target: element_ref(),
        told: stored(None),
    };
    // Scrolled, the shown pane's cell moved: told again where it is.
    let rescroll = {
        let ctx = ctx.clone();
        move |_: &Point<Points>| {
            let current = model.with_untracked(|o| {
                o.cards
                    .iter()
                    .flat_map(|c| &c.cells)
                    .find(|c| c.current)
                    .map(|c| c.pane.clone())
            });
            if let Some(pane) = current {
                tell_anchor(&ctx, anchor.target, pane, anchor.told);
            }
        }
    };
    // In the flow at the window's size, not absolute: the fade's view
    // around it then has the page's size. Hung outside an empty one, the
    // page was drawn but UIKit gave it to no screen reader or UI test
    // (B659).
    view()
        .w(move || px(window().width.0))
        .h(move || px(window().height.0))
        .bg(move || token("background"))
        .test_id("terminal-overview")
        .child(bar(ctx.clone(), model))
        .child(
            scroll()
                .flex_1()
                .test_id("terminal-overview-scroll")
                .on_scroll_end(rescroll)
                .child(
                    view()
                        .px(px(number("space-page")))
                        .pt(px(4.0))
                        .gap(px(12.0))
                        .child(summary(model))
                        .child(cards(ctx.clone(), model, prompts, anchor))
                        .child(view().h(move || px(32.0 + safe_area().bottom.0))),
                ),
        )
        .into_element()
}

/// The page's bar: the window shown (its quick picker), Done.
fn bar(ctx: Ctx, model: Memo<Overview>) -> AnyElement {
    let window =
        computed(move || model.with(|o| o.shown().map(|c| c.place.clone()).unwrap_or_default()));
    // herdr's tab wears its own symbol, as in the breadcrumb (B656).
    let symbol = computed(move || model.with(|o| vocab(o.mux).window_icon));
    let close = {
        let open = ctx.overview;
        Rc::new(move || open.set(false))
    };
    view()
        .pt_safe_area()
        .child(
            view()
                .h(px(44.0))
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(number("space-page")))
                .child(capsule(ctx, window, symbol))
                .child(nav_bar::text_button(
                    t!("Done"),
                    true,
                    "terminal-overview-done",
                    close,
                )),
        )
        .into_element()
}

/// The bar's window capsule: the windows' quick picker (herdr: tabs), as
/// the breadcrumb's window segment opens it.
fn capsule(ctx: Ctx, window: Memo<String>, symbol: Memo<&'static str>) -> AnyElement {
    // The finger's target is the bar's 44 pt; the capsule
    // drawn in it is 36 (B654).
    view()
        .h(px(44.0))
        .min_w(px(44.0))
        .flex_shrink(1.0)
        .justify_center()
        .role(AccessibilityRole::Button)
        .label(move || window.get())
        .test_id("terminal-overview-window")
        .active(|s| s.opacity(0.5))
        .on_press(ctx.asking(Intent::Crumb(Crumb::Window)))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(36.0))
                .px(px(12.0))
                .rounded(px(number("radius-full")))
                .bg(move || token("chrome"))
                .border(px(0.5))
                .border_color(move || token("border-strong"))
                .child(dynamic(move || {
                    let name = symbol.get();
                    untracked(move || {
                        icon(name)
                            .size(IconSize::Sm)
                            .tone(Tone::Muted)
                            .into_element()
                    })
                }))
                .child(
                    chrome_text(TextToken::Subhead, move || window.get())
                        .number_of_lines(1u32)
                        .flex_shrink(1.0)
                        .text_color(move || token("foreground")),
                )
                .child(icon("chevron.down").size(IconSize::Sm).tone(Tone::Muted)),
        )
        .into_element()
}

/// "trainer · 3 panes", and how many wait on you.
fn summary(model: Memo<Overview>) -> AnyElement {
    dynamic(move || {
        let (words, waiting) = model.with(|o| match o.shown() {
            Some(card) => {
                let waiting = card
                    .cells
                    .iter()
                    .filter(|c| c.signal == Some(AgentSignal::Attention))
                    .count();
                (
                    format!(
                        "{} · {}",
                        card.place,
                        crate::plural!("%lld panes", card.cells.len() as i64)
                    ),
                    waiting,
                )
            }
            None => (String::new(), 0),
        });
        untracked(move || {
            // The badge goes under the count when the two don't fit
            // (B836): it follows the user's text size past the chrome's
            // extra large, and at the largest, "1 件があなた待ち" alone is
            // most of the screen's width.
            let mut row = view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .min_h(px(28.0))
                .test_id("terminal-overview-summary")
                .child(
                    chrome_text(TextToken::Footnote, words)
                        .text_color(move || token("muted-foreground")),
                );
            if waiting > 0 {
                row = row.child(
                    crate::ui::status::badge(crate::plural!("%lld waiting", waiting as i64))
                        .variant(crate::ui::status::BadgeVariant::Warning),
                );
            }
            row.into_element()
        })
    })
    .into_element()
}

/// The shown window's pane cards. Another window shown (picked from the
/// windows' picker while the page is up) cross-fades over `fast` (a plain
/// cut under reduced motion, the token's time then 0).
fn cards(ctx: Ctx, model: Memo<Overview>, prompts: Prompts, anchor: Anchor) -> AnyElement {
    let fade = animated_value(1.0_f32);
    let level = stored(None::<String>);
    view()
        .bind_opacity(&fade)
        .child(dynamic(move || {
            let overview = model.get();
            let card = overview.shown().cloned();
            let ctx = ctx.clone();
            let prompts = prompts.clone();
            untracked(move || {
                let now_at = card.as_ref().map(|c| c.window.clone());
                if level.replace(now_at.clone()) != now_at {
                    let ms = Duration::Fast.ms();
                    if ms > 0.0 {
                        fade.set(0.0);
                        drop(fade.timing(1.0, ms, ENTER));
                    }
                }
                match card {
                    Some(card) => pane_grid(ctx, overview, card, prompts, anchor),
                    None => view().into_element(),
                }
            })
        }))
        .into_element()
}

/// Items in rows of as many columns as fit, empty places filling the last.
fn rows_of(items: Vec<AnyElement>) -> AnyElement {
    let columns = columns(window().width.0 - 2.0 * number("space-page"));
    let mut rows = view().gap(px(ROW_GAP));
    let mut items = items.into_iter().peekable();
    while items.peek().is_some() {
        let mut row = view().flex_row().gap(px(GAP)).items_start();
        for _ in 0..columns {
            row = match items.next() {
                Some(item) => row.child(view().flex_1().min_w(px(0.0)).child(item)),
                None => row.child(view().flex_1()),
            };
        }
        rows = rows.child(row);
    }
    rows.into_element()
}

/// A card's words under its box: its title, its second line (the state's
/// dot first, when it has one), and ⋯, the system's menu of `entries`.
#[allow(clippy::too_many_arguments)]
fn card_label(
    title: String,
    line: String,
    said: String,
    status: Option<Status>,
    entries: Vec<MenuEntry>,
    menu_id: String,
    more_label: String,
    act: impl Fn(String) + 'static,
) -> AnyElement {
    // The state's words take a second line rather than lose their end
    // (B834): a card on a phone is some 127 pt wide, and "あなた待ち · 2個の
    // ペイン" or "Wartet auf dich · 2 Panels" is wider. The chrome's text
    // stops growing at extra large, where two lines hold every language's.
    // The dot keeps to the first line.
    let mut second = view().flex_row().items_start().gap(px(6.0)).min_w(px(0.0));
    if let Some(status) = status {
        let first = TextToken::Footnote.size(1.0);
        second = second.child(
            view()
                .flex_shrink_0()
                .h(move || px(crate::face::line(super::size(first))))
                .justify_center()
                .child(status_dot(status)),
        );
    }
    let line_tone = status.map_or("muted-foreground", tone);
    if !line.is_empty() {
        second = second.child(
            chrome_text(TextToken::Footnote, line)
                .number_of_lines(2u32)
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .label(said)
                .text_color(move || token(line_tone)),
        );
    }
    view()
        .flex_row()
        .items_start()
        .gap(px(4.0))
        .mt(px(8.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(
                    chrome_text(TextToken::Headline, title)
                        .number_of_lines(1u32)
                        .text_color(move || token("foreground")),
                )
                .child(second),
        )
        .child(
            view()
                .relative()
                .w(px(44.0))
                .h(px(44.0))
                .mt(px(-8.0))
                .mr(px(-10.0))
                .items_center()
                .justify_center()
                .child(icon("ellipsis").size(IconSize::Md).tone(Tone::Muted))
                .child(menu(
                    entries,
                    Some(&menu_id),
                    move || more_label.clone(),
                    act,
                )),
        )
        .into_element()
}

/// The window's pane page: a card per pane, in rows of as many columns as
/// fit, and last "＋ New pane".
fn pane_grid(
    ctx: Ctx,
    overview: Overview,
    card: Card,
    prompts: Prompts,
    anchor: Anchor,
) -> AnyElement {
    let now = switcher::unix_now();
    let words = Words::here();
    let mut items: Vec<AnyElement> = card
        .cells
        .iter()
        .map(|cell| pane_card(&ctx, &overview, &card, cell, &prompts, now, &words, anchor))
        .collect();
    items.push(new_pane_card(&ctx, &card.window));
    view()
        .test_id(&format!("overview-pane-page-{}", card.window))
        .child(rows_of(items))
        .into_element()
}

/// One pane's card: its picture in the box (the current one ringed), then
/// what runs in it and its state, and ⋯ (the pane's menu, as its long
/// press).
#[allow(clippy::too_many_arguments)]
fn pane_card(
    ctx: &Ctx,
    overview: &Overview,
    card: &Card,
    cell: &Cell,
    prompts: &Prompts,
    now: f64,
    words: &Words,
    anchor: Anchor,
) -> AnyElement {
    let (line, status, said) = match cell.signal {
        Some(signal) => match switcher::state_line(&stop_of(cell, card), now, words) {
            Some((line, status, _)) => (line, Some(status), spoken_state(signal, cell.since, now)),
            None => (String::new(), None, String::new()),
        },
        None => (String::new(), None, String::new()),
    };
    // The pane in its box (the box clips the ring).
    let boxed = view()
        .h(px(BOX))
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .overflow_hidden()
        .bg(move || token("border"))
        .child(cell_view(ctx, cell, status, now, anchor))
        .into_element();
    let pane = cell.pane.clone();
    let act = {
        let ctx = ctx.clone();
        let prompts = prompts.clone();
        let overview = overview.clone();
        move |action: String| act_on_pane(&ctx, &prompts, &overview, &action, &pane)
    };
    let label = card_label(
        cell.name.clone(),
        line,
        said,
        status,
        pane_entries(overview.mux),
        format!("overview-pane-menu-{}", cell.pane),
        t!("More for %@", arg1 = cell.name.clone()),
        act,
    );
    view()
        .test_id(&format!("overview-pane-card-{}", cell.pane))
        .child(boxed)
        .child(label)
        .into_element()
}

/// The pane page's last card: a new pane in its window, at once (a pane
/// has no name to ask for).
fn new_pane_card(ctx: &Ctx, window: &str) -> AnyElement {
    let words = t!("New Pane");
    view()
        .h(px(BOX))
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .border(px(1.0))
        .border_color(move || token("border-strong"))
        .role(AccessibilityRole::Button)
        .label(words.clone())
        .test_id("overview-new-pane")
        .active(|s| s.bg(move || token("accent")))
        .on_press(ctx.asking(Intent::NewPane(window.to_owned())))
        .child(icon("plus").size(IconSize::Lg).tone(Tone::Muted))
        .child(chrome_text(TextToken::Subhead, words).text_color(move || token("muted-foreground")))
        .into_element()
}

/// One pane's picture in its card's box, the current one ringed; its
/// state's dot in the corner. The button: a press switches to it.
fn cell_view(
    ctx: &Ctx,
    cell: &Cell,
    status: Option<Status>,
    now: f64,
    anchor: Anchor,
) -> AnyElement {
    let mut label_parts = vec![cell.name.clone()];
    if let Some(signal) = cell.signal {
        // In words, minutes said as minutes (B653: "13m" is the eye's).
        label_parts.push(spoken_state(signal, cell.since, now));
    }
    if cell.current {
        label_parts.push(t!("current"));
    }
    let pane = cell.pane.clone();
    // Its size, for where a press says it is (B658).
    let size = stored(None::<Size<Points>>);
    let current = cell.current.then(|| {
        let ctx = ctx.clone();
        let pane = cell.pane.clone();
        move || tell_anchor(&ctx, anchor.target, pane.clone(), anchor.told)
    });
    // The host's long-press menu finds the pane by its id.
    let mut v = view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(44.0))
        .relative()
        .p(px(8.0))
        .bg(move || token("card"))
        .on_layout(move |b: &Bounds<Points>| {
            size.replace(Some(b.size));
            if let Some(tell) = &current {
                tell();
            }
        })
        .test_id(&format!("overview-pane-{}", cell.pane))
        .role(AccessibilityRole::Button)
        .label(label_parts.join(", "))
        .accessibility_state(AccessibilityState {
            selected: cell.current,
            ..AccessibilityState::default()
        })
        .active(|s| s.bg(move || token("accent")))
        .on_press({
            let ctx = ctx.clone();
            move |e: &PressEvent| {
                // The cell's place: where the press is in the surface, less
                // where it is in the cell.
                let rect = size.get().map(|s| {
                    [
                        e.page_position.x.0 - e.position.x.0,
                        e.page_position.y.0 - e.position.y.0,
                        s.width.0,
                        s.height.0,
                    ]
                });
                pick_from(&ctx, &pane, rect)
            }
        })
        .child(picture(ctx, &cell.pane));
    if let Some(status) = status {
        v = v.child(
            view()
                .absolute()
                .top(px(8.0))
                .right(px(8.0))
                .child(status_dot(status)),
        );
    }
    if cell.current {
        v = v.element_ref(&anchor.target);
        // The accent's ring, inside the cell (the card's box clips).
        v = v.child(
            view()
                .absolute()
                .inset_0()
                .border(px(2.0))
                .border_color(move || token("primary"))
                .pointer_events(PointerEvents::None),
        );
    }
    v.into_element()
}

/// Pane `pane`'s picture, filling its cell:
/// the end of its screen as tiny text ([`picture_rows`]), as many rows as
/// fit; the terminal's symbol until (unless) the host has one.
/// Decorative.
fn picture(ctx: &Ctx, pane: &str) -> AnyElement {
    let pictures = ctx.pictures;
    let height = signal(0.0_f32);
    let rows = {
        let pane = pane.to_owned();
        computed(move || {
            let fits = (height.get() / PICTURE_LINE).floor().max(0.0) as usize;
            pictures.with(|p| {
                p.get(&pane)
                    .map(|rows| picture_rows(rows, fits))
                    .unwrap_or_default()
            })
        })
    };
    let id = format!("overview-picture-{pane}");
    view()
        .flex_1()
        .min_h(px(0.0))
        .relative()
        .overflow_hidden()
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true)
        .on_layout(move |b: &Bounds<Points>| {
            if height.get_untracked() != b.size.height.0 {
                height.set(b.size.height.0);
            }
        })
        .child(dynamic(move || {
            let rows = rows.get();
            if rows.is_empty() {
                return untracked(|| {
                    view()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .child(icon("terminal").size(IconSize::Lg).tone(Tone::Subtle))
                        .into_element()
                });
            }
            let lines = rows.len() as u32;
            text(rows.join("\n"))
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .w(px(PICTURE_WIDTH))
                .font_family("monospace")
                .text_size(px(PICTURE_TEXT))
                .line_height(LineHeight::Points(px(PICTURE_LINE)))
                .allow_font_scaling(false)
                .number_of_lines(lines)
                .text_overflow(EllipsizeMode::Clip)
                .text_color(move || token("muted-foreground"))
                .test_id(&id)
                .into_element()
        }))
        .into_element()
}

/// A host's message for the overview (`{"op": "overview-act", "action",
/// "pane"}`: its long-press menu's pick) as the page acts on it.
pub(crate) fn hear_act(
    ctx: &Ctx,
    prompts: &Prompts,
    model: Memo<Overview>,
    action: &str,
    pane: &str,
) {
    let overview = model.get_untracked();
    act_on_pane(ctx, prompts, &overview, action, pane);
}

/// What `{"op": "overview-act"}` carries.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActFields {
    pub action: String,
    pub pane: String,
}

/// What `{"op": "overview"}` carries.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenFields {
    pub open: bool,
}

/// What `{"op": "pictures"}` carries: each pane's screen rows.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PicturesFields {
    pub panes: HashMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| (*l).to_owned()).collect()
    }

    #[test]
    fn a_picture_is_the_screens_end_cut_to_what_fits() {
        // The blank tail cut; fewer rows than fit: all of them, from the
        // top, their trailing spaces cut.
        assert_eq!(
            picture_rows(&rows(&["$ ls", "a  b  ", "$ ", "", "  "]), 10),
            ["$ ls", "a  b", "$"]
        );
        // More than fit: the last ones, where the newest output and the
        // prompt are.
        assert_eq!(
            picture_rows(&rows(&["1", "2", "3", "4", ""]), 2),
            ["3", "4"]
        );
        // A blank screen, or no room: nothing.
        assert!(picture_rows(&rows(&["", "  "]), 5).is_empty());
        assert!(picture_rows(&rows(&["x"]), 0).is_empty());
        assert!(picture_rows(&[], 5).is_empty());
        // Never past PICTURE_ROWS.
        let tall: Vec<String> = (0..40).map(|i| i.to_string()).collect();
        let shown = picture_rows(&tall, 100);
        assert_eq!(shown.len(), PICTURE_ROWS);
        assert_eq!(shown[0], "16");
    }

    #[test]
    fn reduced_motion_still_cross_fades_the_overview() {
        assert_eq!(fade_ms(false, true), Duration::Base.nominal_ms());
        assert_eq!(fade_ms(false, false), Duration::Fast.nominal_ms());
        // Reduced: a plain cross-fade, not a cut (the host's shrink and
        // growth don't play).
        assert!(fade_ms(true, true) > 0.0);
        assert_eq!(fade_ms(true, true), fade_ms(true, false));
        // Inside the owner's 250 ms either way.
        for (reduced, showing) in [(false, true), (false, false), (true, true), (true, false)] {
            assert!(fade_ms(reduced, showing) <= 250.0);
        }
    }
    use crate::home::model::{Hook, PaneInfo, SessionInfo, WindowInfo};

    fn english() -> Words {
        Words {
            window: "Window".into(),
            tab: "Tab".into(),
            agent: "agent".into(),
            pane: Rc::new(|i| format!("pane {i}")),
            needs_you: "Needs you".into(),
            working: "Working".into(),
            finished: "Finished".into(),
            now: "now".into(),
        }
    }

    fn window(id: &str, index: i64, name: &str) -> WindowInfo {
        WindowInfo {
            id: id.into(),
            session: "$0".into(),
            index,
            name: name.into(),
            pane_count: 0,
        }
    }

    fn pane(id: &str, window: &str, index: i64, command: &str, path: &str) -> PaneInfo {
        PaneInfo {
            id: id.into(),
            window: window.into(),
            index,
            command: command.into(),
            path: Some(path.into()),
        }
    }

    /// The mockup's session: api (codex working), trainer (claude waiting,
    /// python, zsh), web (zsh, node), docs (claude done), crowded (six
    /// shells).
    fn work(active: &str) -> Control {
        let mut tree = Control {
            mux: HomeMux::Tmux,
            attached: true,
            sessions: vec![SessionInfo {
                id: "$0".into(),
                name: "work".into(),
            }],
            windows: vec![
                window("@2", 2, "2.1.322"),
                window("@1", 1, "codex"),
                window("@3", 3, "zsh"),
                window("@4", 4, "docs"),
                window("@5", 5, "logs"),
            ],
            panes: vec![
                pane("%1", "@1", 0, "codex", "/home/dev/src/api"),
                pane("%3", "@2", 1, "python", "/home/dev/src/trainer"),
                pane("%2", "@2", 0, "2.1.322", "/home/dev/src/trainer"),
                pane("%4", "@2", 2, "zsh", "/home/dev/src/trainer"),
                pane("%5", "@3", 0, "zsh", "/home/dev/src/web"),
                pane("%6", "@3", 1, "node", "/home/dev/src/web"),
                pane("%7", "@4", 0, "2.1.322", "/home/dev/src/docs"),
            ],
            active_session: Some("$0".into()),
            active_pane: Some(active.into()),
            hooks: vec![
                Hook {
                    pane: "%1".into(),
                    state: Some("working".into()),
                    agent: Some("codex".into()),
                    since: Some(1000.0),
                },
                Hook {
                    pane: "%2".into(),
                    state: Some("attention".into()),
                    agent: Some("claude".into()),
                    since: Some(1600.0),
                },
                Hook {
                    pane: "%7".into(),
                    state: Some("done".into()),
                    agent: None,
                    since: None,
                },
            ],
            ..Control::default()
        };
        for i in 0..6 {
            tree.panes
                .push(pane(&format!("%{}", 10 + i), "@5", i, "tail", "/var/log"));
        }
        tree
    }

    #[test]
    fn a_card_per_window_with_its_panes_in_order() {
        let overview = Overview::of(&work("%3"), &english());
        assert_eq!(overview.session, "work");
        let cards: Vec<(&str, Vec<&str>)> = overview
            .cards
            .iter()
            .map(|c| {
                (
                    c.place.as_str(),
                    c.cells.iter().map(|p| p.pane.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            cards,
            [
                ("api", vec!["%1"]),
                ("trainer", vec!["%2", "%3", "%4"]),
                ("web", vec!["%5", "%6"]),
                ("docs", vec!["%7"]),
                ("logs", vec!["%10", "%11", "%12", "%13", "%14", "%15"]),
            ]
        );
        let current: Vec<&str> = overview
            .cards
            .iter()
            .flat_map(|c| &c.cells)
            .filter(|p| p.current)
            .map(|p| p.pane.as_str())
            .collect();
        assert_eq!(current, ["%3"], "the pane shown, once");
        assert!(overview.cards[1].current());
        assert_eq!((overview.panes(), overview.waiting()), (13, 1));
        assert_eq!(
            overview.cards[1].cells[0].name, "claude",
            "the agent, never the version"
        );
    }

    #[test]
    fn a_card_never_says_the_sessions_name_again() {
        // B650: the session is named after the folder trainer's panes work
        // in; the overview's bar says it, the card says the window.
        let mut tree = work("%1");
        tree.sessions[0].name = "Trainer".into();
        let overview = Overview::of(&tree, &english());
        let places: Vec<&str> = overview.cards.iter().map(|c| c.place.as_str()).collect();
        assert_eq!(places, ["api", "Window 2", "web", "docs", "logs"]);
    }

    #[test]
    fn the_page_shows_the_window_of_the_pane_in_view() {
        let overview = Overview::of(&work("%3"), &english());
        assert_eq!(overview.shown().map(|c| c.window.as_str()), Some("@2"));
        let overview = Overview::of(&work("%14"), &english());
        assert_eq!(overview.shown().map(|c| c.window.as_str()), Some("@5"));
        // No pane in view (a reconnect's tree): the first window.
        let overview = Overview::of(&work("%99"), &english());
        assert_eq!(overview.shown().map(|c| c.window.as_str()), Some("@1"));
        assert!(Overview::default().shown().is_none());
    }

    #[test]
    fn as_many_columns_as_fit_two_to_four() {
        // An iPhone: 390 pt less the page's 16 each side.
        assert_eq!(columns(358.0), 2);
        assert_eq!(columns(300.0), 2, "never one");
        // An iPad mini in portrait (744) and landscape (1133).
        assert_eq!(columns(712.0), 4);
        assert_eq!(columns(536.0), 3);
        assert_eq!(columns(1101.0), 4, "four at most");
    }

    #[test]
    fn herdr_speaks_of_tabs() {
        let mut tree = work("%1");
        tree.mux = HomeMux::Herdr;
        let overview = Overview::of(&tree, &english());
        assert_eq!(overview.mux, HomeMux::Herdr);
    }
}
