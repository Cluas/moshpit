//! The CLI lab's recordings (scripts/cli-lab, docs/nsh/CLI-LAB.md) played
//! through the core the way a pane does: each recorded chunk is fed and
//! published as a frame, and what a viewer of those frames saw is checked.
//!
//! - **final**: the last screen and cursor are tmux's own (`capture-pane`),
//!   for the recordings made through tmux;
//! - **typed**: when each key after the first is sent, the text typed so far
//!   is on screen where the whole line ends up (unless the host sent nothing
//!   since the key before: its lag);
//! - **steady**: between the first key and Enter, no frame shows the typed
//!   text shorter than an earlier frame did (a torn frame), and a visible
//!   cursor stays on the input row;
//! - **predicted** (mosh recordings): with mosh's predictive echo on, the
//!   guesses land on the input row only and the cursor does not leave it.
//!
//! A recording is a JSON line per event: `{"o": base64}` bytes the pane got,
//! `{"k": key}` a key sent, `{"ref": rows, "cursor": [x, y, shown]}` tmux's
//! screen at the end; the first line says which CLI, path, size and text.

mod common;
use base64::Engine as _;
use moshpit_term::coalesce::Coalescer;
use moshpit_term::{Options, ScrollOnClear, Terminal, cell_flags as F, predict::PredictMode};
use serde_json::Value;
use std::time::{Duration, Instant};

enum Ev {
    /// Bytes, and when they came (ms).
    Out(Vec<u8>, u64),
    Key(String, u64),
}

struct Rec {
    path: String,
    cols: u16,
    rows: u16,
    text: String,
    events: Vec<Ev>,
    /// tmux's screen and cursor after the first `at` events: the live
    /// capture at the end (tmux recordings), or tmux_ref.py's replay.
    refs: Vec<Ref>,
}

struct Ref {
    at: usize,
    rows: Vec<String>,
    cursor: (u16, u16),
}

fn load(name: &str) -> Rec {
    let file = common::repo()
        .join("crates/term/tests/fixtures/cli-lab")
        .join(format!("{name}.jsonl"));
    let src = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let mut lines = src
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let head = lines.next().unwrap();
    let mut rec = Rec {
        path: head["path"].as_str().unwrap().to_string(),
        cols: head["cols"].as_u64().unwrap() as u16,
        rows: head["rows"].as_u64().unwrap() as u16,
        text: head["text"].as_str().unwrap().to_string(),
        events: vec![],
        refs: vec![],
    };
    for v in lines {
        let t = v["t"].as_u64().unwrap_or(0);
        if let Some(o) = v["o"].as_str() {
            rec.events.push(Ev::Out(
                base64::engine::general_purpose::STANDARD.decode(o).unwrap(),
                t,
            ));
        } else if let Some(k) = v["k"].as_str() {
            rec.events.push(Ev::Key(k.to_string(), t));
        } else if let Some(r) = v["ref"].as_array() {
            let rows = r
                .iter()
                .map(|l| l.as_str().unwrap().trim_end().to_string())
                .collect();
            let c = v["cursor"].as_array().unwrap();
            let at = v["at"].as_u64().map_or(rec.events.len(), |n| n as usize);
            let cursor = (c[0].as_u64().unwrap() as u16, c[1].as_u64().unwrap() as u16);
            rec.refs.push(Ref { at, rows, cursor });
        }
    }
    rec
}

/// What a viewer of the published frames sees.
#[derive(Clone, Default)]
struct Shown {
    rows: Vec<String>,
    /// (col, row) when the cursor is drawn.
    cursor: Option<(u16, u16)>,
}

impl Shown {
    fn apply(&mut self, t: &mut Terminal) {
        let f = t.frame();
        let n = f.rows as usize;
        if f.full || self.rows.len() != n {
            self.rows = vec![String::new(); n];
        } else if f.shift > 0 {
            let s = (f.shift as usize).min(n);
            self.rows.drain(..s);
            self.rows.resize(n, String::new());
        } else if f.shift < 0 {
            let s = (-f.shift as usize).min(n);
            self.rows.truncate(n - s);
            for _ in 0..s {
                self.rows.insert(0, String::new());
            }
        }
        for r in &f.changed {
            let s: String = r
                .cells
                .iter()
                .filter(|c| c.flags & F::SPACER == 0)
                // Concealed text (SGR 8) is drawn in the background's colour:
                // a viewer sees a blank.
                .map(|c| {
                    char::from_u32(c.ch)
                        .filter(|&ch| ch != '\0' && c.fg != c.bg)
                        .unwrap_or(' ')
                })
                .collect();
            if let Some(row) = self.rows.get_mut(r.row as usize) {
                *row = s.trim_end().to_string();
            }
        }
        self.cursor = f.cursor.visible.then_some((f.cursor.col, f.cursor.row));
    }
}

/// One published frame, and how many keys had been sent before it.
struct Step {
    keys: usize,
    shown: Shown,
}

struct Play {
    steps: Vec<Step>,
    /// Keys in order (the text's characters, then "Enter").
    keys: Vec<String>,
    end: Terminal,
}

/// Feeds the recording chunk by chunk, each chunk a read, and publishes a
/// frame when a pane would: when the output held is due
/// ([`moshpit_term::coalesce`], at the chunks' recorded times).
/// `predict`: mosh's predictive echo on, each key acked by the first chunk
/// after it (the host's echo).
fn play(rec: &Rec, predict: Option<PredictMode>) -> Play {
    play_to(rec, predict, rec.events.len())
}

/// As [`play`], the first `n` events only.
fn play_to(rec: &Rec, predict: Option<PredictMode>, n: usize) -> Play {
    let mut t = Terminal::new(
        rec.cols,
        rec.rows,
        Options {
            scrollback: 1000,
            scroll_on_clear: if rec.path.contains("tmux") {
                ScrollOnClear::Tmux
            } else {
                Options::default().scroll_on_clear
            },
            mosh_host: rec.path.starts_with("mosh"),
            ..Options::default()
        },
    );
    if let Some(mode) = predict {
        t.predict_attach(mode, false);
        t.predict_srtt(250.0);
    }
    let mut shown = Shown::default();
    let mut steps = vec![];
    let mut keys: Vec<String> = vec![];
    let mut sent = 0u64;
    let mut acked = 0u64;
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut held = Coalescer::default();
    // A pane's publish: not inside a synchronized update.
    let mut publish = |t: &mut Terminal, shown: &mut Shown, keys: usize, held: &mut Coalescer| {
        held.framed();
        if t.sync_deadline().is_none() {
            shown.apply(t);
            steps.push(Step {
                keys,
                shown: shown.clone(),
            });
        }
    };
    for ev in &rec.events[..n] {
        let now = match ev {
            Ev::Out(_, ms) | Ev::Key(_, ms) => at(*ms),
        };
        // What was due before this event, with the keys sent before it.
        if held.due().is_some_and(|d| d < now) && t.sync_deadline().is_none() {
            publish(&mut t, &mut shown, keys.len(), &mut held);
        }
        match ev {
            Ev::Out(b, _) => {
                t.feed(b);
                if acked < sent {
                    acked = sent;
                    t.predict_echo_ack(acked);
                }
                held.output(now, t.sync_settled());
                if held.ready(now) && t.sync_deadline().is_none() {
                    publish(&mut t, &mut shown, keys.len(), &mut held);
                }
            }
            Ev::Key(k, _) => {
                keys.push(k.clone());
                if predict.is_some() {
                    let bytes = if k == "Enter" {
                        b"\r".to_vec()
                    } else {
                        k.as_bytes().to_vec()
                    };
                    if let Some(batch) = t.predict_input(&bytes) {
                        sent += 1;
                        t.predict_sent(batch, Some(sent));
                    }
                }
            }
        }
    }
    if t.sync_deadline().is_some() {
        t.flush_sync();
    }
    shown.apply(&mut t);
    steps.push(Step {
        keys: keys.len(),
        shown,
    });
    Play {
        steps,
        keys,
        end: t,
    }
}

/// The input line just before Enter: the prompt before the typed text (the
/// row's first characters) and where the text starts (a char index).
struct Input {
    prompt: String,
    col: usize,
}

fn input_of(rec: &Rec, p: &Play) -> Result<Input, String> {
    let n = p
        .keys
        .iter()
        .position(|k| k == "Enter")
        .ok_or("no Enter in the recording")?;
    let before = p
        .steps
        .iter()
        .rev()
        .find(|s| s.keys == n)
        .ok_or("no frame before Enter")?;
    // Over a slow link the whole text may show only after Enter was sent
    // (the echo is a round trip away): the first frame that has it then.
    let later = p.steps.iter().filter(|s| s.keys > n);
    for s in std::iter::once(before).chain(later) {
        for row in &s.shown.rows {
            if let Some(i) = row.find(&rec.text) {
                return Ok(Input {
                    prompt: row[..i].to_string(),
                    col: row[..i].chars().count(),
                });
            }
        }
    }
    Err(format!(
        "{:?} is not on screen before Enter:\n{}",
        rec.text,
        before.shown.rows.join("\n")
    ))
}

/// The input row in a frame (the box can move while one types: a notice
/// printed above it) and how much of the text it shows: the row with the
/// prompt that shows the most.
fn typed(s: &Shown, inp: &Input, text: &str) -> Option<(usize, usize)> {
    s.rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.starts_with(&inp.prompt))
        .map(|(r, row)| {
            // Rows are trimmed: a typed blank at the end shows as nothing.
            let tail = row.chars().skip(inp.col).chain(std::iter::repeat(' '));
            (
                r,
                text.chars().zip(tail).take_while(|(a, b)| a == b).count(),
            )
        })
        .max_by_key(|&(r, n)| (n, std::cmp::Reverse(r)))
}

fn check_final(rec: &Rec, _: &Play) -> Result<(), String> {
    if rec.refs.is_empty() {
        return Err("no tmux reference".into());
    }
    let mut bad = vec![];
    for r in &rec.refs {
        let end = play_to(rec, None, r.at).end;
        let got = common::rows(&end);
        for (i, want) in r.rows.iter().enumerate() {
            // tmux keeps a zero-width space as a cell's text; the core
            // drops it. Neither draws anything.
            let want = want.replace('\u{200b}', "");
            let have = got.get(i).map(|s| s.trim_end()).unwrap_or("");
            if have != want.trim_end() {
                bad.push(format!(
                    "at {}: row {i}:\n  tmux:    {want:?}\n  moshpit: {have:?}",
                    r.at
                ));
            }
        }
        if end.cursor() != r.cursor {
            bad.push(format!(
                "at {}: cursor: tmux {:?}, moshpit {:?}",
                r.at,
                r.cursor,
                end.cursor()
            ));
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

fn check_typed(rec: &Rec, p: &Play) -> Result<(), String> {
    let inp = input_of(rec, p)?;
    let n = rec.text.chars().count();
    let mut bad = vec![];
    // Where each key is in the recording (steps follow events one to one).
    let at: Vec<usize> = rec
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Ev::Key(..)))
        .map(|(i, _)| i)
        .collect();
    // The screen when key k+1 is sent: k keys were sent before it.
    for k in 1..=n {
        // Nothing came back between the two keys: the host had not drawn
        // the key yet (its lag, not the view's).
        let next = at.get(k).copied().unwrap_or(rec.events.len());
        if !rec.events[at[k - 1]..next]
            .iter()
            .any(|e| matches!(e, Ev::Out(..)))
        {
            continue;
        }
        let Some(s) = p.steps.iter().rev().find(|s| s.keys == k) else {
            bad.push(format!("no frame after key {k}"));
            continue;
        };
        let (r, got) = typed(&s.shown, &inp, &rec.text).unwrap_or((0, 0));
        if got < k {
            bad.push(format!(
                "after {k} keys the input row shows {got} of them: {:?}",
                s.shown.rows[r]
            ));
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

fn check_steady(rec: &Rec, p: &Play) -> Result<(), String> {
    let inp = input_of(rec, p)?;
    let enter = p.keys.iter().position(|k| k == "Enter").unwrap();
    let mut best = 0;
    let mut bad = vec![];
    for (i, s) in p.steps.iter().enumerate() {
        if s.keys == 0 || s.keys > enter {
            continue;
        }
        let (r, got) = typed(&s.shown, &inp, &rec.text).unwrap_or((usize::MAX, 0));
        if got < best {
            let row = s
                .shown
                .rows
                .get(r)
                .map(String::as_str)
                .unwrap_or("(no input row)");
            bad.push(format!(
                "frame {i} ({} keys): {got} typed chars shown after {best}: {row:?}",
                s.keys
            ));
        }
        best = best.max(got);
        if let (Some((col, row)), true) = (s.shown.cursor, got > 0) {
            if row as usize != r {
                bad.push(format!(
                    "frame {i} ({} keys): cursor at row {row} col {col}, input row {r}",
                    s.keys
                ));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

/// From the first key to the end (the reply streaming in included), a
/// visible cursor is on an input row: a row that starts with the prompt
/// (the box may move). A frame with the cursor anywhere else is the cursor
/// jumping: a state taken while the CLI was redrawing.
fn check_cursor(rec: &Rec, p: &Play) -> Result<(), String> {
    let inp = input_of(rec, p)?;
    let mut bad = vec![];
    for (i, s) in p.steps.iter().enumerate() {
        let Some((col, row)) = s.shown.cursor.filter(|_| s.keys > 0) else {
            continue;
        };
        let line = s.shown.rows.get(row as usize).map_or("", String::as_str);
        if !line.starts_with(inp.prompt.trim_end()) {
            bad.push(format!(
                "frame {i} ({} keys): cursor at row {row} col {col}: {line:?}",
                s.keys
            ));
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

/// With predictions on, frames differ from the plain replay's only on the
/// input row, and a visible cursor stays on it.
fn check_predicted(rec: &Rec, plain: &Play, mode: PredictMode) -> Result<(), String> {
    let inp = input_of(rec, plain)?;
    let p = play(rec, Some(mode));
    let enter = p.keys.iter().position(|k| k == "Enter").unwrap();
    let mut bad = vec![];
    for (i, (s, q)) in plain.steps.iter().zip(&p.steps).enumerate() {
        if s.keys == 0 || s.keys > enter {
            continue;
        }
        let r0 = typed(&s.shown, &inp, &rec.text).map(|t| t.0);
        for (r, (a, b)) in s.shown.rows.iter().zip(&q.shown.rows).enumerate() {
            if Some(r) != r0 && a != b {
                bad.push(format!(
                    "frame {i} ({} keys): row {r} {a:?} drawn as {b:?}",
                    s.keys
                ));
            }
        }
        if let (Some((col, row)), Some(r0)) = (q.shown.cursor, r0) {
            if row as usize != r0 {
                bad.push(format!(
                    "frame {i} ({} keys): predicted cursor at row {row} col {col}, input row {r0}",
                    s.keys
                ));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

#[derive(Clone, Copy, Debug)]
enum Check {
    Final,
    Typed,
    Steady,
    Cursor,
    Predicted,
}

fn run(name: &str, check: Check) -> Result<(), String> {
    let rec = load(name);
    let plain = play(&rec, None);
    match check {
        Check::Final => check_final(&rec, &plain),
        Check::Typed => check_typed(&rec, &plain),
        Check::Steady => check_steady(&rec, &plain),
        Check::Cursor => check_cursor(&rec, &plain),
        Check::Predicted => check_predicted(&rec, &plain, PredictMode::Always),
    }
}

fn assert_check(name: &str, check: Check) {
    if let Err(e) = run(name, check) {
        let e: String = e.lines().take(20).collect::<Vec<_>>().join("\n");
        panic!("{name} {check:?}:\n{e}");
    }
}

macro_rules! lab {
    ($(#[$m:meta])* $test:ident, $name:literal, $check:ident) => {
        $(#[$m])*
        #[test]
        fn $test() {
            assert_check($name, Check::$check);
        }
    };
}

// One test per recording and check that applies (`lab_table` lists them).
lab!(shell_tmux_final, "shell-tmux", Final);
lab!(shell_tmux_typed, "shell-tmux", Typed);
lab!(shell_tmux_steady, "shell-tmux", Steady);
lab!(shell_ssh_final, "shell-ssh", Final);
lab!(shell_ssh_typed, "shell-ssh", Typed);
lab!(shell_ssh_steady, "shell-ssh", Steady);
lab!(shell_mosh_final, "shell-mosh", Final);
lab!(shell_mosh_typed, "shell-mosh", Typed);
lab!(shell_mosh_steady, "shell-mosh", Steady);
lab!(shell_mosh_predicted, "shell-mosh", Predicted);
lab!(claude_tmux_final, "claude-tmux", Final);
lab!(claude_tmux_typed, "claude-tmux", Typed);
lab!(claude_tmux_steady, "claude-tmux", Steady);
lab!(claude_ssh_final, "claude-ssh", Final);
lab!(claude_ssh_typed, "claude-ssh", Typed);
lab!(claude_ssh_steady, "claude-ssh", Steady);
lab!(claude_mosh_final, "claude-mosh", Final);
lab!(claude_mosh_typed, "claude-mosh", Typed);
lab!(claude_mosh_steady, "claude-mosh", Steady);
lab!(claude_mosh_predicted, "claude-mosh", Predicted);
lab!(claude_mosh_tmux_final, "claude-mosh-tmux", Final);
lab!(claude_mosh_tmux_typed, "claude-mosh-tmux", Typed);
lab!(claude_mosh_tmux_steady, "claude-mosh-tmux", Steady);
lab!(claude_mosh_tmux_predicted, "claude-mosh-tmux", Predicted);
lab!(
    claude_mosh_pred_always_final,
    "claude-mosh-pred-always",
    Final
);
lab!(
    claude_mosh_pred_always_typed,
    "claude-mosh-pred-always",
    Typed
);
lab!(
    claude_mosh_pred_always_steady,
    "claude-mosh-pred-always",
    Steady
);
lab!(
    claude_mosh_pred_always_predicted,
    "claude-mosh-pred-always",
    Predicted
);
lab!(codex_tmux_final, "codex-tmux", Final);
lab!(codex_tmux_typed, "codex-tmux", Typed);
lab!(codex_tmux_steady, "codex-tmux", Steady);
lab!(codex_ssh_final, "codex-ssh", Final);
lab!(codex_ssh_typed, "codex-ssh", Typed);
lab!(codex_ssh_steady, "codex-ssh", Steady);
lab!(codex_mosh_final, "codex-mosh", Final);
lab!(codex_mosh_typed, "codex-mosh", Typed);
lab!(codex_mosh_steady, "codex-mosh", Steady);
lab!(codex_mosh_predicted, "codex-mosh", Predicted);
lab!(codex_mosh_tmux_final, "codex-mosh-tmux", Final);
lab!(codex_mosh_tmux_typed, "codex-mosh-tmux", Typed);
lab!(codex_mosh_tmux_steady, "codex-mosh-tmux", Steady);
lab!(codex_mosh_tmux_predicted, "codex-mosh-tmux", Predicted);
lab!(
    codex_mosh_pred_always_final,
    "codex-mosh-pred-always",
    Final
);
lab!(
    codex_mosh_pred_always_typed,
    "codex-mosh-pred-always",
    Typed
);
lab!(
    codex_mosh_pred_always_steady,
    "codex-mosh-pred-always",
    Steady
);
lab!(
    codex_mosh_pred_always_predicted,
    "codex-mosh-pred-always",
    Predicted
);
lab!(gemini_tmux_final, "gemini-tmux", Final);
lab!(gemini_tmux_typed, "gemini-tmux", Typed);
lab!(gemini_tmux_steady, "gemini-tmux", Steady);
lab!(gemini_ssh_final, "gemini-ssh", Final);
lab!(gemini_ssh_typed, "gemini-ssh", Typed);
lab!(gemini_ssh_steady, "gemini-ssh", Steady);
lab!(gemini_mosh_final, "gemini-mosh", Final);
lab!(gemini_mosh_typed, "gemini-mosh", Typed);
lab!(gemini_mosh_steady, "gemini-mosh", Steady);
lab!(gemini_mosh_predicted, "gemini-mosh", Predicted);
lab!(qwen_tmux_final, "qwen-tmux", Final);
lab!(qwen_tmux_typed, "qwen-tmux", Typed);
lab!(qwen_tmux_steady, "qwen-tmux", Steady);
lab!(qwen_ssh_final, "qwen-ssh", Final);
lab!(qwen_ssh_typed, "qwen-ssh", Typed);
lab!(qwen_ssh_steady, "qwen-ssh", Steady);
lab!(qwen_mosh_final, "qwen-mosh", Final);
lab!(qwen_mosh_typed, "qwen-mosh", Typed);
lab!(qwen_mosh_steady, "qwen-mosh", Steady);
lab!(qwen_mosh_predicted, "qwen-mosh", Predicted);
lab!(copilot_tmux_final, "copilot-tmux", Final);
lab!(copilot_tmux_typed, "copilot-tmux", Typed);
lab!(copilot_tmux_steady, "copilot-tmux", Steady);
lab!(copilot_ssh_final, "copilot-ssh", Final);
lab!(copilot_ssh_typed, "copilot-ssh", Typed);
lab!(copilot_ssh_steady, "copilot-ssh", Steady);
lab!(copilot_mosh_final, "copilot-mosh", Final);
lab!(copilot_mosh_typed, "copilot-mosh", Typed);
lab!(copilot_mosh_steady, "copilot-mosh", Steady);
lab!(copilot_mosh_predicted, "copilot-mosh", Predicted);
lab!(qoder_tmux_final, "qoder-tmux", Final);
lab!(qoder_ssh_final, "qoder-ssh", Final);
lab!(qoder_mosh_final, "qoder-mosh", Final);
// A visible cursor stays on the input row while the CLI answers.
lab!(claude_tmux_cursor, "claude-tmux", Cursor);
lab!(claude_mosh_cursor, "claude-mosh", Cursor);
lab!(codex_tmux_cursor, "codex-tmux", Cursor);
lab!(codex_mosh_cursor, "codex-mosh", Cursor);
lab!(codex_mosh_tmux_cursor, "codex-mosh-tmux", Cursor);
// Codex inside tmux over Moshpit's own transport (`-rs`, recorded with
// mosh_harness) and a 500 ms round trip, a long streamed answer.
lab!(
    codex_mosh_tmux_rtt500_rs_steady,
    "codex-mosh-tmux-rtt500-rs",
    Steady
);
lab!(
    #[ignore = "red: the mosh cursor jump (CLI-LAB.md, Bug 2): the transport applies one host state per round trip, so a state mosh-server snapshotted mid-redraw stays up ~0.75 s"]
    codex_mosh_tmux_rtt500_rs_cursor,
    "codex-mosh-tmux-rtt500-rs",
    Cursor
);

const ALL: &[&str] = &[
    "shell-tmux",
    "shell-ssh",
    "shell-mosh",
    "claude-tmux",
    "claude-ssh",
    "claude-mosh",
    "claude-mosh-tmux",
    "claude-mosh-pred-always",
    "codex-tmux",
    "codex-ssh",
    "codex-mosh",
    "codex-mosh-tmux",
    "codex-mosh-pred-always",
    "gemini-tmux",
    "gemini-ssh",
    "gemini-mosh",
    "qwen-tmux",
    "qwen-ssh",
    "qwen-mosh",
    "copilot-tmux",
    "copilot-ssh",
    "copilot-mosh",
    "qoder-tmux",
    "qoder-ssh",
    "qoder-mosh",
    "codex-mosh-tmux-rtt500-rs",
];

/// `cargo test --test cli_lab -- --ignored --nocapture lab_table`: every
/// recording against every check, one line each (the table in CLI-LAB.md).
#[test]
#[ignore = "report, not a check"]
fn lab_table() {
    for name in ALL {
        for check in [
            Check::Final,
            Check::Typed,
            Check::Steady,
            Check::Cursor,
            Check::Predicted,
        ] {
            let rec = load(name);
            if matches!(check, Check::Final) && rec.refs.is_empty() {
                continue;
            }
            if matches!(check, Check::Predicted) && !rec.path.starts_with("mosh") {
                continue;
            }
            // Qoder exits before it can be typed into: its first screen only.
            if !matches!(check, Check::Final)
                && !rec
                    .events
                    .iter()
                    .any(|e| matches!(e, Ev::Key(k, _) if k == "Enter"))
            {
                continue;
            }
            match run(name, check) {
                Ok(()) => println!("{name:28} {check:?}: pass"),
                Err(e) => println!(
                    "{name:28} {check:?}: FAIL {}",
                    e.lines().take(6).collect::<Vec<_>>().join("\n      ")
                ),
            }
        }
    }
}

/// `LAB=codex-tmux cargo test --test cli_lab -- --ignored --nocapture
/// lab_frames`: the screen a viewer had when each key was sent, and at
/// the end (`LAB_ALL=1`: every frame).
#[test]
#[ignore = "report, not a check"]
fn lab_frames() {
    let Ok(name) = std::env::var("LAB") else {
        return;
    };
    let rec = load(&name);
    let p = play(&rec, None);
    let every = std::env::var("LAB_ALL").is_ok();
    let mut last = usize::MAX;
    for (i, s) in p.steps.iter().enumerate() {
        let next = p.steps.get(i + 1).map(|n| n.keys);
        if every || (next != Some(s.keys) && s.keys != last) {
            last = s.keys;
            println!(
                "--- before key {} ({:?}), frame {i}, cursor {:?}",
                s.keys,
                p.keys.get(s.keys),
                s.shown.cursor
            );
            for (r, row) in s.shown.rows.iter().enumerate() {
                if !row.is_empty() {
                    println!("{r:2}|{row}");
                }
            }
        }
    }
}
