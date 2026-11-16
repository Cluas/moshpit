//! What a view sends the remote for a key or the mouse, encoded by the
//! emulator's own encoders, so every platform sends the same bytes for the
//! same press: wezterm's xterm encoder (DECCKM, `modifyOtherKeys`, CSI u)
//! and, while a program has the kitty keyboard protocol on, wezterm's kitty
//! encoder (press, repeat and release, alternate keys, associated text).
//!
//! The view maps its platform's key to a [`Key`] (the physical key's
//! meaning, not its layout) and passes the modifiers held; the bytes come
//! back for the view to send on its input path, so a key goes out in the
//! order it was pressed with everything else the view sends.

use termwiz::input::KeyCode as TermKey;
use wezterm_input_types::{
    KeyCode as KittyKey, KeyEvent, KeyboardLedStatus, KittyKeyboardFlags, Modifiers,
};
use wezterm_term::{MouseButton, MouseEvent, MouseEventKind};

/// A key, as the C ABI names it (`MP_KEY_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// The text a key makes (unshifted for a letter with Control held).
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1–F24.
    Function(u8),
    /// The modifier keys themselves: only the kitty protocol with "report
    /// all keys" (flag 8) sends anything for them.
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    LeftSuper,
    RightSuper,
    CapsLock,
}

/// `MP_KEY_*` codes.
pub mod codes {
    pub const CHAR: u32 = 0;
    pub const ENTER: u32 = 1;
    pub const TAB: u32 = 2;
    pub const BACKSPACE: u32 = 3;
    pub const ESCAPE: u32 = 4;
    pub const UP: u32 = 5;
    pub const DOWN: u32 = 6;
    pub const LEFT: u32 = 7;
    pub const RIGHT: u32 = 8;
    pub const HOME: u32 = 9;
    pub const END: u32 = 10;
    pub const PAGE_UP: u32 = 11;
    pub const PAGE_DOWN: u32 = 12;
    pub const INSERT: u32 = 13;
    pub const DELETE: u32 = 14;
    pub const LEFT_SHIFT: u32 = 15;
    pub const RIGHT_SHIFT: u32 = 16;
    pub const LEFT_CONTROL: u32 = 17;
    pub const RIGHT_CONTROL: u32 = 18;
    pub const LEFT_ALT: u32 = 19;
    pub const RIGHT_ALT: u32 = 20;
    pub const LEFT_SUPER: u32 = 21;
    pub const RIGHT_SUPER: u32 = 22;
    pub const CAPS_LOCK: u32 = 23;
    /// `FUNCTION + n` is Fn (1–24).
    pub const FUNCTION: u32 = 0x100;
}

/// `MP_MOD_*` bits.
pub mod mods {
    pub const SHIFT: u8 = 1;
    pub const ALT: u8 = 1 << 1;
    pub const CTRL: u8 = 1 << 2;
    pub const SUPER: u8 = 1 << 3;
    pub const CAPS_LOCK: u8 = 1 << 4;
}

impl Key {
    /// The key a C code names; `None` for an unknown code or a bad scalar.
    pub fn from_code(code: u32, scalar: u32) -> Option<Key> {
        use codes as C;
        Some(match code {
            C::CHAR => Key::Char(char::from_u32(scalar)?),
            C::ENTER => Key::Enter,
            C::TAB => Key::Tab,
            C::BACKSPACE => Key::Backspace,
            C::ESCAPE => Key::Escape,
            C::UP => Key::Up,
            C::DOWN => Key::Down,
            C::LEFT => Key::Left,
            C::RIGHT => Key::Right,
            C::HOME => Key::Home,
            C::END => Key::End,
            C::PAGE_UP => Key::PageUp,
            C::PAGE_DOWN => Key::PageDown,
            C::INSERT => Key::Insert,
            C::DELETE => Key::Delete,
            C::LEFT_SHIFT => Key::LeftShift,
            C::RIGHT_SHIFT => Key::RightShift,
            C::LEFT_CONTROL => Key::LeftControl,
            C::RIGHT_CONTROL => Key::RightControl,
            C::LEFT_ALT => Key::LeftAlt,
            C::RIGHT_ALT => Key::RightAlt,
            C::LEFT_SUPER => Key::LeftSuper,
            C::RIGHT_SUPER => Key::RightSuper,
            C::CAPS_LOCK => Key::CapsLock,
            n if (C::FUNCTION + 1..=C::FUNCTION + 24).contains(&n) => {
                Key::Function((n - C::FUNCTION) as u8)
            }
            _ => return None,
        })
    }

    fn is_modifier(self) -> bool {
        matches!(
            self,
            Key::LeftShift
                | Key::RightShift
                | Key::LeftControl
                | Key::RightControl
                | Key::LeftAlt
                | Key::RightAlt
                | Key::LeftSuper
                | Key::RightSuper
                | Key::CapsLock
        )
    }

    /// The xterm encoder's key; `None` for the modifier keys, which send
    /// nothing outside the kitty protocol.
    fn term_key(self) -> Option<TermKey> {
        Some(match self {
            Key::Char(c) => TermKey::Char(c),
            Key::Enter => TermKey::Enter,
            Key::Tab => TermKey::Tab,
            Key::Backspace => TermKey::Backspace,
            Key::Escape => TermKey::Escape,
            Key::Up => TermKey::UpArrow,
            Key::Down => TermKey::DownArrow,
            Key::Left => TermKey::LeftArrow,
            Key::Right => TermKey::RightArrow,
            Key::Home => TermKey::Home,
            Key::End => TermKey::End,
            Key::PageUp => TermKey::PageUp,
            Key::PageDown => TermKey::PageDown,
            Key::Insert => TermKey::Insert,
            Key::Delete => TermKey::Delete,
            Key::Function(n) => TermKey::Function(n),
            _ => return None,
        })
    }

    /// The kitty encoder's key (it spells Enter, Tab, Backspace, Escape and
    /// Delete as the characters they send).
    fn kitty_key(self) -> KittyKey {
        match self {
            Key::Char(c) => KittyKey::Char(c),
            Key::Enter => KittyKey::Char('\r'),
            Key::Tab => KittyKey::Char('\t'),
            Key::Backspace => KittyKey::Char('\x08'),
            Key::Escape => KittyKey::Char('\x1b'),
            Key::Up => KittyKey::UpArrow,
            Key::Down => KittyKey::DownArrow,
            Key::Left => KittyKey::LeftArrow,
            Key::Right => KittyKey::RightArrow,
            Key::Home => KittyKey::Home,
            Key::End => KittyKey::End,
            Key::PageUp => KittyKey::PageUp,
            Key::PageDown => KittyKey::PageDown,
            Key::Insert => KittyKey::Insert,
            Key::Delete => KittyKey::Char('\x7f'),
            Key::Function(n) => KittyKey::Function(n),
            Key::LeftShift => KittyKey::LeftShift,
            Key::RightShift => KittyKey::RightShift,
            Key::LeftControl => KittyKey::LeftControl,
            Key::RightControl => KittyKey::RightControl,
            Key::LeftAlt => KittyKey::LeftAlt,
            Key::RightAlt => KittyKey::RightAlt,
            Key::LeftSuper => KittyKey::LeftWindows,
            Key::RightSuper => KittyKey::RightWindows,
            Key::CapsLock => KittyKey::CapsLock,
        }
    }
}

fn modifiers(bits: u8) -> Modifiers {
    let mut m = Modifiers::NONE;
    if bits & mods::SHIFT != 0 {
        m |= Modifiers::SHIFT;
    }
    if bits & mods::ALT != 0 {
        m |= Modifiers::ALT;
    }
    if bits & mods::CTRL != 0 {
        m |= Modifiers::CTRL;
    }
    if bits & mods::SUPER != 0 {
        m |= Modifiers::SUPER;
    }
    m
}

/// A key's bytes under the kitty keyboard protocol (`flags`, the program's
/// `CSI > n u`): presses, repeats (`repeat` > 1, sent as presses unless the
/// program asked for event types) and releases (only with event types on).
pub(crate) fn kitty(key: Key, mods: u8, down: bool, repeat: u16, flags: u8) -> Vec<u8> {
    let flags = KittyKeyboardFlags::from_bits_truncate(flags as u16);
    if key.is_modifier() {
        return kitty_modifier(key, mods, down, repeat, flags);
    }
    let mut leds = KeyboardLedStatus::empty();
    if mods & mods::CAPS_LOCK != 0 {
        leds |= KeyboardLedStatus::CAPS_LOCK;
    }
    let event = KeyEvent {
        key: key.kitty_key(),
        modifiers: modifiers(mods),
        leds,
        repeat_count: repeat.max(1),
        key_is_down: down,
        raw: None,
    };
    let mut s = event.encode_kitty(flags);
    // A repeat is a press with event type 2 when the program asked for
    // event types (kitty's `CSI … ; mods:2 u`); wezterm's encoder knows
    // presses and releases only.
    if down && repeat > 1 && flags.contains(KittyKeyboardFlags::REPORT_EVENT_TYPES) {
        s = mark_repeat(&s);
    }
    s.into_bytes()
}

/// A modifier key under the kitty protocol: an escape code of its own
/// (kitty's functional key numbers) with "report all keys" on, nothing
/// otherwise. wezterm's encoder finds these numbers only from a platform's
/// raw key event, which a view here does not have.
fn kitty_modifier(
    key: Key,
    mods: u8,
    down: bool,
    repeat: u16,
    flags: KittyKeyboardFlags,
) -> Vec<u8> {
    if !flags.contains(KittyKeyboardFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES) {
        return Vec::new();
    }
    let events = flags.contains(KittyKeyboardFlags::REPORT_EVENT_TYPES);
    if !down && !events {
        return Vec::new();
    }
    let code = match key {
        Key::CapsLock => 57358,
        Key::LeftShift => 57441,
        Key::LeftControl => 57442,
        Key::LeftAlt => 57443,
        Key::LeftSuper => 57444,
        Key::RightShift => 57447,
        Key::RightControl => 57448,
        Key::RightAlt => 57449,
        Key::RightSuper => 57450,
        _ => return Vec::new(),
    };
    let mut m = 1u32;
    for (bit, value) in [
        (mods::SHIFT, 1),
        (mods::ALT, 2),
        (mods::CTRL, 4),
        (mods::SUPER, 8),
        (mods::CAPS_LOCK, 64),
    ] {
        if mods & bit != 0 {
            m += value;
        }
    }
    let event = match (down, repeat > 1, events) {
        (false, _, _) => ":3",
        (true, true, true) => ":2",
        _ => "",
    };
    format!("\x1b[{code};{m}{event}u").into_bytes()
}

/// Turns an encoded press (`CSI params final`) into a repeat: the
/// modifiers' parameter gets `:2`. A press sent as plain text has no event
/// type to carry and stays as it is.
fn mark_repeat(s: &str) -> String {
    let Some(body) = s.strip_prefix("\x1b[") else {
        return s.to_string();
    };
    let Some(last) = body.chars().last() else {
        return s.to_string();
    };
    let params = &body[..body.len() - last.len_utf8()];
    let mut fields: Vec<String> = params.split(';').map(str::to_string).collect();
    if fields.len() < 2 {
        fields.resize(2, String::new());
    }
    let m = &mut fields[1];
    let (value, rest) = match m.find(':') {
        Some(i) => (m[..i].to_string(), m[i..].to_string()),
        None => (m.clone(), String::new()),
    };
    if !rest.is_empty() {
        return s.to_string();
    }
    let value = if value.is_empty() { "1".into() } else { value };
    *m = format!("{value}:2");
    let mut params = fields.join(";");
    // `1;1:2` with nothing else is still the full form kitty expects.
    if params.ends_with(';') {
        params.pop();
    }
    format!("\x1b[{params}{last}")
}

/// The xterm encoder's key and modifiers (`None`: nothing to send).
pub(crate) fn legacy(key: Key, mods: u8) -> Option<(TermKey, Modifiers)> {
    Some((key.term_key()?, modifiers(mods)))
}

/// A mouse report the C ABI names: `kind` 0 press, 1 release, 2 motion;
/// `button` 0 left, 1 middle, 2 right, 3 none, 4 wheel up, 5 wheel down,
/// 6 wheel left, 7 wheel right.
pub(crate) fn mouse_event(kind: u8, button: u8, row: u16, col: u16, mods: u8) -> MouseEvent {
    MouseEvent {
        kind: match kind {
            0 => MouseEventKind::Press,
            1 => MouseEventKind::Release,
            _ => MouseEventKind::Move,
        },
        x: col as usize,
        y: row as i64,
        x_pixel_offset: 0,
        y_pixel_offset: 0,
        button: match button {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            4 => MouseButton::WheelUp(1),
            5 => MouseButton::WheelDown(1),
            6 => MouseButton::WheelLeft(1),
            7 => MouseButton::WheelRight(1),
            _ => MouseButton::None,
        },
        modifiers: modifiers(mods),
    }
}

/// `ESC [ 2 0 0 ~` and `ESC [ 2 0 1 ~`.
pub const PASTE_START: &[u8] = b"\x1b[200~";
pub const PASTE_END: &[u8] = b"\x1b[201~";

/// Pasted text with every bracketed-paste marker taken out, as bytes and
/// until none is left (the app's round-nineteen sanitizer,
/// `BracketedPaste.body`): a program ends a paste at the first end marker
/// it reads, so one inside the text would end it early and run what
/// follows as typed keys. Removing a marker the moment its last byte is
/// read gets there in one pass: the bytes kept never hold a marker, so the
/// only one a new byte can complete ends at that byte.
///
/// No control character goes out either but tab, line feed and carriage
/// return (xterm's default; B738): an escape sequence in a paste ran as
/// keys where the program hadn't asked for bracketed paste, and a ^C or ^D
/// reached the tty as an interrupt or an end of file, bracketed or not.
/// C1 controls and DEL go too; with no ESC left, no marker can be either.
pub fn paste_body(text: &str) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::with_capacity(text.len());
    for &b in text.as_bytes() {
        kept.push(b);
        if b == b'~' && (kept.ends_with(PASTE_START) || kept.ends_with(PASTE_END)) {
            kept.truncate(kept.len() - PASTE_END.len());
        }
    }
    String::from_utf8(kept)
        .expect("removing ASCII markers keeps UTF-8 valid")
        .chars()
        .filter(|&c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .into_bytes()
}

/// What a paste sends: one bracketed paste when the program asked for
/// them (or the host says the remote end re-encodes one, `force`), byte for
/// byte; the text as typed otherwise, its line breaks as Return
/// ([`returns`]); never a marker inside.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let body = paste_body(text);
    if body.is_empty() {
        return body;
    }
    if !bracketed {
        return returns(&body);
    }
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(PASTE_START);
    out.extend_from_slice(&body);
    out.extend_from_slice(PASTE_END);
    out
}

/// A paste's line breaks as Return, as xterm, iTerm and Terminal.app send
/// a paste a program didn't ask to have bracketed (B574): CRLF and a lone
/// LF as CR. Sent as LF, a CRLF text doubled every line break in a program
/// that reads the tty raw or through ICRNL.
pub fn returns(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut after_cr = false;
    for &b in body {
        match b {
            b'\n' if after_cr => {}
            b'\n' => out.push(b'\r'),
            _ => out.push(b),
        }
        after_cr = b == b'\r';
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_never_carries_a_marker() {
        assert_eq!(
            paste("ls\nrm -rf x", true),
            b"\x1b[200~ls\nrm -rf x\x1b[201~"
        );
        assert_eq!(paste("a\x1b[201~b", true), b"\x1b[200~ab\x1b[201~");
        // Taking one out joins bytes that form another: gone too.
        assert_eq!(paste_body("\x1b[20\x1b[201~1~x"), b"x");
        assert_eq!(paste("plain", false), b"plain");
        assert_eq!(paste("\x1b[200~", true), b"", "nothing left: nothing sent");
    }

    /// The Swift `PastePathTests` cases, kept when they went: a marker with
    /// a combining mark after its `~` (one Character to Swift, bytes here)
    /// goes, the mark stays; markers leave an unbracketed paste too, whose
    /// breaks then go as Return (B574).
    #[test]
    fn markers_leave_whatever_follows_them_and_whatever_the_mode() {
        let combining = "ls\x1b[201~\u{301}\ntouch INJECTED\n";
        let assembled = "\x1b[20\x1b[201~1~\ntouch INJECTED\n";
        let bare = "ls\x1b[201~\ntouch INJECTED\n";
        let bracketed = |body: &str| [PASTE_START, body.as_bytes(), PASTE_END].concat();
        assert_eq!(
            paste(combining, true),
            bracketed("ls\u{301}\ntouch INJECTED\n")
        );
        assert_eq!(paste(assembled, true), bracketed("\ntouch INJECTED\n"));
        assert_eq!(paste(bare, true), bracketed("ls\ntouch INJECTED\n"));
        assert_eq!(
            paste(combining, false),
            "ls\u{301}\rtouch INJECTED\r".as_bytes()
        );
        assert_eq!(paste(assembled, false), b"\rtouch INJECTED\r");
        assert_eq!(paste(bare, false), b"ls\rtouch INJECTED\r");
    }

    /// The Swift `BracketedPasteTests` cases, kept when the Swift copy went
    /// (E7c): the app's every paste now goes through [`paste`]. One case
    /// changed on purpose: an empty paste sends nothing, never two bare
    /// markers.
    #[test]
    fn the_swift_suites_cases() {
        let start = "\x1b[200~";
        let end = "\x1b[201~";
        let body = |text: &str| String::from_utf8(paste_body(text)).unwrap();
        let wrapped = |body: &str| [PASTE_START, body.as_bytes(), PASTE_END].concat();
        // Plain text passes through, wrapped once.
        assert_eq!(body("first line\nsecond line"), "first line\nsecond line");
        assert_eq!(paste("/tmp/shot.png", true), wrapped("/tmp/shot.png"));
        assert_eq!(paste("", true), b"", "an empty paste sends nothing");
        // Start and end markers come out.
        assert_eq!(body(&format!("one{end}two")), "onetwo");
        assert_eq!(body(&format!("one{start}two")), "onetwo");
        assert_eq!(body(&format!("{start}{end}{end}")), "");
        // A marker whose `~` carries a combining mark comes out; the mark stays.
        for mark in ["\u{301}", "\u{FE0F}", "\u{1F3FB}", "\u{200D}", "\u{20E3}"] {
            assert_eq!(body(&format!("a{end}{mark}b")), format!("a{mark}b"));
            assert_eq!(body(&format!("a{start}{mark}b")), format!("a{mark}b"));
        }
        // A marker assembled by taking another out comes out too, across kinds.
        assert_eq!(body(&format!("a\x1b[20{end}1~b")), "ab");
        assert_eq!(body(&format!("\x1b[20\x1b[20{end}1~1~")), "");
        assert_eq!(body(&format!("\x1b[20{end}0~x")), "x");
        assert_eq!(body(&format!("\x1b{start}[201~")), "");
        // Bytes that only look like a marker stay, but for the ESC.
        for kept in [
            "\x1b[201",
            "\x1b[202~",
            "[201~",
            "\x1b[2O1~",
            "\x1b\x1b",
            "\x1b[200",
            "~\x1b[20 1~",
            "\x1b[201\u{301}~",
        ] {
            assert_eq!(body(kept), kept.replace('\x1b', ""), "{kept:?}");
        }
        // B738: no control but tab and line breaks; C1 and DEL go too.
        assert_eq!(
            body("a\x1b:!rm -rf ~\u{03}\u{04}\u{7F}\u{9B}2J\tx\r\ny中文🎉"),
            "a:!rm -rf ~2J\tx\r\ny中文🎉"
        );
        assert_eq!(paste("x\x1b[A\u{03}\n", false), b"x[A\r");
        assert_eq!(paste("a\tb\r\nc\nd", true), wrapped("a\tb\r\nc\nd"));
        assert_eq!(paste("x\u{9B}201~y", true), wrapped("x201~y"));
        // B574: unbracketed, line breaks are Return; bracketed, as they are.
        assert_eq!(paste("a\r\nb\nc\rd", false), b"a\rb\rc\rd");
        assert_eq!(paste("\r\n\r\n", false), b"\r\r");
        assert_eq!(paste("中文 no breaks", false), "中文 no breaks".as_bytes());
        assert_eq!(paste("a\r\nb\nc", true), wrapped("a\r\nb\nc"));
    }

    /// Removal the slow, obvious way (every marker, pass after pass, then
    /// no ESC): what [`paste_body`] must agree with, on random pieces of
    /// markers (the Swift suite's 5 000, its seed).
    #[test]
    fn random_pieces_come_out_as_repeated_removal_says() {
        fn naive(text: &str) -> Vec<u8> {
            let mut bytes = text.as_bytes().to_vec();
            loop {
                let mut out = Vec::with_capacity(bytes.len());
                let mut i = 0;
                let mut removed = false;
                while i < bytes.len() {
                    let rest = &bytes[i..];
                    if rest.starts_with(PASTE_START) || rest.starts_with(PASTE_END) {
                        i += PASTE_END.len();
                        removed = true;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                bytes = out;
                if !removed {
                    return bytes.into_iter().filter(|&b| b != 0x1b).collect();
                }
            }
        }
        let pieces = [
            "\x1b[201~",
            "\x1b[200~",
            "\x1b[20",
            "\x1b[201",
            "\x1b[200",
            "1~",
            "0~",
            "~",
            "\x1b",
            "[",
            "2",
            "0",
            "1",
            "x",
            "\u{301}",
            "\n",
            "é",
        ];
        let mut state: u64 = 0x9A57_E019;
        let mut next = |n: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((state >> 33) as usize) % n
        };
        let count =
            |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).filter(|w| *w == needle).count();
        for _ in 0..5_000 {
            let len = next(15);
            let pasted: String = (0..len).map(|_| pieces[next(pieces.len())]).collect();
            let body = paste_body(&pasted);
            assert_eq!(body, naive(&pasted), "{pasted:?}");
            assert_eq!(count(&body, PASTE_END), 0, "{pasted:?}");
            assert_eq!(count(&body, PASTE_START), 0, "{pasted:?}");
            let sent = paste(&pasted, true);
            if !body.is_empty() {
                assert_eq!(count(&sent, PASTE_END), 1, "{pasted:?}");
                assert_eq!(count(&sent, PASTE_START), 1, "{pasted:?}");
            }
        }
    }

    #[test]
    fn a_repeat_carries_event_type_two() {
        assert_eq!(mark_repeat("\x1b[97;1u"), "\x1b[97;1:2u");
        assert_eq!(mark_repeat("\x1b[1;1A"), "\x1b[1;1:2A");
        assert_eq!(mark_repeat("\x1b[97u"), "\x1b[97;1:2u");
        assert_eq!(mark_repeat("a"), "a");
    }
}
