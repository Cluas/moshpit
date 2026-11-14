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
    EggPose, EggProgress, EggView, Look, Moment, PetInfo, PetRecord, PetView, Pose, Seed, Style,
    Thresholds, egg, may_hatch, reconcile,
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
    moment: Option<Moment>,
    /// Counts the moments, so a page plays each once.
    moment_seq: u64,
    thresholds: Thresholds,
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
            ready: s.egg.ready(),
        });
        PetView {
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
        }
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
            moment: None,
            moment_seq: 0,
            thresholds: Thresholds::MEDIUM,
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
            if r.write_cloud {
                if let Some(cloud) = &cloud {
                    cloud.put(record.to_json());
                }
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

    /// A session connected (the egg's third one readies it).
    pub fn connected(&self) {
        let mut s = lock(&self.state);
        if !matches!(s.stored, Stored::NoPet) || s.egg.ready() {
            return;
        }
        s.egg.connected();
        stored::write_egg(&self.defaults, &Readiness::System, &s.egg);
        let ready = s.egg.ready();
        drop(s);
        if ready {
            self.notify();
        }
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
    /// local hour `local_hour`. Only when it may: the egg is ready, iCloud
    /// has settled, the companion is on. Whether it hatched.
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
                let pose = if s.egg.ready() {
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
        m.set_choice("pixel".into(), false);
        assert_eq!(m.seen(t0 + 400 * 86_400), None, "not while it's hidden");
        let again = PetMachine::new(prefs, "phone".into());
        let v = message(&again);
        assert_eq!(
            (
                v["pet"]["name"].as_str(),
                v["pet"]["stage"].as_u64(),
                v["shown"].as_bool()
            ),
            (Some("Pip"), Some(1), Some(false))
        );
    }
}
