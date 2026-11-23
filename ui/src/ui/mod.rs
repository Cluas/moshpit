//! The app's `ui` layer (`docs/nsh/UI-DESIGN.md` §4): what every screen of
//! the redesign is built from. Screens compose these and never style:
//! colors, sizes, type and motion come from here.
//!
//! - [`tokens`]: the semantic color and number tokens, light and dark (R0).
//! - [`typography`]: the type scale and the section title (R1).
//! - [`motion`]: durations, easings, the structural spring, distances and
//!   scales, each resting under reduced motion (R1).
//! - [`icon`]: SF Symbols in three sizes and the tokens' tones (R2).
//! - [`button`]: [`button::Button`] and [`button::IconButton`] (R2).
//! - [`status`]: [`status::Badge`] and [`status::StatusDot`] (R2).
//! - [`feedback`]: the spinner and the hairline separator (R2), and the
//!   progress bar (R17).
//! - [`row`]: [`row::Row`], the list row on fixed lanes (R3).
//! - [`section`]: [`section::Section`], rows on a card with their
//!   separators, a header and a footer (R3).
//! - [`control`]: the system's switch, segmented control and a select row
//!   that opens its menu, in the tokens' colors (R3).
//! - [`empty`]: [`empty::Empty`], a page or a group with nothing in it,
//!   and the skeleton that stands in for rows on their way (R3).
//! - [`notice`]: [`notice::Banner`], what holds for the task at hand, and
//!   the toasts that say briefly how something went (R3).
//! - [`field`]: [`field::Field`] with a [`field::TextInput`] or a
//!   [`field::NumberInput`], and [`field::SecretField`] (R3); a field made
//!   again when its text is set for it (R17).
//! - [`key`]: [`key::Key`], the shortcut bar's keycap, and [`key::Chip`],
//!   a small choice in Settings (R3).
//! - [`swatch`]: [`swatch::Swatch`], a named color choice, and
//!   [`swatch::CursorColor`], the cursor's four (B318) (R3); the system's
//!   color well (R17).
//! - [`page`]: a page of sections under a bar, scrolling under it.
//! - [`mark`]: [`mark::HostMark`], a host's initial on a disc with its
//!   status on the corner, and an agent's glyph (R6).
//! - [`alert`]: the system's alert and action sheet, and the title of a
//!   question before a delete (R17).
//! - [`menu`]: the system's menu, what a ⋯, a bar button or a title opens
//!   (R17).
//! - [`gallery`]: every component, in every variant and state, on one
//!   debug screen (`ui-gallery`).

pub mod alert;
pub mod bubble;
pub mod button;
pub mod companion;
pub mod control;
pub mod document;
pub mod empty;
pub mod feedback;
pub mod field;
pub mod gallery;
pub mod icon;
pub mod key;
pub mod mark;
pub mod menu;
pub mod motion;
pub mod notice;
pub mod page;
pub mod row;
pub mod section;
pub mod status;
pub mod swatch;
pub mod tokens;
pub mod typography;
