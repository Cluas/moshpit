//! The shortcut bar (`ShortcutBarView`), riding the keyboard.
//!
//! The screen's column ends where the keyboard starts
//! ([`Styled::mb_keyboard`] on it, in the commit that hears the keyboard,
//! on its curve), so the bar sits on the keyboard as it moves; with the
//! keyboard down the bar keeps the home indicator's inset under it. Spike
//! G5 measured this landing on the keyboard 8–11 ms ahead of it, where the
//! SwiftUI bar trails by 18–22 ms through some 36 sizes.
//!
//! The chips are the person's toolbar from the shortcut store (the host
//! sends it): a horizontal row the bar scrolls natively, with the keyboard
//! toggle pinned at the trailing edge. A key repeats while it is held, as
//! a hardware key does: a tap sends once, a press held 0.3 s sends and then
//! repeats every 70 ms until the finger lifts or the scroll takes the
//! touch (`KeyRepeatGesture`). The joystick and the scroll thumb read a
//! push: with the keyboard down they engage only once the finger has
//! settled (a swipe to the home screen beginning on the bar must not type
//! an arrow: `StickGesture`).

use std::{rc::Rc, time::Duration as Wait};

use nsh_core::{AccessibilityActionDef, prelude::*};
use serde_json::json;

use super::{
    Ctx, Dir, Intent,
    model::{Bar, Chip, ChipKind},
};
use crate::ui::{
    feedback::touch_target,
    icon::{IconSize, Tone, icon},
    key::{KEY_HEIGHT, KEY_MIN_WIDTH, KeyVariant, content_key, key, symbol_key},
    motion::{Duration, MOVE},
    tokens::{number, over, token},
};

/// How long a press is held before a key repeats, and how often it does.
const HOLD: Wait = Wait::from_millis(300);
const REPEAT: Wait = Wait::from_millis(70);
/// The joystick: travel before a push is a direction, and how many repeat
/// ticks pass before a held direction repeats (`JoystickRepeater`).
const STICK_THRESHOLD: f32 = 7.0;
const SCROLL_THRESHOLD: f32 = 8.0;
const STICK_DELAY_TICKS: u32 = 5;
/// The settle a push needs with the keyboard down, and how far the finger
/// may drift meanwhile (`StickGesture`).
const ENGAGE_DELAY: Wait = Wait::from_millis(50);
const ENGAGE_SLOP: f32 = 8.0;
/// How far the edge fades reach into the row.
const FADE: f32 = 20.0;
/// The keys' spacing, and the row's inset from the bar's leading edge.
const GAP: f32 = 6.0;
const INSET: f32 = 8.0;
/// One arrow of the cluster: the default bar's third slot fits a 375 pt
/// phone with four of them.
const ARROW_WIDTH: f32 = 32.0;

/// The bar's height above the home indicator's inset: a key (36) and 8 pt
/// over and under it, 38 in iPad landscape (`compact`, today's trim).
pub(crate) fn height(compact: bool) -> f32 {
    if compact { 38.0 } else { KEY_HEIGHT + 16.0 }
}

/// The bar's fill: `chrome` over the terminal's background, opaque. The
/// bar sits in the column under the pane, so nothing scrolls beneath it
/// for a blur to show; and its edge fades paint this exact color over the
/// keys (never a mask: SwiftUI's gradient mask drew the keys' text at 1x
/// on device).
fn bar_fill() -> AdaptiveColor {
    let (chrome, under) = (token("chrome"), token("terminal-bg"));
    let mixed = |c: Hsla, u: Hsla| over(Rgba::from(c), Rgba::from(u));
    AdaptiveColor::new(
        mixed(chrome.light, under.light),
        mixed(chrome.dark, under.dark),
    )
}

/// A keycap's stroke, as [`crate::ui::key`] draws one: in light, a white
/// cap on the bar is set off by `border-strong`; in dark, none.
fn cap_stroke() -> AdaptiveColor {
    let stroke = token("border-strong");
    AdaptiveColor {
        light: stroke.light,
        dark: AdaptiveColor::TRANSPARENT.dark,
    }
}

/// How far above and below a 36 pt cap its 44 pt touch reaches.
fn cap_inset() -> f32 {
    ((touch_target() - KEY_HEIGHT) / 2.0).max(0.0)
}

/// A hairline (`border`), `tall` points high: between the arrows, before
/// the keyboard key.
fn rule(tall: f32) -> View {
    view()
        .w(move || px(1.0 / pixel_ratio().max(1.0)))
        .h(px(tall))
        .flex_shrink_0()
        .bg(move || token("border"))
        .accessibility_elements_hidden(true)
}

/// The shortcut bar.
pub(crate) fn shortcut_bar(ctx: Ctx) -> AnyElement {
    let compact = {
        let ctx = ctx.clone();
        computed(move || ctx.compact())
    };
    view()
        .relative()
        .w_full()
        .flex_shrink_0()
        .bg(bar_fill)
        // The home indicator's inset only while the keyboard is down: up,
        // the keyboard covers it (the column's margin is its height).
        .pb(move || {
            px(if keyboard_height() > px(0.0) {
                0.0
            } else {
                safe_area().bottom.0
            })
        })
        .test_id("terminal-shortcut-bar")
        .child(
            view()
                .h(move || px(height(compact.get())))
                .flex_row()
                .items_center()
                .test_id("terminal-shortcut-keys")
                .child(chip_row(ctx.clone()))
                .child(keyboard_toggle(ctx.clone())),
        )
        // The hairline on top.
        .child(
            view()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .h(move || px(1.0 / pixel_ratio().max(1.0)))
                .pointer_events(PointerEvents::None)
                .bg(move || token("border")),
        )
        .into_element()
}

/// The row of chips: scrolls natively, fading out over an edge with chips
/// hidden past it.
fn chip_row(ctx: Ctx) -> AnyElement {
    let offset = animated_value(0.0);
    let viewport = signal(0.0_f32);
    let content = signal(0.0_f32);
    // The row is made again when the chips or what they draw change. What
    // follows a state of its own reads it in place, so a key held through
    // it is not made again under the finger: the mic (its controller's),
    // ctrl's arming (which a combination sent lets go of mid-hold), the
    // keyboard (the joystick asks it when touched).
    let bar = {
        let bar = ctx.bar;
        computed(move || {
            bar.with(|b| Bar {
                mic_active: false,
                ctrl_armed: false,
                keyboard_down: false,
                ..b.clone()
            })
        })
    };
    let chips = dynamic({
        let ctx = ctx.clone();
        move || {
            let bar = bar.get();
            let ctx = ctx.clone();
            untracked(move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(GAP))
                    .px(px(INSET))
                    .on_layout(move |b: &Bounds<Points>| {
                        if (content.get_untracked() - b.size.width.0).abs() > 0.25 {
                            content.set(b.size.width.0);
                        }
                    })
                    .children(
                        bar.chips
                            .iter()
                            .filter_map(|chip| self::chip(chip, &bar, ctx.clone())),
                    )
                    .into_element()
            })
        }
    });
    // The fades, growing in as content goes past an edge. Only a chip
    // past the trailing edge counts: the row's own inset beyond it, with
    // every chip in view, is no reason to fade the last one.
    let fades = dynamic(move || {
        let overflow = (content.get() - viewport.get()).max(0.0);
        let hidden = overflow - INSET > 0.5;
        untracked(move || {
            let leading = offset.interpolate(&[0.0, FADE], &[0.0, 1.0]);
            let mut row = view()
                .absolute()
                .inset_0()
                .flex_row()
                .pointer_events(PointerEvents::None)
                .child(edge_fade(true).bind_opacity(&leading))
                .child(view().flex_1());
            if hidden {
                let trailing =
                    offset.interpolate(&[(overflow - FADE).max(0.0), overflow], &[1.0, 0.0]);
                row = row.child(
                    edge_fade(false)
                        .bind_opacity(&trailing)
                        .test_id("terminal-chip-fade-trailing"),
                );
            }
            row.into_element()
        })
    });
    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .relative()
        .on_layout(move |b: &Bounds<Points>| {
            if (viewport.get_untracked() - b.size.width.0).abs() > 0.25 {
                viewport.set(b.size.width.0);
            }
        })
        .child(
            scroll()
                .horizontal()
                .scrollbar(false)
                .w_full()
                .h_full()
                .scroll_value(&offset)
                .test_id("terminal-chip-row")
                .child(view().h_full().justify_center().child(chips)),
        )
        .child(fades)
        .into_element()
}

/// One edge's fade: clear on the row's side, the bar's fill at the edge,
/// following 1 − t² (most of the reach still shows the chip under it).
fn edge_fade(leading: bool) -> View {
    let fade = view()
        .w(px(FADE))
        .h_full()
        // The bar's own fill, clear on the row's side.
        .gradient_from(move || bar_fill().map(|c| Hsla { a: 0.0, ..c }))
        .gradient_via(move || bar_fill().map(|c| Hsla { a: 0.25, ..c }))
        .gradient_to(bar_fill);
    if leading {
        fade.bg_linear_to_l()
    } else {
        fade.bg_linear_to_r()
    }
}

/// The chip `chip` is, or none (the mic with voice input off).
fn chip(chip: &Chip, bar: &Bar, ctx: Ctx) -> Option<AnyElement> {
    Some(match chip.kind {
        ChipKind::Arrows => arrow_cluster(ctx),
        ChipKind::Dpad => joystick(ctx),
        ChipKind::Scroll => scroll_pad(ctx),
        ChipKind::Mic if !bar.voice => return None,
        ChipKind::Mic => mic_chip(ctx),
        ChipKind::Image => image_chip(ctx, bar),
        ChipKind::Ctrl => ctrl_chip(chip, ctx),
        ChipKind::Key => key_chip(chip, ctx),
        ChipKind::Text | ChipKind::Paste | ChipKind::Files => button_chip(chip, ctx),
    })
}

/// A chip's test id: `shortcut-` and its label (its id when it has none).
fn chip_test_id(chip: &Chip) -> String {
    let name = if chip.label.is_empty() {
        &chip.id
    } else {
        &chip.label
    };
    format!("shortcut-{name}")
}

/// A key's press, as a hardware key's (`KeyRepeatGesture`): once on a
/// tap; held past [`HOLD`], at once and then every [`REPEAT`] until the
/// finger lifts or the scroll takes the touch. The press's own end (a tap
/// that was held into a repeat) sends nothing more.
#[derive(Clone, Copy)]
struct Hold {
    wait: Stored<Option<Spawned<()>>>,
    repeat: Stored<Option<Spawned<()>>>,
    repeated: Stored<bool>,
    /// A finger is on the key: what a key drawn here (an arrow) washes by.
    pressed: Signal<bool>,
}

impl Hold {
    fn new() -> Hold {
        Hold {
            wait: stored(None),
            repeat: stored(None),
            repeated: stored(false),
            pressed: signal(false),
        }
    }

    /// The finger went down: the hold starts.
    fn down(self, fire: Rc<dyn Fn()>) {
        self.stop();
        self.repeated.replace(false);
        self.pressed.set(true);
        self.wait.replace(Some(set_timeout(HOLD, move || {
            self.repeated.replace(true);
            fire();
            self.repeat
                .replace(Some(set_interval(REPEAT, move || fire())));
        })));
    }

    /// The press ended: lifted, moved away, cancelled.
    fn up(self) {
        self.stop();
        self.pressed.set(false);
    }

    /// The press counted as a tap: it sends, unless the hold already did.
    fn tap(self, fire: &dyn Fn()) {
        if !self.repeated.with(|r| *r) {
            fire();
        }
    }

    fn stop(self) {
        for task in [self.wait, self.repeat] {
            if let Some(task) = task.replace(None) {
                task.cancel();
            }
        }
    }
}

/// A single key (⌫, esc, ^C…): repeats while held (`HoldRepeatChip`).
fn key_chip(chip: &Chip, ctx: Ctx) -> AnyElement {
    let id = chip.id.clone();
    let fire: Rc<dyn Fn()> = Rc::new(move || ctx.ask(Intent::Chip(id.clone())));
    let hold = Hold::new();
    let label = SharedString::from(chip.label.clone());
    let spoken = label.clone();
    let on_tap = fire.clone();
    key(label)
        .accessibility_label(move || spoken.clone())
        .test_id(&chip_test_id(chip))
        .on_press_down(move || hold.down(fire.clone()))
        .on_press_up(move || hold.up())
        .on_press(move || hold.tap(&*on_tap))
        .into_element()
}

/// Text, a sequence, paste, files: one send per tap.
fn button_chip(chip: &Chip, ctx: Ctx) -> AnyElement {
    let cap = match chip.kind {
        ChipKind::Paste => symbol_key("doc.on.clipboard", t!("Paste")),
        ChipKind::Files => symbol_key("folder", t!("Browse files")),
        _ => {
            let label = SharedString::from(chip.label.clone());
            let spoken = label.clone();
            key(label).accessibility_label(move || spoken.clone())
        }
    };
    cap.test_id(&chip_test_id(chip))
        .on_press(ctx.asking(Intent::Chip(chip.id.clone())))
        .into_element()
}

/// Sticky ctrl: armed (the accent's wash, selected for screen readers)
/// until its key is pressed, which the host says.
fn ctrl_chip(chip: &Chip, ctx: Ctx) -> AnyElement {
    let bar = ctx.bar;
    let armed = move || bar.with(|b| b.ctrl_armed);
    key("ctrl")
        .variant(KeyVariant::Modifier)
        .armed(armed)
        .accessibility_label(move || {
            SharedString::from(if armed() {
                t!("Control armed")
            } else {
                t!("Control")
            })
        })
        .test_id(&chip_test_id(chip))
        .on_press(ctx.asking(Intent::Chip(chip.id.clone())))
        .into_element()
}

/// The mic: armed while a voice session is up. It follows the session's
/// controller as the voice panel does (the panel is the controller's
/// mirror): lit in the commit the panel comes in, dark in the one it goes;
/// the bar's `micActive` covers a host that posts no panel.
fn mic_chip(ctx: Ctx) -> AnyElement {
    let active = {
        let ctx = ctx.clone();
        computed(move || ctx.dictation.with(Option::is_some) || ctx.bar.with(|b| b.mic_active))
    };
    dynamic(move || {
        let active = active.get();
        let ctx = ctx.clone();
        untracked(move || {
            symbol_key(
                if active { "mic.fill" } else { "mic" },
                if active {
                    t!("Stop voice input")
                } else {
                    t!("Start voice input")
                },
            )
            .armed(move || active)
            .test_id("voice-input")
            .on_press(ctx.asking(Intent::Mic))
            .into_element()
        })
    })
    .into_element()
}

/// The image chip: a tap opens the photo picker; a long press, its menu
/// (the clipboard's image, a photo, this session's sent images by number),
/// which is the host's (`moshpit/press-menu`, over the drawn key and as
/// tall as its touch).
fn image_chip(ctx: Ctx, bar: &Bar) -> AnyElement {
    let mut items = Vec::new();
    if bar.image.clipboard {
        items.push(json!({"id": "paste", "title": t!("Paste image from clipboard"), "image": "doc.on.clipboard"}));
    }
    if bar.image.camera {
        items.push(json!({"id": "camera", "title": t!("Take photo"), "image": "camera"}));
    }
    // Newest first: the one just sent is the likeliest to be wanted again.
    for sent in bar.image.sent.iter().rev() {
        items.push(json!({
            "id": format!("sent:{}", sent.id),
            "title": format!("#{} · {}", sent.number, sent.filename),
            "image": "photo",
            "section": t!("Sent this session"),
        }));
    }
    let tell = ctx.clone();
    view()
        .relative()
        .h(px(touch_target()))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .test_id("shortcut-image")
        // The look only: the menu above takes the touches and is what
        // screen readers find.
        .child(
            view()
                .pointer_events(PointerEvents::None)
                .accessibility_elements_hidden(true)
                .child(symbol_key("photo", t!("Attach image"))),
        )
        .child(
            native("moshpit/press-menu")
                .absolute()
                .inset_0()
                .prop("items", json!(items))
                .prop("identifier", json!("attach-image"))
                .prop("label", json!(t!("Attach image")))
                .on_native_event(move |event: &NativeEvent| match &*event.name {
                    "press" => tell.ask(Intent::Image),
                    "select" => match event.data.as_str() {
                        Some("paste") => tell.ask(Intent::ImagePaste),
                        Some("camera") => tell.ask(Intent::ImageCamera),
                        Some(id) => {
                            if let Some(id) = id.strip_prefix("sent:") {
                                tell.ask(Intent::ImageSent(id.to_owned()));
                            }
                        }
                        None => {}
                    },
                    _ => {}
                }),
        )
        .into_element()
}

/// A keycap's look alone, under keys drawn here (the arrows, the
/// joystick, the scroll thumb): the key's fill and corners, 36 pt in a
/// 44 pt touch.
fn cap_fill() -> View {
    view()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .top(px(cap_inset()))
        .bottom(px(cap_inset()))
        .rounded(px(number("radius-sm")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("key"))
        .border(px(1.0))
        .border_color(cap_stroke)
        .pointer_events(PointerEvents::None)
}

/// The four arrows on one cap (`ArrowCluster`): each a key that sends on a
/// tap and repeats while held, its part of the cap washed while pressed.
fn arrow_cluster(ctx: Ctx) -> AnyElement {
    const ZONES: [(Dir, &str); 4] = [
        (Dir::Left, "arrow.left"),
        (Dir::Up, "arrow.up"),
        (Dir::Down, "arrow.down"),
        (Dir::Right, "arrow.right"),
    ];
    let mut zones = view().flex_row().items_center();
    for (index, (dir, glyph)) in ZONES.into_iter().enumerate() {
        if index > 0 {
            zones = zones.child(rule(18.0));
        }
        zones = zones.child(arrow_zone(ctx.clone(), dir, glyph));
    }
    view()
        .relative()
        .h(px(touch_target()))
        .flex_shrink_0()
        .justify_center()
        .test_id("arrow-cluster")
        .child(cap_fill())
        .child(zones)
        .into_element()
}

/// One arrow of the cluster: as tall as a key's touch.
fn arrow_zone(ctx: Ctx, dir: Dir, glyph: &'static str) -> AnyElement {
    let hold = Hold::new();
    let fire: Rc<dyn Fn()> = Rc::new(move || ctx.ask(Intent::Arrow(dir)));
    let label = match dir {
        Dir::Left => t!("Move left"),
        Dir::Up => t!("Move up"),
        Dir::Down => t!("Move down"),
        Dir::Right => t!("Move right"),
    };
    let on_tap = fire.clone();
    let pressed = hold.pressed;
    view()
        .w(px(ARROW_WIDTH))
        .h(px(touch_target()))
        .items_center()
        .justify_center()
        .role(AccessibilityRole::Button)
        .label(label)
        .test_id(&format!("arrow-{}", dir.name()))
        .on_press_down(move || hold.down(fire.clone()))
        .on_press_up(move || hold.up())
        .on_press(move || hold.tap(&*on_tap))
        .child(
            view()
                .w_full()
                .h(px(KEY_HEIGHT))
                .items_center()
                .justify_center()
                .rounded(px(number("radius-sm")))
                .border_curve(BorderCurve::Continuous)
                .bg(move || {
                    if pressed.get() {
                        token("key-pressed")
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .transition_colors()
                .duration(Duration::Fast.ms())
                .child(icon(glyph).size(IconSize::Key).tone(Tone::Key)),
        )
        .into_element()
}

/// The direction a push of `(dx, dy)` means, by its dominant axis; none
/// inside the dead zone (`JoystickRepeater.direction`).
pub(crate) fn direction(dx: f32, dy: f32, threshold: f32) -> Option<Dir> {
    if dx.abs() < threshold && dy.abs() < threshold {
        return None;
    }
    Some(if dx.abs() > dy.abs() {
        if dx > 0.0 { Dir::Right } else { Dir::Left }
    } else if dy > 0.0 {
        Dir::Down
    } else {
        Dir::Up
    })
}

/// A held direction's cadence (`JoystickRepeater`): it fires the moment a
/// new direction is acquired, then repeats every tick after a short delay.
#[derive(Clone, Copy)]
struct Stick {
    direction: Signal<Option<Dir>>,
    task: Stored<Option<Spawned<()>>>,
}

impl Stick {
    fn new() -> Stick {
        Stick {
            direction: signal(None),
            task: stored(None),
        }
    }

    fn set(&self, dir: Option<Dir>, fire: Rc<dyn Fn(Dir)>) {
        if dir == self.direction.get_untracked() {
            return;
        }
        self.direction.set(dir);
        if let Some(task) = self.task.replace(None) {
            task.cancel();
        }
        let Some(dir) = dir else { return };
        fire(dir);
        let ticks = stored(0_u32);
        self.task.replace(Some(set_interval(REPEAT, move || {
            let n = ticks.with(|t| {
                *t += 1;
                *t
            });
            if n >= STICK_DELAY_TICKS {
                fire(dir);
            }
        })));
    }

    fn stop(&self) {
        self.direction.set(None);
        if let Some(task) = self.task.replace(None) {
            task.cancel();
        }
    }
}

/// A push-and-hold chip (`DirectionPad`, `ScrollPad`): claims its touches
/// (the bar does not scroll under it); with `dwell`, a push engages only
/// once the finger has settled; `read` turns the push into a direction.
/// A key's cap, as tall as a key's touch, washed while it pushes.
fn stick_chip(
    glyph: &'static str,
    dwell: impl Fn() -> bool + 'static,
    read: fn(f32, f32) -> Option<Dir>,
    fire: Rc<dyn Fn(Dir)>,
) -> View {
    let stick = Stick::new();
    // Where the push is measured from (the touch's place when it engaged),
    // and whether it has engaged or failed its settle.
    let origin = stored((0.0_f32, 0.0_f32));
    let engaged = signal(false);
    let failed = stored(false);
    let settle = stored(None::<Spawned<()>>);
    let release = move || {
        if let Some(timer) = settle.replace(None) {
            timer.cancel();
        }
        engaged.set(false);
        stick.stop();
    };
    let pushing = move || stick.direction.get().is_some();
    view()
        .relative()
        .min_w(px(KEY_MIN_WIDTH))
        .h(px(touch_target()))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |e: &ResponderEvent| {
            failed.replace(false);
            origin.replace((e.gesture.delta.x.0, e.gesture.delta.y.0));
            if dwell() {
                engaged.set(false);
                settle.replace(Some(set_timeout(ENGAGE_DELAY, move || {
                    if !failed.with(|f| *f) {
                        engaged.set(true);
                    }
                })));
            } else {
                engaged.set(true);
            }
        })
        .on_responder_move({
            let fire = fire.clone();
            move |e: &ResponderEvent| {
                let (dx, dy) = (e.gesture.delta.x.0, e.gesture.delta.y.0);
                if !engaged.get_untracked() {
                    // Moving before it settled: a swipe passing through.
                    if dx.hypot(dy) > ENGAGE_SLOP {
                        failed.replace(true);
                    }
                    origin.replace((dx, dy));
                    return;
                }
                if failed.with(|f| *f) {
                    return;
                }
                let (ox, oy) = origin.with(|o| *o);
                stick.set(read(dx - ox, dy - oy), fire.clone());
            }
        })
        .on_responder_release(move |_: &ResponderEvent| release())
        .on_responder_terminate(move |_: &ResponderEvent| release())
        .child(
            view()
                .min_w(px(KEY_MIN_WIDTH))
                .h(px(KEY_HEIGHT))
                .px(px(8.0))
                .items_center()
                .justify_center()
                .rounded(px(number("radius-sm")))
                .border_curve(BorderCurve::Continuous)
                .bg(move || token(if pushing() { "primary-soft" } else { "key" }))
                .border(px(1.0))
                .border_color(cap_stroke)
                .transition_colors()
                .duration(Duration::Fast.ms())
                .child(dynamic(move || {
                    let dir = stick.direction.get();
                    untracked(move || {
                        // The glyph nudges toward the push.
                        let (x, y) = match dir {
                            Some(Dir::Up) => (0.0, -3.0),
                            Some(Dir::Down) => (0.0, 3.0),
                            Some(Dir::Left) => (-3.0, 0.0),
                            Some(Dir::Right) => (3.0, 0.0),
                            None => (0.0, 0.0),
                        };
                        view()
                            .translate_x(px(x))
                            .translate_y(px(y))
                            .child(icon(glyph).size(IconSize::Key).tone(if dir.is_some() {
                                Tone::Primary
                            } else {
                                Tone::Key
                            }))
                            .into_element()
                    })
                })),
        )
}

/// The joystick (`DirectionPad`): push toward a direction to send that
/// arrow, hold to repeat. Settles first only with the keyboard down (as
/// it is when the push starts).
fn joystick(ctx: Ctx) -> AnyElement {
    let tell = ctx.clone();
    let fire: Rc<dyn Fn(Dir)> = Rc::new(move |dir| tell.ask(Intent::Arrow(dir)));
    let actions = ctx.clone();
    let bar = ctx.bar;
    stick_chip(
        "arrow.up.and.down.and.arrow.left.and.right",
        move || bar.with_untracked(|b| b.keyboard_down),
        |dx, dy| direction(dx, dy, STICK_THRESHOLD),
        fire,
    )
    .role(AccessibilityRole::Adjustable)
    .label(t!("Arrow keys"))
    .hint(t!("Drag toward a direction to send an arrow key"))
    .accessibility_actions(vec![
        action("up", t!("Move up")),
        action("down", t!("Move down")),
        action("left", t!("Move left")),
        action("right", t!("Move right")),
    ])
    .on_accessibility_action(move |name: &str| {
        let dir = match name {
            "up" | "increment" => Dir::Up,
            "down" | "decrement" => Dir::Down,
            "left" => Dir::Left,
            "right" => Dir::Right,
            _ => return,
        };
        actions.ask(Intent::Arrow(dir));
    })
    .test_id("dpad-joystick")
    .into_element()
}

/// The scroll thumb (`ScrollPad`): push up for older lines, down for newer.
/// Always settles first: nobody reaches for it mid-sentence.
fn scroll_pad(ctx: Ctx) -> AnyElement {
    let tell = ctx.clone();
    let fire: Rc<dyn Fn(Dir)> = Rc::new(move |dir| tell.ask(Intent::Scroll(dir)));
    let actions = ctx.clone();
    stick_chip(
        "arrow.up.and.down",
        || true,
        |_, dy| {
            if dy.abs() < SCROLL_THRESHOLD {
                None
            } else if dy < 0.0 {
                Some(Dir::Up)
            } else {
                Some(Dir::Down)
            }
        },
        fire,
    )
    .role(AccessibilityRole::Adjustable)
    .label(t!("Scroll history"))
    .hint(t!("Drag up or down to scroll the terminal scrollback"))
    .accessibility_actions(vec![
        action("up", t!("Scroll up")),
        action("down", t!("Scroll down")),
    ])
    .on_accessibility_action(move |name: &str| match name {
        "up" | "increment" => actions.ask(Intent::Scroll(Dir::Up)),
        "down" | "decrement" => actions.ask(Intent::Scroll(Dir::Down)),
        _ => {}
    })
    .test_id("scroll-thumb")
    .into_element()
}

fn action(name: &str, label: String) -> AccessibilityActionDef {
    AccessibilityActionDef {
        name: name.into(),
        label: Some(label.into()),
    }
}

/// The keyboard key, pinned at the trailing end after a hairline (the
/// row's fade ends there): raises or puts away the keyboard. Its glyph
/// swaps `keyboard` ⇄ `keyboard.chevron.compact.down`, the old one
/// shrinking away as the new one grows in (`base`); at once under reduced
/// motion.
fn keyboard_toggle(ctx: Ctx) -> AnyElement {
    let bar = ctx.bar;
    let down = computed(move || bar.with(|b| b.keyboard_down));
    // 1: the keyboard is down, `keyboard` shows (it offers to raise it).
    let raise = animated_value(if down.get_untracked() { 1.0 } else { 0.0 });
    effect(move || {
        let target = if down.get() { 1.0 } else { 0.0 };
        if reduce_motion() {
            raise.set(target);
        } else {
            drop(raise.timing(target, Duration::Base.nominal_ms(), MOVE));
        }
    });
    // The face is a key's symbol (`IconSize::Key`): as large as the keys'
    // words, following the text size with them.
    let side = move || px(IconSize::Key.points_now());
    let glyph = move |name: &'static str, shown_at: f32| {
        let (opacity, scale) = if shown_at > 0.5 {
            ([0.0, 1.0], [0.5, 1.0])
        } else {
            ([1.0, 0.0], [1.0, 0.5])
        };
        view()
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .bind_opacity(&raise.interpolate(&[0.0, 1.0], &opacity))
            .bind_scale(&raise.interpolate(&[0.0, 1.0], &scale))
            .child(icon(name).size(IconSize::Key).tone(Tone::Key))
    };
    let face = view()
        .relative()
        .w(side)
        .h(side)
        .child(glyph("keyboard", 1.0))
        .child(glyph("keyboard.chevron.compact.down", 0.0));
    view()
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .pr(px(INSET))
        .flex_shrink_0()
        .child(rule(KEY_HEIGHT - 12.0))
        .child(
            content_key(face, t!("Show keyboard"))
                .variant(KeyVariant::Toggle)
                .accessibility_label(move || {
                    SharedString::from(if down.get() {
                        t!("Show keyboard")
                    } else {
                        t!("Hide keyboard")
                    })
                })
                .test_id("keyboard-toggle")
                .on_press(ctx.asking(Intent::Keyboard)),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_reads_its_dominant_axis_outside_the_dead_zone() {
        assert_eq!(direction(3.0, -4.0, STICK_THRESHOLD), None);
        assert_eq!(direction(10.0, -4.0, STICK_THRESHOLD), Some(Dir::Right));
        assert_eq!(direction(-10.0, 4.0, STICK_THRESHOLD), Some(Dir::Left));
        assert_eq!(direction(5.0, -12.0, STICK_THRESHOLD), Some(Dir::Up));
        assert_eq!(direction(5.0, 12.0, STICK_THRESHOLD), Some(Dir::Down));
        assert_eq!(direction(0.0, 7.0, STICK_THRESHOLD), Some(Dir::Down));
    }

    #[test]
    fn the_bar_is_a_key_and_8_pt_around_it_and_trims_in_ipad_landscape() {
        assert_eq!(height(false), 52.0);
        assert_eq!(height(true), 38.0);
    }
}
