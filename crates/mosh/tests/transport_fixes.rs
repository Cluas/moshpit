//! One test group per deliberate difference from `MoshTransport.swift`
//! (`harness/divergences.json`): `throwaway`, `ttl-rewrite` and `timestamps`.
//! The `ttl-rewrite` group drives the client against a model of
//! mosh-server's user-stream receiver (`networktransport-impl.h`,
//! `user.cc`), including the Swift pruning rule, to show the difference.

use std::collections::BTreeMap;

use moshpit_mosh::compress::{self, CompressionError};
use moshpit_mosh::harness::{SplitMix64, client_instruction, server_datagram};
use moshpit_mosh::transport::{SHUTDOWN_STATE, keystroke_chunk, timestamp16};
use moshpit_mosh::wire::{
    Fragment, FragmentAssembler, HostInstruction, Instruction, UserEvent, decode_user_message,
    encode_host_message, encode_user_message,
};
use moshpit_mosh::{ChannelState, Event, HostFrame, Session, Transport, TransportConfig};

const KEY: [u8; 16] = [3; 16];

fn server() -> Session {
    Session::new(&KEY).unwrap()
}

fn sent_datagrams(transport: &mut Transport) -> Vec<Vec<u8>> {
    transport
        .drain_events()
        .filter_map(|event| match event {
            Event::Transmit(datagram) => Some(datagram),
            _ => None,
        })
        .collect()
}

fn sent(transport: &mut Transport) -> Vec<Instruction> {
    let server = server();
    sent_datagrams(transport)
        .iter()
        .map(|datagram| client_instruction(&server, datagram))
        .collect()
}

// MARK: throwaway

/// Every instruction tells the server it may drop states below the acked
/// one: the oldest state the client still diffs from.
#[test]
fn throwaway_names_the_acked_user_state() {
    let server = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    let first = sent(&mut transport);
    assert_eq!(first[0].throwaway_num, 0);

    // A frame apart (250 ms before an RTT sample), each key goes at once.
    for (seq, typed) in (1u64..=5).enumerate() {
        let num = transport.send(&[b'a' + typed as u8], 300 * typed).unwrap();
        assert_eq!(sent(&mut transport).len(), 1);
        // The server acks it.
        transport.receive(
            &server_datagram(&server, seq as u64, typed, 0, 0, num, &[], u16::MAX),
            300 * typed + 5,
        );
    }
    transport.send(b"z", 1_800);
    let last = sent(&mut transport).pop().unwrap();
    assert_eq!(last.old_num, 6);
    assert_eq!(last.throwaway_num, last.old_num);
}

// MARK: ttl-rewrite, against a model of mosh-server's receiver

/// mosh-server's view of the client's user stream: `Transport::recv` in
/// `networktransport-impl.h` plus `UserStream::diff_from`, with each
/// keystroke byte one event as in `UserStream::apply_string`. A state that
/// does not extend the last processed one is a desync: release builds
/// silently misalign or read out of bounds, assert builds abort.
#[derive(Default)]
struct ServerModel {
    states: BTreeMap<u64, Vec<Event8>>,
    last_processed: Vec<Event8>,
    desyncs: usize,
    /// (typed byte, delivery time) for every keystroke handed to the shell.
    delivered: Vec<(u8, u64)>,
    quench_until: u64,
    quenched: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event8 {
    Byte(u8),
    Resize(i32, i32),
}

impl ServerModel {
    fn new() -> Self {
        let mut model = ServerModel::default();
        model.states.insert(0, Vec::new());
        model
    }

    fn ack(&self) -> u64 {
        *self.states.keys().next_back().unwrap()
    }

    fn receive(&mut self, instruction: &Instruction, now: u64) {
        if self.states.contains_key(&instruction.new_num) {
            return;
        }
        let Some(reference) = self.states.get(&instruction.old_num).cloned() else {
            return;
        };
        self.states
            .retain(|&num, _| num >= instruction.throwaway_num);
        if self.states.len() > 1024 {
            if now < self.quench_until {
                self.quenched += 1;
                return;
            }
            self.quench_until = now + 15_000;
        }
        let mut state = reference;
        for event in decode_user_message(&instruction.diff).unwrap() {
            match event {
                UserEvent::Keystroke(keys) => state.extend(keys.into_iter().map(Event8::Byte)),
                UserEvent::Resize { width, height } => state.push(Event8::Resize(width, height)),
            }
        }
        let newest = instruction.new_num > self.ack();
        self.states.insert(instruction.new_num, state.clone());
        if !newest {
            return;
        }
        if !state.starts_with(&self.last_processed) {
            self.desyncs += 1;
        } else {
            for event in &state[self.last_processed.len()..] {
                if let Event8::Byte(byte) = event {
                    self.delivered.push((*byte, now));
                }
            }
        }
        self.last_processed = state;
    }
}

/// Swift's flush-time rule, applied to the same scenario: strip expired
/// keystrokes from every unacked state, then send acked → newest.
struct SwiftPruningClient {
    states: Vec<(u64, Vec<UserEvent>, u64)>,
    acked: u64,
}

impl SwiftPruningClient {
    fn type_key(&mut self, key: u8, now: u64) {
        let num = self.states.last().unwrap().0 + 1;
        self.states
            .push((num, vec![UserEvent::Keystroke(vec![key])], now));
    }

    fn flush(&mut self, now: u64, ttl: u64) -> Instruction {
        for (num, events, queued) in &mut self.states {
            if *num > self.acked && now - *queued > ttl {
                events.retain(|event| !event.is_keystroke());
            }
        }
        let events: Vec<UserEvent> = self
            .states
            .iter()
            .filter(|(num, _, _)| *num > self.acked)
            .flat_map(|(_, events, _)| events.clone())
            .collect();
        Instruction {
            old_num: self.acked,
            new_num: self.states.last().unwrap().0,
            diff: if events.is_empty() {
                Vec::new()
            } else {
                encode_user_message(&events)
            },
            ..Instruction::default()
        }
    }
}

/// The trigger: the server got a keystroke, its acks were lost for longer
/// than the TTL, and the user typed again. With Swift's rule the next state
/// is shorter than the one the server processed: a desync, and the new key
/// never reaches the shell.
#[test]
fn swift_pruning_desyncs_a_server_that_got_the_keystrokes() {
    let ttl = 10_000;
    let mut model = ServerModel::new();
    let mut client = SwiftPruningClient {
        states: vec![(0, Vec::new(), 0)],
        acked: 0,
    };
    for (i, key) in b"abcde".iter().enumerate() {
        client.type_key(*key, i as u64);
        model.receive(&client.flush(i as u64, ttl), i as u64); // arrives
    }
    // Every ack is lost; 11 s later the user types again.
    client.type_key(b'x', 11_000);
    model.receive(&client.flush(11_000, ttl), 11_000);
    assert_eq!(model.desyncs, 1);
    assert!(!model.delivered.iter().any(|&(byte, _)| byte == b'x'));
}

/// The same trigger against this client: input is held until the server
/// answers, the answer acks the keystrokes, nothing is dropped and the new
/// key is delivered once.
#[test]
fn held_input_resumes_on_an_ack_without_dropping_anything() {
    let server = server();
    let mut model = ServerModel::new();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    for instruction in sent(&mut transport) {
        model.receive(&instruction, 0);
    }
    for (i, key) in b"abcde".iter().enumerate() {
        transport.send(&[*key], 1 + i as u64);
        for instruction in sent(&mut transport) {
            model.receive(&instruction, 1 + i as u64);
        }
    }
    // Acks lost for 11 s (the keys go a frame later, then again each
    // retransmission timeout while the server was heard within 10 s, all
    // arriving).
    for second in 1..=11u64 {
        transport.on_timer(second * 1_000);
        for instruction in sent(&mut transport) {
            model.receive(&instruction, second * 1_000);
        }
    }
    // The keys are past their TTL: the next datagram holds input back.
    transport.send(b"x", 11_500);
    assert!(transport.diagnostics().input_held);
    let held = sent(&mut transport);
    assert!(!held.is_empty());
    assert!(
        held.iter()
            .all(|i| i.new_num == i.old_num && i.diff.is_empty())
    );
    for instruction in &held {
        model.receive(instruction, 11_500);
    }
    // The return path heals: the server's next datagram acks state 6.
    let answer = server_datagram(&server, 0, 1, 0, 1, model.ack(), &[], u16::MAX);
    transport.receive(&answer, 12_000);
    assert!(!transport.diagnostics().input_held);
    for instruction in sent(&mut transport) {
        model.receive(&instruction, 12_000);
    }
    assert_eq!(model.desyncs, 0);
    let bytes: Vec<u8> = model.delivered.iter().map(|&(byte, _)| byte).collect();
    assert_eq!(bytes, b"abcdex");
}

/// When the server never got the keystrokes, they are dropped, but only once
/// a datagram arrives at least the proof delay after holding began.
#[test]
fn held_input_drops_expired_keys_only_after_the_proof_delay() {
    let server = server();
    let config = TransportConfig::default();
    let mut transport = Transport::new(&KEY, config.clone()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    transport.send(b"lost", 300); // a frame on, sent at once; never arrives
    assert!(!sent(&mut transport).is_empty());
    transport.on_timer(11_000); // the retransmission: expired → held from 11 s
    sent(&mut transport);
    assert!(transport.diagnostics().input_held);
    transport.send(b"new", 11_100); // queued, never sent while held
    assert!(sent(&mut transport).iter().all(|i| i.diff.is_empty()));

    // An answer 1 s into the hold proves nothing yet.
    transport.receive(
        &server_datagram(&server, 0, 1, 0, 1, 0, &[], u16::MAX),
        12_000,
    );
    assert!(transport.diagnostics().input_held);
    assert!(sent(&mut transport).iter().all(|i| i.new_num == i.old_num));

    // One 5 s in does: "lost" goes, "new" (fresh) and the resize stay.
    transport.receive(
        &server_datagram(&server, 1, 2, 1, 2, 0, &[], u16::MAX),
        16_000,
    );
    assert!(!transport.diagnostics().input_held);
    let out = sent(&mut transport);
    let last = out.last().unwrap();
    assert_eq!((last.old_num, last.new_num), (0, 3));
    assert_eq!(
        decode_user_message(&last.diff).unwrap(),
        vec![
            UserEvent::Resize {
                width: 80,
                height: 24
            },
            UserEvent::Keystroke(b"new".to_vec()),
        ]
    );
}

/// Keystrokes that expire before they were ever sent are dropped at once:
/// the server cannot hold them.
#[test]
fn unsent_keystrokes_expire_without_a_hold() {
    let server = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    transport.send(b"old", 0);
    transport.on_timer(10_500); // held from here
    transport.send(b"queued", 10_600);
    sent(&mut transport);
    // The server never answers until 31 s; by then "queued" expired unsent.
    transport.receive(
        &server_datagram(&server, 0, 1, 0, 1, 1, &[], u16::MAX),
        31_000,
    );
    let last = sent(&mut transport).pop().unwrap();
    assert_eq!((last.old_num, last.new_num), (1, 3));
    assert!(last.diff.is_empty(), "both keystrokes expired");
}

/// Random loss both ways, random typing, random pauses: the server model
/// never desyncs, never runs a keystroke twice, never runs one older than
/// the TTL, and once the link is healthy everything fresh arrives.
#[test]
fn the_server_never_desyncs_under_random_loss() {
    for seed in 1..=64u64 {
        let mut rng = SplitMix64(seed);
        let server = server();
        let config = TransportConfig::default();
        let ttl = config.keystroke_ttl_ms;
        let mut transport = Transport::new(&KEY, config).unwrap();
        let mut model = ServerModel::new();
        let mut typed_at: BTreeMap<u8, u64> = BTreeMap::new();
        let mut next_key = 0u8;
        let mut last_typed = 0;
        let mut now = 1_000u64;
        let mut seq = 0u64;
        transport.start(80, 24, now);
        transport.on_channel_state(ChannelState::Ready, now);
        let (mut up_ok, mut down_ok) = (true, true);
        for _ in 0..400 {
            now += 50 + rng.below(1_500);
            if rng.below(8) == 0 {
                up_ok = rng.below(3) != 0;
                down_ok = rng.below(3) != 0;
            }
            if rng.below(3) == 0 && next_key < 250 {
                typed_at.insert(next_key, now);
                last_typed = transport.send(&[next_key], now).unwrap();
                next_key += 1;
            }
            transport.on_timer(now);
            if down_ok && rng.below(2) == 0 {
                let answer =
                    server_datagram(&server, seq, seq + 1, 0, 0, model.ack(), &[], u16::MAX);
                seq += 1;
                transport.receive(&answer, now);
            }
            // Delivered when sent: the age checked below is the age at
            // transmission, which is what ghost input is about.
            for datagram in sent_datagrams(&mut transport) {
                if up_ok && rng.below(10) != 0 {
                    model.receive(&client_instruction(&server, &datagram), now);
                }
            }
        }
        // Heal both ways and let it settle.
        for _ in 0..40 {
            now += 500;
            transport.on_timer(now);
            let answer = server_datagram(&server, seq, seq + 1, 0, 0, model.ack(), &[], u16::MAX);
            seq += 1;
            transport.receive(&answer, now);
            for datagram in sent_datagrams(&mut transport) {
                model.receive(&client_instruction(&server, &datagram), now);
            }
        }
        assert_eq!(model.desyncs, 0, "seed {seed}");
        let mut seen = std::collections::BTreeSet::new();
        for &(byte, at) in &model.delivered {
            assert!(seen.insert(byte), "seed {seed}: key {byte} delivered twice");
            assert!(
                at - typed_at[&byte] <= ttl,
                "seed {seed}: key {byte} ran {} ms after it was typed",
                at - typed_at[&byte]
            );
        }
        assert!(!transport.diagnostics().input_held, "seed {seed}");
        // Every typed state is acked once the link heals; past it at most
        // the idle state the last heartbeat added is in flight.
        let acked = transport.server_acked_user_num();
        assert!(acked >= last_typed, "seed {seed}: {acked} < {last_typed}");
        assert!(
            transport.newest_user_num() - acked <= 1,
            "seed {seed}: everything is acked once the link heals"
        );
    }
}

/// Types `count` keys 250 ms apart (a frame apart before any RTT sample, so
/// each goes alone), each answered by the server model at once, with
/// `throwaway_num` as sent or forced to 0 (the Swift transport's value).
fn type_steadily(count: u64, swift_throwaway: bool) -> ServerModel {
    let server = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    let mut model = ServerModel::new();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    let mut now = 0;
    for seq in 0..count {
        now += 250;
        transport.send(&[seq as u8], now);
        let answer = server_datagram(&server, seq, seq + 1, 0, 0, model.ack(), &[], u16::MAX);
        transport.receive(&answer, now);
        for datagram in sent_datagrams(&mut transport) {
            let mut instruction = client_instruction(&server, &datagram);
            if swift_throwaway {
                instruction.throwaway_num = 0;
            }
            model.receive(&instruction, now);
        }
    }
    model
}

/// mosh-server's receive queue: with `throwaway_num` left at 0 every user
/// state stays, and past 1024 the server takes one new state per 15 s (the
/// live test in `harness/` shows the stall with the app's own transport);
/// with the fix it holds only the states still in flight (three here: the
/// answer is a step behind the input).
#[test]
fn the_server_queue_stays_small_with_throwaway() {
    let model = type_steadily(2_000, false);
    assert_eq!(model.quenched, 0);
    assert!(model.states.len() <= 3, "{}", model.states.len());

    let swift = type_steadily(2_000, true);
    assert!(swift.states.len() > 1024);
    assert!(swift.quenched > 0);
}

// MARK: idle states

/// mosh-server's view of when it last heard from its client: the newest
/// user state it received (`mosh-server.cc`, `time_since_remote_state`).
/// `MOSH_SERVER_NETWORK_TMOUT` reaps the session when that grows past it,
/// and frames are retried at frame rate only while it is under 10 s.
#[derive(Default)]
struct ServerClock {
    newest: u64,
    newest_at: u64,
    longest_silence: u64,
}

impl ServerClock {
    fn receive(&mut self, instruction: &Instruction, now: u64) {
        if instruction.new_num > self.newest {
            self.longest_silence = self.longest_silence.max(now - self.newest_at);
            self.newest = instruction.new_num;
            self.newest_at = now;
        }
    }
}

/// A connected client that never types, its timers run every second,
/// against a server that acks what it holds every second: the server hears
/// a new user state every 3 s (the client's ack interval), and each is
/// empty. The Swift and Rust transports used to repeat the acked state: the
/// server heard nothing new after the first second, and with TMOUT=8 reaped
/// the session at 8 s.
#[test]
fn an_idle_client_gives_the_server_a_new_state_every_three_seconds() {
    let server = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    let mut clock = ServerClock::default();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    for instruction in sent(&mut transport) {
        clock.receive(&instruction, 0);
    }
    for second in 1..=60u64 {
        let now = second * 1_000;
        transport.on_timer(now);
        // The server's ack of the newest state it holds.
        let answer = server_datagram(&server, second, second, 0, 0, clock.newest, &[], u16::MAX);
        transport.receive(&answer, now + 1);
        for instruction in sent(&mut transport) {
            assert_eq!(instruction.throwaway_num, instruction.old_num);
            // Idle states add no events: at most the size, until acked.
            let events = decode_user_message(&instruction.diff).unwrap();
            assert!(
                events.is_empty()
                    || events
                        == [UserEvent::Resize {
                            width: 80,
                            height: 24
                        }],
                "{events:?}"
            );
            clock.receive(&instruction, now);
        }
    }
    assert!(
        clock.longest_silence <= 3_000,
        "the server went {} ms without a new user state",
        clock.longest_silence
    );
    assert!(clock.newest >= 20, "{} states in a minute", clock.newest);
    assert!(60_000 - clock.newest_at <= 3_000);
}

/// Typing keeps the server hearing new states anyway: no idle state while
/// input is flowing, and none while input is held (its diff would carry the
/// held keystrokes past their TTL).
#[test]
fn no_idle_state_while_typing_or_held() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    for step in 1..=20u64 {
        let num = transport.send(b"k", step * 500).unwrap();
        assert_eq!(num, step + 1, "every state is a keystroke");
        transport.on_timer(step * 500);
    }
    assert_eq!(transport.newest_user_num(), 21);
    sent(&mut transport);
    // Nothing acked since: the keystrokes expire at 10.5 s and input is
    // held; heartbeats for another 20 s add nothing.
    for second in 11..=30u64 {
        transport.on_timer(second * 1_000);
    }
    assert!(transport.diagnostics().input_held);
    assert_eq!(transport.newest_user_num(), 21);
    assert!(
        sent(&mut transport)
            .iter()
            .all(|i| i.new_num == i.old_num && i.diff.is_empty())
    );
}

/// B522 (the Swift client's deliveryIgnoresIdleStates, lost in the port):
/// a delivery waits for the newest state that carries input, not for an idle
/// state added after it. mosh-server acks an idle state only when it next
/// speaks, up to 3 s on a quiet screen: a herdr key over mosh, a reply to a
/// notification, waited that long after its own bytes had landed.
#[test]
fn a_delivery_does_not_wait_for_an_idle_state() {
    let server = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    sent(&mut transport);
    let typed = transport.send(b"a", 300).unwrap();
    transport.on_timer(300);
    sent(&mut transport);
    let answer = server_datagram(&server, 1, 1, 0, 1, typed, &[], u16::MAX);
    transport.receive(&answer, 400);
    assert!(transport.is_delivered(typed));
    // A quiet 3.6 s: an idle state, unacked.
    transport.on_timer(4_000);
    assert_eq!(transport.newest_user_num(), typed + 1, "an idle state");
    assert_eq!(
        transport.newest_input_num(),
        typed,
        "the idle state carries no input"
    );
    assert!(
        transport.is_delivered(transport.newest_input_num()),
        "the delivery has landed"
    );
    // New input is waited for again.
    let more = transport.send(b"b", 4_100).unwrap();
    assert_eq!(transport.newest_input_num(), more);
    assert!(!transport.is_delivered(more));
}

// MARK: large input

/// mosh-server's side of a long paste: it inflates each instruction into
/// its 4 MiB buffer and drops one that does not fit (a "Crypto exception",
/// never acked: `compressor.cc`, `dos_assert`), applies the rest by
/// `Transport::recv`'s rules (new state already held: ignored; old state
/// not held: ignored; then the throwaway), and hands each newest state's
/// new keystroke bytes to the shell.
#[derive(Default)]
struct ByteServer {
    assembler: FragmentAssembler,
    /// State number → keystroke bytes since state 0.
    states: BTreeMap<u64, Vec<u8>>,
    shell: Vec<u8>,
    refused: usize,
    most_held: usize,
    largest_instruction: usize,
}

impl ByteServer {
    fn new() -> Self {
        let mut server = ByteServer::default();
        server.states.insert(0, Vec::new());
        server
    }

    fn ack(&self) -> u64 {
        *self.states.keys().next_back().unwrap()
    }

    fn receive(&mut self, session: &Session, datagram: &[u8]) {
        let message = session.open(datagram).unwrap();
        let fragment = Fragment::parse(&message.payload).unwrap();
        let Some(payload) = self.assembler.receive(fragment) else {
            return;
        };
        let bytes = match compress::decompress(&payload) {
            Ok(bytes) => bytes,
            Err(CompressionError::TooLarge) => {
                self.refused += 1;
                return;
            }
            Err(error) => panic!("{error}"),
        };
        self.largest_instruction = self.largest_instruction.max(bytes.len());
        let instruction = Instruction::decode(&bytes).unwrap();
        assert_eq!(instruction.throwaway_num, instruction.old_num);
        if self.states.contains_key(&instruction.new_num) {
            return;
        }
        let Some(mut state) = self.states.get(&instruction.old_num).cloned() else {
            return;
        };
        self.states
            .retain(|&num, _| num >= instruction.throwaway_num);
        for event in decode_user_message(&instruction.diff).unwrap() {
            if let UserEvent::Keystroke(keys) = event {
                state.extend(keys);
            }
        }
        if instruction.new_num > self.ack() {
            assert!(state.starts_with(&self.shell), "desync");
            self.shell = state.clone();
        }
        self.states.insert(instruction.new_num, state);
        self.most_held = self.most_held.max(self.states.len());
    }
}

/// Printable bytes, as a paste would be, and not very compressible.
fn paste(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64(seed);
    (0..len).map(|_| 0x21 + rng.below(94) as u8).collect()
}

/// A paste that inflates past mosh-server's 4 MiB buffer arrives whole and
/// in order: it goes as states of at most 64 KiB, chained instructions,
/// each naming its own old state as the throwaway so the server keeps two
/// states, not the paste. It used to go as one instruction, refused by
/// every flush (live: 13 in 14 s) until its keystrokes expired.
#[test]
fn a_paste_past_the_servers_buffer_arrives_whole() {
    let session = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    let mut server = ByteServer::new();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    for datagram in sent_datagrams(&mut transport) {
        server.receive(&session, &datagram);
    }
    let pasted = paste(4 * 1024 * 1024 + 1_000, 7);
    // A frame after the size went (250 ms before any RTT sample): at once.
    let last = transport.send(&pasted, 300).unwrap();
    for datagram in sent_datagrams(&mut transport) {
        server.receive(&session, &datagram);
    }
    assert_eq!(server.refused, 0);
    assert_eq!(server.ack(), last);
    assert_eq!(server.shell, pasted);
    // 0, the size and the first run's end; from then on two at a time
    // (each instruction lets the one before it go), not the whole paste.
    assert!(server.most_held <= 3, "{} states held", server.most_held);
    assert!(server.largest_instruction <= 64 * 1024 + 64);
    // The server's ack ends it: nothing goes again, and the next datagram
    // is a plain ack.
    let ack = server_datagram(&session, 0, 1, 0, 1, server.ack(), &[], u16::MAX);
    transport.receive(&ack, 400);
    assert!(sent(&mut transport).is_empty());
    transport.resume(false, 500);
    let after = sent(&mut transport);
    assert_eq!(after.len(), 1);
    assert_eq!((after[0].old_num, after[0].new_num), (last, last));
}

/// Under a small budget: input splits into states that fit, and the
/// instructions of each datagram chain from a state the server holds or is
/// assumed to hold (the acked one, or the newest already sent while that
/// is still within its retransmission timeout) through the newest, each at
/// most the budget (or one state), together carrying exactly the events
/// after it, in order.
#[test]
fn instructions_stay_under_the_budget_and_chain() {
    let session = server();
    let budget = 100;
    let mut transport = Transport::new(
        &KEY,
        TransportConfig {
            instruction_budget: budget,
            // Minutes pass with states unacked; nothing is about expiry.
            keystroke_ttl_ms: u64::MAX / 4,
            ..TransportConfig::default()
        },
    )
    .unwrap();
    let mut rng = SplitMix64(3);
    transport.start(80, 24, 0);
    // Every user state's events, by number.
    let mut events_of: BTreeMap<u64, Vec<UserEvent>> = BTreeMap::new();
    events_of.insert(
        1,
        vec![UserEvent::Resize {
            width: 80,
            height: 24,
        }],
    );
    let (mut acked, mut newest_sent) = (0, 0);
    let mut current = (80, 24);
    let mut now = 0;
    for step in 0..200u64 {
        // A frame or more apart (250 ms before an RTT sample): input goes
        // at once.
        now += 250 + rng.below(1_000);
        let before = transport.newest_user_num();
        if rng.below(5) == 0 {
            let size = (60 + rng.below(100) as i32, 20 + rng.below(30) as i32);
            if size == current {
                continue; // no change, nothing sent
            }
            current = size;
            transport.resize(size.0, size.1, now);
            events_of.insert(
                before + 1,
                vec![UserEvent::Resize {
                    width: size.0,
                    height: size.1,
                }],
            );
        } else {
            let keys = paste(1 + rng.below(400) as usize, step);
            transport.send(&keys, now);
            for (index, chunk) in keys.chunks(keystroke_chunk(budget)).enumerate() {
                events_of.insert(
                    before + 1 + index as u64,
                    vec![UserEvent::Keystroke(chunk.to_vec())],
                );
            }
        }
        assert_eq!(
            transport.newest_user_num(),
            *events_of.keys().last().unwrap()
        );
        // The datagram's instructions: one chain.
        let mut assembler = FragmentAssembler::new();
        let mut flush = Vec::new();
        for datagram in sent_datagrams(&mut transport) {
            let message = session.open(&datagram).unwrap();
            if let Some(payload) = assembler.receive(Fragment::parse(&message.payload).unwrap()) {
                flush.push(Instruction::decode(&compress::decompress(&payload).unwrap()).unwrap());
            }
        }
        assert!(
            !flush.is_empty(),
            "step {step}: input a frame later goes at once"
        );
        let from = flush[0].old_num;
        assert!(
            from == acked || from == newest_sent,
            "step {step}: from {from}, acked {acked}, newest sent {newest_sent}"
        );
        let mut old = from;
        let mut events = Vec::new();
        for instruction in &flush {
            assert_eq!(instruction.old_num, old);
            assert_eq!(instruction.throwaway_num, old);
            let these = decode_user_message(&instruction.diff).unwrap();
            assert!(
                instruction.diff.len() <= budget || these.len() == 1,
                "{} bytes",
                instruction.diff.len()
            );
            assert!(these.iter().all(|event| match event {
                UserEvent::Keystroke(keys) => keys.len() <= keystroke_chunk(budget),
                UserEvent::Resize { .. } => true,
            }));
            events.extend(these);
            old = instruction.new_num;
        }
        assert_eq!(old, transport.newest_user_num());
        let expected: Vec<UserEvent> = events_of
            .range(from + 1..)
            .flat_map(|(_, events)| events.clone())
            .collect();
        assert_eq!(events, expected, "step {step}");
        newest_sent = old;
        // Now and then the server acks a state it was sent.
        if rng.below(4) == 0 {
            let to = flush[rng.below(flush.len() as u64) as usize].new_num;
            let ack = server_datagram(&session, step, step + 1, 0, 0, to, &[], u16::MAX);
            transport.receive(&ack, now);
            assert!(
                sent_datagrams(&mut transport).is_empty(),
                "an ack sends nothing"
            );
            if to > acked {
                acked = to;
                events_of.retain(|&num, _| num > acked);
            }
        }
    }
}

// MARK: sending discipline

/// A transport whose channel is ready at `at`, with what it sent drained.
fn ready_at(at: u64) -> Transport {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, at);
    transport.on_channel_state(ChannelState::Ready, at);
    sent_datagrams(&mut transport);
    transport
}

/// A frame of `text` from the server: state `old` → `old + 1`, acking user
/// state `ack`, echoing `reply`.
fn output(seq: u64, old: u64, ack: u64, text: &str, reply: u16) -> Vec<u8> {
    server_datagram(
        &server(),
        seq,
        seq + 1,
        old,
        old + 1,
        ack,
        &encode_host_message(&[HostInstruction::HostBytes(text.as_bytes().to_vec())]),
        reply,
    )
}

fn carries(instruction: &Instruction, keys: &[u8]) -> bool {
    decode_user_message(&instruction.diff)
        .unwrap()
        .iter()
        .any(|event| matches!(event, UserEvent::Keystroke(k) if k.windows(keys.len()).any(|w| w == keys)))
}

/// B105: a server that acks a user state the client never sent (only a bad
/// one does) wedged the input for good. The ack became the acked number,
/// every state below it went, and each key typed after got a number no
/// greater than it: never sent, and reported delivered at once. As mosh's
/// `process_acknowledgment_through`, an ack counts only for a state sent.
#[test]
fn an_ack_for_a_state_never_sent_does_not_wedge_the_input() {
    let session = server();
    let mut transport = ready_at(0);
    let typed = transport.send(b"a", 300).unwrap();
    assert!(carries(&sent(&mut transport)[0], b"a"));
    transport.receive(
        &server_datagram(&session, 0, 1, 0, 0, typed + 1000, &[], u16::MAX),
        350,
    );
    assert!(
        transport.server_acked_user_num() < typed,
        "an ack for a state never sent was taken"
    );

    let next = transport.send(b"b", 700).unwrap();
    assert!(next > typed, "the next key reused a number");
    assert!(
        !transport.is_delivered(next),
        "a key never sent was reported delivered"
    );
    assert!(
        sent(&mut transport).iter().any(|i| carries(i, b"b")),
        "the next key was never sent"
    );

    // The real ack still counts.
    transport.receive(
        &server_datagram(&session, 1, 2, 0, 0, next, &[], u16::MAX),
        800,
    );
    assert_eq!(transport.server_acked_user_num(), next);
    assert!(transport.is_delivered(next));
}

/// Output streaming in while a keystroke is in flight neither sends the
/// keystroke again nor answers each frame: the server acks the key within
/// its ACK_DELAY, the client acks the output within its own. Every frame
/// used to flush every unacked state, so the key went once per frame until
/// its ack arrived, and every frame got an ack of its own.
#[test]
fn output_does_not_resend_input_in_flight() {
    let mut transport = ready_at(0);
    let session = server();
    transport.receive(&server_datagram(&session, 0, 1, 0, 0, 1, &[], u16::MAX), 50);
    transport.send(b"k", 300);
    let first = sent(&mut transport);
    assert_eq!(first.len(), 1);
    assert!(carries(&first[0], b"k"));
    // 50 frames 10 ms apart; mosh-server acks the key 100 ms after it.
    let mut after = Vec::new();
    for i in 1..=50u64 {
        let now = 300 + 10 * i;
        let ack = if now >= 400 { 2 } else { 1 };
        transport.receive(&output(i, i - 1, ack, "x", u16::MAX), now);
        transport.on_timer(now);
        after.extend(sent(&mut transport));
    }
    assert!(
        after.iter().all(|i| !carries(i, b"k")),
        "the key went again"
    );
    // One ack ACK_DELAY after the first frame it acks: at 410, 520, 630, 740.
    assert_eq!(
        after.iter().map(|i| i.ack_num).collect::<Vec<_>>(),
        vec![11, 22, 33, 44]
    );
}

/// Unacked input goes again when its retransmission timeout runs out, not
/// before: mosh's SRTT + 4 RTTVAR (50 ms to 1 s) plus its 100 ms ACK_DELAY,
/// measured from when it was last sent.
#[test]
fn unacked_input_goes_again_only_after_the_timeout() {
    let mut transport = ready_at(0);
    let session = server();
    // One 100 ms sample: SRTT 100, RTTVAR 50, so 300 + 100 ms.
    transport.receive(
        &server_datagram(&session, 0, 1, 0, 0, 1, &[], timestamp16(150) - 100),
        150,
    );
    assert_eq!(transport.srtt_ms(), 100.0);
    transport.send(b"k", 1_000);
    assert_eq!(sent(&mut transport).len(), 1);
    let mut copies = Vec::new();
    for now in (1_001..=2_000).step_by(1) {
        transport.on_timer(now);
        for instruction in sent(&mut transport) {
            assert!(carries(&instruction, b"k"));
            copies.push(now);
        }
    }
    assert_eq!(copies, vec![1_400, 1_800]);
}

/// An idle client whose server acks it sends one datagram per 3 s, mosh's
/// ACK_INTERVAL; it used to send one every second.
#[test]
fn an_idle_client_sends_a_datagram_every_three_seconds() {
    let mut transport = ready_at(0);
    let session = server();
    let mut sends = Vec::new();
    let mut acked = 1;
    for now in (100..=30_000).step_by(100) {
        transport.on_timer(now);
        for instruction in sent(&mut transport) {
            acked = instruction.new_num;
            sends.push(now);
        }
        // The server acks the newest user state it was sent.
        transport.receive(
            &server_datagram(&session, now, now, 0, 0, acked, &[], u16::MAX),
            now + 10,
        );
    }
    assert_eq!(sends, (1..=10).map(|n| n * 3_000).collect::<Vec<_>>());
}

/// mosh's prophylactic resend: a few bytes in flight go again with new
/// input (cheaper than doubting them), a long input in flight does not.
#[test]
fn new_input_resends_a_little_in_flight_but_not_a_lot() {
    let mut transport = ready_at(0);
    let session = server();
    transport.receive(&server_datagram(&session, 0, 1, 0, 0, 1, &[], u16::MAX), 50);
    transport.send(b"ab", 300);
    sent(&mut transport);
    transport.send(b"c", 600);
    let next = sent(&mut transport);
    assert_eq!((next[0].old_num, next[0].new_num), (1, 3));
    assert!(carries(&next[0], b"ab") && carries(&next[0], b"c"));

    let big = paste(1_100, 1);
    let big_num = transport.send(&big, 900).unwrap();
    sent(&mut transport);
    transport.send(b"d", 1_200);
    let next = sent(&mut transport);
    assert_eq!(next.len(), 1);
    assert_eq!((next[0].old_num, next[0].new_num), (big_num, big_num + 1));
    assert_eq!(
        decode_user_message(&next[0].diff).unwrap(),
        vec![UserEvent::Keystroke(b"d".to_vec())]
    );
}

/// A 1 MiB paste while the server works through it, acking a state every
/// 50 ms and printing as it goes: everything goes once. Every frame used to
/// send everything still unacked again.
#[test]
fn a_long_paste_goes_once_while_the_server_acks_it() {
    let session = server();
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    let mut byte_server = ByteServer::new();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    for datagram in sent_datagrams(&mut transport) {
        byte_server.receive(&session, &datagram);
    }
    transport.receive(&server_datagram(&session, 0, 1, 0, 0, 1, &[], u16::MAX), 50);
    let pasted = paste(1024 * 1024, 5);
    let last = transport.send(&pasted, 300).unwrap();
    let burst = sent_datagrams(&mut transport);
    let burst_bytes: usize = burst.iter().map(Vec::len).sum();
    for datagram in &burst {
        byte_server.receive(&session, datagram);
    }
    assert_eq!(byte_server.ack(), last);
    // The server acks the next state every 50 ms, with a frame of output.
    let mut later = 0;
    let mut acked = 1;
    let mut i = 0u64;
    while acked < last {
        i += 1;
        let now = 300 + 50 * i;
        acked += 1;
        transport.receive(&output(i, i - 1, acked, "x", u16::MAX), now);
        transport.on_timer(now);
        later += sent_datagrams(&mut transport)
            .iter()
            .map(Vec::len)
            .sum::<usize>();
    }
    assert_eq!(byte_server.shell, pasted);
    assert!(
        later < 4 * 1_024,
        "{later} more bytes after a {burst_bytes}-byte burst: input went again"
    );
}

// MARK: session end

/// mosh-server's last state (`uint64_t(-1)`), from state `old`, with the
/// command's last output: what it sends once its command exited.
fn shutdown(seq: u64, old: u64, text: &str) -> Vec<u8> {
    server_datagram(
        &server(),
        seq,
        seq + 1,
        old,
        SHUTDOWN_STATE,
        0,
        &encode_host_message(&[HostInstruction::HostBytes(text.as_bytes().to_vec())]),
        u16::MAX,
    )
}

/// The remote command exited: mosh-server sends its shutdown state and
/// quits once it is acked. The client shows the last output, acks the state
/// a frame after its last datagram (mosh's "speed up shutdown sequence"),
/// and ends the session there, as mosh-client exits. It used to ack it and
/// go on sending into a server that was gone, until the liveness check
/// called the return path dead.
#[test]
fn the_shutdown_state_ends_the_session() {
    let mut transport = ready_at(0);
    transport.receive(&output(0, 0, 1, "$ exit", u16::MAX), 100);
    transport.on_timer(200); // the output's ack
    assert_eq!(sent(&mut transport).len(), 1);

    transport.receive(&shutdown(1, 1, "logout\r\n"), 300);
    let events: Vec<Event> = transport.drain_events().collect();
    assert!(events.contains(&Event::HostFrame(HostFrame {
        bytes: b"logout\r\n".to_vec(),
        echo_ack: 0,
    })));
    assert!(!events.iter().any(|e| matches!(e, Event::Transmit(_))));
    assert!(!transport.session_ended());
    // A frame (250 ms before an RTT sample) after the ack at 200.
    assert_eq!(transport.next_deadline(), Some(450));
    transport.on_timer(450);
    let events: Vec<Event> = transport.drain_events().collect();
    let acks: Vec<Instruction> = events
        .iter()
        .filter_map(|event| match event {
            Event::Transmit(datagram) => Some(client_instruction(&server(), datagram)),
            _ => None,
        })
        .collect();
    assert_eq!(acks.len(), 1);
    assert_eq!(acks[0].ack_num, SHUTDOWN_STATE);
    assert_eq!(
        &events[events.len() - 2..],
        &[Event::SessionEnded, Event::Finished]
    );
    assert!(transport.session_ended() && transport.is_closed());
    assert_eq!(transport.next_deadline(), None);

    // Nothing goes out after that, whatever happens.
    transport.send(b"x", 500);
    transport.receive(&shutdown(2, 1, ""), 600);
    transport.on_timer(10_000);
    transport.resume(false, 11_000);
    assert!(
        transport
            .drain_events()
            .all(|event| matches!(event, Event::Metrics { .. })),
        "the session is over"
    );
}

/// A shutdown state from a state the client does not hold is a gap like any
/// other: acked as the state it holds, the session going on, until
/// mosh-server's next try comes from that state.
#[test]
fn a_shutdown_state_that_does_not_apply_waits_for_one_that_does() {
    let mut transport = ready_at(0);
    transport.receive(&shutdown(0, 3, "logout"), 100);
    transport.on_timer(200);
    let acks = sent(&mut transport);
    assert!(!acks.is_empty() && acks.iter().all(|i| i.ack_num == 0));
    assert!(!transport.session_ended());

    transport.receive(&shutdown(1, 0, "logout"), 300);
    transport.on_timer(450);
    assert_eq!(sent(&mut transport).last().unwrap().ack_num, SHUTDOWN_STATE);
    assert!(transport.session_ended());
}

/// Held input (`ttl-rewrite`) does not hold the end back: the ack of the
/// shutdown state goes as a plain ack.
#[test]
fn the_shutdown_state_ends_a_session_whose_input_is_held() {
    let mut transport = ready_at(0);
    transport.send(b"lost", 300);
    transport.on_timer(11_000); // the keys expired unacked: held
    sent(&mut transport);
    assert!(transport.diagnostics().input_held);
    transport.receive(&shutdown(0, 0, ""), 11_100);
    transport.on_timer(11_250);
    let last = sent(&mut transport).pop().unwrap();
    assert_eq!((last.old_num, last.new_num), (0, 0));
    assert_eq!(last.ack_num, SHUTDOWN_STATE);
    assert!(transport.session_ended());
}

// MARK: timestamps

fn datagram_with(seq: u64, timestamp: u16, reply: u16) -> Vec<u8> {
    let instruction = Instruction::default();
    moshpit_mosh::harness::server_datagrams(
        &server(),
        seq,
        seq + 1,
        &instruction,
        timestamp,
        reply,
        1300,
    )
    .remove(0)
}

fn sent_messages(transport: &mut Transport) -> Vec<moshpit_mosh::Message> {
    let server = server();
    sent_datagrams(transport)
        .iter()
        .map(|datagram| server.open(datagram).unwrap())
        .collect()
}

/// 0xFFFF is the server's "nothing to echo": never an RTT sample.
#[test]
fn a_no_timestamp_reply_is_not_a_sample() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    // Low 16 bits of the clock under 5000: Swift would take now + 1 ms.
    let now = 3 * 65_536 + 1_000;
    transport.receive(&datagram_with(0, 500, u16::MAX), now);
    assert_eq!(transport.srtt_ms(), 0.0);
    // A real echo is a sample, and 0 is a real value.
    transport.receive(&datagram_with(1, 500, timestamp16(now) - 40), now);
    assert_eq!(transport.srtt_ms(), 40.0);
    transport.receive(&datagram_with(2, 500, 0), 65_536 * 4 + 80);
    assert_eq!(transport.srtt_ms(), 0.875 * 40.0 + 0.125 * 80.0);
}

/// The first sample seeds the estimate even when it is 0 ms.
#[test]
fn a_zero_first_sample_seeds_the_estimate() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.receive(&datagram_with(0, 1, timestamp16(10_000)), 10_000);
    transport.receive(&datagram_with(1, 1, timestamp16(10_000) - 80), 10_000);
    assert_eq!(transport.srtt_ms(), 10.0);
}

/// A reordered or replayed datagram still delivers its payload but feeds
/// neither the RTT nor the timestamp we echo.
#[test]
fn an_older_datagram_feeds_no_timestamps() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 19_990);
    transport.on_channel_state(ChannelState::Ready, 19_990);
    sent_datagrams(&mut transport);
    transport.receive(&datagram_with(5, 1_000, timestamp16(20_000) - 30), 20_000);
    assert_eq!(transport.srtt_ms(), 30.0);
    transport.receive(&datagram_with(4, 2_000, timestamp16(20_010) - 900), 20_010);
    assert_eq!(transport.srtt_ms(), 30.0);
    // The echo is still the in-order datagram's timestamp, advanced 20 ms
    // (the resize goes at once: a frame, 20 ms at this RTT, after the size).
    transport.resize(81, 24, 20_020);
    assert_eq!(sent_messages(&mut transport)[0].timestamp_reply, 1_020);
}

/// The server's timestamp is echoed once, advanced by the time held, and
/// only within a second; otherwise the reply is 0xFFFF (never 0).
#[test]
fn the_echo_is_fresh_and_used_once() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.resize(81, 24, 1_000);
    assert_eq!(sent_messages(&mut transport)[0].timestamp_reply, u16::MAX);

    transport.receive(&datagram_with(0, 7_000, u16::MAX), 2_000);
    sent_datagrams(&mut transport);
    // A frame apart (250 ms before an RTT sample), each goes at once.
    transport.resize(82, 24, 2_250);
    transport.resize(83, 24, 2_500);
    let replies: Vec<u16> = sent_messages(&mut transport)
        .iter()
        .map(|message| message.timestamp_reply)
        .collect();
    assert_eq!(replies, vec![7_250, u16::MAX]);

    transport.receive(&datagram_with(1, 9_000, u16::MAX), 3_000);
    sent_datagrams(&mut transport);
    transport.resize(84, 24, 4_000); // held a full second: too stale
    assert_eq!(sent_messages(&mut transport)[0].timestamp_reply, u16::MAX);
}

/// Our own timestamp skips 0xFFFF, which would read as "no timestamp".
#[test]
fn our_timestamp_never_reads_as_missing() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.resize(81, 24, 65_535);
    assert_eq!(sent_messages(&mut transport)[0].timestamp, 0);
    assert_eq!(timestamp16(65_534), 65_534);
    assert_eq!(timestamp16(65_536), 0);
}

/// In a multi-fragment flush only the first datagram carries the echo.
#[test]
fn a_fragmented_flush_echoes_once() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.on_channel_state(ChannelState::Ready, 0);
    sent_datagrams(&mut transport);
    transport.receive(&datagram_with(0, 100, u16::MAX), 10);
    assert!(sent_datagrams(&mut transport).is_empty());
    let mut rng = SplitMix64(9);
    // A frame after the size went: at once.
    transport.send(&rng.bytes(4_000), 260); // incompressible: 4 fragments
    let replies: Vec<u16> = sent_messages(&mut transport)
        .iter()
        .map(|message| message.timestamp_reply)
        .collect();
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0], 350);
    assert!(replies[1..].iter().all(|&reply| reply == u16::MAX));
}

// MARK: full-redraw

fn frame_datagram(seq: u64, old: u64, new: u64, instructions: &[HostInstruction]) -> Vec<u8> {
    server_datagram(
        &server(),
        seq,
        seq + 1,
        old,
        new,
        0,
        &encode_host_message(instructions),
        u16::MAX,
    )
}

fn shown(transport: &mut Transport) -> Vec<HostFrame> {
    transport
        .drain_events()
        .filter_map(|event| match event {
            Event::HostFrame(frame) => Some(frame),
            _ => None,
        })
        .collect()
}

fn resize(width: i32, height: i32) -> HostInstruction {
    HostInstruction::Resize { width, height }
}

fn bytes(text: &str) -> HostInstruction {
    HostInstruction::HostBytes(text.as_bytes().to_vec())
}

/// Frames at the narrow width and after it are hidden until the repaint at
/// the real width; an echo ack in a hidden frame still lands.
#[test]
fn a_redraw_hides_frames_until_the_repaint() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.request_full_redraw(10);
    sent(&mut transport);
    transport.receive(
        &frame_datagram(
            0,
            0,
            1,
            &[HostInstruction::EchoAck(1), resize(79, 24), bytes("narrow")],
        ),
        20,
    );
    assert_eq!(
        shown(&mut transport),
        vec![HostFrame {
            bytes: Vec::new(),
            echo_ack: 1
        }]
    );
    transport.receive(&frame_datagram(1, 1, 2, &[bytes("still narrow")]), 30);
    assert!(shown(&mut transport).is_empty());
    transport.receive(
        &frame_datagram(2, 2, 3, &[resize(80, 24), bytes("repaint")]),
        40,
    );
    assert_eq!(shown(&mut transport)[0].bytes, b"repaint");
    transport.receive(&frame_datagram(3, 3, 4, &[bytes("after")]), 50);
    assert_eq!(shown(&mut transport)[0].bytes, b"after");
    assert_eq!(transport.diagnostics().applied_host_num, 4);
}

/// The narrow resize goes out alone; the real width follows the frame at it.
#[test]
fn a_redraw_restores_the_width_after_the_narrow_frame() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(100, 30, 0);
    sent(&mut transport);
    transport.request_full_redraw(10);
    let narrowing = sent(&mut transport).pop().unwrap();
    assert_eq!(
        decode_user_message(&narrowing.diff).unwrap().last(),
        Some(&UserEvent::Resize {
            width: 99,
            height: 30
        })
    );
    // An unrelated frame first: shown, nothing restored yet.
    transport.receive(&frame_datagram(0, 0, 1, &[bytes("output")]), 20);
    assert_eq!(shown(&mut transport)[0].bytes, b"output");
    transport.receive(
        &frame_datagram(1, 1, 2, &[resize(99, 30), bytes("narrow")]),
        30,
    );
    // It goes with the ack of the output, within mosh's ACK_DELAY.
    transport.tick(120);
    let restoring = sent(&mut transport).pop().unwrap();
    assert_eq!(
        decode_user_message(&restoring.diff).unwrap().last(),
        Some(&UserEvent::Resize {
            width: 100,
            height: 30
        })
    );
}

/// A real resize repaints anyway: it ends a redraw, and a narrow frame that
/// shows up afterwards is shown like any other.
#[test]
fn a_resize_cancels_a_redraw() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.request_full_redraw(10);
    transport.resize(120, 40, 20);
    sent(&mut transport);
    transport.receive(
        &frame_datagram(0, 0, 1, &[resize(79, 24), bytes("narrow")]),
        30,
    );
    assert_eq!(shown(&mut transport)[0].bytes, b"narrow");
    assert!(
        sent(&mut transport)
            .iter()
            .all(|i| !decode_user_message(&i.diff)
                .unwrap()
                .ends_with(&[UserEvent::Resize {
                    width: 80,
                    height: 24
                }]))
    );
}

/// Frames are hidden for at most the limit, whatever the server does.
#[test]
fn a_redraw_hides_frames_for_a_bounded_time() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(80, 24, 0);
    transport.request_full_redraw(0);
    transport.receive(
        &frame_datagram(0, 0, 1, &[resize(79, 24), bytes("narrow")]),
        100,
    );
    transport.receive(&frame_datagram(1, 1, 2, &[bytes("hidden")]), 5_100);
    assert!(shown(&mut transport).is_empty());
    transport.receive(&frame_datagram(2, 2, 3, &[bytes("late")]), 5_101);
    assert_eq!(shown(&mut transport)[0].bytes, b"late");
}

/// Two columns is as narrow as it goes: a redraw there widens by one.
#[test]
fn a_redraw_at_two_columns_widens() {
    let mut transport = Transport::new(&KEY, TransportConfig::default()).unwrap();
    transport.start(2, 5, 0);
    transport.request_full_redraw(0);
    let nudge = sent(&mut transport).pop().unwrap();
    assert_eq!(
        decode_user_message(&nudge.diff).unwrap().last(),
        Some(&UserEvent::Resize {
            width: 3,
            height: 5
        })
    );
}
