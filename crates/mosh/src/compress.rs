//! The zlib format (RFC 1950) mosh wraps every transport instruction in: its
//! `Compressor` calls zlib's `compress` and `uncompress`, so a payload is a
//! 2-byte header, raw DEFLATE and a big-endian Adler-32 of the uncompressed
//! bytes. Ported from `MoshCompression.swift`, which frames Apple's raw
//! DEFLATE by hand; here the DEFLATE is miniz_oxide's and the framing is done
//! the same way: header `78 9C`, the trailer computed by [`adler32`].
//!
//! Compressed bytes are valid zlib but not byte-identical to the app's
//! (Apple's encoder is zlib 1.2.12's deflate at level 5); only the framing
//! is. Decompression (`inflate.rs`) follows zlib's rules, and is
//! checked against both the real zlib and the app.

use crate::inflate::{InflateError, inflate};

/// mosh's `Compressor::BUFFER_SIZE`: zlib's `uncompress` there refuses any
/// instruction that inflates to more, so no server sends one.
pub const MAX_DECOMPRESSED_LEN: usize = 2048 * 2048;

/// miniz_oxide level 6, the zlib default the `78 9C` header announces.
const LEVEL: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionError {
    /// Fewer than 6 bytes, or a header zlib's `uncompress` rejects: wrong
    /// check bits, a method other than DEFLATE, a window over 32 KiB, or a
    /// preset dictionary.
    BadHeader,
    /// The DEFLATE data is invalid or truncated.
    InflateFailed,
    /// The Adler-32 trailer does not match the inflated bytes.
    BadChecksum,
    /// The payload inflates past [`MAX_DECOMPRESSED_LEN`].
    TooLarge,
}

impl std::fmt::Display for CompressionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CompressionError::BadHeader => "bad zlib header",
            CompressionError::InflateFailed => "invalid DEFLATE data",
            CompressionError::BadChecksum => "zlib Adler-32 mismatch",
            CompressionError::TooLarge => "zlib payload inflates past mosh's 4 MiB limit",
        })
    }
}

impl std::error::Error for CompressionError {}

/// zlib-wraps `data`: header `78 9C` (32 KiB window, default level, exactly
/// what `compress2(..., Z_DEFAULT_COMPRESSION)` writes), raw DEFLATE, then
/// the Adler-32 of `data`, big-endian.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let raw = miniz_oxide::deflate::compress_to_vec(data, LEVEL);
    let mut out = Vec::with_capacity(raw.len() + 6);
    out.extend_from_slice(&[0x78, 0x9C]);
    out.extend_from_slice(&raw);
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Inflates a zlib stream the way zlib's `uncompress` does: header, DEFLATE
/// data, then the Adler-32 that follows it; bytes after the trailer are
/// ignored, as zlib ignores them; at most mosh's 4 MiB buffer.
///
/// Fix (zlib-strict): the Swift version stripped two header bytes and four
/// trailer bytes unchecked and raw-inflated what lay between with Apple's
/// decoder, so it accepted a wrong header or checksum, and Apple's DEFLATE
/// also lets through a stored block whose NLEN does not match and an invalid
/// code-length set. RFC 1950 requires a decompressor to check header and
/// checksum, and mosh's own client inflates with zlib's `uncompress` (into
/// mosh's 4 MiB buffer); this one rejects what zlib rejects, as the Swift
/// client (the system zlib) now does too. The DEFLATE data used to go
/// through miniz_oxide, which also accepts a code set of a single code
/// longer than one bit; zlib refuses it, and when the garbage it decoded
/// matched the trailer Rust accepted a stream both other clients refuse
/// (`inflate.rs`).
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, CompressionError> {
    if data.len() < 6 || !header_ok(data[0], data[1]) {
        return Err(CompressionError::BadHeader);
    }
    let (out, used) = inflate(&data[2..], MAX_DECOMPRESSED_LEN).map_err(|error| match error {
        InflateError::Invalid => CompressionError::InflateFailed,
        InflateError::TooLarge => CompressionError::TooLarge,
    })?;
    let trailer = data
        .get(2 + used..2 + used + 4)
        .ok_or(CompressionError::InflateFailed)?;
    if u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]) != adler32(&out) {
        return Err(CompressionError::BadChecksum);
    }
    Ok(out)
}

/// zlib's `inflate` header checks: FCHECK, CM = 8, CINFO <= 7, no FDICT.
fn header_ok(cmf: u8, flg: u8) -> bool {
    (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0
        && cmf & 0x0F == 8
        && cmf >> 4 <= 7
        && flg & 0x20 == 0
}

/// Adler-32 (RFC 1950), the zlib trailer.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    // The largest n with 255 n (n + 1) / 2 + (n + 1)(MOD - 1) < 2^32: sums
    // over this many bytes cannot overflow before the modulo (zlib's NMAX).
    const BLOCK: usize = 5_552;
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(BLOCK) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}
