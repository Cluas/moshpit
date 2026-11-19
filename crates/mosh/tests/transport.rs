//! `MoshTransportStateMachineTests.swift`, one to one, on the pure state
//! machine: synthetic datagrams sealed with the real key instead of a fake
//! channel, a virtual clock instead of sleeps. Where a fix changes what a
//! Swift test expects, the test says so.

use moshpit_mosh::harness::{client_instruction, server_datagram, server_datagrams};
use moshpit_mosh::proto::ProtoWriter;
use moshpit_mosh::screen::{Framebuffer, new_frame};
use moshpit_mosh::transport::timestamp16;
use moshpit_mosh::wire::{HostInstruction, HostMessage, decode_user_message, encode_host_message};
use moshpit_mosh::{
    ChannelState, Event, HostFrame, Instruction, Session, Transport, TransportConfig, UserEvent,
};

const KEY: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// A transport, the server's view of the same key, and a virtual clock.
struct Rig {
    transport: Transport,
    server: Session,
    now: u64,
}

impl Rig {
    fn new() -> Self {
        Self::with(TransportConfig::default())
    }

    fn with(config: TransportConfig) -> Self {
        Self {
            transport: Transport::new(&KEY, config).unwrap(),
            server: Session::new(&KEY).unwrap(),
            now: 1_000_000,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn datagram(&self, seq: u64, id: u64, old: u64, new: u64, ack: u64, diff: &[u8]) -> Vec<u8> {
        server_datagram(&self.server, seq, id, old, new, ack, diff, u16::MAX)
    }

    fn receive(&mut self, datagram: &[u8]) {
        self.transport.receive(datagram, self.now);
    }

    fn advance(&mut self, ms: u64) {
        self.now += ms;
        self.transport.on_timer(self.now);
    }

    /// Time passes and whatever is due goes out (what the send timer does
    /// once the channel is ready; this rig never reports it).
    fn pass(&mut self, ms: u64) {
        self.now += ms;
        self.transport.tick(self.now);
    }

    fn events(&mut self) -> Vec<Event> {
        self.transport.drain_events().collect()
    }

    fn sent(&mut self) -> Vec<Instruction> {
        let server = self.server.clone();
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Transmit(datagram) => Some(client_instruction(&server, &datagram)),
                _ => None,
            })
            .collect()
    }

    fn frames(&mut self) -> Vec<HostFrame> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::HostFrame(frame) => Some(frame),
                _ => None,
            })
            .collect()
    }
}

/// A well-formed host diff carrying `text` as terminal output.
fn host_diff(text: &str) -> Vec<u8> {
    encode_host_message(&[HostInstruction::HostBytes(text.as_bytes().to_vec())])
}

/// A host diff the way `Complete::diff_from` writes one: the moved echo ack
/// first, then the output, if any.
fn host_diff_with_ack(text: Option<&str>, echo_ack: u64) -> Vec<u8> {
    let mut instructions = vec![HostInstruction::EchoAck(echo_ack)];
    if let Some(text) = text {
        instructions.push(HostInstruction::HostBytes(text.as_bytes().to_vec()));
    }
    encode_host_message(&instructions)
}

fn keys_in(diff: &[u8]) -> Vec<u8> {
    decode_user_message(diff)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event {
            UserEvent::Keystroke(keys) => Some(keys),
            UserEvent::Resize { .. } => None,
        })
        .flatten()
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// MARK: Apply

/// an in-order diff applies its host bytes and advances appliedHostNum
#[test]
fn in_order_apply() {
    let mut rig = Rig::new();
    let packet = rig.datagram(0, 1, 0, 1, 0, &host_diff("hello"));
    rig.receive(&packet);

    assert_eq!(rig.frames()[0].bytes, b"hello");
    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, 1);
    assert_eq!(diag.parse_failures, 0);
    assert_eq!(diag.gap_events, 0);
    assert_eq!(diag.datagrams_received, 1);
}

/// requestFullRedraw nudges the width and never rewinds the applied state
///
/// Fix (full-redraw): the narrower width goes out alone and the real one
/// only after the server has rendered a frame at it (that frame is hidden);
/// Swift sends both in one state, which a real server answers with nothing.
#[test]
fn full_redraw_resync() {
    let mut rig = Rig::new();
    rig.transport.start(80, 24, rig.now);

    // Screen at state 3.
    let first = rig.datagram(0, 1, 0, 3, 0, &host_diff("screen"));
    rig.receive(&first);
    assert_eq!(rig.transport.diagnostics().applied_host_num, 3);
    rig.events();

    rig.transport.request_full_redraw(rig.now);

    // The width nudge went out, a frame after the datagram that acked the
    // screen and carried the size...
    rig.pass(250);
    let sent = rig.sent();
    let diff = &sent.last().unwrap().diff;
    assert!(
        decode_user_message(diff)
            .unwrap()
            .ends_with(&[UserEvent::Resize {
                width: 79,
                height: 24
            }]),
        "the nudge must carry the narrower width"
    );
    // ...and the applied state is untouched (a rewind froze the screen).
    assert_eq!(rig.transport.diagnostics().applied_host_num, 3);

    // The server renders at 79 columns: applied, acked, not shown, and the
    // real width follows.
    let narrow = encode_host_message(&[
        HostInstruction::Resize {
            width: 79,
            height: 24,
        },
        HostInstruction::HostBytes(b"\x1b[H\x1b[2Jnarrow".to_vec()),
    ]);
    let next = rig.datagram(1, 2, 3, 4, 0, &narrow);
    rig.receive(&next);
    // ...with the ack of that frame, within mosh's ACK_DELAY.
    rig.pass(100);
    let events = rig.events();
    assert!(!events.iter().any(|e| matches!(e, Event::HostFrame(_))));
    let restore = events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Transmit(datagram) => Some(client_instruction(&rig.server, datagram)),
            _ => None,
        })
        .unwrap();
    assert_eq!(restore.ack_num, 4);
    assert!(
        decode_user_message(&restore.diff)
            .unwrap()
            .ends_with(&[UserEvent::Resize {
                width: 80,
                height: 24
            }])
    );

    // The repaint at 80 columns is shown, and in-order diffs keep applying:
    // no gap, no freeze.
    let repaint = encode_host_message(&[
        HostInstruction::Resize {
            width: 80,
            height: 24,
        },
        HostInstruction::HostBytes(b"\x1b[H\x1b[2Jfull".to_vec()),
    ]);
    let next = rig.datagram(2, 3, 4, 5, 0, &repaint);
    rig.receive(&next);
    assert_eq!(rig.frames()[0].bytes, b"\x1b[H\x1b[2Jfull");
    let more = rig.datagram(3, 4, 5, 6, 0, &host_diff("more"));
    rig.receive(&more);
    assert_eq!(rig.frames()[0].bytes, b"more");
    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, 6);
    assert_eq!(diag.gap_events, 0);

    // The transport's own idea of its size survived: a same-size resize is
    // still de-duped into silence.
    rig.transport.resize(80, 24, rig.now);
    assert!(rig.sent().is_empty());
}

/// consecutive in-order diffs apply in sequence
#[test]
fn in_order_sequence() {
    let mut rig = Rig::new();
    let first = rig.datagram(0, 1, 0, 1, 0, &host_diff("a"));
    let second = rig.datagram(1, 2, 1, 2, 0, &host_diff("b"));
    rig.receive(&first);
    rig.receive(&second);

    let frames = rig.frames();
    assert_eq!(frames[0].bytes, b"a");
    assert_eq!(frames[1].bytes, b"b");
    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, 2);
    assert_eq!(diag.parse_failures, 0);
    assert_eq!(diag.gap_events, 0);
}

/// an empty diff advances appliedHostNum without emitting host bytes
#[test]
fn empty_diff_advances() {
    let mut rig = Rig::new();
    let packet = rig.datagram(0, 1, 0, 3, 0, &[]);
    rig.receive(&packet);

    assert!(rig.frames().is_empty());
    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, 3);
    assert_eq!(diag.parse_failures, 0);
    assert_eq!(diag.gap_events, 0);
}

// MARK: Gap

/// a gap (oldNum doesn't match) is counted and does not advance state
#[test]
fn gap_does_not_advance() {
    let mut rig = Rig::new();
    // appliedHostNum is 0, but this diff claims to extend state 5.
    let packet = rig.datagram(0, 1, 5, 6, 0, &host_diff("skipped"));
    rig.receive(&packet);

    let diag = rig.transport.diagnostics();
    assert_eq!(
        diag.applied_host_num, 0,
        "gap must not advance the applied state"
    );
    assert_eq!(diag.gap_events, 1);
    assert_eq!(diag.parse_failures, 0);
}

// MARK: Parse failure (regression guard for e61a9b3)

/// a diff that fails to parse does NOT advance appliedHostNum
#[test]
fn parse_failure_does_not_advance() {
    let mut rig = Rig::new();
    // Field 1 / wire type 2 declaring a 5-byte length with only 1 byte left.
    let malformed = [0x0A, 0x05, 0x01];
    assert!(
        HostMessage::decode(&malformed).is_err(),
        "fixture must fail to parse"
    );

    let packet = rig.datagram(0, 1, 0, 1, 0, &malformed);
    rig.receive(&packet);

    let diag = rig.transport.diagnostics();
    assert_eq!(
        diag.applied_host_num, 0,
        "silently acking unparseable content is the blank-screen bug from e61a9b3"
    );
    assert_eq!(diag.parse_failures, 1);
    assert_eq!(diag.gap_events, 0);
    // Still acks, the true (unchanged) state, within mosh's ACK_DELAY.
    assert!(rig.sent().is_empty());
    rig.pass(100);
    let sent = rig.sent();
    assert!(!sent.is_empty());
    assert_eq!(sent[0].ack_num, 0);
}

/// a later valid diff still can't apply on top of an un-advanced state
#[test]
fn parse_failure_then_gap() {
    let mut rig = Rig::new();
    let bad = rig.datagram(0, 1, 0, 1, 0, &[0x0A, 0x05, 0x01]);
    rig.receive(&bad);
    // Server's next diff builds on state 1, which we never applied → gap.
    let next = rig.datagram(1, 2, 1, 2, 0, &host_diff("x"));
    rig.receive(&next);

    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, 0);
    assert_eq!(diag.parse_failures, 1);
    assert_eq!(diag.gap_events, 1);
}

// MARK: Roaming

/// an authenticated reply clears the roaming flag
#[test]
fn authenticated_reply_clears_roaming() {
    let mut rig = Rig::new();
    rig.transport.start(80, 24, rig.now);
    rig.transport.on_better_path(true, rig.now);
    assert!(rig.transport.roaming());

    let packet = rig.datagram(0, 1, 0, 1, 0, &host_diff("hi"));
    rig.receive(&packet);
    assert!(!rig.transport.roaming());
}

// MARK: Return-path watchdog

/// the return-path watchdog fires when a ready socket never receives a datagram
#[test]
fn return_path_dead_fires_when_silent() {
    let mut rig = Rig::with(TransportConfig {
        return_path_deadline_ms: 100,
        ..TransportConfig::default()
    });
    rig.transport.start(80, 24, rig.now);
    // Ready, but the server's replies never arrive.
    rig.transport.on_channel_state(ChannelState::Ready, rig.now);
    rig.advance(400);
    assert!(rig.events().contains(&Event::ReturnPathDead));
}

/// the return-path watchdog does not fire once any datagram arrives
#[test]
fn return_path_dead_silent_when_data_arrives() {
    let mut rig = Rig::with(TransportConfig {
        return_path_deadline_ms: 300,
        ..TransportConfig::default()
    });
    rig.transport.start(80, 24, rig.now);
    rig.transport.on_channel_state(ChannelState::Ready, rig.now);

    // A reply lands well before the deadline → the watchdog stands down.
    let packet = rig.datagram(0, 1, 0, 1, 0, &host_diff("hi"));
    rig.receive(&packet);
    rig.advance(600);

    assert!(!rig.events().contains(&Event::ReturnPathDead));
    assert!(rig.transport.diagnostics().datagrams_received >= 1);
}

// MARK: Keystroke delivery TTL

/// keystrokes past their TTL stop retransmitting; resizes survive the prune
///
/// Fix (ttl-rewrite): the stale keystroke never goes out again, as in Swift,
/// but the surviving resize waits until the server answers: Swift strips the
/// keystroke out of the already-sent state at once, which desynchronises a
/// server that did receive it. Nothing ever arrives here, so input is held;
/// a later datagram that still does not ack it releases the resize alone.
#[test]
fn expired_keystrokes_drop_on_flush() {
    let mut rig = Rig::with(TransportConfig {
        keystroke_ttl_ms: 50,
        ..TransportConfig::default()
    });
    rig.transport.start(80, 24, rig.now); // resize queued as state 1
    rig.transport.send(b"STALE-SECRET", rig.now); // keystroke state 2, flushes now
    rig.events();

    // The link stays dead past the TTL: nothing acked, nothing received.
    rig.now += 200;
    rig.transport.resume(false, rig.now); // the recovery-time flush
    let held = rig.sent();
    assert!(!held.is_empty());
    let last = held.last().unwrap();
    assert!(
        !contains(&last.diff, b"STALE-SECRET"),
        "an expired keystroke replaying into the shell is the ghost-input bug"
    );
    assert_eq!((last.old_num, last.new_num), (0, 0), "held: no user state");
    assert!(rig.transport.diagnostics().input_held);

    // The server answers 5 s later without acking anything: it never got
    // the keystroke, so now it is dropped and the rest goes out.
    rig.now += 5_000;
    let answer = rig.datagram(0, 1, 0, 1, 0, &[]);
    rig.receive(&answer);
    let sent = rig.sent();
    let last = sent.last().unwrap();
    assert!(!contains(&last.diff, b"STALE-SECRET"));
    // State 3 is empty: 5.2 s after the server was last sent a new state,
    // the flush gives it one (idle_state).
    assert_eq!((last.old_num, last.new_num), (0, 3));
    assert_eq!(
        decode_user_message(&last.diff).unwrap(),
        vec![UserEvent::Resize {
            width: 80,
            height: 24
        }],
        "the resize event is idempotent state and must survive"
    );
    assert!(!rig.transport.diagnostics().input_held);
}

/// fresh keystrokes keep retransmitting — typing through a brief blip still delivers
#[test]
fn fresh_keystrokes_survive_flush() {
    let mut rig = Rig::with(TransportConfig {
        keystroke_ttl_ms: 10_000,
        ..TransportConfig::default()
    });
    rig.transport.start(80, 24, rig.now);
    rig.transport.send(b"FRESH-KEYS", rig.now);
    rig.events();

    rig.transport.resume(false, rig.now);
    let sent = rig.sent();
    assert!(!sent.is_empty());
    assert!(
        contains(&keys_in(&sent.last().unwrap().diff), b"FRESH-KEYS"),
        "un-expired keystrokes are exactly what SSP must keep retransmitting"
    );
}

// MARK: SRTT

/// SRTT smooths toward the sampled round-trip time and damps spikes
#[test]
fn srtt_converges_and_damps() {
    let mut rig = Rig::new();
    let mut seq = 0;
    let mut id = 1;
    // Six round trips each reporting a 120 ms RTT; old == new == 0 keeps
    // this to the SRTT path with no apply or gap.
    for _ in 0..6 {
        let reply = timestamp16(rig.now).wrapping_sub(120);
        let packet = server_datagram(&rig.server, seq, id, 0, 0, 0, &[], reply);
        seq += 1;
        id += 1;
        rig.receive(&packet);
        rig.now += 10;
    }
    let converged = rig.transport.srtt_ms();
    assert!(
        converged > 90.0 && converged < 220.0,
        "SRTT should converge near the 120ms sample, got {converged}"
    );

    // A single spike is blended in (0.875 srtt + 0.125 sample), not adopted.
    let reply = timestamp16(rig.now).wrapping_sub(600);
    let spike = server_datagram(&rig.server, seq, id, 0, 0, 0, &[], reply);
    rig.receive(&spike);
    let after = rig.transport.srtt_ms();
    assert!(after > converged, "the spike should pull SRTT upward");
    assert!(
        after < 360.0,
        "but the EWMA should damp it well below the 600ms sample, got {after}"
    );
}

/// a forced resume replaces the flow instead of diagnosing it
#[test]
fn forced_resume_rebuilds_flow() {
    let mut rig = Rig::new();
    rig.transport.start(80, 24, rig.now);
    rig.transport.on_channel_state(ChannelState::Ready, rig.now);
    rig.events();

    rig.transport.resume(true, rig.now);
    let events = rig.events();
    assert_eq!(
        events.iter().filter(|e| **e == Event::RebuildFlow).count(),
        1,
        "the zombie flow must be replaced"
    );
    assert_eq!(rig.transport.diagnostics().flow_rebuilds, 1);

    // The un-forced path stays conservative: a ready channel is left alone,
    // only flushed.
    rig.transport.on_channel_state(ChannelState::Ready, rig.now);
    rig.events();
    rig.transport.resume(false, rig.now);
    let events = rig.events();
    assert!(!events.contains(&Event::RebuildFlow));
    assert!(!events.contains(&Event::RestartFlow));
    assert!(events.iter().any(|e| matches!(e, Event::Transmit(_))));
}

// MARK: Echo ack (predictive echo's clock)

/// send reports the user state its keystrokes ride in
#[test]
fn send_reports_user_state() {
    let mut rig = Rig::new();
    rig.transport.start(80, 24, rig.now); // state 1: the size
    assert_eq!(rig.transport.send(b"a", rig.now), Some(2));
    assert_eq!(rig.transport.send(b"b", rig.now), Some(3));
    assert_eq!(rig.transport.send(b"", rig.now), None);
}

/// the echo ack rides with the output, and a diff that only moves it still lands
#[test]
fn echo_ack_frames() {
    let mut rig = Rig::new();
    let first = rig.datagram(0, 1, 0, 1, 0, &host_diff_with_ack(Some("a"), 2));
    rig.receive(&first);
    assert_eq!(
        rig.frames(),
        vec![HostFrame {
            bytes: b"a".to_vec(),
            echo_ack: 2
        }]
    );

    // 50 ms later the server acks the next keystroke with no new output:
    // exactly the frame that confirms a prediction.
    let second = rig.datagram(1, 2, 1, 2, 0, &host_diff_with_ack(None, 3));
    rig.receive(&second);
    assert_eq!(
        rig.frames(),
        vec![HostFrame {
            bytes: Vec::new(),
            echo_ack: 3
        }]
    );

    // Output with no ack instruction keeps the last one.
    let third = rig.datagram(2, 3, 2, 3, 0, &host_diff("b"));
    rig.receive(&third);
    assert_eq!(
        rig.frames(),
        vec![HostFrame {
            bytes: b"b".to_vec(),
            echo_ack: 3
        }]
    );
    assert_eq!(rig.transport.diagnostics().applied_host_num, 3);
}

// MARK: Delivery (what a herdr arrow key waits on over mosh)

/// waitUntilDelivered holds until the server acks the newest user state
#[test]
fn delivery_waits_for_the_ack() {
    let mut rig = Rig::new();
    let first = rig.transport.send(b"g", rig.now).unwrap();
    let newest = rig.transport.send(b"i", rig.now).unwrap();
    assert!(!rig.transport.is_delivered(newest));

    // An ack that stops short of the newest state is not delivery.
    let partial = rig.datagram(0, 1, 0, 1, first, &[]);
    rig.receive(&partial);
    assert!(rig.events().contains(&Event::Acked(first)));
    assert!(!rig.transport.is_delivered(newest));

    // A server acks only what it was sent (B105): the "i" goes out a frame
    // after the "g".
    rig.pass(300);
    let full = rig.datagram(1, 2, 1, 2, newest, &[]);
    rig.receive(&full);
    assert!(rig.events().contains(&Event::Acked(newest)));
    assert!(rig.transport.is_delivered(newest));
}

/// waitUntilDelivered never strands its caller: nothing outstanding, already acked, closed
#[test]
fn delivery_never_strands() {
    let mut rig = Rig::new();
    // Nothing queued yet.
    assert!(rig.transport.is_delivered(rig.transport.newest_user_num()));

    let typed = rig.transport.send(b"x", rig.now).unwrap();
    let ack = rig.datagram(0, 1, 0, 1, typed, &[]);
    rig.receive(&ack);
    assert!(rig.transport.is_delivered(rig.transport.newest_user_num())); // already acked

    rig.transport.send(b"y", rig.now); // never acked
    assert!(!rig.transport.is_delivered(rig.transport.newest_user_num()));
    rig.transport.close();
    assert!(rig.transport.is_delivered(rig.transport.newest_user_num())); // closing releases it
    assert!(rig.events().contains(&Event::Finished));
}

#[test]
fn rig_sanity_proto_writer_matches_host_encoder() {
    // The Swift tests build host diffs with ProtoWriter by hand; the
    // encoder used above must write the same bytes.
    let mut host_bytes = ProtoWriter::new();
    host_bytes.field_bytes(4, b"hello");
    let mut instruction = ProtoWriter::new();
    instruction.field_bytes(2, host_bytes.bytes());
    let mut message = ProtoWriter::new();
    message.field_bytes(1, instruction.bytes());
    assert_eq!(message.bytes(), host_diff("hello"));
}

// MARK: Diffs from an older state (CLI-LAB.md, Bug 2)

/// mosh-server sends a small diff from the state the client last acked
/// (`attempt_prospective_resend_optimization`), not from the newest state
/// it sent: over a 500 ms round trip the live probe saw `0→1, 0→2, 0→3,
/// 0→4, 1→5, 1→6, 1→7, 5→8`. mosh-client keeps every state since the acked
/// one and applies each. Holding only the newest, this transport applied
/// one state per round trip (5 of every 15 received, frames ~750 ms apart
/// instead of 250 ms), and whatever the server snapshotted mid-redraw
/// stayed up that long.
#[test]
fn a_diff_from_an_older_held_state_applies() {
    let mut rig = Rig::new();
    let first = rig.datagram(0, 1, 0, 1, 0, &host_diff("a"));
    rig.receive(&first);
    assert_eq!(rig.frames().len(), 1);
    // The ack for state 1 is still on its way: the server diffs from 0.
    let second = rig.datagram(1, 2, 0, 2, 0, &host_diff("ab"));
    rig.receive(&second);
    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, 2);
    assert_eq!(diag.gap_events, 0);
    // The terminal shows "a": it gets only what state 2 adds.
    let frames = rig.frames();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].bytes, b"b");
}

/// One server state every 250 ms over a 500 ms round trip, each diffed from
/// the newest state the server has heard acked (and throwing away the ones
/// before it), as mosh-server does for small changes. Every state is shown,
/// a frame every 250 ms, and the terminal fed every frame always looks like
/// the server's screen of the state shown.
#[test]
fn at_a_500_ms_round_trip_every_state_is_shown() {
    const ONE_WAY_MS: u64 = 250;
    const STATES: u64 = 40;
    let mut rig = Rig::new();
    let start = rig.now;
    rig.transport.start(80, 24, start);
    rig.events();

    // The server's screens: a clock on the top line, a prompt being typed
    // at, and output scrolling the region above the status line.
    let mut screens = vec![Framebuffer::new(80, 24)];
    for n in 1..=STATES {
        let mut screen = screens.last().unwrap().clone();
        let text = match n % 4 {
            0 => format!("\x1b[1;23r\x1b[23;1H\nline {n}\x1b[r"),
            1 => format!("\x1b[24;1H\x1b[7m status {n} \x1b[0m\x1b[K"),
            _ => String::new(),
        } + &format!(
            "\x1b[1;60HTue 19 Nov 12:00:{:02}\x1b[22;1H$ {}",
            n % 60,
            "x".repeat(n as usize % 30)
        );
        screen.apply(text.as_bytes());
        screens.push(screen);
    }

    // In flight: (arrival time, datagram) each way.
    let mut to_client: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut to_server: Vec<(u64, u64)> = Vec::new();
    let mut server_acked = 0;
    let mut pane = Framebuffer::new(80, 24);
    let mut frame_times = Vec::new();
    let mut seq = 0;
    let end = start + (STATES + 4) * 250;
    while rig.now < end {
        let now = rig.now;
        // The server hears acks...
        for &(_, ack) in to_server.iter().filter(|(at, _)| *at <= now) {
            server_acked = server_acked.max(ack);
        }
        to_server.retain(|(at, _)| *at > now);
        // ...and every 250 ms sends its next state, from the acked one.
        let since = now - start;
        if since > 0 && since.is_multiple_of(250) && since / 250 <= STATES {
            let new = since / 250;
            let old = server_acked;
            let diff = encode_host_message(&[HostInstruction::HostBytes(new_frame(
                true,
                &screens[old as usize],
                &screens[new as usize],
            ))]);
            let instruction = Instruction {
                old_num: old,
                new_num: new,
                throwaway_num: old,
                diff,
                ..Instruction::default()
            };
            for datagram in
                server_datagrams(&rig.server, seq, seq + 1, &instruction, 0, u16::MAX, 1300)
            {
                to_client.push((now + ONE_WAY_MS, datagram));
                seq += 1;
            }
        }
        let arrived: Vec<Vec<u8>> = to_client
            .iter()
            .filter(|(at, _)| *at <= now)
            .map(|(_, datagram)| datagram.clone())
            .collect();
        to_client.retain(|(at, _)| *at > now);
        for datagram in arrived {
            rig.receive(&datagram);
        }
        rig.pass(10);
        for event in rig.events() {
            match event {
                Event::HostFrame(frame) => {
                    pane.apply(&frame.bytes);
                    frame_times.push(rig.now);
                    let shown = rig.transport.diagnostics().applied_host_num as usize;
                    assert!(pane.looks_like(&screens[shown]), "state {shown}");
                }
                Event::Transmit(datagram) => {
                    let ack = client_instruction(&rig.server, &datagram).ack_num;
                    to_server.push((rig.now + ONE_WAY_MS, ack));
                }
                _ => {}
            }
        }
    }

    let diag = rig.transport.diagnostics();
    assert_eq!(diag.applied_host_num, STATES);
    assert_eq!(diag.host_states_shown, STATES);
    assert_eq!(diag.gap_events, 0);
    // Most frames were rendered from a held state: the diff named a state
    // older than the one the terminal showed.
    assert!(diag.host_frames_rendered > STATES / 2, "{diag:?}");
    // The server throws states away a round trip behind: a few held.
    assert!(diag.held_states <= 5, "{diag:?}");
    let gaps: Vec<u64> = frame_times.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|&gap| gap <= 260), "{gaps:?}");
    eprintln!(
        "{} states shown of {STATES}, frames every {} ms, {} held ({} bytes)",
        diag.host_states_shown,
        gaps.iter().sum::<u64>() / gaps.len() as u64,
        diag.held_states,
        diag.held_bytes
    );
}
