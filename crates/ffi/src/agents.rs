//! The agent-activity machine (`moshpit-agents`) for the app: what
//! `AgentActivityMonitor` drives, through UniFFI. The machine decides; the
//! app applies — ActivityKit, the notification center, WidgetCenter and the
//! App Group writes stay Swift.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use moshpit_agents::{self as agents, Effect, Monitor};
use moshpit_seal::copy::{InterruptionLevel, Language};
use moshpit_seal::diagnostics::DiagnosticsFile;
use moshpit_seal::standing::Entry;
use moshpit_seal::{Date, PushFinishesFile, PushStandingFile};

/// The user's switches, and the language the cards are written in (a BCP-47
/// tag; the app's own localization).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentSettingsValue {
    pub notifications: bool,
    pub live_activity: bool,
    pub show_detail: bool,
    pub sound: bool,
    pub language: String,
}

impl From<&AgentSettingsValue> for agents::Settings {
    fn from(value: &AgentSettingsValue) -> Self {
        Self {
            notifications: value.notifications,
            live_activity: value.live_activity,
            show_detail: value.show_detail,
            sound: value.sound,
            language: Language::from_tag(&value.language),
        }
    }
}

/// Where a pane lives, from the app's tree. `window_name`, `workspace`
/// (herdr's, none under tmux) and `path` are what Home names it from
/// (`moshpit_ui`'s `pane_place`), so the island says it as Home does.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentPaneWhereValue {
    pub command: String,
    pub session: Option<String>,
    pub window: Option<String>,
    #[uniffi(default = None)]
    pub window_name: Option<String>,
    #[uniffi(default = None)]
    pub workspace: Option<String>,
    #[uniffi(default = None)]
    pub path: Option<String>,
    /// The window's index and the pane's index in it: what tells two rows
    /// apart that would read the same.
    #[uniffi(default = None)]
    pub window_index: Option<i64>,
    #[uniffi(default = None)]
    pub pane_index: Option<i64>,
}

impl From<AgentPaneWhereValue> for agents::PaneWhere {
    fn from(value: AgentPaneWhereValue) -> Self {
        let name = moshpit_ui::home::model::pane_place(
            &value.command,
            value.window_name.as_deref().unwrap_or(""),
            value.workspace.as_deref(),
            value.path.as_deref(),
        );
        Self {
            command: value.command,
            session: value.session,
            window: value.window,
            name,
            window_index: value.window_index,
            pane_index: value.pane_index,
        }
    }
}

/// One pane of a hook poll; `since` in unix seconds.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentHookInValue {
    pub pane: String,
    pub state: Option<String>,
    pub agent: Option<String>,
    pub since: Option<f64>,
    pub command: Option<String>,
    pub title: Option<String>,
    pub place: AgentPaneWhereValue,
}

/// One row of the island and the widget; `started_at` in unix seconds,
/// `state` the raw word (`working`, `attention`, `done`, `idle`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentRowValue {
    pub id: String,
    pub connection_id: String,
    pub pane_id: String,
    pub command: String,
    pub location: String,
    /// "moshpit · mac": Home's name for the pane and the host.
    pub place: String,
    pub detail: Option<String>,
    pub state: String,
    pub started_at: f64,
    /// When its connection dropped (Unix seconds): shown as it was,
    /// greyed, its time stopped there.
    pub paused_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentIslandValue {
    pub agents: Vec<AgentRowValue>,
    pub working_count: u32,
    pub attention_count: u32,
    pub headline_id: Option<String>,
}

/// How a card may interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentCardLevel {
    Passive,
    Active,
    TimeSensitive,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentCardValue {
    pub id: String,
    pub title: String,
    /// Where: the connection's name and the session, the card's subtitle
    /// (B639).
    pub subtitle: String,
    pub body: String,
    pub sound: bool,
    pub level: AgentCardLevel,
    pub connection_id: String,
    pub pane_id: String,
}

/// What the app does, in order.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AgentEffectValue {
    Island { state: AgentIslandValue },
    EndIsland,
    Widget { state: AgentIslandValue },
    Post { card: AgentCardValue },
    Withdraw { id: String },
    SaveAnnounced { record: HashMap<String, f64> },
    RequestAuthorization,
    /// A line for the log.
    Note { text: String },
    /// An agent finished a turn (the companion pet counts it).
    Finished { connection_id: String, pane_id: String },
}

fn island(state: agents::IslandState) -> AgentIslandValue {
    AgentIslandValue {
        agents: state
            .agents
            .into_iter()
            .map(|a| AgentRowValue {
                id: a.id,
                connection_id: a.connection_id,
                pane_id: a.pane_id,
                command: a.command,
                location: a.location,
                place: a.place,
                detail: a.detail,
                state: a.state.raw().to_owned(),
                started_at: a.started_at,
                paused_at: a.paused_at,
            })
            .collect(),
        working_count: state.working_count as u32,
        attention_count: state.attention_count as u32,
        headline_id: state.headline_id,
    }
}

fn effect(effect: Effect) -> AgentEffectValue {
    match effect {
        Effect::Island(state) => AgentEffectValue::Island { state: island(state) },
        Effect::EndIsland => AgentEffectValue::EndIsland,
        Effect::Widget(state) => AgentEffectValue::Widget { state: island(state) },
        Effect::Post(card) => AgentEffectValue::Post {
            card: AgentCardValue {
                id: card.id,
                title: card.text.title,
                subtitle: card.text.subtitle,
                body: card.text.body,
                sound: card.text.sound,
                level: match card.text.level {
                    InterruptionLevel::Passive => AgentCardLevel::Passive,
                    InterruptionLevel::Active => AgentCardLevel::Active,
                    InterruptionLevel::TimeSensitive => AgentCardLevel::TimeSensitive,
                },
                connection_id: card.connection_id,
                pane_id: card.pane_id,
            },
        },
        Effect::Withdraw(id) => AgentEffectValue::Withdraw { id },
        Effect::SaveAnnounced(record) => AgentEffectValue::SaveAnnounced { record: record.into_iter().collect() },
        Effect::RequestAuthorization => AgentEffectValue::RequestAuthorization,
        Effect::Note(text) => AgentEffectValue::Note { text },
        Effect::Finished { conn, pane } => AgentEffectValue::Finished { connection_id: conn, pane_id: pane },
    }
}

/// The machine. Every call says what time it is and answers with what to
/// do.
#[derive(uniffi::Object)]
pub struct AgentMachine {
    monitor: Mutex<Monitor>,
}

impl AgentMachine {
    /// When each of `conn`'s panes entered the state it is in (Home's
    /// record reads it, `home`).
    pub(crate) fn since(&self, conn: &str) -> BTreeMap<String, f64> {
        let monitor = self.monitor.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        monitor.state_since_by_pane(conn)
    }

    fn drive(&self, step: impl FnOnce(&mut Monitor)) -> Vec<AgentEffectValue> {
        let mut monitor = self.monitor.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        step(&mut monitor);
        monitor.take_effects().into_iter().map(effect).collect()
    }
}

#[uniffi::export]
impl AgentMachine {
    /// `standing_path` and `finishes_path` are the files the notification
    /// service shares (none without the App Group); `announced` the record
    /// as stored; `announce_grace` how long a question stands before it is
    /// announced (30 s; a device harness shortens it).
    #[uniffi::constructor]
    pub fn new(
        settings: AgentSettingsValue,
        standing_path: Option<String>,
        finishes_path: Option<String>,
        announced: HashMap<String, f64>,
        announce_grace: f64,
        now: f64,
    ) -> Arc<Self> {
        let announced: BTreeMap<String, f64> = announced.into_iter().collect();
        Arc::new(Self {
            monitor: Mutex::new(Monitor::new(
                agents::Tunables { announce_grace, ..agents::Tunables::default() },
                (&settings).into(),
                standing_path.map(PushStandingFile::new),
                finishes_path.map(PushFinishesFile::new),
                &announced,
                now,
            )),
        })
    }

    /// The switches as they are now, and what changing them does: the
    /// island ended when the Live Activity is switched off (B436).
    pub fn set_settings(&self, settings: AgentSettingsValue) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.set_settings((&settings).into()))
    }

    pub fn is_tracking(&self) -> bool {
        self.monitor.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_tracking()
    }

    pub fn track(&self, conn: String, label: String) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.track(&conn, &label))
    }

    pub fn untrack(&self, conn: String, keep_waiting_cards: bool, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.untrack(&conn, keep_waiting_cards, now))
    }

    /// The connection's tree: where each of its panes lives.
    pub fn tree(&self, conn: String, places: HashMap<String, AgentPaneWhereValue>) {
        let places = places.into_iter().map(|(pane, place)| (pane, place.into())).collect();
        self.drive(|monitor| monitor.tree(&conn, places));
    }

    /// One poll of `conn`'s stamps. `interval`: how often its stamps
    /// arrive, in seconds — tmux's, the sweep that polls them; herdr's,
    /// [`herdr_stamp_interval`]. A stamp holds that long and a margin
    /// (`moshpit_agents::stamp_hold`, B346).
    pub fn hooks(
        &self,
        conn: String,
        hooks: Vec<AgentHookInValue>,
        now: f64,
        interval: f64,
    ) -> Vec<AgentEffectValue> {
        let hooks = hooks
            .into_iter()
            .map(|h| agents::Hook {
                pane: h.pane,
                state: h.state,
                agent: h.agent,
                since: h.since,
                command: h.command,
                title: h.title,
                place: h.place.into(),
            })
            .collect();
        self.drive(|monitor| monitor.hooks(&conn, hooks, now, interval))
    }

    pub fn output(&self, conn: String, pane: String, place: AgentPaneWhereValue, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.output(&conn, &pane, place.into(), now))
    }

    pub fn bell(&self, conn: String, pane: String, place: AgentPaneWhereValue, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.bell(&conn, &pane, place.into(), now))
    }

    /// `conn`'s link dropped (`up` false) or came back: its agents pause,
    /// and resume.
    pub fn link(&self, conn: String, up: bool, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.link(&conn, up, now))
    }

    /// The person typed into `pane` from the phone: a bell's question is
    /// answered (B663).
    pub fn input(&self, conn: String, pane: String, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.input(&conn, &pane, now))
    }

    /// The system would not start the island the last effect asked for: it
    /// is asked for again shortly.
    pub fn island_not_shown(&self, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.island_not_shown(now))
    }

    /// `visible_conn`/`visible_pane`: the pane on screen (no pane: every
    /// pane of that connection).
    pub fn sweep(&self, visible_conn: Option<String>, visible_pane: Option<String>, now: f64) -> Vec<AgentEffectValue> {
        let visible = visible_conn.map(|conn| agents::Visible { conn, pane: visible_pane });
        self.drive(|monitor| monitor.sweep(visible.as_ref(), now))
    }

    /// The announced record as stored, once it can be believed: it joins
    /// the one this process kept, then is saved (B348).
    pub fn adopt_announced(&self, announced: HashMap<String, f64>, now: f64) -> Vec<AgentEffectValue> {
        let announced: BTreeMap<String, f64> = announced.into_iter().collect();
        self.drive(|monitor| monitor.adopt_announced(&announced, now))
    }

    pub fn cycle_headline(&self, now: f64) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.cycle_headline(now))
    }

    /// The app was suspended and is back (B822): a question its stamps
    /// raised waits for its connection's next poll before it is announced.
    pub fn resumed(&self) -> Vec<AgentEffectValue> {
        self.drive(|monitor| monitor.resumed())
    }

    /// The agent cards the notification center holds, by identifier, and
    /// the saved connections' ids (none while they can't be told): a card
    /// nothing stands behind any more is withdrawn (B433).
    pub fn delivered(&self, ids: Vec<String>, saved: Option<Vec<String>>, now: f64) -> Vec<AgentEffectValue> {
        let saved: Option<std::collections::BTreeSet<String>> = saved.map(|ids| ids.into_iter().collect());
        self.drive(|monitor| monitor.delivered(&ids, saved.as_ref(), now))
    }

    pub fn state_since_by_pane(&self, conn: String) -> HashMap<String, f64> {
        let monitor = self.monitor.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        monitor.state_since_by_pane(&conn).into_iter().collect()
    }
}

/// One prompt in the waiting set the notification service and the machine
/// share (`moshpit_seal::standing::Entry`): the pane, who asked and what,
/// and the episode it belongs to (the host's `@moshpit_since`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StandingEntryValue {
    pub pane: String,
    pub agent: Option<String>,
    pub title: Option<String>,
    pub since: i64,
}

/// The waiting set and the rung finishes as the notification service keeps
/// them (crates/seal since E7b): what a push records, written the way the
/// service writes it, for the tests that put a push beside the machine.
/// The app itself only hands the machine their paths.
#[derive(uniffi::Object)]
pub struct PushFilesHandle {
    standing: PushStandingFile,
    finishes: PushFinishesFile,
}

#[uniffi::export]
impl PushFilesHandle {
    #[uniffi::constructor]
    pub fn new(standing_path: String, finishes_path: String) -> Arc<Self> {
        Arc::new(Self {
            standing: PushStandingFile::new(standing_path),
            finishes: PushFinishesFile::new(finishes_path),
        })
    }

    /// A pushed question, recorded at `now` as the service records it.
    /// True on an edge: nobody else on the connection was waiting.
    pub fn note_standing(&self, conn: String, entry: StandingEntryValue, now: f64) -> bool {
        let now = Date::from_unix(now);
        self.standing.note_standing(
            &conn,
            Entry {
                pane: entry.pane,
                agent: entry.agent,
                title: entry.title,
                location: None,
                since: entry.since,
                recorded_at: now,
                lifetime: None,
            },
            now,
        )
    }

    /// Who is waiting on `conn` at `now`.
    pub fn standing(&self, conn: String, now: f64) -> Vec<StandingEntryValue> {
        self.standing
            .standing(&conn, Date::from_unix(now))
            .into_iter()
            .map(|entry| StandingEntryValue {
                pane: entry.pane,
                agent: entry.agent,
                title: entry.title,
                since: entry.since,
            })
            .collect()
    }

    /// A pushed finish that rang, as the service records it.
    pub fn note_ring(&self, conn: String, pane: String, ended_at: i64, now: f64) -> bool {
        self.finishes
            .note_ring(&conn, &pane, ended_at, Date::from_unix(now))
    }
}

/// One line the notification service left (`moshpit_seal::diagnostics`):
/// when, in seconds since 1970, and what.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PushDiagnosticLine {
    pub at: f64,
    pub text: String,
}

/// The notification service's lines at or after `since` (seconds since
/// 1970), oldest first, from its ring at `path` (the App Group's
/// `push-diagnostics.json`): a separate process, whose log the app can't
/// read, so the diagnostics screen merges these in.
#[uniffi::export]
pub fn push_diagnostics_recent(path: String, since: f64) -> Vec<PushDiagnosticLine> {
    DiagnosticsFile::new(path)
        .recent(Date::from_unix(since))
        .into_iter()
        .map(|line| PushDiagnosticLine {
            at: line.at.unix(),
            text: line.text,
        })
        .collect()
}

/// The category the diagnostics screen files those lines under.
#[uniffi::export]
pub fn push_diagnostics_source() -> String {
    moshpit_seal::diagnostics::SOURCE.to_owned()
}

#[cfg(test)]
mod push_files_tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ffi-push-files-{}", moshpit_seal::random::hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_pushed_question_stands_and_a_ring_is_kept() {
        let dir = scratch();
        let files = PushFilesHandle::new(
            dir.join("standing-attention.json").display().to_string(),
            dir.join("rung-finishes.json").display().to_string(),
        );
        let now = 1_762_800_000.0;
        let entry = StandingEntryValue {
            pane: "%3".into(),
            agent: Some("claude".into()),
            title: Some("Bash: npm test".into()),
            since: 1_762_799_990,
        };
        assert!(
            files.note_standing("C".into(), entry.clone(), now),
            "the first question is an edge"
        );
        assert!(
            !files.note_standing("C".into(), entry.clone(), now + 1.0),
            "the same question again is not"
        );
        assert_eq!(files.standing("C".into(), now + 2.0), vec![entry]);
        assert!(files.standing("other".into(), now).is_empty());
        assert!(files.note_ring("C".into(), "%3".into(), 1_762_800_010, now + 20.0));
        assert!(
            !files.note_ring("C".into(), "%3".into(), 1_762_800_010, now + 21.0),
            "rung once"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_services_lines_come_back_from_a_moment_on() {
        let dir = scratch();
        let path = dir.join("push-diagnostics.json");
        let ring = DiagnosticsFile::new(&path);
        ring.record("older", Date::from_unix(1_762_800_000.0));
        // The ring keeps whole seconds (ISO 8601, as the Swift ring did).
        ring.record(
            "opened with the newest pairing",
            Date::from_unix(1_762_800_060.7),
        );
        let lines = push_diagnostics_recent(path.display().to_string(), 1_762_800_030.0);
        assert_eq!(
            lines,
            vec![PushDiagnosticLine {
                at: 1_762_800_060.0,
                text: "opened with the newest pairing".into()
            }]
        );
        assert!(
            push_diagnostics_recent(dir.join("absent.json").display().to_string(), 0.0).is_empty()
        );
        assert_eq!(push_diagnostics_source(), "push·extension");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
