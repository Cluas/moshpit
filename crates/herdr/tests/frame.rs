//! The frame channel's wire handling: what is parsed out of the stream, and
//! what is put back into it. Ported from
//! `Tests/MoshpitTests/Services/Herdr/HerdrFrameChannelTests.swift`, plus the
//! paste and arrow-key statics of `SessionHub` (`HerdrSSHFrameModeTests`,
//! `HerdrControlClientTests.arrowKeyNameMapping`) and the port's own fixes.

use moshpit_herdr::frame::{
    self, Frame, FrameParser, MouseAction, MouseButton, READY_MARKER, RELEASE, ScrollSource,
};
use moshpit_herdr::json;

fn frame_line(seq: u64, width: i64, height: i64, full: bool, payload: &[u8]) -> String {
    format!(
        r#"{{"type":"terminal.frame","seq":{seq},"encoding":"ansi","width":{width},"height":{height},"full":{full},"bytes":"{}"}}"#,
        moshpit_herdr::base64::encode(payload)
    )
}

fn line(seq: u64, payload: &str) -> String {
    frame_line(seq, 60, 20, false, payload.as_bytes())
}

fn object(text: &str) -> json::Object {
    json::parse_object(text.as_bytes()).expect("a JSON object")
}

// MARK: - Parsing

#[test]
fn a_whole_frame_line_decodes_to_ready_to_feed_bytes() {
    let mut parser = FrameParser::new();
    let frames =
        parser.consume(format!("{}\n", frame_line(1, 60, 20, true, b"\x1b[2Jhi")).as_bytes());
    assert_eq!(
        frames,
        vec![Frame::Screen {
            seq: 1,
            width: 60,
            height: 20,
            full: true,
            bytes: b"\x1b[2Jhi".to_vec(),
        }]
    );
}

#[test]
fn frames_split_across_reads_are_reassembled() {
    let mut parser = FrameParser::new();
    let bytes = format!("{}\n", line(7, "abc")).into_bytes();
    let cut = bytes.len() / 3;
    assert!(parser.consume(&bytes[..cut]).is_empty());
    assert!(parser.consume(&bytes[cut..cut * 2]).is_empty());
    let frames = parser.consume(&bytes[cut * 2..]);
    let [Frame::Screen { seq, bytes, .. }] = frames.as_slice() else {
        panic!("expected one screen frame, got {frames:?}");
    };
    assert_eq!(*seq, 7);
    assert_eq!(bytes, b"abc");
}

#[test]
fn several_frames_in_one_read_all_come_back_in_order() {
    let mut parser = FrameParser::new();
    let blob = format!("{}\n{}\n", line(1, "a"), line(2, "b"));
    let seqs: Vec<u64> = parser
        .consume(blob.as_bytes())
        .iter()
        .map(|frame| match frame {
            Frame::Screen { seq, .. } => *seq,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(seqs, [1, 2]);
}

#[test]
fn terminal_closed_is_surfaced_so_the_session_can_drop_its_target() {
    let mut parser = FrameParser::new();
    let frames = parser.consume(b"{\"type\":\"terminal.closed\",\"reason\":\"pane gone\"}\n");
    assert_eq!(
        frames,
        vec![Frame::Closed {
            reason: Some("pane gone".to_owned())
        }]
    );
    // herdr writes a null reason when it has none.
    let frames = parser.consume(b"{\"reason\":null,\"type\":\"terminal.closed\"}\n");
    assert_eq!(frames, vec![Frame::Closed { reason: None }]);
}

// MARK: - Tolerating the shell it runs inside

/// The channel lives in a login shell on a PTY: macOS prints "The default
/// interactive shell is now zsh." and the command echoes once before
/// `stty -echo` takes effect.
#[test]
fn shell_banners_and_prompts_between_frames_are_skipped() {
    let mut parser = FrameParser::new();
    let blob = format!(
        "The default interactive shell is now zsh.\r\nstty raw -echo; herdr terminal session control w1:p1\r\n{}\n",
        line(1, "ok")
    );
    assert_eq!(parser.consume(blob.as_bytes()).len(), 1);
}

/// Verbatim from herdr 0.8.0 talking to a 0.7.3 server: plain text, CRLF.
/// Dropping it left a black pane with no story.
#[test]
fn a_rejected_handshake_reaches_the_app_as_a_fault_not_silence() {
    let mut parser = FrameParser::new();
    let frames = parser.consume(
        b"herdr: server rejected handshake (version 16): client version 19 is newer than server version 16; please upgrade the herdr server\r\n",
    );
    let [Frame::Fault { message }] = frames.as_slice() else {
        panic!("expected a fault, got {frames:?}");
    };
    assert!(message.starts_with("server rejected handshake"));
    assert!(!message.ends_with('\r'));
}

#[test]
fn a_json_error_envelope_is_a_fault_too() {
    let mut parser = FrameParser::new();
    let frames = parser.consume(
        br#"{"id":"cli:api:snapshot","error":{"code":"protocol_mismatch","message":"client protocol 19 is newer than server protocol 16\nRun herdr server stop"}}
"#,
    );
    // First line only: the CLI's remediation prose is sized for a terminal.
    assert_eq!(
        frames,
        vec![Frame::Fault {
            message: "client protocol 19 is newer than server protocol 16".to_owned()
        }]
    );
}

#[test]
fn a_prompt_glued_to_the_front_of_a_frame_line_still_parses() {
    let mut parser = FrameParser::new();
    let blob = format!("dev@host ~ % {}\n", line(3, "z"));
    assert_eq!(parser.consume(blob.as_bytes()).len(), 1);
}

/// What makes retargeting safe without resetting the parser.
#[test]
fn a_truncated_line_costs_one_frame_then_the_stream_recovers() {
    let mut parser = FrameParser::new();
    assert!(parser.consume(b"{\"type\":\"terminal.fra").is_empty());
    let frames = parser.consume(format!("\n{}\n", line(9, "back")).as_bytes());
    assert!(matches!(frames.as_slice(), [Frame::Screen { seq: 9, .. }]));
}

#[test]
fn unknown_message_types_are_ignored_rather_than_treated_as_errors() {
    let mut parser = FrameParser::new();
    assert!(
        parser
            .consume(b"{\"type\":\"terminal.future\",\"x\":1}\n")
            .is_empty()
    );
}

#[test]
fn a_runaway_stream_with_no_newline_cant_grow_without_bound() {
    let mut parser = FrameParser::new();
    let chunk = vec![b'x'; 1_000_000];
    for _ in 0..20 {
        assert!(parser.consume(&chunk).is_empty());
    }
    // Still functional after the cap kicked in.
    let frames = parser.consume(format!("\n{}\n", line(1, "alive")).as_bytes());
    assert_eq!(frames.len(), 1);
}

/// Fix (buffer-cap): the chunk that ends an over-long line keeps the frames
/// after it. Swift tested the cap on everything buffered, complete lines
/// included, and dropped them all.
#[test]
fn frames_after_an_over_long_line_in_the_same_chunk_survive_the_cap() {
    let mut parser = FrameParser::new();
    let junk = vec![b'x'; FrameParser::MAX_BUFFER_BYTES - 10];
    assert!(parser.consume(&junk).is_empty());
    let mut chunk = vec![b'x'; 20];
    chunk.extend_from_slice(format!("\n{}\n{}\n", line(1, "a"), line(2, "b")).as_bytes());
    let frames = parser.consume(&chunk);
    assert_eq!(
        frames.len(),
        2,
        "the complete frames in the chunk were dropped"
    );
    // An unfinished tail past the cap still goes.
    assert!(
        parser
            .consume(&vec![b'y'; FrameParser::MAX_BUFFER_BYTES + 1])
            .is_empty()
    );
    assert_eq!(
        parser
            .consume(format!("\n{}\n", line(3, "c")).as_bytes())
            .len(),
        1
    );
}

#[test]
fn every_way_of_cutting_a_stream_gives_the_same_frames() {
    let stream = format!(
        "banner\r\n{}\nherdr: lost connection to server: server closed connection\r\n{}\n{{\"type\":\"terminal.closed\"}}\n{}\n",
        line(1, "a"),
        frame_line(2, 80, 24, true, "é\x1b[1;1H".as_bytes()),
        READY_MARKER
    );
    let whole = FrameParser::new().consume(stream.as_bytes());
    assert_eq!(whole.len(), 5);
    for size in 1..40 {
        let mut parser = FrameParser::new();
        let cut: Vec<Frame> = stream
            .as_bytes()
            .chunks(size)
            .flat_map(|chunk| parser.consume(chunk))
            .collect();
        assert_eq!(cut, whole, "chunks of {size}");
    }
}

// MARK: - Commands we send

#[test]
fn the_boot_line_disables_echo_and_does_not_exec() {
    let command = frame::start("w1:p2", 52, 30, None, false);
    // Without -echo the PTY would feed our own JSON back into the frame
    // stream; without raw mode, canonical input truncates at 4096 bytes.
    assert!(command.starts_with("stty raw -echo; "));
    // `exec` would kill the shell with the command; retargeting needs it.
    assert!(!command.contains("exec "));
    assert!(command.contains("terminal session control 'w1:p2'"));
    assert!(command.ends_with("--cols 52 --rows 30"));
    // `--takeover` only on the person's word: following the focus took the
    // pane from whichever device had it.
    assert!(!command.contains("--takeover"));
    assert!(frame::start("w1:p2", 52, 30, None, true).ends_with("--cols 52 --rows 30 --takeover"));
}

#[test]
fn a_custom_herdr_path_is_honored_in_the_boot_line() {
    let command = frame::start("w1:p1", 40, 20, Some("/opt/herdr/bin/herdr"), false);
    assert!(command.contains("/opt/herdr/bin/herdr terminal session control"));
    assert!(!command.contains("$HOME/.local/bin"));
}

#[test]
fn input_is_base64_so_arbitrary_bytes_survive_the_json_round_trip() {
    // An escape sequence plus a multi-byte character.
    let mut data = vec![0x1b, 0x5b, 0x41];
    data.extend_from_slice("é".as_bytes());
    let line = frame::input(&data);
    let json = object(&line);
    assert_eq!(json.str("type"), Some("terminal.input"));
    let decoded = moshpit_herdr::base64::decode(json.str("bytes").unwrap()).unwrap();
    assert_eq!(decoded, data);
}

#[test]
fn resize_is_valid_json_and_never_sends_a_zero_dimension() {
    let json = object(&frame::resize(0, -4));
    assert_eq!(json.i64("cols"), Some(1));
    assert_eq!(json.i64("rows"), Some(1));
}

#[test]
fn scroll_carries_direction_and_the_source_herdr_routes_on() {
    let up = object(&frame::scroll(true, 3, ScrollSource::Wheel));
    assert_eq!(up.str("direction"), Some("up"));
    assert_eq!(up.i64("lines"), Some(3));
    // `wheel` lets the SERVER decide mouse report vs scrollback.
    assert_eq!(up.str("source"), Some("wheel"));

    let down = object(&frame::scroll(false, 0, ScrollSource::PageKey));
    assert_eq!(down.str("direction"), Some("down"));
    assert_eq!(down.i64("lines"), Some(1)); // clamped
    assert_eq!(down.str("source"), Some("page_key"));
}

#[test]
fn release_is_the_detach_message_not_a_kill() {
    assert_eq!(object(RELEASE).str("type"), Some("terminal.release"));
}

/// Fix (size-clamp): herdr takes a `u16` above zero. Swift clamped resize,
/// scroll and retarget only below, and the boot and one-shot lines not at
/// all; past 65 535 herdr prints `input ignored`, which the app reads as a
/// refused attach.
#[test]
fn sizes_are_clamped_to_what_herdr_accepts_everywhere() {
    let resize = object(&frame::resize(70_000, 100_000));
    assert_eq!(resize.i64("cols"), Some(65_535));
    assert_eq!(resize.i64("rows"), Some(65_535));
    assert_eq!(
        object(&frame::scroll(true, 1 << 40, ScrollSource::Wheel)).i64("lines"),
        Some(65_535)
    );
    assert_eq!(frame::retarget("t", 70_000, 0, false), "t 65535 1");
    assert!(frame::start("t", 0, -2, None, true).ends_with("--cols 1 --rows 1 --takeover"));
    assert!(frame::boot("t", 0, 99_999, None, false, "C1").ends_with("'t' 1 65535"));
}

/// No Swift counterpart: herdr ≥ 0.9.2 takes pointer events on the frame
/// channel and encodes them for the pane's own mouse mode.
#[test]
fn a_mouse_event_is_the_shape_herdrs_server_reads() {
    let json = object(&frame::mouse(
        MouseAction::Down,
        MouseButton::Left,
        12,
        4,
        0,
    ));
    assert_eq!(json.str("type"), Some("terminal.mouse"));
    assert_eq!(json.str("action"), Some("down"));
    assert_eq!(json.str("button"), Some("left"));
    assert_eq!(json.i64("column"), Some(12));
    assert_eq!(json.i64("row"), Some(4));
    assert_eq!(json.i64("modifiers"), Some(0));
    let drag = object(&frame::mouse(
        MouseAction::Drag,
        MouseButton::Right,
        -3,
        70_000,
        2,
    ));
    assert_eq!(drag.str("action"), Some("drag"));
    assert_eq!(drag.i64("column"), Some(0));
    assert_eq!(drag.i64("row"), Some(65_535));
    assert!(frame::takes_mouse("0.9.2"));
    assert!(frame::takes_mouse("0.9.5"));
    assert!(frame::takes_mouse("0.10.0"));
    assert!(!frame::takes_mouse("0.9.1"));
    assert!(!frame::takes_mouse("0.8.0"));
}

// MARK: - The retarget loop

#[test]
fn the_boot_line_carries_its_first_target_and_keeps_reading_for_more() {
    let command = frame::boot("term_abc", 52, 30, None, false, "C1");
    // Same two PTY hazards as the one-shot line, and again inside the loop.
    assert!(command.starts_with("stty raw -echo; "));
    assert_eq!(command.matches("stty raw -echo").count(), 2);
    // `sh -c`, so fish or csh only ever sees one quoted argument.
    assert!(command.contains("/bin/sh -c "));
    // The first target rides in as an argument, never a second line.
    assert!(command.ends_with("'term_abc' 52 30"));
    assert!(command.contains("terminal session control \"$1\""));
    // `--takeover` only for a target the person chose: a fourth word on
    // the line (the first target's rides in as `$4`).
    assert!(command.contains(r#"--cols "${2:-80}" --rows "${3:-24}" ${4:+--takeover}; fi"#));
    assert!(frame::boot("term_abc", 52, 30, None, true, "C1").ends_with("'term_abc' 52 30 take"));
    // EOF ends the loop; a failed attach does not.
    assert!(command.contains("read -r t c r k || break"));
    assert!(command.contains(r#"set -- "$t" "$c" "$r" "$k"; done"#));
    // The marker is what tells the app the loop is really there.
    assert!(command.contains(READY_MARKER));
}

#[test]
fn the_loop_drops_our_own_json_commands_instead_of_attaching_to_them() {
    let command = frame::boot("t", 10, 10, None, false, "C1");
    // A `{` line is ours, never a pane id: dropped with the stale target.
    assert!(command.contains(r#"case "$t" in "{"*) set --; continue;; esac"#));
    // …checked BEFORE the line becomes the next target.
    let drop = command.find("case \"$t\"").unwrap();
    let adopt = command.find("set -- \"$t\"").unwrap();
    assert!(drop < adopt);
}

#[test]
fn a_hang_up_mid_handshake_is_told_apart_from_other_faults() {
    let mut parser = FrameParser::new();
    let frames = parser.consume(b"herdr: lost connection to server: server closed connection\r\n");
    assert_eq!(
        frames,
        vec![Frame::Fault {
            message: "lost connection to server: server closed connection".to_owned()
        }]
    );
    assert!(frame::is_handshake_drop(
        "lost connection to server: server closed connection"
    ));
    assert!(!frame::is_handshake_drop(
        "lost connection to remote Herdr: broken pipe"
    ));
    assert!(!frame::is_handshake_drop(
        "terminal session control input ignored: invalid json command"
    ));
}

#[test]
fn a_custom_herdr_path_cant_break_out_of_the_boot_lines_quoting() {
    let command = frame::boot("t", 10, 10, Some("/opt/o'clock/herdr"), false, "C1");
    assert!(command.contains("/opt/o'\\''clock/herdr terminal session control"));
}

#[test]
fn the_loops_ready_marker_parses_as_a_frame_of_its_own() {
    let mut parser = FrameParser::new();
    assert_eq!(
        parser.consume(format!("{READY_MARKER}\n").as_bytes()),
        vec![Frame::LoopReady]
    );
}

#[test]
fn a_retarget_is_one_line_the_loops_read_can_split() {
    assert_eq!(frame::retarget("term_abc", 52, 30, false), "term_abc 52 30");
    // The person's pick: a fourth word, `--takeover` in the loop.
    assert_eq!(
        frame::retarget("term_abc", 52, 30, true),
        "term_abc 52 30 take"
    );
    // The loop passes these straight to --cols/--rows, which reject 0.
    assert_eq!(frame::retarget("t", 0, -3, false), "t 1 1");
}

// MARK: - Pastes and keys (SessionHub)

#[test]
fn a_paste_is_one_complete_bracketed_paste() {
    assert_eq!(
        frame::bracketed_paste("first line\nsecond line"),
        b"\x1b[200~first line\nsecond line\x1b[201~"
    );
    assert_eq!(
        frame::bracketed_paste("/tmp/moshpit-uploads/shot.png"),
        b"\x1b[200~/tmp/moshpit-uploads/shot.png\x1b[201~"
    );
}

/// B738: no control character goes out in a paste but tab, line feed and
/// carriage return (xterm's default). An escape sequence in it ran as keys
/// where the program hadn't asked for bracketed paste, and a ^C or ^D hit
/// the tty as an interrupt or an end of file, bracketed or not. C1
/// controls and DEL go too; text, CJK and emoji stay as they are.
#[test]
fn b738_a_paste_carries_no_control_but_tab_and_line_breaks() {
    assert_eq!(
        frame::bracketed_paste("a\u{1b}[31mred\u{3}\u{4}end\tx\r\ny\u{9b}2J\u{7f}中文🎉"),
        "\u{1b}[200~a[31mredend\tx\r\ny2J中文🎉\u{1b}[201~".as_bytes()
    );
    assert_eq!(
        frame::paste_body("\u{1b}:!rm -rf ~\u{1b}\n"),
        ":!rm -rf ~\n"
    );
    // Tabs and line breaks stay as they were; neither an end marker nor
    // the 8-bit CSI (U+009B) can close the paste: one end, its own.
    assert_eq!(
        frame::bracketed_paste("a\tb\r\nc\nd"),
        b"\x1b[200~a\tb\r\nc\nd\x1b[201~"
    );
    for (pasted, body) in [("x\u{1b}[201~y", "xy"), ("x\u{9b}201~y", "x201~y")] {
        let paste = frame::bracketed_paste(pasted);
        assert_eq!(paste, format!("\u{1b}[200~{body}\u{1b}[201~").as_bytes());
        assert!(!paste[..paste.len() - 6].windows(6).any(|w| w == b"\x1b[201~"));
        assert!(!String::from_utf8(paste).unwrap().contains('\u{9b}'));
    }
}

#[test]
fn an_end_marker_inside_the_pasted_text_cant_end_the_paste_early() {
    assert_eq!(
        frame::bracketed_paste("one\x1b[201~two"),
        b"\x1b[200~onetwo\x1b[201~"
    );
}

/// Fix (paste-end-marker): removing markers in one pass can put one
/// together; the Swift version sent `ESC[201~` mid-paste for this text.
#[test]
fn an_end_marker_assembled_by_removing_one_is_removed_too() {
    // One pass leaves "a" ESC[20 + 1~ "b" = "a" ESC[201~ "b".
    assert_eq!(
        frame::bracketed_paste("a\x1b[20\x1b[201~1~b"),
        b"\x1b[200~ab\x1b[201~"
    );
    assert_eq!(
        frame::bracketed_paste("\x1b[20\x1b[20\x1b[201~1~1~"),
        b"\x1b[200~\x1b[201~"
    );
}

/// As the app's `BracketedPaste` (round 19): no marker of either kind stays
/// in a paste, including one assembled across kinds, and deep nesting comes
/// out in one pass.
#[test]
fn start_markers_are_removed_too_and_nesting_is_linear() {
    assert_eq!(
        frame::bracketed_paste("one\x1b[200~two"),
        b"\x1b[200~onetwo\x1b[201~"
    );
    // Removing the end marker leaves ESC[20 + 0~: a start marker.
    assert_eq!(
        frame::bracketed_paste("\x1b[20\x1b[201~0~x"),
        b"\x1b[200~x\x1b[201~"
    );
    // A marker can't be completed across the paste's own start marker.
    assert_eq!(frame::bracketed_paste("~"), b"\x1b[200~~\x1b[201~");
    let depth = 50_000;
    let nested = "\x1b[20".repeat(depth) + "\x1b[201~" + &"1~".repeat(depth) + "\necho INJECTED\n";
    assert_eq!(
        frame::bracketed_paste(&nested),
        b"\x1b[200~\necho INJECTED\n\x1b[201~"
    );
}

#[test]
fn only_a_lone_arrow_sequence_maps_to_a_key_name_in_both_decckm_variants() {
    assert_eq!(frame::arrow_key_name(&[0x1B, 0x5B, 0x41]), Some("up"));
    assert_eq!(frame::arrow_key_name(&[0x1B, 0x4F, 0x42]), Some("down"));
    assert_eq!(frame::arrow_key_name(&[0x1B, 0x5B, 0x43]), Some("right"));
    assert_eq!(frame::arrow_key_name(&[0x1B, 0x4F, 0x44]), Some("left"));
    // Not arrows: plain text, batched sequences, other CSI.
    assert_eq!(frame::arrow_key_name(b"A"), None);
    assert_eq!(
        frame::arrow_key_name(&[0x1B, 0x5B, 0x41, 0x1B, 0x5B, 0x41]),
        None
    );
    assert_eq!(frame::arrow_key_name(&[0x1B, 0x5B, 0x48]), None); // Home
}

/// B360: the input channel hands over what was written since its last
/// read as one chunk; its arrows are found where they are.
#[test]
fn a_chunk_splits_into_its_lone_arrows_and_the_bytes_between_them() {
    use frame::Piece::{Arrow, Bytes};
    assert_eq!(
        frame::split_arrows(b"\x1b[A\x1b[A\x1bOA"),
        [Arrow("up"), Arrow("up"), Arrow("up")],
        "key repeat"
    );
    assert_eq!(
        frame::split_arrows(b"ab\x1b[Ccd\x1bODe"),
        [
            Bytes(b"ab"),
            Arrow("right"),
            Bytes(b"cd"),
            Arrow("left"),
            Bytes(b"e")
        ]
    );
    assert_eq!(frame::split_arrows(b"\x1b[A"), [Arrow("up")]);
    assert_eq!(frame::split_arrows(b"plain"), [Bytes(b"plain")]);
    assert!(frame::split_arrows(b"").is_empty());
    // A parameterised arrow, a bare ESC, Home, and a character whose
    // UTF-8 is never split: all as written.
    for whole in [&b"\x1b[1;5A"[..], b"\x1b", b"\x1b[H", "终\x1bx".as_bytes()] {
        assert_eq!(frame::split_arrows(whole), [Bytes(whole)], "{whole:?}");
    }
    assert_eq!(
        frame::split_arrows("终\x1b[B端".as_bytes()),
        [
            Bytes("终".as_bytes()),
            Arrow("down"),
            Bytes("端".as_bytes())
        ]
    );
    // A bracketed paste is text to its end marker, arrows and all; an
    // arrow after it is a key again.
    assert_eq!(
        frame::split_arrows(b"\x1b[A\x1b[200~a\x1b[Ab\x1b[201~\x1b[B"),
        [
            Arrow("up"),
            Bytes(b"\x1b[200~a\x1b[Ab\x1b[201~"),
            Arrow("down")
        ]
    );
    let unended = b"x\x1b[200~a\x1b[A";
    assert_eq!(frame::split_arrows(unended), [Bytes(unended)]);
}
