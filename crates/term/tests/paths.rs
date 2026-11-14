//! Paths as links (T3): the paths on screen are offered to the host as
//! candidates; the ones it confirms draw as links on exactly their own
//! cells (wide ones and wrapped ones included) and a tap on any of them
//! opens the URI the host gave; whatever links a path's first cell already
//! (OSC 8, a URL) wins. OSC 7 reaches the host as an event. The app's
//! RemotePathsTests tagging cases, on the core.

mod common;
use common::*;
use moshpit_term::{Event, Frame, Terminal, cell_flags};

const KINDS: &[&str] = &["png", "jpg", "mp4", "md", "pdf", "txt"];

fn uri(path: &str) -> String {
    format!("moshpit-preview://{path}")
}

/// The viewport columns of `row` a frame draws as links.
fn link_cols(frame: &Frame, row: u16) -> Vec<usize> {
    frame
        .changed
        .iter()
        .find(|r| r.row == row)
        .map(|r| {
            r.cells
                .iter()
                .enumerate()
                .filter(|(_, c)| c.flags & cell_flags::LINK != 0)
                .map(|(i, _)| i)
                .collect()
        })
        .unwrap_or_default()
}

fn confirm(t: &mut Terminal, links: &[(&str, &str)]) -> bool {
    t.set_path_links(
        links
            .iter()
            .map(|(token, path)| (false, token.to_string(), uri(path))),
    )
}

#[test]
fn a_confirmed_path_behind_cjk_prose_draws_and_taps_on_its_own_cells() {
    let mut t = term(60, 6, 100);
    t.set_path_rules(KINDS);
    let prefix = "  3. 对比图："; // 2 + 3 + 3 wide + 1 wide = 13 columns
    t.feed(
        format!(
            "{prefix}qchar/out/GripLab.png（准备）\r\nsee https://example.com/x.png out/b.png\r\n"
        )
        .as_bytes(),
    );
    assert_eq!(
        t.path_candidates(),
        [
            ("qchar/out/GripLab.png".to_string(), false),
            ("out/b.png".to_string(), false)
        ],
        "the URL's own path is not offered"
    );
    let first = t.frame();
    assert!(
        link_cols(&first, 0).is_empty(),
        "nothing is a link before the host answers"
    );
    let url_cols = link_cols(&first, 1);

    assert!(confirm(
        &mut t,
        &[("qchar/out/GripLab.png", "/r/qchar/out/GripLab.png")]
    ));
    assert!(
        !confirm(
            &mut t,
            &[("qchar/out/GripLab.png", "/r/qchar/out/GripLab.png")]
        ),
        "the same set again changes nothing"
    );
    let f = t.frame();
    assert!(f.full, "a new set redraws");
    let start = 13;
    let len = "qchar/out/GripLab.png".len();
    assert_eq!(link_cols(&f, 0), (start..start + len).collect::<Vec<_>>());
    let (link, runs) = t.link_at(0, start as u16).expect("a tap on its first cell");
    assert_eq!(link, uri("/r/qchar/out/GripLab.png"));
    assert_eq!(runs, vec![(0, start as u16, (start + len - 1) as u16)]);
    assert!(t.link_at(0, (start + len - 1) as u16).is_some());
    assert!(t.link_at(0, (start - 1) as u16).is_none());
    assert!(t.link_at(0, (start + len) as u16).is_none());
    assert_eq!(link_cols(&f, 1), url_cols, "the URL keeps its own link");
    assert_eq!(t.link_at(1, 4).unwrap().0, "https://example.com/x.png");
}

#[test]
fn a_wide_name_is_a_link_across_its_wide_columns() {
    let mut t = term(60, 4, 0);
    t.set_path_rules(KINDS);
    t.feed("改稿：episodes/nolift/改稿建议.md。\r\n".as_bytes());
    confirm(
        &mut t,
        &[(
            "episodes/nolift/改稿建议.md",
            "/r/episodes/nolift/改稿建议.md",
        )],
    );
    let f = t.frame();
    let first = 6;
    let last = first + "episodes/nolift/".len() + 4 * 2 + ".md".len() - 1;
    assert_eq!(
        link_cols(&f, 0),
        (first..=last).collect::<Vec<_>>(),
        "the full-width period is not"
    );
    assert_eq!(
        t.link_at(0, last as u16).unwrap().0,
        uri("/r/episodes/nolift/改稿建议.md")
    );
}

#[test]
fn a_cjk_named_path_is_a_link_from_the_start_the_host_confirmed() {
    let mut t = term(60, 4, 0);
    t.set_path_rules(KINDS);
    t.feed("已经输出到成品/粗剪.mp4。\r\n".as_bytes());
    let offered: Vec<String> = t.path_candidates().into_iter().map(|c| c.0).collect();
    assert_eq!(
        offered[0], "已经输出到成品/粗剪.mp4",
        "every start, longest first"
    );
    assert!(offered.contains(&"成品/粗剪.mp4".to_string()));
    assert!(offered.contains(&"/粗剪.mp4".to_string()));
    confirm(&mut t, &[("成品/粗剪.mp4", "/r/成品/粗剪.mp4")]);
    let f = t.frame();
    let first = 5 * 2;
    let last = first + 2 * 2 + 1 + 2 * 2 + ".mp4".len() - 1;
    assert_eq!(
        link_cols(&f, 0),
        (first..=last).collect::<Vec<_>>(),
        "the prose before it stays prose"
    );
    // Settled: the shorter starts inside it are not asked about again.
    let offered: Vec<String> = t.path_candidates().into_iter().map(|c| c.0).collect();
    assert!(
        offered.contains(&"成品/粗剪.mp4".to_string()),
        "still offered: the host keeps it confirmed"
    );
    assert!(!offered.contains(&"品/粗剪.mp4".to_string()));
    assert!(!offered.contains(&"/粗剪.mp4".to_string()));
}

#[test]
fn a_wrapped_path_is_one_link_over_both_rows() {
    let mut t = term(12, 4, 0);
    t.set_path_rules(KINDS);
    t.feed(b"ab out/abcdefgh.png\r\n");
    assert_eq!(
        t.path_candidates(),
        [("out/abcdefgh.png".to_string(), false)]
    );
    confirm(&mut t, &[("out/abcdefgh.png", "/r/out/abcdefgh.png")]);
    let f = t.frame();
    assert_eq!(link_cols(&f, 0), (3..12).collect::<Vec<_>>());
    assert_eq!(link_cols(&f, 1), (0..7).collect::<Vec<_>>());
    let (_, runs) = t.link_at(1, 2).unwrap();
    assert_eq!(runs, vec![(0, 3, 11), (1, 0, 6)]);
}

#[test]
fn whatever_links_a_path_already_wins() {
    let mut t = term(40, 4, 0);
    t.set_path_rules(KINDS);
    t.feed(b"\x1b]8;;https://example.com/\x1b\\out/a.png\x1b]8;;\x1b\\ out/b.png\r\n");
    assert_eq!(
        t.path_candidates(),
        [("out/b.png".to_string(), false)],
        "an OSC 8 link is its own"
    );
    confirm(
        &mut t,
        &[("out/a.png", "/r/out/a.png"), ("out/b.png", "/r/out/b.png")],
    );
    assert_eq!(t.link_at(0, 0).unwrap().0, "https://example.com/");
    assert_eq!(t.link_at(0, 10).unwrap().0, uri("/r/out/b.png"));
}

#[test]
fn folders_and_files_are_confirmed_apart() {
    let mut t = term(40, 4, 0);
    t.set_path_rules(KINDS);
    t.feed(b"see out/plots\r\n");
    assert_eq!(t.path_candidates(), [("out/plots".to_string(), true)]);
    // Confirmed as a file: not this candidate.
    confirm(&mut t, &[("out/plots", "/r/out/plots")]);
    assert!(t.link_at(0, 5).is_none());
    t.set_path_links([(true, "out/plots".to_string(), uri("/r/out/plots"))]);
    assert_eq!(t.link_at(0, 5).unwrap().0, uri("/r/out/plots"));
}

#[test]
fn turned_off_there_are_no_candidates_and_no_links() {
    let mut t = term(40, 4, 0);
    t.feed(b"out/a.png\r\n");
    assert!(
        t.path_candidates().is_empty(),
        "off until the platform says what it previews"
    );
    assert!(
        !confirm(&mut t, &[("out/a.png", "/r/out/a.png")]),
        "nothing to confirm while off"
    );
    t.set_path_rules(KINDS);
    confirm(&mut t, &[("out/a.png", "/r/out/a.png")]);
    assert!(t.link_at(0, 1).is_some());
    t.set_path_rules(&[]);
    assert!(t.path_candidates().is_empty());
    assert!(
        t.link_at(0, 1).is_none(),
        "the confirmed ones are forgotten"
    );
}

#[test]
fn osc_7_reaches_the_host_with_its_url() {
    let mut t = term(40, 4, 0);
    t.feed(b"\x1b]7;file://box/tmp/a%20b\x07");
    t.feed(b"\x1b]7;not a url\x07");
    t.feed(b"\x1b]7;file://box/srv\x1b\\\x1b]2;t\x07");
    let dirs: Vec<String> = t
        .take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Directory(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(dirs, ["file://box/tmp/a%20b", "", "file://box/srv"]);
}
