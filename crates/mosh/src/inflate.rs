//! DEFLATE (RFC 1951) decoding by zlib's rules, for [`crate::compress`].
//!
//! mosh inflates with zlib's `uncompress`, and so does the Swift client
//! (the system zlib), so this decoder takes its rules from zlib 1.2.12's
//! `inflate.c` and `inftrees.c` and refuses exactly what zlib refuses:
//!
//! - a code set that is over-subscribed, or incomplete, except a
//!   literal/length or distance set of a single one-bit code (`inftrees.c`:
//!   `left > 0 && (type == CODES || max != 1)`); a set without any code is
//!   accepted, and using it is an error;
//! - more than 286 literal/length or 30 distance codes, a length repeat
//!   with nothing to repeat or past the end, no end-of-block code;
//! - literal/length symbols 286 and 287 and distance symbols 30 and 31 (in
//!   fixed blocks, where they have codes), a distance past the start of the
//!   output, a stored block whose NLEN is not the complement of LEN, block
//!   type 3, and a stream that ends early.
//!
//! miniz_oxide, which the crate inflated with before, also accepted a set of
//! a single code longer than one bit: it then decoded garbage, and a stream
//! whose garbage matched its Adler-32 was taken where zlib refuses it.
//! (`zlib`'s `INFLATE_STRICT` window check is not part of the default build
//! mosh links, so a distance is only checked against the output.)

/// Longest code DEFLATE allows.
const MAX_BITS: usize = 15;
/// Codes up to this long decode with one table lookup.
const FAST_BITS: u32 = 9;
const FAST_MASK: u64 = (1 << FAST_BITS) - 1;

/// The order the code-length code lengths are sent in.
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
/// Length symbols 257..=285: base length and extra bits.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Distance symbols 0..=29: base distance and extra bits.
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InflateError {
    /// Anything zlib reports as a data error, or a stream that ends early.
    Invalid,
    /// The output would pass the caller's limit.
    TooLarge,
}

/// Inflates the DEFLATE stream at the start of `input` into at most `limit`
/// bytes. Returns the output and how many bytes of `input` the stream took,
/// its last byte included (what follows is the zlib trailer).
pub(crate) fn inflate(input: &[u8], limit: usize) -> Result<(Vec<u8>, usize), InflateError> {
    let mut bits = Bits {
        input,
        pos: 0,
        buf: 0,
        count: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => stored(&mut bits, &mut out, limit)?,
            1 => {
                let (lengths, distances) = fixed_codes();
                codes(&mut bits, &mut out, limit, &lengths, &distances)?;
            }
            2 => {
                let (lengths, distances) = dynamic_codes(&mut bits)?;
                codes(&mut bits, &mut out, limit, &lengths, &distances)?;
            }
            _ => return Err(InflateError::Invalid),
        }
        if last {
            break;
        }
    }
    // The rest of the last byte is padding; whole bytes read ahead are not
    // part of the stream.
    bits.align();
    Ok((out, bits.pos - (bits.count / 8) as usize))
}

/// LSB-first bit reader over the input. Reads ahead whole bytes; `count`
/// bits of `buf` are unread.
struct Bits<'a> {
    input: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
}

impl Bits<'_> {
    fn refill(&mut self) {
        while self.count <= 56 {
            let Some(&byte) = self.input.get(self.pos) else {
                break;
            };
            self.buf |= u64::from(byte) << self.count;
            self.pos += 1;
            self.count += 8;
        }
    }

    /// The next `n` bits (n <= 32), LSB first; the stream ending first is an
    /// error.
    fn take(&mut self, n: u32) -> Result<u32, InflateError> {
        if n == 0 {
            return Ok(0);
        }
        if self.count < n {
            self.refill();
            if self.count < n {
                return Err(InflateError::Invalid);
            }
        }
        let value = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        Ok(value)
    }

    /// Drops the bits left in the current byte.
    fn align(&mut self) {
        let partial = self.count % 8;
        self.buf >>= partial;
        self.count -= partial;
    }

    /// Byte-aligned and with nothing read ahead: `pos` is the next byte.
    fn unread(&mut self) {
        self.align();
        self.pos -= (self.count / 8) as usize;
        self.buf = 0;
        self.count = 0;
    }
}

/// A stored block: LEN, its complement, LEN raw bytes.
fn stored(bits: &mut Bits, out: &mut Vec<u8>, limit: usize) -> Result<(), InflateError> {
    bits.unread();
    let input = bits.input;
    let header = input
        .get(bits.pos..bits.pos + 4)
        .ok_or(InflateError::Invalid)?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return Err(InflateError::Invalid);
    }
    bits.pos += 4;
    // zlib copies what it has before it finds the stream cut short, so
    // passing the limit on the way is reported first.
    let available = usize::from(len).min(input.len() - bits.pos);
    if out.len() + available > limit {
        return Err(InflateError::TooLarge);
    }
    out.extend_from_slice(&input[bits.pos..bits.pos + available]);
    bits.pos += available;
    if available < usize::from(len) {
        return Err(InflateError::Invalid);
    }
    Ok(())
}

/// The codes of a fixed block (RFC 1951 3.2.6). As in zlib, all 32 distance
/// symbols have a code; 30 and 31 are refused when used.
fn fixed_codes() -> (Huffman, Huffman) {
    let mut lengths = [0u8; 288];
    lengths[..144].fill(8);
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    lengths[280..].fill(8);
    let lengths = Huffman::new(&lengths, Kind::LiteralLengths).expect("complete");
    let distances = Huffman::new(&[5; 32], Kind::Distances).expect("complete");
    (lengths, distances)
}

/// The header of a dynamic block: the code-length code, then the
/// literal/length and distance code lengths coded with it.
fn dynamic_codes(bits: &mut Bits) -> Result<(Huffman, Huffman), InflateError> {
    let literals = bits.take(5)? as usize + 257;
    let distances = bits.take(5)? as usize + 1;
    let code_lengths = bits.take(4)? as usize + 4;
    if literals > 286 || distances > 30 {
        return Err(InflateError::Invalid);
    }
    let mut lengths = [0u8; 19];
    for &symbol in &ORDER[..code_lengths] {
        lengths[symbol] = bits.take(3)? as u8;
    }
    let code = Huffman::new(&lengths, Kind::CodeLengths)?;

    let total = literals + distances;
    let mut lengths = [0u8; 286 + 30];
    let mut have = 0;
    while have < total {
        let symbol = code.decode(bits)?;
        let (length, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                if have == 0 {
                    return Err(InflateError::Invalid);
                }
                (lengths[have - 1], 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            _ => (0, 11 + bits.take(7)? as usize),
        };
        if have + repeat > total {
            return Err(InflateError::Invalid);
        }
        lengths[have..have + repeat].fill(length);
        have += repeat;
    }
    if lengths[256] == 0 {
        return Err(InflateError::Invalid);
    }
    Ok((
        Huffman::new(&lengths[..literals], Kind::LiteralLengths)?,
        Huffman::new(&lengths[literals..total], Kind::Distances)?,
    ))
}

/// The symbols of a fixed or dynamic block, up to its end-of-block code.
fn codes(
    bits: &mut Bits,
    out: &mut Vec<u8>,
    limit: usize,
    lengths: &Huffman,
    distances: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = lengths.decode(bits)?;
        if symbol < 256 {
            if out.len() >= limit {
                return Err(InflateError::TooLarge);
            }
            out.push(symbol as u8);
            continue;
        }
        if symbol == 256 {
            return Ok(());
        }
        let index = usize::from(symbol - 257);
        if index >= LENGTH_BASE.len() {
            return Err(InflateError::Invalid);
        }
        let length =
            usize::from(LENGTH_BASE[index]) + bits.take(u32::from(LENGTH_EXTRA[index]))? as usize;
        let index = usize::from(distances.decode(bits)?);
        if index >= DISTANCE_BASE.len() {
            return Err(InflateError::Invalid);
        }
        let distance = usize::from(DISTANCE_BASE[index])
            + bits.take(u32::from(DISTANCE_EXTRA[index]))? as usize;
        if distance > out.len() {
            return Err(InflateError::Invalid);
        }
        if out.len() + length > limit {
            return Err(InflateError::TooLarge);
        }
        let start = out.len() - distance;
        if distance >= length {
            out.extend_from_within(start..start + length);
        } else {
            for at in start..start + length {
                out.push(out[at]);
            }
        }
    }
}

/// What a code set codes for: zlib checks the code-length code alone
/// strictly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    CodeLengths,
    LiteralLengths,
    Distances,
}

/// A canonical Huffman code (RFC 1951 3.2.2).
struct Huffman {
    /// Codes of each length.
    count: [u16; MAX_BITS + 1],
    /// Symbols in code order.
    symbols: [u16; 288],
    /// For the next `FAST_BITS` bits of input: `symbol << 4 | length` of the
    /// code they start with, 0 if that code is longer (or there is none).
    fast: [u16; 1 << FAST_BITS],
}

impl Huffman {
    /// zlib's `inflate_table` checks: no over-subscribed set, and no
    /// incomplete one but a literal/length or distance set of a single
    /// one-bit code. A set without codes is accepted; decoding with it fails.
    fn new(lengths: &[u8], kind: Kind) -> Result<Self, InflateError> {
        let mut count = [0u16; MAX_BITS + 1];
        for &length in lengths {
            count[usize::from(length)] += 1;
        }
        count[0] = 0;
        let max = (1..=MAX_BITS).rev().find(|&length| count[length] != 0);
        if let Some(max) = max {
            let mut left: i32 = 1;
            for &codes in &count[1..] {
                left = (left << 1) - i32::from(codes);
                if left < 0 {
                    return Err(InflateError::Invalid);
                }
            }
            if left > 0 && (kind == Kind::CodeLengths || max != 1) {
                return Err(InflateError::Invalid);
            }
        }
        let mut offsets = [0u16; MAX_BITS + 2];
        for length in 1..=MAX_BITS {
            offsets[length + 1] = offsets[length] + count[length];
        }
        let mut symbols = [0u16; 288];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                let slot = &mut offsets[usize::from(length)];
                symbols[usize::from(*slot)] = symbol as u16;
                *slot += 1;
            }
        }
        let mut fast = [0u16; 1 << FAST_BITS];
        let (mut code, mut index) = (0u32, 0usize);
        for (length, &codes) in count
            .iter()
            .enumerate()
            .take(FAST_BITS as usize + 1)
            .skip(1)
        {
            for _ in 0..codes {
                // Codes are sent from their most significant bit: reversed,
                // they index the bits as read.
                let reversed = code.reverse_bits() >> (32 - length);
                let entry = symbols[index] << 4 | length as u16;
                let mut slot = reversed as usize;
                while slot < fast.len() {
                    fast[slot] = entry;
                    slot += 1 << length;
                }
                code += 1;
                index += 1;
            }
            code <<= 1;
        }
        Ok(Self {
            count,
            symbols,
            fast,
        })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, InflateError> {
        bits.refill();
        let entry = self.fast[(bits.buf & FAST_MASK) as usize];
        if entry != 0 {
            let length = u32::from(entry & 0xF);
            // A shorter code would have matched: the input ends inside one.
            if length > bits.count {
                return Err(InflateError::Invalid);
            }
            bits.buf >>= length;
            bits.count -= length;
            return Ok(entry >> 4);
        }
        // A longer code, or none: one bit at a time (puff.c's decode).
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for length in 1..=MAX_BITS {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.count[length]);
            if code - count < first {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(InflateError::Invalid)
    }
}
