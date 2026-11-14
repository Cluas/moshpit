//! Helpers the tests share: a terminal's text, the repository's recorded
//! byte streams.
#![allow(dead_code)]

use moshpit_term::{Options, Terminal};
use std::path::PathBuf;

pub fn term(cols: u16, rows: u16, scrollback: usize) -> Terminal {
    Terminal::new(
        cols,
        rows,
        Options {
            scrollback,
            ..Options::default()
        },
    )
}

/// Screen rows' text (trailing blanks trimmed) and wrap marks.
pub fn screen(t: &Terminal) -> Vec<(String, bool)> {
    let (_, rows) = t.size();
    (0..rows as i32).map(|r| t.row_text(r)).collect()
}

pub fn rows(t: &Terminal) -> Vec<String> {
    screen(t).into_iter().map(|r| r.0).collect()
}

/// History rows, oldest first.
pub fn history(t: &Terminal) -> Vec<(String, bool)> {
    let n = t.history_len() as i32;
    (-n..0).map(|r| t.row_text(r)).collect()
}

/// Rows' text and wrap marks.
pub type Rows = Vec<(String, bool)>;

/// Everything a reader can tell apart: history, screen, cursor, title.
pub fn state(t: &Terminal) -> (Rows, Rows, (u16, u16), String) {
    (history(t), screen(t), t.cursor(), t.title().to_string())
}

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// tmux's `-CC` escaping: `\ooo` for bytes below space and the backslash.
pub fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\'
            && s.len() >= i + 4
            && s[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((s[i + 1] - b'0') * 64 + (s[i + 2] - b'0') * 8 + (s[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Pane `%id`'s output in a recorded `-CC` capture, in its recorded reads
/// (the captures used here have one pane and no resize).
pub fn tmux_pane(capture: &str, pane: u32) -> Vec<Vec<u8>> {
    let raw = std::fs::read(
        repo()
            .join("crates/tmux/tests/fixtures/captures")
            .join(format!("{capture}.bin")),
    )
    .unwrap();
    let output = format!("%output %{pane} ");
    // `%extended-output %id age … : data` while the client is paused.
    let extended = format!("%extended-output %{pane} ");
    raw.split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .filter_map(|l| {
            if let Some(data) = l.strip_prefix(output.as_bytes()) {
                return Some(unescape(data));
            }
            let rest = l.strip_prefix(extended.as_bytes())?;
            let at = rest.windows(3).position(|w| w == b" : ")?;
            Some(unescape(&rest[at + 3..]))
        })
        .collect()
}

/// herdr's recorded frames: (cols, rows, bytes).
pub fn herdr_frames() -> Vec<(u16, u16, Vec<u8>)> {
    use base64::Engine as _;
    let text =
        std::fs::read_to_string(repo().join("crates/herdr/tests/fixtures/recorded/frames.jsonl"))
            .unwrap();
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["type"] == "terminal.frame")
        .map(|v| {
            (
                v["width"].as_u64().unwrap() as u16,
                v["height"].as_u64().unwrap() as u16,
                base64::engine::general_purpose::STANDARD
                    .decode(v["bytes"].as_str().unwrap())
                    .unwrap(),
            )
        })
        .collect()
}
