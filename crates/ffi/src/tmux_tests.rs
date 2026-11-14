//! The parser's C ABI against the parser itself: every committed capture,
//! cut into feeds every way the G2 harness cuts them, goes through
//! `mp_tmux_parser_feed`; the events rebuilt from the C structs must be the
//! events `ControlParser` produces for the same feeds — and, rendered the
//! harness's way, the Swift parser's recorded output.

use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};

use crate::tmux::{
    MpBytes, MpTmuxEvent, MpTmuxParser, feed_result, kind, mp_ffi_abi_version, mp_tmux_parser_feed,
    mp_tmux_parser_free, mp_tmux_parser_new, mp_tmux_parser_reset,
};
use moshpit_tmux::harness::{self, Step};
use moshpit_tmux::{CommandResponse, ControlError, ControlParser, Event};

const PLANS: &[&str] = &[
    "whole",
    "bytes",
    "rand:1:3",
    "rand:2:17",
    "rand:3:256",
    "rand:4:4096",
    "inside:1",
    "inside:2",
    "reset:5:64",
];

fn fixtures(group: &str) -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tmux/tests/fixtures")
        .join(group);
    let mut found: Vec<PathBuf> = fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no fixtures in {}", folder.display());
    found
}

fn bytes(field: &MpBytes) -> Vec<u8> {
    if field.len == 0 {
        return Vec::new();
    }
    // SAFETY: the library's promise: `len` readable bytes during the callback.
    unsafe { std::slice::from_raw_parts(field.ptr, field.len) }.to_vec()
}

fn text(field: &MpBytes) -> String {
    String::from_utf8(bytes(field)).expect("the ABI promises UTF-8 text")
}

/// The owned event a C struct describes.
fn event(c: &MpTmuxEvent) -> Event {
    let (a, b) = (text(&c.first), || text(&c.second));
    match c.kind {
        kind::PANE_OUTPUT => Event::PaneOutput {
            pane_id: a,
            data: bytes(&c.second),
        },
        kind::LAYOUT_CHANGE => Event::LayoutChange {
            window_id: a,
            layout: b(),
            zoomed: c.flag,
        },
        kind::WINDOW_ADD => Event::WindowAdd { window_id: a },
        kind::WINDOW_CLOSE => Event::WindowClose { window_id: a },
        kind::WINDOW_RENAMED => Event::WindowRenamed {
            window_id: a,
            name: b(),
        },
        kind::SESSION_CHANGED => Event::SessionChanged {
            session_id: a,
            name: b(),
        },
        kind::SESSION_RENAMED => Event::SessionRenamed {
            session_id: a,
            name: b(),
        },
        kind::SESSION_WINDOW_CHANGED => Event::SessionWindowChanged {
            session_id: a,
            window_id: b(),
        },
        kind::SESSIONS_CHANGED => Event::SessionsChanged,
        kind::UNLINKED_WINDOW_ADD => Event::UnlinkedWindowAdd { window_id: a },
        kind::UNLINKED_WINDOW_CLOSE => Event::UnlinkedWindowClose { window_id: a },
        kind::UNLINKED_WINDOW_RENAMED => Event::UnlinkedWindowRenamed {
            window_id: a,
            name: b(),
        },
        kind::CLIENT_SESSION_CHANGED => Event::ClientSessionChanged {
            client: a,
            session_id: b(),
            name: text(&c.third),
        },
        kind::ACTIVE_PANE_CHANGED => Event::ActivePaneChanged {
            window_id: a,
            pane_id: b(),
        },
        kind::PAUSE => Event::Pause { pane_id: a },
        kind::CONTINUE => Event::Continue { pane_id: a },
        kind::CLIENT_DETACHED => Event::ClientDetached { payload: a },
        kind::EXIT => Event::Exit {
            reason: c.flag.then_some(a),
        },
        kind::COMMAND_RESPONSE => {
            let lines = if c.line_count == 0 {
                &[][..]
            } else {
                // SAFETY: the library's promise, as for the fields.
                unsafe { std::slice::from_raw_parts(c.lines, c.line_count) }
            };
            Event::CommandResponse(CommandResponse {
                command_id: c.command_id,
                command_num: c.command_num,
                is_error: c.flag,
                lines: lines.iter().map(text).collect(),
                from_client: c.from_client,
            })
        }
        kind::PROTOCOL_ERROR => Event::ProtocolError(ControlError::MalformedLine(a)),
        other => panic!("unknown event kind {other}"),
    }
}

unsafe extern "C" fn collect(context: *mut c_void, c: *const MpTmuxEvent) {
    // SAFETY: the tests pass a `Vec<Event>`; the library a live event.
    let (events, c) = unsafe { (&mut *context.cast::<Vec<Event>>(), &*c) };
    events.push(event(c));
}

/// The events of `data` fed along `steps` through the C ABI.
fn through_c(data: &[u8], steps: &[Step]) -> Vec<Event> {
    let parser: *mut MpTmuxParser = mp_tmux_parser_new();
    let mut events: Vec<Event> = Vec::new();
    for step in steps {
        match step {
            Step::Feed(range) => {
                let chunk = &data[range.clone()];
                // SAFETY: a live parser, `chunk` readable, `collect`'s context.
                let result = unsafe {
                    mp_tmux_parser_feed(
                        parser,
                        chunk.as_ptr(),
                        chunk.len(),
                        Some(collect),
                        (&raw mut events).cast(),
                    )
                };
                assert_eq!(result, feed_result::OK);
            }
            // SAFETY: a live parser.
            Step::Reset => unsafe { mp_tmux_parser_reset(parser) },
        }
    }
    // SAFETY: made above, freed once.
    unsafe { mp_tmux_parser_free(parser) };
    events
}

/// The events of `data` fed along `steps` straight into the parser.
fn direct(data: &[u8], steps: &[Step]) -> Vec<Event> {
    let mut parser = ControlParser::new();
    let mut events = Vec::new();
    for step in steps {
        match step {
            Step::Feed(range) => parser.feed(&data[range.clone()], |e| events.push(e)),
            Step::Reset => parser.reset(),
        }
    }
    events
}

/// The events as the harness's JSON lines.
fn rendered(events: &[Event]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for event in events {
        harness::write_event(&mut out, event);
    }
    lines(&out)
}

fn lines(output: &[u8]) -> Vec<Vec<u8>> {
    let mut lines: Vec<Vec<u8>> = output.split(|&b| b == b'\n').map(<[u8]>::to_vec).collect();
    lines.pop(); // after the last newline
    lines
}

/// The Swift parser's recorded event lines, without the `end` record.
fn golden(capture: &Path) -> Vec<Vec<u8>> {
    let mut golden = lines(&fs::read(capture.with_extension("swift.jsonl")).unwrap());
    let end = golden.pop().expect("an end record");
    assert!(end.starts_with(br#"{"ev":"end""#), "{}", capture.display());
    golden
}

#[test]
fn the_library_reports_the_header_s_abi_version() {
    let header = include_str!("../include/moshpit_ffi.h");
    let declared = header
        .lines()
        .find_map(|line| line.strip_prefix("#define MP_FFI_ABI_VERSION "))
        .expect("MP_FFI_ABI_VERSION in the header")
        .trim()
        .parse::<u32>()
        .unwrap();
    assert_eq!(mp_ffi_abi_version(), declared);
}

/// B275: whose a block is crosses to C. Before, the C event had no field
/// for it and the app's controller took a hook's block (flags 0) for the
/// reply to its next command.
#[test]
fn a_blocks_flags_cross_the_c_abi() {
    let data = b"%begin 1 1 0\n%end 1 1 0\n\
        %begin 2 2 1\nours\n%end 2 2 1\n\
        %begin 3 3 0\nhook\n%end 3 3 0\n\
        %begin 4 4 0\n%error 4 4 0\n\
        %begin 5 5\n%end 5 5\n\
        %begin 6 6 x\n%end 6 6 x\n\
        %begin 7 7 3\n%end 7 7 3\n\
        %begin 8 8 2\n%end 8 8 2\n";
    let events = through_c(data, &[Step::Feed(0..data.len())]);
    let whose: Vec<(i64, bool, bool)> = events
        .iter()
        .map(|event| match event {
            Event::CommandResponse(r) => (r.command_num, r.is_error, r.from_client),
            other => panic!("{other:?}"),
        })
        .collect();
    // The first block is the boot command's, swallowed. Missing or
    // unreadable flags read as ours; bit 1 is what counts.
    assert_eq!(
        whose,
        [
            (2, false, true),
            (3, false, false),
            (4, true, false),
            (5, false, true),
            (6, false, true),
            (7, false, true),
            (8, false, false),
        ]
    );
    assert_eq!(events, direct(data, &[Step::Feed(0..data.len())]));
}

#[test]
fn every_capture_crosses_the_c_abi_unchanged_however_it_is_fed() {
    let mut checked = 0;
    for group in ["captures", "synthetic", "divergences"] {
        for capture in fixtures(group) {
            let data = fs::read(&capture).unwrap();
            for plan in PLANS {
                let steps = harness::plan(plan, &data).unwrap();
                assert_eq!(
                    through_c(&data, &steps),
                    direct(&data, &steps),
                    "{} fed {plan}",
                    capture.display()
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 9 + 1 + 7);
}

#[test]
fn through_the_c_abi_the_events_are_the_swift_parser_s() {
    let mut checked = 0;
    for group in ["captures", "synthetic"] {
        for capture in fixtures(group) {
            let data = fs::read(&capture).unwrap();
            let steps = harness::plan("rand:9:512", &data).unwrap();
            assert!(
                rendered(&through_c(&data, &steps)) == golden(&capture),
                "{}",
                capture.display()
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 10);
}

#[test]
fn every_event_kind_crosses() {
    // One line per kind, each with the fields it carries.
    let stream = b"%begin 1 1 0\n%end 1 1 0\n\
        %output %3 a\\033b\n\
        %layout-change @1 b25d,80x24,0,0,0 b25d,80x24,0,0,0 *Z\n\
        %window-add @2\n%window-close @2\n%window-renamed @1 one two\n\
        %session-changed $1 main\n%session-renamed $1 new name\n\
        %session-window-changed $1 @1\n%sessions-changed\n\
        %unlinked-window-add @7\n%unlinked-window-close @7\n%unlinked-window-renamed @7 seven\n\
        %client-session-changed /dev/ttys001 $2 other one\n\
        %window-pane-changed @1 %3\n%pause %3\n%continue %3\n\
        %client-detached client-12\n\
        %begin 1700000000 5 1\nline one\n\nline three\n%error 1700000000 5 1\n\
        %end 9 9 0\n%exit too far behind\n%exit\n";
    let steps = harness::plan("whole", stream).unwrap();
    let events = through_c(stream, &steps);
    assert_eq!(events, direct(stream, &steps));
    assert_eq!(events.len(), 21);
    assert_eq!(
        events[0],
        Event::PaneOutput {
            pane_id: "%3".into(),
            data: b"a\x1bb".to_vec()
        }
    );
    assert_eq!(
        events[17],
        Event::CommandResponse(CommandResponse {
            command_id: 1_700_000_000,
            command_num: 5,
            is_error: true,
            lines: vec!["line one".into(), String::new(), "line three".into()],
            from_client: true,
        })
    );
    assert_eq!(
        events[18],
        Event::ProtocolError(ControlError::MalformedLine("%end 9 9 0".into()))
    );
    assert_eq!(
        events[19],
        Event::Exit {
            reason: Some("too far behind".into())
        }
    );
    assert_eq!(events[20], Event::Exit { reason: None });
}
