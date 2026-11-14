//! T9: a tmux pane's terminal keeps what tmux keeps. The same bytes go to
//! a private tmux (its own socket, no configuration, a pane whose program
//! writes them, as a `-CC` client's pane receives them) and to a terminal
//! in tmux's mode ([`ScrollOnClear::Tmux`]); the history and the screen
//! are compared line for line. Skipped where tmux is not installed.

mod common;
use moshpit_term::{Options, ScrollOnClear, Terminal};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

const COLS: u16 = 40;
const ROWS: u16 = 10;

fn tmux() -> Option<PathBuf> {
    [
        "/opt/homebrew/bin/tmux",
        "/usr/local/bin/tmux",
        "/usr/bin/tmux",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// What tmux keeps of `data`: its history and its screen, each line trimmed
/// at the right.
fn reference(tmux: &Path, data: &[u8]) -> (Vec<String>, Vec<String>) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    // Short: a Unix socket's path is at most 104 bytes, and the system's
    // temporary directory is long on macOS.
    let dir = PathBuf::from(format!("/tmp/mp9-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.bin");
    let done = dir.join("done");
    std::fs::write(&input, data).unwrap();
    let socket = "t9".to_owned();
    let run = |args: &[&str]| -> String {
        let out = Command::new(tmux)
            .args(["-L", &socket, "-f", "/dev/null"])
            .args(args)
            .env("TMUX_TMPDIR", &dir)
            .env("HOME", &dir)
            .env_remove("TMUX")
            .output()
            .expect("tmux runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let script = format!(
        "stty -onlcr; sleep 0.3; cat '{}'; touch '{}'; exec sleep 100000",
        input.display(),
        done.display()
    );
    run(&[
        "new-session",
        "-d",
        "-x",
        &COLS.to_string(),
        "-y",
        &ROWS.to_string(),
        "sh",
        "-c",
        &script,
    ]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done.exists() {
        assert!(Instant::now() < deadline, "tmux never ran the pane");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    let kept: usize = run(&["display", "-p", "#{history_size}"])
        .trim()
        .parse()
        .unwrap();
    let history = if kept == 0 {
        vec![]
    } else {
        run(&["capture-pane", "-p", "-S", &format!("-{kept}"), "-E", "-1"])
            .lines()
            .take(kept)
            .map(|l| l.trim_end().to_owned())
            .collect()
    };
    let screen = run(&["capture-pane", "-p"])
        .lines()
        .take(ROWS as usize)
        .map(|l| l.trim_end().to_owned())
        .collect();
    run(&["kill-server"]);
    let _ = std::fs::remove_dir_all(&dir);
    (history, screen)
}

/// What a terminal in tmux's mode keeps of `data`.
fn ours(data: &[u8]) -> (Vec<String>, Vec<String>) {
    let mut t = Terminal::new(
        COLS,
        ROWS,
        Options {
            scroll_on_clear: ScrollOnClear::Tmux,
            ..Options::default()
        },
    );
    t.feed(data);
    let history = common::history(&t)
        .into_iter()
        .map(|r| r.0.trim_end().to_owned())
        .collect();
    let mut screen: Vec<String> = common::rows(&t)
        .into_iter()
        .map(|r| r.trim_end().to_owned())
        .collect();
    screen.resize(ROWS as usize, String::new());
    (history, screen)
}

fn lines(n: usize, tag: &str) -> Vec<u8> {
    (1..=n)
        .flat_map(|i| format!("{tag}-{i}\r\n").into_bytes())
        .collect()
}

/// A Claude Code-style inline frame: a synchronized update that goes back
/// up over the last eight lines and rewrites them.
fn agent_frame(f: usize) -> Vec<u8> {
    let mut s = String::from("\x1b[?2026h\x1b[8A");
    for l in 0..8 {
        s += &format!(
            "\x1b[2K\x1b[38;5;{}m* thinking {f} line {l}\x1b[0m\r\n",
            f % 200
        );
    }
    s += "\x1b[?2026l";
    s.into_bytes()
}

#[allow(clippy::vec_init_then_push)] // One case a push, each with its name.
fn cases() -> Vec<(&'static str, Vec<u8>)> {
    let mut v = vec![];
    v.push((
        "clear (ESC [H ESC [J)",
        [lines(25, "out"), b"\x1b[H\x1b[Jafter".to_vec()].concat(),
    ));
    v.push((
        "ESC [J, not from the top left",
        [lines(7, "out"), b"\x1b[3;1H\x1b[Jafter".to_vec()].concat(),
    ));
    v.push((
        "ESC [2J, the cursor mid-screen",
        [lines(7, "out"), b"\x1b[3;5H\x1b[2Jafter".to_vec()].concat(),
    ));
    v.push((
        "clear on the alternate screen",
        [
            b"\x1b[?1049h".to_vec(),
            lines(7, "alt"),
            b"\x1b[H\x1b[Jafter\x1b[?1049l".to_vec(),
        ]
        .concat(),
    ));
    v.push((
        "clear twice",
        [lines(7, "out"), b"\x1b[H\x1b[J\x1b[H\x1b[Jx".to_vec()].concat(),
    ));
    // A TUI redrawing in place from the top, erasing below what it drew.
    let mut tui = lines(12, "before");
    for f in 0..30 {
        tui.extend(b"\x1b[H");
        for r in 0..5 {
            tui.extend(format!("frame {f} row {r}\x1b[K\r\n").into_bytes());
        }
        tui.extend(b"\x1b[J");
    }
    v.push(("a TUI redrawing in place", tui));
    // One that clears from the top left every frame (`watch`-like).
    let mut watch = lines(4, "before");
    for f in 0..12 {
        watch.extend(b"\x1b[H\x1b[J");
        watch.extend(format!("every 1s: frame {f}\r\nvalue {}\r\n", f * 7).into_bytes());
    }
    v.push(("a TUI clearing every frame", watch));
    // Claude Code's inline redraw, after its scrolled output.
    let mut agent = lines(30, "log");
    agent.extend(lines(8, "draft"));
    for f in 0..50 {
        agent.extend(agent_frame(f));
    }
    v.push(("a Claude-style inline redraw", agent));
    // Claude Code clearing to redraw everything when its frame overflows.
    v.push((
        "a Claude-style clear and redraw",
        [
            lines(30, "log"),
            b"\x1b[2J\x1b[3J\x1b[H".to_vec(),
            lines(6, "redrawn"),
        ]
        .concat(),
    ));
    v
}

#[test]
fn a_tmux_pane_keeps_what_tmux_keeps() {
    let Some(tmux) = tmux() else {
        eprintln!("no tmux: skipped");
        return;
    };
    let mut wrong = vec![];
    for (name, data) in cases() {
        let want = reference(&tmux, &data);
        let got = ours(&data);
        if got != want {
            wrong.push(format!("{name}:\n  tmux: {want:?}\n  ours: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A terminal of its own (plain SSH, mosh, herdr) is unchanged: `ESC [J`
/// from the top left erases, keeping nothing.
#[test]
fn a_terminal_of_its_own_keeps_its_clear() {
    let mut t = common::term(COLS, ROWS, 1000);
    t.feed(&[lines(7, "out"), b"\x1b[H\x1b[Jafter".to_vec()].concat());
    assert!(common::history(&t).is_empty());
}

/// The one difference, by construction: tmux counts a line as used once a
/// program wrote to it, spaces included (its `cellused`); our emulator
/// keeps no such mark, so a line of written spaces below the last line
/// with text is not kept (an erased line is not kept by either). The lines
/// with text, and everything above them, are the same.
#[test]
fn written_spaces_below_the_text_are_the_one_difference() {
    let Some(tmux) = tmux() else {
        eprintln!("no tmux: skipped");
        return;
    };
    let data = b"a\r\n\r\n   \r\n\x1b[H\x1b[Jafter";
    let (want, _) = reference(&tmux, data);
    let (got, _) = ours(data);
    assert_eq!(want, ["a", "", ""], "tmux keeps the blank and the spaces");
    assert_eq!(got, ["a"]);
    let erased = b"a\r\nxyz\r\x1b[2K\r\n\x1b[H\x1b[Jafter";
    assert_eq!(
        reference(&tmux, erased).0,
        ours(erased).0,
        "an erased line: the same"
    );
}
