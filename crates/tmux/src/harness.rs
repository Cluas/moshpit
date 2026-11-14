//! The G2 equivalence harness's side of the parser: feed plans and the JSON
//! line format, shared by `examples/cc_events.rs` and the capture tests.
//! `harness/swift/main.swift` implements the same plans and prints the same
//! bytes for the Swift parser; see `harness/equivalence.py`.

use std::ops::Range;

use crate::control::{ControlError, ControlParser, Event};

/// SplitMix64, the plan's only source of randomness (same constants as the
/// Swift side).
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

    fn below(&mut self, bound: u64) -> usize {
        usize::try_from(self.next_u64() % bound).expect("bound fits usize")
    }
}

/// One step of a plan: feed a byte range of the capture, or reset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Feed(Range<usize>),
    Reset,
}

/// The steps of a plan over `data`:
///
/// - `whole`: one feed
/// - `bytes`: one byte per feed
/// - `fixed:N`: N bytes per feed
/// - `rand:SEED:MAX`: feed sizes uniform in `1..=MAX`
/// - `inside:SEED`: cuts only inside UTF-8 sequences and octal escapes, each
///   candidate taken with probability 1/3
/// - `reset:SEED:MAX`: like `rand`, plus a reset after ~1 in 64 feeds
pub fn plan(spec: &str, data: &[u8]) -> Result<Vec<Step>, String> {
    let fields: Vec<&str> = spec.split(':').collect();
    let number = |index: usize| -> Result<u64, String> {
        fields
            .get(index)
            .and_then(|field| field.parse().ok())
            .ok_or_else(|| format!("bad plan {spec:?}"))
    };
    let count = data.len();
    let sized = |next: &mut dyn FnMut() -> usize| {
        let mut steps = Vec::new();
        let mut at = 0;
        while at < count {
            let end = count.min(at + next());
            steps.push(Step::Feed(at..end));
            at = end;
        }
        steps
    };
    let steps = match fields[0] {
        "whole" => {
            if count > 0 {
                vec![Step::Feed(0..count)]
            } else {
                Vec::new()
            }
        }
        "bytes" => sized(&mut || 1),
        "fixed" => {
            let size = usize::try_from(number(1)?).map_err(|e| e.to_string())?;
            if size == 0 {
                return Err(format!("bad plan {spec:?}"));
            }
            sized(&mut || size)
        }
        "rand" => {
            let mut rng = SplitMix64(number(1)?);
            let max = number(2)?.max(1);
            sized(&mut || 1 + rng.below(max))
        }
        "reset" => {
            let mut rng = SplitMix64(number(1)?);
            let max = number(2)?.max(1);
            let mut steps = Vec::new();
            let mut at = 0;
            while at < count {
                let end = count.min(at + 1 + rng.below(max));
                steps.push(Step::Feed(at..end));
                at = end;
                if rng.next_u64().is_multiple_of(64) {
                    steps.push(Step::Reset);
                }
            }
            steps
        }
        "inside" => {
            let mut rng = SplitMix64(number(1)?);
            let is_octal = |byte: u8| (b'0'..=b'7').contains(&byte);
            let mut steps = Vec::new();
            let mut start = 0;
            for cut in 1..count {
                let inside_utf8 = (0x80..=0xBF).contains(&data[cut]);
                let after_backslash = data[cut - 1] == b'\\';
                let after_one_digit = cut >= 2 && data[cut - 2] == b'\\' && is_octal(data[cut - 1]);
                let after_two_digits = cut >= 3
                    && data[cut - 3] == b'\\'
                    && is_octal(data[cut - 2])
                    && is_octal(data[cut - 1]);
                if (inside_utf8 || after_backslash || after_one_digit || after_two_digits)
                    && rng.next_u64().is_multiple_of(3)
                {
                    steps.push(Step::Feed(start..cut));
                    start = cut;
                }
            }
            if start < count {
                steps.push(Step::Feed(start..count));
            }
            steps
        }
        _ => return Err(format!("unknown plan {spec:?}")),
    };
    Ok(steps)
}

/// FNV-1a over every step: a feed hashes its length (u64 little endian), a
/// reset hashes `u64::MAX`.
pub fn plan_hash(steps: &[Step]) -> u64 {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    for step in steps {
        let word = match step {
            Step::Feed(range) => range.len() as u64,
            Step::Reset => u64::MAX,
        };
        for byte in word.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// Where the feeds end: `(cuts, inside UTF-8, inside an escape)`. A cut is
/// inside a UTF-8 sequence when a continuation byte follows it, and inside a
/// backslash escape when it follows the backslash or one or two of its
/// octal digits.
pub fn cut_stats(steps: &[Step], data: &[u8]) -> (usize, usize, usize) {
    let is_octal = |byte: u8| (b'0'..=b'7').contains(&byte);
    let (mut cuts, mut utf8, mut octal) = (0, 0, 0);
    for step in steps {
        let Step::Feed(range) = step else { continue };
        let cut = range.end;
        if cut >= data.len() {
            continue;
        }
        cuts += 1;
        if (0x80..=0xBF).contains(&data[cut]) {
            utf8 += 1;
        }
        if data[cut - 1] == b'\\'
            || (cut >= 2 && data[cut - 2] == b'\\' && is_octal(data[cut - 1]))
            || (cut >= 3
                && data[cut - 3] == b'\\'
                && is_octal(data[cut - 2])
                && is_octal(data[cut - 1]))
        {
            octal += 1;
        }
    }
    (cuts, utf8, octal)
}

/// Run `data` through a fresh parser along a plan: the JSON lines, ending
/// with the `end` record, exactly as `cc_events events` prints them.
pub fn run(spec: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let steps = plan(spec, data)?;
    let mut parser = ControlParser::new();
    let mut out = Vec::new();
    let (mut feeds, mut resets) = (0usize, 0usize);
    for step in &steps {
        match step {
            Step::Feed(range) => {
                parser.feed(&data[range.clone()], |event| write_event(&mut out, &event));
                feeds += 1;
            }
            Step::Reset => {
                parser.reset();
                resets += 1;
            }
        }
    }
    out.extend_from_slice(br#"{"ev":"end","plan":"#);
    write_string(&mut out, spec);
    let (cuts, in_utf8, in_octal) = cut_stats(&steps, data);
    out.extend_from_slice(
        format!(
            r#","feeds":{feeds},"resets":{resets},"bytes":{},"cuts":{cuts},"in_utf8":{in_utf8},"in_octal":{in_octal},"plan_hash":"{:016x}"}}"#,
            data.len(),
            plan_hash(&steps)
        )
        .as_bytes(),
    );
    out.push(b'\n');
    Ok(out)
}

/// One event as one JSON line (with its newline). Key order and escaping are
/// fixed: the Swift side writes the very same bytes.
pub fn write_event(out: &mut Vec<u8>, event: &Event) {
    let field = |out: &mut Vec<u8>, name: &str, value: &str| {
        out.extend_from_slice(b",\"");
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\":");
        write_string(out, value);
    };
    let kind = |out: &mut Vec<u8>, name: &str| {
        out.extend_from_slice(br#"{"ev":""#);
        out.extend_from_slice(name.as_bytes());
        out.push(b'"');
    };
    match event {
        Event::PaneOutput { pane_id, data } => {
            kind(out, "output");
            field(out, "pane", pane_id);
            out.extend_from_slice(br#","hex":""#);
            for byte in data {
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0F));
            }
            out.push(b'"');
        }
        Event::LayoutChange {
            window_id,
            layout,
            zoomed,
        } => {
            kind(out, "layout-change");
            field(out, "window", window_id);
            field(out, "layout", layout);
            out.extend_from_slice(if *zoomed {
                br#","zoomed":true"#.as_slice()
            } else {
                br#","zoomed":false"#.as_slice()
            });
        }
        Event::WindowAdd { window_id } => {
            kind(out, "window-add");
            field(out, "window", window_id);
        }
        Event::WindowClose { window_id } => {
            kind(out, "window-close");
            field(out, "window", window_id);
        }
        Event::WindowRenamed { window_id, name } => {
            kind(out, "window-renamed");
            field(out, "window", window_id);
            field(out, "name", name);
        }
        Event::SessionChanged { session_id, name } => {
            kind(out, "session-changed");
            field(out, "session", session_id);
            field(out, "name", name);
        }
        Event::SessionRenamed { session_id, name } => {
            kind(out, "session-renamed");
            field(out, "session", session_id);
            field(out, "name", name);
        }
        Event::SessionWindowChanged {
            session_id,
            window_id,
        } => {
            kind(out, "session-window-changed");
            field(out, "session", session_id);
            field(out, "window", window_id);
        }
        Event::SessionsChanged => kind(out, "sessions-changed"),
        Event::UnlinkedWindowAdd { window_id } => {
            kind(out, "unlinked-window-add");
            field(out, "window", window_id);
        }
        Event::UnlinkedWindowClose { window_id } => {
            kind(out, "unlinked-window-close");
            field(out, "window", window_id);
        }
        Event::UnlinkedWindowRenamed { window_id, name } => {
            kind(out, "unlinked-window-renamed");
            field(out, "window", window_id);
            field(out, "name", name);
        }
        Event::ClientSessionChanged {
            client,
            session_id,
            name,
        } => {
            kind(out, "client-session-changed");
            field(out, "client", client);
            field(out, "session", session_id);
            field(out, "name", name);
        }
        Event::ActivePaneChanged { window_id, pane_id } => {
            kind(out, "window-pane-changed");
            field(out, "window", window_id);
            field(out, "pane", pane_id);
        }
        Event::Pause { pane_id } => {
            kind(out, "pause");
            field(out, "pane", pane_id);
        }
        Event::Continue { pane_id } => {
            kind(out, "continue");
            field(out, "pane", pane_id);
        }
        Event::ClientDetached { payload } => {
            kind(out, "client-detached");
            field(out, "payload", payload);
        }
        Event::Exit { reason } => {
            kind(out, "exit");
            match reason {
                Some(reason) => field(out, "reason", reason),
                None => out.extend_from_slice(br#","reason":null"#),
            }
        }
        Event::CommandResponse(response) => {
            kind(out, "response");
            out.extend_from_slice(
                format!(
                    r#","id":{},"num":{},"error":{},"lines":["#,
                    response.command_id, response.command_num, response.is_error
                )
                .as_bytes(),
            );
            for (index, line) in response.lines.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(out, line);
            }
            out.push(b']');
        }
        Event::ProtocolError(ControlError::MalformedLine(message)) => {
            kind(out, "protocol-error");
            field(out, "message", message);
        }
    }
    out.extend_from_slice(b"}\n");
}

/// `"`, `\` and C0 controls escaped (controls always as `\u00xx`, lower
/// hex), everything else verbatim UTF-8.
fn write_string(out: &mut Vec<u8>, text: &str) {
    out.push(b'"');
    for &byte in text.as_bytes() {
        match byte {
            b'"' => out.extend_from_slice(br#"\""#),
            b'\\' => out.extend_from_slice(br"\\"),
            0x00..=0x1F => {
                out.extend_from_slice(br"\u00");
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0F));
            }
            _ => out.push(byte),
        }
    }
    out.push(b'"');
}

fn hex_digit(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + nibble - 10
    }
}
