//! The control plane's commands and how their results are read. Ported from
//! `Tests/MoshpitTests/Services/Herdr/HerdrControlClientTests.swift` (every
//! test whose subject is a command line, a result's reading or a decision;
//! the lane, timer and ordering tests belong to the transport) and
//! `Tests/MoshpitTests/Models/AgentTaskRequestTests.swift` (validation).
//!
//! Where a Swift test drives the client against a fake runner and asserts on
//! the command it sent, the Rust test builds that command; where it asserts
//! on what the client made of a reply, the Rust test reads the reply.

use std::time::Duration;

use moshpit_herdr::control::{
    self, AgentTaskRequest, CommandResult, FirstMessageStep, PollEffect, PollReading, PollState,
    Refusal, RemovalFailure, TaskFailure, TaskIncomplete, TaskInvalid, TaskOutcome,
    WorktreeRemoval,
};
use moshpit_herdr::snapshot;
use moshpit_herdr::version::VersionReport;

const TWO_WORKSPACES: &str = r#"{"id":"cli:api:snapshot","result":{"snapshot":{
  "workspaces":[{"workspace_id":"w1","label":"~","focused":true},
                {"workspace_id":"w2","label":"api","focused":false}],
  "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
  "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":false},
           {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":true}],
  "layouts":[],
  "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p2"}}}"#;

const WORKTREE_CREATED: &str = r#"{"id":"cli:worktree:create","result":{
  "type":"worktree_created",
  "workspace":{"workspace_id":"w4","label":"fix-scroll","focused":true},
  "root_pane":{"pane_id":"w4:p1","cwd":"/Users/dev/.herdr/worktrees/moshpit/fix-scroll"}}}"#;

/// `output` and the exit-status trailer every command carries: what a real
/// shell sends back, read.
fn exited(code: i64, output: &str) -> CommandResult {
    control::parse_result(&format!("{output}\n{}{code}", control::EXIT_CODE_MARKER))
}

fn read(raw: &str) -> PollReading {
    control::read_poll(&control::parse_result(raw))
}

fn tree() -> snapshot::Snapshot {
    snapshot::decode(TWO_WORKSPACES).unwrap().snapshot
}

// MARK: - Reading

#[test]
fn the_first_poll_populates_the_tree() {
    let mut state = PollState::new();
    let effects = state.apply(read(TWO_WORKSPACES));
    assert_eq!(state.snapshot.sessions.len(), 2);
    assert_eq!(state.snapshot.active_pane_id.as_deref(), Some("w1:p2"));
    assert!(state.snapshot.is_attached);
    assert!(effects.contains(&PollEffect::HooksUpdated));
    assert!(control::command_line(control::SNAPSHOT, None).contains("api snapshot"));
}

#[test]
fn a_custom_binary_path_is_used_verbatim_for_control_commands_too() {
    assert!(
        control::command_line(control::SNAPSHOT, Some("/opt/herdr/bin/herdr"))
            .starts_with("/opt/herdr/bin/herdr api snapshot")
    );
}

#[test]
fn without_a_custom_path_the_path_prefix_rides_along() {
    assert!(control::command_line(control::SNAPSHOT, None).contains("$HOME/.local/bin"));
}

/// The command's run failed (the channel hiccupped): there is no reading at
/// all, and the client keeps what it had. Here: no reading is applied.
#[test]
fn a_failed_or_garbage_read_keeps_the_last_good_tree() {
    let mut state = PollState::new();
    state.apply(read(TWO_WORKSPACES));
    assert_eq!(read("bash: herdr: command not found"), PollReading::NoNews);
    state.apply(read("bash: herdr: command not found"));
    assert_eq!(state.snapshot.sessions.len(), 2);
}

/// B445: a read that failed for another reason than "no server" (herdr's
/// request timed out, its CLI crashed) is nothing new. It emptied the tree
/// and dropped the terminal out of its slot until the next good read.
#[test]
fn only_a_read_that_says_no_server_is_no_server() {
    assert_eq!(
        control::read_poll(&exited(
            1,
            r#"{"error":{"code":"server_not_running","message":"no server"}}"#
        )),
        PollReading::NoServer
    );
    assert_eq!(
        control::read_poll(&exited(
            1,
            r#"Error: Os { code: 2, kind: NotFound, message: "No such file or directory" }"#
        )),
        PollReading::NoServer,
        "herdr 0.7.3, no socket"
    );
    assert_eq!(
        control::read_poll(&exited(
            1,
            r#"Error: Os { code: 61, kind: ConnectionRefused, message: "Connection refused" }"#
        )),
        PollReading::NoServer,
        "0.7.3, a dead server's socket"
    );
    assert_eq!(
        control::read_poll(&exited(127, "sh: herdr: command not found")),
        PollReading::NoServer,
        "no herdr, no server"
    );
    assert_eq!(
        control::read_poll(&exited(1, "herdr: request timed out")),
        PollReading::NoNews
    );
    assert_eq!(
        control::read_poll(&exited(101, "thread 'main' panicked at src/cli.rs")),
        PollReading::NoNews
    );
    let mut state = PollState::new();
    state.apply(read(TWO_WORKSPACES));
    state.apply(control::read_poll(&exited(1, "herdr: request timed out")));
    assert_eq!(state.snapshot.sessions.len(), 2, "the tree stays");
    assert!(!state.server_not_running);
}

#[test]
fn server_not_running_empties_the_tree_but_remembers_it_was_attached() {
    let mut state = PollState::new();
    state.apply(read(TWO_WORKSPACES));
    state.apply(read(
        r#"{"error":{"code":"server_not_running","message":"no herdr server is running"},"id":"cli:api:snapshot"}"#,
    ));
    assert!(state.snapshot.sessions.is_empty());
    assert!(state.server_not_running);
    // …so the home card shows an empty list, not a stuck "Attaching…".
    assert!(state.snapshot.ever_attached);
}

/// The panes' terminal ids, directories and worktree repos go with the
/// tree: the ids stayed, and a pick or a reply for a pane of the gone tree
/// pointed the mosh renderer at a terminal no server has.
#[test]
fn server_not_running_takes_the_panes_terminal_ids_with_the_tree() {
    let mut state = PollState::new();
    state.apply(read(
        r#"{"id":"cli:api:snapshot","result":{"snapshot":{
          "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
          "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
          "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true,"terminal_id":"t-1","cwd":"/r"},
                   {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":false,"terminal_id":"t-2","cwd":"/r"}],
          "layouts":[],
          "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}"#,
    ));
    assert_eq!(state.terminal_ids.len(), 2);
    assert!(!state.pane_cwds.is_empty());
    state.apply(read(
        r#"{"error":{"code":"server_not_running","message":"no herdr server is running"},"id":"cli:api:snapshot"}"#,
    ));
    assert!(state.server_not_running);
    assert!(
        state.terminal_ids.is_empty(),
        "the gone tree's terminal ids: {:?}",
        state.terminal_ids
    );
    assert!(state.pane_cwds.is_empty());
    assert!(state.worktree_repos.is_empty());
}

/// herdr 0.7.3 prints a bare Rust error for a dead socket, not 0.8.0's
/// structured `server_not_running`; the exit status is 1 in both.
#[test]
fn a_dead_socket_is_detected_by_exit_status_not_by_error_wording() {
    let mut state = PollState::new();
    state.apply(read(TWO_WORKSPACES));
    assert!(!state.server_not_running);
    state.apply(read(&format!(
        "Error: Os {{ code: 2, kind: NotFound, message: \"No such file or directory\" }}\n{}1",
        control::EXIT_CODE_MARKER
    )));
    assert!(state.server_not_running);
    assert!(state.snapshot.sessions.is_empty());
}

#[test]
fn a_zero_exit_with_junk_output_is_no_new_information_not_no_server() {
    let mut state = PollState::new();
    state.apply(read(TWO_WORKSPACES));
    state.apply(read(&format!(
        "some unrelated chatter\n{}0",
        control::EXIT_CODE_MARKER
    )));
    assert!(!state.server_not_running);
    assert_eq!(state.snapshot.sessions.len(), 2);
}

/// herdr's direct attach is exclusive per terminal: another client's
/// `--takeover` evicts the frame channel with the focus unchanged, so every
/// read must re-assert the focused pane or the channel never re-attaches.
#[test]
fn the_focused_pane_is_reported_on_every_poll_not_only_when_it_changes() {
    let mut state = PollState::new();
    let focus = |effects: Vec<PollEffect>| -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                PollEffect::FocusedPane(pane) => Some(pane),
                _ => None,
            })
            .collect()
    };
    assert_eq!(focus(state.apply(read(TWO_WORKSPACES))), ["w1:p2"]);
    assert_eq!(focus(state.apply(read(TWO_WORKSPACES))), ["w1:p2"]);
}

// MARK: - Poll cadence

#[test]
fn an_unchanging_tree_eases_the_poll_off_the_fast_cadence() {
    let mut state = PollState::new();
    state.apply(read(TWO_WORKSPACES));
    assert_eq!(state.interval, control::FAST_POLL_INTERVAL);
    for _ in 0..=control::IDLE_POLL_THRESHOLD {
        state.apply(read(TWO_WORKSPACES));
    }
    assert_eq!(state.interval, control::IDLE_POLL_INTERVAL);
}

#[test]
fn any_change_snaps_the_cadence_back_immediately() {
    let mut state = PollState::new();
    for _ in 0..7 {
        state.apply(read(TWO_WORKSPACES));
    }
    assert_eq!(state.interval, control::IDLE_POLL_INTERVAL);
    // Focus moved: that's activity.
    state.apply(read(&TWO_WORKSPACES.replace(
        r#""focused_pane_id":"w1:p2""#,
        r#""focused_pane_id":"w1:p1""#,
    )));
    assert_eq!(state.interval, control::FAST_POLL_INTERVAL);
}

#[test]
fn quicken_drops_back_to_fast_which_the_frame_channels_recovery_rides_on() {
    let mut state = PollState::new();
    for _ in 0..7 {
        state.apply(read(TWO_WORKSPACES));
    }
    state.quicken();
    assert_eq!(state.interval, control::FAST_POLL_INTERVAL);
}

#[test]
fn losing_the_server_also_returns_to_the_fast_cadence() {
    let mut state = PollState::new();
    for _ in 0..7 {
        state.apply(read(TWO_WORKSPACES));
    }
    state.apply(read(&format!(
        "Error: Os {{ code: 2 }}\n{}1",
        control::EXIT_CODE_MARKER
    )));
    assert!(state.server_not_running);
    assert_eq!(state.interval, control::FAST_POLL_INTERVAL);
}

#[test]
fn with_push_up_an_idle_tree_polls_only_as_a_safety_net() {
    let mut state = PollState::new();
    state.set_push_active(true);
    for _ in 0..7 {
        state.apply(read(TWO_WORKSPACES));
    }
    assert_eq!(state.interval, control::PUSHED_POLL_INTERVAL);
    // Push died: back to fast at once.
    state.set_push_active(false);
    assert_eq!(state.interval, control::FAST_POLL_INTERVAL);
    assert_eq!(control::PUSHED_POLL_INTERVAL, Duration::from_secs(45));
}

#[test]
fn every_command_carries_the_exit_status_trailer() {
    assert!(
        control::command_line(control::SNAPSHOT, None)
            .ends_with(&format!("; echo \"{}$?\"", control::EXIT_CODE_MARKER))
    );
}

#[test]
fn exit_status_parsing_splits_the_trailer_off_the_output() {
    let with_code = control::parse_result(&format!("payload\n{}7\n", control::EXIT_CODE_MARKER));
    assert_eq!(with_code.exit_code, Some(7));
    assert!(with_code.failed());
    assert_eq!(with_code.output.trim(), "payload");

    let ok = control::parse_result(&format!("payload\n{}0", control::EXIT_CODE_MARKER));
    assert_eq!(ok.exit_code, Some(0));
    assert!(!ok.failed());

    // No trailer (a shell that died before echoing): unknown, not failed.
    let missing = control::parse_result("payload only");
    assert_eq!(missing.exit_code, None);
    assert!(!missing.failed());

    // The LAST marker counts, as the last thing printed.
    let twice = control::parse_result(&format!("a{0}1\nb{0}2", control::EXIT_CODE_MARKER));
    assert_eq!(twice.exit_code, Some(2));
    assert_eq!(twice.output, format!("a{}1\nb", control::EXIT_CODE_MARKER));
    // A malformed trailer is no trailer: the text stays whole, no status
    // (B193; it read `007z` as 7, and cut the empty one's marker off).
    for raw in [
        "x__moshpit_rc=007z",
        "x__moshpit_rc=",
        "x__moshpit_rc= 0",
        "x__moshpit_rc=0x",
    ] {
        let read = control::parse_result(raw);
        assert_eq!(read.exit_code, None, "{raw}");
        assert_eq!(read.output, raw, "{raw}");
    }
    let spaced = control::parse_result("x__moshpit_rc=007 \r\n");
    assert_eq!((spaced.output.as_str(), spaced.exit_code), ("x", Some(7)));
}

// MARK: - Commands

/// Everything the sheets can trigger, and the herdr CLI call it becomes.
#[test]
fn sheet_actions_map_to_the_right_herdr_subcommands() {
    assert_eq!(control::select_session("w2"), "workspace focus 'w2'");
    assert_eq!(control::select_window("w1:t1"), "tab focus 'w1:t1'");
    assert_eq!(control::kill_session("w2"), "workspace close 'w2'");
    assert_eq!(control::kill_window("w1:t1"), "tab close 'w1:t1'");
    assert_eq!(control::kill_pane("w1:p1"), "pane close 'w1:p1'");
    assert_eq!(
        control::rename_session("w1", "api"),
        "workspace rename 'w1' 'api'"
    );
    assert_eq!(
        control::rename_window("w1:t1", "logs"),
        "tab rename 'w1:t1' 'logs'"
    );
    assert_eq!(
        control::new_session(Some("build")),
        "workspace create --focus --label 'build'"
    );
    assert_eq!(control::new_session(Some("")), "workspace create --focus");
    assert_eq!(control::new_window(None), "tab create --focus");
    // Splits the FOCUSED pane, taken from the snapshot.
    assert_eq!(
        control::new_pane(&tree()).as_deref(),
        Some("pane split 'w1:p2' --direction right --focus")
    );
}

#[test]
fn focusing_a_pane_tries_agent_focus_with_the_zoom_bounce_fallback_in_the_same_exec() {
    let sent = control::select_pane("w1:p1", None);
    assert!(sent.contains("agent focus 'w1:p1' 2>/dev/null || {"));
    assert!(sent.contains("pane zoom --pane 'w1:p1' --on"));
    assert!(sent.contains("pane zoom --pane 'w1:p1' --off"));
    // --on before --off, or the bounce un-zooms panes that were zoomed.
    assert!(sent.find("--on").unwrap() < sent.find("--off").unwrap());
}

#[test]
fn arrow_keys_ride_pane_send_keys_and_nothing_else() {
    assert_eq!(
        control::send_keys(&["up"], "w1:p1").as_deref(),
        Some("pane send-keys 'w1:p1' 'up'")
    );
    assert_eq!(
        control::send_keys(&["up", "down", "up"], "w1:p1").as_deref(),
        Some("pane send-keys 'w1:p1' 'up' 'down' 'up'")
    );
    assert_eq!(control::send_keys(&[], "w1:p1"), None);
}

#[test]
fn immersive_mode_picking_a_pane_zooms_it_focus_and_fill_in_one_command() {
    let sent = control::immersive_zoom("w1:p1");
    assert_eq!(sent, "pane zoom --pane 'w1:p1' --on 2>/dev/null || true");
    assert!(!sent.contains("agent focus"));
}

#[test]
fn immersive_mode_a_tab_switch_zooms_whatever_pane_the_next_snapshot_names() {
    let mut state = PollState::new();
    state.immersive_zoom = true;
    state.zoom_after_next_tree();
    let effects = state.apply(read(TWO_WORKSPACES));
    // The fixture's focused pane is w1:p2: the zoom follows the snapshot.
    assert!(effects.contains(&PollEffect::Zoom("w1:p2".to_owned())));
    // Once.
    assert!(
        !state
            .apply(read(TWO_WORKSPACES))
            .iter()
            .any(|e| matches!(e, PollEffect::Zoom(_)))
    );
}

#[test]
fn without_immersive_mode_zooms_only_ever_come_as_a_bounce() {
    let mut state = PollState::new();
    state.zoom_after_next_tree();
    let effects = state.apply(read(TWO_WORKSPACES));
    assert!(!effects.iter().any(|e| matches!(e, PollEffect::Zoom(_))));
    // The focus fallback's bounce carries its --off in the same exec.
    let bounce = control::select_pane("w1:p1", None);
    assert!(!bounce.contains("--on 2>/dev/null || true"));
    assert!(bounce.contains("--off"));
}

#[test]
fn labels_from_the_server_are_shell_quoted_not_interpolated_raw() {
    let sent = control::command_line(&control::rename_session("w1", "it's; rm -rf ~"), None);
    let quoted = r"'it'\''s; rm -rf ~'";
    assert!(sent.contains(&format!("workspace rename 'w1' {quoted}")));
    // The only thing after the label is the exit-status trailer.
    let after = sent.split(quoted).last().unwrap();
    assert!(after.starts_with(&format!("; echo \"{}", control::EXIT_CODE_MARKER)));
}

#[test]
fn new_pane_with_no_known_panes_sends_nothing_rather_than_a_bad_target() {
    assert_eq!(control::new_pane(&snapshot::Snapshot::default()), None);
}

#[test]
fn new_tab_and_pane_from_home_focus_their_workspace_and_tab_first() {
    let tree = tree();
    assert_eq!(
        control::new_window_in_session(&tree, "w2", None),
        ["workspace focus 'w2'", "tab create --focus"]
    );
    assert_eq!(
        control::new_window_in_session(&tree, "w1", Some("x")),
        ["tab create --focus --label 'x'"]
    );
    assert_eq!(
        control::new_pane_in_window(&tree, "w1:t1").unwrap(),
        ["pane split 'w1:p2' --direction right --focus"]
    );
    assert_eq!(control::new_pane_in_window(&tree, "w9:t9"), None);
}

// MARK: - Agent tasks

fn task(prompt: &str) -> AgentTaskRequest {
    AgentTaskRequest {
        repo_path: "/Users/dev/code/moshpit".to_owned(),
        branch: "fix-scroll".to_owned(),
        agent: "claude".to_owned(),
        prompt: prompt.to_owned(),
    }
}

/// The chain: the worktree, then the agent typed into its own pane (`agent
/// start` would land in the CLI's own directory).
#[test]
fn a_task_creates_the_worktree_then_types_the_agent_into_its_own_pane() {
    let create = control::worktree_create(&task(""));
    assert!(create.contains("worktree create --cwd '/Users/dev/code/moshpit'"));
    assert!(create.contains("--branch 'fix-scroll'") && create.contains("--focus"));
    let pane = control::worktree_created(Some(&exited(0, WORKTREE_CREATED)), "fix-scroll").unwrap();
    assert_eq!(pane, "w4:p1");
    assert_eq!(
        control::pane_run(&pane, "claude"),
        "pane run 'w4:p1' 'claude'"
    );
    let launched = exited(0, r#"{"id":"cli:pane:run","result":{"type":"ok"}}"#);
    assert_eq!(control::agent_launched(Some(&launched), "claude"), None);
}

#[test]
fn invalid_input_never_reaches_the_host() {
    let mut bad = task("");
    bad.branch = "two words".to_owned();
    assert_eq!(bad.validation_error(), Some(TaskInvalid::BranchSpace));
}

/// herdr's own words for what goes wrong (branch exists, dirty repo).
#[test]
fn herdrs_own_error_text_is_what_the_user_sees() {
    let refused = exited(
        1,
        r#"{"error":{"code":"worktree_create_failed","message":"branch 'fix-scroll' already exists"},"id":"cli:worktree:create"}"#,
    );
    assert_eq!(
        control::worktree_created(Some(&refused), "fix-scroll"),
        Err(TaskOutcome::Failed(TaskFailure::Herdr(
            "branch 'fix-scroll' already exists".to_owned()
        )))
    );
}

/// B72: a create that exits 0 with nothing to read, or whose exit status
/// never came back, named no pane: nothing was started in it.
#[test]
fn a_worktree_create_with_no_answer_to_read_is_a_failure_not_a_started_task() {
    for answer in [
        exited(0, ""),
        exited(0, "Created worktree fix-scroll"),
        control::parse_result(""),
    ] {
        let outcome = control::worktree_created(Some(&answer), "fix-scroll").unwrap_err();
        let TaskOutcome::Failed(TaskFailure::Unconfirmed { branch }) = &outcome else {
            panic!("{answer:?} was read as {outcome:?}");
        };
        assert_eq!(branch, "fix-scroll");
        assert!(outcome.english().unwrap().contains("“fix-scroll”"));
    }
    assert_eq!(
        control::worktree_created(Some(&exited(1, "")), "x"),
        Err(TaskOutcome::Failed(TaskFailure::CreateFailed))
    );
    assert_eq!(
        control::worktree_created(None, "x"),
        Err(TaskOutcome::Failed(TaskFailure::Unreachable))
    );
    assert_eq!(
        control::worktree_created(Some(&exited(0, r#"{"result":{"type":"x"}}"#)), "x"),
        Err(TaskOutcome::Incomplete(TaskIncomplete::NoPane))
    );
}

// MARK: The first message

/// herdr 0.7.5 replaced `agent send` with `agent prompt` (B58).
#[test]
fn the_first_message_goes_the_way_the_hosts_herdr_takes_it() {
    for (version, verb) in [
        ("0.7.3", "agent send"),
        ("0.7.4", "agent send"),
        ("0.7.5", "agent prompt"),
        ("0.8.0", "agent prompt"),
        ("0.9.5", "agent prompt"),
    ] {
        assert_eq!(
            control::first_message("fix the scroll", "w4:p1", Some(version)),
            format!("{verb} 'w4:p1' 'fix the scroll'"),
            "herdr {version}"
        );
    }
    // Unreadable means today's herdr.
    assert_eq!(
        control::first_message("x", "p", None),
        "agent prompt 'p' 'x'"
    );
}

/// B74: before 0.7.5 `agent send` types and stops; Return is ours to press.
#[test]
fn on_herdr_before_0_7_5_the_first_message_is_typed_then_return_is_pressed() {
    let delivered = exited(0, r#"{"id":"cli:agent:send","result":{"type":"ok"}}"#);
    for version in ["0.7.3", "0.7.4"] {
        assert_eq!(
            control::after_first_message(Some(&delivered), "claude", false, Some(version)),
            FirstMessageStep::Submit
        );
    }
    assert_eq!(control::submit("w4:p1"), "pane send-keys 'w4:p1' 'Enter'");
    // herdr prints nothing for a key it pressed.
    assert_eq!(
        control::after_submit(Some(&exited(0, "")), "claude"),
        FirstMessageStep::Started
    );
}

/// `agent prompt` presses Enter itself; a second one would land in the
/// agent's composer.
#[test]
fn on_current_herdr_nothing_presses_return_after_agent_prompt() {
    let prompted = exited(
        0,
        r#"{"id":"cli:agent:prompt","result":{"type":"agent_prompted","agent":{"terminal_id":"t4","agent":"claude","agent_status":"working"}}}"#,
    );
    for version in ["0.7.5", "0.9.5"] {
        assert_eq!(
            control::after_first_message(Some(&prompted), "claude", false, Some(version)),
            FirstMessageStep::Started
        );
    }
}

#[test]
fn a_first_message_typed_but_never_sent_says_so_and_how_to_send_it() {
    let refused = exited(
        1,
        r#"{"id":"cli:request","error":{"code":"pane_not_found","message":"pane w4:p1 not found"}}"#,
    );
    let FirstMessageStep::Incomplete(incomplete) = control::after_submit(Some(&refused), "claude")
    else {
        panic!("a first message left unsent was reported as sent");
    };
    let text = TaskOutcome::Incomplete(incomplete).english().unwrap();
    assert!(text.contains("pane w4:p1 not found"));
    assert!(text.contains("Press Return in the pane"));
}

/// An agent waiting at a question is turned away with `agent_blocked`, and
/// asking again can't answer it.
#[test]
fn a_first_message_herdr_turns_away_is_said_in_herdrs_words() {
    let refusal = exited(
        1,
        r#"{"id":"cli:agent:prompt","error":{"code":"agent_blocked","message":"agent w4:p1 is blocked and requires interactive input"}}"#,
    );
    let step = control::after_first_message(Some(&refusal), "claude", false, Some("0.9.5"));
    let FirstMessageStep::Incomplete(TaskIncomplete::MessageRefused { reason, .. }) = step else {
        panic!("a refused first message was reported as {step:?}");
    };
    assert_eq!(
        reason,
        Refusal::Said("agent w4:p1 is blocked and requires interactive input".to_owned())
    );
}

/// `agent prompt` only takes a pane its agent was detected in; detection
/// trails `pane run`.
#[test]
fn the_first_message_waits_for_herdr_to_see_the_agent_then_goes() {
    let not_yet = exited(
        1,
        r#"{"id":"cli:agent:prompt","error":{"code":"agent_not_found","message":"agent target w4:p1 not found"}}"#,
    );
    assert!(control::agent_not_up_yet(&not_yet));
    assert_eq!(
        control::after_first_message(Some(&not_yet), "claude", false, Some("0.9.5")),
        FirstMessageStep::Retry
    );
}

#[test]
fn an_agent_that_never_comes_up_is_reported_not_waited_on_for_good() {
    let not_ready = exited(
        1,
        r#"{"id":"cli:agent:prompt","error":{"code":"agent_not_ready","message":"agent w4:p1 is not an active named agent"}}"#,
    );
    assert_eq!(
        control::after_first_message(Some(&not_ready), "claude", false, None),
        FirstMessageStep::Retry
    );
    assert_eq!(
        control::after_first_message(Some(&not_ready), "claude", true, None),
        FirstMessageStep::Incomplete(TaskIncomplete::AgentNotReady {
            agent: "claude".to_owned()
        })
    );
}

#[test]
fn a_host_that_stops_answering_is_said_not_waited_on() {
    assert_eq!(
        control::after_first_message(None, "claude", false, None),
        FirstMessageStep::Incomplete(TaskIncomplete::HostStoppedAnswering)
    );
}

/// The agent's own launch can fail too.
#[test]
fn an_agent_that_didnt_start_is_said() {
    let refused = exited(
        1,
        r#"{"id":"cli:pane:run","error":{"code":"pane_not_found","message":"pane w4:p1 not found"}}"#,
    );
    let outcome = control::agent_launched(Some(&refused), "claude").unwrap();
    assert_eq!(
        outcome.english().unwrap(),
        "The worktree is ready, but claude didn't start: pane w4:p1 not found"
    );
}

// MARK: - refusal and friends

#[test]
fn a_refusal_is_herdrs_error_text_else_the_first_printed_line_else_the_status() {
    assert_eq!(control::refusal(None), Some(Refusal::Unreachable));
    assert_eq!(
        control::refusal(Some(&exited(1, r#"{"error":{"code":"only_code"}}"#))),
        Some(Refusal::Said("only_code".to_owned()))
    );
    // A JSON error counts even with exit 0.
    assert_eq!(
        control::refusal(Some(&exited(0, r#"{"error":{"message":"m"}}"#))),
        Some(Refusal::Said("m".to_owned()))
    );
    assert_eq!(
        control::refusal(Some(&exited(2, "   \n  first printed line  \nsecond"))),
        Some(Refusal::Said("first printed line".to_owned()))
    );
    assert_eq!(
        control::refusal(Some(&exited(3, ""))),
        Some(Refusal::ExitStatus(3))
    );
    assert_eq!(
        Refusal::ExitStatus(3).english(),
        "herdr exited with status 3"
    );
    assert_eq!(control::refusal(Some(&exited(0, ""))), None);
    // A long line is cut at 200 characters.
    let long = "x".repeat(300);
    assert_eq!(
        control::refusal(Some(&exited(1, &long))),
        Some(Refusal::Said("x".repeat(200)))
    );
}

/// B90: a result whose exit status never came back is not a success unless
/// herdr printed its `result` (the app's rule since round eighteen). Before
/// that, a `pane run` or `agent prompt` the channel dropped mid-flight was
/// reported done.
/// B193: the client reads a command's status off the trailer only when the
/// trailer is the last thing printed (`client::read_result`, which is
/// `control::parse_result`, the app's rule too since round 36); taking the
/// marker anywhere read an answer cut off inside text that holds the marker
/// as a status.
#[test]
fn the_client_reads_the_trailer_only_at_the_end_of_the_output() {
    use moshpit_herdr::client::read_result;
    let marker = control::EXIT_CODE_MARKER;
    let cut = format!(r#"{{"result":{{"label":"{marker}0 build"#);
    assert_eq!(control::parse_result(&cut).exit_code, None);
    assert_eq!(read_result(cut.as_bytes()).exit_code, None);
    assert_eq!(read_result(cut.as_bytes()).output, cut);
    let title = format!("{{\"title\":\"{marker}7\"}}");
    assert_eq!(read_result(title.as_bytes()).exit_code, None);
    // The real trailer, behind text that holds the marker too.
    let whole = format!("{cut}\"}}}}}}\n{marker}3\r\n");
    let read = read_result(whole.as_bytes());
    assert_eq!(read.exit_code, Some(3));
    assert_eq!(read.output, format!("{cut}\"}}}}}}\n"));
    assert_eq!(
        read_result(format!("ok\n{marker}0").as_bytes()).exit_code,
        Some(0)
    );
    assert_eq!(
        read_result(format!("x{marker}007z").as_bytes()).exit_code,
        None
    );
}

#[test]
fn a_result_without_an_exit_status_counts_only_when_herdr_answered() {
    let lost = CommandResult {
        output: String::new(),
        exit_code: None,
    };
    assert_eq!(control::refusal(Some(&lost)), Some(Refusal::CutOff));
    assert_eq!(Refusal::CutOff.english(), "herdr's answer was cut off");
    // Printed text without the trailer may be half an answer: cut off too.
    let printed = CommandResult {
        output: "bash: herdr: command not found\n".to_owned(),
        exit_code: None,
    };
    assert_eq!(control::refusal(Some(&printed)), Some(Refusal::CutOff));
    // A JSON object that isn't herdr's result is no evidence either.
    let other = CommandResult {
        output: r#"{"type":"ok"}"#.to_owned(),
        exit_code: None,
    };
    assert_eq!(control::refusal(Some(&other)), Some(Refusal::CutOff));
    let answered = CommandResult {
        output: r#"{"id":"cli:pane:run","result":{"type":"ok"}}"#.to_owned(),
        exit_code: None,
    };
    assert_eq!(control::refusal(Some(&answered)), None);
    // The first message then says the host stopped answering, rather than
    // closing the form as if it were sent.
    assert!(matches!(
        control::after_first_message(Some(&lost), "claude", false, None),
        FirstMessageStep::Incomplete(TaskIncomplete::MessageRefused { .. })
    ));
}

// MARK: - Repositories and agents

#[test]
fn repo_discovery_resolves_the_panes_repos_and_merges_the_scan_after() {
    let resolve = control::pane_repos(&[
        "/Users/dev/code/other",
        "/Users/dev/code/moshpit/Moshpit",
        "/Users/dev/code/other",
    ])
    .unwrap();
    assert!(
        resolve
            .starts_with("for d in '/Users/dev/code/moshpit/Moshpit' '/Users/dev/code/other'; do")
    );
    assert!(resolve.contains("rev-parse --show-toplevel"));
    assert_eq!(control::pane_repos(&[]), None);
    // Follows symlinks; prunes hidden dirs and the media libraries.
    assert!(control::HOME_SCAN.starts_with("find -L"));
    assert!(control::HOME_SCAN.contains("! -name .git -prune"));
    assert!(control::HOME_SCAN.contains("-name Library"));
    assert!(control::HOME_SCAN.contains("-name Pictures"));
    let from_panes = control::repo_lines("/Users/dev/code/moshpit\n/Users/dev/code/other\n");
    let scanned = control::repo_lines("/Users/dev/code/scanned\n/Users/dev/code/moshpit\n");
    assert_eq!(
        control::merge_repos(&from_panes, &scanned),
        [
            "/Users/dev/code/moshpit",
            "/Users/dev/code/other",
            "/Users/dev/code/scanned"
        ]
    );
}

#[test]
fn repo_lines_keep_absolute_paths_once_and_drop_the_noise() {
    assert_eq!(
        control::repo_lines("noise\n  /a  \n/a\r\ngit: fatal\n/b"),
        ["/a", "/b"]
    );
}

#[test]
fn a_dead_link_on_the_pane_resolve_falls_back_to_the_raw_cwd_guesses() {
    assert_eq!(
        control::open_repo_guesses(&[
            "/Users/dev/code/other",
            "/Users/dev/code/moshpit/Moshpit",
            "/Users/dev/code/other"
        ]),
        ["/Users/dev/code/moshpit/Moshpit", "/Users/dev/code/other"]
    );
}

#[test]
fn agent_names_come_from_herdrs_own_manifest_list() {
    assert_eq!(
        control::agent_names(
            r#"[{"agent":"pi","active_version":"1"},{"agent":"claude","active_version":"2"},{"agent":"codex"}]"#
        ),
        ["claude", "codex", "pi"]
    );
    assert!(control::agent_names("").is_empty());
    assert!(control::agent_names("noise [1,2] tail").is_empty());
}

/// Fix (manifest-range-trap): a `]` before the first `[` crashed the app
/// (`raw[start...end]` with end < start).
#[test]
fn a_closing_bracket_before_the_opening_one_is_no_list_not_a_crash() {
    assert!(control::agent_names("a]b[").is_empty());
    assert!(control::agent_names("] [{\"agent\":\"claude\"}").is_empty());
}

// MARK: - Worktree removal

#[test]
fn a_dirty_checkout_comes_back_as_needs_force_never_silently_forced() {
    let dirty = exited(
        1,
        r#"{"error":{"code":"dirty_worktree_requires_force","message":"fatal: contains modified or untracked files, use --force to delete it"},"id":"cli:worktree:remove"}"#,
    );
    assert_eq!(
        control::worktree_removal(Some(&dirty)),
        WorktreeRemoval::NeedsForce
    );
    let command = control::worktree_remove("w4", false);
    assert!(command.contains("worktree remove --workspace 'w4'"));
    assert!(!command.contains("--force"));
}

#[test]
fn forcing_is_explicit_and_only_happens_when_asked_for() {
    assert!(control::worktree_remove("w4", true).contains("--force"));
    let removed = exited(
        0,
        r#"{"id":"cli:worktree:remove","result":{"type":"worktree_removed","forced":true,"workspace_id":"w4"}}"#,
    );
    assert_eq!(
        control::worktree_removal(Some(&removed)),
        WorktreeRemoval::Removed
    );
}

#[test]
fn any_other_failure_carries_herdrs_own_message() {
    let missing = exited(
        1,
        r#"{"error":{"code":"not_found","message":"workspace w9 not found"},"id":"cli:worktree:remove"}"#,
    );
    assert_eq!(
        control::worktree_removal(Some(&missing)),
        WorktreeRemoval::Failed(RemovalFailure::Herdr("workspace w9 not found".to_owned()))
    );
    assert_eq!(
        control::worktree_removal(Some(&exited(1, "boom"))),
        WorktreeRemoval::Failed(RemovalFailure::Failed)
    );
    assert_eq!(
        control::worktree_removal(None),
        WorktreeRemoval::Failed(RemovalFailure::Unreachable)
    );
}

// MARK: - Version skew

const STATUS_OLD_CLIENT: &str = "client:
  version: 0.8.0
  channel: stable
  protocol: 19

server:
  status: running
  version: 0.9.5
  protocol: 22
  compatible: no
  socket: /Users/u/.config/herdr/herdr.sock

__moshpit_rc=0";

const STATUS_OLD_SERVER: &str = "client:
  version: 0.9.5
  channel: stable
  protocol: 22

server:
  status: running
  version: 0.8.0
  endpoint_compatible: no

__moshpit_rc=0";

fn report(raw: &str) -> Option<VersionReport> {
    VersionReport::parse(&control::parse_result(raw).output)
}

#[test]
fn restart_refuses_when_the_herdr_on_path_is_the_older_side() {
    let report = report(STATUS_OLD_CLIENT);
    assert!(PollState::restart_would_downgrade(report.as_ref()));
    assert_eq!(
        report,
        Some(VersionReport {
            client: "0.8.0".to_owned(),
            server: "0.9.5".to_owned()
        })
    );
}

#[test]
fn restart_still_runs_when_the_server_is_the_older_side() {
    assert!(!PollState::restart_would_downgrade(
        report(STATUS_OLD_SERVER).as_ref()
    ));
}

#[test]
fn an_unreadable_status_doesnt_block_the_restart() {
    let unreadable = report("error: unknown command 'status'\n__moshpit_rc=2");
    assert_eq!(unreadable, None);
    assert!(!PollState::restart_would_downgrade(unreadable.as_ref()));
}

#[test]
fn a_protocol_mismatch_asks_herdr_status_once_per_episode_and_keeps_the_versions() {
    let mismatch = format!(
        "{}\n__moshpit_rc=0",
        r#"{"id":"cli:api:snapshot","error":{"code":"protocol_mismatch","message":"client protocol 19 is older than server protocol 22; upgrade the Herdr client before using this command"}}"#
    );
    let mut state = PollState::new();
    let effects = state.apply(read(&mismatch));
    assert_eq!(
        effects.first(),
        Some(&PollEffect::DiagnoseSkew),
        "diagnose before reporting"
    );
    assert!(
        state
            .protocol_mismatch
            .as_deref()
            .unwrap()
            .contains("older than server")
    );
    state.note_versions(report(STATUS_OLD_CLIENT));
    assert_eq!(
        state.version_skew,
        Some(VersionReport {
            client: "0.8.0".to_owned(),
            server: "0.9.5".to_owned()
        })
    );
    assert_eq!(state.interval, control::IDLE_POLL_INTERVAL);
    assert!(state.last_read_mismatched());
    // Once per episode.
    assert!(
        !state
            .apply(read(&mismatch))
            .contains(&PollEffect::DiagnoseSkew)
    );
    // Healed: the skew is forgotten with it.
    state.apply(read(TWO_WORKSPACES));
    assert_eq!(state.protocol_mismatch, None);
    assert_eq!(state.version_skew, None);
}

// MARK: - Agent task request (AgentTaskRequestTests)

fn valid() -> AgentTaskRequest {
    AgentTaskRequest {
        repo_path: "/Users/dev/code/moshpit".to_owned(),
        branch: "fix-scroll-jump".to_owned(),
        agent: "claude".to_owned(),
        prompt: String::new(),
    }
}

#[test]
fn a_complete_request_passes() {
    assert_eq!(valid().validation_error(), None);
}

#[test]
fn repo_and_agent_are_both_required() {
    let mut no_repo = valid();
    no_repo.repo_path = "  ".to_owned();
    assert_eq!(
        no_repo.validation_error().map(TaskInvalid::english),
        Some("Pick a repository")
    );
    let mut no_agent = valid();
    no_agent.agent = String::new();
    assert_eq!(
        no_agent.validation_error().map(TaskInvalid::english),
        Some("Pick an agent")
    );
}

#[test]
fn branch_names_git_would_reject_are_caught_here() {
    for branch in [
        "",
        "  ",
        "two words",
        "-leading-dash",
        "/leading-slash",
        "trailing-slash/",
        "double..dot",
        "ends.lock",
        "star*",
        "question?",
        "colon:",
        "tilde~",
        "caret^",
        "bracket[",
        "back\\slash",
        "del\u{7f}",
        "tab\there",
    ] {
        let mut request = valid();
        request.branch = branch.to_owned();
        assert!(
            request.validation_error().is_some(),
            "{branch:?} should be rejected"
        );
    }
}

#[test]
fn branch_names_git_accepts_pass_through() {
    for branch in [
        "fix-scroll-jump",
        "feature/thing",
        "v2",
        "a_b.c",
        "用户-分支",
    ] {
        let mut request = valid();
        request.branch = branch.to_owned();
        assert_eq!(
            request.validation_error(),
            None,
            "{branch:?} should be accepted"
        );
    }
}

#[test]
fn surrounding_whitespace_doesnt_make_a_name_invalid() {
    let mut request = valid();
    request.branch = "  fix-scroll  ".to_owned();
    assert_eq!(request.validation_error(), None);
    assert_eq!(request.branch(), "fix-scroll");
    assert!(
        control::worktree_create(&request).contains("--branch 'fix-scroll' --label 'fix-scroll'")
    );
}
