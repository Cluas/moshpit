//! The system's alert and action sheet (`docs/nsh/UI-DESIGN.md` §4.10):
//! NSH's `alert_dialog`, which the host shows as UIKit's own, with its
//! screen reader, keyboard and iPad anchor. Pages ask through here rather
//! than through `nsh_base`: the `ui` layer is where a page takes a system
//! control from.
//!
//! A question before a delete says the decision in its title
//! ([`delete_title`]), only what is new in its body (what goes, whether it
//! can come back), and offers Cancel and the verb, destructive.

use nsh_core::prelude::*;

pub use nsh_base::{AlertDialogOptions, alert_dialog};

/// The title of the question before deleting `name` ("Delete build-box?").
pub fn delete_title(name: &str) -> String {
    t!("Delete %@?", arg1 = name)
}
