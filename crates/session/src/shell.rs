//! A plain shell on an SSH terminal channel: the host's bytes into the
//! pane's byte channel, the person's keys and sizes out, and how the
//! channel ended.
//!
//! One task reads the channel and pushes what it reads into the stream as
//! it reads it (no hop, so no reordering); one task writes, one operation
//! at a time in the order asked (keys, then a resize, then keys). The read
//! end is moved into the reader: two readers on one channel (B4) can't be
//! written.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moshpit_ssh::{Event, Exit, Process, ProcessReader, ProcessWriter};
use moshpit_stream::Stream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// What the session asks of a shell.
#[derive(Debug)]
pub enum ShellCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// How a shell channel ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEnd {
    /// The host closed the channel in order: the shell exited.
    Exited { status: Option<u32> },
    /// The connection went away under it.
    Lost,
}

/// A running shell.
pub struct ShellLeg {
    pub commands: mpsc::UnboundedSender<ShellCommand>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl ShellLeg {
    /// Starts reading `pty` into `output`. `inbound` is stamped with
    /// `clock()` on every chunk (proof of life for the keepalive); `ended`
    /// hears how the channel ended, after every byte before it was pushed.
    pub fn spawn(
        pty: Process,
        output: Arc<Stream>,
        inbound: Arc<AtomicU64>,
        clock: impl Fn() -> u64 + Send + 'static,
        ended: Box<dyn FnOnce(ShellEnd) + Send>,
    ) -> Self {
        let (reader, writer) = pty.split();
        let (commands, rx) = mpsc::unbounded_channel();
        Self {
            commands,
            reader: tokio::spawn(read(reader, output, inbound, clock, ended)),
            writer: tokio::spawn(write(writer, rx)),
        }
    }

    /// Stops both tasks; the channel closes with its connection.
    pub fn stop(&self) {
        self.reader.abort();
        self.writer.abort();
    }
}

impl Drop for ShellLeg {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn read(
    mut reader: ProcessReader,
    output: Arc<Stream>,
    inbound: Arc<AtomicU64>,
    clock: impl Fn() -> u64,
    ended: Box<dyn FnOnce(ShellEnd) + Send>,
) {
    let mut status = None;
    let end = loop {
        match reader.next().await {
            Some(Event::Stdout(bytes)) | Some(Event::Stderr(bytes)) => {
                output.push(&bytes);
                inbound.store(clock(), Ordering::Relaxed);
            }
            Some(Event::Exited(Exit::Status(code))) => status = Some(code),
            Some(Event::Exited(Exit::Signal { .. })) | Some(Event::Eof) => {}
            Some(Event::Closed) => break ShellEnd::Exited { status },
            Some(Event::Lost) | None => break ShellEnd::Lost,
        }
    };
    ended(end);
}

async fn write(writer: ProcessWriter, mut commands: mpsc::UnboundedReceiver<ShellCommand>) {
    while let Some(command) = commands.recv().await {
        let result = match command {
            ShellCommand::Input(bytes) => writer.write(&bytes).await,
            ShellCommand::Resize { cols, rows } => {
                writer.resize(u32::from(cols), u32::from(rows)).await
            }
        };
        if result.is_err() {
            // The channel or its connection is gone; the reader says which.
            break;
        }
    }
}
