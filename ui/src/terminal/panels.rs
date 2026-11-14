//! The panels that ride above the shortcut bar, in the column (the pane
//! gives them room): voice input (`DictationOverlayView`), an image upload
//! (`ImageAttachmentOverlayView`), a host file on its way to the preview
//! (`RemotePreviewHUD`). Each composes first and commits only on the
//! person's word: nothing reaches the terminal until Insert or Send.
//!
//! A panel (`docs/nsh/UI-DESIGN.md` §4.19, §6.3) floats 8 pt above the bar
//! and 8 in from the sides: `popover`, `radius-lg`, `shadow-lg` (a hairline
//! of `border-strong` in light), a header, its body and a row of actions.
//! It comes in rising 16 pt from the bar, fading in and growing from 0.96
//! (`slow`), and goes fading out (`base`, easing in), what it said kept
//! while it goes; under reduced motion it comes and goes at once. Its text
//! stops growing at extra large, as the rest of the terminal's chrome does.

use std::rc::Rc;

use nsh_core::prelude::*;
use serde_json::json;

use super::{
    Ctx, Intent, PanelAction, X_LARGE,
    model::{
        Attachment, AttachmentPhase, Dictation, DictationPhase, Preview, PreviewPhase, Thumbnail,
    },
    symbol,
};
use crate::{
    face,
    ui::{
        button::{Size, Variant, button},
        feedback::{self, SpinnerSize, spinner},
        icon::{IconSize, Tone, icon},
        motion::{Duration, ENTER, LEAVE, distance, scale},
        status::{BadgeVariant, badge},
        tokens::{SHADOW_LG, number, token},
        typography::TextToken,
    },
};

/// A thumbnail's side in the image panel.
const THUMBNAIL: f32 = 56.0;

/// The panels, stacked above the shortcut bar.
pub(crate) fn panels(ctx: Ctx) -> AnyElement {
    let level = ctx.level;
    view()
        .w_full()
        .flex_shrink_0()
        .child(panel(ctx.dictation, "dictation-overlay", {
            let ctx = ctx.clone();
            move |d: Dictation| dictation_body(&d, level, ctx.clone())
        }))
        .child(panel(ctx.attachment, "image-attachment-overlay", {
            let ctx = ctx.clone();
            move |a: Attachment| attachment_body(&a, ctx.clone())
        }))
        .child(panel(ctx.preview, "remote-preview-hud", {
            let ctx = ctx.clone();
            move |p: Preview| preview_body(&p, ctx.clone())
        }))
        .into_element()
}

/// A panel coming in.
fn panel_enter() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(Duration::Slow.nominal_ms()).easing(ENTER))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [distance::ENTER, 0.0])
        .values(MotionChannel::Scale, [scale::ENTER, 1.0])
}

/// A panel going: it fades out where it is.
fn panel_leave() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(Duration::Base.nominal_ms()).easing(LEAVE))
        .values(MotionChannel::Opacity, [1.0, 0.0])
}

/// The panel `id` while the host posts one (`source`), drawn by `body`
/// from what it last posted: mounted with its enter motion, kept as it was
/// while its leaving plays (NSH has no exit transition), then gone.
fn panel<T>(
    source: Signal<Option<T>>,
    id: &'static str,
    body: impl Fn(T) -> AnyElement + 'static,
) -> AnyElement
where
    T: Clone + PartialEq + 'static,
{
    let reduced = reduce_motion();
    // What the panel says: the host's, or what it last said while it goes.
    let last = stored(None::<T>);
    let content = computed(move || {
        let now = source.get();
        if now.is_some() {
            last.replace(now.clone());
            now
        } else {
            last.with(|l| l.clone())
        }
    });
    let shown = computed(move || source.with(Option::is_some));
    let mounted = signal(shown.get_untracked());
    // The motion of the panel mounted now, to play its leaving on.
    let moving = stored(None::<Motion>);
    let was = stored(shown.get_untracked());
    let pending = stored(None::<Spawned<()>>);
    effect(move || {
        let show = shown.get();
        if was.replace(show) == show {
            return;
        }
        if let Some(timer) = pending.replace(None) {
            timer.cancel();
        }
        let playing = moving.get().filter(|_| !reduced && mounted.get_untracked());
        match (show, playing) {
            // Back while it faded: in again from where it is.
            (true, Some(m)) => drop(m.play(panel_enter())),
            (true, None) => mounted.set(true),
            (false, Some(m)) => {
                drop(m.play(panel_leave()));
                pending.replace(Some(set_timeout(
                    std::time::Duration::from_millis(Duration::Base.nominal_ms() as u64),
                    move || {
                        mounted.set(false);
                        last.replace(None);
                    },
                )));
            }
            (false, None) => {
                mounted.set(false);
                last.replace(None);
            }
        }
    });
    let body = Rc::new(body);
    when(
        move || mounted.get(),
        move || {
            let body = body.clone();
            let mut card = card(id).child(dynamic(move || {
                let value = content.get();
                let body = body.clone();
                untracked(move || match value {
                    Some(value) => body(value),
                    None => view().into_element(),
                })
            }));
            if !reduced {
                let m = motion().entering(panel_enter());
                moving.replace(Some(m));
                card = card.bind_motion(&m);
            }
            card
        },
    )
    .into_element()
}

/// A panel's card.
fn card(id: &'static str) -> View {
    view()
        .mx(px(8.0))
        .mb(px(8.0))
        .p(px(12.0))
        .gap(px(12.0))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("popover"))
        .border(px(1.0))
        .border_color(move || {
            let stroke = token("border-strong");
            AdaptiveColor {
                light: stroke.light,
                dark: AdaptiveColor::TRANSPARENT.dark,
            }
        })
        .shadow([BoxShadow {
            offset_x: px(0.0),
            offset_y: px(SHADOW_LG.y),
            blur: px(SHADOW_LG.blur),
            spread: px(0.0),
            color: color(token("shadow-lg")).into(),
            inset: false,
        }])
        .test_id(id)
}

/// `content` in the text style `style` and the color token `tone`, its
/// size following the user's no further than extra large.
fn words(content: impl IntoReactive<SharedString>, style: TextToken, tone: &'static str) -> Text {
    let text_style = style.style();
    let text = text(content)
        .text_size(move || {
            px(face::point_size_clamped(
                text_style,
                font_scale(),
                0,
                X_LARGE,
            ))
        })
        .line_height(face::line_height())
        .font_weight(style.weight())
        .allow_font_scaling(false)
        .text_color(move || token(tone));
    match style.design() {
        face::Design::Monospaced => text.font_family("monospace"),
        _ => text,
    }
}

/// A header's status word: mono caption, tracked, one line.
fn status(content: String, tone: &'static str) -> Text {
    words(content, TextToken::MonoCaption, tone)
        .letter_spacing(px(0.6))
        .number_of_lines(1u32)
        .flex_shrink(1.0)
        .min_w(px(0.0))
}

/// What a panel says in a sentence: the panel's text, wrapping.
fn paragraph(content: String, tone: &'static str) -> Text {
    words(content, TextToken::Subhead, tone)
}

/// A header: what is going on (a mark and a word), then what trails it.
fn header(mark: AnyElement, word: Text) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .min_h(px(20.0))
        .child(mark)
        .child(word)
        .child(view().flex_1().min_w(px(0.0)))
}

/// An icon in a header, in a tone.
fn mark(name: &'static str, tone: Tone) -> AnyElement {
    icon(name).size(IconSize::Sm).tone(tone).into_element()
}

/// A row of actions.
fn actions() -> View {
    view().flex_row().items_center().gap(px(8.0))
}

/// A quantity's words (a size), quiet, on one line.
fn quantity(content: String) -> Text {
    words(content, TextToken::MonoCaption, "muted-foreground").number_of_lines(1u32)
}

// --- Voice -----------------------------------------------------------------------

/// The voice panel: what the engine is doing, which engine, what it heard
/// (the settled words, then its live guess, quieter), and Insert / Send.
fn dictation_body(d: &Dictation, level: Signal<f32>, ctx: Ctx) -> AnyElement {
    view()
        .gap(px(12.0))
        .child(dictation_header(d, level))
        .child(dictation_content(d))
        .child(dictation_actions(d, ctx))
        .into_element()
}

fn dictation_header(d: &Dictation, level: Signal<f32>) -> AnyElement {
    let (mark, word) = match &d.phase {
        DictationPhase::Starting => (
            spinner(SpinnerSize::Sm),
            status(t!("STARTING…"), "muted-foreground"),
        ),
        DictationPhase::Downloading { progress } => (
            spinner(SpinnerSize::Sm),
            status(
                t!(
                    "DOWNLOADING SPEECH MODEL %lld%%",
                    arg1 = ((progress * 100.0) as i64).to_string()
                ),
                "muted-foreground",
            ),
        ),
        DictationPhase::Loading => (
            spinner(SpinnerSize::Sm),
            status(t!("LOADING SPEECH MODEL…"), "muted-foreground"),
        ),
        DictationPhase::Listening => (
            mark("mic.fill", Tone::Primary),
            status(t!("LISTENING"), "primary-text"),
        ),
        DictationPhase::Finishing => (
            spinner(SpinnerSize::Sm),
            status(t!("FINISHING…"), "muted-foreground"),
        ),
        DictationPhase::Interrupted => (
            mark("pause.circle.fill", Tone::Warning),
            status(t!("INTERRUPTED"), "warning-text"),
        ),
        DictationPhase::Failed { .. } => (
            mark("mic.slash.fill", Tone::Warning),
            status(t!("VOICE INPUT"), "warning-text"),
        ),
    };
    let mut row = header(mark, word);
    if d.phase == DictationPhase::Listening {
        row = row.child(level_meter(level));
    }
    if !d.engine.is_empty() && !matches!(d.phase, DictationPhase::Failed { .. }) {
        row = row.child(
            view()
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .test_id("dictation-engine-label")
                .child(badge(d.engine.clone()).variant(BadgeVariant::Neutral)),
        );
    }
    row.into_element()
}

/// Five bars rising with the mic's loudness, center-weighted
/// (`DictationLevelMeter`); still, at their middle height, under reduced
/// motion.
fn level_meter(level: Signal<f32>) -> AnyElement {
    const WEIGHTS: [f32; 5] = [0.45, 0.75, 1.0, 0.75, 0.5];
    let reduced = reduce_motion();
    view()
        .h(px(18.0))
        .flex_row()
        .items_center()
        .gap(px(2.5))
        .flex_shrink_0()
        .accessibility_elements_hidden(true)
        .test_id("dictation-level")
        .children(WEIGHTS.map(|w| {
            view()
                .w(px(3.0))
                .h(move || {
                    let loudness = if reduced { 0.5 } else { level.get() };
                    px(3.0 + 13.0 * w * loudness)
                })
                .rounded_full()
                .bg(move || token("primary"))
        }))
        .into_element()
}

fn dictation_content(d: &Dictation) -> AnyElement {
    if let DictationPhase::Failed { message, .. } = &d.phase {
        return paragraph(message.clone(), "foreground").into_element();
    }
    if d.finalized.is_empty() && d.volatile.is_empty() {
        let content = if d.phase == DictationPhase::Interrupted {
            t!("Another app took the microphone before anything was heard.")
        } else {
            t!(
                "Speak — the text lands here first. Insert types it; Send types it and presses Return."
            )
        };
        return paragraph(content, "muted-foreground").into_element();
    }
    // The settled words, then the engine's guess, quieter; hugging the text
    // up to about six lines, then scrolling, pinned to the newest words.
    let spoken = words("", TextToken::Subhead, "foreground")
        .child(text(d.finalized.clone()).text_color(move || token("foreground")))
        .child(if d.finalized.is_empty() { "" } else { " " })
        .child(text(d.volatile.clone()).text_color(move || token("muted-foreground")));
    scroll()
        .max_h(px(132.0))
        .scrollbar(false)
        .test_id("dictation-transcript")
        .child(spoken)
        .into_element()
}

fn dictation_actions(d: &Dictation, ctx: Ctx) -> AnyElement {
    let ask = |action: PanelAction| {
        let ctx = ctx.clone();
        move || ctx.ask(Intent::Dictation(action))
    };
    if let DictationPhase::Failed { settings, .. } = &d.phase {
        let mut row = actions().child(view().flex_1()).child(
            button(t!("Dismiss"))
                .variant(Variant::Ghost)
                .size(Size::Sm)
                .on_press(ask(PanelAction::Cancel)),
        );
        if *settings {
            row = row.child(
                button(t!("Open Settings"))
                    .variant(Variant::Primary)
                    .size(Size::Sm)
                    .on_press(ask(PanelAction::Settings)),
            );
        }
        return row.into_element();
    }
    let idle = !d.insertable;
    actions()
        .child(
            button(t!("Cancel"))
                .variant(Variant::Ghost)
                .size(Size::Sm)
                .on_press(ask(PanelAction::Cancel)),
        )
        .child(view().flex_1())
        .child(
            button(t!("Insert"))
                .variant(Variant::Secondary)
                .size(Size::Sm)
                .icon("text.insert")
                .disabled(move || idle)
                .test_id("dictation-insert")
                .on_press(ask(PanelAction::Insert)),
        )
        .child(
            button(t!("Send"))
                .variant(Variant::Primary)
                .size(Size::Sm)
                .icon("return")
                .disabled(move || idle)
                .test_id("dictation-send")
                .on_press(ask(PanelAction::Send)),
        )
        .into_element()
}

// --- Images ----------------------------------------------------------------------

/// The image panel: the upload's progress, the images (each waiting on
/// its upload, then with the number it can be inserted by again), Insert
/// / Send.
fn attachment_body(a: &Attachment, ctx: Ctx) -> AnyElement {
    view()
        .gap(px(12.0))
        .child(attachment_header(a))
        .child(attachment_content(a))
        .child(attachment_actions(a, ctx))
        .into_element()
}

fn attachment_header(a: &Attachment) -> AnyElement {
    let (mark, word) = match &a.phase {
        AttachmentPhase::Processing => (
            spinner(SpinnerSize::Sm),
            status(t!("PREPARING…"), "muted-foreground"),
        ),
        AttachmentPhase::Uploading { fraction } => (
            spinner(SpinnerSize::Sm),
            status(
                t!(
                    "UPLOADING %lld%%",
                    arg1 = ((fraction * 100.0) as i64).to_string()
                ),
                "primary-text",
            ),
        ),
        AttachmentPhase::Ready => (
            mark("checkmark.circle.fill", Tone::Primary),
            status(t!("READY TO INSERT"), "primary-text"),
        ),
        AttachmentPhase::Failed { .. } => (
            mark("exclamationmark.triangle.fill", Tone::Warning),
            status(t!("UPLOAD FAILED"), "warning-text"),
        ),
    };
    let mut row = header(mark, word);
    if let Some(size) = &a.size {
        row = row.child(quantity(size.clone()));
    }
    row.into_element()
}

fn attachment_content(a: &Attachment) -> AnyElement {
    if let AttachmentPhase::Failed { message } = &a.phase {
        return paragraph(message.clone(), "foreground").into_element();
    }
    if a.images.is_empty() {
        return paragraph(t!("Reading the selected images…"), "muted-foreground").into_element();
    }
    scroll()
        .horizontal()
        .scrollbar(false)
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .children(a.images.iter().map(thumbnail)),
        )
        .into_element()
}

/// One image: the host's thumbnail (`moshpit/thumbnail`), `radius-sm`
/// (the panel's 20 less its 12 inset); dimmed under a spinner until it
/// lands, then its session number.
fn thumbnail(image: &Thumbnail) -> AnyElement {
    let mut cell = view()
        .relative()
        .w(px(THUMBNAIL))
        .h(px(THUMBNAIL))
        .flex_shrink_0()
        .rounded(px(number("radius-sm")))
        .border_curve(BorderCurve::Continuous)
        .overflow_hidden()
        .bg(move || token("muted"))
        .child(
            native("moshpit/thumbnail")
                .absolute()
                .inset_0()
                .prop("image", json!(image.id)),
        );
    if !image.landed {
        cell = cell.child(
            view()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .bg(move || token("scrim"))
                .child(spinner(SpinnerSize::Sm)),
        );
    }
    match image.number {
        Some(number) if image.landed => {
            cell = cell.child(
                view()
                    .absolute()
                    .left(px(4.0))
                    .bottom(px(4.0))
                    .child(badge(format!("#{number}")).mono()),
            );
        }
        _ if image.landed => {
            cell = cell.child(
                view()
                    .absolute()
                    .right(px(4.0))
                    .bottom(px(4.0))
                    .child(mark("checkmark.circle.fill", Tone::Success)),
            );
        }
        _ => {}
    }
    cell.into_element()
}

fn attachment_actions(a: &Attachment, ctx: Ctx) -> AnyElement {
    let ask = |action: PanelAction| {
        let ctx = ctx.clone();
        move || ctx.ask(Intent::Attachment(action))
    };
    if matches!(a.phase, AttachmentPhase::Failed { .. }) {
        return actions()
            .child(
                button(t!("Dismiss"))
                    .variant(Variant::Ghost)
                    .size(Size::Sm)
                    .on_press(ask(PanelAction::Cancel)),
            )
            .child(view().flex_1())
            .child(
                button(t!("Retry"))
                    .variant(Variant::Primary)
                    .size(Size::Sm)
                    .icon("arrow.clockwise")
                    .test_id("image-attach-retry")
                    .on_press(ask(PanelAction::Retry)),
            )
            .into_element();
    }
    let waiting = a.phase != AttachmentPhase::Ready;
    actions()
        .child(
            button(t!("Cancel"))
                .variant(Variant::Ghost)
                .size(Size::Sm)
                .on_press(ask(PanelAction::Cancel)),
        )
        .child(view().flex_1())
        .child(
            button(t!("Insert"))
                .variant(Variant::Secondary)
                .size(Size::Sm)
                .icon("text.insert")
                .disabled(move || waiting)
                .test_id("image-attach-insert")
                .on_press(ask(PanelAction::Insert)),
        )
        .child(
            button(t!("Send"))
                .variant(Variant::Primary)
                .size(Size::Sm)
                .icon("return")
                .disabled(move || waiting)
                .test_id("image-attach-send")
                .on_press(ask(PanelAction::Send)),
        )
        .into_element()
}

// --- A host file's preview --------------------------------------------------------

/// The preview panel: the file on its way, its progress and size (a large
/// one asks first).
fn preview_body(p: &Preview, ctx: Ctx) -> AnyElement {
    let (mark, word) = match &p.phase {
        PreviewPhase::Opening => (
            spinner(SpinnerSize::Sm),
            status(t!("OPENING…"), "muted-foreground"),
        ),
        PreviewPhase::Downloading { fraction, .. } => (
            spinner(SpinnerSize::Sm),
            status(
                t!(
                    "DOWNLOADING %lld%%",
                    arg1 = ((fraction * 100.0) as i64).to_string()
                ),
                "primary-text",
            ),
        ),
        PreviewPhase::Confirm { .. } => (
            mark("arrow.down.circle", Tone::Primary),
            status(t!("LARGE FILE"), "primary-text"),
        ),
        PreviewPhase::Failed { .. } => (
            mark("exclamationmark.triangle.fill", Tone::Warning),
            status(t!("CAN'T OPEN"), "warning-text"),
        ),
    };
    let mut top = header(mark, word);
    if let PreviewPhase::Downloading {
        size: Some(size), ..
    } = &p.phase
    {
        top = top.child(quantity(size.clone()));
    }
    let kind = p.symbol.clone();
    let file = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(
            view()
                .w(move || px(IconSize::Md.points_now()))
                .items_center()
                .flex_shrink_0()
                .accessibility_elements_hidden(true)
                .child(dynamic(move || {
                    let tint = Rgba::from(color(token("muted-foreground")));
                    let kind = kind.clone();
                    let points = IconSize::Md.points_now();
                    untracked(move || {
                        symbol(&kind, points, IconSize::Md.weight(), tint).into_element()
                    })
                })),
        )
        .child(
            words(p.name.clone(), TextToken::Subhead, "foreground")
                .number_of_lines(1u32)
                .text_overflow(nsh_core::EllipsizeMode::Middle)
                .flex_shrink(1.0)
                .min_w(px(0.0)),
        );
    let mut body = view().gap(px(12.0)).child(top).child(file);
    match &p.phase {
        PreviewPhase::Downloading { fraction, .. } => {
            let fraction = fraction.clamp(0.0, 1.0);
            body = body.child(view().w_full().test_id("remote-preview-progress").child(
                feedback::progress(
                    move || fraction,
                    move || {
                        t!(
                            "DOWNLOADING %lld%%",
                            arg1 = ((fraction * 100.0) as i64).to_string()
                        )
                    },
                ),
            ));
        }
        PreviewPhase::Failed { message } => {
            body = body.child(paragraph(message.clone(), "foreground"));
        }
        PreviewPhase::Confirm { size } => {
            body = body.child(paragraph(
                t!(
                    "This file is %@. Download it to preview?",
                    arg1 = size.clone()
                ),
                "foreground",
            ));
        }
        PreviewPhase::Opening => {}
    }
    let ask = |action: PanelAction| {
        let ctx = ctx.clone();
        move || ctx.ask(Intent::Preview(action))
    };
    let cancel = || {
        button(t!("Cancel"))
            .variant(Variant::Ghost)
            .size(Size::Sm)
            .on_press(ask(PanelAction::Cancel))
    };
    let row = actions().child(view().flex_1());
    let row = match &p.phase {
        PreviewPhase::Confirm { .. } => row.child(cancel()).child(
            button(t!("Download"))
                .variant(Variant::Primary)
                .size(Size::Sm)
                .on_press(ask(PanelAction::Confirm)),
        ),
        PreviewPhase::Failed { .. } => row.child(
            button(t!("OK"))
                .variant(Variant::Secondary)
                .size(Size::Sm)
                .on_press(ask(PanelAction::Cancel)),
        ),
        _ => row.child(cancel()),
    };
    body.child(row).into_element()
}
