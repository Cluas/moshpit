//! Home's navigation bar: UIKit's bar with a large title
//! (`.navigationTitle("Moshpit")`, `.navigationBarTitleDisplayMode(.large)`)
//! and the toolbar's buttons, Settings and Add Connection.
//!
//! Home hides SwiftUI's bar like every NSH page (G5's second rule) and draws
//! this one in its place. A large title is UIKit's bar at its tallest: the
//! inline row (the toolbar's buttons, the inline title once it shows) over
//! a row of its own holding the title in the large title style. As the list
//! scrolls up the large title goes with it, under the inline row, shrinking
//! a little and fading, and the inline title fades in and rises as it
//! leaves; the bar's look comes in once it has gone; pulled down, the large
//! title follows the list and grows a little; let go half way, the list
//! settles at the nearer end (UI-DESIGN §4.18, [`LargeTitleMotion`]). The
//! offset and the pull are the scroll view's, natively
//! ([`Scrollable::scroll_value`], [`Scrollable::pull_value`]): no engine
//! work per frame but the settling.
//!
//! Below iOS 26 the bar is transparent over the page until the large title
//! has scrolled away, then it wears the system's chrome material and a
//! hairline; on iOS 26 and later the buttons are glass and the bar has no
//! background of its own, and iOS 26's scroll edge effect comes in instead.
//! Either is the shared bar's look ([`nav_bar::edge_look`], the theme's
//! values), here coming in with the inline title.

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::{
    face::{self, TextStyle},
    nav_bar::{self, BarContent, BarMetrics},
    theme,
    ui::{
        button::{IconVariant, icon_button},
        motion::{distance, scale},
        tokens::{number, token},
    },
};

/// The large title's row at Large: UIKit's (96 pt of bar on an iPhone, the
/// inline row's 44 and this).
pub const LARGE_ROW: f32 = 52.0;

/// The large title's size at the user's text size: the large title style,
/// bold, growing with Dynamic Type as UIKit's bar lets it (to the largest
/// size short of the accessibility sizes).
pub fn large_title_size() -> f32 {
    face::point_size_clamped(
        TextStyle::LargeTitle,
        font_scale(),
        face::LARGE,
        face::AX1 - 1,
    )
}

/// The large title's row at the user's text size: UIKit grows it by a
/// third of what the title grows (54 pt for its 40 pt at extra extra extra
/// large; measured), the title rising into the inline row above.
pub fn large_row() -> f32 {
    LARGE_ROW + (large_title_size() - 34.0) / 3.0
}

/// The space the bar takes at the top of the page with its large title
/// showing: where the list starts.
pub fn space(bar: BarMetrics) -> Points {
    px(nav_bar::bar_space(bar).0 + large_row())
}

/// Where the large title starts: the page's margin, where the cards' edge is
/// (UI-DESIGN §6.1).
fn margin() -> f32 {
    number("space-page")
}

/// The column the large title is in: the list's, at most
/// `content-max-width` wide and centered, so the title starts where the
/// cards do on a wide window too.
fn column() -> f32 {
    number("content-max-width")
}

/// A clamped linear ramp: from `.1[0]` at `.0[0]` to `.1[1]` at `.0[1]`,
/// held past either end, as [`AnimatedValue::interpolate`] maps.
pub type Ramp = ([f32; 2], [f32; 2]);

/// The value of `ramp` at `x`: what the bound interpolation shows there.
pub fn at(ramp: Ramp, x: f32) -> f32 {
    let ([x0, x1], [y0, y1]) = ramp;
    if x <= x0 || x1 <= x0 {
        return y0;
    }
    if x >= x1 {
        return y1;
    }
    y0 + (y1 - y0) * (x - x0) / (x1 - x0)
}

/// How far the list is pulled past its top for the large title's whole
/// stretch, and the stretch (UI-DESIGN §4.18 (a)).
pub const STRETCH_PULL: f32 = 64.0;
pub const STRETCHED: f32 = 1.08;

/// Where, as a share of the collapse, the large title starts to fade and the
/// inline title starts to come in (§4.18 (b)).
pub const FADE_FROM: f32 = 0.4;
pub const INLINE_FROM: f32 = 0.6;

/// How far above its row's bottom the large title's baseline sits, at every
/// text size (UIKit's, measured; `LARGE_TITLE_BOTTOM` keeps it there).
/// Once the list has moved the baseline under the bar only the descenders
/// are left below it, a sliver: the large title has faded out by then.
pub const BASELINE_UP: f32 = 17.0;

/// The large title's motion (UI-DESIGN §4.18 (a), (b), (c)), as ramps of
/// the list's offset and pull: what the bar binds natively, and what the
/// tests read. `collapse` is the large title's row ([`large_row`]),
/// `appear` the bar's `bar-edge-appear`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LargeTitleMotion {
    pub collapse: f32,
    pub appear: f32,
}

impl LargeTitleMotion {
    /// At the user's text size, in the surface's theme.
    pub fn now() -> LargeTitleMotion {
        LargeTitleMotion {
            collapse: large_row(),
            appear: theme::BarEdge::current().appear,
        }
    }

    /// (a) Pulled past the top: the large title grows from its leading
    /// baseline, 1 to [`STRETCHED`] over [`STRETCH_PULL`].
    pub fn stretch(self) -> Ramp {
        ([0.0, STRETCH_PULL], [1.0, STRETCHED])
    }

    /// (b) Scrolled up: the large title shrinks toward the bar, 1 to
    /// `scale::ENTER` over the collapse...
    pub fn shrink(self) -> Ramp {
        ([0.0, self.collapse], [1.0, scale::ENTER])
    }

    /// ...and fades from [`FADE_FROM`] of it, gone by the time its
    /// baseline reaches the bar ([`BASELINE_UP`] short of the collapse's
    /// end): what is left of it under the bar then is the tips of its
    /// descenders, which would read as a stray dash.
    pub fn fade(self) -> Ramp {
        let from = FADE_FROM * self.collapse;
        (
            [from, (self.collapse - BASELINE_UP).max(from + 1.0)],
            [1.0, 0.0],
        )
    }

    /// The inline title comes in from [`INLINE_FROM`] of the collapse...
    pub fn inline_in(self) -> Ramp {
        ([INLINE_FROM * self.collapse, self.collapse], [0.0, 1.0])
    }

    /// ...rising `distance::RISE` into place.
    pub fn inline_rise(self) -> Ramp {
        (
            [INLINE_FROM * self.collapse, self.collapse],
            [distance::RISE, 0.0],
        )
    }

    /// The bar's look comes in over the collapse's last `appear` points:
    /// once the large title has gone and rows start under the bar.
    pub fn edge(self) -> Ramp {
        (
            [(self.collapse - self.appear).max(0.0), self.collapse],
            [0.0, 1.0],
        )
    }

    /// (c) Where a list come to rest at `offset` settles: half collapsed or
    /// less back to the top, more to the collapse; nowhere outside it.
    pub fn settle(self, offset: f32) -> Option<f32> {
        if offset <= 0.0 || offset >= self.collapse {
            return None;
        }
        Some(if offset < self.collapse / 2.0 {
            0.0
        } else {
            self.collapse
        })
    }
}

/// `value` through `ramp`, natively.
fn bind(value: &AnimatedValue, (input, output): Ramp) -> AnimatedNode {
    value.interpolate(&input, &output)
}

/// A button of the bar's trailing group (or its leading one): an SF
/// Symbol, its label for screen readers, the id UI tests find it by.
#[derive(Clone)]
pub struct BarButton {
    pub symbol: &'static str,
    pub label: String,
    pub test_id: &'static str,
    pub on_press: Rc<dyn Fn()>,
}

/// The bar, laid over the top of the page (absolute, full width), and the
/// large title under its inline row; `offset` and `pull` follow the list's
/// scroll offset and its pull past the top.
pub fn bar(
    metrics: BarMetrics,
    title: String,
    leading: Option<BarButton>,
    trailing: Vec<BarButton>,
    offset: AnimatedValue,
    pull: AnimatedValue,
) -> AnyElement {
    let built = |b: BarButton| -> Build { Rc::new(move || button(b.clone())) };
    large_bar(
        metrics,
        title,
        leading.map(built),
        trailing.into_iter().map(built).collect(),
        offset,
        pull,
    )
}

/// What builds a button of a large-title bar (again whenever the bar is
/// rebuilt for a new text size).
pub type Build = Rc<dyn Fn() -> AnyElement>;

/// [`bar`] with buttons of any kind: Settings' root wears the same large
/// title with a text button, Done (UI-DESIGN §6.4).
pub fn large_bar(
    metrics: BarMetrics,
    title: String,
    leading: Option<Build>,
    trailing: Vec<Build>,
    offset: AnimatedValue,
    pull: AnimatedValue,
) -> AnyElement {
    // Rebuilt when the text size changes: the large title's row is the
    // collapse the motion runs over.
    dynamic(move || {
        let motion = LargeTitleMotion::now();
        let (title, leading, trailing) = (title.clone(), leading.clone(), trailing.clone());
        untracked(move || {
            let glass = nav_bar::glass();
            let inset = nav_bar::edge_inset(&BarContent::Symbol(""), glass);
            let inline_in = bind(&offset, motion.inline_in());
            let inline_rise = bind(&offset, motion.inline_rise());
            let edge = bind(&offset, motion.edge());
            let inline_row = view()
                .relative()
                .h(px(metrics.height))
                .flex_row()
                .items_center()
                .children(leading.map(|b| {
                    view()
                        .h_full()
                        .flex_row()
                        .items_center()
                        .pl(px(inset))
                        .child(b())
                }))
                .child(view().flex_1())
                .child(
                    view()
                        .h_full()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .pr(px(inset))
                        .children(trailing.into_iter().map(|b| b())),
                )
                .child(
                    view()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .pt(move || {
                            px(
                                nav_bar::Placement::new(metrics.height, nav_bar::text_size())
                                    .anchored_top,
                            )
                        })
                        .pointer_events(PointerEvents::None)
                        .child(
                            view().bind_translate_y(&inline_rise).child(
                                text(title.clone())
                                    .text_size(move || px(nav_bar::text_size()))
                                    .line_height(face::line_height())
                                    .allow_font_scaling(false)
                                    .number_of_lines(1u32)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || token("foreground"))
                                    .bind_opacity(&inline_in)
                                    .accessibility_elements_hidden(true)
                                    .test_id("nav-bar-inline-title"),
                            ),
                        ),
                );
            view()
                .absolute()
                .inset_0()
                .pointer_events(PointerEvents::BoxNone)
                .test_id("nav-bar")
                .child(nav_bar::edge_look(metrics, glass, || true, &edge))
                .child(large_title(metrics, title, &offset, &pull, motion))
                .child(view().pt_safe_area().child(inline_row))
        })
    })
}

/// A button of the bar: R2's bar icon button (a glass circle on glass, the
/// accent's symbol on the bar's material below).
fn button(button: BarButton) -> AnyElement {
    let BarButton {
        symbol,
        label,
        test_id,
        on_press,
    } = button;
    icon_button(symbol, label)
        .variant(IconVariant::Bar)
        .on_press(move || on_press())
        .test_id(test_id)
        .into_element()
}

/// The large title: under the inline row, moving with the list (up and
/// under the inline row as it scrolls, down as it is pulled), clipped where
/// the inline row ends; it shrinks and fades as it goes and grows as it is
/// pulled, both from its leading baseline ([`LargeTitleMotion`]).
fn large_title(
    metrics: BarMetrics,
    title: String,
    offset: &AnimatedValue,
    pull: &AnimatedValue,
    motion: LargeTitleMotion,
) -> AnyElement {
    let follow = offset.mul(-1.0);
    let stretch = bind(pull, motion.stretch());
    let shrink = bind(offset, motion.shrink());
    let fade = bind(offset, motion.fade());
    view()
        .absolute()
        .top(move || nav_bar::bar_space(metrics))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .overflow_hidden()
        .pointer_events(PointerEvents::None)
        .items_center()
        .child(
            view()
                .w_full()
                .max_w(px(column()))
                .h(move || px(large_row()))
                .px(move || px(margin()))
                .justify_end()
                .items_start()
                .bind_translate_y(&follow)
                .child(
                    view()
                        .origin_bottom_left()
                        .bind_scale(&stretch)
                        .test_id("nav-bar-large-stretch")
                        .child(
                            view()
                                .origin_bottom_left()
                                .bind_scale(&shrink)
                                .bind_opacity(&fade)
                                .test_id("nav-bar-large-shrink")
                                .child(
                                    text(title)
                                        .text_size(move || px(large_title_size()))
                                        .line_height(face::line_height())
                                        .allow_font_scaling(false)
                                        .number_of_lines(1u32)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || token("foreground"))
                                        .role(AccessibilityRole::Header)
                                        .mb(move || {
                                            px(LARGE_TITLE_BOTTOM
                                                - (large_title_size() - 34.0)
                                                    * LARGE_TITLE_BOTTOM_SHRINK)
                                        })
                                        .test_id("nav-bar-title"),
                                ),
                        ),
                ),
        )
        .into_element()
}

/// From the large title's line box to the bottom of its row at Large,
/// scaled with the title: its baseline [`BASELINE_UP`] above the row's
/// bottom, as UIKit sets it (measured).
const LARGE_TITLE_BOTTOM: f32 = 6.8;

/// How much less room under the title there is per point it grows (its
/// baseline stays about 17 pt above the row's bottom).
const LARGE_TITLE_BOTTOM_SHRINK: f32 = 0.257;
