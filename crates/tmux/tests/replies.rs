//! B273: a reply is the command's whose it is. tmux answers a control
//! client's command with one `%begin … %end` block, and the controller
//! pairs blocks with commands by position; two kinds of block break the
//! count. Both read off tmux 3.6a on a private server (`-L`, `-f /dev/null`):
//!
//! - blocks for commands nobody here sent: an `after-<command>` hook that
//!   one of ours fired (`set-hook -g after-select-window 'refresh-client
//!   -S'`, then `select-window`: its own block, then the hook's), the late
//!   half of an `if-shell -b` or `run-shell -b`. tmux flags those 0 and the
//!   replies to this client's own lines 1.
//! - a command that answers more than once: an `if-shell` that runs its
//!   command (two blocks), a `source-file` (one, then one per command in the
//!   file), `a ; b` (one each), a `run-shell -C`.
//!
//! Either, once, put every later reply one command off — the misfiling that
//! cost one mosh + tmux connect in twenty its renderer (B271).

use std::cell::RefCell;
use std::rc::Rc;

use moshpit_tmux::controller::{Config, answers_once};
use moshpit_tmux::rig::{Responder, Rig, block, ok};

const WINDOWS: [&str; 2] = [
    "$0 @0 0 81x24,0,0,0 1 1 main",
    "$0 @1 1 81x24,0,0,1 0 1 work",
];
const PANES: [&str; 2] = ["%0 @0 0 80 24 1 0 0 bash", "%1 @1 0 80 24 1 0 0 vim"];

/// tmux with one session, two windows, a pane each; `extra` adds what a
/// command's answer brings after its block (a hook's, a second one).
fn world(extra: impl Fn(&str) -> String + 'static) -> Responder {
    Box::new(move |command: &str, _| {
        let mut answer = if command.starts_with("list-sessions") {
            ok(["$0 1 main"])
        } else if command.starts_with("list-windows") {
            ok(WINDOWS)
        } else if command.starts_with("list-panes -a -F '#{pane_id} #{window_id}") {
            ok(PANES)
        } else if command.starts_with("capture-pane -p -e -S ") {
            ok(["HISTORY"])
        } else if command.starts_with("capture-pane -p -e -t ") {
            ok(["SCREEN"])
        } else {
            None
        }
        .unwrap_or_default();
        answer.after = extra(command);
        Some(answer)
    })
}

fn attached(responder: Responder) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.respond(responder);
    rig.attach();
    rig.settle();
    rig
}

/// Every listing landed where it belongs: the session's name is a name, the
/// windows and panes are tmux's.
fn assert_in_step(rig: &Rig) {
    let snapshot = rig.controller.snapshot();
    let sessions: Vec<(&str, &str)> = snapshot
        .sessions
        .values()
        .map(|s| (s.id.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(sessions, [("$0", "main")], "{:?}", rig.commands);
    let windows: Vec<(&str, &str)> = snapshot
        .windows
        .values()
        .map(|w| (w.id.as_str(), w.name.as_str()))
        .collect();
    assert_eq!(windows, [("@0", "main"), ("@1", "work")], "{:?}", rig.commands);
    let panes: Vec<&str> = snapshot.panes.keys().map(String::as_str).collect();
    assert_eq!(panes, ["%0", "%1"]);
}

/// A block tmux writes for a command this client never sent: flags 0.
fn hook_block(num: i64, lines: &[&str]) -> String {
    let lines: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    block(num, &lines, false).replace(&format!(" {num} {num} 1\n"), &format!(" {num} {num} 0\n"))
}

#[test]
fn a_hooks_block_after_our_command_answers_nothing_of_ours() {
    // `after-select-window` and `after-split-window` hooks, as people set
    // them: each select-window and split-window of ours is answered by its
    // own block and then the hook's.
    let mut rig = attached(world(|command| {
        if command.starts_with("select-window") || command.starts_with("split-window") {
            hook_block(9000, &[])
        } else {
            String::new()
        }
    }));
    assert_in_step(&rig);
    let seen = rig.seen.len();
    rig.with(|c, now| c.select_window("@1", now));
    rig.settle();
    // The window's pane is filled from its own captures: the history, then
    // the screen. One block off, the history capture took the mouse
    // probe's reply and the screen capture the history's.
    let filled: String = rig.seen[seen..]
        .iter()
        .filter_map(|seen| match seen {
            moshpit_tmux::rig::Seen::Pane(pane, bytes) if pane == "%1" => {
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            _ => None,
        })
        .collect();
    let history = filled.find("HISTORY");
    let screen = filled.find("SCREEN");
    assert!(
        history.is_some() && screen.is_some() && history < screen,
        "{filled:?}"
    );
    rig.with(|c, now| c.new_pane(now));
    rig.settle();
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert_in_step(&rig);
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@1")
    );
}

#[test]
fn a_late_background_block_answers_nothing_of_ours() {
    // `if-shell -b` / `run-shell -b` from a binding or a hook: their
    // command's block comes whenever the job ends, flags 0, between our
    // replies.
    let mut rig = attached(world(|_| String::new()));
    rig.with(|c, now| c.refresh(now));
    // Before any of the refresh's replies.
    rig.push(&hook_block(9100, &["$9 1 intruder"]));
    rig.settle();
    assert_in_step(&rig);
}

#[test]
fn a_command_that_answers_twice_is_fenced_and_the_next_reply_is_its_own() {
    // tmux answers an `if-shell` that runs its command with its own block,
    // then the command's.
    let mut rig = attached(world(|command| {
        if command.starts_with("if-shell ") {
            block(9200, &["inner".to_owned()], false)
        } else {
            String::new()
        }
    }));
    let before = rig.commands.len();
    rig.with(|c, now| c.run_command("if-shell -F 1 'display-message -p inner'", now));
    assert_eq!(
        rig.commands[before..],
        [
            "if-shell -F 1 'display-message -p inner'".to_owned(),
            "display-message -p 'moshpit-fence-1'".to_owned(),
        ]
    );
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert_in_step(&rig);
}

#[test]
fn a_sourced_file_and_a_sequence_take_all_their_blocks() {
    // `source-file` answers once for itself and once per command in the
    // file; `a ; b`, once each; an error among them too.
    let sourced: Rc<RefCell<usize>> = Rc::default();
    let count = sourced.clone();
    let mut rig = attached(world(move |command| {
        if command.starts_with("source-file ") {
            *count.borrow_mut() += 1;
            [
                block(9300, &["one".to_owned()], false),
                block(9301, &["no such option".to_owned()], true),
                block(9302, &["three".to_owned()], false),
            ]
            .concat()
        } else if command.starts_with("display-message -p a ;") {
            block(9310, &["b".to_owned()], false)
        } else {
            String::new()
        }
    }));
    rig.with(|c, now| c.run_command("source-file ~/.config/tmux/extra.conf", now));
    rig.with(|c, now| c.run_command("display-message -p a ; display-message -p b", now));
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert_eq!(*sourced.borrow(), 1);
    assert_in_step(&rig);
}

#[test]
fn a_command_that_answers_once_goes_unfenced() {
    let mut rig = attached(world(|_| String::new()));
    let before = rig.commands.len();
    rig.with(|c, now| c.run_command("display-message -p hi", now));
    assert_eq!(rig.commands[before..], ["display-message -p hi".to_owned()]);
}

#[test]
fn a_fenced_command_still_waiting_hears_of_a_closed_channel() {
    // The channel goes before the fence's block: nothing waits forever, and
    // the next attach starts counting afresh.
    let mut rig = attached(world(|_| String::new()));
    rig.with(|c, now| c.run_command("if-shell -F 1 'display-message -p x'", now));
    rig.with(|c, now| c.channel_closed(now));
    rig.answered = rig.commands.len();
    rig.attach();
    rig.settle();
    assert_in_step(&rig);
}

#[test]
fn which_commands_answer_once() {
    for command in [
        "list-sessions -F '#{session_id} #{session_attached} #{session_name}'",
        "display-message -p -t %0 '#{cursor_x} #{cursor_y}'",
        "select-layout -t '@0' '4b9d,203x60,0,0[203x30,0,0,0,203x29,0,31{101x29,0,31,1,101x29,102,31,2}]'",
        "send-keys -t %0 -l 'it'\\''s; {fine}'",
        "set-option -t '$5' -u status",
        "display-message -p \"a;b\"",
        "refresh-client -A %0:continue",
    ] {
        assert!(answers_once(command), "{command}");
    }
    for command in [
        "if-shell -F 1 'display-message -p x'",
        "if -F 1 'display-message -p x'",
        "source-file ~/.tmux.conf",
        "source ~/.tmux.conf",
        "run-shell -C 'display-message -p x'",
        "display-message -p a ; display-message -p b",
        "display-message -p a \\; display-message -p b",
        "display-message -p a; display-message -p b",
        "new-window { display-message -p x }",
        "neww",
        "my-alias",
        "",
    ] {
        assert!(!answers_once(command), "{command}");
    }
}

/// Every command the controller sends on its own, in every committed
/// script, answers once: none of them needs a fence (a new one that would
/// gets it, and shows up here first).
#[test]
fn the_controllers_own_commands_all_answer_once() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/controller");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // A `run` step's command is the person's, not the controller's.
        let mut step = "";
        for line in text.lines() {
            if line.starts_with('@') {
                step = line.split_once(' ').map_or("", |(_, step)| step);
                continue;
            }
            let Some(command) = line.strip_prefix("> ") else {
                continue;
            };
            if step.starts_with("run ") && !command.starts_with("display-message -p 'moshpit-fence-") {
                continue;
            }
            seen += 1;
            assert!(
                answers_once(command),
                "{}: {command}",
                path.file_name().unwrap().to_string_lossy()
            );
        }
    }
    assert!(seen > 1000, "{seen}");
}

#[test]
fn a_begin_lines_flags_say_whose_command_it_was() {
    use moshpit_tmux::{ControlParser, Event};
    let mut parser = ControlParser::new();
    let mut events = Vec::new();
    parser.feed(
        b"%begin 1 1 0\n%end 1 1 0\n\
          %begin 1 2 1\n%end 1 2 1\n\
          %begin 1 3 0\nhook\n%end 1 3 0\n\
          %begin 1 4\n%end 1 4\n\
          %begin 1 5 x\n%end 1 5 x\n\
          %begin 1 6 3\n%end 1 6 3\n\
          %begin 1 7 2\n%end 1 7 2\n",
        |event| events.push(event),
    );
    let flags: Vec<(i64, bool)> = events
        .iter()
        .filter_map(|event| match event {
            Event::CommandResponse(r) => Some((r.command_num, r.from_client)),
            _ => None,
        })
        .collect();
    // The first block is the boot command's, swallowed.
    assert_eq!(
        flags,
        [(2, true), (3, false), (4, true), (5, true), (6, true), (7, false)]
    );
}



/// A server that flags no reply as ours (every block 0) is read the old
/// way: every block a reply. tmux has flagged its replies to a control
/// client's lines 1 since control mode began (3.6a, read; the code since
/// 1.8 sets it), so this is insurance, not a case seen.
#[test]
fn a_server_that_flags_nothing_is_read_the_old_way() {
    let mut rig = Rig::new(Config::default());
    rig.respond(world(|_| String::new()));
    rig.with(|c, now| c.attach(now));
    rig.boot();
    // Answer by hand, every block flagged 0.
    for _ in 0..8 {
        while rig.answered < rig.commands.len() {
            let command = rig.commands[rig.answered].clone();
            rig.answered += 1;
            let lines: Vec<&str> = if command.starts_with("list-sessions") {
                vec!["$0 1 main"]
            } else if command.starts_with("list-windows") {
                WINDOWS.to_vec()
            } else if command.starts_with("list-panes -a -F '#{pane_id} #{window_id}") {
                PANES.to_vec()
            } else {
                vec![]
            };
            let num = 5000 + rig.answered as i64;
            rig.push(&hook_block(num, &lines));
        }
    }
    assert_in_step(&rig);
}
