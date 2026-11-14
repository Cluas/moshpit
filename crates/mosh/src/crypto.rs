//! mosh's datagram crypto and message framing (`crypto.cc`, `network.cc`),
//! ported from `MoshCrypto.swift`.
//!
//! A datagram is `nonce[8] || OCB(timestamp[2] || timestamp_reply[2] ||
//! payload)`. The 8 clear bytes are a big-endian 64-bit value whose top bit
//! is the direction and whose low 63 bits are the sender's sequence number;
//! the OCB nonce is those 8 bytes behind 4 zero bytes. Timestamps are
//! big-endian; the payload is one transport fragment.

use crate::ocb::{NONCE_LEN, Ocb3, OcbError, TAG_LEN};

/// Clear nonce bytes at the front of every datagram.
pub const WIRE_NONCE_LEN: usize = 8;
/// Bytes a datagram adds around its payload: clear nonce, the two
/// timestamps and the tag.
pub const DATAGRAM_OVERHEAD: usize = WIRE_NONCE_LEN + 4 + TAG_LEN;

const DIRECTION_BIT: u64 = 1 << 63;
const SEQUENCE_MASK: u64 = !DIRECTION_BIT;

/// Which way a datagram travels: the top bit of the wire nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    ToServer,
    ToClient,
}

/// An opened datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub timestamp: u16,
    pub timestamp_reply: u16,
    pub payload: Vec<u8>,
    /// The low 63 bits of the wire nonce.
    pub seq: u64,
    pub direction: Direction,
}

/// Why a datagram did not open. `PacketTooShort` is the Swift
/// `CryptoError.packetTooShort`, `AuthenticationFailed` its OCB3 error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// Shorter than nonce plus tag, or a plaintext without both timestamps.
    PacketTooShort,
    AuthenticationFailed,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OpenError::PacketTooShort => "mosh datagram too short",
            OpenError::AuthenticationFailed => "mosh datagram failed authentication",
        })
    }
}

impl std::error::Error for OpenError {}

/// The OCB key schedule of one mosh session (the `MOSH CONNECT` key).
#[derive(Debug, Clone)]
pub struct Session {
    ocb: Ocb3,
}

impl Session {
    /// - `key`: the 16-byte AES-128 session key.
    pub fn new(key: &[u8]) -> Result<Self, OcbError> {
        Ok(Self {
            ocb: Ocb3::new(key)?,
        })
    }

    /// Seals one datagram. `seq` must be unique and increasing per session
    /// and direction; only its low 63 bits are used.
    pub fn seal(
        &self,
        seq: u64,
        direction: Direction,
        timestamp: u16,
        timestamp_reply: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + DATAGRAM_OVERHEAD);
        self.seal_into(
            seq,
            direction,
            timestamp,
            timestamp_reply,
            payload,
            &mut out,
        );
        out
    }

    /// [`Session::seal`] appending to `out`.
    pub fn seal_into(
        &self,
        seq: u64,
        direction: Direction,
        timestamp: u16,
        timestamp_reply: u16,
        payload: &[u8],
        out: &mut Vec<u8>,
    ) {
        let wire_nonce = wire_nonce(seq, direction);
        let start = out.len();
        out.extend_from_slice(&wire_nonce);
        out.extend_from_slice(&timestamp.to_be_bytes());
        out.extend_from_slice(&timestamp_reply.to_be_bytes());
        out.extend_from_slice(payload);
        let mut nonce = [0u8; NONCE_LEN];
        nonce[4..].copy_from_slice(&wire_nonce);
        self.ocb
            .seal_in_place(&nonce, &[], out, start + WIRE_NONCE_LEN)
            .expect("a datagram is far below the OCB length limit");
    }

    /// Opens a received datagram, decoding sequence number and direction.
    pub fn open(&self, packet: &[u8]) -> Result<Message, OpenError> {
        if packet.len() < WIRE_NONCE_LEN + TAG_LEN {
            return Err(OpenError::PacketTooShort);
        }
        let (wire_nonce, sealed) = packet.split_at(WIRE_NONCE_LEN);
        let mut nonce = [0u8; NONCE_LEN];
        nonce[4..].copy_from_slice(wire_nonce);
        let mut plaintext = sealed.to_vec();
        self.ocb
            .open_in_place(&nonce, &[], &mut plaintext)
            .map_err(|_| OpenError::AuthenticationFailed)?;
        if plaintext.len() < 4 {
            return Err(OpenError::PacketTooShort);
        }
        let nonce64 = u64::from_be_bytes(wire_nonce.try_into().expect("8 bytes"));
        let timestamp = u16::from_be_bytes([plaintext[0], plaintext[1]]);
        let timestamp_reply = u16::from_be_bytes([plaintext[2], plaintext[3]]);
        plaintext.drain(..4);
        Ok(Message {
            timestamp,
            timestamp_reply,
            payload: plaintext,
            seq: nonce64 & SEQUENCE_MASK,
            direction: if nonce64 & DIRECTION_BIT != 0 {
                Direction::ToClient
            } else {
                Direction::ToServer
            },
        })
    }
}

fn wire_nonce(seq: u64, direction: Direction) -> [u8; WIRE_NONCE_LEN] {
    let bit = match direction {
        Direction::ToServer => 0,
        Direction::ToClient => DIRECTION_BIT,
    };
    (bit | (seq & SEQUENCE_MASK)).to_be_bytes()
}
