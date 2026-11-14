//! The equivalence harness's Rust side: one case per line in, one result line
//! out, in the format `harness/swift/main.swift` prints for the app's Swift
//! code. `harness/equivalence.py` feeds both the same cases and compares;
//! `tests/equivalence.rs` checks the committed corpus against the recorded
//! Swift output without swiftc.
//!
//! A case is a kind and its arguments, separated by spaces: `x:<hex>` is a
//! string (its UTF-8 bytes, or raw bytes for byte payloads), `-` is nil,
//! anything else is a number or a word. Results print strings as `"…"`
//! with printable ASCII as itself and every other scalar as `\u{HEX}`,
//! bytes as `b:<hex>`, nil as `nil`.

use std::fmt::Write as _;
use std::ops::Range;

use crate::control::{self, CommandResult, PollEffect, PollReading, PollState};
use crate::frame::{self, Frame, FrameParser, ScrollSource};
use crate::json::{self, Number, Value};
use crate::launch;
use crate::snapshot::{self, Decoded, Snapshot};
use crate::socket::{self, Output, SocketClient, SocketError};
use crate::version::{self, VersionReport};

/// SplitMix64, the plans' only source of randomness (same constants as the
/// Swift side and the tmux harness).
#[derive(Debug, Clone)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The chunks a plan cuts `len` bytes into: `whole`, `bytes`, `fixed:N`,
/// `rand:SEED:MAX` (sizes uniform in `1..=MAX`).
pub fn plan(spec: &str, len: usize) -> Result<Vec<Range<usize>>, String> {
    let fields: Vec<&str> = spec.split(':').collect();
    let number = |index: usize| -> Result<u64, String> {
        fields
            .get(index)
            .and_then(|field| field.parse().ok())
            .filter(|&value| value > 0)
            .ok_or_else(|| format!("bad plan {spec:?}"))
    };
    let mut sizes: Box<dyn FnMut() -> usize> = match fields[0] {
        "whole" => Box::new(move || len.max(1)),
        "bytes" => Box::new(|| 1),
        "fixed" => {
            let size = usize::try_from(number(1)?).map_err(|error| error.to_string())?;
            Box::new(move || size)
        }
        "rand" => {
            let mut rng = SplitMix64(
                fields
                    .get(1)
                    .and_then(|f| f.parse().ok())
                    .ok_or("bad seed")?,
            );
            let max = number(2)?;
            Box::new(move || usize::try_from(1 + rng.next_u64() % max).unwrap_or(1))
        }
        _ => return Err(format!("bad plan {spec:?}")),
    };
    let mut chunks = Vec::new();
    let mut at = 0;
    while at < len {
        let end = len.min(at + sizes());
        chunks.push(at..end);
        at = end;
    }
    Ok(chunks)
}

/// A string as the harness prints it.
pub fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for scalar in value.chars() {
        match scalar {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(scalar),
            _ => {
                let _ = write!(out, "\\u{{{:X}}}", u32::from(scalar));
            }
        }
    }
    out.push('"');
    out
}

fn quoted_opt(value: Option<&str>) -> String {
    value.map_or_else(|| "nil".to_owned(), quoted)
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("odd hex {text:?}"));
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).map_err(|error| error.to_string()))
        .collect()
}

/// A case's argument list.
struct Args<'a> {
    tokens: std::slice::Iter<'a, &'a str>,
}

impl Args<'_> {
    fn next(&mut self) -> Result<&str, String> {
        self.tokens
            .next()
            .copied()
            .ok_or_else(|| "missing argument".to_owned())
    }

    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let token = self.next()?;
        let hex = token
            .strip_prefix("x:")
            .ok_or_else(|| format!("not bytes: {token:?}"))?;
        unhex(hex)
    }

    fn opt_bytes(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.tokens.as_slice().first() == Some(&"-") {
            self.tokens.next();
            return Ok(None);
        }
        self.bytes().map(Some)
    }

    fn string(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?).map_err(|error| error.to_string())
    }

    fn opt_string(&mut self) -> Result<Option<String>, String> {
        self.opt_bytes()?
            .map(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()))
            .transpose()
    }

    fn int(&mut self) -> Result<i64, String> {
        let token = self.next()?;
        token
            .parse()
            .map_err(|_| format!("not a number: {token:?}"))
    }

    fn rest(&mut self) -> Vec<&str> {
        self.tokens.by_ref().copied().collect()
    }
}

pub fn frame_text(frame: &Frame) -> String {
    match frame {
        Frame::Screen {
            seq,
            width,
            height,
            full,
            bytes,
        } => format!("screen({seq},{width},{height},{full},b:{})", hex(bytes)),
        Frame::Closed { reason } => format!("closed({})", quoted_opt(reason.as_deref())),
        Frame::LoopReady => "ready".to_owned(),
        Frame::Fault { message } => format!("fault({})", quoted(message)),
    }
}

/// A JSON value as the harness prints it: numbers by class and by what the
/// casts make of them.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::String(text) => quoted(text),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(value_text).collect();
            format!("[{}]", items.join(","))
        }
        Value::Object(object) => object_text(object),
        Value::Bool(_) | Value::Number(_) => {
            let class = match value {
                Value::Bool(_) => "bool".to_owned(),
                Value::Number(Number::Int(_)) => "int".to_owned(),
                Value::Number(Number::UInt(_)) => "uint".to_owned(),
                Value::Number(Number::Double(value)) => format!("dbl:{:016x}", value.to_bits()),
                Value::Number(Number::Decimal(_)) => "dec".to_owned(),
                _ => unreachable!(),
            };
            let opt = |value: Option<String>| value.unwrap_or_else(|| "-".to_owned());
            format!(
                "n({class},{},{},{})",
                opt(value.as_i64().map(|v| v.to_string())),
                opt(value.as_u64().map(|v| v.to_string())),
                opt(value.as_bool().map(|v| v.to_string())),
            )
        }
    }
}

pub fn object_text(object: &json::Object) -> String {
    let entries: Vec<String> = object
        .iter()
        .map(|(key, value)| format!("{}:{}", quoted(key), value_text(value)))
        .collect();
    format!("{{{}}}", entries.join(","))
}

pub fn snapshot_text(snapshot: &Snapshot) -> String {
    let sessions: Vec<String> = snapshot
        .sessions
        .values()
        .map(|s| format!("{}=({},{})", quoted(&s.id), quoted(&s.name), s.is_attached))
        .collect();
    let windows: Vec<String> = snapshot
        .windows
        .values()
        .map(|w| {
            format!(
                "{}=({},{},{},{},{},{},{})",
                quoted(&w.id),
                quoted(&w.session_id),
                quoted(&w.name),
                w.index,
                quoted(&w.layout),
                w.is_active,
                w.pane_count,
                w.is_zoomed
            )
        })
        .collect();
    let panes: Vec<String> = snapshot
        .panes
        .values()
        .map(|p| {
            format!(
                "{}=({},{},{},{},{},{})",
                quoted(&p.id),
                quoted(&p.window_id),
                p.index,
                quoted(&p.command),
                p.width,
                p.height,
                p.is_active
            )
        })
        .collect();
    format!(
        "sessions[{}] windows[{}] panes[{}] active({},{},{}) attached({},{})",
        sessions.join(" "),
        windows.join(" "),
        panes.join(" "),
        quoted_opt(snapshot.active_session_id.as_deref()),
        quoted_opt(snapshot.active_window_id.as_deref()),
        quoted_opt(snapshot.active_pane_id.as_deref()),
        snapshot.is_attached,
        snapshot.ever_attached,
    )
}

fn map_text(map: &std::collections::BTreeMap<String, String>) -> String {
    let entries: Vec<String> = map
        .iter()
        .map(|(key, value)| format!("{}={}", quoted(key), quoted(value)))
        .collect();
    format!("[{}]", entries.join(" "))
}

fn hooks_text(hooks: &std::collections::BTreeMap<String, snapshot::AgentHook>) -> String {
    let entries: Vec<String> = hooks
        .iter()
        .map(|(id, hook)| {
            format!(
                "{}=({},{},{})",
                quoted(id),
                quoted_opt(hook.state.map(|state| state.as_str())),
                quoted_opt(hook.agent.as_deref()),
                quoted_opt(hook.title.as_deref())
            )
        })
        .collect();
    format!("[{}]", entries.join(" "))
}

pub fn decoded_text(decoded: &Decoded) -> String {
    format!(
        "{} hooks{} cwds{} tids{} wt{}",
        snapshot_text(&decoded.snapshot),
        hooks_text(&decoded.agent_hooks),
        map_text(&decoded.pane_cwds),
        map_text(&decoded.terminal_ids),
        map_text(&decoded.worktree_repos),
    )
}

fn report_text(report: Option<&VersionReport>) -> String {
    match report {
        None => "nil".to_owned(),
        Some(report) => format!(
            "({},{},{},{},{})",
            quoted(&report.client),
            quoted(&report.server),
            report.mismatched(),
            report.client_is_older(),
            quoted(&report.notice().english())
        ),
    }
}

fn result_arg(args: &mut Args<'_>) -> Result<Option<CommandResult>, String> {
    let output = args.string()?;
    let exit = args.next()?;
    Ok(match exit {
        "none" => None,
        "-" => Some(CommandResult {
            output,
            exit_code: None,
        }),
        code => Some(CommandResult {
            output,
            exit_code: Some(code.parse().map_err(|_| format!("bad exit {code:?}"))?),
        }),
    })
}

fn split_list(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(',').collect()
    }
}

/// Run one case line; the line it prints.
pub fn run_case(line: &str) -> Result<String, String> {
    let tokens: Vec<&str> = line.split(' ').filter(|token| !token.is_empty()).collect();
    let Some((&kind, rest)) = tokens.split_first() else {
        return Err("empty case".to_owned());
    };
    let mut args = Args {
        tokens: rest.iter(),
    };
    let out = match kind {
        // MARK: frame channel
        "frames" => {
            let spec = args.next()?.to_owned();
            let stream = args.bytes()?;
            let mut parser = FrameParser::new();
            let mut frames = Vec::new();
            for chunk in plan(&spec, stream.len())? {
                frames.extend(parser.consume(&stream[chunk]).iter().map(frame_text));
            }
            format!("[{}]", frames.join(" "))
        }
        "line" => frame::parse_line(&args.bytes()?)
            .map_or_else(|| "nil".to_owned(), |frame| frame_text(&frame)),
        "drop" => frame::is_handshake_drop(&args.string()?).to_string(),
        "start" | "boot" => {
            // The target, the grid, the herdr, `take` for the person's
            // pick (`--takeover`) or `plain`; a boot line then the
            // connection whose pid file it keeps.
            let target = args.string()?;
            let (cols, rows) = (args.int()?, args.int()?);
            let path = args.opt_string()?;
            let takeover = args.next()? == "take";
            if kind == "start" {
                quoted(&frame::start(
                    &target,
                    cols,
                    rows,
                    path.as_deref(),
                    takeover,
                ))
            } else {
                let connection = args.string()?;
                quoted(&frame::boot(
                    &target,
                    cols,
                    rows,
                    path.as_deref(),
                    takeover,
                    &connection,
                ))
            }
        }
        "retarget" => {
            let target = args.string()?;
            let (cols, rows) = (args.int()?, args.int()?);
            let takeover = args.next()? == "take";
            quoted(&frame::retarget(&target, cols, rows, takeover))
        }
        "input" => quoted(&frame::input(&args.bytes()?)),
        "resize" => quoted(&frame::resize(args.int()?, args.int()?)),
        "scroll" => {
            let up = args.int()? != 0;
            let lines = args.int()?;
            let source = match args.next()? {
                "wheel" => ScrollSource::Wheel,
                "page_key" => ScrollSource::PageKey,
                other => return Err(format!("bad source {other:?}")),
            };
            quoted(&frame::scroll(up, lines, source))
        }
        "consts" => format!(
            "{} {} {}",
            quoted(frame::RELEASE),
            quoted(frame::READY_MARKER),
            quoted(frame::READY_TYPE)
        ),
        "paste" => format!("b:{}", hex(&frame::bracketed_paste(&args.string()?))),
        "arrow" => quoted_opt(frame::arrow_key_name(&args.bytes()?)),
        // MARK: JSON and snapshots
        "json" => json::parse(&args.bytes()?).map_or_else(|| "nil".to_owned(), |v| value_text(&v)),
        "firstobj" => json::first_object(&args.string()?)
            .map_or_else(|| "nil".to_owned(), |o| object_text(&o)),
        "snapshot" => {
            snapshot::decode(&args.string()?).map_or_else(|| "nil".to_owned(), |d| decoded_text(&d))
        }
        "paneindex" => snapshot::pane_index(&args.string()?).to_string(),
        "notrunning" => snapshot::is_server_not_running(&args.string()?).to_string(),
        "mismatch" => quoted_opt(snapshot::protocol_mismatch(&args.string()?).as_deref()),
        "skew" => {
            let message = args.string()?;
            format!(
                "{} {}",
                snapshot::looks_like_version_skew(&message),
                snapshot::client_is_older(&message)
            )
        }
        // MARK: versions
        "status" => report_text(VersionReport::parse(&args.string()?).as_ref()),
        "vcompare" => {
            let (a, b) = (args.string()?, args.string()?);
            match version::compare(&a, &b) {
                std::cmp::Ordering::Less => "-1",
                std::cmp::Ordering::Equal => "0",
                std::cmp::Ordering::Greater => "1",
            }
            .to_owned()
        }
        // MARK: socket
        "socket" => socket_case(&mut args)?,
        "request" => {
            let method = args.string()?;
            let mut client = SocketClient::new();
            let (_, line) = client
                .request(&method, socket::EMPTY_PARAMS)
                .map_err(|error| format!("{error:?}"))?;
            quoted(line.trim_end_matches('\n'))
        }
        "subscribe" => {
            let kinds = args.string()?;
            let scoped = args.string()?;
            let scoped: Vec<(&str, &str)> = split_list(&scoped)
                .into_iter()
                .map(|entry| entry.split_once('=').unwrap_or((entry, "")))
                .collect();
            let mut client = SocketClient::new();
            let (_, line) = client
                .subscribe(&split_list(&kinds), &scoped)
                .map_err(|error| format!("{error:?}"))?;
            quoted(line.trim_end_matches('\n'))
        }
        "pumpboot" => quoted(&socket::pump_boot_line()),
        "kinds" => {
            let mut scoped = socket::PANE_SCOPED_KINDS;
            scoped.sort_unstable();
            format!("{} | {}", socket::GLOBAL_KINDS.join(" "), scoped.join(" "))
        }
        // MARK: launch
        "launch.attach" => quoted(&launch::attach_command(args.opt_string()?.as_deref())),
        "launch.command" => {
            let sub = args.string()?;
            quoted(&launch::command(&sub, args.opt_string()?.as_deref()))
        }
        "launch.daemon" => quoted(&launch::daemon_command(args.opt_string()?.as_deref())),
        "launch.key" => {
            let id = args.string()?;
            quoted(&launch::mosh_renderer_key(&id, &args.string()?))
        }
        "launch.paths" => {
            let key = args.string()?;
            format!(
                "{} {}",
                quoted(&launch::mosh_target_path(&key)),
                quoted(&launch::mosh_pid_path(&key))
            )
        }
        "launch.loop" => {
            let key = args.string()?;
            quoted(&launch::raw_attach_loop_command(
                &key,
                args.opt_string()?.as_deref(),
            ))
        }
        "launch.cleanup" => quoted(&launch::stale_renderer_cleanup_command(&args.string()?)),
        "launch.retarget" => {
            // Whether the person chose the pane (`--takeover`), then the
            // terminal and the key.
            let takeover = args.int()? != 0;
            let terminal = args.string()?;
            let key = args.string()?;
            // The herdr the loop runs, when the case names one.
            let path = if args.tokens.as_slice().is_empty() {
                None
            } else {
                args.opt_string()?
            };
            quoted(&launch::retarget_command(
                &terminal,
                &key,
                path.as_deref(),
                takeover,
            ))
        }
        "launch.missing" => launch::retarget_missing(&args.bytes()?).to_string(),
        "launch.probe" => quoted(&launch::raw_attach_probe_command(
            args.opt_string()?.as_deref(),
        )),
        "launch.check" => {
            let terminal = args.string()?;
            quoted(&launch::raw_attach_check_command(
                &terminal,
                &args.string()?,
            ))
        }
        "launch.watch" => quoted(&launch::raw_attach_watch_command(&args.string()?)),
        "launch.loopup" => quoted(&launch::raw_attach_loop_up_command(&args.string()?)),
        "launch.isup" => launch::loop_up(&args.bytes()?).to_string(),
        "launch.stop" => match launch::renderer_stop(&args.bytes()?) {
            Some(launch::RendererStop::Parked {
                status,
                terminal,
                said,
            }) => format!("parked({status},{},{})", quoted(&terminal), quoted(&said)),
            Some(launch::RendererStop::LoopGone) => "loopGone".to_owned(),
            None => "nil".to_owned(),
        },
        "launch.stopkind" => {
            let status = args.int()?;
            let said = args.string()?;
            let listed = args.int()? != 0;
            let up = args.int()? != 0;
            match launch::renderer_stop_kind(status, &said, listed, up) {
                launch::StopKind::TakenOver => "takenOver",
                launch::StopKind::Gone => "gone",
                launch::StopKind::Refused => "refused",
            }
            .to_owned()
        }
        "launch.landed" => launch::retarget_landed(&args.bytes()?).to_string(),
        "launch.up" => launch::raw_attach_up(&args.bytes()?).to_string(),
        "quote" => quoted(&launch::quote(&args.string()?)),
        // MARK: control
        "ctl.parse" => {
            let result = control::parse_result(&args.string()?);
            format!(
                "({},{})",
                quoted(&result.output),
                result
                    .exit_code
                    .map_or_else(|| "nil".to_owned(), |code| code.to_string())
            )
        }
        "ctl.refusal" => {
            let result = result_arg(&mut args)?;
            quoted_opt(
                control::refusal(result.as_ref())
                    .map(|r| r.english())
                    .as_deref(),
            )
        }
        "ctl.notup" => {
            let result = result_arg(&mut args)?.ok_or("notup needs a result")?;
            control::agent_not_up_yet(&result).to_string()
        }
        "ctl.errmsg" => quoted_opt(
            json::first_object(&args.string()?)
                .and_then(|object| control::error_message(&object))
                .as_deref(),
        ),
        "ctl.first" => {
            let message = args.string()?;
            let pane = args.string()?;
            let version = args.opt_string()?;
            format!(
                "{} {}",
                quoted(&control::first_message(&message, &pane, version.as_deref())),
                control::types_without_submitting(version.as_deref())
            )
        }
        "ctl.submit" => quoted(&control::submit(&args.string()?)),
        "ctl.homescan" => quoted(control::HOME_SCAN),
        "ctl.validate" => {
            let request = control::AgentTaskRequest {
                repo_path: args.string()?,
                branch: args.string()?,
                agent: args.string()?,
                prompt: String::new(),
            };
            quoted_opt(request.validation_error().map(|invalid| invalid.english()))
        }
        "ctl.repolines" => {
            let lines = control::repo_lines(&args.string()?);
            let lines: Vec<String> = lines.iter().map(|line| quoted(line)).collect();
            format!("[{}]", lines.join(" "))
        }
        "ctl.cmd" => command_case(&mut args)?,
        "ctl.poll" => poll_case(&mut args)?,
        other => return Err(format!("unknown case kind {other:?}")),
    };
    Ok(format!("{kind} {out}"))
}

fn socket_case(args: &mut Args<'_>) -> Result<String, String> {
    let spec = args.next()?.to_owned();
    let requests = args.int()?;
    let stream = args.bytes()?;
    let mut client = SocketClient::new();
    for _ in 0..requests {
        client
            .request("probe", socket::EMPTY_PARAMS)
            .map_err(|error| format!("{error:?}"))?;
    }
    let mut events = Vec::new();
    let mut replies = std::collections::BTreeMap::new();
    let mut take = |outputs: Vec<Output>, events: &mut Vec<String>| {
        for output in outputs {
            match output {
                Output::PumpReady => {}
                Output::Event(event) => events.push(quoted(&event.kind)),
                Output::Reply { id, result } => {
                    let text = match result {
                        Ok(line) => format!("ok({})", quoted(&line)),
                        Err(SocketError::Server { code, message }) => {
                            format!("server({},{})", quoted(&code), quoted(&message))
                        }
                        Err(SocketError::ConnectionClosed) => "closed".to_owned(),
                        Err(SocketError::Timeout) => "timeout".to_owned(),
                    };
                    replies.insert(id[2..].parse::<u64>().unwrap_or(0), text);
                }
            }
        }
    };
    for chunk in plan(&spec, stream.len())? {
        let outputs = client.feed(&stream[chunk]);
        take(outputs, &mut events);
    }
    let ready = client.pump_ready();
    let outputs = client.finish();
    take(outputs, &mut events);
    let replies: Vec<String> = replies
        .iter()
        .map(|(id, text)| format!("mp{id}={text}"))
        .collect();
    Ok(format!(
        "ready={ready} events[{}] replies[{}]",
        events.join(" "),
        replies.join(" ")
    ))
}

/// The command lines one control-client action sends, given the tree the
/// client last read (`snapshot`, an `api snapshot` reply).
fn command_case(args: &mut Args<'_>) -> Result<String, String> {
    let tree = args.string()?;
    let path = args.opt_string()?;
    let path = path.as_deref();
    let action = args.next()?.to_owned();
    let snapshot = snapshot::decode(&tree)
        .map(|decoded| decoded.snapshot)
        .unwrap_or_default();
    let line = |sub: String| control::command_line(&sub, path);
    let lines: Vec<String> = match action.as_str() {
        "sel-session" => vec![line(control::select_session(&args.string()?))],
        "sel-window" => vec![line(control::select_window(&args.string()?))],
        "sel-pane" => vec![line(control::select_pane(&args.string()?, path))],
        "sel-pane-imm" => vec![line(control::immersive_zoom(&args.string()?))],
        "kill-session" => vec![line(control::kill_session(&args.string()?))],
        "kill-window" => vec![line(control::kill_window(&args.string()?))],
        "kill-pane" => vec![line(control::kill_pane(&args.string()?))],
        "ren-session" => {
            let id = args.string()?;
            vec![line(control::rename_session(&id, &args.string()?))]
        }
        "ren-window" => {
            let id = args.string()?;
            vec![line(control::rename_window(&id, &args.string()?))]
        }
        "new-session" => vec![line(control::new_session(args.opt_string()?.as_deref()))],
        "new-window" => vec![line(control::new_window(args.opt_string()?.as_deref()))],
        "new-pane" => control::new_pane(&snapshot).into_iter().map(line).collect(),
        "new-window-in" => {
            let session = args.string()?;
            let name = args.opt_string()?;
            control::new_window_in_session(&snapshot, &session, name.as_deref())
                .into_iter()
                .map(line)
                .collect()
        }
        "new-pane-in" => control::new_pane_in_window(&snapshot, &args.string()?)
            .unwrap_or_default()
            .into_iter()
            .map(line)
            .collect(),
        "keys" => {
            let pane = args.string()?;
            let keys = args.string()?;
            control::send_keys(&split_list(&keys), &pane)
                .into_iter()
                .map(line)
                .collect()
        }
        "remove" => {
            let id = args.string()?;
            vec![line(control::worktree_remove(&id, args.int()? != 0))]
        }
        "manifests" => {
            // The reply to `server agent-manifests --json`, and the names.
            let raw = args.string()?;
            let names: Vec<String> = control::agent_names(&raw)
                .iter()
                .map(|n| quoted(n))
                .collect();
            return Ok(format!(
                "cmds[{}] names[{}]",
                quoted(&launch::command(control::AGENT_MANIFESTS, path)),
                names.join(" ")
            ));
        }
        other => return Err(format!("unknown action {other:?}")),
    };
    let lines: Vec<String> = lines.iter().map(|line| quoted(line)).collect();
    Ok(format!("cmds[{}]", lines.join(" ")))
}

/// Successive `api snapshot` replies through the poll; the state after each.
fn poll_case(args: &mut Args<'_>) -> Result<String, String> {
    let status = args.opt_string()?;
    let mut state = PollState::new();
    let mut out = Vec::new();
    for token in args.rest() {
        let raw = Args {
            tokens: [token].iter(),
        }
        .string()?;
        let reading: PollReading = control::read_poll(&control::parse_result(&raw));
        let mut focus = Vec::new();
        for effect in state.apply(reading) {
            match effect {
                PollEffect::DiagnoseSkew => {
                    let report = status
                        .as_deref()
                        .map(control::parse_result)
                        .and_then(|result| VersionReport::parse(&result.output));
                    state.note_versions(report);
                }
                PollEffect::FocusedPane(pane) => focus.push(quoted(&pane)),
                PollEffect::Mismatch(_) | PollEffect::HooksUpdated | PollEffect::Zoom(_) => {}
            }
        }
        out.push(format!(
            "[{} hooks{} loaded={} snr={} mm={} skew={} iv={} focus[{}]]",
            snapshot_text(&state.snapshot),
            hooks_text(&state.agent_hooks),
            state.agent_hooks_loaded,
            state.server_not_running,
            quoted_opt(state.protocol_mismatch.as_deref()),
            report_text(state.version_skew.as_ref()),
            state.interval.as_secs(),
            focus.join(" ")
        ));
    }
    Ok(out.join(" "))
}

/// Every case of a corpus file (one per line; blank lines and `#` comments
/// skipped), with its result or the reason it could not run.
pub fn run_corpus(corpus: &str) -> Vec<String> {
    corpus
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| run_case(line).unwrap_or_else(|error| format!("error {error}")))
        .collect()
}
