//! The top bar of a page pushed in the app's navigation, as UIKit draws a
//! navigation bar with an inline title (a SwiftUI page's `navigationTitle`,
//! `.navigationBarTitleDisplayMode(.inline)` and a toolbar button): an NSH
//! page hides SwiftUI's bar (G5's second rule) and draws this one in its
//! place, so the page, not SwiftUI, owns its chrome, on every platform.
//!
//! - Below iOS 26: iOS 15–25's bar. The back button is the system's chevron
//!   and the previous page's title in the accent, the title is centered, a
//!   text button sits at the trailing edge. The bar is transparent over the
//!   page (its scroll edge appearance) until content scrolls under it, then
//!   has the system's chrome material and a hairline.
//! - iOS 26 and later: Liquid Glass. The back button is a glass circle with
//!   the chevron alone, the trailing button a glass capsule. The bar has no
//!   background; once content scrolls under it, iOS 26's scroll edge effect
//!   blurs and dims what is under the bar, so the title and the glass stand
//!   clear of the rows.
//! - Either look behind the items comes in as the page's list scrolls under
//!   the bar ([`ScrollEdge`]: what the list reports, what the bar reads);
//!   its values are the theme's ([`theme::bar_edge`]), none of them here.
//! - Its height is the bar it stands in for ([`BarMetrics`]): the host
//!   measures the navigation bar it hides (56 pt tall in a sheet, 50 pt on
//!   an iPad screen, 44 pt on an iPhone's). Its margins are UIKit's for the
//!   device (20 pt on iPad, 16 on iPhone): the bar's own layout margins
//!   (8 pt) are not where UIKit places its items.
//! - Its text follows Dynamic Type as a system bar's does: the body style's
//!   size from Large to extra extra large (17 to 21 pt), never smaller or
//!   larger (UIKit shows the Large Content Viewer instead). The title and
//!   the back button's text keep the baseline they have at Large as they
//!   grow, the chevron rises with them, and a trailing button's text stays
//!   centered ([`Placement`]): UIKit's bar, measured.
//!
//! The geometry was measured against the SwiftUI pages on the iPad mini
//! (iOS 18.2). The chevron and a trailing button's symbol are SF Symbols,
//! drawn by the host's native component `moshpit/symbol`.

use std::rc::Rc;

use nsh_core::{
    PropertyValue, StyleKey, StyleValue,
    platform::{Idiom, Os},
    prelude::*,
};
use serde::Deserialize;

use crate::{
    face::{self, TextStyle},
    theme,
    ui::{
        button::{IconButtonSize, IconVariant, Variant, button, icon_button},
        feedback::{SpinnerSize, spinner},
        icon::{IconSize, Tone, icon},
        menu::{self, MenuEntry},
        motion::Duration,
        tokens::token,
        typography::TextToken,
    },
};

/// What the host measured of the navigation bar the page hides
/// (`UINavigationBar`'s height, in points, without the status bar's safe
/// area).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarMetrics {
    pub height: f32,
}

impl BarMetrics {
    /// UIKit's bar outside a sheet: 50 pt on iPad, 44 pt on iPhone.
    pub fn standard(idiom: Idiom) -> BarMetrics {
        BarMetrics {
            height: if idiom == Idiom::Pad { 50.0 } else { 44.0 },
        }
    }

    /// The host's measurement, else [`Self::standard`] for this device.
    pub fn or_standard(metrics: Option<BarMetrics>) -> BarMetrics {
        metrics.unwrap_or_else(|| BarMetrics::standard(idiom()))
    }
}

/// Where UIKit places a bar's items: 20 pt from the edges on iPad, 16 pt on
/// iPhone and in what is as narrow as one on the iPad (a popover's 400 pt,
/// measured: the terminal's pickers' ＋).
pub(crate) fn margin() -> f32 {
    crate::ui::tokens::number(if idiom() == Idiom::Pad && window().width.0 > 420.0 {
        "space-page-regular"
    } else {
        "space-page"
    })
}

/// Whether the bar is drawn with Liquid Glass (iOS 26 and later).
///
/// The launch argument `-MOSHPIT_PREVIEW_GLASS` (or `MOSHPIT_PREVIEW_GLASS`
/// in the environment, as a test run sets it) draws it so below iOS 26 as
/// well: the glass itself is missing there (a blur stands in), the rest of
/// iOS 26's bar (its scroll edge effect, its layout) shows. How a test
/// device on iOS 18 sees that bar.
pub fn glass() -> bool {
    os() == Os::Ios && (os_version().0 >= 26 || preview_glass())
}

/// Whether the app was launched to preview iOS 26's bar ([`glass`]).
fn preview_glass() -> bool {
    static PREVIEW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PREVIEW.get_or_init(|| {
        std::env::args().any(|arg| arg == "-MOSHPIT_PREVIEW_GLASS")
            || std::env::var_os("MOSHPIT_PREVIEW_GLASS").is_some()
    })
}

/// The size of the bar's title and buttons at the user's text size: the
/// body style's (the title's headline is the same size), from Large to
/// extra extra large.
pub fn text_size() -> f32 {
    face::point_size_clamped(TextStyle::Body, font_scale(), face::LARGE, face::XX_LARGE)
}

/// The space the bar takes at the top of its page, the status bar's safe
/// area included: where the page's content starts.
pub fn bar_space(bar: BarMetrics) -> Points {
    px(safe_area().top.0 + bar.height)
}

/// The bar's text size at Large, where it starts to grow.
const LARGE_TEXT: f32 = 17.0;

/// Where the bar's text sits at text size `size` in a bar `height` tall
/// (UIKit's bar, measured at Large and at extra extra large and larger).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The top of the title's and the back button's text: their baseline
    /// stays where it is at Large (17 pt, the line centered in the bar).
    pub anchored_top: f32,
    /// The top of a trailing button's text: its line centered in the bar.
    pub centered_top: f32,
    /// How far the back chevron rises from the bar's middle.
    pub chevron_rise: f32,
}

impl Placement {
    pub fn new(height: f32, size: f32) -> Placement {
        // SF Pro's ascender (1950 of 2048 units a point) and its line
        // height (UIKit's `lineHeight`, `face::LINE_HEIGHT`).
        const ASCENT: f32 = 1950.0 / 2048.0;
        let line = |size: f32| size * face::LINE_HEIGHT;
        let baseline = (height - line(LARGE_TEXT)) / 2.0 + ASCENT * LARGE_TEXT;
        Placement {
            anchored_top: baseline - ASCENT * size,
            centered_top: (height - line(size)) / 2.0,
            // Measured: 2 pt higher at 21 pt than at 17.
            chevron_rise: (size - LARGE_TEXT).max(0.0) * 0.5,
        }
    }
}

/// How far left of the bar's margin the back button's chevron sits:
/// UIKit's.
const BACK_INSET: f32 = 7.0;

/// How far a bar's text button pads its text (R2's `Button`, `bar`), and
/// how far a bar's icon button's symbol sits in from its 44 pt box
/// without glass: what the bar's edges take off, so the text and the
/// symbol sit on the margin as UIKit's do.
const TEXT_PAD: f32 = 8.0;
const SYMBOL_SLACK: f32 = (44.0 - 22.0) / 2.0;

/// What a trailing button shows.
pub enum BarContent {
    /// Its label, in text.
    Text,
    /// An SF Symbol (SwiftUI's `Image(systemName:)` in a toolbar item), its
    /// label for screen readers.
    Symbol(&'static str),
    /// An SF Symbol in the app's accent circle, its label for screen
    /// readers: the add buttons of SSH Keys, Select Key and Shortcuts
    /// (`Image(systemName:)` at 14 pt semibold in the accent, in a 32 pt
    /// circle of the accent at 11% stroked with it at 24%).
    Circled(&'static str),
    /// Its label, in text, that can be busy: while `busy` holds, the
    /// spinner takes the label's place at the same width, the button takes
    /// no press and screen readers hear it busy (Send while the mail is
    /// prepared, Done while a step runs). Made by [`BarButton::busy`].
    Busy(Memo<bool>),
}

/// A button of the bar: at its trailing edge, or at its leading edge in the
/// back button's place.
pub struct BarButton {
    /// What it says (or, with a symbol, what screen readers say).
    pub label: String,
    /// Text or a symbol.
    pub content: BarContent,
    /// Whether it can be pressed now.
    pub enabled: Memo<bool>,
    /// Its text in the semibold of a confirming action (SwiftUI's
    /// `.fontWeight(.semibold)` on a bar button: Save, Send).
    pub emphasized: bool,
    /// The id UI tests find it by.
    pub test_id: &'static str,
    /// What a press does (not called for a button with a [`Self::menu`]).
    pub on_press: Box<dyn Fn()>,
    /// The system's pop-up menu a press opens instead (SwiftUI's `Menu` as a
    /// toolbar item): the button is then UIKit's, over what the bar draws,
    /// and carries [`Self::test_id`] as its accessibility identifier.
    pub menu: Option<BarMenu>,
}

impl BarButton {
    /// A text button that is busy while `busy` holds ([`BarContent::Busy`]):
    /// the spinner in its label's place, no press. Off unless asked; a
    /// symbol's button is left as it is.
    pub fn busy(mut self, busy: Memo<bool>) -> Self {
        if matches!(self.content, BarContent::Text) {
            self.content = BarContent::Busy(busy);
        }
        self
    }
}

/// What a bar button's menu offers, and what hears the pick.
pub struct BarMenu {
    pub entries: Vec<MenuEntry>,
    pub on_select: Box<dyn Fn(String)>,
}

/// Where a page's list stands against its bar: what the bar reads to set
/// itself off from content scrolled under it. The list reports to it
/// ([`ScrollEdge::follow`]) whether content is under the bar, which shows
/// the bar's look at all (none at the list's top: UIKit's bar has none at
/// its scroll edge), and its offset, natively, which brings the look in
/// over the first points of scrolling as the list moves, with no engine
/// work per frame. A `Copy` handle the page and its bar share, made in the
/// page's body.
#[derive(Clone, Copy)]
pub struct ScrollEdge {
    under: Signal<bool>,
    offset: AnimatedValue,
}

impl Default for ScrollEdge {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollEdge {
    /// At the list's top.
    pub fn new() -> ScrollEdge {
        ScrollEdge {
            under: signal(false),
            offset: animated_value(0.0),
        }
    }

    /// Whether content is under the bar.
    pub fn under(self) -> bool {
        self.under.get()
    }

    /// `scroll`, the page's list, reporting where it stands.
    pub fn follow(self, scroll: Scroll) -> Scroll {
        let under = self.under;
        scroll
            .scroll_value(&self.offset)
            .on_scroll(move |event: &ScrollEvent| {
                let now = event.offset.y.0 > 0.0;
                if under.get_untracked() != now {
                    under.set(now);
                }
            })
    }

    /// How much of the bar's look shows: none at the list's top, whole once
    /// it has scrolled the theme's `bar-edge-appear` points.
    fn coming_in(self) -> AnimatedNode {
        let appear = theme::BarEdge::current().appear.max(0.1);
        self.offset.interpolate(&[0.0, appear], &[0.0, 1.0])
    }
}

/// A bar's settings.
pub struct NavBar {
    /// Its height.
    pub metrics: BarMetrics,
    /// The previous page's title, beside the back chevron.
    pub back_title: String,
    /// The page's title ([`fixed`] for one that never changes).
    pub title: Box<dyn Fn() -> String>,
    /// A text button in the back button's place, for the first page of a
    /// sheet's stack (SwiftUI's `.cancellationAction`: Done, Cancel); the
    /// back button is drawn only without it.
    pub leading: Option<BarButton>,
    /// The trailing buttons, in the order they show (the last at the
    /// edge): SwiftUI's `.topBarTrailing` items, then its
    /// `.confirmationAction`.
    pub trailing: Vec<BarButton>,
    /// What the back button does.
    pub on_back: Box<dyn Fn()>,
    /// Where the page's list stands against the bar: the bar sets itself
    /// off from content scrolled under it.
    pub scrolled: ScrollEdge,
}

/// A title that never changes.
pub fn fixed(title: String) -> Box<dyn Fn() -> String> {
    Box::new(move || title.clone())
}

/// A text of the bar: its size follows [`text_size`].
pub(crate) fn bar_text(content: impl IntoReactive<SharedString>) -> Text {
    text(content)
        .text_size(move || px(text_size()))
        .line_height(face::line_height())
        .allow_font_scaling(false)
        .number_of_lines(1u32)
}

/// Where the bar's text sits now, in a bar of `metrics`.
fn placement(metrics: BarMetrics) -> Placement {
    Placement::new(metrics.height, text_size())
}

/// The bar, laid over the top of its page (absolute, full width): it takes
/// the status bar's safe area itself.
pub fn nav_bar(bar: NavBar) -> impl IntoElement {
    nav_bar_with(bar, true)
}

/// The bar of the first page of a sheet's stack that has no button in the
/// back button's place (the multiplexer's pickers, a folder of the host):
/// nothing at its leading edge.
pub fn root_bar(bar: NavBar) -> impl IntoElement {
    nav_bar_with(bar, false)
}

fn nav_bar_with(bar: NavBar, back: bool) -> impl IntoElement {
    let NavBar {
        metrics,
        back_title,
        title,
        leading,
        trailing,
        on_back,
        scrolled,
    } = bar;
    let glass = glass();
    let title: Rc<dyn Fn() -> String> = Rc::from(title);
    // The bar's width and its sides', as laid out: the title is centered
    // between the wider of the sides and the other edge's mirror of it
    // (B748).
    let bar_w = signal(0.0_f32);
    let leading_w = signal(0.0_f32);
    let trailing_w = signal(0.0_f32);
    let leading = match leading {
        Some(button) => view()
            .h_full()
            .flex_row()
            .items_center()
            .pl(px(edge_inset(&button.content, glass)))
            .child(bar_button(button, glass))
            .into_element(),
        None if back => back_button(back_title, on_back, glass, bar_w, title.clone()),
        None => view().into_element(),
    };
    let leading = view()
        .h_full()
        .flex_shrink_0()
        .on_layout(move |b: &Bounds<Points>| leading_w.set(b.size.width.0))
        .child(leading);
    let trailing_inset = trailing
        .last()
        .map_or(0.0, |button| edge_inset(&button.content, glass));
    let trailing = view()
        .h_full()
        .flex_row()
        .items_center()
        .gap_2()
        .pr(px(trailing_inset))
        .on_layout(move |b: &Bounds<Points>| trailing_w.set(b.size.width.0))
        .children(
            trailing
                .into_iter()
                .map(move |button| bar_button(button, glass)),
        );
    // UIKit's: centered on the bar while it fits between the wider side
    // and that side's mirror; else centered between the sides as they are;
    // cut at its tail only when even that is too little.
    let insets = {
        let title = title.clone();
        computed(move || {
            let (lead, trail) = (leading_w.get(), trailing_w.get());
            let side = lead.max(trail) + TITLE_GAP;
            let words = bar_width(&title(), FontWeight::SEMIBOLD);
            if words <= bar_w.get() - 2.0 * side + 0.5 {
                (side, side)
            } else {
                (lead + TITLE_GAP, trail + TITLE_GAP)
            }
        })
    };
    let row = view()
        .relative()
        .h(px(metrics.height))
        .flex_row()
        .items_center()
        .on_layout(move |b: &Bounds<Points>| bar_w.set(b.size.width.0))
        .child(leading)
        .child(view().flex_1())
        .child(trailing)
        // The title, centered on the bar, on the baseline it has at Large,
        // in what the wider side leaves it on both: cut at its tail when
        // even that is too little, never under a button (B748).
        .child(
            view()
                .absolute()
                .top(px(0.0))
                .bottom(px(0.0))
                .left(move || px(insets.get().0))
                .right(move || px(insets.get().1))
                .items_center()
                .pt(move || px(placement(metrics).anchored_top))
                .pointer_events(PointerEvents::None)
                .child(
                    bar_text({
                        let title = title.clone();
                        move || SharedString::from(title())
                    })
                    .max_w(relative(1.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || token("foreground"))
                    .role(AccessibilityRole::Header)
                    .test_id("nav-bar-title"),
                ),
        );
    let behind = edge_look(
        metrics,
        glass,
        move || scrolled.under(),
        &scrolled.coming_in(),
    );
    view()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .test_id("nav-bar")
        .child(behind)
        .child(view().pt_safe_area().child(row))
}

/// A style that hides its view (CSS's `visibility: hidden`: laid out, not
/// drawn) unless `shown`.
fn shown_style(shown: bool) -> nsh_core::element::Style {
    let style = nsh_core::element::Style::default();
    if shown {
        style
    } else {
        style.with(StyleKey::Visibility, PropertyValue::Bool(true))
    }
}

/// What sets a bar of `metrics` off from content scrolled under it
/// ([`theme::bar_edge`]'s look), laid over the top of the page behind the
/// bar's items: on glass, iOS 26's scroll edge effect (what is under the
/// bar blurred into the page's color, masked whole under the bar and
/// fading out past it); below iOS 26, the system's chrome material with a
/// hairline. Drawn only while `shown()` (laid out, not drawn otherwise:
/// nothing at the list's top, no blur at work), as much of it as `opacity`
/// says. Takes no touches. Home's bar brings it in with its inline title.
pub(crate) fn edge_look(
    metrics: BarMetrics,
    glass: bool,
    shown: impl Fn() -> bool + 'static,
    opacity: &AnimatedNode,
) -> AnyElement {
    let base = view()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .pointer_events(PointerEvents::None)
        .bind_opacity(opacity);
    if glass {
        base.h(move || px(bar_space(metrics).0 + theme::BarEdge::current().reach))
            .inline_style(move || edge_style(metrics, shown()))
            .backdrop_blur(px(theme::BarEdge::current().blur))
            .bg(theme::bar_edge_tint)
            .test_id("nav-bar-edge")
            .into_element()
    } else {
        base.h(move || bar_space(metrics))
            .inline_style(move || shown_style(shown()))
            .backdrop_blur(px(theme::bar_edge::MATERIAL_BLUR))
            .backdrop_material(theme::bar_edge::MATERIAL)
            // The material's variant follows the trace under it, which
            // follows the appearance (UIKit's chrome, dark or light).
            .bg(theme::bar_edge::material_trace)
            .border_b(px(1.0 / pixel_ratio().max(1.0)))
            .border_color(theme::bar_hairline)
            .test_id("nav-bar-background")
            .into_element()
    }
}

/// The scroll edge effect's mask (whole down to `bar-edge-fade` above the
/// bar's bottom, gone at the end of its reach past it), hidden unless
/// `shown`.
fn edge_style(metrics: BarMetrics, shown: bool) -> nsh_core::element::Style {
    let whole = (bar_space(metrics).0 - theme::BarEdge::current().fade).max(0.0);
    let mask = format!("linear-gradient(to bottom, black {whole}px, transparent)");
    let style = shown_style(shown);
    match StyleKey::MaskImage.parse_value(StyleValue::Str(&mask)) {
        Ok(mask) => style.with(StyleKey::MaskImage, mask),
        Err(_) => style,
    }
}

/// How far in from the bar's edge a button at it starts, so what it draws
/// sits on the margin: a glass circle or capsule's edge, a text's first
/// letter, a symbol's side (UIKit's), the accent's circle.
pub(crate) fn edge_inset(content: &BarContent, glass: bool) -> f32 {
    if glass {
        return margin();
    }
    match content {
        BarContent::Text | BarContent::Busy(_) => margin() - TEXT_PAD,
        BarContent::Symbol(_) => margin() - SYMBOL_SLACK,
        BarContent::Circled(_) => margin(),
    }
}

/// A bar's text button that can be busy ([`BarContent::Busy`]), drawn as
/// a bar text button is (R2's `Bar`, `BarCommit` when `emphasized`): while
/// `busy` holds its label fades out and the spinner stands in its place,
/// the width unchanged (§4.2's loading); it takes no press, and screen
/// readers hear it busy. Its own press target: `nsh_base`'s button says
/// only selected.
fn busy_text(
    label: String,
    emphasized: bool,
    enabled: Memo<bool>,
    busy: Memo<bool>,
    on_press: std::rc::Rc<dyn Fn()>,
    test_id: &'static str,
) -> AnyElement {
    let inert = move || !enabled.get() || busy.get();
    let weight = if emphasized {
        FontWeight::SEMIBOLD
    } else {
        FontWeight::NORMAL
    };
    view()
        .role(AccessibilityRole::Button)
        .label(label.clone())
        .disabled(inert)
        .accessibility_state(move || AccessibilityState {
            disabled: inert(),
            busy: busy.get(),
            ..AccessibilityState::default()
        })
        .test_id(test_id)
        .on_press(move || {
            if !inert() {
                on_press();
            }
        })
        .relative()
        .flex_row()
        .items_center()
        .justify_center()
        .min_h(px(44.0))
        .px(px(TEXT_PAD))
        .opacity(move || if enabled.get() { 1.0_f32 } else { 0.4 })
        .transition_all()
        .duration(Duration::Fast.ms())
        .active(|s| s.opacity(0.2))
        .child(
            view()
                .opacity(move || if busy.get() { 0.0_f32 } else { 1.0 })
                .child(
                    TextToken::Body
                        .apply(text(label))
                        .font_weight(weight)
                        .text_color(move || token(Tone::Primary.token()))
                        .number_of_lines(1u32),
                ),
        )
        .child(when(
            move || busy.get(),
            || {
                view()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .child(spinner(SpinnerSize::Sm))
            },
        ))
        .into_element()
}

/// The room between the title and the wider of the bar's sides.
const TITLE_GAP: f32 = 8.0;

/// Between the back button's chevron and its words (R2's medium button).
const BACK_GAP: f32 = 8.0;

/// What the back button says (B748), as UIKit's: the page under it while
/// that fits beside the centered title, else "Back", else nothing but the
/// chevron (its name still the page under it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackLabel {
    Full,
    Back,
    Chevron,
}

/// The longest label whose button fits in `room` (the bar's half less
/// the title's and a gap), its chrome (padding, chevron, the inset before
/// it) `chrome` wide: the full title `full` wide, else "Back" `back` wide,
/// else the chevron. Before the bar is laid out (no chrome to weigh): the
/// full title, as it was.
fn back_label(room: f32, chrome: Option<f32>, full: f32, back: f32) -> BackLabel {
    let Some(chrome) = chrome else {
        return BackLabel::Full;
    };
    if chrome + full <= room + 0.5 {
        BackLabel::Full
    } else if chrome + back <= room + 0.5 {
        BackLabel::Back
    } else {
        BackLabel::Chevron
    }
}

/// The width `words` take on one line in the bar's text at `weight`.
fn bar_width(words: &str, weight: FontWeight) -> f32 {
    measure_text(&format!("{} {}px system-ui", weight.0, text_size()), words).0
}

/// The back button (§4.18): on glass the chevron alone in a glass circle
/// (an icon button); below iOS 26 the chevron and the previous page's
/// title in the accent (a bar text button), the chevron where UIKit's
/// sits, the title giving way to "Back" and then to the chevron alone
/// where the bar's centered title leaves it too little room (B748).
fn back_button(
    title: String,
    on_back: Box<dyn Fn()>,
    glass: bool,
    bar_w: Signal<f32>,
    page_title: Rc<dyn Fn() -> String>,
) -> AnyElement {
    let on_back: std::rc::Rc<dyn Fn()> = std::rc::Rc::from(on_back);
    let place = view().h_full().flex_row().items_center();
    let named = if title.is_empty() {
        t!("Back")
    } else {
        title.clone()
    };
    if glass {
        return place
            .pl(px(margin()))
            .child(
                icon_button("chevron.backward", named)
                    .variant(IconVariant::Bar)
                    .on_press(move || on_back())
                    .test_id("nav-bar-back"),
            )
            .into_element();
    }
    let inset = (margin() - BACK_INSET - TEXT_PAD).max(0.0);
    // What the button takes besides its words: the inset before it, its
    // padding, the chevron at the user's size and the gap after it (R2's
    // bar button at its medium size). Worked out, not read off a layout,
    // so a label never decides from the frame of the one before it.
    let chrome = move || inset + 2.0 * TEXT_PAD + IconSize::Md.points_now() + BACK_GAP;
    let label = {
        let full = title.clone();
        computed(move || {
            let words = bar_width(&page_title(), FontWeight::SEMIBOLD);
            let room = bar_w.get() / 2.0 - words / 2.0 - TITLE_GAP;
            back_label(
                room,
                (bar_w.get() > 0.0).then(chrome),
                bar_width(&full, FontWeight::NORMAL),
                bar_width(&t!("Back"), FontWeight::NORMAL),
            )
        })
    };
    dynamic(move || {
        let label = label.get();
        let (title, named, on_back) = (title.clone(), named.clone(), on_back.clone());
        untracked(move || {
            let words = match label {
                BackLabel::Full => title,
                BackLabel::Back => t!("Back"),
                BackLabel::Chevron => String::new(),
            };
            let place = view().h_full().flex_row().items_center();
            if words.is_empty() {
                return place
                    .pl(px(inset))
                    .child(
                        icon_button("chevron.backward", named)
                            .variant(IconVariant::Bar)
                            .on_press(move || on_back())
                            .test_id("nav-bar-back"),
                    )
                    .into_element();
            }
            place
                .pl(px(inset))
                .child(
                    button(words)
                        .variant(Variant::Bar)
                        .icon("chevron.backward")
                        .on_press(move || on_back())
                        .test_id("nav-bar-back"),
                )
                .into_element()
        })
    })
}

/// A text button of the bar, for a bar drawn elsewhere (Settings' large
/// title, [`crate::home::bar::large_bar`]): Done, in the commit's semibold
/// when `emphasized`.
pub fn text_button(
    label: String,
    emphasized: bool,
    test_id: &'static str,
    on_press: std::rc::Rc<dyn Fn()>,
) -> AnyElement {
    bar_button(
        BarButton {
            label,
            content: BarContent::Text,
            enabled: computed(|| true),
            emphasized,
            test_id,
            on_press: Box::new(move || on_press()),
            menu: None,
        },
        glass(),
    )
}

/// A button of the bar (§4.18), R2's: a text in a bar button (in a glass
/// capsule on glass; Save, Send semibold), a symbol in a bar icon button,
/// the accent's circle in a tinted small one. A button with a menu is
/// UIKit's, laid over a drawing of it that takes no touch (a second
/// pressable under the menu would be a covered one).
fn bar_button(button_: BarButton, glass: bool) -> AnyElement {
    let BarButton {
        label,
        content,
        enabled,
        emphasized,
        test_id,
        on_press,
        menu,
    } = button_;
    if let Some(BarMenu { entries, on_select }) = menu {
        let drawing = match content {
            BarContent::Text | BarContent::Busy(_) => {
                let weight = if emphasized {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                };
                bar_text(label.clone())
                    .font_weight(weight)
                    // As R2's bar button draws its words (and the symbols
                    // here their tones): the accent's text, subtle when off.
                    .text_color(move || {
                        token(if enabled.get() {
                            Tone::Primary.token()
                        } else {
                            Tone::Subtle.token()
                        })
                    })
                    .into_element()
            }
            BarContent::Symbol(name) | BarContent::Circled(name) => {
                let tone = move || match (glass, enabled.get()) {
                    (_, false) => Tone::Subtle,
                    (true, true) => Tone::Foreground,
                    (false, true) => Tone::Primary,
                };
                let symbol = dynamic(move || {
                    let tone = tone();
                    untracked(move || icon(name).size(IconSize::Lg).tone(tone))
                });
                let circle = view()
                    .w(px(44.0))
                    .h(px(44.0))
                    .items_center()
                    .justify_center();
                if glass {
                    circle
                        .rounded_full()
                        .glass_effect(nsh_core::GlassEffect::Regular)
                        .child(symbol)
                        .into_element()
                } else {
                    circle.child(symbol).into_element()
                }
            }
        };
        let said = label.clone();
        return view()
            .relative()
            .h_full()
            .flex_row()
            .items_center()
            .child(
                view()
                    .h_full()
                    .flex_row()
                    .items_center()
                    .accessibility_elements_hidden(true)
                    .child(drawing),
            )
            .child(menu::menu(
                entries,
                Some(test_id),
                move || said.clone(),
                on_select,
            ))
            .into_element();
    }
    let on_press: std::rc::Rc<dyn Fn()> = std::rc::Rc::from(on_press);
    let inert = move || !enabled.get();
    match content {
        BarContent::Text => {
            let control = button(label)
                .variant(if emphasized {
                    Variant::BarCommit
                } else {
                    Variant::Bar
                })
                .disabled(inert)
                .on_press(move || on_press())
                .test_id(test_id);
            if glass {
                view()
                    .h(px(44.0))
                    .px(px(6.0))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .glass_effect(nsh_core::GlassEffect::Regular)
                    .child(control)
                    .into_element()
            } else {
                control.into_element()
            }
        }
        BarContent::Busy(busy) => {
            let control = busy_text(label, emphasized, enabled, busy, on_press, test_id);
            if glass {
                view()
                    .h(px(44.0))
                    .px(px(6.0))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .glass_effect(nsh_core::GlassEffect::Regular)
                    .child(control)
                    .into_element()
            } else {
                control
            }
        }
        BarContent::Symbol(name) => icon_button(name, label)
            .variant(IconVariant::Bar)
            .disabled(inert)
            .on_press(move || on_press())
            .test_id(test_id)
            .into_element(),
        BarContent::Circled(name) => icon_button(name, label)
            .variant(IconVariant::Tinted)
            .size(IconButtonSize::Sm)
            .disabled(inert)
            .on_press(move || on_press())
            .test_id(test_id)
            .into_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UIKit's order: the page under it while it fits beside the centered
    /// title, then "Back", then the chevron; before the first layout, the
    /// page under it.
    #[test]
    fn the_back_title_gives_way_in_uikits_order() {
        assert_eq!(back_label(100.0, None, 500.0, 40.0), BackLabel::Full);
        assert_eq!(back_label(100.0, Some(40.0), 60.0, 40.0), BackLabel::Full);
        assert_eq!(back_label(100.0, Some(40.0), 61.0, 40.0), BackLabel::Back);
        assert_eq!(back_label(70.0, Some(40.0), 61.0, 40.0), BackLabel::Chevron);
        assert_eq!(back_label(-10.0, Some(40.0), 1.0, 1.0), BackLabel::Chevron);
    }
}
