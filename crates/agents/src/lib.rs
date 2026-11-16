//! What Moshpit makes of its coding agents (`AgentActivityMonitor`): the
//! Vibe Island's state machine, with no platform in it.
//!
//! Two signals feed it. The hooks an agent fires stamp its pane
//! (`@moshpit_state`, `@moshpit_since`, …) and the platform polls them
//! ([`Monitor::hooks`]); on a pane no hook governs, the output it prints and
//! the bell it rings stand in ([`Monitor::output`], [`Monitor::bell`]). Out
//! come [`Effect`]s: what the Live Activity and the widget should show
//! (sent only when it changes, and again before either would read as
//! stale), which cards to post or withdraw and what they say, and the
//! announced-episode record to keep.
//!
//! The rules it keeps, each with the story behind it in the Swift monitor it
//! replaces:
//! * a question must stand [`Tunables::announce_grace`] before it is
//!   announced, and is announced once per episode, across reconnects and
//!   relaunches (the record);
//! * the waiting set ([`PushStandingFile`], shared with the notification
//!   service) decides sound: only the 0→1 edge rings; a prompt a poll shows
//!   answered leaves it, whoever recorded it;
//! * a finish rings only for a turn of [`Tunables::done_threshold`] or
//!   more, and once between this card and the pushed one
//!   ([`PushFinishesFile`]);
//! * a disconnect keeps the cards of prompts still waiting; a delete takes
//!   them.
//!
//! Time is whatever the caller says it is: every entry point takes `now`
//! (unix seconds), and the grace fires on the next [`Monitor::sweep`] after
//! it runs out.

use std::collections::{BTreeMap, BTreeSet};

use moshpit_seal::copy::{self, CardText, Language};
use moshpit_seal::standing::{Entry, LIFETIME_SECONDS};
use moshpit_seal::{Date, PushFinishesFile, PushStandingFile};
use serde::Serialize;

/// The connection's id as the platform writes it (Swift's `uuidString`),
/// and a pane's id within it.
pub type Key = (String, String);

/// A pane's state, as the island and the widget show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Working,
    Attention,
    Done,
    Idle,
}

impl AgentState {
    /// Most urgent first.
    pub fn rank(self) -> u8 {
        match self {
            Self::Attention => 0,
            Self::Working => 1,
            Self::Done => 2,
            Self::Idle => 3,
        }
    }

    /// A hook's `@moshpit_state`; nil for anything a hook doesn't write.
    pub fn from_hook(raw: Option<&str>) -> Option<Self> {
        match raw? {
            "working" => Some(Self::Working),
            "attention" => Some(Self::Attention),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn raw(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Attention => "attention",
            Self::Done => "done",
            Self::Idle => "idle",
        }
    }
}

/// The knobs, with the monitor's values as defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tunables {
    /// Working becomes done after this much quiet (output heuristic).
    pub idle_after: f64,
    /// Sustained agent output before a pane earns a row.
    pub promotion_delay: f64,
    /// A heuristic finish stays listed this long.
    pub done_linger: f64,
    /// Rows the island carries.
    pub max_agents: usize,
    /// The least a stamp may come later than its poll's interval: a stamp
    /// holds for the interval plus this or half the interval, whichever is
    /// more ([`stamp_hold`]). A pane is hook-owned that long and goes back to
    /// the heuristic once no stamp came in it; the same window decides
    /// whether a finish is live. tmux's 2 s poll: 5 s, as it always was.
    pub stamp_margin: f64,
    /// How long a question must stand before the phone hears of it.
    pub announce_grace: f64,
    /// Announced-episode records older than this are dropped on load.
    pub announced_ttl: f64,
    /// An unchanged state is sent to the island again after this, inside
    /// its 120 s stale date. Not to the widget (B701): each write reloads
    /// its timelines, from a daily budget; it is written when it changes,
    /// and goes stale by its own timeline.
    pub resend_unchanged_after: f64,
    /// An island the system would not start is asked for again after this
    /// (B701).
    pub retry_island_after: f64,
    /// A finished turn this long or longer is worth a sound.
    pub done_threshold: i64,
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            idle_after: 4.0,
            promotion_delay: 2.0,
            done_linger: 25.0,
            max_agents: 4,
            stamp_margin: 3.0,
            announce_grace: 30.0,
            announced_ttl: 24.0 * 60.0 * 60.0,
            resend_unchanged_after: 60.0,
            retry_island_after: 15.0,
            done_threshold: 180,
        }
    }
}

/// The user's switches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub notifications: bool,
    pub live_activity: bool,
    /// "Show detail on lock screen".
    pub show_detail: bool,
    /// The attention sound.
    pub sound: bool,
    pub language: Language,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notifications: true,
            live_activity: true,
            show_detail: true,
            sound: true,
            language: Language::En,
        }
    }
}

/// Where a pane lives, from the platform's view of the multiplexer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneWhere {
    /// The foreground command the tree last showed.
    pub command: String,
    /// The session's display name.
    pub session: Option<String>,
    /// The window's display title.
    pub window: Option<String>,
    /// Where it is as Home names it (its workspace, a window someone named,
    /// its folder; `moshpit_ui`'s `pane_place`): none when nothing does.
    pub name: Option<String>,
    /// Its window's index and its own index in the window: what tells two
    /// rows apart that would read the same.
    pub window_index: Option<i64>,
    pub pane_index: Option<i64>,
}

/// One pane of a hook poll.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hook {
    pub pane: String,
    /// `@moshpit_state`.
    pub state: Option<String>,
    /// `@moshpit_agent`.
    pub agent: Option<String>,
    /// `@moshpit_since`, unix seconds.
    pub since: Option<f64>,
    /// What runs in the pane now, polled fresh.
    pub command: Option<String>,
    /// `@moshpit_title`.
    pub title: Option<String>,
    pub place: PaneWhere,
}

/// One row of the island and the widget.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Agent {
    /// `"<conn>:<pane>"`.
    pub id: String,
    pub connection_id: String,
    pub pane_id: String,
    pub command: String,
    pub location: String,
    /// Where it is as Home says it: "moshpit · mac", Home's name for the
    /// pane and the host; the host alone when nothing names the pane. Never
    /// a window's raw title (a version, `3: 1`).
    pub place: String,
    pub detail: Option<String>,
    pub state: AgentState,
    /// Unix seconds.
    pub started_at: f64,
    /// When its connection dropped (B662), Unix seconds: shown as it was,
    /// greyed, its time stopped there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<f64>,
}

/// How long a dropped connection's agents stay paused before they go.
pub const PAUSED_FOR: f64 = 15.0 * 60.0;

/// What the island (and the widget) shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IslandState {
    pub agents: Vec<Agent>,
    pub working_count: usize,
    pub attention_count: usize,
    /// The agent the pill shows; its pane is where a tap goes.
    pub headline_id: Option<String>,
}

/// A card to post: the words and where a tap lands.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The notification's identifier — the pushed copy's collapse id too.
    pub id: String,
    pub text: CardText,
    pub connection_id: String,
    pub pane_id: String,
}

/// What the platform does.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Show this on the Live Activity (asking for one if none is up).
    Island(IslandState),
    /// No agents: end every island.
    EndIsland,
    /// Mirror this for the widget (its `updatedAt` is the platform's now).
    Widget(IslandState),
    Post(Card),
    Withdraw(String),
    /// Keep the announced-episode record: `"<conn>|<pane>"` → unix seconds.
    SaveAnnounced(BTreeMap<String, f64>),
    /// Ask for permission to post (a connection is being watched).
    RequestAuthorization,
    /// An agent finished a turn: the hook's done edge, every one, whatever
    /// the sound and the cards make of it (the companion pet counts them).
    Finished { conn: String, pane: String },
    /// A decision worth a log line: the app's diagnostics, and what the
    /// device harnesses read (`announcing attention: pane %3 …`).
    Note(String),
}

/// Which pane is on screen. `pane` nil: every pane of the connection (a
/// session with no multiplexer is its one pane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visible {
    pub conn: String,
    pub pane: Option<String>,
}

/// Commands that are a shell, not an agent.
const SHELLS: &[&str] = &[
    "zsh", "bash", "sh", "fish", "dash", "ksh", "tmux", "login", "-zsh", "-bash", "-sh", "ssh",
    "mosh", "",
];

/// Commands the output heuristic may promote to a row.
const AGENT_COMMANDS: &[&str] = &[
    "claude", "codex", "gemini", "qwen", "qoder", "factory", "codebuddy", "aider", "goose",
    "opencode",
];

/// Moshpit's own plumbing stamps a pane for real (a pairing self-test).
pub const SELF_TEST_AGENT: &str = "moshpit-selftest";

/// A connection's waiting card, `<this><conn>`: the identifier the app
/// posts it under and the pushed copy's collapse id.
pub const ATTENTION_CARD: &str = "moshpit.attention.";

/// A pane's finish card, `<this><conn>.<pane>`, likewise.
pub const DONE_CARD: &str = "moshpit.done.";

/// Whether `command` (a pane's foreground) is a shell: the agent that
/// ran there is gone. Also what decides where a queued share may go
/// (moshpit_app::share, B622).
pub fn is_shell(command: &str) -> bool {
    SHELLS.contains(&command)
}

#[derive(Debug, Clone)]
struct Pane {
    command: String,
    place: PaneWhere,
    detail: Option<String>,
    state: AgentState,
    since: f64,
    last_output: f64,
    first_active: f64,
    visible: bool,
    hook_owned: bool,
    last_hook_seen: f64,
    last_hook_state: Option<AgentState>,
    /// How long the last stamp holds ([`stamp_hold`]): until then the pane
    /// is the hook's, and a stamp that follows within it continues the
    /// episode.
    hold: f64,
    /// When its turn began (B666): the first `working` after it was last
    /// idle or done. A finish is timed from here, the questions on the way
    /// included; none while no turn runs.
    turn_since: Option<f64>,
}

/// The machine.
pub struct Monitor {
    tunables: Tunables,
    settings: Settings,
    standing: Option<PushStandingFile>,
    finishes: Option<PushFinishesFile>,
    conns: BTreeMap<String, String>,
    /// Each connection's tree as the platform last showed it: where every
    /// pane lives, seen by a poll or not.
    trees: BTreeMap<String, BTreeMap<String, PaneWhere>>,
    panes: BTreeMap<Key, Pane>,
    announced: BTreeMap<Key, f64>,
    pending: BTreeMap<Key, (f64, f64)>,
    /// Connections whose link is down, since when (B662).
    down: BTreeMap<String, f64>,
    posted_attention: BTreeSet<Key>,
    /// Questions announced that the waiting set couldn't hold (B821), with
    /// when: no set (no shared container) or a write that failed. Their
    /// card is drawn from here, as it would be from the set.
    unkept: BTreeMap<Key, f64>,
    /// Connections no poll has come in for since the platform was
    /// suspended ([`Monitor::resumed`], B822): what they last showed may
    /// have changed.
    unconfirmed: BTreeSet<String>,
    /// Questions whose grace ran out while their connection was
    /// unconfirmed: waiting for its poll, noted once.
    awaiting_poll: BTreeSet<Key>,
    /// Connections a poll has come in for since they were tracked: what
    /// stands there can be told.
    polled: BTreeSet<String>,
    /// Connections with a waiting card up that is judged once their first
    /// poll is in ([`Monitor::delivered`]).
    unjudged: BTreeSet<String>,
    headline_offset: usize,
    widget_sent: Option<(IslandState, f64)>,
    /// The pane on screen at the last sweep (B449).
    on_screen: Option<Visible>,
    island_sent: Option<(IslandState, f64)>,
    island_ended: bool,
    effects: Vec<Effect>,
}

impl Monitor {
    /// `announced` is the record as stored; what is a day old is dropped.
    /// `standing` and `finishes` are the files shared with the notification
    /// service; none (no shared container) means every prompt is an edge,
    /// carded from what this app announced (B821), and every long finish
    /// rings.
    pub fn new(
        tunables: Tunables,
        settings: Settings,
        standing: Option<PushStandingFile>,
        finishes: Option<PushFinishesFile>,
        announced: &BTreeMap<String, f64>,
        now: f64,
    ) -> Self {
        let announced = decode_announced(announced, now, tunables.announced_ttl)
            .into_iter()
            .filter_map(|(key, at)| {
                let (conn, pane) = key.split_once('|')?;
                Some(((conn.to_owned(), pane.to_owned()), at))
            })
            .collect();
        Self {
            tunables,
            settings,
            standing,
            finishes,
            conns: BTreeMap::new(),
            trees: BTreeMap::new(),
            panes: BTreeMap::new(),
            announced,
            pending: BTreeMap::new(),
            down: BTreeMap::new(),
            posted_attention: BTreeSet::new(),
            unkept: BTreeMap::new(),
            unconfirmed: BTreeSet::new(),
            awaiting_poll: BTreeSet::new(),
            polled: BTreeSet::new(),
            unjudged: BTreeSet::new(),
            headline_offset: 0,
            widget_sent: None,
            on_screen: None,
            island_sent: None,
            island_ended: false,
            effects: Vec::new(),
        }
    }

    /// The announced record as stored, read once the preferences can be
    /// believed (B348). A launch before the first unlock (a prewarm) read
    /// it as empty, and its first save would have written that over the
    /// stored one: every question still standing announced again after a
    /// reboot, its earlier card never withdrawn. The stored episodes join the
    /// ones this process recorded — its own win, being newer — what is a
    /// day old is dropped, and the record is saved. A
    /// question standing now that the stored record names is the one
    /// announced before: its announcement is called off, and its card is
    /// ours to withdraw once it is answered.
    pub fn adopt_announced(&mut self, raw: &BTreeMap<String, f64>, now: f64) {
        let stored = decode_announced(raw, now, self.tunables.announced_ttl);
        for (flat, episode) in stored {
            let Some((conn, pane)) = flat.split_once('|') else {
                continue;
            };
            let key: Key = (conn.to_owned(), pane.to_owned());
            if self.announced.contains_key(&key) {
                continue;
            }
            self.announced.insert(key.clone(), episode);
            let standing = self
                .panes
                .get(&key)
                .is_some_and(|p| p.state == AgentState::Attention && p.since == episode);
            if standing {
                if self.pending.get(&key).is_some_and(|(at, _)| *at == episode) {
                    self.pending.remove(&key);
                }
                self.posted_attention.insert(key);
            }
        }
        self.save_announced();
    }

    /// What to do, in order, since the last call.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// The user's switches, as they are now. Turning the Live Activity off
    /// ends the island at once (B436): it used to stay up, frozen and read
    /// as "paused", until every agent it listed was gone — while connected,
    /// a finished agent stays listed, so in practice until a disconnect, or
    /// iOS's eight hours.
    pub fn set_settings(&mut self, settings: Settings) {
        let switched_off = self.settings.live_activity && !settings.live_activity;
        let cards_on = !self.settings.notifications && settings.notifications;
        self.settings = settings;
        if switched_off {
            self.end_island_while_off();
        }
        // Connections are watched whatever the switches say (B590): cards
        // switched on with some watched need the permission to post.
        if cards_on && self.is_tracking() {
            self.effects.push(Effect::RequestAuthorization);
        }
    }

    /// With the Live Activity off, no island stays up: ended once, and
    /// asked for afresh when it is switched on again.
    fn end_island_while_off(&mut self) {
        if !self.island_ended {
            self.effects.push(Effect::Note("live activity switched off — island ended".to_owned()));
            self.effects.push(Effect::EndIsland);
            self.island_ended = true;
        }
        self.island_sent = None;
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Whether anything is being watched (the platform's sweep runs while so).
    pub fn is_tracking(&self) -> bool {
        !self.conns.is_empty()
    }

    // MARK: - Connections

    /// Watch a connection, `label` being what the user calls it, whatever
    /// the switches say (B590): what reaches the phone is decided when it
    /// would, from the settings as they are then. One tracked with both off
    /// used to be dropped here — B538 made the app always track, and this
    /// still let it go — so turning either on later did nothing for it
    /// until it reconnected.
    pub fn track(&mut self, conn: &str, label: &str) {
        self.conns.insert(conn.to_owned(), label.to_owned());
        if self.settings.notifications {
            self.effects.push(Effect::RequestAuthorization);
        }
    }

    /// Stop watching. `keep_waiting_cards`: the user disconnected, and the
    /// agents there are still waiting — their cards stay up, and in the
    /// set; the next session's poll takes them up again. A deleted
    /// connection takes its cards with it.
    pub fn untrack(&mut self, conn: &str, keep_waiting_cards: bool, now: f64) {
        if keep_waiting_cards {
            self.posted_attention.retain(|key| key.0 != conn);
        } else {
            // A delete (B437): only the prompts this app posted used to
            // leave, and the card was then redrawn from what was left —
            // the notification service's — so a delete could post a fresh
            // "needs you" for the connection it deleted, and one the app
            // never posted for kept the pushed card for good. Its whole
            // waiting set goes, its record, and its card, not redrawn.
            if let Some(standing) = &self.standing {
                standing.clear_all(conn, Date::from_unix(now));
            }
            self.posted_attention.retain(|key| key.0 != conn);
            if self.announced.keys().any(|key| key.0 == conn) {
                self.announced.retain(|key, _| key.0 != conn);
                self.save_announced();
            }
            self.effects.push(Effect::Note(format!("deleted: {conn} — its waiting card withdrawn")));
            self.effects.push(Effect::Withdraw(format!("{ATTENTION_CARD}{conn}")));
        }
        self.conns.remove(conn);
        self.down.remove(conn);
        self.trees.remove(conn);
        self.polled.remove(conn);
        self.unjudged.remove(conn);
        self.unconfirmed.remove(conn);
        self.panes.retain(|key, _| key.0 != conn);
        self.pending.retain(|key, _| key.0 != conn);
        self.sync(now);
    }

    /// The agent cards the notification center holds, by identifier (B433),
    /// and the connections saved on this phone — `None` while that can't be
    /// told (before the first unlock). The platform hands them over when a
    /// connection is tracked and when the app comes forward.
    ///
    /// A waiting card the notification service posted is one this machine
    /// knew nothing of. Its prompt left the waiting set when the turn's
    /// `done` was pushed, or expired from it (fifteen minutes then; six
    /// hours since B660, so a question still waiting keeps its card), and
    /// nothing was left to take the card down: a question answered at the
    /// desk stayed "needs you" on the lock screen, beside its own
    /// "finished", for good. A connection's waiting card goes now when
    /// nothing there waits: no entry in the set, no card of this app's, no
    /// pane of it in attention — once its first poll has said which panes
    /// are; one with a pane still waiting is this machine's to take down
    /// when that pane is answered. A connection that is no longer saved
    /// takes its waiting card, its entries and its finish cards along.
    pub fn delivered(&mut self, ids: &[String], saved: Option<&BTreeSet<String>>, now: f64) {
        let is_saved = |conn: &str| saved.is_none_or(|saved| saved.iter().any(|id| id.eq_ignore_ascii_case(conn)));
        let at = Date::from_unix(now);
        for id in ids {
            if let Some(conn) = id.strip_prefix(ATTENTION_CARD) {
                if !is_saved(conn) {
                    self.effects.push(Effect::Note(format!("delivered: {conn} is not saved — its card withdrawn")));
                    if let Some(standing) = &self.standing {
                        standing.clear_all(conn, at);
                    }
                    self.posted_attention.retain(|key| key.0 != conn);
                    self.effects.push(Effect::Withdraw(id.clone()));
                } else if self.conns.contains_key(conn) && !self.polled.contains(conn) {
                    self.unjudged.insert(conn.to_owned());
                } else {
                    self.judge_waiting_card(conn, now);
                }
            } else if let Some((conn, _pane)) = id.strip_prefix(DONE_CARD).and_then(|rest| rest.split_once('.'))
                && !is_saved(conn)
            {
                self.effects.push(Effect::Withdraw(id.clone()));
            }
        }
    }

    /// Whether `conn`'s waiting card still says something true; withdrawn
    /// when it doesn't ([`Monitor::delivered`]).
    fn judge_waiting_card(&mut self, conn: &str, now: f64) {
        let at = Date::from_unix(now);
        let standing = self.standing.as_ref().is_some_and(|file| !file.standing(conn, at).is_empty());
        if standing || self.posted_attention.iter().any(|key| key.0 == conn) {
            return;
        }
        let waiting: Vec<Key> = self
            .panes
            .iter()
            .filter(|(key, p)| key.0 == conn && p.state == AgentState::Attention)
            .map(|(key, _)| key.clone())
            .collect();
        if waiting.is_empty() {
            self.effects.push(Effect::Note(format!("delivered: nothing waits on {conn} — its card withdrawn")));
            self.effects.push(Effect::Withdraw(format!("{ATTENTION_CARD}{conn}")));
        } else {
            // The card is up for a question still standing: taken down with
            // it, as one this app posted would be.
            self.posted_attention.extend(waiting);
        }
    }

    /// The connection's tree: where each of its panes lives. What a card
    /// says about a pane no poll has named yet — a prompt the notification
    /// service recorded — comes from here.
    pub fn tree(&mut self, conn: &str, places: BTreeMap<String, PaneWhere>) {
        if self.conns.contains_key(conn) {
            self.trees.insert(conn.to_owned(), places);
        }
    }

    /// `conn`'s link dropped (`up` false) or came back (B662). The machine
    /// had no such input: the polls stopped, the sweep took that for the
    /// stamps going away, and a working agent showed done within seconds,
    /// the island ended and started again. Down, its agents keep the state
    /// they had, paused; a "needs you" only a bell raised goes (nothing can
    /// tell any more), and no grace runs out into a card; past
    /// [`PAUSED_FOR`] they go. Back up, they resume: the next poll says
    /// the real state, and a question still unannounced gets its grace
    /// again.
    pub fn link(&mut self, conn: &str, up: bool, now: f64) {
        if !self.conns.contains_key(conn) {
            return;
        }
        if up {
            if self.down.remove(conn).is_none() {
                return;
            }
            let mut asking: Vec<(Key, f64)> = Vec::new();
            for (key, p) in &mut self.panes {
                if key.0 == conn {
                    // The next poll has its hold to arrive in.
                    p.last_hook_seen = now;
                    p.last_output = now;
                    if p.state == AgentState::Attention {
                        asking.push((key.clone(), p.since));
                    }
                }
            }
            self.effects.push(Effect::Note(format!("link up: {conn} — its agents resume")));
            // A question the drop kept from being announced gets a grace
            // from now: the polls say by then whether it still stands.
            for (key, episode) in asking {
                if should_announce(AgentState::Attention, episode, self.announced.get(&key).copied()) {
                    self.schedule_announce(key, episode, now);
                }
            }
        } else {
            if self.down.contains_key(conn) {
                return;
            }
            self.down.insert(conn.to_owned(), now);
            let rung: Vec<Key> = self
                .panes
                .iter()
                .filter(|(key, p)| key.0 == conn && !p.hook_owned && p.state == AgentState::Attention)
                .map(|(key, _)| key.clone())
                .collect();
            for key in &rung {
                self.panes.remove(key);
            }
            self.pending.retain(|key, _| key.0 != conn);
            self.effects.push(Effect::Note(format!("link down: {conn} — its agents paused")));
        }
        self.sync(now);
    }

    /// The platform was suspended, and is back (B822): its sweeps and its
    /// polls stopped meanwhile — iOS freezes the app in the background — so
    /// what each connection last showed may no longer hold. Until a poll
    /// for it comes in, a question its stamps raised is not announced: the
    /// first sweep back used to announce what the last poll before the
    /// suspension showed, sound and all, though it was answered at the desk
    /// meanwhile, and withdraw it a poll later.
    pub fn resumed(&mut self) {
        self.unconfirmed = self.conns.keys().cloned().collect();
        self.effects.push(Effect::Note("resumed: announcements wait for each connection's poll".to_owned()));
    }

    /// Rotate which agent the pill shows.
    pub fn cycle_headline(&mut self, now: f64) {
        self.headline_offset += 1;
        self.sync(now);
    }

    /// When each tracked pane of `conn` entered its state.
    pub fn state_since_by_pane(&self, conn: &str) -> BTreeMap<String, f64> {
        self.panes
            .iter()
            .filter(|(key, _)| key.0 == conn)
            .map(|(key, pane)| (key.1.clone(), pane.since))
            .collect()
    }

    // MARK: - The output heuristic (panes no hook governs)

    /// Output from a pane.
    pub fn output(&mut self, conn: &str, pane: &str, place: PaneWhere, now: f64) {
        let key: Key = (conn.to_owned(), pane.to_owned());
        if let Some(p) = self.panes.get_mut(&key)
            && hook_governs(p, now)
        {
            p.last_output = now;
            return;
        }
        let command = place.command.clone();
        // A visible agent back at its shell has finished its turn.
        if is_shell(&command) {
            if let Some(p) = self.panes.get_mut(&key)
                && p.visible
                && p.state == AgentState::Working
            {
                p.state = AgentState::Done;
                p.since = now;
                self.sync(now);
            }
            return;
        }
        let known = AGENT_COMMANDS.contains(&command.as_str());
        let promotion_delay = self.tunables.promotion_delay;
        if let Some(p) = self.panes.get_mut(&key) {
            p.command = command;
            p.place = place;
            p.last_output = now;
            if !p.visible {
                if known && now - p.first_active >= promotion_delay {
                    p.visible = true;
                    p.state = AgentState::Working;
                    p.since = now;
                }
            } else if p.state != AgentState::Working {
                p.state = AgentState::Working;
                p.since = now;
            }
        } else if known {
            self.panes.insert(
                key,
                Pane {
                    command,
                    place,
                    detail: None,
                    state: AgentState::Working,
                    since: now,
                    last_output: now,
                    first_active: now,
                    visible: false,
                    hook_owned: false,
                    last_hook_seen: f64::NEG_INFINITY,
                    last_hook_state: None,
                    // No stamp yet: read only once one makes it the hook's.
                    hold: 0.0,
                    turn_since: None,
                },
            );
        } else {
            return;
        }
        self.sync(now);
    }

    /// A bell from a pane: attention at once, through the same grace. Only
    /// a known agent's (B663): one the pane's foreground names, or one whose
    /// stamp said it is there while no shell is in front. A shell's bell (a
    /// tab completion with nothing to complete, a build that rings when it
    /// ends, `printf '\a'`) asks nothing: it used to raise "zsh needs you".
    pub fn bell(&mut self, conn: &str, pane: &str, place: PaneWhere, now: f64) {
        let key: Key = (conn.to_owned(), pane.to_owned());
        if let Some(p) = self.panes.get(&key)
            && hook_governs(p, now)
        {
            return;
        }
        let foreground = display_command(&place.command);
        let stamped = self.panes.get(&key).is_some_and(|p| p.last_hook_state.is_some());
        if !AGENT_COMMANDS.contains(&foreground) && !(stamped && !is_shell(foreground)) {
            self.effects.push(Effect::Note(format!(
                "bell: pane {pane} runs no known agent ({}) — not a question",
                if foreground.is_empty() { "nothing" } else { foreground }
            )));
            return;
        }
        let command = place.command.clone();
        let p = self.panes.entry(key.clone()).or_insert_with(|| Pane {
            command: command.clone(),
            place: place.clone(),
            detail: None,
            state: AgentState::Attention,
            since: now,
            last_output: now,
            first_active: now,
            visible: true,
            hook_owned: false,
            last_hook_seen: f64::NEG_INFINITY,
            last_hook_state: None,
            hold: 0.0,
            turn_since: None,
        });
        if !is_shell(&command) {
            p.command = command;
        }
        p.place = place;
        p.state = AgentState::Attention;
        p.since = now;
        p.visible = true;
        let episode = p.since;
        if should_announce(AgentState::Attention, episode, self.announced.get(&key).copied()) {
            self.schedule_announce(key, episode, now);
        }
        self.sync(now);
    }

    /// The person typed into `pane` from the phone (B663): a "needs you"
    /// only a bell raised is answered, and the pane is back at work (its
    /// card goes). A stamp's question is the stamp's to clear.
    pub fn input(&mut self, conn: &str, pane: &str, now: f64) {
        let key: Key = (conn.to_owned(), pane.to_owned());
        let Some(p) = self.panes.get_mut(&key) else {
            return;
        };
        if p.hook_owned || p.state != AgentState::Attention {
            return;
        }
        p.state = AgentState::Working;
        p.since = now;
        p.last_output = now;
        self.pending.remove(&key);
        self.effects.push(Effect::Note(format!("input: pane {pane} answered its bell")));
        self.sync(now);
    }

    // MARK: - The hooks

    /// One poll of `conn`'s hook stamps. The stamp is authoritative for
    /// every pane it names. `interval`: how often `conn`'s stamps arrive,
    /// in seconds — the poll that reads them (tmux: the monitor's sweep;
    /// herdr: its client's poll, longer while its push channel is up). A
    /// stamp holds that long and a margin ([`stamp_hold`], B346): herdr's
    /// come every 8 s, or 45 s with push, and a fixed 5 s let every herdr
    /// agent fall to the heuristic between polls — its time restarting, a
    /// working one shown done, and no turn ever ending in a card.
    pub fn hooks(&mut self, conn: &str, hooks: Vec<Hook>, now: f64, interval: f64) {
        if !self.conns.contains_key(conn) {
            return;
        }
        // A poll came: the link is up (B662), and what it shows is current
        // (B822).
        if self.down.contains_key(conn) {
            self.link(conn, true, now);
        }
        self.unconfirmed.remove(conn);
        let hold = stamp_hold(interval, self.tunables.stamp_margin);
        let mut changed = false;
        let mut answered: BTreeMap<String, f64> = BTreeMap::new();
        let named: BTreeSet<String> = hooks.iter().map(|hook| hook.pane.clone()).collect();
        for hook in hooks {
            let Some(mapped) = AgentState::from_hook(hook.state.as_deref()) else { continue };
            let new_state = reclassify(mapped, hook.title.as_deref(), hook.command.as_deref());
            // What the title says, its markup off (B595).
            let title = hook.title.as_deref().map(plain_detail).filter(|t| !t.is_empty());
            if hook.agent.as_deref() == Some(SELF_TEST_AGENT) {
                continue;
            }
            let key: Key = (conn.to_owned(), hook.pane.clone());
            let command = hook.agent.clone().unwrap_or_else(|| hook.place.command.clone());
            // A stamp with no `since` (herdr reports no transition time) is
            // dated the first time it is seen — and a reconnect or a
            // relaunch, which forget the panes, used to date a question
            // still standing anew and announce it again, once per
            // reconnect. One the record has, on a pane not seen to leave
            // attention since, is the episode it names. Seen to leave, the
            // record lets go, so the next question is a new one.
            //
            // A question the push recorded is dated by the push (B592): the
            // host's own time, which the app's copy then shares, so it's
            // the same question and doesn't ring again. Dated by the
            // phone's clock, it counted as another.
            let since = match hook.since {
                Some(since) => since,
                None if new_state == AgentState::Attention => self
                    .announced
                    .get(&key)
                    .copied()
                    .or_else(|| self.pushed_since(&key, now))
                    .unwrap_or(now),
                None => {
                    if self.announced.remove(&key).is_some() {
                        self.save_announced();
                    }
                    now
                }
            };
            // A stamp out of attention answers what waited before it: dated
            // by the host when it can be; herdr's, by now (B592) — its
            // answers cleared nothing, and a question answered inside the
            // grace left the push's card standing for good.
            if new_state != AgentState::Attention {
                answered.insert(hook.pane.clone(), hook.since.unwrap_or(now));
            }
            let previous = self
                .panes
                .get(&key)
                .map(|p| (p.since, p.last_hook_state, p.last_hook_seen, p.hold));
            // The turn this stamp is part of (B666): a finish was timed from
            // the last episode alone, so a twenty-minute task that asked one
            // question at minute nineteen read "1 min" and rang nothing.
            let in_turn = |state: AgentState| matches!(state, AgentState::Working | AgentState::Attention);
            let turn_began = self
                .panes
                .get(&key)
                .and_then(|p| in_turn(p.state).then(|| p.turn_since.unwrap_or(p.since)));
            // The earliest start known: a stamp can date the turn before
            // what the phone saw of it (a bell's "needs you" first).
            let turn_since = in_turn(new_state).then(|| turn_began.map_or(since, |began| began.min(since)));
            // A herdr question the app dated by the phone, not yet announced,
            // that the push has since recorded (B824): the push's date is the
            // question's (B592), whichever came first. The host's push comes
            // after the host's grace, inside the app's, and the app's
            // announcement then counted as another question and rang again.
            let pushed = if hook.since.is_none() && new_state == AgentState::Attention {
                self.pushed_since(&key, now)
            } else {
                None
            };
            let announced_episode = self.announced.get(&key).copied();
            match self.panes.get_mut(&key) {
                Some(p) => {
                    // Taking a pane over is a new episode when the stamp
                    // dates it; ownership regained by a stamp that doesn't
                    // (herdr's), in the state the pane is in, is the same
                    // episode: its time doesn't restart (B346). A question
                    // dated later than the one waiting is a new one (B593):
                    // the pane was answered and asked again between two
                    // polls, and the second was never announced.
                    let state_changed = p.state != new_state
                        || (!p.hook_owned && hook.since.is_some())
                        || (new_state == AgentState::Attention
                            && hook.since.is_some_and(|stamped| stamped > p.since));
                    let dated_by_push = pushed.filter(|pushed| {
                        !state_changed
                            && p.state == AgentState::Attention
                            && *pushed != p.since
                            && announced_episode != Some(p.since)
                    });
                    p.command = command.clone();
                    p.place = hook.place.clone();
                    p.detail = title.clone();
                    p.state = new_state;
                    if state_changed {
                        p.since = since;
                    } else if let Some(pushed) = dated_by_push {
                        // Its grace runs on as it was: the question has
                        // waited since the app first saw it.
                        if let Some((episode, _)) = self.pending.get_mut(&key)
                            && *episode == p.since
                        {
                            *episode = pushed;
                        }
                        p.since = pushed;
                    }
                    p.visible = true;
                    p.hook_owned = true;
                    p.last_hook_seen = now;
                    p.last_hook_state = Some(mapped);
                    p.last_output = now;
                    p.hold = hold;
                    p.turn_since = turn_since;
                }
                None => {
                    self.panes.insert(
                        key.clone(),
                        Pane {
                            command: command.clone(),
                            place: hook.place.clone(),
                            detail: title.clone(),
                            state: new_state,
                            since,
                            last_output: now,
                            first_active: now,
                            visible: true,
                            hook_owned: true,
                            last_hook_seen: now,
                            last_hook_state: Some(mapped),
                            hold,
                            turn_since,
                        },
                    );
                }
            }
            changed = true;

            let episode = self.panes[&key].since;
            // A prompt this app announced, still standing after a
            // disconnect or a relaunch: its card is ours to withdraw again.
            if new_state == AgentState::Attention && self.announced.get(&key) == Some(&episode) {
                self.posted_attention.insert(key.clone());
            }
            if should_announce(new_state, episode, self.announced.get(&key).copied()) {
                self.schedule_announce(key.clone(), episode, now);
            }
            let (previous_since, previous_hook, previous_seen, previous_hold) = match previous {
                Some((since, hook, seen, hold)) => (Some(since), hook, Some(seen), hold),
                None => (None, None, None, hold),
            };
            // Judged by what the previous stamp promised: push coming up
            // or going away changes the interval between two stamps.
            if is_turn_ending(mapped, new_state, previous_hook, previous_seen, now, previous_hold) {
                // Before the card: `post_done` stops early when cards are off
                // or the pane was on screen, and every finish counts here.
                self.effects.push(Effect::Finished { conn: key.0.clone(), pane: key.1.clone() });
                let duration = turn_began.or(previous_since).map_or(0.0, |began| since - began);
                let detail = if self.settings.show_detail { title.clone() } else { None };
                self.post_done(&key, &command, detail, duration, since, now);
            }
        }
        self.clear_answered(conn, &answered, now);
        changed |= self.forget_gone(conn, &named, now);
        if changed {
            self.sync(now);
        }
        self.polled.insert(conn.to_owned());
        if self.unjudged.remove(conn) {
            self.judge_waiting_card(conn, now);
        }
    }

    /// The panes of `conn` that are gone: neither this poll — which lists
    /// every pane of the host's server, stamped or not — nor the tree holds
    /// them any more (their window was killed, their session ended). No
    /// signal will ever come from them, and a question they asked can't be
    /// answered: they leave the island, their grace is called off, and a
    /// question of theirs leaves the waiting set, its card redrawn or
    /// withdrawn — also one this app announced before a relaunch or a
    /// disconnect (the record names it), and one the notification service
    /// recorded for a pane this machine was showing. One used to stand as
    /// "needs you" for as long as the connection was watched — `attention`
    /// sticks until something from the pane moves it — announced if it
    /// closed inside the grace, its card up for good, and its entry counted
    /// as waiting, so the next question elsewhere came in silent. Without a
    /// tree nothing is told gone. Whether the island changed.
    fn forget_gone(&mut self, conn: &str, named: &BTreeSet<String>, now: f64) -> bool {
        let Some(tree) = self.trees.get(conn) else { return false };
        let is_gone = |key: &Key| key.0 == conn && !named.contains(&key.1) && !tree.contains_key(&key.1);
        let gone: Vec<Key> = self.panes.keys().filter(|key| is_gone(key)).cloned().collect();
        let recorded: Vec<Key> = self
            .announced
            .keys()
            .filter(|key| is_gone(key) && !self.panes.contains_key(*key))
            .cloned()
            .collect();
        if gone.is_empty() && recorded.is_empty() {
            return false;
        }
        let at = Date::from_unix(now);
        let standing: BTreeSet<String> = self
            .standing
            .as_ref()
            .map(|file| file.standing(conn, at).into_iter().map(|entry| entry.pane).collect())
            .unwrap_or_default();
        let mut cleared = false;
        for key in &gone {
            self.effects.push(Effect::Note(format!("gone: pane {} closed — forgotten", key.1)));
            let was_waiting = self.panes.remove(key).is_some_and(|p| p.state == AgentState::Attention);
            self.pending.remove(key);
            if was_waiting && (standing.contains(&key.1) || self.posted_attention.contains(key)) {
                cleared |= self.clear_gone(key, at);
            }
        }
        for key in &recorded {
            if standing.contains(&key.1) || self.posted_attention.contains(key) {
                self.effects.push(Effect::Note(format!("gone: pane {} closed — standing cleared", key.1)));
                cleared |= self.clear_gone(key, at);
            }
        }
        for key in gone.iter().chain(&recorded) {
            self.announced.remove(key);
        }
        self.save_announced();
        if cleared {
            let empty = self.standing.as_ref().is_none_or(|file| file.standing(conn, at).is_empty());
            if self.settings.notifications || empty {
                self.render_digest(conn, false, now);
            }
        }
        !gone.is_empty()
    }

    /// A gone pane's question leaves the waiting set, whoever recorded it.
    fn clear_gone(&mut self, key: &Key, at: Date) -> bool {
        if let Some(standing) = &self.standing {
            standing.clear(&key.0, &key.1, at);
        }
        self.posted_attention.remove(key);
        true
    }

    // MARK: - The sweep

    /// Every two seconds while anything is tracked: announcements whose
    /// grace ran out, a card for the pane on screen acknowledged, and the
    /// heuristic's ageing.
    pub fn sweep(&mut self, visible: Option<&Visible>, now: f64) {
        self.on_screen = visible.cloned();
        self.fire_due_announcements(now);
        if let Some(visible) = visible {
            let seen: Vec<Key> = self
                .posted_attention
                .iter()
                .filter(|key| {
                    key.0 == visible.conn && visible.pane.as_deref().is_none_or(|pane| pane == key.1)
                })
                .cloned()
                .collect();
            for key in seen {
                self.effects.push(Effect::Note(format!(
                    "acknowledged: pane {} viewed — standing cleared",
                    key.1
                )));
                if let Some(standing) = &self.standing {
                    standing.clear(&key.0, &key.1, Date::from_unix(now));
                }
                self.posted_attention.remove(&key);
                self.render_digest(&key.0, false, now);
            }
        }
        let tunables = self.tunables;
        let mut changed = false;
        let mut forget: Vec<Key> = Vec::new();
        // A connection down past its pause: its agents go (B662).
        let expired: Vec<String> = self
            .down
            .iter()
            .filter(|(_, since)| now - **since > PAUSED_FOR)
            .map(|(conn, _)| conn.clone())
            .collect();
        for conn in &expired {
            self.down.remove(conn);
            let before = self.panes.len();
            self.panes.retain(|key, _| &key.0 != conn);
            changed |= self.panes.len() != before;
            self.effects.push(Effect::Note(format!("link down past its pause: {conn} — its agents go")));
        }
        for (key, p) in &mut self.panes {
            // A dropped connection's agents hold still (B662).
            if self.down.contains_key(&key.0) {
                continue;
            }
            if p.hook_owned && now - p.last_hook_seen > p.hold {
                p.hook_owned = false;
                p.last_output = now;
                p.first_active = now;
                changed = true;
                continue;
            }
            if hook_governs(p, now) {
                continue;
            }
            if p.visible {
                match p.state {
                    AgentState::Working => {
                        if now - p.last_output > tunables.idle_after {
                            p.state = AgentState::Done;
                            p.since = now;
                            changed = true;
                        }
                    }
                    AgentState::Done => {
                        if now - p.since > tunables.done_linger {
                            forget.push(key.clone());
                        }
                    }
                    AgentState::Idle => forget.push(key.clone()),
                    AgentState::Attention => {}
                }
            } else if now - p.last_output > tunables.promotion_delay + tunables.idle_after {
                forget.push(key.clone());
            }
        }
        if !forget.is_empty() {
            for key in &forget {
                self.panes.remove(key);
                self.announced.remove(key);
            }
            self.save_announced();
            changed = true;
        }
        // Or the island due again (B701): only a change or a stamped poll
        // made a sync, and the sync is what sends an unchanged island again.
        // Rows a bell or output raised have no poll behind them: their island
        // was never sent again and went stale ("paused") with the app in
        // front. Once a minute, not every sweep.
        let resend = self.tunables.resend_unchanged_after;
        let island_due = self
            .island_sent
            .as_ref()
            .is_some_and(|(sent, at)| keeps_fresh(sent) && (now - at >= resend || now < *at));
        if changed || island_due {
            self.sync(now);
        }
    }

    /// The system would not start the island the last [`Effect::Island`]
    /// asked for (B701): it was taken as shown, and not asked for again
    /// until the state changed — for good, with nothing to change it. It is
    /// asked for again after [`Tunables::retry_island_after`].
    pub fn island_not_shown(&mut self, now: f64) {
        if let Some((_, at)) = self.island_sent.as_mut() {
            *at = now - self.tunables.resend_unchanged_after + self.tunables.retry_island_after;
            self.effects.push(Effect::Note("island not shown — asked for again shortly".to_owned()));
        }
    }

    /// When the push recorded `key`'s question, by the host's clock.
    fn pushed_since(&self, key: &Key, now: f64) -> Option<f64> {
        self.standing
            .as_ref()?
            .standing(&key.0, Date::from_unix(now))
            .into_iter()
            .find(|entry| entry.pane == key.1)
            .map(|entry| entry.since as f64)
    }

    // MARK: - Announcing

    fn schedule_announce(&mut self, key: Key, episode: f64, now: f64) {
        if self.pending.get(&key).is_some_and(|(pending, _)| *pending == episode) {
            return;
        }
        self.effects.push(Effect::Note(format!(
            "announce scheduled: pane {} grace {}s",
            key.1, self.tunables.announce_grace
        )));
        self.pending.insert(key, (episode, now + self.tunables.announce_grace));
    }

    /// Every grace that has run out, in the order they ran out: the order
    /// their timers fire in, which decides whose prompt is the edge.
    fn fire_due_announcements(&mut self, now: f64) {
        // A grace that ends more than a grace from now was scheduled before
        // the clock was set back: it ends a grace from now instead, not as
        // much later as the clock went back.
        let latest = now + self.tunables.announce_grace;
        for (_, at) in self.pending.values_mut() {
            if *at > latest {
                *at = latest;
            }
        }
        let mut due: Vec<(f64, Key, f64)> = self
            .pending
            .iter()
            .filter(|(_, (_, at))| *at <= now)
            .map(|(key, (episode, at))| (*at, key.clone(), *episode))
            .collect();
        due.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (at, key, episode) in due {
            self.pending.remove(&key);
            // The window did its job if the question is gone or superseded.
            let Some(p) = self.panes.get(&key).filter(|p| p.state == AgentState::Attention && p.since == episode)
            else {
                self.awaiting_poll.remove(&key);
                self.effects.push(Effect::Note(format!(
                    "announce cancelled: pane {} left the episode within the grace",
                    key.1
                )));
                continue;
            };
            // A stamp's question waits, after a suspension, for the poll that
            // says it still stands (B822). An app suspended mid-grace swept on
            // its return before any poll was in: what its last poll showed,
            // maybe answered at the desk meanwhile, was announced, sound and
            // all, as the app came back, and withdrawn a poll later.
            if p.last_hook_state == Some(AgentState::Attention) && self.unconfirmed.contains(&key.0) {
                if self.awaiting_poll.insert(key.clone()) {
                    self.effects.push(Effect::Note(format!(
                        "announce waits: pane {} — no poll since the app was suspended",
                        key.1
                    )));
                }
                self.pending.insert(key, (episode, at));
                continue;
            }
            self.awaiting_poll.remove(&key);
            let (command, detail) = (p.command.clone(), p.detail.clone());
            self.announced.insert(key.clone(), episode);
            self.save_announced();
            // On screen when its grace ran out (B591): the person is looking
            // at the question. It used to be posted, banner and sound, and
            // withdrawn by the same sweep as viewed; it's acknowledged now,
            // and the record keeps it from being announced later.
            if self
                .on_screen
                .as_ref()
                .is_some_and(|v| v.conn == key.0 && v.pane.as_deref().is_none_or(|pane| pane == key.1))
            {
                self.effects.push(Effect::Note(format!("prompt on screen: pane {} — no card", key.1)));
                // A copy the push recorded is seen too: it leaves the set,
                // the card redrawn from what's left, or withdrawn.
                let at = Date::from_unix(now);
                if let Some(standing) = &self.standing
                    && standing.standing(&key.0, at).iter().any(|entry| entry.pane == key.1)
                {
                    standing.clear(&key.0, &key.1, at);
                    self.render_digest(&key.0, false, now);
                }
                continue;
            }
            let detail = if self.settings.show_detail { detail } else { None };
            self.post_attention(&key, &command, detail, episode, now);
        }
    }

    fn post_attention(&mut self, key: &Key, command: &str, detail: Option<String>, episode: f64, now: f64) {
        let location = self.panes.get(key).map(|p| self.location(key, &p.place)).unwrap_or_default();
        self.effects.push(Effect::Note(format!("announcing attention: pane {} on {location}", key.1)));
        if !self.settings.notifications {
            return;
        }
        let at = Date::from_unix(now);
        let (edge, kept) = match &self.standing {
            Some(standing) => {
                let edge = standing.note_standing(
                    &key.0,
                    Entry {
                        pane: key.1.clone(),
                        agent: Some(display_command(command).to_owned()),
                        title: detail,
                        location: Some(self.place(key)),
                        since: episode as i64,
                        recorded_at: at,
                        lifetime: Some(LIFETIME_SECONDS),
                    },
                    at,
                );
                let kept = standing
                    .standing(&key.0, at)
                    .iter()
                    .any(|entry| entry.pane == key.1 && entry.since == episode as i64);
                (edge, kept)
            }
            None => (true, false),
        };
        if kept {
            self.unkept.remove(key);
        } else {
            self.effects.push(Effect::Note(format!(
                "waiting set can't hold pane {} — carded from what this app announced",
                key.1
            )));
            self.unkept.insert(key.clone(), now);
        }
        self.posted_attention.insert(key.clone());
        self.render_digest(&key.0, edge, now);
    }

    /// Draw — or withdraw — the one card a connection's waiting prompts
    /// share, from the newest standing entry.
    fn render_digest(&mut self, conn: &str, edge: bool, now: f64) {
        let id = format!("{ATTENTION_CARD}{conn}");
        let standing = self.waiting(conn, now);
        let Some(newest) = standing.first() else {
            self.effects.push(Effect::Withdraw(id));
            return;
        };
        let fallback = self.place(&(conn.to_owned(), newest.pane.clone()));
        let name = self.conns.get(conn).cloned().unwrap_or_default();
        let text = digest(newest, standing.len(), self.settings, edge, &fallback, &name);
        self.effects.push(Effect::Post(Card {
            id,
            text,
            connection_id: conn.to_owned(),
            pane_id: newest.pane.clone(),
        }));
    }

    /// What waits on `conn`, newest first: the waiting set's entries, and
    /// the questions this app announced that the set couldn't hold (B821).
    /// The card was drawn from the set alone, so with no set — no shared
    /// container — or a write that failed, an announced question withdrew
    /// the card it should have posted.
    fn waiting(&mut self, conn: &str, now: f64) -> Vec<Entry> {
        let mut entries = self
            .standing
            .as_ref()
            .map(|file| file.standing(conn, Date::from_unix(now)))
            .unwrap_or_default();
        let posted = &self.posted_attention;
        self.unkept.retain(|key, _| posted.contains(key));
        let unkept: Vec<Entry> = self
            .unkept
            .iter()
            .filter(|(key, _)| key.0 == conn && !entries.iter().any(|entry| entry.pane == key.1))
            .filter_map(|(key, at)| {
                let p = self.panes.get(key)?;
                Some(Entry {
                    pane: key.1.clone(),
                    agent: Some(display_command(&p.command).to_owned()),
                    title: p.detail.clone(),
                    location: Some(self.place(key)),
                    since: p.since as i64,
                    recorded_at: Date::from_unix(*at),
                    lifetime: Some(LIFETIME_SECONDS),
                })
            })
            .collect();
        if !unkept.is_empty() {
            entries.extend(unkept);
            entries.sort_by(|a, b| b.recorded_at.partial_cmp(&a.recorded_at).unwrap_or(std::cmp::Ordering::Equal));
        }
        entries
    }

    /// The finish card: a long turn rings once between this card and the
    /// pushed copy the host sent when the turn stopped.
    fn post_done(&mut self, key: &Key, command: &str, detail: Option<String>, duration: f64, ended_at: f64, now: f64) {
        if !self.settings.notifications {
            return;
        }
        // A turn that finished on screen needs no card (B449): the person
        // watched it end. It used to post one all the same, a banner over
        // the very pane, then sit in the list as news. Its finish counts as
        // rung, so the pushed copy, if the host's arrives, is quiet too.
        if self
            .on_screen
            .as_ref()
            .is_some_and(|v| v.conn == key.0 && v.pane.as_deref().is_none_or(|pane| pane == key.1))
        {
            self.effects.push(Effect::Note(format!("finish on screen: pane {} — no card", key.1)));
            if let Some(finishes) = &self.finishes {
                finishes.note_ring(&key.0, &key.1, ended_at as i64, Date::from_unix(now));
            }
            return;
        }
        let threshold = self.tunables.done_threshold;
        let duration = duration as i64;
        let rings = !copy::done_wakes(duration, threshold)
            || self.finishes.as_ref().is_none_or(|finishes| {
                finishes.note_ring(&key.0, &key.1, ended_at as i64, Date::from_unix(now))
            });
        let text = copy::done(
            display_command(command),
            detail.as_deref(),
            &self.place(key),
            duration,
            self.settings.sound,
            threshold,
            rings,
            self.settings.language,
        );
        self.effects.push(Effect::Post(Card {
            id: format!("{DONE_CARD}{}.{}", key.0, key.1),
            text,
            connection_id: key.0.clone(),
            pane_id: key.1.clone(),
        }));
    }

    /// The prompts a poll shows answered leave the waiting set, whoever put
    /// them there — only entries older than the state the pane is in now
    /// (host clock on both sides), so a stale reply answers nothing.
    fn clear_answered(&mut self, conn: &str, answered: &BTreeMap<String, f64>, now: f64) {
        if answered.is_empty() {
            return;
        }
        let Some(standing) = &self.standing else { return };
        let at = Date::from_unix(now);
        let gone: Vec<String> = standing
            .standing(conn, at)
            .into_iter()
            .filter(|entry| answered.get(&entry.pane).is_some_and(|since| (entry.since as f64) < *since))
            .map(|entry| entry.pane)
            .collect();
        if gone.is_empty() {
            return;
        }
        for pane in &gone {
            self.effects.push(Effect::Note(format!(
                "answered: pane {pane} left attention — standing cleared"
            )));
            standing.clear(conn, pane, at);
            self.posted_attention.remove(&(conn.to_owned(), pane.clone()));
        }
        let empty = standing.standing(conn, at).is_empty();
        if self.settings.notifications || empty {
            self.render_digest(conn, false, now);
        }
    }

    // MARK: - What shows

    fn sync(&mut self, now: f64) {
        // Panes that left `attention` leave the set; their card follows.
        let attention_now: BTreeSet<Key> = self
            .panes
            .iter()
            .filter(|(_, p)| p.state == AgentState::Attention)
            .map(|(key, _)| key.clone())
            .collect();
        let stale: Vec<Key> = self.posted_attention.difference(&attention_now).cloned().collect();
        if !stale.is_empty() {
            if let Some(standing) = &self.standing {
                for key in &stale {
                    standing.clear(&key.0, &key.1, Date::from_unix(now));
                }
            }
            self.posted_attention = self.posted_attention.intersection(&attention_now).cloned().collect();
            let conns: BTreeSet<String> = stale.into_iter().map(|key| key.0).collect();
            for conn in conns {
                self.render_digest(&conn, false, now);
            }
        }

        let state = self.build_state(now, true);
        // The widget draws its first row, the most urgent agent, and opens
        // that one (B594); the headline is the island's, which cycling
        // moves. Its state said the island's, and the app's widget its
        // first row: the two disagreed in every script with a cycle. Its
        // finished agents stay (B707 is the island's).
        let mut widget = self.build_state(now, false);
        widget.headline_id = widget.agents.first().map(|agent| agent.id.clone());
        // Only when it changed (B701): an unchanged state was written again
        // each minute a stamped poll came, every write a reload of the
        // widget's timelines, which iOS budgets by the day. Its own timeline
        // turns it stale when the app stops writing.
        if needs_sending(&widget, self.widget_sent.as_ref(), now, f64::INFINITY) {
            self.effects.push(Effect::Widget(widget.clone()));
            self.widget_sent = Some((widget, now));
        }
        if state.agents.is_empty() {
            if !self.island_ended {
                self.effects.push(Effect::EndIsland);
                self.island_ended = true;
            }
            self.island_sent = None;
            return;
        }
        if !self.settings.live_activity {
            // Also an island a launch with the switch off finds up.
            self.end_island_while_off();
            return;
        }
        self.island_ended = false;
        let resend = match &self.island_sent {
            Some((sent, _)) if keeps_fresh(sent) => self.tunables.resend_unchanged_after,
            _ => f64::INFINITY,
        };
        if needs_sending(&state, self.island_sent.as_ref(), now, resend) {
            self.effects.push(Effect::Island(state.clone()));
            self.island_sent = Some((state, now));
        }
    }

    /// What the island shows (`island`), or the widget.
    fn build_state(&self, now: f64, island: bool) -> IslandState {
        // A finished agent's row on the island (B707): its finish is news
        // for a moment, then the row goes — at once when its pane is on
        // screen. A stamped pane kept its "done" for as long as the polls
        // read the stamp, and the stamp stays until the next turn: the
        // island never ended while any agent had finished, up to iOS's
        // eight hours. The heuristic's rows already went after the same
        // linger. The widget keeps them (it keeps nothing alive), and so
        // does Home, which reads the stamps itself.
        let linger = self.tunables.done_linger;
        let on_screen = |key: &Key| {
            self.on_screen
                .as_ref()
                .is_some_and(|v| v.conn == key.0 && v.pane.as_deref().is_none_or(|pane| pane == key.1))
        };
        let shows = |key: &Key, p: &Pane| {
            p.visible && !(island && p.state == AgentState::Done && (now - p.since > linger || on_screen(key)))
        };
        let mut visible: Vec<(&Key, &Pane)> = self.panes.iter().filter(|(key, p)| shows(key, p)).collect();
        let paused = |key: &Key| self.down.contains_key(&key.0);
        let paused_at = |key: &Key| self.down.get(&key.0).copied();
        // Paused rows (a dropped connection's, B662) after the live ones.
        visible.sort_by(|a, b| {
            paused(a.0)
                .cmp(&paused(b.0))
                .then(a.1.state.rank().cmp(&b.1.state.rank()))
                .then(b.1.since.partial_cmp(&a.1.since).unwrap_or(std::cmp::Ordering::Equal))
        });
        let agents: Vec<Agent> = visible
            .iter()
            .take(self.tunables.max_agents)
            .map(|(key, p)| Agent {
                id: format!("{}:{}", key.0, key.1),
                connection_id: key.0.clone(),
                pane_id: key.1.clone(),
                command: display_command(&p.command).to_owned(),
                location: self.location(key, &p.place),
                place: self.named_place(key, &p.place),
                detail: if self.settings.show_detail {
                    p.detail.clone().filter(|d| !names_only_the_agent(d, &p.command))
                } else {
                    None
                },
                state: p.state,
                started_at: p.since,
                paused_at: paused_at(key),
            })
            .collect();
        let agents = tell_apart(agents, &visible);
        let headline_id = if agents.is_empty() {
            None
        } else {
            Some(agents[self.headline_offset % agents.len()].id.clone())
        };
        IslandState {
            working_count: visible
                .iter()
                .filter(|(key, p)| !paused(key) && p.state == AgentState::Working)
                .count(),
            attention_count: visible
                .iter()
                .filter(|(key, p)| !paused(key) && p.state == AgentState::Attention)
                .count(),
            agents,
            headline_id,
        }
    }

    /// "host · session · window", the island's line.
    fn location(&self, key: &Key, place: &PaneWhere) -> String {
        let host = self.conns.get(&key.0).cloned().unwrap_or_default();
        let parts: Vec<&str> = [place.session.as_deref(), place.window.as_deref()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            host
        } else {
            format!("{host} · {}", parts.join(" · "))
        }
    }

    /// "name · host", the island's place: Home's name for the pane (from
    /// the tree when it has one) and the host; the host alone without.
    fn named_place(&self, key: &Key, place: &PaneWhere) -> String {
        let host = self.conns.get(&key.0).cloned().unwrap_or_default();
        let name = place.name.as_deref().or_else(|| {
            self.trees
                .get(&key.0)
                .and_then(|tree| tree.get(&key.1))
                .and_then(|p| p.name.as_deref())
        });
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) if !host.is_empty() => format!("{name} · {host}"),
            Some(name) => name.to_owned(),
            None => host,
        }
    }

    /// "label · session", a card's line: the session from the tree, or
    /// from what the pane's own signals said.
    fn place(&self, key: &Key) -> String {
        let label = self.conns.get(&key.0).cloned().unwrap_or_default();
        let session = self
            .trees
            .get(&key.0)
            .and_then(|tree| tree.get(&key.1))
            .map(|place| place.session.as_deref())
            .unwrap_or_else(|| self.panes.get(key).and_then(|p| p.place.session.as_deref()));
        match session {
            Some(session) => copy::place(&label, Some(session)),
            None => label,
        }
    }

    fn save_announced(&mut self) {
        let flat = self
            .announced
            .iter()
            .map(|(key, at)| (format!("{}|{}", key.0, key.1), *at))
            .collect();
        self.effects.push(Effect::SaveAnnounced(flat));
    }
}

fn hook_governs(p: &Pane, now: f64) -> bool {
    p.hook_owned && now - p.last_hook_seen <= p.hold
}

/// How long a stamp holds when its connection's stamps arrive every
/// `interval` seconds: the interval, and a margin for a late poll that
/// grows with it — `margin`, or half the interval once that is more
/// (B346). tmux's 2 s: 5 s; herdr's 8 s: 12 s; herdr's 45 s with push:
/// 67.5 s, room for a safety-net poll that is late on a slow link.
pub fn stamp_hold(interval: f64, margin: f64) -> f64 {
    let interval = interval.max(0.0);
    interval + margin.max(interval / 2.0)
}

/// A login shell's leading "-" off.
/// Whether a hook's title only names the agent ("Claude Code" for claude,
/// "Codex CLI"): no news, so the island and the cards leave it out.
pub fn names_only_the_agent(detail: &str, command: &str) -> bool {
    let squeeze = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let detail = squeeze(detail);
    let agent = squeeze(display_command(command));
    !agent.is_empty()
        && [agent.clone(), format!("{agent}code"), format!("{agent}cli")].contains(&detail)
}

/// A hook's title as words to show (the island, the lock screen, the
/// cards): the XML-like wrappers an agent's message can come in
/// (`<agent-message from="…">…</agent-message>`, also with backslashes,
/// `<\agent-message …>`, `<\/agent-message>`) taken off with their
/// attributes, nested ones too; runs of whitespace fold to one space. A
/// `<` that opens no tag stays ("a < b", "x<5"), and so does a lone
/// `<Name>` with nothing to wrap ("Vec<String>"): a tag goes when it has
/// attributes, a hyphen or colon in its name, or its pair.
pub fn plain_detail(text: &str) -> String {
    struct Tag {
        start: usize,
        end: usize,
        name: String,
        closing: bool,
        marked: bool,
    }
    let bytes = text.as_bytes();
    let mut tags: Vec<Tag> = Vec::new();
    let mut at = 0;
    while let Some(offset) = text[at..].find('<') {
        let start = at + offset;
        let mut i = start + 1;
        // `<\name`, `<\/name`, `</name`.
        if bytes.get(i) == Some(&b'\\') {
            i += 1;
        }
        let closing = bytes.get(i) == Some(&b'/');
        if closing {
            i += 1;
        }
        let name_start = i;
        if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
            at = start + 1;
            continue;
        }
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        {
            i += 1;
        }
        let name = text[name_start..i].to_owned();
        // Its attributes run to the next `>`, with no `<` before it.
        let Some(close) = text[i..].find('>') else {
            at = start + 1;
            continue;
        };
        let inside = &text[i..i + close];
        if inside.contains('<') || (!inside.is_empty() && !inside.starts_with(char::is_whitespace) && inside != "/") {
            at = start + 1;
            continue;
        }
        let attributes = !inside.trim().trim_end_matches('/').trim().is_empty();
        let end = i + close + 1;
        tags.push(Tag {
            start,
            end,
            marked: attributes || name.contains(['-', ':']),
            name,
            closing,
        });
        at = end;
    }
    // A tag with its pair goes too.
    for a in 0..tags.len() {
        if tags[a].marked {
            continue;
        }
        let paired = tags.iter().enumerate().any(|(b, other)| {
            b != a && other.name == tags[a].name && other.closing != tags[a].closing
        });
        if paired {
            tags[a].marked = true;
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for tag in tags.iter().filter(|t| t.marked) {
        out.push_str(&text[from..tag.start]);
        out.push(' ');
        from = tag.end;
    }
    out.push_str(&text[from..]);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rows that would read the same ("claude · x · mac" twice: two panes in
/// one workspace) get what tells them apart (B596): the window's index
/// when their windows differ, else the pane's ("x · mac · 2").
fn tell_apart(mut agents: Vec<Agent>, panes: &[(&Key, &Pane)]) -> Vec<Agent> {
    let index = |id: &str| panes.iter().find(|(key, _)| format!("{}:{}", key.0, key.1) == id).map(|(_, p)| &p.place);
    let reads = |a: &Agent| (a.command.clone(), a.place.clone());
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, agent) in agents.iter().enumerate() {
        groups.entry(reads(agent)).or_default().push(i);
    }
    for rows in groups.values().filter(|rows| rows.len() > 1) {
        let places: Vec<Option<&PaneWhere>> = rows.iter().map(|i| index(&agents[*i].id)).collect();
        let windows: BTreeSet<Option<i64>> = places.iter().map(|p| p.and_then(|p| p.window_index)).collect();
        let by_window = windows.len() == rows.len() && !windows.contains(&None);
        for (i, place) in rows.iter().zip(places) {
            let pane = &agents[*i].pane_id;
            let fallback = || pane.trim_start_matches(|c: char| !c.is_ascii_digit()).to_owned();
            let mark = match place {
                Some(p) if by_window => p.window_index.map(|w| w.to_string()),
                Some(p) => match (windows.len() == 1, p.window_index, p.pane_index) {
                    (true, _, Some(n)) => Some(n.to_string()),
                    (false, Some(w), Some(n)) => Some(format!("{w}.{n}")),
                    _ => None,
                },
                None => None,
            }
            .unwrap_or_else(fallback);
            if !mark.is_empty() {
                agents[*i].place = format!("{} · {mark}", agents[*i].place);
            }
        }
    }
    agents
}

pub fn display_command(command: &str) -> &str {
    command.strip_prefix('-').unwrap_or(command)
}

/// Whether a pane in `state`, its episode begun at `episode`, should be
/// announced given what was last announced for it.
pub fn should_announce(state: AgentState, episode: f64, last_announced: Option<f64>) -> bool {
    state == AgentState::Attention && last_announced != Some(episode)
}

/// A stamp the host got wrong, or that time made wrong: a shell in the
/// foreground means the agent is gone (done); Claude's idle reminder is a
/// parked agent, not a question.
pub fn reclassify(state: AgentState, title: Option<&str>, foreground: Option<&str>) -> AgentState {
    if foreground.is_some_and(is_shell) {
        return AgentState::Done;
    }
    if state == AgentState::Attention
        && title.is_some_and(|title| title.to_lowercase().contains("waiting for your input"))
    {
        return AgentState::Done;
    }
    state
}

/// Whether a poll is a turn ending: the host says done now and it shows as
/// done; it said working or attention last poll; and that was recent.
pub fn is_turn_ending(
    hook: AgentState,
    shown: AgentState,
    previous_hook: Option<AgentState>,
    previous_hook_seen: Option<f64>,
    now: f64,
    grace: f64,
) -> bool {
    hook == AgentState::Done
        && shown == AgentState::Done
        && matches!(previous_hook, Some(AgentState::Working | AgentState::Attention))
        && previous_hook_seen.is_some_and(|seen| now - seen <= grace)
}

/// Whether `state` goes out, given what was last sent there and when — and
/// again when the clock was set back past that send, rather than not until
/// it is a minute past it again.
/// Whether an unchanged island is sent again when due (B701): one showing
/// a live row, to keep its stale date ahead. Paused rows (a dropped link,
/// B662) can't go out of date: sent once, then only when they change or
/// the link comes back.
pub fn keeps_fresh(state: &IslandState) -> bool {
    state.agents.iter().any(|agent| agent.paused_at.is_none())
}

pub fn needs_sending(state: &IslandState, last: Option<&(IslandState, f64)>, now: f64, resend_after: f64) -> bool {
    match last {
        None => true,
        Some((sent, at)) => sent != state || now - at >= resend_after || now < *at,
    }
}

/// The announced-episode record as stored, without what is older than
/// `ttl`. An episode is dated by the host's clock (its `@moshpit_since`),
/// so the phone's can't say one is "from the future" (B823): a host ahead
/// of the phone by more than a minute had every entry dropped as such at
/// launch, and each question still waiting was announced, and rang, again.
pub fn decode_announced(raw: &BTreeMap<String, f64>, now: f64, ttl: f64) -> BTreeMap<String, f64> {
    raw.iter()
        .filter(|(_, at)| now - **at <= ttl)
        .map(|(key, at)| (key.clone(), *at))
        .collect()
}

/// The waiting card, drawn from the newest standing entry. The entry may
/// be the notification service's, which records what the push said: the
/// detail is held to the switch here, and `fallback` stands in for a place
/// the entry doesn't carry. `name` (the connection's) names an entry with
/// no agent, as the notification service names a push with none (the
/// card read "agent needs you" in English, whatever the language), and the
/// place when there is no name either.
pub fn digest(newest: &Entry, count: usize, settings: Settings, edge: bool, fallback: &str, name: &str) -> CardText {
    let named = if name.trim().is_empty() { fallback } else { name };
    let who = newest.agent.as_deref().filter(|agent| !agent.is_empty()).unwrap_or(named);
    let stored = newest.location.as_deref().map(str::trim).unwrap_or("");
    copy::attention(
        who,
        count as i64,
        if settings.show_detail { newest.title.as_deref() } else { None },
        if stored.is_empty() { fallback } else { stored },
        edge,
        settings.sound,
        settings.language,
    )
}

#[doc(hidden)]
pub mod harness;

#[cfg(test)]
mod tests;
