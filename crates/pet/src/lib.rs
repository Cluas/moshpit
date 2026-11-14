//! Moshpit's companion pet (moshpit-design `docs/nsh/PET-DESIGN.md`
//! §15–16): one pet per person, hatched once from a secure seed the
//! platform supplies, growing in three stages with a mutation at each, a
//! handful of easter eggs, and a record that merges between devices.
//!
//! - [`record`]: the stored facts and every rule that changes them;
//! - [`genome`]: what a pet is, derived from its seed;
//! - [`easter`]: the five easter eggs as pure rules;
//! - [`sprite`]: a pose of a pet as a [`Sprite`] (cells and a palette per
//!   appearance), its short encoding for the Live Activity, and a crisp
//!   PNG for an image element;
//! - [`view`]: what the screens are told (`{"op": "pet", …}`);
//! - [`watch`]: a connection made and a mosh roam lived through, from the
//!   sessions' events.
//!
//! No platform code: the platform supplies the seed, the clock, the local
//! hour and the device id, and persists the record (`moshpit-store`'s
//! `pet` module).

mod art;
pub mod easter;
pub mod genome;
mod png;
pub mod record;
pub mod sprite;
pub mod view;
pub mod watch;

#[cfg(test)]
mod tests;

pub use easter::Egg;
pub use genome::{
    Eyes, Genome, Growth, Mutation, Pattern, Seed, Size, Slot, Species, Stage, Traits, evolve,
};
pub use record::{
    EggProgress, NextStage, PetRecord, Reconciled, RecordError, Thresholds, may_hatch, merge,
    reconcile,
};
pub use sprite::{Appearance, EggPose, Ink, Look, Pose, Sprite, egg, egg_durations};
pub use view::{EggView, Moment, PetInfo, PetView, Style};
