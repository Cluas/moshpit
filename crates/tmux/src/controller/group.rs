//! The phone's own view of a session (B459): a tmux grouped session.
//!
//! A session's current window belongs to the session, and every client on
//! it shows that window: the phone switching windows switched every desktop
//! on the same session with it (tmux's own semantics, kept until 11-14).
//! The phone now attaches through a session of its own in the session's
//! group — `new-session -t '$S'` — which shares S's windows (their panes,
//! their processes) but has its own current window. The person never sees
//! it: the snapshot, the lists and the breadcrumb name S, and a desktop's
//! `tmux ls` shows it as `<S>~mp-<device>-<tag>`.
//!
//! Its life:
//!
//! - Only on tmux 3.8 and later (B812). Every earlier version (3.2a–3.7,
//!   all tried on private servers) can be crashed — the whole server, every
//!   session on the host — with a group in place: a group whose current
//!   window index a manual `move-window -r` left behind goes NULL on the
//!   next close (`server_client_loop`, `curw->window` at 0x10), whatever
//!   the group's options. The attach reads `#{version}` first; below 3.8,
//!   or unread, the phone stays on S as before groups (B457 guards the
//!   restore).
//! - Made from this `-CC` client without `-d`: tmux makes it and moves the
//!   client into it in one step, and every command after it runs there — a
//!   restore's `select-window` included. Then the mark `@moshpit_group_of`
//!   naming S.
//! - Never `destroy-unattached` (B812): tmux reaps such a session without
//!   detaching a client still pointing at it — a phone's control client
//!   the server marked exiting ("too far behind": a phone in the
//!   background, its connection stalled) — and that client's next redraw
//!   check crashed the server (3.2a–3.7). The phone ends its group itself:
//!   the old one right behind a switch to another, its own on the person's
//!   end of the session ([`Controller::group_release_command`]); what a
//!   dropped connection left, the next attach sweeps (a marked group no
//!   client is on). `kill-session` detaches whatever still points at it.
//! - Groups an earlier version of the app left with `destroy-unattached on`
//!   lose it at the sweep.
//! - S killed under it (a desktop's `kill-session`): the windows would live
//!   on in the group alone; ours goes too, as the desktop meant.
//!
//! What it does not change: a window's active pane and its zoom are the
//! window's own in tmux, shared by every session holding it, so a pane
//! picked in a split window a desktop also shows still moves its active
//! pane. The reconnect restore keeps B457's check for that part.
//!
//! A tmux that refuses the group (or a reply without its id) leaves the
//! phone on S itself, as before, B457 guarding the restore.
//!
//! So does a control client on S that isn't Moshpit's (iTerm2's tmux
//! integration): when a group goes, tmux tells every control client on S
//! that each of its windows closed — they live on in S, but such a client
//! may close its tabs for them. The attach asks first who is on S, and which
//! control clients are Moshpit's: each Moshpit `-CC` client (a phone's, a
//! mosh sidecar) keeps its name in the server option `@moshpit_cc`.

use std::collections::{BTreeMap, BTreeSet};

use crate::control::CommandResponse;
use crate::format::is_valid_id;

use super::{Controller, Reply};

/// What every phone's group carries in its name, after S's.
pub const GROUP_MARK: &str = "~mp-";

/// The group's session option naming the session it views (`$N`): only a
/// session with it is ever swept.
pub const GROUP_OF_OPTION: &str = "@moshpit_group_of";

/// The server option listing Moshpit's own control clients by name: a
/// control client on S not in it is a desktop's.
pub const CLIENTS_OPTION: &str = "@moshpit_cc";

/// The first tmux whose grouped sessions can't crash the server (B812).
pub const GROUPS_FROM: (u32, u32) = (3, 8);

/// `#{version}` as a (major, minor): `3.8`, `3.8a`, `next-3.9`; `None`
/// for anything else (`master`, an empty reply).
pub fn tmux_version(raw: &str) -> Option<(u32, u32)> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("next-").unwrap_or(raw);
    let (major, rest) = raw.split_once('.')?;
    let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Whether a server of this `#{version}` may hold a phone's group (B812):
/// 3.8 and later, `next-3.9` included; `master` (a build from the tree)
/// is newer than any release. Anything else — `openbsd-7.6`, an empty
/// reply, what doesn't parse — is unknown, and gets no group.
pub fn groups_are_safe_on(raw: &str) -> bool {
    raw.trim() == "master" || tmux_version(raw).is_some_and(|version| version >= GROUPS_FROM)
}

/// The read of the server's version (B812), answered `v=<#{version}>`.
pub const VERSION_READ: &str = "display-message -p 'v=#{version}'";

/// Whether `name` is a phone's group, by its name.
pub fn is_group_name(name: &str) -> bool {
    name.contains(GROUP_MARK)
}

/// A phone group: the session it views, its name, its id once tmux said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhoneGroup {
    pub session: String,
    pub name: String,
    pub id: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct GroupState {
    /// Ours: asked for, then confirmed by its id.
    pub current: Option<PhoneGroup>,
    /// tmux refused one on this connection: S itself from here on.
    pub refused: bool,
    /// Every session named as a group at the last session listing.
    pub group_ids: BTreeSet<String>,
    /// The real session the last `%session-changed` put this client on.
    pub client_on: Option<String>,
    /// Between the make and its `%session-changed`: tmux echoes every
    /// window of S as added to the group, all of them known.
    pub entering: bool,
    /// The group's current window is the lowest one, not S's: the next
    /// window listing lands it where S is, unless something picked first.
    pub landing: bool,
    /// Landed on another phone's group at attach: the attach to our own
    /// group of its session is still the fresh one.
    pub fresh_pending: bool,
    /// Makes on this connection: each takes the next tag.
    pub made: u64,
    /// Makes tmux refused for a name already taken, in a row.
    pub taken: u32,
    /// The attach's check of who is on the session is under way: the
    /// restore waits for its answer (and for the group it makes).
    pub deciding: bool,
    /// The group was made after the attach's listings: list again once it
    /// is confirmed (the landing, the restore).
    pub late: bool,
    /// The sessions a control client that isn't Moshpit's was on at the
    /// attach's check: no group of those.
    pub foreign: BTreeSet<String>,
    /// The server is tmux 3.8 or later (B812): `None` until `#{version}`
    /// is read, `Some(false)` when it is earlier or unread.
    pub supported: Option<bool>,
    /// The version read is out.
    pub version_asked: bool,
    /// Plain switches sent before the version was known (B812), whose
    /// `%session-changed` is still to come.
    pub unversioned_switches: u32,
    /// Every phone's group at the last sweep, by id: the session each is
    /// of (`@moshpit_group_of`).
    pub marked: BTreeMap<String, String>,
    /// The check's answers, until the last one comes.
    pub check_self: Option<String>,
    pub check_rows: Option<Vec<(bool, String, String)>>,
}

impl Controller {
    /// Our group's name, while we have one.
    pub fn phone_group(&self) -> Option<&str> {
        self.group.current.as_ref().map(|group| group.name.as_str())
    }

    /// Whether this client views its session through a group of its own
    /// (tmux confirmed it).
    pub(super) fn in_group(&self) -> bool {
        self.group
            .current
            .as_ref()
            .is_some_and(|group| group.id.is_some())
    }

    /// The session tmux has this client on: our group once confirmed, else
    /// the one shown.
    pub(super) fn client_session_id(&self) -> Option<String> {
        match self
            .group
            .current
            .as_ref()
            .and_then(|group| group.id.clone())
        {
            Some(id) => Some(id),
            None => self.snapshot.active_session_id.clone(),
        }
    }

    /// The real session `id` (named `name`) stands for: S for our group.
    pub(super) fn group_target(&self, id: &str, name: &str) -> Option<String> {
        let group = self.group.current.as_ref()?;
        (group.id.as_deref() == Some(id) || group.name == name).then(|| group.session.clone())
    }

    /// Our group of `session`, unless one is already ours, tmux refused, a
    /// desktop's control client was on it at the attach, or the server's
    /// tmux is one a group can crash (B812). A mosh sidecar's too (B773):
    /// the phone's renderer is moved into it.
    pub(super) fn wants_group_for(&self, session: &str) -> bool {
        self.group.supported == Some(true) && self.group_possible(session)
    }

    /// [`Self::wants_group_for`] before the version is known: the attach
    /// asks, and decides once the answers are in.
    pub(super) fn group_possible(&self, session: &str) -> bool {
        !self.group.refused
            && !self.group.foreign.contains(session)
            && self
                .group
                .current
                .as_ref()
                .is_none_or(|group| group.session != session)
    }

    /// Make our group of `session` (named `name`) and move this client into
    /// it: before anything else is sent, so all that follows runs there.
    pub(super) fn enter_group(&mut self, session: &str, name: &str) {
        if !is_valid_id(session) {
            return;
        }
        let group = PhoneGroup {
            session: session.to_owned(),
            name: self.group_name_for(session, name),
            id: None,
        };
        self.group.made += 1;
        let quoted = format!("'{}'", group.name);
        // `=name:`: set-option takes a pane target, which reads an exact
        // session name only with the colon (3.6a: `-t '=name'` is "no such
        // session").
        let exact = format!("'={}:'", group.name);
        // The group this client leaves: ended behind the move (B812: no
        // `destroy-unattached` ends it for us).
        let left = self
            .group
            .current
            .as_ref()
            .and_then(|current| current.id.clone());
        self.send_command(
            format!("new-session -t '{session}' -s {quoted} -P -F '#{{session_id}}'"),
            Reply::GroupMade {
                group: group.clone(),
            },
        );
        self.send(format!(
            "set-option -t {exact} {GROUP_OF_OPTION} '{session}'"
        ));
        if let Some(left) = &left {
            self.group.group_ids.remove(left);
        }
        if self.config.renders_output {
            // This client was its only one, and has moved.
            if let Some(left) = &left {
                self.send(format!("kill-session -t '{left}'"));
            }
        } else {
            // Over mosh the renderer may still be on it: ended once the
            // renderer has moved too ([`Self::bring_renderer_into_group`]),
            // else left to the next attach's sweep — a kill under the
            // renderer detached it, and ended the mosh session (B583).
            self.sidecar.group_left = left;
        }
        self.group.current = Some(group);
        self.group.entering = true;
        self.group.landing = true;
        // B773: a mosh sidecar lands the group where the session is right
        // away (a group starts on its lowest window), and brings the phone's
        // renderer in behind, in the same queue: a size probe around the
        // move reads it (B150). Not listed yet: at the listing's landing.
        if !self.config.renders_output {
            let active = self
                .snapshot
                .windows
                .values()
                .find(|window| window.session_id == session && window.is_active)
                .map(|window| window.id.clone());
            match active {
                Some(window) => {
                    self.send(format!("select-window -t {window}"));
                    self.group.landing = false;
                    self.bring_renderer_into_group();
                }
                None => self.sidecar.renderer_awaits_landing = true,
            }
        }
    }

    /// `<S>~mp-<device>-<n>-<tag>`: S's name in the characters tmux keeps,
    /// the device, S's number (two sessions whose names read alike once cut
    /// never share a group's name) and three base-36 digits of the clock and
    /// this connection's makes, so two phones on one session never take the
    /// same name.
    fn group_name_for(&self, session: &str, session_name: &str) -> String {
        let base: String = session_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(20)
            .collect();
        let base = if base.is_empty() {
            "s".to_owned()
        } else {
            base
        };
        let device: String = self
            .config
            .size_owner_label
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        let device = if device.is_empty() {
            "phone".to_owned()
        } else {
            device
        };
        let number = session.trim_start_matches('$');
        let mut n = (self.unix_seconds() as u64 + self.group.made) % 46_656;
        let mut tag = [b'0'; 3];
        for slot in tag.iter_mut().rev() {
            *slot = b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize];
            n /= 36;
        }
        format!(
            "{base}{GROUP_MARK}{device}-{number}-{}",
            String::from_utf8_lossy(&tag)
        )
    }

    /// Who is on `session` (named `name`), asked before the attach makes a
    /// group: this client's name, the clients, and which are Moshpit's.
    pub(super) fn check_for_foreign_control(&mut self, session: &str, name: &str) {
        self.group.deciding = true;
        self.group.check_self = None;
        self.group.check_rows = None;
        self.send_command(
            "display-message -p 'self=#{client_name}'".to_owned(),
            Reply::GroupCheckSelf,
        );
        self.send_command(
            "list-clients -F 'cc=#{client_control_mode} #{session_id} #{client_name}'".to_owned(),
            Reply::GroupCheckClients,
        );
        self.send_command(
            format!("show-options -sqv {CLIENTS_OPTION}"),
            Reply::GroupCheckMarks {
                session: session.to_owned(),
                name: name.to_owned(),
            },
        );
    }

    pub(super) fn group_check_self_answered(&mut self, response: &CommandResponse) {
        self.group.check_self = response
            .lines
            .first()
            .and_then(|line| moshpit_herdr::text::trim_whitespaces(line).strip_prefix("self="))
            .filter(|name| !name.is_empty() && !response.is_error)
            .map(str::to_owned);
    }

    pub(super) fn group_check_clients_answered(&mut self, response: &CommandResponse) {
        self.group.check_rows = (!response.is_error).then(|| {
            response
                .lines
                .iter()
                .filter_map(|line| {
                    let mut words = line.splitn(3, ' ');
                    let control = words.next()?.strip_prefix("cc=")? == "1";
                    let session = words.next().filter(|s| is_valid_id(s))?;
                    let name = words.next().filter(|n| !n.is_empty())?;
                    Some((control, session.to_owned(), name.to_owned()))
                })
                .collect()
        });
    }

    /// The check's last answer: which sessions have a control client that is
    /// neither this one nor in Moshpit's list; the list kept to the clients
    /// still here, and this one; then the group, or — a desktop's control
    /// client on `session` — the session itself.
    pub(super) fn group_check_marks_answered(
        &mut self,
        session: &str,
        name: &str,
        response: &CommandResponse,
    ) {
        let marks: BTreeSet<String> = if response.is_error {
            BTreeSet::new()
        } else {
            response
                .lines
                .iter()
                .flat_map(|line| line.split_whitespace())
                .map(str::to_owned)
                .collect()
        };
        let me = self
            .group
            .check_self
            .take()
            .or_else(|| self.size.our_client_name.clone());
        let rows = self.group.check_rows.take().unwrap_or_default();
        self.group.foreign = rows
            .iter()
            .filter(|(control, _, client)| {
                *control && Some(client) != me.as_ref() && !marks.contains(client)
            })
            .map(|(_, session, _)| session.clone())
            .collect();
        // Moshpit's list: the ones still attached, and this one.
        match &me {
            Some(me) => {
                let mut kept: BTreeSet<String> = rows
                    .iter()
                    .map(|(_, _, client)| client.clone())
                    .filter(|client| marks.contains(client))
                    .collect();
                kept.insert(me.clone());
                let list: Vec<String> = kept
                    .into_iter()
                    .filter(|client| !client.contains(['\'', ' ']))
                    .collect();
                self.send(format!(
                    "set-option -s {CLIENTS_OPTION} '{}'",
                    list.join(" ")
                ));
            }
            None => self.register_as_moshpit(),
        }
        // Fix (B581): a session picked while the check was out (a tap on
        // the Home tree as the phone connects) went into its own group
        // then — even S again, after another; making one now moved the
        // phone back, or made a second. The pick stands.
        let picked = self.snapshot.active_session_id.as_deref() != Some(session)
            || self.group.current.is_some();
        if picked {
            log::info!(
                "tmux: another session was picked while the attach asked: no group of {session} (B581)"
            );
            // B812: picked before the version was in, it went onto that
            // session itself: its group now, as the pick would have made.
            let active = self.snapshot.active_session_id.clone();
            if let Some(active) = active
                && self.group.current.is_none()
                && self.group.supported == Some(true)
                && self.group_possible(&active)
            {
                let name = self
                    .snapshot
                    .sessions
                    .get(&active)
                    .map(|info| info.name.clone())
                    .unwrap_or_default();
                self.group.late = true;
                self.enter_group(&active, &name);
                return;
            }
            self.group.deciding = false;
            self.apply_pending_restore_if_ready();
            return;
        }
        if self.group.foreign.contains(session) {
            log::info!(
                "tmux: a control client that isn't Moshpit's is on {session}: the phone stays on the session, no group (B459; B457 guards the restore)"
            );
            self.group.deciding = false;
            self.apply_pending_restore_if_ready();
            return;
        }
        if self.group.supported != Some(true) {
            // B812: a tmux a group can crash.
            self.group.deciding = false;
            self.apply_pending_restore_if_ready();
            return;
        }
        self.group.late = true;
        self.enter_group(session, name);
    }

    /// Read the server's version once a connection, ahead of everything a
    /// fresh attach asks (B812): the group decisions wait for it. Not
    /// before tmux has said where this client is: a command written
    /// earlier lands in the shell, and is dropped.
    pub(super) fn read_server_version(&mut self) {
        if self.group.supported.is_some() || self.group.version_asked || !self.snapshot.is_attached
        {
            return;
        }
        self.group.version_asked = true;
        self.send_command(VERSION_READ.to_owned(), Reply::ServerVersion);
    }

    pub(super) fn server_version_answered(&mut self, response: &CommandResponse) {
        let version = response
            .lines
            .first()
            .filter(|_| !response.is_error)
            .and_then(|line| moshpit_herdr::text::trim_whitespaces(line).strip_prefix("v="))
            .unwrap_or("")
            .to_owned();
        let safe = groups_are_safe_on(&version);
        if !safe {
            log::info!(
                "tmux {version:?}: no group for the phone (B812: a group can crash a tmux before 3.8)"
            );
        }
        self.group.supported = Some(safe);
    }

    /// This client onto `session` itself, no group. Before the version is
    /// known (B812) its `%session-changed` comes back after it: counted, so
    /// that echo is not taken for a move to make a group of — the person
    /// may have picked another session since.
    pub(super) fn switch_without_group(&mut self, session: &str) {
        self.send(format!("switch-client -t {session}"));
        if self.group.supported.is_none() {
            self.group.unversioned_switches += 1;
        }
    }

    /// The person's end of the session (B812): our group ended, over a
    /// channel outside `-CC`. tmux's `destroy-unattached` used to.
    pub fn group_release_command(&self) -> Option<String> {
        let id = self.group.current.as_ref()?.id.clone()?;
        is_valid_id(&id).then(|| format!("kill-session -t '{id}'"))
    }

    /// This control client into Moshpit's list (a mosh sidecar's attach).
    pub(super) fn register_as_moshpit(&mut self) {
        self.send(format!(
            "set-option -s -a -F {CLIENTS_OPTION} ' #{{client_name}}'"
        ));
    }

    /// The make's reply: the group's id, or — refused, or no id — S itself
    /// from here on.
    pub(super) fn group_made(&mut self, group: PhoneGroup, response: &CommandResponse) {
        let id = response
            .lines
            .first()
            .map(|line| moshpit_herdr::text::trim_whitespaces(line).to_owned())
            .filter(|id| !response.is_error && id.starts_with('$') && is_valid_id(id));
        if self.group.current.as_ref().map(|g| &g.name) != Some(&group.name) {
            return;
        }
        let taken = response.is_error
            && response
                .lines
                .first()
                .is_some_and(|line| line.contains("duplicate session"));
        let late = std::mem::take(&mut self.group.late);
        match id {
            Some(id) => {
                log::info!(
                    "tmux: the phone views {} through its own group {} (B459)",
                    group.session,
                    group.name
                );
                self.group.taken = 0;
                if let Some(current) = self.group.current.as_mut() {
                    current.id = Some(id.clone());
                }
                self.group.group_ids.insert(id);
                // B773: a sidecar's status line goes with the group's id.
                self.sync_status_line();
                if late {
                    // Made after the attach's listings: list again, in the
                    // group (the landing, then the restore).
                    self.group.deciding = false;
                    self.refresh_windows_and_panes(false, None);
                }
            }
            None if taken && self.group.taken < 2 => {
                // The name was taken (two makes in one second): the next tag.
                self.group.taken += 1;
                let base = group
                    .name
                    .split(GROUP_MARK)
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                self.group.current = None;
                self.group.late = late;
                self.enter_group(&group.session, &base);
            }
            None => {
                log::info!(
                    "tmux: no group for {}: tmux refused it; the phone stays on the session (B459)",
                    group.session
                );
                self.group.current = None;
                self.group.refused = true;
                self.group.entering = false;
                self.group.landing = false;
                // A switch that went by the group: the plain switch instead.
                if self.group.client_on.as_deref() != Some(group.session.as_str()) {
                    self.send(format!("switch-client -t {}", group.session));
                }
                if late {
                    self.group.deciding = false;
                    self.apply_pending_restore_if_ready();
                }
            }
        }
    }

    /// What an attach sweeps: every marked group no client is on, and one
    /// whose session is gone (a client that died between the make and its
    /// option, an old tmux) — never a session without the mark.
    pub(super) fn sweep_groups(&mut self) {
        self.send_command(
            format!(
                "list-sessions -F '#{{session_id}} #{{session_attached}} #{{{GROUP_OF_OPTION}}}'"
            ),
            Reply::GroupSweep,
        );
    }

    pub(super) fn groups_swept(&mut self, response: &CommandResponse) {
        if response.is_error {
            return;
        }
        let rows: Vec<(String, i64, String)> = response
            .lines
            .iter()
            .filter_map(|line| {
                let mut words = line.split(' ');
                let id = words
                    .next()
                    .filter(|id| id.starts_with('$') && is_valid_id(id))?;
                let attached = words.next().and_then(|n| n.parse::<i64>().ok())?;
                // A mark is a session id; anything else is no mark.
                let of = words
                    .next()
                    .map(str::trim)
                    .filter(|of| of.starts_with('$') && is_valid_id(of))
                    .unwrap_or("")
                    .to_owned();
                Some((id.to_owned(), attached, of))
            })
            .collect();
        self.group.marked = rows
            .iter()
            .filter(|(_, _, of)| !of.is_empty())
            .map(|(id, _, of)| (id.clone(), of.clone()))
            .collect();
        let ours = self.group.current.as_ref().and_then(|g| g.id.clone());
        let mut doomed = Vec::new();
        let mut defused = Vec::new();
        for (id, attached, of) in &rows {
            if of.is_empty() || Some(id) == ours.as_ref() {
                continue;
            }
            // B812: never one a client is on (another phone's, made by
            // any version): `kill-session` would end that phone's view.
            // Its session gone, it waits for its phone to leave.
            if *attached == 0 {
                doomed.push(id.clone());
            } else {
                defused.push(id.clone());
            }
        }
        for id in doomed {
            self.send(format!("kill-session -t '{id}'"));
        }
        // B812: another phone's group still in use, made by an earlier
        // version with `destroy-unattached on`: off, so tmux never reaps it
        // under a client it still points at.
        for id in defused {
            self.send(format!("set-option -t '{id}' destroy-unattached off"));
        }
    }

    /// The session listing found our group's session gone: the group alone
    /// would keep its windows alive. It goes too — its id, for the caller
    /// to kill once this client has moved off it (B582).
    pub(super) fn drop_orphaned_group(&mut self, listed: &BTreeSet<String>) -> Option<String> {
        let group = self.group.current.clone()?;
        let id = group.id?;
        if listed.contains(&group.session) || !self.group.group_ids.contains(&id) {
            return None;
        }
        self.group.current = None;
        self.group.entering = false;
        self.group.landing = false;
        Some(id)
    }
}
