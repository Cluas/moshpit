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
    bubble::speech,
    button::{Size, Variant, button},
    companion::{Figure, companion},
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

/// Where the companion sits and how, from what Home shows.
pub fn perch(connections: &[Connection], fallback: &str) -> Option<(Perch, Pose)> {
    if !model::needs_you(connections, fallback).is_empty() {
        return Some((Perch::NeedsYou, Pose::Waiting));
    }
    let rank = |s: AgentState| match s {
        AgentState::NeedsYou => 0,
        AgentState::Working => 1,
        AgentState::Done => 2,
        AgentState::Idle => 3,
    };
    let busiest = connections
        .iter()
        .filter(|c| c.link == Link::Live)
        .filter_map(|c| {
            model::agent_rows(c, fallback)
                .iter()
                .map(|a| a.state)
                .min_by_key(|s| rank(*s))
                .map(|s| (rank(s), c, s))
        })
        .min_by_key(|(r, ..)| *r);
    if let Some((_, c, state)) = busiest {
        let pose = match state {
            AgentState::NeedsYou => Pose::Waiting,
            AgentState::Working => Pose::Working,
            AgentState::Done => Pose::Done,
            AgentState::Idle => Pose::Idle,
        };
        return Some((Perch::Host(c.id.clone()), pose));
    }
    if let Some(c) = connections.iter().find(|c| c.link.has_session()) {
        let pose = match c.link {
            Link::Dead if c.failure.is_some() => Pose::Error,
            Link::Dead => Pose::Sleeping,
            Link::Stalled => Pose::Error,
            _ => Pose::Idle,
        };
        return Some((Perch::Host(c.id.clone()), pose));
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
        }
    }

    /// `cue` happened at `now`: the pet says a line if it talks and the
    /// core says it may, and screen readers hear it.
    pub(crate) fn cue(&self, cue: Cue, fill: Fill, now: f64) {
        let Some(view) = self.view.get_untracked() else {
            return;
        };
        if !talks(&view) {
            return;
        }
        let hour = self.visible.with_untracked(|v| v.hour);
        let Some(parts) = speaker(&view, hour) else {
            return;
        };
        let line = self.chatter.borrow_mut().cue(cue, who(&parts), now);
        self.show(line, &view, fill);
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
        let n = self.said.get() + 1;
        self.said.set(n);
        announce_for_accessibility(t!("%@: %@", arg1 = name, arg2 = words.clone()));
        self.saying.set(Some((n, words)));
    }
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
    view()
        .w_full()
        .relative()
        .child(child)
        .child(when(move || here.get(), move || perched(ctx.clone())))
        .into_element()
}

fn perched(ctx: Ctx) -> AnyElement {
    let pet = ctx.pet.clone();
    let pose = {
        let ctx = ctx.clone();
        computed(move || {
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
    let saying = pet.saying;
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
    view()
        .absolute()
        .top(px(-ABOVE))
        .right(px(FROM_END))
        .w(px(TARGET))
        .h(px(TARGET))
        .items_center()
        .justify_center()
        .z_index(1)
        .role(AccessibilityRole::Button)
        .label(label)
        .hint(hint)
        .test_id("home-pet")
        .on_press(press)
        .child(dynamic(move || match shown.get() {
            Some(f) => untracked(move || companion(move || f, ART)),
            None => untracked(|| view().into_element()),
        }))
        // What it says (§17.3): on its leading side, the bubble's bottom
        // level with its middle, so a second line grows upward.
        .child(
            view()
                .absolute()
                .right(px(TARGET + SAYS_GAP))
                .bottom(px(TARGET / 2.0 - SAYS_HALF_LINE))
                .w(px(crate::ui::bubble::MAX_WIDTH))
                .items_end()
                .pointer_events(PointerEvents::None)
                .child(speech(saying)),
        )
        .into_element()
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
    let length = if reduce_motion() {
        Duration::from_millis(900)
    } else {
        figure.length()
    };
    pet.showing.set(Some(figure));
    let showing = pet.showing;
    // Dropping the handle leaves the timer running (it ends with the owner).
    drop(set_timeout(length, move || {
        if showing.get_untracked() == Some(figure) {
            showing.set(None);
        }
    }));
}

/// What a screen reader hears: its name, and what it is.
fn label_of(view: &PetView) -> String {
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
        assert_eq!(perch(&[], "agent"), None);
        let saved = state(
            r#"{"connections":[{"id":"a","name":"a","initial":"A","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"saved"}],"version":""}"#,
        );
        assert_eq!(perch(&saved, "agent"), Some((Perch::Saved, Pose::Sleeping)));
    }

    #[test]
    fn a_live_host_without_agents_has_it_idle_and_a_failed_one_upset() {
        let live = state(
            r#"{"connections":[{"id":"a","name":"a","initial":"A","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"live"}],"version":""}"#,
        );
        assert_eq!(
            perch(&live, "agent"),
            Some((Perch::Host("a".into()), Pose::Idle))
        );
        let dead = state(
            r#"{"connections":[{"id":"b","name":"b","initial":"B","user":"u","host":"h","mosh":false,"udpPort":60000,"link":"dead"}],"version":""}"#,
        );
        assert_eq!(
            perch(&dead, "agent"),
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
