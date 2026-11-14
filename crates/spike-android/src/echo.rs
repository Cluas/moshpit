//! The spike's stand-in for a remote shell: echoes what the view writes, the
//! way a line discipline in cooked mode would, so typing, backspace and
//! arrows can be checked by eye on the screen and byte by byte in the log.
//!
//! - Printable text (UTF-8, wide CJK included) is echoed as it came.
//! - Enter (`\r`) starts a new prompt.
//! - Backspace (DEL or ^H) erases the last echoed item, by its width.
//! - Arrow keys (`ESC [ A` … and their application-mode `ESC O A` …) show as
//!   dim arrows, so their order is visible; other escape sequences show as
//!   `^[` and their text, other control bytes as `^X`.
//!
//! Input may split anywhere (a sequence or a UTF-8 character across two
//! writes): the unfinished tail waits for the next chunk.

/// What the prompt looks like.
pub const PROMPT: &[u8] = b"\x1b[36m$\x1b[0m ";

#[derive(Debug, Default)]
pub struct LineEcho {
    /// Display widths of what is echoed on the current line, last last.
    cells: Vec<u8>,
    /// An unfinished escape sequence or UTF-8 character.
    pending: Vec<u8>,
}

impl LineEcho {
    pub fn new() -> LineEcho {
        LineEcho::default()
    }

    /// The bytes to echo for `input`.
    pub fn feed(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len() + 8);
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.extend_from_slice(input);
        let mut i = 0;
        while i < bytes.len() {
            match parse(&bytes[i..]) {
                Token::Incomplete => {
                    self.pending = bytes[i..].to_vec();
                    break;
                }
                Token::Char(len, width) => {
                    out.extend_from_slice(&bytes[i..i + len]);
                    self.cells.push(width);
                    i += len;
                }
                Token::Enter => {
                    out.extend_from_slice(b"\r\n");
                    out.extend_from_slice(PROMPT);
                    self.cells.clear();
                    i += 1;
                }
                Token::Erase => {
                    if let Some(width) = self.cells.pop() {
                        out.extend(std::iter::repeat_n(b'\x08', width as usize));
                        out.extend_from_slice(b"\x1b[K");
                    } else {
                        out.push(b'\x07');
                    }
                    i += 1;
                }
                Token::Arrow(len, arrow) => {
                    out.extend_from_slice(b"\x1b[2m");
                    out.extend_from_slice(arrow.as_bytes());
                    out.extend_from_slice(b"\x1b[22m");
                    self.cells.push(1);
                    i += len;
                }
                Token::Other(len) => {
                    for &b in &bytes[i..i + len] {
                        self.caret(&mut out, b);
                    }
                    i += len;
                }
            }
        }
        out
    }

    /// `b` as `^X` (or itself, inside a sequence's printable part).
    fn caret(&mut self, out: &mut Vec<u8>, b: u8) {
        match b {
            0x00..=0x1f => {
                out.extend_from_slice(&[b'^', b + 0x40]);
                self.cells.push(2);
            }
            0x7f => {
                out.extend_from_slice(b"^?");
                self.cells.push(2);
            }
            _ => {
                out.push(b);
                self.cells.push(1);
            }
        }
    }
}

enum Token {
    /// `len` bytes of one character `width` columns wide.
    Char(usize, u8),
    Enter,
    Erase,
    /// An arrow key's sequence (`len` bytes) and what to show for it.
    Arrow(usize, &'static str),
    /// Anything else, `len` bytes, shown in caret notation.
    Other(usize),
    /// Needs more bytes.
    Incomplete,
}

fn parse(b: &[u8]) -> Token {
    match b[0] {
        b'\r' => Token::Enter,
        0x7f | 0x08 => Token::Erase,
        0x1b => escape(b),
        0x00..=0x1f => Token::Other(1),
        0x20..=0x7e => Token::Char(1, 1),
        lead => {
            let len = match lead {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Token::Other(1),
            };
            if b.len() < len {
                return Token::Incomplete;
            }
            match std::str::from_utf8(&b[..len]) {
                Ok(s) => Token::Char(len, width(s.chars().next().unwrap_or(' '))),
                Err(_) => Token::Other(1),
            }
        }
    }
}

/// An escape sequence at the start of `b`: CSI (`ESC [` … final byte) or
/// SS3 (`ESC O` and one byte).
fn escape(b: &[u8]) -> Token {
    let Some(&kind) = b.get(1) else {
        return Token::Incomplete;
    };
    match kind {
        b'[' => {
            let Some(end) = b[2..].iter().position(|c| (0x40..=0x7e).contains(c)) else {
                return Token::Incomplete;
            };
            let len = 2 + end + 1;
            match (&b[2..len - 1], b[len - 1]) {
                (b"", final_byte) => {
                    arrow(final_byte).map_or(Token::Other(len), |a| Token::Arrow(len, a))
                }
                _ => Token::Other(len),
            }
        }
        b'O' => {
            let Some(&final_byte) = b.get(2) else {
                return Token::Incomplete;
            };
            arrow(final_byte).map_or(Token::Other(3), |a| Token::Arrow(3, a))
        }
        _ => Token::Other(1),
    }
}

fn arrow(final_byte: u8) -> Option<&'static str> {
    Some(match final_byte {
        b'A' => "↑",
        b'B' => "↓",
        b'C' => "→",
        b'D' => "←",
        _ => return None,
    })
}

/// Columns `c` takes in a terminal: 2 for the East Asian wide ranges, 1
/// otherwise (good enough for an echo).
fn width(c: char) -> u8 {
    match c as u32 {
        0x1100..=0x115f
        | 0x2e80..=0x303e
        | 0x3041..=0x33ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe30..=0xfe4f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6
        | 0x1f300..=0x1f64f
        | 0x1f900..=0x1f9ff
        | 0x20000..=0x3fffd => 2,
        _ => 1,
    }
}

/// A readable form of input bytes for the log: printable text as is, keys
/// by name, the rest in hex.
pub fn describe(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let (len, text): (usize, String) = match parse(&bytes[i..]) {
            Token::Char(len, _) => (
                len,
                String::from_utf8_lossy(&bytes[i..i + len]).into_owned(),
            ),
            Token::Enter => (1, "<Enter>".into()),
            Token::Erase if bytes[i] == 0x7f => (1, "<DEL>".into()),
            Token::Erase => (1, "<BS>".into()),
            Token::Arrow(len, arrow) => (
                len,
                match arrow {
                    "↑" => "<Up>",
                    "↓" => "<Down>",
                    "→" => "<Right>",
                    _ => "<Left>",
                }
                .into(),
            ),
            Token::Other(len) => (len, hex(&bytes[i..i + len])),
            Token::Incomplete => (bytes.len() - i, format!("<partial {}>", hex(&bytes[i..]))),
        };
        out.push_str(&text);
        i += len;
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(chunks: &[&[u8]]) -> Vec<u8> {
        let mut line = LineEcho::new();
        chunks.iter().flat_map(|c| line.feed(c)).collect()
    }

    #[test]
    fn text_comes_back_as_typed() {
        assert_eq!(echo(&[b"ls -la"]), b"ls -la");
        assert_eq!(echo(&["你好".as_bytes()]), "你好".as_bytes());
    }

    #[test]
    fn enter_starts_a_new_prompt() {
        let mut expected = b"a\r\n".to_vec();
        expected.extend_from_slice(PROMPT);
        assert_eq!(echo(&[b"a\r"]), expected);
    }

    #[test]
    fn backspace_erases_by_width() {
        assert_eq!(echo(&[b"ab\x7f"]), b"ab\x08\x1b[K");
        let mut expected = "你".as_bytes().to_vec();
        expected.extend_from_slice(b"\x08\x08\x1b[K");
        assert_eq!(echo(&["你".as_bytes(), b"\x7f"]), expected);
        // Held backspace past the start of the line rings instead.
        assert_eq!(echo(&[b"a\x7f\x7f"]), b"a\x08\x1b[K\x07");
    }

    #[test]
    fn arrows_show_in_order_and_erase_as_one() {
        let out = echo(&[b"\x1b[A\x1b[B\x1bOC\x1b[D"]);
        let text = String::from_utf8(out).unwrap();
        let shown: String = text.replace("\x1b[2m", "").replace("\x1b[22m", "");
        assert_eq!(shown, "↑↓→←");
        assert!(echo(&[b"\x1b[A\x7f"]).ends_with(b"\x08\x1b[K"));
    }

    #[test]
    fn a_sequence_or_character_split_across_writes_waits() {
        let whole = echo(&["x你\x1b[Ay".as_bytes()]);
        let bytes = "x你\x1b[Ay".as_bytes();
        for cut in 1..bytes.len() {
            assert_eq!(echo(&[&bytes[..cut], &bytes[cut..]]), whole, "cut at {cut}");
        }
    }

    #[test]
    fn describe_names_the_keys() {
        assert_eq!(
            describe(b"ab\r\x7f\x1b[A\x1b[D"),
            "ab<Enter><DEL><Up><Left>"
        );
        assert_eq!(describe("你好".as_bytes()), "你好");
        assert_eq!(describe(b"\x03"), "03");
    }
}
