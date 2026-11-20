//! B751 on a private herdr (`--session mw10b-…` under a temp HOME, its
//! socket checked to be its own): `herdr pane read <id> --format ansi`, the
//! overview's picture of a herdr pane never shown, moves nothing. Not the
//! focus (B683 pinned who moves it), not the layout, not a pane's size, not
//! who owns the size: herdr's whole snapshot reads the same after reading
//! every pane.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_b751_herdr_read -- --ignored --nocapture
//! ```

mod support;

use support::PrivateHerdr;

#[test]
#[ignore = "live: a private herdr server"]
fn reading_a_pane_moves_nothing() {
    if !PrivateHerdr::available() {
        eprintln!("skipped: no herdr");
        return;
    }
    let herdr = PrivateHerdr::start();
    let a = herdr.panes()[0].clone();
    herdr.run(&["pane", "split", &a, "--direction", "right"]);
    let b = herdr
        .panes()
        .into_iter()
        .find(|pane| *pane != a)
        .expect("the split's pane");
    if herdr.run(&["agent", "focus", &a]).1 != 0 || herdr.focused().as_deref() != Some(&a) {
        herdr.run(&["pane", "zoom", "--pane", &a, "--on"]);
        herdr.run(&["pane", "zoom", "--pane", &a, "--off"]);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let focused = herdr.focused();
    let (before, _) = herdr.run(&["api", "snapshot"]);
    for pane in [&a, &b, &a, &b] {
        let (text, code) = herdr.run(&["pane", "read", pane, "--format", "ansi"]);
        assert_eq!(code, 0, "{pane}: {text}");
        println!("read {pane}: {} lines", text.lines().count());
    }
    let (after, _) = herdr.run(&["api", "snapshot"]);
    assert_eq!(herdr.focused(), focused, "the focus stayed");
    assert_eq!(before, after, "the snapshot is the same after the reads");
}

/// The read the app sends (`control::pane_read`) gives the pane's screen
/// from its top row, not the last lines of scrollback and screen together:
/// after a clear (Claude's redraw) herdr's default source answered lines
/// from before it, and a swipe's peek showed an older screen than the one
/// that replaced it.
#[test]
#[ignore = "live: a private herdr server"]
fn the_read_is_the_screen_after_a_clear() {
    if !PrivateHerdr::available() {
        eprintln!("skipped: no herdr");
        return;
    }
    let herdr = PrivateHerdr::start();
    let pane = herdr.panes()[0].clone();
    let script = herdr.home.join("bin/paint");
    std::fs::write(
        &script,
        "#!/bin/sh\ni=0; while [ $i -lt 200 ]; do echo \"old line $i\"; i=$((i+1)); done\n\
         printf '\\033[2J\\033[H'\nprintf 'TOP visible\\n'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    herdr.run(&["pane", "run", &pane, "paint"]);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let command = moshpit_herdr::control::pane_read(&format!("{pane}"));
    let args: Vec<&str> = command.split_whitespace().map(|arg| arg.trim_matches('\'')).collect();
    let (text, code) = herdr.run(&args);
    assert_eq!(code, 0, "{text}");
    assert!(text.starts_with("TOP visible"), "the screen from its top: {text:?}");
    assert!(!text.contains("old line"), "no scrollback: {text:?}");
}
