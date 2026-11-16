//! R5 (UI-DESIGN §6.1, §8): Home's model as pure functions, on the
//! acceptance fixture: a herdr host with 13 agents over 12 workspaces (1
//! waiting, 2 working, 1 done, 9 idle; tabs left `Tab 1`, some workspaces
//! `~`), a tmux host with 3 agents and other panes, an offline host and a
//! saved one. No screen: R6 draws these rows.

mod common;

use common::home_fixture::{AGENT, fixture, herdr_host, hook, tmux_host};
use moshpit_ui::home::model::{
    AgentState, CardBody, Connection, ConnectionFailure, Control, FailureReason, HostRow, Link,
    Mismatch, Mux, Summary, card_body, folder_name, host_rows, is_unnamed_window, needs_you,
    pane_place, pane_title, summary,
};

fn titles(rows: &[HostRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            HostRow::Agent(a) => format!("{:?} {} ({})", a.state, a.title, a.agent),
            HostRow::MoreWorking(n) => format!("{n} more working"),
            HostRow::Idle {
                count,
                titles,
                more,
                expanded,
            } => format!(
                "{count} idle: {} +{more}{}",
                titles.join(", "),
                if *expanded { " (open)" } else { "" }
            ),
            HostRow::Other(o) => format!("other {} · {}", o.program, o.title),
            HostRow::All { mux, count } => format!("all {mux:?} · {count}"),
        })
        .collect()
}

/// §8 R5: 13 agents and 12 workspaces make a card of at most five rows;
/// the waiting agent heads Home and is not in its host's card.
#[test]
fn the_acceptance_fixture() {
    let connections = fixture();
    let waiting = needs_you(&connections, AGENT);
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        (
            waiting[0].host.as_str(),
            waiting[0].row.title.as_str(),
            waiting[0].row.agent.as_str(),
            waiting[0].row.path.as_deref(),
            waiting[0].row.since,
        ),
        (
            "gpu-01",
            "trainer",
            "claude",
            Some("/home/dev/src/trainer"),
            Some(1_000.0)
        )
    );

    let gpu = host_rows(&connections[0], false, AGENT);
    assert_eq!(
        titles(&gpu),
        [
            "Working api (claude)",
            "Working web (claude)",
            "Done docs (claude)",
            "9 idle: api, proj-4, infra, proj-7 +5",
            "all Herdr · 12",
        ]
    );
    assert!(gpu.len() <= 5);

    let rows = [gpu, host_rows(&connections[1], false, AGENT)].concat();
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r, HostRow::Agent(a) if a.state == AgentState::NeedsYou || a.pane == "w3:p1")),
        "the waiting agent only in NEEDS YOU"
    );

    assert_eq!(
        titles(&host_rows(&connections[1], false, AGENT)),
        [
            "Working trainer (claude)",
            "Done cli (aider)",
            "1 idle: frontend +0",
            "other zsh · zsh · 4",
            "other nvim · api",
            "other tail · logs",
            "all Tmux · 1",
        ]
    );
    // The offline host and the saved one list nothing: their bodies are a
    // row of their own.
    assert!(host_rows(&connections[2], false, AGENT).is_empty());
    assert!(host_rows(&connections[3], false, AGENT).is_empty());
    assert_eq!(
        summary(&connections, AGENT),
        Summary::NeedsYou { waiting: 1 }
    );
}

/// §6.1's naming rules.
#[test]
fn a_pane_is_called_by_its_project() {
    let tmux = tmux_host();
    let control = tmux.control.as_ref().unwrap();
    let named = |id: &str| {
        let pane = control.panes.iter().find(|p| p.id == id).unwrap();
        pane_title(control, pane, AGENT)
    };
    // `2.1.322` is claude, in the folder it works in (rules 1 and 2).
    let trainer = named("%1");
    assert_eq!(
        (
            trainer.title.as_str(),
            trainer.agent.as_deref(),
            trainer.program.as_str()
        ),
        ("trainer", Some("claude"), "claude")
    );
    // A window the user named stays (rule 3), over the folder.
    assert_eq!(named("%2").title, "frontend");
    // Not an agent: no agent; the folder names it.
    let nvim = named("%5");
    assert_eq!((nvim.title.as_str(), nvim.agent), ("api", None));

    // herdr: the workspace's name, unless it is `~`; `Tab 1` is no name
    // (rule 4).
    let herdr = herdr_host();
    let control = herdr.control.as_ref().unwrap();
    let named = |id: &str| {
        let pane = control.panes.iter().find(|p| p.id == id).unwrap();
        pane_title(control, pane, AGENT).title
    };
    assert_eq!(named("w2:p1"), "web");
    assert_eq!(named("w4:p1"), "proj-4", "`~`: the folder");

    // No path yet (an older host, a failed read): "agent · window".
    let mut bare = control.clone();
    for p in &mut bare.panes {
        p.path = None;
    }
    let pane = bare.panes.iter().find(|p| p.id == "w4:p1").unwrap();
    assert_eq!(pane_title(&bare, pane, AGENT).title, "claude · Tab 1");
    let mut bare = tmux.control.clone().unwrap();
    for p in &mut bare.panes {
        p.path = None;
    }
    let pane = bare.panes.iter().find(|p| p.id == "%1").unwrap();
    assert_eq!(
        pane_title(&bare, pane, AGENT).title,
        "claude · 1",
        "tmux's own name: the index"
    );

    for (name, command) in [
        ("2.1.322", "2.1.322"),
        ("claude", "2.1.322"),
        ("node", "node"),
        ("zsh", "vim"),
        ("Tab 1", ""),
        ("Tab 12", "claude"),
        ("", "zsh"),
        ("1", "claude"),
        ("12", "zsh"),
    ] {
        assert!(is_unnamed_window(name, command), "{name:?} ({command})");
    }
    for (name, command) in [("frontend", "claude"), ("Tab one", ""), ("logs", "tail")] {
        assert!(!is_unnamed_window(name, command), "{name:?} ({command})");
    }
    assert_eq!(folder_name("/home/dev/src/api/").as_deref(), Some("api"));
    assert_eq!(folder_name("/Users/me"), None, "a home is no project");
    assert_eq!(folder_name("/root"), None);
    assert_eq!(folder_name("/"), None);
    assert_eq!(folder_name("/srv/my app").as_deref(), Some("my app"));

    // Where a pane is, for the island: the same names, none when nothing
    // names it (the island then says the host alone) — never a version or
    // a bare index.
    for (command, window, workspace, path, place) in [
        ("2.1.322", "2.1.322", None, Some("/Users/me"), None),
        ("claude", "1", None, None, None),
        ("claude", "moshpit", None, Some("/srv/other"), Some("moshpit")),
        ("claude", "claude", None, Some("/home/dev/src/x/"), Some("x")),
        ("claude", "Tab 1", Some("builder"), Some("/srv/other"), Some("builder")),
        ("claude", "Tab 1", Some("~"), Some("/srv/proj-4"), Some("proj-4")),
    ] {
        assert_eq!(
            pane_place(command, window, workspace, path).as_deref(),
            place,
            "{command} in {window:?}"
        );
    }
}

/// The summary says only what is so: never "nothing works" beside a
/// working agent (§6.1 decision 2, pain point 4).
#[test]
fn the_summary_says_only_what_is_so() {
    let mut connections = fixture();
    assert_eq!(
        summary(&connections, AGENT),
        Summary::NeedsYou { waiting: 1 }
    );
    // The waiting agent answered: it works now.
    let control = connections[0].control.as_mut().unwrap();
    control
        .hooks
        .iter_mut()
        .find(|h| h.pane == "w3:p1")
        .unwrap()
        .state = Some("working".into());
    assert_eq!(
        summary(&connections, AGENT),
        Summary::Agents {
            working: 4,
            done: 2,
            idle: 10,
        }
    );
    // Only the offline host left up? It isn't up: nothing at the top.
    let gone: Vec<Connection> = connections[2..].to_vec();
    assert_eq!(summary(&gone, AGENT), Summary::Nothing);
    // A live host without a multiplexer, or with no agents.
    let plain = vec![Connection {
        id: "c".into(),
        name: "plain".into(),
        link: Link::Live,
        ..Connection::default()
    }];
    assert_eq!(summary(&plain, AGENT), Summary::Online { connections: 1 });
    // A waiting agent on a host still attaching doesn't count yet: its card
    // spins, the summary reads the hosts that list agents.
    let control = connections[0].control.as_mut().unwrap();
    control
        .hooks
        .iter_mut()
        .find(|h| h.pane == "w3:p1")
        .unwrap()
        .state = Some("attention".into());
    connections[0].link = Link::Attaching;
    assert_eq!(
        summary(&connections, AGENT),
        Summary::Agents {
            working: 1,
            done: 1,
            idle: 1,
        }
    );
}

/// B552: a host still dialling or attaching is not online yet: the line
/// read "1 connection online" over its card spinning "Connecting…". A
/// connected host whose tmux attach stalled is (its card says why, and it
/// is connected); so is one with a herdr version skew.
#[test]
fn a_host_is_online_once_its_card_stops_spinning() {
    let host = |link: Link| Connection {
        id: "c".into(),
        name: "plain".into(),
        link,
        ..Connection::default()
    };
    for spinning in [Link::Connecting, Link::Reconnecting, Link::Attaching] {
        assert_eq!(
            summary(&[host(spinning)], AGENT),
            Summary::Nothing,
            "{spinning:?}"
        );
    }
    for up in [Link::Live, Link::Stalled] {
        assert_eq!(
            summary(&[host(up)], AGENT),
            Summary::Online { connections: 1 },
            "{up:?}"
        );
    }
    assert_eq!(
        summary(&[host(Link::Live), host(Link::Connecting)], AGENT),
        Summary::Online { connections: 1 }
    );
}

/// Past six working agents the rest fold into one row; the idle group opens
/// in place; at most three other panes.
#[test]
fn long_lists_fold() {
    let mut host = herdr_host();
    let control = host.control.as_mut().unwrap();
    for h in &mut control.hooks {
        if h.pane != "w3:p1" {
            h.state = Some("working".into());
            h.since = Some(5_000.0);
        }
    }
    let rows = host_rows(&host, false, AGENT);
    assert_eq!(
        rows.iter()
            .filter(|r| matches!(r, HostRow::Agent(_)))
            .count(),
        6
    );
    assert_eq!(rows[6], HostRow::MoreWorking(6));
    assert_eq!(rows.len(), 8, "{:?}", titles(&rows));

    let open = host_rows(&herdr_host(), true, AGENT);
    let at = open
        .iter()
        .position(|r| matches!(r, HostRow::Idle { expanded: true, .. }))
        .unwrap();
    assert_eq!(
        open[at + 1..]
            .iter()
            .filter(|r| matches!(r, HostRow::Agent(a) if a.state == AgentState::Idle))
            .count(),
        9
    );
    assert_eq!(
        host_rows(&tmux_host(), false, AGENT)
            .iter()
            .filter(|r| matches!(r, HostRow::Other(_)))
            .count(),
        3,
        "five other panes, three shown"
    );
}

/// NEEDS YOU: the one waiting longest first, a newcomer at the end, a wait
/// of unknown length after the known ones; hosts not up don't count.
#[test]
fn needs_you_is_an_inbox() {
    let mut connections = fixture();
    let tmux = connections[1].control.as_mut().unwrap();
    tmux.hooks
        .push(hook("%5", "attention", Some("codex"), Some(500.0)));
    tmux.hooks
        .push(hook("%6", "attention", Some("claude"), None));
    let order: Vec<(String, String)> = needs_you(&connections, AGENT)
        .into_iter()
        .map(|w| (w.host, w.row.pane))
        .collect();
    assert_eq!(
        order,
        [
            ("build-box".to_owned(), "%5".to_owned()),
            ("gpu-01".to_owned(), "w3:p1".to_owned()),
            ("build-box".to_owned(), "%6".to_owned()),
        ]
    );
    connections[0].link = Link::Stalled;
    assert!(
        needs_you(&connections, AGENT)
            .iter()
            .all(|w| w.host != "gpu-01"),
        "a stalled attach lists nothing"
    );
}

/// §6.1's card states: one row for each state that is not a live host's
/// list, and none for a saved host (it is a row in SAVED).
#[test]
fn each_state_has_its_card_body() {
    let with = |link: Link, control: Option<Control>| Connection {
        id: "c".into(),
        name: "box".into(),
        link,
        control,
        ..Connection::default()
    };
    let tmux = tmux_host().control;
    let herdr = herdr_host().control;
    assert_eq!(card_body(&with(Link::Saved, None), false, AGENT), None);
    assert_eq!(
        card_body(&with(Link::Connecting, None), false, AGENT),
        Some(CardBody::Connecting { mux: None })
    );
    assert_eq!(
        card_body(&with(Link::Reconnecting, tmux.clone()), false, AGENT),
        Some(CardBody::Connecting { mux: None }),
        "redialling after a drop spins in place"
    );
    assert_eq!(
        card_body(&with(Link::Attaching, herdr.clone()), false, AGENT),
        Some(CardBody::Connecting {
            mux: Some(Mux::Herdr)
        })
    );
    assert_eq!(
        card_body(&with(Link::Attaching, None), false, AGENT),
        Some(CardBody::Connecting {
            mux: Some(Mux::Tmux)
        }),
        "tmux attaching before its tree came"
    );
    assert_eq!(
        card_body(&with(Link::Stalled, tmux.clone()), false, AGENT),
        Some(CardBody::RetryAttach)
    );

    // Gone: ended, failed on the credentials, failed otherwise.
    let gone = |failure: Option<FailureReason>| Connection {
        lost_at: Some(1_000.0),
        failure: failure.map(|reason| ConnectionFailure {
            reason,
            detail: "the session's own words".into(),
            unreachable: None,
            leg: None,
        }),
        ..with(Link::Dead, None)
    };
    assert_eq!(
        card_body(&gone(None), false, AGENT),
        Some(CardBody::Reconnect {
            lost_at: Some(1_000.0),
            reason: None,
            unreachable: None,
            leg: None,
        })
    );
    assert_eq!(
        card_body(&gone(Some(FailureReason::Auth)), false, AGENT),
        Some(CardBody::EditConnection)
    );
    for reason in [
        FailureReason::HostKey,
        FailureReason::HostKeyChanged,
        FailureReason::Unreachable,
        FailureReason::Timeout,
        FailureReason::Proxy,
        FailureReason::Other,
    ] {
        assert_eq!(
            card_body(&gone(Some(reason)), false, AGENT),
            Some(CardBody::Reconnect {
                lost_at: Some(1_000.0),
                reason: Some(reason),
                unreachable: None,
                leg: None,
            }),
            "{reason:?}"
        );
    }

    // Live: no multiplexer, a herdr over a version skew, a host's rows.
    assert_eq!(
        card_body(&with(Link::Live, None), false, AGENT),
        Some(CardBody::OpenTerminal)
    );
    let skew = Mismatch {
        notice: "herdr 0.9 here, 0.8 on the server".into(),
        restart: true,
        restarting: false,
    };
    let mut skewed = herdr_host();
    skewed.control.as_mut().unwrap().mismatch = Some(skew.clone());
    assert_eq!(
        card_body(&skewed, false, AGENT),
        Some(CardBody::Mismatch(skew))
    );
    assert!(
        needs_you(std::slice::from_ref(&skewed), AGENT).is_empty(),
        "a skewed herdr's agents are not listed elsewhere either"
    );
    assert_eq!(
        summary(std::slice::from_ref(&skewed), AGENT),
        Summary::Online { connections: 1 }
    );
    let live = herdr_host();
    assert_eq!(
        card_body(&live, true, AGENT),
        Some(CardBody::Rows(host_rows(&live, true, AGENT)))
    );
}

/// B518: a card's "waiting" badge and the status line count what NEEDS
/// YOU lists, no more. A hook left for a pane that has gone, a stalled
/// attach and a herdr over a version skew each said "1 waiting" over an
/// inbox with nothing in it to tap.
#[test]
fn waiting_counts_what_needs_you_lists() {
    let agree = |connections: &[Connection], what: &str| {
        assert_eq!(
            connections.iter().map(Connection::waiting).sum::<usize>(),
            needs_you(connections, AGENT).len(),
            "{what}"
        );
    };
    let mut connections = fixture();
    agree(&connections, "the fixture");
    assert_eq!(connections[0].waiting(), 1);

    connections[1].control.as_mut().unwrap().hooks.push(hook(
        "%99",
        "attention",
        Some("claude"),
        None,
    ));
    agree(&connections, "a hook for a pane that has gone");

    connections[0].link = Link::Stalled;
    agree(&connections, "a stalled attach");
    assert_eq!(connections[0].waiting(), 0);

    let mut skewed = fixture();
    skewed[0].control.as_mut().unwrap().mismatch = Some(Mismatch {
        notice: "herdr 0.9 here, 0.8 on the server".into(),
        restart: true,
        restarting: false,
    });
    agree(&skewed, "a herdr over a version skew");
    assert_eq!(skewed[0].waiting(), 0);
}
