//! What VoiceOver and a long language find on a screen (B744), read from
//! the headless layout (`Screen::layout_nodes`, the dump B659's test reads)
//! and the screen's selectors:
//!
//! - every view that takes a press itself has a role a screen reader
//!   announces as something to press, and a name (its label, else the
//!   texts in it);
//! - it is at least 44 × 44 pt to touch, its `hit_slop` counted;
//! - no view around it, or around a text, is of no size (B659: VoiceOver
//!   never reaches what is inside one);
//! - nothing that isn't in a sideways scroll goes past the screen's edges;
//! - no text is cut short (more lines, or more width, than it was given),
//!   squeezed to no width, or overlaps another, and none goes past a view
//!   that clips it.
//!
//! A text is cut short when, measured as the headless engine lays it out,
//! it needs more lines than its limit gives it (or, on a line that doesn't
//! wrap, more width than its box). The engine is `common::text_engine`'s:
//! Approx's 0.55 em a character, a wide (CJK) one at a full em as on a
//! phone.

use std::collections::{HashMap, HashSet};

use nsh_core::{AttributedText, Bounds, EllipsizeMode, NodeId, Points, TextEngine, ViewKind};
use nsh_testing::{Headless, LayoutNode};

use super::Natives;

/// The roles a screen reader announces as something to press.
pub const PRESS_ROLES: &[&str] = &[
    "button",
    "link",
    "switch",
    "imagebutton",
    "tab",
    "menuitem",
    "checkbox",
    "radio",
    "adjustable",
    "keyboardkey",
    "combobox",
    "search",
    "spinbutton",
];

/// The smallest a target may be, each way (Apple's HIG).
pub const TARGET: f32 = 44.0;

/// Every view `selector` matches, by its node id.
pub fn ids(app: &Headless, selector: &str) -> Vec<NodeId> {
    let screen = app.screen();
    let n = screen.count(selector).unwrap_or(0);
    (0..n)
        .filter_map(|k| screen.id(&format!("{selector} >> nth={k}")).ok())
        .collect()
}

/// What a finding is about: its test id, else its text, else its node id.
fn describe(node: &LayoutNode) -> String {
    if let Some(id) = &node.test_id {
        return format!("id:{id}");
    }
    if let Some(text) = &node.text {
        return format!("text:{:?}", text.text.text);
    }
    format!("view:{}", node.id)
}

/// The size a touch on `node` lands in: its bounds grown by its
/// `hit_slop` (which iOS's hit test honours: `nsh-platform-ios`'s
/// `inside`, and the core's overflow reaches over it), cut to every view
/// around it that clips (`clipsToBounds` stops the hit test there).
/// `around` is the views around `node`, nearest first.
pub fn touch_size(node: &LayoutNode, around: &[&LayoutNode], natives: &Natives) -> (f32, f32) {
    let slop = natives.hit_slop(node.id);
    let b = node.bounds;
    let (mut x0, mut y0) = (b.origin.x.0 - slop.left.0, b.origin.y.0 - slop.top.0);
    let mut x1 = b.origin.x.0 + b.size.width.0 + slop.right.0;
    let mut y1 = b.origin.y.0 + b.size.height.0 + slop.bottom.0;
    for clip in around.iter().filter(|a| a.clips) {
        let c = clip.bounds;
        x0 = x0.max(c.origin.x.0);
        y0 = y0.max(c.origin.y.0);
        x1 = x1.min(c.origin.x.0 + c.size.width.0);
        y1 = y1.min(c.origin.y.0 + c.size.height.0);
    }
    ((x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// The size a touch on the shown view with test id `id` lands in
/// ([`touch_size`]); panics when there is none.
pub fn touch_size_of(app: &Headless, natives: &Natives, id: &str) -> (f32, f32) {
    let nodes = app.screen().layout_nodes();
    let by_id: HashMap<NodeId, &LayoutNode> = nodes.iter().map(|n| (n.id, n)).collect();
    let node = nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(id) && !n.hidden)
        .unwrap_or_else(|| panic!("no view {id}"));
    let mut around = Vec::new();
    let mut at = node.parent;
    while let Some(view) = by_id.get(&at) {
        around.push(*view);
        at = view.parent;
    }
    touch_size(node, &around, natives)
}

/// What is wrong on `app`'s screen now, one line each, for a viewport
/// `width` wide. `skip` names test ids (and the views inside them) the
/// audit leaves alone: a native view drawing its own (the terminal's
/// grid), or what another bug already covers. A target's size counts its
/// `hit_slop`, as `natives` (the screen's observer) saw it.
pub fn audit(app: &Headless, natives: &Natives, width: f32, skip: &[&str]) -> Vec<String> {
    let screen = app.screen();
    let nodes = screen.layout_nodes();
    let by_id: HashMap<NodeId, &LayoutNode> = nodes.iter().map(|n| (n.id, n)).collect();
    let ancestors = |node: &LayoutNode| {
        let mut out = Vec::new();
        let mut at = node.parent;
        while let Some(around) = by_id.get(&at) {
            out.push(*around);
            at = around.parent;
        }
        out
    };
    let skipped = |node: &LayoutNode| {
        std::iter::once(node)
            .chain(ancestors(node))
            .any(|n| n.test_id.as_deref().is_some_and(|id| skip.contains(&id)))
    };
    let sideways = |node: &LayoutNode| {
        ancestors(node)
            .iter()
            .any(|a| a.scroll.as_ref().is_some_and(|s| s.horizontal))
    };
    let mut roled = HashSet::new();
    let mut named = HashSet::new();
    for role in PRESS_ROLES {
        roled.extend(ids(app, &format!("role:{role}")));
        named.extend(ids(app, &format!("role:{role}[/\\S/]")));
    }

    // Text fields, the app's and the host's own (the password's secure
    // field is a native view).
    let inputs: Vec<&LayoutNode> = nodes
        .iter()
        .filter(|n| matches!(n.kind, ViewKind::Input | ViewKind::Native))
        .collect();
    let mut found = Vec::new();
    let shown = |node: &LayoutNode| {
        !node.hidden && node.bounds.size.width.0 > 0.0 && node.bounds.size.height.0 > 0.0
    };
    let empty_around = |node: &LayoutNode| {
        ancestors(node)
            .into_iter()
            .find(|a| a.bounds.size.width.0 <= 0.0 || a.bounds.size.height.0 <= 0.0)
    };
    let right = |b: &Bounds<Points>| b.origin.x.0 + b.size.width.0;

    // What no one sees: a view at no opacity (it, or a view around it: a
    // measuring copy), or one a later layer as wide as the screen, with a
    // background nothing shows through, covers whole (a screen drawn over
    // another: the pane overview over the terminal). The views are in tree
    // order, which is the order they are drawn in, so a covering view
    // comes after what it covers. A smaller view drawn over a text still
    // counts as hiding it: that is for the audit to find.
    let opacity = |node: &LayoutNode| {
        ancestors(node)
            .iter()
            .fold(natives.opacity(node.id), |o, a| o * natives.opacity(a.id))
    };
    let index: HashMap<NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let contains = |outer: &Bounds<Points>, inner: &Bounds<Points>| {
        outer.origin.x.0 <= inner.origin.x.0 + 0.5
            && outer.origin.y.0 <= inner.origin.y.0 + 0.5
            && right(outer) + 0.5 >= right(inner)
            && outer.origin.y.0 + outer.size.height.0 + 0.5
                >= inner.origin.y.0 + inner.size.height.0
    };
    let unseen = |node: &LayoutNode| {
        opacity(node) <= 0.01
            || nodes[index[&node.id] + 1..].iter().any(|over| {
                !over.hidden
                    && over.bounds.size.width.0 >= width - 0.5
                    && natives.opaque_background(over.id)
                    && opacity(over) >= 0.99
                    && contains(&over.bounds, &node.bounds)
            })
    };

    // Squeezed out: words laid out with no width at all, which the loop
    // below (shown views only) never sees: a middle-shortened address
    // beside a badge as wide as its lane.
    for node in nodes.iter().filter(|n| {
        !n.hidden
            && !skipped(n)
            && n.bounds.size.width.0 < 1.0
            && n.bounds.size.height.0 > 0.0
            && n.text
                .as_ref()
                .is_some_and(|t| !t.text.text.trim().is_empty())
            && empty_around(n).is_none()
            && !sideways(n)
            && opacity(n) > 0.01
    }) {
        found.push(format!(
            "{}: squeezed to nothing ({:.0} pt)",
            describe(node),
            node.bounds.size.width.0
        ));
    }

    for node in nodes.iter().filter(|n| shown(n) && !skipped(n)) {
        let what = describe(node);
        let presses = screen.press_target(&format!("view:{}", node.id)) == Ok(Some(node.id));
        // A text field or a native control is the platform's, trait and
        // all; so is a field's row, whose press focuses the field in it.
        let platforms = matches!(node.kind, ViewKind::Input | ViewKind::Native)
            || inputs
                .iter()
                .any(|input| ancestors(input).iter().any(|a| a.id == node.id));
        if presses && !platforms {
            if !roled.contains(&node.id) {
                found.push(format!(
                    "{what}: takes a press but has no role a screen reader announces"
                ));
            } else if !named.contains(&node.id) {
                found.push(format!("{what}: a control with no name"));
            }
        }
        if presses {
            let (w, h) = touch_size(node, &ancestors(node), natives);
            if w < TARGET - 0.5 || h < TARGET - 0.5 {
                found.push(format!(
                    "{what}: a target {w:.0} × {h:.0} pt, under {TARGET}"
                ));
            }
        }
        let seen = !unseen(node);
        if presses || node.text.is_some() {
            if let Some(around) = empty_around(node) {
                found.push(format!(
                    "{what}: inside a view of no size ({}), out of a screen reader's reach",
                    describe(around)
                ));
            }
            if seen
                && !sideways(node)
                && (node.bounds.origin.x.0 < -0.5 || right(&node.bounds) > width + 0.5)
            {
                found.push(format!(
                    "{what}: past the screen's edge (x {:.0} to {:.0} of {width})",
                    node.bounds.origin.x.0,
                    right(&node.bounds)
                ));
            }
        }
        let Some(text) = node.text.as_ref().filter(|_| seen) else {
            continue;
        };
        let attributed = &text.text;
        if attributed.text.trim().is_empty() {
            continue;
        }
        // Cut short: the text measured as the headless engine lays it out
        // needs more lines (or, on one unwrapped line, more width) than it
        // was given. A head or middle ellipsis (a path) is the design.
        let given = text.content.size.width.0;
        // Squeezed out: words with no width left to show any of them,
        // however they would shorten (a middle-shortened address beside
        // a badge as wide as the lane).
        if given < 1.0 && !sideways(node) {
            found.push(format!("{what}: squeezed to nothing ({given:.0} pt)"));
            continue;
        }
        // The engine's 0.55 em a Latin character is a tenth wider than San
        // Francisco's (about 0.5 em): a Latin text is given that much more
        // room before it counts as cut. CJK, measured at its width, gets
        // none.
        let latin = !attributed.text.chars().any(|c| (c as u32) >= 0x1100);
        let room = if latin { given / 0.9 } else { given };
        let designed = matches!(
            attributed.ellipsize,
            EllipsizeMode::Head | EllipsizeMode::Middle
        );
        let engine = super::text_engine::WideTextEngine;
        let mut free: AttributedText = (**attributed).clone();
        free.max_lines = 0;
        let short = if attributed.no_wrap {
            engine.measure(&engine.prepare(&free), None).width.0 > room + 0.5
        } else {
            let need = engine
                .measure(&engine.prepare(&free), Some(Points(room)))
                .height
                .0;
            let have = engine
                .measure(&engine.prepare(attributed), Some(Points(room)))
                .height
                .0;
            need > have + 0.5
        };
        if short && !designed && !sideways(node) {
            found.push(format!(
                "{what}: cut short ({} line(s) in {given:.0} pt)",
                if attributed.no_wrap {
                    1
                } else {
                    attributed.max_lines
                }
            ));
        }
        for clip in ancestors(node).into_iter().filter(|a| a.clips) {
            if node.bounds.origin.x.0 < clip.bounds.origin.x.0 - 0.5
                || right(&node.bounds) > right(&clip.bounds) + 0.5
            {
                found.push(format!("{what}: clipped by {}", describe(clip)));
                break;
            }
        }
    }

    // Texts drawn over one another (neither inside the other).
    let texts: Vec<&LayoutNode> = nodes
        .iter()
        .filter(|n| {
            shown(n)
                && !skipped(n)
                && !unseen(n)
                && n.text
                    .as_ref()
                    .is_some_and(|t| !t.text.text.trim().is_empty())
        })
        .collect();
    for (i, a) in texts.iter().enumerate() {
        for b in &texts[i + 1..] {
            let overlap_w = right(&a.bounds).min(right(&b.bounds))
                - a.bounds.origin.x.0.max(b.bounds.origin.x.0);
            let bottom = |n: &LayoutNode| n.bounds.origin.y.0 + n.bounds.size.height.0;
            let overlap_h = bottom(a).min(bottom(b)) - a.bounds.origin.y.0.max(b.bounds.origin.y.0);
            if overlap_w > 2.0 && overlap_h > 2.0 {
                found.push(format!("{} overlaps {}", describe(a), describe(b)));
            }
        }
    }
    found
}
