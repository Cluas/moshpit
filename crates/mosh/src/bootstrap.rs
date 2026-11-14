//! Starting `mosh-server` over SSH: the command line, and the parsing of
//! what the server prints before it daemonizes (`MOSH CONNECT <port> <key>`
//! and `[mosh-server detached, pid = N]`). Ported from `MoshBootstrap.swift`;
//! running the command is the SSH layer's job.

use std::fmt;

/// Directories a non-interactive exec shell usually lacks but Homebrew and
/// user installs live in. Mirrors `HostCapabilities.extraPathDirs` (Swift),
/// which the host probe and every launch command share; `$HOME` expands on
/// the remote side.
pub const EXTRA_PATH_DIRS: &str =
    "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin";

/// What a successful bootstrap yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    /// Where to send datagrams: the SSH host.
    pub host: String,
    /// Never 0: a port that cannot connect is rejected at parse time.
    pub udp_port: u16,
    /// The AES-128 session key.
    pub key: [u8; 16],
    /// The detached server's pid, when its line was captured; lets the
    /// session reap a server whose client never made first contact. `None`
    /// means reaping is skipped, never guessed.
    pub server_pid: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapError {
    /// No `MOSH CONNECT` line (or one without a numeric port); carries the
    /// whole output for the error message.
    NoConnectLine(String),
    /// The port field is a number but not a usable port (0, negative, or
    /// over 65535).
    UnusablePort(i64),
    BadKey,
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootstrapError::NoConnectLine(output) => write!(
                f,
                "mosh-server did not print a MOSH CONNECT line. Output: {}",
                sanitized_excerpt(output)
            ),
            BootstrapError::UnusablePort(port) => write!(
                f,
                "mosh-server reported UDP port {port}, which is not a usable port"
            ),
            BootstrapError::BadKey => f.write_str("mosh-server returned a malformed session key"),
        }
    }
}

impl std::error::Error for BootstrapError {}

/// The remote command line: `mosh-server new -s -c 256 [-p a[:b]] -l
/// LANG=<locale>`.
///
/// `-s` binds the UDP socket to the server-side address of the SSH
/// connection, `-c 256` asks for 256 colours. A configured port range rides
/// along as `-p`; degenerate values degrade instead of failing remotely: an
/// unusable start (outside 1...65535) drops `-p` (mosh-server's default
/// 60000-61000), an unusable end keeps the start as a single port. A bare
/// binary name gets [`EXTRA_PATH_DIRS`] appended to `PATH`, because the exec
/// channel is a non-interactive shell; an explicit path is used as given.
pub fn command(
    server_binary: &str,
    locale: &str,
    port_range_start: Option<i64>,
    port_range_end: Option<i64>,
) -> String {
    let mut cmd = format!("{server_binary} new -s -c 256");
    if !server_binary.contains('/') {
        cmd = format!("PATH=\"$PATH:{EXTRA_PATH_DIRS}\" {cmd}");
    }
    if let Some(start) = port_range_start.filter(|start| (1..=65_535).contains(start)) {
        match port_range_end {
            Some(end) if (start..=65_535).contains(&end) && end > start => {
                cmd.push_str(&format!(" -p {start}:{end}"));
            }
            _ => cmd.push_str(&format!(" -p {start}")),
        }
    }
    cmd.push_str(&format!(" -l LANG={locale}"));
    cmd
}

/// [`parse`] for the raw bytes of the exec channel, decoded the way the app
/// decodes them (invalid UTF-8 becomes U+FFFD).
pub fn parse_bytes(output: &[u8], host: &str) -> Result<Credentials, BootstrapError> {
    parse(&String::from_utf8_lossy(output), host)
}

/// Parses `mosh-server new` output.
///
/// The connect line is the first line starting with `MOSH CONNECT `; lines
/// end at any of CR, LF, CRLF, VT, FF, NEL, U+2028 and U+2029 (Swift's
/// `Character.isNewline`). Its fields are separated by runs of spaces: port
/// third, key fourth. The key is mosh's 22 unpadded base64 characters.
///
/// Fix (unicode-syntax): the prefix and the field separators are matched as
/// bytes, as mosh-server writes them. The Swift version matched grapheme
/// clusters, so a combining mark right after a space glued the space to the
/// next field.
pub fn parse(output: &str, host: &str) -> Result<Credentials, BootstrapError> {
    let no_line = || BootstrapError::NoConnectLine(output.to_owned());
    let line = output
        .split(is_newline)
        .find(|line| line.starts_with("MOSH CONNECT "))
        .ok_or_else(no_line)?;
    let parts: Vec<&str> = line.split(' ').filter(|part| !part.is_empty()).collect();
    if parts.len() < 4 {
        return Err(no_line());
    }
    // Swift's `Int(_:)`: an optional sign and ASCII digits, within 64 bits.
    let port: i64 = parts[2].parse().map_err(|_| no_line())?;
    // A banner or interleaved stderr can move a pid, timestamp or version
    // into this field; out of range used to crash the app (UInt16 narrowing).
    let udp_port = u16::try_from(port)
        .ok()
        .filter(|&port| port != 0)
        .ok_or(BootstrapError::UnusablePort(port))?;
    let key = decode_key(parts[3])?;
    Ok(Credentials {
        host: host.to_owned(),
        udp_port,
        key,
        server_pid: detached_pid(output),
    })
}

/// Swift's `Character.isNewline`, scalar by scalar (CRLF splits twice, and
/// the empty piece between is never a connect line).
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// The key token, padded with `=` to a multiple of four and decoded the way
/// Foundation's `Data(base64Encoded:)` does, must give 16 bytes.
fn decode_key(token: &str) -> Result<[u8; 16], BootstrapError> {
    let mut padded = token.as_bytes().to_vec();
    // Swift pads by Character count; for the only tokens that can decode
    // (ASCII ones) that is the byte count.
    let mut count = token.chars().count();
    while !count.is_multiple_of(4) {
        padded.push(b'=');
        count += 1;
    }
    foundation_base64_decode(&padded)
        .and_then(|key| <[u8; 16]>::try_from(key).ok())
        .ok_or(BootstrapError::BadKey)
}

/// Foundation's `Data(base64Encoded:)` without options (swift-foundation
/// `Base64._decode`), quirks included: trailing `=` beyond the last data
/// quartet are ignored, the bits a short final quartet leaves over are not
/// checked, and an input of four or more `=` and nothing else decodes to a
/// single zero byte.
fn foundation_base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }
    let Some(last_data) = input.iter().rposition(|&byte| byte != b'=') else {
        return (input.len() >= 4).then(|| vec![0]);
    };
    let to_parse = (last_data + 1).div_ceil(4) * 4;
    if to_parse > input.len() {
        return None;
    }
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(to_parse / 4 * 3);
    let full_chunks = to_parse / 4 - 1;
    let (chunks, _) = input[..full_chunks * 4].as_chunks::<4>();
    for chunk in chunks {
        let mut x = 0u32;
        for &byte in chunk {
            x = x << 6 | value(byte)?;
        }
        out.extend_from_slice(&x.to_be_bytes()[1..]);
    }
    let last = &input[full_chunks * 4..to_parse];
    let optional = |byte: u8| (byte != b'=').then_some(byte);
    let (a2, a3) = (optional(last[2]), optional(last[3]));
    // `=` in the first two places is a bad character; in the last two it
    // only drops that output byte (so `QQ=A` gives bytes 0 and 2).
    let mut x = value(last[0])? << 18 | value(last[1])? << 12;
    if let Some(a2) = a2 {
        x |= value(a2)? << 6;
    }
    if let Some(a3) = a3 {
        x |= value(a3)?;
    }
    out.push((x >> 16) as u8);
    if a2.is_some() {
        out.push((x >> 8) as u8);
    }
    if a3.is_some() {
        out.push(x as u8);
    }
    Some(out)
}

/// The pid from mosh-server's `[mosh-server detached, pid = N]` stderr line,
/// which the exec channel interleaves into the output.
///
/// Fix (pid-anchor): the Swift version took the first match of
/// `pid\s*=\s*(\d+)` anywhere in the output, so a shell startup file that
/// prints e.g. `rapid = 5` before mosh-server's own line named an unrelated
/// process, which the session later `kill`s on the user's host. Here only
/// the detach line counts, and pids 0 (`kill 0` signals the whole process
/// group) and 1 (init; a forked server is never either) are refused.
fn detached_pid(output: &str) -> Option<i64> {
    const MARKER: &str = "mosh-server detached, pid = ";
    let start = output.find(MARKER)? + MARKER.len();
    let digits: &str = &output[start..];
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse().ok().filter(|&pid: &i64| pid > 1)
}

/// A short excerpt of remote output that is safe to show in an error: at
/// most 100 characters (a CRLF counts as one), each control, format,
/// default-ignorable or noncharacter code point (except the plain space)
/// replaced by a visible `\u{XXXX}`, `…` when cut, `<empty>` when nothing is
/// left. The output comes from a host we do not control and ends up in
/// alerts and logs, where escape sequences must stay inert.
///
/// Fix (excerpt-unicode): the Swift version counts grapheme clusters, and
/// with `CharacterSet.illegalCharacters` it also escapes every unassigned
/// code point of planes 0-3 and 14 in Foundation's Unicode tables (138,502
/// code points on macOS 27), while it keeps the noncharacters of planes 4-16.
/// No Unicode-version table here. Both agree on ASCII and on every assigned
/// code point.
pub fn sanitized_excerpt(output: &str) -> String {
    const MAX_UNITS: usize = 100;
    let mut escaped = String::new();
    let mut units = 0;
    let mut truncated = false;
    let mut chars = output.chars().peekable();
    while let Some(c) = chars.next() {
        if units == MAX_UNITS {
            truncated = true;
            break;
        }
        units += 1;
        push_escaped(&mut escaped, c);
        if c == '\r' && chars.peek() == Some(&'\n') {
            push_escaped(&mut escaped, '\n');
            chars.next();
        }
    }
    if escaped.is_empty() {
        return "<empty>".to_owned();
    }
    if truncated {
        escaped.push('…');
    }
    escaped
}

fn push_escaped(out: &mut String, c: char) {
    if c == ' ' || !needs_escape(u32::from(c)) {
        out.push(c);
    } else {
        out.push_str(&format!("\\u{{{:04X}}}", u32::from(c)));
    }
}

/// C0 and C1 controls, `Cf` format characters, default-ignorable code points
/// and noncharacters (Unicode 15.1).
fn needs_escape(scalar: u32) -> bool {
    const RANGES: &[(u32, u32)] = &[
        (0x0000, 0x001F),
        (0x007F, 0x009F),
        (0x00AD, 0x00AD),
        (0x034F, 0x034F),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x115F, 0x1160),
        (0x17B4, 0x17B5),
        (0x180B, 0x180F),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x206F),
        (0x3164, 0x3164),
        (0xFDD0, 0xFDEF),
        (0xFE00, 0xFE0F),
        (0xFEFF, 0xFEFF),
        (0xFFA0, 0xFFA0),
        (0xFFF0, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0000, 0xE0FFF),
    ];
    scalar & 0xFFFE == 0xFFFE || RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&scalar))
}
