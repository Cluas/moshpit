//! The mosh sidecar: a control-plane-only controller
//! (`renders_output == false`). mosh renders tmux's own TUI through a regular
//! tmux client — the phone's renderer — and this `-CC` client only tracks
//! sessions, windows and panes for the breadcrumb and the sheets, and sizes
//! the renderer's window to the phone.
//!
//! Which session the phone shows is the renderer's, learned from rows that
//! name it by pid AND name, never from `#{session_attached}` (that counts
//! every client, the desktop's and this sidecar's own). The renderer is
//! identified by the hub's attach check: the ONE non-control client that
//! appears between two counts. Only it moves on a session switch; no other
//! client — a desktop's terminal first of all — is ever touched.

use std::collections::BTreeSet;

use crate::clients::{CLIENT_ROW_FORMAT, ClientIdentity, ClientRow};
use crate::control::CommandResponse;
use crate::format::is_valid_id;

use super::timers::{Action, Slot};
use super::{Controller, Notification, Reply};

#[derive(Debug, Default)]
pub(super) struct SidecarState {
    /// The phone's own renderer, once identified.
    pub renderer_client: Option<ClientIdentity>,
    /// The renderer's pid, when the host told it outright (mosh's program is
    /// tmux's client, `exec`ed: mosh-server's own child): the client with
    /// it is the renderer, whatever else attached meanwhile.
    pub renderer_pid: Option<i64>,
    /// The session the renderer sits on.
    pub renderer_session_id: Option<String>,
    /// Bumped when a switch starts and when it sends its moves: a listing
    /// sent before shows the renderer where the switch takes it away from.
    pub session_switches: u64,
    pub renderer_locate_in_flight: bool,
    /// Sessions Moshpit created (by id): no status line while the phone is
    /// on one.
    pub owned_session_ids: BTreeSet<String>,
    /// The one session whose status line this controller turned off.
    pub status_hidden_session_id: Option<String>,
    /// The client names listed just before the owned sessions are read.
    pub owned_listed_clients: Option<BTreeSet<String>>,
    /// B773: the renderer goes into our group once the group has landed
    /// where its session is.
    pub renderer_awaits_landing: bool,
    /// B812: the group the sidecar left, ended once the renderer has left
    /// it too.
    pub group_left: Option<String>,
    /// B773, B812: a group we don't hold the renderer was found on (the
    /// last sidecar's, or one an older build made), ended once the renderer
    /// has left it.
    pub renderer_group: Option<String>,
    /// B872, B871: ids of groups we left or ended. A listing written before
    /// the renderer moved still names one as the renderer's session; taken
    /// for the session the person went to, it got a group of a dead group,
    /// and so on without end. Ids are never reused, so rows naming them are
    /// stale for good.
    pub dead_groups: BTreeSet<String>,
    /// B874: the session the renderer just left, held until
    /// [`Controller::bring_renderer_into_group`] moves the renderer into the
    /// new group so tmux doesn't re-squash the left session while the
    /// phone's renderer still sits on it.
    pub pending_hand_back: Option<(String, Option<String>)>,
}

/// The session option that marks a session Moshpit made (B86): the mark
/// lives on the server, so a sidecar that never saw the session made — the
/// app killed and opened again — still knows it.
pub const OWNED_OPTION: &str = "@moshpit_owned";

/// The session option that says which control client turned that session's
/// status line off (its `#{client_name}`): one no longer attached left it
/// off when it died, and the next sidecar gives it back (B86).
pub const STATUS_HIDDEN_OPTION: &str = "@moshpit_status_hidden";

impl Controller {
    /// The phone's mosh renderer, once identified.
    pub fn renderer_client(&self) -> Option<&ClientIdentity> {
        self.sidecar.renderer_client.as_ref()
    }

    /// The session the renderer sits on.
    pub fn renderer_session_id(&self) -> Option<&str> {
        self.sidecar.renderer_session_id.as_deref()
    }

    /// The sessions Moshpit created.
    pub fn owned_session_ids(&self) -> &BTreeSet<String> {
        &self.sidecar.owned_session_ids
    }

    /// Count the clients attached WITHOUT control mode (renderers, desktop
    /// terminals), answered with [`Notification::ClientCount`] (-1: not
    /// attached, refused, or the channel closed first). A count never says
    /// which client is the phone's renderer (B84: the one client that
    /// appeared between two counts was taken for it, and a desktop attaching
    /// in the same second was): the renderer is known by its pid
    /// ([`Self::expect_renderer_pid`]), as the app knows it. Nothing in the
    /// product counts any more; the harness's scripts and the tests do.
    pub fn count_non_control_clients(&mut self, now: u64) -> u64 {
        self.begin_input(now);
        let request = self.request_id();
        if self.snapshot.is_attached {
            let switches = self.sidecar.session_switches;
            self.enqueue(
                format!("list-clients -F '{CLIENT_ROW_FORMAT}'"),
                Reply::ClientCount { request, switches },
                true,
            );
        } else {
            self.notify(Notification::ClientCount { request, count: -1 });
        }
        self.finish_input();
        request
    }

    pub(super) fn client_count_answered(
        &mut self,
        request: u64,
        switches: u64,
        response: &CommandResponse,
    ) {
        if response.is_error {
            self.notify(Notification::ClientCount { request, count: -1 });
            return;
        }
        let clients: Vec<ClientRow> = response
            .lines
            .iter()
            .filter_map(|line| ClientRow::parse(line))
            .filter(|row| !row.is_control)
            .collect();
        // The rows still say where an already known renderer sits.
        self.note_renderer_rows(&clients, switches);
        self.notify(Notification::ClientCount {
            request,
            count: clients.len() as i64,
        });
    }

    /// The renderer is the non-control client with this pid: mosh's own
    /// program, known by its parent rather than by counting clients around
    /// an attach (a desktop attaching in the same second was taken for the
    /// renderer that way, B84). Looked for now when attached, else at the
    /// attach.
    pub fn expect_renderer_pid(&mut self, pid: i64, now: u64) {
        self.begin_input(now);
        if !self.config.renders_output && pid > 0 {
            if self
                .sidecar
                .renderer_client
                .as_ref()
                .is_some_and(|client| client.pid != pid)
            {
                // A different renderer (a new mosh session): forget the old.
                self.sidecar.renderer_client = None;
                self.sidecar.renderer_session_id = None;
            }
            self.sidecar.renderer_pid = Some(pid);
            self.locate_renderer();
        }
        self.finish_input();
    }

    /// Ask tmux where the renderer is now and follow it (it moves on its
    /// own too: `prefix s` in the mosh TUI); then decide the status line.
    pub(super) fn locate_renderer(&mut self) {
        if self.config.renders_output
            || !self.snapshot.is_attached
            || (self.sidecar.renderer_client.is_none() && self.sidecar.renderer_pid.is_none())
        {
            self.sync_status_line();
            return;
        }
        if self.sidecar.renderer_locate_in_flight {
            return;
        }
        self.sidecar.renderer_locate_in_flight = true;
        let switches = self.sidecar.session_switches;
        self.send_command(
            format!("list-clients -F '{CLIENT_ROW_FORMAT}'"),
            Reply::LocateRenderer { switches },
        );
    }

    pub(super) fn locate_renderer_answered(&mut self, switches: u64, response: &CommandResponse) {
        self.sidecar.renderer_locate_in_flight = false;
        if !response.is_error {
            let rows: Vec<ClientRow> = response
                .lines
                .iter()
                .filter_map(|l| ClientRow::parse(l))
                .collect();
            self.note_renderer_rows(&rows, switches);
        }
        self.sync_status_line();
    }

    /// Follow the renderer to its session, or forget the session when it is
    /// not among the rows (detached, its mosh gone).
    fn note_renderer_rows(&mut self, rows: &[ClientRow], switches: u64) {
        if self.config.renders_output {
            return;
        }
        if self.sidecar.renderer_client.is_none()
            && let Some(pid) = self.sidecar.renderer_pid
        {
            self.sidecar.renderer_client = rows
                .iter()
                .filter(|row| !row.is_control && row.pid == Some(pid))
                .find_map(ClientRow::identity);
        }
        let Some(renderer) = self.sidecar.renderer_client.clone() else {
            return;
        };
        let Some(row) = rows
            .iter()
            .find(|row| !row.is_control && row.identity().as_ref() == Some(&renderer))
        else {
            self.sidecar.renderer_session_id = None;
            return;
        };
        // B872: listed on a group we left or ended: the renderer is on its way.
        if self.sidecar.dead_groups.contains(&row.session_id) {
            return;
        }
        // Listed before a switch we started: that switch is moving it.
        if switches != self.sidecar.session_switches {
            return;
        }
        let session = row.session_id.clone();
        self.follow_renderer(&session);
    }

    /// The sidecar takes the renderer's session as its own, and moves its
    /// own client there (tmux reports layout changes only for a control
    /// client's own session).
    fn follow_renderer(&mut self, session: &str) {
        if self.config.renders_output || !is_valid_id(session) {
            return;
        }
        // B773: in our group, the renderer views the group's session.
        if let Some(group) = self.group.current.as_ref()
            && group.id.as_deref() == Some(session)
        {
            self.sidecar.renderer_session_id = Some(group.session.clone());
            return;
        }
        // B773, B812: on a group we don't hold — the last sidecar's (its
        // SSH leg back, mosh never gone), or one an older build made — the
        // renderer views the session that group is of. Taken for a session,
        // it got a group of the group, and the phone showed the group.
        if let Some(of) = self.group.marked.get(session).cloned()
            && of != session
        {
            self.follow_renderer_out_of(session, &of);
            return;
        }
        self.sidecar.renderer_session_id = Some(session.to_owned());
        if self.snapshot.active_session_id.as_deref() == Some(session) {
            // We view it already, through our group: the renderer comes in.
            self.bring_renderer_into_group();
            return;
        }
        let window = self
            .snapshot
            .windows_in_id_order()
            .into_iter()
            .find(|window| window.session_id == session && window.is_active)
            .map(|window| window.id.clone());
        let pane = window.as_ref().and_then(|window| {
            self.snapshot
                .panes_in_id_order()
                .into_iter()
                .find(|pane| &pane.window_id == window && pane.is_active)
                .map(|pane| pane.id.clone())
        });
        let snapshot = self.snap();
        snapshot.active_session_id = Some(session.to_owned());
        snapshot.active_window_id = window;
        snapshot.active_pane_id = pane;
        self.sync_status_line();
        if self.wants_group_for(session) {
            // B773: into a group of it, the renderer after us.
            let name = self
                .snapshot
                .sessions
                .get(session)
                .map(|info| info.name.clone())
                .unwrap_or_default();
            self.enter_group(session, &name);
        } else {
            self.switch_without_group(session);
        }
        self.refresh_windows_and_panes(false, None);
    }

    /// B773: the phone's renderer into our group, so the session's own
    /// current window — a desktop's — never moves with the phone's. Not
    /// before the group stands where the session is: a group starts on its
    /// lowest window, and the phone would show that for a moment.
    pub(super) fn bring_renderer_into_group(&mut self) {
        if self.config.renders_output {
            return;
        }
        let Some(group) = self.group.current.clone() else {
            return;
        };
        if self.group.landing {
            self.sidecar.renderer_awaits_landing = true;
            return;
        }
        self.sidecar.renderer_awaits_landing = false;
        let Some(renderer) = self.sidecar.renderer_client.clone() else {
            self.flush_pending_hand_back(None);
            return;
        };
        if self.sidecar.renderer_session_id.as_deref() != Some(group.session.as_str()) {
            if self.sidecar.renderer_session_id.is_none() {
                self.flush_pending_hand_back(None);
            }
            return;
        }
        let target = match &group.id {
            Some(id) => format!("'{id}'"),
            None => format!("'={}'", group.name),
        };
        // B871: the move into the group is what sizes the session by the
        // renderer, and it comes after the switch's own probe (the group
        // lands first): read the layouts around this move too, else a window
        // it squashed has no record to be handed back from.
        self.probe_switch_layouts(&group.session);
        self.send(format!("switch-client -c {} -t {target}", renderer.name));
        // The move is what puts the group on the phone's screen: its status
        // line goes off with it, whatever the sidecar believes it hid before
        // (a belief carried over from an earlier sidecar, or one set while
        // the group's id was still unknown, never reached the screen).
        if let Some(id) = &group.id {
            self.assert_status_off(id);
        }
        // B812: the group both have now left, behind the move.
        if let Some(left) = self.sidecar.group_left.take() {
            self.send(format!("kill-session -t '{left}'"));
        }
        self.end_renderer_group();
        self.flush_pending_hand_back(Some(&group.session));
    }

    pub(super) fn flush_pending_hand_back(&mut self, current_session: Option<&str>) {
        if let Some((left, left_window)) = self.sidecar.pending_hand_back.take()
            && current_session != Some(left.as_str())
        {
            self.hand_back_left_session(&left, left_window.as_deref());
        }
    }

    /// The renderer on `group`, a group of `session` we don't hold: it
    /// comes into our group of `session`, or — none to come (tmux before
    /// 3.8, B812) — onto `session` itself; `group` ends behind it.
    fn follow_renderer_out_of(&mut self, group: &str, session: &str) {
        if self.group.marked.contains_key(session) {
            // A group of a group: marks no build of ours sets. Not chased.
            return;
        }
        self.sidecar.renderer_group = Some(group.to_owned());
        if !self.group_will_take_renderer(session)
            && let Some(renderer) = self.sidecar.renderer_client.clone()
        {
            self.send(format!("switch-client -c {} -t {session}", renderer.name));
            self.end_renderer_group();
        }
        self.follow_renderer(session);
    }

    /// Whether a group of ours on `session` will take the renderer in: ours
    /// already, the attach's check still deciding, or one made as the
    /// sidecar follows it there.
    fn group_will_take_renderer(&self, session: &str) -> bool {
        let ours = self
            .group
            .current
            .as_ref()
            .is_some_and(|group| group.session == session);
        self.group.supported == Some(true)
            && (ours
                || (self.group_possible(session)
                    && (self.group.deciding
                        || self.snapshot.active_session_id.as_deref() != Some(session))))
    }

    /// The group the renderer was found on, now it has left: ended unless
    /// a client is still on it — another phone's, picked in mosh's own
    /// session list. A listing sent before shows the renderer still there.
    fn end_renderer_group(&mut self) {
        let Some(group) = self.sidecar.renderer_group.take() else {
            return;
        };
        self.group.marked.remove(&group);
        self.sidecar.dead_groups.insert(group.clone());
        self.sidecar.session_switches = self.sidecar.session_switches.wrapping_add(1);
        self.send(format!(
            "if-shell -F -t '{group}' '#{{==:#{{session_attached}},0}}' \"kill-session -t '{group}'\""
        ));
    }

    /// The window the sidecar may size: the active one, when it is on the
    /// renderer's session.
    pub(super) fn renderer_window_id(&self) -> Option<String> {
        let session = self.sidecar.renderer_session_id.as_deref()?;
        if self.snapshot.active_session_id.as_deref() != Some(session) {
            return None;
        }
        let window = self.snapshot.active_window_id.as_deref()?;
        (self.snapshot.windows.get(window)?.session_id == session).then(|| window.to_owned())
    }

    /// Size the renderer's window to the mosh grid (`syncMoshWindow`): the
    /// pin. Nothing at all while the renderer is unknown, detached or
    /// elsewhere — the window would be somebody else's.
    ///
    /// This client reports no size of its own (B270). It used to report the
    /// mosh grid first, against its 80×24 PTY dragging tmux's sizing down —
    /// but a control client that never reported a size is no part of tmux's
    /// sizing (3.6a: a `-CC` client on an 80×24 PTY leaves a 200×60 window
    /// alone), and one that did is: with the phone tmux's only client, the
    /// windows handed back on the disconnect went straight back to the
    /// phone's grid with it.
    pub fn sync_mosh_window(&mut self, cols: i64, rows: i64, now: u64) {
        self.begin_input(now);
        if !self.config.renders_output && self.snapshot.is_attached {
            self.last_client_size = (cols, rows);
            if let Some(window) = self.renderer_window_id() {
                self.capture_pristine_layout(&window);
                self.send(format!("resize-window -t {window} -x {cols} -y {rows}"));
                self.size.resized_windows.insert(window);
            }
        }
        self.finish_input();
    }

    /// The sidecar's session switch: `switch-client` alone would move only
    /// this invisible client. The renderer comes along — and nothing else.
    /// `left`/`left_window`: where the renderer was, handed back once it
    /// has gone.
    pub(super) fn sidecar_switch(
        &mut self,
        session: &str,
        left: Option<String>,
        left_window: Option<String>,
    ) {
        self.send_command(
            format!("list-clients -F '{CLIENT_ROW_FORMAT}'"),
            Reply::SidecarSwitch {
                session: session.to_owned(),
                left,
                left_window,
            },
        );
    }

    pub(super) fn sidecar_switch_answered(
        &mut self,
        session: &str,
        left: Option<String>,
        left_window: Option<String>,
        response: &CommandResponse,
    ) {
        let rows: Vec<ClientRow> = if response.is_error {
            Vec::new()
        } else {
            response
                .lines
                .iter()
                .filter_map(|l| ClientRow::parse(l))
                .collect()
        };
        let renderer = self.sidecar.renderer_client.clone();
        let moving: Vec<String> = match &renderer {
            Some(client) => rows
                .iter()
                .filter(|row| !row.is_control && row.identity().as_ref() == Some(client))
                .map(|row| row.name.clone())
                .collect(),
            None => Vec::new(),
        };
        // The renderer sizes the session it lands on (its latest client, the
        // only one when the phone is alone): what the move squashes is read
        // on both sides of it, as our own switch's is (B150).
        if !moving.is_empty() {
            self.probe_switch_layouts(session);
        }
        let already_in_group = self
            .group
            .current
            .as_ref()
            .is_some_and(|group| group.session == session);
        let grouped = self.wants_group_for(session);
        if !grouped && !already_in_group {
            for name in &moving {
                self.send(format!("switch-client -c {name} -t {session}"));
            }
        }
        if renderer.is_some() && !response.is_error {
            self.sidecar.renderer_session_id = (!moving.is_empty()).then(|| session.to_owned());
        }
        if grouped {
            // B773: into a group of it; the renderer follows at its landing.
            let name = self
                .snapshot
                .sessions
                .get(session)
                .map(|info| info.name.clone())
                .unwrap_or_default();
            self.enter_group(session, &name);
            if moving.is_empty() {
                self.sidecar.renderer_awaits_landing = false;
            }
        } else if already_in_group {
            // B874: another path (such as GroupCheckMarks) already created
            // this session's group before SidecarSwitch answered; keep it
            // and bring the renderer in now rather than switching out.
            if moving.is_empty() {
                self.sidecar.renderer_awaits_landing = false;
            } else {
                self.bring_renderer_into_group();
            }
        } else {
            self.switch_without_group(session);
        }
        self.sidecar.session_switches = self.sidecar.session_switches.wrapping_add(1);
        if !moving.is_empty() {
            if (grouped || already_in_group) && self.sidecar.renderer_awaits_landing {
                match &self.sidecar.pending_hand_back {
                    Some((orig, _)) if orig == session => {
                        self.sidecar.pending_hand_back = None;
                    }
                    Some(_) => {}
                    None => {
                        if let Some(left) = left.filter(|left| left != session) {
                            self.sidecar.pending_hand_back = Some((left, left_window));
                        }
                    }
                }
            } else {
                self.flush_pending_hand_back(Some(session));
                if let Some(left) = left.filter(|left| left != session) {
                    self.hand_back_left_session(&left, left_window.as_deref());
                }
            }
        }
    }

    /// The sessions Moshpit made, and the status lines a dead sidecar left
    /// off (B86), read from the server at the sidecar's attach: who is
    /// attached, then every session's marks — a FIFO pair, the clients'
    /// reply first.
    ///
    /// The sessions Moshpit created were known only in memory (and the
    /// ledger, across a rebuilt sidecar): an app killed while the phone sat
    /// on one left its status line off, and the next launch never knew the
    /// session for Moshpit's — the status line stayed off for good, the
    /// desktop's with it.
    pub(super) fn read_owned_sessions(&mut self) {
        self.send_command(
            "list-clients -F '#{client_name}'".to_owned(),
            Reply::OwnedClients,
        );
        self.send_command(
            format!(
                "list-sessions -F '#{{session_id}} owned=#{{{OWNED_OPTION}}} hidden=#{{{STATUS_HIDDEN_OPTION}}}'"
            ),
            Reply::OwnedSessions,
        );
    }

    pub(super) fn owned_clients_answered(&mut self, response: &CommandResponse) {
        self.sidecar.owned_listed_clients = (!response.is_error).then(|| {
            response
                .lines
                .iter()
                .map(|line| moshpit_herdr::text::trim_whitespaces(line).to_owned())
                .filter(|name| !name.is_empty())
                .collect()
        });
    }

    /// Take the marked sessions as Moshpit's; give a status line back that a
    /// client no longer attached turned off — unless the phone is on that
    /// session, where it stays off, ours now. Nothing without the clients'
    /// listing (a stamp can't be judged dead without it).
    pub(super) fn owned_sessions_answered(&mut self, response: &CommandResponse) {
        let clients = self.sidecar.owned_listed_clients.take();
        if response.is_error {
            return;
        }
        let current = self.snapshot.active_session_id.clone();
        for line in &response.lines {
            let mut words = line.splitn(3, ' ');
            let (Some(id), Some(owned), Some(stamp)) = (
                words.next().filter(|id| id.starts_with('$') && is_valid_id(id)),
                words.next().and_then(|word| word.strip_prefix("owned=")),
                words.next().and_then(|word| word.strip_prefix("hidden=")),
            ) else {
                continue;
            };
            let owned = owned == "1";
            let stamp = moshpit_herdr::text::trim_whitespaces(stamp);
            if owned {
                self.sidecar.owned_session_ids.insert(id.to_owned());
            }
            let Some(clients) = &clients else {
                continue;
            };
            if stamp.is_empty() || clients.contains(stamp) {
                continue;
            }
            if current.as_deref() == Some(id) {
                // The phone is on it: it stays off, stamped ours below.
                if self.sidecar.status_hidden_session_id.as_deref() == Some(id) {
                    self.send(hide_stamp_command(id));
                }
                continue;
            }
            if self.sidecar.status_hidden_session_id.as_deref() == Some(id) {
                self.sidecar.status_hidden_session_id = None;
            }
            self.send(format!("set-option -t '{id}' -u status"));
            self.send(format!("set-option -t '{id}' -u {STATUS_HIDDEN_OPTION}"));
        }
        self.sync_status_line();
    }

    /// The status line over mosh: off on the session the phone is on (the
    /// user's own included), given back by `-u` when it leaves, on the
    /// disconnect, or when a dead sidecar's stamp is found.
    pub(super) fn sync_status_line(&mut self) {
        if self.config.renders_output || !self.snapshot.is_attached {
            return;
        }
        let Some(current) = self.snapshot.active_session_id.clone() else {
            return;
        };
        // B773: the renderer views our group of it: the group's own status
        // line is the one on the phone's screen.
        let shown = self
            .group
            .current
            .as_ref()
            .filter(|group| group.session == current)
            .and_then(|group| group.id.clone())
            .unwrap_or_else(|| current.clone());
        // 1.0.8: mosh draws tmux's whole TUI, so Moshpit's own breadcrumbs
        // replace its status line on whatever session the phone is on — the
        // user's too. `-u` hands it back (the stamp says it was us), and
        // nothing is ever written to the user's config.
        let wanted = Some(shown);
        if wanted == self.sidecar.status_hidden_session_id {
            return;
        }
        if let Some(hidden) = self.sidecar.status_hidden_session_id.clone() {
            self.send(format!("set-option -t '{hidden}' -u status"));
            self.send(format!("set-option -t '{hidden}' -u {STATUS_HIDDEN_OPTION}"));
        }
        if let Some(wanted) = &wanted {
            self.send(format!("set-option -t '{wanted}' status off"));
            self.send(hide_stamp_command(wanted));
        }
        self.sidecar.status_hidden_session_id = wanted;
    }

    /// `status off` on `id` and the stamp, again even when the sidecar
    /// already counts it hidden (`set-option` is idempotent); what was hidden
    /// before, if elsewhere, is given back.
    fn assert_status_off(&mut self, id: &str) {
        // Not gated on the snapshot's attach: the move itself is the proof
        // the sidecar is up, and a snapshot not yet caught up must not keep
        // the line on.
        if self.config.renders_output {
            return;
        }
        if let Some(hidden) = self.sidecar.status_hidden_session_id.clone()
            && hidden != id
        {
            self.send(format!("set-option -t '{hidden}' -u status"));
            self.send(format!("set-option -t '{hidden}' -u {STATUS_HIDDEN_OPTION}"));
        }
        self.send(format!("set-option -t '{id}' status off"));
        self.send(hide_stamp_command(id));
        self.sidecar.status_hidden_session_id = Some(id.to_owned());
    }

    /// Give a hidden status line back over the live channel, then wait for
    /// tmux to have run it (`restoreSuppressedStatusAndFlush`): answered with
    /// [`Notification::Flushed`] (the reply, or 2 s). `None`: nothing owed,
    /// nothing to wait for.
    pub fn restore_suppressed_status(&mut self, now: u64) -> Option<u64> {
        self.begin_input(now);
        let mut request = None;
        if self.snapshot.is_attached
            && let Some(hidden) = self.sidecar.status_hidden_session_id.take()
        {
            self.send(format!("set-option -t '{hidden}' -u status"));
            self.send(format!("set-option -t '{hidden}' -u {STATUS_HIDDEN_OPTION}"));
            request = Some(self.flush_control_channel(2000));
        }
        self.finish_input();
        request
    }

    /// One round trip: once it is answered, everything sent before it has
    /// run. Bounded so a dying channel can't hang a teardown.
    fn flush_control_channel(&mut self, bound_ms: u64) -> u64 {
        let request = self.request_id();
        self.waiting_requests.insert(request);
        self.enqueue(
            "display-message -p ok".to_owned(),
            Reply::Flush { request },
            true,
        );
        if self.waiting_requests.contains(&request) {
            self.arm(
                Slot::Flush(request),
                bound_ms,
                Action::FlushTimeout(request),
            );
        }
        request
    }

    pub(super) fn flush_done(&mut self, request: u64) {
        if self.waiting_requests.remove(&request) {
            self.timers.cancel(&Slot::Flush(request));
            self.notify(Notification::Flushed { request });
        }
    }

    /// `%client-session-changed`: the renderer moving by itself takes the
    /// sidecar along.
    pub(super) fn client_session_changed(&mut self, client: &str) {
        if !self.config.renders_output
            && self
                .sidecar
                .renderer_client
                .as_ref()
                .is_some_and(|renderer| renderer.name == client)
        {
            self.locate_renderer();
        }
    }

    /// `%client-detached`: a server-wide broadcast about SOME client — never
    /// ours (that ends as `%exit`). Only the renderer leaving matters: no
    /// window is left that the sidecar may size.
    pub(super) fn client_detached(&mut self, name: &str) {
        if !self.config.renders_output
            && self
                .sidecar
                .renderer_client
                .as_ref()
                .is_some_and(|renderer| renderer.name == name)
            && self.sidecar.renderer_session_id.is_some()
        {
            self.sidecar.renderer_session_id = None;
        }
    }
}

/// Stamps `session`'s hidden status line with this control client's own
/// name (`-F`: tmux expands it for the client that runs the command).
fn hide_stamp_command(session: &str) -> String {
    format!("set-option -t '{session}' -F {STATUS_HIDDEN_OPTION} '#{{client_name}}'")
}
