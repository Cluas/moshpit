//! The control client against the fake host (`client::rig`), in virtual
//! time: ordering, input routing, the frame channel's attach and retarget,
//! the push channel. What each pane received on the host, and in which
//! order, is the evidence (`Rig::typed`).

use moshpit_herdr::client::rig::{Fate, Rig, Rule};
use moshpit_herdr::client::{Action, Config, Event, Notice, NoticeAction, Output, ViewAction};
use moshpit_herdr::control::Refusal;

fn config() -> Config {
    Config::default()
}

fn rule(pattern: &str, delay: Option<u64>, fate: Fate) -> Rule {
    Rule {
        pattern: pattern.to_owned(),
        delay,
        fate,
    }
}

/// Started, attached to the focused pane, its first frame painted.
fn attached() -> Rig {
    let mut rig = Rig::new(config());
    rig.start();
    rig.run(500);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    rig
}

/// A focus change the person asked for (B683: the only kind the phone
/// follows once it shows a pane): they pick the tab, and the server's focus
/// lands on `pane`.
fn asked_focus(rig: &mut Rig, pane: &str) {
    rig.act(Action::SelectWindow("w1:t1".to_owned()));
    rig.run(300);
    rig.host.focused = Some(pane.to_owned());
}

fn count(events: &[Event], wanted: impl Fn(&Event) -> bool) -> usize {
    events.iter().filter(|event| wanted(event)).count()
}

// MARK: - Starting

#[test]
fn the_first_read_names_the_pane_and_the_boot_line_attaches_it() {
    let mut rig = Rig::new(config());
    rig.start();
    assert_eq!(rig.commands(""), ["api snapshot"]);
    rig.run(500);
    let writes = rig.frame_writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].starts_with("stty raw -echo; /bin/sh -c "));
    assert!(writes[0].ends_with("moshpit-herdr 'w1:p1' 80 24"));
    assert_eq!(rig.attach_lines(), ["w1:p1"]);
    assert!(rig.screen("w1:p1").contains("[w1:p1 #1]"));
    assert_eq!(count(&rig.events(), |e| *e == Event::FirstFrame), 1);
    // Typing goes into the frame channel, as protocol lines.
    rig.type_text("w1:p1", "ls\r");
    rig.run(100);
    assert_eq!(rig.typed("w1:p1"), "ls\r");
}

#[test]
fn typing_before_the_first_attach_never_reaches_the_channel_and_says_so() {
    // B5: before the attach, the channel's reader is the login shell (or
    // the loop's `read`), which would eat the keystrokes.
    let mut rig = Rig::new(config());
    rig.start();
    rig.type_text("w1:p1", "early");
    assert!(rig.frame_writes().is_empty());
    assert!(rig.events().contains(&Event::InputDropped {
        pane: "w1:p1".to_owned()
    }));
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::InputDropped)
    );
    rig.run(500);
    assert_eq!(rig.typed("w1:p1"), "");
    // The first frame takes the notice down.
    assert_eq!(rig.client.notice(), None);
}

// MARK: - Command order (B7, B26)

#[test]
fn focus_a_then_b_lands_in_that_order_and_the_app_ends_on_b() {
    let mut rig = attached();
    // A's exec is the slow one: on its own Task it would land after B's.
    rig.host
        .rules
        .push(rule("agent focus 'w1:p2'", Some(400), Fate::Normal));
    rig.host
        .rules
        .push(rule("agent focus 'w1:p1'", Some(10), Fate::Normal));
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.act(Action::SelectPane("w1:p1".to_owned()));
    rig.run(3_000);
    let focuses: Vec<&String> = rig
        .host
        .landed
        .iter()
        .filter(|l| l.starts_with("focus"))
        .collect();
    assert_eq!(focuses, ["focus w1:p2", "focus w1:p1"]);
    assert_eq!(rig.host.focused.as_deref(), Some("w1:p1"));
    assert_eq!(
        rig.client.snapshot().active_pane_id.as_deref(),
        Some("w1:p1")
    );
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert!(!rig.client.pane_switch_in_flight());
}

#[test]
fn a_read_answered_before_a_tap_landed_does_not_pull_the_app_back() {
    let mut rig = attached();
    // The next read leaves now and is answered after the tap was made but
    // reports the old focus (it reached the server first).
    rig.host
        .rules
        .push(rule("api snapshot", Some(300), Fate::Normal));
    rig.host
        .rules
        .push(rule("agent focus", Some(500), Fate::Normal));
    rig.act(Action::Refresh);
    rig.run(100);
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(250);
    // The read came back (old focus); the app stays where the tap put it.
    assert_eq!(
        rig.client.snapshot().active_pane_id.as_deref(),
        Some("w1:p2")
    );
    rig.run(3_000);
    assert_eq!(
        rig.client.snapshot().active_pane_id.as_deref(),
        Some("w1:p2")
    );
    let focused: Vec<String> = rig
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::FocusedPane(pane) => Some(pane.clone()),
            _ => None,
        })
        .collect();
    let after_tap = focused.iter().skip_while(|pane| *pane != "w1:p2");
    assert!(after_tap.clone().all(|pane| pane == "w1:p2"), "{focused:?}");
}

#[test]
fn a_tasks_worktree_takes_its_turn_among_the_taps() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("worktree create", Some(800), Fate::Normal));
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.act(Action::StartAgentTask(task_request("")));
    rig.act(Action::SelectPane("w1:p1".to_owned()));
    rig.run(10_000);
    let landed: Vec<&String> = rig
        .host
        .landed
        .iter()
        .filter(|l| l.starts_with("focus") || l.starts_with("worktree"))
        .collect();
    assert_eq!(
        landed,
        ["focus w1:p2", "worktree create fix-scroll", "focus w1:p1"]
    );
    // One command out at a time.
    assert_eq!(rig.host.focused.as_deref(), Some("w1:p1"));
}

#[test]
fn a_command_that_never_answers_holds_the_next_only_for_its_budget() {
    let mut rig = attached();
    rig.host.rules.push(rule("tab create", None, Fate::Hang));
    let start = rig.now;
    rig.act(Action::NewWindow { name: None });
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(4_900);
    assert!(rig.host.landed.iter().all(|l| !l.starts_with("focus")));
    rig.run(300);
    assert!(rig.host.landed.iter().any(|l| l == "focus w1:p2"));
    assert!(rig.now - start >= 5_000);
    // The hung exec was given up (its channel may be closed).
    assert!(
        rig.log
            .iter()
            .any(|(_, output)| matches!(output, Output::Abandon { .. }))
    );
}

#[test]
fn a_sequence_stops_at_a_step_whose_trailer_was_lost() {
    // B121: the app counted a missing trailer as success and went on.
    let mut rig = attached();
    // Nothing printed, no trailer: no evidence the focus landed.
    rig.host
        .rules
        .push(rule("workspace focus", None, Fate::Output(String::new())));
    rig.host
        .workspaces
        .push(("w9".to_owned(), "other".to_owned()));
    rig.act(Action::NewWindowInSession {
        session: "w9".to_owned(),
        name: Some("logs".to_owned()),
    });
    rig.run(2_000);
    assert_eq!(rig.commands("workspace focus").len(), 1);
    assert!(rig.commands("tab create").is_empty());
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::CommandFailed {
            reason: Refusal::CutOff,
            ..
        }
    )));
}

#[test]
fn a_sequence_whose_step_outlasts_the_hold_sends_nothing_after_it() {
    // B191: the app stopped waiting for the step after the lane's hold but
    // let the step go on, so `tab create --focus` landed after a pane tap
    // queued behind it and took the server's focus from the tapped pane.
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("workspace focus", Some(6_000), Fate::Normal));
    rig.host
        .workspaces
        .push(("w9".to_owned(), "other".to_owned()));
    rig.act(Action::NewWindowInSession {
        session: "w9".to_owned(),
        name: Some("logs".to_owned()),
    });
    rig.run(700);
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(15_000);
    // The tap went once the hold ran out; the slow focus still landed on
    // the host (nothing can call it back), but the sequence went no further.
    assert!(rig.host.landed.iter().any(|l| l == "focus w1:p2"));
    assert!(
        rig.commands("tab create").is_empty(),
        "{:?}",
        rig.host.landed
    );
    assert_eq!(
        rig.client.snapshot().active_pane_id.as_deref(),
        Some("w1:p2")
    );
}

#[test]
fn a_cut_off_answer_whose_text_carries_the_marker_is_no_exit_status() {
    // B193: the app took the marker's last occurrence anywhere in the
    // output. An answer cut off before the trailer, whose JSON holds the
    // marker (a label, a title, a cwd: anything a program on the host
    // sets), read as exit status 0 and the sequence went on.
    let mut rig = attached();
    let cut = format!(
        r#"{{"id":"cli:workspace:focus","result":{{"workspace":{{"label":"{}0 build"#,
        moshpit_herdr::control::EXIT_CODE_MARKER
    );
    rig.host
        .rules
        .push(rule("workspace focus", None, Fate::Output(cut)));
    rig.host
        .workspaces
        .push(("w9".to_owned(), "other".to_owned()));
    rig.act(Action::NewWindowInSession {
        session: "w9".to_owned(),
        name: Some("logs".to_owned()),
    });
    rig.run(2_000);
    assert!(rig.commands("tab create").is_empty());
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::CommandFailed {
            reason: Refusal::CutOff,
            ..
        }
    )));
}

// MARK: - Keys and typing (B2)

#[test]
fn typing_and_arrows_land_in_the_order_they_were_pressed() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("pane send-keys", Some(200), Fate::Normal));
    rig.type_text("w1:p1", "ab");
    rig.type_bytes("w1:p1", b"\x1b[A");
    rig.type_text("w1:p1", "cd");
    rig.type_bytes("w1:p1", b"\x1bOB");
    rig.type_text("w1:p1", "e");
    rig.run(2_000);
    assert_eq!(rig.typed("w1:p1"), "ab<up>cd<down>e");
}

/// B360: arrows that reach the client in one chunk (the input channel
/// hands over what was written since its last read) are each a semantic
/// key, among the text around them.
#[test]
fn arrows_that_arrive_together_are_each_a_key() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("pane send-keys", Some(200), Fate::Normal));
    rig.type_bytes("w1:p1", b"\x1b[A\x1b[Ax\x1bOBy");
    rig.run(2_000);
    assert_eq!(rig.typed("w1:p1"), "<up><up>x<down>y");
}

#[test]
fn a_held_arrow_never_fans_out() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("pane send-keys", Some(300), Fate::Normal));
    for _ in 0..6 {
        rig.type_bytes("w1:p1", b"\x1b[A");
        rig.run(70);
    }
    rig.run(2_000);
    assert_eq!(rig.typed("w1:p1"), "<up>".repeat(6));
    let sends = rig.commands("pane send-keys");
    assert!(sends.len() <= 3, "{sends:?}");
}

#[test]
fn a_key_whose_exec_never_answers_cant_freeze_the_typing_behind_it() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("pane send-keys", None, Fate::Hang));
    rig.type_bytes("w1:p1", b"\x1b[A");
    rig.type_text("w1:p1", "x");
    rig.run(2_900);
    assert_eq!(rig.typed("w1:p1"), "");
    rig.run(300);
    assert_eq!(rig.typed("w1:p1"), "x");
}

#[test]
fn a_paste_reaches_herdr_as_one_bracketed_paste_without_inner_markers() {
    let mut rig = attached();
    rig.client
        .paste("w1:p1", "ls\u{1b}[201~\necho INJECTED\n", rig.now);
    rig.pump();
    rig.run(200);
    assert_eq!(
        rig.typed("w1:p1"),
        "\u{1b}[200~ls\necho INJECTED\n\u{1b}[201~"
    );
}

/// An arrow inside a paste is pasted text, not a key (B360's split keeps
/// a bracketed paste whole).
#[test]
fn an_arrow_inside_a_paste_is_pasted_not_pressed() {
    let mut rig = attached();
    rig.client.paste("w1:p1", "a\u{1b}[Ab", rig.now);
    rig.pump();
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "\u{1b}[200~a\u{1b}[Ab\u{1b}[201~");
}

// MARK: - Switching panes (B25)

#[test]
fn typing_at_the_pane_being_left_after_the_switch_started_goes_nowhere_and_says_so() {
    let mut rig = attached();
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.type_text("w1:p1", "y\r");
    rig.run(2_000);
    assert_eq!(rig.typed("w1:p1"), "");
    assert!(rig.events().contains(&Event::InputDropped {
        pane: "w1:p1".to_owned()
    }));
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    // Never into the loop's `read` either: the only lines the loop acted on
    // are the targets.
    assert_eq!(rig.attach_lines(), ["w1:p1", "w1:p2"]);
}

#[test]
fn typing_at_the_pane_just_picked_waits_for_its_attach_and_follows_it() {
    let mut rig = attached();
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.type_text("w1:p2", "ls\r");
    assert!(
        !rig.frame_writes()
            .iter()
            .any(|line| line.contains("terminal.input"))
    );
    rig.run(2_000);
    assert_eq!(rig.typed("w1:p2"), "ls\r");
    assert_eq!(rig.typed("w1:p1"), "");
    assert!(
        !rig.events()
            .iter()
            .any(|e| matches!(e, Event::InputDropped { .. }))
    );
}

/// B493: typing at a pane the desktop switched to, before its attach is
/// up, was dropped and said (herdr drops input that reaches an attach
/// before it is). It waits for the attach now, as a pick's does, and
/// follows it.
#[test]
fn typing_at_a_pane_a_followed_focus_switched_to_waits_for_its_attach() {
    let mut rig = attached();
    // The focus moves where the person asked (B683: the desktop's own
    // moves aren't followed); the next read reports it.
    asked_focus(&mut rig, "w1:p2");
    rig.act(Action::Refresh);
    rig.run(45);
    assert_eq!(rig.client.frame_target(), Some("w1:p2"));
    rig.type_text("w1:p2", "y\r");
    rig.run(2_000);
    assert_eq!(
        rig.typed("w1:p2"),
        "y\r",
        "held, then sent after the first frame"
    );
    assert!(rig.frames.lost.is_empty(), "lost: {:?}", rig.frames.lost);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    rig.type_text("w1:p2", "z");
    rig.run(200);
    assert_eq!(rig.typed("w1:p2"), "y\rz");
}

/// Held two seconds at most: an attach that takes longer drops it, and
/// says so.
#[test]
fn typing_held_for_a_follow_is_dropped_and_said_past_two_seconds() {
    let mut rig = attached();
    rig.frames.attach_ms = 6_000;
    asked_focus(&mut rig, "w1:p2");
    rig.act(Action::Refresh);
    rig.run(100);
    assert_eq!(rig.client.frame_target(), Some("w1:p2"));
    rig.type_text("w1:p2", "y");
    rig.run(2_500);
    assert_eq!(rig.typed("w1:p2"), "");
    assert!(
        rig.events()
            .iter()
            .any(|e| matches!(e, Event::InputDropped { .. }))
    );
    rig.run(6_000);
    assert_eq!(rig.typed("w1:p2"), "", "not sent late");
}

const WIPE: &str = "\u{1b}[H\u{1b}[2J\u{1b}[3J";

/// What `stream` was told to do, in order.
fn views_on(rig: &Rig, stream: moshpit_herdr::client::StreamId) -> Vec<ViewAction> {
    rig.log
        .iter()
        .filter_map(|(_, output)| match output {
            Output::View {
                stream: s, action, ..
            } if *s == stream => Some(action.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_switch_veils_the_new_panes_terminal_and_wipes_it_with_its_full_frame() {
    let mut rig = attached();
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    let p2 = rig.client.streams()["w1:p2"];
    assert!(matches!(
        views_on(&rig, p2).as_slice(),
        [ViewAction::Veil, ViewAction::ExtendCover { .. }]
    ));
    // B445: nothing is wiped under a cover that may come off before the
    // new pane has painted.
    assert_eq!(rig.screen("w1:p2"), "");
    rig.run(1_000);
    let screen = rig.screen("w1:p2");
    assert!(
        screen.starts_with(WIPE),
        "wiped, history too, with its frame: {screen:?}"
    );
    assert!(screen.contains("[w1:p2 #"));
    assert!(views_on(&rig, p2).contains(&ViewAction::Reveal));
    // The pane left keeps its own screen; nothing of p2's is in it.
    assert!(!rig.screen("w1:p1").contains("w1:p2"));
}

/// B445: the attach a switch asked for was refused before it painted. Its
/// cover lifted onto a screen the switch had wiped, and stayed blank until
/// a resize made herdr paint again (leaving the screen and coming back);
/// now nothing was wiped, and the cover lifts over what the pane showed.
#[test]
fn a_switch_refused_before_its_frame_lifts_its_cover_over_an_unwiped_terminal() {
    let mut rig = attached();
    // The release's close takes long: the new attach never gets to paint.
    rig.frames.latency = 10_000;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(100);
    let now = rig.now;
    rig.client
        .frame_bytes(b"herdr: terminal attach refused: busy\r\n", now);
    rig.pump();
    let p2 = rig.client.streams()["w1:p2"];
    assert_eq!(
        views_on(&rig, p2).last(),
        Some(&ViewAction::Reveal),
        "the cover lifts"
    );
    let screen = rig.screen("w1:p2");
    assert!(
        !screen.contains("\u{1b}[2J"),
        "nothing wiped under it: {screen:?}"
    );
}

#[test]
fn frames_from_the_pane_being_released_stay_on_its_own_terminal() {
    let mut rig = attached();
    rig.frames.latency = 50;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    // p1 paints once more before its release reaches the host.
    rig.run(10);
    rig.host_paint(false);
    rig.run(2_000);
    let p1 = rig.screen("w1:p1");
    let p2 = rig.screen("w1:p2");
    assert!(p1.contains("[w1:p1 #2]"), "{p1:?}");
    assert!(!p2.contains("w1:p1"), "{p2:?}");
}

/// B415: the release's close held back past its budget (a slow link, the
/// pane being left streaming output): what arrives before the close is
/// still that pane's. Its late frames used to be drawn on the next pane,
/// lift its veil and let its held input out before its attach was up.
#[test]
fn frames_before_a_late_release_close_stay_the_released_panes() {
    let mut rig = attached();
    // A second each way: the close comes back 2 s after the release left,
    // past the 1.5 s it gets.
    rig.frames.latency = 1_000;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.type_text("w1:p2", "y");
    // p1 paints just before the release reaches the host: on the client
    // after the budget ran out.
    rig.run(900);
    rig.host_paint(false);
    rig.run(1_100);
    let p2 = rig.screen("w1:p2");
    assert!(!p2.contains("w1:p1"), "{p2:?}");
    assert!(rig.screen("w1:p1").contains("[w1:p1 #2]"));
    rig.run(5_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert!(
        rig.screen("w1:p2").contains("[w1:p2"),
        "{:?}",
        rig.screen("w1:p2")
    );
    assert!(!rig.screen("w1:p2").contains("w1:p1"));
    assert!(rig.frames.lost.is_empty(), "lost: {:?}", rig.frames.lost);
    assert_eq!(rig.typed("w1:p2"), "y");
}

/// B419: herdr says something that isn't a frame (`herdr: …`) while a
/// release is out, and the release's close never comes (the attach died
/// with the server). The refusal is said and backed off; once the backoff
/// is over the next switch writes its line. The release it came during
/// held the channel for good: every switch after it waited for that close,
/// writing nothing, until a redial. (Held since B415: an expected close
/// given up lets the release go.)
#[test]
fn a_herdr_line_during_a_release_whose_close_never_comes_leaves_the_channel_free() {
    let mut rig = attached();
    // The close comes back long after every budget, as good as never.
    rig.frames.latency = 10_000;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(100);
    let now = rig.now;
    // A refusal (B466: a server shutting down is no longer one; its line
    // follows the focus at once, see below).
    rig.client
        .frame_bytes(b"herdr: terminal session control failed: busy\r\n", now);
    rig.pump();
    assert!(matches!(rig.client.notice(), Some((Notice::Refused(_), _))));
    rig.run(70_000);
    assert!(
        rig.attach_lines().contains(&"w1:p2".to_owned()),
        "{:?}",
        rig.attach_lines()
    );
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
}

#[test]
fn three_quick_switches_write_one_release_and_still_see_the_next_eviction() {
    // herdr answers two releases with one close. The app wrote one per
    // switch, so after A→B→C it waited for a close that never comes and
    // took the next eviction for it: the target stayed, painted, and
    // typing went into a dead channel.
    let mut rig = attached();
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.act(Action::SelectPane("w1:p1".to_owned()));
    rig.run(2_000);
    let releases = rig
        .frame_writes()
        .iter()
        .filter(|line| line.contains("terminal.release"))
        .count();
    assert_eq!(releases, 1);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert_eq!(rig.attach_lines(), ["w1:p1", "w1:p1"]);
    rig.host_take_over();
    rig.run(20);
    assert_eq!(rig.client.attached_pane(), None);
    // Seen as another device's takeover: said, and the next reads leave
    // the pane with that device.
    rig.run(3_000);
    assert_eq!(rig.attach_lines(), ["w1:p1", "w1:p1"]);
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    // "Open here": taken back, and typing reaches it.
    rig.act(Action::NoticeAction);
    rig.run(500);
    assert_eq!(rig.attach_lines(), ["w1:p1", "w1:p1", "w1:p1"]);
    rig.type_text("w1:p1", "ok");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "ok");
}

#[test]
fn input_ignored_is_not_a_refused_attach() {
    // B117: herdr turns one line away and keeps the attach.
    let mut rig = attached();
    // What herdr 0.9.5 prints for a line it can't parse (probed live).
    rig.client.frame_bytes(
        b"herdr: terminal session control input ignored: invalid json command: expected value at line 1 column 1\r\n",
        rig.now,
    );
    rig.pump();
    rig.run(200);
    assert!(
        rig.events()
            .iter()
            .any(|e| matches!(e, Event::InputIgnored(_)))
    );
    assert_eq!(rig.client.frame_target(), Some("w1:p1"));
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert_eq!(rig.client.notice(), None);
    rig.type_text("w1:p1", "still here");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "still here");
}

/// Another device's `--takeover` ended the attach: the pane was asked for
/// again on the next read — with `--takeover`, taking it from that device —
/// three times in 30 s before a backoff and "retrying shortly", then again:
/// two devices fought over the pane. Now it is said, with "Open here", and
/// left with that device until the person's word.
#[test]
fn a_pane_another_device_takes_over_is_said_and_never_taken_back_on_its_own() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    // Reads come and go, the focus repeats and moves: nothing is taken
    // (B487: once the focus has left it, the pane is asked for plainly,
    // and refused while that device holds it), and typing goes nowhere.
    rig.run(10_000);
    rig.host.focused = Some("w1:p2".to_owned());
    rig.run(10_000);
    assert!(
        rig.frames.took_over.iter().all(|took| !took),
        "taken back on its own"
    );
    assert_eq!(rig.host.held_elsewhere, ["w1:p1"]);
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    rig.type_text("w1:p1", "rm -rf build\r");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "");
    // "Open here": the person's word takes it.
    rig.host.focused = Some("w1:p1".to_owned());
    rig.act(Action::NoticeAction);
    rig.run(500);
    assert_eq!(rig.attach_lines().last().map(String::as_str), Some("w1:p1"));
    assert_eq!(
        rig.frames.took_over.last(),
        Some(&true),
        "the person's word takes it"
    );
    assert_eq!(rig.frames.took_over.iter().filter(|took| **took).count(), 1);
    assert_eq!(rig.host.taken_from_others, ["w1:p1"]);
    assert_eq!(rig.client.notice(), None);
    rig.type_text("w1:p1", "y");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "y");
}

/// B409: "Open here" after the other device moved the shared focus on
/// (the case the test above sets back before the tap): the pane is the
/// server's focus again too, as a pick's is, so the next read keeps the
/// phone on it instead of following the focus to where the other device
/// went.
#[test]
fn open_here_sticks_after_the_other_device_moved_the_focus() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    rig.host.focused = Some("w1:p2".to_owned());
    rig.run(10_000);
    rig.act(Action::NoticeAction);
    rig.run(10_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert_eq!(rig.host.focused.as_deref(), Some("w1:p1"));
    // Plain asks for p1 in between (B487), refused; never the focus's p2.
    assert!(
        !rig.attach_lines().contains(&"w1:p2".to_owned()),
        "followed the focus away: {:?}",
        rig.attach_lines()
    );
    assert_eq!(rig.client.notice(), None);
    rig.type_text("w1:p1", "y");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "y");
}

/// The shared focus moved onto a pane another device holds (that device's
/// person picked it): following it took the pane, every time. A follow's
/// attach is plain now, and herdr refuses it.
#[test]
fn following_the_focus_onto_a_pane_another_device_holds_takes_nothing_and_says_so() {
    let mut rig = attached();
    rig.host.held_elsewhere = vec!["w1:p2".to_owned()];
    asked_focus(&mut rig, "w1:p2");
    rig.run(3_000);
    assert_eq!(rig.attach_lines(), ["w1:p1", "w1:p2"]);
    assert_eq!(rig.frames.took_over, [false, false], "a follow took over");
    assert!(rig.host.taken_from_others.is_empty());
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    assert_eq!(rig.client.attached_pane(), None);
    rig.run(10_000);
    assert_eq!(rig.attach_lines().len(), 2, "a read asked again");
    rig.act(Action::NoticeAction);
    rig.run(500);
    assert_eq!(rig.frames.took_over, [false, false, true]);
    assert_eq!(rig.host.taken_from_others, ["w1:p2"]);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
}

/// Only the person's word asks for `--takeover`: the first attach (the
/// server's focus) and every follow attach plainly; a tap and a reply for
/// another pane take theirs.
#[test]
fn only_a_pick_or_a_reply_takes_over() {
    let mut rig = attached();
    rig.host.held_elsewhere = vec!["w1:p2".to_owned()];
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(2_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert_eq!(rig.host.taken_from_others, ["w1:p2"]);
    asked_focus(&mut rig, "w1:p1");
    rig.run(3_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    let request = rig
        .act(Action::DeliverInput {
            pane: "w1:p2".to_owned(),
            bytes: b"y\r".to_vec(),
        })
        .unwrap();
    rig.run(2_000);
    assert!(rig.events().contains(&Event::Delivered {
        request,
        delivered: true
    }));
    assert_eq!(rig.frames.took_over, [false, true, false, true]);
}

/// The person taps the pane the focus is on its way to while another device
/// holds it: the follow's plain attach is refused, then the tap's takes it.
#[test]
fn a_tap_on_the_pane_a_follow_is_on_its_way_to_takes_it() {
    let mut rig = attached();
    rig.host.held_elsewhere = vec!["w1:p2".to_owned()];
    asked_focus(&mut rig, "w1:p2");
    // The read that follows the focus there, until its plain line is out
    // (herdr's refusal is a round trip behind it).
    rig.client.foreground(rig.now);
    rig.pump();
    for _ in 0..500 {
        if rig
            .frame_writes()
            .iter()
            .any(|line| line.starts_with("w1:p2 "))
        {
            break;
        }
        rig.run(5);
    }
    assert!(
        rig.frame_writes().iter().any(|line| line == "w1:p2 80 24"),
        "{:?}",
        rig.frame_writes()
    );
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(2_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert_eq!(rig.host.taken_from_others, ["w1:p2"]);
    // The follow's plain line went first, and was refused; the tap's
    // `--takeover` came behind it.
    assert_eq!(rig.frames.took_over, [false, false, true]);
    assert_eq!(rig.client.notice(), None);
}

/// The pane another device has closes: the focused one, by itself.
#[test]
fn a_held_pane_that_closes_is_left_for_the_focused_one() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(3_000);
    assert_eq!(rig.attach_lines(), ["w1:p1"]);
    rig.host.panes.retain(|pane| pane.id != "w1:p1");
    rig.host.held_elsewhere.clear();
    rig.host.focused = Some("w1:p2".to_owned());
    rig.run(3_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert_eq!(rig.frames.took_over, [false, false]);
    assert_eq!(rig.client.notice(), None);
}

#[test]
fn a_pane_closed_under_the_attach_is_not_another_client() {
    let mut rig = attached();
    for _ in 0..3 {
        rig.host.panes.retain(|pane| pane.id != "w1:p1");
        rig.host.focused = Some("w1:p2".to_owned());
        rig.host.panes.push(moshpit_herdr::client::rig::FakePane {
            id: "w1:p1".to_owned(),
            tab: "w1:t1".to_owned(),
            workspace: "w1".to_owned(),
            cwd: "/".to_owned(),
            agent: None,
        });
        rig.host.focused = Some("w1:p1".to_owned());
        // herdr ends an attach whose pane went away with a "not found".
        rig.client.frame_bytes(
            b"{\"reason\":\"terminal attach ended: terminal term_1 not found\",\"type\":\"terminal.closed\"}\n",
            rig.now,
        );
        rig.pump();
        rig.run(3_000);
    }
    assert_ne!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
}

#[test]
fn a_refused_handshake_is_said_and_backed_off_from() {
    let mut rig = attached();
    rig.client.frame_bytes(
        b"herdr: server rejected handshake (version 16): client protocol 19 is newer\r\n",
        rig.now,
    );
    rig.pump();
    assert_eq!(rig.client.frame_target(), None);
    let notice = rig
        .client
        .notice()
        .map(|(notice, action)| (notice.clone(), action));
    assert!(matches!(
        notice,
        Some((Notice::Mismatch(_), Some(NoticeAction::RestartServer)))
    ));
    let attaches = rig.attach_lines().len();
    rig.run(20_000);
    assert_eq!(rig.attach_lines().len(), attaches);
}

#[test]
fn a_shell_that_cant_run_the_loop_falls_back_to_the_one_shot_line() {
    let mut rig = Rig::new(config());
    rig.frames.loop_works = false;
    rig.start();
    rig.run(2_000);
    assert_eq!(rig.attach_lines(), Vec::<String>::new());
    rig.run(1_000);
    assert_eq!(rig.attach_lines(), ["w1:p1"]);
    let start = rig.frame_writes().last().cloned().unwrap_or_default();
    // The server's focus: a follow, attached plainly.
    assert!(start.ends_with("terminal session control 'w1:p1' --cols 80 --rows 24"));
    // Switching now: release, the prompt's settle, the one-shot line — the
    // person's pick, `--takeover`.
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(2_000);
    assert_eq!(rig.attach_lines(), ["w1:p1", "w1:p2"]);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert!(rig.frame_writes().last().is_some_and(|line| {
        line.ends_with("terminal session control 'w1:p2' --cols 80 --rows 24 --takeover")
    }));
}

/// B485: a Return right behind a paste (the image panel's Send) reaches the
/// pane 150 ms behind it, not with it: Claude Code drops a Return that
/// comes with the paste of an image's path.
#[test]
fn a_return_right_behind_a_paste_waits_for_it() {
    let mut rig = attached();
    rig.client.paste("w1:p1", " /tmp/a.jpg ", rig.now);
    rig.pump();
    rig.type_bytes("w1:p1", b"\r");
    rig.run(100);
    assert_eq!(
        rig.typed("w1:p1"),
        "\u{1b}[200~ /tmp/a.jpg \u{1b}[201~",
        "the Return waits"
    );
    rig.run(100);
    assert_eq!(rig.typed("w1:p1"), "\u{1b}[200~ /tmp/a.jpg \u{1b}[201~\r");
}

/// B485: the same given while the pane is being switched to (Send pressed
/// during a pick): both are held, and once the switch lands the paste goes
/// and the Return 150 ms behind it. They were let go together.
#[test]
fn a_return_held_behind_a_paste_goes_apart_from_it() {
    let mut rig = attached();
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.client.paste("w1:p2", " /tmp/a.jpg ", rig.now);
    rig.pump();
    rig.type_bytes("w1:p2", b"\r");
    let mut waited = 0;
    while !rig.typed("w1:p2").contains("\u{1b}[201~") {
        assert!(
            waited < 5_000,
            "the switch never landed: {:?}",
            rig.frame_writes()
        );
        rig.run(10);
        waited += 10;
    }
    assert!(
        !rig.typed("w1:p2").ends_with('\r'),
        "the Return waits behind the paste"
    );
    rig.run(100);
    assert!(!rig.typed("w1:p2").ends_with('\r'), "still");
    rig.run(100);
    assert_eq!(rig.typed("w1:p2"), "\u{1b}[200~ /tmp/a.jpg \u{1b}[201~\r");
    assert_eq!(rig.typed("w1:p1"), "");
}

/// B527: the pane's attach ends and comes back (herdr re-attaching the same
/// pane) within a paste's wait: what waited behind the paste waits for the
/// pane's new attach and goes then, still behind the paste. It was dropped
/// and said dropped: the image panel's Send, its Return lost.
#[test]
fn a_return_behind_a_paste_survives_its_panes_reattach() {
    let mut rig = attached();
    // The next attach takes a while to paint.
    rig.frames.attach_ms = 600;
    rig.client.paste("w1:p1", " /tmp/a.jpg ", rig.now);
    rig.pump();
    rig.type_bytes("w1:p1", b"\r");
    rig.run(50);
    assert_eq!(
        rig.typed("w1:p1"),
        "\u{1b}[200~ /tmp/a.jpg \u{1b}[201~",
        "the paste is in, the Return waits"
    );
    rig.host_attach_ends("herdr: server shut down: server is shutting down");
    rig.run(3_000);
    assert!(
        rig.typed("w1:p1").ends_with("\u{1b}[201~\r"),
        "{:?}",
        rig.typed("w1:p1")
    );
    assert!(
        !rig.events()
            .iter()
            .any(|e| matches!(e, Event::InputDropped { .. })),
        "nothing dropped"
    );
}

/// B527, the other way: the attach that comes back is another pane's (the
/// desktop's focus moved meanwhile): what waited behind the paste goes to
/// no other pane, and its loss is said.
#[test]
fn a_return_behind_a_paste_goes_to_no_other_pane() {
    let mut rig = attached();
    rig.client.paste("w1:p1", " /tmp/a.jpg ", rig.now);
    rig.pump();
    rig.type_bytes("w1:p1", b"\r");
    rig.run(50);
    rig.host.focused = Some("w1:p2".to_owned());
    rig.host_attach_ends("herdr: server shut down: server is shutting down");
    rig.run(3_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert_eq!(rig.typed("w1:p1"), "\u{1b}[200~ /tmp/a.jpg \u{1b}[201~");
    assert_eq!(rig.typed("w1:p2"), "", "nothing into another pane");
    assert!(
        rig.events()
            .iter()
            .any(|e| matches!(e, Event::InputDropped { .. })),
        "said"
    );
}

/// B680: a pick's release is still on its way when the server shuts down
/// (a restart, a pane closing under it): the attach being let go says its
/// last words, and the release reaches the reader loop, which ignores it —
/// no close comes. The next attach waited for that close anyway, and when
/// its wait ran out (5 s) nothing named the pane: blank under the veil until
/// a read's re-ask, seconds more.
#[test]
fn a_release_overtaken_by_the_servers_shutdown_does_not_stall_the_next_attach() {
    let mut rig = attached();
    rig.frames.latency = 200;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(50);
    rig.host_restarted_elsewhere();
    let gone_at = rig.now;
    let mut back = None;
    while rig.now < gone_at + 2_500 {
        rig.run(50);
        if rig.client.attached_pane() == Some("w1:p2") {
            back = Some(rig.now - gone_at);
            break;
        }
    }
    assert!(
        back.is_some(),
        "never back on w1:p2 within 2.5 s: {:?}",
        rig.frame_writes()
    );
}

/// B683: the desktop moves herdr's focus while the phone types: the phone
/// stays on its pane, as the active one, and nothing typed is dropped. It
/// switched under the person and dropped what they were typing; herdr's
/// frame channel stays on an unfocused pane, so nothing is lost by staying.
#[test]
fn the_desktop_moving_the_focus_leaves_the_phone_on_its_pane() {
    let mut rig = attached();
    rig.host.focused = Some("w1:p2".to_owned());
    let mut typed = String::new();
    for i in 0..10 {
        let text = format!("k{i}");
        rig.type_text("w1:p1", &text);
        typed.push_str(&text);
        rig.run(600);
    }
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert_eq!(rig.typed("w1:p1"), typed);
    assert_eq!(rig.typed("w1:p2"), "");
    assert!(
        !rig.events()
            .iter()
            .any(|e| matches!(e, Event::InputDropped { .. })),
        "nothing dropped"
    );
    assert_eq!(rig.client.snapshot().active_pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(rig.client.server_focus(), Some("w1:p2"));
    assert!(
        rig.client
            .panes()
            .iter()
            .any(|(pane, _, active)| pane == "w1:p1" && *active),
        "the phone's pane is the active one"
    );
}

/// B683: a workspace the phone makes is followed onto the pane it focuses
/// (the person's own focus change).
#[test]
fn a_workspace_the_phone_makes_is_followed() {
    let mut rig = attached();
    rig.act(Action::NewSession { name: Some("new".to_owned()) });
    rig.run(3_000);
    let focused = rig.host.focused.clone().unwrap();
    assert_ne!(focused, "w1:p1");
    assert_eq!(rig.client.attached_pane(), Some(focused.as_str()));
}

#[test]
fn an_unanswered_release_still_switches_but_typing_waits_for_the_paint() {
    let mut rig = attached();
    // The host takes the release far too long to answer.
    rig.frames.latency = 900;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.type_text("w1:p2", "hi");
    rig.run(1_600);
    // The line went after the ack's wait; typing is still held.
    assert!(
        rig.frame_writes()
            .iter()
            .any(|line| line.starts_with("w1:p2 "))
    );
    assert!(
        !rig.frame_writes()
            .iter()
            .any(|line| line.contains("terminal.input"))
    );
    rig.run(5_000);
    assert_eq!(rig.typed("w1:p2"), "hi");
    assert_eq!(rig.typed("w1:p1"), "");
}

// MARK: - Delivering to one pane (notification replies)

#[test]
fn a_reply_for_another_pane_switches_there_first_then_goes() {
    let mut rig = attached();
    let request = rig
        .act(Action::DeliverInput {
            pane: "w1:p2".to_owned(),
            bytes: b"y\r".to_vec(),
        })
        .unwrap();
    rig.run(2_000);
    assert_eq!(rig.typed("w1:p2"), "y\r");
    assert_eq!(rig.typed("w1:p1"), "");
    assert!(rig.events().contains(&Event::Delivered {
        request,
        delivered: true
    }));
}

#[test]
fn a_reply_that_cant_reach_its_pane_in_time_says_so() {
    let mut rig = attached();
    // The pane's attach never comes up.
    rig.frames.attach_ms = 60_000;
    let request = rig
        .act(Action::DeliverInput {
            pane: "w1:p2".to_owned(),
            bytes: b"y\r".to_vec(),
        })
        .unwrap();
    rig.run(6_000);
    assert!(rig.events().contains(&Event::Delivered {
        request,
        delivered: false
    }));
    assert_eq!(rig.typed("w1:p2"), "");
}

/// A reply for the pane another device has: nothing reaches it, and it is
/// no reason to take the pane from that device ("Open here" is).
#[test]
fn a_reply_for_the_pane_another_device_has_is_refused_without_taking_it() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(200);
    let request = rig
        .act(Action::DeliverInput {
            pane: "w1:p1".to_owned(),
            bytes: b"y\r".to_vec(),
        })
        .unwrap();
    rig.run(200);
    assert!(rig.events().contains(&Event::Delivered {
        request,
        delivered: false
    }));
    rig.run(5_000);
    assert_eq!(rig.attach_lines(), ["w1:p1"], "taken back for a reply");
    assert_eq!(rig.typed("w1:p1"), "");
}

// MARK: - Helpers

fn task_request(prompt: &str) -> moshpit_herdr::control::AgentTaskRequest {
    moshpit_herdr::control::AgentTaskRequest {
        repo_path: "/home/dev/code/app".to_owned(),
        branch: "fix-scroll".to_owned(),
        agent: "claude".to_owned(),
        prompt: prompt.to_owned(),
    }
}

// MARK: - New agent task

fn task_outcome(rig: &Rig) -> Option<moshpit_herdr::control::TaskOutcome> {
    rig.events().into_iter().find_map(|event| match event {
        Event::Task { outcome, .. } => Some(outcome),
        _ => None,
    })
}

use moshpit_herdr::control::{
    RemovalFailure, TaskFailure, TaskIncomplete, TaskOutcome, WorktreeRemoval,
};

#[test]
fn a_task_makes_the_worktree_starts_the_agent_in_its_pane_and_sends_the_first_message() {
    let mut rig = attached();
    rig.act(Action::StartAgentTask(task_request("  fix the bug \n")));
    rig.run(10_000);
    assert_eq!(task_outcome(&rig), Some(TaskOutcome::Started));
    assert_eq!(
        rig.commands("worktree create"),
        [
            "worktree create --cwd '/home/dev/code/app' --branch 'fix-scroll' --label 'fix-scroll' --focus --json"
        ]
    );
    assert_eq!(rig.typed("w2:p3"), "claude\rfix the bug\r");
    // herdr 0.9.5's snapshot named the version: no `herdr status` needed.
    assert!(rig.commands("status").is_empty());
}

#[test]
fn the_first_message_waits_for_herdr_to_see_the_agent_then_goes() {
    let mut rig = attached();
    rig.host.detect_after = 6_000;
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(15_000);
    assert_eq!(task_outcome(&rig), Some(TaskOutcome::Started));
    let tries = rig.commands("agent prompt").len();
    assert!((4..=6).contains(&tries), "{tries}");
    assert_eq!(rig.typed("w2:p3"), "claude\rgo\r");
}

#[test]
fn an_agent_that_never_comes_up_is_reported_not_waited_on_for_good() {
    let mut rig = attached();
    rig.host.detect_after = 1_000_000;
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(40_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Incomplete(TaskIncomplete::AgentNotReady {
            agent: "claude".to_owned()
        }))
    );
}

#[test]
fn on_herdr_before_0_7_5_the_message_is_typed_then_return_is_pressed() {
    let mut rig = attached();
    rig.host.snapshot_names_version = false;
    rig.host.version = "0.7.3".to_owned();
    rig.host.client_version = "0.7.3".to_owned();
    rig.act(Action::Refresh);
    rig.run(200);
    assert_eq!(rig.client.server_version(), None);
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(10_000);
    assert_eq!(task_outcome(&rig), Some(TaskOutcome::Started));
    assert_eq!(rig.commands("status").len(), 1);
    assert_eq!(rig.commands("agent send").len(), 1);
    assert_eq!(rig.typed("w2:p3"), "claude\rgo\r");
    assert_eq!(rig.commands("pane send-keys 'w2:p3' 'Enter'").len(), 1);
}

#[test]
fn an_unreadable_version_means_todays_herdr() {
    let mut rig = Rig::new(config());
    rig.host.snapshot_names_version = false;
    rig.host.rules.push(rule("status", None, Fate::Hang));
    rig.start();
    rig.run(500);
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(15_000);
    assert_eq!(task_outcome(&rig), Some(TaskOutcome::Started));
    assert_eq!(rig.commands("agent prompt").len(), 1);
}

#[test]
fn a_worktree_create_with_no_answer_to_read_is_not_a_started_task() {
    // B72: exit 0 and no JSON.
    let mut rig = attached();
    rig.host.rules.push(rule(
        "worktree create",
        None,
        Fate::Output(format!("\n{}0\n", moshpit_herdr::control::EXIT_CODE_MARKER)),
    ));
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(5_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Failed(TaskFailure::Unconfirmed {
            branch: "fix-scroll".to_owned()
        }))
    );
    assert!(rig.commands("pane run").is_empty());
}

#[test]
fn a_worktree_create_the_host_never_answers_fails_in_bounded_time() {
    // B89.
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("worktree create", None, Fate::Hang));
    let start = rig.now;
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(59_000);
    assert_eq!(task_outcome(&rig), None);
    rig.run(2_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Failed(TaskFailure::HostStoppedAnswering {
            branch: "fix-scroll".to_owned()
        }))
    );
    let at = rig
        .log
        .iter()
        .find(|(_, output)| matches!(output, Output::Event(Event::Task { .. })))
        .map(|(at, _)| *at)
        .unwrap();
    assert_eq!(at - start, 60_000);
    assert!(rig.commands("pane run").is_empty());
}

#[test]
fn a_create_still_queued_when_its_owner_gives_up_is_never_sent() {
    // A step ahead that holds the lane past the task's patience.
    let mut config = config();
    config.budgets.command_hold = 120_000;
    let mut rig = Rig::new(config);
    rig.host.rules.push(rule("tab create", None, Fate::Hang));
    rig.start();
    rig.run(500);
    rig.act(Action::NewWindow { name: None });
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(70_000);
    assert!(matches!(
        task_outcome(&rig),
        Some(TaskOutcome::Failed(
            TaskFailure::HostStoppedAnswering { .. }
        ))
    ));
    assert!(rig.commands("worktree create").is_empty());
}

#[test]
fn a_pane_run_cut_off_before_its_exit_status_is_not_an_agent_started() {
    // B90.
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("pane run", None, Fate::CutTrailer));
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(5_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Incomplete(TaskIncomplete::AgentDidNotStart {
            agent: "claude".to_owned(),
            reason: Refusal::CutOff
        }))
    );
    assert!(rig.commands("agent prompt").is_empty());
    assert_eq!(
        TaskOutcome::Incomplete(TaskIncomplete::AgentDidNotStart {
            agent: "claude".to_owned(),
            reason: Refusal::CutOff
        })
        .english()
        .unwrap(),
        "The worktree is ready, but claude didn't start: herdr's answer was cut off"
    );
}

#[test]
fn a_pane_run_the_host_never_answers_is_said_and_no_message_follows() {
    let mut rig = attached();
    rig.host.rules.push(rule("pane run", None, Fate::Hang));
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(30_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Incomplete(TaskIncomplete::LaunchUnanswered {
            agent: "claude".to_owned()
        }))
    );
    assert!(rig.commands("agent prompt").is_empty());
}

#[test]
fn a_first_message_cut_off_before_its_exit_status_is_not_sent() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("agent prompt", None, Fate::Output(String::new())));
    rig.act(Action::StartAgentTask(task_request("go")));
    rig.run(10_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Incomplete(TaskIncomplete::MessageRefused {
            agent: "claude".to_owned(),
            reason: Refusal::CutOff
        }))
    );
}

#[test]
fn a_task_cancelled_while_its_worktree_is_made_starts_no_agent() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("worktree create", Some(500), Fate::Normal));
    let request = rig.act(Action::StartAgentTask(task_request("go"))).unwrap();
    rig.run(100);
    rig.act(Action::CancelAgentTask(request));
    rig.run(5_000);
    assert_eq!(
        task_outcome(&rig),
        Some(TaskOutcome::Incomplete(TaskIncomplete::Cancelled))
    );
    assert!(rig.commands("pane run").is_empty());
}

#[test]
fn invalid_input_never_reaches_the_host() {
    let mut rig = attached();
    let mut request = task_request("go");
    request.branch = "bad branch".to_owned();
    let before = rig.commands("").len();
    rig.act(Action::StartAgentTask(request));
    assert!(matches!(
        task_outcome(&rig),
        Some(TaskOutcome::Failed(TaskFailure::Invalid(_)))
    ));
    assert_eq!(rig.commands("").len(), before);
}

// MARK: - Worktree removal (B119)

fn removal(rig: &Rig) -> Option<WorktreeRemoval> {
    rig.events().into_iter().find_map(|event| match event {
        Event::Removal { outcome, .. } => Some(outcome),
        _ => None,
    })
}

#[test]
fn a_dirty_checkout_needs_force_and_forcing_removes_it() {
    let mut rig = attached();
    rig.host
        .workspaces
        .push(("w7".to_owned(), "task".to_owned()));
    rig.host.worktrees.insert("w7".to_owned(), true);
    rig.act(Action::RemoveWorktree {
        session: "w7".to_owned(),
        force: false,
    });
    rig.run(2_000);
    assert_eq!(removal(&rig), Some(WorktreeRemoval::NeedsForce));
    rig.log.clear();
    rig.act(Action::RemoveWorktree {
        session: "w7".to_owned(),
        force: true,
    });
    rig.run(2_000);
    assert_eq!(removal(&rig), Some(WorktreeRemoval::Removed));
    assert!(rig.host.landed.contains(&"worktree remove w7".to_owned()));
}

#[test]
fn a_removal_the_host_never_answers_fails_in_bounded_time() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("worktree remove", None, Fate::Hang));
    rig.act(Action::RemoveWorktree {
        session: "w7".to_owned(),
        force: false,
    });
    rig.run(59_000);
    assert_eq!(removal(&rig), None);
    rig.run(2_000);
    assert_eq!(
        removal(&rig),
        Some(WorktreeRemoval::Failed(RemovalFailure::NoAnswer))
    );
}

#[test]
fn a_removal_cut_off_before_its_exit_status_is_not_removed() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("worktree remove", None, Fate::Output(String::new())));
    rig.act(Action::RemoveWorktree {
        session: "w7".to_owned(),
        force: false,
    });
    rig.run(2_000);
    assert_eq!(
        removal(&rig),
        Some(WorktreeRemoval::Failed(RemovalFailure::CutOff))
    );
}

// MARK: - Discovery (B120)

fn repos(rig: &Rig) -> Option<Vec<String>> {
    rig.events().into_iter().find_map(|event| match event {
        Event::Repos { repos, .. } => Some(repos),
        _ => None,
    })
}

#[test]
fn repo_discovery_puts_the_panes_repos_first_then_the_scan() {
    let mut rig = attached();
    rig.act(Action::DiscoverRepos);
    rig.run(1_000);
    assert_eq!(
        repos(&rig),
        Some(vec![
            "/home/dev/code/app".to_owned(),
            "/home/dev/code/other".to_owned()
        ])
    );
}

#[test]
fn a_resolve_that_hangs_falls_back_to_the_raw_directories_in_bounded_time() {
    let mut rig = attached();
    rig.host.rules.push(rule("for d in", None, Fate::Hang));
    rig.host.rules.push(rule("find -L", None, Fate::Hang));
    rig.act(Action::DiscoverRepos);
    rig.run(9_000);
    assert_eq!(repos(&rig), None);
    rig.run(1_500);
    assert_eq!(
        repos(&rig),
        Some(vec![
            "/home/dev/code/app".to_owned(),
            "/home/dev/code/app/sub".to_owned()
        ])
    );
}

#[test]
fn the_agent_list_comes_from_herdrs_manifests_and_never_hangs() {
    let mut rig = attached();
    rig.act(Action::AgentNames);
    rig.run(500);
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::Agents { names, answered: true, .. } if names == &["claude", "codex", "pi"]
    )));
    rig.log.clear();
    rig.host
        .rules
        .push(rule("agent-manifests", None, Fate::Hang));
    rig.act(Action::AgentNames);
    rig.run(10_500);
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::Agents { names, answered: false, .. } if names.is_empty()
    )));
}

// MARK: - Reading the tree

#[test]
fn an_unchanging_tree_eases_the_cadence_and_any_change_snaps_it_back() {
    let mut rig = attached();
    rig.run(20_000);
    assert_eq!(rig.client.poll_interval_ms(), 8_000);
    rig.host.focused = Some("w1:p2".to_owned());
    // The server's focus (B683: the phone keeps its own pane).
    for _ in 0..1_000 {
        if rig.client.server_focus() == Some("w1:p2") {
            break;
        }
        rig.run(10);
    }
    assert_eq!(rig.client.server_focus(), Some("w1:p2"));
    assert_eq!(rig.client.poll_interval_ms(), 2_000);
}

#[test]
fn a_read_that_never_answers_doesnt_stop_the_reads() {
    // The app's poll waited forever, and its loop with it.
    let mut rig = attached();
    rig.host.rules.push(rule("api snapshot", None, Fate::Hang));
    rig.run(30_000);
    let reads = rig.commands("api snapshot").len();
    assert!(reads >= 3, "{reads}");
    rig.host.rules.clear();
    rig.host.focused = Some("w1:p2".to_owned());
    rig.run(12_000);
    assert_eq!(rig.client.server_focus(), Some("w1:p2"));
}

#[test]
fn no_server_empties_the_tree_and_garbage_keeps_it() {
    let mut rig = attached();
    rig.host.rules.push(rule(
        "api snapshot",
        None,
        Fate::Output("Last login: today\n__moshpit_rc=0\n".to_owned()),
    ));
    rig.act(Action::Refresh);
    rig.run(200);
    assert_eq!(rig.client.snapshot().sessions.len(), 1);
    rig.host.rules.clear();
    rig.host.running = false;
    rig.act(Action::Refresh);
    rig.run(200);
    assert!(rig.client.server_not_running());
    assert!(rig.client.snapshot().sessions.is_empty());
}

// MARK: - Version skew and restart

const MISMATCH: &str = "client protocol 22 is newer than server protocol 19; run herdr server stop";

#[test]
fn a_mismatch_is_named_in_versions_and_offers_a_restart_only_when_it_upgrades() {
    let mut rig = attached();
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    let (notice, action) = rig.client.notice().map(|(n, a)| (n.clone(), a)).unwrap();
    assert!(matches!(notice, Notice::Skew(_)), "{notice:?}");
    assert_eq!(
        notice.english(),
        "The host's herdr server is 0.8.0, older than its herdr 0.9.5"
    );
    assert_eq!(action, Some(NoticeAction::RestartServer));
    // Once per episode.
    rig.run(20_000);
    assert_eq!(rig.commands("status").len(), 1);
}

#[test]
fn restart_refuses_and_never_stops_the_server_when_the_herdr_on_path_is_older() {
    let mut rig = attached();
    rig.host.client_version = "0.8.0".to_owned();
    let request = rig.act(Action::RestartServer).unwrap();
    rig.run(5_000);
    assert!(rig.commands("server stop").is_empty());
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::Restart { request: r, outcome: moshpit_herdr::client::RestartOutcome::WouldDowngrade(_) } if *r == request
    )));
    let (notice, action) = rig.client.notice().map(|(n, a)| (n.clone(), a)).unwrap();
    assert!(notice.english().contains("update that copy"));
    assert_eq!(action, None);
}

/// B408: the banner's Restart, on a server that runs the host's `herdr`
/// already (fixed elsewhere since the banner went up), stops nothing:
/// `server stop` would exit every pane's process. The versions are read
/// right before the stop, not taken from the read that raised the banner.
#[test]
fn a_restart_stops_nothing_when_the_versions_now_agree() {
    let mut rig = attached();
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    assert_eq!(
        rig.client.notice().map(|(_, action)| action),
        Some(Some(NoticeAction::RestartServer))
    );
    // Upgraded on the host, outside the app.
    rig.host.mismatch = None;
    rig.host.version = "0.9.5".to_owned();
    let request = rig.act(Action::NoticeAction).unwrap();
    rig.run(10_000);
    assert!(
        rig.commands("server stop").is_empty(),
        "{:?}",
        rig.commands("")
    );
    assert!(!rig.host.landed.contains(&"server stop".to_owned()));
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::Restart { request: r, outcome: moshpit_herdr::client::RestartOutcome::Unneeded } if *r == request
    )));
    assert_eq!(rig.client.notice(), None);
    assert!(!rig.client.is_restarting());
}

/// B408: over mosh (no frame channel, so no painted frame to say the
/// server answers again) a skew banner goes once a read comes back a tree:
/// fixed outside the app, it no longer offers a Restart that would stop a
/// healthy server. A failed restart's Retry goes the same way.
#[test]
fn over_mosh_a_read_that_comes_back_a_tree_clears_the_skew_and_the_failed_restart() {
    let mut config = config();
    config.frame_channel = false;
    let mut rig = Rig::new(config);
    rig.start();
    rig.run(500);
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    assert!(
        matches!(
            rig.client.notice(),
            Some((Notice::Skew(_), Some(NoticeAction::RestartServer)))
        ),
        "{:?}",
        rig.client.notice()
    );
    rig.host.mismatch = None;
    rig.host.version = "0.9.5".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    assert_eq!(rig.client.notice(), None);
    // A restart that didn't take (the link stopped answering): its Retry,
    // then the link back and a healthy read.
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    rig.host.rules.push(rule("herdr", None, Fate::Hang));
    rig.act(Action::NoticeAction);
    rig.run(200_000);
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::RestartFailed, Some(NoticeAction::Retry)))
    );
    rig.host.rules.clear();
    rig.host.mismatch = None;
    rig.host.version = "0.9.5".to_owned();
    rig.act(Action::Refresh);
    rig.run(10_000);
    assert_eq!(rig.client.notice(), None);
}

#[test]
fn a_restart_stops_starts_and_reads_until_the_server_answers() {
    let mut rig = attached();
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    // The banner's button.
    let request = rig.act(Action::NoticeAction).unwrap();
    assert_eq!(
        rig.client.notice().map(|(n, _)| n.clone()),
        Some(Notice::Restarting)
    );
    // The versions read before the stop (B408): still the old server.
    rig.run(100);
    // The new server is today's.
    rig.host.mismatch = None;
    rig.host.version = "0.9.5".to_owned();
    rig.run(10_000);
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::Restart { request: r, outcome: moshpit_herdr::client::RestartOutcome::Restarted } if *r == request
    )));
    assert_eq!(
        rig.host
            .landed
            .iter()
            .filter(|l| *l == "server stop")
            .count(),
        1
    );
    assert!(rig.host.landed.contains(&"server start".to_owned()));
    assert_eq!(rig.client.notice(), None);
    assert!(!rig.client.is_restarting());
}

/// B418: the screen parked on a pane another device took, then a version
/// skew put its banner up over that one; the person restarted the server.
/// Stopping it ended every client's attach, the other device's too: the
/// phone follows the focus again and attaches by itself. It went on taking
/// the pane for the other device's, with no banner and no button: typing
/// said not sent, and only a pick from the tree brought it back.
#[test]
fn a_server_restart_ends_the_hold_another_device_had() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    assert_eq!(
        rig.client.notice().map(|(_, action)| action),
        Some(Some(NoticeAction::RestartServer))
    );
    rig.act(Action::NoticeAction);
    // The versions read before the stop: still the old server.
    rig.run(100);
    rig.host.mismatch = None;
    rig.host.version = "0.9.5".to_owned();
    rig.run(10_000);
    assert!(rig.host.landed.contains(&"server stop".to_owned()));
    assert_eq!(rig.client.notice(), None);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    rig.type_text("w1:p1", "y");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "y");
}

/// B418: the same, with the versions put right elsewhere before the tap
/// (B408: nothing is restarted). The other device still has the pane: the
/// screen says so again, with "Open here", where it went quiet.
#[test]
fn an_unneeded_restart_says_again_that_another_device_has_the_pane() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    rig.host.mismatch = Some(MISMATCH.to_owned());
    rig.host.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(100);
    rig.host.mismatch = None;
    rig.host.version = "0.9.5".to_owned();
    rig.act(Action::NoticeAction);
    rig.run(10_000);
    assert!(rig.commands("server stop").is_empty());
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    rig.act(Action::NoticeAction);
    rig.run(500);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert_eq!(rig.host.taken_from_others, ["w1:p1"]);
}

#[test]
fn a_restart_on_a_dead_link_ends_in_bounded_time() {
    let mut rig = attached();
    rig.host.rules.push(rule("herdr", None, Fate::Hang));
    let request = rig.act(Action::RestartServer).unwrap();
    rig.run(200_000);
    assert!(rig.events().iter().any(|event| matches!(
        event,
        Event::Restart { request: r, outcome: moshpit_herdr::client::RestartOutcome::Failed } if *r == request
    )));
    assert_eq!(
        rig.client.notice().map(|(n, a)| (n.clone(), a)),
        Some((Notice::RestartFailed, Some(NoticeAction::Retry)))
    );
}

#[test]
fn creating_on_a_host_with_no_server_starts_one_with_a_first_workspace_named_as_asked() {
    let mut rig = Rig::new(config());
    rig.host.running = false;
    rig.host.workspaces.clear();
    rig.host.tabs.clear();
    rig.host.panes.clear();
    rig.host.focused = None;
    rig.start();
    rig.run(500);
    assert!(rig.client.server_not_running());
    rig.act(Action::NewSession {
        name: Some("work".to_owned()),
    });
    rig.run(5_000);
    assert!(rig.host.landed.contains(&"server start".to_owned()));
    // The app dropped the name on this path.
    assert!(
        rig.host
            .landed
            .contains(&"workspace create work".to_owned())
    );
    assert!(!rig.client.server_not_running());
    assert_eq!(rig.client.snapshot().sessions.len(), 1);
    assert_eq!(rig.client.attached_pane(), Some("w2:p3"));
}

// MARK: - Push (B118)

#[test]
fn push_subscribes_once_the_pump_is_up_then_reads_on_events() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.start();
    assert!(rig.push_open);
    // The pump is the channel's program, never a line typed into a login
    // shell still starting (B267).
    assert_eq!(
        rig.push_program.as_deref(),
        Some(moshpit_herdr::socket::pump_boot_line().as_str())
    );
    rig.run(500);
    rig.open_push();
    let writes: Vec<Vec<u8>> = rig
        .log
        .iter()
        .filter_map(|(_, output)| match output {
            Output::PushWrite(bytes) => Some(bytes.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 1, "only the subscribe: {writes:?}");
    assert!(String::from_utf8_lossy(&writes[0]).contains("events.subscribe"));
    let reads = rig.commands("api snapshot").len();
    rig.push_subscribed();
    assert!(rig.client.push_active());
    // 0.9.5 replays nothing on subscribe: the tree is read now.
    assert_eq!(rig.commands("api snapshot").len(), reads + 1);
    rig.run(1_000);
    let reads = rig.commands("api snapshot").len();
    // A burst is one read, once it goes quiet.
    for kind in [
        "tab_created",
        "tab_focused",
        "pane_focused",
        "layout_updated",
    ] {
        rig.push_event(kind);
        rig.run(50);
    }
    rig.run(1_000);
    assert_eq!(rig.commands("api snapshot").len(), reads + 1);
}

#[test]
fn a_push_channel_that_dies_puts_the_poll_back_in_charge_and_comes_back() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.start();
    rig.run(500);
    rig.open_push();
    rig.push_subscribed();
    rig.run(200_000);
    assert_eq!(rig.client.poll_interval_ms(), 45_000);
    rig.client.push_closed(rig.now);
    rig.pump();
    assert!(!rig.client.push_active());
    assert_eq!(rig.client.poll_interval_ms(), 2_000);
    assert!(rig.events().contains(&Event::PushActive(false)));
    // Tried again after a while.
    rig.push_open = false;
    rig.run(11_000);
    assert!(rig.push_open);
}

/// B682: push was up and the server restarted quickly (the desktop
/// upgraded herdr): its socket closed under the pump, and the server
/// answered again before any read found it gone. Push came back only on
/// its 10 s retry — the tree and the agents' states that slow meanwhile.
/// It opens again at the first read that finds the server answering.
#[test]
fn push_opens_again_at_the_first_read_after_a_quick_restart() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.start();
    rig.run(500);
    rig.open_push();
    rig.push_subscribed();
    rig.run(60_000);
    assert!(rig.client.push_active());
    let now = rig.now;
    rig.client.push_closed(now);
    rig.pump();
    rig.push_open = false;
    rig.run(2_000);
    assert!(rig.push_open, "opened again at the first read");
    rig.open_push();
    rig.push_subscribed();
    assert!(rig.client.push_active());
    // Dying again at once: no faster than its retry this time (a channel
    // that keeps dying isn't reopened at every read).
    let now = rig.now;
    rig.client.push_closed(now);
    rig.pump();
    rig.push_open = false;
    rig.run(5_000);
    assert!(!rig.push_open, "not at every read");
    rig.run(6_000);
    assert!(rig.push_open, "its retry");
}

/// B422: push was up and the server went away for longer than the retry
/// waits (an upgrade with the server stopped a while): the one retry found
/// no server to subscribe to. Push stayed off for the rest of the session,
/// the agents' states as late as the poll; it opens again once a read
/// finds the server back.
#[test]
fn push_opens_again_once_the_server_answers_after_its_retry_found_none() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.start();
    rig.run(500);
    rig.open_push();
    rig.push_subscribed();
    assert!(rig.client.push_active());
    // The server stops: its socket closes under the pump.
    rig.host.running = false;
    let now = rig.now;
    rig.client.push_closed(now);
    rig.pump();
    rig.push_open = false;
    rig.run(11_000);
    assert!(rig.push_open, "tried again");
    // Still no server: the pump finds no socket, the channel ends.
    let now = rig.now;
    rig.client.push_closed(now);
    rig.pump();
    rig.push_open = false;
    rig.run(60_000);
    assert!(!rig.push_open, "nothing to subscribe to without a server");
    // The server is back: the next read (every 2 s) finds it.
    rig.host.running = true;
    rig.run(5_000);
    assert!(rig.push_open, "not opened again once the server answered");
    rig.open_push();
    rig.push_subscribed();
    assert!(rig.client.push_active());
    // A server that never comes back costs nothing: no more tries.
    let now = rig.now;
    rig.client.push_closed(now);
    rig.pump();
    rig.push_open = false;
    rig.host.running = false;
    rig.run(11_000);
    let now = rig.now;
    rig.client.push_closed(now);
    rig.pump();
    rig.push_open = false;
    rig.run(120_000);
    assert!(!rig.push_open);
}

/// B210: over mosh, push rides the control connection, which is redialled
/// while mosh carries on. The channel goes with its connection unannounced
/// (its reader is dropped with it): the poll is back in charge at once, and
/// push opens again on the next connection — not on a timer that may fire
/// while there is none.
#[test]
fn push_lost_with_its_connection_puts_the_poll_back_and_opens_again_on_the_next() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.start();
    rig.run(500);
    rig.open_push();
    rig.push_subscribed();
    rig.run(200_000);
    assert_eq!(rig.client.poll_interval_ms(), 45_000);
    rig.client.push_connection_lost(rig.now);
    rig.pump();
    assert!(!rig.client.push_active());
    assert_eq!(rig.client.poll_interval_ms(), 2_000);
    assert!(rig.events().contains(&Event::PushActive(false)));
    // Nothing to open it on until a connection is back.
    rig.push_open = false;
    rig.run(60_000);
    assert!(!rig.push_open);
    rig.client.connection_restored(rig.now);
    rig.pump();
    assert!(rig.push_open);
    rig.open_push();
    rig.push_subscribed();
    assert!(rig.client.push_active());
}

/// A connection lost while push was still coming up (or waiting for its
/// retry after a server restart) is tried again on the next one too; a
/// fresh connection with push already given up for cause asks nothing.
#[test]
fn push_on_its_way_when_the_connection_went_comes_back_with_the_next() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config.clone());
    rig.start();
    assert!(rig.push_open);
    rig.client.push_connection_lost(rig.now);
    rig.pump();
    rig.push_open = false;
    rig.run(30_000);
    assert!(!rig.push_open, "no pump-ready timeout reopens it meanwhile");
    rig.client.connection_restored(rig.now);
    rig.pump();
    assert!(rig.push_open);

    // A host without python3: push given up; a new connection doesn't
    // bring it back.
    let mut rig = Rig::new(config);
    rig.push_answers = false;
    rig.start();
    rig.run(500);
    rig.open_push();
    rig.run(16_000);
    rig.push_open = false;
    rig.client.connection_restored(rig.now);
    rig.pump();
    assert!(!rig.push_open);
}

#[test]
fn a_host_without_python_gives_push_up_once() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.push_answers = false;
    rig.start();
    rig.run(500);
    rig.open_push();
    rig.run(16_000);
    assert!(!rig.push_open);
    assert!(!rig.client.push_active());
    rig.run(60_000);
    assert!(!rig.push_open);
}

// MARK: - Closing

#[test]
fn closing_answers_everything_still_waited_on() {
    let mut rig = attached();
    rig.host.rules.push(rule("worktree", None, Fate::Hang));
    rig.host.rules.push(rule("find -L", None, Fate::Hang));
    let task = rig.act(Action::StartAgentTask(task_request("go"))).unwrap();
    let remove = rig
        .act(Action::RemoveWorktree {
            session: "w7".to_owned(),
            force: false,
        })
        .unwrap();
    let repos = rig.act(Action::DiscoverRepos).unwrap();
    rig.run(100);
    rig.client.close(true, rig.now);
    rig.pump();
    let events = rig.events();
    assert!(events.contains(&Event::Task {
        request: task,
        outcome: TaskOutcome::Failed(TaskFailure::HostStoppedAnswering {
            branch: "fix-scroll".to_owned()
        })
    }));
    assert!(events.contains(&Event::Removal {
        request: remove,
        outcome: WorktreeRemoval::Failed(RemovalFailure::Unreachable)
    }));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Repos { request, .. } if *request == repos))
    );
    assert!(
        rig.frame_writes()
            .last()
            .unwrap()
            .contains("terminal.release")
    );
    assert_eq!(rig.client.next_deadline(), None);
}

// MARK: - mosh's immersive zoom

#[test]
fn over_mosh_a_pick_zooms_the_pane_and_a_tab_switch_zooms_what_the_next_tree_names() {
    let mut config = config();
    config.frame_channel = false;
    config.immersive_zoom = true;
    let mut rig = Rig::new(config);
    rig.start();
    rig.run(500);
    assert!(rig.frame_writes().is_empty());
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(1_000);
    assert_eq!(
        rig.commands("pane zoom"),
        ["pane zoom --pane 'w1:p2' --on 2>/dev/null || true"]
    );
    rig.act(Action::SelectWindow("w1:t1".to_owned()));
    rig.run(1_000);
    assert_eq!(rig.commands("pane zoom").len(), 2);
}

#[test]
fn a_pointer_goes_to_a_server_that_takes_it() {
    use moshpit_herdr::frame::{MouseAction, MouseButton};
    let mut rig = attached();
    let click = Action::Mouse {
        pane: "w1:p1".to_owned(),
        action: MouseAction::Down,
        button: MouseButton::Left,
        column: 3,
        row: 4,
        modifiers: 0,
    };
    rig.act(click.clone());
    assert!(
        rig.frame_writes()
            .iter()
            .any(|line| line.contains("terminal.mouse"))
    );
    let mut old = Rig::new(config());
    old.host.version = "0.9.0".to_owned();
    old.start();
    old.run(500);
    old.act(click);
    assert!(
        !old.frame_writes()
            .iter()
            .any(|line| line.contains("terminal.mouse"))
    );
}

#[test]
fn nothing_reaches_an_attach_before_it_is_up() {
    // herdr 0.9.5 drops input that reaches an attach before its first frame
    // (probed: 9 of 10 lines written right behind the attach line lost).
    // The app wrote typing, and a notification's reply, right behind it.
    let mut rig = attached();
    rig.frames.attach_ms = 300;
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.type_text("w1:p2", "a");
    rig.run(150);
    // The attach line is out; the attach isn't up yet.
    assert!(
        rig.frame_writes()
            .iter()
            .any(|line| line.starts_with("w1:p2 "))
    );
    rig.type_text("w1:p2", "b");
    let reply = rig
        .act(Action::DeliverInput {
            pane: "w1:p2".to_owned(),
            bytes: b"c".to_vec(),
        })
        .unwrap();
    rig.run(2_000);
    assert_eq!(rig.frames.lost, Vec::<String>::new());
    assert_eq!(rig.typed("w1:p2"), "abc");
    assert!(rig.events().contains(&Event::Delivered {
        request: reply,
        delivered: true
    }));
}

#[test]
fn a_push_channel_that_never_opens_is_given_up_in_bounded_time() {
    let mut config = config();
    config.push = true;
    let mut rig = Rig::new(config);
    rig.start();
    assert!(rig.push_open);
    // The host never says whether the channel opened.
    rig.run(16_000);
    assert!(!rig.push_open, "closed after its wait");
    assert!(!rig.client.push_active());
}

// MARK: - The frame loop as the channel's program (the Canary report)

/// The channel's terminal runs the reader loop itself (`frame_loop`): the
/// session opens it with `frame::loop_program`. A boot line typed into a
/// login shell still starting (oh-my-zsh with zsh-autosuggestions and
/// zsh-syntax-highlighting, live) never ran, and every re-ask typed a
/// release and a one-shot line into the same shell: a blinking cursor,
/// forever.
fn looped(ready_after: Option<u64>) -> Rig {
    let mut rig = Rig::new(Config {
        frame_loop: true,
        ..config()
    });
    rig.frames.loop_works = ready_after.is_some();
    let ready_at = rig.now + ready_after.unwrap_or(0);
    rig.frames.run_loop_program(ready_at);
    rig.start();
    rig
}

#[test]
fn the_loop_as_the_program_gets_its_first_target_once_it_reads_and_nothing_else() {
    // The loop says it reads after the first read named the pane.
    let mut rig = looped(Some(400));
    rig.run(150);
    assert!(
        rig.frame_writes().is_empty(),
        "written before the loop reads"
    );
    rig.run(1_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    let writes = rig.frame_writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(
        writes[0].starts_with("w1:p1 "),
        "a target for the loop: {writes:?}"
    );
    assert!(rig.frames.shell_lines.is_empty());
    rig.type_text("w1:p1", "ls\r");
    rig.run(200);
    assert_eq!(rig.typed("w1:p1"), "ls\r");
    // A switch is a word to the loop.
    rig.act(Action::SelectPane("w1:p2".into()));
    rig.run(1_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert!(rig.frames.shell_lines.is_empty());
    assert!(
        rig.frame_writes()
            .iter()
            .all(|line| !line.contains("terminal session control") && !line.contains("stty")),
        "{:?}",
        rig.frame_writes()
    );
}

#[test]
fn a_loop_that_never_says_it_reads_is_said_and_retried_by_redialling() {
    let mut rig = looped(None);
    rig.run(10_000);
    assert!(rig.frame_writes().is_empty(), "{:?}", rig.frame_writes());
    assert_eq!(
        rig.client.notice(),
        Some((&Notice::InputDropped, Some(NoticeAction::Retry)))
    );
    // Retry: the channel is given up, the session redials.
    rig.act(Action::NoticeAction);
    rig.run(10);
    assert!(
        rig.events().contains(&Event::FrameChannelLost),
        "{:?}",
        rig.events()
    );
    assert_eq!(rig.client.notice(), None);
}

#[test]
fn attaches_that_never_paint_are_said_and_nothing_is_typed_into_a_shell() {
    let mut rig = looped(Some(100));
    // herdr takes the attach but never paints it.
    rig.frames.attach_ms = 60_000;
    rig.run(12_000);
    assert_eq!(
        rig.client.notice(),
        Some((&Notice::InputDropped, Some(NoticeAction::Retry)))
    );
    assert!(rig.frames.shell_lines.is_empty());
    assert!(
        rig.frame_writes()
            .iter()
            .all(|line| !line.contains("terminal session control") && !line.contains("stty")),
        "{:?}",
        rig.frame_writes()
    );
}

// MARK: - B466: an attach's last words are not all refusals

/// The server stopped or restarted (an upgrade on the desktop): the
/// attach's CLI says `herdr: server shut down: server is shutting down`.
/// It was said as a refusal and backed off from for 30 s, the dead
/// server's last frame on screen; now the next read decides: no server
/// (the empty state), or the focused pane, attached at once.
#[test]
fn a_server_restart_reattaches_at_once_and_is_no_refusal() {
    let mut rig = attached();
    rig.host_attach_ends("herdr: server shut down: server is shutting down");
    rig.run(3_000);
    assert!(
        !matches!(rig.client.notice(), Some((Notice::Refused(_), _))),
        "{:?}",
        rig.client.notice()
    );
    assert_eq!(
        rig.attach_lines()
            .iter()
            .filter(|line| *line == "w1:p1")
            .count(),
        2,
        "attached again: {:?}",
        rig.attach_lines()
    );
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
}

/// The pane on screen was closed on the desktop, and the attach's CLI
/// said so (`terminal attach ended: terminal … not found`). Backed off
/// from as a refusal, the closed pane stayed on screen for 30 s; now the
/// terminal follows the focus to the pane herdr moved to.
#[test]
fn the_pane_on_screen_closed_on_the_desktop_follows_the_focus() {
    let mut rig = attached();
    rig.host.panes.retain(|pane| pane.id != "w1:p1");
    rig.host.focused = Some("w1:p2".to_owned());
    rig.host_attach_ends(
        "herdr: server shut down: terminal attach ended: terminal term_1 not found",
    );
    rig.run(3_000);
    assert!(!matches!(
        rig.client.notice(),
        Some((Notice::Refused(_), _))
    ));
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"));
    assert!(rig.screen("w1:p2").contains("[w1:p2 #"));
}

/// Another device took the pane, said as the CLI's last words: the pane
/// is theirs, said with its take-back, not a refusal backed off from.
#[test]
fn a_takeover_said_by_the_cli_is_another_device_not_a_refusal() {
    let mut rig = attached();
    rig.host.held_elsewhere.push("w1:p1".to_owned());
    rig.host_attach_ends("herdr: server shut down: terminal attach taken over");
    rig.run(3_000);
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    assert_eq!(
        rig.client.attached_pane(),
        None,
        "never taken back without the person"
    );
}

// MARK: - B467: a hold another device had ends with the server

/// Another device held the pane, and the desktop restarted herdr between
/// two reads: every attach ended with the server, theirs too, and every
/// terminal is new. The phone kept saying the pane was on another device
/// and never followed the focus again until the person tapped "Open here".
#[test]
fn a_server_restarted_elsewhere_ends_the_hold_another_device_had() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    rig.host_restarted_elsewhere();
    rig.act(Action::Refresh);
    // After the device that had it has had its moment (B681).
    rig.run(9_000);
    assert_eq!(rig.client.notice(), None, "the hold ended with the server");
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
}

/// The same, with the server seen down between the reads (stopped on the
/// desktop, started again later).
#[test]
fn a_server_seen_down_ends_the_hold_another_device_had() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    rig.host.running = false;
    rig.host.held_elsewhere.clear();
    rig.act(Action::Refresh);
    rig.run(500);
    assert!(rig.client.server_not_running());
    rig.host.running = true;
    rig.act(Action::Refresh);
    rig.run(9_000);
    assert_ne!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
}

/// B681 (reworked): the server restarts (the desktop upgraded herdr) and
/// every client's attach ends with it; each device asks for its pane again.
/// Another device that had only been showing "open on another device" asked
/// first and kept the pane the person was using. A takeover by the device
/// that had it fixed that race, but would steal the pane from a device
/// whose person picked it in that moment. Now nobody takes over: the device
/// that had the pane asks plainly at once, and a pane another device holds
/// by then stays theirs, said.
#[test]
fn a_restart_never_takes_a_pane_another_device_picked_meanwhile() {
    let mut rig = attached();
    rig.host_restarted_elsewhere();
    // The other device's person picked it after the restart.
    rig.host.held_elsewhere.push("w1:p1".to_owned());
    let asked = rig.frames.took_over.len();
    rig.run(10_000);
    assert!(
        rig.frames.took_over[asked..].iter().all(|took| !took),
        "{:?}",
        rig.frames.took_over
    );
    assert!(rig.host.taken_from_others.is_empty());
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
}

/// B681 (reworked), the device that only showed "open on another device"
/// when the server restarted: it waits a moment before asking for a pane
/// again — the device that had the pane asks at once and gets it — then
/// asks plainly, and gets it if nobody has.
#[test]
fn a_device_showing_the_notice_waits_its_moment_after_a_restart() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(2_000);
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    let lines = rig.attach_lines().len();
    rig.host_restarted_elsewhere();
    rig.act(Action::Refresh);
    rig.run(5_000);
    assert_eq!(rig.attach_lines().len(), lines, "asked at once: {:?}", rig.attach_lines());
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice),
        "still said while it waits"
    );
    rig.run(4_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"), "nobody had it: ours");
    assert_eq!(rig.frames.took_over.last(), Some(&false), "plainly");
}

// MARK: - B470: a pick whose focus couldn't run

/// The person picked a pane, and the command moving herdr's focus there
/// couldn't run (the control connection dropped under it: over mosh the
/// next connection takes what comes after). The next read reported the
/// focus where it was and pulled the screen back, silently; the pick was
/// lost. Now the focus is asked once more, and the pick holds.
#[test]
fn a_pick_whose_focus_could_not_run_is_asked_again_and_holds() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("agent focus 'w1:p2'", None, Fate::UnreachableOnce));
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(1);
    assert!(
        rig.client.pane_switch_in_flight(),
        "still switching while it is asked again"
    );
    rig.run(5_000);
    assert_eq!(rig.host.focused.as_deref(), Some("w1:p2"), "asked again");
    assert_eq!(rig.client.attached_pane(), Some("w1:p2"), "not pulled back");
    assert!(!rig.client.pane_switch_in_flight());
}

/// One more time only: a focus that never runs is given up on, not asked
/// for forever.
#[test]
fn a_pick_whose_focus_never_runs_is_asked_once_more_then_given_up() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("agent focus 'w1:p2'", None, Fate::Unreachable));
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(10_000);
    assert_eq!(rig.commands("agent focus 'w1:p2'").len(), 2);
    assert!(!rig.client.pane_switch_in_flight());
}

/// What was queued behind the pick (a reply's focus check) sees the focus
/// asked again: the retry takes the failed step's place in line, not the
/// end of it, where the check read the old focus first and the reply was
/// refused as never focused.
#[test]
fn a_check_queued_behind_the_pick_sees_the_focus_asked_again() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("agent focus 'w1:p2'", None, Fate::UnreachableOnce));
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    let request = rig
        .act(Action::ConfirmFocus { budget_ms: 5_000 })
        .expect("a request");
    rig.run(5_000);
    assert!(
        rig.events().iter().any(|event| matches!(
            event,
            Event::FocusConfirmed { request: r, pane } if *r == request && pane.as_deref() == Some("w1:p2")
        )),
        "{:?}",
        rig.events()
            .iter()
            .filter(|e| matches!(e, Event::FocusConfirmed { .. }))
            .collect::<Vec<_>>()
    );
}

// MARK: - B487: a pane the other device left comes back by itself

/// Another device took the pane on screen, then moved on to another pane
/// (the server's focus went with it, and its attach left ours). The phone
/// kept saying the pane was on another device until the person tapped
/// "Open here". Now a plain attach — never a takeover — asks for it again
/// when the focus moves away from it, and it's ours again: the notice
/// goes, and the other device's focus doesn't pull us after it.
#[test]
fn a_pane_the_other_device_left_comes_back_with_a_plain_attach() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    // The other device goes to p2: it holds p2 now, and has let p1 go.
    rig.host.held_elsewhere = vec!["w1:p2".to_owned()];
    rig.host.focused = Some("w1:p2".to_owned());
    rig.act(Action::Refresh);
    rig.run(3_000);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"), "ours again");
    assert_eq!(rig.client.notice(), None);
    assert!(rig.host.taken_from_others.is_empty(), "nothing taken");
    rig.run(20_000);
    assert_eq!(
        rig.client.attached_pane(),
        Some("w1:p1"),
        "their focus doesn't pull us after it"
    );
    assert!(!rig.attach_lines().contains(&"w1:p2".to_owned()));
}

/// Still theirs: the plain attach is refused, said as before, and asked
/// again at most every ten seconds.
#[test]
fn a_pane_still_held_is_asked_for_plainly_at_most_every_ten_seconds() {
    let mut rig = attached();
    rig.host_take_over();
    rig.run(100);
    rig.host.held_elsewhere = vec!["w1:p1".to_owned(), "w1:p2".to_owned()];
    rig.host.focused = Some("w1:p2".to_owned());
    rig.act(Action::Refresh);
    rig.run(25_000);
    let asks = rig
        .attach_lines()
        .iter()
        .filter(|line| *line == "w1:p1")
        .count();
    assert!(
        (2..=4).contains(&asks),
        "the first attach and a probe each ten seconds at most: {asks}"
    );
    assert!(
        rig.frames.took_over.iter().all(|took| !took),
        "never a takeover"
    );
    assert_eq!(
        rig.client.notice().map(|(notice, _)| notice.clone()),
        Some(Notice::OnAnotherDevice)
    );
    assert_eq!(rig.client.attached_pane(), None);
}

// MARK: - B490: a refusal's backoff, said and cut short

fn refused(rig: &mut Rig) {
    rig.host_attach_ends("herdr: terminal session control failed: busy");
    rig.run(100);
    assert!(matches!(rig.client.notice(), Some((Notice::Refused(_), _))));
}

fn p1_attaches(rig: &Rig) -> usize {
    rig.attach_lines()
        .iter()
        .filter(|line| *line == "w1:p1")
        .count()
}

/// A refusal was said with no button and backed off from for 30 s. Now it
/// says how long ("Retry" too), and the button asks again at once —
/// never a restart of the server.
#[test]
fn a_refusal_says_its_wait_and_retry_asks_again_at_once() {
    let mut rig = attached();
    refused(&mut rig);
    assert_eq!(
        rig.client.notice().map(|(_, action)| action),
        Some(Some(NoticeAction::Retry))
    );
    assert_eq!(rig.client.retry_wait(), Some(30_000));
    rig.act(Action::NoticeAction);
    rig.run(1_000);
    assert_eq!(p1_attaches(&rig), 2, "asked again at once");
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    assert_eq!(rig.client.notice(), None);
    assert_eq!(rig.client.retry_wait(), None);
    assert!(
        rig.commands("server stop").is_empty(),
        "a retry is no restart"
    );
}

/// Back from the background: whatever was backed off from is asked again.
#[test]
fn the_return_asks_again_at_once() {
    let mut rig = attached();
    refused(&mut rig);
    let now = rig.now + 100;
    rig.client.foreground(now);
    rig.pump();
    rig.run(1_000);
    assert_eq!(p1_attaches(&rig), 2);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
}

/// A server of another version answering (upgraded on the host): asked
/// again at once, not after the rest of the wait.
#[test]
fn a_new_server_version_asks_again_at_once() {
    let mut rig = attached();
    refused(&mut rig);
    rig.host.version = "0.9.6".to_owned();
    rig.act(Action::Refresh);
    rig.run(1_000);
    assert_eq!(p1_attaches(&rig), 2);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
}

// MARK: - B491: a pick that couldn't be made is said

/// The pick's focus couldn't run, even asked again (B470): the next read
/// pulled the screen back to the pane it was on without a word. It says
/// so now, for a moment.
#[test]
fn a_pick_that_couldnt_be_made_is_said_for_a_moment() {
    let mut rig = attached();
    rig.host
        .rules
        .push(rule("agent focus 'w1:p2'", None, Fate::Unreachable));
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(500);
    assert_eq!(
        rig.commands("agent focus 'w1:p2'").len(),
        2,
        "asked once more first"
    );
    assert_eq!(
        rig.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action)),
        Some((Notice::SwitchFailed, None))
    );
    rig.run(4_000);
    assert_eq!(rig.client.notice(), None, "only for a moment");
}
