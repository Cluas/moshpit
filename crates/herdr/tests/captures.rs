//! The port against live herdr 0.9.5 output, recorded from a private server
//! (its own HOME, session `moshpit-w2`) by `harness/record.py`:
//! `tests/fixtures/recorded/`. The frame channel while typing, resizing,
//! scrolling, clicking and pasting; every CLI reply the control plane reads;
//! `herdr status`; an `events.subscribe` conversation.

use std::fs;

use moshpit_herdr::control::{self, CommandResult, PollReading, Refusal, WorktreeRemoval};
use moshpit_herdr::frame::{self, Frame, FrameParser};
use moshpit_herdr::json;
use moshpit_herdr::snapshot;
use moshpit_herdr::socket::{self, Output, SocketClient};
use moshpit_herdr::version::VersionReport;

fn bytes(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/recorded/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn text(name: &str) -> String {
    String::from_utf8(bytes(name)).expect("UTF-8")
}

struct Reply {
    command: String,
    result: CommandResult,
}

fn replies() -> Vec<Reply> {
    text("replies.jsonl")
        .lines()
        .map(|line| {
            let object = json::parse_object(line.as_bytes()).expect("a reply");
            Reply {
                command: object.str("command").unwrap().to_owned(),
                result: CommandResult {
                    output: object.str("output").unwrap().to_owned(),
                    exit_code: object.i64("exit"),
                },
            }
        })
        .collect()
}

fn reply(command: &str) -> CommandResult {
    replies()
        .into_iter()
        .find(|reply| reply.command == command)
        .unwrap_or_else(|| panic!("no recorded reply to {command}"))
        .result
}

fn screens(frames: &[Frame]) -> String {
    frames
        .iter()
        .filter_map(|frame| match frame {
            Frame::Screen { bytes, .. } => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .collect()
}

// MARK: - The frame channel

#[test]
fn a_live_frame_stream_decodes_in_order_however_it_is_cut() {
    let stream = bytes("frames.jsonl");
    let frames = FrameParser::new().consume(&stream);
    assert_eq!(frames.len(), 41);
    let seqs: Vec<u64> = frames
        .iter()
        .filter_map(|frame| match frame {
            Frame::Screen { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, (1..=40).collect::<Vec<u64>>());
    // The first frame is a full repaint at the attach's size; each resize
    // brings one at the new size.
    assert!(matches!(
        frames[0],
        Frame::Screen {
            width: 80,
            height: 24,
            full: true,
            ..
        }
    ));
    for (width, height) in [(100, 30), (60, 20)] {
        assert!(frames.iter().any(|frame| matches!(
            frame,
            Frame::Screen { width: w, height: h, full: true, .. } if *w == width && *h == height
        )));
    }
    // The release ends the channel, herdr says why.
    assert_eq!(
        frames.last(),
        Some(&Frame::Closed {
            reason: Some("detached".to_owned())
        })
    );
    for size in [1, 7, 4096] {
        let mut parser = FrameParser::new();
        let cut: Vec<Frame> = stream
            .chunks(size)
            .flat_map(|chunk| parser.consume(chunk))
            .collect();
        assert_eq!(cut, frames, "chunks of {size}");
    }
    // What was typed comes back painted.
    assert!(screens(&frames).contains("hello"));
}

/// `terminal.mouse` (herdr ≥ 0.9.2), at 0-based column 12, row 4, with the
/// program's SGR mouse reporting on: the program read the press and the
/// release at 1-based 13;5. No Swift counterpart (frame::mouse).
#[test]
fn a_mouse_event_reaches_a_program_that_asked_for_one() {
    let painted = screens(&FrameParser::new().consume(&bytes("frames.jsonl")));
    assert!(painted.contains("^[[<0;13;5M"));
    assert!(painted.contains("^[[<0;13;5m"));
    let log = text("frames-log.jsonl");
    let down = frame::mouse(frame::MouseAction::Down, frame::MouseButton::Left, 12, 4, 0);
    let wrote: Vec<json::Object> = log
        .lines()
        .map(|line| json::parse_object(line.as_bytes()).unwrap())
        .filter_map(|entry| {
            entry
                .str("line")
                .and_then(|line| json::parse_object(line.as_bytes()))
        })
        .filter(|command| command.str("type") == Some("terminal.mouse"))
        .collect();
    assert_eq!(wrote.first(), json::parse_object(down.as_bytes()).as_ref());
}

/// At zsh's prompt, live: the port's paste of
/// `ls◌́⏎echo SAFE$((40+2))⏎` is one paste, inserted, not run (no SAFE42
/// on screen). The Swift payloads for the same kind of text end the paste
/// early, and herdr's zsh runs what follows (INJ42: paste-marker-unicode,
/// NEST42: paste-end-marker).
#[test]
fn pasted_text_runs_only_when_its_end_marker_survives() {
    let painted = screens(&FrameParser::new().consume(&bytes("frames.jsonl")));
    assert!(!painted.contains("SAFE42"), "the port's paste ran");
    assert!(
        painted.contains("INJ42"),
        "the recording no longer shows the injection"
    );
    assert!(painted.contains("NEST42"));
    // What the port sends for those two texts: one paste each.
    for text in [
        "ls\u{1b}[201~\u{301}\necho INJ$((40+2))\n",
        "\u{1b}[20\u{1b}[201~1~\necho NEST$((40+2))\n",
    ] {
        let paste = frame::bracketed_paste(text);
        let body = &paste[6..paste.len() - 6];
        assert!(!body.windows(6).any(|window| window == b"\x1b[201~"));
    }
}

/// herdr's complaint about a command it can't take arrives on stderr, which
/// the app's PTY merges into the stream: a fault, not a handshake drop.
#[test]
fn a_command_herdr_rejects_comes_back_as_a_fault() {
    let frames = FrameParser::new().consume(&bytes("frames.stderr"));
    assert_eq!(
        frames,
        vec![Frame::Fault {
            message: "terminal session control input ignored: invalid json command: invalid value: integer `70000`, expected u16".to_owned()
        }]
    );
    let Frame::Fault { message } = &frames[0] else {
        unreachable!()
    };
    assert!(!frame::is_handshake_drop(message));
}

#[test]
fn an_attach_to_a_missing_pane_closes_with_herdrs_reason() {
    let refused = reply("terminal session control w9:p9 --cols 80 --rows 24 --takeover");
    assert_eq!(
        FrameParser::new().consume(refused.output.as_bytes()),
        vec![Frame::Closed {
            reason: Some(
                "terminal session control failed: terminal target w9:p9 not found".to_owned()
            )
        }]
    );
}

// MARK: - The control plane's replies

#[test]
fn snapshots_from_a_live_server_decode() {
    let snapshots: Vec<CommandResult> = replies()
        .into_iter()
        .filter(|reply| reply.command == "api snapshot")
        .map(|reply| reply.result)
        .collect();
    // Every read the recorder took, in order.
    assert_eq!(snapshots.len(), 8);
    // No server yet: exit 1 and server_not_running.
    assert_eq!(snapshots[0].exit_code, Some(1));
    assert!(snapshot::is_server_not_running(&snapshots[0].output));
    assert_eq!(control::read_poll(&snapshots[0]), PollReading::NoServer);
    // A fresh server comes up empty: decoded, nothing in it, not attached.
    let empty = snapshot::decode(&snapshots[1].output).expect("an empty tree");
    assert!(empty.snapshot.sessions.is_empty());
    assert!(!empty.snapshot.is_attached);
    // One workspace, one pane.
    let first = snapshot::decode(&snapshots[2].output).expect("the tree");
    assert_eq!(first.snapshot.panes.len(), 1);
    assert_eq!(first.snapshot.active_pane_id.as_deref(), Some("w1:p1"));
    // Two workspaces, the first with two tabs, one of them split.
    let full = snapshot::decode(&snapshots[3].output).expect("the tree");
    assert_eq!(full.snapshot.sessions.len(), 2);
    assert_eq!(full.snapshot.sessions["w1"].name, "rec");
    assert_eq!(full.snapshot.windows_in_session("w1").len(), 2);
    assert_eq!(full.snapshot.panes.len(), 4);
    assert_eq!(full.snapshot.active_pane_id.as_deref(), Some("w1:p3"));
    assert!(full.terminal_ids.values().all(|id| id.starts_with("term_")));
    assert_eq!(full.pane_cwds.len(), 4);
    // The worktree's workspace is marked as one.
    let with_worktree = snapshot::decode(&snapshots[4].output).unwrap();
    assert_eq!(
        with_worktree
            .worktree_repos
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        ["repo"]
    );
}

#[test]
fn live_refusals_read_as_herdrs_words() {
    for command in ["agent focus w1:p1", "agent prompt w1:p1 hello"] {
        let result = reply(command);
        assert_eq!(
            control::refusal(Some(&result)),
            Some(Refusal::Said("agent target w1:p1 not found".to_owned()))
        );
        // Detection hasn't named an agent: worth asking again.
        assert!(control::agent_not_up_yet(&result));
    }
    let missing = reply("pane close w9:p9");
    assert_eq!(
        control::refusal(Some(&missing)),
        Some(Refusal::Said("pane w9:p9 not found".to_owned()))
    );
    assert!(!control::agent_not_up_yet(&missing));
    // A key pressed, a command typed: nothing printed, exit 0, done.
    for command in [
        "pane send-keys w1:p1 Enter",
        "pane run w1:p1 true",
        "pane zoom --pane w1:p1 --on",
    ] {
        assert_eq!(control::refusal(Some(&reply(command))), None, "{command}");
    }
}

#[test]
fn a_live_worktree_task_reads_through_create_and_both_removals() {
    let create = replies()
        .into_iter()
        .find(|reply| reply.command.starts_with("worktree create"))
        .unwrap()
        .result;
    assert_eq!(
        control::worktree_created(Some(&create), "rec-task").as_deref(),
        Ok("w4:p1")
    );
    assert_eq!(
        control::worktree_removal(Some(&reply("worktree remove --workspace w4"))),
        WorktreeRemoval::NeedsForce
    );
    assert_eq!(
        control::worktree_removal(Some(&reply("worktree remove --workspace w4 --force"))),
        WorktreeRemoval::Removed
    );
}

/// herdr 0.9.5 wraps the manifests in an envelope (`{"result":{"manifests":
/// [...]}}`, a bare array before): first `[` to last `]` still spans exactly
/// the list.
#[test]
fn live_agent_manifests_name_the_agents_herdr_detects() {
    let names = control::agent_names(&reply("server agent-manifests --json").output);
    assert!(names.contains(&"claude".to_owned()));
    assert!(names.contains(&"codex".to_owned()));
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert!(names.len() >= 10);
}

#[test]
fn live_status_reports_both_ends_only_while_a_server_runs() {
    let running = VersionReport::parse(&text("status-running.txt")).unwrap();
    assert_eq!(
        running,
        VersionReport {
            client: "0.9.5".to_owned(),
            server: "0.9.5".to_owned()
        }
    );
    assert!(!running.mismatched());
    assert_eq!(VersionReport::parse(&text("status-stopped.txt")), None);
    assert_eq!(VersionReport::parse(&reply("status").output), Some(running));
}

// MARK: - The socket

/// herdr 0.9.5 answers the subscribe and then streams live mutations; unlike
/// 0.8.0 (tests/fixtures/app/herdr-socket-events.jsonl) it replays no
/// existing state first. The push driver only reads events as "something
/// changed", so nothing depends on the replay.
#[test]
fn a_live_subscription_answers_then_streams_mutations() {
    let mut client = SocketClient::new();
    let (id, _) = client
        .subscribe(&socket::GLOBAL_KINDS, &[])
        .expect("a request");
    let outputs = client.feed(&bytes("socket.jsonl"));
    let Some(Output::Reply {
        id: replied,
        result: Ok(line),
    }) = outputs.first()
    else {
        panic!("the subscribe was not answered first: {outputs:?}");
    };
    assert_eq!(replied, &id);
    assert!(line.contains("subscription_started"));
    let kinds: Vec<&str> = outputs
        .iter()
        .filter_map(|output| match output {
            Output::Event(event) => Some(event.kind.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "workspace_focused",
            "tab_created",
            "tab_focused",
            "pane_created",
            "pane_focused",
            "layout_updated",
            "workspace_renamed",
            "tab_closed"
        ]
    );
}
