//! The controller's pure text rules, ported from the Swift suites
//! `TmuxPlausibleLayoutTests`, `TmuxLayoutWidthTests`,
//! `TmuxPasteEscapingTests`, `TmuxIdOrderTests` and the frame-width case of
//! `TmuxSessionControllerTests`.

use moshpit_tmux::format::{
    command_string_literal as literal, cycled, frame_exceeds_width, is_plausible_layout,
    is_valid_id, pane_count, window_width,
};
use moshpit_tmux::model::{PaneInfo, SessionInfo, Snapshot, id_precedes};

#[test]
fn accepts_real_layouts() {
    assert!(is_plausible_layout("21be,200x50,0,0"));
    assert!(is_plausible_layout(
        "4b9d,203x60,0,0[203x30,0,0,0,203x29,0,31{101x29,0,31,1,101x29,102,31,2}]"
    ));
}

#[test]
fn rejects_junk_layouts() {
    for junk in [
        "",
        "can't find window: @9",
        "21be",
        "21bez,200x50,0,0",
        "21be,200x50,0,0; rm -rf /",
        "21be,200x50,0,0'",
    ] {
        assert!(!is_plausible_layout(junk), "{junk:?}");
    }
}

#[test]
fn window_width_from_simple_and_split_layouts() {
    assert_eq!(window_width("bb62,355x62,0,0,1"), Some(355));
    assert_eq!(window_width("1a2b,70x35,0,0,4"), Some(70));
    assert_eq!(
        window_width("34de,80x24,0,0{40x24,0,0,1,39x24,41,0,2}"),
        Some(80)
    );
    assert_eq!(window_width(""), None);
    assert_eq!(window_width("justchecksum"), None);
}

#[test]
fn pane_count_one_per_leaf() {
    assert_eq!(pane_count("bb62,355x62,0,0,1"), Some(1));
    assert_eq!(
        pane_count("34de,80x24,0,0{40x24,0,0,1,39x24,41,0,2}"),
        Some(2)
    );
    assert_eq!(
        pane_count("3992,47x40,0,0{45x40,0,0,167,1x40,46,0[1x20,46,0,168,1x19,46,21,169]}"),
        Some(3)
    );
    assert_eq!(pane_count(""), None);
    assert_eq!(pane_count("justchecksum"), None);
}

#[test]
fn ids_have_tmux_shape() {
    for good in ["%0", "@12", "$3"] {
        assert!(is_valid_id(good), "{good}");
    }
    for bad in [
        "",
        "%",
        "x1",
        "%1a",
        "%\u{301}5",
        "%５",
        "%1;x",
        "$3;reboot",
    ] {
        assert!(!is_valid_id(bad), "{bad:?}");
    }
}

#[test]
fn paste_literal_escapes() {
    assert_eq!(literal("hello world"), "\"hello world\"");
    assert_eq!(literal("a\\b"), r#""a\\b""#);
    assert_eq!(literal("a\"b"), r#""a\"b""#);
    assert_eq!(literal("a'b"), r#""a'b""#);
    assert_eq!(literal("a\nb"), r#""a\nb""#);
    assert!(!literal("a\nb").contains('\n'));
    assert_eq!(literal("a\rb"), r#""a\rb""#);
    assert_eq!(literal("a\tb"), r#""a\tb""#);
    assert_eq!(literal("$HOME"), r#""\$HOME""#);
    assert_eq!(literal("~root"), r#""\~root""#);
    assert_eq!(literal("a~b"), r#""a\~b""#);
    assert_eq!(literal("\u{1}"), "\"\\u0001\"");
    assert_eq!(literal("\u{7f}"), "\"\\u007f\"");
    assert_eq!(literal("#{pane_id}"), "\"#{pane_id}\"");
    assert_eq!(literal("中文🎉"), "\"中文🎉\"");
    let out = literal("line1\nline2 has $HOME, ~root, \"quote\", back\\slash, 中文🎉");
    assert!(out.starts_with('"') && out.ends_with('"'));
    assert!(!out.contains('\n'));
    for part in [
        "\\n",
        "\\$HOME",
        "\\~root",
        "\\\"quote\\\"",
        "back\\\\slash",
        "中文🎉",
    ] {
        assert!(out.contains(part), "{part}");
    }
}

#[test]
fn hyperlink_payloads_do_not_count_toward_frame_width() {
    let osc = "\u{1b}[38;5;246m  ⎿ \u{1b}[39m\u{1b}]8;id=1qrnb37;file:///Users/dev/.claude/image-cache/5d263c4f/1.jpeg\u{1b}\\[Image #1]\u{1b}]8;;\u{1b}\\";
    assert!(!frame_exceeds_width(&[osc], 65));
    let bel = "\u{1b}]0;window title that is quite long indeed\u{7}visible";
    assert!(!frame_exceeds_width(&[bel], 20));
    assert!(frame_exceeds_width(&["W".repeat(80)], 65));
    // Wide and zero-width scalars.
    assert!(frame_exceeds_width(&["中".repeat(35)], 69));
    assert!(!frame_exceeds_width(&["e\u{301}".repeat(60)], 60));
}

#[test]
fn cycling_wraps_and_never_lands_on_itself() {
    let ids: Vec<String> = ["%1", "%2", "%3"].iter().map(|s| s.to_string()).collect();
    assert_eq!(cycled(&ids, Some("%3"), true).as_deref(), Some("%1"));
    assert_eq!(cycled(&ids, Some("%1"), false).as_deref(), Some("%3"));
    assert_eq!(cycled(&ids, Some("%9"), true), None);
    assert_eq!(cycled(&ids[..1], Some("%1"), true), None);
    assert_eq!(cycled(&ids, None, true), None);
}

// TmuxIdOrderTests.listedInTheOrderMade
#[test]
fn panes_and_sessions_list_in_the_order_made() {
    let mut snapshot = Snapshot {
        active_window_id: Some("@1".into()),
        ..Snapshot::default()
    };
    for id in ["%10", "%2", "%9"] {
        let mut pane = PaneInfo::placeholder(id, "@1");
        pane.command = "bash".into();
        snapshot.panes.insert(id.into(), pane);
    }
    for id in ["$10", "$1", "$2"] {
        snapshot.sessions.insert(
            id.into(),
            SessionInfo {
                id: id.into(),
                name: format!("s{id}"),
                is_attached: false,
            },
        );
    }
    let panes: Vec<&str> = snapshot
        .active_panes()
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(panes, ["%2", "%9", "%10"]);
    let sessions: Vec<&str> = snapshot
        .sorted_sessions()
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(sessions, ["$1", "$2", "$10"]);
    assert!(id_precedes("w1:p2", "w1:p10"));
    assert!(!id_precedes("$2", "$2"));
}
