//! Decoding `herdr api snapshot`, and `herdr status`. Ported from
//! `Tests/MoshpitTests/Services/Herdr/HerdrSnapshotTests.swift` (both
//! suites). The main fixture is a REAL capture from herdr 0.7.3 (protocol
//! 16) driven by Moshpit itself: two workspaces, the first with two tabs, the
//! first tab split into two panes.

use std::cmp::Ordering;

use moshpit_herdr::snapshot::{self, AgentState};
use moshpit_herdr::version::{self, VersionNotice, VersionReport};

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/app/herdr-snapshot.json"
    ))
    .expect("the app's herdr-snapshot.json capture")
}

fn ids<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    items.into_iter().collect()
}

// MARK: - The real payload

#[test]
fn real_0_7_3_capture_decodes_into_the_apps_three_levels() {
    let decoded = snapshot::decode(&fixture()).expect("decodes");
    let snap = &decoded.snapshot;

    // workspace → session
    assert_eq!(snap.sessions.len(), 2);
    assert_eq!(snap.sessions["w1"].name, "~");
    assert_eq!(snap.sessions["w2"].name, "api");
    assert!(snap.sessions["w1"].is_attached);
    assert!(!snap.sessions["w2"].is_attached);

    // tab → window, grouped under its workspace
    assert_eq!(snap.windows.len(), 3);
    assert_eq!(
        ids(snap.windows_in_session("w1").iter().map(|w| w.id.as_str())),
        ["w1:t1", "w1:t2"]
    );
    assert_eq!(snap.windows["w1:t2"].name, "logs");
    assert_eq!(snap.windows["w1:t2"].index, 2);
    assert_eq!(snap.windows["w1:t1"].pane_count, 2);

    // pane
    assert_eq!(snap.panes.len(), 4);
    assert_eq!(
        ids(snap.panes_in_window("w1:t1").iter().map(|p| p.id.as_str())),
        ["w1:p1", "w1:p3"]
    );
    assert_eq!(snap.active_pane_id.as_deref(), Some("w1:p3"));
    assert_eq!(snap.active_window_id.as_deref(), Some("w1:t1"));
    assert_eq!(snap.active_session_id.as_deref(), Some("w1"));
    assert!(snap.panes["w1:p3"].is_active);
}

#[test]
fn pane_geometry_comes_from_the_layouts_array_not_the_pane_objects() {
    let decoded = snapshot::decode(&fixture()).unwrap();
    // The split tab: two panes side by side, each half of the 44-col area.
    assert_eq!(decoded.snapshot.panes["w1:p1"].width, 22);
    assert_eq!(decoded.snapshot.panes["w1:p3"].width, 22);
    assert_eq!(decoded.snapshot.panes["w1:p1"].height, 32);
}

#[test]
fn a_snapshot_naming_a_workspace_counts_as_attached() {
    let decoded = snapshot::decode(&fixture()).unwrap();
    assert!(decoded.snapshot.is_attached);
    assert!(decoded.snapshot.ever_attached);
}

// MARK: - Index derivation

#[test]
fn pane_index_comes_from_the_id_so_poll_order_cant_reshuffle_panes() {
    for (id, expected) in [("w1:p1", 1), ("w1:p3", 3), ("w12:p47", 47), ("nonsense", 0)] {
        assert_eq!(snapshot::pane_index(id), expected, "{id}");
    }
    // `Int(_:)`'s rules: a sign is fine, anything else after the digits is not.
    assert_eq!(snapshot::pane_index("w1:p+5"), 5);
    assert_eq!(snapshot::pane_index("w1:p-5"), -5);
    assert_eq!(snapshot::pane_index("w1:p2:p3"), 0);
    assert_eq!(snapshot::pane_index("w1:p99999999999999999999"), 0);
}

// MARK: - Version tolerance

#[test]
fn newer_fields_decode_and_older_payloads_dont_break() {
    // 0.7.3 omits every one of these keys; this is the same payload with the
    // 0.8.0 additions present.
    let raw = r#"{"id":"x","result":{"snapshot":{
      "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
      "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],
      "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true,
                "agent_status":"working","agent":"claude","display_agent":"Claude Code",
                "terminal_title_stripped":"claude — moshpit"}],
      "layouts":[],
      "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}"#;
    let decoded = snapshot::decode(raw).unwrap();
    assert_eq!(decoded.snapshot.panes["w1:p1"].command, "Claude Code");
    let hook = &decoded.agent_hooks["w1:p1"];
    assert_eq!(hook.state, Some(AgentState::Working));
    assert_eq!(hook.agent.as_deref(), Some("Claude Code"));
    assert_eq!(hook.title.as_deref(), Some("claude — moshpit"));
    // No geometry reported → the model's own defaults, not a crash.
    assert_eq!(decoded.snapshot.panes["w1:p1"].width, 80);
}

#[test]
fn a_bare_snapshot_without_a_response_envelope_still_decodes() {
    let raw = r#"{"workspaces":[{"workspace_id":"w1","label":"solo","focused":true}],
     "tabs":[],"panes":[],"layouts":[]}"#;
    assert_eq!(
        snapshot::decode(raw).unwrap().snapshot.sessions["w1"].name,
        "solo"
    );
}

// MARK: - Agent status → the app's stamp vocabulary

fn pane_with(status: &str) -> String {
    format!(
        r#"{{"result":{{"snapshot":{{
      "workspaces":[{{"workspace_id":"w1","label":"~","focused":true}}],
      "tabs":[{{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}}],
      "panes":[{{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"{status}"}}],
      "layouts":[]}}}}}}"#
    )
}

#[test]
fn agent_status_maps_onto_the_stamps_the_island_and_sheets_already_speak() {
    for (status, stamp) in [
        ("working", "working"),
        ("blocked", "attention"),
        ("done", "done"),
    ] {
        let decoded = snapshot::decode(&pane_with(status)).unwrap();
        assert_eq!(
            decoded.agent_hooks["w1:p1"].state.map(AgentState::as_str),
            Some(stamp)
        );
    }
}

#[test]
fn unknown_produces_no_stamp_so_nothing_lights_up_without_cause() {
    let decoded = snapshot::decode(&pane_with("unknown")).unwrap();
    assert_eq!(
        decoded.agent_hooks.get("w1:p1").and_then(|hook| hook.state),
        None
    );
}

/// `idle` passes through by its own name, so the Agents section can show a
/// named idle agent as a quiet row. (That it never lights anything is
/// `AgentSignal`'s rule, in the app's UI layer.)
#[test]
fn idle_passes_through_by_name() {
    let decoded = snapshot::decode(&pane_with("idle")).unwrap();
    assert_eq!(decoded.agent_hooks["w1:p1"].state, Some(AgentState::Idle));
}

// MARK: - Junk in, no crash out

#[test]
fn shell_noise_before_the_json_is_skipped_not_fatal() {
    let raw = "Welcome to Ubuntu 24.04\n{\"result\":{\"snapshot\":{\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"~\"}],\"tabs\":[],\"panes\":[],\"layouts\":[]}}}";
    assert!(snapshot::decode(raw).is_some());
}

#[test]
fn a_brace_inside_a_label_cant_confuse_the_object_scanner() {
    let raw = r#"{"result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"weird{name}"}],
     "tabs":[],"panes":[],"layouts":[]}}}"#;
    assert_eq!(
        snapshot::decode(raw).unwrap().snapshot.sessions["w1"].name,
        "weird{name}"
    );
}

#[test]
fn payloads_that_arent_snapshots_decode_to_none_rather_than_an_empty_tree() {
    for raw in ["", "not json at all", "{}", "{\"result\":{}}"] {
        assert!(snapshot::decode(raw).is_none(), "{raw:?}");
    }
}

/// Shape verbatim from `herdr api snapshot` against a real 0.8.0 server:
/// the pane has agent_status but no agent; identity moved to `agents`.
#[test]
fn the_0_8_agents_array_supplies_the_identity_the_pane_no_longer_carries() {
    let raw = r#"{"result":{"snapshot":{
     "workspaces":[{"workspace_id":"wP","label":"sample-app","focused":true}],
     "tabs":[{"tab_id":"wP:t1","workspace_id":"wP","label":"claude","number":1,"focused":true}],
     "panes":[{"pane_id":"wP:p1","tab_id":"wP:t1","workspace_id":"wP","focused":true,
               "agent_status":"idle","cwd":"/Users/u/code/sample-app",
               "terminal_title_stripped":"claude --resume 7d04"}],
     "layouts":[],
     "agents":[{"agent":"claude","agent_status":"idle","pane_id":"wP:p1",
                "workspace_id":"wP","tab_id":"wP:t1","focused":true}],
     "focused_workspace_id":"wP","focused_tab_id":"wP:t1","focused_pane_id":"wP:p1"}}}"#;
    let decoded = snapshot::decode(raw).unwrap();
    let hook = &decoded.agent_hooks["wP:p1"];
    assert_eq!(hook.agent.as_deref(), Some("claude"));
    assert_eq!(hook.state, Some(AgentState::Idle));
    assert_eq!(decoded.pane_cwds["wP:p1"], "/Users/u/code/sample-app");
    // And a 0.7-style pane that carries its own agent field keeps working
    // without any agents array at all.
    let old = r#"{"result":{"snapshot":{
     "workspaces":[{"workspace_id":"w1","label":"x"}],
     "tabs":[],"layouts":[],
     "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","agent":"codex","agent_status":"working"}]}}}"#;
    let legacy = snapshot::decode(old).unwrap();
    assert_eq!(legacy.agent_hooks["w1:p1"].agent.as_deref(), Some("codex"));
    assert_eq!(legacy.agent_hooks["w1:p1"].state, Some(AgentState::Working));
}

#[test]
fn server_not_running_is_recognized_as_its_own_state() {
    let raw = r#"{"error":{"code":"server_not_running","message":"no herdr server is running at /x/herdr.sock"},"id":"cli:api:snapshot"}"#;
    assert!(snapshot::decode(raw).is_none());
    assert!(snapshot::is_server_not_running(raw));
    // A different error must NOT read as "no server".
    assert!(!snapshot::is_server_not_running(
        r#"{"error":{"code":"not_found"}}"#
    ));
}

#[test]
fn a_protocol_mismatch_keeps_its_first_sentence() {
    let raw = r#"{"id":"cli:api:snapshot","error":{"code":"protocol_mismatch","message":"client protocol 19 is older than server protocol 22; upgrade the Herdr client before using this command"}}"#;
    assert_eq!(
        snapshot::protocol_mismatch(raw).as_deref(),
        Some("client protocol 19 is older than server protocol 22")
    );
    assert_eq!(
        snapshot::protocol_mismatch(r#"{"error":{"code":"protocol_mismatch"}}"#).as_deref(),
        Some("herdr client/server protocol mismatch")
    );
    assert_eq!(
        snapshot::protocol_mismatch(raw.replace("protocol_mismatch", "x").as_str()),
        None
    );
}

#[test]
fn version_skew_is_recognized_in_both_wire_shapes() {
    assert!(snapshot::looks_like_version_skew(
        "client protocol 19 is newer than server protocol 16"
    ));
    assert!(snapshot::looks_like_version_skew(
        "server rejected handshake (version 16): …"
    ));
    assert!(!snapshot::looks_like_version_skew("pane w1:p1 not found"));
}

// MARK: - Version skew direction

#[test]
fn a_mismatch_message_says_which_side_is_older() {
    assert!(snapshot::client_is_older(
        "client protocol 19 is older than server protocol 22"
    ));
    assert!(!snapshot::client_is_older(
        "client protocol 19 is newer than server protocol 16"
    ));
    // The attach CLI's refusal names no direction: not "older".
    assert!(!snapshot::client_is_older(
        "server rejected handshake (version 16): …"
    ));
}

#[test]
fn a_workspace_made_from_a_repo_is_a_worktree_and_others_are_not() {
    let raw = r#"{"result":{"snapshot":{
      "workspaces":[
        {"workspace_id":"w1","label":"~","focused":true},
        {"workspace_id":"w4","label":"fix-scroll",
         "worktree":{"is_linked_worktree":true,"repo_name":"moshpit",
                     "checkout_path":"/Users/x/.herdr/worktrees/moshpit/fix-scroll"}}],
      "tabs":[],"panes":[],"layouts":[]}}}"#;
    let decoded = snapshot::decode(raw).unwrap();
    assert_eq!(
        decoded.worktree_repos.get("w4").map(String::as_str),
        Some("moshpit")
    );
    assert_eq!(decoded.worktree_repos.get("w1"), None);
}

/// The Swift cast `as? [[String: Any]]` is all or nothing; so is this one.
#[test]
fn workspaces_that_arent_all_objects_are_no_snapshot() {
    let raw = r#"{"workspaces":[{"workspace_id":"w1"},5],"tabs":[],"panes":[],"layouts":[]}"#;
    assert!(snapshot::decode(raw).is_none());
    let raw = r#"{"workspaces":[{"workspace_id":"w1"}],"tabs":[{"tab_id":"t"},"x"],"panes":[],"layouts":[]}"#;
    assert!(snapshot::decode(raw).unwrap().snapshot.windows.is_empty());
}

// MARK: - herdr status → version report

/// Captured on the host: the stale 0.8.0 binary against a 0.9.5 server.
const OLD_CLIENT: &str = "client:
  version: 0.8.0
  channel: stable
  protocol: 19

server:
  status: running
  version: 0.9.5
  protocol: 22
  compatible: no
  socket: /Users/u/.config/herdr/herdr.sock";

/// Captured on the host: 0.9.5 against itself — a different field set.
const MATCHED: &str = "client:
  version: 0.9.5
  channel: stable
  protocol: 22
  endpoint_protocol_generation: 1

server:
  status: running
  version: 0.9.5
  endpoint_compatible: yes";

#[test]
fn the_0_8_0_shape_client_older_than_the_server() {
    let report = VersionReport::parse(OLD_CLIENT).unwrap();
    assert_eq!(
        report,
        VersionReport {
            client: "0.8.0".to_owned(),
            server: "0.9.5".to_owned()
        }
    );
    assert!(report.mismatched());
    assert!(report.client_is_older());
    assert_eq!(
        report.notice(),
        VersionNotice::ClientOlder {
            client: "0.8.0".to_owned(),
            server: "0.9.5".to_owned()
        }
    );
    assert_eq!(
        report.notice().english(),
        "herdr on the host's PATH is 0.8.0, its server 0.9.5 — update that copy"
    );
}

#[test]
fn the_0_9_5_shape_same_version_both_ends_is_no_skew() {
    let report = VersionReport::parse(MATCHED).unwrap();
    assert!(!report.mismatched());
    assert!(!report.client_is_older());
}

#[test]
fn the_server_being_the_older_side_is_a_skew_the_restart_fixes() {
    let report = VersionReport {
        client: "0.9.5".to_owned(),
        server: "0.8.0".to_owned(),
    };
    assert!(report.mismatched());
    assert!(!report.client_is_older());
    assert_eq!(
        report.notice().english(),
        "The host's herdr server is 0.8.0, older than its herdr 0.9.5"
    );
}

#[test]
fn no_server_version_no_report() {
    assert_eq!(
        VersionReport::parse("client:\n  version: 0.9.5\n\nserver:\n  status: not running\n"),
        None
    );
    assert_eq!(
        VersionReport::parse("error: unknown command 'status'"),
        None
    );
    assert_eq!(VersionReport::parse(""), None);
}

#[test]
fn login_noise_above_the_report_doesnt_confuse_the_sections() {
    let report =
        VersionReport::parse(&format!("Last login: Tue on ttys001\n{OLD_CLIENT}")).unwrap();
    assert_eq!(report.client, "0.8.0");
    assert_eq!(report.server, "0.9.5");
}

#[test]
fn versions_compare_by_number_not_by_string() {
    assert_eq!(version::compare("0.10.0", "0.9.5"), Ordering::Greater);
    assert_eq!(version::compare("0.8.0", "0.9.5"), Ordering::Less);
    assert_eq!(version::compare("1.0", "1.0.0"), Ordering::Equal);
    assert_eq!(version::compare("0.9.5-preview", "0.9.5"), Ordering::Equal);
    // Empty components don't count, as with Swift's `split`.
    assert_eq!(version::compare("1..2", "1.2"), Ordering::Equal);
}

/// Fix (version-digits): Swift read a component's numeric Characters and
/// parsed them as an `Int`, so these all came out 0.
#[test]
fn a_component_is_its_leading_ascii_digits_of_any_length() {
    assert_eq!(version::compare("1٣.0", "1.0"), Ordering::Equal);
    assert_eq!(version::compare("0.5\u{301}", "0.5"), Ordering::Equal);
    assert_eq!(
        version::compare("0.99999999999999999999", "0.1"),
        Ordering::Greater
    );
    assert_eq!(
        version::compare("99999999999999999999", "99999999999999999998"),
        Ordering::Greater
    );
    assert_eq!(version::compare("007", "7"), Ordering::Equal);
}
