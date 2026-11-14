//! The burst the spike pushes through the byte channel: text a terminal can
//! take at full speed (numbered lines of letters, digits and some CJK, no
//! control bytes but `\r\n`), cut into chunks of random sizes, hashed as it
//! goes. The view hashes what it drains the same way (`BurstProbe.java`);
//! equal hashes and lengths mean nothing was lost, duplicated or reordered.

/// FNV-1a, 64 bits: what `BurstProbe.java` computes too.
#[derive(Debug, Clone, Copy)]
pub struct Fnv64(pub u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv64 {
    pub fn update(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = h;
    }
}

/// xorshift64*: small, fast, deterministic from a seed.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // Never zero (xorshift's fixed point).
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15 | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// `0..n` (`n` > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// A chunk size from 1 B to 64 KB, spread evenly over the powers of two: as
/// many tiny pushes as huge ones.
pub fn chunk_size(rng: &mut Rng) -> usize {
    let bits = rng.below(17) as u32;
    1 + rng.below(1 << bits) as usize
}

/// The burst's text, an endless byte stream cut wherever [`Text::fill`]
/// stops (inside a line or a UTF-8 character too).
#[derive(Debug)]
pub struct Text {
    rng: Rng,
    line: u64,
    /// The rest of the current line.
    pending: Vec<u8>,
    at: usize,
}

const WORDS: &[&str] = &[
    "tmux",
    "mosh",
    "ssh",
    "agent",
    "claude",
    "pane",
    "burst",
    "终端",
    "智能体",
    "字节",
    "通道",
    "ok",
];

impl Text {
    pub fn new(seed: u64) -> Text {
        Text {
            rng: Rng::new(seed.rotate_left(17)),
            line: 0,
            pending: Vec::new(),
            at: 0,
        }
    }

    /// Appends the next `n` bytes to `out`.
    pub fn fill(&mut self, out: &mut Vec<u8>, mut n: usize) {
        while n > 0 {
            if self.at == self.pending.len() {
                self.next_line();
            }
            let take = n.min(self.pending.len() - self.at);
            out.extend_from_slice(&self.pending[self.at..self.at + take]);
            self.at += take;
            n -= take;
        }
    }

    fn next_line(&mut self) {
        self.line += 1;
        self.pending.clear();
        self.at = 0;
        self.pending
            .extend_from_slice(format!("{:07} ", self.line).as_bytes());
        let words = self.rng.below(12);
        for _ in 0..words {
            let word = WORDS[self.rng.below(WORDS.len() as u64) as usize];
            self.pending.extend_from_slice(word.as_bytes());
            self.pending.push(b' ');
        }
        self.pending.extend_from_slice(b"\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_the_reference() {
        // FNV-1a 64 test vectors.
        let mut h = Fnv64::default();
        h.update(b"");
        assert_eq!(h.0, 0xcbf2_9ce4_8422_2325);
        let mut h = Fnv64::default();
        h.update(b"a");
        assert_eq!(h.0, 0xaf63_dc4c_8601_ec8c);
        let mut h = Fnv64::default();
        h.update(b"foobar");
        assert_eq!(h.0, 0x8594_4171_f739_67e8);
    }

    #[test]
    fn chunks_span_one_byte_to_64_kb() {
        let mut rng = Rng::new(7);
        let sizes: Vec<usize> = (0..20_000).map(|_| chunk_size(&mut rng)).collect();
        assert!(sizes.iter().all(|&s| (1..=65536).contains(&s)));
        assert!(sizes.contains(&1));
        assert!(sizes.iter().any(|&s| s > 32768));
    }

    #[test]
    fn the_text_is_the_same_however_it_is_cut() {
        let mut whole = Vec::new();
        Text::new(3).fill(&mut whole, 100_000);
        let mut cut = Vec::new();
        let mut text = Text::new(3);
        let mut rng = Rng::new(9);
        while cut.len() < whole.len() {
            let n = chunk_size(&mut rng).min(whole.len() - cut.len());
            text.fill(&mut cut, n);
        }
        assert_eq!(cut, whole);
        // Only printable text and line ends.
        let text = String::from_utf8(whole).unwrap();
        assert!(
            text.chars()
                .all(|c| c == '\r' || c == '\n' || !c.is_control())
        );
    }
}
