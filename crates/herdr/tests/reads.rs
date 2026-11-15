//! The overview's pictures of herdr panes the phone doesn't show (B751), on
//! the fake host in virtual time: one `pane read` each for the panes not
//! on screen, two at a time; the text goes to the overview, never into a
//! pane; a pane the phone moves to before the answer keeps its live screen;
//! the overview closing leaves nothing asked or waited for. (That a read
//! moves nothing on a real herdr is checked against a private server:
//! crates/session/tests/live_b751_herdr_read.rs.)

use moshpit_herdr::client::rig::{FakePane, Fate, Rig, Rule};
use moshpit_herdr::client::{Action, Config, Event, Output};

const SCREEN: &str = "dev@box:~/code/app$ cargo test\ntest result: ok\n__moshpit_rc=0\n";

/// Started, attached to the focused `w1:p1`, with `w1:p2` and these more
/// panes in its tab; every `pane read` answers [`SCREEN`] after `delay`.
fn started(more: &[&str], delay: u64) -> Rig {
    let mut rig = Rig::new(Config::default());
    for id in more {
        rig.host.panes.push(FakePane {
            id: (*id).to_owned(),
            tab: "w1:t1".to_owned(),
            workspace: "w1".to_owned(),
            cwd: "/home/dev/code/app".to_owned(),
            agent: None,
        });
    }
    rig.host.rules.push(Rule {
        pattern: "pane read".to_owned(),
        delay: Some(delay),
        fate: Fate::Output(SCREEN.to_owned()),
    });
    rig.start();
    rig.run(500);
    assert_eq!(rig.client.attached_pane(), Some("w1:p1"));
    rig
}

fn panes(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| (*p).to_owned()).collect()
}

/// The panes a `pane read` went out for, in order.
fn read(rig: &Rig) -> Vec<String> {
    rig.log
        .iter()
        .filter_map(|(_, output)| match output {
            Output::Exec { command, .. } if command.contains("pane read") => {
                ["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w1:p5"]
                    .into_iter()
                    .find(|id| command.contains(&format!("'{id}'")))
                    .map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

fn pictures(rig: &Rig) -> Vec<(String, Vec<String>)> {
    rig.events()
        .into_iter()
        .filter_map(|event| match event {
            Event::PaneRead { pane, rows } => Some((pane, rows)),
            _ => None,
        })
        .collect()
}

#[test]
fn every_pane_not_on_screen_is_read_once_two_at_a_time() {
    let mut rig = started(&["w1:p3", "w1:p4", "w1:p5"], 200);
    let all = panes(&["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w1:p5"]);
    rig.act(Action::ReadPanes(all.clone()));
    rig.run(10);
    assert_eq!(read(&rig).len(), 2, "two at a time: {:?}", read(&rig));
    // Asked again while they are out: nothing new.
    rig.act(Action::ReadPanes(all));
    rig.run(2_000);
    let mut asked = read(&rig);
    asked.sort();
    assert_eq!(
        asked,
        ["w1:p2", "w1:p3", "w1:p4", "w1:p5"],
        "the pane shown is never read"
    );
    let heard = pictures(&rig);
    assert_eq!(heard.len(), 4, "{heard:?}");
    for (_, rows) in &heard {
        assert_eq!(rows, &["dev@box:~/code/app$ cargo test", "test result: ok"]);
    }
    assert_eq!(rig.client.reads_pending(), 0);
}

#[test]
fn a_read_never_reaches_a_panes_terminal() {
    let mut rig = started(&["w1:p3"], 50);
    rig.act(Action::ReadPanes(panes(&["w1:p2", "w1:p3"])));
    rig.run(1_000);
    assert_eq!(pictures(&rig).len(), 2);
    // Its text is in no output but the exec and the overview's event: not
    // the frame channel's, not a pane's input.
    for (_, output) in &rig.log {
        if !matches!(
            output,
            Output::Exec { .. } | Output::Event(Event::PaneRead { .. })
        ) {
            assert!(!format!("{output:?}").contains("cargo test"), "{output:?}");
        }
    }
    assert!(
        rig.host.typed.values().all(|bytes| bytes.is_empty()),
        "{:?}",
        rig.host.typed
    );
}

#[test]
fn a_read_dropped_because_the_phone_went_to_that_pane() {
    let mut rig = started(&[], 1_000);
    rig.act(Action::ReadPanes(panes(&["w1:p2"])));
    rig.run(10);
    assert_eq!(read(&rig), ["w1:p2"]);
    // The person picks it before the answer: its live screen is on the
    // phone now, and the read must not stand in for it.
    rig.act(Action::SelectPane("w1:p2".to_owned()));
    rig.run(2_000);
    assert!(pictures(&rig).is_empty(), "{:?}", pictures(&rig));
}

#[test]
fn closing_the_overview_leaves_nothing_asked_or_waited_for() {
    let mut rig = started(&["w1:p3", "w1:p4"], 1_000);
    rig.act(Action::ReadPanes(panes(&["w1:p2", "w1:p3", "w1:p4"])));
    rig.run(10);
    assert_eq!(rig.client.reads_pending(), 3);
    rig.act(Action::StopReads);
    assert_eq!(rig.client.reads_pending(), 0, "nothing subscribed");
    rig.run(3_000);
    assert!(pictures(&rig).is_empty());
    assert_eq!(read(&rig).len(), 2, "the queued one never went out");
}
