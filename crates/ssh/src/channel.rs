//! A process on the host: a command on an exec channel, or a shell on a
//! PTY. Its output, its exit, its end — and the difference between a host
//! that closed the channel (a program that finished, an `exit` typed into
//! the shell) and a connection that went away under it (B103).

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use russh::client::Msg;
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Sig};
use tokio::sync::watch;

use crate::connection::ConnectionEnd;
use crate::error::SshError;

/// stderr travels as SSH "extended data" with this code (RFC 4254 §5.2).
const STDERR: u32 = 1;

/// What a terminal asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyRequest {
    pub term: String,
    pub cols: u32,
    pub rows: u32,
    /// Sent before the shell starts, without asking for a reply.
    pub env: Vec<(String, String)>,
    /// `None` for the login shell.
    pub command: Option<String>,
}

impl PtyRequest {
    /// What the app asks for: `xterm-256color`, and `LANG=en_US.UTF-8` so a
    /// Linux host that would start in the C locale speaks UTF-8 (CJK input
    /// and output otherwise come out as `???`). OpenSSH accepts `LANG` by
    /// default.
    pub fn new(cols: u32, rows: u32) -> Self {
        Self {
            term: "xterm-256color".to_owned(),
            cols,
            rows,
            env: vec![("LANG".to_owned(), "en_US.UTF-8".to_owned())],
            command: None,
        }
    }
}

/// How a program ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Status(u32),
    Signal {
        /// Without the `SIG` prefix: `TERM`, `KILL`, …
        signal: String,
        core_dumped: bool,
        message: String,
    },
}

/// What a process sends back, in the order it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Stdout(Bytes),
    Stderr(Bytes),
    /// The program ended. Output may still follow until the channel closes.
    Exited(Exit),
    /// The host won't send more output.
    Eof,
    /// The host closed the channel: the program is done. Last event.
    Closed,
    /// The connection went away (or was closed) before the host closed the
    /// channel. Last event.
    Lost,
}

/// The reading side of a process.
pub struct ProcessReader {
    half: ChannelReadHalf,
    /// Events that arrived before the host confirmed the request.
    early: VecDeque<Event>,
    finished: bool,
    /// The channel's close, owed once neither side is held.
    channel: Arc<Owned>,
}

/// A channel the host agreed to open, closed once no one reads or writes
/// it (B720). A process dropped without a close — a pump given up, herdr's
/// push abandoned — was only no longer read: the host kept its session
/// open against MaxSessions until the connection ended.
struct Owned {
    /// Taken only to close it, when the last side goes.
    half: Option<ChannelWriteHalf<Msg>>,
    /// The host closed it, or our close went: nothing owed.
    done: AtomicBool,
}

impl Owned {
    fn half(&self) -> &ChannelWriteHalf<Msg> {
        self.half.as_ref().expect("held until the channel is dropped")
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if *self.done.get_mut() {
            return;
        }
        if let (Some(half), Ok(runtime)) =
            (self.half.take(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move {
                let _ = half.close().await;
            });
        }
    }
}

impl ProcessReader {
    /// The next thing the process sent; `None` after [`Event::Closed`] or
    /// [`Event::Lost`].
    pub async fn next(&mut self) -> Option<Event> {
        if let Some(event) = self.early.pop_front() {
            self.finished |= is_last(&event);
            return Some(event);
        }
        if self.finished {
            return None;
        }
        loop {
            match self.half.wait().await {
                Some(msg) => {
                    if let Some(event) = event_of(msg) {
                        if event == Event::Closed {
                            self.channel.done.store(true, Ordering::Relaxed);
                        }
                        self.finished |= is_last(&event);
                        return Some(event);
                    }
                }
                None => {
                    self.finished = true;
                    return Some(Event::Lost);
                }
            }
        }
    }
}

/// The writing side of a process. Cheap to clone; writes, resizes and
/// signals from every clone go out one at a time, in the order they were
/// made (the byte path never reorders).
#[derive(Clone)]
pub struct ProcessWriter {
    inner: Arc<WriterInner>,
}

struct WriterInner {
    channel: Arc<Owned>,
    host: String,
    order: tokio::sync::Mutex<()>,
    /// The connection's end. A write waiting for window on a connection
    /// that died would wait forever: nobody grows the window of a dead one.
    end: watch::Receiver<Option<ConnectionEnd>>,
}

impl ProcessWriter {
    /// Sends bytes to the process's stdin (or the terminal's input).
    pub async fn write(&self, bytes: &[u8]) -> Result<(), SshError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let data = Bytes::copy_from_slice(bytes);
        self.in_turn(self.inner.channel.half().data_bytes(data)).await
    }

    /// Tells the terminal its new size.
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        self.in_turn(self.inner.channel.half().window_change(cols, rows, 0, 0))
            .await
    }

    /// Delivers a signal to the process (OpenSSH honours this for commands
    /// run without a PTY).
    pub async fn signal(&self, signal: Sig) -> Result<(), SshError> {
        self.in_turn(self.inner.channel.half().signal(signal)).await
    }

    /// Ends the process's stdin.
    pub async fn close_stdin(&self) -> Result<(), SshError> {
        self.in_turn(self.inner.channel.half().eof()).await
    }

    /// Asks the host to close the channel; the reader then ends with
    /// [`Event::Closed`].
    pub async fn close(&self) -> Result<(), SshError> {
        let closed = self.in_turn(self.inner.channel.half().close()).await;
        self.inner.channel.done.store(true, Ordering::Relaxed);
        closed
    }

    /// One operation at a time, in call order, given up if the connection
    /// ends first.
    async fn in_turn(
        &self,
        operation: impl Future<Output = Result<(), russh::Error>>,
    ) -> Result<(), SshError> {
        let _turn = self.inner.order.lock().await;
        let mut end = self.inner.end.clone();
        tokio::select! {
            result = operation => result.map_err(|e| self.closed(e)),
            _ = end.wait_for(Option::is_some) => Err(SshError::Closed {
                host: self.inner.host.clone(),
            }),
        }
    }

    fn closed(&self, e: russh::Error) -> SshError {
        match e {
            russh::Error::SendError => SshError::Closed {
                host: self.inner.host.clone(),
            },
            e => SshError::Protocol(e),
        }
    }
}

/// A process the host agreed to run.
pub struct Process {
    reader: ProcessReader,
    writer: ProcessWriter,
}

impl Process {
    /// Waits for the host's answer to the request just sent on `channel`
    /// (exec, shell, subsystem), keeping anything that arrives first.
    pub(crate) async fn opened(
        channel: Channel<Msg>,
        host: String,
        end: watch::Receiver<Option<ConnectionEnd>>,
        what: &str,
    ) -> Result<Self, SshError> {
        let (mut read, write) = channel.split();
        let mut early = VecDeque::new();
        loop {
            match read.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) => {
                    // Refused, the channel is still open on the host: closed
                    // here, or it holds a session there (B720).
                    let _ = write.close().await;
                    return Err(SshError::Refused {
                        host,
                        what: what.to_owned(),
                    });
                }
                Some(msg) => {
                    if let Some(event) = event_of(msg) {
                        let last = is_last(&event);
                        early.push_back(event);
                        if last {
                            break;
                        }
                    }
                }
                None => return Err(SshError::Closed { host }),
            }
        }
        let channel = Arc::new(Owned {
            half: Some(write),
            done: AtomicBool::new(early.contains(&Event::Closed)),
        });
        Ok(Self {
            reader: ProcessReader {
                half: read,
                early,
                finished: false,
                channel: channel.clone(),
            },
            writer: ProcessWriter {
                inner: Arc::new(WriterInner {
                    channel,
                    host,
                    order: tokio::sync::Mutex::new(()),
                    end,
                }),
            },
        })
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.reader.next().await
    }

    pub async fn write(&self, bytes: &[u8]) -> Result<(), SshError> {
        self.writer.write(bytes).await
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        self.writer.resize(cols, rows).await
    }

    pub async fn signal(&self, signal: Sig) -> Result<(), SshError> {
        self.writer.signal(signal).await
    }

    pub async fn close_stdin(&self) -> Result<(), SshError> {
        self.writer.close_stdin().await
    }

    pub async fn close(&self) -> Result<(), SshError> {
        self.writer.close().await
    }

    pub fn writer(&self) -> ProcessWriter {
        self.writer.clone()
    }

    /// Separates reading (one task pumps the output) from writing (any
    /// number of callers).
    pub fn split(self) -> (ProcessReader, ProcessWriter) {
        (self.reader, self.writer)
    }

    /// Reads until the channel is closed or lost.
    pub async fn collect(mut self) -> Output {
        let mut output = Output::default();
        while let Some(event) = self.next().await {
            match event {
                Event::Stdout(bytes) => output.chunks.push((Stream::Stdout, bytes)),
                Event::Stderr(bytes) => output.chunks.push((Stream::Stderr, bytes)),
                Event::Exited(exit) => output.exit = Some(exit),
                Event::Eof => {}
                Event::Closed => output.closed = true,
                Event::Lost => output.closed = false,
            }
        }
        output
    }
}

/// Which of a process's two outputs a chunk came on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Everything a finished process produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// In arrival order.
    pub chunks: Vec<(Stream, Bytes)>,
    /// `None` when the channel ended without the host reporting how the
    /// program ended — which, with `closed` false, means the connection
    /// dropped mid-command: its output is partial.
    pub exit: Option<Exit>,
    /// The host closed the channel (rather than the connection going away).
    pub closed: bool,
}

impl Output {
    pub fn stdout(&self) -> Vec<u8> {
        self.of(Some(Stream::Stdout))
    }

    pub fn stderr(&self) -> Vec<u8> {
        self.of(Some(Stream::Stderr))
    }

    /// stdout and stderr interleaved as they arrived: what the app's
    /// `executeCommand` returns (`mergeStreams: true`).
    pub fn merged(&self) -> Vec<u8> {
        self.of(None)
    }

    pub fn status(&self) -> Option<u32> {
        match self.exit {
            Some(Exit::Status(status)) => Some(status),
            _ => None,
        }
    }

    /// Exited with status 0 and the host closed the channel.
    pub fn success(&self) -> bool {
        self.closed && self.status() == Some(0)
    }

    fn of(&self, stream: Option<Stream>) -> Vec<u8> {
        let mut out = Vec::new();
        for (from, bytes) in &self.chunks {
            if stream.is_none_or(|s| s == *from) {
                out.extend_from_slice(bytes);
            }
        }
        out
    }
}

fn is_last(event: &Event) -> bool {
    matches!(event, Event::Closed | Event::Lost)
}

fn event_of(msg: ChannelMsg) -> Option<Event> {
    match msg {
        ChannelMsg::Data { data } => Some(Event::Stdout(data)),
        ChannelMsg::ExtendedData { data, ext } if ext == STDERR => Some(Event::Stderr(data)),
        ChannelMsg::ExitStatus { exit_status } => Some(Event::Exited(Exit::Status(exit_status))),
        ChannelMsg::ExitSignal {
            signal_name,
            core_dumped,
            error_message,
            ..
        } => Some(Event::Exited(Exit::Signal {
            signal: signal_name_of(&signal_name),
            core_dumped,
            message: error_message,
        })),
        ChannelMsg::Eof => Some(Event::Eof),
        ChannelMsg::Close => Some(Event::Closed),
        _ => None,
    }
}

fn signal_name_of(signal: &Sig) -> String {
    match signal {
        Sig::ABRT => "ABRT",
        Sig::ALRM => "ALRM",
        Sig::FPE => "FPE",
        Sig::HUP => "HUP",
        Sig::ILL => "ILL",
        Sig::INT => "INT",
        Sig::KILL => "KILL",
        Sig::PIPE => "PIPE",
        Sig::QUIT => "QUIT",
        Sig::SEGV => "SEGV",
        Sig::TERM => "TERM",
        Sig::USR1 => "USR1",
        Sig::Custom(name) => return name.clone(),
    }
    .to_owned()
}
