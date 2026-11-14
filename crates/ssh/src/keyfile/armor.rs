//! PEM armour around a private key, and OpenSSL's "traditional"
//! encryption of what is inside it.

use zeroize::Zeroizing;

use super::ImportError;
use super::cbc::{BlockCipher, decrypt_cbc_padded};

/// `-----BEGIN <label>-----`, optional RFC 1421 headers, base64,
/// `-----END <label>-----`: the first such block that holds a private key,
/// so a `.pub` pasted alongside is passed over. Line endings, indentation
/// and line breaks lost on the way don't matter.
pub(super) struct Armor {
    pub label: String,
    headers: Vec<(String, String)>,
    pub body: Zeroizing<Vec<u8>>,
}

impl Armor {
    /// The first private key block in `text`: [`ImportError::Unrecognized`]
    /// when there is none, [`ImportError::Damaged`] when its end line is
    /// missing or its base64 doesn't decode.
    pub fn find(text: &str) -> Result<Armor, ImportError> {
        const BEGIN: &str = "-----BEGIN ";
        const DASHES: &str = "-----";
        let mut from = 0;
        while let Some(at) = text[from..].find(BEGIN) {
            let label_start = from + at + BEGIN.len();
            let Some(label_len) = text[label_start..].find(DASHES) else {
                break;
            };
            let label = &text[label_start..label_start + label_len];
            from = label_start + label_len + DASHES.len();
            if !label.ends_with("PRIVATE KEY") {
                continue;
            }
            let end_line = format!("-----END {label}-----");
            let Some(end) = text[from..].find(&end_line) else {
                return Err(ImportError::Damaged);
            };
            let mut headers = Vec::new();
            let mut base64 = Zeroizing::new(String::new());
            for raw in text[from..from + end].split(is_newline) {
                let line = raw.trim_matches(is_horizontal_space);
                // Headers come first; base64 never holds a colon.
                if base64.is_empty()
                    && let Some((name, value)) = line.split_once(':')
                {
                    headers.push((
                        name.trim_matches(is_horizontal_space).to_owned(),
                        value.trim_matches(is_horizontal_space).to_owned(),
                    ));
                } else {
                    base64.extend(line.chars().filter(|c| !c.is_whitespace()));
                }
            }
            let body = strict_base64(&base64).ok_or(ImportError::Damaged)?;
            if body.is_empty() {
                return Err(ImportError::Damaged);
            }
            return Ok(Armor {
                label: label.to_owned(),
                headers,
                body,
            });
        }
        Err(ImportError::Unrecognized)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// OpenSSL's traditional encryption: `Proc-Type: 4,ENCRYPTED` and a
    /// `DEK-Info` naming the cipher and IV. What `ssh-keygen -m PEM` writes
    /// for a key with a passphrase.
    pub fn is_legacy_encrypted(&self) -> bool {
        self.header("Proc-Type")
            .is_some_and(|v| v.to_uppercase().contains("ENCRYPTED"))
    }

    /// The DER inside, decrypted when it is encrypted.
    pub fn legacy_decrypted(
        &self,
        passphrase: Option<&[u8]>,
    ) -> Result<Zeroizing<Vec<u8>>, ImportError> {
        if !self.is_legacy_encrypted() {
            return Ok(self.body.clone());
        }
        let info = self.header("DEK-Info").ok_or(ImportError::Damaged)?;
        let (name, iv) = info.split_once(',').ok_or(ImportError::Damaged)?;
        let (name, iv) = (
            name.trim_matches(is_horizontal_space),
            iv.trim_matches(is_horizontal_space),
        );
        let iv = from_hex(iv).ok_or(ImportError::Damaged)?;
        let cipher = BlockCipher::from_pem_name(name)
            .ok_or_else(|| ImportError::UnsupportedEncryption(name.to_owned()))?;
        if iv.len() != cipher.block_size() {
            return Err(ImportError::Damaged);
        }
        let passphrase = passphrase.ok_or(ImportError::NeedsPassphrase)?;
        // OpenSSL's EVP_BytesToKey: MD5 over (previous ‖ passphrase ‖ salt),
        // one iteration, the salt being the IV's first eight bytes.
        let salt = &iv[..8];
        let mut key = Zeroizing::new(Vec::with_capacity(cipher.key_len() + 16));
        let mut previous: Option<[u8; 16]> = None;
        while key.len() < cipher.key_len() {
            let mut md5 = md5::Context::new();
            if let Some(previous) = previous {
                md5.consume(previous);
            }
            md5.consume(passphrase);
            md5.consume(salt);
            let digest = md5.finalize().0;
            key.extend_from_slice(&digest);
            previous = Some(digest);
        }
        decrypt_cbc_padded(cipher, &self.body, &key[..cipher.key_len()], &iv)
            .ok_or(ImportError::WrongPassphrase)
    }
}

/// What Swift's `Character.isNewline` is: the line breaks a paste can carry.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Spaces and tabs (and other space separators), not line breaks.
fn is_horizontal_space(c: char) -> bool {
    c.is_whitespace() && !is_newline(c)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 as Foundation's `Data(base64Encoded:)` takes it: the standard
/// alphabet, whole quanta, `=` only as the last one or two characters.
/// `None` for anything else, whitespace included.
pub(super) fn strict_base64(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return None;
    }
    let data = &bytes[..bytes.len() - padding];
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len() / 4 * 3));
    let (mut acc, mut held) = (0u32, 0);
    for &byte in data {
        let value = ALPHABET.iter().position(|&c| c == byte)? as u32;
        acc = acc << 6 | value;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((acc >> held) as u8);
            acc &= (1 << held) - 1;
        }
    }
    Some(out)
}

pub(super) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Hex digits ("1FAEAA92…") as bytes; `None` for anything else.
fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |b: u8| (b as char).to_digit(16);
            Some((digit(pair[0])? << 4 | digit(pair[1])?) as u8)
        })
        .collect()
}
