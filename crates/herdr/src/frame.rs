//! herdr's frame channel, `herdr terminal session control`: the
//! newline-delimited JSON it prints, and the lines the app writes into it.
//!
//! A port of `Moshpit/Services/Herdr/HerdrFrameChannel.swift`, plus the two
//! pieces of `SessionHub.swift` that shape frame-channel input: bracketed
//! pastes ([`bracketed_paste`]) and arrow keys sent as semantic keys
//! ([`arrow_key_name`]).
//!
//! The channel runs inside a login shell on a PTY, so the stream carries
//! more than frames: a profile's banner before the command starts, a prompt
//! between two targets, herdr's own complaints as plain text. Anything that
//! is not a frame is skipped, never treated as corruption, and a line cut off
//! mid-write glues onto the next one, fails to parse and is dropped; the
//! next target opens with a full repaint.

use memchr::memchr;

use crate::{base64, json, launch, text};

/// One message from the frame channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A screen update. `bytes` are escape sequences ready to feed into an
    /// emulator: absolutely positioned cell runs (`ESC[3;6H` per run), a
    /// painted screen rather than a log, so they never add up to local
    /// scrollback. A `full` one starts with its own clear and home.
    Screen {
        seq: u64,
        width: i64,
        height: i64,
        full: bool,
        bytes: Vec<u8>,
    },
    /// The channel ended on the server's side (pane closed, server stopped).
    Closed { reason: Option<String> },
    /// The app's own retarget loop announcing it is up and reading targets
    /// ([`READY_MARKER`]): what licenses a one-word retarget.
    LoopReady,
    /// herdr refused or aborted the attach and said so, as a JSON error
    /// envelope or as the CLI's plain `herdr: …` line. Must reach the user.
    Fault { message: String },
}

/// The `type` of [`READY_MARKER`]. Prefixed `moshpit.` because nothing on
/// the server ever emits it.
pub const READY_TYPE: &str = "moshpit.ready";

/// Printed by [`boot`] before its first attach, so the app can tell "the
/// retarget loop is running" from "the login shell couldn't take that shape".
pub const READY_MARKER: &str = r#"{"type":"moshpit.ready"}"#;

/// Detach without killing the pane; sent before retargeting so the server
/// releases its exclusive attach owner.
pub const RELEASE: &str = r#"{"type":"terminal.release"}"#;

/// The attach CLI saw the server hang up mid-handshake: the only place
/// `terminal session control` prints this. Seen live as client/server skew
/// the server can't even parse (herdr 0.9.5 dropping a 0.8.0 hello), so the
/// message names no versions; worth a `herdr status` before it reaches the
/// user raw.
pub fn is_handshake_drop(message: &str) -> bool {
    message.starts_with("lost connection to server")
}

/// Incremental parser for the frame channel.
#[derive(Debug, Default)]
pub struct FrameParser {
    pending: Vec<u8>,
}

impl FrameParser {
    /// Hard cap on an unfinished line. A full redraw of a large pane runs to
    /// tens of KB; this only stops a stream that never produces a newline (a
    /// wedged channel, a binary flood) from growing until the app is killed.
    ///
    /// Fix (buffer-cap): the Swift parser tested the cap on everything
    /// buffered after appending a chunk, so a chunk that carried the end of
    /// an over-long line lost the complete frames after it too. Here complete
    /// lines are always parsed, and only an unfinished tail past the cap is
    /// dropped.
    pub const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;

    pub fn new() -> Self {
        Self::default()
    }

    /// Feed channel bytes, get back the frames they completed, in order.
    pub fn consume(&mut self, data: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut rest = data;
        // Lines are taken in place, never by shifting a buffer (the Swift
        // parser's `removeSubrange` per line made a chunk of many frames
        // quadratic); only the trailing partial line is copied.
        while let Some(newline) = memchr(b'\n', rest) {
            let frame = if self.pending.is_empty() {
                parse_line(&rest[..newline])
            } else {
                self.pending.extend_from_slice(&rest[..newline]);
                let frame = parse_line(&self.pending);
                self.pending.clear();
                frame
            };
            frames.extend(frame);
            rest = &rest[newline + 1..];
        }
        self.pending.extend_from_slice(rest);
        if self.pending.len() > Self::MAX_BUFFER_BYTES {
            self.pending = Vec::new();
        }
        frames
    }
}

/// One line, or `None` if it isn't a frame. A port of
/// `HerdrFrameParser.parse(_:)`.
pub fn parse_line(line: &[u8]) -> Option<Frame> {
    // From the first `{`: a prompt with no trailing newline prefixes the
    // next line, and dropping it costs nothing.
    let Some(start) = memchr(b'{', line) else {
        // No JSON at all, but the CLI reports a refused handshake as plain
        // text, `herdr: server rejected handshake…`. With `stty raw -echo`
        // in force nothing the app writes comes back, so a line opening with
        // "herdr: " can only be the binary talking.
        let decoded = text::decode_utf8(line)?;
        // Newlines too: the PTY ends lines with CRLF.
        let trimmed = text::trim_whitespaces_and_newlines(&decoded);
        return trimmed.strip_prefix("herdr: ").map(|message| Frame::Fault {
            message: message.to_owned(),
        });
    };
    let object = json::parse_object(&line[start..])?;
    let Some(kind) = object.str("type") else {
        // An error envelope has no type: `{"id":…,"error":{"code","message"}}`
        // (protocol_mismatch is the live example). First line only; the
        // CLI's remediation prose is sized for a terminal.
        let message = object.object("error")?.str("message")?;
        let brief = message.split('\n').next().unwrap_or(message);
        return Some(Frame::Fault {
            message: brief.to_owned(),
        });
    };
    match kind {
        "terminal.frame" => Some(Frame::Screen {
            seq: object.u64("seq").unwrap_or(0),
            width: object.i64("width").unwrap_or(0),
            height: object.i64("height").unwrap_or(0),
            full: object.bool("full").unwrap_or(false),
            bytes: base64::decode(object.str("bytes")?)?,
        }),
        "terminal.closed" => Some(Frame::Closed {
            reason: object.str("reason").map(str::to_owned),
        }),
        READY_TYPE => Some(Frame::LoopReady),
        _ => None,
    }
}

/// What herdr's `terminal session control` accepts for a size or a line
/// count: a `u16` above zero.
///
/// Fix (size-clamp): the Swift builders clamped only below, and the boot and
/// one-shot lines not at all. `--cols 0` is refused by herdr, and a resize
/// past 65 535 makes herdr print `herdr: terminal session control input
/// ignored: …`, which the app reads as a refused attach (a `Fault`): it
/// stops claiming the pane and backs off for 30 s while the attach keeps
/// painting. Not reachable from the iOS terminal (SwiftTerm never reports
/// fewer than 2 × 1 cells, and no screen holds 65 536 columns); a core that
/// other terminal views drive should not depend on that.
pub fn clamp_cells(value: i64) -> u16 {
    value.clamp(1, i64::from(u16::MAX)) as u16
}

/// Where the frame loop ([`boot`]) records its own pid, per connection: the
/// next connection's boot line stops the attach a connection that dropped
/// left holding its pane.
pub fn loop_pid_path(connection_id: &str) -> String {
    format!("$HOME/.moshpit/herdr-frame-{connection_id}.pid")
}

/// Boot line for a one-shot channel, written into an interactive remote
/// shell (append `\r` at the call site).
///
/// - `stty raw -echo` is mandatory: on a PTY the line discipline would echo
///   every JSON command back into the frame stream, and canonical mode would
///   cut base64 input lines at 4096 bytes.
/// - No `exec`: the shell has to survive the attach so the channel can be
///   retargeted without rebuilding the SSH channel.
/// - `--takeover` only when the person chose the pane (`takeover`: a tap, a
///   reply for it, "Open here"). Following the shared focus always took
///   over, so a second device lost its pane whenever this one followed the
///   focus onto it; a plain attach to a terminal another client holds is
///   refused instead (`terminal.closed`, "… already has an attached client;
///   retry with --takeover", herdr 0.9.5).
pub fn start(
    target: &str,
    cols: i64,
    rows: i64,
    custom_path: Option<&str>,
    takeover: bool,
) -> String {
    format!(
        "stty raw -echo; {} terminal session control {} --cols {} --rows {}{}",
        launch::attach_command(custom_path),
        launch::quote(target),
        clamp_cells(cols),
        clamp_cells(rows),
        if takeover { " --takeover" } else { "" },
    )
}

/// Boot line for a channel that can be retargeted in one word: a reader
/// loop that attaches, and when the attach ends reads ONE line,
/// `<target> <cols> <rows>`, and attaches again (40 ms a switch against 545
/// ms for a command at the login shell's prompt, measured on herdr 0.8.0).
///
/// - The first target rides in as `$1`: written as a second line it would
///   race the outer shell, which would run it as a command.
/// - `sh -c`, so a login shell that can't parse a `while` loop (fish, csh)
///   only ever sees one quoted argument; the whole script is quoted, so a
///   custom path with a quote in it can't break out.
/// - `stty raw -echo` again inside the loop: the attach that just exited
///   owned the tty.
/// - `read … || break`: EOF ends the loop, a failed attach does not.
/// - A line opening with `{` is the app's own command meant for an attach
///   that already died; it is dropped, and the stale target with it.
/// - `--takeover` only for a target the person chose: a fourth word on the
///   line (`take`, [`retarget`]); the first target's rides in as `$4`. A
///   follow's attach is plain, and herdr refuses it while another client
///   holds the terminal.
/// - The loop records its pid ([`loop_pid_path`]) and, before that, stops
///   the attach a dropped connection of the same connection left behind:
///   sshd keeps a dead connection's session — its loop and its attach,
///   holding the pane — until TCP gives up on it, and a plain attach to the
///   pane this device had would be refused by this device's own ghost.
///   Only a child of that loop (its command line names this connection's
///   pid file) whose command line is `terminal session control` is killed,
///   never what reused a pid.
pub fn boot(
    target: &str,
    cols: i64,
    rows: i64,
    custom_path: Option<&str>,
    takeover: bool,
    connection_id: &str,
) -> String {
    format!(
        "stty raw -echo; /bin/sh -c {} moshpit-herdr {} {} {}{}",
        launch::quote(&loop_script(custom_path, connection_id)),
        launch::quote(target),
        clamp_cells(cols),
        clamp_cells(rows),
        if takeover { " take" } else { "" },
    )
}

/// The reader loop of [`boot`] as the frame channel's own program (the SSH
/// terminal channel's command), with no first target: it says it reads and
/// waits for one ([`retarget`] lines). Nothing is typed into a login shell —
/// there is none: a shell still starting swallowed the boot line or chewed
/// on it (a flushed type-ahead, a plugin re-highlighting every character, a
/// startup prompt reading stdin), and the channel never came up. The loop
/// ending (EOF) ends the channel.
pub fn loop_program(custom_path: Option<&str>, connection_id: &str) -> String {
    format!(
        "stty raw -echo; exec /bin/sh -c {} moshpit-herdr",
        launch::quote(&loop_script(custom_path, connection_id))
    )
}

fn loop_script(custom_path: Option<&str>, connection_id: &str) -> String {
    let herdr = launch::attach_command(custom_path);
    let pid = loop_pid_path(connection_id);
    let looped = launch::command_line_of("$l");
    let child = launch::command_line_of("$c");
    format!(
        "f=\"{pid}\"; mkdir -p \"$HOME/.moshpit\" 2>/dev/null; \
         l=$(cat \"$f\" 2>/dev/null); case \"$l\" in ''|*[!0-9]*) ;; *) {looped}\
         case \"$a\" in *\"herdr-frame-{connection_id}.pid\"*) \
         for c in $(pgrep -P \"$l\" 2>/dev/null || ps -A -o pid= -o ppid= 2>/dev/null | awk -v l=\"$l\" '$2 == l {{ print $1 }}'); do \
         {child}case \"$a\" in *\"terminal session control\"*) kill -9 \"$c\" 2>/dev/null ;; esac; done ;; esac ;; esac; \
         {{ echo $$ > \"$f\"; }} 2>/dev/null; \
         printf '%s\\n' '{READY_MARKER}'; while :; do \
         if [ -n \"$1\" ]; then stty raw -echo; \
         {herdr} terminal session control \"$1\" \
         --cols \"${{2:-80}}\" --rows \"${{3:-24}}\" ${{4:+--takeover}}; fi; \
         read -r t c r k || break; \
         case \"$t\" in \"{{\"*) set --; continue;; esac; \
         set -- \"$t\" \"$c\" \"$r\" \"$k\"; done"
    )
}

/// One line for the loop's `read`: where to point next, at what size, and
/// `take` when the person chose it (`--takeover`). Only valid after [`boot`]
/// announced itself, and only once the previous attach has released (until
/// then the attach owns stdin and swallows this line).
pub fn retarget(target: &str, cols: i64, rows: i64, takeover: bool) -> String {
    format!(
        "{target} {} {}{}",
        clamp_cells(cols),
        clamp_cells(rows),
        if takeover { " take" } else { "" }
    )
}

/// Keystrokes, as base64 so escape sequences, UTF-8 continuation bytes and a
/// pasted NUL survive the JSON round trip.
pub fn input(data: &[u8]) -> String {
    format!(
        r#"{{"type":"terminal.input","bytes":"{}"}}"#,
        base64::encode(data)
    )
}

pub fn resize(cols: i64, rows: i64) -> String {
    format!(
        r#"{{"type":"terminal.resize","cols":{},"rows":{}}}"#,
        clamp_cells(cols),
        clamp_cells(rows),
    )
}

/// How herdr's server should route a scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollSource {
    /// A drag or wheel gesture: the server turns it into a mouse report for
    /// a program that grabbed the mouse, or scrollback movement otherwise —
    /// the decision tmux makes the app compute itself.
    Wheel,
    /// PageUp/PageDown, for the keyboard bar's paging keys.
    PageKey,
}

impl ScrollSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ScrollSource::Wheel => "wheel",
            ScrollSource::PageKey => "page_key",
        }
    }
}

pub fn scroll(up: bool, lines: i64, source: ScrollSource) -> String {
    format!(
        r#"{{"type":"terminal.scroll","direction":"{}","lines":{},"source":"{}"}}"#,
        if up { "up" } else { "down" },
        clamp_cells(lines),
        source.as_str(),
    )
}

/// What a pointer did, for [`mouse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Down,
    Up,
    Drag,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// The first herdr whose frame channel takes [`mouse`]: 0.9.2 ("`terminal
/// session control` accepts `terminal.mouse` events", #4685). An older one
/// prints `herdr: terminal session control input ignored: …` for it, which
/// the app reads as a refused attach.
pub const MOUSE_VERSION: &str = "0.9.2";

/// Whether a server running `server_version` takes [`mouse`].
pub fn takes_mouse(server_version: &str) -> bool {
    crate::version::compare(server_version, MOUSE_VERSION) != std::cmp::Ordering::Less
}

/// A pointer event at a 0-based cell of the attached pane. No Swift
/// counterpart: the app sends nothing for a tap over herdr (SessionHub's
/// `clickActiveTerminal` returns early, "its frame protocol has no click
/// command", which herdr stopped being true of in 0.9.2). herdr's server
/// encodes the event for the pane's own mouse mode and drops it when the
/// program has not enabled mouse reporting — the server-side truth the app
/// cannot see from repaint frames, as with [`scroll`]. Only for a server
/// that [`takes_mouse`]. `modifiers` are crossterm's `KeyModifiers` bits,
/// which herdr decodes them as (shift 1, control 2, alt 4).
pub fn mouse(
    action: MouseAction,
    button: MouseButton,
    column: i64,
    row: i64,
    modifiers: u8,
) -> String {
    let action = match action {
        MouseAction::Down => "down",
        MouseAction::Up => "up",
        MouseAction::Drag => "drag",
        MouseAction::Move => "move",
    };
    let button = match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    };
    let cell = |value: i64| value.clamp(0, i64::from(u16::MAX));
    format!(
        r#"{{"type":"terminal.mouse","action":"{action}","button":"{button}","column":{},"row":{},"modifiers":{modifiers}}}"#,
        cell(column),
        cell(row),
    )
}

const PASTE_START: &str = "\u{1b}[200~";
const PASTE_END: &str = "\u{1b}[201~";

/// `text` as one complete bracketed paste, for the frame channel
/// (`SessionHub.ActiveSession.herdrPaste`).
///
/// Over SSH + herdr every paste goes out bracketed whatever the local
/// emulator thinks: that emulator is fed repaint frames that carry no mode
/// changes, so its bracketed-paste flag never turns on. herdr ≥ 0.9 re-
/// encodes an input that is exactly one bracketed paste against the pane's
/// real mode (markers kept when the program asked for them, dropped when it
/// didn't); an older server passes them through, which is what the programs
/// a phone pastes into ask for anyway.
///
/// An end marker inside the text is dropped, since the server only takes an
/// input whose FIRST end marker is its last bytes; one left in the body ends
/// the paste early and the rest is typed, newlines and all (verified live:
/// the pasted command runs). Start markers are dropped too, as the app's
/// `BracketedPaste` does: no paste carries a marker inside it.
///
/// - Fix (paste-marker-unicode): Swift's removal searched by composed
///   character sequence and missed a marker whose `~` carries a combining
///   mark. Markers are removed byte for byte here.
/// - Fix (paste-end-marker): Swift removed markers in one pass, so the text
///   `ESC[20`, `ESC[201~`, `1~` came out as a whole end marker. Each marker
///   is removed as its last byte is read, which leaves none: the bytes kept
///   so far never hold one, so the only one a new byte can complete ends at
///   that byte. One pass, where repeating `replace` until nothing changes
///   was quadratic in the nesting.
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(PASTE_START.len() + text.len() + PASTE_END.len());
    body.extend_from_slice(PASTE_START.as_bytes());
    body.extend_from_slice(paste_body(text).as_bytes());
    body.extend_from_slice(PASTE_END.as_bytes());
    body
}

/// `text` with every bracketed-paste start and end marker taken out, until
/// none is left (`BracketedPaste.body` / `sanitized` in the app): the one
/// sanitizer every paste path goes through, whoever adds the brackets — the
/// frame channel here ([`bracketed_paste`]), tmux's `paste-buffer -p`
/// (crates/tmux). Markers are matched byte for byte, and each is removed as
/// its last byte is read, so removing one can't join the bytes around it
/// into another (see [`bracketed_paste`]'s fixes). Taking ASCII out of UTF-8
/// leaves UTF-8, so the result is still the text it was, less the markers.
pub fn paste_body(text: &str) -> String {
    let mut kept: Vec<u8> = Vec::with_capacity(text.len());
    for &byte in text.as_bytes() {
        kept.push(byte);
        if byte == b'~' && kept.len() >= PASTE_END.len() && ends_with_marker(&kept) {
            kept.truncate(kept.len() - PASTE_END.len());
        }
    }
    String::from_utf8(kept).expect("removing ASCII markers keeps UTF-8 valid")
}

/// Whether `bytes` ends with `ESC[200~` or `ESC[201~`.
fn ends_with_marker(bytes: &[u8]) -> bool {
    let tail = &bytes[bytes.len() - PASTE_END.len()..];
    tail == PASTE_START.as_bytes() || tail == PASTE_END.as_bytes()
}

/// A payload that is exactly one arrow-key sequence, as its herdr key name
/// (`SessionHub.ActiveSession.arrowKeyName`). Either DECCKM variant: over
/// herdr the local emulator never learns the pane's cursor-key mode, so the
/// sender's choice can't be trusted, and the key goes to `pane send-keys`
/// for the server to encode. Anything else (batched keys, other sequences,
/// text) is not a key.
pub fn arrow_key_name(data: &[u8]) -> Option<&'static str> {
    lone_arrow(data)
}

/// One piece of what the view wrote: a lone arrow, or the bytes between
/// arrows, as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece<'a> {
    Arrow(&'static str),
    Bytes(&'a [u8]),
}

/// `bytes` as its lone arrows and the runs between them, in order (B360).
/// A pane's input channel hands over everything written since it was last
/// read as one chunk, so key repeat or a busy moment brings arrows
/// together, alone or among other keys: each arrow still goes as a key name
/// for the multiplexer to encode by the pane's real cursor-key mode
/// ([`arrow_key_name`]). An arrow is exactly `ESC [ A–D` or `ESC O A–D`; a
/// sequence with parameters (`ESC [ 1 ; 5 A`) never is, nor is anything
/// inside a bracketed paste: pasted text, whatever it holds, is text to its
/// end marker (or the chunk's end). ESC never appears inside a UTF-8
/// character, so no character is cut.
pub fn split_arrows(bytes: &[u8]) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let (mut start, mut at) = (0, 0);
    while at + 3 <= bytes.len() {
        if bytes[at..].starts_with(PASTE_START.as_bytes()) {
            let body = at + PASTE_START.len();
            at = bytes[body..]
                .windows(PASTE_END.len())
                .position(|window| window == PASTE_END.as_bytes())
                .map_or(bytes.len(), |end| body + end + PASTE_END.len());
            continue;
        }
        match lone_arrow(&bytes[at..at + 3]) {
            Some(key) => {
                if start < at {
                    pieces.push(Piece::Bytes(&bytes[start..at]));
                }
                pieces.push(Piece::Arrow(key));
                at += 3;
                start = at;
            }
            None => at += 1,
        }
    }
    if start < bytes.len() {
        pieces.push(Piece::Bytes(&bytes[start..]));
    }
    pieces
}

fn lone_arrow(data: &[u8]) -> Option<&'static str> {
    match data {
        [0x1B, b'[' | b'O', last] => match last {
            b'A' => Some("up"),
            b'B' => Some("down"),
            b'C' => Some("right"),
            b'D' => Some("left"),
            _ => None,
        },
        _ => None,
    }
}
