//! B87: a window linked into more than one session (`link-window`). tmux
//! 3.6a, read off a private server: a window in sessions A and B, the
//! control client on A —
//!
//! - unlinked from B: A's client gets `%window-close @W`, the window still
//!   in A;
//! - unlinked from A: `%unlinked-window-close @W`, the window still in B;
//! - killed (or unlinked from its last session): `%unlinked-window-close`.
//!
//! A close says the window left a session, not that it is gone. The Swift
//! controller removed it on either, its panes' terminals with it, until the
//! next listing — and when it was the window on screen, the phone left it.

use std::cell::RefCell;
use std::rc::Rc;

use moshpit_tmux::controller::Config;
use moshpit_tmux::rig::{Responder, Rig, Seen, ok};

/// What tmux would list: `(session, window, index, active)` rows.
#[derive(Default)]
struct World {
    links: Vec<(&'static str, &'static str, u8, bool)>,
}

fn responder(world: Rc<RefCell<World>>) -> Responder {
    Box::new(move |command: &str, _| {
        let world = world.borrow();
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main", "$1 0 other"]);
        }
        if command.starts_with("list-windows") {
            return ok(world.links.iter().map(|(session, window, index, active)| {
                format!(
                    "{session} {window} {index} 80x24,0,0,{} {} 1 w{}",
                    &window[1..],
                    u8::from(*active),
                    &window[1..]
                )
            }));
        }
        if command.starts_with("list-panes -a -F '#{pane_id} #{window_id}") {
            // One pane a window, `%N` in `@N`, listed once however many
            // sessions link it.
            let mut windows: Vec<&str> = world.links.iter().map(|link| link.1).collect();
            windows.sort_unstable();
            windows.dedup();
            return ok(windows
                .into_iter()
                .map(|w| format!("%{} {w} 0 80 24 1 0 0 sh", &w[1..])));
        }
        None
    })
}

/// The phone on `$0`, its window `@1` on screen; `@1` linked into `$0`
/// and `$1`.
fn linked() -> (Rig, Rc<RefCell<World>>) {
    let world = Rc::new(RefCell::new(World {
        links: vec![
            ("$0", "@0", 0, false),
            ("$0", "@1", 1, true),
            ("$1", "@1", 5, false),
            ("$1", "@2", 0, true),
        ],
    }));
    let mut rig = Rig::new(Config::default());
    rig.respond(responder(world.clone()));
    rig.attach();
    rig.settle();
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@1"));
    assert!(rig.controller.stream_of("%1").is_some());
    (rig, world)
}

fn closed(rig: &Rig, from: usize) -> Vec<String> {
    rig.seen[from..]
        .iter()
        .filter_map(|seen| match seen {
            Seen::Closed(pane, _) => Some(pane.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_linked_window_shows_in_our_sessions_tree() {
    let (rig, _) = linked();
    assert_eq!(rig.controller.snapshot().windows["@1"].session_id, "$0");
}

#[test]
fn unlinked_from_another_session_the_window_on_screen_stays() {
    let (mut rig, world) = linked();
    let seen = rig.seen.len();
    world.borrow_mut().links.retain(|link| *link != ("$1", "@1", 5, false));
    rig.push("%window-close @1\n");
    // Nothing removed on the notification's word.
    let snapshot = rig.controller.snapshot();
    assert!(snapshot.windows.contains_key("@1"));
    assert!(snapshot.panes.contains_key("%1"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@1"));
    rig.settle();
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.windows["@1"].session_id, "$0");
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@1"));
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%1"));
    assert!(closed(&rig, seen).is_empty(), "{:?}", closed(&rig, seen));
    assert!(rig.controller.stream_of("%1").is_some());
}

#[test]
fn unlinked_from_our_session_the_window_moves_to_the_other() {
    let (mut rig, world) = linked();
    let seen = rig.seen.len();
    world.borrow_mut().links = vec![
        ("$0", "@0", 0, true),
        ("$1", "@1", 5, false),
        ("$1", "@2", 0, true),
    ];
    rig.push("%unlinked-window-close @1\n");
    rig.settle();
    let snapshot = rig.controller.snapshot();
    // Alive in $1: listed there; its pane keeps its terminal.
    assert_eq!(snapshot.windows["@1"].session_id, "$1");
    assert!(closed(&rig, seen).is_empty(), "{:?}", closed(&rig, seen));
    // Our session's current window is on screen now.
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
}

#[test]
fn a_linked_window_killed_everywhere_goes_at_the_listing() {
    let (mut rig, world) = linked();
    let seen = rig.seen.len();
    world.borrow_mut().links = vec![("$0", "@0", 0, true), ("$1", "@2", 0, true)];
    rig.push("%unlinked-window-close @1\n%unlinked-window-close @1\n");
    rig.settle();
    let snapshot = rig.controller.snapshot();
    assert!(!snapshot.windows.contains_key("@1"));
    assert!(!snapshot.panes.contains_key("%1"));
    assert_eq!(closed(&rig, seen), ["%1"]);
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
}

#[test]
fn a_window_in_one_session_goes_at_once() {
    let (mut rig, _) = linked();
    let seen = rig.seen.len();
    let written = rig.commands.len();
    rig.push("%unlinked-window-close @0\n");
    assert!(!rig.controller.snapshot().windows.contains_key("@0"));
    assert_eq!(closed(&rig, seen), ["%0"]);
    assert_eq!(rig.commands.len(), written, "no listing needed");
}
