//! Pure protocol parser for tmux's control mode (`tmux -CC`).
//!
//! A port of `Moshpit/Services/Tmux/TmuxControlClient.swift`. Bytes go in
//! through [`ControlParser::feed`], [`Event`]s come out, in stream order.
//!
//! Responsibilities:
//!   - buffer raw transport bytes and split them into lines on `\n` (one
//!     trailing `\r` stripped, so CRLF and LF transports look the same);
//!   - recognise notification lines (`%output`, `%window-add`, ...);
//!   - track the open `%begin … %end / %error` block and deliver its lines as
//!     one [`CommandResponse`];
//!   - decode tmux's octal escapes in `%output` payloads, so callers get the
//!     pane's raw byte stream.
//!
//! Not its job: session, window and pane state (the controller owns that),
//! buffering output per pane, or the transport. The parser never reads a
//! socket; the caller pumps bytes in.
//!
//! The semantics are the Swift parser's, line for line. Five fixes were
//! made here first and have since been ported to the Swift parser (see
//! `harness/divergences.json`): each is marked `Fix (<id>)` below, next to
//! what the Swift parser did before it (at fd87248f).

use std::fmt;

use memchr::memchr;
use memchr::memmem;

/// A reply block cannot legitimately hold more lines than the biggest
/// capture the app ever requests (a 2 000-line backfill plus a screen). Past
/// this, the block's terminator was lost in transit and the block would
/// otherwise swallow every later reply and `%output`: it is force-closed.
pub const MAX_BLOCK_LINES: usize = 5000;

const OUTPUT_PREFIX: &[u8] = b"%output ";

/// Prefixes [`decode_line`] accepts after leading junk (a DCS prologue, a
/// shell banner).
const KEYWORDS: [&str; 12] = [
    "%begin",
    "%end",
    "%error",
    "%output",
    "%layout",
    "%window",
    "%session",
    "%pause",
    "%continue",
    "%client",
    "%exit",
    "%unlinked",
];

/// Reported through [`Event::ProtocolError`] when a recognised `%foo` line
/// cannot be decoded (a `%output` with no pane id, a stray `%end`, ...).
/// Unknown directives are deliberately not errors: they pass silently, for
/// forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    MalformedLine(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlError::MalformedLine(line) => write!(f, "malformed control line: {line}"),
        }
    }
}

impl std::error::Error for ControlError {}

/// Result of one tmux command, echoed inside a `%begin / %end / %error`
/// block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResponse {
    /// First field of the `%begin` line (tmux puts the command's time here).
    pub command_id: i64,
    /// Second field: the command number, increments per command.
    pub command_num: i64,
    /// Whether tmux closed the block with `%error` rather than `%end`.
    pub is_error: bool,
    /// The lines between `%begin` and the terminator, in order, each decoded
    /// as UTF-8 (lossily) without its line ending.
    pub lines: Vec<String>,
    /// The `%begin` line's flags say the command came from this client's
    /// own input (bit 1; `true` when the field is missing or unreadable).
    /// tmux also writes blocks for commands nobody here sent: an
    /// `after-<command>` hook that one of ours fired, the late half of an
    /// `if-shell -b` / `run-shell -b`, the boot command's own reply — all
    /// with flags 0 (3.6a, read off a private server; `cmdq_guard` passes
    /// `CMDQ_STATE_CONTROL`). Those answer no command of ours.
    pub from_client: bool,
}

/// One thing the control stream said. Ids keep tmux's own spelling: `$0`
/// for sessions, `@0` for windows, `%0` for panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `%output %pane <data>`: the pane's bytes with octal escapes decoded.
    /// Not necessarily valid UTF-8 — tmux flushes mid-character.
    PaneOutput { pane_id: String, data: Vec<u8> },
    /// `%layout-change @window <layout> [visible-layout] [flags]`.
    LayoutChange {
        window_id: String,
        layout: String,
        zoomed: bool,
    },
    /// `%window-add @window`.
    WindowAdd { window_id: String },
    /// `%window-close @window`.
    WindowClose { window_id: String },
    /// `%window-renamed @window <name>`.
    WindowRenamed { window_id: String, name: String },
    /// `%session-changed $session <name>`; the name may be empty.
    SessionChanged { session_id: String, name: String },
    /// `%session-renamed $session <name>`: any client renamed any session.
    /// The id is not empty; the name is the rest of the line.
    SessionRenamed { session_id: String, name: String },
    /// `%session-window-changed $session @window`.
    SessionWindowChanged {
        session_id: String,
        window_id: String,
    },
    /// `%sessions-changed`: a session was created or destroyed, anywhere on
    /// the server.
    SessionsChanged,
    /// `%unlinked-window-add @window`: a window was linked into a session
    /// that is not this client's.
    UnlinkedWindowAdd { window_id: String },
    /// `%unlinked-window-close @window`. tmux 3.6a sends this, not
    /// `%window-close`, for a window killed in this client's own session too:
    /// the window is already unlinked when the notification goes out.
    UnlinkedWindowClose { window_id: String },
    /// `%unlinked-window-renamed @window <name>`, cut like
    /// `%window-renamed`.
    UnlinkedWindowRenamed { window_id: String, name: String },
    /// `%client-session-changed <client> $session <name>`: another client
    /// attached to or switched to a session. Client and id are not empty
    /// and hold no space; the name is the rest of the line.
    ClientSessionChanged {
        client: String,
        session_id: String,
        name: String,
    },
    /// `%window-pane-changed @window %pane`.
    ActivePaneChanged { window_id: String, pane_id: String },
    /// `%pause %pane`: answer with `refresh-client -A '%pane:continue'`.
    Pause { pane_id: String },
    /// `%continue %pane`.
    Continue { pane_id: String },
    /// `%client-detached [client]`: whatever followed the directive, or "".
    ClientDetached { payload: String },
    /// `%exit [reason]`: this control session ended.
    Exit { reason: Option<String> },
    /// A completed `%begin … %end / %error` block.
    CommandResponse(CommandResponse),
    /// A recognised line that could not be decoded, or a recovery note.
    ProtocolError(ControlError),
}

#[derive(Debug)]
struct OpenBlock {
    command_id: i64,
    command_num: i64,
    from_client: bool,
    lines: Vec<String>,
    /// `%end <cid> <num>` and `%error <cid> <num>` for this block, built
    /// once at `%begin`: the embedded-terminator check runs on every line
    /// of the block, and a backfill reply is thousands of lines (B102).
    end_needle: Box<[u8]>,
    error_needle: Box<[u8]>,
}

/// Bytes an embedded terminator may carry after its ids: a space and the
/// flags digit(s).
const MAX_TERMINATOR_TAIL: usize = 8;

/// The parser. Feed it transport bytes; it calls back with every event the
/// complete lines among them produce.
#[derive(Debug)]
pub struct ControlParser {
    /// Bytes after the last `\n`: a line still on its way.
    partial: Vec<u8>,
    /// The `%begin` block being collected, if any.
    open_block: Option<OpenBlock>,
    /// Ids of the most recent real `%begin`, for truncated-line recovery:
    /// command numbers increase per command on a connection, so a protocol
    /// line glued onto a truncated one carries the next number, while pane
    /// content quoting an old dump carries stale ones. Kept across
    /// [`reset`](Self::reset), like the Swift parser.
    last_block_cid: i64,
    last_block_num: i64,
    /// True while the next completed block is the boot command's own reply
    /// (the `-CC attach` / `-CC new` line, which nobody queued a callback
    /// for). One stream can carry several control sessions — a failed
    /// preferred attach emits `%begin/%error/%exit` and the fallback a fresh
    /// banner — so this is re-armed by every `%exit`.
    awaiting_boot_block: bool,
}

impl Default for ControlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlParser {
    pub fn new() -> Self {
        Self {
            partial: Vec::new(),
            open_block: None,
            last_block_cid: 0,
            last_block_num: 0,
            awaiting_boot_block: true,
        }
    }

    /// Feed transport bytes. Every complete line is dispatched now and its
    /// events handed to `emit` in order; a trailing partial line waits for
    /// the next call. Empty input is fine.
    pub fn feed(&mut self, data: &[u8], mut emit: impl FnMut(Event)) {
        if data.is_empty() {
            return;
        }
        let mut rest = data;
        if !self.partial.is_empty() {
            let Some(newline) = memchr(b'\n', rest) else {
                self.partial.extend_from_slice(rest);
                return;
            };
            let mut line = std::mem::take(&mut self.partial);
            line.extend_from_slice(&rest[..newline]);
            self.dispatch_line(strip_cr(&line), &mut emit);
            line.clear();
            self.partial = line;
            rest = &rest[newline + 1..];
        }
        // Complete lines straight from the caller's buffer, no copy.
        while let Some(newline) = memchr(b'\n', rest) {
            self.dispatch_line(strip_cr(&rest[..newline]), &mut emit);
            rest = &rest[newline + 1..];
        }
        self.partial.extend_from_slice(rest);
    }

    /// [`feed`](Self::feed), collecting the events.
    pub fn feed_collect(&mut self, data: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        self.feed(data, |event| events.push(event));
        events
    }

    /// Drop any partial line and any open block, and expect a boot block
    /// again. For a reconnect, or to resync after a protocol error.
    pub fn reset(&mut self) {
        self.partial = Vec::new();
        self.open_block = None;
        self.awaiting_boot_block = true;
    }

    // MARK: Line dispatch

    fn dispatch_line<F: FnMut(Event)>(&mut self, raw: &[u8], emit: &mut F) {
        if self.open_block.is_some() {
            self.dispatch_block_line(raw, emit);
            return;
        }

        // `%output` is handled on raw bytes, before any string decoding: the
        // payload is the pane's byte stream (tmux escapes only control bytes
        // and backslash; bytes >= 0x80 pass verbatim) and tmux flushes
        // mid-character, so a UTF-8 sequence can straddle two events.
        if raw.starts_with(OUTPUT_PREFIX) {
            self.handle_output(raw, emit);
            return;
        }

        let line = decode_line(raw);
        // Outside a block only `%` lines are notifications; anything else is
        // transport noise (a banner, a DCS prologue, an ssh MOTD).
        if !line.starts_with('%') {
            return;
        }

        if line.starts_with("%begin ") {
            self.handle_begin(&line, emit);
        } else if let Some(body) = line.strip_prefix("%layout-change ") {
            handle_layout_change(&line, body, emit);
        } else if let Some(id) = line.strip_prefix("%window-add ") {
            emit_id(&line, id, emit, |window_id| Event::WindowAdd { window_id });
        } else if let Some(id) = line.strip_prefix("%window-close ") {
            emit_id(&line, id, emit, |window_id| Event::WindowClose {
                window_id,
            });
        } else if let Some(body) = line.strip_prefix("%window-renamed ") {
            match body.split_once(' ') {
                Some((window_id, name)) => emit(Event::WindowRenamed {
                    window_id: window_id.to_owned(),
                    name: name.to_owned(),
                }),
                None => emit(malformed(&line)),
            }
        } else if let Some(body) = line.strip_prefix("%session-changed ") {
            match body.split_once(' ') {
                Some((session_id, name)) => emit(Event::SessionChanged {
                    session_id: session_id.to_owned(),
                    name: name.to_owned(),
                }),
                // Some tmux builds send the id alone.
                None if !body.is_empty() => emit(Event::SessionChanged {
                    session_id: body.to_owned(),
                    name: String::new(),
                }),
                None => emit(malformed(&line)),
            }
        } else if let Some(body) = line.strip_prefix("%session-renamed ") {
            match body.split_once(' ') {
                Some((session_id, name)) if !session_id.is_empty() => emit(Event::SessionRenamed {
                    session_id: session_id.to_owned(),
                    name: name.to_owned(),
                }),
                _ => emit(malformed(&line)),
            }
        } else if let Some(body) = line.strip_prefix("%session-window-changed ") {
            match split_swift(body, 1, true)[..] {
                [session_id, window_id] => emit(Event::SessionWindowChanged {
                    session_id: session_id.to_owned(),
                    window_id: window_id.to_owned(),
                }),
                _ => emit(malformed(&line)),
            }
        } else if let Some(body) = line.strip_prefix("%window-pane-changed ") {
            match split_swift(body, 1, true)[..] {
                [window_id, pane_id] => emit(Event::ActivePaneChanged {
                    window_id: window_id.to_owned(),
                    pane_id: pane_id.to_owned(),
                }),
                _ => emit(malformed(&line)),
            }
        } else if line == "%sessions-changed" || line.starts_with("%sessions-changed ") {
            emit(Event::SessionsChanged);
        } else if let Some(id) = line.strip_prefix("%unlinked-window-add ") {
            emit_id(&line, id, emit, |window_id| Event::UnlinkedWindowAdd {
                window_id,
            });
        } else if let Some(id) = line.strip_prefix("%unlinked-window-close ") {
            emit_id(&line, id, emit, |window_id| Event::UnlinkedWindowClose {
                window_id,
            });
        } else if let Some(body) = line.strip_prefix("%unlinked-window-renamed ") {
            match body.split_once(' ') {
                Some((window_id, name)) => emit(Event::UnlinkedWindowRenamed {
                    window_id: window_id.to_owned(),
                    name: name.to_owned(),
                }),
                None => emit(malformed(&line)),
            }
        } else if let Some(body) = line.strip_prefix("%client-session-changed ") {
            match body.split_once(' ') {
                Some((client, rest)) if !client.is_empty() => match rest.split_once(' ') {
                    Some((session_id, name)) if !session_id.is_empty() => {
                        emit(Event::ClientSessionChanged {
                            client: client.to_owned(),
                            session_id: session_id.to_owned(),
                            name: name.to_owned(),
                        });
                    }
                    _ => emit(malformed(&line)),
                },
                _ => emit(malformed(&line)),
            }
        } else if let Some(id) = line.strip_prefix("%pause ") {
            emit_id(&line, id, emit, |pane_id| Event::Pause { pane_id });
        } else if let Some(id) = line.strip_prefix("%continue ") {
            emit_id(&line, id, emit, |pane_id| Event::Continue { pane_id });
        } else if line.starts_with("%client-detached") {
            let payload = line.strip_prefix("%client-detached ").unwrap_or("");
            emit(Event::ClientDetached {
                payload: payload.to_owned(),
            });
        } else if line == "%exit" || line.starts_with("%exit ") {
            // A control session ended on this stream; whatever block comes
            // next is the following session's boot reply.
            self.awaiting_boot_block = true;
            let reason = line.strip_prefix("%exit ").unwrap_or("");
            emit(Event::Exit {
                reason: (!reason.is_empty()).then(|| reason.to_owned()),
            });
        } else if line.starts_with("%end ")
            || line == "%end"
            || line.starts_with("%error ")
            || line == "%error"
        {
            // A terminator without a `%begin`: reported, so logs show the
            // desync.
            emit(malformed(&line));
        }
        // Anything else is an unknown directive: skipped, forward-compat.
    }

    /// A line while a block is open. Captured verbatim: a capture-pane dump
    /// can hold any text, including `%keyword` lines. Only the open block's
    /// own terminator ends it.
    fn dispatch_block_line<F: FnMut(Event)>(&mut self, raw: &[u8], emit: &mut F) {
        let line = foundation_string(raw);

        // Truncated-line recovery, in-block half: a lossy hop can drop a
        // content line's tail and glue the block's own terminator onto it.
        if let Some((content, terminator)) = self.split_embedded_terminator(&line) {
            if !content.is_empty() {
                self.push_block_line(content);
            }
            let is_error = terminator.starts_with("%error");
            self.handle_block_end(&terminator, is_error, emit);
            return;
        }

        // Fix (block-cap-terminator): the open block's own terminator closes
        // it even when the block already holds MAX_BLOCK_LINES lines. Swift
        // checks the line cap first, so a block of exactly 5000 lines came
        // out as an error response plus a stray-`%end` protocol error.
        if self.terminates_open_block(&line, "%end") {
            self.handle_block_end(&line, false, emit);
            return;
        }
        if self.terminates_open_block(&line, "%error") {
            self.handle_block_end(&line, true, emit);
            return;
        }

        // Runaway block: its terminator was lost outright. Deliver what
        // accumulated as an error (the sender's callback holds a FIFO slot;
        // silence would shift pairing for good) and re-dispatch this line as
        // if the block had closed.
        if self
            .open_block
            .as_ref()
            .is_some_and(|block| block.lines.len() >= MAX_BLOCK_LINES)
        {
            let block = self.open_block.take().expect("block is open");
            emit(malformed(&format!(
                "block #{} exceeded {} lines — forcing close",
                block.command_id, MAX_BLOCK_LINES
            )));
            self.deliver_block(block, true, emit);
            self.dispatch_line(raw, emit);
            return;
        }

        self.push_block_line(line);
    }

    fn push_block_line(&mut self, line: String) {
        if let Some(block) = self.open_block.as_mut() {
            block.lines.push(line);
        }
    }

    // MARK: Block lifecycle

    /// `"<content>%end <cid> <num> …"` where cid and num are the open
    /// block's own ids: `(content, terminator)`. `None` when there is no
    /// such suffix, or when it starts the line (the prefix checks handle
    /// that).
    fn split_embedded_terminator(&self, line: &str) -> Option<(String, String)> {
        let block = self.open_block.as_ref()?;
        for needle in [&block.end_needle, &block.error_needle] {
            // Fix (B102): only the end of the line can hold it. The last
            // occurrence must leave at most MAX_TERMINATOR_TAIL bytes after
            // it, so it starts in the line's last `needle + tail` bytes —
            // searching that window finds the same last occurrence, or
            // proves there is none that could count. The whole-line search
            // with a needle formatted per line made a backfill reply parse
            // at 3–6 MB/s. The needle is ASCII, so a match lies on
            // character boundaries even when the window's start does not.
            let window = line
                .len()
                .saturating_sub(needle.len() + MAX_TERMINATOR_TAIL);
            let Some(at) = memmem::rfind(&line.as_bytes()[window..], needle).map(|at| window + at)
            else {
                continue;
            };
            // It must run to the end of the line, allowing trailing flags.
            let tail = &line[at + needle.len()..];
            if tail.len() > MAX_TERMINATOR_TAIL
                || !tail.bytes().all(|b| b == b' ' || b.is_ascii_digit())
            {
                continue;
            }
            // Fix (terminator-number-boundary): the number must end where
            // the needle does. Swift accepted any digit-and-space tail, so
            // "%end T 10" matched inside "%end T 105" and closed block 10
            // early on a line of pane content.
            if !(tail.is_empty() || tail.starts_with(' ')) {
                continue;
            }
            if at == 0 {
                return None;
            }
            return Some((line[..at].to_owned(), line[at..].to_owned()));
        }
        None
    }

    /// `…%begin <cid> <num> <flag>` at the end of an `%output` payload whose
    /// num is a plausible next command number (after the last real block,
    /// within a small window): the offset where `%begin` starts. Content
    /// quoting an old dump carries stale numbers and fails the window.
    fn embedded_begin_offset(&self, payload: &[u8]) -> Option<usize> {
        let at = memmem::rfind(payload, b"%begin ")?;
        if at == 0 {
            return None;
        }
        let suffix = std::str::from_utf8(&payload[at..]).ok()?;
        let parts: Vec<&str> = suffix.split(' ').collect();
        let [_, cid, num, flag] = parts[..] else {
            return None;
        };
        let cid = parse_int(cid)?;
        let num = parse_int(num)?;
        // Fix (unicode-syntax): an ASCII digit. Swift took any one Character
        // with a Unicode numeric value ("五", "٣", "½").
        if !(flag.len() == 1 && flag.as_bytes()[0].is_ascii_digit()) {
            return None;
        }
        // Fix (overflow-trap): the distances in 128 bits. Swift computes
        // `abs(cid - lastBlockCid)` in Int, which traps — the app crashes —
        // when pane output ends in a `%begin` with a huge negative id.
        let plausible = num > self.last_block_num
            && i128::from(num) - i128::from(self.last_block_num) <= 200
            && (i128::from(cid) - i128::from(self.last_block_cid)).abs() <= 86_400;
        plausible.then_some(at)
    }

    /// True when `line` is the open block's own terminator: `%end`/`%error`
    /// echoing the ids of its `%begin` (or the bare keyword). A `%end`-shaped
    /// line with other ids is pane content — a capture of a pane that itself
    /// shows control-mode text.
    fn terminates_open_block(&self, line: &str, keyword: &str) -> bool {
        let Some(block) = self.open_block.as_ref() else {
            return false;
        };
        if line == keyword {
            return true;
        }
        if !line
            .strip_prefix(keyword)
            .is_some_and(|rest| rest.starts_with(' '))
        {
            return false;
        }
        let parts = split_swift(line, 4, false);
        if parts.len() < 3 {
            return false;
        }
        match (parse_int(parts[1]), parse_int(parts[2])) {
            (Some(cid), Some(num)) => cid == block.command_id && num == block.command_num,
            _ => false,
        }
    }

    /// `%begin <commandId> <commandNum> [flags]`.
    fn handle_begin<F: FnMut(Event)>(&mut self, line: &str, emit: &mut F) {
        let parts = split_swift(line, 4, false);
        let ids = match parts[..] {
            [_, cid, num, ..] => parse_int(cid).zip(parse_int(num)),
            _ => None,
        };
        let Some((cid, num)) = ids else {
            emit(malformed(line));
            return;
        };
        // `[flags]`: a block for a command that came from this client's
        // input has bit 1 set. Missing or unreadable reads as ours, the old
        // way of reading every block.
        let from_client = parts
            .get(3)
            .and_then(|flags| parse_int(flags))
            .is_none_or(|flags| flags & 1 == 1);
        // A block still open means its terminator never came. Deliver it as
        // an error response: its sender's callback holds a FIFO slot.
        if let Some(stale) = self.open_block.take() {
            emit(malformed(&format!(
                "orphan %begin while block #{} open",
                stale.command_id
            )));
            self.deliver_block(stale, true, emit);
        }
        self.last_block_cid = cid;
        self.last_block_num = num;
        self.open_block = Some(OpenBlock {
            command_id: cid,
            command_num: num,
            from_client,
            lines: Vec::new(),
            end_needle: format!("%end {cid} {num}").into_bytes().into(),
            error_needle: format!("%error {cid} {num}").into_bytes().into(),
        });
    }

    /// `%end <id> <num>` or `%error <id> <num>`.
    fn handle_block_end<F: FnMut(Event)>(&mut self, line: &str, is_error: bool, emit: &mut F) {
        let Some(block) = self.open_block.take() else {
            emit(malformed(line));
            return;
        };
        let parts = split_swift(line, 4, false);
        if parts.len() >= 3
            && let Some(cid) = parse_int(parts[1])
            && cid != block.command_id
        {
            emit(malformed(&format!(
                "mismatched terminator id {cid} for block #{}",
                block.command_id
            )));
        }
        self.deliver_block(block, is_error, emit);
    }

    /// The one exit for every completed block — closed, orphaned or forced.
    /// The boot line's own reply never had a queued callback, so whichever
    /// way a boot block leaves, it is swallowed here exactly once.
    fn deliver_block<F: FnMut(Event)>(&mut self, block: OpenBlock, is_error: bool, emit: &mut F) {
        if self.awaiting_boot_block {
            self.awaiting_boot_block = false;
            return;
        }
        emit(Event::CommandResponse(CommandResponse {
            command_id: block.command_id,
            command_num: block.command_num,
            is_error,
            lines: block.lines,
            from_client: block.from_client,
        }));
    }

    // MARK: %output

    /// `%output %pane <data>`. The pane id is ASCII; the payload stays bytes
    /// throughout.
    fn handle_output<F: FnMut(Event)>(&mut self, raw: &[u8], emit: &mut F) {
        let body = &raw[OUTPUT_PREFIX.len()..];
        let pane = match memchr(b' ', body) {
            Some(sep) if sep > 0 => foundation_strict(&body[..sep]).map(|pane| (pane, sep)),
            _ => None,
        };
        let Some((pane_id, sep)) = pane else {
            emit(malformed(&String::from_utf8_lossy(raw)));
            return;
        };
        let payload = &body[sep + 1..];
        // Truncated-line recovery, output half: a lossy hop cut this line's
        // tail and newline, gluing the next protocol line onto it.
        if let Some(begin_at) = self.embedded_begin_offset(payload) {
            let begin_line = String::from_utf8_lossy(&payload[begin_at..]).into_owned();
            emit(malformed("recovered %begin glued to truncated %output"));
            emit(Event::PaneOutput {
                pane_id,
                data: decode_octal_escapes(&payload[..begin_at]),
            });
            self.handle_begin(&begin_line, emit);
            return;
        }
        emit(Event::PaneOutput {
            pane_id,
            data: decode_octal_escapes(payload),
        });
    }
}

// MARK: Notification helpers

/// `%layout-change @window <layout> [visible_layout] [flags]`.
fn handle_layout_change<F: FnMut(Event)>(line: &str, body: &str, emit: &mut F) {
    let parts = split_swift(body, 3, true);
    if parts.len() < 2 {
        emit(malformed(line));
        return;
    }
    // Fix (zoom-flags): zoomed when a window-flags field carries `Z`. tmux
    // 3.6a sends `*Z` only for the session's current window with no other
    // flag; a zoomed window that is not current reads `-Z` or `Z`, a marked
    // pane makes it `*MZ`. Swift matched `*Z` exactly and reported those
    // zoomed windows as unzoomed.
    let zoomed = parts[2..].iter().any(|part| is_zoomed_flags(part));
    emit(Event::LayoutChange {
        window_id: parts[0].to_owned(),
        layout: parts[1].to_owned(),
        zoomed,
    });
}

/// A tmux window-flags field (`#!~*-MZ`) with the zoomed flag in it. Layout
/// strings never qualify: they always contain digits.
fn is_zoomed_flags(field: &str) -> bool {
    field.contains('Z') && field.bytes().all(|b| b"#!~*-MZ".contains(&b))
}

fn emit_id<F: FnMut(Event)>(
    line: &str,
    id: &str,
    emit: &mut F,
    event: impl FnOnce(String) -> Event,
) {
    if id.is_empty() {
        emit(malformed(line));
    } else {
        emit(event(id.to_owned()));
    }
}

fn malformed(text: &str) -> Event {
    Event::ProtocolError(ControlError::MalformedLine(text.to_owned()))
}

// MARK: Text helpers

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Swift's `String(data: raw, encoding: .utf8) ?? String(decoding: raw, as:
/// UTF8.self)`: valid UTF-8 loses one leading byte-order mark (Foundation
/// drops it), anything else is decoded lossily, U+FFFD per maximal invalid
/// subpart (the same replacement both standard libraries use).
fn foundation_string(raw: &[u8]) -> String {
    match foundation_strict(raw) {
        Some(text) => text,
        None => String::from_utf8_lossy(raw).into_owned(),
    }
}

/// Swift's `String(data: raw, encoding: .utf8)`: `None` for invalid UTF-8,
/// one leading byte-order mark dropped.
fn foundation_strict(raw: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?;
    Some(text.strip_prefix('\u{FEFF}').unwrap_or(text).to_owned())
}

/// Decode a non-`%output` line and drop leading bytes before the first `%`
/// that starts a tmux keyword: transports that prepend a DCS (`\eP1000p…`)
/// or a banner to the first line.
fn decode_line(raw: &[u8]) -> String {
    let decoded = foundation_string(raw);
    if decoded.starts_with('%') {
        return decoded;
    }
    let Some(percent) = decoded.find('%') else {
        return decoded;
    };
    let candidate = &decoded[percent..];
    if KEYWORDS
        .iter()
        .any(|keyword| candidate.starts_with(keyword))
    {
        candidate.to_owned()
    } else {
        decoded
    }
}

/// `Int(text)` in Swift: optional sign, ASCII digits, no overflow.
fn parse_int(text: &str) -> Option<i64> {
    text.parse().ok()
}

/// Swift's `split(separator: " ", maxSplits:, omittingEmptySubsequences:)`,
/// step for step: with omission on, only non-empty pieces count towards
/// `max_splits`, and the remainder after the last split is kept as is.
fn split_swift(text: &str, max_splits: usize, omit_empty: bool) -> Vec<&str> {
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
        if bytes[at] != b' ' {
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

// MARK: Octal escapes

/// tmux's `%output` payload spells bytes outside printable ASCII, and the
/// backslash itself, as `\NNN` (three octal digits); older versions also
/// send `\\`. Everything else passes through. Digits above `\377` wrap, the
/// way the Swift decoder's `UInt8` arithmetic does; a lone backslash stays.
pub fn decode_octal_escapes(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    while let Some(offset) = memchr(b'\\', &raw[at..]) {
        let slash = at + offset;
        out.extend_from_slice(&raw[at..slash]);
        let rest = &raw[slash + 1..];
        match rest {
            [d0, d1, d2, ..] if is_octal(*d0) && is_octal(*d1) && is_octal(*d2) => {
                let value =
                    u32::from(d0 - b'0') * 64 + u32::from(d1 - b'0') * 8 + u32::from(d2 - b'0');
                out.push(value as u8);
                at = slash + 4;
            }
            [b'\\', ..] => {
                out.push(b'\\');
                at = slash + 2;
            }
            _ => {
                out.push(b'\\');
                at = slash + 1;
            }
        }
    }
    out.extend_from_slice(&raw[at..]);
    out
}

fn is_octal(byte: u8) -> bool {
    (b'0'..=b'7').contains(&byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_swift_matches_the_swift_standard_library() {
        // Expected values from Swift 6.4's `split(separator:maxSplits:
        // omittingEmptySubsequences:)` on the same strings.
        assert_eq!(split_swift("a  b c  d", 3, true), ["a", "b", "c", " d"]);
        assert_eq!(split_swift("a b c ", 3, true), ["a", "b", "c"]);
        assert_eq!(split_swift(" $1 @2", 1, true), ["$1", "@2"]);
        assert_eq!(split_swift("$1  @2", 1, true), ["$1", " @2"]);
        assert_eq!(split_swift("$1 ", 1, true), ["$1"]);
        assert_eq!(split_swift("", 4, true), Vec::<&str>::new());
        assert_eq!(split_swift("", 4, false), [""]);
        assert_eq!(split_swift("%end  5 2", 4, false), ["%end", "", "5", "2"]);
        assert_eq!(
            split_swift("a b c d e f", 4, false),
            ["a", "b", "c", "d", "e f"]
        );
        assert_eq!(split_swift("x", 0, true), ["x"]);
    }

    #[test]
    fn parse_int_matches_swift_int_init() {
        assert_eq!(parse_int("42"), Some(42));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("-0"), Some(0));
        assert_eq!(parse_int("007"), Some(7));
        for bad in ["", "+", "-", " 1", "1 ", "0x1", "١", "99999999999999999999"] {
            assert_eq!(parse_int(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn foundation_decoding_drops_one_leading_bom_from_valid_utf8_only() {
        assert_eq!(foundation_string(b"\xef\xbb\xbfabc"), "abc");
        assert_eq!(foundation_string(b"\xef\xbb\xbf\xef\xbb\xbfx"), "\u{feff}x");
        assert_eq!(
            foundation_string(b"\xef\xbb\xbfab\xff"),
            "\u{feff}ab\u{fffd}"
        );
        assert_eq!(foundation_string(b"q\xef\xbb\xbfr"), "q\u{feff}r");
        assert_eq!(foundation_strict(b"\xff"), None);
    }

    #[test]
    fn octal_values_past_377_wrap_like_swift() {
        assert_eq!(decode_octal_escapes(br"\400\777"), [0x00, 0xFF]);
        assert_eq!(decode_octal_escapes(br"\12"), br"\12");
        assert_eq!(decode_octal_escapes(br"a\\\134b\"), br"a\\b\");
    }
}
