//! Moshpit's recorded byte streams against real truths:
//!
//! - the mosh pane-switch capture against tmux's own `capture-pane` of each
//!   phase (Tests/MoshpitTests/Fixtures/mosh-switch, the field bug of
//!   white blocks after a pane switch);
//! - every single-pane tmux `-CC` capture against the same bytes replayed
//!   into a real tmux 3.6a pane (tests/fixtures/tmux-oracle, written by
//!   spike/term-core/tmux_oracle.py): screen, history and cursor;
//! - and every recorded stream (tmux panes, herdr frames, mosh) in its
//!   recorded reads, in one read, one byte per read and in reads of a few
//!   sizes: the screen must not depend on where the network cut the bytes.

mod common;
use common::*;

#[test]
fn mosh_pane_switch_matches_tmux() {
    let dir = repo().join("Tests/MoshpitTests/Fixtures/mosh-switch");
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    let mut t = term(
        meta["cols"].as_u64().unwrap() as u16,
        meta["rows"].as_u64().unwrap() as u16,
        500,
    );
    let sgr = |s: &str| {
        // capture-pane -e's SGR escapes out, trailing blanks off.
        let mut out = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\x1b' {
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out.trim_end_matches(' ').to_string()
    };
    for phase in meta["phases"].as_array().unwrap() {
        t.feed(&std::fs::read(dir.join(phase["bytes"].as_str().unwrap())).unwrap());
        let truth = std::fs::read_to_string(dir.join(phase["truth"].as_str().unwrap())).unwrap();
        let got = rows(&t);
        for (r, line) in truth.lines().enumerate() {
            assert_eq!(got[r], sgr(line), "phase {} row {r}", phase["index"]);
        }
    }
}

#[test]
fn tmux_captures_match_a_real_tmux() {
    let dir = repo().join("crates/term/tests/fixtures/tmux-oracle");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    assert!(names.len() >= 8);
    for path in names {
        let want: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let reads = tmux_pane(
            want["capture"].as_str().unwrap(),
            want["pane"].as_u64().unwrap() as u32,
        );
        let mut t = term(
            want["cols"].as_u64().unwrap() as u16,
            want["rows"].as_u64().unwrap() as u16,
            2000,
        );
        for r in &reads {
            t.feed(r);
        }
        let strs = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect()
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(rows(&t), strs(&want["screen"]), "{name}: screen");
        let hist: Vec<String> = history(&t).into_iter().map(|r| r.0).collect();
        assert_eq!(hist, strs(&want["history"]), "{name}: history");
        let c = want["cursor"].as_array().unwrap();
        assert_eq!(
            t.cursor(),
            (c[0].as_u64().unwrap() as u16, c[1].as_u64().unwrap() as u16),
            "{name}: cursor"
        );
        assert_eq!(
            t.modes().alt_screen,
            want["alt_screen"].as_bool().unwrap(),
            "{name}: alternate screen"
        );
    }
}

/// The same bytes, cut five ways, give the same terminal.
fn cut_invariant(name: &str, cols: u16, rows: u16, reads: &[Vec<u8>]) {
    let all: Vec<u8> = reads.concat();
    let run = |cuts: &mut dyn Iterator<Item = &[u8]>| {
        let mut t = term(cols, rows, 2000);
        for c in cuts {
            t.feed(c);
        }
        state(&t)
    };
    let recorded = run(&mut reads.iter().map(|r| r.as_slice()));
    assert_eq!(
        run(&mut std::iter::once(all.as_slice())),
        recorded,
        "{name}: one read"
    );
    for size in [1, 2, 3, 7, 4096] {
        assert_eq!(
            run(&mut all.chunks(size)),
            recorded,
            "{name}: reads of {size}"
        );
    }
}

#[test]
fn recorded_streams_do_not_depend_on_their_cuts() {
    for capture in [
        "plain",
        "cjk-emoji",
        "ansi",
        "altscreen",
        "boot-chain",
        "pause",
        "session-switch",
    ] {
        for pane in [0, 1, 9] {
            let reads = tmux_pane(capture, pane);
            if !reads.is_empty() {
                cut_invariant(&format!("{capture} %{pane}"), 80, 24, &reads);
            }
        }
    }
    // herdr's frames at their first size (the stream resizes later).
    let frames = herdr_frames();
    let (cols, rows) = (frames[0].0, frames[0].1);
    let same: Vec<Vec<u8>> = frames
        .iter()
        .take_while(|f| (f.0, f.1) == (cols, rows))
        .map(|f| f.2.clone())
        .collect();
    cut_invariant("herdr frames", cols, rows, &same);
    let dir = repo().join("Tests/MoshpitTests/Fixtures/mosh-switch");
    let mosh: Vec<Vec<u8>> = (0..4)
        .map(|i| std::fs::read(dir.join(format!("phase{i}.bin"))).unwrap())
        .collect();
    cut_invariant("mosh pane switch", 90, 50, &mosh);
}

#[test]
fn herdr_frames_follow_their_sizes() {
    // Every frame drawn at the size it says; nothing panics, the last one is
    // on screen with its cursor where the frame put it.
    let mut t = term(80, 24, 2000);
    let mut size = (80, 24);
    for (cols, rows, bytes) in herdr_frames() {
        if (cols, rows) != size {
            t.resize(cols, rows);
            size = (cols, rows);
        }
        t.feed(&bytes);
        assert_eq!(t.size(), size);
    }
    assert!(rows(&t).iter().any(|r| !r.is_empty()));
}

#[test]
fn random_escape_soup_never_panics_and_does_not_depend_on_cuts() {
    // Escape-heavy random bytes (and pieces of real sequences), fed in
    // random cuts with resizes and scrolling in between: no panic, and the
    // same screen as one read.
    let pieces: [&[u8]; 16] = [
        b"\x1b",
        b"[",
        b"?",
        b"2026h",
        b"2026l",
        b"k",
        b"\\",
        b"]8;;http://x\x07",
        b"\x1b[3J",
        "中".as_bytes(),
        "\u{1f468}\u{200d}".as_bytes(),
        "\u{fe0f}".as_bytes(),
        b"\r\n",
        b"\x1b[5;80H",
        b"\xc3",
        b"\xbc",
    ];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..200 {
        let mut bytes = Vec::new();
        let mut last_piece = usize::MAX;
        for _ in 0..400 {
            let r = next();
            if r % 3 == 0 {
                bytes.push((r >> 8) as u8);
                last_piece = usize::MAX;
            } else {
                let k = (r >> 8) as usize % pieces.len();
                // Not VS16 twice in a row: wezterm gives `9` + VS16 two
                // columns and `9` + VS16 + VS16 one, so a cut between the
                // two can leave it on the next row (docs/WEZTERM-PATCHES.md,
                // W6's one known exception).
                if k == 11 && last_piece == 11 {
                    continue;
                }
                bytes.extend_from_slice(pieces[k]);
                last_piece = k;
            }
        }
        let mut whole = term(20, 6, 50);
        whole.feed(&bytes);
        whole.flush_sync();
        let mut cut = term(20, 6, 50);
        let mut i = 0;
        while i < bytes.len() {
            let n = 1 + (next() % 9) as usize;
            cut.feed(&bytes[i..(i + n).min(bytes.len())]);
            i += n;
        }
        cut.flush_sync();
        assert_eq!(state(&whole), state(&cut), "round {round}");
        // And the rest of the API on whatever that left.
        whole.resize(7, 3);
        whole.scroll(3);
        whole.select_start(moshpit_term::SelectKind::Word, 1, 2, false);
        whole.select_extend(2, 6, true);
        let _ = whole.selection_text();
        let _ = whole.frame();
        let _ = whole.search("x", false);
        let _ = whole.link_at(0, 0);
        whole.resize(30, 9);
        let _ = whole.frame();
    }
}
