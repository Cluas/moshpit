//! A list row (`docs/nsh/UI-DESIGN.md` §4.5): the row every list, settings
//! group and card of the redesign is made of.
//!
//! Its lanes are fixed, so the rows of a section line up on one spine:
//!
//! ```text
//! [leading 28] [title / subtitle ……] [value] [accessory]
//! ```
//!
//! - The leading lane is 28 pt (an icon, a tile, a mark). A row without one
//!   can still keep the lane ([`Row::reserve_leading`]) so its title starts
//!   where its neighbours' do.
//! - The title takes one line; a subtitle under it one more. A path in the
//!   subtitle shortens in its middle ([`Row::mono_subtitle`]). A subtitle
//!   that explains the row wraps instead ([`Row::wrap_subtitle`]).
//! - The value sits at the trailing edge. The title keeps its own width and
//!   the value takes what is left (B317): when the two do not fit, the value
//!   gives way four times faster than the title, so a label never wraps
//!   while its value still has room.
//! - The value goes under the title from the first accessibility text
//!   size on, when the title alone would take more than 60% of the content
//!   lane, when a value that must be read whole ([`Row::whole_value`], a
//!   choice's name) does not fit beside it, or when asked
//!   ([`Row::stacked`]). The row measures its words with the platform's
//!   text engine (`measure_text`) against its laid-out width.
//! - The accessory is a chevron, a check, a disclosure arrow, a spinner
//!   while loading, a control ([`Row::trailing`]: a switch), or a badge
//!   ([`Row::badge`]), whose words the row says after its title.
//! - A side action ([`Row::side_action`]) sits just before the accessory,
//!   as UIKit's detail button sits before a disclosure chevron: its own
//!   44 pt target over a lane the row keeps for it, so the row keeps its
//!   press and its chevron, and the side action takes only its own.
//!
//! A row with an action ([`Row::on_press`]) is `nsh_base`'s button (a
//! disclosure row its own target, which screen readers hear expanded or
//! collapsed: `nsh_base`'s button says only selected): one target over the
//! whole row, its press a wash (`accent`) that fills the
//! row inside its card's corners. Rows draw no separators: the
//! [`super::section::Section`] they sit in does, at the inset a row
//! declares ([`Row::inset`]).

use std::rc::Rc;

use nsh_core::{EllipsizeMode, prelude::*};

use crate::{
    face::{self, Design},
    ui::{
        feedback::{SpinnerSize, spinner},
        icon::{IconSize, Tone, icon},
        motion::Duration,
        section::Inset,
        status::{Badge, DotSize, Status, status_dot},
        tokens::{number, token},
        typography::TextToken,
    },
};

/// The horizontal padding of a row (and of a field: the same spine).
pub const PADDING_X: f32 = 16.0;
/// The vertical padding of a row: with a 17 pt body line, 44 pt tall.
pub const PADDING_Y: f32 = 8.0;
/// The leading lane's width.
pub const LEADING: f32 = 28.0;
/// The gap between lanes.
pub const GAP: f32 = 12.0;
/// A one-line row's least height (the touch target).
pub const HEIGHT_MD: f32 = 44.0;
/// A row with a subtitle: its least height.
pub const HEIGHT_LG: f32 = 60.0;
/// The value lane's least width, beside the title: a value narrower than
/// this keeps only its own width ([`value_least`]).
pub const VALUE_MIN: f32 = 56.0;
/// A side action's touch target ([`Row::side_action`]).
pub const SIDE: f32 = 44.0;
/// The lane a side action keeps in the row: its target less a gap, the
/// target reaching half a gap into the space on each side (the symbol is
/// far narrower), so a long title keeps its room.
const SIDE_LANE: f32 = SIDE - GAP;
/// Past this share of the content lane, the title puts the value under it.
pub const TITLE_SHARE: f32 = 0.6;

/// What a row's title is drawn in when the row is an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowTone {
    Default,
    /// The accent: Reconnect, a positive action.
    Primary,
    /// Delete…, Disconnect.
    Destructive,
}

enum Accessory {
    None,
    Chevron,
    Check(Rc<dyn Fn() -> bool>),
    Disclosure(Rc<dyn Fn() -> bool>),
    /// Boxed: an element is much larger than the other accessories.
    Element(Box<AnyElement>),
}

/// A row ([`row`]).
pub struct Row {
    title: Rc<dyn Fn() -> SharedString>,
    subtitle: Option<Rc<dyn Fn() -> SharedString>>,
    mono_subtitle: bool,
    wrap_subtitle: bool,
    detail: Option<Rc<dyn Fn() -> SharedString>>,
    /// The words of a badge in the accessory lane, said after the title.
    badge: Option<SharedString>,
    /// A machine's name after the subtitle's words (`claude · ` then
    /// `~/src/api`): SF Mono, shortened in its middle, the words kept.
    subtitle_mono: Option<Rc<dyn Fn() -> SharedString>>,
    value: Option<Rc<dyn Fn() -> SharedString>>,
    value_dot: Option<Status>,
    label: Option<Rc<dyn Fn() -> SharedString>>,
    mono_value: bool,
    whole_value: bool,
    value_tone: Tone,
    leading: Option<AnyElement>,
    reserve_leading: bool,
    accessory: Accessory,
    tone: RowTone,
    heading: bool,
    on_press: Option<Rc<dyn Fn()>>,
    on_long_press: Option<Rc<dyn Fn()>>,
    selected: Rc<dyn Fn() -> bool>,
    disabled: Rc<dyn Fn() -> bool>,
    loading: Rc<dyn Fn() -> bool>,
    stacked: Rc<dyn Fn() -> bool>,
    below: Option<AnyElement>,
    /// A button just before the accessory, over a lane the row keeps for
    /// it ([`Row::side_action`]).
    side: Option<AnyElement>,
    test_id: Option<SharedString>,
    /// What the built row is put in (a swipe tray: B499), keeping its
    /// place on the section (its inset, its leading lane).
    wrap: Option<Box<dyn FnOnce(AnyElement) -> AnyElement>>,
}

/// A row titled `title`.
pub fn row(title: impl Into<SharedString>) -> Row {
    let title: SharedString = title.into();
    Row {
        title: Rc::new(move || title.clone()),
        subtitle: None,
        mono_subtitle: false,
        wrap_subtitle: false,
        detail: None,
        badge: None,
        subtitle_mono: None,
        value: None,
        value_dot: None,
        label: None,
        mono_value: false,
        whole_value: false,
        value_tone: Tone::Muted,
        leading: None,
        reserve_leading: false,
        accessory: Accessory::None,
        tone: RowTone::Default,
        heading: false,
        on_press: None,
        on_long_press: None,
        selected: Rc::new(|| false),
        disabled: Rc::new(|| false),
        loading: Rc::new(|| false),
        stacked: Rc::new(|| false),
        below: None,
        side: None,
        test_id: None,
        wrap: None,
    }
}

impl Row {
    /// The title, following state.
    pub fn title_with(mut self, title: impl Fn() -> SharedString + 'static) -> Self {
        self.title = Rc::new(title);
        self
    }

    pub fn subtitle(self, subtitle: impl Into<SharedString>) -> Self {
        let subtitle: SharedString = subtitle.into();
        self.subtitle_with(move || subtitle.clone())
    }

    pub fn subtitle_with(mut self, subtitle: impl Fn() -> SharedString + 'static) -> Self {
        self.subtitle = Some(Rc::new(subtitle));
        self
    }

    /// The subtitle wraps onto as many lines as it needs, cut nowhere: it
    /// says what the row does, which a shortened line would lose.
    pub fn wrap_subtitle(mut self) -> Self {
        self.wrap_subtitle = true;
        self
    }

    /// The subtitle is what a machine names (`dev@build.example.com`, a
    /// path): SF Mono, shortened in its middle.
    pub fn mono_subtitle(mut self) -> Self {
        self.mono_subtitle = true;
        self
    }

    /// A badge in the accessory lane (a key's algorithm, Secure Enclave),
    /// its words said right after the title: what tells two rows apart for
    /// a screen reader.
    pub fn badge(mut self, badge: Badge) -> Self {
        self.badge = Some(badge.words().clone());
        self.accessory = Accessory::Element(Box::new(badge.into_element()));
        self
    }

    /// A second line under the subtitle (what a key is used for), in the
    /// subtitle's muted ink, shortened at its tail.
    pub fn detail(mut self, detail: impl Fn() -> SharedString + 'static) -> Self {
        self.detail = Some(Rc::new(detail));
        self
    }

    /// A subtitle of words and then a machine's name (§6.1: `claude ·
    /// ~/src/api`): `words` in the system font, kept whole, then `mono` in
    /// SF Mono, shortened in its middle when the two do not fit. Empty
    /// words: the name alone.
    pub fn subtitle_parts(
        mut self,
        words: impl Fn() -> SharedString + 'static,
        mono: impl Fn() -> SharedString + 'static,
    ) -> Self {
        self.subtitle = Some(Rc::new(words));
        self.subtitle_mono = Some(Rc::new(mono));
        self
    }

    pub fn value(self, value: impl Into<SharedString>) -> Self {
        let value: SharedString = value.into();
        self.value_with(move || value.clone())
    }

    pub fn value_with(mut self, value: impl Fn() -> SharedString + 'static) -> Self {
        self.value = Some(Rc::new(value));
        self
    }

    /// The value is what a machine names (a path, a port): SF Mono,
    /// shortened in its middle.
    pub fn mono_value(mut self) -> Self {
        self.mono_value = true;
        self
    }

    /// The value is read whole (a choice's name: "Adaptive"): never
    /// shortened; when it does not fit beside the title, it goes under it.
    pub fn whole_value(mut self) -> Self {
        self.whole_value = true;
        self
    }

    /// The value's ink (muted unless it says a state: `warning` for "2m
    /// waiting").
    pub fn value_tone(mut self, tone: Tone) -> Self {
        self.value_tone = tone;
        self
    }

    /// A status dot before the value (`● 12m` for an agent at work: it
    /// breathes); the value's words carry the state to screen readers.
    pub fn value_dot(mut self, status: Status) -> Self {
        self.value_dot = Some(status);
        self
    }

    /// What screen readers say for the row, in place of its title,
    /// subtitle and value in that order (§6.1: "api, claude, working 12
    /// minutes, ~/src/api").
    pub fn label_with(mut self, label: impl Fn() -> SharedString + 'static) -> Self {
        self.label = Some(Rc::new(label));
        self
    }

    /// Something in the leading lane, centred in its 28 pt.
    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_element());
        self
    }

    /// An SF Symbol in the leading lane, muted.
    pub fn leading_icon(self, name: &'static str) -> Self {
        self.leading(icon(name).tone(Tone::Muted))
    }

    /// A settings tile in the leading lane: the symbol on `muted`, 28 pt,
    /// `radius-sm` (§6.4).
    pub fn leading_tile(self, name: &'static str) -> Self {
        let tile = view()
            .w(px(LEADING))
            .h(px(LEADING))
            .rounded(px(number("radius-sm")))
            .bg(move || token("muted"))
            .items_center()
            .justify_center()
            .child(icon(name));
        self.leading(tile)
    }

    /// Keep the leading lane empty, so the title lines up with rows that
    /// have something there.
    pub fn reserve_leading(mut self) -> Self {
        self.reserve_leading = true;
        self
    }

    /// A chevron: the row opens a page.
    pub fn chevron(mut self) -> Self {
        self.accessory = Accessory::Chevron;
        self
    }

    /// A check while `checked` holds: the row is a choice of one.
    pub fn check(mut self, checked: impl Fn() -> bool + 'static) -> Self {
        let checked: Rc<dyn Fn() -> bool> = Rc::new(checked);
        self.selected = checked.clone();
        self.accessory = Accessory::Check(checked);
        self
    }

    /// A disclosure arrow, turned while `expanded` holds: the row shows or
    /// hides rows under it.
    pub fn disclosure(mut self, expanded: impl Fn() -> bool + 'static) -> Self {
        self.accessory = Accessory::Disclosure(Rc::new(expanded));
        self
    }

    /// A control in the accessory lane (a switch). The control takes its
    /// own presses: a row holding one takes none (an ancestor's press
    /// would fire with the control's, NSH's native views).
    pub fn trailing(mut self, control: impl IntoElement) -> Self {
        self.accessory = Accessory::Element(Box::new(control.into_element()));
        self
    }

    /// A button just before the accessory (a 44 pt icon button: ＋ beside
    /// "All sessions ›"), as UIKit's detail button before a chevron. It
    /// lies over a lane the row keeps for it rather than inside the row,
    /// so the row keeps its own press and the button takes only its own
    /// (an ancestor's press would fire with it).
    pub fn side_action(mut self, button: impl IntoElement) -> Self {
        self.side = Some(button.into_element());
        self
    }

    /// The row heads a group (a host's session over its windows): screen
    /// readers jump to it by headings. It keeps its press.
    pub fn heading(mut self) -> Self {
        self.heading = true;
        self
    }

    pub fn tone(mut self, tone: RowTone) -> Self {
        self.tone = tone;
        self
    }

    /// The row's action: the whole row is one target.
    pub fn on_press(mut self, on_press: impl Fn() + 'static) -> Self {
        self.on_press = Some(Rc::new(on_press));
        self
    }

    /// What a long press on a row with an action does (the same actions
    /// as its press, as a menu would offer them).
    pub fn on_long_press(mut self, on_long_press: impl Fn() + 'static) -> Self {
        self.on_long_press = Some(Rc::new(on_long_press));
        self
    }

    pub fn selected(mut self, selected: impl Fn() -> bool + 'static) -> Self {
        self.selected = Rc::new(selected);
        self
    }

    pub fn disabled(mut self, disabled: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Rc::new(disabled);
        self
    }

    /// While this holds, a spinner takes the accessory lane and the row
    /// takes no press.
    pub fn loading(mut self, loading: impl Fn() -> bool + 'static) -> Self {
        self.loading = Rc::new(loading);
        self
    }

    /// The value goes under the title (a long label, a long translation);
    /// from AX1 on it always does.
    pub fn stacked(mut self, stacked: impl Fn() -> bool + 'static) -> Self {
        self.stacked = Rc::new(stacked);
        self
    }

    /// Something under the title (and the subtitle) in the content lane,
    /// hidden from screen readers (the row says its words): a terminal
    /// theme's eight colors. Its box is `{id}-below` of a row that has an
    /// id.
    pub fn below(mut self, below: impl IntoElement) -> Self {
        self.below = Some(below.into_element());
        self
    }

    /// Its accessibility identifier (a host's menus and tests find the row
    /// by it: `home-pane-<connection>-<pane>`).
    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }

    /// The built row put in what `wrap` makes of it (a swipe tray around
    /// it: B499); the section still places it as this row.
    pub fn wrapped(mut self, wrap: impl FnOnce(AnyElement) -> AnyElement + 'static) -> Self {
        self.wrap = Some(Box::new(wrap));
        self
    }

    /// Where the separator above this row starts: at the title, which is
    /// past the leading lane when the row has (or keeps) one.
    pub fn inset(&self) -> Inset {
        if self.leading.is_some() || self.reserve_leading {
            Inset::Lead
        } else {
            Inset::Content
        }
    }
}

/// Whether the user's text size is an accessibility size (rows stack).
fn accessibility_size() -> bool {
    face::category(font_scale()) >= face::AX1
}

/// A row's line of words at the user's text size: one line, cut, up to the
/// largest standard size; as many as it needs at an accessibility size,
/// as iOS wraps them (B753). `0` is no limit.
fn lines_at_text_size() -> u32 {
    u32::from(!accessibility_size())
}

/// The least the value lane keeps beside the title for a value `natural`
/// points wide (its dot and the dot's gap included): [`VALUE_MIN`], or
/// less for a shorter value. A short value ("1m") kept 56 pt, and the
/// subtitle beside it was cut with room to spare (B750).
pub fn value_least(natural: f32) -> f32 {
    natural.max(0.0).ceil().min(VALUE_MIN)
}

/// The width `words` take on one line in `token` at the user's text
/// size, by the platform's text engine.
fn natural_width(token: TextToken, words: &str) -> f32 {
    let family = match token.design() {
        Design::Monospaced => "ui-monospace",
        _ => "system-ui",
    };
    let font = format!(
        "{} {}px {}",
        token.weight().0,
        token.size(font_scale()),
        family
    );
    measure_text(&font, words).0
}

/// `text` on one line, shortened at its tail (or middle for `mono`).
fn line(token: TextToken, words: Text, mono: bool) -> Text {
    let text = token.apply(words).number_of_lines(1u32);
    if mono {
        text.text_overflow(EllipsizeMode::Middle)
    } else {
        text
    }
}

impl IntoElement for Row {
    fn into_element(mut self) -> AnyElement {
        let wrap = self.wrap.take();
        let element = self.build();
        match wrap {
            Some(wrap) => wrap(element),
            None => element,
        }
    }
}

impl Row {
    fn build(self) -> AnyElement {
        let Row {
            title,
            subtitle,
            mono_subtitle,
            wrap_subtitle,
            detail,
            badge,
            subtitle_mono,
            value,
            value_dot,
            label: said,
            mono_value,
            whole_value,
            value_tone,
            leading,
            reserve_leading,
            accessory,
            tone,
            heading,
            on_press,
            on_long_press,
            selected,
            disabled,
            loading,
            stacked,
            below,
            side: side_action,
            test_id,
            wrap: _,
        } = self;
        let ink = match tone {
            RowTone::Default => "foreground",
            RowTone::Primary => "primary-text",
            RowTone::Destructive => "destructive-text",
        };
        let has_subtitle = subtitle.is_some();
        // What screen readers say for the whole row.
        let label = {
            let (title, subtitle, mono, detail, value) = (
                title.clone(),
                subtitle.clone(),
                subtitle_mono.clone(),
                detail.clone(),
                value.clone(),
            );
            move || -> SharedString {
                if let Some(said) = &said {
                    return said();
                }
                let mut words = vec![title().to_string()];
                words.extend(badge.as_ref().map(ToString::to_string));
                let sub = [
                    subtitle.as_ref().map(|s| s().to_string()),
                    mono.as_ref().map(|m| m().to_string()),
                ];
                let sub: Vec<String> = sub
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.is_empty())
                    .collect();
                if !sub.is_empty() {
                    words.push(sub.join(" "));
                }
                words.extend(detail.as_ref().map(|d| d().to_string()));
                words.extend(value.as_ref().map(|v| v().to_string()));
                words.join(", ").into()
            }
        };
        // The content lane's width as laid out (the lanes less the leading
        // lane), and whether the title and the value fit side by side in
        // it: the title within its share, a whole value beside it.
        let lanes_width = signal(0.0_f32);
        let wraps_title = side_action.is_some();
        let lead = if leading.is_some() || reserve_leading {
            LEADING + GAP
        } else {
            0.0
        };
        let crowded = {
            let (title, value) = (title.clone(), value.clone());
            computed(move || {
                let Some(value) = &value else { return false };
                let room = lanes_width.get() - lead;
                if room <= 0.0 {
                    return false;
                }
                let title = natural_width(TextToken::Body, &title());
                if title > TITLE_SHARE * room {
                    return true;
                }
                let token = if mono_value {
                    TextToken::MonoSubhead
                } else {
                    TextToken::Subhead
                };
                whole_value && title + GAP + natural_width(token, &value()) > room
            })
        };
        // The least the value lane keeps beside the title: its own width
        // when that is less than VALUE_MIN (B750).
        let least = {
            let value = value.clone();
            computed(move || {
                let Some(value) = &value else { return 0.0 };
                let token = if mono_value {
                    TextToken::MonoSubhead
                } else {
                    TextToken::Subhead
                };
                let dot = match value_dot {
                    Some(_) => DotSize::Sm.points() + 6.0,
                    None => 0.0,
                };
                value_least(natural_width(token, &value()) + dot)
            })
        };
        // Side by side, or (stacked, an accessibility text size, crowded)
        // the value under the title.
        let side = {
            let stacked = stacked.clone();
            move || !stacked() && !accessibility_size() && !crowded.get()
        };
        // The value's text, made where it shows: beside the title (its lane,
        // or the title's line in a Mail row) on one line, shortened; under
        // it as many lines as the title takes at the user's text size
        // (B837): from AX1 it wraps as the title does (B753), and below
        // AX1, where a crowded row also puts it there (B750), one line.
        let value_text = value.map(|value| {
            move |under: bool| {
                let value = value.clone();
                let mut words = line(
                    if mono_value {
                        TextToken::MonoSubhead
                    } else {
                        TextToken::Subhead
                    },
                    text(move || value()),
                    mono_value,
                )
                .text_color(move || token(value_tone.token()))
                .tabular_nums();
                if under {
                    words = words.number_of_lines(lines_at_text_size).min_w(px(0.0));
                }
                match value_dot {
                    // The dot beside the words, the pair at the lane's end.
                    Some(status) => view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .min_w(px(0.0))
                        .child(status_dot(status).size(DotSize::Sm))
                        .child(words.flex_shrink(1.0))
                        .into_element(),
                    None => words.into_element(),
                }
            }
        });

        // A row with a subtitle and a value reads as Mail's do (B752): the
        // value on the title's line at its trailing end, the subtitle under
        // them across the whole lane. At an accessibility text size (or
        // stacked) the value goes under the title, which wraps.
        let mail = has_subtitle && value_text.is_some();
        let on_title_line = {
            let stacked = stacked.clone();
            move || !stacked() && !accessibility_size()
        };
        let title_words =
            line(TextToken::Body, text(move || title()), false).text_color(move || token(ink));
        let title_view = if mail {
            let wraps = on_title_line.clone();
            let shown = on_title_line.clone();
            let make = value_text.clone().expect("a mail row has a value");
            view()
                .flex_row()
                .items_start()
                .gap(px(GAP))
                .min_w(px(0.0))
                .child(
                    title_words
                        .number_of_lines(move || u32::from(wraps()))
                        .flex_grow(1.0)
                        .flex_shrink(1.0)
                        .min_w(px(0.0)),
                )
                .child(when(shown, move || {
                    // One title line tall, the value centred in it; it gives
                    // way four times faster than the title, down to its
                    // least width (B317, B750).
                    view()
                        .flex_shrink(4.0)
                        .min_w(move || px(least.get()))
                        .h(move || {
                            px(face::frame(
                                TextToken::Body.size(font_scale()),
                                pixel_ratio(),
                            ))
                        })
                        .justify_center()
                        .items_end()
                        .child(make(false))
                }))
                .into_element()
        } else if wraps_title {
            // Beside a side action the title has less room: it wraps
            // rather than being cut ("Tous les espaces de travail").
            title_words.number_of_lines(0u32).into_element()
        } else {
            title_words
                .number_of_lines(lines_at_text_size)
                .into_element()
        };
        // The content lane: title, the subtitle under it, and (stacked) the
        // value under them.
        let mut content = view().min_w(px(0.0)).gap(px(2.0)).child(title_view);
        match (subtitle, subtitle_mono) {
            (Some(words), Some(mono)) => {
                // The words keep their width; the name shortens in its
                // middle in what is left. Where even the words don't fit
                // (an accessibility text size), they shorten too, at their
                // tail, once the name has given way: never clipped bare.
                let shown = words.clone();
                // At an accessibility text size: the words and the name in
                // one line of text that wraps (B753), nothing cut.
                let (all_words, all_mono) = (words.clone(), mono.clone());
                content = content.child(when(accessibility_size, move || {
                    let (words, mono) = (all_words.clone(), all_mono.clone());
                    TextToken::Subhead
                        .apply(text(move || {
                            let words = words();
                            if words.is_empty() {
                                mono().to_string()
                            } else {
                                format!("{words} · {}", mono())
                            }
                        }))
                        .text_color(move || token("muted-foreground"))
                }));
                content = content.child(when(
                    || !accessibility_size(),
                    move || {
                        let (shown, words, mono) = (shown.clone(), words.clone(), mono.clone());
                        view()
                            .flex_row()
                            .min_w(px(0.0))
                            .child(when(
                                move || !shown().is_empty(),
                                move || {
                                    let words = words.clone();
                                    line(
                                        TextToken::Subhead,
                                        text(move || format!("{} · ", words())),
                                        false,
                                    )
                                    .flex_shrink(1.0)
                                    .min_w(px(0.0))
                                    .text_color(move || token("muted-foreground"))
                                },
                            ))
                            .child(
                                line(TextToken::MonoSubhead, text(move || mono()), true)
                                    // Gives way long before the words do.
                                    .flex_shrink(1000.0)
                                    .min_w(px(0.0))
                                    .text_color(move || token("muted-foreground")),
                            )
                    },
                ));
            }
            (Some(subtitle), None) => {
                let style = if mono_subtitle {
                    TextToken::MonoSubhead
                } else {
                    TextToken::Subhead
                };
                let words = text(move || subtitle());
                let words = if wrap_subtitle {
                    style.apply(words)
                } else {
                    line(style, words, mono_subtitle).number_of_lines(lines_at_text_size)
                };
                content = content.child(words.text_color(move || token("muted-foreground")));
            }
            _ => {}
        }
        if let Some(detail) = detail {
            content = content.child(
                line(TextToken::Subhead, text(move || detail()), false)
                    .number_of_lines(lines_at_text_size)
                    .text_color(move || token("muted-foreground")),
            );
        }
        if let Some(below) = below {
            let mut under = view()
                .pt(px(4.0))
                .accessibility_elements_hidden(true)
                .child(below);
            if let Some(id) = &test_id {
                under = under.test_id(&format!("{id}-below"));
            }
            content = content.child(under);
        }
        if let Some(make) = value_text.clone() {
            if mail {
                let under = on_title_line.clone();
                content = content.child(when(move || !under(), move || make(true)));
            } else {
                let side = side.clone();
                content = content.child(when(move || !side(), move || make(true)));
            }
        }

        let mut lanes = view()
            .flex_1()
            .min_w(px(0.0))
            .flex_row()
            .items_center()
            .gap(px(GAP))
            .on_layout(move |frame: &Bounds<Points>| {
                if lanes_width.get_untracked() != frame.size.width.0 {
                    lanes_width.set(frame.size.width.0);
                }
            });
        if leading.is_some() || reserve_leading {
            let mut lane = view()
                .w(px(LEADING))
                .flex_shrink_0()
                .items_center()
                .justify_center();
            if let Some(leading) = leading {
                lane = lane.child(leading);
            }
            lanes = lanes.child(lane);
        }
        match value_text {
            None => lanes = lanes.child(content.flex_1()),
            // The value is in the content's title line.
            Some(_) if mail => lanes = lanes.child(content.flex_1()),
            Some(make) => {
                // Both start at their own width: the value takes what is
                // left, and when the two do not fit it gives way four times
                // faster than the title, down to its least width, its text
                // shortened inside its lane (B317).
                let side = side.clone();
                let beside = side.clone();
                lanes = lanes.child(content.flex_shrink(1.0)).child(
                    view()
                        .flex_grow(1.0)
                        .flex_shrink(4.0)
                        .min_w(move || px(if beside() { least.get() } else { 0.0 }))
                        .items_end()
                        .child(when(side, move || {
                            view().max_w(relative(1.0)).items_end().child(make(false))
                        })),
                );
            }
        }

        // A disclosure row says whether it is open.
        let expanded = match &accessory {
            Accessory::Disclosure(expanded) => Some(expanded.clone()),
            _ => None,
        };
        // A control (a switch) keeps to the title's first line when the
        // title wraps (B753); the chevron and the rest stay centred.
        let control = matches!(accessory, Accessory::Element(_));
        let lane = match accessory {
            Accessory::None => None,
            Accessory::Chevron => Some(
                icon("chevron.right")
                    .size(IconSize::Sm)
                    .tone(Tone::Subtle)
                    .into_element(),
            ),
            // The check comes and goes over `base` (at once while motion is
            // reduced: the token's time is then 0).
            Accessory::Check(checked) => Some(
                view()
                    .opacity(move || if checked() { 1.0_f32 } else { 0.0 })
                    .transition_opacity()
                    .duration(Duration::Base.ms())
                    .ease_in_out()
                    .child(icon("checkmark").size(IconSize::Sm).tone(Tone::Primary))
                    .into_element(),
            ),
            Accessory::Disclosure(expanded) => Some(
                view()
                    .rotate(move || {
                        Radians(if expanded() {
                            std::f32::consts::PI
                        } else {
                            0.0
                        })
                    })
                    .transition_transform()
                    .duration(Duration::Base.ms())
                    .ease_in_out()
                    .child(icon("chevron.down").size(IconSize::Sm).tone(Tone::Subtle))
                    .into_element(),
            ),
            Accessory::Element(element) => Some(*element),
        };
        // How wide the accessory is drawn: where a side action lies, from
        // the trailing edge.
        let slot_width = signal(16.0_f32);
        // While loading, a spinner lies over the accessory, which fades out
        // (it is never made again: a control keeps its state).
        let accessory = {
            let loading_now = loading.clone();
            let mut slot = view()
                .relative()
                .flex_shrink_0()
                .min_w(px(16.0))
                .items_center()
                .justify_center()
                .on_layout(move |b: &Bounds<Points>| {
                    if slot_width.get_untracked() != b.size.width.0 {
                        slot_width.set(b.size.width.0);
                    }
                });
            // At an accessibility text size, at the top, as tall as the
            // title's first line, the control centred in it. Read as the
            // row is made: a size changed while it shows takes effect the
            // next time the row is made (NSH's alignment isn't reactive).
            if control && accessibility_size() {
                slot = slot.self_start().min_h(px(face::frame(
                    TextToken::Body.size(font_scale()),
                    pixel_ratio(),
                )));
            }
            if let Some(lane) = lane {
                let loading = loading.clone();
                slot = slot.child(
                    view()
                        .opacity(move || if loading() { 0.0_f32 } else { 1.0 })
                        .child(lane),
                );
            }
            let slot = slot.child(when(
                move || loading_now(),
                || {
                    view()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .child(spinner(SpinnerSize::Sm))
                },
            ));
            if side_action.is_some() {
                // The side action's lane, kept empty: the button lies over
                // it from outside the row.
                view()
                    .flex_row()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(GAP))
                    .child(
                        view()
                            .w(px(SIDE_LANE))
                            .h(px(1.0))
                            .accessibility_elements_hidden(true),
                    )
                    .child(slot)
            } else {
                slot
            }
        };

        let height = if has_subtitle { HEIGHT_LG } else { HEIGHT_MD };
        let dim = {
            let disabled = disabled.clone();
            move || if disabled() { 0.4_f32 } else { 1.0 }
        };
        let fill = {
            let selected = selected.clone();
            move || {
                if selected() {
                    token("selection")
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            }
        };
        let built = match (on_press, expanded) {
            (Some(on_press), expanded) if expanded.is_some() || on_long_press.is_some() => {
                // Its own target: nsh_base's button cannot say expanded,
                // nor take a long press.
                let inert = {
                    let (disabled, loading) = (disabled.clone(), loading.clone());
                    Rc::new(move || disabled() || loading())
                };
                let state = inert.clone();
                let picked = selected.clone();
                let mut control = view()
                    .role(if heading {
                        AccessibilityRole::Header
                    } else {
                        AccessibilityRole::Button
                    })
                    .disabled(move || inert())
                    .label(label)
                    .accessibility_state(move || AccessibilityState {
                        disabled: state(),
                        selected: picked(),
                        expanded: expanded.as_ref().map(|expanded| expanded()),
                        ..AccessibilityState::default()
                    })
                    .on_press(move || on_press())
                    .flex_row()
                    .items_center()
                    .gap(px(GAP))
                    .min_h(px(height))
                    .px(px(PADDING_X))
                    .py(px(PADDING_Y))
                    .opacity(dim)
                    .bg(fill)
                    .transition_colors()
                    .duration(Duration::Instant.ms())
                    .active(|s| s.bg(move || token("accent")))
                    .focus_visible(|s| s.bg(move || token("accent")));
                if let Some(long) = on_long_press {
                    control = control.on_long_press(move || long());
                }
                if let Some(id) = &test_id {
                    control = control.test_id(id);
                }
                control.child(lanes).child(accessory).into_element()
            }
            (Some(on_press), _) => {
                let inert = {
                    let (disabled, loading) = (disabled.clone(), loading.clone());
                    move || disabled() || loading()
                };
                let mut control = nsh_base::button()
                    .role(heading.then_some(AccessibilityRole::Header))
                    .disabled(inert)
                    .selected(move || selected())
                    .accessibility_label(move || Some(label()))
                    .on_press(move || on_press())
                    .flex_row()
                    .items_center()
                    .gap(px(GAP))
                    .min_h(px(height))
                    .px(px(PADDING_X))
                    .py(px(PADDING_Y))
                    .opacity(dim)
                    .bg(fill)
                    .transition_colors()
                    .duration(Duration::Instant.ms())
                    .active(|s| s.bg(move || token("accent")))
                    .focus_visible(|s| s.bg(move || token("accent")));
                if let Some(id) = &test_id {
                    control = control.test_id(id);
                }
                control.child(lanes).child(accessory).into_element()
            }
            (None, _) => {
                let mut plain = view()
                    .flex_row()
                    .items_center()
                    .gap(px(GAP))
                    .min_h(px(height))
                    .px(px(PADDING_X))
                    .py(px(PADDING_Y))
                    .opacity(dim)
                    .bg(fill);
                if let Some(id) = &test_id {
                    plain = plain.test_id(id);
                }
                plain.child(lanes).child(accessory).into_element()
            }
        };
        match side_action {
            None => built,
            Some(side) => view()
                .relative()
                .child(built)
                .child(
                    view()
                        .absolute()
                        .top(px(0.0))
                        .bottom(px(0.0))
                        .right(move || px(PADDING_X + slot_width.get() + GAP / 2.0))
                        .w(px(SIDE))
                        .items_center()
                        .justify_center()
                        .child(side),
                )
                .into_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_value_keeps_only_its_own_width() {
        // "1m" needs about 18 pt: the rest goes to the subtitle (B750).
        assert_eq!(value_least(17.3), 18.0);
        assert_eq!(value_least(0.0), 0.0);
        // A long one keeps VALUE_MIN and shortens inside it (B317).
        assert_eq!(value_least(120.0), VALUE_MIN);
        assert_eq!(value_least(VALUE_MIN), VALUE_MIN);
    }
}
