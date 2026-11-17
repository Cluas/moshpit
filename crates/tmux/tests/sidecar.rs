//! The mosh sidecar (`renders_output: false`), ported from
//! `TmuxSidecarSwitchTests.swift`: sessions by id, only the phone's own
//! renderer moves, the sidecar follows the renderer and nobody else, the rest
//! of the server's notifications, and the status line on sessions Moshpit
//! created. The fake clients render whatever `-F` format is asked for, in
//! the shape tmux 3.6a prints (recorded on a private socket).

use std::cell::RefCell;
use std::rc::Rc;

use moshpit_tmux::controller::{Config, Notification, PinLedger, VERSION_READ};
use moshpit_tmux::rig::{Rig, err, ok};

#[derive(Clone, Debug)]
struct FakeClient {
    control: bool,
    pid: i64,
    session_id: &'static str,
    session_name: String,
    name: String,
}

impl FakeClient {
    fn render(&self, format: &str) -> String {
        format
            .replace(
                "#{client_control_mode}",
                if self.control { "1" } else { "0" },
            )
            .replace("#{client_pid}", &self.pid.to_string())
            .replace("#{session_id}", self.session_id)
            .replace("#{client_session}", &self.session_name)
            .replace("#{session_name}", &self.session_name)
            .replace("#{client_name}", &self.name)
    }
}

fn client(
    control: bool,
    pid: i64,
    session_id: &'static str,
    session_name: &str,
    name: &str,
) -> FakeClient {
    FakeClient {
        control,
        pid,
        session_id,
        session_name: session_name.into(),
        name: name.into(),
    }
}

#[derive(Default)]
struct Fake {
    clients: Vec<FakeClient>,
    sessions: Vec<String>,
    windows: Vec<String>,
    panes: Vec<String>,
    /// The sessions' `@moshpit_owned` / `@moshpit_status_hidden` marks, as
    /// the owned-sessions listing prints them (`$5 owned=1 hidden=…`).
    marks: Vec<String>,
    /// B773: the phone's group made (`new-session -t`), the sidecar moved
    /// into it, as tmux 3.6a does; otherwise refused, and the sidecar stays
    /// on the session itself as it did before groups.
    groups: bool,
    made: u32,
    /// `#{version}`, when not the rig's own (B812).
    version: Option<&'static str>,
}

impl Fake {
    /// What `@moshpit_group_of` holds for session `id`: for a phone's group
    /// (`<name>~mp-…`), the id of the session named `<name>`; else nothing.
    fn group_of(&self, id: &str) -> String {
        let rows: Vec<Vec<&str>> = self
            .sessions
            .iter()
            .map(|row| row.splitn(3, ' ').collect())
            .collect();
        let Some(name) = rows
            .iter()
            .find(|row| row.first() == Some(&id))
            .and_then(|row| row.get(2))
        else {
            return String::new();
        };
        let Some((base, _)) = name.split_once("~mp-") else {
            return String::new();
        };
        rows.iter()
            .find(|row| row.get(2) == Some(&base))
            .map(|row| row[0].to_owned())
            .unwrap_or_default()
    }
}

type Shared = Rc<RefCell<Fake>>;

fn list_clients_format(command: &str) -> Option<&str> {
    let rest = command.strip_prefix("list-clients -F '")?;
    rest.split_once('\'').map(|(format, _)| format)
}

fn responder(fake: Shared) -> moshpit_tmux::rig::Responder {
    Box::new(move |command, _| {
        let mut fake = fake.borrow_mut();
        if let Some(format) = list_clients_format(command) {
            return ok(fake
                .clients
                .iter()
                .map(|c| c.render(format))
                .collect::<Vec<_>>());
        }
        if command == VERSION_READ
            && let Some(version) = fake.version
        {
            return ok([format!("v={version}")]);
        }
        if command.starts_with(
            "list-sessions -F '#{session_id} #{session_attached} #{@moshpit_group_of}'",
        ) {
            // The sweep's listing (B459, B812).
            return ok(fake
                .sessions
                .iter()
                .map(|row| {
                    let mut words = row.splitn(3, ' ');
                    let id = words.next().unwrap_or_default();
                    let attached = words.next().unwrap_or("0");
                    format!("{id} {attached} {}", fake.group_of(id))
                })
                .collect::<Vec<_>>());
        }
        if command == "display-message -p 'self=#{client_name}'" {
            // Who this control client is (B459's check of who is on a
            // session): the sidecar's own.
            let me = fake
                .clients
                .iter()
                .find(|c| c.control)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            return ok([format!("self={me}")]);
        }
        if let Some(rest) = command.strip_prefix("new-session -t ") {
            if !fake.groups {
                return err(["sessions should be nested with care"]);
            }
            // `new-session -t '$S' -s 'NAME' -P -F ...`: made, and this
            // (control) client moved into it.
            let name = rest
                .split(" -s '")
                .nth(1)
                .and_then(|r| r.split('\'').next())
                .unwrap_or("g")
                .to_owned();
            fake.made += 1;
            let id: &'static str = Box::leak(format!("$9{}", fake.made).into_boxed_str());
            fake.sessions.push(format!("{id} 1 {name}"));
            for c in &mut fake.clients {
                if c.control {
                    c.session_id = id;
                    c.session_name = name.clone();
                }
            }
            return ok([id]);
        }
        if let Some(rest) = command.strip_prefix("switch-client -c ") {
            // tmux moves that client when it gets here in the queue.
            let words: Vec<&str> = rest.split(' ').collect();
            if words.len() == 3 && words[1] == "-t" {
                let target = words[2].trim_matches('\'');
                // `=name`: the session by its exact name (a group's, B773).
                let found = match target.strip_prefix('=') {
                    Some(name) => fake
                        .sessions
                        .iter()
                        .find(|s| s.splitn(3, ' ').nth(2) == Some(name))
                        .cloned(),
                    None => fake
                        .sessions
                        .iter()
                        .find(|s| s.starts_with(&format!("{target} ")))
                        .cloned(),
                };
                if let Some(row) = found {
                    let target = row.split(' ').next().unwrap_or(target).to_owned();
                    let name = row.splitn(3, ' ').nth(2).unwrap_or("").to_owned();
                    let id: &'static str = Box::leak(target.to_owned().into_boxed_str());
                    for c in &mut fake.clients {
                        if c.name == words[0] {
                            c.session_id = id;
                            c.session_name = name.clone();
                        }
                    }
                }
            }
            return None;
        }
        if command.starts_with("list-sessions") && command.contains(" owned=#{@moshpit_owned}") {
            return ok(fake.marks.clone());
        }
        if command.starts_with("list-sessions") {
            return ok(fake.sessions.clone());
        }
        if command.starts_with("list-windows") {
            return ok(fake.windows.clone());
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok(fake.panes.clone());
        }
        if command.starts_with("new-session") {
            return ok(["$5"]);
        }
        None
    })
}

struct Sidecar {
    rig: Rig,
    fake: Shared,
}

impl Sidecar {
    fn make(
        sessions: &[&str],
        windows: &[&str],
        clients: Vec<FakeClient>,
        ledger: Option<PinLedger>,
    ) -> Self {
        Self::make_with(sessions, windows, clients, ledger, false)
    }

    /// B773: a tmux that makes the phone's group.
    fn grouped(sessions: &[&str], windows: &[&str], clients: Vec<FakeClient>) -> Self {
        Self::make_with(sessions, windows, clients, None, true)
    }

    fn make_with(
        sessions: &[&str],
        windows: &[&str],
        clients: Vec<FakeClient>,
        ledger: Option<PinLedger>,
        groups: bool,
    ) -> Self {
        let fake = Rc::new(RefCell::new(Fake {
            clients,
            sessions: sessions.iter().map(|s| s.to_string()).collect(),
            windows: windows.iter().map(|s| s.to_string()).collect(),
            panes: Vec::new(),
            marks: Vec::new(),
            groups,
            ..Fake::default()
        }));
        let mut rig = Rig::new(Config {
            renders_output: false,
            ..Config::default()
        });
        if let Some(ledger) = ledger {
            rig.with(|c, _| c.inherit_pin_ledger(ledger));
        }
        rig.respond(responder(fake.clone()));
        rig.attach();
        rig.settle();
        Self { rig, fake }
    }

    /// One client count, as the hub's attach check runs it.
    fn count(&mut self) -> i64 {
        let request = self.rig.with(|c, now| c.count_non_control_clients(now));
        self.rig.settle();
        self.rig
            .notes()
            .iter()
            .find_map(|n| match n {
                Notification::ClientCount { request: r, count } if *r == request => Some(*count),
                _ => None,
            })
            .unwrap()
    }

    /// The phone's renderer attaches, known by its pid (mosh-server's child,
    /// read over SSH), as the session tells the controller (B84: no count
    /// names it).
    fn attach_renderer(&mut self, renderer: FakeClient) {
        let pid = renderer.pid;
        self.fake.borrow_mut().clients.push(renderer);
        self.rig.with(|c, now| c.expect_renderer_pid(pid, now));
        self.rig.settle();
    }

    fn switch_session(&mut self, session: &str) -> Vec<String> {
        self.rig.with(|c, now| c.select_session(session, now));
        self.rig.settle();
        self.rig.commands.clone()
    }

    fn land(&mut self, session: &str, name: &str) {
        self.rig
            .push(&format!("%session-changed {session} {name}\n"));
        self.rig.settle();
    }

    /// The `status` options set and unset (the stamps beside them aside).
    fn status_commands(&self) -> Vec<String> {
        self.rig
            .commands
            .iter()
            .filter(|c| {
                c.starts_with("set-option")
                    && c.contains("status")
                    && !c.contains("@moshpit_status_hidden")
            })
            .cloned()
            .collect()
    }

    /// The `@moshpit_*` session marks set and unset (B86).
    fn mark_commands(&self) -> Vec<String> {
        self.rig
            .commands
            .iter()
            .filter(|c| {
                c.starts_with("set-option")
                    && (c.contains("@moshpit_owned") || c.contains("@moshpit_status_hidden"))
            })
            .cloned()
            .collect()
    }

    fn resizes(&self) -> Vec<String> {
        self.rig
            .commands
            .iter()
            .filter(|c| c.starts_with("resize-window") || c.starts_with("refresh-client -C"))
            .cloned()
            .collect()
    }

    fn moves(commands: &[String]) -> Vec<&String> {
        commands
            .iter()
            .filter(|c| c.starts_with("switch-client -c "))
            .collect()
    }
}

fn sidecar_client() -> FakeClient {
    client(true, 45613, "$1", "a|b", "client-45613")
}
fn renderer() -> FakeClient {
    client(false, 45533, "$1", "a|b", "/dev/ttys026")
}
fn desktop() -> FakeClient {
    client(false, 41001, "$0", "alpha", "/dev/ttys009")
}

// MARK: - Killing the renderer's session (B583)

/// mosh's renderer is mosh's own `exec tmux attach`: tmux detaches a
/// client whose session is destroyed (`detach-on-destroy`), which ended the
/// mosh session. It moves first — this client too — and the kill follows.
/// The live proof is `live_group.rs`.
#[test]
fn killing_the_renderers_session_moves_the_renderer_first() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 2 a|b", "$2 0 main"],
        &[],
        vec![desktop(), sidecar_client()],
        None,
    );
    s.attach_renderer(renderer());
    let from = s.rig.commands.len();
    s.rig.with(|c, now| c.kill_session("$1", now));
    let sent = s.rig.since(from).to_vec();
    let at = |prefix: &str| sent.iter().position(|c| c == prefix);
    let kill = at("kill-session -t $1").expect("the kill");
    let renderer = at("switch-client -c /dev/ttys026 -t $0").expect("the renderer moves");
    let own = at("switch-client -t $0").expect("this client moves");
    assert!(renderer < kill && own < kill, "{sent:?}");
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
}

/// Another session's kill moves nobody.
#[test]
fn killing_another_session_leaves_the_renderer_alone() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 2 a|b", "$2 0 main"],
        &[],
        vec![desktop(), sidecar_client()],
        None,
    );
    s.attach_renderer(renderer());
    let from = s.rig.commands.len();
    s.rig.with(|c, now| c.kill_session("$2", now));
    s.rig.settle();
    assert!(
        !s.rig
            .since(from)
            .iter()
            .any(|c| c.starts_with("switch-client")),
        "{:?}",
        s.rig.since(from)
    );
}

// MARK: - Session ids, not names

#[test]
fn a_renderer_on_a_session_named_with_a_pipe_comes_along() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 2 a|b", "$2 0 main"],
        &[],
        vec![desktop(), sidecar_client()],
        None,
    );
    s.attach_renderer(renderer());
    s.rig.settle();
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
    let commands = s.switch_session("$2");
    assert!(
        commands
            .iter()
            .any(|c| c.starts_with("switch-client -c /dev/ttys026 -t $2"))
    );
    assert!(!commands.iter().any(|c| c.contains("b|/dev/")));
    assert!(!commands.iter().any(|c| c.contains("/dev/ttys009")));
}

#[test]
fn a_rename_reaches_the_snapshot_and_the_switch_still_finds_the_renderer() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 2 a|b", "$2 0 main"],
        &[],
        vec![desktop(), sidecar_client()],
        None,
    );
    s.attach_renderer(renderer());
    s.rig.push("%session-renamed $1 x|y\n");
    for c in &mut s.fake.borrow_mut().clients {
        if c.session_id == "$1" {
            c.session_name = "x|y".into();
        }
    }
    assert_eq!(s.rig.controller.snapshot().sessions["$1"].name, "x|y");
    let commands = s.switch_session("$2");
    assert!(
        commands
            .iter()
            .any(|c| c.starts_with("switch-client -c /dev/ttys026 -t $2"))
    );
    assert!(!commands.iter().any(|c| c.contains("/dev/ttys009")));
}

#[test]
fn an_owned_session_gets_its_status_back_by_id() {
    let mut s = Sidecar::make(&["$1 2 a|b"], &[], vec![sidecar_client(), renderer()], None);
    s.fake.borrow_mut().sessions.push("$5 0 mine".into());
    s.rig.with(|c, now| c.new_session(Some("mine"), now));
    s.rig.settle();
    assert!(
        s.status_commands()
            .iter()
            .any(|c| c.contains("$5") && c.contains("status off"))
    );
    s.rig.push("%session-renamed $5 renamed\n");
    let request = s
        .rig
        .with(|c, now| c.restore_suppressed_status(now))
        .unwrap();
    s.rig.settle();
    assert!(s.rig.notes().contains(&Notification::Flushed { request }));
    assert!(s.rig.has("set-option -t '$5' -u status"));
    assert!(!s.rig.commands.iter().any(|c| c.contains("-t 'mine'")));
}

// MARK: - Only the phone's own renderer

fn work(control: bool, pid: i64, name: &str) -> FakeClient {
    client(control, pid, "$1", "work", name)
}

#[test]
fn a_desktop_on_the_renderers_session_stays() {
    let mut s = Sidecar::make(
        &["$0 0 alpha", "$1 3 work", "$2 0 main"],
        &[],
        vec![
            work(false, 41001, "/dev/ttys009"),
            work(true, 45613, "/dev/ttys030"),
        ],
        None,
    );
    s.attach_renderer(work(false, 45533, "/dev/ttys026"));
    s.rig.settle();
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
    let commands = s.switch_session("$2");
    assert_eq!(
        Sidecar::moves(&commands),
        ["switch-client -c /dev/ttys026 -t $2"]
    );
}

#[test]
fn with_no_session_known_only_the_renderer_comes() {
    let mut s = Sidecar::make(
        &["$0 0 alpha", "$1 0 work", "$2 0 main"],
        &[],
        vec![
            client(false, 41002, "$0", "alpha", "/dev/ttys010"),
            work(false, 41001, "/dev/ttys009"),
            work(true, 45613, "/dev/ttys030"),
        ],
        None,
    );
    assert_eq!(s.rig.controller.snapshot().active_session_id, None);
    s.attach_renderer(work(false, 45533, "/dev/ttys026"));
    let commands = s.switch_session("$2");
    assert_eq!(
        Sidecar::moves(&commands),
        ["switch-client -c /dev/ttys026 -t $2"]
    );
    assert!(
        !commands
            .iter()
            .any(|c| c.contains("/dev/ttys009") || c.contains("/dev/ttys010"))
    );
}

#[test]
fn an_unknown_renderer_moves_no_client() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 3 work", "$2 0 main"],
        &[],
        vec![
            client(false, 41002, "$0", "alpha", "/dev/ttys010"),
            work(false, 41001, "/dev/ttys009"),
            work(true, 45613, "/dev/ttys030"),
            work(false, 45533, "/dev/ttys026"),
        ],
        None,
    );
    let commands = s.switch_session("$2");
    assert!(Sidecar::moves(&commands).is_empty());
    assert!(
        commands
            .iter()
            .any(|c| c.starts_with("switch-client -t $2"))
    );
}

#[test]
fn a_client_that_got_the_renderers_tty_is_not_the_renderer() {
    let mut s = Sidecar::make(
        &["$0 0 alpha", "$1 2 work", "$2 0 main"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        None,
    );
    s.attach_renderer(work(false, 45533, "/dev/ttys026"));
    s.fake.borrow_mut().clients = vec![
        work(true, 45613, "/dev/ttys030"),
        work(false, 50001, "/dev/ttys026"),
    ];
    let commands = s.switch_session("$2");
    assert!(Sidecar::moves(&commands).is_empty());
}

#[test]
fn two_clients_between_two_counts_are_neither_taken() {
    let mut s = Sidecar::make(
        &["$0 0 alpha", "$1 3 work", "$2 0 main"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        None,
    );
    assert_eq!(s.count(), 0);
    s.fake.borrow_mut().clients.extend([
        work(false, 45533, "/dev/ttys026"),
        work(false, 41001, "/dev/ttys009"),
    ]);
    assert_eq!(s.count(), 2);
    let commands = s.switch_session("$2");
    assert!(Sidecar::moves(&commands).is_empty());
}

#[test]
fn a_rebuilt_sidecar_still_knows_the_renderer() {
    let mut first = Sidecar::make(
        &["$0 0 alpha", "$1 3 work", "$2 0 main"],
        &[],
        vec![
            work(false, 41001, "/dev/ttys009"),
            work(true, 45613, "/dev/ttys030"),
        ],
        None,
    );
    first.attach_renderer(work(false, 45533, "/dev/ttys026"));
    let ledger = first.rig.controller.take_pin_ledger();
    let mut second = Sidecar::make(
        &["$0 0 alpha", "$1 3 work", "$2 0 main"],
        &[],
        vec![
            work(false, 41001, "/dev/ttys009"),
            work(true, 45999, "/dev/ttys031"),
            work(false, 45533, "/dev/ttys026"),
        ],
        Some(ledger),
    );
    let commands = second.switch_session("$2");
    assert_eq!(
        Sidecar::moves(&commands),
        ["switch-client -c /dev/ttys026 -t $2"]
    );
    let before = first.rig.commands.len();
    let retired = first.switch_session("$0");
    assert!(Sidecar::moves(&retired[before..]).is_empty());
}

// MARK: - Which session the sidecar is on

fn aaa(control: bool, pid: i64, name: &str) -> FakeClient {
    client(control, pid, "$0", "aaa", name)
}
fn zzz_desktop() -> FakeClient {
    client(false, 41001, "$1", "zzz", "/dev/ttys009")
}
const AAA_ZZZ: [&str; 2] = [
    "$0 @0 0 b25d,80x24,0,0,0 1 1 zsh",
    "$1 @1 0 b25e,200x50,0,0,1 1 1 vim",
];

fn relist(s: &mut Sidecar) {
    s.rig.with(|c, now| c.refresh(now));
    s.rig.settle();
}

#[test]
fn a_relisting_keeps_the_sidecar_on_the_phones_session() {
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$1 1 zzz"],
        &AAA_ZZZ,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    s.land("$0", "aaa");
    s.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    relist(&mut s);
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
    assert_eq!(
        s.rig.controller.snapshot().active_window_id.as_deref(),
        Some("@0")
    );
    s.rig.with(|c, now| c.sync_mosh_window(40, 20, now));
    s.rig.settle();
    assert_eq!(s.resizes(), ["resize-window -t @0 -x 40 -y 20"]);
}

#[test]
fn the_renderer_moving_by_itself_takes_the_sidecar_along() {
    let windows = [AAA_ZZZ[0], AAA_ZZZ[1], "$2 @2 0 b25f,80x24,0,0,2 1 1 htop"];
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$2 1 mmm", "$1 1 zzz"],
        &windows,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    s.land("$0", "aaa");
    s.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    s.rig.settle();
    for c in &mut s.fake.borrow_mut().clients {
        if c.pid == 45533 {
            c.session_id = "$2";
            c.session_name = "mmm".into();
        }
    }
    s.rig.with(|c, now| c.refresh(now));
    s.rig.settle();
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$2")
    );
    assert!(s.rig.commands.contains(&"switch-client -t $2".to_owned()));
    assert!(Sidecar::moves(&s.rig.commands).is_empty());
    s.rig.with(|c, now| c.sync_mosh_window(40, 20, now));
    s.rig.settle();
    assert_eq!(s.resizes(), ["resize-window -t @2 -x 40 -y 20"]);
}

#[test]
fn an_unidentified_renderer_follows_nothing_and_sizes_nothing() {
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$1 1 zzz"],
        &AAA_ZZZ,
        vec![
            aaa(true, 45613, "/dev/ttys030"),
            zzz_desktop(),
            aaa(false, 45533, "/dev/ttys026"),
        ],
        None,
    );
    s.land("$0", "aaa");
    relist(&mut s);
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
    s.rig.with(|c, now| c.sync_mosh_window(40, 20, now));
    s.rig.with(|c, now| c.repin_active_window(now));
    s.rig.settle();
    assert!(s.resizes().is_empty());
}

#[test]
fn a_detached_renderer_sizes_nothing_any_more() {
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$1 1 zzz"],
        &AAA_ZZZ,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    s.land("$0", "aaa");
    s.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    s.rig.settle();
    s.fake.borrow_mut().clients.retain(|c| c.pid != 45533);
    s.rig
        .push("%client-detached /dev/ttys026\n%session-renamed $0 aaa2\n");
    s.rig.with(|c, now| c.sync_mosh_window(40, 20, now));
    s.rig.settle();
    assert!(s.resizes().is_empty());
}

#[test]
fn a_rebuilt_sidecar_finds_the_renderer_before_it_sizes() {
    let mut first = Sidecar::make(
        &["$0 1 aaa", "$1 1 zzz"],
        &AAA_ZZZ,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    first.land("$0", "aaa");
    first.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    first.rig.settle();
    let ledger = first.rig.controller.take_pin_ledger();
    let rebuilt = client(true, 45999, "$1", "zzz", "/dev/ttys031");
    let mut second = Sidecar::make(
        &["$0 1 aaa", "$1 2 zzz"],
        &AAA_ZZZ,
        vec![rebuilt, zzz_desktop(), aaa(false, 45533, "/dev/ttys026")],
        Some(ledger),
    );
    second.land("$1", "zzz");
    let snapshot = second.rig.controller.snapshot();
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
    assert!(
        second
            .rig
            .commands
            .contains(&"switch-client -t $0".to_owned())
    );
    second.rig.with(|c, now| c.sync_mosh_window(40, 20, now));
    second.rig.settle();
    assert_eq!(second.resizes(), ["resize-window -t @0 -x 40 -y 20"]);
}

#[test]
fn a_switch_is_not_undone_by_a_listing_sent_before_its_moves() {
    let windows = [AAA_ZZZ[0], AAA_ZZZ[1], "$2 @2 0 b25f,80x24,0,0,2 1 1 htop"];
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$2 0 mmm", "$1 1 zzz"],
        &windows,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    s.land("$0", "aaa");
    s.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    s.rig.settle();
    let before = s.rig.commands.len();
    s.rig.with(|c, now| c.refresh(now));
    s.rig.with(|c, now| c.select_session("$2", now));
    s.rig.settle();
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$2")
    );
    let switches: Vec<&String> = s
        .rig
        .since(before)
        .iter()
        .filter(|c| c.starts_with("switch-client"))
        .collect();
    assert_eq!(
        switches,
        ["switch-client -c /dev/ttys026 -t $2", "switch-client -t $2"]
    );
}

// MARK: - The rest of the server

#[test]
fn a_session_created_elsewhere_is_listed_once_for_the_burst() {
    let mut s = Sidecar::make(
        &["$0 1 aaa"],
        &[AAA_ZZZ[0]],
        vec![aaa(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$0", "aaa");
    let before = s.rig.commands.len();
    s.fake.borrow_mut().sessions.push("$2 0 third".into());
    s.fake
        .borrow_mut()
        .windows
        .push("$2 @5 0 b25f,80x24,0,0,5 1 1 tmux".into());
    s.rig
        .push("%unlinked-window-add @5\n%sessions-changed\n%unlinked-window-renamed @5 tmux\n");
    let sent = |s: &Sidecar, prefix: &str| {
        s.rig
            .since(before)
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    };
    assert_eq!(sent(&s, "list-sessions"), 1);
    assert_eq!(sent(&s, "list-windows"), 1);
    s.rig.settle();
    let snapshot = s.rig.controller.snapshot();
    assert_eq!(snapshot.sessions["$2"].name, "third");
    assert_eq!(snapshot.windows["@5"].session_id, "$2");
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
}

#[test]
fn a_session_killed_elsewhere_goes_and_its_window_at_once() {
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$2 0 third"],
        &[AAA_ZZZ[0], "$2 @5 0 b25f,80x24,0,0,5 1 1 bash"],
        vec![aaa(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$0", "aaa");
    assert!(s.rig.controller.snapshot().windows.contains_key("@5"));
    s.fake
        .borrow_mut()
        .sessions
        .retain(|x| !x.starts_with("$2 "));
    s.fake
        .borrow_mut()
        .windows
        .retain(|x| !x.starts_with("$2 "));
    s.rig
        .push("%unlinked-window-renamed @5 bash\n%sessions-changed\n%unlinked-window-close @5\n");
    assert!(!s.rig.controller.snapshot().windows.contains_key("@5"));
    s.rig.settle();
    assert!(!s.rig.controller.snapshot().sessions.contains_key("$2"));
    assert!(s.rig.controller.snapshot().sessions.contains_key("$0"));
}

#[test]
fn a_window_renamed_in_another_session_is_renamed_here() {
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$2 0 other"],
        &[AAA_ZZZ[0], "$2 @4 0 b25f,80x24,0,0,4 1 1 o2"],
        vec![aaa(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$0", "aaa");
    s.rig.push("%unlinked-window-renamed @4 o2renamed\n");
    assert_eq!(s.rig.controller.snapshot().windows["@4"].name, "o2renamed");
}

#[test]
fn the_window_on_screen_killed_lands_where_tmux_did() {
    let mut s = Sidecar::make(
        &["$0 1 aaa"],
        &[
            "$0 @0 0 b25d,80x24,0,0,0 0 1 zsh",
            "$0 @2 1 b25e,80x24,0,0,2 0 1 htop",
            "$0 @1 2 b25f,80x24,0,0,1 1 1 vim",
        ],
        vec![aaa(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$0", "aaa");
    assert_eq!(
        s.rig.controller.snapshot().active_window_id.as_deref(),
        Some("@1")
    );
    s.fake.borrow_mut().windows = vec![
        "$0 @0 0 b25d,80x24,0,0,0 0 1 zsh".into(),
        "$0 @2 1 b25e,80x24,0,0,2 1 1 htop".into(),
    ];
    s.rig
        .push("%session-window-changed $0 @2\n%unlinked-window-close @1\n");
    assert!(!s.rig.controller.snapshot().windows.contains_key("@1"));
    s.rig.settle();
    assert_eq!(
        s.rig.controller.snapshot().active_window_id.as_deref(),
        Some("@2")
    );
}

/// Fix (sidecar-current-window, B80): over mosh the screen is tmux's current
/// window, so the breadcrumb follows `prefix n` in the TUI and another
/// client's `select-window` on the renderer's session.
#[test]
fn the_sidecar_follows_tmuxs_current_window() {
    let mut s = Sidecar::make(
        &["$0 1 aaa"],
        &[
            "$0 @0 0 b25d,80x24,0,0,0 1 1 zsh",
            "$0 @2 1 b25e,80x24,0,0,2 0 1 htop",
        ],
        vec![aaa(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$0", "aaa");
    assert_eq!(
        s.rig.controller.snapshot().active_window_id.as_deref(),
        Some("@0")
    );
    s.rig.push("%session-window-changed $0 @2\n");
    assert_eq!(
        s.rig.controller.snapshot().active_window_id.as_deref(),
        Some("@2")
    );
    // Another session's window change is still not ours.
    s.rig.push("%session-window-changed $7 @0\n");
    assert_eq!(
        s.rig.controller.snapshot().active_window_id.as_deref(),
        Some("@2")
    );
}

#[test]
fn the_renderer_switching_by_itself_is_followed_from_the_notification() {
    let windows = [AAA_ZZZ[0], AAA_ZZZ[1], "$2 @2 0 b25f,80x24,0,0,2 1 1 htop"];
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$2 0 mmm", "$1 1 zzz"],
        &windows,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    s.land("$0", "aaa");
    s.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    s.rig.settle();
    for c in &mut s.fake.borrow_mut().clients {
        if c.pid == 45533 {
            c.session_id = "$2";
            c.session_name = "mmm".into();
        }
    }
    s.fake.borrow_mut().sessions = vec!["$0 1 aaa".into(), "$2 1 mmm".into(), "$1 1 zzz".into()];
    s.rig.push("%client-session-changed /dev/ttys026 $2 mmm\n");
    s.rig.settle();
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$2")
    );
    assert!(s.rig.commands.contains(&"switch-client -t $2".to_owned()));
}

#[test]
fn another_client_switching_is_relisted_and_not_followed() {
    let windows = [AAA_ZZZ[0], AAA_ZZZ[1], "$2 @2 0 b25f,80x24,0,0,2 1 1 htop"];
    let mut s = Sidecar::make(
        &["$0 1 aaa", "$2 0 mmm", "$1 1 zzz"],
        &windows,
        vec![aaa(true, 45613, "/dev/ttys030"), zzz_desktop()],
        None,
    );
    s.land("$0", "aaa");
    s.attach_renderer(aaa(false, 45533, "/dev/ttys026"));
    s.rig.settle();
    s.fake.borrow_mut().sessions = vec!["$0 1 aaa".into(), "$2 1 mmm".into(), "$1 0 zzz".into()];
    s.rig.push("%client-session-changed /dev/ttys009 $2 mmm\n");
    s.rig.settle();
    let snapshot = s.rig.controller.snapshot();
    assert!(snapshot.sessions["$2"].is_attached);
    assert!(!snapshot.sessions["$1"].is_attached);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
}

#[test]
fn a_burst_costs_one_listing_of_each_kind_and_one_more_after() {
    let mut s = Sidecar::make(
        &["$0 1 aaa"],
        &[AAA_ZZZ[0]],
        vec![aaa(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$0", "aaa");
    let before = s.rig.commands.len();
    s.rig.push("%sessions-changed\n%client-session-changed /dev/ttys009 $0 aaa\n%sessions-changed\n%unlinked-window-add @7\n%unlinked-window-add @8\n%unlinked-window-add @9\n%window-renamed @0 marker\n");
    assert_eq!(s.rig.controller.snapshot().windows["@0"].name, "marker");
    let sent = |s: &Sidecar, prefix: &str| {
        s.rig
            .since(before)
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    };
    assert_eq!(sent(&s, "list-sessions"), 1);
    assert_eq!(sent(&s, "list-windows"), 1);
    s.rig.settle();
    assert_eq!(sent(&s, "list-sessions"), 2);
    assert_eq!(sent(&s, "list-windows"), 2);
}

// MARK: - The status line

#[test]
fn the_users_sessions_get_no_status_option() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 2 work"],
        &[],
        vec![
            client(false, 41002, "$0", "alpha", "/dev/ttys010"),
            work(true, 45613, "/dev/ttys030"),
        ],
        None,
    );
    s.land("$1", "work");
    s.switch_session("$0");
    s.land("$0", "alpha");
    assert!(s.status_commands().is_empty());
    let request = s.rig.with(|c, now| c.restore_suppressed_status(now));
    assert_eq!(request, None, "the disconnect owes nothing either");
    assert!(s.status_commands().is_empty());
}

#[test]
fn an_owned_session_goes_without_status_while_on_it_and_gets_it_back() {
    let mut s = Sidecar::make(
        &["$1 2 work"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$1", "work");
    s.fake.borrow_mut().sessions.push("$5 0 mine".into());
    s.rig.with(|c, now| c.new_session(Some("mine"), now));
    s.rig.settle();
    s.land("$5", "mine");
    assert_eq!(s.status_commands(), ["set-option -t '$5' status off"]);
    s.switch_session("$1");
    s.land("$1", "work");
    assert_eq!(
        s.status_commands(),
        [
            "set-option -t '$5' status off",
            "set-option -t '$5' -u status"
        ]
    );
}

#[test]
fn a_rebuilt_sidecar_still_owes_the_status_line() {
    let mut first = Sidecar::make(
        &["$1 2 work"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        None,
    );
    first.land("$1", "work");
    first.fake.borrow_mut().sessions.push("$5 0 mine".into());
    first.rig.with(|c, now| c.new_session(Some("mine"), now));
    first.rig.settle();
    assert!(
        first
            .status_commands()
            .iter()
            .any(|c| c.contains("status off"))
    );
    let ledger = first.rig.controller.take_pin_ledger();
    let mut second = Sidecar::make(
        &["$1 2 work", "$5 0 mine"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        Some(ledger),
    );
    second.land("$1", "work");
    assert_eq!(second.status_commands(), ["set-option -t '$5' -u status"]);
}

#[test]
fn a_rebuilt_sidecar_on_that_session_keeps_it_off_until_the_disconnect() {
    let mut first = Sidecar::make(
        &["$1 2 work"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        None,
    );
    first.land("$1", "work");
    first.fake.borrow_mut().sessions.push("$5 0 mine".into());
    first.rig.with(|c, now| c.new_session(Some("mine"), now));
    first.rig.settle();
    let ledger = first.rig.controller.take_pin_ledger();
    let mut second = Sidecar::make(
        &["$1 1 work", "$5 1 mine"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        Some(ledger),
    );
    second.land("$5", "mine");
    assert!(second.status_commands().is_empty());
    second.rig.with(|c, now| c.restore_suppressed_status(now));
    second.rig.settle();
    assert_eq!(second.status_commands(), ["set-option -t '$5' -u status"]);
}

// MARK: - The renderer by its pid

#[test]
fn the_renderer_is_known_by_its_pid_whoever_else_attached() {
    // A desktop on another session and the phone's renderer, both there
    // before any count: counting around an attach can't tell them apart
    // (B84); mosh's program, known by its parent's pid, can.
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 1 work", "$2 0 main"],
        &[],
        vec![
            desktop(),
            work(true, 45613, "client-45613"),
            work(false, 45533, "/dev/ttys026"),
        ],
        None,
    );
    s.rig.with(|c, now| c.expect_renderer_pid(45533, now));
    s.rig.settle();
    assert_eq!(
        s.rig.controller.renderer_client().map(|c| c.name.as_str()),
        Some("/dev/ttys026")
    );
    assert_eq!(s.rig.controller.renderer_session_id(), Some("$1"));
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1"),
        "the sidecar follows the renderer"
    );
    let commands = s.switch_session("$2");
    assert_eq!(
        Sidecar::moves(&commands),
        ["switch-client -c /dev/ttys026 -t $2"],
        "only the renderer moves, never the desktop"
    );
}

/// B84, the very race: the hub counts clients, the renderer's attach
/// fails, a desktop attaches in the same second, the hub counts again —
/// one client "appeared" and counting took the desktop for the renderer
/// (a session switch then moved the desktop's terminal). On the core the
/// renderer is known by its pid (mosh-server's child) and nothing counts:
/// the session never asks for a count (`count_non_control_clients` has no
/// caller outside the harness), and even a count taken around the attach
/// decides nothing once the pid is known — the renderer that never came
/// is nobody, and the desktop is never touched.
#[test]
fn a_desktop_attaching_while_the_renderers_attach_fails_is_never_the_renderer() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 0 work"],
        &[],
        vec![work(true, 45613, "client-45613")],
        None,
    );
    // mosh-server's child, read over SSH: its attach is about to fail.
    s.rig.with(|c, now| c.expect_renderer_pid(51000, now));
    s.rig.settle();
    s.count();
    s.fake.borrow_mut().clients.push(desktop());
    assert_eq!(s.count(), 1, "the desktop is the one new client");
    assert_eq!(s.rig.controller.renderer_client(), None);
    let commands = s.switch_session("$1");
    assert!(
        Sidecar::moves(&commands).is_empty(),
        "nothing moves the desktop: {:?}",
        Sidecar::moves(&commands)
    );
    s.rig.with(|c, now| c.sync_mosh_window(70, 35, now));
    assert!(
        !s.rig.has("resize-window"),
        "no window is the renderer's to size"
    );
}

#[test]
fn a_pid_no_client_has_names_nobody() {
    let mut s = Sidecar::make(
        &["$0 1 alpha", "$1 0 work"],
        &[],
        vec![desktop(), work(true, 45613, "client-45613")],
        None,
    );
    s.rig.with(|c, now| c.expect_renderer_pid(777, now));
    s.rig.settle();
    assert_eq!(s.rig.controller.renderer_client(), None);
    // Nothing to size: no window is the renderer's.
    s.rig.with(|c, now| c.sync_mosh_window(70, 35, now));
    assert!(!s.rig.has("resize-window"));
    let commands = s.switch_session("$1");
    assert!(Sidecar::moves(&commands).is_empty());
}

// MARK: - The layouts a session switch squashes (round thirty-four)

const WIDE_ONE: &str = "bf8c,200x60,0,0[200x30,0,0,0,200x29,0,31{100x29,0,31,1,99x29,101,31,2}]";
const SQUASHED_ONE: &str = "307e,60x20,0,0[60x10,0,0,0,60x9,0,11{30x9,0,11,1,29x9,31,11,2}]";
const WIDE_TWO: &str = "c1a2,200x60,0,0{100x60,0,0,3,99x60,101,0[99x30,101,0,4,99x29,101,31,5]}";
const SQUASHED_TWO: &str = "51b0,60x20,0,0{30x20,0,0,3,29x20,31,0[29x10,31,0,4,29x9,31,11,5]}";

/// Two sessions of a 3-pane window each, the phone's renderer on `$1`: a
/// window is at the phone's grid while the renderer sits on its session.
fn two_split_sessions() -> (Sidecar, Rc<RefCell<Vec<String>>>) {
    split_sessions_with(false, Rc::new(RefCell::new(true)))
}

fn split_sessions_with(
    groups: bool,
    show_target: Rc<RefCell<bool>>,
) -> (Sidecar, Rc<RefCell<Vec<String>>>) {
    let fake = Rc::new(RefCell::new(Fake {
        clients: vec![
            work(true, 45613, "client-45613"),
            work(false, 45533, "/dev/ttys026"),
        ],
        sessions: vec!["$1 1 work".into(), "$2 0 play".into()],
        windows: Vec::new(),
        panes: Vec::new(),
        marks: Vec::new(),
        groups,
        ..Fake::default()
    }));
    let on = |fake: &Fake, session: &str| {
        fake.clients.iter().any(|c| {
            !c.control && (c.session_id == session || fake.group_of(c.session_id) == session)
        })
    };
    let replays = Rc::new(RefCell::new(Vec::new()));
    let seen = replays.clone();
    let mut inner = responder(fake.clone());
    let state = fake.clone();
    let target_visible = show_target;
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.respond(Box::new(move |command, at| {
        {
            let fake = state.borrow();
            let one = if on(&fake, "$1") {
                SQUASHED_ONE
            } else {
                WIDE_ONE
            };
            let two = if on(&fake, "$2") {
                SQUASHED_TWO
            } else {
                WIDE_TWO
            };
            if command.starts_with("list-windows -t '$2' -F '#{window_id} ") {
                return ok([format!("@2 3 0 %3 {two}")]);
            }
            if command.starts_with("list-windows -t '$1' -F '#{window_id} ") {
                return ok([format!("@1 3 0 %0 {one}")]);
            }
            if command.starts_with("list-windows -a -F '#{window_id} ") {
                return ok([format!("@1 3 0 %0 {one}"), format!("@2 3 0 %3 {two}")]);
            }
            if command.starts_with("list-windows") {
                let mut rows = vec![format!("$1 @1 0 {one} 1 3 one")];
                if *target_visible.borrow() {
                    rows.push(format!("$2 @2 0 {two} 1 3 two"));
                }
                return ok(rows);
            }
            if command.starts_with("display-message -p -t '@1' '#{window_layout}'") {
                return ok([one]);
            }
            if command.starts_with("display-message -p -t '@1' '") {
                return ok([format!("3 0 %0 {one}")]);
            }
        }
        if command.starts_with("select-layout") {
            seen.borrow_mut().push(command.to_owned());
        }
        inner(command, at)
    }));
    // The renderer had squashed `$1` before the sidecar existed (B270).
    rig.with(|c, _| {
        c.note_layouts_before_renderer(&format!("@1 3 0 %0 {WIDE_ONE}\n@2 3 0 %3 {WIDE_TWO}\n"))
    });
    rig.attach();
    rig.settle();
    rig.push("%session-changed $1 work\n");
    rig.settle();
    rig.with(|c, now| c.expect_renderer_pid(45533, now));
    rig.settle();
    (Sidecar { rig, fake }, replays)
}

/// Round thirty-four: moving the renderer to another session sizes that
/// session's windows by it (with the phone alone, every one at the phone's
/// grid) — and the sidecar recorded none of them first, so the disconnect
/// gave them back squashed. The windows are read on both sides of the move;
/// what the move resized keeps its earlier layout as its record.
#[test]
fn a_session_switch_records_the_layouts_the_renderers_move_squashes() {
    let (mut s, _) = two_split_sessions();
    assert_eq!(s.rig.controller.renderer_session_id(), Some("$1"));
    let from = s.rig.commands.len();
    s.switch_session("$2");
    let commands = s.rig.since(from).to_vec();
    let probe = |c: &String| c.starts_with("list-windows -t '$2' -F '#{window_id} ");
    let probes: Vec<usize> = (0..commands.len())
        .filter(|&i| probe(&commands[i]))
        .collect();
    let moved = commands
        .iter()
        .position(|c| c.starts_with("switch-client -c /dev/ttys026 -t $2"))
        .unwrap();
    assert_eq!(probes.len(), 2, "{commands:?}");
    assert!(probes[0] < moved && moved < probes[1], "{commands:?}");
    let ledger = s.rig.controller.take_pin_ledger();
    assert_eq!(
        ledger.pristine_layouts.get("@2").map(|p| p.layout.as_str()),
        Some(WIDE_TWO),
        "{ledger:?}"
    );
}

/// The session the renderer left is nobody's of the phone's any more: its
/// pinned window goes back to automatic sizing, and its records are
/// replayed (after the move, never before it) and consumed once they took.
#[test]
fn a_session_switch_hands_the_session_left_back() {
    let (mut s, replays) = two_split_sessions();
    s.rig.with(|c, now| c.sync_mosh_window(60, 20, now));
    s.rig.settle();
    assert!(s.rig.has("resize-window -t @1 -x 60 -y 20"));
    let from = s.rig.commands.len();
    s.switch_session("$2");
    let commands = s.rig.since(from).to_vec();
    let moved = commands
        .iter()
        .position(|c| c.starts_with("switch-client -c /dev/ttys026 -t $2"))
        .unwrap();
    let unpin = commands
        .iter()
        .position(|c| c == "set-option -u -w -t @1 window-size")
        .unwrap_or_else(|| panic!("{commands:?}"));
    let replay = commands
        .iter()
        .position(|c| c == &format!("select-layout -t '@1' '{WIDE_ONE}'"))
        .unwrap_or_else(|| panic!("{commands:?}"));
    assert!(moved < unpin && unpin < replay, "{commands:?}");
    assert_eq!(replays.borrow().len(), 1);
    let ledger = s.rig.controller.take_pin_ledger();
    assert!(!ledger.pristine_layouts.contains_key("@1"), "{ledger:?}");
    assert!(!ledger.resized_windows.contains("@1"), "{ledger:?}");
}

// MARK: - The renderer lookup behind the zoom (round thirty-four)

/// Round thirty-four, the root of the late renderer: the attach's immersive
/// zoom was `if-shell -F … 'resize-pane -Z'`, and tmux answers an `if-shell`
/// that runs its command with two reply blocks (its own, then the
/// command's; 3.6a, a private server). Replies are matched to commands in
/// send order: the renderer lookup sent right behind the zoom got the zoom's
/// empty block — renderer not found, its own reply dropped — and no window
/// was ever pinned. The fake answers as tmux does.
#[test]
fn the_renderer_lookup_right_behind_the_attachs_zoom_gets_its_own_reply() {
    const SPLIT: &str = "1ac3,60x19,0,0[60x17,0,0,0,60x1,0,18,1]";
    let fake = Rc::new(RefCell::new(Fake {
        clients: vec![
            work(false, 81444, "/dev/ttys033"),
            work(true, 81449, "/dev/ttys034"),
        ],
        sessions: vec!["$1 2 work".into()],
        windows: vec![format!("$1 @0 0 {SPLIT} 1 2 bash")],
        panes: Vec::new(),
        marks: Vec::new(),
        ..Fake::default()
    }));
    let mut tmux = responder(fake.clone());
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.with(|c, now| c.begin_control_mode(now));
    rig.boot();
    rig.push("%session-changed $1 work\n");
    // Answered one by one, as tmux would: the zoom probe says "a split,
    // unzoomed"; an `if-shell` that runs its command answers twice.
    let mut expected = false;
    let mut block = 500;
    while rig.answered < rig.commands.len() {
        let command = rig.commands[rig.answered].clone();
        if !expected && (command.starts_with("if-shell ") || command.starts_with("resize-pane -Z"))
        {
            // The pid arrives while the zoom is on its way: its lookup goes
            // out right behind it.
            expected = true;
            rig.with(|c, now| c.expect_renderer_pid(81444, now));
        }
        let lines = if command.starts_with("display-message -p -t '@0' '#{window_panes} ") {
            vec![format!("2 0 %0 {SPLIT}")]
        } else {
            tmux(&command, rig.answered)
                .map(|a| a.lines)
                .unwrap_or_default()
        };
        rig.answered += 1;
        block += 1;
        let mut text = moshpit_tmux::rig::block(block, &lines, false);
        if command.starts_with("if-shell ") {
            block += 1;
            text.push_str(&moshpit_tmux::rig::block(block, &[], false));
        }
        rig.push(&text);
    }
    assert!(expected, "no zoom: {:?}", rig.commands);
    assert_eq!(
        rig.controller.renderer_client().map(|c| c.name.as_str()),
        Some("/dev/ttys033"),
        "{:?}",
        rig.commands
    );
    assert_eq!(rig.controller.renderer_session_id(), Some("$1"));
}

/// The zoom is a plain toggle now, decided on our side: a probe older than
/// a toggle we sent reads "unzoomed" for a window that is zoomed by then,
/// and asks again instead of toggling it back.
#[test]
fn a_zoom_probe_older_than_our_toggle_asks_again() {
    const SPLIT: &str = "1ac3,60x19,0,0[60x17,0,0,0,60x1,0,18,1]";
    let zoomed = Rc::new(RefCell::new(false));
    let z = zoomed.clone();
    let mut rig = Rig::new(Config::default());
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("display-message -p -t '@0' '#{window_panes} ") {
            return ok([format!("2 {} %0 {SPLIT}", u8::from(*z.borrow()))]);
        }
        if command == "resize-pane -Z -t @0" {
            let now = !*z.borrow();
            *z.borrow_mut() = now;
        }
        None
    }));
    rig.attach();
    rig.push(&format!(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 {SPLIT} 1 2 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 60 17 1 0 0 bash\n%1 @0 1 60 1 0 0 0 bash\n%end 3 3 1\n\n"
    ));
    rig.answered = rig.answered.max(3);
    // Two probes out before either is answered (the attach's and a pane
    // selection's), both answered "unzoomed".
    rig.with(|c, now| c.select_pane("%1", now));
    rig.settle();
    assert!(*zoomed.borrow(), "{:?}", rig.commands);
    assert!(!rig.commands.iter().any(|c| c.starts_with("if-shell")));
}

// MARK: - Sessions Moshpit made, known from the server (B86)

#[test]
fn a_session_moshpit_makes_is_marked_on_it_and_its_hidden_status_stamped() {
    let mut s = Sidecar::make(
        &["$1 2 work"],
        &[],
        vec![work(true, 45613, "/dev/ttys030")],
        None,
    );
    s.land("$1", "work");
    s.fake.borrow_mut().sessions.push("$5 0 mine".into());
    s.rig.with(|c, now| c.new_session(Some("mine"), now));
    s.rig.settle();
    s.land("$5", "mine");
    assert_eq!(
        s.mark_commands(),
        [
            "set-option -t '$5' @moshpit_owned 1",
            "set-option -t '$5' -F @moshpit_status_hidden '#{client_name}'",
        ]
    );
    s.switch_session("$1");
    s.land("$1", "work");
    assert_eq!(
        s.mark_commands().last().map(String::as_str),
        Some("set-option -t '$5' -u @moshpit_status_hidden")
    );
}

/// B86: the app killed while the phone sat on a session Moshpit made. Its
/// status line stayed off; the next launch's sidecar knew nothing of the
/// session (Moshpit's sessions lived in memory) and never gave it back. Now
/// the server says: the session is marked Moshpit's, and the client that
/// turned its status line off is no longer attached.
#[test]
fn a_sidecar_after_a_killed_app_gives_back_the_status_line_it_left_off() {
    let mut s = make_marked(
        &["$1 owned= hidden=", "$5 owned=1 hidden=client-99999"],
        ("$1", "work"),
    );
    assert_eq!(s.status_commands(), ["set-option -t '$5' -u status"]);
    assert!(s.rig.controller.owned_session_ids().contains("$5"));
    assert!(!s.rig.controller.owned_session_ids().contains("$1"));
    // And it is Moshpit's still: on it, the status line goes off again.
    s.switch_session("$5");
    s.land("$5", "mine");
    assert_eq!(
        s.status_commands().last().map(String::as_str),
        Some("set-option -t '$5' status off")
    );
}

/// Back on that very session after the kill: it stays off, now ours.
#[test]
fn a_sidecar_after_a_killed_app_on_that_session_keeps_it_off() {
    let mut s = make_marked(
        &["$1 owned= hidden=", "$5 owned=1 hidden=client-99999"],
        ("$5", "mine"),
    );
    assert!(
        !s.status_commands().iter().any(|c| c.contains("-u status")),
        "{:?}",
        s.status_commands()
    );
    s.switch_session("$1");
    s.land("$1", "work");
    assert_eq!(
        s.status_commands().last().map(String::as_str),
        Some("set-option -t '$5' -u status")
    );
}

/// A stamp of a client still attached is another phone's, on that session
/// now: left alone. And the user's own sessions are never touched — not
/// even one whose status line is off (theirs, unstamped).
#[test]
fn a_live_stamp_and_the_users_sessions_are_left_alone() {
    let mut s = make_marked_with(
        &[
            "$1 owned= hidden=",
            "$5 owned=1 hidden=client-45999",
            "$6 owned= hidden=",
        ],
        ("$1", "work"),
        vec![
            work(true, 45613, "client-45613"),
            work(true, 45999, "client-45999"),
        ],
    );
    s.switch_session("$6");
    s.land("$6", "theirs");
    assert!(s.status_commands().is_empty(), "{:?}", s.status_commands());
    assert!(s.mark_commands().is_empty(), "{:?}", s.mark_commands());
}

fn make_marked(marks: &[&str], landing: (&str, &str)) -> Sidecar {
    make_marked_with(marks, landing, vec![work(true, 45613, "client-45613")])
}

fn make_marked_with(marks: &[&str], landing: (&str, &str), clients: Vec<FakeClient>) -> Sidecar {
    let fake = Rc::new(RefCell::new(Fake {
        clients,
        sessions: vec!["$1 1 work".into(), "$5 0 mine".into(), "$6 0 theirs".into()],
        windows: Vec::new(),
        panes: Vec::new(),
        marks: marks.iter().map(|m| m.to_string()).collect(),
        ..Fake::default()
    }));
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.respond(responder(fake.clone()));
    // As the session's sidecar starts: tmux's client already running, its
    // first `%session-changed` the attach.
    rig.with(|c, now| c.begin_control_mode(now));
    rig.boot();
    rig.push(&format!("%session-changed {} {}\n", landing.0, landing.1));
    rig.settle();
    Sidecar { rig, fake }
}

/// An attach's first three commands are the discovery's, sidecar or not:
/// the owned-session reads (B86) go after them. They went first, and a host
/// answering the first three as the discovery (the app's tests do, as
/// their own tmux) filed the sessions' listing under the owned reads.
#[test]
fn a_sidecars_attach_sends_the_discovery_first() {
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.attach();
    assert!(rig.commands.len() >= 5, "{:?}", rig.commands);
    assert!(rig.commands[0].starts_with("list-sessions -F '#{session_id} #{session_attached}"));
    assert!(rig.commands[1].starts_with("list-windows -a -F"));
    assert!(rig.commands[2].starts_with("list-panes -a -F '#{pane_id} "));
    assert!(
        rig.commands[3..]
            .iter()
            .any(|c| c.contains(" owned=#{@moshpit_owned}"))
    );
}

// MARK: - The phone's group over mosh (B773)

/// The renderer's session and name, as the fake tmux has it now.
fn renderer_on(s: &Sidecar, pid: i64) -> String {
    s.fake
        .borrow()
        .clients
        .iter()
        .find(|c| c.pid == pid)
        .map(|c| c.session_name.clone())
        .unwrap_or_default()
}

/// B773: the sidecar makes the phone's group of the renderer's session,
/// lands it on the session's current window, and brings the renderer in
/// behind — never moving the session itself, which a desktop on it shows.
#[test]
fn the_sidecar_brings_the_renderer_into_the_phones_group() {
    let mut s = Sidecar::grouped(
        &["$1 2 work"],
        &[
            "$1 @3 0 b25d,80x24,0,0,3 0 1 zsh",
            "$1 @4 1 b25e,80x24,0,0,4 1 1 vim",
        ],
        vec![
            work(true, 45613, "/dev/ttys030"),
            work(false, 41002, "/dev/ttys010"),
        ],
    );
    s.land("$1", "work");
    s.attach_renderer(work(false, 45533, "/dev/ttys026"));
    s.rig.push("%session-changed $91 work~mp-iPhone-1-1hc\n");
    s.rig.settle();
    let commands = s.rig.commands.clone();
    let made = commands
        .iter()
        .position(|c| c.starts_with("new-session -t '$1' -s 'work~mp-"))
        .unwrap_or_else(|| panic!("{commands:#?}"));
    let landed = commands
        .iter()
        .position(|c| c == "select-window -t @4")
        .unwrap_or_else(|| panic!("{commands:#?}"));
    let moved = commands
        .iter()
        .position(|c| c.starts_with("switch-client -c /dev/ttys026 -t"))
        .unwrap_or_else(|| panic!("{commands:#?}"));
    assert!(made < landed && landed < moved, "{commands:#?}");
    assert!(renderer_on(&s, 45533).starts_with("work~mp-"));
    assert_eq!(renderer_on(&s, 41002), "work", "the desktop stays");
    assert!(
        !commands
            .iter()
            .any(|c| c.starts_with("switch-client -c /dev/ttys010")),
        "{commands:#?}"
    );
    // The person sees the session, not the group.
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
    assert_eq!(s.rig.controller.renderer_session_id(), Some("$1"));
}

/// B773: a session picked on the phone takes the sidecar into a group of
/// it, and the renderer behind; the desktop stays on its session.
#[test]
fn a_session_pick_takes_the_renderer_into_a_group_of_it() {
    let mut s = Sidecar::grouped(
        &["$0 1 alpha", "$1 2 work"],
        &[
            "$0 @0 0 b25d,80x24,0,0,0 1 1 zsh",
            "$1 @3 0 b25e,80x24,0,0,3 1 1 vim",
        ],
        vec![
            work(true, 45613, "/dev/ttys030"),
            work(false, 41002, "/dev/ttys010"),
        ],
    );
    s.land("$1", "work");
    s.attach_renderer(work(false, 45533, "/dev/ttys026"));
    s.rig.push("%session-changed $91 work~mp-iPhone-1-1hc\n");
    s.rig.settle();
    let base = s.rig.commands.len();
    s.switch_session("$0");
    let sent = s.rig.since(base).to_vec();
    assert!(
        sent.iter()
            .any(|c| c.starts_with("new-session -t '$0' -s 'alpha~mp-")),
        "{sent:#?}"
    );
    assert!(
        !sent
            .iter()
            .any(|c| c.starts_with("switch-client -c /dev/ttys010")),
        "{sent:#?}"
    );
    assert!(
        renderer_on(&s, 45533).starts_with("alpha~mp-"),
        "{:?}: {sent:#?}",
        renderer_on(&s, 45533)
    );
    assert_eq!(renderer_on(&s, 41002), "work", "the desktop stays");
}

/// B773: a session Moshpit made shows no status line on the phone: the
/// group's, the one on the phone's screen, is turned off.
#[test]
fn an_owned_sessions_status_line_is_hidden_on_the_phones_group() {
    // The marks are on the server before the sidecar starts (B86).
    let fake = Rc::new(RefCell::new(Fake {
        clients: vec![work(true, 45613, "/dev/ttys030")],
        sessions: vec!["$1 2 work".into()],
        windows: vec!["$1 @3 0 b25d,80x24,0,0,3 1 1 zsh".into()],
        marks: vec!["$1 owned=1 hidden=".into()],
        groups: true,
        ..Fake::default()
    }));
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.respond(responder(fake.clone()));
    rig.with(|c, now| c.begin_control_mode(now));
    rig.boot();
    rig.push("%session-changed $1 work\n");
    rig.settle();
    let mut s = Sidecar { rig, fake };
    s.attach_renderer(work(false, 45533, "/dev/ttys026"));
    s.rig.push("%session-changed $91 work~mp-iPhone-1-1hc\n");
    s.rig.settle();
    assert!(
        s.status_commands()
            .iter()
            .any(|c| c == "set-option -t '$91' status off"),
        "{:#?}",
        s.status_commands()
    );
}

/// The sidecar's group commands that end a session (B812).
fn kills(commands: &[String]) -> Vec<&String> {
    commands
        .iter()
        .filter(|c| c.starts_with("kill-session") || c.starts_with("if-shell"))
        .collect()
}

/// The old group the last sidecar (or an older build) left the renderer on:
/// `$7`, of `work` (`$1`), the renderer its one client.
const OLD_GROUP: &str = "work~mp-iPhone-1-0ab";

fn rebuilt_on_old_group(groups: bool, version: Option<&'static str>) -> Sidecar {
    let fake = Rc::new(RefCell::new(Fake {
        clients: vec![
            work(true, 45613, "/dev/ttys030"),
            work(false, 41002, "/dev/ttys010"),
            client(false, 45533, "$7", OLD_GROUP, "/dev/ttys026"),
        ],
        sessions: vec!["$1 2 work".into(), format!("$7 1 {OLD_GROUP}")],
        windows: vec![
            "$1 @3 0 b25d,80x24,0,0,3 0 1 zsh".into(),
            "$1 @4 1 b25e,80x24,0,0,4 1 1 vim".into(),
        ],
        groups,
        version,
        ..Fake::default()
    }));
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.respond(responder(fake.clone()));
    rig.with(|c, now| c.begin_control_mode(now));
    rig.boot();
    rig.push("%session-changed $1 work\n");
    rig.settle();
    let mut s = Sidecar { rig, fake };
    s.rig.with(|c, now| c.expect_renderer_pid(45533, now));
    s.rig.settle();
    s
}

/// B773, B812: a rebuilt sidecar (its SSH leg back, mosh never gone) finds
/// the renderer on a group it doesn't hold — the last sidecar's, or one an
/// older build made. It took that group for a session: a group of the
/// group, and the phone showing the group's name. The renderer views the
/// session the group is of: it comes into our group of that session, and
/// the old group ends behind it unless a client is still on it.
#[test]
fn a_rebuilt_sidecar_takes_the_renderer_out_of_an_old_group() {
    let s = rebuilt_on_old_group(true, None);
    let commands = s.rig.commands.clone();
    assert!(
        !commands
            .iter()
            .any(|c| c.starts_with("new-session -t '$7'")),
        "no group of a group: {commands:#?}"
    );
    let on = renderer_on(&s, 45533);
    assert!(
        on.starts_with("work~mp-") && on != OLD_GROUP,
        "the renderer in our group: {on:?}: {commands:#?}"
    );
    let moved = commands
        .iter()
        .position(|c| c.starts_with("switch-client -c /dev/ttys026 -t"))
        .unwrap();
    let ended = commands
        .iter()
        .position(|c| {
            c == "if-shell -F -t '$7' '#{==:#{session_attached},0}' \"kill-session -t '$7'\""
        })
        .unwrap_or_else(|| panic!("the old group ends: {commands:#?}"));
    assert!(moved < ended, "{commands:#?}");
    assert_eq!(renderer_on(&s, 41002), "work", "the desktop stays");
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1"),
        "the phone shows the session"
    );
    assert_eq!(s.rig.controller.renderer_session_id(), Some("$1"));
}

/// B812: the same on a tmux before 3.8, where the phone gets no group: the
/// renderer comes onto the session itself, and the old group — one a
/// window renumbered under it can crash the server with — ends behind it.
#[test]
fn before_tmux_3_8_a_rebuilt_sidecar_takes_the_renderer_onto_the_session() {
    let s = rebuilt_on_old_group(false, Some("3.6a"));
    let commands = s.rig.commands.clone();
    assert!(
        !commands.iter().any(|c| c.starts_with("new-session -t ")),
        "no group: {commands:#?}"
    );
    assert_eq!(renderer_on(&s, 45533), "work", "{commands:#?}");
    let moved = commands
        .iter()
        .position(|c| c == "switch-client -c /dev/ttys026 -t $1")
        .unwrap_or_else(|| panic!("{commands:#?}"));
    let ended = commands
        .iter()
        .position(|c| c.starts_with("if-shell -F -t '$7' "))
        .unwrap_or_else(|| panic!("the old group ends: {:#?}", kills(&commands)));
    assert!(moved < ended, "{commands:#?}");
    assert_eq!(
        s.rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
    assert_eq!(s.rig.controller.renderer_session_id(), Some("$1"));
}

/// B874: when another window (`@9`) is locally active at the moment of the
/// switch (or `renderer_window_id()` returns `None`), `@1` in the left
/// session's `resized_windows` is still unpinned before its layout replay.
#[test]
fn sidecar_switch_unpins_all_resized_windows_when_renderer_window_differs() {
    let (mut s, replays) = two_split_sessions();
    s.rig.with(|c, now| c.sync_mosh_window(60, 20, now));
    s.rig.settle();
    assert!(s.rig.controller.resized_windows().contains("@1"));
    // Simulate active_window_id pointing at another session's window so
    // `renderer_window_id()` returns `None` right as `select_session("$2")` fires.
    s.rig.push("%session-window-changed $1 @9\n");
    s.rig.settle();
    assert!(s.rig.controller.resized_windows().contains("@1"));
    let from = s.rig.commands.len();
    s.switch_session("$2");
    let commands = s.rig.since(from).to_vec();
    let unpin = commands
        .iter()
        .position(|c| c == "set-option -u -w -t @1 window-size")
        .unwrap_or_else(|| panic!("expected @1 unpinned: {commands:?}"));
    let replay = commands
        .iter()
        .position(|c| c == &format!("select-layout -t '@1' '{WIDE_ONE}'"))
        .unwrap_or_else(|| panic!("expected @1 replayed: {commands:?}"));
    assert!(unpin < replay, "{commands:?}");
    assert_eq!(replays.borrow().len(), 1);
    let ledger = s.rig.controller.take_pin_ledger();
    assert!(!ledger.resized_windows.contains("@1"), "{ledger:?}");
}

/// B874: in grouped mode (tmux 3.8+), when the target session's active
/// window has not landed in `snapshot.windows` yet (`renderer_awaits_landing`),
/// handing back the left session waits until `bring_renderer_into_group()`
/// moves the renderer into the target group and kills the old group.
#[test]
fn sidecar_switch_defers_hand_back_while_renderer_awaits_group_landing() {
    let show_target = Rc::new(RefCell::new(false));
    let (mut s, replays) = split_sessions_with(true, show_target.clone());
    s.rig.push("%session-changed $91 work~mp-iPhone-1-1hc\n");
    s.rig.settle();
    s.rig.with(|c, now| c.sync_mosh_window(60, 20, now));
    s.rig.settle();
    assert!(s.rig.controller.resized_windows().contains("@1"));

    let from = s.rig.commands.len();
    s.switch_session("$2");
    let before_land = s.rig.since(from).to_vec();
    assert!(
        before_land
            .iter()
            .any(|c| c.starts_with("new-session -t '$2' -s 'play~mp-")),
        "{before_land:#?}"
    );
    assert!(
        !before_land
            .iter()
            .any(|c| c.starts_with("switch-client -c /dev/ttys026")),
        "renderer waits for target landing: {before_land:#?}"
    );
    assert!(
        !before_land
            .iter()
            .any(|c| c == "set-option -u -w -t @1 window-size" || c.starts_with("select-layout")),
        "must not hand back $1 before renderer leaves: {before_land:#?}"
    );

    *show_target.borrow_mut() = true;
    s.rig.push("%window-add @2\n");
    s.rig.settle();
    let all = s.rig.since(from).to_vec();
    let moved = all
        .iter()
        .position(|c| c == "switch-client -c /dev/ttys026 -t '$92'")
        .unwrap_or_else(|| panic!("{all:#?}"));
    let killed = all
        .iter()
        .position(|c| c.starts_with("kill-session -t '$91'"))
        .unwrap_or_else(|| panic!("{all:#?}"));
    let unpin = all
        .iter()
        .position(|c| c == "set-option -u -w -t @1 window-size")
        .unwrap_or_else(|| panic!("{all:#?}"));
    let replay = all
        .iter()
        .position(|c| c == &format!("select-layout -t '@1' '{WIDE_ONE}'"))
        .unwrap_or_else(|| panic!("{all:#?}"));
    assert!(
        moved < killed && killed < unpin && unpin < replay,
        "{all:#?}"
    );
    assert_eq!(replays.borrow().len(), 1);
    let ledger = s.rig.controller.take_pin_ledger();
    assert!(!ledger.resized_windows.contains("@1"), "{ledger:?}");
    assert!(!ledger.pristine_layouts.contains_key("@1"), "{ledger:?}");
}

/// B874: if `GroupCheckMarks` already created `$2`'s group while `SidecarSwitch`
/// was in flight, `SidecarSwitch` brings the renderer into that group and
/// does not fall back to an ungrouped `switch-client -t $2`.
#[test]
fn sidecar_switch_moves_renderer_when_group_already_exists_for_target() {
    let fake = Rc::new(RefCell::new(Fake {
        clients: vec![
            work(true, 45613, "client-45613"),
            work(false, 45533, "/dev/ttys026"),
        ],
        sessions: vec!["$1 1 work".into(), "$2 0 play".into()],
        windows: vec![
            format!("$1 @1 0 {SQUASHED_ONE} 1 3 one"),
            format!("$2 @2 0 {WIDE_TWO} 1 3 two"),
        ],
        groups: true,
        ..Fake::default()
    }));
    let mut rig = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    rig.respond(responder(fake.clone()));
    rig.attach();
    // Answer initial discovery (`ServerVersion`, `list-sessions`, `list-windows`, etc.)
    // so `snapshot` holds `$1` and `$2`, then push `%session-changed $1 work`
    // (which starts `check_for_foreign_control` for `$1`).
    rig.settle();
    rig.push("%session-changed $1 work\n");
    // Before `GroupCheckMarks` is answered, tell the sidecar who the renderer is
    // and switch to `$2` (`select_session` updates `active_session_id = $2` and
    // queues `LocateRenderer` + `SidecarSwitch` behind `GroupCheckMarks`).
    rig.with(|c, now| {
        c.expect_renderer_pid(45533, now);
        c.select_session("$2", now);
    });
    // `GroupCheckMarks` answers first: sees `picked == true` (`active_session_id == $2`),
    // enters `$2`'s group (`play~mp-...`), and leaves the renderer on `$1` until
    // `SidecarSwitch` answers right behind it.
    rig.settle();
    let s = Sidecar { rig, fake };
    assert!(
        !s.rig.commands.iter().any(|c| c == "switch-client -t $2"),
        "must not fall back to ungrouped switch: {:#?}",
        s.rig.commands
    );
    assert!(
        renderer_on(&s, 45533).starts_with("play~mp-"),
        "renderer moved into $2's group: {} ({:#?})",
        renderer_on(&s, 45533),
        s.rig.commands
    );
}
