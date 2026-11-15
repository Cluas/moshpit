//! Shared code for the harness (`harness/`, `examples/`) and the golden
//! tests: the seeded generator, hex helpers, server-side encoding, and the
//! two case formats ([`wire`], [`ssp`]). The app's Swift client printed the
//! same formats until it was deleted (E2); its recorded outputs are the
//! frozen goldens `tests/equivalence.rs` reads.

pub mod ssp;
pub mod wire;

/// SplitMix64, the harness's only source of randomness (same constants as the
/// Swift side and the tmux harness).
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

    /// Uniform in `0..bound` (`bound > 0`).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u64() as u8).collect()
    }

    /// Random bytes, `0..bound` of them.
    pub fn bytes_below(&mut self, bound: u64) -> Vec<u8> {
        let len = self.below(bound) as usize;
        self.bytes(len)
    }

    /// Random bytes, `low..low + span` of them.
    pub fn bytes_between(&mut self, low: u64, span: u64) -> Vec<u8> {
        let len = (low + self.below(span)) as usize;
        self.bytes(len)
    }
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0xF)] as char);
    }
    out
}

/// Parses hex (either case); `None` on odd length or a non-hex digit.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| (c as char).to_digit(16);
            Some((digit(pair[0])? * 16 + digit(pair[1])?) as u8)
        })
        .collect()
}

use crate::compress;
use crate::crypto::{Direction, Session};
use crate::wire::{Fragment, FragmentAssembler, Instruction, fragment};

/// Encodes a server→client instruction the way mosh-server does: protobuf,
/// zlib, fragments of at most `mtu` bytes, each sealed with consecutive
/// sequence numbers from `seq`.
#[allow(clippy::too_many_arguments)]
pub fn server_datagrams(
    session: &Session,
    seq: u64,
    id: u64,
    instruction: &Instruction,
    timestamp: u16,
    timestamp_reply: u16,
    mtu: usize,
) -> Vec<Vec<u8>> {
    let compressed = compress::compress(&instruction.encode());
    fragment(&compressed, id, mtu)
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            session.seal(
                seq + index as u64,
                Direction::ToClient,
                timestamp,
                timestamp_reply,
                &piece.encode(),
            )
        })
        .collect()
}

/// One single-fragment server datagram (what most tests need).
#[allow(clippy::too_many_arguments)]
pub fn server_datagram(
    session: &Session,
    seq: u64,
    id: u64,
    old_num: u64,
    new_num: u64,
    ack_num: u64,
    diff: &[u8],
    timestamp_reply: u16,
) -> Vec<u8> {
    let instruction = Instruction {
        old_num,
        new_num,
        ack_num,
        diff: diff.to_vec(),
        ..Instruction::default()
    };
    let mut datagrams = server_datagrams(session, seq, id, &instruction, 0, timestamp_reply, 1300);
    assert_eq!(datagrams.len(), 1, "test payloads should fit one fragment");
    datagrams.remove(0)
}

/// The instruction inside a single-fragment client datagram, as a mosh
/// server would read it.
pub fn client_instruction(session: &Session, datagram: &[u8]) -> Instruction {
    let message = session.open(datagram).expect("client datagram opens");
    assert_eq!(message.direction, Direction::ToServer);
    let fragment = Fragment::parse(&message.payload).expect("fragment");
    let payload = FragmentAssembler::new()
        .receive(fragment)
        .expect("single-fragment instruction");
    Instruction::decode(&compress::decompress(&payload).expect("zlib")).expect("instruction")
}
