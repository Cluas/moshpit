//! Behaviour the Swift suites never pinned down: truncated-line recovery,
//! the runaway-block cap, boot-block swallowing across resets, Foundation's
//! decoding quirks, the five documented fixes, and chunking invariance.

use moshpit_tmux::harness::SplitMix64;
use moshpit_tmux::{CommandResponse, ControlError, ControlParser, Event, MAX_BLOCK_LINES};

const T: i64 = 1_700_000_000;

/// A parser past its boot block, whose last real block is (T, 100).
fn booted() -> ControlParser {
    let mut parser = ControlParser::new();
    assert_eq!(
        parser.feed_collect(b"%begin 1700000000 100 0\r\n%end 1700000000 100 0\r\n"),
        []
    );
    parser
}

/// A `%begin … 1` block's event (these inputs' replies are flagged ours).
fn response(num: i64, is_error: bool, lines: &[&str]) -> Event {
    Event::CommandResponse(CommandResponse {
        command_id: T,
        command_num: num,
        is_error,
        lines: lines.iter().map(|line| line.to_string()).collect(),
        from_client: true,
    })
}

fn error(message: &str) -> Event {
    Event::ProtocolError(ControlError::MalformedLine(message.into()))
}

fn output(pane: &str, data: &[u8]) -> Event {
    Event::PaneOutput {
        pane_id: pane.into(),
        data: data.to_vec(),
    }
}

fn window_add(id: &str) -> Event {
    Event::WindowAdd {
        window_id: id.into(),
    }
}

fn layout(zoomed: bool) -> Event {
    Event::LayoutChange {
        window_id: "@1".into(),
        layout: "b25d,80x24,0,0,0".into(),
        zoomed,
    }
}

// MARK: Truncated-line recovery

#[test]
fn glued_terminator_closes_its_block_and_keeps_the_content() {
    let mut parser = booted();
    let events = parser.feed_collect(
        b"%begin 1700000000 101 1\r\nline one\r\ncut short%end 1700000000 101 1\r\n\
          %begin 1700000000 102 1\r\nglued%error 1700000000 102 1\r\n",
    );
    assert_eq!(
        events,
        [
            response(101, false, &["line one", "cut short"]),
            response(102, true, &["glued"]),
        ]
    );
}

#[test]
fn glued_begin_after_a_truncated_output_opens_its_block() {
    let mut parser = booted();
    let events = parser.feed_collect(
        b"%output %0 before the cut%begin 1700000001 101 1\r\nreply\r\n%end 1700000001 101 1\r\n",
    );
    assert_eq!(
        events,
        [
            error("recovered %begin glued to truncated %output"),
            output("%0", b"before the cut"),
            Event::CommandResponse(CommandResponse {
                command_id: T + 1,
                command_num: 101,
                is_error: false,
                lines: vec!["reply".into()],
                from_client: true,
            }),
        ]
    );
}

#[test]
fn begin_text_that_is_not_a_plausible_next_block_stays_pane_output() {
    let mut parser = booted();
    for payload in [
        "quoting %begin 1700000000 5 1",   // stale number
        "future %begin 1700000000 301 1",  // more than 200 ahead
        "far %begin 1600000000 101 1",     // a day away
        "two %begin 1700000000 101 10",    // flag of two digits
        "extra %begin 1700000000 101 1 x", // five fields
        "%begin 1700000000 101 1",         // at payload start
        "plus %begin +1700000000 +101 +1", // a signed flag
    ] {
        let line = format!("%output %0 {payload}\r\n");
        assert_eq!(
            parser.feed_collect(line.as_bytes()),
            [output("%0", payload.as_bytes())],
            "{payload}"
        );
    }
    // Before any real block the window is measured from (0, 0), which no
    // tmux time field is near.
    let mut fresh = ControlParser::new();
    assert_eq!(
        fresh.feed_collect(b"%output %0 x%begin 1700000000 1 1\r\n"),
        [output("%0", b"x%begin 1700000000 1 1")]
    );
}

#[test]
fn reset_keeps_the_recovery_window() {
    let mut parser = booted();
    parser.reset();
    let events = parser.feed_collect(
        b"%output %0 x%begin 1700000000 101 1\r\n%end 1700000000 101 1\r\n%window-add @1\r\n",
    );
    // Recovered against the ids from before the reset; the block it opens
    // is the new session's boot reply and is swallowed.
    assert_eq!(
        events,
        [
            error("recovered %begin glued to truncated %output"),
            output("%0", b"x"),
            window_add("@1"),
        ]
    );
}

// MARK: Runaway blocks

#[test]
fn a_block_that_never_ends_is_forced_closed_at_the_cap() {
    let mut parser = booted();
    let mut stream = b"%begin 1700000000 101 1\r\n".to_vec();
    for _ in 0..MAX_BLOCK_LINES {
        stream.extend_from_slice(b"x\r\n");
    }
    stream.extend_from_slice(b"%window-add @9\r\n");
    let events = parser.feed_collect(&stream);
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0],
        error("block #1700000000 exceeded 5000 lines — forcing close")
    );
    assert!(matches!(&events[1],
        Event::CommandResponse(r) if r.is_error && r.lines.len() == MAX_BLOCK_LINES));
    // The line that tripped the cap is dispatched again, outside the block.
    assert_eq!(events[2], window_add("@9"));
}

// MARK: Boot blocks

#[test]
fn every_session_swallows_exactly_one_boot_block() {
    let mut parser = ControlParser::new();
    let events = parser.feed_collect(
        b"%begin 1 1 0\r\n%end 1 1 0\r\n%begin 1 2 1\r\na\r\n%end 1 2 1\r\n\
          %exit server exited\r\n%begin 1 3 0\r\n%end 1 3 0\r\n%begin 1 4 1\r\n%end 1 4 1\r\n",
    );
    assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], Event::CommandResponse(r) if r.command_num == 2));
    assert_eq!(
        events[1],
        Event::Exit {
            reason: Some("server exited".into())
        }
    );
    assert!(matches!(&events[2], Event::CommandResponse(r) if r.command_num == 4));
}

#[test]
fn a_boot_block_is_swallowed_even_when_forced_closed() {
    let mut parser = ControlParser::new();
    let mut stream = b"%begin 1 1 0\r\n".to_vec();
    for _ in 0..=MAX_BLOCK_LINES {
        stream.extend_from_slice(b"x\r\n");
    }
    let events = parser.feed_collect(&stream);
    assert_eq!(
        events,
        [error("block #1 exceeded 5000 lines — forcing close")]
    );
}

// MARK: Foundation's decoding

#[test]
fn a_leading_bom_is_dropped_from_valid_utf8_like_foundation_does() {
    let mut parser = booted();
    let events = parser.feed_collect(
        b"%begin 1700000000 101 1\r\n\xef\xbb\xbfbom\r\n\xef\xbb\xbfbad\xff\r\n%end 1700000000 101 1\r\n\
          %output \xef\xbb\xbf%3 x\r\n",
    );
    assert_eq!(
        events,
        [
            response(101, false, &["bom", "\u{feff}bad\u{fffd}"]),
            output("%3", b"x"),
        ]
    );
}

// MARK: Documented fixes (harness/divergences.json)

/// Fix (overflow-trap): Swift traps on `abs(cid - lastBlockCid)`.
#[test]
fn fix_a_huge_negative_glued_id_is_pane_output_not_a_crash() {
    let mut parser = booted();
    let payload = "x%begin -9223372036854775808 101 1";
    assert_eq!(
        parser.feed_collect(format!("%output %0 {payload}\r\n").as_bytes()),
        [output("%0", payload.as_bytes())]
    );
    let payload = "x%begin 9223372036854775807 101 1";
    assert_eq!(
        parser.feed_collect(format!("%output %0 {payload}\r\n").as_bytes()),
        [output("%0", payload.as_bytes())]
    );
}

/// Fix (block-cap-terminator): a block of exactly MAX_BLOCK_LINES lines
/// closes on its own terminator.
#[test]
fn fix_a_block_at_the_line_cap_closes_on_its_own_terminator() {
    let mut parser = booted();
    let mut stream = b"%begin 1700000000 101 1\r\n".to_vec();
    for _ in 0..MAX_BLOCK_LINES {
        stream.extend_from_slice(b"\r\n");
    }
    stream.extend_from_slice(b"%end 1700000000 101 1\r\n%window-add @1\r\n");
    let events = parser.feed_collect(&stream);
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0],
        Event::CommandResponse(r) if !r.is_error && r.lines.len() == MAX_BLOCK_LINES));
    assert_eq!(events[1], window_add("@1"));
}

/// Fix (terminator-number-boundary): `%end T 10` is not found inside
/// `%end T 105`.
#[test]
fn fix_a_longer_command_number_does_not_close_the_block() {
    let mut parser = ControlParser::new();
    parser.feed_collect(b"%begin 1700000000 9 0\r\n%end 1700000000 9 0\r\n");
    let events = parser.feed_collect(
        b"%begin 1700000000 10 1\r\n\
          log: %end 1700000000 105 1\r\n\
          log: %error 1700000000 107\r\n\
          bar\r\n\
          %end 1700000000 10 1\r\n",
    );
    assert_eq!(
        events,
        [Event::CommandResponse(CommandResponse {
            command_id: T,
            command_num: 10,
            is_error: false,
            lines: vec![
                "log: %end 1700000000 105 1".into(),
                "log: %error 1700000000 107".into(),
                "bar".into()
            ],
            from_client: true,
        })]
    );
}

/// Fix (unicode-syntax): protocol delimiters are bytes, digits are ASCII.
#[test]
fn fix_protocol_syntax_is_matched_on_bytes_and_ascii_digits() {
    let mut parser = booted();
    let events = parser
        .feed_collect("%window-renamed @1 \u{301}x\r\n%session-changed $1 \u{301}s\r\n".as_bytes());
    assert_eq!(
        events,
        [
            Event::WindowRenamed {
                window_id: "@1".into(),
                name: "\u{301}x".into()
            },
            Event::SessionChanged {
                session_id: "$1".into(),
                name: "\u{301}s".into()
            },
        ]
    );
    // A CJK numeral is no flag digit: no phantom block.
    let payload = "x%begin 1700000000 101 \u{4e94}";
    assert_eq!(
        parser.feed_collect(format!("%output %0 {payload}\r\n").as_bytes()),
        [output("%0", payload.as_bytes())]
    );
    // An Arabic-Indic digit is no flag either: the block stays open.
    let events = parser.feed_collect(
        "%begin 1700000000 102 1\r\nfoo%end 1700000000 102 \u{663}\r\n%end 1700000000 102 1\r\n"
            .as_bytes(),
    );
    assert_eq!(
        events,
        [response(102, false, &["foo%end 1700000000 102 \u{663}"])]
    );
}

/// Fix (zoom-flags): any window-flags field with Z means zoomed.
#[test]
fn fix_every_zoomed_flags_field_reads_as_zoomed() {
    let mut parser = booted();
    for (flags, zoomed) in [
        ("*Z", true),
        ("-Z", true),
        ("Z", true),
        ("*MZ", true),
        ("#*Z", true),
        ("!-Z", true),
        ("*", false),
        ("-", false),
        ("", false),
        ("*Zx", false),
    ] {
        let line = format!("%layout-change @1 b25d,80x24,0,0,0 b25d,80x24,0,0,0 {flags}\r\n");
        assert_eq!(
            parser.feed_collect(line.as_bytes()),
            [layout(zoomed)],
            "{flags:?}"
        );
    }
    // The three-field form of older tmux.
    assert_eq!(
        parser.feed_collect(b"%layout-change @1 b25d,80x24,0,0,0 -Z\r\n"),
        [layout(true)]
    );
}

// MARK: Chunking

/// A stream of every line shape, cut at random: the events never depend on
/// where the feeds split it.
#[test]
fn chunking_never_changes_the_events() {
    let mut rng = SplitMix64(7);
    let shapes: [&[u8]; 12] = [
        b"%output %1 plain \\033[1mbold\\033[0m \\134 \xe6\xb5\x8b\xe8\xaf\x95\r\n",
        b"%output %2 \xe2\x94\r\n%output %2 \x80\xf0\x9f\x98\x80\r\n",
        b"%begin 1700000000 101 1\r\n\xe4\xbd\xa0\xe5\xa5\xbd \xff\r\n%end 1700000000 101 1\r\n",
        b"%begin 1700000000 101 1\r\n%end 9 9 9\r\ncut%end 1700000000 101 1\r\n",
        b"%layout-change @1 b25d,80x24,0,0,0 b25d,80x24,0,0,0 -Z\r\n",
        b"%window-renamed @2 \xe6\xb1\x89\xe5\xad\x97\r\n",
        b"%session-changed $1 main\n",
        b"\x1bP1000p%window-add @3\r\n",
        b"%exit\r\n%begin 1 1 0\r\n%end 1 1 0\r\n",
        b"%output %0 x%begin 1700000000 102 1\r\nglued\r\n%end 1700000000 102 1\r\n",
        b"garbage line\r\n\r\n%pause %1\r\n%continue %1\r\n",
        b"%end 5 5 5\r\n%client-detached /dev/ttys1\r\n",
    ];
    let mut stream = b"%begin 1700000000 100 0\r\n%end 1700000000 100 0\r\n".to_vec();
    for _ in 0..3000 {
        stream.extend_from_slice(shapes[(rng.next_u64() % shapes.len() as u64) as usize]);
    }
    let whole = ControlParser::new().feed_collect(&stream);
    assert!(whole.len() > 3000);
    for max in [1u64, 2, 7, 64, 1500] {
        let mut parser = ControlParser::new();
        let mut events = Vec::new();
        let mut at = 0;
        while at < stream.len() {
            let end = stream.len().min(at + 1 + (rng.next_u64() % max) as usize);
            parser.feed(&stream[at..end], |event| events.push(event));
            at = end;
        }
        assert!(events == whole, "feeds of up to {max} bytes");
    }
}
