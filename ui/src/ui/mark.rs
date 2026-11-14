//! Who a row is about (`docs/nsh/UI-DESIGN.md` §4.13): a host's mark (its
//! initial on a disc, a status dot on its corner) and an agent's (a glyph,
//! no tile).
//!
//! A host mark is decoration beside the host's name: screen readers hear
//! the name and its state from the row or head it sits in, not from the
//! mark. An agent's mark is the state's glyph, filled and colored only for
//! a state (§3.6: the waiting hand is solid amber), muted otherwise.

use nsh_core::prelude::*;

use crate::ui::{
    icon::{IconSize, Tone, icon},
    status::{DotSize, Status, status_dot},
    tokens::token,
    typography::TextToken,
};

/// A host mark's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkSize {
    /// 40 pt: a connection card's head.
    Md,
    /// 28 pt: in a row's leading lane (SAVED), which is 28 pt wide.
    Sm,
}

impl MarkSize {
    pub fn points(self) -> f32 {
        match self {
            MarkSize::Md => 40.0,
            MarkSize::Sm => 28.0,
        }
    }

    /// The initial's size: `mono-subhead` on the head's mark, a step down
    /// (`mono-footnote`) in a row's lane.
    fn text(self) -> TextToken {
        match self {
            MarkSize::Md => TextToken::MonoSubhead,
            MarkSize::Sm => TextToken::MonoFootnote,
        }
    }
}

/// The ring around a mark's status dot, in the card's color, so the dot
/// reads apart from the disc under it.
const RING: f32 = 2.0;

/// A host's mark ([`host_mark`]).
pub struct HostMark {
    initial: SharedString,
    size: MarkSize,
    status: Option<Status>,
    test_id: Option<SharedString>,
}

/// The mark of a host whose name starts with `initial`.
pub fn host_mark(initial: impl Into<SharedString>) -> HostMark {
    HostMark {
        initial: initial.into(),
        size: MarkSize::Md,
        status: None,
        test_id: None,
    }
}

impl HostMark {
    pub fn size(mut self, size: MarkSize) -> Self {
        self.size = size;
        self
    }

    /// The connection's state, as a dot on the mark's corner: live, a
    /// spinner while it connects, a hollow ring offline, red when it
    /// failed. Offline or failed, the disc goes grey too.
    pub fn status(mut self, status: Status) -> Self {
        self.status = Some(status);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for HostMark {
    fn into_element(self) -> AnyElement {
        let HostMark {
            initial,
            size,
            status,
            test_id,
        } = self;
        let side = size.points();
        let down = matches!(status, Some(Status::Offline | Status::Error));
        let (fill, ink) = if down {
            ("muted", "muted-foreground")
        } else {
            ("primary-soft", "primary-text")
        };
        let mut mark = view()
            .relative()
            .w(px(side))
            .h(px(side))
            .flex_shrink_0()
            .rounded_full()
            .bg(move || token(fill))
            .items_center()
            .justify_center()
            .accessibility_elements_hidden(true)
            .child(
                size.text()
                    .apply(text(initial))
                    .font_weight(FontWeight::SEMIBOLD)
                    .number_of_lines(1u32)
                    .text_color(move || token(ink)),
            );
        if let Some(status) = status {
            let dot = match size {
                MarkSize::Md => DotSize::Md,
                MarkSize::Sm => DotSize::Sm,
            };
            // A spinner is wider than a dot: it sits on the corner in its
            // own 16 pt.
            let inner = if status == Status::Connecting {
                16.0
            } else {
                dot.points()
            };
            let outer = inner + 2.0 * RING;
            mark = mark.child(
                view()
                    .absolute()
                    .right(px(-RING))
                    .bottom(px(-RING))
                    .w(px(outer))
                    .h(px(outer))
                    .rounded_full()
                    .bg(move || token("card"))
                    .items_center()
                    .justify_center()
                    .child(status_dot(status).size(dot)),
            );
        }
        if let Some(id) = test_id {
            mark = mark.test_id(&id);
        }
        mark.into_element()
    }
}

/// An agent's mark for `status` (an agent's: attention, working, done,
/// idle), in a row's 28 pt leading lane: the waiting hand solid amber, the
/// done check blue, the sparkle muted (dimmer while idle).
pub fn agent_mark(status: Status) -> AnyElement {
    let (name, tone) = match status {
        Status::Attention => ("hand.raised.fill", Tone::Warning),
        Status::Done => ("checkmark.circle", Tone::Info),
        Status::Idle => ("sparkle", Tone::Subtle),
        _ => ("sparkle", Tone::Muted),
    };
    icon(name).size(IconSize::Md).tone(tone).into_element()
}
