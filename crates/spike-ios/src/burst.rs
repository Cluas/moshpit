//! The bench's burst: an endless, deterministic stream of terminal output
//! (text, colors, cursor moves, line erases, wide and multi-byte
//! characters), cut into ragged chunks, with the hash both sides compare.
//!
//! The stream holds nothing that makes a terminal answer (no device
//! status or attribute queries, no title or color queries, no resizes): the
//! view must only ever draw it.

/// FNV-1a, 64 bits: order-sensitive, trivial to write the same in Swift
/// (`(h ^ b) &* 0x100000001b3`).
pub fn fnv1a(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// FNV-1a's starting value.
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// splitmix64: a seed in, a deterministic sequence out.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n` > 0; the bias is irrelevant here).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A chunk size from 1 B to 64 KB, log-uniform: as many chunks under
    /// 100 B as between 1 and 64 KB, as a PTY's reads come.
    pub fn chunk_size(&mut self) -> usize {
        let bits = self.below(17) as u32; // 2^0 ..= 2^16
        let low = 1usize << bits.saturating_sub(1);
        let high = 1usize << bits;
        (low + self.below((high - low + 1) as u64) as usize).clamp(1, 1 << 16)
    }
}

const WORDS: &[&str] = &[
    "the",
    "agent",
    "wrote",
    "src/main.rs",
    "cargo",
    "build",
    "--release",
    "ok",
    "error[E0382]:",
    "borrow",
    "of",
    "moved",
    "value:",
    "`stream`",
    "warning:",
    "unused",
    "Compiling",
    "tokio",
    "v1.54.1",
    "Finished",
    "test",
    "result:",
    "passed;",
    "0",
    "failed;",
    "Running",
    "diff",
    "--git",
    "a/b",
    "@@",
    "-1,3",
    "+1,4",
    "->",
    "=>",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    "|",
    "λ",
    "naïve",
    "café",
    "Grüße",
    "→",
    "✓",
    "✗",
    "…",
    "中文",
    "终端",
    "智能体",
    "日本語",
    "한국어",
    "🙂",
    "🚀",
    "┌──┐",
    "│",
    "└──┘",
    "█▓▒░",
];

/// An endless stream of terminal output, the same for the same seed.
pub struct Generator {
    rng: Rng,
    pending: Vec<u8>,
    at: usize,
    column: usize,
}

impl Generator {
    pub fn new(seed: u64) -> Generator {
        Generator {
            rng: Rng::new(seed ^ 0x6d6f_7368_7069_7421),
            pending: Vec::with_capacity(256),
            at: 0,
            column: 0,
        }
    }

    /// Fills `out` with the next bytes of the stream.
    pub fn fill(&mut self, out: &mut [u8]) {
        let mut filled = 0;
        while filled < out.len() {
            if self.at == self.pending.len() {
                self.pending.clear();
                self.at = 0;
                self.segment();
            }
            let n = (self.pending.len() - self.at).min(out.len() - filled);
            out[filled..filled + n].copy_from_slice(&self.pending[self.at..self.at + n]);
            filled += n;
            self.at += n;
        }
    }

    /// Appends one segment to `pending`.
    fn segment(&mut self) {
        let roll = self.rng.below(100);
        let p = &mut self.pending;
        match roll {
            // Words, most of the stream.
            0..=69 => {
                let word = WORDS[self.rng.below(WORDS.len() as u64) as usize];
                p.extend_from_slice(word.as_bytes());
                p.push(b' ');
                self.column += word.chars().count() + 1;
            }
            // Colors: basic, bright, 256 and true color, attributes, resets.
            70..=81 => {
                let sgr = match self.rng.below(6) {
                    0 => format!("\x1b[{}m", 30 + self.rng.below(8)),
                    1 => format!("\x1b[{}m", 90 + self.rng.below(8)),
                    2 => format!("\x1b[38;5;{}m", self.rng.below(256)),
                    3 => format!(
                        "\x1b[38;2;{};{};{}m",
                        self.rng.below(256),
                        self.rng.below(256),
                        self.rng.below(256)
                    ),
                    4 => format!("\x1b[{}m", [1, 2, 3, 4, 7, 9][self.rng.below(6) as usize]),
                    _ => "\x1b[0m".to_owned(),
                };
                p.extend_from_slice(sgr.as_bytes());
            }
            // A new line.
            82..=91 => {
                p.extend_from_slice(b"\r\n");
                self.column = 0;
            }
            // A tab, or a carriage return over the line.
            92..=94 => {
                p.push(if self.rng.below(2) == 0 { b'\t' } else { b'\r' });
            }
            // Erase to the end of the line, or all of it.
            95..=96 => {
                p.extend_from_slice(if self.rng.below(2) == 0 {
                    b"\x1b[K"
                } else {
                    b"\x1b[2K"
                });
            }
            // Move the cursor somewhere on a 24x80 screen, or home.
            97..=98 => {
                let cup = format!(
                    "\x1b[{};{}H",
                    1 + self.rng.below(24),
                    1 + self.rng.below(80)
                );
                p.extend_from_slice(cup.as_bytes());
            }
            // Clear the screen, rarely.
            _ => {
                if self.rng.below(20) == 0 {
                    p.extend_from_slice(b"\x1b[2J\x1b[H");
                } else {
                    p.extend_from_slice(b"\x1b[0m\r\n");
                }
                self.column = 0;
            }
        }
        if self.column > 72 {
            self.pending.extend_from_slice(b"\r\n");
            self.column = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_makes_the_same_stream_whatever_the_cut() {
        let mut whole = vec![0u8; 1 << 20];
        Generator::new(42).fill(&mut whole);
        let mut cut = Vec::new();
        let mut generator = Generator::new(42);
        let mut sizes = Rng::new(9);
        while cut.len() < whole.len() {
            let n = sizes.chunk_size().min(whole.len() - cut.len());
            let mut chunk = vec![0u8; n];
            generator.fill(&mut chunk);
            cut.extend_from_slice(&chunk);
        }
        assert_eq!(fnv1a(FNV_OFFSET, &cut), fnv1a(FNV_OFFSET, &whole));
        assert_eq!(cut, whole);
    }

    #[test]
    fn the_stream_never_asks_the_terminal_anything() {
        let mut bytes = vec![0u8; 4 << 20];
        Generator::new(7).fill(&mut bytes);
        // Every escape is a CSI with numeric parameters ending in SGR (m),
        // a cursor move (H) or an erase (K, J): nothing a terminal answers,
        // nothing that resizes it. The only other controls are CR, LF, TAB.
        let mut i = 0;
        let mut kinds = [0usize; 4];
        while i < bytes.len() - 32 {
            match bytes[i] {
                0x1b => {
                    assert_eq!(bytes[i + 1], b'[', "a non-CSI escape at {i}");
                    let mut j = i + 2;
                    while bytes[j].is_ascii_digit() || bytes[j] == b';' {
                        j += 1;
                    }
                    let k = b"mHKJ".iter().position(|&f| f == bytes[j]);
                    let k =
                        k.unwrap_or_else(|| panic!("CSI ending in {:?} at {i}", bytes[j] as char));
                    kinds[k] += 1;
                    i = j + 1;
                }
                b'\r' | b'\n' | b'\t' => i += 1,
                b => {
                    assert!(b >= 0x20 && b != 0x7f, "control byte {b:#04x} at {i}");
                    i += 1;
                }
            }
        }
        assert!(kinds.iter().all(|&n| n > 0), "{kinds:?}");
        // Text in it: true color, wide characters, emoji; UTF-8 throughout
        // (a chunk may cut a character, the stream never does).
        let end = bytes.iter().rposition(|&b| b == b'\n').unwrap();
        let text = std::str::from_utf8(&bytes[..=end]).expect("UTF-8");
        assert!(text.contains("\x1b[38;2;"));
        assert!(text.contains("中文") && text.contains("🚀"));
    }

    #[test]
    fn chunk_sizes_cover_one_byte_to_64_kb() {
        let mut rng = Rng::new(1);
        let sizes: Vec<_> = (0..100_000).map(|_| rng.chunk_size()).collect();
        assert_eq!(*sizes.iter().min().unwrap(), 1);
        let max = *sizes.iter().max().unwrap();
        assert!((60_000..=1 << 16).contains(&max), "largest {max}");
        let small = sizes.iter().filter(|&&s| s < 100).count();
        assert!(small > 30_000, "{small} chunks under 100 B");
    }

    #[test]
    fn fnv1a_matches_the_reference_values() {
        assert_eq!(fnv1a(FNV_OFFSET, b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(FNV_OFFSET, b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(FNV_OFFSET, b"foobar"), 0x8594_4171_f739_67e8);
    }
}
