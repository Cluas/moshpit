//! Real bells in a byte stream, without parsing the rest of it
//! (`BellDetector.swift`).
//!
//! While a window's pin is handed back (or the window follows another
//! device's size) its pane output is not painted, so no terminal parser is
//! there to raise the bell — and the bell is the "an agent needs you" signal.
//! A bare `0x07` scan is wrong: OSC strings end with BEL, and agents set the
//! title on every redraw. So this tracks whether the stream is inside an
//! OSC/DCS/APC/PM/SOS string (`ESC ]`, `P`, `_`, `^`, `X` … BEL or `ESC \`)
//! and reports a BEL only outside one. State carries across chunks.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BellDetector {
    in_string: bool,
    after_escape: bool,
}

impl BellDetector {
    /// Feed a chunk; true if it held at least one real bell.
    pub fn contains_bell(&mut self, data: &[u8]) -> bool {
        let mut rang = false;
        for &byte in data {
            if self.after_escape {
                self.after_escape = false;
                match byte {
                    0x5D | 0x50 | 0x5F | 0x5E | 0x58 => self.in_string = true,
                    0x5C => self.in_string = false,
                    _ => {}
                }
                continue;
            }
            match byte {
                0x1B => self.after_escape = true,
                0x07 => {
                    if self.in_string {
                        self.in_string = false;
                    } else {
                        rang = true;
                    }
                }
                _ => {}
            }
        }
        rang
    }

    /// Forget any half-read sequence.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_not_a_bell() {
        let mut detector = BellDetector::default();
        assert!(!detector.contains_bell(b"\x1b]0;title\x07"));
        assert!(detector.contains_bell(b"\x07"));
        // Split across chunks.
        assert!(!detector.contains_bell(b"\x1b]0;ti"));
        assert!(!detector.contains_bell(b"tle\x07"));
        assert!(!detector.contains_bell(b"\x1bP"));
        assert!(!detector.contains_bell(b"x\x1b\\"));
        assert!(detector.contains_bell(b"a\x07b"));
    }
}
