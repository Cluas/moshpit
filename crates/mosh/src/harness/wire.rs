//! Wire cases: one operation per line, one result line each, the same on
//! both front ends. Fields are `name=value`, bytes in lower-case hex, numbers
//! in decimal, `-` for none; lists are comma-separated (`-` when empty).
//!
//! | case | result |
//! | --- | --- |
//! | `ocb.seal key nonce ad pt` | `ok ct` |
//! | `ocb.open key nonce ad ct` | `ok pt` |
//! | `crypto.seal key seq dir ts tsr payload` | `ok packet` |
//! | `crypto.open key packet` | `ok seq dir ts tsr payload` |
//! | `zlib.compress data` | `ok head tail roundtrip out` (`out` is not compared) |
//! | `zlib.decompress data` | `ok data` |
//! | `adler data` | `ok value` |
//! | `inst.encode v old new ack thr diff chaff` | `ok bytes` |
//! | `inst.decode data` | `ok v old new ack thr diff chaff` |
//! | `user.encode events` | `ok bytes` |
//! | `user.decode data` | `ok events` |
//! | `host.decode data` | `ok bytes resize echo` |
//! | `frag.encode id num final contents` | `ok bytes` |
//! | `frag.parse data` | `ok id num final contents` |
//! | `frag.split payload id mtu` | `ok frags` |
//! | `frag.assemble frags` | `ok out` (`-` per incomplete feed) |
//! | `boot.parse output host` | `ok port key pid` |
//! | `boot.command bin locale start end` | `ok command` |
//! | `boot.excerpt output` | `ok excerpt` |
//! | `pipe.send key seq id mtu ts tsr zlib` | `ok packets` |
//! | `pipe.recv key packets` | `ok out` (one item per packet, `;`-separated) |
//!
//! Failures are `err <name>`, with the Swift error case names. User events
//! are `k<hex>` (keystroke) and `r<w>x<h>` (resize).

use std::collections::BTreeMap;

use super::{SplitMix64, hex, unhex};
use crate::bootstrap::{self, BootstrapError};
use crate::compress::{self, CompressionError};
use crate::crypto::{Direction, OpenError, Session};
use crate::ocb::{Ocb3, OcbError};
use crate::wire::{
    Fragment, FragmentAssembler, HostInstruction, HostMessage, Instruction, UserEvent, WireError,
    decode_user_message, encode_host_message, encode_user_message, fragment,
};

/// Runs one case line; `None` for blank and `#` comment lines.
pub fn run(line: &str) -> Option<String> {
    let line = line.trim_end();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut tokens = line.split(' ');
    let op = tokens.next().unwrap_or_default();
    let fields: BTreeMap<&str, &str> = tokens.filter_map(|token| token.split_once('=')).collect();
    let bytes = |name: &str| unhex(fields.get(name).copied().unwrap_or("")).expect("hex field");
    let number = |name: &str| -> u64 { fields[name].parse().expect("number field") };
    let optional = |name: &str| -> Option<i64> {
        match fields[name] {
            "-" => None,
            value => Some(value.parse().expect("number field")),
        }
    };
    let result = match op {
        "ocb.seal" => Ocb3::new(&bytes("key"))
            .and_then(|ocb| ocb.seal(&bytes("pt"), &bytes("nonce"), &bytes("ad")))
            .map(|ct| format!("ct={}", hex(&ct)))
            .map_err(ocb_error),
        "ocb.open" => Ocb3::new(&bytes("key"))
            .and_then(|ocb| ocb.open(&bytes("ct"), &bytes("nonce"), &bytes("ad")))
            .map(|pt| format!("pt={}", hex(&pt)))
            .map_err(ocb_error),
        "crypto.seal" => Session::new(&bytes("key"))
            .map_err(ocb_error)
            .map(|session| {
                let packet = session.seal(
                    number("seq"),
                    direction(number("dir")),
                    number("ts") as u16,
                    number("tsr") as u16,
                    &bytes("payload"),
                );
                format!("packet={}", hex(&packet))
            }),
        "crypto.open" => Session::new(&bytes("key"))
            .map_err(ocb_error)
            .and_then(|session| session.open(&bytes("packet")).map_err(open_error))
            .map(|message| {
                format!(
                    "seq={} dir={} ts={} tsr={} payload={}",
                    message.seq,
                    dir_number(message.direction),
                    message.timestamp,
                    message.timestamp_reply,
                    hex(&message.payload)
                )
            }),
        "zlib.compress" => {
            let data = bytes("data");
            let out = compress::compress(&data);
            let roundtrip = compress::decompress(&out).ok() == Some(data);
            Ok(format!(
                "head={} tail={} roundtrip={} out={}",
                hex(&out[..2]),
                hex(&out[out.len() - 4..]),
                u8::from(roundtrip),
                hex(&out)
            ))
        }
        "zlib.decompress" => compress::decompress(&bytes("data"))
            .map(|data| format!("data={}", hex(&data)))
            .map_err(compression_error),
        "adler" => Ok(format!("{:08x}", compress::adler32(&bytes("data")))),
        "inst.encode" => {
            let instruction = Instruction {
                protocol_version: number("v") as u32,
                old_num: number("old"),
                new_num: number("new"),
                ack_num: number("ack"),
                throwaway_num: number("thr"),
                diff: bytes("diff"),
                chaff: bytes("chaff"),
            };
            Ok(format!("bytes={}", hex(&instruction.encode())))
        }
        "inst.decode" => Instruction::decode(&bytes("data"))
            .map(|i| instruction_fields(&i))
            .map_err(wire_error),
        "user.encode" => Ok(format!(
            "bytes={}",
            hex(&encode_user_message(&parse_events(fields["events"])))
        )),
        "user.decode" => decode_user_message(&bytes("data"))
            .map(|events| format!("events={}", format_events(&events)))
            .map_err(wire_error),
        "host.decode" => HostMessage::decode(&bytes("data"))
            .map(|host| host_fields(&host))
            .map_err(wire_error),
        "frag.encode" => {
            let piece = Fragment::new(
                number("id"),
                number("num") as u16,
                number("final") != 0,
                bytes("contents"),
            );
            Ok(format!("bytes={}", hex(&piece.encode())))
        }
        "frag.parse" => Fragment::parse(&bytes("data"))
            .map(|piece| {
                format!(
                    "id={} num={} final={} contents={}",
                    piece.id,
                    piece.num,
                    u8::from(piece.is_final),
                    hex(&piece.contents)
                )
            })
            .map_err(wire_error),
        "frag.split" => {
            let pieces = fragment(&bytes("payload"), number("id"), number("mtu") as usize);
            let encoded: Vec<String> = pieces.iter().map(|piece| hex(&piece.encode())).collect();
            Ok(format!("frags={}", list(&encoded)))
        }
        "frag.assemble" => {
            let mut assembler = FragmentAssembler::new();
            let out: Vec<String> = split_list(fields["frags"])
                .iter()
                .map(|item| match Fragment::parse(&unhex(item).expect("hex")) {
                    Err(error) => format!("e:{}", wire_error(error)),
                    Ok(piece) => assembler
                        .receive(piece)
                        .map_or_else(|| "-".to_owned(), |payload| format!("p{}", hex(&payload))),
                })
                .collect();
            Ok(format!("out={}", out.join(",")))
        }
        "boot.parse" => {
            let host = String::from_utf8_lossy(&bytes("host")).into_owned();
            bootstrap::parse_bytes(&bytes("output"), &host)
                .map(|creds| {
                    format!(
                        "port={} key={} pid={}",
                        creds.udp_port,
                        hex(&creds.key),
                        creds
                            .server_pid
                            .map_or_else(|| "-".to_owned(), |pid| pid.to_string())
                    )
                })
                .map_err(boot_error)
        }
        "boot.command" => {
            let command = bootstrap::command(
                &String::from_utf8_lossy(&bytes("bin")),
                &String::from_utf8_lossy(&bytes("locale")),
                optional("start"),
                optional("end"),
            );
            Ok(hex(command.as_bytes()))
        }
        "boot.excerpt" => {
            let output = String::from_utf8_lossy(&bytes("output")).into_owned();
            Ok(hex(bootstrap::sanitized_excerpt(&output).as_bytes()))
        }
        "pipe.send" => Session::new(&bytes("key"))
            .map_err(ocb_error)
            .map(|session| {
                let seq = number("seq");
                let pieces = fragment(&bytes("zlib"), number("id"), number("mtu") as usize);
                let packets: Vec<String> = pieces
                    .iter()
                    .enumerate()
                    .map(|(index, piece)| {
                        hex(&session.seal(
                            seq + index as u64,
                            Direction::ToServer,
                            number("ts") as u16,
                            number("tsr") as u16,
                            &piece.encode(),
                        ))
                    })
                    .collect();
                format!("packets={}", list(&packets))
            }),
        "pipe.recv" => Session::new(&bytes("key"))
            .map_err(ocb_error)
            .map(|session| format!("out={}", receive_pipeline(&session, fields["packets"]))),
        other => Err(format!("unknown-op:{other}")),
    };
    Some(match result {
        Ok(fields) => format!("{op} ok {fields}"),
        Err(name) => format!("{op} err {name}"),
    })
}

/// The client's receive path minus the state machine: open, fragment,
/// reassemble, inflate, decode; the diff as host output (from the server)
/// or user events (from a client).
fn receive_pipeline(session: &Session, packets: &str) -> String {
    let mut assembler = FragmentAssembler::new();
    let mut out = Vec::new();
    for item in split_list(packets) {
        let packet = unhex(item).expect("hex");
        let message = match session.open(&packet) {
            Ok(message) => message,
            Err(error) => {
                out.push(format!("x:{}", open_error(error)));
                continue;
            }
        };
        let head = format!(
            "{}:{}:{}:{}",
            message.seq,
            dir_number(message.direction),
            message.timestamp,
            message.timestamp_reply
        );
        let piece = match Fragment::parse(&message.payload) {
            Ok(piece) => piece,
            Err(error) => {
                out.push(format!("f:{head}:{}", wire_error(error)));
                continue;
            }
        };
        let Some(payload) = assembler.receive(piece) else {
            out.push(format!("p:{head}"));
            continue;
        };
        let bytes = match compress::decompress(&payload) {
            Ok(bytes) => bytes,
            Err(error) => {
                out.push(format!("z:{head}:{}", compression_error(error)));
                continue;
            }
        };
        let instruction = match Instruction::decode(&bytes) {
            Ok(instruction) => instruction,
            Err(error) => {
                out.push(format!("i:{head}:{}", wire_error(error)));
                continue;
            }
        };
        let diff = if instruction.diff.is_empty() {
            "-".to_owned()
        } else if message.direction == Direction::ToClient {
            match HostMessage::decode(&instruction.diff) {
                Ok(host) => format!("h:{}", host_fields(&host).replace(' ', "/")),
                Err(error) => format!("he:{}", wire_error(error)),
            }
        } else {
            match decode_user_message(&instruction.diff) {
                Ok(events) => format!("u:{}", format_events(&events)),
                Err(error) => format!("ue:{}", wire_error(error)),
            }
        };
        out.push(format!(
            "I:{head}:{}:{}",
            instruction_fields(&instruction).replace(' ', "/"),
            diff
        ));
    }
    out.join(";")
}

fn instruction_fields(i: &Instruction) -> String {
    format!(
        "v={} old={} new={} ack={} thr={} diff={} chaff={}",
        i.protocol_version,
        i.old_num,
        i.new_num,
        i.ack_num,
        i.throwaway_num,
        hex(&i.diff),
        hex(&i.chaff)
    )
}

fn host_fields(host: &HostMessage) -> String {
    format!(
        "bytes={} resize={} echo={}",
        hex(&host.host_bytes),
        host.resize
            .map_or_else(|| "-".to_owned(), |(w, h)| format!("{w}x{h}")),
        host.echo_ack
            .map_or_else(|| "-".to_owned(), |echo| echo.to_string())
    )
}

fn direction(number: u64) -> Direction {
    if number == 0 {
        Direction::ToServer
    } else {
        Direction::ToClient
    }
}

fn dir_number(direction: Direction) -> u8 {
    match direction {
        Direction::ToServer => 0,
        Direction::ToClient => 1,
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.join(",")
    }
}

fn split_list(text: &str) -> Vec<&str> {
    if text == "-" || text.is_empty() {
        Vec::new()
    } else {
        text.split(',').collect()
    }
}

/// `k<hex>` and `r<w>x<h>` items.
pub fn format_events(events: &[UserEvent]) -> String {
    let items: Vec<String> = events
        .iter()
        .map(|event| match event {
            UserEvent::Keystroke(keys) => format!("k{}", hex(keys)),
            UserEvent::Resize { width, height } => format!("r{width}x{height}"),
        })
        .collect();
    list(&items)
}

pub fn parse_events(text: &str) -> Vec<UserEvent> {
    split_list(text)
        .into_iter()
        .map(|item| match item.split_at(1) {
            ("k", keys) => UserEvent::Keystroke(unhex(keys).expect("hex")),
            ("r", size) => {
                let (w, h) = size.split_once('x').expect("WxH");
                UserEvent::Resize {
                    width: w.parse().expect("width"),
                    height: h.parse().expect("height"),
                }
            }
            _ => panic!("bad event {item}"),
        })
        .collect()
}

fn ocb_error(error: OcbError) -> String {
    match error {
        OcbError::InvalidKeyLength => "invalidKeyLength",
        OcbError::InvalidNonceLength => "invalidNonceLength",
        OcbError::CiphertextTooShort => "ciphertextTooShort",
        OcbError::AuthenticationFailed => "authenticationFailed",
        OcbError::MessageTooLong => "messageTooLong",
    }
    .to_owned()
}

fn open_error(error: OpenError) -> String {
    match error {
        OpenError::PacketTooShort => "packetTooShort",
        OpenError::AuthenticationFailed => "authenticationFailed",
    }
    .to_owned()
}

fn compression_error(error: CompressionError) -> String {
    match error {
        CompressionError::BadHeader => "badHeader",
        CompressionError::InflateFailed => "inflateFailed",
        CompressionError::BadChecksum => "badChecksum",
        CompressionError::TooLarge => "tooLarge",
    }
    .to_owned()
}

fn wire_error(error: WireError) -> String {
    match error {
        WireError::TruncatedVarint => "truncatedVarint".to_owned(),
        WireError::TruncatedField => "truncatedField".to_owned(),
        WireError::UnsupportedWireType(wire_type) => format!("unsupportedWireType({wire_type})"),
        WireError::TruncatedFragment => "truncatedFragment".to_owned(),
    }
}

fn boot_error(error: BootstrapError) -> String {
    match error {
        BootstrapError::NoConnectLine(_) => "noConnectLine".to_owned(),
        BootstrapError::UnusablePort(port) => format!("unusablePort({port})"),
        BootstrapError::BadKey => "badKey".to_owned(),
    }
}

// MARK: - Generator

/// Seeded random cases, `count` of them, payloads up to `max_len` bytes.
/// They include the triggers of the divergences both implementations now
/// fix (harness/divergences.json): corrupted zlib trailers that still
/// inflate, over-wide int32 varints, combining marks and other non-ASCII in
/// bootstrap output, `pid =` outside the detach line. The open divergence
/// (`excerpt-unicode`) is not generated: `boot.excerpt` output stays ASCII.
pub fn generate(seed: u64, count: usize, max_len: usize) -> Vec<String> {
    let mut rng = SplitMix64(seed);
    let mut cases = Vec::with_capacity(count);
    let ops: &[(&str, u64)] = &[
        ("ocb.seal", 6),
        ("ocb.open", 8),
        ("crypto.seal", 5),
        ("crypto.open", 8),
        ("zlib.compress", 6),
        ("zlib.decompress", 10),
        ("adler", 3),
        ("inst.encode", 5),
        ("inst.decode", 8),
        ("user.encode", 5),
        ("user.decode", 7),
        ("host.decode", 8),
        ("frag.encode", 3),
        ("frag.parse", 4),
        ("frag.split", 4),
        ("frag.assemble", 6),
        ("boot.parse", 10),
        ("boot.command", 5),
        ("boot.excerpt", 4),
        ("pipe.send", 4),
        ("pipe.recv", 6),
    ];
    let total: u64 = ops.iter().map(|(_, weight)| weight).sum();
    while cases.len() < count {
        let mut pick = rng.below(total);
        let op = ops
            .iter()
            .find(|(_, weight)| {
                if pick < *weight {
                    true
                } else {
                    pick -= weight;
                    false
                }
            })
            .map(|(op, _)| *op)
            .unwrap();
        cases.push(generate_case(&mut rng, op, max_len));
    }
    cases
}

fn generate_case(rng: &mut SplitMix64, op: &str, max_len: usize) -> String {
    let len = |rng: &mut SplitMix64| random_len(rng, max_len);
    match op {
        "ocb.seal" => {
            let key = odd_len(rng, 16, &[0, 15, 17, 32]);
            let nonce = odd_len(rng, 12, &[0, 8, 11, 13, 16]);
            let ad = if rng.below(4) == 0 {
                let n = len(rng);
                rng.bytes(n)
            } else {
                Vec::new()
            };
            let n = len(rng);
            let pt = rng.bytes(n);
            format!(
                "ocb.seal key={} nonce={} ad={} pt={}",
                hex(&key),
                hex(&nonce),
                hex(&ad),
                hex(&pt)
            )
        }
        "ocb.open" => {
            let key = rng.bytes(16);
            let nonce = odd_len(rng, 12, &[0, 11, 13]);
            let ad = if rng.below(4) == 0 {
                let n = len(rng);
                rng.bytes(n)
            } else {
                Vec::new()
            };
            let n = len(rng);
            let pt = rng.bytes(n);
            let ct = match Ocb3::new(&key).and_then(|ocb| ocb.seal(&pt, &pad12(&nonce), &ad)) {
                Ok(ct) => ct,
                Err(_) => rng.bytes(n + 16),
            };
            let ct = if rng.below(2) == 0 {
                mutate(rng, &ct)
            } else {
                ct
            };
            format!(
                "ocb.open key={} nonce={} ad={} ct={}",
                hex(&key),
                hex(&nonce),
                hex(&ad),
                hex(&ct)
            )
        }
        "crypto.seal" => {
            let n = len(rng);
            format!(
                "crypto.seal key={} seq={} dir={} ts={} tsr={} payload={}",
                hex(&rng.bytes(16)),
                edge_u64(rng),
                rng.below(2),
                rng.below(65_536),
                rng.below(65_536),
                hex(&rng.bytes(n))
            )
        }
        "crypto.open" => {
            let key = rng.bytes(16);
            let session = Session::new(&key).unwrap();
            let n = len(rng);
            let mut packet = if rng.below(8) == 0 {
                // A plaintext too short for the two timestamps.
                let mut packet = rng.bytes(8);
                let mut nonce = [0u8; 12];
                nonce[4..].copy_from_slice(&packet);
                let short = rng.bytes_below(4);
                packet.extend(Ocb3::new(&key).unwrap().seal(&short, &nonce, &[]).unwrap());
                packet
            } else {
                session.seal(
                    edge_u64(rng),
                    direction(rng.below(2)),
                    rng.below(65_536) as u16,
                    rng.below(65_536) as u16,
                    &rng.bytes(n),
                )
            };
            if rng.below(3) == 0 {
                packet = mutate(rng, &packet);
            }
            format!("crypto.open key={} packet={}", hex(&key), hex(&packet))
        }
        "zlib.compress" => format!("zlib.compress data={}", hex(&sample_data(rng, max_len))),
        "zlib.decompress" => {
            let data = sample_data(rng, max_len);
            let mut stream = compress::compress(&data);
            match rng.below(6) {
                0 => {}
                1 => {
                    // Only the trailer: valid DEFLATE, wrong Adler-32.
                    let at = stream.len() - 1 - rng.below(4) as usize;
                    stream[at] ^= 1 << rng.below(8);
                }
                2 => stream.extend(rng.bytes_between(1, 8)),
                _ => stream = mutate(rng, &stream),
            }
            format!("zlib.decompress data={}", hex(&stream))
        }
        "adler" => format!("adler data={}", hex(&sample_data(rng, max_len))),
        "inst.encode" => {
            let i = random_instruction(rng, max_len);
            format!(
                "inst.encode v={} old={} new={} ack={} thr={} diff={} chaff={}",
                i.protocol_version,
                i.old_num,
                i.new_num,
                i.ack_num,
                i.throwaway_num,
                hex(&i.diff),
                hex(&i.chaff)
            )
        }
        "inst.decode" => {
            let encoded = random_instruction(rng, max_len).encode();
            let data = match rng.below(4) {
                0 => encoded,
                1 => with_unknown_fields(rng, &encoded),
                _ => mutate(rng, &encoded),
            };
            format!("inst.decode data={}", hex(&data))
        }
        "user.encode" => format!(
            "user.encode events={}",
            format_events(&random_events(rng, max_len))
        ),
        "user.decode" => {
            let encoded = encode_user_message(&random_events(rng, max_len));
            let data = match rng.below(4) {
                0 => encoded,
                1 => with_unknown_fields(rng, &encoded),
                _ => mutate(rng, &encoded),
            };
            format!("user.decode data={}", hex(&data))
        }
        "host.decode" => {
            let encoded = encode_host_message(&random_host(rng, max_len));
            let data = match rng.below(4) {
                0 => encoded,
                1 => with_unknown_fields(rng, &encoded),
                _ => mutate(rng, &encoded),
            };
            format!("host.decode data={}", hex(&data))
        }
        "frag.encode" => {
            let n = len(rng);
            format!(
                "frag.encode id={} num={} final={} contents={}",
                edge_u64(rng),
                rng.below(65_536),
                rng.below(2),
                hex(&rng.bytes(n))
            )
        }
        "frag.parse" => {
            let n = len(rng);
            let mut data = Fragment::new(
                edge_u64(rng),
                rng.below(65_536) as u16,
                rng.below(2) == 1,
                rng.bytes(n),
            )
            .encode();
            if rng.below(3) == 0 {
                data.truncate(rng.below(14) as usize);
            }
            format!("frag.parse data={}", hex(&data))
        }
        "frag.split" => {
            let n = len(rng);
            let mtu = [0, 1, 9, 10, 11, 12, 64, 500, 1300, 1400][rng.below(10) as usize];
            let n = if mtu < 64 { n.min(200) } else { n };
            format!(
                "frag.split payload={} id={} mtu={}",
                hex(&rng.bytes(n)),
                edge_u64(rng),
                mtu
            )
        }
        "frag.assemble" => {
            let frags = random_fragment_stream(rng, max_len);
            format!("frag.assemble frags={}", list(&frags))
        }
        "boot.parse" => {
            let output = random_bootstrap_output(rng);
            format!(
                "boot.parse output={} host={}",
                hex(output.as_bytes()),
                hex(b"10.0.0.5")
            )
        }
        "boot.command" => {
            let bins = [
                "mosh-server",
                "/opt/homebrew/bin/mosh-server",
                "m",
                "./mosh",
                "~/bin/x",
            ];
            let locales = ["en_US.UTF-8", "C.UTF-8", "L", "zh_CN.UTF-8"];
            let port = |rng: &mut SplitMix64| -> String {
                match rng.below(6) {
                    0 => "-".to_owned(),
                    1 => ["0", "-1", "65535", "65536", "70000", "1"][rng.below(6) as usize]
                        .to_owned(),
                    _ => (59_990 + rng.below(30)).to_string(),
                }
            };
            format!(
                "boot.command bin={} locale={} start={} end={}",
                hex(bins[rng.below(bins.len() as u64) as usize].as_bytes()),
                hex(locales[rng.below(locales.len() as u64) as usize].as_bytes()),
                port(rng),
                port(rng)
            )
        }
        "boot.excerpt" => {
            let n = rng.below(220) as usize;
            let text: Vec<u8> = (0..n)
                .map(|_| match rng.below(10) {
                    0 => rng.below(32) as u8,
                    1 => 0x7F,
                    2 => b'\r',
                    3 => b'\n',
                    _ => 0x20 + rng.below(95) as u8,
                })
                .collect();
            format!("boot.excerpt output={}", hex(&text))
        }
        "pipe.send" => {
            let n = len(rng);
            let zlib = compress::compress(&rng.bytes(n));
            format!(
                "pipe.send key={} seq={} id={} mtu={} ts={} tsr={} zlib={}",
                hex(&rng.bytes(16)),
                rng.below(1 << 40),
                edge_u64(rng),
                [64, 500, 1300][rng.below(3) as usize],
                rng.below(65_536),
                rng.below(65_536),
                hex(&zlib)
            )
        }
        "pipe.recv" => {
            let key = rng.bytes(16);
            let packets = random_server_stream(rng, &key, max_len);
            format!("pipe.recv key={} packets={}", hex(&key), list(&packets))
        }
        other => panic!("no generator for {other}"),
    }
}

/// Mostly small, sometimes empty or at block and fragment edges, rarely up
/// to `max_len`.
fn random_len(rng: &mut SplitMix64, max_len: usize) -> usize {
    match rng.below(10) {
        0 => 0,
        1 => [1, 15, 16, 17, 31, 32, 33, 1289, 1290, 1291][rng.below(10) as usize].min(max_len),
        2 => rng.below(max_len as u64 + 1) as usize,
        _ => rng.below(64.min(max_len as u64) + 1) as usize,
    }
}

fn odd_len(rng: &mut SplitMix64, good: usize, bad: &[usize]) -> Vec<u8> {
    let n = if rng.below(10) == 0 {
        bad[rng.below(bad.len() as u64) as usize]
    } else {
        good
    };
    rng.bytes(n)
}

fn pad12(nonce: &[u8]) -> Vec<u8> {
    let mut out = nonce.to_vec();
    out.resize(12, 0);
    out
}

fn edge_u64(rng: &mut SplitMix64) -> u64 {
    match rng.below(6) {
        0 => [0, 1, 127, 128, 16_383, 16_384][rng.below(6) as usize],
        1 => [u64::MAX, u64::MAX >> 1, 1 << 63, u32::MAX as u64, 1 << 32][rng.below(5) as usize],
        _ => {
            let bits = rng.below(63) + 1;
            rng.below(1 << bits)
        }
    }
}

/// Text-like, repetitive, random or terminal-like data.
fn sample_data(rng: &mut SplitMix64, max_len: usize) -> Vec<u8> {
    let n = random_len(rng, max_len);
    match rng.below(4) {
        0 => rng.bytes(n),
        1 => vec![rng.below(256) as u8; n],
        2 => (0..n).map(|i| b"the quick brown fox "[i % 20]).collect(),
        _ => {
            let mut out = Vec::new();
            while out.len() < n {
                let row = 1 + rng.below(40);
                out.extend(format!("\x1b[{row};1H\x1b[0;3{}m", rng.below(8)).bytes());
                let width = rng.below(60) as usize;
                out.extend((0..width).map(|_| 0x20 + rng.below(95) as u8));
            }
            out.truncate(n);
            out
        }
    }
}

/// One of: flip a bit, truncate, append, replace, delete or insert a byte.
fn mutate(rng: &mut SplitMix64, data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    if out.is_empty() {
        return rng.bytes_below(4);
    }
    let at = rng.below(out.len() as u64) as usize;
    match rng.below(6) {
        0 => out[at] ^= 1 << rng.below(8),
        1 => out.truncate(at),
        2 => {
            let extra = rng.bytes_between(1, 8);
            out.extend(extra);
        }
        3 => out[at] = rng.below(256) as u8,
        4 => {
            out.remove(at);
        }
        _ => out.insert(at, rng.below(256) as u8),
    }
    out
}

/// Appends unknown fields of every skippable wire type, and sometimes a
/// group start (unsupported) or a known field with the wrong wire type.
fn with_unknown_fields(rng: &mut SplitMix64, data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    let field = 8 + rng.below(100);
    match rng.below(6) {
        0 => out.extend(varint(field << 3)),
        1 => {
            out.extend(varint(field << 3 | 1));
            out.extend(rng.bytes(8));
        }
        2 => {
            out.extend(varint(field << 3 | 2));
            let n = rng.below(10);
            out.extend(varint(n));
            out.extend(rng.bytes(n as usize));
        }
        3 => {
            out.extend(varint(field << 3 | 5));
            out.extend(rng.bytes(4));
        }
        4 => out.extend(varint(field << 3 | (3 + rng.below(5)))),
        _ => {
            // A known field number with another wire type.
            out.extend(varint(2 << 3 | 5));
            out.extend(rng.bytes(4));
        }
    }
    out
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn random_instruction(rng: &mut SplitMix64, max_len: usize) -> Instruction {
    let n = random_len(rng, max_len);
    let chaff = if rng.below(3) == 0 {
        rng.bytes_below(17)
    } else {
        Vec::new()
    };
    Instruction {
        protocol_version: if rng.below(4) == 0 {
            edge_u64(rng) as u32
        } else {
            2
        },
        old_num: edge_u64(rng),
        new_num: edge_u64(rng),
        ack_num: edge_u64(rng),
        throwaway_num: edge_u64(rng),
        diff: rng.bytes(n),
        chaff,
    }
}

fn random_events(rng: &mut SplitMix64, max_len: usize) -> Vec<UserEvent> {
    let count = rng.below(6);
    (0..count)
        .map(|_| {
            if rng.below(3) == 0 {
                let edge = [0, 1, 80, 24, 1000, -1, i32::MAX, i32::MIN];
                UserEvent::Resize {
                    width: edge[rng.below(8) as usize],
                    height: edge[rng.below(8) as usize],
                }
            } else {
                let n = random_len(rng, max_len);
                UserEvent::Keystroke(rng.bytes(n))
            }
        })
        .collect()
}

fn random_host(rng: &mut SplitMix64, max_len: usize) -> Vec<HostInstruction> {
    let count = rng.below(5);
    (0..count)
        .map(|_| match rng.below(4) {
            0 => HostInstruction::EchoAck(edge_u64(rng)),
            1 => HostInstruction::Resize {
                width: 1 + rng.below(300) as i32,
                height: 1 + rng.below(100) as i32,
            },
            _ => HostInstruction::HostBytes(sample_data(rng, max_len)),
        })
        .collect()
}

/// Fragments of a few instructions, shuffled, duplicated, dropped, with an
/// occasional bogus number past the final one or a truncated fragment.
fn random_fragment_stream(rng: &mut SplitMix64, max_len: usize) -> Vec<String> {
    let mut stream: Vec<Vec<u8>> = Vec::new();
    let mut id = rng.below(1000);
    for _ in 0..1 + rng.below(4) {
        let n = random_len(rng, max_len.max(200));
        let payload = rng.bytes(n);
        let mtu = 10 + 1 + rng.below(80) as usize;
        let mut pieces: Vec<Vec<u8>> = fragment(&payload, id, mtu)
            .iter()
            .map(Fragment::encode)
            .collect();
        if pieces.len() > 1 && rng.below(3) == 0 {
            let a = rng.below(pieces.len() as u64) as usize;
            let b = rng.below(pieces.len() as u64) as usize;
            pieces.swap(a, b);
        }
        if rng.below(4) == 0 && !pieces.is_empty() {
            let at = rng.below(pieces.len() as u64) as usize;
            pieces.remove(at);
        }
        if rng.below(4) == 0 && !pieces.is_empty() {
            let at = rng.below(pieces.len() as u64) as usize;
            let copy = pieces[at].clone();
            pieces.insert(at, copy);
        }
        if rng.below(8) == 0 {
            pieces.push(Fragment::new(id, 0x7FF0, false, rng.bytes(3)).encode());
        }
        if rng.below(10) == 0 {
            pieces.push(rng.bytes_below(10));
        }
        stream.extend(pieces);
        id = match rng.below(4) {
            0 => id.saturating_sub(1 + rng.below(3)),
            1 => id,
            _ => id + 1 + rng.below(3),
        };
    }
    stream.iter().map(|piece| hex(piece)).collect()
}

/// A server's datagrams: real instructions with host output, fragmented,
/// sometimes reordered, duplicated or corrupted, a few in the wrong
/// direction or under another key.
fn random_server_stream(rng: &mut SplitMix64, key: &[u8], max_len: usize) -> Vec<String> {
    let session = Session::new(key).unwrap();
    let mut packets = Vec::new();
    let mut seq = rng.below(100);
    let mut num = 0;
    for id in 1..=1 + rng.below(5) {
        let instruction = Instruction {
            old_num: num,
            new_num: num + 1 + rng.below(3),
            ack_num: rng.below(10),
            throwaway_num: num,
            diff: encode_host_message(&random_host(rng, max_len.max(600))),
            chaff: rng.bytes_below(17),
            ..Instruction::default()
        };
        num = instruction.new_num;
        let compressed = compress::compress(&instruction.encode());
        let mtu = [200, 500, 1300][rng.below(3) as usize];
        for piece in fragment(&compressed, id, mtu) {
            let dir = if rng.below(12) == 0 {
                Direction::ToServer
            } else {
                Direction::ToClient
            };
            let mut packet = session.seal(
                seq,
                dir,
                rng.below(65_536) as u16,
                rng.below(65_536) as u16,
                &piece.encode(),
            );
            seq += 1;
            if rng.below(12) == 0 {
                packet = mutate(rng, &packet);
            }
            packets.push(packet);
        }
    }
    if packets.len() > 1 && rng.below(3) == 0 {
        let a = rng.below(packets.len() as u64) as usize;
        let b = rng.below(packets.len() as u64) as usize;
        packets.swap(a, b);
    }
    packets.iter().map(|packet| hex(packet)).collect()
}

/// `mosh-server new` output: banners (some saying `pid = N` or non-ASCII),
/// blank lines, every newline `Character.isNewline` knows, the connect line
/// with ports and keys good and bad, extra spaces and combining marks glued
/// to its spaces, the detach line with pids good and bad.
fn random_bootstrap_output(rng: &mut SplitMix64) -> String {
    const KEY: &str = "AAECAwQFBgcICQoLDA0ODw";
    let newline = |rng: &mut SplitMix64| {
        [
            "\n", "\r\n", "\r", "\r\r\n", "\x0b", "\x0c", "\u{85}", "\u{2028}", "\u{2029}",
        ][rng.below(9) as usize]
    };
    let mut out = String::new();
    for _ in 0..rng.below(3) {
        let banner = [
            "Welcome to Ubuntu",
            "Last login: today",
            "",
            "mosh-server (mosh 1.4.0)",
            "rapid = 5",
            "pid = 4242",
            "caf\u{e9} \u{301}na\u{ef}ve \u{1F600}",
            "MOSH CONNECT \u{301}60009 AAECAwQFBgcICQoLDA0ODw",
        ];
        out.push_str(banner[rng.below(8) as usize]);
        out.push_str(newline(rng));
    }
    if rng.below(10) != 0 {
        let port = match rng.below(8) {
            0 => [
                "0",
                "-1",
                "65536",
                "70000",
                "99999999999",
                "99999999999999999999",
            ][rng.below(6) as usize]
                .to_owned(),
            1 => ["sixty", "+60001", "-0", "", "6e4", "0x10"][rng.below(6) as usize].to_owned(),
            _ => (1 + rng.below(65_535)).to_string(),
        };
        let key = match rng.below(10) {
            0 => format!("{KEY}=="),
            1 => format!("{KEY}="),
            2 => KEY.replace('w', "x"),
            3 => KEY[..21].to_owned(),
            4 => format!("{KEY}A"),
            5 => KEY.replace('E', "-"),
            6 => format!("{KEY}===="),
            _ => {
                let raw = SplitMix64(rng.next_u64()).bytes(16);
                base64_encode(&raw)[..22].to_owned()
            }
        };
        // Sometimes a combining mark glued to a space: one Character with
        // it, but not a space to mosh-server.
        let spaces = |rng: &mut SplitMix64| {
            let mut gap = " ".repeat(1 + (rng.below(5) == 0) as usize);
            if rng.below(12) == 0 {
                gap.push('\u{301}');
            }
            gap
        };
        let lead = if rng.below(6) == 0 { " " } else { "" };
        out.push_str(&format!(
            "{lead}MOSH CONNECT{}{port}{}{key}",
            spaces(rng),
            spaces(rng)
        ));
        if rng.below(5) == 0 {
            out.push_str(" trailing junk");
        }
        out.push_str(newline(rng));
    }
    if rng.below(3) != 0 {
        out.push_str(newline(rng));
        out.push_str("mosh-server (mosh 1.4.0) [build mosh 1.4.0]");
        out.push_str(newline(rng));
        let pid = match rng.below(7) {
            0 => "0".to_owned(),
            1 => "99999999999999999999".to_owned(),
            2 => "1".to_owned(),
            _ => (1 + rng.below(99_999)).to_string(),
        };
        out.push_str(&format!("[mosh-server detached, pid = {pid}]"));
        out.push_str(newline(rng));
    }
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut word = [0u8; 3];
        word[..chunk.len()].copy_from_slice(chunk);
        let n = u32::from(word[0]) << 16 | u32::from(word[1]) << 8 | u32::from(word[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `pipe.recv` cases over a recorded session: `key <b64>` then one
/// `up|down <hex>` line per datagram. One case for the server's datagrams
/// and one for the client's, each in recorded order.
pub fn session_cases(text: &str) -> Vec<String> {
    let mut key = None;
    let (mut down, mut up) = (Vec::new(), Vec::new());
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("key", b64)) => {
                let creds =
                    bootstrap::parse(&format!("MOSH CONNECT 1 {b64}\n"), "").expect("recorded key");
                key = Some(creds.key);
            }
            Some(("down", data)) => down.push(data.to_owned()),
            Some(("up", data)) => up.push(data.to_owned()),
            _ => {}
        }
    }
    let key = hex(&key.expect("a key line"));
    [down, up]
        .iter()
        .map(|packets| format!("pipe.recv key={key} packets={}", list(packets)))
        .collect()
}
