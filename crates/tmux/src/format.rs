//! tmux's text formats as the controller reads and writes them: `-F` rows cut
//! into fields, ids, layout strings, the command-string literal a paste
//! travels in, and the width check every repair frame passes. Ports of the
//! pure statics of `TmuxSessionController.swift` and `TmuxFormatFields.swift`.
//!
//! Everything is cut and matched on bytes: tmux prints its protocol in ASCII,
//! and a field cut at an ASCII byte of valid UTF-8 is valid UTF-8 (the B17 /
//! B35 rule the Swift side follows too).

/// The fields of one line of `-F` output, cut at every `separator` byte —
/// `String.tmuxFields(separator:maxSplits:omittingEmptySubsequences:)`, which
/// is Swift's `split` over the line's UTF-8: with `omit_empty`, empty pieces
/// are dropped and don't count towards `max_splits`; after the last split the
/// rest of the line is one field, separators and all.
pub fn fields(text: &str, separator: u8, max_splits: usize, omit_empty: bool) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    if max_splits == 0 || bytes.is_empty() {
        if !(bytes.is_empty() && omit_empty) {
            parts.push(text);
        }
        return parts;
    }
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != separator {
            at += 1;
            continue;
        }
        let appended = !(omit_empty && start == at);
        if appended {
            parts.push(&text[start..at]);
        }
        at += 1;
        start = at;
        if appended && parts.len() == max_splits {
            break;
        }
    }
    if start != bytes.len() || !omit_empty {
        parts.push(&text[start..]);
    }
    parts
}

/// The end of a pane listing line, `#{pane_current_command}
/// #{pane_current_path}`: the command, and the path when there is one. Both
/// may hold spaces; the path is absolute, and a command (a process's name)
/// never holds a `/`, so the path starts at the first ` /`. Without one (an
/// empty path, or a line of the format before the path) the whole is the
/// command, less the separator an empty path leaves.
pub fn command_and_path(rest: &str) -> (&str, Option<&str>) {
    match rest.find(" /") {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest.strip_suffix(' ').unwrap_or(rest), None),
    }
}

/// [`fields`] with every split taken and empty fields dropped — the plain
/// `tmuxFields(separator:)`.
pub fn words(text: &str, separator: u8) -> Vec<&str> {
    fields(text, separator, usize::MAX, true)
}

/// `%<digits>`, `@<digits>` or `$<digits>`: the only shape an id may have to
/// be spliced, unquoted, into a `-t` argument (`isValidTmuxId`).
pub fn is_valid_id(id: &str) -> bool {
    match id.as_bytes() {
        [b'%' | b'@' | b'$', digits @ ..] => {
            !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

/// `Int(text)` in Swift: an optional sign and ASCII digits, within `Int64`.
pub fn swift_int(text: &str) -> Option<i64> {
    text.parse().ok()
}

/// `Double(text)` in Swift (`TimeInterval(text)`): what Rust's parser takes
/// (decimal, `inf`/`infinity`/`nan` in any case) plus C's hexadecimal
/// floats (`0x1.8p1`), which Swift accepts too. No surrounding whitespace.
pub fn swift_double(text: &str) -> Option<f64> {
    if let Ok(value) = text.parse::<f64>() {
        return Some(value);
    }
    hex_double(text)
}

fn hex_double(text: &str) -> Option<f64> {
    let (negative, rest) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    let rest = rest
        .strip_prefix("0x")
        .or_else(|| rest.strip_prefix("0X"))?;
    let (mantissa, exponent) = match rest.find(['p', 'P']) {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole
        .bytes()
        .chain(fraction.bytes())
        .all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let mut value = 0.0f64;
    for digit in whole.bytes() {
        value = value * 16.0 + f64::from(hex_value(digit));
    }
    let mut scale = 1.0 / 16.0;
    for digit in fraction.bytes() {
        value += f64::from(hex_value(digit)) * scale;
        scale /= 16.0;
    }
    if let Some(exponent) = exponent {
        let power: i32 = exponent.parse().ok()?;
        value *= 2f64.powi(power);
    }
    Some(if negative { -value } else { value })
}

fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => digit - b'A' + 10,
    }
}

/// The window's width in a `#{window_layout}` (`<sum>,<W>x<H>,<x>,<y>…`).
pub fn window_width(layout: &str) -> Option<i64> {
    let parts = words(layout, b',');
    if parts.len() < 2 {
        return None;
    }
    words(parts[1], b'x').first().and_then(|w| swift_int(w))
}

/// The window's size in a `#{window_layout}`: `(W, H)`, both positive.
pub fn window_size(layout: &str) -> Option<(i64, i64)> {
    let parts = words(layout, b',');
    let size = words(parts.get(1)?, b'x');
    match size[..] {
        [w, h] => match (swift_int(w), swift_int(h)) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        },
        _ => None,
    }
}

/// How many panes a layout describes: one per leaf `WxH,X,Y,ID` (the Swift
/// regex `[0-9]+x[0-9]+,[0-9]+,[0-9]+,[0-9]+`, matched left to right without
/// overlap); `None` when there is none. Layouts are ASCII.
pub fn pane_count(layout: &str) -> Option<i64> {
    let bytes = layout.as_bytes();
    let mut count = 0;
    let mut at = 0;
    while at < bytes.len() {
        match leaf_at(bytes, at) {
            Some(end) => {
                count += 1;
                at = end;
            }
            None => at += 1,
        }
    }
    (count > 0).then_some(count)
}

/// The end of a `WxH,X,Y,ID` match starting at `at`, if one does.
fn leaf_at(bytes: &[u8], at: usize) -> Option<usize> {
    let mut at = digits_end(bytes, at)?;
    for delimiter in *b"x,,," {
        if bytes.get(at) != Some(&delimiter) {
            return None;
        }
        at = digits_end(bytes, at + 1)?;
    }
    Some(at)
}

/// The end of the run of ASCII digits at `at`, which must hold one at least.
fn digits_end(bytes: &[u8], at: usize) -> Option<usize> {
    let run = bytes[at.min(bytes.len())..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    (run > 0).then_some(at + run)
}

/// Whether `layout` is a tmux layout string: four hex digits, a comma, then
/// digits and layout punctuation only (`isPlausibleLayout`): the gate that
/// keeps a surprise reply out of `select-layout`.
pub fn is_plausible_layout(layout: &str) -> bool {
    // Character counts in Swift; anything not ASCII fails the checks below,
    // so byte counts decide the same.
    if layout.len() <= 6 || layout.len() >= 4096 {
        return false;
    }
    let parts = fields(layout, b',', 1, true);
    if parts.len() != 2 || parts[0].len() != 4 || !parts[0].bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    parts[1]
        .bytes()
        .all(|b| b.is_ascii_digit() || b"x,[]{}".contains(&b))
}

/// `text` as a double-quoted tmux command-string literal, for a `-CC` line
/// (`tmuxCommandStringLiteral`): the control channel parses each line with
/// tmux's command grammar, which inside double quotes still expands `$VAR`
/// and a leading `~` and reads `\n \r \t \e \uXXXX \ooo`, so those and the
/// quote and backslash are escaped, and no raw control byte is left.
pub fn command_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for scalar in text.chars() {
        match scalar {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '$' => out.push_str("\\$"),
            '~' => out.push_str("\\~"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A session or window name as one word of a control-mode line
/// (`tmuxNameArgument`), for `new-session -s`, `new-window -n`,
/// `rename-session` and `rename-window`: a command-string literal, so no
/// line break can reach the line, with every `#` doubled, because tmux
/// expands the name as a format before it keeps it (`##` is a `#`). A
/// positional name goes after `--`, or one starting with `-` is a flag.
///
/// B139 (the Swift controller is fixed the same way): the name went out
/// single-quoted, as a shell word — a line break in it ended the command
/// and ran the rest as a tmux command of its own (`run-shell` included,
/// tmux 3.6a), and `#{session_name}` was stored as the session's name.
pub fn name_argument(name: &str) -> String {
    command_string_literal(&name.replace('#', "##"))
}

/// Why a typed name is not used (`MultiplexerName.refusal`): it holds a
/// control character — a line break, a tab — which is a paste accident, not
/// a name. The host shows it as the notice.
pub const NAME_REFUSAL: &str = "Names can't contain line breaks or other control characters.";

/// Whether [`NAME_REFUSAL`] applies to `name`.
pub fn refuses_name(name: &str) -> bool {
    name.chars().any(char::is_control)
}

/// Whether any `capture-pane -e` line is laid out for a grid wider than
/// `cols` (`frameExceedsWidth`): the tell of a frame captured at another
/// client's size. Width is estimated so that it never overestimates (see
/// [`estimated_columns`]); escape sequences count nothing, and an OSC or DCS
/// payload — a hyperlink's URL — is skipped to its terminator.
pub fn frame_exceeds_width<S: AsRef<str>>(lines: &[S], cols: i64) -> bool {
    for line in lines {
        let mut width: i64 = 0;
        let mut scalars = line.as_ref().chars().peekable();
        while let Some(scalar) = scalars.next() {
            if scalar == '\u{1b}' {
                let Some(&intro) = scalars.peek() else {
                    continue;
                };
                if intro == '[' {
                    scalars.next();
                    while scalars
                        .peek()
                        .is_some_and(|p| (0x20..=0x3f).contains(&(*p as u32)))
                    {
                        scalars.next();
                    }
                    scalars.next();
                } else if intro == ']' || intro == 'P' {
                    scalars.next();
                    while let Some(p) = scalars.next() {
                        if p == '\u{7}' {
                            break;
                        }
                        if p == '\u{1b}' {
                            if scalars.peek() == Some(&'\\') {
                                scalars.next();
                            }
                            break;
                        }
                    }
                } else {
                    while scalars
                        .peek()
                        .is_some_and(|p| (0x20..=0x2f).contains(&(*p as u32)))
                    {
                        scalars.next();
                    }
                    scalars.next();
                }
                continue;
            }
            width += estimated_columns(scalar);
            if width > cols {
                return true;
            }
        }
    }
    false
}

/// A conservative `wcwidth`: 0 for combining and zero-width marks, 2 only for
/// unambiguously wide ranges, else 1 (`estimatedColumns`).
pub fn estimated_columns(scalar: char) -> i64 {
    match scalar as u32 {
        0x0300..=0x036f | 0x200b..=0x200f | 0xfe00..=0xfe0f | 0x20d0..=0x20ff => 0,
        0x1100..=0x115f
        | 0x2e80..=0x303e
        | 0x3041..=0x33ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe30..=0xfe4f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6
        | 0x1f300..=0x1f64f
        | 0x1f900..=0x1f9ff
        | 0x20000..=0x2fffd
        | 0x30000..=0x3fffd => 2,
        _ => 1,
    }
}

/// The id after `active` in `ids`, wrapping (`cycled(_:from:forward:)`);
/// `None` with fewer than two ids, an unknown `active`, or a cycle back onto
/// itself.
pub fn cycled(ids: &[String], active: Option<&str>, forward: bool) -> Option<String> {
    let active = active?;
    if ids.len() < 2 {
        return None;
    }
    let index = ids.iter().position(|id| id == active)?;
    let count = ids.len();
    let next = if forward {
        (index + 1) % count
    } else {
        (index + count - 1) % count
    };
    (ids[next] != active).then(|| ids[next].clone())
}

/// `ESC [ <row+1> ; <col+1> H` for a 0-based cursor (`#{cursor_x}`,
/// `#{cursor_y}`).
///
/// Fix (cursor-overflow): Swift adds 1 in `Int` and traps on `Int.max`; a
/// reply that pairs with the wrong command can carry any number.
pub fn cursor_position(col: i64, row: i64) -> String {
    format!("\u{1b}[{};{}H", i128::from(row) + 1, i128::from(col) + 1)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_listing_line_ends_in_its_command_and_path() {
        assert_eq!(
            command_and_path("zsh /Users/dev/code/app"),
            ("zsh", Some("/Users/dev/code/app"))
        );
        // Both may hold spaces.
        assert_eq!(
            command_and_path("Google Chrome Helper /Users/dev/My Projects/app"),
            ("Google Chrome Helper", Some("/Users/dev/My Projects/app"))
        );
        // An empty command, an empty path, the format before the path.
        assert_eq!(command_and_path(" /srv"), ("", Some("/srv")));
        assert_eq!(command_and_path("zsh "), ("zsh", None));
        assert_eq!(command_and_path("zsh"), ("zsh", None));
        assert_eq!(command_and_path("2.1.269 /"), ("2.1.269", Some("/")));
    }

    use super::*;

    /// B139, against tmux 3.6a on a private server: what these send is
    /// stored exactly (`##` is a `#`; `\$`, `\~`, `\"`, `\\` are themselves).
    #[test]
    fn a_name_is_one_literal_with_its_hashes_doubled() {
        assert_eq!(name_argument("logs"), "\"logs\"");
        assert_eq!(name_argument("#{session_name}"), "\"##{session_name}\"");
        assert_eq!(name_argument("a#(x)#"), "\"a##(x)##\"");
        assert_eq!(
            name_argument("it's $HOME ~ \"q\" \\"),
            r#""it's \$HOME \~ \"q\" \\""#
        );
        // A line break never reaches the line, whatever calls this.
        assert_eq!(name_argument("a\nb"), "\"a\\nb\"");
        assert!(refuses_name("a\nb") && refuses_name("a\tb") && refuses_name("\u{7f}"));
        assert!(refuses_name("a\u{85}b"));
        assert!(!refuses_name("👩\u{200D}💻 e\u{301} \u{FEFF}x"));
    }

    #[test]
    fn fields_follow_swift_split() {
        assert_eq!(fields("a  b c  d", b' ', 3, true), ["a", "b", "c", " d"]);
        assert_eq!(fields("$0 @11 4 l 0 1 ", b' ', 6, false).len(), 7);
        assert_eq!(fields("%3 @1 1 80 11 0 0 0 ", b' ', 8, false)[8], "");
        assert_eq!(fields("a|b|c|d|e|f|g", b'|', 5, false)[5], "f|g");
        assert_eq!(words(",,80x24", b','), ["80x24"]);
    }

    #[test]
    fn swift_doubles() {
        assert_eq!(swift_double("12"), Some(12.0));
        assert_eq!(swift_double(" 12"), None);
        assert_eq!(swift_double("0x1p3"), Some(8.0));
        assert_eq!(swift_double("-0x1.8p1"), Some(-3.0));
        assert_eq!(swift_double("1e3"), Some(1000.0));
        assert_eq!(swift_double("1_0"), None);
        assert_eq!(swift_double(""), None);
        assert!(swift_double("nan").unwrap().is_nan());
        assert_eq!(swift_double("Infinity"), Some(f64::INFINITY));
    }

    #[test]
    fn cursor_position_never_overflows() {
        assert_eq!(cursor_position(4, 0), "\u{1b}[1;5H");
        assert_eq!(
            cursor_position(i64::MAX, 0),
            "\u{1b}[1;9223372036854775808H"
        );
    }
}
