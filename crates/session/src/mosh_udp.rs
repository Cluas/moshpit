//! mosh's UDP side: the loop that runs `crates/mosh`'s [`Transport`] on a
//! datagram flow — its timers, its flow rebuilds and roaming, the host's
//! frames into the pane's byte channel.
//!
//! One task owns the transport, the flow and the clock: datagrams in, input
//! from the session, and the transport's deadline, whichever comes first;
//! then whatever the transport asks, in order (a datagram out, a frame for
//! the terminal, a fresh flow). A frame's bytes go straight into the
//! stream from here, in the order the transport applied them: no hop.
//!
//! Predictive echo's evidence rides the same channel: after a frame that
//! moves mosh's echo acknowledgement, a mark ([`crate::view_marks::ECHO_ACK`])
//! says how many bytes of the input written into the channel the server has
//! echoed (if it was going to) — the input's own byte offsets, which the view
//! counts as it writes, so it never needs mosh's state numbers. The view
//! judges its guesses by it exactly where the frame landed.
//!
//! The flow is a connected UDP socket ([`SystemUdp`]); behind the
//! [`UdpNetwork`] seam the tests swap in an in-memory network that drops and
//! blackholes on command ([`fake`]). mosh doesn't care which socket a
//! datagram comes from: a rebuild (the transport's liveness check, a forced
//! resume) or a roam (the network changed) opens a fresh one, and the
//! server re-homes to it on the next datagram.

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moshpit_mosh::{ChannelState, Diagnostics, Event as MoshEvent, Transport, TransportConfig};
use moshpit_stream::{Mark, Stream};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Opens datagram flows to a server.
pub trait UdpNetwork: Send + Sync {
    fn open(&self, server: SocketAddr) -> BoxFuture<'static, io::Result<Box<dyn UdpFlow>>>;
}

/// One flow: datagrams to and from the server it was opened for.
pub trait UdpFlow: Send + Sync {
    fn send<'a>(&'a self, datagram: &'a [u8]) -> BoxFuture<'a, io::Result<()>>;
    fn recv<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, io::Result<usize>>;
}

/// The system's UDP: a socket bound to the wildcard of the server's family,
/// connected to the server (so the kernel picks the route, and an ICMP error
/// comes back as an error on this socket alone).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemUdp;

impl UdpNetwork for SystemUdp {
    fn open(&self, server: SocketAddr) -> BoxFuture<'static, io::Result<Box<dyn UdpFlow>>> {
        Box::pin(async move {
            let local: SocketAddr = if server.is_ipv4() {
                (Ipv4Addr::UNSPECIFIED, 0).into()
            } else {
                (Ipv6Addr::UNSPECIFIED, 0).into()
            };
            let socket = UdpSocket::bind(local).await?;
            socket.connect(server).await?;
            Ok(Box::new(SystemFlow(socket)) as Box<dyn UdpFlow>)
        })
    }
}

struct SystemFlow(UdpSocket);

impl UdpFlow for SystemFlow {
    fn send<'a>(&'a self, datagram: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move { self.0.send(datagram).await.map(|_| ()) })
    }

    fn recv<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, io::Result<usize>> {
        Box::pin(self.0.recv(buf))
    }
}

/// What the session tells the mosh loop.
#[derive(Debug)]
pub enum MoshCommand {
    /// Keystrokes or a paste, in the order typed. `end`: the offset of the
    /// byte after them in everything the view wrote into the channel.
    Keys {
        bytes: Vec<u8>,
        end: u64,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    /// The app came back: `force` after a real suspension (a fresh flow and
    /// a full repaint), otherwise a datagram now so the server re-homes.
    Resume {
        force: bool,
    },
    /// The network path changed: a fresh flow on the new path.
    Roam,
    /// The person's Repaint (the mosh diagnostics): mosh-server redraws the
    /// whole screen (`Transport::request_full_redraw`), the field check for
    /// a screen that drifted from the server's (B338).
    Repaint,
    /// mosh with tmux, a scroll over a pane whose program doesn't want the
    /// mouse (B362): page tmux's history in copy-mode on the client mosh
    /// renders — `prefix` + `[` to enter it (only when not in it), then
    /// PageUp (positive `lines`) or PageDown (negative, only while in it),
    /// at most a page every [`PAGE_EVERY_MS`]. The next keystrokes leave
    /// copy-mode first (`q`, on the same lane), so they reach the shell.
    CopyScroll {
        lines: i64,
        prefix: Vec<u8>,
    },
    /// mosh with tmux, a mouse report for the program in tmux's client (a
    /// scroll's wheel over a pane whose program wants the mouse, B362): out
    /// of the copy-mode a scroll may have left the client in first (`q`, on
    /// the same lane), so the report reaches the program, not copy-mode.
    Mouse(Vec<u8>),
    /// Answer once mosh-server has acknowledged every user state queued so
    /// far (it acknowledges a state once it has written its keystrokes into
    /// the terminal): a keystroke sent before is in the tty. Answered at
    /// once when nothing is outstanding; dropped unanswered when the loop
    /// ends. Bounding the wait is the asker's.
    Barrier(tokio::sync::oneshot::Sender<()>),
    /// Stop: no more datagrams.
    Close,
}

/// The fastest a scroll pages tmux's copy-mode (a page a tick would rip
/// through the history on a fling).
pub const PAGE_EVERY_MS: u64 = 180;

/// Whether a scroll put tmux's client (the one mosh renders) in copy-mode,
/// and when it last paged (B362).
#[derive(Debug, Default)]
pub(crate) struct CopyMode {
    on: bool,
    last_page_at: Option<u64>,
}

impl CopyMode {
    /// The keys for one scroll step, or nothing (a step down while not in
    /// copy-mode, or too soon after the last page).
    pub(crate) fn page(&mut self, lines: i64, prefix: &[u8], now: u64) -> Option<Vec<u8>> {
        if lines == 0
            || self
                .last_page_at
                .is_some_and(|at| now.saturating_sub(at) < PAGE_EVERY_MS)
        {
            return None;
        }
        let keys = if lines > 0 {
            let mut keys = Vec::new();
            if !self.on {
                keys.extend_from_slice(prefix);
                keys.push(b'[');
            }
            keys.extend_from_slice(b"\x1b[5~");
            self.on = true;
            keys
        } else if self.on {
            b"\x1b[6~".to_vec()
        } else {
            return None;
        };
        self.last_page_at = Some(now);
        Some(keys)
    }

    /// Keystrokes after a scroll: `q` leaves copy-mode first, once.
    pub(crate) fn leave(&mut self, keys: &mut Vec<u8>) {
        if std::mem::take(&mut self.on) {
            keys.insert(0, b'q');
        }
    }
}

/// What the mosh loop tells the session.
#[derive(Debug, Clone, PartialEq)]
pub enum MoshNote {
    /// The first datagram from the server arrived: from here on the server
    /// is never killed for having been unreachable.
    FirstContact,
    /// Nothing comes back: offer SSH. `first_contact`: whether anything ever
    /// did (if not, the server is waiting for a client that won't come, and
    /// is reaped).
    ReturnPathDead {
        first_contact: bool,
    },
    /// A datagram from the server, after a [`MoshNote::ReturnPathDead`]: the
    /// path heals (B400). The next outage is reported again.
    ReturnPathAlive,
    /// mosh-server ended the session; the transport acked it and closed.
    SessionEnded,
    /// mosh-server is gone: after first contact its port refuses
    /// ([`Refusals`]) and nothing comes back (the host rebooted, the server
    /// was killed, or timed out while the phone was away). The loop stops;
    /// the session dials again (B402). It used to go on sending into the
    /// closed port: a frozen screen behind a live pill, then the banner
    /// offering SSH on a network that was fine.
    ServerGone,
    Metrics {
        srtt_ms: f64,
        roaming: bool,
    },
    /// The protocol's counters (the pill's long-press popover), at most
    /// every [`DIAGNOSTICS_INTERVAL`] while they move.
    Diagnostics(Diagnostics),
    /// The loop is over (closed, or after the session ended).
    Finished,
}

/// A running mosh loop.
pub struct MoshLoop {
    pub commands: mpsc::UnboundedSender<MoshCommand>,
    pub task: JoinHandle<()>,
}

/// What a mosh loop starts from.
pub struct MoshStart {
    pub server: SocketAddr,
    pub key: [u8; 16],
    pub config: TransportConfig,
    pub grid: (u16, u16),
    /// Stamped with `clock()` whenever a frame brings bytes for the screen:
    /// the switch veil of mosh + herdr lifts once a repaint is over.
    pub painted: Option<Painted>,
}

/// Where the loop says when the screen last got bytes, on the session's
/// clock.
pub struct Painted {
    pub stamp: Arc<AtomicU64>,
    pub clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

/// Starts the loop on the current runtime. `output` gets the host's bytes,
/// `notes` what the session needs to hear.
pub fn spawn(
    net: Arc<dyn UdpNetwork>,
    start: MoshStart,
    output: Arc<Stream>,
    notes: Box<dyn Fn(MoshNote) + Send + Sync>,
) -> MoshLoop {
    let (commands, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run(net, start, output, rx, notes));
    MoshLoop { commands, task }
}

/// The transport's clock: milliseconds since the loop began, plus a second
/// (the transport's timers never see 0).
#[derive(Clone, Copy)]
struct Clock {
    epoch: Instant,
}

const CLOCK_OFFSET_MS: u64 = 1_000;

impl Clock {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + CLOCK_OFFSET_MS
    }

    fn instant(&self, ms: u64) -> Instant {
        self.epoch + Duration::from_millis(ms.saturating_sub(CLOCK_OFFSET_MS))
    }
}

/// How often the RTT goes up to the UI at most (it moves with every
/// datagram); a roaming change goes at once.
const METRICS_INTERVAL: Duration = Duration::from_secs(1);

/// How often the protocol's counters go up to the UI at most while they
/// move (the Swift transport published on every datagram).
pub const DIAGNOSTICS_INTERVAL: Duration = Duration::from_millis(500);

enum Woke {
    Datagram(usize),
    RecvFailed(io::Error),
    Command(Option<MoshCommand>),
    Timer,
}

async fn recv_on(flow: &Option<Box<dyn UdpFlow>>, buf: &mut [u8]) -> io::Result<usize> {
    match flow {
        Some(flow) => flow.recv(buf).await,
        None => std::future::pending().await,
    }
}

/// An error a UDP socket reports for an ICMP message about an earlier
/// datagram (the server's port closed for a moment, a router's complaint):
/// not this flow's death.
fn is_transient(error: &io::Error) -> bool {
    is_refusal(error) || error.kind() == io::ErrorKind::Interrupted
}

/// The server's port refused an earlier datagram (ICMP port unreachable:
/// nothing listens there; `ConnectionReset` where the system words it so).
fn is_refusal(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset
    )
}

/// Refusals that make the server gone, at least this many…
pub const GONE_REFUSALS: u32 = 3;
/// …spanning at least this long, with nothing heard from the server since
/// the first.
pub const GONE_AFTER_MS: u64 = 3_000;

/// The server's port refusing, since the server was last heard (B402).
/// Counted only after first contact: a port that refuses from the start is
/// a firewall rejecting mosh's UDP, where dialing again would only loop —
/// the return-path banner's offer of SSH is the answer there. A refusal or
/// two is a moment's complaint; refusals for seconds on end, with nothing
/// back, are a server no longer there.
#[derive(Debug, Default)]
pub(crate) struct Refusals {
    first_at: Option<u64>,
    count: u32,
}

impl Refusals {
    /// A refusal at `now`: whether the server is gone.
    pub(crate) fn refused(&mut self, now: u64) -> bool {
        let first = *self.first_at.get_or_insert(now);
        self.count = self.count.saturating_add(1);
        self.count >= GONE_REFUSALS && now.saturating_sub(first) >= GONE_AFTER_MS
    }

    /// A datagram came from the server.
    pub(crate) fn heard(&mut self) {
        *self = Self::default();
    }
}

async fn run(
    net: Arc<dyn UdpNetwork>,
    start: MoshStart,
    output: Arc<Stream>,
    mut commands: mpsc::UnboundedReceiver<MoshCommand>,
    notes: Box<dyn Fn(MoshNote) + Send + Sync>,
) {
    let clock = Clock {
        epoch: Instant::now(),
    };
    let mut transport = match Transport::new(&start.key, start.config) {
        Ok(transport) => transport,
        Err(error) => {
            log::error!("mosh: the session key is unusable: {error:?}");
            notes(MoshNote::Finished);
            return;
        }
    };
    let mut flow: Option<Box<dyn UdpFlow>> = None;
    let (cols, rows) = start.grid;
    transport.start(i32::from(cols), i32::from(rows), clock.now());
    open_flow(&net, start.server, &mut flow, &mut transport, clock).await;

    let mut buf = vec![0u8; 65_536];
    let mut echo = EchoOffsets::default();
    let mut first_contact = false;
    let mut commands_open = true;
    let mut metrics_sent: Option<(Instant, bool)> = None;
    let mut diagnostics_sent: Option<(Instant, Diagnostics)> = None;
    // Barriers waiting for the server's acknowledgement: (the newest user
    // state when asked, who waits).
    let mut barriers: Vec<(u64, tokio::sync::oneshot::Sender<()>)> = Vec::new();
    let mut finished = false;
    let mut copy = CopyMode::default();
    let mut refusals = Refusals::default();
    let mut gone = false;
    // A ReturnPathDead said, and nothing heard since.
    let mut path_reported = false;
    loop {
        while let Some(event) = transport.poll_event() {
            match event {
                MoshEvent::Transmit(datagram) => {
                    if let Some(current) = &flow
                        && let Err(error) = current.send(&datagram).await
                    {
                        if is_refusal(&error) {
                            gone |= first_contact && refusals.refused(clock.now());
                        } else if !is_transient(&error) {
                            log::info!("mosh: send failed ({error}); a fresh flow follows");
                            flow = None;
                            transport.on_channel_state(ChannelState::Failed, clock.now());
                        }
                    }
                }
                MoshEvent::HostFrame(frame) => {
                    if !frame.bytes.is_empty() {
                        output.push(&frame.bytes);
                        if let Some(painted) = &start.painted {
                            painted.stamp.store((painted.clock)(), Ordering::Relaxed);
                        }
                    }
                    if let Some(covered) = echo.acked(frame.echo_ack) {
                        output.mark(Mark {
                            code: crate::tmux_leg::mark::ECHO_ACK,
                            arg: i64::try_from(covered).unwrap_or(i64::MAX),
                        });
                    }
                }
                MoshEvent::RebuildFlow | MoshEvent::RestartFlow => {
                    open_flow(&net, start.server, &mut flow, &mut transport, clock).await;
                }
                MoshEvent::ReturnPathDead => {
                    path_reported = true;
                    notes(MoshNote::ReturnPathDead { first_contact });
                }
                MoshEvent::SessionEnded => notes(MoshNote::SessionEnded),
                MoshEvent::Metrics { srtt_ms, roaming } => {
                    let due = match metrics_sent {
                        None => true,
                        Some((at, was_roaming)) => {
                            was_roaming != roaming || at.elapsed() >= METRICS_INTERVAL
                        }
                    };
                    if due {
                        metrics_sent = Some((Instant::now(), roaming));
                        notes(MoshNote::Metrics { srtt_ms, roaming });
                    }
                }
                MoshEvent::Acked(acked) => {
                    for (_, reply) in barriers.extract_if(.., |(wanted, _)| *wanted <= acked) {
                        let _ = reply.send(());
                    }
                }
                MoshEvent::Finished => finished = true,
            }
        }
        let diagnostics = transport.diagnostics();
        if !first_contact && diagnostics.datagrams_received > 0 {
            first_contact = true;
            notes(MoshNote::FirstContact);
        }
        let due = match &diagnostics_sent {
            None => true,
            Some((at, sent)) => *sent != diagnostics && at.elapsed() >= DIAGNOSTICS_INTERVAL,
        };
        if due {
            diagnostics_sent = Some((Instant::now(), diagnostics.clone()));
            notes(MoshNote::Diagnostics(diagnostics));
        }
        if finished {
            break;
        }
        if gone {
            log::info!("mosh: the server's port keeps refusing; mosh-server is gone");
            notes(MoshNote::ServerGone);
            break;
        }
        let deadline = transport.next_deadline().map(|ms| clock.instant(ms));
        let woke = tokio::select! {
            received = recv_on(&flow, &mut buf) => match received {
                Ok(len) => Woke::Datagram(len),
                Err(error) => Woke::RecvFailed(error),
            },
            command = commands.recv(), if commands_open => Woke::Command(command),
            () = sleep_until(deadline) => Woke::Timer,
        };
        let now = clock.now();
        match woke {
            Woke::Datagram(len) => {
                refusals.heard();
                transport.receive(&buf[..len], now);
                // The transport counts anything that lands as the path
                // alive, even a datagram that doesn't decrypt.
                if std::mem::take(&mut path_reported) {
                    notes(MoshNote::ReturnPathAlive);
                }
            }
            Woke::RecvFailed(error) => {
                if is_refusal(&error) {
                    gone |= first_contact && refusals.refused(now);
                } else if !is_transient(&error) {
                    log::info!("mosh: receive failed ({error}); a fresh flow follows");
                    flow = None;
                    transport.on_channel_state(ChannelState::Failed, now);
                }
            }
            Woke::Command(Some(command)) => match command {
                MoshCommand::Keys { mut bytes, end } => {
                    // Out of the copy-mode a scroll put tmux's client in,
                    // on the same lane, so the keys reach the shell (B362).
                    copy.leave(&mut bytes);
                    match transport.send(&bytes, now) {
                        Some(num) => echo.sent(num, end),
                        // Nothing queued, nothing to echo.
                        None => echo.skip(end),
                    }
                }
                MoshCommand::CopyScroll { lines, prefix } => {
                    // Not the view's bytes: nothing of `end` to echo.
                    if let Some(keys) = copy.page(lines, &prefix, now) {
                        let _ = transport.send(&keys, now);
                    }
                }
                MoshCommand::Mouse(mut bytes) => {
                    // Neither the view's bytes.
                    if !bytes.is_empty() {
                        copy.leave(&mut bytes);
                        let _ = transport.send(&bytes, now);
                    }
                }
                MoshCommand::Resize { cols, rows } => {
                    transport.resize(i32::from(cols), i32::from(rows), now)
                }
                MoshCommand::Resume { force } => transport.resume(force, now),
                MoshCommand::Barrier(reply) => {
                    let counters = transport.diagnostics();
                    if counters.server_acked_user_num >= counters.last_user_num {
                        let _ = reply.send(());
                    } else {
                        barriers.push((counters.last_user_num, reply));
                    }
                }
                MoshCommand::Roam => {
                    open_flow(&net, start.server, &mut flow, &mut transport, clock).await;
                    transport.on_better_path(true, clock.now());
                }
                MoshCommand::Repaint => transport.request_full_redraw(now),
                MoshCommand::Close => transport.close(),
            },
            Woke::Command(None) => {
                commands_open = false;
                transport.close();
            }
            Woke::Timer => transport.on_timer(now),
        }
    }
    notes(MoshNote::Finished);
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// mosh's user state numbers → the input's byte offsets: which state
/// carried the input up to which offset, so an echo ack (a state number)
/// becomes "the input up to here" for the view.
#[derive(Debug, Default)]
pub(crate) struct EchoOffsets {
    /// (state number, offset after its input), oldest first.
    sent: std::collections::VecDeque<(u64, u64)>,
    /// Input that never became a state (nothing to wait for), up to here.
    skipped: u64,
    /// The offset last told.
    told: u64,
}

impl EchoOffsets {
    pub fn sent(&mut self, num: u64, end: u64) {
        self.sent.push_back((num, end));
    }

    pub fn skip(&mut self, end: u64) {
        self.skipped = self.skipped.max(end);
    }

    /// The echo ack `ack` came: the offset it covers, if it moved.
    pub fn acked(&mut self, ack: u64) -> Option<u64> {
        let mut covered = self.told.max(self.skipped);
        while let Some(&(num, end)) = self.sent.front() {
            if num > ack {
                break;
            }
            covered = covered.max(end);
            self.sent.pop_front();
        }
        if covered > self.told {
            self.told = covered;
            Some(covered)
        } else {
            None
        }
    }
}

/// A fresh flow in place of the current one; the transport hears it is
/// ready (it nudges a datagram out) or failed (it tries again in a second).
async fn open_flow(
    net: &Arc<dyn UdpNetwork>,
    server: SocketAddr,
    flow: &mut Option<Box<dyn UdpFlow>>,
    transport: &mut Transport,
    clock: Clock,
) {
    *flow = None;
    match net.open(server).await {
        Ok(fresh) => {
            *flow = Some(fresh);
            transport.on_channel_state(ChannelState::Ready, clock.now());
        }
        Err(error) => {
            log::info!("mosh: no flow to {server} ({error})");
            transport.on_channel_state(ChannelState::Failed, clock.now());
        }
    }
}

/// An in-memory network for tests: flows that reach a [`fake::Server`]
/// end, with switches to drop either direction.
#[doc(hidden)]
pub mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct State {
        next_flow: u64,
        /// Server → client queues, by flow.
        flows: HashMap<u64, mpsc::UnboundedSender<Down>>,
        drop_up: bool,
        drop_down: bool,
        /// Nothing listens on the server's port: each datagram sent comes
        /// back as a refusal on its flow (ICMP port unreachable).
        refuse: bool,
        /// The flow the server last heard from (where its replies go, as a
        /// mosh-server replies to the client's latest address).
        latest: Option<u64>,
        opened: u64,
        sent_up: u64,
    }

    /// What a flow's receive gets.
    enum Down {
        Datagram(Vec<u8>),
        Refused,
    }

    /// The network: opens flows, holds the switches.
    #[derive(Clone)]
    pub struct Net {
        state: Arc<Mutex<State>>,
        up: mpsc::UnboundedSender<Vec<u8>>,
    }

    /// The server's end of the network.
    pub struct Server {
        state: Arc<Mutex<State>>,
        up: mpsc::UnboundedReceiver<Vec<u8>>,
    }

    pub fn network() -> (Net, Server) {
        let state = Arc::new(Mutex::new(State::default()));
        let (up, rx) = mpsc::unbounded_channel();
        (
            Net {
                state: state.clone(),
                up,
            },
            Server { state, up: rx },
        )
    }

    impl Net {
        /// Drop everything the client sends.
        pub fn drop_up(&self, drop: bool) {
            self.state.lock().unwrap().drop_up = drop;
        }

        /// Drop everything the server sends (a dead return path).
        pub fn drop_down(&self, drop: bool) {
            self.state.lock().unwrap().drop_down = drop;
        }

        /// The server's port closes (mosh-server gone): every datagram the
        /// client sends is refused.
        pub fn refuse(&self, refuse: bool) {
            self.state.lock().unwrap().refuse = refuse;
        }

        /// Flows opened so far.
        pub fn opened(&self) -> u64 {
            self.state.lock().unwrap().opened
        }

        /// Datagrams the client sent (dropped or not).
        pub fn sent_up(&self) -> u64 {
            self.state.lock().unwrap().sent_up
        }
    }

    impl UdpNetwork for Net {
        fn open(&self, _server: SocketAddr) -> BoxFuture<'static, io::Result<Box<dyn UdpFlow>>> {
            let (tx, rx) = mpsc::unbounded_channel();
            let id = {
                let mut state = self.state.lock().unwrap();
                state.next_flow += 1;
                state.opened += 1;
                let id = state.next_flow;
                state.flows.insert(id, tx);
                id
            };
            let flow = Flow {
                id,
                state: self.state.clone(),
                up: self.up.clone(),
                down: tokio::sync::Mutex::new(rx),
            };
            Box::pin(async move { Ok(Box::new(flow) as Box<dyn UdpFlow>) })
        }
    }

    struct Flow {
        id: u64,
        state: Arc<Mutex<State>>,
        up: mpsc::UnboundedSender<Vec<u8>>,
        down: tokio::sync::Mutex<mpsc::UnboundedReceiver<Down>>,
    }

    impl Drop for Flow {
        fn drop(&mut self) {
            self.state.lock().unwrap().flows.remove(&self.id);
        }
    }

    impl UdpFlow for Flow {
        fn send<'a>(&'a self, datagram: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
            Box::pin(async move {
                let deliver = {
                    let mut state = self.state.lock().unwrap();
                    state.sent_up += 1;
                    if state.refuse {
                        if let Some(own) = state.flows.get(&self.id) {
                            let _ = own.send(Down::Refused);
                        }
                        false
                    } else if state.drop_up {
                        false
                    } else {
                        state.latest = Some(self.id);
                        true
                    }
                };
                if deliver {
                    let _ = self.up.send(datagram.to_vec());
                }
                Ok(())
            })
        }

        fn recv<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, io::Result<usize>> {
            Box::pin(async move {
                let mut down = self.down.lock().await;
                match down.recv().await {
                    Some(Down::Datagram(datagram)) => {
                        let len = datagram.len().min(buf.len());
                        buf[..len].copy_from_slice(&datagram[..len]);
                        Ok(len)
                    }
                    Some(Down::Refused) => Err(io::ErrorKind::ConnectionRefused.into()),
                    None => std::future::pending().await,
                }
            })
        }
    }

    impl Server {
        /// The next datagram the client sent (and the network let through).
        pub async fn recv(&mut self) -> Option<Vec<u8>> {
            self.up.recv().await
        }

        /// A datagram the client sent, if one is waiting.
        pub fn try_recv(&mut self) -> Option<Vec<u8>> {
            self.up.try_recv().ok()
        }

        /// Sends to the client's latest flow, unless the return path drops it.
        pub fn send(&self, datagram: Vec<u8>) {
            let state = self.state.lock().unwrap();
            if state.drop_down {
                return;
            }
            if let Some(tx) = state.latest.and_then(|id| state.flows.get(&id)) {
                let _ = tx.send(Down::Datagram(datagram));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use moshpit_mosh::Session;
    use moshpit_mosh::harness::{client_instruction, server_datagram};
    use moshpit_mosh::transport::SHUTDOWN_STATE;
    use moshpit_mosh::wire::{
        HostInstruction, UserEvent, decode_user_message, encode_host_message,
    };

    use super::*;

    const KEY: [u8; 16] = *b"0123456789abcdef";

    /// A mosh-server on the fake network that echoes every keystroke: the
    /// user stream applied by state number (a diff counts only from a state
    /// it holds), the host stream diffed from whatever the client acked.
    struct EchoServer {
        session: Session,
        end: fake::Server,
        seq: u64,
        /// The keystrokes of each user state it holds.
        user: BTreeMap<u64, Vec<u8>>,
        newest_user: u64,
        echoed: usize,
        /// Everything it printed, and how much of it each host state holds.
        printed: Vec<u8>,
        host: BTreeMap<u64, usize>,
        host_num: u64,
        client_acked_host: u64,
        echo: bool,
        /// Every resize the client sent, in order.
        resizes: Arc<std::sync::Mutex<Vec<(i32, i32)>>>,
    }

    impl EchoServer {
        fn new(end: fake::Server) -> Self {
            Self {
                session: Session::new(&KEY).unwrap(),
                end,
                seq: 0,
                user: BTreeMap::from([(0, Vec::new())]),
                newest_user: 0,
                echoed: 0,
                printed: Vec::new(),
                host: BTreeMap::from([(0, 0)]),
                host_num: 0,
                client_acked_host: 0,
                echo: true,
                resizes: Arc::default(),
            }
        }

        /// Serves until the network goes quiet for good (the test ends).
        async fn serve(mut self) {
            while let Some(datagram) = self.end.recv().await {
                self.take(&datagram);
            }
        }

        fn take(&mut self, datagram: &[u8]) {
            let instruction = client_instruction(&self.session, datagram);
            self.client_acked_host = self.client_acked_host.max(instruction.ack_num);
            if let Some(base) = self.user.get(&instruction.old_num).cloned()
                && instruction.new_num > self.newest_user
            {
                let mut keys = base;
                for event in decode_user_message(&instruction.diff).unwrap() {
                    match event {
                        UserEvent::Keystroke(bytes) => keys.extend_from_slice(&bytes),
                        UserEvent::Resize { width, height } => {
                            self.resizes.lock().unwrap().push((width, height));
                        }
                    }
                }
                self.user.insert(instruction.new_num, keys.clone());
                self.newest_user = instruction.new_num;
                if self.echo && keys.len() > self.echoed {
                    let fresh = keys[self.echoed..].to_vec();
                    self.echoed = keys.len();
                    self.print(&fresh);
                    return;
                }
            }
            self.reply();
        }

        fn print(&mut self, bytes: &[u8]) {
            self.printed.extend_from_slice(bytes);
            self.host_num += 1;
            self.host.insert(self.host_num, self.printed.len());
            self.reply();
        }

        /// The current host state, diffed from the client's acked one, with
        /// the echo ack (every user state it holds has been echoed).
        fn reply(&mut self) {
            let from = self.host.get(&self.client_acked_host).copied().unwrap_or(0);
            let diff = if self.printed.len() > from {
                encode_host_message(&[
                    HostInstruction::HostBytes(self.printed[from..].to_vec()),
                    HostInstruction::EchoAck(self.newest_user),
                ])
            } else {
                Vec::new()
            };
            let datagram = server_datagram(
                &self.session,
                self.seq,
                self.seq,
                self.client_acked_host,
                self.host_num,
                self.newest_user,
                &diff,
                u16::MAX,
            );
            self.seq += 1;
            self.end.send(datagram);
        }

        /// mosh-server's goodbye: the shutdown state.
        fn shut_down(&mut self) {
            let datagram = server_datagram(
                &self.session,
                self.seq,
                self.seq,
                self.host_num,
                SHUTDOWN_STATE,
                self.newest_user,
                &[],
                u16::MAX,
            );
            self.seq += 1;
            self.end.send(datagram);
        }
    }

    fn server_addr() -> SocketAddr {
        "192.0.2.7:60001".parse().unwrap()
    }

    struct Bench {
        net: fake::Net,
        stream: Arc<Stream>,
        notes: mpsc::UnboundedReceiver<(Duration, MoshNote)>,
        mosh: MoshLoop,
        started: Instant,
    }

    fn start(net: fake::Net) -> Bench {
        let (stream, _input) = moshpit_stream::open();
        let stream = Arc::new(stream);
        let (tx, notes) = mpsc::unbounded_channel();
        let started = Instant::now();
        let mosh = spawn(
            Arc::new(net.clone()),
            MoshStart {
                server: server_addr(),
                key: KEY,
                config: TransportConfig::default(),
                grid: (80, 24),
                painted: None,
            },
            stream.clone(),
            Box::new(move |note| {
                let _ = tx.send((started.elapsed(), note));
            }),
        );
        Bench {
            net,
            stream,
            notes,
            mosh,
            started,
        }
    }

    impl Bench {
        /// Waits (virtual time) for a note matching `want`.
        async fn until(&mut self, want: impl Fn(&MoshNote) -> bool) -> (Duration, MoshNote) {
            loop {
                let (at, note) = self.notes.recv().await.expect("the loop reports");
                if want(&note) {
                    return (at, note);
                }
            }
        }

        fn read(&self) -> Vec<u8> {
            let mut out = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = moshpit_stream::view::read(self.stream.id(), &mut buf);
                out.extend_from_slice(&buf[..n]);
                if n < buf.len() {
                    return out;
                }
            }
        }

        /// Reads the stream until `text` has arrived (virtual time passing).
        async fn read_until(&self, text: &[u8]) -> Vec<u8> {
            let mut seen = Vec::new();
            for _ in 0..2_000 {
                seen.extend(self.read());
                if seen.windows(text.len().max(1)).any(|w| w == text) {
                    return seen;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!(
                "never saw {:?}; got {:?}",
                String::from_utf8_lossy(text),
                String::from_utf8_lossy(&seen)
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn keys_echo_into_the_stream_in_order() {
        let (net, end) = fake::network();
        let bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        let mut end = 0;
        for chunk in [&b"ls"[..], b" -la", b"\r"] {
            end += chunk.len() as u64;
            bench
                .mosh
                .commands
                .send(MoshCommand::Keys {
                    bytes: chunk.to_vec(),
                    end,
                })
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert_eq!(bench.read_until(b"ls -la\r").await, b"ls -la\r");
    }

    #[test]
    fn echo_acks_become_input_offsets() {
        let mut echo = EchoOffsets::default();
        echo.sent(1, 2);
        echo.sent(2, 5);
        echo.sent(4, 9);
        assert_eq!(echo.acked(0), None, "nothing echoed yet");
        assert_eq!(echo.acked(1), Some(2));
        assert_eq!(echo.acked(1), None, "told once");
        assert_eq!(echo.acked(3), Some(5), "state 3 was a resize: up to 2");
        assert_eq!(echo.acked(9), Some(9));
        // Input that never became a state is nobody's to wait for.
        echo.skip(12);
        echo.sent(5, 14);
        assert_eq!(echo.acked(4), Some(12));
        assert_eq!(echo.acked(5), Some(14));
        assert_eq!(echo.acked(2), None, "never backwards");
    }

    /// Reads the stream, marks and all, until `want` holds.
    async fn read_marked_until(
        stream: &Stream,
        want: impl Fn(&[u8], &[(usize, Mark)]) -> bool,
    ) -> (Vec<u8>, Vec<(usize, Mark)>) {
        let (mut bytes, mut marks) = (Vec::new(), Vec::new());
        let mut buf = [0u8; 4096];
        for _ in 0..2_000 {
            loop {
                let (n, mark) = moshpit_stream::view::read_marked(stream.id(), &mut buf);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(mark) = mark {
                    marks.push((bytes.len(), mark));
                    continue;
                }
                if n < buf.len() {
                    break;
                }
            }
            if want(&bytes, &marks) {
                return (bytes, marks);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("never: {:?} {marks:?}", String::from_utf8_lossy(&bytes));
    }

    #[tokio::test(start_paused = true)]
    async fn the_echo_ack_lands_after_the_frame_that_echoed_the_input() {
        let (net, end) = fake::network();
        let bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        let mut end = 0;
        for chunk in [&b"ab"[..], b"cde", b"f"] {
            end += chunk.len() as u64;
            bench
                .mosh
                .commands
                .send(MoshCommand::Keys {
                    bytes: chunk.to_vec(),
                    end,
                })
                .unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        let (bytes, marks) = read_marked_until(&bench.stream, |_, marks| {
            marks
                .iter()
                .any(|(_, m)| m.code == crate::tmux_leg::mark::ECHO_ACK && m.arg == 6)
        })
        .await;
        assert_eq!(bytes, b"abcdef");
        let mut last = 0;
        for (at, mark) in &marks {
            assert_eq!(mark.code, crate::tmux_leg::mark::ECHO_ACK);
            assert!(mark.arg > last, "monotonic: {marks:?}");
            last = mark.arg;
            // Never before the bytes it vouches for: the echo of the first
            // `arg` input bytes is already in the stream.
            assert!(*at as i64 >= mark.arg, "{marks:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_dead_return_path_is_reported_at_eight_seconds_without_contact() {
        let (net, end) = fake::network();
        net.drop_down(true);
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        let (at, note) = bench
            .until(|n| matches!(n, MoshNote::ReturnPathDead { .. } | MoshNote::FirstContact))
            .await;
        assert_eq!(
            note,
            MoshNote::ReturnPathDead {
                first_contact: false
            }
        );
        assert!(
            at >= Duration::from_secs(8) && at < Duration::from_millis(8_100),
            "{at:?}"
        );
        assert!(bench.net.sent_up() >= 3, "it kept sending meanwhile");
    }

    /// B402: refusals make the server gone only three or more of them,
    /// spanning three seconds or more, with nothing heard since the first.
    #[test]
    fn refusals_make_the_server_gone_only_in_number_and_over_time() {
        let mut refusals = Refusals::default();
        assert!(!refusals.refused(0));
        assert!(!refusals.refused(100));
        assert!(!refusals.refused(200), "three, but in a moment");
        assert!(refusals.refused(3_000), "four, over three seconds");
        let mut refusals = Refusals::default();
        assert!(!refusals.refused(0));
        assert!(!refusals.refused(3_500), "two, however far apart");
        refusals.heard();
        assert!(!refusals.refused(4_000), "heard from: counted afresh");
        assert!(!refusals.refused(6_000));
        assert!(!refusals.refused(6_900));
        assert!(refusals.refused(7_000));
    }

    /// B402: mosh-server gone after contact (its port refuses every
    /// datagram): the loop says so within seconds and stops. It used to
    /// count the refusals as a moment's complaint for good, sending into
    /// the closed port until the return-path banner.
    #[tokio::test(start_paused = true)]
    async fn a_server_whose_port_refuses_after_contact_is_gone() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        let gone_at = bench.started.elapsed();
        bench.net.refuse(true);
        let (at, note) = bench
            .until(|n| {
                matches!(
                    n,
                    MoshNote::ServerGone | MoshNote::ReturnPathDead { .. } | MoshNote::Finished
                )
            })
            .await;
        assert_eq!(note, MoshNote::ServerGone);
        let after = at - gone_at;
        assert!(
            after >= Duration::from_millis(GONE_AFTER_MS) && after < Duration::from_secs(12),
            "{after:?}"
        );
        let (_, next) = bench
            .until(|n| !matches!(n, MoshNote::Diagnostics(_) | MoshNote::Metrics { .. }))
            .await;
        assert_eq!(next, MoshNote::Finished, "the loop stopped");
    }

    /// A port that refuses from the start is a firewall rejecting mosh's
    /// UDP, not a server gone: dialing again would only loop. The
    /// return-path report (and its offer of SSH) stands.
    #[tokio::test(start_paused = true)]
    async fn refusals_before_contact_are_not_a_server_gone() {
        let (net, end) = fake::network();
        net.refuse(true);
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        let (_, note) = bench
            .until(|n| matches!(n, MoshNote::ServerGone | MoshNote::ReturnPathDead { .. }))
            .await;
        assert_eq!(
            note,
            MoshNote::ReturnPathDead {
                first_contact: false
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silence_after_contact_rebuilds_the_flow_and_reports_after_the_third() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        let cut = bench.started.elapsed();
        bench.net.drop_down(true);
        let (at, note) = bench
            .until(|n| matches!(n, MoshNote::ReturnPathDead { .. }))
            .await;
        assert_eq!(
            note,
            MoshNote::ReturnPathDead {
                first_contact: true
            }
        );
        // Three rebuilds, a liveness check (every 3 s) after 9 s of silence
        // each: a fresh flow per rebuild.
        assert_eq!(bench.net.opened(), 1 + 3);
        let silent = at - cut;
        assert!(
            silent >= Duration::from_secs(9) && silent <= Duration::from_secs(20),
            "{silent:?}"
        );
        // The path comes back: the next datagram lands and typing works.
        bench.net.drop_down(false);
        bench
            .mosh
            .commands
            .send(MoshCommand::Keys {
                bytes: b"back".to_vec(),
                end: 4,
            })
            .unwrap();
        bench.read_until(b"back").await;
    }

    /// B400: the server heard again after a dead return path is said, once;
    /// the next outage is reported again.
    #[tokio::test(start_paused = true)]
    async fn a_healed_return_path_is_said_and_the_next_outage_reported_again() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        let return_path = |n: &MoshNote| {
            matches!(
                n,
                MoshNote::ReturnPathDead { .. } | MoshNote::ReturnPathAlive
            )
        };
        let within = Duration::from_secs(60);
        for outage in 0..2 {
            bench.net.drop_down(true);
            let (_, note) = tokio::time::timeout(within, bench.until(return_path))
                .await
                .expect("the outage is reported");
            assert_eq!(
                note,
                MoshNote::ReturnPathDead {
                    first_contact: true
                },
                "outage {outage}"
            );
            bench.net.drop_down(false);
            let (_, note) = tokio::time::timeout(within, bench.until(return_path))
                .await
                .expect("the heal is said");
            assert_eq!(note, MoshNote::ReturnPathAlive, "outage {outage}");
        }
        // Heard on and on: said once per outage.
        tokio::time::sleep(Duration::from_secs(5)).await;
        while let Ok((_, note)) = bench.notes.try_recv() {
            assert!(!return_path(&note), "{note:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_servers_goodbye_ends_the_session_after_its_ack() {
        let (net, mut end) = fake::network();
        let mut bench = start(net);
        let first = end.recv().await.unwrap();
        let mut server = EchoServer::new(end);
        server.take(&first);
        bench.until(|n| *n == MoshNote::FirstContact).await;
        server.shut_down();
        // The client acks the shutdown state, then says the session ended.
        let mut acked = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            while let Some(datagram) = server.end.try_recv() {
                let instruction = client_instruction(&server.session, &datagram);
                acked |= instruction.ack_num == SHUTDOWN_STATE;
            }
            if acked {
                break;
            }
        }
        assert!(acked, "the shutdown state is acked so mosh-server can quit");
        bench.until(|n| *n == MoshNote::SessionEnded).await;
        bench.until(|n| *n == MoshNote::Finished).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(bench.mosh.task.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn a_roam_moves_to_a_fresh_flow_and_the_session_carries_on() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        bench
            .mosh
            .commands
            .send(MoshCommand::Keys {
                bytes: b"a".to_vec(),
                end: 1,
            })
            .unwrap();
        bench.read_until(b"a").await;
        bench.mosh.commands.send(MoshCommand::Roam).unwrap();
        bench
            .mosh
            .commands
            .send(MoshCommand::Keys {
                bytes: b"b".to_vec(),
                end: 2,
            })
            .unwrap();
        bench.read_until(b"b").await;
        assert_eq!(bench.net.opened(), 2);
    }

    /// B362: a scroll over a shell on mosh + tmux pages tmux's history in
    /// copy-mode on the client mosh renders: in with the prefix and `[`
    /// once, PageUp; PageDown only while in it; a page at most every 180 ms.
    #[test]
    fn copy_mode_enters_once_goes_down_only_inside_and_keeps_its_pace() {
        let mut copy = CopyMode::default();
        let prefix = [0x02];
        assert_eq!(
            copy.page(-3, &prefix, 0),
            None,
            "nothing below the live screen"
        );
        assert_eq!(copy.page(3, &prefix, 0), Some(b"\x02[\x1b[5~".to_vec()));
        assert_eq!(
            copy.page(3, &prefix, 100),
            None,
            "a page at most every 180 ms"
        );
        assert_eq!(
            copy.page(3, &prefix, 200),
            Some(b"\x1b[5~".to_vec()),
            "in already"
        );
        assert_eq!(copy.page(-3, &prefix, 400), Some(b"\x1b[6~".to_vec()));
        let mut keys = b"ls".to_vec();
        copy.leave(&mut keys);
        assert_eq!(keys, b"qls", "the next keys leave copy-mode first");
        let mut keys = b"ls".to_vec();
        copy.leave(&mut keys);
        assert_eq!(keys, b"ls", "once");
        assert_eq!(
            copy.page(-3, &prefix, 1000),
            None,
            "out: nothing to page down"
        );
        assert_eq!(
            copy.page(3, &[0x01], 1000),
            Some(b"\x01[\x1b[5~".to_vec()),
            "in again, with the server's own prefix"
        );
    }

    /// The scroll's keys go out on the session's own lane, and what is
    /// typed next leaves copy-mode before it reaches the shell.
    #[tokio::test(start_paused = true)]
    async fn a_copy_mode_scroll_goes_out_and_the_next_keys_leave_it_first() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        bench
            .mosh
            .commands
            .send(MoshCommand::CopyScroll {
                lines: 3,
                prefix: vec![0x02],
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        bench
            .mosh
            .commands
            .send(MoshCommand::Keys {
                bytes: b"ls".to_vec(),
                end: 2,
            })
            .unwrap();
        assert_eq!(bench.read_until(b"qls").await, b"\x02[\x1b[5~qls");
    }

    /// A wheel for a mouse app after a tick that entered copy-mode (its
    /// flags not read yet) leaves copy-mode first, on the same lane, so
    /// the program gets it; the next one goes alone.
    #[tokio::test(start_paused = true)]
    async fn a_wheel_after_a_copy_mode_tick_leaves_it_first() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        let wheel = b"\x1b[<64;40;12M".to_vec();
        let commands = &bench.mosh.commands;
        commands
            .send(MoshCommand::CopyScroll {
                lines: 3,
                prefix: vec![0x02],
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        commands.send(MoshCommand::Mouse(wheel.clone())).unwrap();
        commands.send(MoshCommand::Mouse(Vec::new())).unwrap();
        commands.send(MoshCommand::Mouse(wheel.clone())).unwrap();
        let mut want = b"\x02[\x1b[5~q".to_vec();
        want.extend_from_slice(&wheel);
        want.extend_from_slice(&wheel);
        assert_eq!(bench.read_until(&want).await, want);
    }

    /// B338: Repaint asks mosh-server for a complete redraw. The width one
    /// narrower goes alone (the server then sends a whole frame at it), as
    /// `Transport::request_full_redraw` does; nothing like it went out
    /// before.
    #[tokio::test(start_paused = true)]
    async fn a_repaint_asks_the_server_for_a_complete_redraw() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        let server = EchoServer::new(end);
        let resizes = server.resizes.clone();
        tokio::spawn(server.serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let narrow = (79, 24);
        assert!(
            !resizes.lock().unwrap().contains(&narrow),
            "{:?}",
            resizes.lock().unwrap()
        );
        bench.mosh.commands.send(MoshCommand::Repaint).unwrap();
        for _ in 0..400 {
            if resizes.lock().unwrap().contains(&narrow) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            resizes.lock().unwrap().contains(&narrow),
            "no narrowed width reached the server: {:?}",
            resizes.lock().unwrap()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn close_stops_the_loop() {
        let (net, end) = fake::network();
        let mut bench = start(net);
        tokio::spawn(EchoServer::new(end).serve());
        bench.until(|n| *n == MoshNote::FirstContact).await;
        bench.mosh.commands.send(MoshCommand::Close).unwrap();
        bench.until(|n| *n == MoshNote::Finished).await;
        let sent = bench.net.sent_up();
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(bench.net.sent_up(), sent, "nothing after the close");
    }
}
