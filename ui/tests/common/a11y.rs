//! What VoiceOver and a long language find on a screen (B744), read from
//! the headless layout (`Screen::layout_nodes`, the dump B659's test reads)
//! and the screen's selectors:
//!
//! - every view that takes a press itself has a role a screen reader
//!   announces as something to press, and a name (its label, else the
//!   texts in it);
//! - it is at least 44 × 44 pt;
//! - no view around it, or around a text, is of no size (B659: VoiceOver
//!   never reaches what is inside one);
//! - nothing that isn't in a sideways scroll goes past the screen's edges;
//! - no text is cut short (more lines, or more width, than it was given)
//!   or overlaps another, and none goes past a view that clips it.
//!
//! A text is cut short when, measured as the headless engine lays it out,
//! it needs more lines than its limit gives it (or, on a line that doesn't
//! wrap, more width than its box). The headless engine measures every
//! character at `0.55 ×` its font size: about right for Latin text,
//! half of what Chinese, Japanese and Korean take, so their verdicts here
//! are optimistic and the device has the last word.

use std::collections::{HashMap, HashSet};

use nsh_core::{
    ApproxTextEngine, AttributedText, Bounds, EllipsizeMode, NodeId, Points, TextEngine, ViewKind,
};
use nsh_testing::{Headless, LayoutNode};

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

/// What is wrong on `app`'s screen now, one line each, for a viewport
/// `width` wide. `skip` names test ids (and the views inside them) the
/// audit leaves alone: a native view drawing its own (the terminal's
/// grid), or what another bug already covers.
pub fn audit(app: &Headless, width: f32, skip: &[&str]) -> Vec<String> {
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
            let size = node.bounds.size;
            if size.width.0 < TARGET - 0.5 || size.height.0 < TARGET - 0.5 {
                found.push(format!(
                    "{what}: a target {:.0} × {:.0} pt, under {TARGET}",
                    size.width.0, size.height.0
                ));
            }
        }
        if presses || node.text.is_some() {
            if let Some(around) = empty_around(node) {
                found.push(format!(
                    "{what}: inside a view of no size ({}), out of a screen reader's reach",
                    describe(around)
                ));
            }
            if !sideways(node)
                && (node.bounds.origin.x.0 < -0.5 || right(&node.bounds) > width + 0.5)
            {
                found.push(format!(
                    "{what}: past the screen's edge (x {:.0} to {:.0} of {width})",
                    node.bounds.origin.x.0,
                    right(&node.bounds)
                ));
            }
        }
        let Some(text) = &node.text else { continue };
        let attributed = &text.text;
        if attributed.text.trim().is_empty() {
            continue;
        }
        // Cut short: the text measured as the headless engine lays it out
        // needs more lines (or, on one unwrapped line, more width) than it
        // was given. A head or middle ellipsis (a path) is the design.
        let given = text.content.size.width.0;
        // The engine's 0.55 em a character is a tenth wider than San
        // Francisco's Latin letters (about 0.5 em): a Latin text is given
        // that much more room before it counts as cut. CJK, which the
        // engine undercounts instead, gets none.
        let latin = !attributed.text.chars().any(|c| (c as u32) >= 0x1100);
        let room = if latin { given / 0.9 } else { given };
        let designed = matches!(
            attributed.ellipsize,
            EllipsizeMode::Head | EllipsizeMode::Middle
        );
        let engine = ApproxTextEngine;
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
