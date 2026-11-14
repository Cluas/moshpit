//! mosh's predictive echo as an overlay on the core's frames: the app's
//! `MoshPredictionTests` scenarios, the core playing the screen and the
//! test playing mosh-server (host bytes and the echo ack). The emulator's
//! own grid never holds a guess (the diff-peer contract).

mod common;
use common::*;
use moshpit_term::{
    Terminal, cell_flags as F,
    predict::{Event, InputScanner, PredictMode, predictable},
};
use std::collections::HashMap;

struct Rig {
    t: Terminal,
    next_state: u64,
    shown: HashMap<u16, String>,
    cursor: (u16, u16),
}

impl Rig {
    fn new(mode: PredictMode, srtt: f64) -> Rig {
        let mut t = term(20, 4, 0);
        t.feed(b"$ ");
        t.predict_attach(mode, false);
        t.predict_srtt(srtt);
        let mut rig = Rig {
            t,
            next_state: 1,
            shown: HashMap::new(),
            cursor: (0, 0),
        };
        rig.look();
        rig
    }

    /// Takes a frame and keeps every row's text as drawn.
    fn look(&mut self) {
        let f = self.t.frame();
        for r in &f.changed {
            let mut s = String::new();
            for c in &r.cells {
                if c.flags & F::SPACER != 0 {
                    continue;
                }
                s.push(char::from_u32(c.ch).unwrap_or(' '));
            }
            self.shown.insert(r.row, s.trim_end().to_string());
        }
        self.cursor = (f.cursor.row, f.cursor.col);
    }

    fn row(&mut self, r: u16) -> String {
        self.look();
        self.shown.get(&r).cloned().unwrap_or_default()
    }

    fn type_(&mut self, keys: &str) {
        if let Some(batch) = self.t.predict_input(keys.as_bytes()) {
            self.t.predict_sent(batch, Some(self.next_state));
        }
        self.next_state += 1;
    }

    fn host(&mut self, bytes: &str) {
        self.t.feed(bytes.as_bytes());
        self.t.predict_echo_ack(self.next_state - 1);
    }

    fn warm_up(&mut self, keys: &str) {
        self.type_(keys);
        self.host(keys);
    }
}

impl Rig {
    /// The cursor as drawn (row, col).
    fn drawn_cursor(&mut self) -> (u16, u16) {
        self.look();
        self.cursor
    }
}

#[test]
fn the_first_guess_waits_then_typing_shows_at_once() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.type_("a");
    assert_eq!(rig.row(0), "$", "an unconfirmed epoch shows nothing");
    rig.host("a");
    assert_eq!(rig.row(0), "$ a");
    rig.type_("b");
    assert_eq!(
        rig.row(0),
        "$ ab",
        "confirmed: the guess shows before the echo"
    );
    assert_eq!(rig.drawn_cursor(), (0, 4), "and the cursor after it");
    assert_eq!(
        rig.t.row_text(0).0,
        "$ a",
        "the emulator holds only what the server sent"
    );
    rig.host("b");
    assert_eq!(rig.row(0), "$ ab");
}

#[test]
fn a_wrong_guess_in_a_shown_epoch_drops_every_guess() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.warm_up("a");
    rig.type_("bc");
    assert_eq!(rig.row(0), "$ abc");
    rig.host("X");
    assert_eq!(
        rig.row(0),
        "$ aX",
        "the server's screen, no guess left on it"
    );
    rig.type_("d");
    assert_eq!(rig.row(0), "$ aX", "the next guesses wait again");
}

#[test]
fn a_prompt_that_does_not_echo_never_shows_a_character() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.host("\r\nPassword: ");
    rig.type_("secret");
    assert_eq!(rig.row(1), "Password:");
    rig.host("");
    assert_eq!(rig.row(1), "Password:");
    rig.type_("x");
    assert_eq!(rig.row(1), "Password:");
}

#[test]
fn backspace_pulls_the_rest_of_the_line_left() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.warm_up("abc");
    rig.type_("\x7f");
    assert_eq!(rig.row(0), "$ ab");
}

#[test]
fn after_return_nothing_is_guessed_until_the_server_places_the_cursor() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.warm_up("a");
    rig.type_("\r");
    rig.type_("x");
    assert_eq!(rig.row(1), "", "no guess on the next line");
    rig.host("\r\n$ ");
    rig.type_("y");
    rig.host("y");
    rig.type_("z");
    assert_eq!(rig.row(1), "$ yz");
}

#[test]
fn wide_characters_are_the_servers_to_draw() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.warm_up("a");
    rig.type_("中");
    assert_eq!(rig.row(0), "$ a");
    assert!(!predictable('中'));
    assert!(!predictable('\u{301}'));
    assert!(predictable('a'));
    assert!(predictable('é'));
}

#[test]
fn adaptive_hides_guesses_on_a_fast_link() {
    let mut rig = Rig::new(PredictMode::Adaptive, 10.0);
    rig.warm_up("a");
    rig.type_("b");
    assert_eq!(rig.row(0), "$ a", "a 10 ms link: no guesses");
    let mut rig = Rig::new(PredictMode::Adaptive, 500.0);
    rig.warm_up("a");
    rig.type_("b");
    assert_eq!(rig.row(0), "$ ab", "a 500 ms link: guesses");
}

#[test]
fn reading_history_starts_over() {
    let mut rig = Rig::new(PredictMode::Always, 500.0);
    rig.t.set_scrollback(100);
    rig.t.feed(
        (0..10)
            .map(|i| format!("{i}\r\n"))
            .collect::<String>()
            .as_bytes(),
    );
    rig.t.feed(b"$ ");
    rig.warm_up("a");
    rig.t.scroll(2);
    assert_eq!(
        rig.t.predict_input(b"b"),
        None,
        "no guesses into a scrolled view"
    );
}

#[test]
fn the_keystroke_scanner() {
    let mut s = InputScanner::default();
    assert_eq!(s.scan(b"a\x7f"), [Event::Print('a'), Event::Print('\x7f')]);
    assert_eq!(
        s.scan(b"\x1b[C\x1b[1;5D"),
        [
            Event::Csi {
                final_byte: b'C',
                plain: true
            },
            Event::Csi {
                final_byte: b'D',
                plain: false
            }
        ]
    );
    assert_eq!(
        s.scan(b"\x1bOA\r"),
        [
            Event::Csi {
                final_byte: b'A',
                plain: true
            },
            Event::Execute(13)
        ]
    );
    // A character split between sends.
    assert!(s.scan(&"é".as_bytes()[..1]).is_empty());
    assert_eq!(s.scan(&"é".as_bytes()[1..]), [Event::Print('é')]);
    assert_eq!(s.scan(b"\x1b]0;t\x07x"), [Event::String, Event::Print('x')]);
}
