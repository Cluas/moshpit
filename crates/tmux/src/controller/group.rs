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
//! - Made from this `-CC` client without `-d`: tmux makes it and moves the
//!   client into it in one step, and every command after it runs there — a
//!   restore's `select-window` included. Only then `destroy-unattached on`
//!   (set on a group no client is on yet, tmux destroys it at once) and the
//!   mark `@moshpit_group_of` naming S.
//! - Gone the moment this client leaves it — a switch, a detach, the app
//!   killed, the network dropped — by tmux itself; S keeps every window.
//! - What a client that died between the make and the option left, an
//!   attach sweeps: a marked group no client is on.
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

use std::collections::BTreeSet;

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

    /// Our group of `session`, unless one is already ours, tmux refused, or
    /// a desktop's control client was on it at the attach.
    pub(super) fn wants_group_for(&self, session: &str) -> bool {
        self.config.renders_output
            && !self.group.refused
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
        self.send_command(
            format!("new-session -t '{session}' -s {quoted} -P -F '#{{session_id}}'"),
            Reply::GroupMade {
                group: group.clone(),
            },
        );
        self.send(format!("set-option -t {exact} destroy-unattached on"));
        self.send(format!(
            "set-option -t {exact} {GROUP_OF_OPTION} '{session}'"
        ));
        self.group.current = Some(group);
        self.group.entering = true;
        self.group.landing = true;
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
        self.group.late = true;
        self.enter_group(session, name);
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
        if !self.config.renders_output {
            return;
        }
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
        let ours = self.group.current.as_ref().and_then(|g| g.id.clone());
        let ids: BTreeSet<&str> = rows.iter().map(|(id, _, _)| id.as_str()).collect();
        let mut doomed = Vec::new();
        for (id, attached, of) in &rows {
            if of.is_empty() || Some(id) == ours.as_ref() {
                continue;
            }
            if *attached == 0 || !ids.contains(of.as_str()) {
                doomed.push(id.clone());
            }
        }
        for id in doomed {
            self.send(format!("kill-session -t '{id}'"));
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
