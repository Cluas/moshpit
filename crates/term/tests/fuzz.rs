//! B791: the terminal under bytes no program meant to send. A seeded
//! generator (stable Rust, no fuzzing crate) builds structured garbage —
//! CSI, OSC, DCS and APC sequences with absurd parameters, sixel, kitty and
//! iTerm2 images with bad headers and huge counts, unterminated and nested
//! strings, invalid UTF-8 — and feeds it in pieces, a resize, a frame, a
//! scroll, a selection, a search or a paste between them, as the app does.
//! Every case must finish without a panic, within its time, within its
//! memory (this test binary's own allocator counts).
//!
//! A short, fixed run is a regular test. A longer one:
//!
//! ```sh
//! FUZZ_SECONDS=1200 FUZZ_SEED=$RANDOM cargo test -p moshpit-term --release \
//!   --test fuzz -- --nocapture
//! ```
//!
//! What it finds becomes a regular test with its bytes (tests/fuzz_found.rs).

mod common;
use common::*;
use moshpit_term::ffi::{
    mp_frame_free, mp_term_close, mp_term_feed, mp_term_frame, mp_term_new, mp_term_resize,
    mp_term_scroll,
};
use moshpit_term::{FindOptions, FindStep, SelectKind, Terminal};
use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Panics, caught or not: the C ABI's guard catches what a call panics
/// with, so a panic there shows only here.
static PANICS: AtomicUsize = AtomicUsize::new(0);
static LAST_PANIC: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = NOW.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        NOW.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// SplitMix64: seeded, so a case that fails fails again.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

thread_local! {
    /// When the step before began, and the slowest step of the case so far.
    static STEP: std::cell::RefCell<(Option<Instant>, Duration, String)> =
        const { std::cell::RefCell::new((None, Duration::ZERO, String::new())) };
}

/// Notes a step: what it is, and how long the one before it took.
fn step(log: &mut Vec<String>, what: String) {
    let now = Instant::now();
    STEP.with(|s| {
        let mut s = s.borrow_mut();
        if let (Some(began), Some(last)) = (s.0, log.last()) {
            let took = now - began;
            if took > s.1 {
                s.1 = took;
                s.2 = last.chars().take(160).collect();
            }
        }
        s.0 = Some(now);
    });
    log.push(what);
}

/// A number a program might put in a parameter, sane or not.
fn number(r: &mut Rng) -> String {
    match r.below(8) {
        0 => String::new(),
        1 => "0".into(),
        2..=4 => r.below(200).to_string(),
        5 => r.below(70_000).to_string(),
        6 => (*r.pick(&[u32::MAX as u64, u64::MAX, 4_294_967_296, 99_999_999_999_999])).to_string(),
        _ => "9".repeat(1 + r.below(40) as usize),
    }
}

fn params(r: &mut Rng, sep: &str) -> String {
    let n = match r.below(10) {
        0 => 0,
        9 => 20 + r.below(60),
        _ => 1 + r.below(5),
    };
    (0..n)
        .map(|_| {
            if r.chance(15) {
                format!("{}:{}", number(r), number(r))
            } else {
                number(r)
            }
        })
        .collect::<Vec<_>>()
        .join(sep)
}

fn base64(r: &mut Rng, max: u64) -> String {
    let bytes = r.below(max) as usize;
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s: String = (0..bytes).map(|_| *r.pick(B64) as char).collect();
    if r.chance(20) {
        s.push_str(r.pick(&["=", "==", "===", "!!", "\u{1b}", " "]));
    }
    s
}

fn terminator(r: &mut Rng) -> &'static str {
    r.pick(&["\x1b\\", "\x07", "\x1b\\", "", "\x1b", "\u{9c}", "\x18"])
}

/// One piece of a program's output, rarely well formed.
fn piece(r: &mut Rng) -> Vec<u8> {
    let s: String = match r.below(24) {
        0..=2 => (0..1 + r.below(120))
            .map(|_| (0x20 + r.below(0x5f) as u8) as char)
            .collect(),
        3 => r
            .pick(&[
                "你好世界",
                "👨‍👩‍👧",
                "👍🏽",
                "❤️",
                "e\u{301}",
                "\u{200d}",
                "表\u{fe0f}",
                "한국어",
                "ｱｲ",
                "\u{feff}",
            ])
            .repeat(1 + r.below(30) as usize),
        4 => {
            // Raw bytes: invalid UTF-8, lone continuation bytes, C1 bytes.
            return (0..1 + r.below(64)).map(|_| r.below(256) as u8).collect();
        }
        5 => (0..1 + r.below(10))
            .map(|_| {
                *r.pick(&[
                    '\n', '\r', '\t', '\x08', '\x07', '\x0b', '\x0c', '\x0e', '\x0f', '\0', '\x7f',
                ])
            })
            .collect(),
        6..=8 => {
            let private = r.pick(&["", "", "?", ">", "<", "=", "?>"]);
            let inter = r.pick(&["", "", "", " ", "$", "\"", "!", "'", "*"]);
            let fin = (0x40 + r.below(0x3f) as u8) as char;
            format!("\x1b[{private}{}{inter}{fin}", params(r, ";"))
        }
        9 => {
            let mode = r.pick(&[
                "1049", "47", "1047", "2004", "1000", "1002", "1003", "1006", "1015", "1016",
                "2026", "1", "25", "7", "6", "69", "95", "3", "5", "12", "1004", "2031", "80",
                "8452", "1048",
            ]);
            format!("\x1b[?{mode}{}", r.pick(&["h", "l", "$p", "s", "r"]))
        }
        10 => {
            let op = r.pick(&[
                "r", "s", "S", "T", "L", "M", "@", "P", "X", "b", "J", "K", "G", "d", "H", "f",
                "E", "F", "I", "Z", "g", "t",
            ]);
            format!("\x1b[{};{}{op}", number(r), number(r))
        }
        11..=12 => {
            let code = *r.pick(&[
                0u64, 1, 2, 4, 7, 8, 10, 11, 12, 52, 104, 110, 111, 133, 633, 777, 1337, 9, 99999,
            ]);
            let body: String = match code {
                4 => format!(
                    "{};{}",
                    number(r),
                    r.pick(&["?", "rgb:ff/00/00", "#zzz", ""])
                ),
                8 => format!(
                    "{};{}",
                    r.pick(&["", "id=1", "id="]),
                    r.pick(&["https://example.com", "", "file:///tmp/a"])
                ),
                52 => format!("c;{}", base64(r, 400)),
                133 => r
                    .pick(&["A", "B", "C", "D;0", "D;999999999999", "Z"])
                    .to_string(),
                1337 => format!(
                    "File=inline=1;width={};height={};size={}:{}",
                    number(r),
                    number(r),
                    number(r),
                    base64(r, 600)
                ),
                _ => (0..r.below(200))
                    .map(|_| (0x20 + r.below(0x5f) as u8) as char)
                    .collect(),
            };
            format!("\x1b]{code};{body}{}", terminator(r))
        }
        13..=14 => {
            // Sixel, raster attributes absurd or not, data in any shape.
            let mut s = format!("\x1bP{}q", params(r, ";"));
            if r.chance(70) {
                s.push_str(&format!(
                    "\"{};{};{};{}",
                    number(r),
                    number(r),
                    number(r),
                    number(r)
                ));
            }
            for _ in 0..r.below(40) {
                match r.below(6) {
                    0 => s.push_str(&format!(
                        "#{};2;{};{};{}",
                        number(r),
                        number(r),
                        number(r),
                        number(r)
                    )),
                    1 => s.push_str(&format!(
                        "!{}{}",
                        number(r),
                        (0x3f + r.below(64) as u8) as char
                    )),
                    2 => s.push(*r.pick(&['-', '$'])),
                    3 => s.push_str(&format!("#{}", number(r))),
                    _ => s.extend((0..r.below(30)).map(|_| (0x3f + r.below(64) as u8) as char)),
                }
            }
            s.push_str(terminator(r));
            s
        }
        15 => format!(
            "\x1bP{}{}{}",
            r.pick(&["$q", "+q", "1000p", "q", "|"]),
            params(r, ";"),
            terminator(r)
        ),
        16..=17 => {
            // kitty graphics: keys in any combination, payloads in any shape.
            let keys = [
                "a", "f", "s", "v", "i", "p", "x", "y", "w", "h", "c", "r", "z", "q", "m", "t",
                "o", "I", "d", "X", "Y", "U", "C",
            ];
            let mut ctl = Vec::new();
            for _ in 0..r.below(8) {
                let key = r.pick(&keys);
                let value = match *key {
                    "a" => r
                        .pick(&["T", "t", "p", "d", "q", "f", "a", "c"])
                        .to_string(),
                    "f" => r.pick(&["32", "24", "100", "0", "7"]).to_string(),
                    "o" => "z".into(),
                    "d" => r.pick(&["a", "A", "i", "I", "p", "z", "x"]).to_string(),
                    _ => number(r),
                };
                ctl.push(format!("{key}={value}"));
            }
            format!(
                "\x1b_G{};{}{}",
                ctl.join(","),
                base64(r, 800),
                terminator(r)
            )
        }
        18 => r
            .pick(&[
                "\x1bc", "\x1b7", "\x1b8", "\x1b#8", "\x1b(0", "\x1b)B", "\x1bD", "\x1bM", "\x1bE",
                "\x1bH", "\x1b=", "\x1b>", "\x1bN", "\x1bO", "\x1b%G", "\x1b[!p",
            ])
            .to_string(),
        19 => {
            // Unterminated or nested: a string opened and another inside.
            let open = r.pick(&["\x1b]", "\x1bP", "\x1b_", "\x1b^", "\x1bX", "\x1b["]);
            let inner = r.pick(&["\x1b]8;;x", "\x1bPq#0", "\x1b[31m", "\x1b_Ga=T;", ""]);
            format!(
                "{open}{}{inner}{}",
                params(r, ";"),
                if r.chance(50) { terminator(r) } else { "" }
            )
        }
        20 => format!("\x1b[{}b", number(r)),
        21 => "\x1b[?69h\x1b[5;10s".into(),
        22 => format!("\x1b[{};{}t", number(r), number(r)),
        _ => format!(
            "\x1b]1337;{}\x07",
            r.pick(&[
                "SetMark",
                "CurrentDir=/tmp",
                "ShellIntegrationVersion=99",
                "RemoteHost=a@b"
            ])
        ),
    };
    s.into_bytes()
}

/// One case: pieces fed in random chunks, the app's other calls between.
fn case(r: &mut Rng, t: &mut Terminal, log: &mut Vec<String>) {
    for _ in 0..1 + r.below(40) {
        match r.below(14) {
            0..=7 => {
                let bytes = piece(r);
                let mut at = 0;
                while at < bytes.len() {
                    let n =
                        (1 + r.below(bytes.len() as u64)).min((bytes.len() - at) as u64) as usize;
                    step(
                        log,
                        format!("feed {:?}", String::from_utf8_lossy(&bytes[at..at + n])),
                    );
                    t.feed(&bytes[at..at + n]);
                    at += n;
                }
            }
            8 => {
                let (c, rows) = (1 + r.below(300) as u16, 1 + r.below(150) as u16);
                step(log, format!("resize {c}x{rows}"));
                t.resize(c, rows);
            }
            9 => {
                step(log, "frame".into());
                let _ = t.frame();
            }
            10 => {
                let n = r.below(400) as i32 - 200;
                step(log, format!("scroll {n}"));
                t.scroll(n);
            }
            11 => {
                let kind = *r.pick(&[
                    SelectKind::Cells,
                    SelectKind::Word,
                    SelectKind::Line,
                    SelectKind::Block,
                    SelectKind::Semantic,
                ]);
                let (a, b, c, d) = (
                    r.below(200) as u16,
                    r.below(400) as u16,
                    r.below(200) as u16,
                    r.below(400) as u16,
                );
                step(log, format!("select {kind:?} {a},{b} {c},{d}"));
                t.select_start(kind, a, b, r.chance(50));
                t.select_extend(c, d, r.chance(50));
                let _ = t.selection_text();
                let _ = t.link_at(c, d);
            }
            12 => {
                let text = r
                    .pick(&["a", "你好", "--force", "\u{1b}", "", "[", ".*", "🎉"])
                    .to_string();
                step(log, format!("find {text:?}"));
                let options = FindOptions {
                    case_sensitive: r.chance(50),
                    word_start: r.chance(30),
                    word_end: r.chance(30),
                };
                let _ = t.find(
                    &text,
                    options,
                    *r.pick(&[FindStep::Newest, FindStep::Next, FindStep::Previous]),
                );
            }
            _ => {
                let text = String::from_utf8_lossy(&piece(r)).into_owned();
                step(log, format!("paste {text:?}"));
                let _ = t.paste(&text, r.chance(50));
            }
        }
    }
    let _ = t.frame();
}

/// The same through the C ABI the app's view calls: a terminal made,
/// fed, resized, scrolled and framed by `mp_term_*`.
fn ffi_case(r: &mut Rng, cols: u16, rows: u16, scrollback: u32, log: &mut Vec<String>) {
    let t = mp_term_new(cols, rows, scrollback);
    assert!(!t.is_null());
    for _ in 0..1 + r.below(30) {
        match r.below(10) {
            0..=6 => {
                let bytes = piece(r);
                step(
                    log,
                    format!("mp_term_feed {:?}", String::from_utf8_lossy(&bytes)),
                );
                // SAFETY: a live terminal, bytes of their length.
                unsafe { mp_term_feed(t, bytes.as_ptr(), bytes.len()) };
            }
            7 => {
                let (c, rows) = (r.below(400) as u16, r.below(200) as u16);
                step(log, format!("mp_term_resize {c}x{rows}"));
                // SAFETY: a live terminal.
                unsafe { mp_term_resize(t, c, rows) };
            }
            8 => {
                let n = r.below(400) as i32 - 200;
                step(log, format!("mp_term_scroll {n}"));
                // SAFETY: a live terminal.
                unsafe { mp_term_scroll(t, n) };
            }
            _ => {
                step(log, "mp_term_frame".into());
                // SAFETY: a live terminal; the frame freed once.
                unsafe { mp_frame_free(mp_term_frame(t)) };
            }
        }
    }
    // SAFETY: made above, freed once.
    unsafe { mp_term_close(t) };
}

/// What a case may take: time, and memory beyond what was live before it.
const CASE_TIME: Duration = Duration::from_secs(3);
const CASE_MEMORY: usize = 256 << 20;

fn run(seed: u64, cases: Option<u64>, seconds: Option<u64>) -> Vec<String> {
    let started = Instant::now();
    let mut findings = Vec::new();
    let mut n = 0u64;
    std::panic::set_hook(Box::new(|info| {
        PANICS.fetch_add(1, Relaxed);
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = info.to_string();
        }
    }));
    loop {
        if cases.is_some_and(|c| n >= c)
            || seconds.is_some_and(|s| started.elapsed() > Duration::from_secs(s))
        {
            break;
        }
        let case_seed = seed.wrapping_add(n.wrapping_mul(0x9E37_79B9));
        let mut r = Rng(case_seed);
        let (cols, rows) = (1 + r.below(200) as u16, 1 + r.below(80) as u16);
        let scrollback = *r.pick(&[0usize, 10, 1000]);
        let mut log = vec![format!("term {cols}x{rows} scrollback {scrollback}")];
        let through_ffi = r.chance(30);
        if through_ffi {
            log[0].insert_str(0, "C ABI: ");
        }
        let before = NOW.load(Relaxed);
        PEAK.store(before, Relaxed);
        let panics = PANICS.load(Relaxed);
        STEP.with(|s| *s.borrow_mut() = (None, Duration::ZERO, String::new()));
        let at = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            if through_ffi {
                ffi_case(&mut r, cols, rows, scrollback as u32, &mut log);
            } else {
                let mut t = term(cols, rows, scrollback);
                case(&mut r, &mut t, &mut log);
            }
            step(&mut log, "end".into());
        }));
        let slowest = STEP.with(|s| {
            let s = s.borrow();
            format!("slowest step {:?}: {}", s.1, s.2)
        });
        let took = at.elapsed();
        let peak = PEAK.load(Relaxed).saturating_sub(before);
        let what = if outcome.is_err() || PANICS.load(Relaxed) != panics {
            Some("panic")
        } else if took > CASE_TIME {
            Some("slow")
        } else if peak > CASE_MEMORY {
            Some("memory")
        } else {
            None
        };
        if let Some(what) = what {
            let message = LAST_PANIC.lock().map(|m| m.clone()).unwrap_or_default();
            let why = if what == "panic" {
                message.lines().next().unwrap_or_default().to_owned()
            } else {
                slowest
            };
            eprintln!(
                "{what}: seed {case_seed:#x} ({took:?}, {} MB peak) {why}",
                peak >> 20
            );
            findings.push(format!(
                "{what}: seed {case_seed:#x} ({:?}, {} MB peak) {why}; steps:\n  {}",
                took,
                peak >> 20,
                log.join("\n  ")
            ));
        }
        n += 1;
    }
    let _ = std::panic::take_hook();
    eprintln!(
        "fuzz: {n} cases from seed {seed:#x} in {:?}, {} findings",
        started.elapsed(),
        findings.len()
    );
    findings
}

#[test]
fn b791_garbage_never_panics_hangs_or_blows_up() {
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
    let seed = env("FUZZ_SEED").unwrap_or(0xB791);
    let seconds = env("FUZZ_SECONDS");
    let cases = if seconds.is_some() {
        env("FUZZ_CASES")
    } else {
        Some(env("FUZZ_CASES").unwrap_or(120))
    };
    let findings = run(seed, cases, seconds);
    for f in findings.iter().take(5) {
        eprintln!("{f}\n");
    }
    assert!(
        findings.is_empty(),
        "{} findings (first ones above)",
        findings.len()
    );
}
