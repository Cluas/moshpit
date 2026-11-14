//! mosh protocol 2 on the wire: the transport instruction
//! (`transportinstruction.proto`), user input (`userinput.proto`), host
//! output (`hostinput.proto`) and the transport fragment framing, ported from
//! `MoshWire.swift`. Encodings are byte-identical to the Swift ones: fields in
//! number order, the instruction's fields 1 to 6 always written, `chaff` only
//! when non-empty.

use std::collections::BTreeMap;

pub use crate::proto::WireError;
use crate::proto::{LENGTH_DELIMITED, ProtoReader, ProtoWriter, VARINT};

/// mosh 1.x speaks transport protocol version 2.
pub const PROTOCOL_VERSION: u32 = 2;

/// `TransportBuffers.Instruction`:
///
/// ```proto
/// message Instruction {
///   optional uint32 protocol_version = 1;
///   optional uint64 old_num         = 2;
///   optional uint64 new_num         = 3;
///   optional uint64 ack_num         = 4;
///   optional uint64 throwaway_num   = 5;
///   optional bytes  diff            = 6;
///   optional bytes  chaff           = 7;
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub protocol_version: u32,
    pub old_num: u64,
    pub new_num: u64,
    pub ack_num: u64,
    pub throwaway_num: u64,
    pub diff: Vec<u8>,
    pub chaff: Vec<u8>,
}

impl Default for Instruction {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            old_num: 0,
            new_num: 0,
            ack_num: 0,
            throwaway_num: 0,
            diff: Vec::new(),
            chaff: Vec::new(),
        }
    }
}

impl Instruction {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = ProtoWriter::new();
        writer.field_varint(1, u64::from(self.protocol_version));
        writer.field_varint(2, self.old_num);
        writer.field_varint(3, self.new_num);
        writer.field_varint(4, self.ack_num);
        writer.field_varint(5, self.throwaway_num);
        writer.field_bytes(6, &self.diff);
        if !self.chaff.is_empty() {
            writer.field_bytes(7, &self.chaff);
        }
        writer.into_bytes()
    }

    /// Decodes an instruction. Absent fields keep their defaults (version 2,
    /// zeros, empty); unknown fields and known fields with an unexpected
    /// wire type are skipped.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut instruction = Instruction::default();
        let mut reader = ProtoReader::new(bytes);
        while !reader.is_at_end() {
            let (field, wire_type) = reader.read_key()?;
            match (field, wire_type) {
                (1, VARINT) => instruction.protocol_version = reader.read_varint()? as u32,
                (2, VARINT) => instruction.old_num = reader.read_varint()?,
                (3, VARINT) => instruction.new_num = reader.read_varint()?,
                (4, VARINT) => instruction.ack_num = reader.read_varint()?,
                (5, VARINT) => instruction.throwaway_num = reader.read_varint()?,
                (6, LENGTH_DELIMITED) => {
                    instruction.diff = reader.read_length_delimited()?.to_vec()
                }
                (7, LENGTH_DELIMITED) => {
                    instruction.chaff = reader.read_length_delimited()?.to_vec();
                }
                _ => reader.skip_field(wire_type)?,
            }
        }
        Ok(instruction)
    }
}

/// One user-input event, a proto2 extension of `ClientBuffers.Instruction`:
/// `Keystroke` at field 2 (keys at field 4), `ResizeMessage` at field 3
/// (`int32` width at field 5, height at field 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserEvent {
    Keystroke(Vec<u8>),
    Resize { width: i32, height: i32 },
}

impl UserEvent {
    pub fn is_keystroke(&self) -> bool {
        matches!(self, UserEvent::Keystroke(_))
    }

    /// The bytes this event adds to an encoded `UserMessage`
    /// ([`encode_user_message`]).
    pub fn encoded_len(&self) -> usize {
        let body = match self {
            UserEvent::Keystroke(keys) => field_len(keys.len()),
            UserEvent::Resize { width, height } => {
                2 + varint_len(i64::from(*width) as u64) + varint_len(i64::from(*height) as u64)
            }
        };
        field_len(field_len(body))
    }
}

/// Bytes of a varint.
fn varint_len(value: u64) -> usize {
    (64 - value.max(1).leading_zeros() as usize).div_ceil(7)
}

/// Bytes of a length-delimited field (field numbers under 16) of `len`.
fn field_len(len: usize) -> usize {
    1 + varint_len(len as u64) + len
}

/// Encodes `message UserMessage { repeated Instruction instruction = 1; }`.
pub fn encode_user_message(events: &[UserEvent]) -> Vec<u8> {
    let mut writer = ProtoWriter::new();
    for event in events {
        let mut instruction = ProtoWriter::new();
        match event {
            UserEvent::Keystroke(keys) => {
                let mut keystroke = ProtoWriter::new();
                keystroke.field_bytes(4, keys);
                instruction.field_bytes(2, keystroke.bytes());
            }
            UserEvent::Resize { width, height } => {
                let mut resize = ProtoWriter::new();
                // int32: negative values are sign-extended to ten bytes.
                resize.field_varint(5, i64::from(*width) as u64);
                resize.field_varint(6, i64::from(*height) as u64);
                instruction.field_bytes(3, resize.bytes());
            }
        }
        writer.field_bytes(1, instruction.bytes());
    }
    writer.into_bytes()
}

/// Decodes a `UserMessage`, dropping instructions that carry no known
/// extension (proto2 forward compatibility).
pub fn decode_user_message(bytes: &[u8]) -> Result<Vec<UserEvent>, WireError> {
    let mut events = Vec::new();
    let mut reader = ProtoReader::new(bytes);
    while !reader.is_at_end() {
        let (field, wire_type) = reader.read_key()?;
        if (field, wire_type) != (1, LENGTH_DELIMITED) {
            reader.skip_field(wire_type)?;
            continue;
        }
        if let Some(event) = decode_user_instruction(reader.read_length_delimited()?)? {
            events.push(event);
        }
    }
    Ok(events)
}

/// The last known extension in the instruction wins.
fn decode_user_instruction(bytes: &[u8]) -> Result<Option<UserEvent>, WireError> {
    let mut reader = ProtoReader::new(bytes);
    let mut event = None;
    while !reader.is_at_end() {
        let (field, wire_type) = reader.read_key()?;
        match (field, wire_type) {
            (2, LENGTH_DELIMITED) => {
                let keys = decode_bytes_field(reader.read_length_delimited()?, 4)?;
                event = Some(UserEvent::Keystroke(keys));
            }
            (3, LENGTH_DELIMITED) => {
                let (width, height) = decode_resize(reader.read_length_delimited()?)?;
                event = Some(UserEvent::Resize { width, height });
            }
            _ => reader.skip_field(wire_type)?,
        }
    }
    Ok(event)
}

/// The last `number` bytes field of a submessage (empty when absent).
fn decode_bytes_field(bytes: &[u8], number: u64) -> Result<Vec<u8>, WireError> {
    let mut reader = ProtoReader::new(bytes);
    let mut value: &[u8] = &[];
    while !reader.is_at_end() {
        let (field, wire_type) = reader.read_key()?;
        if (field, wire_type) == (number, LENGTH_DELIMITED) {
            value = reader.read_length_delimited()?;
        } else {
            reader.skip_field(wire_type)?;
        }
    }
    Ok(value.to_vec())
}

/// A `ResizeMessage` (zeros when absent).
///
/// Fix (int32-decode): width and height are `int32`, so a varint is cut to
/// its low 32 bits as protobuf (and mosh-server's parser) does; the Swift
/// decoder kept all 64.
fn decode_resize(bytes: &[u8]) -> Result<(i32, i32), WireError> {
    let mut reader = ProtoReader::new(bytes);
    let (mut width, mut height) = (0, 0);
    while !reader.is_at_end() {
        let (field, wire_type) = reader.read_key()?;
        match (field, wire_type) {
            (5, VARINT) => width = reader.read_varint()? as i32,
            (6, VARINT) => height = reader.read_varint()? as i32,
            _ => reader.skip_field(wire_type)?,
        }
    }
    Ok((width, height))
}

/// One instruction of a `HostBuffers.HostMessage`, the server's diff:
/// `HostBytes` (field 2, `hoststring` at 4) carries terminal output,
/// `ResizeMessage` (field 3) the server framebuffer's new size, `EchoAck`
/// (field 7, `echo_ack_num` at 8) the newest user state the server received
/// at least 50 ms ago. mosh-server's `Complete::diff_from` writes the echo
/// ack first, then a resize, then the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostInstruction {
    HostBytes(Vec<u8>),
    Resize { width: i32, height: i32 },
    EchoAck(u64),
}

/// Encodes a host diff the way mosh-server does. The client never sends
/// one; tests and the harness use it to play the server.
pub fn encode_host_message(instructions: &[HostInstruction]) -> Vec<u8> {
    let mut writer = ProtoWriter::new();
    for instruction in instructions {
        let mut inner = ProtoWriter::new();
        let mut body = ProtoWriter::new();
        match instruction {
            HostInstruction::HostBytes(bytes) => {
                body.field_bytes(4, bytes);
                inner.field_bytes(2, body.bytes());
            }
            HostInstruction::Resize { width, height } => {
                body.field_varint(5, i64::from(*width) as u64);
                body.field_varint(6, i64::from(*height) as u64);
                inner.field_bytes(3, body.bytes());
            }
            HostInstruction::EchoAck(num) => {
                body.field_varint(8, *num);
                inner.field_bytes(7, body.bytes());
            }
        }
        writer.field_bytes(1, inner.bytes());
    }
    writer.into_bytes()
}

/// A decoded host diff, the way the client consumes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostMessage {
    /// Every `HostBytes` payload, concatenated in order.
    pub host_bytes: Vec<u8>,
    /// The last resize, if any.
    pub resize: Option<(i32, i32)>,
    /// The last echo ack, if any (the server only sends it when it moved).
    pub echo_ack: Option<u64>,
}

impl HostMessage {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut message = HostMessage::default();
        let mut reader = ProtoReader::new(bytes);
        while !reader.is_at_end() {
            let (field, wire_type) = reader.read_key()?;
            if (field, wire_type) != (1, LENGTH_DELIMITED) {
                reader.skip_field(wire_type)?;
                continue;
            }
            let mut inner = ProtoReader::new(reader.read_length_delimited()?);
            while !inner.is_at_end() {
                let (field, wire_type) = inner.read_key()?;
                match (field, wire_type) {
                    (2, LENGTH_DELIMITED) => {
                        let bytes = decode_bytes_field(inner.read_length_delimited()?, 4)?;
                        message.host_bytes.extend_from_slice(&bytes);
                    }
                    (3, LENGTH_DELIMITED) => {
                        message.resize = Some(decode_resize(inner.read_length_delimited()?)?);
                    }
                    (7, LENGTH_DELIMITED) => {
                        message.echo_ack = Some(decode_echo_ack(inner.read_length_delimited()?)?);
                    }
                    _ => inner.skip_field(wire_type)?,
                }
            }
        }
        Ok(message)
    }
}

fn decode_echo_ack(bytes: &[u8]) -> Result<u64, WireError> {
    let mut reader = ProtoReader::new(bytes);
    let mut num = 0;
    while !reader.is_at_end() {
        let (field, wire_type) = reader.read_key()?;
        if (field, wire_type) == (8, VARINT) {
            num = reader.read_varint()?;
        } else {
            reader.skip_field(wire_type)?;
        }
    }
    Ok(num)
}

/// Bytes of the fragment header: a big-endian 8-byte instruction id and a
/// big-endian 2-byte fragment number whose top bit marks the last fragment.
pub const FRAGMENT_HEADER_LEN: usize = 10;
const FINAL_BIT: u16 = 0x8000;

/// One transport fragment: a slice of a compressed instruction (not
/// protobuf).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub id: u64,
    /// 15-bit index within the instruction, from 0.
    pub num: u16,
    pub is_final: bool,
    pub contents: Vec<u8>,
}

impl Fragment {
    /// `num` keeps its low 15 bits, like the Swift initializer.
    pub fn new(id: u64, num: u16, is_final: bool, contents: Vec<u8>) -> Self {
        Self {
            id,
            num: num & !FINAL_BIT,
            is_final,
            contents,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FRAGMENT_HEADER_LEN + self.contents.len());
        encode_fragment_into(self.id, self.num, self.is_final, &self.contents, &mut out);
        out
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() < FRAGMENT_HEADER_LEN {
            return Err(WireError::TruncatedFragment);
        }
        let id = u64::from_be_bytes(bytes[..8].try_into().expect("8 bytes"));
        let raw = u16::from_be_bytes([bytes[8], bytes[9]]);
        Ok(Self {
            id,
            num: raw & !FINAL_BIT,
            is_final: raw & FINAL_BIT != 0,
            contents: bytes[FRAGMENT_HEADER_LEN..].to_vec(),
        })
    }
}

/// Writes one fragment (header and contents) to `out`.
pub fn encode_fragment_into(id: u64, num: u16, is_final: bool, contents: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&id.to_be_bytes());
    let num = (num & !FINAL_BIT) | if is_final { FINAL_BIT } else { 0 };
    out.extend_from_slice(&num.to_be_bytes());
    out.extend_from_slice(contents);
}

/// The byte ranges [`fragment`] cuts `len` payload bytes into: at least one
/// range (an empty payload still makes one, final, fragment), each at most
/// `mtu - 10` bytes long (at least 1).
pub fn fragment_ranges(len: usize, mtu: usize) -> impl Iterator<Item = std::ops::Range<usize>> {
    let chunk = mtu.saturating_sub(FRAGMENT_HEADER_LEN).max(1);
    let count = len.div_ceil(chunk).max(1);
    (0..count).map(move |index| index * chunk..len.min((index + 1) * chunk))
}

/// Splits an instruction payload into fragments that each fit in `mtu` bytes
/// once the header is added.
///
/// Fix (fragment-count): past 65,535 fragments (an 84 MB instruction at the
/// transport's MTU) the Swift counter trapped on `UInt16` overflow; here the
/// number keeps wrapping in 15 bits like the Swift initializer masks it. No
/// mosh peer accepts such an instruction anyway: mosh-server inflates at most
/// 4 MiB.
pub fn fragment(payload: &[u8], id: u64, mtu: usize) -> Vec<Fragment> {
    let ranges: Vec<_> = fragment_ranges(payload.len(), mtu).collect();
    let last = ranges.len() - 1;
    ranges
        .into_iter()
        .enumerate()
        .map(|(index, range)| {
            Fragment::new(id, index as u16, index == last, payload[range].to_vec())
        })
        .collect()
}

/// Reassembles fragments into instruction payloads. Only the newest
/// instruction matters: fragments with an older id than the newest seen are
/// dropped, and a newer id discards a partial assembly.
#[derive(Debug, Clone, Default)]
pub struct FragmentAssembler {
    current_id: Option<u64>,
    pieces: BTreeMap<u16, Vec<u8>>,
    final_num: Option<u16>,
}

impl FragmentAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one fragment; the payload comes back once every fragment of the
    /// current instruction has arrived. A completed instruction keeps its
    /// id, so a retransmitted copy assembles again.
    pub fn receive(&mut self, fragment: Fragment) -> Option<Vec<u8>> {
        match self.current_id {
            Some(current) if fragment.id < current => return None,
            Some(current) if fragment.id == current => {}
            _ => self.reset(fragment.id),
        }
        let Fragment {
            num,
            is_final,
            contents,
            ..
        } = fragment;
        self.pieces.insert(num, contents);
        if is_final {
            self.final_num = Some(num);
        }
        let final_num = self.final_num?;
        if self.pieces.len() != usize::from(final_num) + 1 {
            return None;
        }
        // A stray piece numbered past the final one can fill the count while
        // leaving a gap; then wait (Swift does the same).
        if (0..=final_num).any(|num| !self.pieces.contains_key(&num)) {
            return None;
        }
        let mut payload = Vec::new();
        for piece in std::mem::take(&mut self.pieces).into_values() {
            payload.extend_from_slice(&piece);
        }
        self.final_num = None;
        Some(payload)
    }

    fn reset(&mut self, id: u64) {
        self.current_id = Some(id);
        self.pieces.clear();
        self.final_num = None;
    }
}
