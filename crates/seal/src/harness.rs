//! The equivalence harness's Rust side: the case format both front ends
//! run (`harness/swift/main.swift` over the app's sources,
//! `examples/seal_harness.rs` over this crate), and the seeded case
//! generator. `harness/equivalence.py` drives both.
//!
//! One case per line, one result line per case. Strings travel as the hex of
//! their UTF-8 (`~` for the empty string, `-` for none), so any content fits
//! a space-separated line.
//!
//! | case | result |
//! | --- | --- |
//! | `derive S` | `ok ENC MAC` / `err FAILURE` |
//! | `open V IV CT MAC S` | `raw=PT`/`raw!FAILURE` and `status=…`/`status!FAILURE` |
//! | `seal S IV PT` | `ok V IV CT MAC` / `err FAILURE` (PT is plain hex) |
//! | `status PT` | the decoded status (9 fields) / `err` |
//! | `detail X` · `place L S` · `duration N LOCALE` | the string |
//! | `render STATUS… EDGE COUNT SHOWDETAIL SOUND LABEL` | the card's 11 fields |
//! | `location STATUS… LABEL` | the string |
//! | `payload JSON` | `V IV CT MAC` / `-` / `badjson` |
//! | `trysecrets V IV CT MAC S…` | the status / `-` |
//! | `pfile INITIAL OP…` · `sfile INITIAL NOW OP…` | each op's result, then the files |
//! | `jsonser JSON` | pretty and compact `JSONSerialization` output / `err` |
//! | `b64 T` · `uuid T` · `iso T` · `isoenc BITS` · `dbl BITS` · `nsjson JSON` | the value / `-` |
//!
//! A status is `conn host sess pane agent state title ts dur`; a date is the
//! bit pattern of its `timeIntervalSinceReferenceDate`, in hex.

use std::path::Path;

use crate::copy::{self, Language};
use crate::envelope::{self, Envelope, Failure};
use crate::foundation::date::Date;
use crate::foundation::json::{self, Dialect, Style, Value};
use crate::foundation::{base64, double, hex, uuid::Uuid};
use crate::notification::{self, Render};
use crate::pairing::PushPairing;
use crate::pairing_file::PushPairingFile;
use crate::prefs::Prefs;
use crate::standing::{Entry, LIFETIME_SECONDS, PushStandingFile};
use crate::status::Status;

// MARK: - Tokens

fn unhex(token: &str) -> Vec<u8> {
    hex::decode(token).expect("a hex token")
}

/// A string argument: `~` is empty, otherwise hex of UTF-8.
pub fn arg(token: &str) -> String {
    if token == "~" {
        String::new()
    } else {
        String::from_utf8(unhex(token)).expect("UTF-8")
    }
}

fn opt_arg(token: &str) -> Option<String> {
    (token != "-").then(|| arg(token))
}

/// A string as a token.
pub fn out(text: &str) -> String {
    if text.is_empty() {
        "~".to_owned()
    } else {
        hex::encode(text.as_bytes())
    }
}

fn opt_out(text: Option<&str>) -> String {
    text.map_or_else(|| "-".to_owned(), out)
}

fn date_out(date: Option<Date>) -> String {
    date.map_or_else(
        || "-".to_owned(),
        |d| format!("{:x}", d.since_reference().to_bits()),
    )
}

fn date_in(token: &str) -> Date {
    Date(f64::from_bits(
        u64::from_str_radix(token, 16).expect("hex bits"),
    ))
}

fn failure(failure: Failure) -> String {
    match failure {
        Failure::BadSecret => "badSecret".into(),
        Failure::UnsupportedVersion(v) => format!("unsupportedVersion({v})"),
        Failure::Malformed => "malformed".into(),
        Failure::AuthenticationFailed => "authenticationFailed".into(),
        Failure::DecryptFailed => "decryptFailed".into(),
    }
}

fn status_out(s: &Status) -> String {
    [
        out(&s.conn),
        out(&s.host),
        opt_out(s.sess.as_deref()),
        out(&s.pane),
        opt_out(s.agent.as_deref()),
        out(&s.state),
        opt_out(s.title.as_deref()),
        s.ts.to_string(),
        s.dur.map_or_else(|| "-".to_owned(), |d| d.to_string()),
    ]
    .join(" ")
}

fn status_in(f: &[&str], at: usize) -> Status {
    Status {
        conn: arg(f[at]),
        host: arg(f[at + 1]),
        sess: opt_arg(f[at + 2]),
        pane: arg(f[at + 3]),
        agent: opt_arg(f[at + 4]),
        state: arg(f[at + 5]),
        title: opt_arg(f[at + 6]),
        ts: f[at + 7].parse().expect("ts"),
        dur: (f[at + 8] != "-").then(|| f[at + 8].parse().expect("dur")),
    }
}

fn pairing_out(p: &PushPairing) -> String {
    [
        p.connection_id.to_string(),
        out(&p.host_label),
        out(&p.secret_hex),
        out(&p.send_token),
        out(&p.relay_url),
        date_out(Some(p.created_at)),
        opt_out(p.apns_token.as_deref()),
        opt_out(p.apns_env.as_deref()),
        date_out(p.send_token_issued_at),
    ]
    .join(" ")
}

fn entry_out(e: &Entry) -> String {
    [
        out(&e.pane),
        opt_out(e.agent.as_deref()),
        opt_out(e.title.as_deref()),
        opt_out(e.location.as_deref()),
        e.since.to_string(),
        date_out(Some(e.recorded_at)),
    ]
    .join(" ")
}

fn file_out(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => "~".into(),
        Ok(bytes) => hex::encode(&bytes),
        Err(_) => "-".into(),
    }
}

fn files_out(path: &Path) -> String {
    let dir = path.parent().expect("a directory");
    let name = path
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    let mut stamped: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&format!("{name}.backup-"))
                })
                .map(|e| file_out(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    stamped.sort();
    let backup = crate::store::with_extension(path, "backup");
    format!(
        "file={} backup={} stamped={}",
        file_out(path),
        file_out(&backup),
        if stamped.is_empty() {
            "-".to_owned()
        } else {
            stamped.join(",")
        }
    )
}

fn prepare(path: &Path, token: &str) {
    let dir = path.parent().expect("a directory");
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("scratch");
    match token {
        "-" => {}
        "!" => {
            std::fs::write(path, "[]").expect("write");
            set_mode(path, 0o000);
        }
        bytes => std::fs::write(path, unhex(bytes)).expect("write"),
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) {}

fn level(level: copy::InterruptionLevel) -> &'static str {
    match level {
        copy::InterruptionLevel::Passive => "passive",
        copy::InterruptionLevel::Active => "active",
        copy::InterruptionLevel::TimeSensitive => "timeSensitive",
    }
}

// MARK: - Running a case

/// Runs one case line in `scratch`; `None` for blank and `#` comment lines.
pub fn run(line: &str, scratch: &Path) -> Option<String> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let f: Vec<&str> = line.split(' ').collect();
    Some(match f[0] {
        "derive" => match envelope::derive_keys(&arg(f[1])) {
            Ok(keys) => format!("ok {} {}", keys.enc_hex, keys.mac_hex),
            Err(e) => format!("err {}", failure(e)),
        },
        "open" => {
            let envelope = Envelope {
                v: f[1].parse().expect("v"),
                iv: arg(f[2]),
                ct: arg(f[3]),
                mac: arg(f[4]),
            };
            let secret = arg(f[5]);
            let raw = match envelope::open_raw(&envelope, &secret) {
                Ok(pt) => format!("raw={}", hex::encode(&pt)),
                Err(e) => format!("raw!{}", failure(e)),
            };
            let opened = match envelope::open(&envelope, &secret) {
                Ok(status) => format!("status={}", status_out(&status)),
                Err(e) => format!("status!{}", failure(e)),
            };
            format!("{raw} {opened}")
        }
        "seal" => match envelope::seal(&unhex(f[3]), &arg(f[1]), Some(&arg(f[2]))) {
            Ok(e) => format!("ok {} {} {} {}", e.v, out(&e.iv), out(&e.ct), out(&e.mac)),
            Err(e) => format!("err {}", failure(e)),
        },
        "status" => {
            Status::from_json(&unhex(f[1])).map_or_else(|| "err".to_owned(), |s| status_out(&s))
        }
        "detail" => opt_out(copy::detail(opt_arg(f[1]).as_deref()).as_deref()),
        "place" => out(&copy::place(&arg(f[1]), opt_arg(f[2]).as_deref())),
        "duration" => opt_out(
            copy::duration(f[1].parse().expect("seconds"), Language::from_tag(f[2])).as_deref(),
        ),
        "render" => {
            let status = status_in(&f, 1);
            let label = opt_arg(f[14]);
            let card = notification::render(
                &status,
                Render {
                    attention_edge: f[10] == "1",
                    standing_count: f[11].parse().expect("count"),
                    prefs: Prefs {
                        show_detail: f[12] == "1",
                        sound: f[13] == "1",
                    },
                    label: label.as_deref(),
                    language: Language::En,
                    done_rings: true,
                },
            );
            [
                out(&card.title),
                out(&card.body),
                if card.sound { "1" } else { "0" }.to_owned(),
                level(card.level).to_owned(),
                out(&card.connection_id),
                out(&card.pane_id),
                out(&card.agent),
                out(&card.state),
                out(&card.detail),
                "~".to_owned(),
                opt_out(card.self_test_nonce()),
            ]
            .join(" ")
        }
        "location" => out(&notification::location(
            &status_in(&f, 1),
            opt_arg(f[10]).as_deref(),
        )),
        "payload" => match json::parse(
            &unhex(f[1]),
            Dialect::Serialization {
                fragments_allowed: true,
            },
        ) {
            Err(_) => "badjson".into(),
            Ok(value) => Envelope::from_payload(&value).map_or_else(
                || "-".to_owned(),
                |e| format!("{} {} {} {}", e.v, out(&e.iv), out(&e.ct), out(&e.mac)),
            ),
        },
        "trysecrets" => {
            let envelope = Envelope {
                v: f[1].parse().expect("v"),
                iv: arg(f[2]),
                ct: arg(f[3]),
                mac: arg(f[4]),
            };
            let secrets: Vec<String> = f[5..].iter().map(|s| arg(s)).collect();
            notification::open(&envelope, &secrets)
                .map_or_else(|| "-".to_owned(), |s| status_out(&s))
        }
        "pfile" => pfile(&f, scratch),
        "sfile" => sfile(&f, scratch),
        "jsonser" => match json::parse(
            &unhex(f[1]),
            Dialect::Serialization {
                fragments_allowed: true,
            },
        ) {
            Err(_) => "err".into(),
            Ok(value) => {
                let pretty = json::write_serialization(
                    &value,
                    Style {
                        pretty: true,
                        sorted_keys: true,
                        escape_slashes: false,
                    },
                );
                let compact = json::write_serialization(
                    &value,
                    Style {
                        pretty: false,
                        sorted_keys: true,
                        escape_slashes: true,
                    },
                );
                format!(
                    "{} {}",
                    hex::encode(pretty.as_bytes()),
                    hex::encode(compact.as_bytes())
                )
            }
        },
        "b64" => base64::decode(&arg(f[1])).map_or_else(
            || "-".to_owned(),
            |b| {
                if b.is_empty() {
                    "~".into()
                } else {
                    hex::encode(&b)
                }
            },
        ),
        "uuid" => Uuid::parse(&arg(f[1])).map_or_else(|| "-".to_owned(), |u| u.to_string()),
        "iso" => date_out(Date::parse_iso8601(&arg(f[1]))),
        "isoenc" => format!("{{\"at\":\"{}\"}}", date_in(f[1]).iso8601()),
        "dbl" => double::encoder(f64::from_bits(u64::from_str_radix(f[1], 16).expect("bits"))),
        "nsjson" => match json::parse(
            &unhex(f[1]),
            Dialect::Serialization {
                fragments_allowed: true,
            },
        ) {
            Ok(Value::Object(fields)) => {
                let v = fields.get("v");
                let int = v
                    .and_then(Value::as_swift_int)
                    .map_or_else(|| "-".to_owned(), |i| i.to_string());
                let double = v
                    .and_then(Value::as_swift_double)
                    .map_or_else(|| "-".to_owned(), |d| format!("{:x}", d.to_bits()));
                format!("{int} {double}")
            }
            _ => "badjson".into(),
        },
        other => panic!("unknown command {other}"),
    })
}

fn pfile(f: &[&str], scratch: &Path) -> String {
    let path = scratch.join("pfile").join(PushPairingFile::FILE_NAME);
    prepare(&path, f[1]);
    let file = PushPairingFile::new(&path);
    let mut results = Vec::new();
    for op in &f[2..] {
        if let Some(json) = op.strip_prefix("u:") {
            let pairing: PushPairing = json::decode(&unhex(json)).expect("a pairing");
            results.push(if file.upsert(&pairing) { "1" } else { "0" }.to_owned());
        } else if let Some(id) = op.strip_prefix("r:") {
            results.push(
                if file.remove(Uuid::parse(id).expect("a uuid")) {
                    "1"
                } else {
                    "0"
                }
                .to_owned(),
            );
        } else if let Some(json) = op.strip_prefix("w:") {
            let list: Vec<PushPairing> = json::decode(&unhex(json)).expect("pairings");
            results.push(if file.write(&list) { "1" } else { "0" }.to_owned());
        } else if *op == "R" {
            let read = file.read();
            results.push(format!(
                "[{}]",
                read.iter().map(pairing_out).collect::<Vec<_>>().join(";")
            ));
        } else if *op == "C" {
            let (carried, list, unreadable) = file.inspect();
            results.push(format!(
                "carried={carried} list={} unreadable={}",
                u8::from(list),
                u8::from(unreadable)
            ));
        } else if *op == "S" {
            results.push(
                file.secrets_newest_first()
                    .iter()
                    .map(|s| out(s))
                    .collect::<Vec<_>>()
                    .join(","),
            );
        } else if *op == "!" {
            set_mode(&path, 0o644);
        } else {
            panic!("bad pfile op {op}");
        }
    }
    set_mode(&path, 0o644);
    format!("{} || {}", results.join(" | "), files_out(&path))
}

fn sfile(f: &[&str], scratch: &Path) -> String {
    let path = scratch.join("sfile").join(PushStandingFile::FILE_NAME);
    prepare(&path, f[1]);
    let now = Date::from_unix(f[2].parse().expect("now"));
    let file = PushStandingFile::new(&path);
    let mut results = Vec::new();
    for op in &f[3..] {
        let p: Vec<&str> = op.split(':').collect();
        match p[0] {
            "n" => {
                let entry = Entry {
                    pane: arg(p[2]),
                    agent: opt_arg(p[3]),
                    title: opt_arg(p[4]),
                    location: None,
                    since: p[5].parse().expect("since"),
                    recorded_at: date_in(p[6]),
                    lifetime: Some(LIFETIME_SECONDS),
                };
                results.push(
                    if file.note_standing(&arg(p[1]), entry, now) {
                        "1"
                    } else {
                        "0"
                    }
                    .to_owned(),
                );
            }
            "c" => {
                let left = file.clear(&arg(p[1]), &arg(p[2]), now);
                results.push(format!(
                    "[{}]",
                    left.iter().map(entry_out).collect::<Vec<_>>().join(";")
                ));
            }
            "a" => {
                file.clear_all(&arg(p[1]), now);
                results.push("ok".into());
            }
            "R" => {
                let entries = file.standing(&arg(p[1]), now);
                results.push(format!(
                    "[{}]",
                    entries.iter().map(entry_out).collect::<Vec<_>>().join(";")
                ));
            }
            "M" => {
                let all = file.read(now);
                results.push(
                    all.iter()
                        .map(|(conn, list)| format!("{}={}", out(conn), list.len()))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            "!" => set_mode(&path, 0o644),
            other => panic!("bad sfile op {other}"),
        }
    }
    set_mode(&path, 0o644);
    format!("{} || {}", results.join(" | "), files_out(&path))
}

// MARK: - Generating cases

/// SplitMix64, the generator's only source of randomness.
#[derive(Debug, Clone)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    pub fn index(&mut self, len: usize) -> usize {
        usize::try_from(self.below(len as u64)).expect("fits")
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }
}

/// Pieces a generated string is built from: plain text, every kind of
/// whitespace Foundation and Swift tell apart, combining marks, numerals of
/// several scripts, multi-scalar emoji, and JSON's special characters.
const PIECES: &[&str] = &[
    "a",
    "b",
    "Z",
    "q",
    "0",
    "7",
    "42",
    " ",
    "  ",
    "\t",
    "\n",
    "\r\n",
    "\r",
    "\u{A0}",
    "\u{2028}",
    "\u{3000}",
    "\u{85}",
    "\u{301}",
    "e\u{301}",
    " \u{301}",
    "\u{20DD}",
    "长",
    "一",
    "五",
    "三号",
    "٣",
    "½",
    "①",
    "😀",
    "👨\u{200D}👩\u{200D}👧",
    "🇨🇳",
    "é",
    "\"",
    "\\",
    "/",
    "%",
    "$(x)",
    "'",
    "-",
    "_",
    ".",
    ":",
    "|",
    "\u{1}",
    "\u{7f}",
    "\u{FEFF}",
    "\u{200B}",
    "Ⅻ",
    "１",
    "x",
    "build",
    "rm -rf",
    "claude",
];

fn random_text(rng: &mut SplitMix64, max_pieces: u64) -> String {
    let count = rng.below(max_pieces + 1);
    (0..count).map(|_| *rng.pick(PIECES)).collect()
}

/// `text` as a JSON string literal, escaped the many ways JSON allows.
fn json_string(rng: &mut SplitMix64, text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0}'..='\u{1f}' => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            '/' if rng.chance(50) => out.push_str("\\/"),
            c if !c.is_ascii() && rng.chance(20) => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    if rng.chance(50) {
                        out.push_str(&format!("\\u{unit:04X}"));
                    } else {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number literal of some class `JSONSerialization` tells apart.
fn number_literal(rng: &mut SplitMix64) -> String {
    match rng.below(14) {
        0 => rng.pick(&["0", "-0", "1", "-1", "7", "100"]).to_string(),
        1 => (rng.next_u64() as i64 >> rng.below(63)).to_string(),
        2 => rng
            .pick(&[
                "9223372036854775807",
                "-9223372036854775808",
                "9223372036854775808",
                "18446744073709551615",
                "18446744073709551616",
                "-9223372036854775809",
                "123456789012345678901234567890",
            ])
            .to_string(),
        3 => format!("{}.{}", rng.below(1000), rng.below(1_000_000)),
        4 => rng
            .pick(&[
                "0.1", "0.7", "1.0", "1.10", "-0.0", "0.5", "123.456", "2.5e-5", "1e3", "1E3",
                "0.1e1", "1.5e300", "1e-7", "1e15", "1e16", "1e17", "1e21", "1e-400", "4e-324",
                "-1.5",
            ])
            .to_string(),
        5 => {
            let digits = 1 + rng.below(20);
            let mantissa: String = (0..digits)
                .map(|i| {
                    char::from(
                        b'0' + if i == 0 {
                            1 + rng.below(9)
                        } else {
                            rng.below(10)
                        } as u8,
                    )
                })
                .collect();
            let point = rng.below(digits);
            let (int, frac) = mantissa.split_at(
                usize::try_from(point + 1)
                    .expect("small")
                    .min(mantissa.len()),
            );
            if frac.is_empty() {
                int.to_owned()
            } else {
                format!("{int}.{frac}")
            }
        }
        6 => format!(
            "{}e{}{}",
            rng.below(100),
            rng.pick(&["", "+", "-"]),
            rng.below(30)
        ),
        7 => format!(
            "0.{}{}",
            "0".repeat(usize::try_from(rng.below(25)).expect("small")),
            rng.below(1_000_000_000_000_000_000)
        ),
        8 => format!(
            "{}.{}e{}",
            1 + rng.below(9),
            rng.below(1_000_000_000_000_000_000),
            rng.below(40) as i64 - 20
        ),
        9 => {
            let x = (rng.below(1 << 53) as f64)
                * 10f64.powi(i32::try_from(rng.below(60)).expect("small") - 30);
            format!("{x:e}")
        }
        10 => format!("{:e}", f64::from_bits(rng.next_u64() >> 2)),
        11 => format!("-{}", rng.below(u64::MAX)),
        12 => format!("{}000000000000000000000", rng.below(100_000)),
        _ => format!("{}.{}", rng.below(u64::MAX), rng.below(1_000)),
    }
}

/// A JSON value for a field `JSONDecoder` may or may not like.
fn json_value(rng: &mut SplitMix64, depth: u64) -> String {
    match rng.below(if depth > 2 { 4 } else { 6 }) {
        0 => number_literal(rng),
        1 => {
            let text = random_text(rng, 6);
            json_string(rng, &text)
        }
        2 => rng.pick(&["true", "false", "null"]).to_string(),
        3 => rng
            .pick(&["01", "1.", "-", "1e", "--1", "1.2.3", "1e5e5"])
            .to_string(),
        4 => {
            let n = rng.below(4);
            let items: Vec<String> = (0..n).map(|_| json_value(rng, depth + 1)).collect();
            format!(
                "[{}{}]",
                items.join(","),
                if n > 0 && rng.chance(20) { "," } else { "" }
            )
        }
        _ => {
            let n = rng.below(4);
            let items: Vec<String> = (0..n)
                .map(|_| {
                    let key = random_key(rng);
                    format!("{}:{}", json_string(rng, &key), json_value(rng, depth + 1))
                })
                .collect();
            format!(
                "{{{}{}}}",
                items.join(","),
                if n > 0 && rng.chance(20) { "," } else { "" }
            )
        }
    }
}

/// An object key built to probe `JSONSerialization`'s collation: ASCII, and
/// the non-ASCII characters it orders exactly, ignorables and spaces (other
/// non-ASCII keys sort by an approximation; see foundation::collation).
fn random_key(rng: &mut SplitMix64) -> String {
    const KEY_PIECES: &[&str] = &[
        "a",
        "A",
        "b",
        "B",
        "z",
        "Z",
        "0",
        "1",
        "2",
        "9",
        "00",
        "01",
        "10",
        "007",
        "99",
        "100",
        "_",
        "-",
        ".",
        " ",
        "$",
        "@",
        "/",
        "\\",
        ":",
        "#",
        "~",
        "|",
        "\t",
        "\n",
        "\u{1}",
        "\u{7f}",
        "\u{FEFF}",
        "\u{AD}",
        "\u{200B}",
        "\u{A0}",
        "\u{85}",
        "\u{2000}",
        "hooks",
        "Hooks",
        "env",
        "model",
        "PreToolUse",
        "Stop",
        "x1",
        "x01",
        "x10",
        "matcher",
        "command",
        "type",
    ];
    let count = rng.below(4);
    (0..count).map(|_| *rng.pick(KEY_PIECES)).collect()
}

fn string_or_absent(rng: &mut SplitMix64, key: &str, out: &mut Vec<String>) {
    match rng.below(10) {
        0 => {}
        1 => out.push(format!("\"{key}\":null")),
        2 => out.push(format!("\"{key}\":{}", json_value(rng, 2))),
        _ => {
            let text = random_text(rng, 8);
            out.push(format!("\"{key}\":{}", json_string(rng, &text)));
        }
    }
}

/// A status plaintext: usually well-formed, often bent the ways Foundation
/// forgives (or does not).
fn status_json(rng: &mut SplitMix64) -> String {
    let mut fields = Vec::new();
    for key in ["conn", "host", "pane", "state"] {
        if rng.chance(96) {
            let text = random_text(rng, 6);
            fields.push(format!("\"{key}\":{}", json_string(rng, &text)));
        } else {
            string_or_absent(rng, key, &mut fields);
        }
    }
    for key in ["sess", "agent", "title"] {
        string_or_absent(rng, key, &mut fields);
    }
    let ts = if rng.chance(85) {
        (1_700_000_000 + rng.below(100_000_000)).to_string()
    } else {
        json_value(rng, 2)
    };
    if rng.chance(97) {
        fields.push(format!("\"ts\":{ts}"));
    }
    match rng.below(4) {
        0 => {}
        1 => fields.push(format!("\"dur\":{}", rng.below(4000))),
        _ => fields.push(format!("\"dur\":{}", json_value(rng, 2))),
    }
    if rng.chance(30) {
        let key = random_key(rng);
        fields.push(format!("{}:{}", json_string(rng, &key), json_value(rng, 1)));
    }
    if rng.chance(15) {
        // A repeated key: Foundation keeps the first.
        let key = *rng.pick(&["conn", "ts", "dur", "sess", "title"]);
        fields.push(format!("\"{key}\":{}", json_value(rng, 2)));
    }
    // Shuffle.
    for i in (1..fields.len()).rev() {
        let j = rng.index(i + 1);
        fields.swap(i, j);
    }
    let space = |rng: &mut SplitMix64| rng.pick(&["", "", " ", "\n", "\t", "\r\n  "]).to_string();
    let mut text = String::new();
    if rng.chance(5) {
        text.push('\u{FEFF}');
    }
    text.push_str(&space(rng));
    text.push('{');
    let joined: Vec<String> = fields
        .iter()
        .map(|field| format!("{}{field}{}", space(rng), space(rng)))
        .collect();
    text.push_str(&joined.join(","));
    if !fields.is_empty() && rng.chance(10) {
        text.push(',');
    }
    text.push('}');
    text.push_str(&space(rng));
    match rng.below(40) {
        0 => "not json".to_owned(),
        1 => String::new(),
        2 => format!("[{text}]"),
        3 => format!("{text} x"),
        _ => text,
    }
}

fn secret(rng: &mut SplitMix64) -> String {
    let mut bytes = [0u8; 32];
    for b in &mut bytes {
        *b = rng.next_u64() as u8;
    }
    let text = hex::encode(&bytes);
    if rng.chance(20) {
        text.to_uppercase()
    } else {
        text
    }
}

/// A token for a string argument.
fn tok(text: &str) -> String {
    out(text)
}

fn mutate_hex(rng: &mut SplitMix64, text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    match rng.below(7) {
        0 if !chars.is_empty() => {
            let i = rng.index(chars.len());
            chars[i] = *rng.pick(&['0', 'f', 'A', 'g', 'z', ' ']);
        }
        1 => {
            chars.truncate(chars.len().saturating_sub(1 + rng.index(3)));
        }
        2 => chars.push(*rng.pick(&['0', 'a'])),
        3 => return text.to_uppercase(),
        4 => return String::new(),
        5 => return format!("{text}{text}"),
        _ => {}
    }
    chars.into_iter().collect()
}

fn mutate_b64(rng: &mut SplitMix64, text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    match rng.below(8) {
        0 if !chars.is_empty() => {
            let i = rng.index(chars.len());
            chars[i] = *rng.pick(&['A', 'B', '/', '+', 'z', '0']);
        }
        1 => chars.truncate(chars.len().saturating_sub(1 + rng.index(4))),
        2 => return text.trim_end_matches('=').to_owned(),
        3 => return format!("{text}\n"),
        4 => return String::new(),
        5 => return base64::encode(&[rng.next_u64() as u8; 3]),
        6 => return format!("{text}AAAA"),
        _ => {}
    }
    chars.into_iter().collect()
}

/// A UUID from the generator, so a seed always yields the same cases.
fn seeded_uuid(rng: &mut SplitMix64) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&rng.next_u64().to_be_bytes());
    bytes[8..].copy_from_slice(&rng.next_u64().to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// An IV from the generator.
fn seeded_iv(rng: &mut SplitMix64) -> String {
    hex::encode(&rng.next_u64().to_be_bytes()) + &hex::encode(&rng.next_u64().to_be_bytes())
}

/// One minimal case per documented divergence (`harness/divergences.json`),
/// each preceded by a `# <id>: …` comment line.
pub fn divergence_cases() -> Vec<String> {
    use aes::Aes256;
    use cbc::cipher::block_padding::NoPadding;
    use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
    use hmac::{Hmac, Mac};

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let keys = envelope::derive_keys(SECRET).expect("keys");
    let tag = |iv: &str, ct: &str| -> String {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(keys.mac_hex.as_bytes()).expect("key");
        mac.update(format!("v1|{}|{ct}", iv.to_ascii_lowercase()).as_bytes());
        base64::encode(&mac.finalize().into_bytes())
    };
    // An authenticated ciphertext of one raw block, no padding added.
    let raw = |block: [u8; 16]| -> (String, String, String) {
        let key = hex::decode(&keys.enc_hex).expect("key");
        let iv = [7u8; 16];
        let ct = cbc::Encryptor::<Aes256>::new_from_slices(&key, &iv)
            .expect("sizes")
            .encrypt_padded_vec::<NoPadding>(&block);
        let (iv, ct) = (hex::encode(&iv), base64::encode(&ct));
        let mac = tag(&iv, &ct);
        (iv, ct, mac)
    };
    let open = |iv: &str, ct: &str, mac: &str| {
        format!(
            "open 1 {} {} {} {}",
            tok(iv),
            tok(ct),
            tok(mac),
            tok(SECRET)
        )
    };
    let mut cases = vec!["# hex-sign: a secret or IV whose hex pairs carry a sign".to_owned()];
    cases.push(format!("derive {}", tok(&"+f".repeat(32))));
    cases.push(format!("derive {}", tok(&format!("-0{}", &SECRET[2..]))));
    let signed_iv = "+0+1+2+3+4+5+6+7+8+9+a+b+c+d+e+f";
    let sealed =
        envelope::seal(b"{}", SECRET, Some("000102030405060708090a0b0c0d0e0f")).expect("seal");
    cases.push(open(signed_iv, &sealed.ct, &tag(signed_iv, &sealed.ct)));

    cases.push("# base64-padding: '=' where Foundation still decodes".to_owned());
    for text in ["AAAA=", "AA=Q", "===="] {
        cases.push(format!("b64 {}", tok(text)));
    }
    cases.push(open(&sealed.iv, &sealed.ct, &format!("{}=", sealed.mac)));

    cases.push("# padding-lenient: authenticated, not PKCS#7".to_owned());
    let mut inner = [0x41u8; 16];
    inner[14] = 9;
    inner[15] = 2;
    let mut zero = [0x41u8; 16];
    zero[15] = 0;
    let mut big = [0x41u8; 16];
    big[15] = 17;
    for block in [inner, zero, big] {
        let (iv, ct, mac) = raw(block);
        cases.push(open(&iv, &ct, &mac));
    }

    cases.push("# bom-host-label: a host label that starts with U+FEFF (B264)".to_owned());
    let pairing = |id: &str, label: &str| {
        format!(
            "{{\"connectionId\":\"{id}\",\"hostLabel\":\"{label}\",\"secretHex\":\"{SECRET}\",\"sendToken\":\"\",\"relayURL\":\"https://push.example.org/x\",\"createdAt\":784000000}}"
        )
    };
    let marked = pairing("6A62F9FC-E1F5-43FC-BBDA-E3CA6873A507", "\u{FEFF}box");
    let other = pairing("D6BC88EF-9C44-4160-9496-29548C677462", "other");
    // Saved, then read back.
    cases.push(format!("pfile - u:{} R", hex::encode(marked.as_bytes())));
    // Stored by an earlier save, carried through a save of another pairing.
    cases.push(format!(
        "pfile {} u:{} R",
        hex::encode(format!("[{marked}]").as_bytes()),
        hex::encode(other.as_bytes())
    ));

    cases.push("# decimal-bridge: as? Int / as? Double on an NSDecimalNumber".to_owned());
    for literal in [
        "0.123456789012345678",
        "41775049572655816.6",
        "123456789012345678901234567890",
    ] {
        cases.push(format!(
            "nsjson {}",
            hex::encode(format!("{{\"v\":{literal}}}").as_bytes())
        ));
    }
    cases
}

/// `count` seeded cases.
pub fn generate(seed: u64, count: usize) -> Vec<String> {
    let mut rng = SplitMix64(seed);
    let mut cases = Vec::with_capacity(count);
    while cases.len() < count {
        let case = match rng.below(26) {
            0 | 1 => {
                let s = match rng.below(6) {
                    0 => mutate_hex(&mut rng, &secret(&mut SplitMix64(seed))),
                    1 => random_text(&mut rng, 4),
                    _ => secret(&mut rng),
                };
                if s.contains(['+', '-']) {
                    continue; // hex-sign: a documented divergence
                }
                format!("derive {}", tok(&s))
            }
            2..=6 => {
                let key = secret(&mut rng);
                let plaintext = status_json(&mut rng);
                let iv = hex::encode(&rng.next_u64().to_be_bytes())
                    + &hex::encode(&rng.next_u64().to_be_bytes());
                let mut e = envelope::seal(plaintext.as_bytes(), &key, Some(&iv)).expect("seal");
                let mut s = key.clone();
                match rng.below(12) {
                    0 => s = secret(&mut rng),
                    1 => s = key.to_lowercase(),
                    2 => e.iv = mutate_hex(&mut rng, &e.iv),
                    3 => e.ct = mutate_b64(&mut rng, &e.ct),
                    4 => e.mac = mutate_b64(&mut rng, &e.mac),
                    5 => e.v = *rng.pick(&[0, 2, -1, 1, 99]),
                    6 => e.iv = e.iv.to_uppercase(),
                    7 => s = mutate_hex(&mut rng, &key),
                    _ => {}
                }
                if e.iv.contains(['+', '-']) || s.contains(['+', '-']) {
                    continue; // hex-sign
                }
                format!(
                    "open {} {} {} {} {}",
                    e.v,
                    tok(&e.iv),
                    tok(&e.ct),
                    tok(&e.mac),
                    tok(&s)
                )
            }
            7 => {
                let iv = hex::encode(&rng.next_u64().to_be_bytes())
                    + &hex::encode(&rng.next_u64().to_be_bytes());
                let plaintext = status_json(&mut rng);
                format!(
                    "seal {} {} {}",
                    tok(&secret(&mut rng)),
                    tok(&iv),
                    hex::encode(plaintext.as_bytes())
                )
            }
            8 | 9 => format!("status {}", hex::encode(status_json(&mut rng).as_bytes())),
            10 => {
                if rng.chance(5) {
                    "detail -".to_owned()
                } else {
                    let pieces = if rng.chance(30) { 120 } else { 12 };
                    let text = random_text(&mut rng, pieces);
                    format!("detail {}", tok(&text))
                }
            }
            11 => {
                let label = random_text(&mut rng, 3);
                let session = match rng.below(6) {
                    0 => None,
                    1 => Some(
                        rng.pick(&["0", "12", " 3 ", "一", "三号", "½", "1\u{301}", "", "  "])
                            .to_string(),
                    ),
                    _ => Some(random_text(&mut rng, 4)),
                };
                format!(
                    "place {} {}",
                    tok(&label),
                    session.map_or_else(|| "-".to_owned(), |s| tok(&s))
                )
            }
            12 => {
                let seconds: i64 = match rng.below(4) {
                    0 => *rng.pick(&[
                        0, 59, 60, 61, 119, 120, 3599, 3600, 3659, 3660, 7199, 7200, 86_399, 86_400,
                    ]),
                    1 => i64::try_from(rng.below(100_000)).expect("small"),
                    2 => i64::try_from(rng.below(10_000_000_000)).expect("small"),
                    _ => i64::try_from(rng.below(i64::MAX as u64 / 3600)).expect("fits"),
                };
                let locale = rng.pick(&[
                    "en_US", "de", "es", "fr", "ja", "ko", "pt_BR", "zh_Hans", "zh_Hant",
                ]);
                format!("duration {seconds} {locale}")
            }
            13 | 14 => {
                let state = rng
                    .pick(&["attention", "done", "working", "", "Done"])
                    .to_string();
                let opt = |rng: &mut SplitMix64, n| {
                    if rng.chance(20) {
                        "-".to_owned()
                    } else {
                        tok(&random_text(rng, n))
                    }
                };
                let title = if rng.chance(15) {
                    "-".to_owned()
                } else {
                    let pieces = if rng.chance(20) { 100 } else { 10 };
                    tok(&random_text(&mut rng, pieces))
                };
                let fields = [
                    tok(&random_text(&mut rng, 3)),
                    tok(&random_text(&mut rng, 3)),
                    opt(&mut rng, 3),
                    tok(&random_text(&mut rng, 2)),
                    opt(&mut rng, 2),
                    tok(&state),
                    title,
                    (1_700_000_000 + rng.below(100_000_000)).to_string(),
                    if rng.chance(40) {
                        "-".to_owned()
                    } else {
                        rng.pick(&[0, 59, 60, 179, 180, 181, 600, 7260, -5])
                            .to_string()
                    },
                ];
                let flags = format!(
                    "{} {} {} {}",
                    u8::from(rng.chance(50)),
                    rng.pick(&[-1, 0, 1, 2, 3, 10]),
                    u8::from(rng.chance(70)),
                    u8::from(rng.chance(70))
                );
                let label = match rng.below(4) {
                    0 => "-".to_owned(),
                    1 => tok(rng.pick(&["", "  ", "\u{A0}"])),
                    _ => tok(&random_text(&mut rng, 3)),
                };
                if rng.chance(20) {
                    format!("location {} {label}", fields.join(" "))
                } else {
                    format!("render {} {flags} {label}", fields.join(" "))
                }
            }
            15 => {
                let mp = match rng.below(8) {
                    0 => format!(
                        "{{\"v\":{},\"iv\":\"00\",\"ct\":\"c\",\"mac\":\"m\"}}",
                        rng.pick(&[
                            "1",
                            "1.0",
                            "\"1\"",
                            "true",
                            "2",
                            "-1",
                            "1.5",
                            "null",
                            "1e0",
                            "9223372036854775808"
                        ])
                    ),
                    1 => "[1,2]".to_owned(),
                    2 => "\"x\"".to_owned(),
                    3 => "{\"v\":1,\"iv\":\"00\"}".to_owned(),
                    4 => format!(
                        "{{\"v\":1,\"iv\":{},\"ct\":\"c\",\"mac\":\"m\"}}",
                        json_value(&mut rng, 2)
                    ),
                    _ => {
                        let e =
                            envelope::seal(b"{}", &secret(&mut rng), Some(&seeded_iv(&mut rng)))
                                .expect("seal");
                        json::encode(&e)
                    }
                };
                let payload = match rng.below(6) {
                    0 => format!("{{\"aps\":{{\"mutable-content\":1}},\"mp\":{mp}}}"),
                    1 => format!("[{mp}]"),
                    2 => "{\"connectionId\":\"x\",\"paneId\":\"%3\"}".to_owned(),
                    3 => "{".to_owned(),
                    _ => format!("{{\"mp\":{mp}}}"),
                };
                format!("payload {}", hex::encode(payload.as_bytes()))
            }
            16 => {
                let right = secret(&mut rng);
                let e = envelope::seal(
                    status_json(&mut rng).as_bytes(),
                    &right,
                    Some(&seeded_iv(&mut rng)),
                )
                .expect("seal");
                let mut secrets: Vec<String> =
                    (0..rng.below(4)).map(|_| secret(&mut rng)).collect();
                if rng.chance(30) {
                    secrets.push("zz".to_owned());
                }
                if rng.chance(70) {
                    let at = rng.index(secrets.len() + 1);
                    secrets.insert(at, right);
                }
                let list: Vec<String> = secrets.iter().map(|s| tok(s)).collect();
                format!(
                    "trysecrets {} {} {} {} {}",
                    e.v,
                    tok(&e.iv),
                    tok(&e.ct),
                    tok(&e.mac),
                    list.join(" ")
                )
                .trim_end()
                .to_owned()
            }
            17 => pfile_case(&mut rng),
            18 => sfile_case(&mut rng),
            19..=21 => {
                let mut doc = if rng.chance(30) {
                    // A crowded object: twenty keys at once put the sorted-keys
                    // collation through hundreds of comparisons.
                    let mut keys: Vec<String> = Vec::new();
                    while keys.len() < 20 {
                        let key = random_key(&mut rng);
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                    let members: Vec<String> = keys
                        .iter()
                        .enumerate()
                        .map(|(i, key)| format!("{}:{i}", json_string(&mut rng, key)))
                        .collect();
                    format!("{{{}}}", members.join(","))
                } else {
                    json_value(&mut rng, 0)
                };
                if rng.chance(10) {
                    doc = format!("\u{FEFF}{doc}");
                }
                format!("jsonser {}", hex::encode(doc.as_bytes()))
            }
            22 => {
                let text: String = (0..rng.below(13))
                    .map(|_| *rng.pick(&['A', 'Q', '/', '+', 'z', '0', '=']))
                    .collect();
                let body = text.trim_end_matches('=');
                let pad = text.len() - body.len();
                // Foundation's odd padding is a documented divergence.
                if body.contains('=') || (pad > 0 && (pad > 2 || !text.len().is_multiple_of(4))) {
                    continue;
                }
                format!("b64 {}", tok(&text))
            }
            23 => {
                if rng.chance(30) {
                    let u = seeded_uuid(&mut rng).to_string();
                    let t = match rng.below(4) {
                        0 => u.to_lowercase(),
                        1 => mutate_hex(&mut rng, &u),
                        _ => u,
                    };
                    format!("uuid {}", tok(&t))
                } else {
                    let unix =
                        i64::try_from(rng.below(4_102_444_800)).expect("fits") - 1_000_000_000;
                    let date =
                        Date::from_unix(unix as f64 + if rng.chance(30) { 0.75 } else { 0.0 });
                    if rng.chance(50) {
                        format!("isoenc {:x}", date.since_reference().to_bits())
                    } else {
                        let text = date.iso8601();
                        let text = match rng.below(8) {
                            0 => text.replace('Z', "+08:00"),
                            1 => text.replace('Z', "-0530"),
                            2 => text.replace('Z', ".5Z"),
                            3 => text.to_lowercase(),
                            4 => text.replace('Z', ""),
                            5 => text.replace('T', " "),
                            _ => text,
                        };
                        format!("iso {}", tok(&text))
                    }
                }
            }
            24 => {
                let bits = match rng.below(4) {
                    0 => (rng.next_u64() & !(0x7FFu64 << 52)) | (rng.below(0x7FF) << 52),
                    1 => (rng.below(10_000_000_000) as f64 / 1000.0 + 700_000_000.0).to_bits(),
                    2 => (9_007_199_254_740_992.0f64 + (rng.below(8) as f64 - 4.0) * 2.0).to_bits(),
                    _ => (1e-4 * (1.0 + (rng.below(200) as f64 - 100.0) * 1e-6)).to_bits(),
                };
                let x = f64::from_bits(bits);
                if !x.is_finite() {
                    continue;
                }
                format!("dbl {bits:x}")
            }
            _ => {
                let value = json_value(&mut rng, 2);
                // How Swift bridges an NSDecimalNumber is the documented
                // decimal-bridge divergence.
                let decimal = matches!(
                    json::Number::from_literal(value.as_str()).classify(),
                    Ok(json::Classified::Decimal(_))
                );
                if decimal {
                    continue;
                }
                format!(
                    "nsjson {}",
                    hex::encode(format!("{{\"v\":{value}}}").as_bytes())
                )
            }
        };
        cases.push(case);
    }
    cases
}

fn pairing_json(rng: &mut SplitMix64, id: Uuid) -> String {
    let mut fields = vec![
        format!(
            "\"connectionId\":\"{}\"",
            if rng.chance(20) {
                id.to_string().to_lowercase()
            } else {
                id.to_string()
            }
        ),
        {
            let mut label = random_text(rng, 3);
            // bom-host-label (B264): a label starting with U+FEFF is a
            // documented divergence, held by its own cases. Nothing drawn.
            if label.starts_with('\u{FEFF}') {
                label.insert(0, 'x');
            }
            format!("\"hostLabel\":{}", json_string(rng, &label))
        },
        format!("\"secretHex\":\"{}\"", secret(rng)),
        format!(
            "\"sendToken\":\"{}\"",
            if rng.chance(50) {
                String::new()
            } else {
                secret(rng)
            }
        ),
        format!(
            "\"relayURL\":{}",
            json_string(rng, "https://push.example.org/x")
        ),
        format!(
            "\"createdAt\":{}",
            rng.pick(&[
                "784000000",
                "784000000.5",
                "7.84e8",
                "784123456.789012",
                "0",
                "-1"
            ])
        ),
    ];
    if rng.chance(50) {
        fields.push(format!("\"apnsToken\":\"{}\"", secret(rng)));
    }
    if rng.chance(50) {
        fields.push(format!(
            "\"apnsEnv\":\"{}\"",
            rng.pick(&["production", "sandbox"])
        ));
    }
    if rng.chance(50) {
        fields.push(format!(
            "\"sendTokenIssuedAt\":{}",
            rng.pick(&["784000001", "784000001.25", "null"])
        ));
    }
    format!("{{{}}}", fields.join(","))
}

fn foreign_pairing(rng: &mut SplitMix64, id: Uuid) -> String {
    match rng.below(6) {
        0 => format!("{{\"connectionId\":\"{id}\",\"secret\":{{\"v\":2}},\"createdAt\":784000000.0}}"),
        1 => format!("{{\"connectionId\":\"{id}\",\"hostLabel\":\"h\",\"secretHex\":\"x\",\"sendToken\":\"\",\"relayURL\":\"r\",\"createdAt\":\"yesterday\"}}"),
        2 => "{\"connectionId\":\"not-a-uuid\",\"hostLabel\":\"h\",\"secretHex\":\"x\",\"sendToken\":\"\",\"relayURL\":\"r\",\"createdAt\":1}".to_owned(),
        3 => "\"just a string\"".to_owned(),
        4 => "[1,2,3]".to_owned(),
        _ => format!("{{\"connectionId\":\"{id}\",\"hostLabel\":7,\"secretHex\":\"x\",\"sendToken\":\"\",\"relayURL\":\"r/x\",\"createdAt\":0.1,\"extra\":[{}]}}", number_literal(rng)),
    }
}

fn pfile_case(rng: &mut SplitMix64) -> String {
    let ids: Vec<Uuid> = (0..4).map(|_| seeded_uuid(rng)).collect();
    let initial = match rng.below(8) {
        0 => "-".to_owned(),
        1 => "!".to_owned(),
        2 => hex::encode(rng.pick(&[&b"not json"[..], b"{\"pairings\":1}", b"", b"[", b"1"])),
        _ => {
            let n = rng.below(4);
            let entries: Vec<String> = (0..n)
                .map(|i| {
                    let id = ids[usize::try_from(i).expect("small")];
                    if rng.chance(30) {
                        foreign_pairing(rng, id)
                    } else {
                        pairing_json(rng, id)
                    }
                })
                .collect();
            hex::encode(
                format!(
                    "[{}{}]",
                    entries.join(","),
                    if n > 0 && rng.chance(10) { "," } else { "" }
                )
                .as_bytes(),
            )
        }
    };
    let mut ops = Vec::new();
    for _ in 0..1 + rng.below(4) {
        let id = *rng.pick(&ids);
        ops.push(match rng.below(7) {
            0 | 1 => format!("u:{}", hex::encode(pairing_json(rng, id).as_bytes())),
            2 => format!("r:{id}"),
            3 => format!("r:{}", seeded_uuid(rng)),
            4 => {
                let list: Vec<String> = (0..rng.below(3))
                    .map(|i| pairing_json(rng, ids[usize::try_from(i).expect("small")]))
                    .collect();
                format!(
                    "w:{}",
                    hex::encode(format!("[{}]", list.join(",")).as_bytes())
                )
            }
            5 => "C".to_owned(),
            _ => "S".to_owned(),
        });
        if initial == "!" && rng.chance(30) {
            ops.push("!".to_owned());
        }
    }
    ops.push("R".to_owned());
    format!("pfile {initial} {}", ops.join(" "))
}

fn sfile_case(rng: &mut SplitMix64) -> String {
    let now: i64 = 1_762_800_000;
    let conns = [
        "11111111-2222-3333-4444-555555555555",
        "66666666-7777-8888-9999-000000000000",
        "c",
    ];
    let recorded = |rng: &mut SplitMix64| -> f64 {
        let offset = *rng.pick(&[
            0.0, -10.0, -899.0, -900.0, -901.0, -3600.0, 59.0, 61.0, 3600.0, -0.5,
        ]);
        Date::from_unix(now as f64 + offset).since_reference()
    };
    let entry_json = |rng: &mut SplitMix64| -> String {
        if rng.chance(25) {
            match rng.below(5) {
                0 => format!(
                    "{{\"pane\":\"%{}\",\"since\":\"x\",\"recordedAt\":{}}}",
                    rng.below(4),
                    recorded(rng)
                ),
                1 => format!("{{\"since\":1,\"recordedAt\":{}}}", recorded(rng)),
                2 => format!(
                    "{{\"pane\":3,\"since\":1,\"recordedAt\":{}}}",
                    recorded(rng)
                ),
                3 => format!(
                    "{{\"pane\":\"%{}\",\"since\":1,\"recordedAt\":\"now\"}}",
                    rng.below(4)
                ),
                _ => "\"loose\"".to_owned(),
            }
        } else {
            format!(
                "{{\"pane\":\"%{}\",\"agent\":\"claude\",\"since\":{},\"recordedAt\":{}{}}}",
                rng.below(4),
                rng.below(3),
                recorded(rng),
                if rng.chance(30) {
                    ",\"location\":\"m1 · work\""
                } else {
                    ""
                }
            )
        }
    };
    let initial = match rng.below(8) {
        0 => "-".to_owned(),
        1 => "!".to_owned(),
        2 => hex::encode(rng.pick(&[&b"[\"standing\"]"[..], b"nope", b"7"])),
        _ => {
            let mut members: Vec<String> = Vec::new();
            for conn in conns {
                if !rng.chance(60) {
                    continue;
                }
                let value = match rng.below(8) {
                    0 => "\"odd\"".to_owned(),
                    1 => "{\"x\":1}".to_owned(),
                    _ => {
                        let items: Vec<String> =
                            (0..rng.below(4)).map(|_| entry_json(rng)).collect();
                        format!("[{}]", items.join(","))
                    }
                };
                members.push(format!("\"{conn}\":{value}"));
            }
            hex::encode(format!("{{{}}}", members.join(",")).as_bytes())
        }
    };
    let mut ops = Vec::new();
    for _ in 0..1 + rng.below(5) {
        let conn = tok(rng.pick(&conns));
        let pane = tok(&format!("%{}", rng.below(4)));
        ops.push(match rng.below(6) {
            0 | 1 => format!(
                "n:{conn}:{pane}:{}:{}:{}:{:x}",
                if rng.chance(70) {
                    tok("claude")
                } else {
                    "-".to_owned()
                },
                if rng.chance(50) {
                    tok("may I")
                } else {
                    "-".to_owned()
                },
                rng.below(3),
                recorded(rng).to_bits()
            ),
            2 => format!("c:{conn}:{pane}"),
            3 => format!("a:{conn}"),
            4 => format!("R:{conn}"),
            _ => "M".to_owned(),
        });
        if initial == "!" && rng.chance(30) {
            ops.push("!".to_owned());
        }
    }
    ops.push("M".to_owned());
    format!("sfile {initial} {now} {}", ops.join(" "))
}
