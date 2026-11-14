//! A page of sections under a bar (`docs/nsh/UI-DESIGN.md` §3.2): the list
//! a settings page, an editor or a sheet scrolls, its sections `space-6`
//! apart inside the page's inset (`space-page`, `space-page-regular` on a
//! wide iPad page, as the bar's items stand), no wider than
//! `content-max-width` and centred past it, starting `space-4` under the
//! bar and ending clear of the home indicator.
//!
//! It scrolls under the bar as every list does: it reports where it stands
//! to the bar's [`nav_bar::ScrollEdge`], which draws the bar's edge. So
//! this module uses `crate::nav_bar` (the bar's metrics and its space), as
//! [`super::button::IconButton`] does.

use nsh_core::prelude::*;

use crate::{
    nav_bar::{self, BarMetrics},
    ui::tokens::number,
};

/// From the bar to the first section (`space-4`).
pub const TOP: f32 = 16.0;
/// Between two sections (`space-6`).
pub const BETWEEN: f32 = 24.0;
/// Below the last section, over the home indicator (`space-8`).
pub const BOTTOM: f32 = 32.0;

/// The page's list of `sections`, the scroll view `test_id`.
pub fn page(
    bar: BarMetrics,
    scrolled: nav_bar::ScrollEdge,
    test_id: &str,
    sections: impl IntoElements,
) -> Scroll {
    scrolled
        .follow(scroll().absolute().inset_0().test_id(test_id))
        .child(
            view()
                .w_full()
                .px(move || px(nav_bar::margin()))
                .items_center()
                .child(view().h(move || nav_bar::bar_space(bar)))
                .child(
                    view()
                        .w_full()
                        .max_w(px(number("content-max-width")))
                        .pt(px(TOP))
                        .gap(px(BETWEEN))
                        .children(sections.into_elements()),
                )
                .child(view().h(move || px(BOTTOM + safe_area().bottom.0))),
        )
}
