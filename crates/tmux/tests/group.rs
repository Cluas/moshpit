//! The phone's own view of a session (B459): a grouped session the phone's
//! `-CC` client makes and sits on, so its window switches never move a
//! desktop on the same session. Against a small model of tmux 3.6a's
//! grouped sessions, as a private server showed them: `new-session -t`
//! from a control client makes the group and moves the client into it, the
//! group starts on the session's lowest window, a window's current pick is
//! per session and its active pane per window, and a group with
//! `destroy-unattached on` goes when its last client leaves.

use std::cell::RefCell;
use std::rc::Rc;

use moshpit_tmux::controller::Config;
use moshpit_tmux::model::Selection;
use moshpit_tmux::rig::{Answer, Rig, ok};

#[derive(Debug, Clone)]
struct Session {
    id: String,
    name: String,
    current: String,
    /// Clients on it besides the phones the model tracks.
    desktops: i64,
    group_of: Option<String>,
    destroy_unattached: bool,
}

/// One tmux server: sessions sharing S's windows, one active pane per
/// window, and where each phone's client is.
#[derive(Debug)]
struct Tmux {
    sessions: Vec<Session>,
    /// `(window, index, name, panes)`; every session in S's group has them.
    windows: Vec<(String, usize, String, Vec<String>)>,
    /// Each window's active pane.
    active_pane: Vec<(String, String)>,
    /// `(phone, session)`: where each phone's client is.
    clients: Vec<(String, String)>,
    /// `(session, name)`: control clients the model doesn't drive (a
    /// desktop's tmux integration, another device's sidecar).
    control_desktops: Vec<(String, String)>,
    /// The server option `@moshpit_cc`.
    registry: String,
    next: u32,
}

impl Tmux {
    /// `$0 main` with `@1` (current, `%1`) and `@2` (`%2` active, `%3`),
    /// a desktop on it.
    fn new() -> Rc<RefCell<Tmux>> {
        Rc::new(RefCell::new(Tmux {
            sessions: vec![Session {
                id: "$0".into(),
                name: "main".into(),
                current: "@1".into(),
                desktops: 1,
                group_of: None,
                destroy_unattached: false,
            }],
            windows: vec![
                ("@1".into(), 0, "main".into(), vec!["%1".into()]),
                (
                    "@2".into(),
                    1,
                    "logs".into(),
                    vec!["%2".into(), "%3".into()],
                ),
            ],
            active_pane: vec![("@1".into(), "%1".into()), ("@2".into(), "%2".into())],
            clients: Vec::new(),
            control_desktops: Vec::new(),
            registry: String::new(),
            next: 1,
        }))
    }

    fn session(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    fn session_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    fn by_target(&self, target: &str) -> Option<String> {
        let target = target.trim_matches('\'');
        // `=name:`, exact, as a pane target reads it; `=name` alone is no
        // session there (3.6a).
        let target = match target.strip_prefix('=') {
            Some(exact) => exact.strip_suffix(':')?,
            None => target,
        };
        self.sessions
            .iter()
            .find(|s| s.id == target || s.name == target)
            .map(|s| s.id.clone())
    }

    fn attached(&self, id: &str) -> i64 {
        let phones = self.clients.iter().filter(|(_, s)| s == id).count() as i64;
        let control = self
            .control_desktops
            .iter()
            .filter(|(s, _)| s == id)
            .count() as i64;
        phones + control + self.session(id).map_or(0, |s| s.desktops)
    }

    fn client_of(&self, phone: &str) -> Option<String> {
        self.clients
            .iter()
            .find(|(p, _)| p == phone)
            .map(|(_, s)| s.clone())
    }

    /// The phone's client moves to `to`; a group it left with nobody on it
    /// goes, if it was told to.
    fn move_client(&mut self, phone: &str, to: &str) -> String {
        let left = self.client_of(phone);
        self.clients.retain(|(p, _)| p != phone);
        self.clients.push((phone.into(), to.into()));
        let mut notes = String::new();
        if let Some(left) = left
            && left != to
        {
            notes.push_str(&self.reap(&left));
        }
        let name = self.session(to).map(|s| s.name.clone()).unwrap_or_default();
        notes.push_str(&format!("%session-changed {to} {name}\n"));
        notes
    }

    fn reap(&mut self, id: &str) -> String {
        let gone = self.attached(id) == 0 && self.session(id).is_some_and(|s| s.destroy_unattached);
        if gone {
            self.sessions.retain(|s| s.id != id);
            return "%sessions-changed\n".into();
        }
        String::new()
    }

    /// The phone's client is gone (the app killed, the link dropped).
    fn drop_client(&mut self, phone: &str) {
        if let Some(left) = self.client_of(phone) {
            self.clients.retain(|(p, _)| p != phone);
            self.reap(&left);
        }
    }

    fn answer(&mut self, phone: &str, command: &str) -> Option<Answer> {
        let words: Vec<&str> = command.split(' ').collect();
        let flag = |name: &str| {
            words
                .iter()
                .position(|w| *w == name)
                .and_then(|i| words.get(i + 1))
                .map(|w| w.to_string())
        };
        if command.starts_with("new-session -t ") {
            let target = self.by_target(&flag("-t")?)?;
            let name = flag("-s")?.trim_matches('\'').to_owned();
            if self.sessions.iter().any(|s| s.name == name) {
                return Some(Answer {
                    lines: vec![format!("duplicate session: {name}")],
                    error: true,
                    ..Answer::default()
                });
            }
            let id = format!("${}", 100 + self.next);
            self.next += 1;
            let lowest = self.windows.iter().min_by_key(|w| w.1)?.0.clone();
            let _ = target;
            self.sessions.push(Session {
                id: id.clone(),
                name: name.clone(),
                current: lowest,
                desktops: 0,
                group_of: None,
                destroy_unattached: false,
            });
            let mut after = String::new();
            for (window, ..) in &self.windows {
                after.push_str(&format!("%window-add {window}\n"));
            }
            after.push_str("%sessions-changed\n");
            after.push_str(&self.move_client(phone, &id));
            return Some(Answer {
                lines: vec![id],
                after,
                ..Answer::default()
            });
        }
        if command.starts_with("new-session -d -P -F ") {
            // A session of its own (B584), named as given or by its number.
            let id = format!("${}", 5 + self.next);
            self.next += 1;
            let name = flag("-s")
                .map(|n| n.trim_matches('"').to_owned())
                .unwrap_or_else(|| id.trim_start_matches('$').to_owned());
            self.sessions.push(Session {
                id: id.clone(),
                name: name.clone(),
                current: "@9".into(),
                desktops: 0,
                group_of: None,
                destroy_unattached: false,
            });
            let line = if command.contains("#{session_name}") {
                format!("{id} {name}")
            } else {
                id
            };
            return Some(Answer {
                lines: vec![line],
                after: "%sessions-changed\n".into(),
                ..Answer::default()
            });
        }
        if command == "display-message -p 'self=#{client_name}'" {
            return ok([format!("self=/dev/{phone}")]);
        }
        if command.starts_with("list-clients -F 'cc=") {
            let mut rows: Vec<String> = self
                .clients
                .iter()
                .map(|(p, s)| format!("cc=1 {s} /dev/{p}"))
                .collect();
            for s in &self.sessions {
                for n in 0..s.desktops {
                    rows.push(format!("cc=0 {} /dev/desk{n}", s.id));
                }
            }
            for (s, name) in &self.control_desktops {
                rows.push(format!("cc=1 {s} {name}"));
            }
            return ok(rows);
        }
        if command == "show-options -sqv @moshpit_cc" {
            let lines: Vec<String> = (!self.registry.is_empty())
                .then(|| self.registry.clone())
                .into_iter()
                .collect();
            return ok(lines);
        }
        if let Some(value) = command.strip_prefix("set-option -s @moshpit_cc ") {
            self.registry = value.trim_matches('\'').to_owned();
            return ok(Vec::<String>::new());
        }
        if command == "set-option -s -a -F @moshpit_cc ' #{client_name}'" {
            self.registry.push_str(&format!(" /dev/{phone}"));
            return ok(Vec::<String>::new());
        }
        if command.starts_with("set-option -t ") && !command.contains(" -w ") {
            let target = self.by_target(&flag("-t")?)?;
            let session = self.session_mut(&target)?;
            if command.ends_with("destroy-unattached on") {
                session.destroy_unattached = true;
            } else if let Some(of) = flag("@moshpit_group_of") {
                session.group_of = Some(of.trim_matches('\'').to_owned());
            }
            return ok(Vec::<String>::new());
        }
        if command.starts_with("switch-client -t ") {
            let target = self.by_target(&flag("-t")?)?;
            let after = self.move_client(phone, &target);
            return Some(Answer {
                after,
                ..Answer::default()
            });
        }
        if command.starts_with("kill-session -t ") {
            let target = self.by_target(&flag("-t")?)?;
            self.sessions.retain(|s| s.id != target);
            let mut after = "%sessions-changed\n".to_owned();
            if self.client_of(phone).as_deref() == Some(target.as_str())
                && let Some(next) = self.sessions.first().map(|s| s.id.clone())
            {
                after.push_str(&self.move_client(phone, &next));
            }
            return Some(Answer {
                after,
                ..Answer::default()
            });
        }
        if command.starts_with("select-window -t ") {
            let window = flag("-t")?;
            let on = self.client_of(phone)?;
            self.session_mut(&on)?.current = window.clone();
            return Some(Answer {
                after: format!("%session-window-changed {on} {window}\n"),
                ..Answer::default()
            });
        }
        if command.starts_with("select-pane ") {
            let pane = words.last()?.to_string();
            let window = self
                .windows
                .iter()
                .find(|w| w.3.contains(&pane))
                .map(|w| w.0.clone())?;
            for (w, active) in &mut self.active_pane {
                if *w == window {
                    *active = pane.clone();
                }
            }
            return ok(Vec::<String>::new());
        }
        if command == "list-sessions -F '#{session_id} #{session_attached} #{session_name}'" {
            return ok(self
                .sessions
                .iter()
                .map(|s| format!("{} {} {}", s.id, self.attached(&s.id), s.name))
                .collect::<Vec<_>>());
        }
        if command.starts_with(
            "list-sessions -F '#{session_id} #{session_attached} #{@moshpit_group_of}'",
        ) {
            return ok(self
                .sessions
                .iter()
                .map(|s| {
                    format!(
                        "{} {} {}",
                        s.id,
                        self.attached(&s.id),
                        s.group_of.clone().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>());
        }
        if command.starts_with("list-windows -a -F '#{session_id}") {
            let mut rows = Vec::new();
            for s in &self.sessions {
                for (window, index, name, panes) in &self.windows {
                    rows.push(format!(
                        "{} {window} {index} b25d,80x24,0,0,1 {} {} {name}",
                        s.id,
                        u8::from(s.current == *window),
                        panes.len()
                    ));
                }
            }
            return ok(rows);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            let mut rows = Vec::new();
            for (window, _, _, panes) in &self.windows {
                let active = self
                    .active_pane
                    .iter()
                    .find(|(w, _)| w == window)
                    .map(|(_, p)| p.clone());
                for (i, pane) in panes.iter().enumerate() {
                    rows.push(format!(
                        "{pane} {window} {i} 80 24 {} 0 0 bash",
                        u8::from(active.as_ref() == Some(pane))
                    ));
                }
            }
            return ok(rows);
        }
        if command.starts_with("display-message -p '#{client_name}'") {
            return ok([format!("/dev/{phone}")]);
        }
        if command.starts_with("list-clients -F '#{client_control_mode}") {
            let mut rows: Vec<String> = self
                .clients
                .iter()
                .map(|(p, s)| format!("1|111|{s}|/dev/{p}"))
                .collect();
            for s in &self.sessions {
                for n in 0..s.desktops {
                    rows.push(format!("0|222|{}|/dev/desk{n}", s.id));
                }
            }
            for (s, name) in &self.control_desktops {
                rows.push(format!("1|333|{s}|{name}"));
            }
            return ok(rows);
        }
        if command.starts_with("display-message -p -t '$") && command.contains("@moshpit_group_of")
        {
            let target = self.by_target(&flag("-t")?)?;
            return ok([self.session(&target)?.group_of.clone().unwrap_or_default()]);
        }
        None
    }
}

/// A phone (`label`, client `phone`) attached over -CC to the model's
/// latest session, settled.
fn phone(tmux: &Rc<RefCell<Tmux>>, phone: &str, label: &str, restore: Option<Selection>) -> Rig {
    let mut rig = Rig::new(Config {
        size_owner_label: label.into(),
        ..Config::default()
    });
    if let Some(restore) = restore {
        rig.with(|c, _| c.set_pending_restore(Some(restore)));
    }
    let model = tmux.clone();
    let me = phone.to_owned();
    rig.respond(Box::new(move |command, _| {
        model.borrow_mut().answer(&me, command)
    }));
    rig.with(|c, now| c.begin_control_mode(now));
    // The boot line's own `-CC attach` (no target): the session it lands on.
    let landed = tmux.borrow().sessions[0].clone();
    tmux.borrow_mut()
        .clients
        .push((phone.into(), landed.id.clone()));
    rig.push(&format!(
        "%begin 100 0 0\n%end 100 0 0\n\n%session-changed {} {}\n",
        landed.id, landed.name
    ));
    rig.settle();
    rig
}

fn group_of<'a>(tmux: &'a Tmux, phone: &str) -> &'a Session {
    let on = tmux.client_of(phone).expect("the phone is attached");
    tmux.session(&on).expect("its session exists")
}

#[test]
fn a_window_switch_on_the_phone_leaves_a_desktop_on_the_session_where_it_is() {
    let tmux = Tmux::new();
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    rig.with(|c, now| c.select_window("@2", now));
    rig.settle();
    let model = tmux.borrow();
    assert_eq!(
        model.session("$0").unwrap().current,
        "@1",
        "the desktop's window"
    );
    let group = group_of(&model, "ttys003");
    assert!(group.name.starts_with("main~mp-iPhone-"), "{}", group.name);
    assert_eq!(group.current, "@2", "the phone's window");
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@2"));
}

/// The group starts on the session's lowest window; the phone lands where
/// the session is, as an attach to the session itself did.
#[test]
fn the_phone_lands_where_the_session_is() {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions[0].current = "@2".into();
    let rig = phone(&tmux, "ttys003", "iPhone", None);
    let model = tmux.borrow();
    assert_eq!(group_of(&model, "ttys003").current, "@2");
    assert_eq!(model.session("$0").unwrap().current, "@2");
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@2")
    );
}

/// tmux destroys the group when its last client goes — set only once the
/// phone is on it (on a group nobody is on yet, tmux destroys it at once)
/// — and the session keeps every window.
#[test]
fn the_group_goes_with_the_phone_and_the_session_keeps_its_windows() {
    let tmux = Tmux::new();
    let rig = phone(&tmux, "ttys003", "iPhone", None);
    let make = rig
        .index_of("new-session -t '$0' -s 'main~mp-iPhone-")
        .unwrap();
    let option = rig.index_of("set-option -t '=main~mp-iPhone-").unwrap();
    // `=name:`: a pane target reads an exact session name only so (3.6a).
    assert!(
        rig.commands[option].contains(":' "),
        "{}",
        rig.commands[option]
    );
    assert!(make < option);
    assert!(rig.commands[option].ends_with("destroy-unattached on"));
    let group = group_of(&tmux.borrow(), "ttys003").clone();
    assert_eq!(group.group_of.as_deref(), Some("$0"));
    tmux.borrow_mut().drop_client("ttys003");
    let model = tmux.borrow();
    assert!(model.session(&group.id).is_none(), "the group is gone");
    assert_eq!(model.sessions.len(), 1);
    assert_eq!(model.windows.len(), 2, "the session keeps its windows");
}

/// What a phone that died between the make and its option left: a marked
/// group nobody is on, or one whose session is gone. Never a session
/// without the mark, never a live phone's group.
#[test]
fn a_leftover_group_is_swept() {
    let tmux = Tmux::new();
    {
        let mut model = tmux.borrow_mut();
        let group = |id: &str, name: &str, of: Option<&str>, desktops: i64| Session {
            id: id.into(),
            name: name.into(),
            current: "@1".into(),
            desktops,
            group_of: of.map(Into::into),
            destroy_unattached: false,
        };
        let stale = group("$5", "main~mp-iPad-0ab", Some("$0"), 0);
        let unmarked = group("$6", "notes~mp-x", None, 0);
        let live = group("$7", "main~mp-iPad-0ac", Some("$0"), 1);
        let orphan = group("$8", "gone~mp-iPhone-0ad", Some("$99"), 0);
        model.sessions.extend([stale, unmarked, live, orphan]);
    }
    let rig = phone(&tmux, "ttys003", "iPhone", None);
    assert!(rig.has("kill-session -t '$5'"));
    assert!(rig.has("kill-session -t '$8'"));
    assert!(!rig.has("kill-session -t '$6'"));
    assert!(!rig.has("kill-session -t '$7'"));
    assert!(!rig.has("kill-session -t '$0'"));
}

/// The person sees the session, never a group: the list, the breadcrumb's
/// session, the windows' session.
#[test]
fn lists_and_the_breadcrumb_show_the_session() {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions[0].desktops = 0;
    let rig = phone(&tmux, "ttys003", "iPhone", None);
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.sessions.keys().collect::<Vec<_>>(), ["$0"]);
    let main = &snapshot.sessions["$0"];
    assert_eq!(main.name, "main");
    assert!(main.is_attached, "the phone is on it, through its group");
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
    assert!(snapshot.windows.values().all(|w| w.session_id == "$0"));
    assert_eq!(snapshot.windows.len(), 2);
}

/// `%session-changed` for the phone's own group stands for its session.
#[test]
fn the_groups_session_change_maps_to_its_session() {
    let tmux = Tmux::new();
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let group = group_of(&tmux.borrow(), "ttys003").clone();
    let before = rig.commands.len();
    rig.push(&format!("%session-changed {} {}\n", group.id, group.name));
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
    assert_eq!(snapshot.sessions["$0"].name, "main");
    assert!(
        !rig.since(before)
            .iter()
            .any(|c| c.starts_with("new-session"))
    );
    assert_eq!(rig.controller.phone_group(), Some(group.name.as_str()));
}

/// Two phones: a group each, neither moves the other, nor the desktop.
#[test]
fn two_phones_get_two_groups() {
    let tmux = Tmux::new();
    let mut iphone = phone(&tmux, "ttys003", "iPhone", None);
    let mut ipad = phone(&tmux, "ttys004", "iPad", None);
    iphone.with(|c, now| c.select_window("@2", now));
    iphone.settle();
    ipad.with(|c, now| c.select_window("@1", now));
    ipad.settle();
    let model = tmux.borrow();
    let (a, b) = (group_of(&model, "ttys003"), group_of(&model, "ttys004"));
    assert_ne!(a.id, b.id);
    assert!(a.name.contains("~mp-iPhone-") && b.name.contains("~mp-iPad-"));
    assert_eq!((a.current.as_str(), b.current.as_str()), ("@2", "@1"));
    assert_eq!(model.session("$0").unwrap().current, "@1");
    for rig in [&iphone, &ipad] {
        assert_eq!(
            rig.controller.snapshot().sessions.len(),
            1,
            "no group listed"
        );
    }
}

/// A reconnect's restore through the group takes its window (the phone's
/// own) with a desktop on the session; only the window's active pane, which
/// tmux shares by window, stays where the desktop left it (B457).
#[test]
fn a_restore_through_the_group_takes_its_window_and_leaves_the_active_pane() {
    let tmux = Tmux::new();
    let restore = Selection {
        session: "$0".into(),
        window: Some("@2".into()),
        pane: Some("%3".into()),
    };
    let rig = phone(&tmux, "ttys003", "iPhone", Some(restore));
    let model = tmux.borrow();
    assert_eq!(group_of(&model, "ttys003").current, "@2");
    assert_eq!(model.session("$0").unwrap().current, "@1");
    assert!(model.active_pane.contains(&("@2".into(), "%2".into())));
    assert!(!rig.has("select-pane -Z -t %3"));
}

/// Alone on the session, the restore takes the pane too.
#[test]
fn a_restore_alone_takes_the_pane_too() {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions[0].desktops = 0;
    let restore = Selection {
        session: "$0".into(),
        window: Some("@2".into()),
        pane: Some("%3".into()),
    };
    let rig = phone(&tmux, "ttys003", "iPhone", Some(restore));
    assert!(rig.has("select-pane -Z -t %3"));
    assert_eq!(group_of(&tmux.borrow(), "ttys003").current, "@2");
}

/// A switch to another session goes into a group of that one, in one step:
/// the window picked with it lands in the group, never on the session.
#[test]
fn a_session_switch_goes_into_a_group_of_the_other_session() {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions.push(Session {
        id: "$1".into(),
        name: "dev".into(),
        current: "@1".into(),
        desktops: 1,
        group_of: None,
        destroy_unattached: false,
    });
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let first = group_of(&tmux.borrow(), "ttys003").id.clone();
    rig.with(|c, now| c.select_session("$1", now));
    rig.settle();
    rig.with(|c, now| c.select_window("@2", now));
    rig.settle();
    let model = tmux.borrow();
    let group = group_of(&model, "ttys003");
    assert!(group.name.starts_with("dev~mp-iPhone-"), "{}", group.name);
    assert_eq!(group.current, "@2");
    assert_eq!(model.session("$1").unwrap().current, "@1");
    assert!(model.session(&first).is_none(), "the first group went");
    assert!(!rig.has("switch-client -t $1"));
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
}

/// Killing the session from the phone takes the phone's group with it: the
/// group alone would keep its windows running.
#[test]
fn killing_the_session_kills_the_phones_group() {
    let tmux = Tmux::new();
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let group = group_of(&tmux.borrow(), "ttys003").id.clone();
    rig.with(|c, now| c.kill_session("$0", now));
    assert!(rig.has(&format!("kill-session -t '{group}'")));
}

/// B582: tmux detaches a client whose session is destroyed
/// (`detach-on-destroy`): the phone moves into a group of the session it
/// falls back to before the kill goes out (the live proof is
/// `live_group.rs`).
#[test]
fn killing_the_session_on_screen_moves_the_phone_first() {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions.push(Session {
        id: "$1".into(),
        name: "dev".into(),
        current: "@1".into(),
        desktops: 0,
        group_of: None,
        destroy_unattached: false,
    });
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let group = group_of(&tmux.borrow(), "ttys003").id.clone();
    let from = rig.since(0).len();
    rig.with(|c, now| c.kill_session("$0", now));
    let sent = rig.since(from).to_vec();
    let at = |prefix: &str| sent.iter().position(|c| c.starts_with(prefix));
    let moved = at("new-session -t '$1'").expect("into a group of dev");
    assert!(moved < at("kill-session -t $0").unwrap(), "{sent:?}");
    assert!(
        moved < at(&format!("kill-session -t '{group}'")).unwrap(),
        "{sent:?}"
    );
}

/// B582: the same when a desktop killed it: onto the session left, then
/// the group goes.
#[test]
fn a_session_killed_elsewhere_moves_the_phone_before_its_group_goes() {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions.push(Session {
        id: "$1".into(),
        name: "dev".into(),
        current: "@1".into(),
        desktops: 1,
        group_of: None,
        destroy_unattached: false,
    });
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let group = group_of(&tmux.borrow(), "ttys003").id.clone();
    let from = rig.since(0).len();
    tmux.borrow_mut().sessions.retain(|s| s.id != "$0");
    rig.push("%sessions-changed\n");
    rig.settle();
    let sent = rig.since(from).to_vec();
    let at = |prefix: &str| sent.iter().position(|c| c.starts_with(prefix));
    let moved = at("new-session -t '$1'").expect("into a group of dev");
    assert!(
        moved < at(&format!("kill-session -t '{group}'")).unwrap(),
        "{sent:?}"
    );
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
}

/// A desktop killed the session under the phone: its group goes too.
#[test]
fn a_session_killed_elsewhere_takes_the_phones_group() {
    let tmux = Tmux::new();
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let group = group_of(&tmux.borrow(), "ttys003").id.clone();
    tmux.borrow_mut().sessions.retain(|s| s.id != "$0");
    rig.push("%sessions-changed\n");
    rig.settle();
    assert!(rig.has(&format!("kill-session -t '{group}'")));
}

/// Someone killed the phone's group (a desktop's `kill-session` on it):
/// tmux moves the phone's client to the session itself, and the phone makes
/// a new group of it rather than stay on the session.
#[test]
fn a_group_killed_under_the_phone_is_made_again() {
    let tmux = Tmux::new();
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    let first = group_of(&tmux.borrow(), "ttys003").id.clone();
    {
        let mut model = tmux.borrow_mut();
        model.sessions.retain(|s| s.id != first);
        model.clients = vec![("ttys003".into(), "$0".into())];
    }
    rig.push("%sessions-changed\n%session-changed $0 main\n");
    rig.settle();
    assert_eq!(rig.count("new-session -t '$0'"), 2);
    let model = tmux.borrow();
    let again = group_of(&model, "ttys003");
    assert_ne!(again.id, first);
    assert!(again.name.starts_with("main~mp-iPhone-"));
    assert_eq!(rig.controller.phone_group(), Some(again.name.as_str()));
}

/// An attach without a target landed on another phone's group: the phone
/// makes its own, of the session that group views.
#[test]
fn landing_on_another_phones_group_makes_one_of_its_own() {
    let tmux = Tmux::new();
    let mut ipad = phone(&tmux, "ttys004", "iPad", None);
    ipad.settle();
    let theirs = group_of(&tmux.borrow(), "ttys004").clone();
    // The iPhone's boot attach lands on the iPad's group.
    let mut rig = Rig::new(Config::default());
    let model = tmux.clone();
    rig.respond(Box::new(move |command, _| {
        model.borrow_mut().answer("ttys003", command)
    }));
    rig.with(|c, now| c.begin_control_mode(now));
    tmux.borrow_mut()
        .clients
        .push(("ttys003".into(), theirs.id.clone()));
    rig.push(&format!(
        "%begin 100 0 0\n%end 100 0 0\n\n%session-changed {} {}\n",
        theirs.id, theirs.name
    ));
    rig.settle();
    let model = tmux.borrow();
    let mine = group_of(&model, "ttys003");
    assert_ne!(mine.id, theirs.id);
    assert!(mine.name.starts_with("main~mp-iPhone-"), "{}", mine.name);
    assert_eq!(mine.group_of.as_deref(), Some("$0"));
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
}

/// A tmux that refuses the group leaves the phone on the session, as before.
#[test]
fn a_refused_group_leaves_the_phone_on_the_session() {
    let mut rig = Rig::new(Config::default());
    rig.respond(Box::new(|command, _| {
        if command.starts_with("new-session -t ") {
            return Some(Answer {
                lines: vec!["unknown flag".into()],
                error: true,
                ..Answer::default()
            });
        }
        if command
            .starts_with("list-sessions -F '#{session_id} #{session_attached} #{session_name}'")
        {
            return ok(["$0 2 main"]);
        }
        None
    }));
    rig.with(|c, now| c.begin_control_mode(now));
    rig.push("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n");
    rig.settle();
    assert_eq!(rig.controller.phone_group(), None);
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
    assert_eq!(rig.count("new-session -t "), 1, "asked once per connection");
}

/// B459: when a group goes, tmux tells every control client on the session
/// that each of its windows closed — they live on, but a desktop's tmux
/// integration (iTerm2's -CC) may close its tabs for them. With a control
/// client on the session that isn't Moshpit's, the phone stays on the
/// session itself, as before the groups, and B457 guards its restore.
#[test]
fn a_desktops_control_client_keeps_the_phone_on_the_session() {
    let tmux = Tmux::new();
    tmux.borrow_mut()
        .control_desktops
        .push(("$0".into(), "/dev/ttys020".into()));
    let restore = Selection {
        session: "$0".into(),
        window: Some("@2".into()),
        pane: Some("%3".into()),
    };
    let rig = phone(&tmux, "ttys003", "iPhone", Some(restore));
    assert!(!rig.has("new-session -t "), "no group");
    assert_eq!(rig.controller.phone_group(), None);
    let model = tmux.borrow();
    assert_eq!(model.client_of("ttys003").as_deref(), Some("$0"));
    // B457: the restore leaves the session's window where the desktop has it.
    assert_eq!(model.session("$0").unwrap().current, "@1");
    assert!(model.registry.split(' ').any(|n| n == "/dev/ttys003"));
}

/// Moshpit's own control clients (another device's mosh sidecar) keep their
/// names in `@moshpit_cc`: one of those on the session is no reason to stay
/// off a group. The list is kept to the clients still attached, and this one.
#[test]
fn another_moshpit_control_client_is_no_reason_to_stay_off() {
    let tmux = Tmux::new();
    {
        let mut model = tmux.borrow_mut();
        model
            .control_desktops
            .push(("$0".into(), "/dev/ttys009".into()));
        model.registry = " /dev/gone /dev/ttys009".into();
    }
    let rig = phone(&tmux, "ttys003", "iPhone", None);
    assert!(rig.controller.phone_group().is_some());
    let model = tmux.borrow();
    assert!(
        group_of(&model, "ttys003")
            .name
            .starts_with("main~mp-iPhone-")
    );
    assert_eq!(model.registry, "/dev/ttys003 /dev/ttys009");
}

/// A mosh sidecar puts itself on Moshpit's list at its attach.
#[test]
fn a_mosh_sidecar_puts_itself_on_moshpits_list() {
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.with(|c, now| c.begin_control_mode(now));
    rig.push("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n");
    assert!(rig.has("set-option -s -a -F @moshpit_cc ' #{client_name}'"));
    assert!(
        !rig.has("new-session -t "),
        "mosh stays on the session (B457)"
    );
}

/// B581: the attach asks who is on the session before it makes the group
/// (the foreign -CC check), and makes it once the answer is in. A session
/// picked in that round trip (a tap on the Home tree as the phone connects)
/// went into its own group first; then the late answer made the first
/// session's group too, and tmux moved the phone back there: it showed the
/// session it attached to, not the one picked.
fn picked_while_the_attach_asks(picks: &[&str]) -> (Rc<RefCell<Tmux>>, Rig) {
    let tmux = Tmux::new();
    tmux.borrow_mut().sessions.push(Session {
        id: "$1".into(),
        name: "dev".into(),
        current: "@1".into(),
        desktops: 0,
        group_of: None,
        destroy_unattached: false,
    });
    let mut rig = Rig::new(Config::default());
    let model = tmux.clone();
    rig.respond(Box::new(move |command, _| {
        model.borrow_mut().answer("ttys003", command)
    }));
    rig.with(|c, now| c.begin_control_mode(now));
    tmux.borrow_mut()
        .clients
        .push(("ttys003".into(), "$0".into()));
    rig.push("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n");
    // Picked before tmux has answered the attach's check.
    for pick in picks {
        rig.with(|c, now| c.select_session(pick, now));
    }
    rig.settle();
    (tmux, rig)
}

#[test]
fn back_on_the_first_session_while_the_attach_asks_makes_one_group_of_it() {
    let (tmux, rig) = picked_while_the_attach_asks(&["$1", "$0"]);
    let model = tmux.borrow();
    let on = group_of(&model, "ttys003");
    assert_eq!(on.group_of.as_deref(), Some("$0"), "on {}", on.name);
    assert_eq!(rig.count("new-session -t '$0'"), 1, "{:?}", rig.since(0));
}

#[test]
fn a_session_picked_while_the_attach_asks_stays_picked() {
    let (tmux, rig) = picked_while_the_attach_asks(&["$1"]);
    let model = tmux.borrow();
    let on = group_of(&model, "ttys003");
    assert_eq!(on.group_of.as_deref(), Some("$1"), "on {}", on.name);
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
}

/// B584: a session made on the phone is landed on as soon as tmux gives its
/// id — before any listing has its name — and its group was named `s~mp-…`
/// (the name of a session not known yet), in the desktop's `tmux ls` too.
#[test]
fn a_session_made_on_the_phone_names_its_group_after_it() {
    let tmux = Tmux::new();
    let mut rig = phone(&tmux, "ttys003", "iPhone", None);
    rig.with(|c, now| c.new_session(Some("work"), now));
    rig.settle();
    let model = tmux.borrow();
    let on = group_of(&model, "ttys003");
    assert!(on.name.starts_with("work~mp-iPhone-"), "on {}", on.name);
    drop(model);
    rig.with(|c, now| c.new_session(None, now));
    rig.settle();
    let model = tmux.borrow();
    let on = group_of(&model, "ttys003");
    let made = model.session(on.group_of.as_deref().unwrap()).unwrap();
    assert!(
        on.name.starts_with(&format!("{}~mp-iPhone-", made.name)),
        "on {} of {}",
        on.name,
        made.name
    );
}
