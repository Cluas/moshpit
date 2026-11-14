//! A sprite as a PNG, scaled up by whole pixels per cell (nearest
//! neighbour, so it stays crisp: the platform scales an image smoothly,
//! which would blur pixel art). Indexed colour, one transparent index,
//! stored deflate: small enough for a 120 × 120 pose, and no dependency.

use crate::sprite::{Appearance, Sprite};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    b << 16 | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

impl Sprite {
    /// The sprite in `appearance`, `scale` pixels a cell, as PNG bytes.
    pub fn png(&self, appearance: Appearance, scale: u32) -> Vec<u8> {
        let scale = scale.max(1) as usize;
        let side = usize::from(self.side);
        let px = side * scale;
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&(px as u32).to_be_bytes());
        ihdr.extend_from_slice(&(px as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        // Index 0 is transparent; ink i is index i.
        let mut plte = vec![0, 0, 0];
        for ink in &self.palette {
            plte.extend_from_slice(&ink.get(appearance).to_be_bytes()[1..]);
        }
        chunk(&mut out, b"PLTE", &plte);
        chunk(&mut out, b"tRNS", &[0]);
        let mut raw = Vec::with_capacity(px * (px + 1));
        for y in 0..px {
            raw.push(0);
            let row = &self.cells[(y / scale) * side..(y / scale + 1) * side];
            for &c in row {
                raw.extend(std::iter::repeat_n(c, scale));
            }
        }
        let mut z = vec![0x78, 0x01];
        let mut blocks = raw.chunks(65_535).peekable();
        while let Some(block) = blocks.next() {
            z.push(u8::from(blocks.peek().is_none()));
            let len = block.len() as u16;
            z.extend_from_slice(&len.to_le_bytes());
            z.extend_from_slice(&(!len).to_le_bytes());
            z.extend_from_slice(block);
        }
        z.extend_from_slice(&adler32(&raw).to_be_bytes());
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// The PNG as a `data:` URI, for an image element.
    pub fn data_uri(&self, appearance: Appearance, scale: u32) -> String {
        use base64::Engine;
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(self.png(appearance, scale))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite::Ink;

    #[test]
    fn a_png_has_its_chunks_their_checksums_and_its_pixels() {
        let s = Sprite {
            side: 2,
            cells: vec![0, 1, 1, 0],
            palette: vec![Ink {
                light: 0x112233,
                dark: 0x445566,
            }],
        };
        let png = s.png(Appearance::Dark, 3);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            6,
            "2 cells x 3 pixels"
        );
        // Every chunk's CRC checks out, and the palette is the dark one.
        let mut at = 8;
        let mut kinds = Vec::new();
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let body = &png[at + 4..at + 8 + len];
            let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
            assert_eq!(crc32(body), crc);
            kinds.push(String::from_utf8_lossy(&body[..4]).into_owned());
            if &body[..4] == b"PLTE" {
                assert_eq!(&body[4..], &[0, 0, 0, 0x44, 0x55, 0x66]);
            }
            if &body[..4] == b"IDAT" {
                let raw = &body[4 + 2 + 5..body.len() - 4];
                assert_eq!(raw.len(), 6 * 7);
                assert_eq!(
                    &raw[..7],
                    &[0, 0, 0, 0, 1, 1, 1],
                    "a filter byte, then cell 0 then cell 1, three pixels each"
                );
            }
            at += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "PLTE", "tRNS", "IDAT", "IEND"]);
        assert!(
            s.data_uri(Appearance::Light, 1)
                .starts_with("data:image/png;base64,iVBORw0KGgo")
        );
    }
}
