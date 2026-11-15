//! Who a window's size belongs to when two Moshpit devices share it: the
//! tmux window option `@moshpit_size_owner`, `<client_name>|<label>|<stamp>`
//! (`TmuxSessionController.SizeLease`).
//!
//! tmux's own `window-size latest` can't arbitrate between two `-CC` clients
//! (control traffic never counts as activity), so without a lease each device
//! re-pinned the window to its own grid on every drift, forever. The holder is
//! the last device the person touched; the other follows — keeps its last
//! frame under a veil — until the person touches it. The stamp is the
//! holder's last interaction (0 for a passive claim), so a device coming to
//! the front may take an idle window but never one someone is typing into.

use std::collections::BTreeSet;

use crate::format::{fields, swift_double};

/// One window's lease.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeLease {
    /// tmux `#{client_name}` of the holder — its liveness check.
    pub client_name: String,
    /// What the holder calls itself on the follower's veil ("iPad").
    pub label: String,
    /// Unix time (seconds) of the holder's last interaction; 0 when passive.
    pub stamp: f64,
    /// Whether `client_name` was attached when the lease was read.
    pub live: bool,
}

impl SizeLease {
    /// The option's value read back, against the clients attached at the
    /// time. `None` for anything that is not a lease: unset reads as "", a
    /// harness's generic reply as `0 0`. A listing with no clients at all is
    /// a failed one (we are a client ourselves): the holder counts as live —
    /// following a ghost costs one tap, stealing a window costs a fight.
    pub fn parse(raw: &str, clients: &BTreeSet<String>) -> Option<Self> {
        let parts = fields(raw, b'|', 2, false);
        if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
            return None;
        }
        let stamp = swift_double(parts[2])?;
        Some(Self {
            client_name: parts[0].to_owned(),
            label: parts[1].to_owned(),
            stamp,
            live: clients.is_empty() || clients.contains(parts[0]),
        })
    }

    /// The option value we write: the stamp in whole seconds (`Int(stamp)`,
    /// truncated).
    pub fn encoded(&self) -> String {
        format!("{}|{}|{}", self.client_name, self.label, self.stamp as i64)
    }
}

/// What the controller last learned about a window's lease. No entry in the
/// controller's map: never probed.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaseState {
    Unset,
    Held(SizeLease),
}

/// The active window is sized for another live device: the view dims, keeps
/// its last frame and offers a tap to take the window back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeFollow {
    pub window_id: String,
    /// The holder's label, for the veil ("iPad").
    pub owner_label: String,
}

/// Another client picked another pane in the window the phone shows (a
/// desktop's `select-pane`; the active pane is the window's, shared): the
/// phone keeps its own — by id — under a veil, and takes it back at the
/// person's touch, the last touch winning (B587).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneFollow {
    pub window_id: String,
    /// The pane the phone keeps showing.
    pub pane_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clients(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    // TmuxSizeLeaseParsingTests
    #[test]
    fn unset_and_a_generic_reply_are_not_leases() {
        let a = clients(&["a"]);
        for raw in ["", "0 0", "a|iPad", "|iPad|12", "a||12", "a|iPad|soon"] {
            assert_eq!(SizeLease::parse(raw, &a), None, "{raw:?}");
        }
    }

    #[test]
    fn liveness_is_whether_the_holder_is_attached() {
        assert!(
            SizeLease::parse("b|iPad|12", &clients(&["a", "b"]))
                .unwrap()
                .live
        );
        assert!(
            !SizeLease::parse("b|iPad|12", &clients(&["a"]))
                .unwrap()
                .live
        );
    }

    #[test]
    fn an_empty_client_list_reads_live() {
        assert!(SizeLease::parse("b|iPad|12", &clients(&[])).unwrap().live);
    }

    #[test]
    fn fields_and_encoding_round_trip() {
        let parsed =
            SizeLease::parse("moshpit-b|iPad|1757900000", &clients(&["moshpit-b"])).unwrap();
        assert_eq!(parsed.client_name, "moshpit-b");
        assert_eq!(parsed.label, "iPad");
        assert_eq!(parsed.stamp, 1_757_900_000.0);
        assert_eq!(parsed.encoded(), "moshpit-b|iPad|1757900000");
        let ours = SizeLease {
            client_name: "a".into(),
            label: "iPhone".into(),
            stamp: 12.9,
            live: true,
        };
        assert_eq!(ours.encoded(), "a|iPhone|12");
        // The stamp is everything after the second `|`: a label holding the
        // separator leaves no stamp, so no lease.
        assert_eq!(SizeLease::parse("a|i|Pad|3", &clients(&["a"])), None);
    }
}
