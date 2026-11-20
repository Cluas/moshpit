//! The overview's snapshots of panes never shown (B751): one plain
//! `capture-pane` per pane, a few at a time; the rows go to the overview,
//! never into a pane's terminal; a pane shown meanwhile keeps its live
//! screen; the overview closing leaves nothing asked or waited for.

use moshpit_tmux::controller::{Config, Notification};
use moshpit_tmux::rig::{Rig, ok};

/// One session, four windows: `@0` (shown: `%0`, `%1`), `@1` (`%2`,
/// `%3`, `%4`), `@2` (`%5`), `@3` (`%6`). Five panes never shown.
const SEVEN_PANES: &str = "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n$0 @1 1 81x24,0,0,1 0 1 logs\n$0 @2 2 81x24,0,0,2 0 1 web\n$0 @3 3 81x24,0,0,3 0 1 api\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @0 1 80 24 0 0 0 bash\n%2 @1 0 80 24 1 0 0 tail\n%3 @1 1 80 24 0 0 0 tail\n%4 @1 2 80 24 0 0 0 tail\n%5 @2 0 80 24 1 0 0 node\n%6 @3 0 80 24 1 0 0 node\n%end 3 3 1\n\n";

const ALL: [&str; 7] = ["%0", "%1", "%2", "%3", "%4", "%5", "%6"];
const NEVER_SHOWN: [&str; 5] = ["%2", "%3", "%4", "%5", "%6"];

fn attached() -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    rig.push(SEVEN_PANES);
    rig.answered = rig.answered.max(3);
    // The shown window's backfill and repaint, answered.
    rig.respond(Box::new(|command, _| {
        if command.starts_with("capture-pane -p -e -t ") {
            return None;
        }
        ok(["0 0"])
    }));
    rig.settle();
    rig
}

fn panes(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| (*p).to_owned()).collect()
}

/// The snapshot commands sent so far, by pane.
fn asked(rig: &Rig) -> Vec<String> {
    rig.commands
        .iter()
        .filter_map(|c| c.strip_prefix("capture-pane -p -e -t "))
        .map(str::to_owned)
        .collect()
}

fn snapshots(rig: &Rig) -> Vec<(String, Vec<String>)> {
    rig.notes()
        .into_iter()
        .filter_map(|n| match n {
            Notification::PaneSnapshot { pane, rows } => Some((pane, rows)),
            _ => None,
        })
        .collect()
}

/// Answers every snapshot asked with its pane's screen.
fn answer_screens(rig: &mut Rig) {
    rig.respond(Box::new(|command, _| {
        let pane = command.strip_prefix("capture-pane -p -e -t ")?;
        ok([format!("screen of {pane}"), "$ ".to_owned()])
    }));
    rig.settle();
}

#[test]
fn an_overview_of_n_unvisited_panes_asks_n_snapshots_and_fills_n_cards() {
    let mut rig = attached();
    // The shown window's panes got their screens already: not asked.
    rig.with(|c, now| c.snapshot_panes(&panes(&ALL), now));
    rig.drain();
    // A few at a time: the control channel stays free.
    assert_eq!(asked(&rig).len(), 4, "{:?}", rig.commands);
    // Asked again while they are on their way (the tree re-posted):
    // nothing new goes out.
    rig.with(|c, now| c.snapshot_panes(&panes(&ALL), now));
    rig.drain();
    assert_eq!(asked(&rig).len(), 4);
    assert_eq!(rig.controller.snapshots_pending(), NEVER_SHOWN.len());
    answer_screens(&mut rig);
    let mut asked = asked(&rig);
    asked.sort();
    assert_eq!(asked, NEVER_SHOWN, "each never-shown pane once");
    let heard = snapshots(&rig);
    assert_eq!(heard.len(), NEVER_SHOWN.len(), "{heard:?}");
    for (pane, rows) in &heard {
        assert_eq!(rows, &[format!("screen of {pane}"), "$ ".to_owned()]);
    }
    assert_eq!(rig.controller.snapshots_pending(), 0);
}

#[test]
fn a_snapshot_never_reaches_a_panes_terminal() {
    let mut rig = attached();
    let before: Vec<Vec<u8>> = ALL.iter().map(|p| rig.pane_bytes(p)).collect();
    rig.with(|c, now| c.snapshot_panes(&panes(&ALL), now));
    answer_screens(&mut rig);
    assert_eq!(snapshots(&rig).len(), NEVER_SHOWN.len());
    for (pane, was) in ALL.iter().zip(before) {
        let now = rig.pane_bytes(pane);
        assert_eq!(now, was, "{pane}'s byte stream untouched");
        assert!(
            !String::from_utf8_lossy(&now).contains("screen of"),
            "{pane}: a snapshot in its terminal"
        );
    }
}

#[test]
fn a_snapshot_dropped_because_the_live_screen_came_first() {
    // Asked: on its way, its answer not read yet...
    let mut rig = attached();
    rig.with(|c, now| c.snapshot_panes(&panes(&["%2"]), now));
    rig.drain();
    assert_eq!(asked(&rig), ["%2"]);
    // ...when the person goes to %2's window: its live screen is asked for,
    // to come in its own stream.
    rig.with(|c, now| c.select_window("@1", now));
    rig.drain();
    assert!(
        rig.has("capture-pane -p -e -t %2"),
        "the repaint goes out at once: {:?}",
        rig.commands
    );
    // The snapshot's answer comes in after that: dropped, never shown in
    // the live screen's place, never in the pane's terminal.
    answer_screens(&mut rig);
    assert!(snapshots(&rig).is_empty(), "{:?}", snapshots(&rig));
    assert!(!String::from_utf8_lossy(&rig.pane_bytes("%2")).contains("screen of"));
}

#[test]
fn a_snapshot_still_queued_for_a_pane_shown_is_never_sent() {
    let mut rig = attached();
    // Four on their way fill the lane; %2 waits behind them.
    rig.with(|c, now| c.snapshot_panes(&panes(&["%3", "%4", "%5", "%6", "%2"]), now));
    rig.drain();
    assert!(!asked(&rig).contains(&"%2".to_owned()), "{:?}", asked(&rig));
    rig.with(|c, now| c.select_window("@1", now));
    answer_screens(&mut rig);
    assert!(!asked(&rig).contains(&"%2".to_owned()), "{:?}", asked(&rig));
    assert!(!snapshots(&rig).iter().any(|(p, _)| p == "%2"));
    assert_eq!(rig.controller.snapshots_pending(), 0);
}

#[test]
fn closing_the_overview_leaves_nothing_asked_or_waited_for() {
    let mut rig = attached();
    let before = rig.commands.len();
    rig.with(|c, now| c.snapshot_panes(&panes(&ALL), now));
    rig.drain();
    assert_eq!(rig.controller.snapshots_pending(), NEVER_SHOWN.len());
    rig.with(|c, now| c.stop_snapshots(now));
    assert_eq!(rig.controller.snapshots_pending(), 0, "nothing subscribed");
    // The four on their way answer: nobody hears them, and the queued one
    // is never sent.
    answer_screens(&mut rig);
    assert!(snapshots(&rig).is_empty());
    assert_eq!(asked(&rig).len(), 4);
    // Nothing but the four captures went out on the panes' behalf: no
    // size, no subscription.
    assert!(
        rig.since(before)
            .iter()
            .all(|c| c.starts_with("capture-pane -p -e -t ")),
        "{:?}",
        rig.since(before)
    );
}

#[test]
fn a_refused_snapshot_says_nothing_and_the_queue_moves_on() {
    let mut rig = attached();
    rig.respond(Box::new(|command, _| {
        let pane = command.strip_prefix("capture-pane -p -e -t ")?;
        if pane == "%2" {
            return moshpit_tmux::rig::err(["can't find pane: %2"]);
        }
        ok([format!("screen of {pane}")])
    }));
    rig.with(|c, now| c.snapshot_panes(&panes(&ALL), now));
    rig.settle();
    let heard: Vec<String> = snapshots(&rig).into_iter().map(|(p, _)| p).collect();
    assert!(!heard.contains(&"%2".to_owned()));
    assert_eq!(heard.len(), NEVER_SHOWN.len() - 1);
    assert_eq!(rig.controller.snapshots_pending(), 0);
}
