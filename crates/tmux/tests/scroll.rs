//! Scroll routing, taps and swipes, ported from the scroll cases of
//! `TmuxSessionControllerTests.swift`: a program with the mouse on gets the
//! wheel, a shell its local scrollback, an alternate-screen pane without the
//! mouse the arrow keys (B500); never tmux's copy-mode.

use moshpit_tmux::controller::{
    Config, PixelScroll, ScrollRoute, ViewAction, click_bytes, wheel_bytes,
};
use moshpit_tmux::rig::{Rig, ok};

fn attached(alternate: bool) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(70, 30));
    rig.attach();
    rig.push(&format!(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 {} 0 bash\n%end 3 3 1\n\n",
        u8::from(alternate)
    ));
    rig.answered = 3;
    rig.settle();
    rig
}

/// Answer the flag probe with `mouse alternate`.
fn arm_flags(rig: &mut Rig, mouse: bool, alternate: bool) {
    let flags = format!("{} {}", u8::from(mouse), u8::from(alternate));
    rig.respond(Box::new(move |command, _| {
        if command.contains("#{mouse_any_flag}") {
            return ok([flags.clone()]);
        }
        ok(["0 0"])
    }));
    rig.with(|c, now| c.refresh_active_pane_mouse(now));
    rig.settle();
}

fn wheels(rig: &Rig, from: usize) -> usize {
    rig.since(from)
        .iter()
        .filter(|c| c.starts_with("send-keys -t %0 -H 1b 5b 3c"))
        .count()
}

#[test]
fn a_swipe_while_streaming_parks_locally_never_copy_mode_or_wheel() {
    let mut rig = attached(false);
    arm_flags(&mut rig, true, false);
    rig.controller.config_mut().streaming_window = 60.0;
    for i in 0..120 {
        rig.push(&format!("%output %0 stream-line-{i}\\015\\012\n"));
    }
    let before = rig.commands.len();
    rig.with(|c, now| c.scroll_lines(3, now));
    let sent = rig.since(before);
    assert!(
        !sent
            .iter()
            .any(|c| c.starts_with("copy-mode") || c.contains("-X scroll-up"))
    );
    assert_eq!(wheels(&rig, before), 0);
    assert_eq!(
        rig.view_actions("%0").last(),
        Some(&ViewAction::ScrollLocal { lines: 3 })
    );
    // Typing snaps back to live.
    let seen = rig.seen.len();
    rig.with(|c, now| c.send_input(b"k", "%0", now));
    assert!(rig.seen[seen..].iter().any(|s| matches!(
        s,
        moshpit_tmux::rig::Seen::View(_, ViewAction::ReleaseScrollHold)
    )));
}

#[test]
fn an_idle_mouse_app_gets_the_wheel() {
    let mut rig = attached(false);
    arm_flags(&mut rig, true, false);
    let before = rig.commands.len();
    rig.with(|c, now| c.scroll_lines(3, now));
    assert_eq!(wheels(&rig, before), 1);
    let expected: Vec<String> = wheel_bytes(3, 35, 15)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        rig.commands.last().unwrap(),
        &format!("send-keys -t %0 -H {}", expected.join(" "))
    );
}

#[test]
fn an_alt_screen_mouse_app_gets_the_wheel_even_while_painting() {
    let mut rig = attached(true);
    arm_flags(&mut rig, true, true);
    rig.controller.config_mut().streaming_window = 60.0;
    rig.push("%output %0 repaint\n");
    let before = rig.commands.len();
    rig.with(|c, now| c.scroll_lines(-2, now));
    assert_eq!(wheels(&rig, before), 1);
    assert!(
        rig.commands.last().unwrap().contains("3c 36 35 3b"),
        "button 65 = newer"
    );
}

/// B500 (the owner's call on xterm's alternate scroll): an alternate-screen
/// pane without the mouse (less, man, git log) gets the arrow keys, one
/// `send-keys` a tick for tmux to encode by the pane's real cursor-key
/// mode; a drag's pixels the same, whole rows, the rest carried. It got
/// nothing: a swipe over a pager moved nothing.
#[test]
fn an_alt_screen_pane_without_the_mouse_gets_the_arrow_keys() {
    let mut rig = attached(true);
    arm_flags(&mut rig, false, true);
    assert_eq!(
        rig.controller.scroll_route().unwrap().1,
        ScrollRoute::Arrows
    );
    let before = rig.commands.len();
    let seen = rig.seen.len();
    rig.with(|c, now| c.scroll_lines(3, now));
    rig.with(|c, now| c.scroll_lines(-2, now));
    assert_eq!(
        rig.since(before),
        ["send-keys -t %0 -N 3 Up", "send-keys -t %0 -N 2 Down"],
        "older is up"
    );
    assert_eq!(rig.seen.len(), seen, "nothing for the view");
    let before = rig.commands.len();
    let used = rig.with(|c, now| c.scroll_pixels(45.0, 30.0, now));
    assert_eq!(used, PixelScroll::Consumed(45.0));
    let used = rig.with(|c, now| c.scroll_pixels(45.0, 30.0, now));
    assert_eq!(used, PixelScroll::Consumed(45.0));
    assert_eq!(
        rig.since(before),
        ["send-keys -t %0 -N 1 Up", "send-keys -t %0 -N 2 Up"],
        "a row and a half, then the half carried"
    );
}

/// B500 behind B485's paste gate: a swipe over a pager right after a paste
/// waits with what was typed after it, in the order given (the paste, the
/// arrows, the key), once tmux has run the paste. The arrows went at once,
/// ahead of the paste's wait and of the key held behind it.
#[test]
fn arrows_after_a_paste_wait_behind_it_in_order() {
    let mut rig = attached(true);
    arm_flags(&mut rig, false, true);
    rig.settle();
    let before = rig.commands.len();
    rig.with(|c, now| c.send_paste("p", "%0", now));
    rig.with(|c, now| c.scroll_lines(2, now));
    rig.with(|c, now| c.send_input(b"q", "%0", now));
    assert_eq!(
        rig.since(before).len(),
        2,
        "the paste only: {:?}",
        rig.since(before)
    );
    rig.answer_pending();
    rig.wait(150);
    assert_eq!(
        rig.since(before)[2..],
        ["send-keys -t %0 -N 2 Up", "send-keys -t %0 -H 71"]
    );
}

#[test]
fn a_plain_shell_drag_is_the_hosts_local_scroll_and_docks_at_the_end() {
    let mut rig = attached(false);
    assert!(!rig.controller.active_input_flags().wants_mouse);
    let before = rig.commands.len();
    let result = rig.with(|c, now| c.scroll_pixels(3.5 * 16.0 + 0.1234, 16.0, now));
    assert_eq!(result, PixelScroll::Local { pane: "%0".into() });
    assert_eq!(rig.commands.len(), before);
    rig.with(|c, now| c.scroll_did_end(now));
    assert_eq!(
        rig.view_actions("%0").last(),
        Some(&ViewAction::SettleLocalScroll)
    );
    rig.push("%output %0 still-live\n");
    assert!(rig.pane_bytes("%0").ends_with(b"still-live"));
}

#[test]
fn pixel_travel_becomes_whole_wheel_rows_with_the_rest_carried() {
    let mut rig = attached(false);
    arm_flags(&mut rig, true, true);
    let before = rig.commands.len();
    // 10 px per tick against a 16 px cell: rows at 16, 32, 48 → 3 wheels
    // over 5 ticks (50 px).
    for _ in 0..5 {
        let consumed = rig.with(|c, now| c.scroll_pixels(10.0, 16.0, now));
        assert_eq!(consumed, PixelScroll::Consumed(10.0));
    }
    assert_eq!(wheels(&rig, before), 3);
    // A drag's start drops the carry and re-reads the flags.
    rig.with(|c, now| c.scroll_began(now));
    assert!(rig.commands.last().unwrap().contains("#{mouse_any_flag}"));
    rig.settle();
    let before = rig.commands.len();
    rig.with(|c, now| c.scroll_pixels(10.0, 16.0, now));
    assert_eq!(wheels(&rig, before), 0, "the 2 px left over were dropped");
    // An absurd travel saturates instead of trapping.
    rig.with(|c, now| c.scroll_pixels(1e300, 16.0, now));
}

#[test]
fn a_tap_is_a_click_only_for_a_mouse_app() {
    let mut rig = attached(false);
    let before = rig.commands.len();
    rig.with(|c, now| c.click(4, 2, now));
    assert!(!rig.since(before).iter().any(|c| c.starts_with("send-keys")));
    assert!(
        rig.since(before)
            .iter()
            .any(|c| c.contains("#{mouse_any_flag}"))
    );
    arm_flags(&mut rig, true, false);
    rig.with(|c, now| c.click(4, 2, now));
    let click: Vec<String> = b"\x1b[<0;5;3M\x1b[<0;5;3m"
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(rig.has(&format!("send-keys -t %0 -H {}", click.join(" "))));
}

#[test]
fn a_flag_probe_refused_keeps_the_flags() {
    let mut rig = attached(false);
    arm_flags(&mut rig, true, true);
    rig.respond(Box::new(|command, _| {
        if command.contains("#{mouse_any_flag}") {
            return moshpit_tmux::rig::err(["can't find pane: %0"]);
        }
        None
    }));
    rig.with(|c, now| c.refresh_active_pane_mouse(now));
    rig.settle();
    assert!(rig.controller.active_input_flags().wants_mouse);
}

#[test]
fn swipes_cycle_panes_then_windows() {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 2 main\n$0 @1 1 81x24,0,0,1 0 1 logs\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @0 1 80 24 0 0 0 vim\n%2 @1 0 80 24 1 0 0 tail\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.with(|c, now| c.switch_pane_or_window(true, now));
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%1")
    );
    assert!(rig.controller.snapshot().has_swiped_since_attach);
    rig.with(|c, now| c.switch_pane_or_window(true, now));
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%0")
    );
    assert!(rig.controller.snapshot().last_switch_forward);
    // A one-pane window cycles windows.
    rig.with(|c, now| c.select_window("@1", now));
    rig.with(|c, now| c.switch_pane_or_window(false, now));
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@0")
    );
    assert!(!rig.controller.snapshot().last_switch_forward);
}

#[test]
fn wheel_bytes_are_clamped_to_six() {
    assert_eq!(wheel_bytes(0, 1, 1), b"");
    assert_eq!(wheel_bytes(1, 0, 0), b"\x1b[<64;1;1M");
    assert_eq!(wheel_bytes(-100, 3, 4), b"\x1b[<65;3;4M".repeat(6));
    assert_eq!(wheel_bytes(i64::MIN, 1, 1), b"\x1b[<65;1;1M".repeat(6));
    assert_eq!(wheel_bytes(i64::MAX, 1, 1), b"\x1b[<64;1;1M".repeat(6));
}

/// Tap-to-position: an SGR button-0 press and its release at the 1-based
/// cell; the origin and anything left of or above it is 1;1, and a cell at
/// i64's end saturates instead of overflowing (B149, the Swift
/// `clickBytes` this replaced).
#[test]
fn a_click_is_a_press_and_its_release_at_the_one_based_cell() {
    assert_eq!(click_bytes(8, 29), b"\x1b[<0;9;30M\x1b[<0;9;30m");
    assert_eq!(click_bytes(0, 0), b"\x1b[<0;1;1M\x1b[<0;1;1m");
    assert_eq!(click_bytes(-3, -1), b"\x1b[<0;1;1M\x1b[<0;1;1m");
    let max = i64::MAX.to_string();
    assert_eq!(
        click_bytes(i64::MAX, i64::MAX),
        format!("\x1b[<0;{max};{max}M\x1b[<0;{max};{max}m").into_bytes()
    );
    assert_eq!(click_bytes(i64::MIN, i64::MIN), b"\x1b[<0;1;1M\x1b[<0;1;1m");
}

// MARK: - B363: the flags per pane, from every repaint; Claude scrolls itself

/// Attached to one pane running `command` (`alternate` as discovery says),
/// tmux answering every repaint's cursor query with `cursor_reply`.
fn attached_running(command: &str, alternate: bool, cursor_reply: &'static str) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(70, 30));
    rig.attach();
    rig.respond(Box::new(move |command, _| {
        if command.contains("'#{cursor_x} #{cursor_y}") {
            return ok([cursor_reply]);
        }
        ok(["0 0"])
    }));
    rig.push(&format!(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 {} 0 {command}\n%end 3 3 1\n\n",
        u8::from(alternate)
    ));
    rig.answered = 3;
    rig.settle();
    rig
}

fn flag_probes(rig: &Rig) -> usize {
    rig.commands
        .iter()
        .filter(|c| c.ends_with("'#{mouse_any_flag} #{alternate_on}'"))
        .count()
}

/// The attach's own repaint brings the program's flags: a full-screen app
/// running when the phone attaches (every reconnect) gets its first tick as
/// a wheel, with no drag-start probe first. They used to be unread (false,
/// false) after every attach, and the first ticks paged the local history:
/// old screens under a full-screen program.
#[test]
fn the_attachs_repaint_brings_the_flags_and_the_first_tick_is_the_wheel() {
    let mut rig = attached_running("vim", true, "2 1 1 1");
    let flags = rig.controller.active_input_flags();
    assert!(
        flags.wants_mouse && flags.alt_screen && flags.scrolls_itself,
        "{flags:?}"
    );
    assert_eq!(
        flag_probes(&rig),
        0,
        "no probe of its own: the repaint asked"
    );
    let before = rig.commands.len();
    rig.with(|c, now| c.scroll_lines(3, now));
    assert_eq!(wheels(&rig, before), 1);
    assert!(
        !rig.view_actions("%0")
            .iter()
            .any(|a| matches!(a, ViewAction::ScrollLocal { .. })),
        "never the local history"
    );
}

/// No flags yet, discovery says the pane is on the alternate screen: the
/// tick is dropped, never the local history (which holds no history of a
/// full-screen program, only stale frames).
#[test]
fn unknown_flags_on_an_alternate_screen_pane_drop_the_tick() {
    let mut rig = attached_running("htop", true, "0 0");
    assert!(rig.controller.active_input_flags().alt_screen);
    let before = rig.commands.len();
    let seen = rig.seen.len();
    rig.with(|c, now| c.scroll_lines(3, now));
    assert_eq!(rig.commands.len(), before);
    assert_eq!(rig.seen.len(), seen, "nothing for the view either");
    assert_eq!(rig.controller.scroll_route().unwrap().1, ScrollRoute::Drop);
}

/// Claude's full-screen renderer on the primary screen (the mouse on, not
/// the alternate screen) gets the wheel even while it repaints: the pane's
/// history there is leftovers of its repaints and of tmux's reflow, not
/// the conversation. It used to be parked in the local history while it
/// streamed (the m1-pro report).
#[test]
fn claude_with_the_mouse_gets_the_wheel_even_while_repainting() {
    for command in ["claude", "2.1.307"] {
        let mut rig = attached_running(command, false, "2 1 1 0");
        rig.controller.config_mut().streaming_window = 60.0;
        rig.push("%output %0 \\015spinner\n");
        let flags = rig.controller.active_input_flags();
        assert!(
            flags.scrolls_itself && !flags.alt_screen,
            "{command}: {flags:?}"
        );
        let before = rig.commands.len();
        rig.with(|c, now| c.scroll_lines(3, now));
        assert_eq!(wheels(&rig, before), 1, "{command}");
    }
}

/// Claude on the primary screen before tmux has said anything about its
/// mouse: either renderer, so nothing rather than a guess — the local
/// history under a full-screen Claude is its repaint leftovers.
#[test]
fn claude_on_the_primary_screen_with_no_flags_yet_drops_the_tick() {
    let mut rig = attached_running("claude", false, "0 0");
    assert!(rig.controller.active_input_flags().holds_ticks);
    assert_eq!(rig.controller.scroll_route().unwrap().1, ScrollRoute::Drop);
    // An inline Claude (mouse off) is its transcript: the local history.
    rig.respond(Box::new(|command, _| {
        if command.contains("'#{cursor_x} #{cursor_y}") {
            return ok(["2 1 0 0"]);
        }
        ok(["0 0"])
    }));
    rig.with(|c, now| c.resync_active_pane_now(false, now));
    rig.settle();
    assert_eq!(rig.controller.scroll_route().unwrap().1, ScrollRoute::Local);
}

/// The flags belong to the pane they were read for: a switch to a shell
/// right after a full-screen app doesn't send that shell the app's wheel
/// while its own flags are on their way (a wheel into a shell types
/// `^[[<64;…M` at its prompt).
#[test]
fn a_panes_flags_never_route_another_panes_ticks() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(70, 30));
    rig.attach();
    rig.respond(Box::new(|command, _| {
        if command.starts_with("display-message -p -t %0 '#{cursor_x}") {
            return ok(["2 1 1 1"]);
        }
        ok(["0 0"])
    }));
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 2 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 1 0 vim\n%1 @0 1 80 24 0 0 0 bash\n%end 3 3 1\n\n");
    rig.answered = 3;
    rig.settle();
    assert!(rig.controller.active_input_flags().scrolls_itself);
    // To the shell; its flags not answered yet.
    rig.with(|c, now| c.select_pane("%1", now));
    let before = rig.commands.len();
    rig.with(|c, now| c.scroll_lines(3, now));
    assert!(
        !rig.since(before)
            .iter()
            .any(|c| c.starts_with("send-keys -t %1 -H 1b 5b 3c")),
        "no wheel into the shell: {:?}",
        rig.since(before)
    );
    assert_eq!(
        rig.controller.scroll_route().unwrap(),
        ("%1".to_owned(), ScrollRoute::Local)
    );
}
