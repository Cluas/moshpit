//! The socket client against REAL herdr 0.8.0 (protocol 19) captures, and
//! the push driver's pure parts. Ported from
//! `Tests/MoshpitTests/Services/Herdr/HerdrSocketClientTests.swift` and
//! `HerdrPushDriverTests.swift`: `herdr-socket-snapshot.json` is one
//! `session.snapshot` reply line, `herdr-socket-events.jsonl` a live
//! `events.subscribe` stream (the bootstrap replay, existing workspace, tab
//! and pane as created/focused events, then a real `tab.create`).
//!
//! The Swift tests drive an actor with a scripted far end; here the state
//! machine is driven directly: the request's line is what would be written,
//! and replies are fed back in.

use std::time::Duration;

use moshpit_herdr::json;
use moshpit_herdr::snapshot;
use moshpit_herdr::socket::{
    self, GLOBAL_KINDS, InvalidateDebounce, Output, PANE_SCOPED_KINDS, PUMP_READY_MARKER,
    SocketClient, SocketError, Subscription,
};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/app/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn events(outputs: &[Output]) -> Vec<&str> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::Event(event) => Some(event.kind.as_str()),
            _ => None,
        })
        .collect()
}

fn reply<'a>(outputs: &'a [Output], wanted: &str) -> &'a Result<String, SocketError> {
    outputs
        .iter()
        .find_map(|output| match output {
            Output::Reply { id, result } if id == wanted => Some(result),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no reply for {wanted} in {outputs:?}"))
}

// MARK: - Request round trip

#[test]
fn the_request_envelope_carries_a_string_id_the_method_and_mandatory_params() {
    let mut client = SocketClient::new();
    let (id, line) = client.request("ping", socket::EMPTY_PARAMS).unwrap();
    assert!(line.ends_with('\n'));
    let object = json::parse_object(line.trim_end().as_bytes()).unwrap();
    // id must be a STRING: an integer id is silently ignored by herdr.
    assert_eq!(object.str("id"), Some(id.as_str()));
    assert_eq!(object.str("method"), Some("ping"));
    // params must exist even when empty: same silent-ignore failure.
    assert!(object.object("params").is_some());
    let (next, _) = client.request("ping", socket::EMPTY_PARAMS).unwrap();
    assert_eq!((id.as_str(), next.as_str()), ("mp1", "mp2"));
}

#[test]
fn the_real_snapshot_reply_resolves_the_request_and_feeds_the_existing_decoder() {
    let mut client = SocketClient::new();
    let (id, _) = client
        .request("session.snapshot", socket::EMPTY_PARAMS)
        .unwrap();
    // Re-stamp the capture's id ("s1") with the live request's.
    let line = fixture("herdr-socket-snapshot.json")
        .trim()
        .replace(r#""id": "s1""#, &format!(r#""id": "{id}""#));
    let outputs = client.feed(format!("{line}\n").as_bytes());
    let raw = reply(&outputs, &id).as_ref().expect("a reply");
    let decoded = snapshot::decode(raw).expect("the reply is a snapshot");
    assert_eq!(decoded.snapshot.sessions.len(), 1);
    assert_eq!(decoded.snapshot.panes.len(), 1);
    assert_eq!(client.pending().count(), 0);
}

// MARK: - Event stream

#[test]
fn the_real_capture_yields_the_bootstrap_replay_then_the_live_mutation() {
    let mut client = SocketClient::new();
    let capture = fixture("herdr-socket-events.jsonl");
    let lines: Vec<&str> = capture
        .split('\n')
        .filter(|line| !line.is_empty())
        .collect();
    let mut received = Vec::new();
    for line in &lines {
        received.extend(client.feed(format!("{line}\n").as_bytes()));
    }
    received.extend(client.finish());
    let kinds = events(&received);
    assert_eq!(kinds.len(), lines.len());
    // The bootstrap replay: current state arrives as synthetic events.
    assert_eq!(
        kinds[..5],
        [
            "workspace_created",
            "workspace_focused",
            "tab_created",
            "tab_focused",
            "pane_created"
        ]
    );
    // The live tab.create mutation lands after the replay.
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "tab_created").count(),
        2
    );
    assert!(kinds.contains(&"pane_agent_detected"));
}

#[test]
fn partial_chunks_reassemble_and_responses_and_events_interleave_on_one_pipe() {
    let mut client = SocketClient::new();
    let event = format!(
        "{}\n",
        r#"{"event":"pane_focused","data":{"pane_id":"w1:p1"}}"#
    );
    let bytes = event.as_bytes();
    let mut outputs = client.feed(&bytes[..17]);
    assert!(
        outputs.is_empty(),
        "no event until the newline completes it"
    );
    outputs.extend(client.feed(&bytes[17..]));
    // Shell noise between JSON lines is skipped, not fatal.
    outputs.extend(client.feed(b"Last login: Sun Aug 17 on ttys002\n"));
    outputs.extend(client.feed(bytes));
    outputs.extend(client.finish());
    assert_eq!(events(&outputs), ["pane_focused", "pane_focused"]);
}

// MARK: - Failure shapes

#[test]
fn a_server_error_envelope_fails_the_request_with_its_code() {
    let mut client = SocketClient::new();
    let (id, _) = client.request("pane.focus", socket::EMPTY_PARAMS).unwrap();
    let outputs = client.feed(
        format!(r#"{{"id":"{id}","error":{{"code":"pane_not_found","message":"no such pane"}}}}"#)
            .as_bytes(),
    );
    assert!(outputs.is_empty(), "a reply waits for its newline");
    let outputs = client.feed(b"\n");
    assert_eq!(
        reply(&outputs, &id),
        &Err(SocketError::Server {
            code: "pane_not_found".to_owned(),
            message: "no such pane".to_owned()
        })
    );
}

#[test]
fn an_empty_id_error_fails_every_request_in_flight() {
    let mut client = SocketClient::new();
    let (first, _) = client
        .request("session.snapshot", socket::EMPTY_PARAMS)
        .unwrap();
    let (second, _) = client.request("ping", socket::EMPTY_PARAMS).unwrap();
    let outputs = client.feed(
        b"{\"id\":\"\",\"error\":{\"code\":\"invalid_request\",\"message\":\"missing field\"}}\n",
    );
    let expected = Err(SocketError::Server {
        code: "invalid_request".to_owned(),
        message: "missing field".to_owned(),
    });
    assert_eq!(reply(&outputs, &first), &expected);
    assert_eq!(reply(&outputs, &second), &expected);
    assert_eq!(client.pending().count(), 0);
}

#[test]
fn eof_fails_pending_requests_instead_of_hanging_them() {
    let mut client = SocketClient::new();
    let (id, _) = client
        .request("session.snapshot", socket::EMPTY_PARAMS)
        .unwrap();
    let outputs = client.finish();
    assert_eq!(reply(&outputs, &id), &Err(SocketError::ConnectionClosed));
    // …and nothing can be asked after.
    assert_eq!(
        client.request("ping", socket::EMPTY_PARAMS),
        Err(SocketError::ConnectionClosed)
    );
    assert!(client.finish().is_empty());
}

/// The Swift test waits out a real 80 ms deadline; the timer is the
/// transport's, and its expiry is this call.
#[test]
fn no_reply_times_out_rather_than_waiting_forever() {
    let mut client = SocketClient::new();
    let (id, _) = client.request("ping", socket::EMPTY_PARAMS).unwrap();
    assert_eq!(
        client.expire(&id),
        Some(Output::Reply {
            id: id.clone(),
            result: Err(SocketError::Timeout)
        })
    );
    // A late reply then routes to nobody.
    assert!(
        client
            .feed(format!("{{\"id\":\"{id}\",\"result\":{{}}}}\n").as_bytes())
            .is_empty()
    );
    assert_eq!(client.expire(&id), None);
}

// MARK: - Subscription encoding

#[test]
fn pane_scoped_kinds_carry_pane_id_and_global_kinds_omit_it_entirely() {
    let mut client = SocketClient::new();
    let (_, line) = client
        .subscribe(&["tab.created"], &[("pane.agent_status_changed", "w1:p1")])
        .unwrap();
    let object = json::parse_object(line.trim_end().as_bytes()).unwrap();
    assert_eq!(object.str("method"), Some("events.subscribe"));
    let subscriptions = object
        .object("params")
        .and_then(|params| params.objects("subscriptions"))
        .unwrap();
    assert_eq!(subscriptions.len(), 2);
    let global = subscriptions
        .iter()
        .find(|s| s.str("type") == Some("tab.created"))
        .unwrap();
    // Absent, not null: the schema validates per variant.
    assert!(global.get("pane_id").is_none());
    let scoped = subscriptions
        .iter()
        .find(|s| s.str("type") == Some("pane.agent_status_changed"))
        .unwrap();
    assert_eq!(scoped.str("pane_id"), Some("w1:p1"));
    assert_eq!(
        socket::subscribe_params(&[Subscription {
            kind: "a",
            pane_id: None
        }]),
        r#"{"subscriptions":[{"type":"a"}]}"#
    );
}

/// What the Swift client writes is JSONEncoder's: `/` escaped, control
/// characters as `\u00xx`, the rest as itself.
#[test]
fn request_strings_are_escaped_as_jsonencoder_writes_them() {
    let mut client = SocketClient::new();
    let (_, line) = client
        .request(
            "a/b\"\\\u{1}\u{8}\u{c}\n\r\t\u{1f}é😀\u{2028}",
            socket::EMPTY_PARAMS,
        )
        .unwrap();
    assert_eq!(
        line,
        "{\"id\":\"mp1\",\"method\":\"a\\/b\\\"\\\\\\u0001\\b\\f\\n\\r\\t\\u001fé😀\u{2028}\",\"params\":{}}\n"
    );
}

// MARK: - The pump

/// The pump's readiness is the marker at the END of a line: the boot line
/// carries the marker in its own python source, and the shell echoes it.
#[test]
fn the_pump_is_ready_only_when_a_line_ends_with_its_marker() {
    let mut client = SocketClient::new();
    // The echoed boot line ends in `")'`, never in the marker.
    let echoed =
        format!("stty raw -echo; exec python3 -c 'os.write(1,b\"{PUMP_READY_MARKER}\\n\")'\n");
    assert!(client.feed(echoed.as_bytes()).is_empty());
    assert!(!client.pump_ready());
    // zsh glues a window-title escape in front of it; CRLF behind.
    let outputs = client.feed(format!("\x1bkstty\x1b\\{PUMP_READY_MARKER}\r\n").as_bytes());
    assert_eq!(outputs, vec![Output::PumpReady]);
    assert!(client.pump_ready());
    // Once.
    assert!(
        client
            .feed(format!("{PUMP_READY_MARKER}\n").as_bytes())
            .is_empty()
    );
}

// MARK: - Push driver: boot line

/// One word as a POSIX shell reads it: single quotes literal, a backslash
/// outside them taking the next character. `None` for anything else.
fn posix_word(word: &str) -> Option<String> {
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '\'') => quoted = false,
            (true, c) => out.push(c),
            (false, '\'') => quoted = true,
            (false, '\\') => out.push(chars.next()?),
            (false, _) => return None,
        }
    }
    (!quoted).then_some(out)
}

/// Whether a quoted word holds a backslash inside its single quotes, which
/// fish would read as an escape (B138).
fn backslash_inside_quotes(word: &str) -> bool {
    let mut quoted = false;
    for c in word.chars() {
        match c {
            '\'' => quoted = !quoted,
            '\\' if quoted => return true,
            _ => {}
        }
    }
    false
}

#[test]
fn the_boot_line_is_single_line_single_quote_safe_and_kills_the_line_discipline() {
    let line = socket::pump_boot_line();
    // A raw newline would submit the shell command early.
    assert!(!line.contains('\n'));
    // The -c argument is one word every login shell reads alike (B138):
    // hand-quoted, fish read the `\\n` it holds as `\n`.
    let word = line
        .strip_prefix(&format!(
            "stty raw -echo; PATH=\"$PATH:{}\" exec python3 -c ",
            moshpit_herdr::launch::EXTRA_PATH_DIRS
        ))
        .expect("the line discipline killed, then the pump, found as the probe finds python3");
    assert!(!backslash_inside_quotes(word));
    let program = posix_word(word).expect("one word");
    assert!(program.starts_with("exec(\"import os,socket,threading\\n"));
    assert!(program.contains(&format!("b\\\"{PUMP_READY_MARKER}\\\\n\\\")")));
    assert!(line.contains("~/.config/herdr/herdr.sock"));
}

// MARK: - Push driver: subscription kinds

#[test]
fn global_kinds_never_include_a_pane_scoped_kind() {
    let global: std::collections::BTreeSet<&str> = GLOBAL_KINDS.into_iter().collect();
    for kind in PANE_SCOPED_KINDS {
        assert!(
            !global.contains(kind),
            "{kind} subscribed bare hangs up the pipe"
        );
    }
    // No duplicates sneaking a kind in twice.
    assert_eq!(global.len(), GLOBAL_KINDS.len());
}

// MARK: - Push driver: debounce (on an explicit clock)

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// Feeds events at `times` (ms), ticking the timer at every deadline it
/// asks for; returns the times the invalidation fired.
fn fire_times(events: &[u64], until: u64) -> Vec<u64> {
    let mut debounce = InvalidateDebounce::new();
    let mut fired = Vec::new();
    let mut pending = events.iter().copied().peekable();
    for now in 0..=until {
        while pending.peek() == Some(&now) {
            pending.next();
            if debounce.event(ms(now)) {
                fired.push(now);
            }
        }
        if debounce.tick(ms(now)) {
            fired.push(now);
        }
    }
    fired
}

#[test]
fn an_event_burst_collapses_into_one_invalidation_and_a_later_event_fires_again() {
    // The bootstrap-replay shape: a burst in one gulp.
    assert_eq!(fire_times(&[0, 0, 0, 0, 0], 500), [250]);
    assert_eq!(fire_times(&[0, 0, 0, 0, 0, 1000], 1500), [250, 1250]);
}

/// The bug this pins: a guard of "is a timer already running" fires once per
/// interval for as long as events keep coming (37 reads in 8 s, measured).
#[test]
fn a_stream_of_events_spread_over_time_is_still_one_invalidation() {
    let events: Vec<u64> = (0..9).map(|n| n * 100).collect();
    assert_eq!(fire_times(&events, 2000), [1050]);
}

/// …and the cap that keeps the reset from starving a read forever.
#[test]
fn a_stream_longer_than_the_cap_still_gets_read() {
    let events: Vec<u64> = (0..26).map(|n| n * 100).collect();
    let fired = fire_times(&events, 4000);
    assert_eq!(fired.first(), Some(&2000), "the cap fires the read at 2 s");
    assert_eq!(fired, [2000, 2750]);
}
