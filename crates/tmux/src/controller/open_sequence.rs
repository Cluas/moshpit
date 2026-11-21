//! The sequence a pane's live output left unfinished.
//!
//! tmux cuts a pane's output into `%output` lines wherever its pty reads
//! end, so an escape sequence (or a UTF-8 character) often spans two of
//! them: `…ESC[2` then `0;1H…`. Fed straight through, the terminal's
//! parser carries the half sequence from one feed to the next. Bytes the
//! controller writes into the stream itself — a repair frame, the cursor
//! after it, a backfill dump — land in the middle and end it: the second
//! half then reads as text (`0;1H` printed after a full row, the row it
//! moved to drawn in the wrong place: the 02-pane screenshot's garble).
//! tmux's own parser still holds the half sequence when it answers the
//! capture, so the frame knows nothing of it either.
//!
//! So the controller keeps the unfinished sequence's bytes, cancels it
//! (CAN) before its own bytes go in, and replays it ahead of the live
//! output that finishes it. The terminal then reads the sequence whole, as
//! tmux did.

/// Longer unfinished sequences (an image's DCS, a huge OSC 52) are not
/// kept; the replay is given up for them, as before.
const MAX_KEPT: usize = 4096;

/// CAN: ends any sequence in progress, printing nothing.
const CANCEL: u8 = 0x18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    /// `ESC`, maybe with intermediates.
    Escape,
    /// `ESC [` … up to the final byte.
    Csi,
    /// OSC, DCS, SOS, PM, APC: up to BEL or ST.
    Text,
    /// `ESC` inside one of those: ST if `\` follows.
    TextEscape,
    /// A UTF-8 character with this many continuation bytes still to come.
    Utf8(u8),
}

#[derive(Debug, Default)]
pub(super) struct OpenSequence {
    state: State,
    /// The unfinished sequence's bytes, from its first.
    kept: Vec<u8>,
    /// Too long to keep: nothing to replay.
    overflowed: bool,
    /// The terminal's parser was cut off from `kept` by the controller's
    /// own bytes: replay it ahead of the next live output.
    owed: bool,
}

impl OpenSequence {
    /// A pane's live output on its way to the terminal. When owed: a CAN
    /// (whatever the terminal was left in the middle of must not swallow
    /// what follows), the unfinished sequence, then the output.
    pub(super) fn live(&mut self, data: Vec<u8>) -> Vec<u8> {
        let fresh = data.len();
        let out = if std::mem::take(&mut self.owed) {
            let mut out = Vec::with_capacity(data.len() + self.kept.len() + 1);
            out.push(CANCEL);
            if self.state != State::Ground && !self.overflowed {
                out.extend_from_slice(&self.kept);
            }
            out.extend_from_slice(&data);
            out
        } else {
            data
        };
        // Only the new bytes move the state on: the replayed ones were
        // scanned when they first came.
        self.scan(&out[out.len() - fresh..]);
        out
    }

    /// A pane's live output the terminal never sees (held while another
    /// client owns the size): it still moves the sequence on, and the
    /// terminal is owed whatever it leaves open.
    pub(super) fn dropped(&mut self, data: &[u8]) {
        // Open before: the terminal is still in it, and what follows must
        // not continue it. Open after: the terminal never saw its start.
        let open_before = self.state != State::Ground;
        self.scan(data);
        self.owed |= open_before || self.state != State::Ground;
    }

    /// The controller's own bytes for the pane's terminal: a sequence left
    /// open is cancelled first (or the injection's first bytes would finish
    /// it) and owed to the live output after.
    pub(super) fn inject(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        if self.state == State::Ground && !self.owed {
            return bytes;
        }
        self.owed = true;
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(CANCEL);
        out.extend_from_slice(&bytes);
        out
    }

    fn scan(&mut self, data: &[u8]) {
        for &byte in data {
            self.step(byte);
        }
    }

    fn start(&mut self, state: State, byte: u8) {
        self.state = state;
        self.kept.clear();
        self.overflowed = false;
        self.keep(byte);
    }

    fn keep(&mut self, byte: u8) {
        if self.overflowed {
            return;
        }
        if self.kept.len() >= MAX_KEPT {
            self.overflowed = true;
            self.kept = Vec::new();
            return;
        }
        self.kept.push(byte);
    }

    fn finish(&mut self) {
        self.state = State::Ground;
        self.kept.clear();
        self.overflowed = false;
    }

    fn step(&mut self, byte: u8) {
        // Anywhere: CAN and SUB cancel, ESC starts over.
        if byte == CANCEL || byte == 0x1a {
            self.finish();
            return;
        }
        match self.state {
            State::Ground => self.ground(byte),
            State::Utf8(left) => {
                if (0x80..=0xbf).contains(&byte) {
                    if left <= 1 {
                        self.finish();
                    } else {
                        self.state = State::Utf8(left - 1);
                        self.keep(byte);
                    }
                } else {
                    self.finish();
                    self.ground(byte);
                }
            }
            State::Escape => match byte {
                0x1b => self.start(State::Escape, byte),
                b'[' => {
                    self.state = State::Csi;
                    self.keep(byte);
                }
                b']' | b'P' | b'X' | b'^' | b'_' => {
                    self.state = State::Text;
                    self.keep(byte);
                }
                0x20..=0x2f => self.keep(byte),
                // A C0 control is executed at once, not part of the
                // sequence: replaying it would run it twice.
                0x00..=0x1f => {}
                _ => self.finish(),
            },
            State::Csi => match byte {
                0x1b => self.start(State::Escape, byte),
                0x40..=0x7e => self.finish(),
                0x00..=0x1f => {}
                _ => self.keep(byte),
            },
            State::Text => match byte {
                0x07 => self.finish(),
                0x1b => {
                    self.state = State::TextEscape;
                    self.keep(byte);
                }
                _ => self.keep(byte),
            },
            State::TextEscape => {
                if byte == b'\\' {
                    self.finish();
                } else {
                    // The ESC ended the string and starts a sequence of
                    // its own.
                    self.start(State::Escape, 0x1b);
                    self.step(byte);
                }
            }
        }
    }

    fn ground(&mut self, byte: u8) {
        match byte {
            0x1b => self.start(State::Escape, byte),
            0xc2..=0xdf => self.start(State::Utf8(1), byte),
            0xe0..=0xef => self.start(State::Utf8(2), byte),
            0xf0..=0xf4 => self.start(State::Utf8(3), byte),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(chunks: &[&[u8]]) -> OpenSequence {
        let mut open = OpenSequence::default();
        for chunk in chunks {
            open.live(chunk.to_vec());
        }
        open
    }

    #[test]
    fn a_split_csi_is_replayed_after_an_injection() {
        let mut open = fed(&[b"text\x1b[2"]);
        assert_eq!(open.inject(b"\x1b[H".to_vec()), b"\x18\x1b[H");
        assert_eq!(open.live(b"0;1Hrow".to_vec()), b"\x18\x1b[20;1Hrow");
        // Finished: nothing owed or kept any more.
        assert_eq!(open.inject(b"\x1b[H".to_vec()), b"\x1b[H");
        assert_eq!(open.live(b"more".to_vec()), b"more");
    }

    #[test]
    fn finished_sequences_leave_nothing_to_replay() {
        let mut open = fed(&[
            b"\x1b[1;2H",
            b"\x1b]0;title\x07",
            b"\x1bP1$r\x1b\\",
            b"\x1b(B",
            "é".as_bytes(),
        ]);
        assert_eq!(open.inject(b"F".to_vec()), b"F");
        assert_eq!(open.live(b"x".to_vec()), b"x");
    }

    #[test]
    fn a_sequence_split_three_ways_is_kept_whole() {
        let mut open = fed(&[b"\x1b", b"[3", b"8;5"]);
        open.inject(b"\x1b[H".to_vec());
        assert_eq!(open.live(b";1m".to_vec()), b"\x18\x1b[38;5;1m");
    }

    #[test]
    fn a_split_utf8_character_is_replayed() {
        let bytes = "─".as_bytes();
        let mut open = fed(&[&bytes[..1]]);
        open.inject(b"\x1b[H".to_vec());
        assert_eq!(open.live(bytes[1..].to_vec()), [&[CANCEL], bytes].concat());
    }

    #[test]
    fn a_c0_control_inside_a_csi_is_not_replayed() {
        let mut open = fed(&[b"\x1b[2\r"]);
        open.inject(b"\x1b[H".to_vec());
        assert_eq!(open.live(b"J".to_vec()), b"\x18\x1b[2J");
    }

    #[test]
    fn an_esc_ending_a_string_starts_a_new_sequence() {
        let mut open = fed(&[b"\x1b]0;t\x1b["]);
        open.inject(b"\x1b[H".to_vec());
        assert_eq!(open.live(b"1m".to_vec()), b"\x18\x1b[1m");
    }

    #[test]
    fn an_overlong_sequence_is_not_replayed() {
        let mut long = b"\x1bP".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_KEPT + 10));
        let mut open = fed(&[&long]);
        assert_eq!(open.inject(b"\x1b[H".to_vec()), b"\x18\x1b[H");
        assert_eq!(open.live(b"b\x1b\\".to_vec()), b"\x18b\x1b\\");
    }

    #[test]
    fn whole_sequences_dropped_owe_nothing() {
        let mut open = OpenSequence::default();
        open.dropped(b"held\x1b]0;t\x07");
        assert_eq!(open.inject(b"\x1b[H".to_vec()), b"\x1b[H");
        assert_eq!(open.live(b"x".to_vec()), b"x");
    }

    #[test]
    fn a_sequence_finished_while_dropped_is_cancelled_in_the_terminal() {
        let mut open = fed(&[b"\x1b[3"]);
        open.dropped(b"1mheld");
        assert_eq!(open.live(b"x".to_vec()), b"\x18x");
    }

    #[test]
    fn output_dropped_mid_sequence_owes_its_tail() {
        let mut open = OpenSequence::default();
        open.dropped(b"held\x1b[2");
        assert_eq!(open.live(b"0;1H".to_vec()), b"\x18\x1b[20;1H");
    }
}
