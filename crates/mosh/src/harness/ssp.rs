//! SSP scenarios: a client session as steps, the same on both front ends.
//!
//! ```text
//! budget <bytes>         optional, before the key: the instruction budget
//! key <hex>              the session key
//! start <cols> <rows>
//! send <hex>             keystrokes
//! resize <cols> <rows>
//! resume                 a foreground resume (not forced)
//! resume-force           the resume after a real suspension (new flow, redraw)
//! idle <ms>              the clock moves on, then a resume (a datagram now)
//! tick <ms>              the clock moves on, then the send timer fires
//!                        (sends whatever is due: new input at frame rate,
//!                        a retransmission, an ack)
//! redraw                 a requested full redraw
//! recv <hex>             a datagram from the server
//! close
//! ```
//!
//! For each step the runner prints `step <n> <op>`, then `num <n|->` for a
//! send, then every datagram the client sent (`tx seq id num final ts tsr`,
//! and `inst old new ack thr events` once a whole instruction is in), then
//! `ended` if the server's shutdown state ended the session in that step,
//! then `diag received parseFailures gaps applied srtt` (`srtt` as the
//! f64's bits). After the last step come the host frames, `frame <echo ack>
//! <hex>`, in order.
//!
//! Both runners read a clock that starts at 1,000,000 ms and moves only on
//! `idle` and `tick` steps, so the keystroke TTL (set out of reach) never
//! runs out, the watchdog, liveness and restart timers never fire (the
//! channel is never reported ready), the send timer fires only on `tick`
//! steps, idle states (an empty user state 3 s after the last new one)
//! come only after time moved, and the timestamps are the same on both
//! sides. Outputs must be identical.

use std::fmt::Write;

use super::wire::format_events;
use super::{SplitMix64, hex, server_datagrams, unhex};
use crate::compress;
use crate::crypto::{Direction, Session};
use crate::transport::{Event, HostFrame, Transport, TransportConfig, timestamp16};
use crate::wire::{
    Fragment, FragmentAssembler, HostInstruction, Instruction, decode_user_message,
    encode_host_message,
};

/// The runners' clock at the start, in milliseconds; only `idle` moves it.
pub const CLOCK_MS: u64 = 1_000_000;

/// Runs a scenario through the Rust transport and returns its output.
pub fn run(scenario: &str) -> String {
    let mut out = String::new();
    let mut transport: Option<Transport> = None;
    let mut observer: Option<(Session, FragmentAssembler)> = None;
    let mut frames: Vec<HostFrame> = Vec::new();
    // The clock stands still but for idle and tick steps (the Swift
    // runner's too), and the TTL never runs out.
    let mut now = CLOCK_MS;
    let mut config = TransportConfig {
        keystroke_ttl_ms: u64::MAX / 4,
        ..TransportConfig::default()
    };
    for (index, line) in scenario.lines().filter(|line| !line.is_empty()).enumerate() {
        let (op, arg) = line.split_once(' ').unwrap_or((line, ""));
        if op == "budget" {
            config.instruction_budget = arg.parse().expect("budget bytes");
            continue;
        }
        if op == "key" {
            let key = unhex(arg).expect("hex key");
            transport = Some(Transport::new(&key, config.clone()).expect("16-byte key"));
            observer = Some((Session::new(&key).unwrap(), FragmentAssembler::new()));
            continue;
        }
        let transport = transport.as_mut().expect("key first");
        let _ = writeln!(out, "step {index} {op}");
        let numbers = || -> Vec<i32> { arg.split(' ').map(|n| n.parse().unwrap()).collect() };
        match op {
            "start" => {
                let size = numbers();
                transport.start(size[0], size[1], now);
            }
            "send" => {
                let num = transport.send(&unhex(arg).expect("hex"), now);
                let _ = writeln!(out, "num {}", num.map_or("-".to_owned(), |n| n.to_string()));
            }
            "resize" => {
                let size = numbers();
                transport.resize(size[0], size[1], now);
            }
            "resume" => transport.resume(false, now),
            "resume-force" => transport.resume(true, now),
            "idle" => {
                now += arg.parse::<u64>().expect("idle ms");
                transport.resume(false, now);
            }
            "tick" => {
                now += arg.parse::<u64>().expect("tick ms");
                transport.tick(now);
            }
            "redraw" => transport.request_full_redraw(now),
            "recv" => transport.receive(&unhex(arg).expect("hex"), now),
            "close" => transport.close(),
            other => panic!("unknown step {other}"),
        }
        let (session, assembler) = observer.as_mut().unwrap();
        let mut ended = false;
        for event in transport.drain_events() {
            match event {
                Event::Transmit(datagram) => describe_sent(&mut out, session, assembler, &datagram),
                Event::HostFrame(frame) => frames.push(frame),
                Event::SessionEnded => ended = true,
                _ => {}
            }
        }
        if ended {
            out.push_str("ended\n");
        }
        let d = transport.diagnostics();
        let _ = writeln!(
            out,
            "diag received={} parseFailures={} gaps={} applied={} srtt={}",
            d.datagrams_received,
            d.parse_failures,
            d.gap_events,
            d.applied_host_num,
            transport.srtt_ms().to_bits()
        );
    }
    for frame in frames {
        let _ = writeln!(out, "frame {} {}", frame.echo_ack, hex(&frame.bytes));
    }
    out
}

/// One client datagram as a server would read it.
fn describe_sent(
    out: &mut String,
    session: &Session,
    assembler: &mut FragmentAssembler,
    datagram: &[u8],
) {
    let message = session.open(datagram).expect("client datagram opens");
    let piece = Fragment::parse(&message.payload).expect("fragment");
    let _ = writeln!(
        out,
        "tx seq={} id={} num={} final={} ts={} tsr={}",
        message.seq,
        piece.id,
        piece.num,
        u8::from(piece.is_final),
        message.timestamp,
        message.timestamp_reply
    );
    if let Some(payload) = assembler.receive(piece) {
        let instruction = Instruction::decode(&compress::decompress(&payload).expect("zlib"))
            .expect("instruction");
        let events = decode_user_message(&instruction.diff).expect("user message");
        let _ = writeln!(
            out,
            "inst old={} new={} ack={} thr={} events={}",
            instruction.old_num,
            instruction.new_num,
            instruction.ack_num,
            instruction.throwaway_num,
            format_events(&events)
        );
    }
}

/// What the simulated server has received of the client's: the newest user
/// state a whole instruction carried to it. mosh-server acks only states it
/// received, never one the client has queued but not sent yet.
struct Received {
    assembler: FragmentAssembler,
    newest_user_num: u64,
}

impl Received {
    fn new() -> Received {
        Received {
            assembler: FragmentAssembler::new(),
            newest_user_num: 0,
        }
    }

    /// Takes in every datagram `client` sent since the last call (the
    /// simulated link loses nothing on the way up).
    fn take(&mut self, session: &Session, client: &mut Transport) {
        for event in client.drain_events() {
            let Event::Transmit(datagram) = event else {
                continue;
            };
            let message = session.open(&datagram).expect("client datagram opens");
            let piece = Fragment::parse(&message.payload).expect("fragment");
            if let Some(payload) = self.assembler.receive(piece) {
                let instruction =
                    Instruction::decode(&compress::decompress(&payload).expect("zlib"))
                        .expect("instruction");
                self.newest_user_num = self.newest_user_num.max(instruction.new_num);
            }
        }
    }
}

/// A seeded scenario: a session of `steps` steps against a simulated server
/// that sends output diffs, mostly from the state the client has applied
/// (as a server reading the client's acks does), but also from older states
/// and ahead of it (gaps); it repeats and reorders datagrams, fragments large
/// diffs, sends diffs that do not decode, pure acks, garbage and datagrams in
/// the wrong direction, and acks user states it has received (any of them,
/// up to the newest the client sent: mosh-server never acks one the client
/// has only queued). The client
/// asks for redraws (and forced resumes, which redraw too); the server then
/// sends frames at the narrowed size and, later, at the real one. The client
/// idles now and then (the clock moves up to 6.5 s, then a resume), which
/// gives the server idle states, and time passes between events (`tick`:
/// the send timer fires), which sends paced input, retransmissions and
/// delayed acks. Half the scenarios run under a tiny
/// instruction budget (24 to 143 bytes), and input is now and then a few
/// hundred bytes: states split, and flushes chain instructions. Server timestamps and replies are random,
/// 0xFFFF ("none") included. The client's applied state comes from running
/// this crate's transport alongside on the runners' clock; that only steers
/// which inputs are made, both implementations then get them.
pub fn generate(seed: u64, steps: usize) -> String {
    let mut rng = SplitMix64(seed);
    let key = rng.bytes(16);
    let session = Session::new(&key).unwrap();
    let mut config = TransportConfig {
        keystroke_ttl_ms: u64::MAX / 4,
        ..TransportConfig::default()
    };
    let mut out = String::new();
    if rng.below(2) == 0 {
        config.instruction_budget = 24 + rng.below(120) as usize;
        let _ = writeln!(out, "budget {}", config.instruction_budget);
    }
    let mut client = Transport::new(&key, config).unwrap();
    let _ = writeln!(out, "key {}", hex(&key));
    let (mut cols, mut rows) = (80 + rng.below(40) as i32, 24 + rng.below(20) as i32);
    let _ = writeln!(out, "start {cols} {rows}");
    let mut now = CLOCK_MS;
    client.start(cols, rows, now);
    let mut host_num = 0u64;
    let mut sent_states = vec![0u64];
    let mut seq = 0u64;
    let mut id = 1u64;
    let mut echo = 0u64;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut history: Vec<Vec<u8>> = Vec::new();
    // The narrowed size of a redraw in progress, and whether the server has
    // sent a frame at it (then the real size comes back).
    let mut redraw: Option<((i32, i32), bool)> = None;
    let mut received = Received::new();
    for _ in 0..steps {
        received.take(&session, &mut client);
        match rng.below(27) {
            0..=3 => {
                let n = if rng.below(8) == 0 {
                    rng.below(300)
                } else {
                    rng.below(6)
                } as usize;
                let keys: Vec<u8> = (0..n).map(|_| 0x20 + rng.below(95) as u8).collect();
                client.send(&keys, now);
                let _ = writeln!(out, "send {}", hex(&keys));
            }
            4 => {
                if rng.below(2) == 0 {
                    cols = 20 + rng.below(200) as i32;
                    rows = 5 + rng.below(80) as i32;
                }
                client.resize(cols, rows, now);
                let _ = writeln!(out, "resize {cols} {rows}");
            }
            5 => {
                client.resume(false, now);
                out.push_str("resume\n");
            }
            6 | 7 => {
                if rng.below(2) == 0 {
                    client.request_full_redraw(now);
                    out.push_str("redraw\n");
                } else {
                    client.resume(true, now);
                    out.push_str("resume-force\n");
                }
                let width = if cols > 2 { cols - 1 } else { cols + 1 };
                redraw = Some(((width, rows), false));
            }
            8 => {
                let ms = 500 + rng.below(6_000);
                now += ms;
                client.resume(false, now);
                let _ = writeln!(out, "idle {ms}");
            }
            9..=12 => {
                // Time between events, mostly under a frame or a
                // retransmission timeout, now and then past several.
                let ms = if rng.below(6) == 0 {
                    400 + rng.below(2_000)
                } else {
                    1 + rng.below(300)
                };
                now += ms;
                client.tick(now);
                let _ = writeln!(out, "tick {ms}");
            }
            _ => {
                if pending.is_empty() || rng.below(3) != 0 {
                    // The server renders a new state.
                    let applied = client.diagnostics().applied_host_num;
                    let old = match rng.below(12) {
                        0 => sent_states[rng.below(sent_states.len() as u64) as usize],
                        1 => host_num + 1 + rng.below(3),
                        2 => host_num,
                        _ => applied,
                    };
                    let new = old.max(host_num) + 1 + rng.below(2);
                    let diff = match rng.below(12) {
                        0 => Vec::new(),
                        1 => vec![0x0A, 0x05, 0x01],
                        _ => {
                            let mut instructions = Vec::new();
                            if rng.below(3) == 0 {
                                echo += rng.below(3);
                                instructions.push(HostInstruction::EchoAck(echo));
                            }
                            match redraw {
                                Some((narrow, false)) if rng.below(2) == 0 => {
                                    instructions.push(HostInstruction::Resize {
                                        width: narrow.0,
                                        height: narrow.1,
                                    });
                                    redraw = Some((narrow, true));
                                }
                                Some((_, true)) if rng.below(2) == 0 => {
                                    instructions.push(HostInstruction::Resize {
                                        width: cols,
                                        height: rows,
                                    });
                                    redraw = None;
                                }
                                _ if rng.below(10) == 0 => {
                                    instructions.push(HostInstruction::Resize {
                                        width: cols,
                                        height: rows,
                                    });
                                }
                                _ => {}
                            }
                            let len = if rng.below(8) == 0 {
                                1_500 + rng.below(3_000)
                            } else {
                                rng.below(80)
                            };
                            // Incompressible bytes, so long diffs fragment.
                            instructions.push(HostInstruction::HostBytes(rng.bytes(len as usize)));
                            encode_host_message(&instructions)
                        }
                    };
                    let instruction = Instruction {
                        old_num: old,
                        new_num: new,
                        ack_num: rng.below(received.newest_user_num + 1),
                        throwaway_num: old,
                        diff,
                        chaff: rng.bytes_below(17),
                        ..Instruction::default()
                    };
                    host_num = new;
                    sent_states.push(new);
                    // Both runners' clocks read `now`: a reply up to 7 s
                    // behind it is a sample under 5 s or one too large.
                    let timestamp = match rng.below(8) {
                        0 => u16::MAX,
                        _ => rng.below(65_536) as u16,
                    };
                    let reply = match rng.below(3) {
                        0 => u16::MAX,
                        _ => timestamp16(now).wrapping_sub(rng.below(7_000) as u16),
                    };
                    let datagrams =
                        server_datagrams(&session, seq, id, &instruction, timestamp, reply, 1_300);
                    seq += datagrams.len() as u64;
                    id += 1;
                    pending.extend(datagrams);
                    if pending.len() > 1 && rng.below(5) == 0 {
                        let a = rng.below(pending.len() as u64) as usize;
                        let b = rng.below(pending.len() as u64) as usize;
                        pending.swap(a, b);
                    }
                }
                let datagram = match rng.below(14) {
                    0 if !history.is_empty() => {
                        history[rng.below(history.len() as u64) as usize].clone()
                    }
                    1 => rng.bytes_below(60),
                    2 => session.seal(seq, Direction::ToServer, 0, u16::MAX, &rng.bytes(20)),
                    _ => pending.remove(0),
                };
                client.receive(&datagram, now);
                history.push(datagram.clone());
                let _ = writeln!(out, "recv {}", hex(&datagram));
            }
        }
    }
    received.take(&session, &mut client);
    for datagram in pending {
        client.receive(&datagram, now);
        received.take(&session, &mut client);
        let _ = writeln!(out, "recv {}", hex(&datagram));
    }
    // A third of the sessions end the way mosh-server ends one: its shutdown
    // state, from the state the client holds (now and then from one it does
    // not), with the command's last output or none; the client acks it a
    // frame later and is done, so what follows sends nothing.
    if rng.below(3) == 0 {
        let applied = client.diagnostics().applied_host_num;
        let old = if rng.below(4) == 0 {
            applied + 1
        } else {
            applied
        };
        let diff = if rng.below(2) == 0 {
            encode_host_message(&[HostInstruction::HostBytes(b"logout\r\n".to_vec())])
        } else {
            Vec::new()
        };
        let instruction = Instruction {
            old_num: old,
            new_num: crate::transport::SHUTDOWN_STATE,
            ack_num: received.newest_user_num,
            throwaway_num: old,
            diff,
            ..Instruction::default()
        };
        for datagram in server_datagrams(&session, seq, id, &instruction, 0, u16::MAX, 1_300) {
            client.receive(&datagram, now);
            let _ = writeln!(out, "recv {}", hex(&datagram));
        }
        out.push_str("tick 300\nsend 78\ntick 300\nresume\nidle 4000\n");
    }
    out.push_str("close\n");
    out
}

/// A scenario replaying the server's side of a recorded session
/// (`tests/fixtures/live/session-*.txt`) into a client.
pub fn from_session(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("key", b64)) => {
                let creds = crate::bootstrap::parse(&format!("MOSH CONNECT 1 {b64}\n"), "")
                    .expect("recorded key");
                let _ = writeln!(out, "key {}\nstart 80 24", hex(&creds.key));
            }
            Some(("down", data)) => {
                let _ = writeln!(out, "recv {data}");
            }
            _ => {}
        }
    }
    out.push_str("close\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ack each server datagram of `scenario` carries, by step index
    /// (`None`: not a whole server instruction).
    fn server_acks(scenario: &str) -> Vec<(usize, u64)> {
        let mut session = None;
        let mut assembler = FragmentAssembler::new();
        let mut acks = Vec::new();
        for (index, line) in scenario.lines().filter(|l| !l.is_empty()).enumerate() {
            let (op, arg) = line.split_once(' ').unwrap_or((line, ""));
            match op {
                "key" => session = Some(Session::new(&unhex(arg).unwrap()).unwrap()),
                "recv" => {
                    let session = session.as_ref().unwrap();
                    let Ok(message) = session.open(&unhex(arg).unwrap()) else {
                        continue;
                    };
                    if message.direction != Direction::ToClient {
                        continue;
                    }
                    let Ok(piece) = Fragment::parse(&message.payload) else {
                        continue;
                    };
                    let Some(payload) = assembler.receive(piece) else {
                        continue;
                    };
                    let Some(instruction) = compress::decompress(&payload)
                        .ok()
                        .and_then(|bytes| Instruction::decode(&bytes).ok())
                    else {
                        continue;
                    };
                    acks.push((index, instruction.ack_num));
                }
                _ => {}
            }
        }
        acks
    }

    #[test]
    fn the_server_acks_only_user_states_the_client_sent() {
        let mut checked = 0;
        for seed in 1..=60 {
            let scenario = generate(seed, 300);
            let output = run(&scenario);
            // The newest user state sent before each step: the client's own
            // instructions, as the runner prints them.
            let mut sent_before = Vec::new();
            let mut newest = 0u64;
            for line in output.lines() {
                if line.starts_with("step ") {
                    sent_before.push(newest);
                } else if let Some(rest) = line.strip_prefix("inst ") {
                    let new = rest
                        .split(' ')
                        .find_map(|field| field.strip_prefix("new="))
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap();
                    newest = newest.max(new);
                }
            }
            // Step indices count the budget and key lines, which print no step.
            let skipped = scenario
                .lines()
                .take_while(|l| l.starts_with("budget") || l.starts_with("key"))
                .count();
            for (index, ack) in server_acks(&scenario) {
                let sent = sent_before[index - skipped];
                assert!(
                    ack <= sent,
                    "seed {seed}: step {index} acks user state {ack}, the client had sent up to {sent}"
                );
                checked += 1;
            }
        }
        assert!(checked > 1_000, "only {checked} acks checked");
    }
}
