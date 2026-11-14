//! Port of Tests/MoshpitTests/Services/TmuxControlClientParserTests.swift
//! (suite "TmuxControlClient parser", 24 tests) and of the `%session-renamed`
//! test in TmuxSidecarSwitchTests.swift. Each test keeps the Swift
//! function's name, in snake case, and its `@Test` title as the doc comment.

use moshpit_tmux::{CommandResponse, ControlError, ControlParser, Event, decode_octal_escapes};

/// Boot banner: every control session's first block is the boot line's own
/// reply and is swallowed by the parser (no event).
const BOOT: &[u8] = b"%begin 900 500 0\n%end 900 500 0\n";

/// Collects every event the parser emits, like the Swift `CallbackRecorder`.
struct Recorder {
    parser: ControlParser,
    events: Vec<Event>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            parser: ControlParser::new(),
            events: Vec::new(),
        }
    }

    fn booted() -> Self {
        let mut recorder = Self::new();
        recorder.feed(BOOT);
        recorder
    }

    fn feed(&mut self, data: &[u8]) {
        let events = &mut self.events;
        self.parser.feed(data, |event| events.push(event));
    }
}

fn output(pane: &str, data: &[u8]) -> Event {
    Event::PaneOutput {
        pane_id: pane.into(),
        data: data.to_vec(),
    }
}

/// A reply block's event; `%begin … 0` (as these inputs write them): not
/// from this client's input (B273).
fn response(id: i64, num: i64, is_error: bool, lines: &[&str]) -> Event {
    Event::CommandResponse(CommandResponse {
        command_id: id,
        command_num: num,
        is_error,
        lines: lines.iter().map(|line| line.to_string()).collect(),
        from_client: false,
    })
}

/// The same for a `%begin … 1` block.
fn ours(id: i64, num: i64, is_error: bool, lines: &[&str]) -> Event {
    match response(id, num, is_error, lines) {
        Event::CommandResponse(response) => Event::CommandResponse(CommandResponse {
            from_client: true,
            ..response
        }),
        other => other,
    }
}

// MARK: Output

/// %output without escapes routes raw bytes to onPaneOutput
#[test]
fn plain_output() {
    let mut rec = Recorder::booted();
    rec.feed(b"%output %0 hello world\n");
    assert_eq!(rec.events, [output("%0", b"hello world")]);
}

/// %output decodes \NNN octal escapes
#[test]
fn octal_escape_decoding() {
    let mut rec = Recorder::booted();
    // `\033` = ESC (0x1B), `\007` = BEL (0x07).
    rec.feed(b"%output %1 \\033[31mred\\033[0m\\007\n");
    assert_eq!(rec.events, [output("%1", b"\x1b[31mred\x1b[0m\x07")]);
}

/// %output handles \\ as a literal backslash
#[test]
fn escaped_backslash() {
    let mut rec = Recorder::booted();
    rec.feed(b"%output %2 path\\\\to\\\\file\n");
    assert_eq!(rec.events, [output("%2", br"path\to\file")]);
}

/// a UTF-8 char split across two %output events survives byte-exact
#[test]
fn split_utf8_across_output_events() {
    // tmux flushes pane output mid-character: a 3-byte `─` (e2 94 80) can
    // end one %output event after `e2 94` and start the next with `80`.
    // Each line alone is invalid UTF-8; the payloads must come out
    // byte-exact so the terminal's own decoder can reassemble them.
    let mut rec = Recorder::booted();
    let mut first = b"%output %5 ".to_vec();
    first.extend_from_slice(&[0xE2, 0x94, 0x80, 0xE2, 0x94]); // ─ + partial ─
    first.push(b'\n');
    let mut second = b"%output %5 ".to_vec();
    second.extend_from_slice(&[0x80, 0xE2, 0x94, 0x80]); // completes it + one more ─
    second.push(b'\n');

    rec.feed(&first);
    rec.feed(&second);

    assert_eq!(
        rec.events,
        [
            output("%5", &[0xE2, 0x94, 0x80, 0xE2, 0x94]),
            output("%5", &[0x80, 0xE2, 0x94, 0x80]),
        ]
    );
    // Joined: three intact box-drawing characters.
    let joined: Vec<u8> = rec
        .events
        .iter()
        .flat_map(|event| match event {
            Event::PaneOutput { data, .. } => data.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(String::from_utf8(joined).unwrap(), "───");
}

/// raw CJK bytes in %output pass through without String mangling
#[test]
fn raw_cjk_output() {
    let mut rec = Recorder::booted();
    rec.feed("%output %6 测试中文\n".as_bytes());
    assert_eq!(rec.events, [output("%6", "测试中文".as_bytes())]);
}

// MARK: Block framing

/// %begin / response lines / %end collapse to a single onCommandResponse
#[test]
fn happy_path_command_response() {
    let mut rec = Recorder::booted();
    rec.feed(
        b"%begin 1234 7 0\n\
          @0 main 0 81x24,0,0,0 1 1\n\
          @1 logs 1 81x24,0,0,1 0 1\n\
          %end 1234 7 0\n",
    );
    assert_eq!(
        rec.events,
        [response(
            1234,
            7,
            false,
            &["@0 main 0 81x24,0,0,0 1 1", "@1 logs 1 81x24,0,0,1 0 1"]
        )]
    );
}

/// block content lines containing a %keyword substring are kept verbatim
#[test]
fn block_lines_with_percent_keyword_not_truncated() {
    // capture-pane scrollback can contain text that merely includes a tmux
    // %keyword ("80%done", "x%window"); it must round-trip untouched.
    let mut rec = Recorder::booted();
    rec.feed(
        b"%begin 5 2 0\n\
          build is 80%done now\n\
          cpu x%window load\n\
          plain ascii line\n\
          100%end of run\n\
          %end 5 2 0\n",
    );
    assert_eq!(
        rec.events,
        [response(
            5,
            2,
            false,
            &[
                "build is 80%done now",
                "cpu x%window load",
                "plain ascii line",
                "100%end of run"
            ]
        )]
    );
}

/// CJK + box-drawing content survives a block round-trip byte-for-byte
#[test]
fn block_lines_preserve_wide_chars() {
    let mut rec = Recorder::booted();
    rec.feed("%begin 6 3 0\n你好世界 ┌─┐ │x│ └─┘\n%end 6 3 0\n".as_bytes());
    assert_eq!(
        rec.events,
        [response(6, 3, false, &["你好世界 ┌─┐ │x│ └─┘"])]
    );
}

/// %error closes the block with isError=true
#[test]
fn error_block() {
    let mut rec = Recorder::booted();
    rec.feed(b"%begin 99 1 0\nno server running\n%error 99 1 0\n");
    assert_eq!(rec.events, [response(99, 1, true, &["no server running"])]);
}

/// response lines that themselves start with % are captured, not parsed as notifications
#[test]
fn response_lines_starting_with_percent() {
    // list-panes returns lines like `%160 @84 …`.
    let mut rec = Recorder::booted();
    rec.feed(
        b"%begin 1 1 0\n\
          %160 @84 bash 80 24 1\n\
          %161 @84 vim 80 24 0\n\
          %end 1 1 0\n",
    );
    assert_eq!(
        rec.events,
        [response(
            1,
            1,
            false,
            &["%160 @84 bash 80 24 1", "%161 @84 vim 80 24 0"]
        )]
    );
}

// MARK: Notifications

/// Window + session + layout notifications fire in order
#[test]
fn notification_stream() {
    let mut rec = Recorder::booted();
    rec.feed(
        b"%session-changed $0 main\n\
          %window-add @5\n\
          %window-renamed @5 logs\n\
          %layout-change @5 81x24,0,0,8 *Z\n\
          %window-pane-changed @5 %12\n\
          %session-window-changed $0 @5\n\
          %window-close @5\n",
    );
    assert_eq!(
        rec.events,
        [
            Event::SessionChanged {
                session_id: "$0".into(),
                name: "main".into()
            },
            Event::WindowAdd {
                window_id: "@5".into()
            },
            Event::WindowRenamed {
                window_id: "@5".into(),
                name: "logs".into()
            },
            Event::LayoutChange {
                window_id: "@5".into(),
                layout: "81x24,0,0,8".into(),
                zoomed: true
            },
            Event::ActivePaneChanged {
                window_id: "@5".into(),
                pane_id: "%12".into()
            },
            Event::SessionWindowChanged {
                session_id: "$0".into(),
                window_id: "@5".into()
            },
            Event::WindowClose {
                window_id: "@5".into()
            },
        ]
    );
}

/// %pause and %continue route to onPause / onContinue
#[test]
fn pause_continue() {
    let mut rec = Recorder::booted();
    rec.feed(b"%pause %4\n%continue %4\n");
    assert_eq!(
        rec.events,
        [
            Event::Pause {
                pane_id: "%4".into()
            },
            Event::Continue {
                pane_id: "%4".into()
            },
        ]
    );
}

/// %exit with and without reason
#[test]
fn exit_directive() {
    let mut rec = Recorder::booted();
    rec.feed(b"%exit\n%exit detached\n");
    assert_eq!(
        rec.events,
        [
            Event::Exit { reason: None },
            Event::Exit {
                reason: Some("detached".into())
            },
        ]
    );
}

// MARK: The rest of the server

/// notifications about other sessions, windows and clients reach their callbacks
#[test]
fn server_notifications() {
    let mut rec = Recorder::booted();
    // Lines as tmux 3.6a sends them to a -CC client (private socket;
    // captures/session-switch.bin and layout.bin).
    rec.feed(
        b"%unlinked-window-add @5\n\
          %sessions-changed\n\
          %unlinked-window-renamed @5 my window\n\
          %client-session-changed /dev/ttys024 $1 my project\n\
          %client-session-changed client-4242 $0 a|b\n\
          %unlinked-window-close @5\n",
    );
    assert_eq!(
        rec.events,
        [
            Event::UnlinkedWindowAdd {
                window_id: "@5".into()
            },
            Event::SessionsChanged,
            Event::UnlinkedWindowRenamed {
                window_id: "@5".into(),
                name: "my window".into()
            },
            Event::ClientSessionChanged {
                client: "/dev/ttys024".into(),
                session_id: "$1".into(),
                name: "my project".into()
            },
            Event::ClientSessionChanged {
                client: "client-4242".into(),
                session_id: "$0".into(),
                name: "a|b".into()
            },
            Event::UnlinkedWindowClose {
                window_id: "@5".into()
            },
        ]
    );
}

/// a notification about the rest of the server that is cut short is a protocol error
#[test]
fn malformed_server_notification() {
    for line in [
        "%unlinked-window-add ",
        "%unlinked-window-close ",
        "%unlinked-window-renamed @5",
        "%client-session-changed /dev/ttys024 $1",
        "%client-session-changed  $1 x",
        "%client-session-changed /dev/ttys024  x",
    ] {
        let mut rec = Recorder::booted();
        rec.feed(format!("{line}\n").as_bytes());
        assert_eq!(
            rec.events,
            [Event::ProtocolError(ControlError::MalformedLine(
                line.into()
            ))],
            "{line:?}"
        );
    }
}

/// %session-renamed hands over the id and the whole new name
#[test]
fn session_renamed() {
    for (line, id, name) in [
        ("%session-renamed $1 x|y\n", "$1", "x|y"),
        ("%session-renamed $12 my project\n", "$12", "my project"),
        ("%session-renamed $3 \u{301}acute\n", "$3", "\u{301}acute"),
    ] {
        let mut rec = Recorder::booted();
        rec.feed(line.as_bytes());
        assert_eq!(
            rec.events,
            [Event::SessionRenamed {
                session_id: id.into(),
                name: name.into()
            }],
            "{line:?}"
        );
    }
}

// MARK: Forward-compat + errors

/// Unknown %foo directives are silently ignored
#[test]
fn unknown_directive_ignored() {
    let mut rec = Recorder::booted();
    rec.feed(b"%this-is-from-the-future @0 some payload\n");
    assert_eq!(rec.events, []);
}

/// Malformed %output (no pane id) raises onProtocolError
#[test]
fn malformed_output() {
    let mut rec = Recorder::booted();
    rec.feed(b"%output without-pane-id\n");
    // The body text isn't contract; that some protocol error fires is.
    assert!(
        matches!(
            rec.events.first(),
            Some(Event::ProtocolError(ControlError::MalformedLine(_)))
        ),
        "expected a malformed-line protocol error, got {:?}",
        rec.events.first()
    );
}

// MARK: Buffer across feeds

/// Bytes split across feed() calls still produce one whole-line event
#[test]
fn chunked_across_feeds() {
    let mut rec = Recorder::booted();
    // Split in the middle of the payload — the first chunk has no newline.
    rec.feed(b"%output %0 hel");
    assert_eq!(rec.events, [], "no newline yet, the event must wait");
    rec.feed(b"lo world\n");
    assert_eq!(rec.events, [output("%0", b"hello world")]);
}

/// CRLF line endings are stripped just like LF
#[test]
fn crlf_handling() {
    let mut rec = Recorder::booted();
    rec.feed(b"%window-add @9\r\n");
    assert_eq!(
        rec.events,
        [Event::WindowAdd {
            window_id: "@9".into()
        }]
    );
}

// MARK: Octal decoder primitive

/// decodeOctalEscapes passes plain bytes through
#[test]
fn decode_octal_identity() {
    assert_eq!(decode_octal_escapes(b"hello"), b"hello");
}

/// decodeOctalEscapes converts every valid \NNN sequence
#[test]
fn decode_octal_all_values() {
    for (escape, value) in [
        (r"\000", 0x00u8),
        (r"\007", 0x07),
        (r"\033", 0x1B),
        (r"\177", 0x7F),
        (r"\377", 0xFF),
    ] {
        assert_eq!(
            decode_octal_escapes(escape.as_bytes()),
            [value],
            "{escape} should decode to {value:#04x}"
        );
    }
}

/// reset() drops a buffered partial line so subsequent feed reparses
#[test]
fn reset_clears_buffer() {
    let mut rec = Recorder::booted();
    rec.feed(b"%output %0 nev");
    rec.parser.reset();
    rec.feed(b"er-mind\n%window-add @1\n");
    // "er-mind" no longer completes a recognised line (it doesn't start
    // with %), so it is ignored; the %window-add still fires.
    assert_eq!(
        rec.events,
        [Event::WindowAdd {
            window_id: "@1".into()
        }]
    );
}

/// a content line shaped like %end with foreign ids stays inside the block
#[test]
fn mismatched_terminator_stays_content() {
    // A capture-pane frame of a pane that itself shows control-mode text
    // contains literal "%end …" lines. Only the terminator echoing the OPEN
    // block's ids may close it.
    let mut rec = Recorder::booted();
    rec.feed(
        b"%begin 1234 7 0\n\
          %end 999 42 0\n\
          still the same frame\n\
          %end 1234 7 0\n",
    );
    assert_eq!(
        rec.events,
        [response(
            1234,
            7,
            false,
            &["%end 999 42 0", "still the same frame"]
        )]
    );
}

/// every control session's boot block is swallowed — including after a failed preferred attach
#[test]
fn failed_preferred_attach_chain_keeps_pairing() {
    // A preferred-session boot chain (`tmux -CC attach -t 'x' || …`) emits
    // a full %begin/%error/%exit for the failed attach and a fresh banner
    // for the attach that succeeds. Both are replies to boot lines nobody
    // queued a callback for. Deliberately no banner prefix here.
    let mut rec = Recorder::new();
    rec.feed(
        b"%begin 100 530 0\n\
          can't find session: nonexistent\n\
          %error 100 530 0\n\
          %exit\n\
          %begin 100 532 0\n\
          %end 100 532 0\n\
          %session-changed $0 real\n\
          %begin 100 535 1\n\
          real reply\n\
          %end 100 535 1\n",
    );
    assert_eq!(
        rec.events,
        [
            Event::Exit { reason: None },
            Event::SessionChanged {
                session_id: "$0".into(),
                name: "real".into()
            },
            ours(100, 535, false, &["real reply"]),
        ]
    );
}
