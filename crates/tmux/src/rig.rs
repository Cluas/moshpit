//! A tmux stand-in around a [`Controller`], for tests and the equivalence
//! harness: what `MockTmuxTransport` and the answering helpers of the Swift
//! suites do, without the waiting. Commands the controller writes are
//! recorded; tmux's replies are made by a responder, one block per command in
//! send order; time moves only when told to.

use std::collections::BTreeMap;

use crate::controller::{Config, Controller, Notification, Output, StreamId, ViewAction};

/// The rig's clock at the start, in milliseconds.
pub const START_MS: u64 = 1_000_000;
/// The Unix time the rig's clock starts at (2026-09-15T…), in milliseconds.
pub const START_UNIX_MS: i64 = 1_758_000_000_000;

/// How tmux answers one command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    pub lines: Vec<String>,
    /// `%error` instead of `%end`.
    pub error: bool,
    /// Notifications tmux sends right after the block.
    pub after: String,
}

/// A reply with these lines.
pub fn ok<S: Into<String>>(lines: impl IntoIterator<Item = S>) -> Option<Answer> {
    Some(Answer {
        lines: lines.into_iter().map(Into::into).collect(),
        ..Answer::default()
    })
}

/// An `%error` reply with these lines.
pub fn err<S: Into<String>>(lines: impl IntoIterator<Item = S>) -> Option<Answer> {
    Some(Answer {
        lines: lines.into_iter().map(Into::into).collect(),
        error: true,
        ..Answer::default()
    })
}

/// How tmux answers a command (`None`: an empty block).
pub type Responder = Box<dyn FnMut(&str, usize) -> Option<Answer>>;

/// One reply block (`%begin n n 1 … %end n n 1`): flags 1, as tmux writes
/// them for every command it read off this client's own input.
pub fn block(num: i64, lines: &[String], is_error: bool) -> String {
    let end = if is_error { "%error" } else { "%end" };
    let mut text = format!("%begin {num} {num} 1\n");
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&format!("{end} {num} {num} 1\n"));
    text
}

/// The text a fence (`display-message -p '<text>'`, the controller's
/// marker behind a command that may answer more than once) prints: what
/// tmux answers it with.
pub fn fence_text(command: &str) -> Option<&str> {
    command
        .strip_prefix("display-message -p '")?
        .strip_suffix('\'')
        .filter(|text| text.starts_with("moshpit-fence-"))
}

/// What the rig saw the controller do, besides commands.
#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    Pane(String, Vec<u8>),
    View(String, ViewAction),
    Opened(String, StreamId),
    Closed(String, StreamId),
    Note(Notification),
}

pub struct Rig {
    pub controller: Controller,
    pub now: u64,
    /// Every command written, in order.
    pub commands: Vec<String>,
    /// How many of them have been answered.
    pub answered: usize,
    /// Everything else, in order.
    pub seen: Vec<Seen>,
    responder: Responder,
    next_block: i64,
}

impl Rig {
    pub fn new(config: Config) -> Self {
        let config = Config {
            unix_offset_ms: START_UNIX_MS - START_MS as i64,
            ..config
        };
        Self {
            controller: Controller::new(config),
            now: START_MS,
            commands: Vec::new(),
            answered: 0,
            seen: Vec::new(),
            responder: Box::new(|_, _| ok(["0 0"])),
            next_block: 900,
        }
    }

    /// Answer commands with `responder` from now on.
    pub fn respond(&mut self, responder: Responder) {
        self.responder = responder;
    }

    /// Collect what the controller queued.
    pub fn drain(&mut self) {
        while let Some(output) = self.controller.poll_output() {
            match output {
                Output::Command(command) => self.commands.push(command),
                Output::PaneOutput { pane, bytes, .. } => self.seen.push(Seen::Pane(pane, bytes)),
                Output::View { pane, action, .. } => self.seen.push(Seen::View(pane, action)),
                Output::StreamOpened { pane, stream } => self.seen.push(Seen::Opened(pane, stream)),
                Output::StreamClosed { pane, stream } => self.seen.push(Seen::Closed(pane, stream)),
                Output::Notify(Notification::StateChanged) => {}
                Output::Notify(notification) => self.seen.push(Seen::Note(notification)),
            }
        }
    }

    /// Run `f` on the controller at the rig's clock, then collect.
    pub fn with<T>(&mut self, f: impl FnOnce(&mut Controller, u64) -> T) -> T {
        let result = f(&mut self.controller, self.now);
        self.drain();
        result
    }

    /// Bytes from tmux.
    pub fn push(&mut self, text: &str) {
        self.push_bytes(text.as_bytes());
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.controller.receive(bytes, self.now);
        self.drain();
    }

    /// The boot line's own reply (swallowed by the parser), as real tmux
    /// sends it.
    pub fn boot(&mut self) {
        self.push("%begin 100 0 0\n%end 100 0 0\n\n");
    }

    /// `attach()` plus the boot block.
    pub fn attach(&mut self) {
        self.with(|c, now| c.attach(now));
        self.boot();
    }

    /// Answer every command written so far, once each, in order.
    pub fn answer_pending(&mut self) -> usize {
        let mut count = 0;
        while self.answered < self.commands.len() {
            let command = self.commands[self.answered].clone();
            let index = self.answered;
            self.answered += 1;
            // A fence prints its text, whatever the script says (B273).
            let answer = match fence_text(&command) {
                Some(text) => ok([text]).unwrap_or_default(),
                None => (self.responder)(&command, index).unwrap_or_default(),
            };
            self.next_block += 1;
            let mut text = block(self.next_block, &answer.lines, answer.error);
            text.push_str(&answer.after);
            self.push(&text);
            count += 1;
        }
        count
    }

    /// Answer until the controller stops sending (bounded).
    pub fn settle(&mut self) {
        for _ in 0..64 {
            if self.answer_pending() == 0 {
                break;
            }
        }
    }

    /// Move the clock on, firing the timers due.
    pub fn wait(&mut self, ms: u64) {
        self.now += ms;
        self.controller.tick(self.now);
        self.drain();
    }

    /// Move the clock on in steps, answering everything after each.
    pub fn wait_settled(&mut self, ms: u64, step: u64) {
        let target = self.now + ms;
        while self.now < target {
            let next = (self.now + step).min(target);
            self.wait(next - self.now);
            self.settle();
        }
    }

    /// Commands written since index `from`.
    pub fn since(&self, from: usize) -> &[String] {
        &self.commands[from.min(self.commands.len())..]
    }

    pub fn has(&self, prefix: &str) -> bool {
        self.commands.iter().any(|c| c.starts_with(prefix))
    }

    pub fn count(&self, prefix: &str) -> usize {
        self.commands
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }

    pub fn index_of(&self, prefix: &str) -> Option<usize> {
        self.commands.iter().position(|c| c.starts_with(prefix))
    }

    /// Bytes fed to a pane, all of it, in order.
    pub fn pane_bytes(&self, pane: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        for seen in &self.seen {
            if let Seen::Pane(p, b) = seen
                && p == pane
            {
                bytes.extend_from_slice(b);
            }
        }
        bytes
    }

    /// The view actions a pane got, in order.
    pub fn view_actions(&self, pane: &str) -> Vec<ViewAction> {
        self.seen
            .iter()
            .filter_map(|seen| match seen {
                Seen::View(p, action) if p == pane => Some(action.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn notes(&self) -> Vec<Notification> {
        self.seen
            .iter()
            .filter_map(|seen| match seen {
                Seen::Note(note) => Some(note.clone()),
                _ => None,
            })
            .collect()
    }

    /// The panes with a terminal, in the order they got one.
    pub fn opened(&self) -> BTreeMap<String, StreamId> {
        self.controller.streams().clone()
    }
}
