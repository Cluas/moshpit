//! Nothing here yet, and something on its way (`docs/nsh/UI-DESIGN.md`
//! §4.14, §4.15): [`Empty`], the state of a page or a group with nothing in
//! it, and the skeleton ([`SkeletonRow`], [`SkeletonList`]) that stands in
//! for rows of a known shape until they arrive, which [`loaded`] swaps for
//! the content.
//!
//! A skeleton is for a real wait: a page whose data is already there (one
//! settings page to the next) shows its content at once.

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::ui::{
    button::{Button, Size},
    icon::{IconSize, Tone, icon},
    motion::{Duration, ENTER, LEAVE, SKELETON_MS},
    row::{GAP, HEIGHT_LG, HEIGHT_MD, LEADING, PADDING_X, PADDING_Y},
    tokens::{number, token},
    typography::TextToken,
};

/// Where an empty state stands (§4.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmptyVariant {
    /// The whole page: centered, with room above and below, the action
    /// large.
    Page,
    /// Inside a group (no keys yet, an empty folder): leading-aligned and
    /// small, the action small.
    Inline,
}

/// What an empty state shows above (or, inline, before) its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Graphic {
    /// The app's mark (the host's `moshpit/app-icon-mark`, as Home's empty
    /// state draws it).
    Mark,
    /// An SF Symbol, muted.
    Symbol(&'static str),
}

/// The app's mark's side on an empty page (Home's).
const MARK_SIDE: f32 = 72.0;

/// An empty state ([`empty`]).
pub struct Empty {
    title: SharedString,
    message: Option<SharedString>,
    graphic: Option<Graphic>,
    /// A page's own graphic in the mark's place ([`Empty::graphic`]).
    art: Option<AnyElement>,
    action: Option<Button>,
    variant: EmptyVariant,
    test_id: Option<SharedString>,
}

/// An empty page titled `title`, under the app's mark: nothing else until
/// the setters add it.
pub fn empty(title: impl Into<SharedString>) -> Empty {
    Empty {
        title: title.into(),
        message: None,
        graphic: None,
        art: None,
        action: None,
        variant: EmptyVariant::Page,
        test_id: None,
    }
}

impl Empty {
    pub fn variant(mut self, variant: EmptyVariant) -> Self {
        self.variant = variant;
        self
    }

    /// What there will be here, and how to get it: at most two lines.
    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// An SF Symbol for what is missing, in place of the mark (a page's
    /// default); an inline state shows only a symbol.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.graphic = Some(Graphic::Symbol(name));
        self
    }

    /// The app's mark (a page's default): for the app having nothing at
    /// all yet (no connections).
    pub fn mark(mut self) -> Self {
        self.graphic = Some(Graphic::Mark);
        self
    }

    /// A page's own graphic in the mark's place: Home's companion (B746),
    /// which shows the mark itself ([`app_mark`]) while it is away. A page
    /// only; inline there is no place for it.
    pub fn graphic(mut self, art: impl IntoElement) -> Self {
        self.art = Some(art.into_element());
        self
    }

    /// The one way forward ([`crate::ui::button::button`], primary or
    /// secondary). The variant sets its size: large on a page, small inline.
    pub fn action(mut self, action: Button) -> Self {
        self.action = Some(action);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Empty {
    fn into_element(self) -> AnyElement {
        let Empty {
            title,
            message,
            graphic,
            art,
            action,
            variant,
            test_id,
        } = self;
        let mut state = match variant {
            EmptyVariant::Page => page(title, message, art.ok_or(graphic), action),
            EmptyVariant::Inline => inline(title, message, graphic, action),
        };
        if let Some(id) = &test_id {
            state = state.test_id(id);
        }
        state.into_element()
    }
}

/// The app's mark as an empty page draws it (the host's
/// `moshpit/app-icon-mark`): the icon the person picked.
pub fn app_mark() -> AnyElement {
    native("moshpit/app-icon-mark")
        .w(px(MARK_SIDE))
        .h(px(MARK_SIDE))
        .accessibility_elements_hidden(true)
        .into_element()
}

/// An empty page: centered, 48 pt above and 32 below, the graphic (the
/// mark unless a symbol was given) over the title over the message over
/// the action.
fn page(
    title: SharedString,
    message: Option<SharedString>,
    graphic: Result<AnyElement, Option<Graphic>>,
    action: Option<Button>,
) -> View {
    let mut body = view()
        .w_full()
        .items_center()
        .pt(px(48.0))
        .pb(px(32.0))
        .px(px(number("space-page")));
    {
        let graphic = match graphic {
            Ok(art) => art,
            Err(None | Some(Graphic::Mark)) => app_mark(),
            Err(Some(Graphic::Symbol(name))) => icon(name)
                .size(IconSize::Lg)
                .tone(Tone::Muted)
                .into_element(),
        };
        // The page's width, so a graphic of the page's own (Home's
        // companion and what it says) can lay itself out across it.
        body = body.child(view().w_full().items_center().mb(px(16.0)).child(graphic));
    }
    body = body.child(
        TextToken::Title
            .apply(text(title))
            .text_color(move || token("foreground"))
            .text_center()
            .role(AccessibilityRole::Header),
    );
    if let Some(message) = message {
        body = body.child(
            TextToken::Subhead
                .apply(text(message))
                .text_color(move || token("muted-foreground"))
                .text_center()
                .number_of_lines(2)
                .mt(px(8.0)),
        );
    }
    if let Some(action) = action {
        // As wide as the page's content at most, so a long label wraps
        // instead of running off the screen (B749).
        body = body.child(
            view()
                .mt(px(24.0))
                .max_w(relative(1.0))
                .child(action.size(Size::Lg)),
        );
    }
    body
}

/// An empty group, laid out as a row of its card: the symbol in the row's
/// leading lane ([`LEADING`]), the title over the message over the action
/// where a row's title starts, all inside the row's padding
/// ([`PADDING_X`]). Without a symbol the words start at the padding, where
/// a row without a leading lane has its title.
fn inline(
    title: SharedString,
    message: Option<SharedString>,
    graphic: Option<Graphic>,
    action: Option<Button>,
) -> View {
    let mut words = view().flex_1().items_start().gap(px(4.0)).child(
        TextToken::Subhead
            .apply(text(title))
            .text_color(move || token("foreground")),
    );
    if let Some(message) = message {
        words = words.child(
            TextToken::Footnote
                .apply(text(message))
                .text_color(move || token("muted-foreground"))
                .number_of_lines(2),
        );
    }
    if let Some(action) = action {
        words = words.child(view().mt(px(8.0)).child(action.size(Size::Sm)));
    }
    let mut state = view()
        .w_full()
        .flex_row()
        .items_start()
        .gap(px(GAP))
        .px(px(PADDING_X))
        .py(px(12.0));
    match graphic {
        Some(Graphic::Symbol(name)) => {
            state = state.child(
                view()
                    .w(px(LEADING))
                    .flex_shrink_0()
                    .items_center()
                    .child(icon(name).size(IconSize::Md).tone(Tone::Muted)),
            );
        }
        // The mark is the app's, for a page; inline it would be noise.
        Some(Graphic::Mark) | None => {}
    }
    state.child(words)
}

/// The bars' opacity at the two ends of a pulse, and where they rest
/// while motion is reduced (§5.2: a static middle state).
const PULSE_FULL: f32 = 1.0;
const PULSE_LOW: f32 = 0.6;
const PULSE_REST: f32 = 0.8;
/// The bars' fill: `secondary`, which stands off a card in both
/// appearances (`muted` all but vanished on a dark card, R3's device shots).
const BAR: &str = "secondary";

/// A placeholder row ([`skeleton_row`]).
pub struct SkeletonRow {
    leading: bool,
    subtitle: bool,
    test_id: Option<SharedString>,
}

/// A row of a known shape, a title bar over a subtitle bar, on `secondary`,
/// pulsing slowly ([`SKELETON_MS`]); still while motion is reduced.
pub fn skeleton_row() -> SkeletonRow {
    SkeletonRow {
        leading: false,
        subtitle: true,
        test_id: None,
    }
}

impl SkeletonRow {
    /// A square in the row's leading lane ([`crate::ui::row::LEADING`]), where
    /// its mark or icon will be.
    pub fn leading(mut self, leading: bool) -> Self {
        self.leading = leading;
        self
    }

    /// The subtitle bar (on by default).
    pub fn subtitle(mut self, subtitle: bool) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for SkeletonRow {
    fn into_element(self) -> AnyElement {
        let SkeletonRow {
            leading,
            subtitle,
            test_id,
        } = self;
        let scale = font_scale();
        let bar = |width: f32, height: f32| {
            view()
                .w(relative(width))
                .h(px(height))
                .rounded(px(number("radius-xs")))
                .bg(move || token(BAR))
        };
        let mut bars = view()
            .flex_1()
            .gap(px(6.0))
            .child(bar(0.6, TextToken::Subhead.size(scale)));
        if subtitle {
            bars = bars.child(bar(0.4, TextToken::Caption.size(scale)));
        }
        let mut row = view()
            .w_full()
            .flex_row()
            .items_center()
            .gap(px(GAP))
            .px(px(PADDING_X))
            .py(px(PADDING_Y))
            .min_h(px(if subtitle { HEIGHT_LG } else { HEIGHT_MD }))
            .accessibility_elements_hidden(true);
        if leading {
            row = row.child(
                view()
                    .w(px(LEADING))
                    .h(px(LEADING))
                    .flex_shrink_0()
                    .rounded(px(number("radius-sm")))
                    .border_curve(BorderCurve::Continuous)
                    .bg(move || token(BAR)),
            );
        }
        row = row.child(bars);
        row = if reduce_motion() {
            row.opacity(PULSE_REST)
        } else {
            // One breath: full to half and back, on the token's period.
            let breath = animated_value(0.0);
            let breathing = Animation::sequence([
                Animation::timing(
                    &breath,
                    1.0,
                    Timing::new(SKELETON_MS / 2.0, Easing::EaseInOut),
                ),
                Animation::timing(
                    &breath,
                    0.0,
                    Timing::new(SKELETON_MS / 2.0, Easing::EaseInOut),
                ),
            ])
            .looped(None);
            // The effect holds the animation for as long as the row lives
            // (dropping it would stop the pulse).
            effect(move || drop(breathing.start()));
            row.bind_opacity(&breath.interpolate(&[0.0, 1.0], &[PULSE_FULL, PULSE_LOW]))
        };
        if let Some(id) = &test_id {
            row = row.test_id(id);
        }
        row.into_element()
    }
}

/// A list's placeholder ([`skeleton_list`]).
pub struct SkeletonList {
    rows: usize,
    leading: bool,
    subtitle: bool,
    test_id: Option<SharedString>,
}

/// `rows` placeholder rows, one thing to screen readers: "Loading", busy.
pub fn skeleton_list(rows: usize) -> SkeletonList {
    SkeletonList {
        rows,
        leading: false,
        subtitle: true,
        test_id: None,
    }
}

impl SkeletonList {
    /// Each row's leading square ([`SkeletonRow::leading`]).
    pub fn leading(mut self, leading: bool) -> Self {
        self.leading = leading;
        self
    }

    /// Each row's subtitle bar ([`SkeletonRow::subtitle`]).
    pub fn subtitle(mut self, subtitle: bool) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for SkeletonList {
    fn into_element(self) -> AnyElement {
        let SkeletonList {
            rows,
            leading,
            subtitle,
            test_id,
        } = self;
        // As the spinner says it: the bars are hidden, the list is one
        // busy element.
        let mut list = view()
            .w_full()
            .label(t!("Loading"))
            .accessibility_state(AccessibilityState {
                busy: true,
                ..AccessibilityState::default()
            })
            .children((0..rows).map(move |_| skeleton_row().leading(leading).subtitle(subtitle)));
        if let Some(id) = &test_id {
            list = list.test_id(id);
        }
        list.into_element()
    }
}

/// `skeleton()` until `ready()`, then `content()`: the content fades in
/// (`base`, entering) while the skeleton, laid over it, fades out (`base`,
/// leaving) and goes. While motion is reduced the content replaces the
/// skeleton at once; content that is there from the start shows at once,
/// with no skeleton.
pub fn loaded(
    ready: impl Fn() -> bool + 'static,
    skeleton: impl Fn() -> AnyElement + 'static,
    content: impl Fn() -> AnyElement + 'static,
) -> AnyElement {
    let ready = computed(ready);
    let reduced = reduce_motion();
    // A skeleton was shown: the content arriving crossfades with it.
    let waited = stored(false);
    // The skeleton fading out over the content, until it has gone.
    let leaving = signal(false);
    let pending = stored(None::<Spawned<()>>);
    effect(move || {
        let ready = ready.get();
        if let Some(timer) = pending.replace(None) {
            timer.cancel();
        }
        let fades = ready && waited.get() && !reduced;
        leaving.set(fades);
        if fades {
            pending.replace(Some(set_timeout(
                std::time::Duration::from_millis(Duration::Base.nominal_ms() as u64),
                move || leaving.set(false),
            )));
        }
    });
    let skeleton = Rc::new(skeleton);
    let fade = |from: f32, to: f32, easing: Easing| {
        MotionSpec::new(KeyframeTiming::new(Duration::Base.nominal_ms()).easing(easing))
            .values(MotionChannel::Opacity, [from, to])
    };
    view()
        .w_full()
        .relative()
        .child(
            when(
                move || ready.get(),
                move || {
                    if reduced || !waited.get() {
                        return content();
                    }
                    view()
                        .w_full()
                        .bind_motion(&motion().entering(fade(0.0, 1.0, ENTER)))
                        .child(content())
                        .into_element()
                },
            )
            .otherwise({
                let skeleton = skeleton.clone();
                move || {
                    waited.replace(true);
                    skeleton()
                }
            }),
        )
        .child(when(
            move || leaving.get(),
            move || {
                view()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .pointer_events(PointerEvents::None)
                    .accessibility_elements_hidden(true)
                    .bind_motion(&motion().entering(fade(1.0, 0.0, LEAVE)))
                    .child(skeleton())
            },
        ))
        .into_element()
}
