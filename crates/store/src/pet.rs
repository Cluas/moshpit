//! The companion pet, where the app keeps it (moshpit-design
//! `docs/nsh/PET-DESIGN.md` §15.2, §15.8): in the preferences, as `Data`
//! holding JSON —
//!
//! - `moshpit.pet.v1`: the [`PetRecord`], once it has hatched;
//! - `moshpit.pet.egg.v1`: the egg's [`EggProgress`] before that;
//! - `moshpit.pet.style` and `moshpit.pet.shown`: the style the person
//!   picked (Settings › Appearance) and whether the companion is on.
//!
//! A store from before the pet reads as "no pet yet": an egg. A record
//! this build can't keep (a newer format, or bytes that aren't one) reads
//! as [`Stored::Unreadable`] and is never written over, so an older build
//! can't hatch a second pet over a newer build's. The iCloud copy is the
//! platform's; [`moshpit_pet::reconcile`] says which side to write.
//!
//! Written only while the preferences can be believed ([`Readiness`]): a
//! sealed launch reads them as empty, and an egg written then would stand
//! in for the pet that's really there.

use moshpit_pet::{EggProgress, PetRecord, RecordError, Style};

use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;

/// The record's key.
pub const RECORD_KEY: &str = "moshpit.pet.v1";
/// The egg's key.
pub const EGG_KEY: &str = "moshpit.pet.egg.v1";
/// The style's key (`minimal`, `pixel`, `drawn`).
pub const STYLE_KEY: &str = "moshpit.pet.style";
/// Whether the companion is on.
pub const SHOWN_KEY: &str = "moshpit.pet.shown";
/// Whether it talks (「让它说话」).
pub const TALK_KEY: &str = "moshpit.pet.talk";

/// What the preferences hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// Nothing yet: the egg.
    NoPet,
    Pet(PetRecord),
    /// Something this build can't keep: shown as nothing, never replaced.
    Unreadable(RecordError),
}

/// The pet, as stored.
pub fn read(defaults: &dyn Defaults) -> Stored {
    match defaults.get(RECORD_KEY) {
        None => Stored::NoPet,
        Some(Plist::Data(bytes)) => match PetRecord::from_json(&bytes) {
            Ok(record) => Stored::Pet(record),
            Err(e) => Stored::Unreadable(e),
        },
        Some(_) => Stored::Unreadable(RecordError::Malformed("not data".into())),
    }
}

/// Keeps `record` — unless the preferences can't be believed yet, or what
/// is stored is a record this build can't read. Whether it was written.
pub fn write(defaults: &dyn Defaults, readiness: &Readiness, record: &PetRecord) -> bool {
    if !readiness.is_readable(defaults) || matches!(read(defaults), Stored::Unreadable(_)) {
        return false;
    }
    defaults.set(RECORD_KEY, Some(Plist::Data(record.to_json())));
    true
}

/// The egg's progress; nothing yet when there's none or it doesn't decode.
pub fn read_egg(defaults: &dyn Defaults) -> EggProgress {
    match defaults.get(EGG_KEY) {
        Some(Plist::Data(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => EggProgress::default(),
    }
}

/// Keeps the egg's progress, under the same rule. Whether it was written.
pub fn write_egg(defaults: &dyn Defaults, readiness: &Readiness, egg: &EggProgress) -> bool {
    if !readiness.is_readable(defaults) {
        return false;
    }
    let Ok(bytes) = serde_json::to_vec(egg) else {
        return false;
    };
    defaults.set(EGG_KEY, Some(Plist::Data(bytes)));
    true
}

/// The style picked; pixel until one is (PET-DESIGN §8: new installs and
/// existing ones alike).
pub fn read_style(defaults: &dyn Defaults) -> Style {
    match defaults.get(STYLE_KEY) {
        Some(Plist::String(id)) => Style::from_id(&id).unwrap_or_default(),
        _ => Style::default(),
    }
}

/// Whether the companion is on; on until it's turned off.
pub fn read_shown(defaults: &dyn Defaults) -> bool {
    match defaults.get(SHOWN_KEY) {
        Some(Plist::Bool(on)) => on,
        _ => true,
    }
}

/// Whether it talks; on until it's turned off.
pub fn read_talk(defaults: &dyn Defaults) -> bool {
    match defaults.get(TALK_KEY) {
        Some(Plist::Bool(on)) => on,
        _ => true,
    }
}

/// Keeps whether it talks, under the same rule. Whether written.
pub fn write_talk(defaults: &dyn Defaults, readiness: &Readiness, talk: bool) -> bool {
    if !readiness.is_readable(defaults) {
        return false;
    }
    defaults.set(TALK_KEY, Some(Plist::Bool(talk)));
    true
}

/// Keeps the style and the switch, under the same rule. Whether written.
pub fn write_choice(
    defaults: &dyn Defaults,
    readiness: &Readiness,
    style: Style,
    shown: bool,
) -> bool {
    if !readiness.is_readable(defaults) {
        return false;
    }
    defaults.set(STYLE_KEY, Some(Plist::String(style.id().to_owned())));
    defaults.set(SHOWN_KEY, Some(Plist::Bool(shown)));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;

    fn pet() -> PetRecord {
        PetRecord::hatch([7; 16], 1_800_000_000, 14)
    }

    #[test]
    fn a_store_from_before_the_pet_reads_as_an_egg() {
        let defaults = MemoryDefaults::new();
        defaults.set("moshpit.someOtherSetting", Some(Plist::Bool(true)));
        assert_eq!(read(&defaults), Stored::NoPet);
        assert_eq!(read_egg(&defaults), EggProgress::default());
    }

    #[test]
    fn a_pet_is_kept_and_read_back() {
        let defaults = MemoryDefaults::new();
        let mut record = pet();
        record.rename("Pip", 1_800_000_010);
        record.record_finish("phone");
        assert!(write(&defaults, &Readiness::Always, &record));
        assert_eq!(read(&defaults), Stored::Pet(record));
    }

    #[test]
    fn a_newer_or_broken_record_is_never_written_over() {
        let defaults = MemoryDefaults::new();
        let newer = br#"{"version":2,"seed":"07070707070707070707070707070707","hatched_at":1,"hatch_hour":3,"stage":0}"#;
        defaults.set(RECORD_KEY, Some(Plist::Data(newer.to_vec())));
        assert_eq!(read(&defaults), Stored::Unreadable(RecordError::Newer(2)));
        assert!(!write(&defaults, &Readiness::Always, &pet()));
        assert_eq!(defaults.get(RECORD_KEY), Some(Plist::Data(newer.to_vec())));

        defaults.set(RECORD_KEY, Some(Plist::String("x".into())));
        assert!(matches!(read(&defaults), Stored::Unreadable(_)));
        assert!(!write(&defaults, &Readiness::Always, &pet()));
    }

    #[test]
    fn nothing_is_written_before_the_preferences_can_be_believed() {
        let defaults = MemoryDefaults::new();
        defaults.set_protected_data_available(false);
        assert!(!write(&defaults, &Readiness::System, &pet()));
        assert!(!write_egg(
            &defaults,
            &Readiness::System,
            &EggProgress::default()
        ));
        assert_eq!(read(&defaults), Stored::NoPet);
    }

    #[test]
    fn the_style_is_pixel_and_the_companion_on_until_changed() {
        let defaults = MemoryDefaults::new();
        assert_eq!(
            (read_style(&defaults), read_shown(&defaults)),
            (Style::Pixel, true)
        );
        assert!(write_choice(
            &defaults,
            &Readiness::Always,
            Style::Minimal,
            false
        ));
        assert_eq!(
            (read_style(&defaults), read_shown(&defaults)),
            (Style::Minimal, false)
        );
        defaults.set(STYLE_KEY, Some(Plist::String("neon".into())));
        assert_eq!(read_style(&defaults), Style::Pixel);
        assert!(read_talk(&defaults), "it talks until it's told not to");
        assert!(write_talk(&defaults, &Readiness::System, false));
        assert!(!read_talk(&defaults));
    }

    #[test]
    fn the_egg_remembers_its_progress() {
        let defaults = MemoryDefaults::new();
        let mut egg = read_egg(&defaults);
        egg.connected();
        egg.agent_finished();
        assert!(write_egg(&defaults, &Readiness::Always, &egg));
        assert_eq!(read_egg(&defaults), egg);
        assert!(read_egg(&defaults).ready());
        defaults.set(EGG_KEY, Some(Plist::Data(b"nope".to_vec())));
        assert_eq!(read_egg(&defaults), EggProgress::default());
    }
}
