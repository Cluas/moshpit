//! Scripted sessions for the controller's committed logs
//! (`tests/fixtures/controller/`, checked by `tests/controller_logs.rs`).
//! The scripts were the equivalence harness's, which ran each through this
//! controller and the app's Swift `TmuxSessionController` and compared the
//! logs line by line; the Swift side was retired with that controller (E3),
//! and `harness/controller/divergences.json` keeps what the comparison
//! found.
//!
//! A script is one step per line (`#` starts a comment). Text arguments use
//! escapes: `\n \r \t \e \\ \s` (a space) and `\xHH` (one byte).
//!
//! ```text
//! config renders=0 label=iPad steal=10 stamp=5 probe=1.5 quiet=2 streaming=0.5
//! init-size 70 35          client-name moshpit-a      restore $0 @1 %4
//! inherit-pins @1,@2       attach | begin | detach | close
//! recv <text>              bytes from tmux
//! rule ok|err <prefix> => <body lines>[ => <after>]    how tmux answers
//! rules-clear | skip       respond | settle           wait <ms> | waitsettle <ms> <step>
//! select-window @1 | select-pane %1 | select-session $1 | switch next|prev
//! new-window [name] | new-window-in $1 [name] | new-pane | new-pane-in @1
//! new-session [name] | rename-session $1 <name> | kill-session $1
//! rename-window @1 <name> | kill-window @1 | kill-pane %1 | refresh | run <cmd>
//! resize <cols> <rows> | input %0 <hex> | paste %0 <text> | resync 0|1
//! release | repin | take | sync-mosh <cols> <rows> | poll | count | path %0
//! prefix | cancel-copy | mouse | scroll <lines> | scroll-px <dy> <cell>
//! scroll-end | scroll-begin | click <col> <row> | restore-status | ledger
//! lease-release | layout-restore | mint %0 | live 0|1 | dump
//! view-grid %0 <c> <r> | view-size %0 <c> <r> | view-defer %0 <c> <r>
//! view-input %0 <hex> | view-mode %0 | view-return %0 | view-bell %0
//! view-reveal %0
//! ```
//!
//! A command with no rule matching its prefix gets the reply `0 0`; a rule
//! body `-` is an empty block. Replies are numbered from 1001 in the order
//! they are made.
//!
//! The log, per step: `@<n> <step>`, then everything but commands in order
//! (`. pane`, `. view`, `. open`, `. close`, `. note`), then the commands
//! written (`> `), then what the step printed (`= `). Both clocks start at
//! 1,000,000 ms and move only on `wait` steps (timers fire in deadline
//! order), so the logs are deterministic.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::controller::{
    Config, Controller, Notification, Output, PinLedger, PixelScroll, ViewAction,
};
use crate::model::{Selection, id_cmp};
use crate::rig::{START_MS, START_UNIX_MS, block};

/// Decode the script's escapes.
pub fn unescape(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' || at + 1 >= bytes.len() {
            out.push(bytes[at]);
            at += 1;
            continue;
        }
        match bytes[at + 1] {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'e' => out.push(0x1b),
            b's' => out.push(b' '),
            b'\\' => out.push(b'\\'),
            b'x' if at + 3 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[at + 2..at + 4]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        at += 4;
                        continue;
                    }
                    Err(_) => out.push(b'x'),
                }
            }
            other => out.push(other),
        }
        at += 2;
    }
    out
}

/// Bytes for the log: printable ASCII as is (backslash doubled), the rest
/// `\xHH`.
pub fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .filter_map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok())
        .collect()
}

#[derive(Debug, Clone)]
struct Rule {
    prefix: String,
    body: Option<Vec<String>>,
    error: bool,
    after: String,
}

/// Runs a script through the Rust controller and returns its log.
pub fn run(script: &str) -> String {
    let mut runner = Runner::default();
    for (index, line) in script.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        runner.step(index, line);
    }
    runner.out
}

struct Runner {
    config: Config,
    controller: Option<Controller>,
    now: u64,
    out: String,
    commands: Vec<String>,
    answered: usize,
    printed: usize,
    rules: Vec<Rule>,
    next_block: i64,
    items: Vec<String>,
    /// The controller's request ids → the script's (in order of asking).
    requests: BTreeMap<u64, u64>,
    next_request: u64,
}

impl Default for Runner {
    fn default() -> Self {
        Self {
            config: Config {
                unix_offset_ms: START_UNIX_MS - START_MS as i64,
                ..Config::default()
            },
            controller: None,
            now: START_MS,
            out: String::new(),
            commands: Vec::new(),
            answered: 0,
            printed: 0,
            rules: Vec::new(),
            next_block: 1000,
            items: Vec::new(),
            requests: BTreeMap::new(),
            next_request: 0,
        }
    }
}

impl Runner {
    fn controller(&mut self) -> &mut Controller {
        if self.controller.is_none() {
            self.controller = Some(Controller::new(self.config.clone()));
        }
        self.controller.as_mut().expect("made above")
    }

    /// Whether the pane has a terminal (the Swift side's coordinator).
    fn has_view(&mut self, pane: &str) -> bool {
        self.controller().stream_of(pane).is_some()
    }

    fn active_has_view(&mut self) -> bool {
        let pane = self.controller().snapshot().input_pane();
        pane.is_some_and(|pane| self.has_view(&pane))
    }

    fn script_request(&mut self, request: u64) -> u64 {
        if let Some(&id) = self.requests.get(&request) {
            return id;
        }
        self.next_request += 1;
        self.requests.insert(request, self.next_request);
        self.next_request
    }

    fn drain(&mut self) {
        let outputs = self.controller().drain_outputs();
        for output in outputs {
            match output {
                Output::Command(command) => self.commands.push(command),
                Output::PaneOutput { pane, bytes, .. } => {
                    self.items.push(format!("pane {pane} {}", escape(&bytes)));
                }
                Output::View { pane, action, .. } => {
                    let action = match action {
                        ViewAction::Veil => "veil".to_owned(),
                        ViewAction::VeilUnlessCovered => {
                            self.items.push(format!("view {pane} cover?"));
                            "veil".to_owned()
                        }
                        ViewAction::ExtendCover { seconds } => format!("extend {seconds:.3}"),
                        ViewAction::Reveal => "reveal".to_owned(),
                        ViewAction::ReleaseScrollHold => "release-hold".to_owned(),
                        ViewAction::ScrollLocal { lines } => format!("scroll-local {lines}"),
                        ViewAction::SettleLocalScroll => "settle".to_owned(),
                        ViewAction::ApplyDeferredResize => "apply-resize".to_owned(),
                    };
                    self.items.push(format!("view {pane} {action}"));
                }
                Output::StreamOpened { pane, .. } => self.items.push(format!("open {pane}")),
                Output::StreamClosed { pane, .. } => self.items.push(format!("close {pane}")),
                Output::Notify(note) => {
                    let line = match note {
                        Notification::StateChanged => continue,
                        Notification::PaneActivity { pane } => format!("note activity {pane}"),
                        Notification::PaneBell { pane } => format!("note bell {pane}"),
                        Notification::AgentHooksUpdated => "note hooks".to_owned(),
                        Notification::FirstFramePainted => "note first-frame".to_owned(),
                        Notification::ChannelDead { .. } => "note dead".to_owned(),
                        Notification::ClientCount { request, count } => {
                            let id = self.script_request(request);
                            format!("note count {id} {count}")
                        }
                        Notification::PanePath { request, path } => {
                            let id = self.script_request(request);
                            let path = path.map_or("-".to_owned(), |p| escape(p.as_bytes()));
                            format!("note path {id} {path}")
                        }
                        Notification::PrefixKey { request, key } => {
                            let id = self.script_request(request);
                            format!("note prefix {id} {}", escape(key.as_bytes()))
                        }
                        Notification::Flushed { request } => {
                            let id = self.script_request(request);
                            format!("note flushed {id}")
                        }
                        Notification::Pong { request, alive } => {
                            let id = self.script_request(request);
                            format!("note pong {id} {alive}")
                        }
                        Notification::PaneSnapshot { pane, rows } => {
                            format!("note snapshot {pane} {}", rows.len())
                        }
                    };
                    self.items.push(line);
                }
            }
        }
    }

    fn reply_for(&mut self, command: &str) -> String {
        // A fence prints its text, whatever the script says (B273).
        if let Some(text) = crate::rig::fence_text(command) {
            self.next_block += 1;
            return block(self.next_block, &[text.to_owned()], false);
        }
        let rule = self
            .rules
            .iter()
            .find(|rule| command.starts_with(rule.prefix.as_str()))
            .cloned();
        self.next_block += 1;
        let num = self.next_block;
        match rule {
            Some(rule) => {
                let mut text = block(num, &rule.body.unwrap_or_default(), rule.error);
                text.push_str(&rule.after);
                text
            }
            None => block(num, &["0 0".to_owned()], false),
        }
    }

    /// Answer the commands written so far (not those the answers provoke).
    fn respond(&mut self) -> usize {
        let mut count = 0;
        let written = self.commands.len();
        while self.answered < written {
            let command = self.commands[self.answered].clone();
            self.answered += 1;
            let reply = self.reply_for(&command);
            let now = self.now;
            self.controller().receive(reply.as_bytes(), now);
            self.drain();
            count += 1;
        }
        count
    }

    fn settle(&mut self) {
        for _ in 0..64 {
            if self.respond() == 0 {
                break;
            }
        }
    }

    fn wait(&mut self, ms: u64) {
        self.now += ms;
        let now = self.now;
        self.controller().tick(now);
        self.drain();
    }

    fn print(&mut self, line: impl AsRef<str>) {
        self.items.push(format!("={}", line.as_ref()));
    }

    fn step(&mut self, index: usize, line: &str) {
        let (op, arg) = line.split_once(' ').unwrap_or((line, ""));
        let words: Vec<&str> = arg.split(' ').filter(|w| !w.is_empty()).collect();
        let num = |i: usize| -> i64 { words.get(i).and_then(|w| w.parse().ok()).unwrap_or(0) };
        let float = |i: usize| -> f64 { words.get(i).and_then(|w| w.parse().ok()).unwrap_or(0.0) };
        if op == "config" {
            for word in &words {
                let Some((key, value)) = word.split_once('=') else {
                    continue;
                };
                let f = value.parse().unwrap_or(0.0);
                let config = match self.controller.as_mut() {
                    Some(controller) => controller.config_mut(),
                    None => &mut self.config,
                };
                match key {
                    "renders" => config.renders_output = value == "1",
                    "label" => config.size_owner_label = value.to_owned(),
                    "steal" => config.foreground_steal_after = f,
                    "stamp" => config.lease_stamp_refresh_interval = f,
                    "probe" => config.foreground_probe_timeout = f,
                    "quiet" => config.foreign_quiet_repin_delay = f,
                    "streaming" => config.streaming_window = f,
                    _ => {}
                }
            }
            return;
        }
        let _ = writeln!(self.out, "@{index} {line}");
        let now = self.now;
        match op {
            "init-size" => self.controller().set_initial_client_size(num(0), num(1)),
            "client-name" => self.controller().set_our_client_name(Some(arg.to_owned())),
            "restore" => {
                let part = |i: usize| words.get(i).filter(|w| **w != "-").map(|w| w.to_string());
                let selection = Selection {
                    session: words.first().map(|w| w.to_string()).unwrap_or_default(),
                    window: part(1),
                    pane: part(2),
                };
                self.controller().set_pending_restore(Some(selection));
            }
            "inherit-pins" => {
                let pins = arg
                    .split(',')
                    .filter(|w| !w.is_empty())
                    .map(str::to_owned)
                    .collect();
                self.controller()
                    .inherit_pin_ledger(PinLedger::with_pins(pins));
            }
            "attach" => self.controller().attach(now),
            "begin" => self.controller().begin_control_mode(now),
            "detach" => self.controller().detach(now),
            "close" => self.controller().channel_closed(now),
            "recv" => {
                let bytes = unescape(arg);
                self.controller().receive(&bytes, now);
            }
            "rule" => {
                let (kind, rest) = arg.split_once(' ').unwrap_or((arg, ""));
                let parts: Vec<&str> = rest.split(" => ").collect();
                let prefix = String::from_utf8_lossy(&unescape(parts[0])).into_owned();
                let body = parts.get(1).copied().unwrap_or("-");
                let body = (body != "-").then(|| {
                    String::from_utf8_lossy(&unescape(body))
                        .split('\n')
                        .map(str::to_owned)
                        .collect()
                });
                let after = parts
                    .get(2)
                    .map(|after| String::from_utf8_lossy(&unescape(after)).into_owned())
                    .unwrap_or_default();
                self.rules.push(Rule {
                    prefix,
                    body,
                    error: kind == "err",
                    after,
                });
            }
            "rules-clear" => self.rules.clear(),
            // tmux never answers what a control session that ended had sent.
            "skip" => self.answered = self.commands.len(),
            "respond" => {
                self.respond();
            }
            "settle" => self.settle(),
            "wait" => self.wait(num(0) as u64),
            "waitsettle" => {
                let target = self.now + num(0) as u64;
                let step = (num(1) as u64).max(1);
                while self.now < target {
                    let next = (self.now + step).min(target);
                    self.wait(next - self.now);
                    self.settle();
                }
            }
            "select-window" => self.controller().select_window(arg, now),
            "select-pane" => self.controller().select_pane(arg, now),
            "select-session" => self.controller().select_session(arg, now),
            "switch" => self.controller().switch_pane_or_window(arg == "next", now),
            "new-window" => self
                .controller()
                .new_window((!arg.is_empty()).then_some(arg), now),
            "new-window-in" => {
                let (session, name) = arg.split_once(' ').unwrap_or((arg, ""));
                self.controller().new_window_in_session(
                    session,
                    (!name.is_empty()).then_some(name),
                    now,
                );
            }
            "new-pane" => self.controller().new_pane(now),
            "new-pane-in" => self.controller().new_pane_in_window(arg, now),
            "new-session" => self
                .controller()
                .new_session((!arg.is_empty()).then_some(arg), now),
            "rename-session" | "rename-window" => {
                let (id, name) = arg.split_once(' ').unwrap_or((arg, ""));
                let name = String::from_utf8_lossy(&unescape(name)).into_owned();
                if op == "rename-session" {
                    self.controller().rename_session(id, &name, now);
                } else {
                    self.controller().rename_window(id, &name, now);
                }
            }
            "kill-session" => self.controller().kill_session(arg, now),
            "kill-window" => self.controller().kill_window(arg, now),
            "kill-pane" => self.controller().kill_pane(arg, now),
            "refresh" => self.controller().refresh(now),
            "run" => self.controller().run_command(arg, now),
            "resize" => {
                let changed = self.controller().resize_client(num(1), num(0), now);
                self.print(format!(" resize {}", u8::from(changed)));
            }
            "input" => {
                let (pane, hex) = arg.split_once(' ').unwrap_or((arg, ""));
                self.controller().send_input(&unhex(hex), pane, now);
            }
            "paste" => {
                let (pane, text) = arg.split_once(' ').unwrap_or((arg, ""));
                let text = String::from_utf8_lossy(&unescape(text)).into_owned();
                self.controller().send_paste(&text, pane, now);
            }
            "resync" => self.controller().resync_active_pane_now(arg == "1", now),
            "release" => self.controller().release_window_pins(now),
            "repin" => self.controller().repin_active_window(now),
            "take" => self.controller().take_size_ownership(now),
            "sync-mosh" => self.controller().sync_mosh_window(num(0), num(1), now),
            "renderer-pid" => self.controller().expect_renderer_pid(num(0), now),
            "poll" => self.controller().poll_agent_hooks(now),
            "count" => {
                let request = self.controller().count_non_control_clients(now);
                self.script_request(request);
            }
            "path" => {
                let request = self.controller().request_pane_path(arg, now);
                self.script_request(request);
            }
            "prefix" => {
                let request = self.controller().query_prefix_key(now);
                self.script_request(request);
            }
            "cancel-copy" => self.controller().cancel_stale_copy_mode(now),
            "mouse" => self.controller().refresh_active_pane_mouse(now),
            "scroll" => self.controller().scroll_lines(num(0), now),
            "scroll-px" => {
                let (dy, cell) = (float(0), float(1));
                match self.controller().scroll_pixels(dy, cell, now) {
                    PixelScroll::Consumed(consumed) => self.print(format!(" px {consumed:.3}")),
                    PixelScroll::Local { pane } => {
                        self.drain();
                        self.items
                            .push(format!("view {pane} scroll-local-px {dy:.3}"));
                        self.print(format!(" px {dy:.3}"));
                    }
                }
            }
            "scroll-end" => self.controller().scroll_did_end(now),
            "scroll-begin" if self.active_has_view() => self.controller().scroll_began(now),
            "click" if self.active_has_view() => self.controller().click(num(0), num(1), now),
            "scroll-begin" | "click" => {}
            "restore-status" => match self.controller().restore_suppressed_status(now) {
                Some(request) => {
                    self.script_request(request);
                }
                None => {
                    self.next_request += 1;
                    let id = self.next_request;
                    self.items.push(format!("note flushed {id}"));
                }
            },
            "ledger" => {
                let ledger = self.controller().take_pin_ledger();
                let empty = u8::from(ledger.is_empty());
                let pins = sorted(ledger.resized_windows.into_iter().collect());
                self.print(format!(" ledger pins={} empty={empty}", pins.join(",")));
            }
            "lease-release" => {
                for command in self.controller().size_lease_release_commands() {
                    self.print(format!(" {command}"));
                }
            }
            "layout-restore" => {
                for command in self.controller().pristine_layout_restore_commands() {
                    self.print(format!(" {command}"));
                }
            }
            "mint" => {
                self.controller().stream_for_pane(arg);
            }
            "live" => self.controller().set_transport_live(arg == "1"),
            "view-grid" | "view-size" => {
                let pane = words.first().copied().unwrap_or("");
                if self.has_view(pane) {
                    self.controller().view_grid(pane, num(1), num(2), now);
                }
            }
            "view-defer" => {
                let pane = words.first().copied().unwrap_or("");
                let held = self.has_view(pane)
                    && self
                        .controller()
                        .view_should_defer_resize(pane, num(1), num(2), now);
                self.print(format!(" defer {}", u8::from(held)));
            }
            "view-input" => {
                let (pane, hex) = arg.split_once(' ').unwrap_or((arg, ""));
                if self.has_view(pane) {
                    self.controller().view_input(pane, &unhex(hex), now);
                }
            }
            "view-mode" if self.has_view(arg) => {
                self.controller().view_input_mode_changed(arg, now)
            }
            "view-return" if self.has_view(arg) => self.controller().view_return_sent(arg, now),
            "view-bell" if self.has_view(arg) => self.controller().view_bell(arg, now),
            "view-reveal" if self.has_view(arg) => {
                self.items.push(format!("view {arg} reveal"));
                self.controller().view_revealed(arg, now);
            }
            "view-mode" | "view-return" | "view-bell" | "view-reveal" => {}
            "dump" => {
                self.drain();
                let lines = dump(self.controller());
                for line in lines {
                    self.print(format!(" {line}"));
                }
            }
            other => self.print(format!(" unknown step {other}")),
        }
        self.drain();
        self.flush_step();
    }

    fn flush_step(&mut self) {
        let items = std::mem::take(&mut self.items);
        let (printed, items): (Vec<String>, Vec<String>) =
            items.into_iter().partition(|item| item.starts_with('='));
        for item in items {
            let _ = writeln!(self.out, ". {item}");
        }
        for command in &self.commands[self.printed..] {
            let _ = writeln!(self.out, "> {command}");
        }
        self.printed = self.commands.len();
        for line in printed {
            let _ = writeln!(self.out, "{line}");
        }
    }
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort_by(|a, b| id_cmp(a, b));
    ids
}

fn opt(value: Option<&str>) -> String {
    value.map_or("-".to_owned(), |v| escape(v.as_bytes()))
}

/// The controller's state as the log prints it.
pub fn dump(c: &Controller) -> Vec<String> {
    let s = c.snapshot();
    let mut lines = vec![format!(
        "attached={} ever={} session={} window={} pane={} fwd={} swiped={}",
        u8::from(s.is_attached),
        u8::from(s.ever_attached),
        opt(s.active_session_id.as_deref()),
        opt(s.active_window_id.as_deref()),
        opt(s.active_pane_id.as_deref()),
        u8::from(s.last_switch_forward),
        u8::from(s.has_swiped_since_attach)
    )];
    for session in s.sorted_sessions() {
        lines.push(format!(
            "s {} att={} name={}",
            session.id,
            u8::from(session.is_attached),
            escape(session.name.as_bytes())
        ));
    }
    for window in s.windows_in_id_order() {
        lines.push(format!(
            "w {} s={} idx={} act={} panes={} zoom={} name={} layout={}",
            window.id,
            opt(Some(&window.session_id)),
            window.index,
            u8::from(window.is_active),
            window.pane_count,
            u8::from(window.is_zoomed),
            escape(window.name.as_bytes()),
            escape(window.layout.as_bytes())
        ));
    }
    for pane in s.panes_in_id_order() {
        lines.push(format!(
            "p {} w={} idx={} act={} {}x{} cmd={}",
            pane.id,
            opt(Some(&pane.window_id)),
            pane.index,
            u8::from(pane.is_active),
            pane.width,
            pane.height,
            escape(pane.command.as_bytes())
        ));
    }
    let follow = c.size_follow().map_or("-".to_owned(), |f| {
        format!("{}/{}", f.window_id, escape(f.owner_label.as_bytes()))
    });
    let resized = sorted(c.resized_windows().iter().cloned().collect()).join(",");
    let flags = c.active_input_flags();
    let renderer = c.renderer_client().map_or("-".to_owned(), |r| {
        format!("{}/{}", r.pid, escape(r.name.as_bytes()))
    });
    lines.push(format!(
        "follow={} notice={} refreshing={} resized={} mouse={} alt={} client={} hooks-loaded={} painted={} renderer={} renderer-session={} owned={}",
        follow,
        opt(c.notice()),
        u8::from(c.is_refreshing()),
        resized,
        u8::from(flags.wants_mouse),
        u8::from(flags.alt_screen),
        opt(c.our_client_name()),
        u8::from(c.agent_hooks_loaded()),
        u8::from(c.has_painted_since_attach()),
        renderer,
        opt(c.renderer_session_id()),
        sorted(c.owned_session_ids().iter().cloned().collect()).join(",")
    ));
    let mut hooks: Vec<(&String, &crate::hooks::AgentHook)> = c.agent_hooks().iter().collect();
    hooks.sort_by(|a, b| id_cmp(a.0, b.0));
    for (pane, hook) in hooks {
        lines.push(format!(
            "h {} state={} agent={} since={} cmd={} title={}",
            escape(pane.as_bytes()),
            opt(hook.state.as_deref()),
            opt(hook.agent.as_deref()),
            hook.since.map_or("-".to_owned(), |s| format!("{s:.3}")),
            opt(hook.command.as_deref()),
            opt(hook.title.as_deref())
        ));
    }
    lines
}
