//! Whose grid a window follows, ported from `TmuxSizeLeaseTests.swift` and the
//! pin, layout and war cases of `TmuxSessionControllerTests.swift`. One
//! controller ("moshpit-a", an iPhone) against a tmux whose replies describe
//! the other device ("moshpit-b", an iPad).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use moshpit_tmux::controller::{Config, PinLedger, SIZE_PROBE_FORMAT, SWEEP_FORMAT};
use moshpit_tmux::lease::SizeFollow;
use moshpit_tmux::rig::{Rig, START_UNIX_MS, ok};

const CLAIM: &str = "set-option -w -t @0 @moshpit_size_owner";
const RELEASE: &str = "set-option -wu -t @0 @moshpit_size_owner";
const PIN: &str = "resize-window -t @0 -x 70 -y 35";
const PROBE: &str = "show-options -wqv -t @0 @moshpit_size_owner";

/// The other device's state, read by the responder.
#[derive(Clone)]
struct Tmux {
    clients: Vec<String>,
    lease: Option<String>,
    capture: String,
    /// `list-windows -a`, the sweep's rows ([`row`]).
    windows: Vec<String>,
}

impl Default for Tmux {
    fn default() -> Self {
        Self {
            clients: vec!["moshpit-a".into()],
            lease: None,
            capture: "READY".into(),
            windows: Vec::new(),
        }
    }
}

type Shared = Rc<RefCell<Tmux>>;

fn responder(tmux: Shared) -> moshpit_tmux::rig::Responder {
    Box::new(move |command, _| {
        let tmux = tmux.borrow();
        if command.starts_with("list-clients -F '#{client_name}'") {
            return ok(tmux.clients.clone());
        }
        if command.starts_with("show-options -wqv") {
            return ok(tmux.lease.clone());
        }
        if command.starts_with("list-windows -a") && command.contains("zoom=") {
            return ok(tmux.windows.clone());
        }
        if command.starts_with("capture-pane") {
            return ok([tmux.capture.clone()]);
        }
        if command.starts_with("display-message") {
            return ok(["0 0"]);
        }
        None
    })
}

/// The lease line the iPad would have written, idle for `idle` seconds at
/// the rig's clock.
fn lease(rig: &Rig, client: &str, idle: i64) -> String {
    let now = (START_UNIX_MS + (rig.now - moshpit_tmux::rig::START_MS) as i64) / 1000;
    format!("{client}|iPad|{}", now - idle)
}

/// An attached iPhone-sized controller on a window tmux reports at 200
/// columns; the attach pin has gone through the lease gate.
fn rig_with(setup: impl FnOnce(&Rig) -> Tmux) -> (Rig, Shared) {
    let mut rig = Rig::new(Config {
        size_owner_label: "iPhone".into(),
        foreign_quiet_repin_delay: 30.0,
        ..Config::default()
    });
    rig.with(|c, _| {
        c.set_initial_client_size(70, 35);
        c.set_our_client_name(Some("moshpit-a".into()));
    });
    let tmux = Rc::new(RefCell::new(setup(&rig)));
    rig.respond(responder(tmux.clone()));
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 200x50,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 200 50 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.settle();
    (rig, tmux)
}

fn ipad_typing(rig: &Rig) -> Tmux {
    Tmux {
        clients: vec!["moshpit-a".into(), "moshpit-b".into()],
        lease: Some(lease(rig, "moshpit-b", 2)),
        ..Tmux::default()
    }
}

const DRIFT: &str = "%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n";
/// The holder's window changing again: tmux reports a layout only when it
/// changes (the same one again is another session's copy of it, B771).
const DRIFT_AGAIN: &str = "%layout-change @0 c71e,201x50,0,0,0 c71e,201x50,0,0,0 *\n";

fn screen(rig: &Rig) -> String {
    String::from_utf8_lossy(&rig.pane_bytes("%0")).into_owned()
}

// MARK: - Attach: who gets the window

#[test]
fn a_free_window_is_claimed_passively_and_pinned() {
    let (rig, _) = rig_with(|_| Tmux::default());
    assert!(rig.has(PROBE), "attach asks who holds the window");
    assert!(rig.has(&format!("{CLAIM} \"moshpit-a|iPhone|0\"")));
    let claim = rig.index_of(CLAIM).unwrap();
    let pin = rig.index_of(PIN).unwrap();
    assert!(claim < pin);
}

#[test]
fn a_window_a_live_device_just_used_is_left_alone() {
    let (rig, _) = rig_with(ipad_typing);
    assert!(!rig.has(PIN));
    assert!(!rig.has(CLAIM));
    assert_eq!(
        rig.controller.size_follow(),
        None,
        "nothing to veil until the window reads foreign"
    );
}

#[test]
fn an_idle_holder_loses_the_window_to_an_attaching_device() {
    let (rig, _) = rig_with(|rig| Tmux {
        clients: vec!["moshpit-a".into(), "moshpit-b".into()],
        lease: Some(lease(rig, "moshpit-b", 60)),
        ..Tmux::default()
    });
    assert!(rig.has(&format!("{CLAIM} \"moshpit-a|iPhone|0\"")));
    assert!(rig.has(PIN));
    assert_eq!(rig.controller.size_follow(), None);
}

#[test]
fn a_lease_of_a_client_no_longer_attached_is_nobodys() {
    let (rig, _) = rig_with(|rig| Tmux {
        clients: vec!["moshpit-a".into()],
        lease: Some(lease(rig, "moshpit-b", 1)),
        ..Tmux::default()
    });
    assert!(rig.has(&format!("{CLAIM} \"moshpit-a|iPhone|0\"")));
    assert!(rig.has(PIN));
}

/// A row of the dead-pin sweep's window listing (`SWEEP_FORMAT`): the
/// window, zoomed or not, its active pane, the zoom mark, the lease.
fn row(window: &str, zoomed: bool, pane: &str, zoom: &str, lease: &str) -> String {
    format!(
        "{window} {} {pane} zoom={zoom} lease={lease}",
        if zoomed { 1 } else { 0 }
    )
}

/// A controller attached as the app attaches one: control mode begun
/// (`begin_control_mode`, as the tmux leg does), tmux's `%session-changed`
/// first, everything else answered by the responder.
fn attached(setup: impl FnOnce(&Rig) -> Tmux) -> Rig {
    let mut rig = Rig::new(Config {
        size_owner_label: "iPhone".into(),
        ..Config::default()
    });
    rig.with(|c, _| {
        c.set_initial_client_size(70, 35);
        c.set_our_client_name(Some("moshpit-a".into()));
    });
    let tmux = Rc::new(RefCell::new(setup(&rig)));
    rig.respond(responder(tmux));
    rig.with(|c, now| c.begin_control_mode(now));
    rig.boot();
    rig.push("%session-changed $0 main\n");
    rig.settle();
    rig
}

/// B456: a phone that died while a window was pinned to it — the app
/// killed, the network gone, no teardown to hand anything back — left the
/// window at the phone's grid (`resize-window` sets `window-size manual`)
/// with its lease naming a client that no longer exists: the desktop's
/// window stayed phone-sized for good. The next attach hands back every
/// such window — only those: a live device's and an unleased one are left
/// alone.
#[test]
fn an_attach_hands_back_the_windows_a_dead_phone_left_pinned() {
    let rig = attached(|rig| Tmux {
        clients: vec![
            "moshpit-a".into(),
            "moshpit-b".into(),
            "/dev/ttys004".into(),
        ],
        windows: vec![
            row("@1", false, "%1", "", &lease(rig, "moshpit-gone", 600)),
            row("@2", false, "%2", "", &lease(rig, "moshpit-b", 600)),
            row("@3", false, "%3", "", ""),
            row("@4", false, "%4", "", "moshpit-gone|Cluas's iPhone|0"),
        ],
        ..Tmux::default()
    });
    assert!(
        rig.has(&format!("list-windows -a -F '{SWEEP_FORMAT}'")),
        "the attach reads every window's lease"
    );
    for window in ["@1", "@4"] {
        assert!(
            rig.has(&format!("set-option -u -w -t {window} window-size")),
            "{window} handed back"
        );
        assert!(rig.has(&format!("set-option -wu -t {window} @moshpit_size_owner")));
    }
    for window in ["@2", "@3"] {
        assert!(!rig.has(&format!("set-option -u -w -t {window} window-size")));
        assert!(!rig.has(&format!("set-option -wu -t {window} @moshpit_size_owner")));
    }
}

/// B458: the phone zooms a split window's active pane to show it alone,
/// and a phone that died left the zoom too: the desktop's split window
/// stayed one zoomed pane. The phone marks the zoom it makes with its client
/// and the pane; the next attach unzooms a window still zoomed on that pane
/// once the client is gone, and only that: a desktop that zoomed another
/// pane since, a live iPad's zoom, a zoom nobody marked (a desktop's own)
/// and an unzoomed window are left as they are.
#[test]
fn an_attach_unzooms_only_the_zoom_a_dead_phone_made() {
    let rig = attached(|_| Tmux {
        clients: vec![
            "moshpit-a".into(),
            "moshpit-b".into(),
            "/dev/ttys004".into(),
        ],
        windows: vec![
            row("@1", true, "%5", "moshpit-gone %5", ""),
            row("@2", true, "%7", "moshpit-gone %6", ""),
            row("@3", true, "%8", "moshpit-b %8", ""),
            row("@4", true, "%9", "", ""),
            row("@5", false, "%10", "moshpit-gone %10", ""),
        ],
        ..Tmux::default()
    });
    assert!(rig.has("resize-pane -Z -t @1"), "the dead phone's zoom");
    for window in ["@2", "@3", "@4", "@5"] {
        assert!(!rig.has(&format!("resize-pane -Z -t {window}")), "{window}");
    }
    // A gone client's mark goes, zoom or not; a live one's and none stay.
    for window in ["@1", "@2", "@5"] {
        assert!(rig.has(&format!("set-option -wu -t {window} @moshpit_zoomed")));
    }
    for window in ["@3", "@4"] {
        assert!(!rig.has(&format!("set-option -wu -t {window} @moshpit_zoomed")));
    }
}

/// B459: with a phone's group on the server (or a desktop's linked
/// session) `list-windows -a` names each window once per session holding
/// it. The sweep acted on every row: the dead phone's zoom was toggled
/// twice — off, then on again — and stayed.
#[test]
fn a_window_listed_once_per_session_is_swept_once() {
    let rig = attached(|_| Tmux {
        clients: vec!["moshpit-a".into(), "/dev/ttys004".into()],
        windows: vec![
            row("@1", true, "%5", "moshpit-gone %5", "moshpit-gone|iPhone|1"),
            row("@1", true, "%5", "moshpit-gone %5", "moshpit-gone|iPhone|1"),
        ],
        ..Tmux::default()
    });
    assert_eq!(rig.count("resize-pane -Z -t @1"), 1, "{:#?}", rig.commands);
    assert_eq!(rig.count("set-option -wu -t @1 @moshpit_zoomed"), 1);
}

/// The zoom the phone makes is marked with its client and the pane it zooms.
#[test]
fn the_phones_own_zoom_is_marked() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| {
        c.set_initial_client_size(47, 40);
        c.set_our_client_name(Some("moshpit-a".into()));
    });
    let three = "3992,47x40,0,0{45x40,0,0,0,1x40,46,0[1x20,46,0,1,1x19,46,21,2]}";
    let window = format!("$0 @0 0 {three} 1 3 claude");
    let probe = format!("3 0 %1 {three}");
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-windows -a -F '#{session_id}") {
            return ok([window.clone()]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok([
                "%0 @0 0 45 40 0 0 0 claude",
                "%1 @0 1 1 20 1 0 0 claude",
                "%2 @0 2 1 19 0 0 0 claude",
            ]);
        }
        if command.starts_with("display-message -p -t '@0'") {
            return ok([probe.clone()]);
        }
        if command.starts_with("display-message -p '#{client_name}'")
            || command.starts_with("list-clients -F '#{client_name}'")
        {
            return ok(["moshpit-a"]);
        }
        None
    }));
    rig.with(|c, now| c.begin_control_mode(now));
    rig.boot();
    rig.push("%session-changed $0 main\n");
    rig.settle();
    let zoom = rig.index_of("resize-pane -Z -t @0").expect("zoomed");
    let mark = rig
        .index_of("set-option -w -t @0 @moshpit_zoomed \"moshpit-a %1\"")
        .expect("marked");
    assert!(zoom < mark);
}

/// Without a listing of the clients nothing can be judged dead: nothing is
/// handed back.
#[test]
fn no_clients_listed_hands_nothing_back() {
    let rig = attached(|rig| Tmux {
        clients: Vec::new(),
        windows: vec![row(
            "@1",
            true,
            "%1",
            "moshpit-gone %1",
            &lease(rig, "moshpit-gone", 600),
        )],
        ..Tmux::default()
    });
    assert!(rig.has(&format!("list-windows -a -F '{SWEEP_FORMAT}'")));
    assert!(!rig.has("resize-pane -Z -t @1"));
    assert!(!rig.has("set-option -u -w -t @1 window-size"));
}

// MARK: - Following

#[test]
fn drift_on_a_held_window_is_followed_and_its_output_gated() {
    let (mut rig, _) = rig_with(ipad_typing);
    assert!(screen(&rig).contains("READY"));
    let before = rig.commands.len();
    rig.push(DRIFT);
    assert_eq!(
        rig.controller.size_follow(),
        Some(&SizeFollow {
            window_id: "@0".into(),
            owner_label: "iPad".into()
        })
    );
    rig.settle();
    assert!(!rig.since(before).iter().any(|c| c.starts_with(PIN)));
    assert!(!rig.since(before).iter().any(|c| c.starts_with(CLAIM)));
    rig.push("%output %0 IPAD-SIZED\n");
    assert!(!screen(&rig).contains("IPAD-SIZED"));
}

#[test]
fn a_recheck_never_steals_from_a_live_holder() {
    let (mut rig, tmux) = rig_with(ipad_typing);
    rig.push(DRIFT);
    assert!(rig.controller.size_follow().is_some());
    rig.settle();
    tmux.borrow_mut().lease = Some(lease(&rig, "moshpit-b", 60));
    let before = rig.commands.len();
    rig.controller.config_mut().foreign_quiet_repin_delay = 0.1;
    rig.push(DRIFT_AGAIN);
    rig.wait_settled(600, 30);
    let after = rig.since(before);
    assert!(after.iter().filter(|c| c.starts_with(PROBE)).count() >= 2);
    assert!(!after.iter().any(|c| c.starts_with(PIN)));
    assert!(!after.iter().any(|c| c.starts_with(CLAIM)));
    assert_eq!(rig.controller.size_follow().unwrap().owner_label, "iPad");
}

#[test]
fn a_recheck_takes_a_freed_window_and_drops_the_veil() {
    let (mut rig, tmux) = rig_with(ipad_typing);
    rig.push(DRIFT);
    rig.settle();
    tmux.borrow_mut().lease = None;
    let before = rig.commands.len();
    rig.controller.config_mut().foreign_quiet_repin_delay = 0.1;
    rig.push(DRIFT_AGAIN);
    rig.wait_settled(300, 30);
    assert_eq!(rig.controller.size_follow(), None);
    let after = rig.since(before);
    assert!(after.iter().any(|c| c.starts_with(PROBE)));
    assert!(
        after
            .iter()
            .any(|c| c.starts_with(&format!("{CLAIM} \"moshpit-a|iPhone|0\"")))
    );
    assert!(after.iter().any(|c| c.starts_with(PIN)));
}

#[test]
fn a_recheck_takes_the_window_of_a_vanished_holder() {
    let (mut rig, tmux) = rig_with(ipad_typing);
    rig.push(DRIFT);
    rig.settle();
    tmux.borrow_mut().clients = vec!["moshpit-a".into()];
    let before = rig.commands.len();
    rig.controller.config_mut().foreign_quiet_repin_delay = 0.1;
    rig.push(DRIFT_AGAIN);
    rig.wait_settled(300, 30);
    assert_eq!(rig.controller.size_follow(), None);
    let after = rig.since(before);
    assert!(after.iter().any(|c| c.starts_with(CLAIM)));
    assert!(after.iter().any(|c| c.starts_with(PIN)));
}

// MARK: - Taking over

#[test]
fn the_followers_tap_takes_the_window() {
    let (mut rig, _) = rig_with(ipad_typing);
    rig.push(DRIFT);
    assert!(rig.controller.size_follow().is_some());
    let before = rig.commands.len();
    rig.with(|c, now| c.take_size_ownership(now));
    assert_eq!(rig.controller.size_follow(), None);
    let after = rig.since(before).to_vec();
    let claim = after.iter().position(|c| c.starts_with(CLAIM)).unwrap();
    let pin = after.iter().position(|c| c.starts_with(PIN)).unwrap();
    assert!(claim < pin);
    assert!(after[claim].contains("moshpit-a|iPhone|"));
    assert!(!after[claim].contains("|0\""), "an interaction stamps now");
    rig.settle();
    rig.push("%output %0 MINE-AGAIN\n");
    assert!(screen(&rig).contains("MINE-AGAIN"));
}

#[test]
fn a_keystroke_into_a_followed_window_takes_it() {
    let (mut rig, _) = rig_with(ipad_typing);
    rig.push(DRIFT);
    let before = rig.commands.len();
    rig.with(|c, now| c.send_input(b"x", "%0", now));
    let after = rig.since(before).to_vec();
    assert_eq!(rig.controller.size_follow(), None);
    let pin = after.iter().position(|c| c.starts_with(PIN)).unwrap();
    let keys = after
        .iter()
        .position(|c| c.starts_with("send-keys"))
        .unwrap();
    assert!(after.iter().any(|c| c.starts_with(CLAIM)));
    assert!(
        pin < keys,
        "the window is ours before the keystroke lands in it"
    );
}

// MARK: - The stamp

#[test]
fn keystrokes_refresh_the_stamp_throttled() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    rig.controller.config_mut().lease_stamp_refresh_interval = 0.3;
    assert_eq!(rig.count(CLAIM), 1, "attach wrote the passive claim");
    rig.with(|c, now| c.send_input(b"a", "%0", now));
    rig.with(|c, now| c.send_input(b"b", "%0", now));
    assert_eq!(rig.count(CLAIM), 2);
    rig.wait(100);
    rig.with(|c, now| c.send_input(b"c", "%0", now));
    assert_eq!(rig.count(CLAIM), 2);
    let last = rig.commands.iter().rfind(|c| c.starts_with(CLAIM)).unwrap();
    assert!(!last.contains("|0\""));
    rig.wait(300);
    rig.with(|c, now| c.send_input(b"d", "%0", now));
    assert_eq!(rig.count(CLAIM), 3);
}

// MARK: - Background / foreground

#[test]
fn backgrounding_hands_the_lease_back_with_the_pin() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    rig.with(|c, now| c.release_window_pins(now));
    let unpin = rig.index_of("set-option -u -w -t @0 window-size").unwrap();
    let release = rig.index_of(RELEASE).unwrap();
    assert!(unpin < release);
    assert!(rig.has("refresh-client -f ignore-size"));
    assert!(
        !rig.commands.iter().any(|c| c.contains(" -A")),
        "never resize-window -A"
    );
}

#[test]
fn release_without_pins_still_leaves_the_sizing_math() {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.with(|c, now| c.release_window_pins(now));
    assert!(rig.has("refresh-client -f ignore-size"));
    assert!(!rig.has("set-option -u -w"));
}

#[test]
fn the_foreground_asks_first_and_defers_to_a_new_holder() {
    let (mut rig, tmux) = rig_with(|_| Tmux::default());
    rig.with(|c, now| c.release_window_pins(now));
    rig.settle();
    *tmux.borrow_mut() = Tmux {
        clients: vec!["moshpit-a".into(), "moshpit-b".into()],
        lease: Some(lease(&rig, "moshpit-b", 1)),
        ..Tmux::default()
    };
    let before = rig.commands.len();
    rig.controller.config_mut().foreground_probe_timeout = 30.0;
    rig.with(|c, now| c.repin_active_window(now));
    assert!(rig.since(before).iter().any(|c| c.starts_with(PROBE)));
    rig.settle();
    let after = rig.since(before);
    assert!(
        after
            .iter()
            .any(|c| c.starts_with("refresh-client -f !ignore-size"))
    );
    assert!(!after.iter().any(|c| c.starts_with(PIN)));
    assert!(!after.iter().any(|c| c.starts_with(CLAIM)));
    rig.push(DRIFT);
    assert_eq!(rig.controller.size_follow().unwrap().owner_label, "iPad");
}

#[test]
fn the_foreground_takes_a_window_nobody_claimed() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    rig.with(|c, now| c.release_window_pins(now));
    rig.settle();
    let before = rig.commands.len();
    rig.with(|c, now| c.repin_active_window(now));
    rig.settle();
    let after = rig.since(before);
    assert!(after.iter().any(|c| c.starts_with(PROBE)));
    assert!(after.iter().any(|c| c.starts_with(PIN)));
    assert!(
        after
            .iter()
            .any(|c| c.starts_with(&format!("{CLAIM} \"moshpit-a|iPhone|0\"")))
    );
    // ...and repaints after it.
    rig.wait_settled(400, 50);
    let last_pin = rig
        .commands
        .iter()
        .rposition(|c| c.starts_with(PIN))
        .unwrap();
    assert!(
        rig.since(last_pin)
            .iter()
            .any(|c| c.starts_with("capture-pane -p -e -t %0"))
    );
}

#[test]
fn a_silent_foreground_probe_pins_anyway() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    rig.with(|c, now| c.release_window_pins(now));
    rig.settle();
    let before = rig.commands.len();
    rig.controller.config_mut().foreground_probe_timeout = 0.1;
    rig.with(|c, now| c.repin_active_window(now));
    rig.wait(150);
    assert!(rig.since(before).iter().any(|c| c.starts_with(PIN)));
}

#[test]
fn output_while_the_pin_is_handed_back_is_not_painted_but_its_bell_is() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    rig.push("%output %0 abc\n");
    assert!(screen(&rig).ends_with("abc"));
    rig.with(|c, now| c.release_window_pins(now));
    rig.push("%output %0 defghij\n");
    assert!(!screen(&rig).contains("defghij"));
    rig.push("%output %0 \\033]0;title\\007\n");
    rig.push("%output %0 \\007\n");
    let bells = rig
        .notes()
        .iter()
        .filter(|n| matches!(n, moshpit_tmux::controller::Notification::PaneBell { .. }))
        .count();
    assert_eq!(bells, 1, "a title's BEL is no bell");
}

// MARK: - Teardown

#[test]
fn release_commands_unset_only_our_leases_once() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    assert_eq!(
        rig.controller.size_lease_release_commands(),
        ["set-option -wu -t @0 @moshpit_size_owner"]
    );
    assert!(rig.controller.size_lease_release_commands().is_empty());
    let (mut following, _) = rig_with(ipad_typing);
    assert!(
        following
            .controller
            .size_lease_release_commands()
            .is_empty()
    );
}

// MARK: - The war guard

#[test]
fn repeated_drift_backs_off_after_a_few_reclaims() {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.wait(200);
    assert!(rig.has(PIN));
    let base = rig.commands.len();
    // The desktop's width, our pin landing, the desktop's again: a war.
    // (The same change reported again is no new drift, B771.)
    for _ in 0..4 {
        rig.push("%layout-change @0 200x24,0,0,2\n%layout-change @0 70x35,0,0,2\n");
    }
    let reclaims = rig
        .since(base)
        .iter()
        .filter(|c| c.starts_with(PIN))
        .count();
    assert_eq!(
        reclaims, 2,
        "the third trips the backoff, the fourth is suppressed"
    );
}

/// B771: a window in more than one session (each phone views a session
/// through its own group, B459) is reported by `%layout-change` once per
/// session holding it: one change, one drift. Each copy was taken for a
/// drift of its own, re-pinned and counted by the war guard: two phones
/// settling who holds a window bounced it between their sizes several times
/// and could trip the guard, stopping with the lease naming one phone and the
/// window at the other's size, nobody following (live, a private tmux 3.6a:
/// six copies of each change, a `resize-window` sent per copy).
#[test]
fn a_change_reported_once_per_session_is_one_drift() {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.wait(200);
    assert!(rig.has(PIN));
    let base = rig.commands.len();
    rig.push("%layout-change @0 200x24,0,0,2\n%layout-change @0 200x24,0,0,2\n%layout-change @0 200x24,0,0,2\n");
    let reclaims = rig
        .since(base)
        .iter()
        .filter(|c| c.starts_with(PIN))
        .count();
    assert_eq!(reclaims, 1, "{:#?}", rig.since(base));
}

/// B775: a window listing answered after another device's resize, but before
/// that resize's `%layout-change` arrives, already shows the window at the
/// other size; the notification that follows is the drift all the same. It
/// was taken for the same layout reported again (B771's check read the
/// layout the listing had written): no reclaim, no lease read, and two
/// phones stopped with the lease naming one and the window at the other's
/// size, nobody following (live, under load: about 1 run in 12).
#[test]
fn a_drift_a_listing_saw_first_is_still_a_drift() {
    let (mut rig, tmux) = rig_with(|_| Tmux::default());
    let mut inner = responder(tmux.clone());
    rig.respond(Box::new(move |command, n| {
        if command.starts_with("list-windows -a -F '#{session_id}") {
            return ok(["$0 @0 0 c71d,200x50,0,0,0 1 1 main"]);
        }
        inner(command, n)
    }));
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert_eq!(
        rig.controller.snapshot().windows["@0"].layout,
        "c71d,200x50,0,0,0",
        "the listing saw the other device's size"
    );
    let before = rig.commands.len();
    rig.push(DRIFT);
    rig.settle();
    let after = rig.since(before);
    assert!(after.iter().any(|c| c.starts_with(PIN)), "{after:#?}");
}

/// B774: the holder letting go (its leave unpins the window, and tmux sizes
/// it back to the follower) brings the window back to the follower's width:
/// the veil dropped, and the follower now sizes the window — but its lease
/// stayed unset, so a device attaching next took the window from under the
/// person using it. Taking a freed window claims it, as the quiet re-check
/// does (live: the iPhone left, the iPad's veil went, `@moshpit_size_owner`
/// empty).
#[test]
fn a_freed_window_back_at_our_width_is_claimed() {
    let (mut rig, tmux) = rig_with(ipad_typing);
    rig.push(DRIFT);
    rig.settle();
    assert!(rig.controller.size_follow().is_some());
    // The iPad leaves: its lease unset, the window back at our width.
    tmux.borrow_mut().lease = None;
    tmux.borrow_mut().clients = vec!["moshpit-a".into()];
    let before = rig.commands.len();
    rig.push("%layout-change @0 c71d,70x35,0,0,0 c71d,70x35,0,0,0 *\n");
    rig.settle();
    assert_eq!(rig.controller.size_follow(), None);
    let after = rig.since(before);
    assert!(
        after
            .iter()
            .any(|c| c.starts_with(&format!("{CLAIM} \"moshpit-a|iPhone|0\""))),
        "{after:#?}"
    );
}

#[test]
fn a_war_ending_at_the_desktops_width_is_repinned_after_a_quiet_spell() {
    let mut rig = Rig::new(Config {
        foreign_quiet_repin_delay: 0.15,
        ..Config::default()
    });
    rig.with(|c, _| c.set_initial_client_size(69, 60));
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.respond(Box::new(|command, _| {
        if command.starts_with("capture-pane") {
            return ok(["QUIET-REPAIRED"]);
        }
        if command.starts_with("display-message") {
            return ok(["0 0"]);
        }
        None
    }));
    rig.settle();
    for _ in 0..4 {
        rig.push("%layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\n");
        rig.wait(30);
    }
    rig.settle();
    let quiet = rig.commands.len();
    rig.wait_settled(400, 50);
    assert!(
        rig.since(quiet)
            .iter()
            .any(|c| c.starts_with("resize-window"))
    );
    rig.push("%layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\n");
    rig.wait_settled(1500, 50);
    assert!(screen(&rig).contains("QUIET-REPAIRED"));
    rig.push("%output %0 live-after-quiet\n");
    assert!(screen(&rig).ends_with("live-after-quiet"));
}

#[test]
fn foreign_width_output_paints_and_our_width_repaints() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(69, 60));
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.settle();
    rig.push("%layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\n");
    rig.push("%output %0 defghij\n");
    assert!(
        screen(&rig).ends_with("defghij"),
        "no mid-session drops (the 334 model)"
    );
    rig.settle();
    let before = rig.count("capture-pane -p -e -t %0");
    rig.push("%layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\n");
    assert!(rig.count("capture-pane -p -e -t %0") > before);
}

#[test]
fn a_split_that_unzooms_the_window_is_zoomed_again_but_not_a_persons_unzoom() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(47, 40));
    rig.attach();
    let three = "3992,47x40,0,0{45x40,0,0,0,1x40,46,0[1x20,46,0,1,1x19,46,21,2]}";
    rig.push(&format!("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 {three} 1 3 claude\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 45 40 0 0 0 claude\n%1 @0 1 1 20 1 0 0 claude\n%2 @0 2 1 19 0 0 0 claude\n%end 3 3 1\n\n"));
    rig.answered = 3;
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%1")
    );
    rig.settle();
    rig.push(&format!("%layout-change @0 {three} 3992,47x40,0,0,1 *Z\n"));
    rig.settle();
    let zooms = |rig: &Rig| {
        rig.commands
            .iter()
            .filter(|c| c.contains("resize-pane -Z"))
            .count()
    };
    let before = zooms(&rig);
    let four =
        "1c2d,47x40,0,0{45x40,0,0[45x20,0,0,0,45x19,0,21,3],1x40,46,0[1x20,46,0,1,1x19,46,21,2]}";
    let answered = rig.answered;
    rig.push(&format!("%layout-change @0 {four} {four}\n"));
    assert!(
        rig.since(answered)
            .iter()
            .any(|c| c.starts_with("display-message -p -t '@0'"))
    );
    let probe_reply = format!("4 0 %1 {four}");
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("display-message -p -t '@0'") {
            return ok([probe_reply.clone()]);
        }
        None
    }));
    rig.settle();
    assert_eq!(zooms(&rig), before + 1);
    rig.push(&format!("%layout-change @0 {four} 1c2d,47x40,0,0,1 *Z\n"));
    rig.settle();
    let answered = rig.answered;
    rig.push(&format!("%layout-change @0 {four} {four}\n"));
    assert!(
        !rig.since(answered)
            .iter()
            .any(|c| c.starts_with("display-message -p -t '@0'"))
    );
    assert_eq!(zooms(&rig), before + 1);
}

// MARK: - The layout pin

const SPLIT: &str = "4b9d,203x60,0,0[203x30,0,0,0,203x29,0,31{101x29,0,31,1,101x29,102,31,2}]";

/// A three-pane window, pinned to the phone grid; the layout probe answered
/// with `capture`.
fn pinned_split(capture: &str) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push(&format!("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 {SPLIT} 1 3 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 203 30 1 0 0 bash\n%1 @0 1 101 29 0 0 0 bash\n%2 @0 2 101 29 0 0 0 bash\n%end 3 3 1\n\n"));
    rig.answered = 3;
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.wait(200);
    let capture = capture.to_owned();
    rig.respond(Box::new(move |command, _| {
        if command.contains("#{window_layout}") {
            return ok([capture.clone()]);
        }
        None
    }));
    rig.settle();
    rig
}

#[test]
fn the_layout_probe_goes_before_the_pin() {
    let rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    let probe = rig
        .commands
        .iter()
        .position(|c| c.contains("#{window_layout}") && c.contains("@0"))
        .unwrap();
    let pin = rig.index_of("resize-window -t @0").unwrap();
    assert!(probe < pin);
}

#[test]
fn backgrounding_replays_the_layout_after_the_unpin() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.with(|c, now| c.release_window_pins(now));
    let unpin = rig.index_of("set-option -u -w -t @0 window-size").unwrap();
    let restore = rig.index_of("select-layout -t '@0'").unwrap();
    assert!(unpin < restore);
    assert_eq!(
        rig.commands[restore],
        format!("select-layout -t '@0' '{SPLIT}'")
    );
    assert!(
        !rig.since(restore)
            .iter()
            .any(|c| c.starts_with("resize-pane -Z -t '%"))
    );
}

#[test]
fn a_users_own_zoom_comes_back_with_the_layout() {
    let mut rig = pinned_split(&format!("3 1 %2 {SPLIT}"));
    rig.with(|c, now| c.release_window_pins(now));
    let restore = rig.index_of("select-layout -t '@0'").unwrap();
    let zoom = rig.index_of("resize-pane -Z -t '%2'").unwrap();
    assert!(restore < zoom);
}

/// B772: a window another Moshpit device has pinned (`window-size manual`)
/// is found squashed to that device's grid. What that device found before
/// its pin — shared on the window, `@moshpit_layout` — is the record, and
/// the squashed layout is one of Moshpit's own. The second phone recorded
/// the first one's squashed layout as the window's: both phones gone, the
/// window was left in it, at the first phone's grid (live, two phones on a
/// private tmux 3.6a: 50x20 where 120x40 had been).
#[test]
fn a_window_another_device_pinned_is_recorded_as_it_was_before_that_pin() {
    let mut rig = pinned_split(&format!("3 0 %0 {SQUASHED} manual 3|-|{SPLIT}"));
    assert!(
        !rig.has("set-option -w -t @0 @moshpit_layout"),
        "the shared record stays the first device's"
    );
    let restore = rig.controller.pristine_layout_restore_commands();
    assert!(
        restore.contains(&format!("select-layout -t @0 '{SPLIT}'")),
        "{restore:#?}"
    );
}

/// B772: the layout found before the first pin (the probe goes ahead of
/// it) is shared on the window, for a device that comes while it is pinned.
#[test]
fn the_layout_found_is_shared_with_the_next_device() {
    let rig = pinned_split(&format!("3 0 %0 {SPLIT} latest"));
    assert!(
        rig.has(&format!("set-option -w -t @0 @moshpit_layout '3|-|{SPLIT}'")),
        "{:#?}",
        rig.commands
    );
    // A reply of the old shape (no `window-size` read) shares nothing.
    let rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    assert!(!rig.has("set-option -w -t @0 @moshpit_layout"));
}

/// B772: a shared record that describes other panes is somebody's old one:
/// what is found is the record.
#[test]
fn a_shared_record_of_other_panes_is_not_taken() {
    let mut rig = pinned_split(&format!("3 0 %0 {SQUASHED} manual 2|-|{OTHER_SPLIT}"));
    let restore = rig.controller.pristine_layout_restore_commands();
    assert!(
        restore.contains(&format!("select-layout -t @0 '{SQUASHED}'")),
        "{restore:#?}"
    );
}

/// B772: the replay of the record drops the shared one: nobody's pin is on
/// the window any more.
#[test]
fn the_replay_drops_the_shared_record() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT} latest"));
    rig.with(|c, now| c.release_window_pins(now));
    let restore = rig.index_of("select-layout -t '@0'").unwrap();
    let unshare = rig
        .index_of("set-option -wu -t @0 @moshpit_layout")
        .unwrap_or_else(|| panic!("{:#?}", rig.commands));
    assert!(restore < unshare);
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT} latest"));
    let script = rig
        .controller
        .pin_hand_back_script("tmux", &[])
        .expect("a hand-back");
    assert!(
        script.contains("tmux set-option -wu -t @0 @moshpit_layout"),
        "{script}"
    );
}

#[test]
fn a_single_pane_window_has_nothing_to_restore() {
    let mut rig = pinned_split(&format!("1 0 %0 {SPLIT}"));
    rig.with(|c, now| c.release_window_pins(now));
    assert!(!rig.has("select-layout"));
}

#[test]
fn a_stale_record_is_dropped() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-windows") {
            return ok([format!("$0 @0 0 {SPLIT} 1 2 main")]);
        }
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        None
    }));
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert_eq!(rig.controller.snapshot().windows["@0"].pane_count, 2);
    rig.with(|c, now| c.release_window_pins(now));
    assert!(!rig.has("select-layout"));
}

#[test]
fn teardown_hands_the_restore_out_once() {
    // B150: our client still attached — out of the sizing math, and the
    // recorded size reported as ours, first: alone with the session, tmux
    // keeps sizing it by us, and a layout replayed at another size is
    // refitted.
    let mut rig = pinned_split(&format!("3 1 %1 {SPLIT}"));
    rig.controller
        .set_our_client_name(Some("/dev/ttys003".to_owned()));
    assert_eq!(
        rig.controller.pristine_layout_restore_commands(),
        [
            "refresh-client -t '/dev/ttys003' -f ignore-size".to_owned(),
            "refresh-client -t '/dev/ttys003' -C 203x60".to_owned(),
            format!("select-layout -t @0 '{SPLIT}'"),
            "set-option -wu -t @0 @moshpit_layout".to_owned(),
            "resize-pane -Z -t %1".to_owned()
        ]
    );
    assert!(rig.controller.pristine_layout_restore_commands().is_empty());
}

#[test]
fn a_ledgers_restore_sizes_each_window_to_its_record() {
    // No client of ours to report a size with (a ledger's records, its
    // connection gone): with no client attached tmux keeps a window's size,
    // so each goes to its record's size, takes the layout, and is handed
    // back to automatic sizing (B150).
    let mut rig = pinned_split(&format!("3 1 %1 {SPLIT}"));
    assert_eq!(
        rig.controller.pristine_layout_restore_commands(),
        [
            "resize-window -t @0 -x 203 -y 60".to_owned(),
            format!("select-layout -t @0 '{SPLIT}'"),
            "set-option -wu -t @0 @moshpit_layout".to_owned(),
            "set-option -u -w -t @0 window-size".to_owned(),
            "resize-pane -Z -t %1".to_owned()
        ]
    );
}

#[test]
fn the_background_hand_back_reports_the_recorded_size() {
    // B150: `ignore-size` keeps us out of tmux's sizing only while another
    // client is attached; alone, the recorded size reported as ours brings
    // the windows back to it, and the replayed layout fits exactly.
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    let base = rig.commands.len();
    rig.with(|c, now| c.release_window_pins(now));
    let fresh = rig.since(base).to_vec();
    let flag = fresh
        .iter()
        .position(|c| c == "refresh-client -f ignore-size");
    let claim = fresh.iter().position(|c| c == "refresh-client -C 203x60");
    let unpin = fresh
        .iter()
        .position(|c| c == "set-option -u -w -t @0 window-size");
    let replay = fresh
        .iter()
        .position(|c| c == &format!("select-layout -t '@0' '{SPLIT}'"));
    assert!(
        flag < claim && claim < unpin && unpin < replay && flag.is_some(),
        "{fresh:#?}"
    );
}

#[test]
fn without_a_record_the_hand_back_reports_no_size() {
    let mut rig = pinned_split(&format!("1 0 %0 {SPLIT}"));
    let base = rig.commands.len();
    rig.with(|c, now| c.release_window_pins(now));
    assert!(
        !rig.since(base)
            .iter()
            .any(|c| c.starts_with("refresh-client -C")),
        "{:#?}",
        rig.since(base)
    );
}

/// Two windows, a split one and a single pane, as tmux lists them for the
/// size probes: before our size report, at 203x60, and after, squashed.
const SQUASHED: &str = "b2b1,70x35,0,0[70x17,0,0,0,70x17,0,18{35x17,0,18,1,34x17,36,18,2}]";
const OTHER_BEFORE: &str = "6c3d,203x60,0,0,3";
const OTHER_SPLIT: &str = "5aa1,203x60,0,0{101x60,0,0,4,101x60,102,0,5}";

/// A fresh attach through `%session-changed`, its probes answered by
/// `answer` (the commands it gets, and how many probes were answered).
fn fresh_attach(answer: impl Fn(&str, usize) -> Option<String> + 'static, pin_probe: &str) -> Rig {
    let pin_probe = pin_probe.to_owned();
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(70, 35));
    rig.with(|c, now| c.begin_control_mode(now));
    let probes = Rc::new(RefCell::new(0usize));
    let seen = probes.clone();
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("list-windows -t '$0'") {
            let n = *seen.borrow();
            *seen.borrow_mut() += 1;
            return answer(command, n)
                .map(|text| ok(text.split('\n').map(str::to_owned).collect::<Vec<_>>()).unwrap());
        }
        if command.starts_with("display-message -p -t '@0' '#{window_panes}") {
            return ok([pin_probe.clone()]);
        }
        if command.starts_with("list-windows -a") {
            return ok([format!("$0 @0 0 {SQUASHED} 1 3 main")]);
        }
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok([
                "%0 @0 0 70 17 1 0 0 bash",
                "%1 @0 1 35 17 0 0 0 bash",
                "%2 @0 2 34 17 0 0 0 bash",
            ]);
        }
        None
    }));
    rig.boot();
    rig.push("%session-changed $0 main\n");
    rig.settle();
    rig
}

#[test]
fn the_layouts_our_size_report_squashes_are_recorded_before_it() {
    // B150: the phone alone with the session (so tmux's latest client):
    // the attach's own size report resizes every window of the session
    // before discovery, and the layout probe ahead of the pin read the
    // squashed layout. The session's layouts are read on both sides of
    // the report; a window the report resized keeps its earlier layout.
    let mut rig = fresh_attach(
        |_, n| {
            Some(if n == 0 {
                format!("@0 3 0 %0 {SPLIT}\n@1 2 0 %4 {OTHER_SPLIT}")
            } else {
                format!("@0 3 0 %0 {SQUASHED}\n@1 2 0 %4 {OTHER_SPLIT}")
            })
        },
        // What the pin's own probe (behind the report) reads.
        &format!("3 0 %0 {SQUASHED}"),
    );
    assert!(rig.has("resize-window -t @0 -x 70 -y 35"), "pinned");
    let before = rig.index_of("list-windows -t '$0'").unwrap();
    let report = rig.index_of("refresh-client -C 70x35").unwrap();
    assert!(before < report);
    assert_eq!(rig.count("list-windows -t '$0'"), 2);
    rig.controller
        .set_our_client_name(Some("/dev/ttys003".to_owned()));
    let restore = rig.controller.pristine_layout_restore_commands();
    assert!(
        restore.contains(&format!("select-layout -t @0 '{SPLIT}'")),
        "{restore:#?}"
    );
    assert!(restore.contains(&"refresh-client -t '/dev/ttys003' -C 203x60".to_owned()));
    // @1 kept its size: our report didn't touch it, so it isn't ours.
    assert!(!restore.iter().any(|c| c.contains("@1")), "{restore:#?}");
}

#[test]
fn a_session_our_size_report_left_alone_records_only_what_the_pin_squashes() {
    // Another client keeps the session at its size: the report resized
    // nothing, and the pin's own probe (ahead of the pin) records @0.
    let mut rig = fresh_attach(
        |_, _| Some(format!("@0 3 0 %0 {SPLIT}\n@1 1 0 %3 {OTHER_BEFORE}")),
        &format!("3 0 %0 {SPLIT}"),
    );
    assert!(rig.has("resize-window -t @0 -x 70 -y 35"), "pinned");
    let restore = rig.controller.pristine_layout_restore_commands();
    assert!(
        restore.contains(&format!("select-layout -t @0 '{SPLIT}'")),
        "{restore:#?}"
    );
    assert!(!restore.iter().any(|c| c.contains("@1")), "{restore:#?}");
}

#[test]
fn a_session_switch_records_what_it_squashes() {
    // B150: switching to another session makes us its latest client, and
    // tmux sizes its windows by us: read on both sides of the switch.
    let mut rig = pinned_split(&format!("1 0 %0 {SPLIT}"));
    let probes = Rc::new(RefCell::new(0usize));
    let seen = probes.clone();
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("list-windows -t '$1'") {
            *seen.borrow_mut() += 1;
            let layout = if *seen.borrow() == 1 {
                OTHER_SPLIT
            } else {
                SQUASHED
            };
            return ok([format!("@5 2 0 %7 {layout}")]);
        }
        None
    }));
    let base = rig.commands.len();
    rig.with(|c, now| c.select_session("$1", now));
    rig.settle();
    let fresh = rig.since(base).to_vec();
    let before = fresh
        .iter()
        .position(|c| c.starts_with("list-windows -t '$1'"));
    // No `%session-changed` in this rig, so no version read: the plain
    // switch (B812). The group's is `group.rs`'s.
    let switch = fresh
        .iter()
        .position(|c| c.starts_with("switch-client -t $1"));
    let after = fresh
        .iter()
        .rposition(|c| c.starts_with("list-windows -t '$1'"));
    assert!(
        before < switch && switch < after && before.is_some(),
        "{fresh:#?}"
    );
    let restore = rig.controller.pristine_layout_restore_commands();
    assert!(
        restore.contains(&format!("select-layout -t @5 '{OTHER_SPLIT}'")),
        "{restore:#?}"
    );
}

/// B770: switching to another session hands back the window left, as
/// switching windows does — its pin, our lease, the layout we found. It
/// stayed at the phone's grid (`window-size manual`): a desktop on that
/// session saw its window squashed until the phone came back to it or left
/// (live, a private tmux 3.6a: a desktop on main, the phone onto dev, main's
/// window stayed 50×20).
#[test]
fn a_session_switch_hands_back_the_window_left() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    assert!(rig.controller.resized_windows().contains("@0"));
    let base = rig.commands.len();
    rig.with(|c, now| c.select_session("$1", now));
    rig.settle();
    let fresh = rig.since(base).to_vec();
    let switch = fresh
        .iter()
        .position(|c| c.starts_with("switch-client -t $1"))
        .expect("onto $1 (no version read in this rig, B812)");
    let unpin = fresh
        .iter()
        .position(|c| c == "set-option -u -w -t @0 window-size")
        .unwrap_or_else(|| panic!("@0 handed back: {fresh:#?}"));
    let layout = fresh
        .iter()
        .position(|c| *c == format!("select-layout -t '@0' '{SPLIT}'"))
        .unwrap_or_else(|| panic!("@0's layout back: {fresh:#?}"));
    // Once the phone no longer sizes main: after the switch.
    assert!(switch < unpin && unpin < layout, "{fresh:#?}");
    assert!(!rig.controller.resized_windows().contains("@0"));
}

/// Round thirty-four: tmux sizes a session for a switch at the end of its
/// loop, after every command that came with the switch has run (3.6a, a
/// private server: `switch-client -c X -t b \; display -p -t b:0
/// '#{window_width}'` prints the old width, the next command the new one).
/// The read right behind the switch found the windows as they were, the
/// sizes equal, and recorded nothing. It goes out once the first read's
/// reply is in: a later loop.
#[test]
fn a_session_switchs_second_read_waits_for_the_first_ones_reply() {
    let mut rig = pinned_split(&format!("1 0 %0 {SPLIT}"));
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("list-windows -t '$1'") {
            return ok([format!("@5 2 0 %7 {OTHER_SPLIT}")]);
        }
        None
    }));
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.select_session("$1", now));
    let sent = rig.since(base).to_vec();
    let reads = |commands: &[String]| {
        commands
            .iter()
            .filter(|c| c.starts_with("list-windows -t '$1'"))
            .count()
    };
    assert_eq!(reads(&sent), 1, "{sent:#?}");
    assert!(sent.iter().any(|c| c.starts_with("switch-client -t $1")));
    rig.settle();
    assert_eq!(reads(rig.since(base)), 2, "{:#?}", rig.since(base));
}

#[test]
fn a_refused_size_probe_records_nothing() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, now| c.begin_control_mode(now));
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-windows -t") {
            return moshpit_tmux::rig::err(["can't find session"]);
        }
        None
    }));
    rig.boot();
    rig.push("%session-changed $0 main\n");
    rig.settle();
    assert!(rig.controller.pristine_layout_restore_commands().is_empty());
}

#[test]
fn leaving_a_window_keeps_its_record_when_the_replay_did_not_take() {
    // B150: while we still size the session, the replay on leaving a
    // window is refitted to our grid; the record stays for the hand-back
    // that can restore it.
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.push("%window-add @1\n");
    rig.respond(Box::new(|command, _| {
        if command.starts_with("display-message -p -t '@0' '#{window_layout}'") {
            return ok([SQUASHED]);
        }
        if command.starts_with("list-windows") {
            return ok([
                format!("$0 @0 0 {SQUASHED} 0 3 main"),
                "$0 @1 1 b25d,70x35,0,0,3 1 1 main".to_owned(),
            ]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok([
                "%0 @0 0 70 17 1 0 0 bash",
                "%1 @0 1 35 17 0 0 0 bash",
                "%2 @0 2 34 17 0 0 0 bash",
                "%3 @1 0 70 35 1 0 0 bash",
            ]);
        }
        None
    }));
    rig.settle();
    rig.with(|c, now| c.select_window("@1", now));
    rig.settle();
    assert!(rig.has(&format!("select-layout -t '@0' '{SPLIT}'")));
    assert!(rig.has("display-message -p -t '@0' '#{window_layout}'"));
    let base = rig.commands.len();
    rig.with(|c, now| c.release_window_pins(now));
    assert!(
        rig.since(base)
            .contains(&format!("select-layout -t '@0' '{SPLIT}'")),
        "{:#?}",
        rig.since(base)
    );
}

#[test]
fn leaving_a_window_drops_its_record_when_the_replay_took() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.push("%window-add @1\n");
    rig.respond(Box::new(|command, _| {
        if command.starts_with("display-message -p -t '@0' '#{window_layout}'") {
            return ok([SPLIT]);
        }
        if command.starts_with("list-windows") {
            return ok([
                format!("$0 @0 0 {SPLIT} 0 3 main"),
                "$0 @1 1 b25d,70x35,0,0,3 1 1 main".to_owned(),
            ]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok([
                "%0 @0 0 101 30 1 0 0 bash",
                "%1 @0 1 101 29 0 0 0 bash",
                "%2 @0 2 101 29 0 0 0 bash",
                "%3 @1 0 70 35 1 0 0 bash",
            ]);
        }
        None
    }));
    rig.settle();
    rig.with(|c, now| c.select_window("@1", now));
    rig.settle();
    assert!(rig.controller.pristine_layout_restore_commands().is_empty());
}

#[test]
fn repinning_reads_the_layouts_around_its_size_report() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-windows -a") {
            return ok([format!("$0 @0 0 {SPLIT} 1 3 main")]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok([
                "%0 @0 0 203 30 1 0 0 bash",
                "%1 @0 1 101 29 0 0 0 bash",
                "%2 @0 2 101 29 0 0 0 bash",
            ]);
        }
        None
    }));
    rig.push("%session-changed $0 main\n");
    rig.settle();
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
    rig.with(|c, now| c.release_window_pins(now));
    let base = rig.commands.len();
    rig.with(|c, now| c.repin_active_window(now));
    let fresh = rig.since(base).to_vec();
    let before = fresh
        .iter()
        .position(|c| c.starts_with("list-windows -t '$0'"));
    let report = fresh
        .iter()
        .position(|c| c.starts_with("refresh-client -C 70x35"));
    let after = fresh
        .iter()
        .rposition(|c| c.starts_with("list-windows -t '$0'"));
    assert!(
        before < report && report < after && before.is_some(),
        "{fresh:#?}"
    );
}

#[test]
fn repinning_after_a_release_records_again_and_zooms() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.with(|c, now| c.release_window_pins(now));
    let base = rig.commands.len();
    rig.with(|c, now| c.repin_active_window(now));
    rig.settle();
    let fresh = rig.since(base);
    let probe = fresh
        .iter()
        .position(|c| c.contains("#{window_layout}") && c.contains("@0"))
        .unwrap();
    let pin = fresh
        .iter()
        .position(|c| c.starts_with("resize-window -t @0"))
        .unwrap();
    assert!(probe < pin);
    assert!(fresh.iter().any(|c| c.contains("#{window_zoomed_flag}")));
}

// MARK: - The reconnect handoff

#[test]
fn the_pin_ledger_hands_pins_over() {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.wait(200);
    assert!(rig.controller.resized_windows().contains("@0"));
    let ledger = rig.controller.take_pin_ledger();
    assert_eq!(ledger.resized_windows, BTreeSet::from(["@0".to_owned()]));
    assert!(rig.controller.resized_windows().is_empty());
    let mut replacement = Rig::new(Config::default());
    replacement.with(|c, _| c.inherit_pin_ledger(ledger));
    replacement
        .with(|c, _| c.inherit_pin_ledger(PinLedger::with_pins(BTreeSet::from(["@4".to_owned()]))));
    assert_eq!(
        replacement.controller.resized_windows(),
        &BTreeSet::from(["@0".to_owned(), "@4".to_owned()])
    );
    assert!(PinLedger::with_pins(BTreeSet::new()).is_empty());
}

#[test]
fn a_held_resize_is_released_by_the_frame_that_answers_it() {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.settle();
    let deferred = rig.with(|c, now| c.view_should_defer_resize("%0", 70, 30, now));
    assert!(deferred);
    // A size that commits nothing holds nothing.
    let again = rig.with(|c, now| c.view_should_defer_resize("%0", 70, 30, now));
    assert!(!again);
    let seen = rig.seen.len();
    rig.wait(200);
    rig.settle();
    let actions: Vec<_> = rig.seen[seen..].iter().collect();
    let apply = actions
        .iter()
        .position(|s| {
            matches!(
                s,
                moshpit_tmux::rig::Seen::View(
                    _,
                    moshpit_tmux::controller::ViewAction::ApplyDeferredResize
                )
            )
        })
        .unwrap();
    let frame = actions
        .iter()
        .position(
            |s| matches!(s, moshpit_tmux::rig::Seen::Pane(_, b) if b.starts_with(b"\x1b[0m\x1b[H\x1b[K\x1b[2;1H\x1b[J")),
        )
        .unwrap();
    assert!(
        apply < frame,
        "the resize lands in the same turn, right before its frame"
    );
}

// MARK: - Attaching unseen

/// tmux answering discovery with one session, window and pane.
fn tree_responder(tmux: Shared) -> moshpit_tmux::rig::Responder {
    let mut inner = responder(tmux);
    Box::new(move |command, index| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-windows") {
            return ok(["$0 @0 0 200x50,0,0,0 1 1 main"]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok(["%0 @0 0 200 50 1 0 0 bash"]);
        }
        inner(command, index)
    })
}

/// A controller in control mode, attached by tmux's `%session-changed`
/// (the session's boot), released first when `unseen`.
fn boot(unseen: bool) -> Rig {
    let mut rig = Rig::new(Config {
        size_owner_label: "iPhone".into(),
        ..Config::default()
    });
    rig.with(|c, now| {
        c.set_initial_client_size(70, 35);
        c.set_our_client_name(Some("moshpit-a".into()));
        if unseen {
            c.release_window_pins(now);
        }
        c.begin_control_mode(now);
    });
    rig.respond(tree_responder(Rc::new(RefCell::new(Tmux::default()))));
    rig.boot();
    rig.push("%session-changed $0 main\n");
    rig.settle();
    rig
}

#[test]
fn an_attach_nobody_watches_stays_out_of_the_sizing_until_shown() {
    // Shown: the attach claims and pins the window, as ever.
    let shown = boot(false);
    assert!(shown.has(PIN), "{:?}", shown.commands);
    assert!(!shown.has("refresh-client -f ignore-size"));

    // A reconnect while the app is on Home: no pin, and out of the window's
    // sizing before its size is ever counted.
    let mut unseen = boot(true);
    assert!(!unseen.has(PIN), "{:?}", unseen.commands);
    assert!(!unseen.has(CLAIM));
    let ignore = unseen.index_of("refresh-client -f ignore-size").unwrap();
    let size = unseen.index_of("refresh-client -C").unwrap();
    assert!(ignore < size, "{:?}", unseen.commands);

    // Shown at last: it joins the sizing and pins.
    unseen.with(|c, now| c.repin_active_window(now));
    unseen.wait_settled(2_000, 100);
    assert!(unseen.has("refresh-client -f !ignore-size"));
    assert!(unseen.has(PIN), "{:?}", unseen.commands);
}

/// B270: mosh's renderer — tmux's client, mosh-server's program — sizes
/// the session from the moment it attaches, before the sidecar exists: with
/// the phone the only client, every window is at the phone's grid when the
/// sidecar first looks, and the probe ahead of its pin recorded the squashed
/// layout (handed back on the disconnect at the phone's size). The sidecar
/// is told how the windows were before the renderer, lists them again at its
/// first attach, and every window the renderer resized keeps its earlier
/// layout as its record; one it left alone records nothing.
#[test]
fn the_sidecar_records_the_layouts_the_renderer_squashed_before_it_attached() {
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    let lists = Rc::new(RefCell::new(0usize));
    let seen = lists.clone();
    rig.respond(Box::new(move |command, _| {
        if command == format!("list-windows -a -F '{SIZE_PROBE_FORMAT}'") {
            *seen.borrow_mut() += 1;
            return ok([
                format!("@0 3 0 %0 {SQUASHED}"),
                format!("@1 1 0 %3 {OTHER_BEFORE}"),
            ]);
        }
        None
    }));
    rig.with(|c, _| {
        c.note_layouts_before_renderer(&format!(
            "@0 3 0 %0 {SPLIT}\n@1 1 0 %3 {OTHER_BEFORE}\n@2 2 0 %5 {OTHER_SPLIT}\n"
        ))
    });
    rig.boot();
    rig.push("%session-changed $0 main\n");
    rig.settle();
    assert_eq!(
        *lists.borrow(),
        1,
        "the windows weren't listed again at the attach"
    );
    let ledger = rig.with(|c, _| c.take_pin_ledger());
    assert_eq!(
        ledger.pristine_layouts.get("@0").map(|p| p.layout.as_str()),
        Some(SPLIT),
        "{ledger:?}"
    );
    // @1 is one pane, @2 wasn't resized (gone from the listing): nothing.
    assert_eq!(ledger.pristine_layouts.len(), 1, "{ledger:?}");
    // A second attach (a new control session) lists nothing again.
    rig.push("%exit\n");
    rig.push("%session-changed $0 main\n");
    rig.settle();
    assert_eq!(*lists.borrow(), 1);
}

/// A terminal's own controller is never told about a renderer.
#[test]
fn only_the_sidecar_takes_the_layouts_before_the_renderer() {
    let (mut rig, _) = rig_with(|_| Tmux::default());
    rig.with(|c, _| c.note_layouts_before_renderer(&format!("@0 3 0 %0 {SPLIT}\n")));
    rig.settle();
    assert!(!rig.has(&format!("list-windows -a -F '{SIZE_PROBE_FORMAT}'")));
    assert_eq!(rig.with(|c, _| c.renderer_detach_command("tmux")), None);
}

// MARK: - Only over a layout of ours (round thirty-four)

/// The squashed SPLIT after the keyboard came up: 70×20.
const SQUASHED_SHORTER: &str = "a1b2,70x20,0,0[70x10,0,0,0,70x9,0,11{35x9,0,11,1,34x9,36,11,2}]";
/// The desktop's own arrangement at the phone's size: stacked.
const REARRANGED: &str = "c3d4,70x35,0,0[70x11,0,0,0,70x11,0,12,1,70x11,0,24,2]";

fn hand_back(rig: &mut Rig) -> String {
    rig.with(|c, _| c.pin_hand_back_script("tmux", &[]))
        .expect("something is owed")
}

/// With a desktop attached, the disconnect replayed the record over
/// whatever the window had become. Now the record knows the layouts our
/// sizing left the window in — tmux's reflow at our pin, and that reflow
/// reflowed again when our grid moved — and the teardown script replays it
/// only over one of them, read before anything is unpinned.
#[test]
fn the_hand_back_replays_a_record_only_over_a_layout_of_ours() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    // tmux's word on our pin, then on the keyboard coming up.
    rig.push(&format!("%layout-change @0 {SQUASHED} {SQUASHED} *\n"));
    rig.push(&format!(
        "%layout-change @0 {SQUASHED_SHORTER} {SQUASHED_SHORTER} *\n"
    ));
    let script = hand_back(&mut rig);
    let read = script
        .find("l0=$(tmux -C display-message -p -t @0")
        .unwrap_or_else(|| panic!("{script}"));
    let unpin = script
        .find("tmux set-option -u -w -t @0 window-size")
        .unwrap();
    assert!(read < unpin, "{script}");
    // The app's Swift controller builds the same string
    // (`pinHandBackScript`, TmuxSessionControllerTests).
    let steps = [
        "l0=$(tmux -C display-message -p -t @0 '#{window_layout}' </dev/null 2>/dev/null | grep -v '^%' | head -n 1)".to_owned(),
        "tmux set-option -u -w -t @0 window-size".to_owned(),
        format!(
            "case \"$l0\" in \"{SPLIT}\"|\"{SQUASHED}\"|\"{SQUASHED_SHORTER}\") \
             tmux resize-window -t @0 -x 203 -y 60; tmux select-layout -t @0 '{SPLIT}'; \
             tmux set-option -wu -t @0 @moshpit_layout; \
             tmux set-option -u -w -t @0 window-size;; esac"
        ),
        "true".to_owned(),
    ];
    assert_eq!(
        script,
        format!("sh -c {}", moshpit_herdr::launch::quote(&steps.join("; ")))
    );
    assert_eq!(rig.with(|c, _| c.pin_hand_back_script("tmux", &[])), None);
}

/// A rearrangement at the same size is not ours: nothing it leads to is.
#[test]
fn a_rearrangement_by_someone_else_is_not_a_layout_of_ours() {
    let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
    rig.push(&format!("%layout-change @0 {SQUASHED} {SQUASHED} *\n"));
    rig.push(&format!("%layout-change @0 {REARRANGED} {REARRANGED} *\n"));
    rig.push(&format!(
        "%layout-change @0 {SQUASHED_SHORTER} {SQUASHED_SHORTER} *\n"
    ));
    let script = hand_back(&mut rig);
    assert!(
        script.contains(&format!("case \"$l0\" in \"{SPLIT}\"|\"{SQUASHED}\") ")),
        "{script}"
    );
    assert!(!script.contains(&format!("\"{REARRANGED}\"")), "{script}");
}

/// B776: the listing mosh's bootstrap takes before the renderer (B270),
/// run by a shell against a stand-in tmux 3.8: a plain client gets JSON
/// layouts, which no layout the sidecar reads is; every window read as
/// squashed by the renderer, its record a JSON layout the hand-back never
/// matched. Read as a control client, the rows are as the sidecar's own.
#[test]
fn the_bootstraps_layout_listing_reads_as_the_sidecar_does_on_tmux_3_8() {
    const JSON: &str = r#"{"V":2,"L":{"t":"p","w":80,"h":24,"x":0,"y":0,"a":true,"i":0,"I":"%0"}}"#;
    let row = |layout: &str| format!("@0 1 0 %0 {layout} latest ");
    let dir = std::env::temp_dir().join(format!("moshpit-b776-listing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("tmux");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n\
             \"-C list-windows\"*) printf '%%begin 1 1 0\\n%s\\n%%end 1 1 0\\n%%exit\\n' '{}';;\n\
             list-windows*) printf '%s\\n' '{}';;\nesac\n",
            row("b25d,80x24,0,0,0"),
            row(JSON)
        ),
    )
    .unwrap();
    std::process::Command::new("chmod")
        .args(["+x", &fake.display().to_string()])
        .status()
        .unwrap();
    let command = moshpit_tmux::controller::layout_listing_command(&fake.display().to_string());
    let out = std::process::Command::new("/bin/sh")
        .args(["-c", &command])
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        listing.trim_end(),
        row("b25d,80x24,0,0,0").trim_end(),
        "{command}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The script, run by a shell against a stand-in tmux that answers the read
/// with the window's layout and logs the rest: replayed over ours, left
/// alone over the desktop's rearrangement. B776: tmux 3.8 writes a plain
/// client's `#{window_layout}` as JSON (`{"V":2,…}`), a control client's
/// as before; read plainly, it matched no layout of ours, and the window
/// was left zoomed at the phone's layout. The stand-in plays both.
#[test]
fn the_hand_back_script_runs_as_it_reads() {
    const JSON: &str = r#"{"V":2,"L":{"t":"p","w":70,"h":35,"x":0,"y":0,"a":true,"i":0,"I":"%0"}}"#;
    for tmux_38 in [false, true] {
        for (current, replayed) in [(SQUASHED, true), (REARRANGED, false)] {
            let mut rig = pinned_split(&format!("3 0 %0 {SPLIT}"));
            rig.push(&format!("%layout-change @0 {SQUASHED} {SQUASHED} *\n"));
            let dir = std::env::temp_dir().join(format!(
                "moshpit-r34-hand-back-{}-{}-{}",
                std::process::id(),
                u8::from(tmux_38),
                u8::from(replayed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let log = dir.join("log");
            let fake = dir.join("tmux");
            let plain = if tmux_38 { JSON } else { current };
            std::fs::write(
                &fake,
                format!(
                    "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n\
                     \"-C display-message\"*) printf '%%begin 1 1 0\\n%s\\n%%end 1 1 0\\n%%exit\\n' '{current}';;\n\
                     display-message*) printf '%s\\n' '{plain}';;\nesac\n",
                    log.display()
                ),
            )
            .unwrap();
            std::process::Command::new("chmod")
                .args(["+x", &fake.display().to_string()])
                .status()
                .unwrap();
            let script = rig
                .with(|c, _| c.pin_hand_back_script(&fake.display().to_string(), &[]))
                .unwrap();
            let status = std::process::Command::new("/bin/sh")
                .args(["-c", &script])
                .status()
                .unwrap();
            assert!(status.success());
            let ran = std::fs::read_to_string(&log).unwrap();
            assert_eq!(
                ran.contains(&format!("select-layout -t @0 {SPLIT}")),
                replayed,
                "tmux 3.8 {tmux_38}, over {current}: {ran}"
            );
            assert!(ran.contains("set-option -u -w -t @0 window-size"), "{ran}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
