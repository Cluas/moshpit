//! What the pet hears from the sessions, decided from their events: a
//! connection made (the egg's third readies it) and a mosh roam the
//! session lived through (the travel badge, PET-DESIGN §15.7).
//!
//! A connection is a session going from connecting to connected — the
//! person's own connect, not an automatic recovery. A roam is mosh finding
//! a better network path while the session is connected, and the path
//! settling again with the session still connected: had it dropped in
//! between (reconnecting, failed), it didn't live through it.

use std::collections::HashMap;

/// A session's state, as far as the pet cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    /// Reconnecting, ended or failed.
    Down,
}

/// What it heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    Connected,
    Roamed,
}

#[derive(Debug, Default, Clone, Copy)]
struct Session {
    link: Option<Link>,
    roaming: bool,
}

/// Per-session memory of the two signals.
#[derive(Debug, Default)]
pub struct SessionWatch {
    sessions: HashMap<u64, Session>,
}

impl SessionWatch {
    /// A session's state changed.
    pub fn state(&mut self, session: u64, link: Link) -> Option<Heard> {
        let s = self.sessions.entry(session).or_default();
        let before = s.link.replace(link);
        if link != Link::Connected {
            s.roaming = false;
        }
        (before == Some(Link::Connecting) && link == Link::Connected).then_some(Heard::Connected)
    }

    /// mosh said whether it's roaming onto a better path.
    pub fn roaming(&mut self, session: u64, roaming: bool) -> Option<Heard> {
        let s = self.sessions.entry(session).or_default();
        if s.link != Some(Link::Connected) {
            s.roaming = false;
            return None;
        }
        let was = std::mem::replace(&mut s.roaming, roaming);
        (was && !roaming).then_some(Heard::Roamed)
    }

    /// The session is gone.
    pub fn forget(&mut self, session: u64) {
        self.sessions.remove(&session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connect_is_heard_once_and_a_recovery_is_not_a_connect() {
        let mut w = SessionWatch::default();
        assert_eq!(w.state(1, Link::Connecting), None);
        assert_eq!(w.state(1, Link::Connected), Some(Heard::Connected));
        assert_eq!(w.state(1, Link::Connected), None);
        assert_eq!(w.state(1, Link::Down), None);
        assert_eq!(w.state(1, Link::Connected), None, "a reconnect on its own");
        assert_eq!(w.state(2, Link::Connected), None, "no connecting seen");
    }

    #[test]
    fn a_roam_counts_only_when_the_session_stays_up_through_it() {
        let mut w = SessionWatch::default();
        w.state(1, Link::Connecting);
        w.state(1, Link::Connected);
        assert_eq!(w.roaming(1, true), None);
        assert_eq!(w.roaming(1, false), Some(Heard::Roamed));
        assert_eq!(w.roaming(1, false), None);
        w.roaming(1, true);
        w.state(1, Link::Down);
        w.state(1, Link::Connected);
        assert_eq!(w.roaming(1, false), None, "it dropped in between");
        w.forget(1);
        assert_eq!(w.roaming(1, true), None, "not connected as far as it knows");
    }
}
