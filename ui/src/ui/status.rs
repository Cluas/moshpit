//! Short states at a glance (`docs/nsh/UI-DESIGN.md` §4.11, §4.12):
//! [`Badge`], a short state, count or classification, and [`StatusDot`],
//! an agent's or a connection's state as a dot.
//!
//! Neither carries meaning by color alone: a badge is words, and a dot
//! always stands beside words (or is given a label for screen readers).

use nsh_core::prelude::*;

use crate::ui::{
    feedback::{SpinnerSize, spinner},
    icon::{IconSize, icon},
    motion::BREATHE_MS,
    tokens::{number, token},
    typography::TextToken,
};

/// What a badge says (§4.11). Most are neutral; a status color only for
/// that status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BadgeVariant {
    Neutral,
    Outline,
    Primary,
    Success,
    Warning,
    Info,
    Destructive,
}

impl BadgeVariant {
    /// Its fill and its ink, as color tokens.
    fn colors(self) -> (Option<&'static str>, &'static str) {
        match self {
            BadgeVariant::Neutral => (Some("secondary"), "secondary-foreground"),
            BadgeVariant::Outline => (None, "muted-foreground"),
            BadgeVariant::Primary => (Some("primary-soft"), "primary-text"),
            BadgeVariant::Success => (Some("success-soft"), "success-text"),
            BadgeVariant::Warning => (Some("warning-soft"), "warning-text"),
            BadgeVariant::Info => (Some("info-soft"), "info-text"),
            BadgeVariant::Destructive => (Some("destructive-soft"), "destructive-text"),
        }
    }
}

/// A badge's size: 20 or 24 pt tall at the default text size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BadgeSize {
    Sm,
    Md,
}

/// A badge ([`badge`]).
pub struct Badge {
    words: SharedString,
    variant: BadgeVariant,
    size: BadgeSize,
    mono: bool,
    symbol: Option<&'static str>,
    test_id: Option<SharedString>,
}

/// A small, neutral badge saying `words`.
pub fn badge(words: impl Into<SharedString>) -> Badge {
    Badge {
        words: words.into(),
        variant: BadgeVariant::Neutral,
        size: BadgeSize::Sm,
        mono: false,
        symbol: None,
        test_id: None,
    }
}

impl Badge {
    /// What it says (a row holding it says it too: [`super::row::Row::badge`]).
    pub fn words(&self) -> &SharedString {
        &self.words
    }

    pub fn variant(mut self, variant: BadgeVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: BadgeSize) -> Self {
        self.size = size;
        self
    }

    /// Set in SF Mono: what a machine names (`SSH`, `mosh`, `ed25519`).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// An SF Symbol before the words.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.symbol = Some(name);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Badge {
    fn into_element(self) -> AnyElement {
        let Badge {
            words,
            variant,
            size,
            mono,
            symbol,
            test_id,
        } = self;
        let (fill, ink) = variant.colors();
        let (height, padding, radius) = match size {
            BadgeSize::Sm => (20.0, 6.0, "radius-xs"),
            BadgeSize::Md => (24.0, 8.0, "radius-sm"),
        };
        let words = if mono {
            TextToken::MonoCaption
        } else {
            TextToken::Caption
        }
        .apply(text(words))
        .text_color(move || token(ink));
        let mut badge = view()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .flex_shrink_0()
            .self_start()
            .min_h(px(height))
            .px(px(padding))
            .rounded(px(number(radius)))
            .border_curve(BorderCurve::Continuous);
        if let Some(fill) = fill {
            badge = badge.bg(move || token(fill));
        } else {
            badge = badge
                .border(px(1.0))
                .border_color(move || token("border-strong"));
        }
        if let Some(name) = symbol {
            let tone = match variant {
                BadgeVariant::Neutral => crate::ui::icon::Tone::Foreground,
                BadgeVariant::Outline => crate::ui::icon::Tone::Muted,
                BadgeVariant::Primary => crate::ui::icon::Tone::Primary,
                BadgeVariant::Success => crate::ui::icon::Tone::Success,
                BadgeVariant::Warning => crate::ui::icon::Tone::Warning,
                BadgeVariant::Info => crate::ui::icon::Tone::Info,
                BadgeVariant::Destructive => crate::ui::icon::Tone::Destructive,
            };
            badge = badge.child(icon(name).size(IconSize::Sm).tone(tone));
        }
        if let Some(id) = &test_id {
            badge = badge.test_id(id);
        }
        badge.child(words).into_element()
    }
}

/// An agent's or a connection's state, as a dot (§4.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// An agent waits on you.
    Attention,
    /// An agent is working: the dot breathes.
    Working,
    /// An agent finished and you have not looked.
    Done,
    /// An agent at its prompt: a hollow ring.
    Idle,
    /// A connection is up.
    Live,
    /// A connection is being made: a spinner in the dot's place.
    Connecting,
    /// A connection is down: a hollow ring.
    Offline,
    /// A connection failed (refused, a bad password).
    Error,
}

impl Status {
    /// Its fill, or (hollow) its ring's color, as a color token.
    fn color(self) -> &'static str {
        match self {
            Status::Attention => "warning",
            Status::Working | Status::Live => "success",
            Status::Done => "info",
            Status::Idle | Status::Connecting => "muted-foreground",
            Status::Offline => "subtle-foreground",
            Status::Error => "destructive",
        }
    }

    fn hollow(self) -> bool {
        matches!(self, Status::Idle | Status::Offline)
    }
}

/// A dot's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DotSize {
    /// 8 pt: in a row's value lane, beside words.
    Sm,
    /// 10 pt: on a host's mark.
    Md,
}

impl DotSize {
    pub fn points(self) -> f32 {
        match self {
            DotSize::Sm => 8.0,
            DotSize::Md => 10.0,
        }
    }
}

/// A status dot ([`status_dot`]).
pub struct StatusDot {
    status: Status,
    size: DotSize,
    label: Option<SharedString>,
    test_id: Option<SharedString>,
}

/// A page's breath (§4.12): the value every working dot made under the
/// page breathes on, so a page full of agents at work runs one native
/// animation, not one per dot ([`share_breath`]).
#[derive(Clone)]
pub struct Breath(AnimatedValue);

/// Gives the page being made one breath for its working dots: full to 45%
/// and back on the token's period, looped for as long as the page lives.
/// With reduced motion there is none, and the dots hold still.
pub fn share_breath() {
    if reduce_motion() {
        return;
    }
    let breath = animated_value(0.0);
    let breathing = Animation::sequence([
        Animation::timing(
            &breath,
            1.0,
            Timing::new(BREATHE_MS / 2.0, Easing::EaseInOut),
        ),
        Animation::timing(
            &breath,
            0.0,
            Timing::new(BREATHE_MS / 2.0, Easing::EaseInOut),
        ),
    ])
    .looped(None);
    effect(move || drop(breathing.start()));
    provide_context(Breath(breath));
}

/// A small dot for `status`.
pub fn status_dot(status: Status) -> StatusDot {
    StatusDot {
        status,
        size: DotSize::Sm,
        label: None,
        test_id: None,
    }
}

impl StatusDot {
    pub fn size(mut self, size: DotSize) -> Self {
        self.size = size;
        self
    }

    /// What screen readers say for it, where no words stand beside it;
    /// otherwise it is hidden from them and the words carry the state.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for StatusDot {
    fn into_element(self) -> AnyElement {
        let StatusDot {
            status,
            size,
            label,
            test_id,
        } = self;
        let mut dot = if status == Status::Connecting {
            view().child(spinner(SpinnerSize::Sm))
        } else {
            let side = size.points();
            let color = status.color();
            let mut dot = view()
                .w(px(side))
                .h(px(side))
                .flex_shrink_0()
                .rounded_full();
            dot = if status.hollow() {
                dot.border(px(1.5)).border_color(move || token(color))
            } else {
                dot.bg(move || token(color))
            };
            if status == Status::Working
                && !reduce_motion()
                && let Some(Breath(shared)) = use_context::<Breath>()
            {
                // The page's breath (R6): one animation for every dot on it.
                dot = dot.bind_opacity(&shared.interpolate(&[0.0, 1.0], &[1.0, 0.45]));
            } else if status == Status::Working && !reduce_motion() {
                // One breath: full to 45% and back, on the token's period.
                let breath = animated_value(0.0);
                let breathing = Animation::sequence([
                    Animation::timing(
                        &breath,
                        1.0,
                        Timing::new(BREATHE_MS / 2.0, Easing::EaseInOut),
                    ),
                    Animation::timing(
                        &breath,
                        0.0,
                        Timing::new(BREATHE_MS / 2.0, Easing::EaseInOut),
                    ),
                ])
                .looped(None);
                // The effect holds the animation for as long as the dot
                // lives (dropping it would stop the breath).
                effect(move || drop(breathing.start()));
                dot = dot.bind_opacity(&breath.interpolate(&[0.0, 1.0], &[1.0, 0.45]));
            }
            dot
        };
        dot = match label {
            Some(label) => dot.role(AccessibilityRole::Image).label(label),
            None => dot.accessibility_elements_hidden(true),
        };
        if let Some(id) = &test_id {
            dot = dot.test_id(id);
        }
        dot.into_element()
    }
}
