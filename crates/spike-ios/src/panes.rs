//! Spike G5's panes: what the terminal screen shows, made by the core so
//! the screen has real things to keep across keyboards and rebuilds.
//!
//! - [`shell`]: a long history (thousands of numbered lines, some wider
//!   than a phone's grid, so they wrap and reflow), then a prompt that
//!   echoes keys like a line discipline and redraws itself on a resize, as
//!   zsh does on SIGWINCH.
//! - [`tui`]: a full-screen app on the alternate screen, Claude Code's
//!   shape: the conversation at the top, blank filler, an input box and a
//!   footer pinned to the bottom rows. Every resize repaints the whole
//!   screen a beat later (the debounce a TUI and tmux add), which is what
//!   makes a keyboard transition flicker when the view shows it.
//!
//! Nothing here touches a platform: the tasks take the stream's input and
//! push its output, and the tests drive them on the host.

use std::{sync::Arc, time::Duration};

use moshpit_stream::{Input, InputEvent, Stream};

/// The prompt the shell prints.
pub const PROMPT: &[u8] = b"\x1b[38;5;111mg5\x1b[0m \x1b[38;5;150m~/moshpit\x1b[0m $ ";

/// How long the TUI waits for more resizes before it repaints.
pub const REPAINT_DEBOUNCE: Duration = Duration::from_millis(40);

const WORDS: &[&str] = &[
    "agent",
    "cargo",
    "build",
    "--release",
    "ok",
    "warning:",
    "TerminalScreen.swift",
    "keyboard",
    "frame",
    "lock",
    "snapshot",
    "cover",
    "shortcut",
    "bar",
    "rides",
    "the",
    "safe",
    "area",
    "inset",
    "src/ui/screen.rs",
    "NSH",
    "native",
    "component",
    "moshpit/terminal",
    "→",
    "✓",
    "中文",
    "宽字符",
];

/// Line `n` of the shell's history: its number, then words chosen by `n`,
/// from about ten to about 140 columns (a phone shows 50 to 60).
pub fn history_line(n: usize) -> String {
    let mut seed = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5DEE_CE66;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let count = 1 + (next() % 22) as usize;
    let mut line = format!("{n:05} ");
    for i in 0..count {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(WORDS[(next() % WORDS.len() as u64) as usize]);
    }
    line
}

/// The shell's history and first prompt, as the bytes it pushes.
pub fn history(lines: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines * 64);
    for n in 1..=lines {
        out.extend_from_slice(history_line(n).as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(PROMPT);
    out
}

/// The shell's line discipline: what `bytes` typed at a line holding
/// `line` echo, the line updated. Return runs nothing but says what it
/// ran; ^C abandons the line; DEL erases a character; escape sequences
/// (arrows) go back as they came, moving the cursor.
pub fn shell_input(line: &mut String, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                out.extend_from_slice(b"\r\n");
                if !line.trim().is_empty() {
                    out.extend_from_slice(format!("ran: {}\r\n", line.trim()).as_bytes());
                }
                line.clear();
                out.extend_from_slice(PROMPT);
            }
            '\u{3}' => {
                out.extend_from_slice(b"^C\r\n");
                line.clear();
                out.extend_from_slice(PROMPT);
            }
            '\u{7f}' | '\u{8}' => {
                if line.pop().is_some() {
                    out.extend_from_slice(b"\x08 \x08");
                }
            }
            '\u{1b}' => {
                // The sequence whole: ESC, then up to its final byte.
                out.push(0x1b);
                while let Some(&next) = chars.peek() {
                    chars.next();
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(next.encode_utf8(&mut buf).as_bytes());
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            }
            '\t' => {
                line.push_str("  ");
                out.extend_from_slice(b"  ");
            }
            c if c >= ' ' => {
                line.push(c);
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            _ => {}
        }
    }
    out
}

/// The shell: `lines` of history, a prompt, then echo. A resize redraws the
/// prompt line (zsh's SIGWINCH), nothing else.
pub async fn shell(stream: Arc<Stream>, mut input: Input, lines: usize) {
    stream.push(&history(lines));
    let mut line = String::new();
    while let Some(event) = input.next().await {
        crate::probe::record(stream.id(), &event);
        match event {
            InputEvent::Bytes(bytes) => stream.push(&shell_input(&mut line, &bytes)),
            InputEvent::Resize { .. } => {
                let mut out = b"\r\x1b[K".to_vec();
                out.extend_from_slice(PROMPT);
                out.extend_from_slice(line.as_bytes());
                stream.push(&out);
            }
            // The spike's views never paste through the stream.
            InputEvent::Paste(_) => {}
        }
    }
}

/// What the TUI's conversation says, oldest first.
const CONVERSATION: &[&str] = &[
    "> make the shortcut bar ride the keyboard",
    "",
    "⏺ Reading Moshpit/UI/Terminal/TerminalScreen.swift (3509 lines)",
    "⏺ The bar sits in .safeAreaInset(edge: .bottom), so it follows the",
    "  keyboard without being an inputAccessoryView.",
    "",
    "⏺ Update(crates/spike-ios/src/screen.rs)",
    "  ⎿  Added 42 lines, removed 3 lines",
    "",
    "⏺ Done. The terminal keeps its frame while the keyboard moves and",
    "  resizes once when it lands.",
];

/// Cuts `text` to `cols` columns (a wide character counts two).
fn fit(text: &str, cols: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let w = if c.len_utf8() >= 3 && !matches!(c, '⏺' | '⎿' | '─' | '⏵' | '✻' | '→' | '✓')
        {
            2
        } else {
            1
        };
        if used + w > cols {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

/// The TUI's whole screen at `cols` × `rows`, with `draft` in its input box
/// and `sent` (what was submitted) after the conversation.
pub fn tui_frame(cols: u16, rows: u16, draft: &str, sent: &[String]) -> Vec<u8> {
    let (cols, rows) = (usize::from(cols.max(10)), usize::from(rows.max(6)));
    let mut out = String::new();
    // The alternate screen, cursor hidden while drawing, home, clear.
    out.push_str("\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J");
    let header = fit(&format!(" ✻ g5 agent · {cols}×{rows}"), cols);
    out.push_str(&format!("\x1b[1;1H\x1b[7m{header:<cols$}\x1b[0m"));
    // The conversation from row 3, as far as the input box allows.
    let body: Vec<String> = CONVERSATION
        .iter()
        .map(|s| (*s).to_owned())
        .chain(
            sent.iter()
                .flat_map(|s| [String::new(), format!("> {s}"), "⏺ ok".to_owned()]),
        )
        .collect();
    let room = rows.saturating_sub(6);
    let start = body.len().saturating_sub(room);
    for (i, line) in body[start..].iter().enumerate() {
        out.push_str(&format!("\x1b[{};1H{}", i + 3, fit(line, cols)));
    }
    let rule = "─".repeat(cols);
    out.push_str(&format!("\x1b[{};1H\x1b[38;5;240m{rule}\x1b[0m", rows - 3));
    out.push_str(&format!("\x1b[{};1H> {}", rows - 2, fit(draft, cols - 2)));
    out.push_str(&format!("\x1b[{};1H\x1b[38;5;240m{rule}\x1b[0m", rows - 1));
    let footer = fit("  ⏵⏵ bypass permissions on (shift+tab to cycle)", cols);
    out.push_str(&format!("\x1b[{rows};1H\x1b[38;5;180m{footer}\x1b[0m"));
    let caret = 3 + draft.chars().count().min(cols - 3);
    out.push_str(&format!("\x1b[{};{caret}H\x1b[?25h", rows - 2));
    out.into_bytes()
}

/// The TUI: nothing until the view says its size, then the whole screen on
/// every size (repainted [`REPAINT_DEBOUNCE`] after the last of a run of
/// resizes), and its input line on every key.
pub async fn tui(stream: Arc<Stream>, mut input: Input) {
    let mut size: Option<(u16, u16)> = None;
    let mut draft = String::new();
    let mut sent: Vec<String> = Vec::new();
    // A resize waiting for its repaint.
    let mut pending = false;
    loop {
        let event = if pending {
            match tokio::time::timeout(REPAINT_DEBOUNCE, input.next()).await {
                Ok(event) => event,
                Err(_) => {
                    pending = false;
                    if let Some((cols, rows)) = size {
                        stream.push(&tui_frame(cols, rows, &draft, &sent));
                    }
                    continue;
                }
            }
        } else {
            input.next().await
        };
        let Some(event) = event else { return };
        crate::probe::record(stream.id(), &event);
        match event {
            InputEvent::Resize { cols, rows } => {
                size = Some((cols, rows));
                pending = true;
            }
            // The spike's views never paste through the stream.
            InputEvent::Paste(_) => {}
            InputEvent::Bytes(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let mut submitted = false;
                for c in text.chars() {
                    match c {
                        '\r' | '\n' => {
                            if !draft.trim().is_empty() {
                                sent.push(std::mem::take(&mut draft));
                                submitted = true;
                            }
                        }
                        '\u{7f}' | '\u{8}' => {
                            draft.pop();
                        }
                        c if c >= ' ' => draft.push(c),
                        _ => {}
                    }
                }
                let Some((cols, rows)) = size else { continue };
                if submitted {
                    stream.push(&tui_frame(cols, rows, &draft, &sent));
                } else {
                    let line = fit(&draft, usize::from(cols.max(10)) - 2);
                    let row = rows.max(6) - 2;
                    stream.push(
                        format!(
                            "\x1b[{row};1H\x1b[2K> {line}\x1b[{row};{}H",
                            3 + line.chars().count()
                        )
                        .as_bytes(),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshpit_stream::view;

    fn drain(id: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = view::read(id, &mut buf);
            out.extend_from_slice(&buf[..n]);
            if n < buf.len() {
                return out;
            }
        }
    }

    #[test]
    fn history_lines_are_numbered_and_some_wrap_a_phone_grid() {
        let lines: Vec<String> = (1..=500).map(history_line).collect();
        assert!(
            lines
                .iter()
                .enumerate()
                .all(|(i, l)| l.starts_with(&format!("{:05} ", i + 1)))
        );
        assert!(
            lines.iter().any(|l| l.chars().count() > 60),
            "some wrap at 60 columns"
        );
        assert!(lines.iter().any(|l| l.chars().count() < 30));
        assert_eq!(history_line(42), history_line(42), "deterministic");
    }

    #[test]
    fn the_line_discipline_echoes_erases_and_answers_return() {
        let mut line = String::new();
        assert_eq!(shell_input(&mut line, b"ls"), b"ls");
        assert_eq!(shell_input(&mut line, b"\x7f"), b"\x08 \x08");
        assert_eq!(line, "l");
        let out = shell_input(&mut line, b"s\r");
        assert!(out.starts_with(b"s\r\nran: ls\r\n"));
        assert!(out.ends_with(PROMPT));
        assert!(line.is_empty());
        assert_eq!(
            shell_input(&mut line, b"\x1b[A"),
            b"\x1b[A",
            "arrows go back whole"
        );
    }

    #[test]
    fn the_tui_frame_pins_its_footer_to_the_last_row() {
        let frame = String::from_utf8(tui_frame(50, 30, "hi", &[])).unwrap();
        assert!(frame.starts_with("\x1b[?1049h"));
        assert!(frame.contains("\x1b[30;1H\x1b[38;5;180m  ⏵⏵ bypass permissions on"));
        assert!(frame.contains("\x1b[28;1H> hi"));
        assert!(frame.contains("50×30"));
    }

    #[test]
    fn the_tui_repaints_once_after_a_run_of_resizes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let (stream, input) = moshpit_stream::open();
        let stream = Arc::new(stream);
        let id = stream.id();
        runtime.block_on(async {
            let task = tokio::spawn(tui(stream.clone(), input));
            for rows in [40, 35, 30] {
                assert!(view::resize(id, 50, rows));
            }
            tokio::time::sleep(REPAINT_DEBOUNCE * 4).await;
            let out = String::from_utf8(drain(id)).unwrap();
            assert_eq!(out.matches("\x1b[?1049h").count(), 1, "one repaint");
            assert!(out.contains("50×30"), "at the last size");
            assert!(view::write(id, b"ok"));
            tokio::time::sleep(Duration::from_millis(20)).await;
            let out = String::from_utf8(drain(id)).unwrap();
            assert!(out.contains("> ok"), "the input line: {out:?}");
            assert!(!out.contains("\x1b[?1049h"), "a key repaints its line only");
            task.abort();
        });
    }
}
