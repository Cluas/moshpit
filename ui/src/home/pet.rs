//! The companion on Home (moshpit-design `docs/nsh/PET-DESIGN.md` §7.3,
//! §15.2): the egg, or the person's pet, perched on the top edge of the
//! card that holds the agent most in need — NEEDS YOU while anyone waits,
//! else the host whose agent is busiest, else the first host up, else the
//! saved hosts' card — in the pose of that agent's state.
//!
//! The host posts the pet (`{"op": "pet", …}`, [`PetView`]); this page
//! draws it. The person hatches the egg: each tap tells the host
//! (`{"ev": "egg-tap"}`), the first wobbles it, the second cracks it, the
//! third hatches it; an egg ready by itself (an agent's finished turn, the
//! third connection) hatches with no tap. Either way the page asks the
//! host to hatch it (`{"ev": "hatch"}`) when the host says it may (iCloud
//! has settled). It says the pet was seen (`{"ev": "pet-seen"}`, which
//! lets it grow). A tap pets it, with a light tap of the haptics; seven
//! quick taps, a pogo.
//!
//! Once it has hatched, and each time it grows into a new look, Home asks
//! whether to make it the app icon ([`icon_offer`], B668): the host
//! switches only on a yes (`{"ev": "icon", "switch": true}`).
//!
//! It can be dragged ([`perched`]): picked up, it hangs from the finger and
//! sways, following it anywhere on the page. Let go over another card, it
//! goes there: the card it sits on is then the person's while that card is
//! on Home (`{"ev": "pet-home", "home": "host:c-dev"}`, [`Perch::key`]; the
//! host keeps it on this device and posts it back in [`PetView::home`]),
//! in the pose of what that card holds. Let go anywhere else, it crawls
//! back to its card. Either way it crawls to the card's top edge under
//! where it was dropped and holds on there with a little squash and a tap
//! of the haptics. Where along the edge is the person's too
//! (`{"ev": "pet-spot", "spot": 0.4}`, 0 its own place at the trailing end,
//! 1 the leading end, [`PetView::spot`]).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use moshpit_pet::{
    Look, Moment, PetView, Pose, Style,
    easter::Taps,
    talk::{Chatter, Cue},
};
use nsh_core::prelude::*;
use serde_json::json;

use super::{
    Ctx,
    model::{self, AgentState, Connection, Link},
};
use crate::ui::{
    bubble::{Tail, speech_toward},
    button::{Size, Variant, button},
    companion::{Figure, companion},
    motion::Duration as Motion,
    section::{Inset, section},
    tokens::token,
    typography::TextToken,
};

use super::talk::{Fill, Visible, pet_name, speaker, talks, who, words};

/// Where the companion sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Perch {
    /// On NEEDS YOU's card.
    NeedsYou,
    /// On this host's card.
    Host(String),
    /// On the saved hosts' card.
    Saved,
}

impl Perch {
    /// How the host keeps it ([`PetView::home`]): `needs-you`, `saved` or
    /// `host:<id>`.
    pub fn key(&self) -> String {
        match self {
            Perch::NeedsYou => "needs-you".into(),
            Perch::Saved => "saved".into(),
            Perch::Host(id) => format!("host:{id}"),
        }
    }

    /// The perch `key` names; none for anything else.
    pub fn from_key(key: &str) -> Option<Perch> {
        match key {
            "needs-you" => Some(Perch::NeedsYou),
            "saved" => Some(Perch::Saved),
            _ => key
                .strip_prefix("host:")
                .filter(|id| !id.is_empty())
                .map(|id| Perch::Host(id.to_owned())),
        }
    }
}

fn rank(state: AgentState) -> u8 {
    match state {
        AgentState::NeedsYou => 0,
        AgentState::Working => 1,
        AgentState::Done => 2,
        AgentState::Idle => 3,
    }
}

/// A live host's agent most in need, if it has any.
fn busiest(c: &Connection, fallback: &str) -> Option<AgentState> {
    if c.link != Link::Live {
        return None;
    }
    model::agent_rows(c, fallback)
        .iter()
        .map(|a| a.state)
        .min_by_key(|s| rank(*s))
}

/// Its pose on a host's card: its busiest agent's state, else how the
/// host's link is.
fn host_pose(c: &Connection, fallback: &str) -> Pose {
    match busiest(c, fallback) {
        Some(AgentState::NeedsYou) => Pose::Waiting,
        Some(AgentState::Working) => Pose::Working,
        Some(AgentState::Done) => Pose::Done,
        Some(AgentState::Idle) => Pose::Idle,
        None => match c.link {
            Link::Dead if c.failure.is_some() => Pose::Error,
            Link::Dead => Pose::Sleeping,
            Link::Stalled => Pose::Error,
            _ => Pose::Idle,
        },
    }
}

/// Where the companion sits and how, from what Home shows: on the card the
/// person put it on (`home`) while Home shows that card, else on the one
/// Home picks.
pub fn perch(
    connections: &[Connection],
    fallback: &str,
    home: Option<&Perch>,
) -> Option<(Perch, Pose)> {
    let waiting = !model::needs_you(connections, fallback).is_empty();
    match home {
        Some(Perch::NeedsYou) if waiting => return Some((Perch::NeedsYou, Pose::Waiting)),
        Some(Perch::Saved) if connections.iter().any(|c| c.link == Link::Saved) => {
            return Some((Perch::Saved, Pose::Sleeping));
        }
        Some(Perch::Host(id)) => {
            if let Some(c) = connections
                .iter()
                .find(|c| c.id == *id && c.link.has_session())
            {
                return Some((Perch::Host(id.clone()), host_pose(c, fallback)));
            }
        }
        _ => {}
    }
    if waiting {
        return Some((Perch::NeedsYou, Pose::Waiting));
    }
    let most = connections
        .iter()
        .filter_map(|c| busiest(c, fallback).map(|s| (rank(s), c)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, c)| c);
    if let Some(c) = most.or_else(|| connections.iter().find(|c| c.link.has_session())) {
        return Some((Perch::Host(c.id.clone()), host_pose(c, fallback)));
    }
    connections
        .iter()
        .any(|c| c.link == Link::Saved)
        .then_some((Perch::Saved, Pose::Sleeping))
}

/// The art: 40 pt, in a 44 pt target.
const ART: f32 = 40.0;
const TARGET: f32 = 44.0;
/// The target's top above the card's edge: the art's feet 3 pt inside it.
const ABOVE: f32 = 39.0;
/// Clear of the head's ⋯ (its 44 pt target) and a little more.
const FROM_END: f32 = 56.0;
/// The bubble's gap from the pet's target, and half a line's height (its
/// bottom sits that far under the pet's middle).
const SAYS_GAP: f32 = 2.0;
const SAYS_HALF_LINE: f32 = 15.0;

/// Clear of the card's leading corner, at the far end of the edge.
const FROM_START: f32 = 12.0;
/// How far it can be moved along an edge `width` wide, from its own place
/// toward the leading end.
pub fn reach(width: f32) -> f32 {
    (width - FROM_END - TARGET - FROM_START).max(0.0)
}

/// Where it holds on (0 to 1, [`PetView::spot`]) when let go `x` points
/// from its own place (negative: toward the leading end) on an edge
/// `width` wide: under where it was dropped, never past either end.
pub fn spot_at(x: f32, width: f32) -> f32 {
    let reach = reach(width);
    if reach <= 0.0 || !x.is_finite() {
        return 0.0;
    }
    (-x / reach).clamp(0.0, 1.0)
}

/// Its translation from its own place for `spot` on an edge `width` wide.
pub fn x_of(spot: f32, width: f32) -> f32 {
    -spot.clamp(0.0, 1.0) * reach(width)
}

/// A card on the page, where it is: its top left corner in the page's
/// coordinates, its width and height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Place {
    /// The middle of its target, held on this card's edge at `spot`.
    fn held_at(&self, spot: f32) -> (f32, f32) {
        (
            self.x + self.width - FROM_END - TARGET / 2.0 + x_of(spot, self.width),
            self.y - ABOVE + TARGET / 2.0,
        )
    }

    /// Whether the pet's middle at `y` is over this card: on it, or over
    /// its top edge, where it would sit.
    fn takes(&self, y: f32) -> bool {
        y >= self.y - TARGET && y < self.y + self.height
    }
}

/// Where it goes when let go with its middle at `at` (page coordinates):
/// the card it's over among `cards` when that isn't `here`, where along
/// that card's edge (the spot under it) and how far it then is from
/// there (to crawl). None: over its own card, or over none.
pub fn landing(
    at: (f32, f32),
    cards: &[(Perch, Place)],
    here: &Perch,
) -> Option<(Perch, f32, (f32, f32))> {
    if !at.0.is_finite() || !at.1.is_finite() {
        return None;
    }
    if cards.iter().any(|(p, c)| p == here && c.takes(at.1)) {
        return None;
    }
    let (perch, card) = cards.iter().find(|(p, c)| p != here && c.takes(at.1))?;
    let own = card.held_at(0.0);
    let spot = spot_at(at.0 - own.0, card.width);
    let rest = card.held_at(spot);
    Some((perch.clone(), spot, (at.0 - rest.0, at.1 - rest.1)))
}

/// How fast it crawls, in points a second, and the crawl's shortest and
/// longest: a short way is still a few steps, a long one isn't a wait.
const CRAWL_SPEED: f32 = 260.0;
const CRAWL_MIN_MS: f32 = 220.0;
const CRAWL_MAX_MS: f32 = 900.0;
/// One step: up and down by [`HOP`] points.
const STEP_MS: f32 = 150.0;
const HOP: f32 = 3.0;

/// How long the crawl over `distance` points takes, in milliseconds.
pub fn crawl_ms(distance: f32) -> f32 {
    if !distance.is_finite() || distance <= 0.5 {
        return 0.0;
    }
    (distance / CRAWL_SPEED * 1000.0).clamp(CRAWL_MIN_MS, CRAWL_MAX_MS)
}

/// Held, it's a little bigger (lifted toward you) and sways this many
/// degrees either way.
const HELD_SCALE: f32 = 1.12;
const SWAY: f32 = 7.0;
const SWAY_MS: f32 = 900.0;

/// The page's pet state: what the host posted, and the moment last played.
#[derive(Clone)]
pub(crate) struct PetCtx {
    pub(crate) view: Signal<Option<PetView>>,
    pub(crate) seq: Signal<u64>,
    played: Rc<RefCell<u64>>,
    /// A figure played once (a pet, a moment), then back to the pose.
    showing: Signal<Option<Figure>>,
    taps: Rc<RefCell<Taps>>,
    /// Whether Home is on screen, the local hour and day (the host's
    /// `{"op": "visible"}`).
    pub(crate) visible: Signal<Visible>,
    /// Whether it talks, and what (§17): the core's chatter.
    pub(crate) chatter: Rc<RefCell<Chatter>>,
    /// The line in its bubble, numbered.
    pub(crate) saying: Signal<Option<(u64, String)>>,
    said: Rc<Cell<u64>>,
    /// Where the person last let it go (0 to 1), before the host has kept
    /// it and posted it back.
    dropped: Signal<Option<f32>>,
    /// The card it was last put on, before the host has kept it and
    /// posted it back.
    homed: Signal<Option<Perch>>,
    /// Let go over another card: how far from its place on that card it
    /// was let go, for the crawl there once it's drawn on it.
    landing: Rc<Cell<Option<(f32, f32)>>>,
    /// The cards it can sit on, where they are on the page when asked.
    cards: Rc<RefCell<Vec<(u64, Perch, ElementRef)>>>,
    card_ids: Rc<Cell<u64>>,
    /// It's held or crawling back: it doesn't take a new drag, wears its
    /// plain pose, and its card is drawn over the ones around it.
    pub(crate) moving: Signal<bool>,
}

impl PetCtx {
    pub(crate) fn new() -> PetCtx {
        PetCtx {
            view: signal(None),
            seq: signal(0),
            played: Rc::new(RefCell::new(0)),
            showing: signal(None),
            taps: Rc::new(RefCell::new(Taps::default())),
            visible: signal(Visible::default()),
            chatter: Rc::new(RefCell::new(Chatter::default())),
            saying: signal(None),
            said: Rc::new(Cell::new(0)),
            dropped: signal(None),
            homed: signal(None),
            landing: Rc::new(Cell::new(None)),
            cards: Rc::new(RefCell::new(Vec::new())),
            card_ids: Rc::new(Cell::new(0)),
            moving: signal(false),
        }
    }

    /// The card the person put it on: where it was last let go, else what
    /// the host keeps. Tracked.
    pub(crate) fn home(&self) -> Option<Perch> {
        self.homed.get().or_else(|| {
            self.view.with(|v| {
                v.as_ref()
                    .and_then(|v| v.home.as_deref())
                    .and_then(Perch::from_key)
            })
        })
    }

    /// Where the cards it can sit on are on the page now.
    async fn places(&self) -> Vec<(Perch, Place)> {
        let cards: Vec<_> = self
            .cards
            .borrow()
            .iter()
            .map(|(_, p, r)| (p.clone(), *r))
            .collect();
        let mut places = Vec::with_capacity(cards.len());
        for (perch, card) in cards {
            if let Some(m) = card.measure().await {
                places.push((
                    perch,
                    Place {
                        x: m.page.x.0,
                        y: m.page.y.0,
                        width: m.frame.size.width.0,
                        height: m.frame.size.height.0,
                    },
                ));
            }
        }
        places
    }

    /// Where along its card's edge it holds on (0 to 1): where it was last
    /// let go, else where the host keeps it.
    fn spot(&self) -> f32 {
        self.dropped
            .get()
            .unwrap_or_else(|| self.view.with(|v| v.as_ref().map_or(0.0, |v| v.spot)))
    }

    /// `cue` happened at `now`: the pet says a line if it talks and the
    /// core says it may, and screen readers hear it.
    pub(crate) fn cue(&self, cue: Cue, fill: Fill, now: f64) {
        let Some(view) = self.view.get_untracked() else {
            return;
        };
        let hour = self.visible.with_untracked(|v| v.hour);
        let said = super::talk::line(&mut self.chatter.borrow_mut(), cue, &view, hour, fill, now);
        if let Some((name, words)) = said {
            self.say(name, words);
        }
    }

    /// Home has settled after appearing: a milestone kept for this visit,
    /// or the night's or the morning's line.
    pub(crate) fn arrived(&self, now: f64) {
        let Some(view) = self.view.get_untracked() else {
            return;
        };
        if !talks(&view) {
            return;
        }
        let Visible { hour, day, .. } = self.visible.get_untracked();
        let Some(parts) = speaker(&view, hour) else {
            return;
        };
        let line = self.chatter.borrow_mut().arrived(who(&parts), now, day);
        self.show(line, &view, Fill::default());
    }

    fn show(&self, line: Option<&'static str>, view: &PetView, fill: Fill) {
        let name = pet_name(view);
        let Some(words) = line.and_then(|id| {
            words(
                id,
                &Fill {
                    name: name.clone(),
                    ..fill
                },
            )
        }) else {
            return;
        };
        self.say(name, words);
    }

    /// `name` says `words`: in the bubble, and to screen readers.
    fn say(&self, name: String, words: String) {
        say_into(self.saying, &self.said, name, words);
    }
}

/// `name` says `words` into `saying` (the bubble's line, numbered by
/// `said`), and screen readers hear it (§17.3): Home's pet and Settings'
/// card (B607).
pub(crate) fn say_into(
    saying: Signal<Option<(u64, String)>>,
    said: &Cell<u64>,
    name: String,
    words: String,
) {
    let n = said.get() + 1;
    said.set(n);
    announce_for_accessibility(t!("%@: %@", arg1 = name, arg2 = words.clone()));
    saying.set(Some((n, words)));
}

/// The host's `{"op": "pet"}`.
pub(crate) fn parse(fields: serde_json::Value) -> Result<(PetView, u64), String> {
    let seq = fields
        .get("seq")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let mut fields = fields;
    if let Some(map) = fields.as_object_mut() {
        map.remove("seq");
    }
    serde_json::from_value(fields)
        .map(|v| (v, seq))
        .map_err(|e| format!("home: pet: {e}"))
}

/// Whether hosts' marks are their pixel creatures (PET-DESIGN §6.3): with
/// the pixel companion on. Off, or before the host has said, their key
/// faces, today's look.
pub(crate) fn creatures(view: Option<&PetView>) -> bool {
    view.is_some_and(|v| v.shown && v.style == Style::Pixel)
}

fn look(view: &PetView) -> Option<Look> {
    view.pet.as_ref().and_then(|p| p.look())
}

/// What the perch shows for `view` in `pose`; none while the companion is
/// off or minimal.
fn figure(view: &PetView, pose: Pose) -> Option<Figure> {
    if !view.shown || view.style == Style::Minimal {
        return None;
    }
    if let Some(egg) = view.egg {
        return Some(Figure::Egg {
            ready: egg.ready,
            wobble: false,
        });
    }
    look(view).map(|look| Figure::Pet { look, pose })
}

/// The companion over `child` (a card), when it perches there.
pub(crate) fn on(ctx: Ctx, at: Perch, child: AnyElement) -> AnyElement {
    let here = {
        let ctx = ctx.clone();
        let at = at.clone();
        computed(move || {
            ctx.perch
                .with(|p| p.as_ref().is_some_and(|(p, _)| *p == at))
        })
    };
    let width = signal(0.0_f32);
    let moving = ctx.pet.moving;
    // Where it is, for a drop from another card.
    let card = element_ref();
    {
        let pet = ctx.pet.clone();
        let id = pet.card_ids.get() + 1;
        pet.card_ids.set(id);
        pet.cards.borrow_mut().push((id, at.clone(), card));
        cleanup(move || pet.cards.borrow_mut().retain(|(i, ..)| *i != id));
    }
    view()
        .w_full()
        .relative()
        .element_ref(&card)
        // Held or crawling, it goes over the cards after this one.
        .z_index(move || if here.get() && moving.get() { 2 } else { 0 })
        .on_layout(move |b: &Bounds<Points>| width.set(b.size.width.0))
        .child(child)
        .child(when(
            move || here.get(),
            move || perched(ctx.clone(), width),
        ))
        .into_element()
}

/// Over the cards after its own, while it's held or crawling: whether the
/// group around a card it can sit on (`holds`) is drawn over the groups
/// after it, so it goes over them too.
pub(crate) fn over(ctx: &Ctx, holds: fn(&Perch) -> bool) -> impl Fn() -> i32 + 'static {
    let (perch, moving) = (ctx.perch, ctx.pet.moving);
    move || {
        let held = moving.get() && perch.with(|p| p.as_ref().is_some_and(|(p, _)| holds(p)));
        if held { 1 } else { 0 }
    }
}

/// Where it is while dragged and as it crawls back: across the edge (`x`,
/// which the drag moves), the finger's height (`held`), the way still to
/// crawl up or down (`crawl`) and its steps (`hop`); and how it looks held
/// (`lift`, `sway`).
#[derive(Clone, Copy)]
struct Drag {
    x: AnimatedValue,
    held: AnimatedValue,
    crawl: AnimatedValue,
    hop: AnimatedValue,
    lift: AnimatedValue,
    sway: AnimatedValue,
}

impl Drag {
    fn new() -> Drag {
        Drag {
            x: animated_value(0.0),
            held: animated_value(0.0),
            crawl: animated_value(0.0),
            hop: animated_value(0.0),
            lift: animated_value(1.0),
            sway: animated_value(0.0),
        }
    }

    /// Picked up: lifted toward the finger, swaying.
    fn pick_up(self) {
        haptic(Haptic::Impact(HapticImpact::Medium));
        if reduce_motion() {
            return;
        }
        drop(self.lift.spring(HELD_SCALE, settle()));
        drop(
            self.sway.keyframes(
                [
                    Keyframe::new(0.0),
                    Keyframe::new(SWAY),
                    Keyframe::new(0.0),
                    Keyframe::new(-SWAY),
                    Keyframe::new(0.0),
                ],
                KeyframeTiming::new(SWAY_MS)
                    .easing(Easing::EaseInOut)
                    .infinite(),
            ),
        );
    }

    /// Let go `x` across and `y` down from its place: it crawls to `to`
    /// on the edge, in steps, and holds on. How long that takes (ms).
    fn crawl_to(self, x: f32, y: f32, to: f32) -> f32 {
        let hanging = if y.is_finite() { y } else { 0.0 };
        // Where it hangs is now the way back, not the finger's.
        self.crawl.set(hanging);
        self.held.set(0.0);
        self.sway.stop();
        let ms = if reduce_motion() {
            0.0
        } else {
            crawl_ms((to - x).hypot(hanging))
        };
        if ms <= 0.0 {
            self.x.set(to);
            self.crawl.set(0.0);
            self.sway.set(0.0);
            self.lift.set(1.0);
            return 0.0;
        }
        drop(self.sway.timing(0.0, Motion::Fast.ms(), Easing::EaseOut));
        drop(self.lift.timing(1.0, Motion::Fast.ms(), Easing::EaseOut));
        drop(self.x.timing(to, ms, Easing::Linear));
        drop(self.crawl.timing(0.0, ms, Easing::Linear));
        let steps = (ms / STEP_MS).round().max(1.0);
        drop(
            self.hop.keyframes(
                [Keyframe::new(0.0), Keyframe::new(-HOP), Keyframe::new(0.0)],
                KeyframeTiming::new(ms / steps)
                    .easing(Easing::EaseInOut)
                    .iterations(Some(steps as u32)),
            ),
        );
        ms
    }

    /// It got there: a little squash as it takes hold.
    fn hold_on(self) {
        haptic(Haptic::Impact(HapticImpact::Light));
        if reduce_motion() {
            return;
        }
        drop(self.lift.keyframes(
            [
                Keyframe::new(1.0),
                Keyframe::new(0.88),
                Keyframe::new(1.05),
                Keyframe::new(1.0),
            ],
            KeyframeTiming::new(Motion::Slow.ms()).easing(Easing::EaseOut),
        ));
    }
}

/// The spring it's lifted on.
fn settle() -> Spring {
    crate::ui::motion::structural_spring(None)
        .unwrap_or_else(|| Spring::new(1.0).damping_ratio(1.0))
}

/// It perches on the card's top edge `width` wide, where the person put
/// it, and can be dragged along it ([`Drag`]).
fn perched(ctx: Ctx, width: Signal<f32>) -> AnyElement {
    let pet = ctx.pet.clone();
    let saying = pet.saying;
    let moving = pet.moving;
    let drag = Drag::new();
    let spot = {
        let pet = pet.clone();
        computed(move || pet.spot())
    };
    // Where it sits, in layout (so its taps and drags land where it is
    // drawn: hit-testing outside the card follows layout, not the drag's
    // translation): where it was put, unless it is held or crawling (the
    // crawl ends there). `drag.x` is the way from there, 0 at rest.
    let rest = signal(spot.get_untracked());
    effect(move || {
        let at = spot.get();
        if !moving.get_untracked() {
            rest.set(at);
        }
    });
    // Off only while it crawls (the crawl moves `x`); held, the pan must
    // stay on, or turning it off ends the drag it is in.
    let crawling = signal(false);
    let start = move |_: &PanEvent| {
        moving.set(true);
        drag.pick_up();
    };
    // It got to `to_spot` on this card: its place moves under it, it holds
    // on, and the host keeps where.
    let arrive = {
        let pet = pet.clone();
        let tell = ctx.tell.clone();
        move |to_spot: f32| {
            let pet = pet.clone();
            let tell = tell.clone();
            move || {
                rest.set(to_spot);
                drag.x.set(0.0);
                crawling.set(false);
                moving.set(false);
                drag.hold_on();
                let view = pet.view.get_untracked();
                if let Some(look) = view.as_ref().and_then(look) {
                    play(
                        &pet,
                        Figure::Pet {
                            look,
                            pose: Pose::Petted,
                        },
                    );
                }
                if view.as_ref().is_none_or(|v| v.spot != to_spot) {
                    tell(json!({"ev": "pet-spot", "spot": to_spot}));
                }
            }
        }
    };
    let crawl = {
        let arrive = arrive.clone();
        move |x: f32, y: f32, to: f32, to_spot: f32| {
            let ms = drag.crawl_to(x, y, to);
            let arrived = arrive(to_spot);
            if ms <= 0.0 {
                arrived();
            } else {
                drop(set_timeout(Duration::from_millis(ms as u64), arrived));
            }
        }
    };
    // Let go over another card, it was taken off that one: it crawls from
    // where it was let go to its place on this one.
    if let Some((x, y)) = pet.landing.take() {
        let to_spot = rest.get_untracked();
        crawling.set(true);
        drag.x.set(x);
        crawl(x, y, 0.0, to_spot);
    }
    let end = {
        let pet = pet.clone();
        let ctx = ctx.clone();
        let crawl = crawl.clone();
        move |event: &PanEvent| {
            let x = event.value_x.unwrap_or(0.0);
            let y = event.value_y.unwrap_or(0.0);
            let w = width.get_untracked();
            let was = rest.get_untracked();
            let from = x_of(was, w);
            crawling.set(true);
            let back = {
                let pet = pet.clone();
                let crawl = crawl.clone();
                move |to_spot: f32| {
                    pet.dropped.set(Some(to_spot));
                    crawl(x, y, x_of(to_spot, w) - from, to_spot);
                }
            };
            if event.cancelled {
                back(was);
                return;
            }
            let here = ctx
                .perch
                .with_untracked(|p| p.as_ref().map(|(p, _)| p.clone()));
            let Some(here) = here else {
                back(spot_at(from + x, w));
                return;
            };
            // Over which card: the page says where they are.
            let pet = pet.clone();
            let tell = ctx.tell.clone();
            drop(spawn(async move {
                let places = pet.places().await;
                let at = places
                    .iter()
                    .find(|(p, _)| *p == here)
                    .map(|(_, c)| c.held_at(was))
                    .map(|(cx, cy)| (cx + x, cy + y));
                match at.and_then(|at| landing(at, &places, &here)) {
                    Some((to, to_spot, off)) => {
                        // Drawn on that card from here on, where it was let go.
                        pet.landing.set(Some(off));
                        pet.dropped.set(Some(to_spot));
                        tell(json!({"ev": "pet-home", "home": to.key()}));
                        pet.homed.set(Some(to));
                    }
                    None => back(spot_at(from + x, w)),
                }
            }));
        }
    };
    let shown_y = drag.held.add(&drag.crawl).add(&drag.hop);
    // Toward the card's leading end it says its line on its trailing side.
    let leading_half = computed(move || spot.get() > 0.5);
    target(ctx, Some(drag))
        .absolute()
        .top(px(-ABOVE))
        .right(move || px(FROM_END - x_of(rest.get(), width.get())))
        .z_index(1)
        .pan_values(Some(&drag.x), Some(&drag.held))
        .pan_enabled(move || !crawling.get())
        .on_pan_start(start)
        .on_pan_end(end)
        .bind_translate_x(&drag.x)
        .bind_translate_y(&shown_y)
        // What it says (§17.3): beside it, the bubble's bottom level with
        // its middle, so a second line grows upward.
        .child(dynamic(move || {
            let tail = if leading_half.get() {
                Tail::Leading
            } else {
                Tail::Trailing
            };
            untracked(move || {
                let says = says(saying, tail)
                    .absolute()
                    .w(px(crate::ui::bubble::MAX_WIDTH));
                let says = match tail {
                    Tail::Trailing => says.right(px(TARGET + SAYS_GAP)).items_end(),
                    Tail::Leading => says.left(px(TARGET + SAYS_GAP)).items_start(),
                };
                says.into_element()
            })
        }))
        .into_element()
}

/// The companion on the empty Home (B746): no connection yet, so no card
/// to perch on. It waits in the app's mark's place above the page's title,
/// the same egg (three taps hatch it) and the same pet once hatched; the
/// first card it perches on is the next place it goes. While the companion
/// is off or minimal, or before the host has said, the mark is there as
/// before.
pub(crate) fn alone(ctx: Ctx) -> AnyElement {
    let pet_view = ctx.pet.view;
    let shows = computed(move || {
        pet_view.with(|v| v.as_ref().and_then(|v| figure(v, Pose::Idle)).is_some())
    });
    when(
        move || shows.get(),
        move || {
            let saying = ctx.pet.saying;
            // Centred on the page, it says its line in the half of the
            // row on its leading side, as on a card, bounded by that half
            // so a long line wraps rather than leave the screen.
            view()
                .w_full()
                .flex_row()
                .items_center()
                .child(
                    view().flex_1().h(px(TARGET)).relative().child(
                        says(saying, Tail::Trailing)
                            .absolute()
                            .items_end()
                            .left(px(0.0))
                            .right(px(SAYS_GAP)),
                    ),
                )
                .child(target(ctx.clone(), None))
                .child(view().flex_1())
                .into_element()
        },
    )
    .otherwise(crate::ui::empty::app_mark)
    .into_element()
}

/// What the companion says, beside its target: the bubble's bottom level
/// with the target's middle, so a second line grows upward; it takes no
/// taps.
fn says(saying: Signal<Option<(u64, String)>>, tail: Tail) -> View {
    view()
        .bottom(px(TARGET / 2.0 - SAYS_HALF_LINE))
        .pointer_events(PointerEvents::None)
        .child(speech_toward(saying, tail))
}

/// The companion's 44 pt target: the egg or the pet in the pose of where
/// it sits, its taps, the moments it plays (what it says goes beside it,
/// where it sits: [`says`]). On a card it can be dragged (`drag`): held or
/// crawling, it wears its plain pose (a perch's props, a keyboard, a mug,
/// stay on the perch).
fn target(ctx: Ctx, drag: Option<Drag>) -> View {
    let pet = ctx.pet.clone();
    let pose = {
        let ctx = ctx.clone();
        let moving = pet.moving;
        computed(move || {
            if moving.get() {
                return Pose::Idle;
            }
            ctx.perch
                .with(|p| p.as_ref().map_or(Pose::Idle, |(_, pose)| *pose))
        })
    };
    let base = {
        let view = pet.view;
        computed(move || view.with(|v| v.as_ref().and_then(|v| figure(v, pose.get()))))
    };
    // A moment the host announced plays once: the hatch, or a stage grown.
    {
        let pet = pet.clone();
        effect(move || {
            let seq = pet.seq.get();
            let Some(view) = pet.view.get() else { return };
            if seq <= *pet.played.borrow() {
                return;
            }
            *pet.played.borrow_mut() = seq;
            let Some(look) = look(&view) else { return };
            let once = match view.moment {
                Some(Moment::Hatched) => Figure::Pet {
                    look,
                    pose: Pose::Hatch,
                },
                Some(Moment::Grew) => Figure::Pet {
                    look,
                    pose: Pose::Petted,
                },
                None => return,
            };
            play(&pet, once);
        });
    }
    // A tap on the egg tells the host.
    let egg_tell = ctx.tell.clone();
    // The egg asks to hatch whenever the host says it may (each message:
    // the host hatches only while Home is on screen, and only once).
    {
        let tell = ctx.tell.clone();
        effect(move || {
            if pet_may_hatch(&ctx.pet) {
                tell(json!({"ev": "hatch"}));
            }
        });
    }
    let showing = pet.showing;
    let shown = computed(move || showing.get().or_else(|| base.get()));
    let label = {
        let view = pet.view;
        move || view.with(|v| v.as_ref().map(label_of).unwrap_or_default())
    };
    let hint = {
        let view = pet.view;
        move || {
            view.with(|v| match v.as_ref().and_then(|v| v.egg) {
                Some(_) => t!("Tap three times to hatch"),
                None => t!("Tap to pet"),
            })
        }
    };
    let press = {
        let pet = pet.clone();
        let tell = egg_tell;
        move || {
            let Some(view) = pet.view.get_untracked() else {
                return;
            };
            let Some(look) = look(&view) else {
                // The egg: the host counts the taps (the first wobbles it,
                // the second cracks it, the third hatches it); it wobbles
                // here at once, cracked from the second.
                if let Some(egg) = view.egg {
                    let taps = egg.taps.saturating_add(1);
                    let impact = if taps >= 2 {
                        HapticImpact::Medium
                    } else {
                        HapticImpact::Light
                    };
                    haptic(Haptic::Impact(impact));
                    tell(json!({"ev": "egg-tap"}));
                    play(
                        &pet,
                        Figure::Egg {
                            ready: egg.ready || taps >= 2,
                            wobble: true,
                        },
                    );
                }
                return;
            };
            haptic(Haptic::Impact(HapticImpact::Light));
            let now = nsh_core::time::unix_ms() / 1000.0;
            let pogo = pet.taps.borrow_mut().tap(now);
            let pose = if pogo { Pose::Pogo } else { Pose::Petted };
            play(&pet, Figure::Pet { look, pose });
            // A tap always answers (§17.1).
            pet.cue(
                if pogo { Cue::Pogo } else { Cue::Tap },
                Fill::default(),
                now,
            );
        }
    };
    let art = dynamic(move || match shown.get() {
        Some(f) => untracked(move || companion(move || f, ART)),
        None => untracked(|| view().into_element()),
    });
    // Held, it hangs from the top of its head.
    let art = match drag {
        Some(drag) => view()
            .w(px(TARGET))
            .h(px(TARGET))
            .items_center()
            .justify_center()
            .origin_top()
            .bind_scale(&drag.lift)
            .bind_rotate(&drag.sway)
            .child(art)
            .into_element(),
        None => art.into_element(),
    };
    view()
        .w(px(TARGET))
        .h(px(TARGET))
        .items_center()
        .justify_center()
        .role(AccessibilityRole::Button)
        .label(label)
        .hint(hint)
        .test_id("home-pet")
        .on_press(press)
        .child(art)
}

/// Whether Home asks about the app icon now (B668): the host offers it,
/// and no moment is playing (the hatch plays first).
pub(crate) fn offers_icon(pet: &PetCtx) -> bool {
    pet.showing.get().is_none()
        && pet
            .view
            .with(|v| v.as_ref().is_some_and(|v| v.icon.is_some()))
}

/// The pet's size on the icon card.
const OFFER_ART: f32 = 48.0;

/// Home's question once the pet has hatched or grown into a new look
/// (B668, the owner's call): 「把 App 图标换成墩墩吗？」 with 「换」 and
/// 「先不换」. The host switches the icon only on 「换」, and asks no more
/// about this look either way. An icon the person picked by hand makes
/// 「先不换」 the default (the filled button).
pub(crate) fn icon_offer(ctx: Ctx) -> AnyElement {
    let pet_view = ctx.pet.view;
    let offer = computed(move || pet_view.with(|v| v.as_ref().and_then(|v| v.icon.clone())));
    let question = computed(move || {
        offer.with(|o| {
            o.as_ref()
                .map(|o| t!("Make %@ the app icon?", arg1 = species_name(&o.species)))
                .unwrap_or_default()
        })
    });
    let keep_default = computed(move || offer.with(|o| o.as_ref().is_some_and(|o| o.keep_default)));
    let figure = move || {
        pet_view.with(|v| {
            v.as_ref().and_then(look).map(|look| Figure::Pet {
                look,
                pose: Pose::Idle,
            })
        })
    };
    let answer = {
        let tell = ctx.tell.clone();
        move |switch: bool| {
            let tell = tell.clone();
            move || tell(json!({"ev": "icon", "switch": switch}))
        }
    };
    let buttons = dynamic(move || {
        let keep = keep_default.get();
        let change = answer(true);
        let not_yet = answer(false);
        untracked(move || {
            let (change_variant, not_yet_variant) = if keep {
                (Variant::Secondary, Variant::Primary)
            } else {
                (Variant::Primary, Variant::Secondary)
            };
            view()
                .flex_row()
                .justify_end()
                .gap(px(8.0))
                .child(
                    button(t!("Not Yet"))
                        .variant(not_yet_variant)
                        .size(Size::Md)
                        .test_id("home-icon-not-yet")
                        .on_press(not_yet),
                )
                .child(
                    button(t!("Change"))
                        .variant(change_variant)
                        .size(Size::Md)
                        .test_id("home-icon-change")
                        .on_press(change),
                )
                .into_element()
        })
    });
    let body = view()
        .w_full()
        .p(px(crate::ui::row::PADDING_X))
        .gap(px(12.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    view()
                        .w(px(OFFER_ART))
                        .h(px(OFFER_ART))
                        .items_center()
                        .justify_center()
                        .accessibility_elements_hidden(true)
                        .child(dynamic(move || match figure() {
                            Some(f) => untracked(move || companion(move || f, OFFER_ART)),
                            None => untracked(|| view().into_element()),
                        })),
                )
                .child(
                    TextToken::Headline
                        .apply(text(move || question.get()))
                        .flex_shrink(1.0)
                        .text_color(move || token("foreground"))
                        .role(AccessibilityRole::Header),
                ),
        )
        .child(buttons);
    section()
        .item(body, Inset::Content)
        .test_id("home-icon-offer")
        .into_element()
}

fn pet_may_hatch(pet: &PetCtx) -> bool {
    pet.view.with(|v| {
        v.as_ref()
            .is_some_and(|v| v.may_hatch && v.egg.is_some() && v.shown && v.style != Style::Minimal)
    })
}

/// Plays `figure` once, then goes back to the loop.
fn play(pet: &PetCtx, figure: Figure) {
    play_in(pet.showing, figure);
}

/// Shows `figure` in `showing` once, then clears it (Home's pet, Settings'
/// card).
pub(crate) fn play_in(showing: Signal<Option<Figure>>, figure: Figure) {
    let length = if reduce_motion() {
        Duration::from_millis(900)
    } else {
        figure.length()
    };
    showing.set(Some(figure));
    // Dropping the handle leaves the timer running (it ends with the owner).
    drop(set_timeout(length, move || {
        if showing.get_untracked() == Some(figure) {
            showing.set(None);
        }
    }));
}

/// What a screen reader hears: its name, and what it is.
pub(crate) fn label_of(view: &PetView) -> String {
    match &view.pet {
        None => t!(
            "An egg. Tap it three times to hatch it; it also hatches by itself when an agent finishes a task."
        ),
        Some(p) => {
            let name = if p.name.is_empty() {
                species_name(&p.species)
            } else {
                p.name.clone()
            };
            format!("{name}, {}", species_kind(&p.species))
        }
    }
}

/// A species' name in the app's language.
pub(crate) fn species_name(id: &str) -> String {
    match id {
        "tun" => t!("Tun"),
        "coo" => t!("Coo"),
        "dig" => t!("Dig"),
        "hex" => t!("Hex"),
        "ether" => t!("Ether"),
        "moth" => t!("Lumen"),
        other => other.to_owned(),
    }
}

/// What a species is, in the app's language.
pub(crate) fn species_kind(id: &str) -> String {
    match id {
        "tun" => t!("Tardigrade"),
        "coo" => t!("Pigeon"),
        "dig" => t!("Mole"),
        "hex" => t!("Axolotl"),
        "ether" => t!("Jellyfish"),
        "moth" => t!("Moth"),
        other => other.to_owned(),
    }
}

/// Tells the host the pet was seen (it may grow), now and every ten
/// minutes while Home shows.
pub(crate) fn seen(tell: super::Tell) -> Spawned<()> {
    tell(json!({"ev": "pet-seen"}));
    set_interval(Duration::from_secs(600), move || {
        tell(json!({"ev": "pet-seen"}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::model::HomeState;

    fn state(json: &str) -> Vec<Connection> {
        serde_json::from_str::<HomeState>(json).unwrap().connections
    }

    #[test]
    fn it_perches_on_the_saved_card_when_nothing_is_up_and_nowhere_without_hosts() {
        assert_eq!(perch(&[], "agent", None), None);
        let saved = state(
            r#"{"connections":[{"id":"a","name":"a","initial":"A","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"saved"}],"version":""}"#,
        );
        assert_eq!(
            perch(&saved, "agent", None),
            Some((Perch::Saved, Pose::Sleeping))
        );
    }

    #[test]
    fn a_live_host_without_agents_has_it_idle_and_a_failed_one_upset() {
        let live = state(
            r#"{"connections":[{"id":"a","name":"a","initial":"A","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"live"}],"version":""}"#,
        );
        assert_eq!(
            perch(&live, "agent", None),
            Some((Perch::Host("a".into()), Pose::Idle))
        );
        let dead = state(
            r#"{"connections":[{"id":"b","name":"b","initial":"B","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"dead"}],"version":""}"#,
        );
        assert_eq!(
            perch(&dead, "agent", None),
            Some((Perch::Host("b".into()), Pose::Sleeping))
        );
    }

    #[test]
    fn the_hosts_message_parses_with_its_seq() {
        let (view, seq) = parse(json!({"style": "pixel", "shown": true, "egg": {"ready": false}, "may_hatch": false, "seq": 2})).unwrap();
        assert_eq!(seq, 2);
        assert_eq!(
            figure(&view, Pose::Idle),
            Some(Figure::Egg {
                ready: false,
                wobble: false
            })
        );
        let (off, _) =
            parse(json!({"style": "minimal", "shown": true, "may_hatch": false})).unwrap();
        assert_eq!(figure(&off, Pose::Idle), None);
    }

    #[test]
    fn dropped_anywhere_it_holds_on_under_the_drop_and_never_past_the_ends() {
        // A 390 pt phone's card: 390 - 56 - 44 - 12 = 278 pt of edge.
        let w = 390.0;
        assert_eq!(reach(w), 278.0);
        assert_eq!(spot_at(0.0, w), 0.0, "where it was: its own place");
        assert_eq!(spot_at(-139.0, w), 0.5);
        assert_eq!(spot_at(-1000.0, w), 1.0, "past the leading end");
        assert_eq!(spot_at(80.0, w), 0.0, "never over the card's ⋯");
        assert_eq!(x_of(spot_at(-139.0, w), w), -139.0);
        assert_eq!(spot_at(-50.0, 80.0), 0.0, "no room: its own place");
        assert_eq!(spot_at(f32::NAN, w), 0.0);
        // The same spot on a wider card is as far along it.
        assert_eq!(x_of(0.5, 744.0), -0.5 * reach(744.0));
    }

    fn host(id: &str, link: &str) -> String {
        format!(
            r#"{{"id":"{id}","name":"{id}","initial":"A","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"{link}"}}"#
        )
    }

    #[test]
    fn put_on_a_card_it_stays_there_while_home_shows_that_card() {
        let hosts = state(&format!(
            r#"{{"connections":[{},{},{}],"version":""}}"#,
            host("a", "live"),
            host("b", "dead"),
            host("c", "saved")
        ));
        assert_eq!(
            perch(&hosts, "agent", None),
            Some((Perch::Host("a".into()), Pose::Idle)),
            "the first host up"
        );
        let b = Perch::Host("b".into());
        assert_eq!(
            perch(&hosts, "agent", Some(&b)),
            Some((b.clone(), Pose::Sleeping)),
            "in the pose of what that card holds"
        );
        assert_eq!(
            perch(&hosts, "agent", Some(&Perch::Saved)),
            Some((Perch::Saved, Pose::Sleeping))
        );
        // A card Home doesn't show: the one Home picks.
        for gone in [
            Perch::Host("c".into()),
            Perch::Host("z".into()),
            Perch::NeedsYou,
        ] {
            assert_eq!(
                perch(&hosts, "agent", Some(&gone)).map(|(p, _)| p),
                Some(Perch::Host("a".into())),
                "{gone:?}"
            );
        }
    }

    #[test]
    fn the_card_it_was_put_on_is_kept_by_a_key() {
        for perch in [Perch::NeedsYou, Perch::Saved, Perch::Host("c-dev:2".into())] {
            assert_eq!(Perch::from_key(&perch.key()), Some(perch));
        }
        assert_eq!(Perch::Host("c-dev".into()).key(), "host:c-dev");
        assert_eq!(Perch::from_key("host:"), None);
        assert_eq!(Perch::from_key("elsewhere"), None);
    }

    #[test]
    fn let_go_over_another_card_it_goes_there_under_the_finger() {
        let card = |y: f32| Place {
            x: 16.0,
            y,
            width: 358.0,
            height: 120.0,
        };
        let here = Perch::Host("a".into());
        let cards = [
            (Perch::NeedsYou, card(100.0)),
            (here.clone(), card(300.0)),
            (Perch::Saved, card(500.0)),
        ];
        // Its own place on a card: 16 + 358 - 56 - 22 = 296 across, 22 - 39
        // = 17 pt over the edge.
        assert_eq!(card(300.0).held_at(0.0), (296.0, 283.0));
        assert_eq!(landing((296.0, 283.0), &cards, &here), None, "where it was");
        assert_eq!(
            landing((200.0, 400.0), &cards, &here),
            None,
            "on its own card"
        );
        assert_eq!(
            landing((200.0, 700.0), &cards, &here),
            None,
            "under every card"
        );
        assert_eq!(
            landing((200.0, 260.0), &cards, &here),
            None,
            "over its own edge"
        );
        // Dropped on the saved card, past its middle: under the finger.
        let (to, spot, off) = landing((150.0, 520.0), &cards, &here).unwrap();
        assert_eq!(to, Perch::Saved);
        let reach = reach(358.0);
        assert!((spot - 146.0 / reach).abs() < 1e-4, "{spot}");
        assert!(off.0.abs() < 1e-3, "no way across: {off:?}");
        assert!(
            (off.1 - 37.0).abs() < 1e-3,
            "37 pt down from its edge: {off:?}"
        );
        // Over the card above's edge.
        let (to, spot, _) = landing((400.0, 70.0), &cards, &here).unwrap();
        assert_eq!((to, spot), (Perch::NeedsYou, 0.0), "never past its ⋯");
        assert_eq!(landing((f32::NAN, 520.0), &cards, &here), None);
    }

    #[test]
    fn it_crawls_back_at_a_pace_not_a_wait() {
        assert_eq!(crawl_ms(0.0), 0.0, "already there");
        assert_eq!(
            crawl_ms(10.0),
            CRAWL_MIN_MS,
            "a short way is still a few steps"
        );
        assert!((crawl_ms(130.0) - 500.0).abs() < 1e-3);
        assert_eq!(crawl_ms(5_000.0), CRAWL_MAX_MS);
    }

    #[test]
    fn hosts_are_creatures_only_with_the_pixel_companion_on() {
        let view = |style: &str, shown: bool| {
            parse(json!({"style": style, "shown": shown, "may_hatch": false}))
                .unwrap()
                .0
        };
        assert!(creatures(Some(&view("pixel", true))));
        assert!(!creatures(Some(&view("pixel", false))));
        assert!(!creatures(Some(&view("minimal", false))));
        assert!(!creatures(None), "before the host has said: the key faces");
    }
}
