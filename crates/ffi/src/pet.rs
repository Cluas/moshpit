//! The companion pet for the app (`moshpit-pet`, `moshpit-store`'s `pet`;
//! moshpit-design `docs/nsh/PET-DESIGN.md` §15–16), through UniFFI.
//!
//! [`PetMachine`] owns the record and its rules; the app supplies what only
//! a platform has: 16 bytes of its secure random generator at the hatch,
//! the clock and the local hour, a device id, the preferences, and iCloud
//! key-value storage ([`PetCloudHost`]). It tells the app when anything the
//! screens or the island show changed ([`PetListener`]); the app then posts
//! [`PetMachine::message`] to the screens and puts [`PetMachine::island`]
//! in the Live Activity.

use std::sync::{Arc, Mutex, MutexGuard};

use moshpit_pet::{
    EggPose, EggProgress, EggView, IconOffer, Look, Moment, PetInfo, PetRecord, PetView, Pose,
    Seed, Style, Thresholds, egg, icon_id, icon_look, may_hatch, reconcile,
};
use moshpit_store::Readiness;
use moshpit_store::pet::{self as stored, Stored};
use serde_json::json;

use crate::store::{HostDefaults, PreferencesHost};

/// iCloud key-value storage (`NSUbiquitousKeyValueStore`), under
/// `moshpit.pet.v1`: the pet on the person's other devices.
#[uniffi::export(with_foreign)]
pub trait PetCloudHost: Send + Sync {
    /// The record there, if any.
    fn get(&self) -> Option<Vec<u8>>;
    /// Puts the record there.
    fn put(&self, record: Vec<u8>);
}

/// Told when what the screens or the island show changed.
#[uniffi::export(with_foreign)]
pub trait PetListener: Send + Sync {
    fn changed(&self);
}

/// The Live Activity's stills: the pose for the agents' state, and the
/// sleeping one for when the activity goes stale. Each a `px1:` sprite.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PetIsland {
    pub pose: String,
    pub stale: String,
}

struct State {
    stored: Stored,
    egg: EggProgress,
    cloud_settled: bool,
    style: Style,
    shown: bool,
    talk: bool,
    moment: Option<Moment>,
    /// Counts the moments, so a page plays each once.
    moment_seq: u64,
    thresholds: Thresholds,
    /// The app icon as the app last said (B668): the gallery id iOS shows,
    /// and whether the person picked it by hand. None until it says.
    icon: Option<(String, bool)>,
    /// The look whose icon Home last asked about.
    icon_asked: Option<String>,
}

/// The pet, its egg and the person's choices.
#[derive(uniffi::Object)]
pub struct PetMachine {
    defaults: HostDefaults,
    device: String,
    cloud: Mutex<Option<Arc<dyn PetCloudHost>>>,
    listener: Mutex<Option<Arc<dyn PetListener>>>,
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl PetMachine {
    fn notify(&self) {
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener {
            listener.changed();
        }
    }

    /// Keeps the record here and in iCloud.
    fn keep(&self, record: &PetRecord) {
        stored::write(&self.defaults, &Readiness::System, record);
        let cloud = lock(&self.cloud).clone();
        if let Some(cloud) = cloud {
            cloud.put(record.to_json());
        }
    }

    fn view(&self, s: &State) -> PetView {
        let pet = match &s.stored {
            Stored::Pet(record) => Some(PetInfo::of(record, &s.thresholds)),
            _ => None,
        };
        let egg = matches!(s.stored, Stored::NoPet).then(|| EggView {
            ready: s.egg.cracked(),
            taps: s.egg.taps,
        });
        let icon = Self::icon_offer(s, pet.as_ref());
        PetView {
            icon,
            style: s.style,
            shown: s.shown,
            may_hatch: matches!(s.stored, Stored::NoPet)
                && may_hatch(
                    &s.egg,
                    true,
                    s.shown && s.style != Style::Minimal,
                    s.cloud_settled,
                ),
            egg,
            pet,
            moment: s.moment,
            talk: s.talk,
        }
    }

    /// The icon Home offers now (B668): the pet's, once the app has said
    /// which icon it shows, while the companion is shown, when it isn't the
    /// pet's already and this look hasn't been asked about.
    fn icon_offer(s: &State, pet: Option<&PetInfo>) -> Option<IconOffer> {
        let pet = pet?;
        let (current, by_hand) = s.icon.as_ref()?;
        if !s.shown || s.style == Style::Minimal {
            return None;
        }
        let id = icon_id(&pet.species, pet.traits.shiny);
        let look = icon_look(&pet.species, pet.traits.shiny, pet.stage);
        (*current != id && s.icon_asked.as_deref() != Some(look.as_str())).then(|| IconOffer {
            id,
            species: pet.species.clone(),
            keep_default: *by_hand,
        })
    }

    /// The look the pet has now, as an offer names it.
    fn look_now(s: &State) -> Option<String> {
        let Stored::Pet(record) = &s.stored else {
            return None;
        };
        let pet = PetInfo::of(record, &s.thresholds);
        Some(icon_look(&pet.species, pet.traits.shiny, pet.stage))
    }
}

#[uniffi::export]
impl PetMachine {
    /// The pet as this device keeps it. `device_id`: a stable id of this
    /// device (its tasks count under it).
    #[uniffi::constructor]
    pub fn new(preferences: Arc<dyn PreferencesHost>, device_id: String) -> Arc<Self> {
        let defaults = HostDefaults(preferences);
        let state = State {
            stored: stored::read(&defaults),
            egg: stored::read_egg(&defaults),
            cloud_settled: false,
            style: stored::read_style(&defaults),
            shown: stored::read_shown(&defaults),
            talk: stored::read_talk(&defaults),
            moment: None,
            moment_seq: 0,
            thresholds: Thresholds::MEDIUM,
            icon: None,
            icon_asked: stored::read_icon_asked(&defaults),
        };
        Arc::new(Self {
            defaults,
            device: device_id,
            cloud: Mutex::new(None),
            listener: Mutex::new(None),
            state: Mutex::new(state),
        })
    }

    pub fn set_listener(&self, listener: Option<Arc<dyn PetListener>>) {
        *lock(&self.listener) = listener;
    }

    /// iCloud, when the person has it on.
    pub fn set_cloud(&self, cloud: Option<Arc<dyn PetCloudHost>>) {
        *lock(&self.cloud) = cloud;
    }

    /// iCloud has (re)synced: merge what's there with what's here, keep the
    /// result on both sides. The first call settles it, and the egg may
    /// hatch from then on (never a second pet).
    pub fn cloud_changed(&self) {
        let cloud = lock(&self.cloud).clone();
        let remote = cloud
            .as_ref()
            .and_then(|c| c.get())
            .and_then(|b| PetRecord::from_json(&b).ok());
        let mut s = lock(&self.state);
        s.cloud_settled = true;
        let local = match &s.stored {
            Stored::Pet(r) => Some(r.clone()),
            Stored::NoPet => None,
            // Never merged into or over.
            Stored::Unreadable(_) => {
                drop(s);
                return self.notify();
            }
        };
        let r = reconcile(local.as_ref(), remote.as_ref());
        if let Some(record) = &r.record {
            if r.write_local {
                stored::write(&self.defaults, &Readiness::System, record);
                s.stored = Stored::Pet(record.clone());
            }
            if r.write_cloud
                && let Some(cloud) = &cloud
            {
                cloud.put(record.to_json());
            }
        }
        drop(s);
        self.notify();
    }

    /// No iCloud (off, or not signed in): settled without it.
    pub fn cloud_unavailable(&self) {
        lock(&self.state).cloud_settled = true;
        self.notify();
    }

    /// A tap on the egg on Home: the first wobbles it, the second cracks
    /// it, the third hatches it (the screen then asks for the hatch, which
    /// waits for iCloud to settle). Counted in memory only. The taps so far.
    pub fn tap_egg(&self) -> u8 {
        let mut s = lock(&self.state);
        if !matches!(s.stored, Stored::NoPet) {
            return 0;
        }
        let before = s.egg.taps;
        let taps = s.egg.tap();
        drop(s);
        if taps != before {
            self.notify();
        }
        taps
    }

    /// The egg's and the hatch's state in a log line: what a device report
    /// is read from (the host adds whether Home shows).
    pub fn status(&self) -> String {
        let s = lock(&self.state);
        let pet = match &s.stored {
            Stored::Pet(_) => "pet",
            Stored::NoPet => "egg",
            Stored::Unreadable(_) => "unreadable",
        };
        let shown = s.shown && s.style != Style::Minimal;
        format!(
            "{pet} connections={} agent_finished={} taps={} cloud_settled={} shown={shown} may_hatch={}",
            s.egg.connections,
            s.egg.agent_finished,
            s.egg.taps,
            s.cloud_settled,
            matches!(s.stored, Stored::NoPet) && may_hatch(&s.egg, true, shown, s.cloud_settled),
        )
    }

    /// A session connected (the egg's third one readies it).
    pub fn connected(&self) {
        let mut s = lock(&self.state);
        if !matches!(s.stored, Stored::NoPet) || s.egg.ready() {
            return;
        }
        s.egg.connected();
        stored::write_egg(&self.defaults, &Readiness::System, &s.egg);
        drop(s);
        // Told every time, for the log line (the screens see no change
        // until it's ready).
        self.notify();
    }

    /// An agent finished a turn: the egg cracks, or the pet counts it.
    pub fn agent_finished(&self) {
        let mut s = lock(&self.state);
        match &mut s.stored {
            Stored::Pet(record) => {
                record.record_finish(&self.device);
                let record = record.clone();
                drop(s);
                self.keep(&record);
            }
            Stored::NoPet => {
                if s.egg.agent_finished {
                    return;
                }
                s.egg.agent_finished();
                stored::write_egg(&self.defaults, &Readiness::System, &s.egg);
                drop(s);
            }
            Stored::Unreadable(_) => return,
        }
        self.notify();
    }

    /// A mosh session roamed and kept going.
    pub fn roamed(&self) {
        let mut s = lock(&self.state);
        let Stored::Pet(record) = &mut s.stored else {
            return;
        };
        if record.record_roam().is_none() {
            return;
        }
        let record = record.clone();
        drop(s);
        self.keep(&record);
        self.notify();
    }

    /// The hatch, from 16 secure random bytes, at `now` (unix seconds) and
    /// local hour `local_hour`. Only when it may: the egg is tapped three
    /// times or ready by itself, iCloud has settled, the companion is on.
    /// Whether it hatched.
    pub fn hatch(&self, seed: Vec<u8>, now: i64, local_hour: u8) -> bool {
        let Ok(seed) = Seed::try_from(seed.as_slice()) else {
            return false;
        };
        let mut s = lock(&self.state);
        if !matches!(s.stored, Stored::NoPet)
            || !may_hatch(
                &s.egg,
                true,
                s.shown && s.style != Style::Minimal,
                s.cloud_settled,
            )
        {
            return false;
        }
        let record = PetRecord::hatch(seed, now, local_hour);
        s.stored = Stored::Pet(record.clone());
        s.moment = Some(Moment::Hatched);
        s.moment_seq += 1;
        drop(s);
        self.keep(&record);
        self.notify();
        true
    }

    /// Home shows the pet now: it grows if it's due. The stage it grew to.
    pub fn seen(&self, now: i64) -> Option<u8> {
        let mut s = lock(&self.state);
        let shown = s.shown && s.style != Style::Minimal;
        let thresholds = s.thresholds;
        let Stored::Pet(record) = &mut s.stored else {
            return None;
        };
        let stage = record.grow(now, &thresholds, shown)?;
        let record = record.clone();
        s.moment = Some(Moment::Grew);
        s.moment_seq += 1;
        drop(s);
        self.keep(&record);
        self.notify();
        Some(stage as u8)
    }

    /// Renames it at `now`; an empty name gives back the species' name.
    pub fn rename(&self, name: String, now: i64) {
        let mut s = lock(&self.state);
        let Stored::Pet(record) = &mut s.stored else {
            return;
        };
        record.rename(&name, now);
        let record = record.clone();
        drop(s);
        self.keep(&record);
        self.notify();
    }

    /// The style (`minimal`, `pixel`; `drawn` isn't offered yet) and the
    /// companion's switch.
    pub fn set_choice(&self, style: String, shown: bool) {
        let mut s = lock(&self.state);
        let style = match Style::from_id(&style) {
            Some(Style::Drawn) | None => s.style,
            Some(style) => style,
        };
        if (s.style, s.shown) == (style, shown) {
            return;
        }
        s.style = style;
        s.shown = shown;
        stored::write_choice(&self.defaults, &Readiness::System, style, shown);
        drop(s);
        self.notify();
    }

    /// Whether it talks on Home (「让它说话」).
    pub fn set_talk(&self, talk: bool) {
        let mut s = lock(&self.state);
        if s.talk == talk {
            return;
        }
        s.talk = talk;
        stored::write_talk(&self.defaults, &Readiness::System, talk);
        drop(s);
        self.notify();
    }

    /// How fast it grows: `faster`, `medium` (the default) or `slower`.
    pub fn set_pace(&self, pace: String) {
        let thresholds = match pace.as_str() {
            "faster" => Thresholds::FASTER,
            "slower" => Thresholds::SLOWER,
            _ => Thresholds::MEDIUM,
        };
        lock(&self.state).thresholds = thresholds;
        self.notify();
    }

    /// The app icon iOS shows now (B668): its gallery id, and whether the
    /// person picked it by hand. Home offers the pet's icon only once the
    /// app has said.
    pub fn set_icon(&self, current: String, by_hand: bool) {
        let mut s = lock(&self.state);
        if s.icon.as_ref() == Some(&(current.clone(), by_hand)) {
            return;
        }
        s.icon = Some((current, by_hand));
        drop(s);
        self.notify();
    }

    /// The person answered Home's icon card (B668), either way: this look
    /// is not asked about again; the next one is.
    pub fn icon_answered(&self) {
        let mut s = lock(&self.state);
        let Some(look) = Self::look_now(&s) else {
            return;
        };
        stored::write_icon_asked(&self.defaults, &Readiness::System, &look);
        s.icon_asked = Some(look);
        drop(s);
        self.notify();
    }

    /// The screens' message: `{"op": "pet", …}` ([`PetView`]), with `seq`
    /// counting the moments so a page plays each once.
    pub fn message(&self) -> String {
        let s = lock(&self.state);
        let mut value = serde_json::to_value(self.view(&s)).unwrap_or_else(|_| json!({}));
        value["op"] = json!("pet");
        value["seq"] = json!(s.moment_seq);
        value.to_string()
    }

    /// The Live Activity's stills for the agents' headline state (`working`,
    /// `attention`, `done`, `idle`), or none while the companion is off.
    pub fn island(&self, state: String) -> Option<PetIsland> {
        let s = lock(&self.state);
        if !s.shown || s.style == Style::Minimal {
            return None;
        }
        match &s.stored {
            Stored::NoPet => {
                let pose = if s.egg.cracked() {
                    EggPose::Ready
                } else {
                    EggPose::Still
                };
                let sprite = egg(pose, 0)?.encode();
                Some(PetIsland {
                    pose: sprite.clone(),
                    stale: sprite,
                })
            }
            Stored::Pet(record) => {
                let look: Look = record.look();
                let pose = match state.as_str() {
                    "working" => Pose::Working,
                    "attention" => Pose::Waiting,
                    "done" => Pose::Done,
                    _ => Pose::Idle,
                };
                Some(PetIsland {
                    pose: look.island(pose)?,
                    stale: look.island(Pose::Sleeping)?,
                })
            }
            Stored::Unreadable(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::store::PlistValue;

    #[derive(Default)]
    struct Prefs(Mutex<HashMap<String, PlistValue>>);

    impl PreferencesHost for Prefs {
        fn get(&self, key: String) -> Option<PlistValue> {
            lock(&self.0).get(&key).cloned()
        }
        fn set(&self, key: String, value: Option<PlistValue>) {
            match value {
                Some(v) => lock(&self.0).insert(key, v),
                None => lock(&self.0).remove(&key),
            };
        }
        fn protected_data_available(&self) -> bool {
            true
        }
    }

    #[derive(Default)]
    struct Cloud(Mutex<Option<Vec<u8>>>);

    impl PetCloudHost for Cloud {
        fn get(&self) -> Option<Vec<u8>> {
            lock(&self.0).clone()
        }
        fn put(&self, record: Vec<u8>) {
            *lock(&self.0) = Some(record);
        }
    }

    #[derive(Default)]
    struct Count(Mutex<u32>);

    impl PetListener for Count {
        fn changed(&self) {
            *lock(&self.0) += 1;
        }
    }

    fn message(m: &PetMachine) -> serde_json::Value {
        serde_json::from_str(&m.message()).unwrap()
    }

    #[test]
    fn an_egg_hatches_once_after_icloud_settles_and_both_devices_get_the_same_pet() {
        let cloud = Arc::new(Cloud::default());
        let phone = PetMachine::new(Arc::new(Prefs::default()), "phone".into());
        let listener = Arc::new(Count::default());
        phone.set_listener(Some(listener.clone()));
        phone.set_cloud(Some(cloud.clone()));
        assert_eq!(message(&phone)["egg"]["ready"], false);
        phone.agent_finished();
        assert_eq!(message(&phone)["egg"]["ready"], true);
        assert!(
            !phone.hatch(vec![1; 16], 1_800_000_000, 14),
            "not before iCloud settles"
        );
        phone.cloud_changed();
        assert_eq!(message(&phone)["may_hatch"], true);
        assert!(!phone.hatch(vec![1; 15], 1_800_000_000, 14), "16 bytes");
        assert!(phone.hatch(vec![1; 16], 1_800_000_000, 14));
        assert!(
            !phone.hatch(vec![2; 16], 1_800_000_001, 14),
            "no second pet"
        );
        let m = message(&phone);
        assert_eq!(
            (m["moment"].as_str(), m["seq"].as_u64()),
            (Some("hatched"), Some(1))
        );
        assert!(m["egg"].is_null() && m["pet"]["seed"] == "01010101010101010101010101010101");
        assert!(*lock(&listener.0) >= 3);

        // The iPad, with iCloud: it adopts the phone's pet, no egg.
        let ipad = PetMachine::new(Arc::new(Prefs::default()), "ipad".into());
        ipad.set_cloud(Some(cloud.clone()));
        ipad.cloud_changed();
        assert_eq!(message(&ipad)["pet"]["seed"], m["pet"]["seed"]);
        ipad.agent_finished();
        phone.cloud_changed();
        assert_eq!(
            message(&phone)["pet"]["tasks"],
            1,
            "the iPad's task reached the phone"
        );
    }

    /// B668, the owner's call: after the hatch Home asks whether to make
    /// the pet the app icon (「把 App 图标换成墩墩吗？」 换 / 先不换), and again
    /// when it grows into a new look; nothing switches without a yes. An
    /// icon picked by hand is still asked about, 「先不换」 the default.
    #[test]
    fn the_pet_offers_its_icon_once_a_look_and_never_switches_it() {
        let prefs = Arc::new(Prefs::default());
        let m = PetMachine::new(prefs.clone(), "phone".into());
        m.cloud_unavailable();
        m.agent_finished();
        assert!(m.hatch(vec![1; 16], 1_800_000_000, 14));
        assert!(
            message(&m)["icon"].is_null(),
            "not before the app says which icon it shows"
        );
        m.set_icon("default".into(), false);
        let pet = message(&m)["pet"].clone();
        let species = pet["species"].as_str().unwrap().to_owned();
        let id = icon_id(&species, pet["traits"]["shiny"].as_bool().unwrap());
        let offer = message(&m)["icon"].clone();
        assert_eq!(
            (
                offer["id"].as_str(),
                offer["species"].as_str(),
                offer["keep_default"].as_bool()
            ),
            (Some(id.as_str()), Some(species.as_str()), Some(false))
        );
        // The pet's icon already: nothing to ask.
        m.set_icon(id.clone(), false);
        assert!(message(&m)["icon"].is_null());
        // 先不换: this look isn't asked about again, after a relaunch either.
        m.set_icon("pit".into(), true);
        assert!(!message(&m)["icon"].is_null());
        m.icon_answered();
        assert!(message(&m)["icon"].is_null());
        let relaunched = PetMachine::new(prefs.clone(), "phone".into());
        relaunched.cloud_unavailable();
        relaunched.set_icon("pit".into(), true);
        assert!(message(&relaunched)["icon"].is_null(), "asked once a look");
        // It grows into a new look: asked again; picked by hand, 先不换 first.
        for _ in 0..10 {
            relaunched.agent_finished();
        }
        assert!(
            relaunched.seen(1_800_000_000 + 4 * 86_400).is_some(),
            "it grew"
        );
        let offer = message(&relaunched)["icon"].clone();
        assert_eq!(
            (offer["id"].as_str(), offer["keep_default"].as_bool()),
            (Some(id.as_str()), Some(true))
        );
        // The companion off: no card.
        relaunched.set_choice("pixel".into(), false);
        assert!(message(&relaunched)["icon"].is_null());
    }

    #[test]
    fn three_taps_hatch_the_egg_the_second_cracks_it_and_the_hatch_waits_for_icloud() {
        let m = PetMachine::new(Arc::new(Prefs::default()), "phone".into());
        let listener = Arc::new(Count::default());
        m.set_listener(Some(listener.clone()));
        let egg = |m: &PetMachine| message(m)["egg"].clone();
        // Before the hatch the island shows the egg, whole, then cracked.
        let still = |pose| moshpit_pet::egg(pose, 0).unwrap().encode();
        let island = |m: &PetMachine| m.island("working".into()).unwrap();
        assert_eq!(island(&m).pose, still(EggPose::Still));
        assert_eq!(m.tap_egg(), 1);
        assert_eq!(
            (egg(&m)["ready"].as_bool(), egg(&m)["taps"].as_u64()),
            (Some(false), Some(1))
        );
        assert_eq!(m.tap_egg(), 2);
        assert_eq!(egg(&m)["ready"], true, "the second tap cracks it");
        assert_eq!(island(&m).pose, still(EggPose::Ready));
        assert_eq!(m.tap_egg(), 3);
        // iCloud hasn't settled: no hatch yet, however often it's asked.
        assert_eq!(message(&m)["may_hatch"], false);
        assert!(!m.hatch(vec![4; 16], 1_800_000_000, 9));
        assert!(
            m.status().contains("taps=3 cloud_settled=false"),
            "{}",
            m.status()
        );
        // Once it settles, the screen sees it may, and asks.
        m.cloud_unavailable();
        assert_eq!(message(&m)["may_hatch"], true);
        assert!(m.hatch(vec![4; 16], 1_800_000_000, 9));
        assert_eq!(message(&m)["moment"], "hatched");
        assert_ne!(island(&m).pose, still(EggPose::Ready), "the pet now");
        assert!(message(&m)["egg"].is_null());
        assert_eq!(m.tap_egg(), 0, "a pet isn't an egg");
        assert!(*lock(&listener.0) >= 5, "each tap told");
        // A relaunch starts the taps over; nothing else was kept for them.
        let fresh = PetMachine::new(Arc::new(Prefs::default()), "phone".into());
        fresh.cloud_unavailable();
        fresh.tap_egg();
        fresh.tap_egg();
        assert_eq!(
            message(&fresh)["may_hatch"],
            false,
            "two taps don't hatch it"
        );
    }

    #[test]
    fn an_egg_ready_by_itself_still_hatches_with_no_tap() {
        let prefs = Arc::new(Prefs::default());
        let m = PetMachine::new(prefs.clone(), "phone".into());
        m.cloud_unavailable();
        for _ in 0..2 {
            m.connected();
        }
        assert_eq!(message(&m)["may_hatch"], false);
        m.connected();
        assert_eq!(message(&m)["egg"]["ready"], true, "the third connection");
        assert_eq!(message(&m)["egg"]["taps"], 0);
        assert_eq!(message(&m)["may_hatch"], true);
        assert!(
            m.status()
                .starts_with("egg connections=3 agent_finished=false taps=0")
        );
        assert!(m.hatch(vec![5; 16], 1_800_000_000, 9));
        assert!(m.status().starts_with("pet "));
    }

    #[test]
    fn the_island_gets_a_pose_and_a_sleeping_still_and_nothing_while_off() {
        let m = PetMachine::new(Arc::new(Prefs::default()), "phone".into());
        let egg_island = m.island("working".into()).unwrap();
        assert!(egg_island.pose.starts_with("px1:"));
        m.agent_finished();
        m.cloud_unavailable();
        assert!(m.hatch(vec![9; 16], 1_800_000_000, 10));
        let island = m.island("attention".into()).unwrap();
        assert_ne!(island.pose, island.stale);
        assert!(island.pose.len() < 600 && island.stale.len() < 600);
        m.set_choice("minimal".into(), true);
        assert_eq!(m.island("working".into()), None);
        m.set_choice("drawn".into(), false);
        assert_eq!(
            message(&m)["style"],
            "minimal",
            "hand-drawn isn't offered yet"
        );
        assert_eq!(message(&m)["shown"], false);
    }

    #[test]
    fn it_grows_when_seen_and_the_choice_and_name_are_kept() {
        let prefs = Arc::new(Prefs::default());
        let m = PetMachine::new(prefs.clone(), "phone".into());
        m.agent_finished();
        m.cloud_unavailable();
        let t0 = 1_800_000_000;
        assert!(m.hatch(vec![3; 16], t0, 10));
        for _ in 0..10 {
            m.agent_finished();
        }
        assert_eq!(m.seen(t0 + 86_400), None, "three days");
        assert_eq!(m.seen(t0 + 3 * 86_400), Some(1));
        assert_eq!(message(&m)["moment"], "grew");
        m.rename("Pip".into(), t0 + 10);
        assert_eq!(message(&m)["talk"], true);
        m.set_talk(false);
        m.set_choice("pixel".into(), false);
        assert_eq!(m.seen(t0 + 400 * 86_400), None, "not while it's hidden");
        let again = PetMachine::new(prefs, "phone".into());
        let v = message(&again);
        assert_eq!(
            (
                v["pet"]["name"].as_str(),
                v["pet"]["stage"].as_u64(),
                v["shown"].as_bool(),
                v["talk"].as_bool()
            ),
            (Some("Pip"), Some(1), Some(false), Some(false))
        );
    }
}
