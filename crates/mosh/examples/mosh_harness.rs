//! The mosh harness's command line over this crate, which `harness/live.py`
//! drives against a real mosh-server. (A Swift twin over the app's mosh
//! client printed the same formats until that client was deleted, E2.)
//!
//!   mosh_harness wire <cases> [<out>]
//!   mosh_harness ssp <scenario> [<out>]
//!   mosh_harness bench <what> <size> <iterations>
//!   mosh_harness live <host> <port> <key> <ttl-ms> <steps...>
//!   mosh_harness gen-wire <seed> <count> <max-len>     seeded wire cases
//!   mosh_harness gen-ssp <seed> <steps>                a seeded scenario
//!   mosh_harness session-cases <session.txt>           pipe.recv cases
//!   mosh_harness session-ssp <session.txt>             a replay scenario

use std::io::Write;
use std::net::UdpSocket;
use std::process::exit;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moshpit_mosh::harness::{self, hex, unhex};
use moshpit_mosh::wire::{
    Fragment, FragmentAssembler, HostInstruction, HostMessage, Instruction, UserEvent,
    encode_host_message, encode_user_message, fragment,
};
use moshpit_mosh::{ChannelState, Direction, Event, Ocb3, Session, Transport, TransportConfig};
use moshpit_mosh::{bootstrap, compress};

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    exit(2)
}

fn wall_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

/// A live session over a real UDP socket, steps as in the Swift harness:
/// wait:MS, type:HEX, typeeach:COUNT:INTERVAL_MS (byte i is 0x21 + i % 94),
/// pastefile:PATH (the file's bytes as one input), redraw,
/// resize:COLS:ROWS, mark:LABEL, rebuild (a forced resume: a new socket).
fn live(args: &[String]) {
    if args.len() < 4 {
        fail("usage: live <host> <port> <key> <ttl-ms> <steps...>");
    }
    let (host, port) = (&args[0], &args[1]);
    let credentials = bootstrap::parse(&format!("MOSH CONNECT {port} {}\n", args[2]), host)
        .unwrap_or_else(|error| fail(&error.to_string()));
    let ttl: u64 = args[3].parse().unwrap_or_else(|_| fail("bad ttl"));
    let config = TransportConfig {
        keystroke_ttl_ms: ttl,
        ..TransportConfig::default()
    };
    let mut transport = Transport::new(&credentials.key, config).unwrap();
    let connect = || {
        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        socket
            .connect((credentials.host.as_str(), credentials.udp_port))
            .unwrap();
        socket
    };
    let mut socket = connect();
    let epoch = Instant::now();
    let now = |epoch: Instant| epoch.elapsed().as_millis() as u64 + 1_000;
    let stdout = std::io::stdout();
    let emit = |line: String| {
        let mut out = stdout.lock();
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    };

    // Expand the steps into a schedule of (due, action).
    enum Action {
        Type(Vec<u8>),
        Paste(Vec<u8>),
        Redraw,
        Resize(i32, i32),
        Mark(String),
        Rebuild,
    }
    let mut schedule = Vec::new();
    let mut at = 0u64;
    for step in &args[4..] {
        let fields: Vec<&str> = step.split(':').collect();
        match fields[0] {
            "wait" => at += fields[1].parse::<u64>().unwrap(),
            "type" => schedule.push((at, Action::Type(unhex(fields[1]).unwrap()))),
            "typeeach" => {
                let (count, interval): (usize, u64) =
                    (fields[1].parse().unwrap(), fields[2].parse().unwrap());
                for i in 0..count {
                    schedule.push((at, Action::Type(vec![0x21 + (i % 94) as u8])));
                    at += interval;
                }
            }
            "pastefile" => {
                schedule.push((
                    at,
                    Action::Paste(std::fs::read(fields[1]).unwrap_or_else(|error| {
                        fail(&format!("cannot read {}: {error}", fields[1]))
                    })),
                ))
            }
            "redraw" => schedule.push((at, Action::Redraw)),
            "resize" => schedule.push((
                at,
                Action::Resize(fields[1].parse().unwrap(), fields[2].parse().unwrap()),
            )),
            "mark" => schedule.push((at, Action::Mark(fields[1].to_owned()))),
            "rebuild" => schedule.push((at, Action::Rebuild)),
            _ => fail(&format!("unknown step {step}")),
        }
    }
    let end = at;
    let start = now(epoch);
    emit(format!("start {}", wall_ms()));
    transport.start(80, 24, start);
    transport.on_channel_state(ChannelState::Ready, start);
    let mut next = 0;
    let mut buffer = [0u8; 4096];
    loop {
        while let Some(event) = transport.poll_event() {
            match event {
                Event::Transmit(datagram) => {
                    let _ = socket.send(&datagram);
                }
                Event::HostFrame(frame) => emit(format!(
                    "frame {} {} {}",
                    wall_ms(),
                    frame.echo_ack,
                    hex(&frame.bytes)
                )),
                Event::RebuildFlow => {
                    socket = connect();
                    transport.on_channel_state(ChannelState::Ready, now(epoch));
                    emit(format!("rebuild {}", wall_ms()));
                }
                Event::ReturnPathDead => emit(format!("returnpathdead {}", wall_ms())),
                Event::SessionEnded => emit(format!("ended {}", wall_ms())),
                Event::RestartFlow | Event::Acked(_) | Event::Metrics { .. } => {}
                Event::Finished => {}
            }
        }
        if transport.session_ended() {
            break;
        }
        let t = now(epoch);
        if next < schedule.len() && schedule[next].0 + start <= t {
            match &schedule[next].1 {
                Action::Type(keys) => {
                    let num = transport.send(keys, t).unwrap_or(0);
                    emit(format!("sent {} {num} {}", wall_ms(), hex(keys)));
                }
                Action::Paste(data) => {
                    let num = transport.send(data, t).unwrap_or(0);
                    emit(format!("paste {} {num} {}", wall_ms(), data.len()));
                }
                Action::Redraw => {
                    transport.request_full_redraw(t);
                    emit(format!("redraw {}", wall_ms()));
                }
                Action::Resize(cols, rows) => {
                    transport.resize(*cols, *rows, t);
                    emit(format!("resize {} {cols} {rows}", wall_ms()));
                }
                Action::Mark(label) => emit(format!("mark {} {label}", wall_ms())),
                Action::Rebuild => transport.resume(true, t),
            }
            next += 1;
            continue;
        }
        if next >= schedule.len() && t >= start + end {
            break;
        }
        let due_step = schedule
            .get(next)
            .map_or(start + end, |(due, _)| start + due);
        let due = transport
            .next_deadline()
            .map_or(due_step, |d| d.min(due_step));
        let wait = due.saturating_sub(t).clamp(1, 50);
        socket
            .set_read_timeout(Some(Duration::from_millis(wait)))
            .unwrap();
        match socket.recv(&mut buffer) {
            Ok(len) => transport.receive(&buffer[..len], now(epoch)),
            Err(_) => transport.on_timer(now(epoch)),
        }
        transport.on_timer(now(epoch));
    }
    let d = transport.diagnostics();
    emit(format!(
        "diag received={} applied={} gaps={} parseFailures={}",
        d.datagrams_received, d.applied_host_num, d.gap_events, d.parse_failures
    ));
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| fail(&format!("{path}: {error}")))
}

fn write_out(args: &[String], text: &str) {
    match args.get(1) {
        Some(path) => std::fs::write(path, text).unwrap(),
        None => std::io::stdout().write_all(text.as_bytes()).unwrap(),
    }
}

fn wire(args: &[String]) {
    let input = read(
        args.first()
            .unwrap_or_else(|| fail("usage: wire <cases> [<out>]")),
    );
    let mut out = String::new();
    for line in input.lines() {
        if let Some(result) = harness::wire::run(line) {
            out.push_str(&result);
            out.push('\n');
        }
    }
    write_out(args, &out);
}

fn ssp(args: &[String]) {
    let input = read(
        args.first()
            .unwrap_or_else(|| fail("usage: ssp <scenario> [<out>]")),
    );
    write_out(args, &harness::ssp::run(&input));
}

/// Keystroke-like input: shell commands, repeated to `size` bytes.
fn bench_input(size: usize) -> Vec<u8> {
    let text = b"git status && ls -la /usr/local/bin | head -20\r";
    (0..size).map(|i| text[i % text.len()]).collect()
}

/// Terminal-like output: 80-column rows of printable bytes.
fn bench_output(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| {
            if i % 80 == 79 {
                0x0A
            } else {
                0x20 + ((i * 7 + i / 80) % 95) as u8
            }
        })
        .collect()
}

/// The same work as the Swift harness's bench, step for step.
fn bench(args: &[String]) {
    let (Some(what), Some(size), Some(iterations)) = (
        args.first(),
        args.get(1).and_then(|n| n.parse::<usize>().ok()),
        args.get(2).and_then(|n| n.parse::<usize>().ok()),
    ) else {
        fail("usage: bench send|recv|seal <size> <iterations>")
    };
    let key: Vec<u8> = (0..16).collect();
    let session = Session::new(&key).unwrap();
    let mut sink = 0usize;
    let start = Instant::now();
    match what.as_str() {
        "send" => {
            // The client's send path: encode, compress, fragment, seal.
            let diff = encode_user_message(&[UserEvent::Keystroke(bench_input(size))]);
            let mut seq = 0;
            for n in 0..iterations as u64 {
                let instruction = Instruction {
                    old_num: n,
                    new_num: n + 1,
                    ack_num: 7,
                    diff: diff.clone(),
                    ..Instruction::default()
                };
                let compressed = compress::compress(&instruction.encode());
                for piece in fragment(&compressed, n, 1300) {
                    sink = sink.wrapping_add(
                        session
                            .seal(seq, Direction::ToServer, 1, 2, &piece.encode())
                            .len(),
                    );
                    seq += 1;
                }
            }
        }
        "recv" => {
            // The client's receive path for one server diff.
            let instruction = Instruction {
                old_num: 1,
                new_num: 2,
                diff: encode_host_message(&[HostInstruction::HostBytes(bench_output(size))]),
                ..Instruction::default()
            };
            let compressed = compress::compress(&instruction.encode());
            let packets: Vec<Vec<u8>> = fragment(&compressed, 1, 1300)
                .iter()
                .enumerate()
                .map(|(index, piece)| {
                    session.seal(index as u64, Direction::ToClient, 1, 2, &piece.encode())
                })
                .collect();
            for _ in 0..iterations {
                let mut assembler = FragmentAssembler::new();
                for packet in &packets {
                    let opened = session.open(packet).unwrap();
                    if let Some(payload) =
                        assembler.receive(Fragment::parse(&opened.payload).unwrap())
                    {
                        let i =
                            Instruction::decode(&compress::decompress(&payload).unwrap()).unwrap();
                        sink = sink
                            .wrapping_add(HostMessage::decode(&i.diff).unwrap().host_bytes.len());
                    }
                }
            }
        }
        "seal" => {
            let ocb = Ocb3::new(&key).unwrap();
            let payload = bench_input(size);
            for _ in 0..iterations {
                sink = sink.wrapping_add(ocb.seal(&payload, &[0; 12], &[]).unwrap().len());
            }
        }
        _ => fail("bench send|recv|seal"),
    }
    let per_op = start.elapsed().as_nanos() as f64 / iterations as f64;
    println!(
        "bench {what} size={size} iterations={iterations} ns_per_op={per_op:.0} mb_per_s={:.1} sink={}",
        size as f64 / per_op * 1000.0,
        sink & 1
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(mode) = args.first() else {
        fail(
            "usage: mosh_harness wire|ssp|bench|live|gen-wire|gen-ssp|session-cases|session-ssp ...",
        )
    };
    let rest = &args[1..];
    let number = |index: usize| -> u64 {
        rest.get(index)
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| fail("bad number argument"))
    };
    match mode.as_str() {
        "live" => live(rest),
        "wire" => wire(rest),
        "ssp" => ssp(rest),
        "bench" => bench(rest),
        "gen-wire" => {
            for case in harness::wire::generate(number(0), number(1) as usize, number(2) as usize) {
                println!("{case}");
            }
        }
        "gen-ssp" => print!("{}", harness::ssp::generate(number(0), number(1) as usize)),
        "session-cases" => {
            for case in harness::wire::session_cases(&read(&rest[0])) {
                println!("{case}");
            }
        }
        "session-ssp" => print!("{}", harness::ssp::from_session(&read(&rest[0]))),
        _ => fail(&format!("unknown mode {mode}")),
    }
}
