//! herdr's JSON socket API (`~/.config/herdr/herdr.sock`) over any byte
//! pipe: a sans-IO port of `Moshpit/Services/Herdr/HerdrSocketClient.swift`
//! and of the pure parts of `HerdrPushDriver.swift`.
//!
//! [`SocketClient`] is the actor's state without its tasks: the caller
//! writes the lines it hands out, feeds back what the pipe returns, and runs
//! the timers ([`SocketClient::expire`]). Wire facts, verified against herdr
//! 0.8.0 (protocol 19) and the captures in `tests/fixtures/`:
//!
//! - Requests are one-line JSON `{"id","method","params"}`; `id` must be a
//!   string (an integer id is silently ignored) and `params` must be there
//!   even when empty.
//! - Success is `{"id","result":{…}}`, failure `{"id","error":{code,message}}`.
//!   A request the server cannot parse comes back with `"id":""`, and the
//!   server hangs up right after.
//! - Subscription pushes are `{"event","data"}` lines between the replies,
//!   with no id and no sequence number. Subscribing replays the current
//!   state first, as synthetic created/focused events, so a subscribe is a
//!   full bootstrap.

use std::collections::BTreeSet;
use std::time::Duration;

use memchr::memchr;

use crate::{json, text};

/// One subscription push: its kind (`pane_agent_status_changed`,
/// `tab_created`, …). Snake case on the wire; subscription requests use
/// dotted names (`pane.agent_status_changed`), herdr's asymmetry. The one
/// consumer treats every event as "something changed" and re-reads the
/// snapshot, so the kind is all that is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketEvent {
    pub kind: String,
}

/// Why a request did not get its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketError {
    /// `{"id":…,"error":{code,message}}`.
    Server { code: String, message: String },
    /// The pipe ended (EOF, teardown) with the request in flight; herdr also
    /// closes the connection after an `invalid_request`.
    ConnectionClosed,
    /// No reply in time: the pipe may be alive but wedged (a half-open SSH
    /// after suspension), whose remedy differs from a closed one.
    Timeout,
}

/// What the pipe's bytes came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// The pump said it holds a socket ([`PUMP_READY_MARKER`]): from now on
    /// writes reach herdr, not a login shell. Once per client.
    PumpReady,
    Event(SocketEvent),
    /// A request's answer: its whole reply line, for decoders such as
    /// [`crate::snapshot::decode`], or why there is none.
    Reply {
        id: String,
        result: Result<String, SocketError>,
    },
}

/// Printed by the pump the moment it holds a socket. Until then nothing
/// reads what is written: a subscribe written straight after the pump
/// started was lost every time.
pub const PUMP_READY_MARKER: &str = "MOSHPIT_PUMP_READY";

/// The pump, PTY-shaped: kill the line discipline (echo would feed the app's
/// JSON back into the parser; canonical mode cuts lines at 4096 bytes), then
/// replace the shell with a python bridge to herdr's socket. python3 because
/// it is on effectively every host that runs agents, and it is probed, not
/// assumed — found as the probe finds it, with
/// [`crate::launch::EXTRA_PATH_DIRS`] after the PATH the command gets. One
/// line (the script's newlines are `\n` escapes inside a python string), and
/// `exec`, so the channel's teardown kills the pump. The push channel's
/// program, run by `sh -c` (B267: typed into the login shell it was lost to
/// a shell still starting); the `-c` program is one word all the same
/// ([`crate::launch::quote`]): fish read the `\\n` it holds inside single
/// quotes as `\n`, and python then met a raw newline inside a bytes literal
/// (B138).
pub fn pump_boot_line() -> String {
    let script = [
        "import os,socket,threading\\n",
        "s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)\\n",
        "s.connect(os.path.expanduser(\\\"~/.config/herdr/herdr.sock\\\"))\\n",
        "os.write(1,b\\\"",
        PUMP_READY_MARKER,
        "\\\\n\\\")\\n",
        "def up():\\n",
        " while True:\\n",
        "  d=os.read(0,65536)\\n",
        "  if not d: break\\n",
        "  s.sendall(d)\\n",
        " s.shutdown(socket.SHUT_WR)\\n",
        "threading.Thread(target=up,daemon=True).start()\\n",
        "while True:\\n",
        " d=s.recv(65536)\\n",
        " if not d: break\\n",
        " os.write(1,d)",
    ]
    .concat();
    format!(
        "stty raw -echo; PATH=\"$PATH:{}\" exec python3 -c {}",
        crate::launch::EXTRA_PATH_DIRS,
        crate::launch::quote(&format!("exec(\"{script}\")"))
    )
}

/// Every subscription kind that takes no filter: the schema minus the three
/// pane-scoped kinds, which REJECT a bare subscription with an
/// `invalid_request` (and the server then hangs up on the whole pipe).
pub const GLOBAL_KINDS: [&str; 24] = [
    "workspace.created",
    "workspace.updated",
    "workspace.metadata_updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
    "tab.created",
    "tab.closed",
    "tab.renamed",
    "tab.moved",
    "tab.focused",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];

/// The kinds that need a `pane_id`, never in [`GLOBAL_KINDS`]. Agent status
/// flips still arrive promptly: they bump the pane's revision and come as
/// global `pane.updated` events.
pub const PANE_SCOPED_KINDS: [&str; 3] = [
    "pane.agent_status_changed",
    "pane.scroll_changed",
    "pane.output_matched",
];

/// A `subscriptions` entry. `pane_id` is omitted, not null, when absent: the
/// schema validates per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subscription<'a> {
    pub kind: &'a str,
    pub pane_id: Option<&'a str>,
}

/// `events.subscribe`'s params.
pub fn subscribe_params(subscriptions: &[Subscription<'_>]) -> String {
    let entries: Vec<String> = subscriptions
        .iter()
        .map(|subscription| match subscription.pane_id {
            Some(pane) => format!(
                "{{\"type\":{},\"pane_id\":{}}}",
                json::encode_string(subscription.kind),
                json::encode_string(pane)
            ),
            None => format!("{{\"type\":{}}}", json::encode_string(subscription.kind)),
        })
        .collect();
    format!("{{\"subscriptions\":[{}]}}", entries.join(","))
}

/// Params for a request that takes none: mandatory on the wire anyway.
pub const EMPTY_PARAMS: &str = "{}";

/// The state of one socket conversation.
#[derive(Debug, Default)]
pub struct SocketClient {
    partial: Vec<u8>,
    pump_ready: bool,
    next_id: u64,
    pending: BTreeSet<String>,
    closed: bool,
}

impl SocketClient {
    /// Same cap and reason as the frame parser's
    /// ([`crate::frame::FrameParser::MAX_BUFFER_BYTES`]), and the same fix
    /// (buffer-cap): complete lines are routed before the cap is applied.
    pub const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;

    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    pub fn pump_ready(&self) -> bool {
        self.pump_ready
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Requests sent and not yet answered.
    pub fn pending(&self) -> impl Iterator<Item = &str> {
        self.pending.iter().map(String::as_str)
    }

    /// Start a request: its id (`mp1`, `mp2`, …) and the line to write.
    /// `params` is a JSON object, e.g. [`EMPTY_PARAMS`] or
    /// [`subscribe_params`].
    pub fn request(&mut self, method: &str, params: &str) -> Result<(String, String), SocketError> {
        if self.closed {
            return Err(SocketError::ConnectionClosed);
        }
        let id = format!("mp{}", self.next_id);
        self.next_id += 1;
        let line = format!(
            "{{\"id\":{},\"method\":{},\"params\":{params}}}\n",
            json::encode_string(&id),
            json::encode_string(method),
        );
        self.pending.insert(id.clone());
        Ok((id, line))
    }

    /// `events.subscribe` for global `kinds` and per-pane ones.
    pub fn subscribe(
        &mut self,
        kinds: &[&str],
        pane_scoped: &[(&str, &str)],
    ) -> Result<(String, String), SocketError> {
        let subscriptions: Vec<Subscription<'_>> = kinds
            .iter()
            .map(|kind| Subscription {
                kind,
                pane_id: None,
            })
            .chain(pane_scoped.iter().map(|(kind, pane)| Subscription {
                kind,
                pane_id: Some(pane),
            }))
            .collect();
        self.request("events.subscribe", &subscribe_params(&subscriptions))
    }

    /// Pipe bytes in: complete lines are routed, a partial one waits.
    /// Lines that aren't JSON (shell noise on a PTY pipe) are skipped.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Output> {
        let mut outputs = Vec::new();
        let mut rest = data;
        while let Some(newline) = memchr(b'\n', rest) {
            let line = if self.partial.is_empty() {
                text::decode_utf8_lossy(&rest[..newline])
            } else {
                self.partial.extend_from_slice(&rest[..newline]);
                let line = text::decode_utf8_lossy(&self.partial);
                self.partial.clear();
                line
            };
            self.route(line, &mut outputs);
            rest = &rest[newline + 1..];
        }
        self.partial.extend_from_slice(rest);
        if self.partial.len() > Self::MAX_BUFFER_BYTES {
            self.partial = Vec::new();
        }
        outputs
    }

    /// The pipe ended: every request in flight fails, and no event follows.
    pub fn finish(&mut self) -> Vec<Output> {
        if self.closed {
            return Vec::new();
        }
        self.closed = true;
        self.fail_all(&SocketError::ConnectionClosed)
    }

    /// `id`'s deadline passed. A late reply then routes to nobody.
    pub fn expire(&mut self, id: &str) -> Option<Output> {
        self.fail(id, SocketError::Timeout)
    }

    /// `id` failed outside the conversation (its write did not go out).
    pub fn fail(&mut self, id: &str, error: SocketError) -> Option<Output> {
        self.pending.remove(id).then(|| Output::Reply {
            id: id.to_owned(),
            result: Err(error),
        })
    }

    fn fail_all(&mut self, error: &SocketError) -> Vec<Output> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|id| Output::Reply {
                id,
                result: Err(error.clone()),
            })
            .collect()
    }

    fn route(&mut self, line: String, outputs: &mut Vec<Output>) {
        // At the END of the line, never merely contained: the boot line
        // carries the marker in its own python source, and the shell echoes
        // that line before `stty -echo` takes effect. Not the whole line
        // either: zsh can glue a window-title escape in front of it.
        if text::trim_whitespaces_and_newlines(&line).ends_with(PUMP_READY_MARKER) {
            if !self.pump_ready {
                self.pump_ready = true;
                outputs.push(Output::PumpReady);
            }
            return;
        }
        let Some(object) = json::first_object(&line) else {
            return;
        };
        if let Some(kind) = object.str("event") {
            if !self.closed {
                outputs.push(Output::Event(SocketEvent {
                    kind: kind.to_owned(),
                }));
            }
            return;
        }
        let Some(id) = object.str("id") else {
            return;
        };
        if id.is_empty() {
            // The server could not tell which request this was, and hangs
            // up next: everything in flight fails now, not at its timeout.
            let error = server_error(&object).unwrap_or(SocketError::Server {
                code: "invalid_request".to_owned(),
                message: line.clone(),
            });
            outputs.extend(self.fail_all(&error));
            return;
        }
        if !self.pending.remove(id) {
            return;
        }
        let result = match server_error(&object) {
            Some(error) => Err(error),
            None => Ok(line.clone()),
        };
        outputs.push(Output::Reply {
            id: id.to_owned(),
            result,
        });
    }
}

fn server_error(object: &json::Object) -> Option<SocketError> {
    let error = object.object("error")?;
    Some(SocketError::Server {
        code: error.str("code").unwrap_or("unknown").to_owned(),
        message: error.str("message").unwrap_or("").to_owned(),
    })
}

/// The push driver's invalidation debounce, on an explicit clock (`now` is
/// any monotonic time): a burst of events costs one snapshot read once it
/// goes quiet, and a stream that never stops still gets one at least every
/// [`InvalidateDebounce::MAX_DEFERRAL`].
///
/// Each event RESETS the wait. (It once only looked like a debounce: the
/// guard was "is a timer already running", which fires once per interval
/// for as long as events keep coming; the subscribe's own bootstrap replay
/// made 37 snapshot reads in 8 seconds.)
#[derive(Debug, Clone, Default)]
pub struct InvalidateDebounce {
    burst_started: Option<Duration>,
    deadline: Option<Duration>,
}

impl InvalidateDebounce {
    pub const DELAY: Duration = Duration::from_millis(250);
    pub const MAX_DEFERRAL: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self::default()
    }

    /// An event at `now`. True when the invalidation fires right away (the
    /// burst has been deferring it for [`Self::MAX_DEFERRAL`]); otherwise it
    /// is due at [`Self::deadline`].
    pub fn event(&mut self, now: Duration) -> bool {
        let started = *self.burst_started.get_or_insert(now);
        if now.saturating_sub(started) >= Self::MAX_DEFERRAL {
            self.fire();
            return true;
        }
        self.deadline = Some(now + Self::DELAY);
        false
    }

    /// When the pending invalidation is due, if one is.
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// The timer at `now`: true when the invalidation fires.
    pub fn tick(&mut self, now: Duration) -> bool {
        match self.deadline {
            Some(deadline) if now >= deadline => {
                self.fire();
                true
            }
            _ => false,
        }
    }

    fn fire(&mut self) {
        self.burst_started = None;
        self.deadline = None;
    }
}
