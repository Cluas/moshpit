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
pub fn paste_body(text: &str) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::with_capacity(text.len());
    for &b in text.as_bytes() {
        kept.push(b);
        if b == b'~' && (kept.ends_with(PASTE_START) || kept.ends_with(PASTE_END)) {
            kept.truncate(kept.len() - PASTE_END.len());
        }
    }
    kept
}

/// What a paste sends: one bracketed paste when the program asked for
/// them (or the host says the remote end re-encodes one, `force`), the
/// bare text otherwise; never a marker inside.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let body = paste_body(text);
    if body.is_empty() {
        return body;
    }
    if !bracketed {
        return body;
    }
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(PASTE_START);
    out.extend_from_slice(&body);
    out.extend_from_slice(PASTE_END);
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

    #[test]
    fn a_repeat_carries_event_type_two() {
        assert_eq!(mark_repeat("\x1b[97;1u"), "\x1b[97;1:2u");
        assert_eq!(mark_repeat("\x1b[1;1A"), "\x1b[1;1:2A");
        assert_eq!(mark_repeat("\x1b[97u"), "\x1b[97;1:2u");
        assert_eq!(mark_repeat("a"), "a");
    }
}
