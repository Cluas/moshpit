//! The timer decisions `MoshTransport.swift` makes with sleeping `Task`s
//! (the send timer, return-path watchdog, liveness rebuilds, restart after
//! a failure) and the lifecycle around them, pinned on the virtual clock.

use moshpit_mosh::harness::server_datagram;
use moshpit_mosh::{ChannelState, Event, Session, Transport, TransportConfig};

const KEY: [u8; 16] = [7; 16];
const T0: u64 = 50_000;

fn transport() -> Transport {
    Transport::new(&KEY, TransportConfig::default()).unwrap()
}

fn transmits(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::Transmit(_)))
        .count()
}

fn drain(transport: &mut Transport) -> Vec<Event> {
    transport.drain_events().collect()
}

/// Runs the clock from `from` to `to` in 100 ms steps, firing due timers.
fn run(transport: &mut Transport, from: u64, to: u64) -> Vec<Event> {
    let mut events = Vec::new();
    let mut now = from;
    while now < to {
        now += 100;
        transport.on_timer(now);
        events.extend(drain(transport));
    }
    events
}

fn ping(seq: u64) -> Vec<u8> {
    server_datagram(
        &Session::new(&KEY).unwrap(),
        seq,
        seq + 1,
        0,
        0,
        0,
        &[],
        u16::MAX,
    )
}

#[test]
fn nothing_is_sent_or_armed_before_the_channel_is_ready() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    assert_eq!(transport.next_deadline(), None);
    assert!(drain(&mut transport).is_empty());
}

/// The first datagram announces the client (and the window size). Nothing
/// comes back here, so the size is sent again a retransmission timeout
/// later, each time: mosh's 1 s timeout before any RTT sample, plus its
/// 100 ms ACK_DELAY. That retrying at frame rate stops 10 s after the
/// channel became ready (mosh's ACTIVE_RETRY_TIMEOUT, from the server last
/// heard, which here it never was); then only the 3 s ack goes, carrying
/// the size again (the server holds nothing newer).
#[test]
fn ready_announces_the_client_then_retries_by_timeout_then_by_ack() {
    let mut transport = transport();
    transport.start(100, 30, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    assert_eq!(transmits(&drain(&mut transport)), 1);
    assert_eq!(transport.next_deadline(), Some(T0 + 1_100));
    // At 1.1, 2.2, ... 9.9 s: nine.
    let events = run(&mut transport, T0, T0 + 10_000);
    assert_eq!(transmits(&events), 9);
    // Then every 3 s from the last: 12.9, 15.9, 18.9 s.
    let events = run(&mut transport, T0 + 10_000, T0 + 20_000);
    assert_eq!(transmits(&events), 3);
}

/// A second ready (a restarted flow) sends a datagram at once. The size
/// sent 0.7 s before is still unacked, and a few bytes go again with any
/// datagram (mosh's prophylactic resend), so its timeout runs from here.
#[test]
fn a_second_ready_announces_again_and_restarts_the_timeout() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    drain(&mut transport);
    transport.on_channel_state(ChannelState::Ready, T0 + 700);
    assert_eq!(transmits(&drain(&mut transport)), 1);
    assert_eq!(transport.next_deadline(), Some(T0 + 1_800));
}

#[test]
fn the_watchdog_fires_once_and_is_never_rearmed() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    let events = run(&mut transport, T0, T0 + 7_900);
    assert!(!events.contains(&Event::ReturnPathDead));
    let events = run(&mut transport, T0 + 7_900, T0 + 8_000);
    assert_eq!(
        events
            .iter()
            .filter(|e| **e == Event::ReturnPathDead)
            .count(),
        1
    );
    // A later ready (a restarted flow) does not arm it again.
    transport.on_channel_state(ChannelState::Ready, T0 + 8_000);
    let events = run(&mut transport, T0 + 8_000, T0 + 30_000);
    assert!(!events.contains(&Event::ReturnPathDead));
}

#[test]
fn any_datagram_disarms_the_watchdog_even_one_that_does_not_decrypt() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    transport.receive(b"not a mosh datagram at all", T0 + 10);
    let events = run(&mut transport, T0, T0 + 8_500);
    assert!(!events.contains(&Event::ReturnPathDead));
    assert_eq!(transport.diagnostics().datagrams_received, 1);
}

#[test]
fn a_silent_flow_is_rebuilt_every_check_and_the_third_rebuild_reports_it() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    // Heard once, then nothing while the send timer keeps sending.
    transport.receive(&ping(0), T0 + 100);
    drain(&mut transport);
    let events = run(&mut transport, T0 + 100, T0 + 30_000);
    let rebuilds: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| **e == Event::RebuildFlow)
        .map(|(index, _)| index)
        .collect();
    let dead: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| **e == Event::ReturnPathDead)
        .map(|(index, _)| index)
        .collect();
    // Checks run every 3 s from the first flush (T0): 9.1 s of silence is
    // first seen at T0 + 12 s, then every check rebuilds again.
    assert!(rebuilds.len() >= 6, "{} rebuilds", rebuilds.len());
    assert_eq!(dead.len(), 1, "reported once, on the third rebuild");
    assert!(dead[0] < rebuilds[2] && dead[0] > rebuilds[1]);
    assert_eq!(transport.diagnostics().flow_rebuilds, rebuilds.len() as u64);
}

#[test]
fn liveness_leaves_a_flow_that_hears_back_alone() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    let mut now = T0;
    let mut events = Vec::new();
    for seq in 0..20 {
        now += 1_500;
        transport.receive(&ping(seq), now);
        transport.on_timer(now);
        events.extend(drain(&mut transport));
    }
    assert!(!events.contains(&Event::RebuildFlow));
}

#[test]
fn liveness_needs_first_contact() {
    // A flow that never heard anything is the watchdog's case, not this one.
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    let events = run(&mut transport, T0, T0 + 60_000);
    assert!(!events.contains(&Event::RebuildFlow));
}

/// A failed flow never comes back (Network.framework restarts only a
/// waiting one, and ignores `restart()` otherwise): a second later it is
/// replaced, and the new flow announces the client. It used to be
/// "restarted", which left it dead until the liveness check rebuilt it.
#[test]
fn a_failed_flow_is_replaced_a_second_later() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Ready, T0);
    drain(&mut transport);
    transport.on_channel_state(ChannelState::Failed, T0 + 10);
    assert!(drain(&mut transport).is_empty());
    transport.on_timer(T0 + 1_009);
    assert!(drain(&mut transport).is_empty());
    transport.on_timer(T0 + 1_010);
    let events = drain(&mut transport);
    assert_eq!(events[0], Event::RebuildFlow);
    assert!(!events.contains(&Event::RestartFlow));
    assert_eq!(transmits(&events), 1, "the new flow announces us");
    assert_eq!(transport.diagnostics().flow_rebuilds, 1);
}

/// A flow replaced in the meantime (a forced resume) has not failed: the
/// failure's replacement does not replace it again.
#[test]
fn a_failure_replacement_skips_a_flow_already_replaced() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Failed, T0);
    transport.resume(true, T0 + 500);
    transport.on_channel_state(ChannelState::Ready, T0 + 600);
    drain(&mut transport);
    transport.on_timer(T0 + 1_000);
    assert!(!drain(&mut transport).contains(&Event::RebuildFlow));
    assert_eq!(transport.diagnostics().flow_rebuilds, 1);
}

#[test]
fn closing_cancels_a_pending_restart_and_finishes_once() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_channel_state(ChannelState::Failed, T0);
    transport.close();
    transport.on_channel_state(ChannelState::Cancelled, T0 + 10);
    transport.on_timer(T0 + 2_000);
    assert_eq!(drain(&mut transport), vec![Event::Finished]);
    assert_eq!(transport.next_deadline(), None);
    // Nothing goes out after close, though state numbers still advance.
    assert_eq!(transport.send(b"x", T0 + 3_000), Some(2));
    assert!(drain(&mut transport).is_empty());
}

#[test]
fn a_cancelled_channel_ends_the_host_stream() {
    let mut transport = transport();
    transport.on_channel_state(ChannelState::Cancelled, T0);
    assert_eq!(drain(&mut transport), vec![Event::Finished]);
}

/// A waiting flow is restarted, a failed one replaced (restart does nothing
/// for it); either way a datagram goes out.
#[test]
fn resume_restarts_a_waiting_flow_and_replaces_a_failed_one() {
    for (state, wanted) in [
        (ChannelState::Waiting, Event::RestartFlow),
        (ChannelState::Failed, Event::RebuildFlow),
    ] {
        let mut transport = transport();
        transport.start(80, 24, T0);
        transport.on_channel_state(state, T0);
        drain(&mut transport);
        transport.resume(false, T0 + 5);
        let events = drain(&mut transport);
        assert_eq!(events[0], wanted, "{state:?}");
        assert_eq!(transmits(&events), 1);
    }
}

#[test]
fn a_better_path_nudges_a_packet_out_and_reports_roaming() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.on_better_path(true, T0);
    let events = drain(&mut transport);
    assert_eq!(
        events[0],
        Event::Metrics {
            srtt_ms: 0.0,
            roaming: true
        }
    );
    assert_eq!(transmits(&events), 1);
    // The same value again is not news.
    transport.on_better_path(true, T0 + 1);
    assert!(drain(&mut transport).is_empty());
    // Losing the better path reports, without a nudge.
    transport.on_better_path(false, T0 + 2);
    assert_eq!(
        drain(&mut transport),
        vec![Event::Metrics {
            srtt_ms: 0.0,
            roaming: false
        }]
    );
}

#[test]
fn start_runs_once() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.start(120, 40, T0);
    assert_eq!(transport.newest_user_num(), 1);
}

#[test]
fn nothing_comes_out_after_close() {
    let mut transport = transport();
    transport.start(80, 24, T0);
    transport.close();
    drain(&mut transport);
    let output =
        moshpit_mosh::wire::encode_host_message(&[moshpit_mosh::wire::HostInstruction::HostBytes(
            b"late".to_vec(),
        )]);
    let datagram = server_datagram(
        &Session::new(&KEY).unwrap(),
        0,
        1,
        0,
        1,
        0,
        &output,
        u16::MAX,
    );
    transport.receive(&datagram, T0 + 10);
    transport.on_better_path(true, T0 + 20);
    transport.resume(true, T0 + 30);
    transport.request_full_redraw(T0 + 40);
    let events = drain(&mut transport);
    assert!(
        events.iter().all(|e| matches!(e, Event::Metrics { .. })),
        "{events:?}"
    );
    assert_eq!(transport.diagnostics().applied_host_num, 0);
}
