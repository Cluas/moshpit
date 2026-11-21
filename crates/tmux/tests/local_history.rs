//! What a tmux pane's bytes leave in the terminal core's history (B364):
//! the controller's backfill and repair frames, fed to the terminal the app
//! draws a pane with (tmux's scroll-on-clear on), must leave tmux's history
//! — line for line, nothing missing, nothing twice — however many times the
//! screen is repainted.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use moshpit_term::{Options, ScrollOnClear, Terminal};
use moshpit_tmux::controller::Config;
use moshpit_tmux::rig::{Rig, Seen, block, ok};

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

/// tmux as a pane printing `L1`, `L2`, … : `printed` lines so far, the
/// screen holding the last `ROWS - 1` above an empty cursor row.
fn printing_pane_answer(command: &str, printed: i64) -> Vec<String> {
    let first_on_screen = printed - (ROWS - 2);
    if command.starts_with("capture-pane -p -e -S") {
        return (1..first_on_screen).map(|n| format!("L{n}")).collect();
    }
    if command.starts_with("capture-pane") {
        let mut rows: Vec<String> = (first_on_screen..=printed)
            .map(|n| format!("L{n}"))
            .collect();
        rows.push(String::new());
        return rows;
    }
    if command.starts_with("display-message -p -t %0 '#{cursor_x} #{cursor_y}") {
        return vec![format!("0 {} 0 0", ROWS - 1)];
    }
    vec!["0 0".to_owned()]
}

/// The `L<n>` rows of the terminal, history then screen.
fn printed_rows(term: &Terminal) -> Vec<i64> {
    let history = term.history_len() as i32;
    (-history..ROWS as i32)
        .filter_map(|line| term.row_text(line).0.strip_prefix('L')?.parse().ok())
        .collect()
}

/// B585: the attach's history dump and its screen frame were two captures a
/// round trip apart — the frame went out only once the dump's reply was in.
/// A pane printing in between scrolled lines off tmux's screen that were in
/// neither: gone from the phone's history (the output itself pushed blank
/// rows there instead). tmux runs the commands a client sent together back
/// to back; here, what the pane prints lands right after the batch the dump
/// went out in.
#[test]
fn a_pane_printing_through_the_attach_loses_no_line() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(COLS, ROWS));
    rig.attach();
    rig.push(&format!(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 {COLS}x{ROWS},0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 {COLS} {ROWS} 1 0 0 bash\n%end 3 3 1\n\n"
    ));
    rig.answered = rig.answered.max(3);
    rig.with(|c, now| c.view_grid("%0", COLS, ROWS, now));
    let mut printed = 150;
    let mut batch_end: Option<usize> = None;
    let mut block_number = 500;
    for _ in 0..400 {
        if batch_end.is_some_and(|end| rig.answered >= end) && printed == 150 {
            // The pane prints three lines once that batch has run.
            rig.push("%output %0 L151\\015\\012L152\\015\\012L153\\015\\012\n");
            printed = 153;
        }
        let Some(command) = rig.commands.get(rig.answered).cloned() else {
            break;
        };
        rig.answered += 1;
        if batch_end.is_none() && command.starts_with("capture-pane -p -e -S") {
            batch_end = Some(rig.commands.len());
        }
        block_number += 1;
        let lines = printing_pane_answer(&command, printed);
        rig.push(&block(block_number, &lines, false));
    }
    assert_eq!(printed, 153, "the dump went out: {:?}", rig.commands);
    rig.respond(Box::new(|command, _| {
        ok(printing_pane_answer(command, 153))
    }));
    rig.wait_settled(3000, 100);
    let term = terminal(&rig);
    let rows = printed_rows(&term);
    let missing: Vec<i64> = (1..=153).filter(|n| !rows.contains(n)).collect();
    assert_eq!(rows, (1..=153).collect::<Vec<i64>>(), "missing {missing:?}");
}

/// tmux's screen while a TUI redraws row by row (`ESC[<row>;1H` + text,
/// as Claude Code and the screenshot stage draw): row 5 is full to the
/// last column (pending wrap) and the next row's cursor move is cut in
/// two — tmux's pty reads split it, `ESC[2` closing one `%output` and
/// `0;1H…` opening the next. A repaint (capture + cursor) answered in
/// between: tmux holds the half sequence in its own parser, so its frame
/// and cursor know nothing of it.
fn split_sequence_screen() -> Vec<String> {
    let mut rows: Vec<String> = (1..ROWS).map(|n| format!("row {n}")).collect();
    rows[4] = "x".repeat(COLS as usize);
    rows.push(String::new());
    rows
}

fn split_sequence_rig() -> Rig {
    let mut rig = attached();
    rig.respond(Box::new(|command, _| {
        if command.starts_with("capture-pane -p -e -S") {
            return ok(history());
        }
        if command.starts_with("capture-pane") {
            return ok(split_sequence_screen());
        }
        if command.starts_with("display-message -p -t %0 '#{cursor_x} #{cursor_y}") {
            // Right after the full row: tmux's cursor on its last column.
            return ok([format!("{} 4 0 0", COLS - 1)]);
        }
        ok(["0 0"])
    }));
    rig
}

fn draw_full_row_then_split(rig: &mut Rig) {
    let full = "x".repeat(COLS as usize);
    rig.push(&format!("%output %0 \\033[5;1H{full}\\033[2\n"));
}

fn finish_split_row(rig: &mut Rig) {
    rig.push("%output %0 0;1Hthe next row\n");
}

/// The 02-pane screenshot's garble, en-US iPhone: a repaint landing
/// between the two halves of a split escape sequence ate the first half,
/// so `0;1H` was printed as text after the full row (wrapping onto the
/// next) and the row it should have moved to was drawn in the wrong
/// place. The terminal must read the sequence whole, as tmux did.
#[test]
fn a_repaint_between_the_halves_of_a_split_sequence_keeps_it_whole() {
    let mut rig = split_sequence_rig();
    draw_full_row_then_split(&mut rig);
    rig.with(|c, now| c.resync_active_pane_now(false, now));
    rig.settle();
    finish_split_row(&mut rig);
    let term = terminal(&rig);
    let screen = local_screen(&term);
    assert_eq!(screen[4], "x".repeat(COLS as usize), "{screen:#?}");
    assert_eq!(screen[5], "row 6", "{screen:#?}");
    assert_eq!(screen[19], "the next row", "{screen:#?}");
    assert!(!screen.iter().any(|row| row.contains(";1H")), "{screen:#?}");
}

/// The same split with the repaint's frame and its cursor answered on
/// either side of the second half: the cursor's move is the repaint's
/// last word, and the sequence still reads whole after it.
#[test]
fn a_split_sequence_survives_a_repaint_answered_around_it() {
    let mut rig = split_sequence_rig();
    draw_full_row_then_split(&mut rig);
    rig.with(|c, now| c.resync_active_pane_now(false, now));
    // The frame's reply only.
    let frame = rig.index_of("capture-pane -p -e -t").expect("frame asked");
    while rig.answered <= frame {
        let command = rig.commands[rig.answered].clone();
        rig.answered += 1;
        let lines = if command.starts_with("capture-pane -p -e -t") {
            split_sequence_screen()
        } else {
            vec!["0 0".to_owned()]
        };
        rig.push(&block(700 + rig.answered as i64, &lines, false));
    }
    rig.settle();
    finish_split_row(&mut rig);
    let term = terminal(&rig);
    let screen = local_screen(&term);
    assert_eq!(screen[4], "x".repeat(COLS as usize), "{screen:#?}");
    assert_eq!(screen[19], "the next row", "{screen:#?}");
    assert!(!screen.iter().any(|row| row.contains(";1H")), "{screen:#?}");
}
