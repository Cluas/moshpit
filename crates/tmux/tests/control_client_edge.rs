//! Port of Tests/MoshpitTests/Services/TmuxControlClientEdgeTests.swift
//! (suite "TmuxControlClient parser — edge cases", 16 tests). Each test keeps
//! the Swift function's name, in snake case, and its `@Test` title as the doc
//! comment. Like the Swift `EdgeRecorder`, responses are compared by line
//! count and protocol errors without their text.

use moshpit_tmux::{ControlParser, Event};

/// Boot banner: every control session's first block is the boot line's own
/// reply and is swallowed by the parser (no event).
const BOOT: &[u8] = b"%begin 900 500 0\n%end 900 500 0\n";

/// The Swift `EdgeRecorder.Event` shape.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Edge {
    Output(String, Vec<u8>),
    WindowAdd(String),
    WindowClose(String),
    WindowRenamed(String, String),
    LayoutChange(String, String, bool),
    SessionChanged(String, String),
    SessionRenamed(String, String),
    SessionWindowChanged(String, String),
    SessionsChanged,
    UnlinkedWindowAdd(String),
    UnlinkedWindowClose(String),
    UnlinkedWindowRenamed(String, String),
    ClientSessionChanged(String, String, String),
    ActivePaneChanged(String, String),
    Pause(String),
    Continue(String),
    PaneModeChanged(String),
    ClientDetached(String),
    Exit(Option<String>),
    CommandResponse {
        id: i64,
        num: i64,
        is_error: bool,
        line_count: usize,
    },
    ProtocolError,
}

impl From<Event> for Edge {
    fn from(event: Event) -> Self {
        match event {
            Event::PaneOutput { pane_id, data } => Edge::Output(pane_id, data),
            Event::WindowAdd { window_id } => Edge::WindowAdd(window_id),
            Event::WindowClose { window_id } => Edge::WindowClose(window_id),
            Event::WindowRenamed { window_id, name } => Edge::WindowRenamed(window_id, name),
            Event::LayoutChange {
                window_id,
                layout,
                zoomed,
            } => Edge::LayoutChange(window_id, layout, zoomed),
            Event::SessionChanged { session_id, name } => Edge::SessionChanged(session_id, name),
            Event::SessionRenamed { session_id, name } => Edge::SessionRenamed(session_id, name),
            Event::SessionWindowChanged {
                session_id,
                window_id,
            } => Edge::SessionWindowChanged(session_id, window_id),
            Event::SessionsChanged => Edge::SessionsChanged,
            Event::UnlinkedWindowAdd { window_id } => Edge::UnlinkedWindowAdd(window_id),
            Event::UnlinkedWindowClose { window_id } => Edge::UnlinkedWindowClose(window_id),
            Event::UnlinkedWindowRenamed { window_id, name } => {
                Edge::UnlinkedWindowRenamed(window_id, name)
            }
            Event::ClientSessionChanged {
                client,
                session_id,
                name,
            } => Edge::ClientSessionChanged(client, session_id, name),
            Event::ActivePaneChanged { window_id, pane_id } => {
                Edge::ActivePaneChanged(window_id, pane_id)
            }
            Event::Pause { pane_id } => Edge::Pause(pane_id),
            Event::Continue { pane_id } => Edge::Continue(pane_id),
            Event::PaneModeChanged { pane_id } => Edge::PaneModeChanged(pane_id),
            Event::ClientDetached { payload } => Edge::ClientDetached(payload),
            Event::Exit { reason } => Edge::Exit(reason),
            Event::CommandResponse(response) => Edge::CommandResponse {
                id: response.command_id,
                num: response.command_num,
                is_error: response.is_error,
                line_count: response.lines.len(),
            },
            Event::ProtocolError(_) => Edge::ProtocolError,
        }
    }
}

struct EdgeRecorder {
    parser: ControlParser,
    events: Vec<Edge>,
}

impl EdgeRecorder {
    fn booted() -> Self {
        let mut recorder = Self {
            parser: ControlParser::new(),
            events: Vec::new(),
        };
        recorder.feed(BOOT);
        recorder
    }

    fn feed(&mut self, data: &[u8]) {
        let events = &mut self.events;
        self.parser.feed(data, |event| events.push(event.into()));
    }
}

fn output(pane: &str, data: &[u8]) -> Edge {
    Edge::Output(pane.into(), data.to_vec())
}

// MARK: Octal escapes inside %output

/// Octal-escaped newline (\012) inside %output stays inside the payload, not a line break
#[test]
fn octal_newline_does_not_split_lines() {
    let mut rec = EdgeRecorder::booted();
    // tmux escapes any byte < 0x20 or > 0x7E. An embedded newline is `\012`.
    rec.feed(b"%output %0 first\\012second\n");
    assert_eq!(rec.events, [output("%0", b"first\nsecond")]);
}

/// Octal-escaped CR + LF in payload do not get treated as transport CRLF
#[test]
fn octal_crlf_does_not_split_lines() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%output %0 alpha\\015\\012beta\n");
    assert_eq!(rec.events, [output("%0", b"alpha\r\nbeta")]);
}

// MARK: Aggressive fragmentation

/// Feeding one byte at a time still produces exactly one notification
#[test]
fn feed_single_byte_chunks() {
    let mut rec = EdgeRecorder::booted();
    for byte in b"%window-add @42\n" {
        rec.feed(std::slice::from_ref(byte));
    }
    assert_eq!(rec.events, [Edge::WindowAdd("@42".into())]);
}

/// Multiple complete lines in a single feed all fire in source order
#[test]
fn many_lines_in_one_feed() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%window-add @1\n%window-add @2\n%window-add @3\n%window-close @2\n");
    assert_eq!(
        rec.events,
        [
            Edge::WindowAdd("@1".into()),
            Edge::WindowAdd("@2".into()),
            Edge::WindowAdd("@3".into()),
            Edge::WindowClose("@2".into()),
        ]
    );
}

// MARK: Empty / minimal payloads

/// %output with empty payload still fires onPaneOutput with empty Data
#[test]
fn empty_output_payload() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%output %0 \n");
    assert_eq!(rec.events, [output("%0", b"")]);
}

/// %session-changed without a name string emits an empty-name event
#[test]
fn session_changed_without_name() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%session-changed $7\n");
    assert_eq!(
        rec.events,
        [Edge::SessionChanged("$7".into(), String::new())]
    );
}

// MARK: Block framing pathologies

/// A `%begin`-looking line inside an open block is captured as content, not parsed as a nested block
#[test]
fn nested_begin_captured_as_content() {
    // Once inside `%begin … %end`, only the open block's own terminator
    // closes it: `%end 2 2 0` here is pane content (a capture of a pane that
    // shows control-mode text), not a terminator.
    let mut rec = EdgeRecorder::booted();
    rec.feed(
        b"%begin 1 1 0\n\
          first content line\n\
          %begin 2 2 0\n\
          second content line\n\
          %end 2 2 0\n\
          %end 1 1 0\n",
    );
    let responses: Vec<&Edge> = rec
        .events
        .iter()
        .filter(|event| matches!(event, Edge::CommandResponse { .. }))
        .collect();
    assert_eq!(responses.len(), 1, "exactly one block should close");
    assert_eq!(
        responses[0],
        &Edge::CommandResponse {
            id: 1,
            num: 1,
            is_error: false,
            line_count: 4
        },
        "all four payload lines — including both %-shaped ones — belong to block 1"
    );
    assert!(
        !rec.events.contains(&Edge::ProtocolError),
        "nothing about this frame is a protocol violation under id-matched termination"
    );
}

/// %end without a matching %begin surfaces as a protocol error
#[test]
fn end_without_begin() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%end 0 0 0\n");
    assert_eq!(rec.events, [Edge::ProtocolError]);
}

/// Response lines starting with %output / %window-add are captured verbatim, not parsed
#[test]
fn response_line_with_notification_prefix() {
    // list-windows can return rows that incidentally start with %.
    let mut rec = EdgeRecorder::booted();
    rec.feed(
        b"%begin 1 1 0\n\
          %output not actually a notification\n\
          %window-add looks-like-add\n\
          %end 1 1 0\n",
    );
    assert_eq!(
        rec.events,
        [Edge::CommandResponse {
            id: 1,
            num: 1,
            is_error: false,
            line_count: 2
        }]
    );
}

// MARK: Layout change permutations

/// %layout-change with the optional visible-layout and *Z flags sets zoomed=true
#[test]
fn layout_change_with_visible_layout_and_zoom() {
    let mut rec = EdgeRecorder::booted();
    // Real tmux: %layout-change <window-id> <window-layout> [visible_layout] [<flags>]
    rec.feed(b"%layout-change @5 81x24,0,0,8 81x24,0,0,8 *Z\n");
    assert_eq!(
        rec.events,
        [Edge::LayoutChange("@5".into(), "81x24,0,0,8".into(), true)]
    );
}

/// %layout-change without the zoom flag sets zoomed=false
#[test]
fn layout_change_without_zoom() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%layout-change @5 81x24,0,0,8\n");
    assert_eq!(
        rec.events,
        [Edge::LayoutChange("@5".into(), "81x24,0,0,8".into(), false)]
    );
}

// MARK: DCS prologue

/// Leading DCS prologue before the first %begin is skipped, not treated as garbage
#[test]
fn dcs_prologue_skipped() {
    let mut rec = EdgeRecorder::booted();
    // Some transports prepend `\eP1000p` before the first tmux notification.
    rec.feed(b"\x1bP1000p%window-add @0\n");
    assert_eq!(rec.events, [Edge::WindowAdd("@0".into())]);
}

// MARK: Large payloads

/// Single %output line carrying ~16 KB of escape-free text routes once
#[test]
fn large_output_payload() {
    let mut rec = EdgeRecorder::booted();
    let big = vec![b'x'; 16 * 1024];
    let mut line = b"%output %0 ".to_vec();
    line.extend_from_slice(&big);
    line.push(b'\n');
    rec.feed(&line);
    assert_eq!(rec.events, [output("%0", &big)]);
}

// MARK: %client-detached variants

/// Bare %client-detached and %client-detached <id> both fire onClientDetached
#[test]
fn client_detached_bare_and_payload() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(b"%client-detached\n%client-detached client-7\n");
    assert_eq!(
        rec.events,
        [
            Edge::ClientDetached(String::new()),
            Edge::ClientDetached("client-7".into()),
        ]
    );
}

// MARK: Forward-compat noise mixed with real events

/// Unknown directives between real events do not desync the parser
#[test]
fn unknown_directives_do_not_desync() {
    let mut rec = EdgeRecorder::booted();
    rec.feed(
        b"%window-add @1\n\
          %future-directive @1 some payload\n\
          %not-a-real-thing\n\
          %window-close @1\n",
    );
    assert_eq!(
        rec.events,
        [Edge::WindowAdd("@1".into()), Edge::WindowClose("@1".into())]
    );
}

// MARK: reset() during open block

/// reset() abandons an in-flight %begin block without emitting onCommandResponse
#[test]
fn reset_during_open_block() {
    let mut rec = EdgeRecorder::booted();
    // Open a block but never close it.
    rec.feed(b"%begin 5 5 0\nline-one\n");
    rec.parser.reset();
    // A fresh notification fires normally, no leftover response.
    rec.feed(b"%window-add @9\n");
    assert_eq!(rec.events, [Edge::WindowAdd("@9".into())]);
}
