//! B841: a URL over several rows copied on the phone came out with a space
//! where a row ended. The copy held a line break there, and a single-line
//! field pastes a break as a space. Each test is one way the rows of a URL
//! can reach the copy. Claude Code (2.1.325, in tmux 3.6a) breaks a long URL
//! at the right edge itself and links every row to the whole address
//! (`claude_code_output`): the copy that kept the break was a selection
//! reaching past the link, from the bullet before it or into the blanks
//! and rows after it.

mod common;
use common::*;
use moshpit_term::SelectKind;

const URL: &str = "https://example.com/docx/ZqLmOFywpjhAb3dEfGh/edit?usp=sharing";

/// A selection reaching past the URL (the words before it, the line after)
/// keeps the breaks before and after it, and loses the ones inside it.
#[test]
fn a_selection_past_a_broken_url_joins_only_the_url() {
    // The URL starts after "See: " and the program broke it at the edge.
    let lead = "See: ";
    let (first, rest) = URL.split_at(40 - lead.len());
    let mut t = term(40, 6, 100);
    t.feed(format!("{lead}{first}\r\n{rest} done\r\nnext line\r\n").as_bytes());
    assert_eq!(t.link_at(1, 2).map(|l| l.0).as_deref(), Some(URL));
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(2, 8, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some(format!("{lead}{URL} done\nnext line").as_str())
    );

    // The same with an OSC 8 link a program re-opened on each row.
    let osc8 = |text: &str| format!("\x1b]8;;{URL}\x1b\\{text}\x1b]8;;\x1b\\");
    let mut t = term(40, 6, 100);
    t.feed(
        format!(
            "{lead}{}\r\n  {} done\r\nnext line\r\n",
            osc8(first),
            osc8(rest)
        )
        .as_bytes(),
    );
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(2, 8, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some(format!("{lead}{URL} done\nnext line").as_str())
    );
}

/// tmux's capture of a pane (the attach's repaint, the history above it)
/// has no soft wraps: a URL the terminal wrapped comes back as full rows
/// broken by the capture. It still copies as one address.
#[test]
fn a_url_from_a_tmux_capture_copies_whole() {
    // What `capture-pane -p -e` gives for `echo $URL` in 24 columns: the
    // rows the terminal wrapped, each a line of its own.
    let rows: Vec<&str> = URL
        .as_bytes()
        .chunks(24)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    let mut t = term(24, 8, 100);
    t.feed(format!("{}\r\n$ ", rows.join("\r\n")).as_bytes());
    let last = rows.len() as u16 - 1;
    assert_eq!(t.link_at(last, 1).map(|l| l.0).as_deref(), Some(URL));
    t.select_start(SelectKind::Semantic, 1, 3, false);
    assert_eq!(t.selection_text().as_deref(), Some(URL), "a long press");
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(last, 23, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some(URL),
        "a drag over its rows"
    );
}

/// The link's own address, as a tap's Copy Link gives it: whole, for every
/// way its rows were broken.
#[test]
fn copy_link_is_the_whole_address() {
    let (first, rest) = URL.split_at(38);
    let mut t = term(40, 6, 100);
    t.feed(format!("  {first}\r\n  {rest}\r\n").as_bytes());
    for (row, col) in [(0, 5), (1, 4)] {
        assert_eq!(
            t.link_at(row, col).map(|l| l.0).as_deref(),
            Some(URL),
            "row {row}"
        );
    }
}

/// Claude Code's long URL, as 2.1.325 printed it in a 60-column tmux 3.6a
/// pane: "⏺ " and the address, its first 58 characters to the right edge,
/// the rest on the next row behind a 2-column indent the cursor jumps (it
/// never writes it), each row inside an OSC 8 link to the whole address
/// (one id, BEL-terminated), re-opened on the second.
const CC_URL: &str = "https://example.com/docx/ZqLmOFywpjhAb3dEfGh/edit?usp=sharing&ouid=112233445566778899&rtpof=true&sd=true";

fn claude_code_output() -> String {
    let (first, rest) = CC_URL.split_at(58);
    let open = format!("\x1b]8;id=zts17q;{CC_URL}\x07");
    format!(
        "\x1b[38;5;231m\x1b[49m⏺\x1b[39m {open}\x1b[94m{first}\x1b]8;;\x07\r\x1b[2C\x1b[1B{open}{rest}\x1b[39m\x1b]8;;\x07\r\n\r\n✻ Baked for 3s"
    )
}

/// The same two rows as tmux's `capture-pane -p -e` gives them back (the
/// attach's repaint): the indent written as blanks, ST-terminated links.
fn claude_code_captured() -> String {
    let (first, rest) = CC_URL.split_at(58);
    let open = format!("\x1b]8;id=zts17q;{CC_URL}\x1b\\");
    format!(
        "\x1b[38;5;231m\x1b[49m⏺\x1b[39m \x1b[94m{open}{first}\x1b]8;;\x1b\\\r\n\x1b[39m  \x1b[94m{open}{rest}\x1b]8;;\x1b\\\x1b[39m\r\n\r\n✻ Baked for 3s"
    )
}

#[test]
fn claude_codes_bullet_is_one_column() {
    let mut t = term(60, 8, 100);
    t.feed(claude_code_output().as_bytes());
    let r = rows(&t);
    assert!(r[0].starts_with("⏺ https://"), "{:?}", r[0]);
    assert!(
        r[0].ends_with("usp=shar"),
        "the first row ends at the edge: {:?}",
        r[0]
    );
    assert_eq!(r[1], format!("  {}", &CC_URL[58..]));
}

#[test]
fn claude_codes_url_copies_whole() {
    for (how, bytes) in [
        ("live", claude_code_output()),
        ("captured", claude_code_captured()),
    ] {
        let mut t = term(60, 8, 100);
        t.feed(bytes.as_bytes());
        assert_eq!(
            t.link_at(1, 5).map(|l| l.0).as_deref(),
            Some(CC_URL),
            "{how}: the tail's link"
        );
        for row in [0, 1] {
            t.select_start(SelectKind::Semantic, row, 10, false);
            assert_eq!(
                t.selection_text().as_deref(),
                Some(CC_URL),
                "{how}: a long press on row {row}"
            );
        }
        t.select_start(SelectKind::Cells, 0, 2, false);
        t.select_extend(1, 59, true);
        assert_eq!(
            t.selection_text().as_deref(),
            Some(CC_URL),
            "{how}: a drag from its first character"
        );
        t.select_start(SelectKind::Cells, 0, 0, false);
        t.select_extend(1, 59, true);
        assert_eq!(
            t.selection_text().as_deref(),
            Some(format!("⏺ {CC_URL}").as_str()),
            "{how}: a drag from the bullet"
        );
    }
}

/// What stays a break: a complete URL that happens to end at the edge,
/// then prose; a label broken over rows; the rows after a link.
#[test]
fn breaks_that_are_not_inside_a_link_stay() {
    let whole = "https://example.com/abcdefghijklmnopqr";
    let mut t = term(40, 6, 100);
    t.feed(format!("  {whole}\r\n  and then more\r\n").as_bytes());
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(1, 39, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some(format!("  {whole}\n  and then more").as_str()),
        "a URL that ends at the edge, then prose"
    );

    let url = "https://example.com/x";
    let osc8 = |text: &str| format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\");
    let mut t = term(12, 6, 100);
    t.feed(format!("> {}\r\n{} <\r\n", osc8("the proj"), osc8("docs page")).as_bytes());
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(1, 11, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("> the proj\ndocs page <"),
        "a label keeps its rows in a larger selection"
    );

    let mut t = term(60, 8, 100);
    t.feed(claude_code_output().as_bytes());
    t.select_start(SelectKind::Cells, 0, 2, false);
    t.select_extend(3, 20, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some(format!("{CC_URL}\n\n✻ Baked for 3s").as_str()),
        "the rows after the link"
    );
}
