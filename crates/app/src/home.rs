//! Home's record (`moshpit_ui::home::model::HomeState`, the `{"op":
//! "home"}` message), built once for every platform from what the Rust
//! session layer says.
//!
//! Each platform built it on its own until now: iOS in Swift
//! (`HomeSnapshot` in `HomeNSHPage.swift`), from its session objects —
//! which on the Rust session path were copies of this same layer's events —
//! and Android from its own aggregate of them (`crates/android`'s `Conn`).
//! Here it reads the registry's [`SessionView`] of each saved connection's
//! session (its state, the tmux or herdr tree and stamps, when it went live
//! and when it was lost) and the agents monitor's clock, and the host adds
//! only what is its own: which connections are saved, which one it is
//! disconnecting, the version line, whether Home is the iPad's sidebar, the
//! language the app shows.
//!
//! On the same session it says what the Swift builder said: the
//! cross-check (`harness/home/`) runs both over one fixture. Where
//! Android's builder differed, Swift's rule is the one kept:
//! - a pane's agent stamp has a time on every multiplexer: its own when
//!   it carries one (tmux's `@moshpit_since`), else the monitor's — Android
//!   sent none for herdr;
//! - a card's initial is its name's first character as a person counts
//!   characters (Swift's `prefix(1)`), not its first code point;
//! - every list is in id order, herdr's too;
//! - the herdr version skew is said in the app's language.
//!
//! What changed is where a card's flags live: in the session's view, not
//! in iOS's metrics registry, which stamped them once a connect finished
//! and kept them past it — a dead card no longer says how long it was up,
//! a disconnected one is never degraded, and a fall back to SSH on a
//! reconnect shows.
//!
//! Only the Rust session path is read: on iOS Home in NSH requires it (with
//! the session switch off, the SwiftUI Home shows).

use std::collections::BTreeMap;

use moshpit_session::{
    Cause, Degrade, Failure, FailureKind, HerdrView, SessionState, SessionView, TmuxView,
    UnreachableWhy,
};
use moshpit_ui::home::model::{
    Connection, ConnectionFailure, Control, FailureReason, HomeState, Hook, Link, Mismatch, Mux,
    PaneInfo, SessionInfo, Transport, WindowInfo, Worktree,
};
use moshpit_ui::home::{Message, words};
use moshpit_ui::ui::{mark::Face, tokens::FACES};
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

/// One saved connection, as the host's store has it, and its session.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedConnection {
    /// Its id, as the store writes it (an upper-case UUID string): what the
    /// page's intents name and the agents monitor keys panes by.
    pub id: String,
    /// The name the store displays it by (its name, else its host).
    pub name: String,
    /// The fingerprint of its host key as trusted (`SHA256:…`, the known
    /// hosts' entry for its SSH port); none before a key is trusted.
    pub fingerprint: Option<String>,
    /// The emoji picked for it, which its mark shows in place of its face.
    pub emoji: Option<String>,
    pub user: String,
    pub host: String,
    pub mosh: bool,
    /// The first port of its mosh UDP range.
    pub udp_port: u32,
    /// Its multiplexer is tmux: a live session whose tree has never
    /// attached is attaching.
    pub tmux: bool,
    /// The host is disconnecting it (its card shows a spinner).
    pub disconnecting: bool,
    /// The registry's view of its session, when it has one.
    pub session: Option<SessionView>,
}

/// Everything Home is built from.
pub struct HomeInput<'a> {
    /// In the store's order.
    pub connections: &'a [SavedConnection],
    /// The agents monitor's clock for a connection (its id): when each of
    /// its panes entered the state it is in, in Unix seconds. Empty where
    /// the platform runs no monitor.
    pub since: &'a dyn Fn(&str) -> BTreeMap<String, f64>,
    /// The footer's version line.
    pub version: &'a str,
    /// Home is the iPad's sidebar.
    pub sidebar: bool,
    /// The language the app shows (a BCP 47 tag).
    pub locale: &'a str,
}

/// What Home shows now.
pub fn state(input: &HomeInput) -> HomeState {
    HomeState {
        connections: input
            .connections
            .iter()
            .map(|saved| connection(saved, input))
            .collect(),
        version: input.version.to_owned(),
        sidebar: input.sidebar,
    }
}

/// `{"op": "home", …}`.
pub fn message(input: &HomeInput) -> String {
    Message::Home(Box::new(state(input))).to_json()
}

fn connection(saved: &SavedConnection, input: &HomeInput) -> Connection {
    let view = saved.session.as_ref();
    Connection {
        id: saved.id.clone(),
        name: saved.name.clone(),
        initial: initial(&saved.name),
        face: saved.fingerprint.as_deref().map(face),
        emoji: saved.emoji.clone(),
        user: saved.user.clone(),
        host: saved.host.clone(),
        mosh: saved.mosh,
        udp_port: saved.udp_port,
        link: view.map_or(Link::Saved, |v| {
            let tree = v.tmux_view.as_ref();
            link(
                &v.state,
                saved.tmux,
                tree.map(|t| &t.snapshot),
                v.attach_stalled,
                tree.is_some_and(|t| t.no_session),
            )
        }),
        transport: view.map_or(Transport::Live, |v| transport(&v.state)),
        degraded: view.is_some_and(|v| v.degrade == Some(Degrade::MoshServerMissing)),
        connected_at: view.and_then(|v| v.connected_at),
        roaming: view.is_some_and(|v| v.mosh_roaming),
        disconnecting: saved.disconnecting,
        control: view.and_then(|v| control(v, &(input.since)(&saved.id), input.locale)),
        lost_at: view.and_then(|v| v.lost_at),
        failure: view.and_then(|v| match &v.state {
            SessionState::Failed(failure) => Some(failure_of(failure)),
            _ => None,
        }),
    }
}

/// A card's initial: its name's first character as a person counts them,
/// upper-cased (`String(displayName.prefix(1)).uppercased()`).
pub fn initial(name: &str) -> String {
    name.graphemes(true)
        .next()
        .map(str::to_uppercase)
        .unwrap_or_default()
}

/// A trusted host key's face (§4.13): the one seam from a fingerprint, as
/// the known hosts keep it, to the [`Face`] a mark draws. Whatever makes
/// faces lives behind it (another generator, a pixel creature, can take its
/// place); the store, the emoji over it, the mark's sizes and "no key, the
/// initial" don't change with it.
///
/// Today's: from the SHA-256 of the fingerprint, a 5 × 5 grid mirrored left
/// to right, the digest's first fifteen bits its left three columns (bit
/// `row * 3 + column`), its last byte the hue. A key with no cell set (one
/// in 32768) has them all. The same key, the same face; another key,
/// another face.
pub fn face(fingerprint: &str) -> Face {
    const SIDE: usize = 5;
    let digest = Sha256::digest(fingerprint.as_bytes());
    let bits = match u16::from_le_bytes([digest[0], digest[1]]) & 0x7FFF {
        0 => 0x7FFF,
        bits => bits,
    };
    let cells = (0..SIDE * SIDE)
        .map(|at| {
            let (row, column) = (at / SIDE, at % SIDE);
            let column = column.min(SIDE - 1 - column);
            (bits >> (row * 3 + column)) & 1 == 1
        })
        .collect();
    Face {
        side: SIDE as u8,
        cells,
        hue: digest[31] % FACES.len() as u8,
    }
}

/// Where a session is (`HomeSnapshot.link`): dead, then connecting, then
/// an attach that hasn't happened or stalled, then live. `tmux_expected`:
/// the connection's multiplexer is tmux; `tmux`: its tree, once the
/// control plane runs; `no_session`: its server was found with no session
/// (`TmuxView::no_session`).
pub fn link(
    state: &SessionState,
    tmux_expected: bool,
    tmux: Option<&moshpit_tmux::model::Snapshot>,
    stalled: bool,
    no_session: bool,
) -> Link {
    match state {
        SessionState::Failed(_) | SessionState::Ended(_) => return Link::Dead,
        SessionState::Connecting => return Link::Connecting,
        SessionState::Reconnecting => return Link::Reconnecting,
        SessionState::Connected => {}
    }
    // No attach under way and none stalled: the server waits for the
    // person's first session (B228) — live, its empty tree and its +.
    if no_session {
        return Link::Live;
    }
    if tmux_expected
        && let Some(tmux) = tmux
        && !tmux.is_attached
        && !tmux.ever_attached
        && !stalled
    {
        return Link::Attaching;
    }
    if stalled { Link::Stalled } else { Link::Live }
}

/// The transport's state as every surface shows it
/// (`TerminalViewModel.connState`): the session's state already folds an
/// automatic reconnect into reconnecting.
pub fn transport(state: &SessionState) -> Transport {
    match state {
        SessionState::Connecting => Transport::Connecting,
        SessionState::Reconnecting => Transport::Reconnecting,
        SessionState::Failed(_) | SessionState::Ended(_) => Transport::Offline,
        SessionState::Connected => Transport::Live,
    }
}

/// Why a session failed, as the screen words it.
pub fn failure_of(failure: &Failure) -> ConnectionFailure {
    let reason = match (&failure.kind, &failure.cause) {
        (FailureKind::Credentials, _) => FailureReason::Auth,
        (FailureKind::HostKey, Cause::HostKey { changed: true, .. }) => {
            FailureReason::HostKeyChanged
        }
        (FailureKind::HostKey, _) => FailureReason::HostKey,
        (
            FailureKind::Unreachable,
            Cause::Unreachable {
                why: UnreachableWhy::TimedOut,
                ..
            },
        ) => FailureReason::Timeout,
        (FailureKind::Unreachable, _) => FailureReason::Unreachable,
        (
            FailureKind::Handshake,
            Cause::Handshake {
                timed_out: true, ..
            },
        ) => FailureReason::Timeout,
        (FailureKind::Proxy, _) => FailureReason::Proxy,
        _ => FailureReason::Other,
    };
    ConnectionFailure {
        reason,
        detail: failure.log.clone(),
    }
}

/// The session's multiplexer, tmux's before herdr's
/// (`HomeSnapshot.control`).
fn control(view: &SessionView, since: &BTreeMap<String, f64>, locale: &str) -> Option<Control> {
    if let Some(tmux) = &view.tmux_view {
        return Some(tmux_control(tmux, since));
    }
    view.herdr
        .as_ref()
        .map(|herdr| herdr_control(herdr, since, locale))
}

fn sorted<T>(items: impl Iterator<Item = T>, id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut items: Vec<T> = items.collect();
    items.sort_by(|a, b| id(a).cmp(id(b)));
    items
}

fn tmux_control(tmux: &TmuxView, since: &BTreeMap<String, f64>) -> Control {
    let snapshot = &tmux.snapshot;
    Control {
        mux: Mux::Tmux,
        attached: snapshot.is_attached,
        refreshing: tmux.refreshing,
        sessions: sorted(snapshot.sessions.values(), |s| &s.id)
            .into_iter()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect(),
        windows: sorted(snapshot.windows.values(), |w| &w.id)
            .into_iter()
            .map(|w| WindowInfo {
                id: w.id.clone(),
                session: w.session_id.clone(),
                index: w.index,
                name: w.name.clone(),
                pane_count: w.pane_count,
            })
            .collect(),
        panes: sorted(snapshot.panes.values(), |p| &p.id)
            .into_iter()
            .map(|p| PaneInfo {
                id: p.id.clone(),
                window: p.window_id.clone(),
                index: p.index,
                command: p.command.clone(),
                path: p.path.clone(),
            })
            .collect(),
        active_session: snapshot.active_session_id.clone(),
        active_window: snapshot.active_window_id.clone(),
        active_pane: snapshot.active_pane_id.clone(),
        hooks: sorted(tmux.agent_hooks.iter(), |(pane, _)| pane.as_str())
            .into_iter()
            .map(|(pane, hook)| Hook {
                pane: pane.clone(),
                state: hook.state.clone(),
                agent: hook.agent.clone(),
                since: hook.since.or_else(|| since.get(pane).copied()),
            })
            .collect(),
        mismatch: None,
        worktrees: Vec::new(),
    }
}

fn herdr_control(herdr: &HerdrView, since: &BTreeMap<String, f64>, locale: &str) -> Control {
    let snapshot = &herdr.snapshot;
    let mismatch = herdr.protocol_mismatch.as_ref().map(|message| {
        let skew = herdr.version_skew.as_ref();
        Mismatch {
            notice: skew.map_or_else(
                || message.clone(),
                |s| words::herdr_skew(s.client_is_older(), &s.client, &s.server, locale),
            ),
            restart: !skew.map_or_else(
                || moshpit_herdr::snapshot::client_is_older(message),
                |s| s.client_is_older(),
            ),
            restarting: herdr.restarting,
        }
    });
    Control {
        mux: Mux::Herdr,
        attached: snapshot.is_attached,
        refreshing: herdr.refreshing,
        sessions: sorted(snapshot.sessions.values(), |s| &s.id)
            .into_iter()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect(),
        windows: sorted(snapshot.windows.values(), |w| &w.id)
            .into_iter()
            .map(|w| WindowInfo {
                id: w.id.clone(),
                session: w.session_id.clone(),
                index: w.index,
                name: w.name.clone(),
                pane_count: w.pane_count,
            })
            .collect(),
        panes: sorted(snapshot.panes.values(), |p| &p.id)
            .into_iter()
            .map(|p| PaneInfo {
                id: p.id.clone(),
                window: p.window_id.clone(),
                index: p.index,
                command: p.command.clone(),
                path: herdr.pane_cwds.get(&p.id).cloned(),
            })
            .collect(),
        active_session: snapshot.active_session_id.clone(),
        active_window: snapshot.active_window_id.clone(),
        active_pane: snapshot.active_pane_id.clone(),
        hooks: sorted(herdr.agent_hooks.iter(), |(pane, _)| pane.as_str())
            .into_iter()
            .map(|(pane, hook)| Hook {
                pane: pane.clone(),
                state: hook.state.map(|s| s.as_str().to_owned()),
                agent: hook.agent.clone(),
                since: since.get(pane).copied(),
            })
            .collect(),
        mismatch,
        // A workspace's repo only while the workspace is in the tree: the
        // client keeps what it learned of one after it closes.
        worktrees: sorted(herdr.worktree_repos.iter(), |(session, _)| session.as_str())
            .into_iter()
            .filter(|(session, _)| snapshot.sessions.contains_key(*session))
            .map(|(session, repo)| Worktree {
                session: session.clone(),
                repo: repo.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
