//! The Swift monitor's suites (AgentActivityMonitorTests,
//! AgentStandingTests), on the machine.

use super::*;
use moshpit_seal::copy::InterruptionLevel;

const CONN: &str = "0F0E0D0C-0000-4000-8000-000000000002";
const T0: f64 = 1_790_000_000.0;

/// A scratch directory, gone when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("agents-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Bench {
    monitor: Monitor,
    standing: PushStandingFile,
    finishes: PushFinishesFile,
    _dir: Scratch,
}

impl Bench {
    fn new(settings: Settings, announced: &[(&str, f64)]) -> Self {
        let dir = Scratch::new();
        let standing = PushStandingFile::new(dir.0.join("standing-attention.json"));
        let finishes = PushFinishesFile::new(dir.0.join("rung-finishes.json"));
        let record = announced
            .iter()
            .map(|(pane, at)| (format!("{CONN}|{pane}"), *at))
            .collect();
        let mut monitor = Monitor::new(
            Tunables::default(),
            settings,
            Some(standing.clone()),
            Some(finishes.clone()),
            &record,
            T0,
        );
        monitor.track(CONN, "devbox");
        monitor.take_effects();
        Self { monitor, standing, finishes, _dir: dir }
    }

    fn quiet() -> Self {
        Self::new(Settings { notifications: false, ..Settings::default() }, &[])
    }

    fn poll(&mut self, pane: &str, state: &str, since: f64, now: f64) -> Vec<Effect> {
        self.monitor.hooks(CONN, vec![hook(pane, state, since)], now, TMUX);
        self.monitor.take_effects()
    }

    /// What the notification service records when a push opens.
    fn pushed(&self, pane: &str, sent_at: i64, now: f64) -> bool {
        self.standing.note_standing(
            CONN,
            Entry {
                pane: pane.into(),
                agent: Some("claude".into()),
                title: Some("Bash: npm test".into()),
                location: None,
                since: sent_at,
                recorded_at: Date::from_unix(now),
            },
            Date::from_unix(now),
        )
    }

    fn standing_panes(&self, now: f64) -> Vec<String> {
        self.standing.standing(CONN, Date::from_unix(now)).into_iter().map(|e| e.pane).collect()
    }
}

fn hook(pane: &str, state: &str, since: f64) -> Hook {
    Hook {
        pane: pane.into(),
        state: Some(state.into()),
        agent: Some("claude".into()),
        since: Some(since),
        command: Some("claude".into()),
        title: Some("Bash: npm test".into()),
        place: PaneWhere { command: "claude".into(), session: Some("work".into()), window: Some("1: api".into()) },
    }
}

fn posts(effects: &[Effect]) -> Vec<&Card> {
    effects.iter().filter_map(|e| if let Effect::Post(card) = e { Some(card) } else { None }).collect()
}

fn withdrawn(effects: &[Effect]) -> Vec<&str> {
    effects.iter().filter_map(|e| if let Effect::Withdraw(id) = e { Some(id.as_str()) } else { None }).collect()
}

fn islands(effects: &[Effect]) -> usize {
    effects.iter().filter(|e| matches!(e, Effect::Island(_))).count()
}

fn widgets(effects: &[Effect]) -> usize {
    effects.iter().filter(|e| matches!(e, Effect::Widget(_))).count()
}

// MARK: - Announcing an episode (AgentActivityMonitorTests)

#[test]
fn a_standing_prompt_is_announced_once_not_once_per_poll() {
    assert!(should_announce(AgentState::Attention, T0, None));
    assert!(!should_announce(AgentState::Attention, T0, Some(T0)));
}

#[test]
fn a_new_prompt_is_announced_even_on_the_same_pane_and_only_attention_announces() {
    assert!(should_announce(AgentState::Attention, T0 + 60.0, Some(T0)));
    for state in [AgentState::Working, AgentState::Done, AgentState::Idle] {
        assert!(!should_announce(state, T0, None));
    }
}

#[test]
fn the_record_drops_what_is_a_day_old_or_from_the_future() {
    let raw: BTreeMap<String, f64> = [
        ("a|%1".to_owned(), T0 - 3600.0),
        ("a|%2".to_owned(), T0 - 2.0 * 86_400.0),
        ("a|%3".to_owned(), T0 + 3600.0),
    ]
    .into_iter()
    .collect();
    let kept = decode_announced(&raw, T0, 86_400.0);
    assert_eq!(kept.keys().collect::<Vec<_>>(), ["a|%1"]);
}

#[test]
fn a_prompt_is_announced_after_the_grace_once_and_a_relaunch_does_not_announce_it_again() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%1", "attention", T0, T0 + 1.0);
    bench.monitor.sweep(None, T0 + 20.0);
    assert!(posts(&bench.monitor.take_effects()).is_empty(), "inside the grace");
    bench.monitor.sweep(None, T0 + 31.5);
    let effects = bench.monitor.take_effects();
    let card = posts(&effects)[0];
    assert_eq!(card.id, format!("moshpit.attention.{CONN}"));
    assert_eq!(card.text.title, "claude needs you");
    assert_eq!(card.text.body, "Bash: npm test — devbox · work");
    assert!(card.text.sound);
    assert_eq!(card.text.level, InterruptionLevel::TimeSensitive);
    let saved = effects.iter().find_map(|e| if let Effect::SaveAnnounced(m) = e { Some(m.clone()) } else { None });
    assert_eq!(saved.unwrap().get(&format!("{CONN}|%1")), Some(&T0));

    // The same prompt on every later poll, and after a relaunch: nothing.
    bench.poll("%1", "attention", T0, T0 + 33.0);
    bench.monitor.sweep(None, T0 + 70.0);
    assert!(posts(&bench.monitor.take_effects()).is_empty());
    let mut relaunched = Bench::new(Settings::default(), &[("%1", T0)]);
    relaunched.poll("%1", "attention", T0, T0 + 100.0);
    relaunched.monitor.sweep(None, T0 + 140.0);
    assert!(posts(&relaunched.monitor.take_effects()).is_empty());
}

#[test]
fn a_prompt_answered_inside_the_grace_is_never_announced() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%1", "attention", T0, T0 + 1.0);
    bench.poll("%1", "working", T0 + 10.0, T0 + 11.0);
    bench.monitor.sweep(None, T0 + 40.0);
    assert!(posts(&bench.monitor.take_effects()).is_empty());
}

// MARK: - Reclassifying (AgentActivityMonitorTests)

#[test]
fn an_idle_reminder_attention_is_a_parked_agent_and_a_dead_agent_heals() {
    assert_eq!(
        reclassify(AgentState::Attention, Some("Claude is waiting for your input"), Some("claude")),
        AgentState::Done
    );
    assert_eq!(reclassify(AgentState::Attention, Some("Bash: ls"), Some("claude")), AgentState::Attention);
    for state in [AgentState::Attention, AgentState::Working] {
        assert_eq!(reclassify(state, None, Some("zsh")), AgentState::Done);
    }
}

// MARK: - Turn endings (AgentActivityMonitorTests)

#[test]
fn turn_endings() {
    let g = 5.0;
    // A live working→done poll.
    assert!(is_turn_ending(AgentState::Done, AgentState::Done, Some(AgentState::Working), Some(T0), T0 + 2.0, g));
    // The attach redraw: heuristic working over a host that said done.
    assert!(!is_turn_ending(AgentState::Done, AgentState::Done, Some(AgentState::Done), Some(T0), T0 + 2.0, g));
    // The first poll after a launch or reconnect.
    assert!(!is_turn_ending(AgentState::Done, AgentState::Done, None, None, T0, g));
    // A finish that happened while not polling was the push's.
    assert!(!is_turn_ending(AgentState::Done, AgentState::Done, Some(AgentState::Working), Some(T0), T0 + 600.0, g));
    // A healed fossil is bookkeeping, not a finish.
    assert!(!is_turn_ending(AgentState::Attention, AgentState::Done, Some(AgentState::Working), Some(T0), T0 + 2.0, g));
}

#[test]
fn untracking_an_unknown_connection_and_cycling_with_no_agents_are_harmless() {
    let mut bench = Bench::quiet();
    bench.monitor.untrack("nobody", false, T0);
    bench.monitor.cycle_headline(T0);
    assert!(posts(&bench.monitor.take_effects()).is_empty());
}

#[test]
fn the_self_test_leaves_no_phantom_agent() {
    let mut bench = Bench::quiet();
    let mut test = hook("%9", "done", T0);
    test.agent = Some(SELF_TEST_AGENT.into());
    bench.monitor.hooks(CONN, vec![test], T0, TMUX);
    assert!(bench.monitor.state_since_by_pane(CONN).is_empty());
}

// MARK: - The waiting set (AgentStandingTests)

#[test]
fn a_pushed_prompt_answered_while_the_app_polls_leaves_the_set_and_the_next_question_rings() {
    let mut bench = Bench::quiet();
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    bench.poll("%1", "attention", T0, T0 + 32.0);
    assert_eq!(bench.standing_panes(T0 + 32.0), ["%1"]);
    bench.poll("%1", "working", T0 + 40.0, T0 + 41.0);
    assert!(bench.standing_panes(T0 + 41.0).is_empty());
    assert!(bench.pushed("%1", T0 as i64 + 240, T0 + 240.0), "the next question was no edge");
}

#[test]
fn answering_one_of_two_leaves_the_other_and_a_stale_poll_answers_nothing() {
    let mut bench = Bench::quiet();
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    assert!(!bench.pushed("%2", T0 as i64 + 35, T0 + 35.0));
    bench.monitor.hooks(
        CONN,
        vec![hook("%1", "working", T0 + 50.0), hook("%2", "attention", T0 + 4.0)],
        T0 + 51.0,
        TMUX,
    );
    assert_eq!(bench.standing_panes(T0 + 51.0), ["%2"]);
    // A reply that sat in the socket: older than the question.
    bench.poll("%2", "working", T0 - 120.0, T0 + 52.0);
    assert_eq!(bench.standing_panes(T0 + 52.0), ["%2"]);
}

#[test]
fn a_card_redrawn_from_a_pushed_entry_keeps_show_detail_off_and_says_where() {
    let pushed = Entry {
        pane: "%1".into(),
        agent: Some("claude".into()),
        title: Some("Bash: cat ~/.ssh/id_ed25519".into()),
        location: None,
        since: T0 as i64 + 31,
        recorded_at: Date::from_unix(T0),
    };
    let off = Settings { show_detail: false, ..Settings::default() };
    let hidden = digest(&pushed, 1, off, false, "devbox · work", "devbox");
    assert!(!hidden.body.contains("id_ed25519"));
    assert_eq!(hidden.body, "devbox · work");
    let shown = digest(&pushed, 2, Settings::default(), false, "devbox · work", "devbox");
    assert_eq!(shown.body, "Bash: cat ~/.ssh/id_ed25519 — devbox · work");
    assert_eq!(shown.title, "claude +1 needs you");
    let mut placed = pushed.clone();
    placed.location = Some("m1-pro · api".into());
    assert_eq!(digest(&placed, 1, off, false, "devbox · work", "devbox").body, "m1-pro · api");
}

// MARK: - Sending only what changed (B304)

#[test]
fn an_unchanged_state_is_not_sent_again_on_every_poll() {
    let mut bench = Bench::quiet();
    let mut all = Vec::new();
    for i in 0..10 {
        all.extend(bench.poll("%1", "working", T0, T0 + i as f64 * 2.0));
    }
    assert_eq!(islands(&all), 1, "ten polls of the same state");
    assert_eq!(widgets(&all), 1);
    let changed = bench.poll("%1", "attention", T0 + 60.0, T0 + 21.0);
    assert_eq!(islands(&changed), 1, "a change still goes out");
    assert_eq!(widgets(&changed), 1);
    let mut quiet = Vec::new();
    for i in 0..10 {
        quiet.extend(bench.poll("%1", "attention", T0 + 60.0, T0 + 22.0 + i as f64));
    }
    assert_eq!(islands(&quiet), 0);
}

#[test]
fn an_unchanged_state_is_sent_again_before_it_would_read_as_stale() {
    let state = IslandState { agents: vec![], working_count: 0, attention_count: 0, headline_id: None };
    let sent = (state.clone(), T0);
    assert!(!needs_sending(&state, Some(&sent), T0 + 10.0, 60.0));
    assert!(needs_sending(&state, Some(&sent), T0 + 60.0, 60.0));
    assert!(needs_sending(&state, None, T0, 60.0));
}

#[test]
fn the_island_ends_once_when_the_agents_are_gone_and_comes_back_with_them() {
    let mut bench = Bench::quiet();
    bench.poll("%1", "working", T0, T0);
    bench.monitor.untrack(CONN, false, T0 + 1.0);
    let ended = bench.monitor.take_effects();
    assert_eq!(ended.iter().filter(|e| matches!(e, Effect::EndIsland)).count(), 1);
    bench.monitor.cycle_headline(T0 + 2.0);
    assert!(!bench.monitor.take_effects().iter().any(|e| matches!(e, Effect::EndIsland)));
    bench.monitor.track(CONN, "devbox");
    let back = bench.poll("%1", "working", T0, T0 + 3.0);
    assert_eq!(islands(&back), 1, "the new island gets the state at once");
}

// MARK: - A long finish rings once

#[test]
fn a_long_finish_the_push_already_rang_is_a_silent_card_here() {
    let mut bench = Bench::new(Settings::default(), &[]);
    let ended = T0 + 600.0;
    assert!(bench.finishes.note_ring(CONN, "%1", ended as i64 + 1, Date::from_unix(ended)));
    bench.poll("%1", "working", T0, ended - 1.0);
    let effects = bench.poll("%1", "done", ended, ended + 1.0);
    let done = posts(&effects).into_iter().find(|c| c.id.starts_with("moshpit.done.")).unwrap();
    assert!(!done.text.sound, "one turn's finish rang twice");
    assert_eq!(done.text.level, InterruptionLevel::Passive);
    assert_eq!(done.text.title, "✓ claude finished · 10 min");
}

/// The companion pet counts every finished turn: one `Finished` per turn,
/// for its connection and pane, with the cards off too (`post_done` stops
/// early then), none while it works, and none for the same done again.
#[test]
fn every_finished_turn_is_one_finished_effect_with_the_cards_off_too() {
    let finished = |effects: &[Effect]| {
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Finished { conn, pane } if conn == CONN && pane == "%1"))
            .count()
    };
    for mut bench in [Bench::new(Settings::default(), &[]), Bench::quiet()] {
        // Polled while it works, as the hooks are, then done.
        assert_eq!(finished(&bench.poll("%1", "working", T0, T0 + 599.0)), 0);
        assert_eq!(finished(&bench.poll("%1", "done", T0 + 600.0, T0 + 601.0)), 1);
        assert_eq!(finished(&bench.poll("%1", "done", T0 + 600.0, T0 + 606.0)), 0, "the same done again");
        assert_eq!(finished(&bench.poll("%1", "working", T0 + 700.0, T0 + 799.0)), 0);
        assert_eq!(finished(&bench.poll("%1", "done", T0 + 800.0, T0 + 801.0)), 1, "the next turn");
    }
}

/// B442: with the sound off the record wasn't consulted, so a long finish
/// the push had already put on screen came here `Active` again — one turn,
/// the screen woken twice. The record decides whatever the sound switch.
#[test]
fn a_long_finish_the_push_already_showed_wakes_nothing_here_with_the_sound_off() {
    let mut bench = Bench::new(Settings { sound: false, ..Settings::default() }, &[]);
    let ended = T0 + 600.0;
    assert!(bench.finishes.note_ring(CONN, "%1", ended as i64 + 1, Date::from_unix(ended)));
    bench.poll("%1", "working", T0, ended - 1.0);
    let effects = bench.poll("%1", "done", ended, ended + 1.0);
    let done = posts(&effects).into_iter().find(|c| c.id.starts_with("moshpit.done.")).unwrap();
    assert!(!done.text.sound);
    assert_eq!(done.text.level, InterruptionLevel::Passive, "one turn woke the screen twice");

    let mut first = Bench::new(Settings { sound: false, ..Settings::default() }, &[]);
    first.poll("%1", "working", T0, ended - 1.0);
    let effects = first.poll("%1", "done", ended, ended + 1.0);
    let done = posts(&effects).into_iter().find(|c| c.id.starts_with("moshpit.done.")).unwrap();
    assert_eq!((done.text.sound, done.text.level), (false, InterruptionLevel::Active));
    assert!(!first.finishes.note_ring(CONN, "%1", ended as i64, Date::from_unix(ended + 2.0)), "not recorded");
}

#[test]
fn a_long_finish_nobody_rang_rings_here_and_the_push_after_it_doesnt() {
    let mut bench = Bench::new(Settings::default(), &[]);
    let ended = T0 + 600.0;
    bench.poll("%1", "working", T0, ended - 1.0);
    let effects = bench.poll("%1", "done", ended, ended + 1.0);
    let done = posts(&effects).into_iter().find(|c| c.id.starts_with("moshpit.done.")).unwrap();
    assert!(done.text.sound);
    assert_eq!(done.text.level, InterruptionLevel::Active);
    assert!(!bench.finishes.note_ring(CONN, "%1", ended as i64 + 1, Date::from_unix(ended + 2.0)));
}

#[test]
fn a_short_finish_is_silent_and_shows_no_duration() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%1", "working", T0, T0 + 19.0);
    let effects = bench.poll("%1", "done", T0 + 20.0, T0 + 21.0);
    let done = posts(&effects)[0];
    assert_eq!(done.text.title, "✓ claude finished");
    assert!(!done.text.sound);
    assert_eq!(done.text.level, InterruptionLevel::Passive);
}

// MARK: - Disconnecting

#[test]
fn disconnecting_leaves_a_waiting_card_and_answering_it_seen_later_takes_it_down() {
    let asked = T0;
    let mut bench = Bench::new(Settings::default(), &[("%1", asked)]);
    bench.standing.note_standing(
        CONN,
        Entry { pane: "%1".into(), agent: Some("claude".into()), title: None, location: Some("devbox".into()),
                since: asked as i64, recorded_at: Date::from_unix(asked + 60.0) },
        Date::from_unix(asked + 60.0),
    );
    bench.poll("%1", "attention", asked, asked + 61.0);
    let card = format!("moshpit.attention.{CONN}");
    bench.monitor.untrack(CONN, true, asked + 62.0);
    assert!(!withdrawn(&bench.monitor.take_effects()).contains(&card.as_str()));
    assert_eq!(bench.standing_panes(asked + 62.0), ["%1"]);

    bench.monitor.track(CONN, "devbox");
    bench.poll("%1", "attention", asked, asked + 120.0);
    let effects = bench.poll("%1", "working", asked + 300.0, asked + 121.0);
    assert!(withdrawn(&effects).contains(&card.as_str()));
    assert!(bench.standing_panes(asked + 121.0).is_empty());
}

#[test]
fn a_card_announced_before_a_relaunch_is_withdrawn_once_answered_even_past_the_sets_lifetime() {
    let asked = T0;
    let mut bench = Bench::new(Settings::default(), &[("%1", asked)]);
    bench.poll("%1", "attention", asked, asked + 3600.0);
    let effects = bench.poll("%1", "working", asked + 3600.0, asked + 3601.0);
    assert!(withdrawn(&effects).contains(&format!("moshpit.attention.{CONN}").as_str()));
}

#[test]
fn deleting_a_connection_takes_its_cards_with_it() {
    let asked = T0;
    let mut bench = Bench::new(Settings::default(), &[("%1", asked)]);
    bench.poll("%1", "attention", asked, asked + 1.0);
    bench.monitor.untrack(CONN, false, asked + 2.0);
    assert!(withdrawn(&bench.monitor.take_effects()).contains(&format!("moshpit.attention.{CONN}").as_str()));
}

#[test]
fn looking_at_a_prompt_acknowledges_it() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%1", "attention", T0, T0 + 1.0);
    bench.monitor.sweep(None, T0 + 31.0);
    bench.monitor.take_effects();
    assert_eq!(bench.standing_panes(T0 + 31.0), ["%1"]);
    let visible = Visible { conn: CONN.into(), pane: Some("%1".into()) };
    bench.monitor.sweep(Some(&visible), T0 + 33.0);
    assert!(withdrawn(&bench.monitor.take_effects()).contains(&format!("moshpit.attention.{CONN}").as_str()));
    assert!(bench.standing_panes(T0 + 33.0).is_empty());
}

// MARK: - The output heuristic

#[test]
fn agent_output_earns_a_row_after_the_promotion_delay_and_goes_done_when_quiet() {
    let mut bench = Bench::quiet();
    let at = |cmd: &str| PaneWhere { command: cmd.into(), session: Some("work".into()), window: None };
    bench.monitor.output(CONN, "%2", at("cargo"), T0);
    assert!(bench.monitor.state_since_by_pane(CONN).is_empty(), "a build is not an agent");
    bench.monitor.output(CONN, "%1", at("claude"), T0);
    bench.monitor.output(CONN, "%1", at("claude"), T0 + 1.0);
    assert_eq!(islands(&bench.monitor.take_effects()), 0, "not yet a row");
    bench.monitor.output(CONN, "%1", at("claude"), T0 + 2.5);
    assert_eq!(islands(&bench.monitor.take_effects()), 1);
    bench.monitor.sweep(None, T0 + 7.0);
    let effects = bench.monitor.take_effects();
    let Some(Effect::Island(state)) = effects.iter().find(|e| matches!(e, Effect::Island(_))) else {
        panic!("no island update: {effects:?}")
    };
    assert_eq!(state.agents[0].state, AgentState::Done);
    bench.monitor.sweep(None, T0 + 33.0);
    assert!(bench.monitor.take_effects().contains(&Effect::EndIsland));
}

#[test]
fn a_bell_is_attention_at_once_through_the_grace() {
    let mut bench = Bench::new(Settings::default(), &[]);
    let at = PaneWhere { command: "zsh".into(), session: None, window: None };
    bench.monitor.bell(CONN, "%3", at, T0);
    bench.monitor.sweep(None, T0 + 31.0);
    let effects = bench.monitor.take_effects();
    let card = posts(&effects)[0];
    assert_eq!(card.text.title, "zsh needs you");
    assert_eq!(card.text.body, "devbox");
}

#[test]
fn the_island_is_most_urgent_first_four_rows_and_a_cycled_headline() {
    let mut bench = Bench::quiet();
    let hooks: Vec<Hook> = (1..=5)
        .map(|i| hook(&format!("%{i}"), if i == 3 { "attention" } else { "working" }, T0 + i as f64))
        .collect();
    bench.monitor.hooks(CONN, hooks, T0 + 10.0, TMUX);
    let effects = bench.monitor.take_effects();
    let Some(Effect::Island(state)) = effects.iter().find(|e| matches!(e, Effect::Island(_))) else { panic!() };
    assert_eq!(state.agents.len(), 4);
    assert_eq!(state.agents[0].pane_id, "%3");
    assert_eq!(state.agents[1].pane_id, "%5", "newest working next");
    assert_eq!((state.working_count, state.attention_count), (4, 1));
    assert_eq!(state.headline_id.as_deref(), Some(format!("{CONN}:%3").as_str()));
    assert_eq!(state.agents[0].location, "devbox · work · 1: api");
    bench.monitor.cycle_headline(T0 + 11.0);
    let effects = bench.monitor.take_effects();
    let Some(Effect::Island(state)) = effects.iter().find(|e| matches!(e, Effect::Island(_))) else { panic!() };
    assert_eq!(state.headline_id.as_deref(), Some(format!("{CONN}:%5").as_str()));
}

// MARK: - What the cross-check found

/// Two graces running out before the same sweep announce in the order they
/// ran out — each has its own timer in the app — and the first is the edge.
#[test]
fn graces_due_at_one_sweep_announce_in_the_order_they_ran_out() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%2", "attention", T0 + 21.0, T0 + 21.0);
    bench.poll("%1", "attention", T0 + 25.0, T0 + 25.0);
    bench.monitor.sweep(None, T0 + 57.0);
    let effects = bench.monitor.take_effects();
    let cards = posts(&effects);
    assert_eq!(cards[0].pane_id, "%2", "the grace that ran out first");
    assert!(cards[0].text.sound);
    assert!(cards[1].text.title.ends_with("+1 needs you"));
    assert!(!cards[1].text.sound);
}

/// A prompt the notification service recorded, on a pane no poll has named
/// yet: its card still says which session, from the tree.
#[test]
fn a_card_for_a_pane_no_poll_named_says_where_from_the_tree() {
    let mut bench = Bench::quiet();
    bench.monitor.set_settings(Settings::default());
    let place = |command: &str| PaneWhere { command: command.into(), session: Some("work".into()), window: Some("1: api".into()) };
    bench.monitor.tree(CONN, [("%2".to_owned(), place("codex"))].into_iter().collect());
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    assert!(!bench.pushed("%2", T0 as i64 + 35, T0 + 35.0));
    let effects = bench.poll("%1", "working", T0 + 50.0, T0 + 51.0);
    let card = posts(&effects)[0];
    assert_eq!(card.pane_id, "%2");
    assert_eq!(card.text.body, "Bash: npm test — devbox · work");
}

// MARK: - A pane that is gone (r43)

fn unstamped(pane: &str) -> Hook {
    Hook {
        pane: pane.into(),
        command: Some("zsh".into()),
        place: PaneWhere { command: "zsh".into(), session: Some("work".into()), window: Some("1: api".into()) },
        ..Hook::default()
    }
}

fn tree_of(panes: &[&str]) -> BTreeMap<String, PaneWhere> {
    panes
        .iter()
        .map(|pane| {
            let place = PaneWhere { command: "claude".into(), session: Some("work".into()), window: Some("1: api".into()) };
            ((*pane).to_owned(), place)
        })
        .collect()
}

/// A poll lists every pane of the host's server, stamped or not. A pane it
/// no longer names, and the tree no longer holds, was closed (its window
/// killed, its session ended): its question can't be answered any more and
/// no signal will ever come from it. It used to stand as "needs you" on the
/// island for as long as the connection was watched, its card up and in the
/// waiting set, which kept the next question elsewhere from ringing.
#[test]
fn a_pane_closed_while_its_agent_waits_leaves_the_island_and_takes_its_card() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.tree(CONN, tree_of(&["%1", "%2"]));
    bench.monitor.hooks(CONN, vec![hook("%1", "attention", T0), unstamped("%2")], T0 + 1.0, TMUX);
    bench.monitor.sweep(None, T0 + 31.0);
    assert_eq!(posts(&bench.monitor.take_effects()).len(), 1, "announced");
    assert_eq!(bench.standing_panes(T0 + 31.0), ["%1"]);

    // %1's window is closed.
    bench.monitor.tree(CONN, tree_of(&["%2"]));
    bench.monitor.hooks(CONN, vec![unstamped("%2")], T0 + 40.0, TMUX);
    let mut effects = bench.monitor.take_effects();
    for t in [42.0, 44.0, 46.0, 48.0, 100.0] {
        bench.monitor.hooks(CONN, vec![unstamped("%2")], T0 + t, TMUX);
        bench.monitor.sweep(None, T0 + t);
        effects.extend(bench.monitor.take_effects());
    }
    assert!(withdrawn(&effects).contains(&format!("moshpit.attention.{CONN}").as_str()), "{effects:?}");
    assert!(bench.standing_panes(T0 + 100.0).is_empty());
    assert!(bench.monitor.state_since_by_pane(CONN).is_empty(), "still listed");
    assert!(effects.contains(&Effect::EndIsland));
}

/// Closed inside the grace: never announced.
#[test]
fn a_pane_closed_inside_the_grace_is_never_announced() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.tree(CONN, tree_of(&["%1", "%2"]));
    bench.monitor.hooks(CONN, vec![hook("%1", "attention", T0), unstamped("%2")], T0 + 1.0, TMUX);
    bench.monitor.tree(CONN, tree_of(&["%2"]));
    let mut effects = Vec::new();
    for t in [10.0, 12.0, 14.0, 16.0, 18.0, 31.0, 33.0] {
        bench.monitor.hooks(CONN, vec![unstamped("%2")], T0 + t, TMUX);
        bench.monitor.sweep(None, T0 + t);
        effects.extend(bench.monitor.take_effects());
    }
    assert!(posts(&effects).is_empty(), "{effects:?}");
    assert!(bench.standing_panes(T0 + 33.0).is_empty());
}

/// What a pane closing must not be confused with: one the poll still names
/// (another session's, which the tree may not show), and one the tree still
/// holds but this poll doesn't (a poll older than the tree).
#[test]
fn a_pane_the_poll_or_the_tree_still_holds_is_not_gone() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.tree(CONN, tree_of(&["%1", "%2"]));
    bench.monitor.hooks(CONN, vec![hook("%1", "attention", T0), hook("%2", "attention", T0)], T0 + 1.0, TMUX);
    // %1 named by the poll, not in the tree; %2 in the tree, not named.
    bench.monitor.tree(CONN, tree_of(&["%2"]));
    bench.monitor.hooks(CONN, vec![hook("%1", "attention", T0)], T0 + 3.0, TMUX);
    bench.monitor.sweep(None, T0 + 31.0);
    let effects = bench.monitor.take_effects();
    assert_eq!(bench.monitor.state_since_by_pane(CONN).len(), 2);
    assert!(!posts(&effects).is_empty());
    assert_eq!(bench.standing_panes(T0 + 31.0).len(), 2);
}

// MARK: - A clock set back (r43)

/// Every time the machine keeps is the wall clock the platform passes. Set
/// back (by hand, or a network time correction) while a question stood its
/// grace, it held the announcement back by as much — an hour back, an hour
/// of silence for a waiting agent; the app's grace was a timer, which a
/// clock change doesn't touch. And the island, sent again only once its
/// last send lay a minute in the past, went unsent for as long, reading
/// "paused" after two minutes.
#[test]
fn a_clock_set_back_neither_holds_a_question_back_nor_lets_the_island_go_stale() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%1", "attention", T0, T0 + 1.0);
    let back = T0 - 3600.0;
    let mut effects = Vec::new();
    for step in 0..=35 {
        let now = back + 2.0 * step as f64;
        effects.extend(bench.poll("%1", "attention", T0, now));
        bench.monitor.sweep(None, now);
        effects.extend(bench.monitor.take_effects());
    }
    assert_eq!(posts(&effects).len(), 1, "announced within two graces of real time");
    assert!(islands(&effects) >= 1, "the island was sent again");
}

/// The card the app left up for a question — across a relaunch, or a
/// disconnect that kept it — whose pane closed while the app was away: no
/// poll will ever name the pane again, so nothing took it down.
#[test]
fn a_card_left_up_for_a_pane_that_closed_while_the_app_was_away_is_withdrawn() {
    let asked = T0;
    let mut bench = Bench::new(Settings::default(), &[("%1", asked)]);
    bench.standing.note_standing(
        CONN,
        Entry { pane: "%1".into(), agent: Some("claude".into()), title: None, location: Some("devbox".into()),
                since: asked as i64, recorded_at: Date::from_unix(asked + 60.0) },
        Date::from_unix(asked + 60.0),
    );
    bench.monitor.tree(CONN, tree_of(&["%2"]));
    bench.monitor.hooks(CONN, vec![unstamped("%2")], asked + 120.0, TMUX);
    let effects = bench.monitor.take_effects();
    assert!(withdrawn(&effects).contains(&format!("moshpit.attention.{CONN}").as_str()), "{effects:?}");
    assert!(bench.standing_panes(asked + 120.0).is_empty());
}

// MARK: - A question with no `since` (herdr)

/// herdr reports no transition time: the machine dates an episode the
/// first time it sees it.
fn herdr(pane: &str, state: &str) -> Hook {
    Hook { since: None, ..hook(pane, state, 0.0) }
}

/// A reconnect or a relaunch forgets the panes; herdr's stamps carry no
/// `since` to say the question is the one already announced. It used to be
/// dated anew and announced again — once per reconnect, for as long as the
/// agent kept waiting. A question the record has, on a pane not seen to
/// leave attention since, is that one: no announcement, no sound, and its
/// card is ours to take down once it is answered.
#[test]
fn a_herdr_question_still_standing_after_a_reconnect_or_a_relaunch_is_not_announced_again() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.hooks(CONN, vec![herdr("%1", "attention")], T0 + 1.0, HERDR_IDLE);
    bench.monitor.sweep(None, T0 + 31.0);
    assert_eq!(posts(&bench.monitor.take_effects()).len(), 1, "announced");

    bench.monitor.untrack(CONN, true, T0 + 40.0);
    bench.monitor.track(CONN, "devbox");
    bench.monitor.hooks(CONN, vec![herdr("%1", "attention")], T0 + 50.0, HERDR_IDLE);
    bench.monitor.sweep(None, T0 + 90.0);
    assert!(posts(&bench.monitor.take_effects()).is_empty(), "announced again after a reconnect");

    let mut relaunched = Bench::new(Settings::default(), &[("%1", T0 + 1.0)]);
    relaunched.monitor.hooks(CONN, vec![herdr("%1", "attention")], T0 + 100.0, HERDR_IDLE);
    relaunched.monitor.sweep(None, T0 + 140.0);
    assert!(posts(&relaunched.monitor.take_effects()).is_empty(), "announced again after a relaunch");

    // Answered: the card comes down; the next question is new, and rings.
    bench.monitor.hooks(CONN, vec![herdr("%1", "working")], T0 + 100.0, HERDR_IDLE);
    assert!(withdrawn(&bench.monitor.take_effects()).contains(&format!("moshpit.attention.{CONN}").as_str()));
    bench.monitor.hooks(CONN, vec![herdr("%1", "attention")], T0 + 110.0, HERDR_IDLE);
    bench.monitor.sweep(None, T0 + 141.0);
    let effects = bench.monitor.take_effects();
    let cards = posts(&effects);
    assert_eq!(cards.len(), 1, "a new question");
    assert!(cards[0].text.sound);
}

/// Seen to leave attention, a herdr pane's next question is a new one,
/// however it comes back — a reconnect included.
#[test]
fn a_herdr_question_after_the_pane_left_attention_is_announced_across_a_reconnect() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.hooks(CONN, vec![herdr("%1", "attention")], T0 + 1.0, HERDR_IDLE);
    bench.monitor.sweep(None, T0 + 31.0);
    bench.monitor.hooks(CONN, vec![herdr("%1", "working")], T0 + 40.0, HERDR_IDLE);
    bench.monitor.untrack(CONN, true, T0 + 45.0);
    bench.monitor.take_effects();
    bench.monitor.track(CONN, "devbox");
    bench.monitor.hooks(CONN, vec![herdr("%1", "attention")], T0 + 50.0, HERDR_IDLE);
    bench.monitor.sweep(None, T0 + 81.0);
    assert_eq!(posts(&bench.monitor.take_effects()).len(), 1);
}

// MARK: - Stamps held for their poll's interval (B346)

/// tmux's stamps: the monitor's sweep polls them every 2 s.
const TMUX: f64 = crate::harness::POLL_INTERVAL;
/// herdr's poll once nothing has changed, and while its push channel is up
/// (`crates/herdr` control.rs).
const HERDR_IDLE: f64 = 8.0;
const HERDR_PUSHED: f64 = 45.0;

/// One agent working from `T0` to `T0 + 200`, then done. Stamps every
/// `cadence(t)` seconds, dated by the host when `stamped` (tmux) and not
/// when it isn't (herdr); the sweep every 2 s. What the island showed at
/// each change (seconds after `T0`, state, started), the cards posted, and
/// when the stamp saying done was read.
fn cadence(cadence: impl Fn(f64) -> f64, stamped: bool) -> (Vec<(f64, AgentState, f64)>, Vec<Card>, f64) {
    let mut bench = Bench::new(Settings::default(), &[]);
    let end = 200.0;
    let (mut shown, mut cards) = (Vec::new(), Vec::new());
    let (mut next, mut t, mut read_done) = (0.0, 0.0, f64::NAN);
    while t <= 260.0 {
        if t >= next {
            let working = t < end;
            let mut stamp = hook("%1", if working { "working" } else { "done" }, T0 + if working { 0.0 } else { end });
            if !stamped {
                stamp.since = None;
            }
            if !working && read_done.is_nan() {
                read_done = t;
            }
            bench.monitor.hooks(CONN, vec![stamp], T0 + t, cadence(t));
            next = t + cadence(t);
        }
        bench.monitor.sweep(None, T0 + t);
        for effect in bench.monitor.take_effects() {
            match effect {
                Effect::Island(state) => {
                    if let Some(row) = state.agents.first().map(|a| (t, a.state, a.started_at - T0))
                        && shown.last().map(|(_, s, at)| (*s, *at)) != Some((row.1, row.2))
                    {
                        shown.push(row);
                    }
                }
                Effect::Post(card) => cards.push(card),
                _ => {}
            }
        }
        t += 2.0;
    }
    (shown, cards, read_done)
}

/// The turn's end: one "finished" card for the pane.
fn finished(cards: &[Card]) -> Vec<&str> {
    cards
        .iter()
        .filter(|c| c.id.starts_with("moshpit.done.") && c.text.title.contains("finished"))
        .map(|c| c.text.title.as_str())
        .collect()
}

/// Working all along, then done at the stamp that says so; the time never
/// restarts while it works.
fn worked_then_finished(shown: &[(f64, AgentState, f64)], done_at: f64) {
    let working: Vec<_> = shown.iter().filter(|(t, _, _)| *t < done_at).collect();
    assert!(
        working.iter().all(|(_, s, started)| *s == AgentState::Working && *started == 0.0),
        "shown as something else while it worked: {shown:?}"
    );
    assert!(
        shown.iter().any(|(t, s, _)| *t == done_at && *s == AgentState::Done),
        "{shown:?}"
    );
}

/// tmux's hold is the 5 s it always was; herdr's grow with its poll.
#[test]
fn a_stamp_holds_for_its_interval_and_a_margin_that_grows_with_it() {
    let margin = Tunables::default().stamp_margin;
    assert_eq!(stamp_hold(TMUX, margin), 5.0, "tmux, as before");
    assert_eq!(stamp_hold(HERDR_IDLE, margin), 12.0);
    assert_eq!(stamp_hold(HERDR_PUSHED, margin), 67.5);
    assert_eq!(stamp_hold(0.0, margin), margin);
    assert_eq!(stamp_hold(-3.0, margin), margin, "no interval below none");
    assert_eq!(stamp_hold(f64::NAN, margin), margin);
}

#[test]
fn tmux_every_two_seconds_stays_as_it_was() {
    let (shown, cards, read_done) = cadence(|_| TMUX, true);
    worked_then_finished(&shown, read_done);
    assert_eq!(finished(&cards), ["✓ claude finished · 3 min"]);
}

/// herdr reads every 8 s once nothing changes, and its stamps have no time
/// of their own: the 5 s grace lapsed between two polls, so the agent's time
/// restarted at every poll, and its turn never ended in a card.
#[test]
fn herdr_every_eight_seconds_keeps_its_agent_its_time_and_its_finish() {
    let (shown, cards, read_done) = cadence(|_| HERDR_IDLE, false);
    worked_then_finished(&shown, read_done);
    assert_eq!(finished(&cards), ["✓ claude finished · 3 min"]);
}

/// With its push channel up herdr reads every 45 s (push says when
/// something changes): the agent was shown done between reads, working
/// again at the next, and never finished.
#[test]
fn herdr_with_push_every_45_seconds_keeps_its_agent_and_its_finish() {
    let (shown, cards, read_done) = cadence(|_| HERDR_PUSHED, false);
    // The end is seen at the first read past 200 s, up to 45 s later.
    assert!(read_done > 200.0);
    worked_then_finished(&shown, read_done);
    assert_eq!(finished(&cards).len(), 1, "{:?}", finished(&cards));
}

/// Push comes up mid-turn and goes away again: each stamp carries the
/// interval of the poll that read it, and the one after is judged by what
/// that stamp promised.
#[test]
fn push_coming_and_going_mid_turn_changes_the_hold_with_the_next_stamp() {
    let pushed = |t: f64| if (40.0..160.0).contains(&t) { HERDR_PUSHED } else { HERDR_IDLE };
    let (shown, cards, read_done) = cadence(pushed, false);
    // Read every 8 s, then every 45 s from 40 s (at 86, 132 and 178 s),
    // every 8 s again once push has gone; the end read at 202 s.
    worked_then_finished(&shown, read_done);
    assert_eq!(finished(&cards), ["✓ claude finished · 3 min"]);
}

/// Ownership regained by a stamp with no time of its own, in the state the
/// pane is in, continues the episode.
#[test]
fn regaining_a_pane_in_the_same_state_keeps_its_time() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.hooks(CONN, vec![herdr("%1", "working")], T0, TMUX);
    // No stamp within the hold (a reconnect): the heuristic has it.
    bench.monitor.sweep(None, T0 + 8.0);
    bench.monitor.take_effects();
    bench.monitor.hooks(CONN, vec![herdr("%1", "working")], T0 + 10.0, TMUX);
    assert_eq!(bench.monitor.state_since_by_pane(CONN).get("%1"), Some(&T0));
    // A stamp that dates the episode still dates it.
    bench.monitor.sweep(None, T0 + 18.0);
    bench.monitor.hooks(CONN, vec![hook("%1", "working", T0 + 15.0)], T0 + 20.0, TMUX);
    assert_eq!(bench.monitor.state_since_by_pane(CONN).get("%1"), Some(&(T0 + 15.0)));
}

// MARK: - The record read sealed (B348)

/// The record a save wrote, if any.
fn saved_record(effects: &[Effect]) -> Option<BTreeMap<String, f64>> {
    effects
        .iter()
        .find_map(|e| if let Effect::SaveAnnounced(m) = e { Some(m.clone()) } else { None })
}

/// A launch before the first unlock read the record as empty; once it can
/// be believed, the stored episodes join this process's own (its own win),
/// what has expired stays out, and the merged record is saved.
#[test]
fn the_stored_record_adopted_late_keeps_its_episodes_and_this_processs() {
    let mut bench = Bench::new(Settings::default(), &[]);
    // This process announces %2's question meanwhile.
    bench.poll("%2", "attention", T0 + 5.0, T0 + 5.0);
    bench.monitor.sweep(None, T0 + 36.0);
    bench.monitor.take_effects();
    let stored = BTreeMap::from([
        (format!("{CONN}|%1"), T0 - 600.0),
        (format!("{CONN}|%2"), T0 - 900.0),
        (format!("{CONN}|%9"), T0 - 2.0 * 24.0 * 3600.0),
    ]);
    bench.monitor.adopt_announced(&stored, T0 + 40.0);
    let saved = saved_record(&bench.monitor.take_effects()).expect("the merged record is saved");
    assert_eq!(saved.get(&format!("{CONN}|%1")), Some(&(T0 - 600.0)), "the stored episode");
    assert_eq!(saved.get(&format!("{CONN}|%2")), Some(&(T0 + 5.0)), "this process's, being newer");
    assert!(!saved.contains_key(&format!("{CONN}|%9")), "a day old: dropped");
}

/// A question still standing when the device restarts, seen again before
/// the record could be read: it is the one announced before — not announced
/// again — and once answered its card comes down.
#[test]
fn a_question_announced_before_a_sealed_launch_is_not_announced_again() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.poll("%1", "attention", T0, T0 + 1.0);
    bench.monitor.adopt_announced(&BTreeMap::from([(format!("{CONN}|%1"), T0)]), T0 + 10.0);
    bench.monitor.take_effects();
    bench.monitor.sweep(None, T0 + 40.0);
    assert!(posts(&bench.monitor.take_effects()).is_empty(), "announced again");
    let answered = bench.poll("%1", "working", T0 + 50.0, T0 + 50.0);
    assert!(
        withdrawn(&answered).contains(&format!("moshpit.attention.{CONN}").as_str()),
        "its card stayed up"
    );
    // A newer question on that pane is new: it rings.
    bench.poll("%1", "attention", T0 + 60.0, T0 + 60.0);
    bench.monitor.sweep(None, T0 + 91.0);
    assert_eq!(posts(&bench.monitor.take_effects()).len(), 1);
}

// MARK: - The cards the notification center holds (B433)

fn waiting_card() -> String {
    format!("{ATTENTION_CARD}{CONN}")
}

fn saved(conns: &[&str]) -> BTreeSet<String> {
    conns.iter().map(|conn| (*conn).to_owned()).collect()
}

/// The app asleep: the push rang for %1's question, the person answered it
/// at the desk, and the turn's `done` push took the prompt out of the set.
/// The card it rang with was left up. The app comes forward, its first
/// poll says nothing waits there, and the card goes — not before that poll.
#[test]
fn a_pushed_card_whose_turn_ended_after_the_answer_is_withdrawn_once_the_poll_says_so() {
    let mut bench = Bench::new(Settings::default(), &[]);
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    bench.standing.clear(CONN, "%1", Date::from_unix(T0 + 200.0));
    bench.monitor.delivered(&[waiting_card()], Some(&saved(&[CONN])), T0 + 300.0);
    assert!(withdrawn(&bench.monitor.take_effects()).is_empty(), "judged before the first poll");
    let effects = bench.poll("%1", "done", T0 + 190.0, T0 + 302.0);
    assert!(withdrawn(&effects).contains(&waiting_card().as_str()), "{effects:?}");
}

/// The question answered at the desk, the app opened more than fifteen
/// minutes later, without connecting: the entry has expired, and the card
/// goes at once.
#[test]
fn a_pushed_card_found_past_the_sets_lifetime_is_withdrawn() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.untrack(CONN, true, T0);
    bench.monitor.take_effects();
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    let later = T0 + 31.0 + 16.0 * 60.0;
    bench.monitor.delivered(&[waiting_card()], Some(&saved(&[CONN])), later);
    assert!(withdrawn(&bench.monitor.take_effects()).contains(&waiting_card().as_str()));
}

/// While the prompt is in the set the card stays: the set's own rules take
/// it down when it is answered.
#[test]
fn a_pushed_card_whose_prompt_is_in_the_set_stays() {
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.untrack(CONN, true, T0);
    bench.monitor.take_effects();
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    bench.monitor.delivered(&[waiting_card()], Some(&saved(&[CONN])), T0 + 120.0);
    assert!(withdrawn(&bench.monitor.take_effects()).is_empty());
}

/// Past the set's lifetime, but the poll shows the question still
/// standing: the card is true and stays — and goes once it is answered,
/// as one this app posted would.
#[test]
fn a_pushed_card_for_a_question_still_waiting_stays_and_goes_with_its_answer() {
    let mut bench = Bench::new(Settings::default(), &[]);
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    let later = T0 + 31.0 + 16.0 * 60.0;
    bench.monitor.delivered(&[waiting_card()], Some(&saved(&[CONN])), later);
    let effects = bench.poll("%1", "attention", T0, later + 1.0);
    assert!(withdrawn(&effects).is_empty(), "{effects:?}");
    let effects = bench.poll("%1", "working", later + 5.0, later + 6.0);
    assert!(withdrawn(&effects).contains(&waiting_card().as_str()), "{effects:?}");
}

/// A connection no longer saved takes its waiting card, its entries and its
/// finish cards along; another connection's are left, and with the saved
/// connections unknown (before the first unlock) no finish card is judged.
#[test]
fn the_cards_of_a_connection_no_longer_saved_go_with_its_entries() {
    const OTHER: &str = "0F0E0D0C-0000-4000-8000-000000000009";
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.untrack(CONN, true, T0);
    bench.monitor.take_effects();
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    let done = format!("{DONE_CARD}{CONN}.%1");
    let other_done = format!("{DONE_CARD}{OTHER}.%2");
    let ids = [waiting_card(), done.clone(), other_done.clone()];
    bench.monitor.delivered(&ids, None, T0 + 60.0);
    assert!(withdrawn(&bench.monitor.take_effects()).is_empty());
    bench.monitor.delivered(&ids, Some(&saved(&[OTHER])), T0 + 60.0);
    let gone = bench.monitor.take_effects();
    assert_eq!(withdrawn(&gone), [waiting_card().as_str(), done.as_str()]);
    assert!(bench.standing_panes(T0 + 60.0).is_empty());
}

// MARK: - A delete takes the whole wait (B437)

/// The app posted %1's card; the notification service recorded %2 as
/// well. The delete used to clear %1 only and redraw the card from %2: a
/// fresh "needs you" for the connection just deleted. Now the whole set
/// goes and the card is withdrawn, nothing posted.
#[test]
fn deleting_a_connection_takes_the_whole_wait_and_posts_nothing() {
    let asked = T0;
    let mut bench = Bench::new(Settings::default(), &[("%1", asked)]);
    bench.poll("%1", "attention", asked, asked + 1.0);
    let _ = bench.pushed("%2", asked as i64 + 31, asked + 31.0);
    bench.monitor.untrack(CONN, false, asked + 40.0);
    let effects = bench.monitor.take_effects();
    assert!(posts(&effects).is_empty(), "{effects:?}");
    assert!(withdrawn(&effects).contains(&waiting_card().as_str()));
    assert!(bench.standing_panes(asked + 40.0).is_empty());
}

/// The push rang while the app slept and the connection is deleted before
/// it ever connected: its card goes and its entry with it.
#[test]
fn deleting_a_connection_the_app_never_posted_for_takes_the_pushed_card() {
    let mut bench = Bench::new(Settings::default(), &[]);
    assert!(bench.pushed("%1", T0 as i64 + 31, T0 + 31.0));
    bench.monitor.untrack(CONN, false, T0 + 40.0);
    let effects = bench.monitor.take_effects();
    assert!(withdrawn(&effects).contains(&waiting_card().as_str()), "{effects:?}");
    assert!(bench.standing_panes(T0 + 40.0).is_empty());
}

/// An entry with no agent named the card "agent needs you", in English
/// whatever the language; the notification service names such a push by
/// its host. It is the connection's name now, and the place without one.
#[test]
fn a_card_for_an_entry_with_no_agent_names_the_connection() {
    let entry = Entry {
        pane: "%1".into(),
        agent: None,
        title: None,
        location: Some("devbox · work".into()),
        since: T0 as i64,
        recorded_at: Date::from_unix(T0),
    };
    let zh = Settings { language: Language::ZhHans, ..Settings::default() };
    assert_eq!(digest(&entry, 1, Settings::default(), false, "devbox · work", "devbox").title, "devbox needs you");
    assert!(digest(&entry, 1, zh, false, "devbox · work", "devbox").title.starts_with("devbox"));
    assert_eq!(digest(&entry, 1, Settings::default(), false, "devbox · work", "").title, "devbox · work needs you");
}

// MARK: - The Live Activity switch (B436)

fn ends(effects: &[Effect]) -> usize {
    effects.iter().filter(|e| matches!(e, Effect::EndIsland)).count()
}

/// Switched off with agents listed, the island used to stay up, frozen,
/// until every agent left. It ends at once, once; nothing is sent to it
/// while off; switched on again, the next change asks for a new one.
#[test]
fn switching_the_live_activity_off_ends_the_island_at_once_and_on_brings_it_back() {
    let mut bench = Bench::quiet();
    assert_eq!(islands(&bench.poll("%1", "working", T0, T0)), 1);
    let off = Settings { live_activity: false, ..bench.monitor.settings() };
    bench.monitor.set_settings(off);
    assert_eq!(ends(&bench.monitor.take_effects()), 1);
    bench.monitor.set_settings(off);
    let effects = bench.poll("%1", "attention", T0 + 5.0, T0 + 6.0);
    assert_eq!((islands(&effects), ends(&effects)), (0, 0), "{effects:?}");
    bench.monitor.set_settings(Settings { live_activity: true, ..off });
    let effects = bench.poll("%1", "working", T0 + 10.0, T0 + 11.0);
    assert_eq!(islands(&effects), 1, "{effects:?}");
}

/// A launch with the switch off may find an island an earlier process
/// left up: the first state it would have sent ends it instead.
#[test]
fn with_the_live_activity_off_the_first_sync_ends_an_island_left_up() {
    let mut bench = Bench::new(Settings { live_activity: false, ..Settings::default() }, &[]);
    let effects = bench.poll("%1", "working", T0, T0);
    assert_eq!((islands(&effects), ends(&effects)), (0, 1), "{effects:?}");
}

// MARK: - A finish on screen (B449)

/// B449: a turn that finished while its pane was on screen posted a finish
/// card all the same — over the pane the person was watching, then left in
/// the list. No card now, and the finish counts as rung so the pushed copy
/// is quiet. Another pane's finish still posts.
#[test]
fn a_turn_that_finishes_on_screen_posts_no_card() {
    let mut bench = Bench::new(Settings::default(), &[]);
    let visible = Visible { conn: CONN.into(), pane: Some("%1".into()) };
    bench.monitor.sweep(Some(&visible), T0);
    bench.monitor.take_effects();
    bench.poll("%1", "working", T0, T0 + 595.0);
    let effects = bench.poll("%1", "done", T0 + 597.0, T0 + 597.0);
    assert!(posts(&effects).iter().all(|c| !c.id.starts_with(DONE_CARD)), "{effects:?}");
    assert!(!bench.finishes.note_ring(CONN, "%1", (T0 + 597.0) as i64, Date::from_unix(T0 + 598.0)), "not counted as rung");

    // Another pane's finish posts.
    let mut bench = Bench::new(Settings::default(), &[]);
    bench.monitor.sweep(Some(&visible), T0);
    bench.monitor.take_effects();
    bench.poll("%2", "working", T0, T0 + 595.0);
    let effects = bench.poll("%2", "done", T0 + 597.0, T0 + 597.0);
    assert!(posts(&effects).iter().any(|c| c.id.starts_with(DONE_CARD)), "{effects:?}");
}
