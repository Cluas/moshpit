//! Colours: the theme's 16, xterm's 6×6×6 cube and grey ramp, and the
//! default foreground, background and cursor. A program's OSC 4/10/11
//! changes are the emulator's and win over these.

use wezterm_term::color::{ColorPalette, SrgbaTuple};

/// An sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub ansi: [Rgb; 16],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
}

const fn rgb(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

impl Default for Palette {
    /// xterm's colours on the app's dark background.
    fn default() -> Self {
        Self {
            ansi: [
                rgb(0x000000),
                rgb(0xcd0000),
                rgb(0x00cd00),
                rgb(0xcdcd00),
                rgb(0x0000ee),
                rgb(0xcd00cd),
                rgb(0x00cdcd),
                rgb(0xe5e5e5),
                rgb(0x7f7f7f),
                rgb(0xff0000),
                rgb(0x00ff00),
                rgb(0xffff00),
                rgb(0x5c5cff),
                rgb(0xff00ff),
                rgb(0x00ffff),
                rgb(0xffffff),
            ],
            foreground: rgb(0xe5e5e5),
            background: rgb(0x000000),
            cursor: rgb(0xe5e5e5),
        }
    }
}

fn tuple(c: Rgb) -> SrgbaTuple {
    SrgbaTuple(
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        1.0,
    )
}

impl Palette {
    /// Colour `i` of the 256.
    pub fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Rgb {
                    r: level(i / 36),
                    g: level(i / 6 % 6),
                    b: level(i % 6),
                }
            }
            _ => {
                let v = 8 + 10 * (i - 232);
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    /// The emulator's palette (what OSC 4/10/11 queries answer from).
    pub(crate) fn to_wezterm(self) -> ColorPalette {
        let mut p = ColorPalette::default();
        for i in 0..=255u8 {
            p.colors.0[i as usize] = tuple(self.indexed(i));
        }
        p.foreground = tuple(self.foreground);
        p.background = tuple(self.background);
        p.cursor_bg = tuple(self.cursor);
        p.cursor_border = tuple(self.cursor);
        p.cursor_fg = tuple(self.background);
        p
    }
}

pub(crate) fn from_tuple(t: SrgbaTuple) -> Rgb {
    let (r, g, b, _) = t.to_srgb_u8();
    Rgb { r, g, b }
}

/// SGR 2: two thirds of the colour.
pub fn dim(c: Rgb) -> Rgb {
    Rgb {
        r: (c.r as u16 * 2 / 3) as u8,
        g: (c.g as u16 * 2 / 3) as u8,
        b: (c.b as u16 * 2 / 3) as u8,
    }
}
