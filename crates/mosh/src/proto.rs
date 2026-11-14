//! The minimal protobuf (proto2 wire format) mosh's messages need: varints,
//! keys and length-delimited fields. Ported from `ProtoWriter` and
//! `ProtoReader` in `MoshWire.swift`, with the same limits: a varint is at
//! most ten bytes, unknown fields of wire types 0, 1, 2 and 5 are skipped,
//! groups (3, 4) and the reserved types 6 and 7 are errors.

/// Why a message or fragment did not decode (the Swift `MoshWireError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The input ends inside a varint, or a varint runs past ten bytes.
    TruncatedVarint,
    /// A length-delimited or fixed-width field runs past the input.
    TruncatedField,
    /// A wire type the decoder cannot skip.
    UnsupportedWireType(u8),
    /// A transport fragment shorter than its 10-byte header.
    TruncatedFragment,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::TruncatedVarint => f.write_str("truncated protobuf varint"),
            WireError::TruncatedField => f.write_str("truncated protobuf field"),
            WireError::UnsupportedWireType(wire_type) => {
                write!(f, "unsupported protobuf wire type {wire_type}")
            }
            WireError::TruncatedFragment => {
                f.write_str("transport fragment shorter than its header")
            }
        }
    }
}

impl std::error::Error for WireError {}

/// Wire type 0.
pub const VARINT: u8 = 0;
/// Wire type 2.
pub const LENGTH_DELIMITED: u8 = 2;

/// Append-only protobuf writer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtoWriter {
    bytes: Vec<u8>,
}

impl ProtoWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn varint(&mut self, value: u64) {
        let mut value = value;
        while value >= 0x80 {
            self.bytes.push((value as u8) | 0x80);
            value >>= 7;
        }
        self.bytes.push(value as u8);
    }

    /// A wire-type-0 field.
    pub fn field_varint(&mut self, number: u32, value: u64) {
        self.key(number, VARINT);
        self.varint(value);
    }

    /// A wire-type-2 field.
    pub fn field_bytes(&mut self, number: u32, value: &[u8]) {
        self.key(number, LENGTH_DELIMITED);
        self.varint(value.len() as u64);
        self.bytes.extend_from_slice(value);
    }

    fn key(&mut self, number: u32, wire_type: u8) {
        self.varint(u64::from(number) << 3 | u64::from(wire_type));
    }
}

/// Forward-only protobuf reader over a borrowed buffer.
#[derive(Debug, Clone)]
pub struct ProtoReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub fn read_varint(&mut self) -> Result<u64, WireError> {
        let mut result = 0u64;
        let mut shift = 0u32;
        while let Some(&byte) = self.bytes.get(self.pos) {
            self.pos += 1;
            // At shift 63 only the low bit of the tenth byte still fits;
            // the rest falls off the top, as in Swift and protobuf.
            result |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift > 63 {
                return Err(WireError::TruncatedVarint);
            }
        }
        Err(WireError::TruncatedVarint)
    }

    /// A field key: the field number and the wire type.
    pub fn read_key(&mut self) -> Result<(u64, u8), WireError> {
        let key = self.read_varint()?;
        Ok((key >> 3, (key & 0x7) as u8))
    }

    pub fn read_length_delimited(&mut self) -> Result<&'a [u8], WireError> {
        let length = self.read_varint()?;
        let remaining = self.bytes.len() - self.pos;
        if length > remaining as u64 {
            return Err(WireError::TruncatedField);
        }
        let start = self.pos;
        self.pos += length as usize;
        Ok(&self.bytes[start..self.pos])
    }

    /// Skips a field of `wire_type` (an unknown field).
    pub fn skip_field(&mut self, wire_type: u8) -> Result<(), WireError> {
        match wire_type {
            0 => self.read_varint().map(drop),
            1 => self.advance(8),
            2 => self.read_length_delimited().map(drop),
            5 => self.advance(4),
            other => Err(WireError::UnsupportedWireType(other)),
        }
    }

    fn advance(&mut self, count: usize) -> Result<(), WireError> {
        if self.bytes.len() - self.pos < count {
            return Err(WireError::TruncatedField);
        }
        self.pos += count;
        Ok(())
    }
}
