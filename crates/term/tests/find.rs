//! Find (T3), the core's half of the find bar: the text as written, every
//! match in history and on screen counted, the newest first, round past
//! either end, the view scrolled to the match and the match drawn marked.

mod common;
use common::*;
use moshpit_term::{FindOptions, FindStep, Terminal, cell_flags};

fn found(t: &mut Terminal, text: &str, step: FindStep) -> Option<(usize, usize, (u16, u16))> {
    t.find(text, FindOptions::default(), step)
        .map(|f| (f.index, f.count, f.matched.start))
}

#[test]
fn find_counts_every_match_starts_at_the_newest_and_goes_round() {
    let mut t = term(20, 4, 100);
    for i in 0..30 {
        let word = if i % 10 == 3 { "Needle" } else { "hay" };
        t.feed(format!("{i:02} {word}\r\n").as_bytes());
    }
    // Lines 03, 13, 23: the last two in history, 23 just above the screen.
    let (index, count, _) = found(&mut t, "needle", FindStep::Newest).expect("any case");
    assert_eq!((index, count), (2, 3), "the newest of three");
    assert!(t.display_offset() > 0, "scrolled back to it");
    assert_eq!(found(&mut t, "needle", FindStep::Previous).unwrap().0, 1);
    assert_eq!(found(&mut t, "needle", FindStep::Previous).unwrap().0, 0);
    assert_eq!(
        found(&mut t, "needle", FindStep::Previous).unwrap().0,
        2,
        "round past the oldest"
    );
    assert_eq!(
        found(&mut t, "needle", FindStep::Next).unwrap().0,
        0,
        "round past the newest"
    );
    assert_eq!(
        found(&mut t, "needle", FindStep::Newest).unwrap().0,
        0,
        "the same text keeps the place"
    );
    let f = t.frame();
    let marked = f
        .changed
        .iter()
        .any(|r| r.cells.iter().any(|c| c.flags & cell_flags::MATCH != 0));
    assert!(marked, "the match draws marked");
    assert!(found(&mut t, "", FindStep::Newest).is_none());
    let f = t.frame();
    assert!(
        f.changed
            .iter()
            .all(|r| r.cells.iter().all(|c| c.flags & cell_flags::MATCH == 0)),
        "cleared"
    );
}

#[test]
fn find_takes_the_text_as_written() {
    let mut t = term(30, 4, 10);
    t.feed(b"a.b axb (x) Rust rusty\r\n");
    assert_eq!(
        found(&mut t, "a.b", FindStep::Newest).map(|f| f.1),
        Some(1),
        "a dot is a dot"
    );
    assert_eq!(
        found(&mut t, "(x)", FindStep::Newest).map(|f| f.2),
        Some((0, 8))
    );
    assert_eq!(
        found(&mut t, "rust", FindStep::Newest).map(|f| f.1),
        Some(2)
    );
    let exact = FindOptions {
        case_sensitive: true,
        ..FindOptions::default()
    };
    assert_eq!(
        t.find("rust", exact, FindStep::Newest).map(|f| f.count),
        Some(1)
    );
    let words = FindOptions {
        word_start: true,
        word_end: true,
        ..FindOptions::default()
    };
    assert_eq!(
        t.find("rust", words, FindStep::Newest).map(|f| f.count),
        Some(1),
        "rusty is not the word"
    );
    assert!(found(&mut t, "nothing here", FindStep::Newest).is_none());
}
