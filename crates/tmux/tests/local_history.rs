//! What a tmux pane's bytes leave in the terminal core's history (B364):
//! the controller's backfill and repair frames, fed to the terminal the app
//! draws a pane with (tmux's scroll-on-clear on), must leave tmux's history
//! — line for line, nothing missing, nothing twice — however many times the
//! screen is repainted.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use moshpit_term::{Options, ScrollOnClear, Terminal};
use moshpit_tmux::controller::Config;
use moshpit_tmux::rig::{Rig, Seen, ok};

const COLS: i64 = 80;
const ROWS: i64 = 24;

/// tmux's history above the screen.
fn history() -> Vec<String> {
    (1..=100).map(|n| format!("history {n}")).collect()
}

/// tmux's screen.
fn screen() -> Vec<String> {
    let mut rows: Vec<String> = (1..ROWS).map(|n| format!("screen {n}")).collect();
    rows.push("$ ".to_owned());
    rows
}

/// Attached, one window with one primary-screen pane, its view reporting
/// the grid; tmux answers the backfill with `history()` (the first one
/// laid out for a wider client when `first_dump_wide`), every frame with
/// `screen()`, every cursor query with the prompt's place.
fn attached_with(first_dump_wide: bool) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(COLS, ROWS));
    rig.attach();
    let dumps = Arc::new(AtomicUsize::new(0));
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("capture-pane -p -e -S") {
            if first_dump_wide && dumps.fetch_add(1, Ordering::SeqCst) == 0 {
                return ok(["w".repeat(COLS as usize + 20)]);
            }
            return ok(history());
        }
        if command.starts_with("capture-pane") {
            return ok(screen());
        }
        if command.starts_with("display-message -p -t %0 '#{cursor_x} #{cursor_y}'") {
            return ok([format!("2 {}", ROWS - 1)]);
        }
        ok(["0 0"])
    }));
    rig.push(&format!(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 {COLS}x{ROWS},0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 {COLS} {ROWS} 1 0 0 bash\n%end 3 3 1\n\n"
    ));
    rig.answered = rig.answered.max(3);
    rig.with(|c, now| c.view_grid("%0", COLS, ROWS, now));
    rig.settle();
    rig
}

fn attached() -> Rig {
    attached_with(false)
}

/// Everything the controller gave the pane's terminal, fed to the
/// terminal the app draws it with.
fn terminal(rig: &Rig) -> Terminal {
    let mut term = Terminal::new(
        COLS as u16,
        ROWS as u16,
        Options {
            scroll_on_clear: ScrollOnClear::Tmux,
            ..Options::default()
        },
    );
    for seen in &rig.seen {
        if let Seen::Pane(pane, bytes) = seen
            && pane == "%0"
        {
            term.feed(bytes);
        }
    }
    term.flush_sync();
    term
}

fn local_history(term: &Terminal) -> Vec<String> {
    let history = term.history_len() as i32;
    (-history..0).map(|line| term.row_text(line).0).collect()
}

fn local_screen(term: &Terminal) -> Vec<String> {
    (0..ROWS as i32).map(|line| term.row_text(line).0).collect()
}

/// tmux's screen as the terminal reads a row back (trailing blanks
/// trimmed).
fn screen_rows() -> Vec<String> {
    screen()
        .into_iter()
        .map(|row| row.trim_end().to_owned())
        .collect()
}

#[test]
fn the_attach_leaves_tmuxs_history_then_its_screen() {
    let rig = attached();
    let term = terminal(&rig);
    assert_eq!(local_history(&term), history());
    assert_eq!(local_screen(&term), screen_rows());
}

/// Every repaint blanks the screen it redraws without pushing it into the
/// history: a frame after each Enter, a refresh, a switch's two passes —
/// the history stays tmux's (it gained one copy of the screen a repaint).
#[test]
fn repaints_leave_the_history_as_it_was() {
    let mut rig = attached();
    for _ in 0..3 {
        rig.with(|c, now| c.view_return_sent("%0", now));
        rig.wait_settled(500, 50);
    }
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    rig.settle();
    rig.with(|c, now| c.refresh(now));
    rig.wait_settled(1500, 50);
    let term = terminal(&rig);
    assert_eq!(local_history(&term), history());
    assert_eq!(local_screen(&term), screen_rows());
}

/// A dump laid out for another client's width is refused and taken again
/// once the sizes settle; the retry lands on a screen already painted. It
/// replaces whatever the history held, and the screen is drawn on top
/// again: tmux's history, then its screen.
#[test]
fn a_dump_retried_onto_a_painted_screen_leaves_tmuxs_history() {
    let mut rig = attached_with(true);
    rig.wait_settled(3000, 100);
    assert!(
        rig.commands
            .iter()
            .filter(|c| c.starts_with("capture-pane -p -e -S"))
            .count()
            >= 2,
        "the dump was taken again: {:?}",
        rig.commands
    );
    let term = terminal(&rig);
    assert_eq!(local_history(&term), history());
    assert_eq!(local_screen(&term), screen_rows());
}
