//! The bench for the live tests: a private sshd on the loopback (its own
//! host key, `authorized_keys` and pid file in a temp directory; the
//! machine's sshd and `~/.ssh` are never touched), a TCP relay that can cut
//! or stall the connections through it, a UDP proxy in front of
//! mosh-server that drops either direction, a private tmux server
//! (`tmux -L moshpit-w10-<pid>-<n> -f /dev/null`, never the default one),
//! and a terminal view on a byte channel.
//!
//! Everything started here is stopped when its value drops: sshd's whole
//! process group, the tmux server, and (by pid, `Bench::drop`) every
//! mosh-server a session reported.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moshpit_session::mosh_udp::BoxFuture;
use moshpit_session::{
    ConnectionSpec, CredentialError, Credentials, Event, EventKind, Events, PaneInfo, Services,
    SessionId, SessionState, Sessions, StreamId, SystemUdp, Timing, UdpFlow, UdpNetwork,
};
use moshpit_ssh::{Auth, KnownHosts, Target};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

pub const SSHD: &str = "/usr/sbin/sshd";

/// A fixture key of the ssh crate's tests.
pub fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ssh/tests/fixtures/keys")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `ssh-keygen -lf`'s fingerprint of a fixture key.
pub fn fingerprint(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ssh/tests/fixtures/fingerprints.txt");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let fields: Vec<&str> = l.split_whitespace().collect();
            (fields[0] == name).then(|| fields[2].to_owned())
        })
        .unwrap()
}

pub fn whoami() -> String {
    let out = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub fn unique(label: &str) -> String {
    format!(
        "{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

/// A directory that goes away with the value.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("moshpit-w10-{}", unique(label)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    pub fn join(&self, rest: &str) -> PathBuf {
        self.0.join(rest)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// sshd

/// An sshd of our own, in its own process group, killed (with every
/// connection it forked) when dropped.
pub struct Sshd {
    pub port: u16,
    dir: Scratch,
    child: Child,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let group = format!("-{}", self.child.id());
        for signal in ["-TERM", "-KILL"] {
            let _ = Command::new("kill").args([signal, "--", &group]).output();
        }
        let _ = self.child.wait();
    }
}

impl Sshd {
    pub fn available() -> bool {
        Path::new(SSHD).exists()
    }

    /// Starts sshd with the fixture Ed25519 host key and the fixture
    /// Ed25519 client key authorized. Every session it starts gets
    /// `MOSH_SERVER_NETWORK_TMOUT=300`: a mosh-server the tests failed to
    /// stop still goes away on its own.
    pub async fn start() -> Sshd {
        Self::start_with(&[]).await
    }

    /// The same, with more of the sessions' environment set (`SetEnv`,
    /// which sshd applies after the account's own, `HOME` included).
    pub async fn start_with(env: &[(&str, &str)]) -> Sshd {
        Self::start_configured(env, &[]).await
    }

    /// The same, with `lines` added to the config (`Subsystem sftp
    /// internal-sftp -d <dir>`: SFTP starting in a scratch directory instead
    /// of the account's real home).
    pub async fn start_configured(env: &[(&str, &str)], lines: &[&str]) -> Sshd {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        let dir = Scratch::new("sshd");
        let host_key = dir.join("host_ed25519");
        std::fs::write(&host_key, fixture("ed25519")).unwrap();
        std::fs::set_permissions(&host_key, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(dir.join("authorized_keys"), fixture("ed25519.pub")).unwrap();
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        let config = format!(
            "HostKey {host_key}\nPort {port}\nListenAddress 127.0.0.1\nPidFile {pid}\n\
             AuthorizedKeysFile {keys}\nStrictModes no\nUsePAM no\nUseDNS no\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\n\
             AcceptEnv LANG LC_*\nSetEnv MOSH_SERVER_NETWORK_TMOUT=300{env}\nLogLevel {level}\n{lines}",
            host_key = host_key.display(),
            // `LIVE_SSHD_LOGLEVEL=DEBUG3` for a run that hunts a channel
            // the host refused (sshd keeps the first value it reads).
            level = std::env::var("LIVE_SSHD_LOGLEVEL").unwrap_or_else(|_| "VERBOSE".to_owned()),
            pid = dir.join("sshd.pid").display(),
            keys = dir.join("authorized_keys").display(),
            env = env
                .iter()
                .map(|(name, value)| format!(" {name}={value}"))
                .collect::<String>(),
            lines = lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        );
        std::fs::write(dir.join("sshd_config"), config).unwrap();
        let child = Command::new(SSHD)
            .args(["-D", "-f"])
            .arg(dir.join("sshd_config"))
            .arg("-E")
            .arg(dir.join("sshd.log"))
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let sshd = Sshd { port, dir, child };
        for _ in 0..200 {
            if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return sshd;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("sshd didn't start: {}", sshd.log());
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("sshd.log")).unwrap_or_default()
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// The listener's pid (every connection is a child of it).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sign-ins sshd accepted so far.
    pub fn accepted(&self) -> usize {
        self.log().matches("Accepted publickey").count()
    }
}

// ---------------------------------------------------------------------------
// The TCP relay

/// The relay's line: `stalled` while a stall is on; `epoch` counts stalls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    stalled: bool,
    epoch: u64,
}

/// A TCP relay in front of sshd. `cut` closes the connections through it
/// (new ones work). `stall` freezes every connection there is and every one
/// accepted until `heal` — for good, as a link that died without a word
/// stays dead: after the heal only new connections move.
pub struct Relay {
    pub port: u16,
    line: watch::Sender<Line>,
    cuts: watch::Sender<u64>,
    pub accepted: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Relay {
    pub async fn start(to: SocketAddr) -> Relay {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (line, _) = watch::channel(Line {
            stalled: false,
            epoch: 0,
        });
        let (cuts, _) = watch::channel(0u64);
        let accepted = Arc::new(AtomicUsize::new(0));
        let (lines, cut_rx, count) = (line.clone(), cuts.clone(), accepted.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let mut line = lines.subscribe();
                let at_accept = *line.borrow_and_update();
                let mut cuts = cut_rx.subscribe();
                cuts.borrow_and_update();
                tokio::spawn(async move {
                    let Ok(server) = TcpStream::connect(to).await else {
                        return;
                    };
                    let _ = client.set_nodelay(true);
                    let _ = server.set_nodelay(true);
                    let (cr, cw) = client.into_split();
                    let (sr, sw) = server.into_split();
                    let frozen = Frozen {
                        born_stalled: at_accept.stalled,
                        epoch: at_accept.epoch,
                    };
                    let up = tokio::spawn(pipe(cr, sw, line.clone(), frozen));
                    let down = tokio::spawn(pipe(sr, cw, line, frozen));
                    let _ = cuts.changed().await;
                    up.abort();
                    down.abort();
                });
            }
        });
        Relay {
            port,
            line,
            cuts,
            accepted,
            task,
        }
    }

    /// Closes every connection through the relay.
    pub fn cut(&self) {
        self.cuts.send_modify(|n| *n += 1);
    }

    pub fn stall(&self) {
        self.line.send_modify(|line| {
            line.stalled = true;
            line.epoch += 1;
        });
    }

    pub fn heal(&self) {
        self.line.send_modify(|line| line.stalled = false);
    }

    pub fn connections(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

/// When a connection stops moving for good.
#[derive(Clone, Copy)]
struct Frozen {
    born_stalled: bool,
    epoch: u64,
}

impl Frozen {
    fn now(&self, line: &Line) -> bool {
        self.born_stalled || line.epoch != self.epoch
    }
}

async fn pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    mut line: watch::Receiver<Line>,
    frozen: Frozen,
) {
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        if frozen.now(&line.borrow_and_update()) {
            // Dead without a word: hold everything until the cut.
            std::future::pending::<()>().await;
        }
        tokio::select! {
            read = from.read(&mut buf) => match read {
                Ok(0) | Err(_) => {
                    let _ = to.shutdown().await;
                    return;
                }
                Ok(n) => {
                    if frozen.now(&line.borrow()) {
                        std::future::pending::<()>().await;
                    }
                    if to.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            },
            changed = line.changed() => if changed.is_err() { return },
        }
    }
}

// ---------------------------------------------------------------------------
// UDP between the session and mosh-server

/// Switches on the datagrams between the sessions and their mosh-servers.
#[derive(Default)]
pub struct UdpFaults {
    pub drop_up: AtomicBool,
    pub drop_down: AtomicBool,
    pub up: AtomicU64,
    pub down: AtomicU64,
    /// One-way delay on every datagram, both directions (0: none).
    pub delay_ms: AtomicU64,
}

impl UdpFaults {
    pub fn blackhole(&self, on: bool) {
        self.drop_up.store(on, Ordering::SeqCst);
        self.drop_down.store(on, Ordering::SeqCst);
    }
}

/// The system's UDP, through a proxy per server that applies the faults.
pub struct ProxiedUdp {
    pub faults: Arc<UdpFaults>,
    proxies: Mutex<HashMap<SocketAddr, (SocketAddr, JoinHandle<()>)>>,
}

impl Drop for ProxiedUdp {
    fn drop(&mut self) {
        for (_, (_, task)) in self.proxies.lock().unwrap().drain() {
            task.abort();
        }
    }
}

impl ProxiedUdp {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            faults: Arc::new(UdpFaults::default()),
            proxies: Mutex::new(HashMap::new()),
        })
    }

    fn proxy_for(&self, server: SocketAddr) -> SocketAddr {
        let mut proxies = self.proxies.lock().unwrap();
        if let Some((front, _)) = proxies.get(&server) {
            return *front;
        }
        let front = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        front.set_nonblocking(true).unwrap();
        let back = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        back.connect(server).unwrap();
        back.set_nonblocking(true).unwrap();
        let addr = front.local_addr().unwrap();
        let faults = self.faults.clone();
        let task = tokio::spawn(async move {
            let front = Arc::new(UdpSocket::from_std(front).unwrap());
            let back = Arc::new(UdpSocket::from_std(back).unwrap());
            let mut client: Option<SocketAddr> = None;
            let (mut a, mut b) = (vec![0u8; 65536], vec![0u8; 65536]);
            loop {
                let delay = Duration::from_millis(faults.delay_ms.load(Ordering::SeqCst));
                tokio::select! {
                    got = front.recv_from(&mut a) => if let Ok((n, from)) = got {
                        faults.up.fetch_add(1, Ordering::SeqCst);
                        client = Some(from);
                        if !faults.drop_up.load(Ordering::SeqCst) {
                            if delay.is_zero() {
                                let _ = back.send(&a[..n]).await;
                            } else {
                                let (back, datagram) = (back.clone(), a[..n].to_vec());
                                tokio::spawn(async move {
                                    tokio::time::sleep(delay).await;
                                    let _ = back.send(&datagram).await;
                                });
                            }
                        }
                    },
                    got = back.recv(&mut b) => if let Ok(n) = got {
                        faults.down.fetch_add(1, Ordering::SeqCst);
                        if let Some(to) = client
                            && !faults.drop_down.load(Ordering::SeqCst)
                        {
                            if delay.is_zero() {
                                let _ = front.send_to(&b[..n], to).await;
                            } else {
                                let (front, datagram) = (front.clone(), b[..n].to_vec());
                                tokio::spawn(async move {
                                    tokio::time::sleep(delay).await;
                                    let _ = front.send_to(&datagram, to).await;
                                });
                            }
                        }
                    },
                }
            }
        });
        proxies.insert(server, (addr, task));
        addr
    }
}

impl UdpNetwork for ProxiedUdp {
    fn open(&self, server: SocketAddr) -> BoxFuture<'static, std::io::Result<Box<dyn UdpFlow>>> {
        let proxy = self.proxy_for(server);
        SystemUdp.open(proxy)
    }
}

// ---------------------------------------------------------------------------
// tmux

/// A private tmux server, killed when dropped.
pub struct Tmux {
    pub socket: String,
    pub binary: String,
}

impl Drop for Tmux {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        // tmux leaves its socket file behind.
        let _ = std::fs::remove_file(self.socket_path());
    }
}

impl Tmux {
    pub fn new() -> Tmux {
        Tmux::named("moshpit-w10")
    }

    /// A private server whose socket (`-L`) starts with `label`.
    pub fn named(label: &str) -> Tmux {
        let binary = [
            "/opt/homebrew/bin/tmux",
            "/usr/local/bin/tmux",
            "/usr/bin/tmux",
        ]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .expect("tmux")
        .to_owned();
        let tmux = Tmux {
            socket: unique(label),
            binary,
        };
        let _ = tmux.command().arg("kill-server").output();
        tmux
    }

    /// Where `-L` puts the socket (`TMUX_TMPDIR` is unset for every tmux
    /// started here).
    pub fn socket_path(&self) -> PathBuf {
        let uid = Command::new("id").arg("-u").output().unwrap();
        let uid = String::from_utf8_lossy(&uid.stdout).trim().to_owned();
        PathBuf::from(format!("/tmp/tmux-{uid}/{}", self.socket))
    }

    /// The command prefix a session's spec gets (`tmux_path`).
    pub fn prefix(&self) -> String {
        format!("{} -L {} -f /dev/null", self.binary, self.socket)
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR")
            .env("SHELL", "/bin/sh")
            .env("ENV", "/dev/null")
            .env("PS1", "$ ")
            .env("LANG", "en_US.UTF-8");
        command
    }

    pub fn run(&self, args: &[&str]) -> String {
        let out = self.command().args(args).output().expect("tmux runs");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    }

    /// A detached session running `/bin/sh`.
    pub fn new_session(&self, name: &str) {
        let out = self
            .command()
            .args([
                "new-session",
                "-d",
                "-s",
                name,
                "-x",
                "80",
                "-y",
                "24",
                "/bin/sh",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// `list-sessions` names, or empty when there's no server.
    pub fn sessions(&self) -> Vec<String> {
        self.run(&["list-sessions", "-F", "#{session_name}"])
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// The sessions

/// Signs in with the fixture Ed25519 key.
pub struct Keys;

impl Credentials for Keys {
    fn auth<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Auth, CredentialError>> {
        Box::pin(async { Ok(Auth::key(fixture("ed25519"))) })
    }
}

/// Signs in as [`Keys`] does, but every read from the `hold_from`-th on
/// (0-based) waits for `gate`: a redial held mid-dial, after its start.
pub struct GatedKeys {
    reads: std::sync::atomic::AtomicUsize,
    hold_from: usize,
    gate: Arc<tokio::sync::Notify>,
}

impl GatedKeys {
    pub fn new(hold_from: usize, gate: Arc<tokio::sync::Notify>) -> Self {
        Self {
            reads: std::sync::atomic::AtomicUsize::new(0),
            hold_from,
            gate,
        }
    }
}

impl Credentials for GatedKeys {
    fn auth<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Auth, CredentialError>> {
        Box::pin(async move {
            let read = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if read >= self.hold_from {
                self.gate.notified().await;
            }
            Ok(Auth::key(fixture("ed25519")))
        })
    }
}

/// The fixture host key, trusted for `host:port`.
pub fn trusting(addr: SocketAddr) -> Arc<KnownHosts> {
    Arc::new(KnownHosts::in_memory(BTreeMap::from([(
        format!("{}:{}", addr.ip(), addr.port()),
        fingerprint("ed25519"),
    )])))
}

/// Short timings so a fault shows within seconds.
pub fn quick() -> Timing {
    Timing {
        dial_timeout: Duration::from_secs(5),
        handshake_timeout: Duration::from_secs(8),
        channel_timeout: Duration::from_secs(8),
        bootstrap_timeout: Duration::from_secs(15),
        keepalive_interval: Duration::from_millis(1_500),
        inbound_proof: Duration::from_millis(1_400),
        probe_timeout: Duration::from_millis(1_000),
        foreground_probe_timeout: Duration::from_millis(1_000),
        force_reconnect_after: Duration::from_secs(2),
        attach_stall: Duration::from_secs(4),
        ..Timing::default()
    }
}

/// A spec for the bench's sshd (through `port`), running `/bin/sh`.
pub fn spec(port: u16) -> ConnectionSpec {
    let mut spec = ConnectionSpec::ssh(unique("conn"), Target::new(whoami(), "127.0.0.1", port));
    spec.pty_command = Some("/bin/sh".to_owned());
    spec.initial_grid = (100, 30);
    spec
}

/// A registry on the test's runtime, its events, and the mosh-servers to
/// stop afterwards.
pub struct Bench {
    pub sessions: Sessions,
    pub events: Events,
    pub seen: Vec<Event>,
    pub udp: Arc<ProxiedUdp>,
}

impl Bench {
    pub fn new(services: Services, timing: Timing) -> Bench {
        let (sessions, events) = Sessions::new(tokio::runtime::Handle::current(), services, timing);
        Bench {
            sessions,
            events,
            seen: Vec::new(),
            udp: ProxiedUdp::new(),
        }
    }

    /// Services for `addr`'s sshd (trusted), through the bench's UDP proxy.
    pub fn services(udp: &Arc<ProxiedUdp>, trusted: SocketAddr) -> Services {
        let mut services = Services::new(Arc::new(Keys), trusting(trusted));
        services.udp = udp.clone();
        services
    }

    pub fn with(addr: SocketAddr, timing: Timing) -> Bench {
        let udp = ProxiedUdp::new();
        let services = Self::services(&udp, addr);
        let mut bench = Bench::new(services, timing);
        bench.udp = udp;
        bench
    }

    /// The next event, within `seconds`.
    pub async fn next(&mut self, seconds: u64) -> Option<Event> {
        let event = tokio::time::timeout(Duration::from_secs(seconds), self.events.recv())
            .await
            .ok()
            .flatten()?;
        self.seen.push(event.clone());
        Some(event)
    }

    /// Waits for an event of `id` that `want` picks; panics after
    /// `seconds`, listing what was seen.
    pub async fn until<T>(
        &mut self,
        id: SessionId,
        seconds: u64,
        want: impl Fn(&EventKind) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, self.events.recv()).await {
                Ok(Some(event)) => {
                    self.seen.push(event.clone());
                    if event.session == id
                        && let Some(found) = want(&event.kind)
                    {
                        return found;
                    }
                }
                _ => panic!("timed out; seen: {:#?}", self.tail()),
            }
        }
    }

    pub async fn state(
        &mut self,
        id: SessionId,
        seconds: u64,
        want: impl Fn(&SessionState) -> bool,
    ) -> SessionState {
        self.until(id, seconds, |kind| match kind {
            EventKind::State(state) if want(state) => Some(state.clone()),
            _ => None,
        })
        .await
    }

    /// Waits until the session is connected (now, or by a later event).
    pub async fn connected(&mut self, id: SessionId) {
        if self
            .sessions
            .view(id)
            .is_some_and(|view| view.state == SessionState::Connected)
        {
            return;
        }
        self.state(id, 30, |s| *s == SessionState::Connected).await;
    }

    /// The panes of an attach newer than `after` (the view's, if it has
    /// them already; else the next `Panes` event).
    pub async fn panes_after(
        &mut self,
        id: SessionId,
        after: u64,
        seconds: u64,
    ) -> (u64, Vec<PaneInfo>) {
        if let Some(view) = self.sessions.view(id)
            && view.attach > after
            && !view.panes.is_empty()
        {
            return (view.attach, view.panes);
        }
        self.until(id, seconds, |kind| match kind {
            EventKind::Panes { attach, panes } if *attach > after && !panes.is_empty() => {
                Some((*attach, panes.clone()))
            }
            _ => None,
        })
        .await
    }

    pub async fn panes(&mut self, id: SessionId, seconds: u64) -> (u64, Vec<PaneInfo>) {
        if let Some(view) = self.sessions.view(id)
            && !view.panes.is_empty()
        {
            return (view.attach, view.panes);
        }
        self.until(id, seconds, |kind| match kind {
            EventKind::Panes { attach, panes } if !panes.is_empty() => {
                Some((*attach, panes.clone()))
            }
            _ => None,
        })
        .await
    }

    /// Events seen so far that aren't metrics or view chatter.
    pub fn tail(&self) -> Vec<&Event> {
        self.seen
            .iter()
            .filter(|e| {
                !matches!(
                    e.kind,
                    EventKind::MoshMetrics { .. } | EventKind::View { .. } | EventKind::TreeChanged
                )
            })
            .collect()
    }

    pub fn states(&self, id: SessionId) -> Vec<SessionState> {
        self.seen
            .iter()
            .filter(|e| e.session == id)
            .filter_map(|e| match &e.kind {
                EventKind::State(state) => Some(state.clone()),
                _ => None,
            })
            .collect()
    }

    /// Drains what is queued without waiting.
    pub fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.seen.push(event);
        }
    }
}

/// Stops a process we started (a mosh-server), if it is still there.
pub fn kill(pid: i64) {
    let _ = Command::new("kill").arg(pid.to_string()).output();
}

pub fn alive(pid: i64) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Stops the mosh-server a session reported when dropped.
pub struct Reaper(pub Option<i64>);

impl Drop for Reaper {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            kill(pid);
        }
    }
}

// ---------------------------------------------------------------------------
// A terminal view on a byte channel

/// The view's end of a pane's byte channel: a waker that rings a `Notify`,
/// and what it read.
pub struct Term {
    pub stream: StreamId,
    rung: Arc<Notify>,
    pub screen: Vec<u8>,
}

impl Term {
    pub fn attach(stream: StreamId) -> Term {
        let rung = Arc::new(Notify::new());
        let bell = rung.clone();
        assert!(moshpit_stream::view::attach(stream, move || bell.notify_one()));
        Term {
            stream,
            rung,
            screen: Vec::new(),
        }
    }

    pub fn write(&self, bytes: &[u8]) -> bool {
        moshpit_stream::view::write(self.stream, bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> bool {
        moshpit_stream::view::resize(self.stream, cols, rows)
    }

    /// Reads what is queued.
    pub fn read(&mut self) -> usize {
        let mut buf = [0u8; 16 * 1024];
        let mut total = 0;
        loop {
            let n = moshpit_stream::view::read(self.stream, &mut buf);
            self.screen.extend_from_slice(&buf[..n]);
            total += n;
            if n < buf.len() {
                return total;
            }
        }
    }

    /// Reads until `needle` shows up after byte `from` of the screen;
    /// `None` after `seconds`.
    pub async fn wait_for_from(
        &mut self,
        from: usize,
        needle: &str,
        seconds: u64,
    ) -> Option<usize> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        loop {
            self.read();
            if let Some(at) = find(
                &self.screen[from.min(self.screen.len())..],
                needle.as_bytes(),
            ) {
                return Some(from + at);
            }
            if tokio::time::timeout_at(deadline, self.rung.notified())
                .await
                .is_err()
            {
                self.read();
                return find(
                    &self.screen[from.min(self.screen.len())..],
                    needle.as_bytes(),
                )
                .map(|at| from + at);
            }
        }
    }

    pub async fn wait_for(&mut self, needle: &str, seconds: u64) -> bool {
        self.wait_for_from(0, needle, seconds).await.is_some()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.screen).into_owned()
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        moshpit_stream::view::detach(self.stream);
    }
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// p50, p95, p99 and max of `samples` (µs).
pub fn percentiles(samples: &mut [u128]) -> (u128, u128, u128, u128) {
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
    (at(0.50), at(0.95), at(0.99), *samples.last().unwrap())
}

// ---------------------------------------------------------------------------
// herdr

pub const HERDR: &str = "/opt/homebrew/bin/herdr";

/// A private herdr server: `herdr --session mw10b-<pid>-<n>` with
/// HOME a fresh directory (a temporary HOME alone is not enough: probed,
/// `HOME=/tmp/x herdr status` finds the user's own default server). Every
/// command here carries the private `--session`; the server is stopped and
/// the directory removed when the value drops. Panes run zsh with a bare
/// `% ` prompt; `claude` is a copy of `/bin/cat`.
pub struct PrivateHerdr {
    pub home: PathBuf,
    pub session: String,
    server: Option<Child>,
}

impl PrivateHerdr {
    pub fn available() -> bool {
        Path::new(HERDR).exists()
    }

    pub fn start() -> PrivateHerdr {
        // Short: the socket's path must fit a sockaddr_un (104 bytes).
        let session = unique("mw10b");
        let home = PathBuf::from(format!("/tmp/w10bh-{session}"));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::copy("/bin/cat", home.join("bin/claude")).unwrap();
        std::fs::write(home.join(".zshrc"), "PROMPT='%% '\nunsetopt BEEP\n").unwrap();
        let home = home.canonicalize().unwrap();
        let mut herdr = PrivateHerdr {
            home,
            session,
            server: None,
        };
        let (status, _) = herdr.run(&["status"]);
        assert!(
            status.contains(&herdr.home.display().to_string()),
            "the session's socket is not private: {status}"
        );
        use std::os::unix::process::CommandExt;
        let server = Command::new(HERDR)
            .args(["--session", &herdr.session, "server"])
            .env_clear()
            .envs(herdr.env())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        herdr.server = Some(server);
        for _ in 0..100 {
            if herdr.run(&["api", "snapshot"]).1 == 0 {
                herdr.run(&["workspace", "create", "--focus", "--label", "live"]);
                return herdr;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the private herdr server never answered");
    }

    /// Stops the server and starts it again (its terminals numbered anew),
    /// with a workspace focused.
    pub fn restart(&mut self) {
        self.run(&["server", "stop"]);
        if let Some(mut server) = self.server.take() {
            for _ in 0..50 {
                if let Ok(Some(_)) = server.try_wait() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let group = format!("-{}", server.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).output();
            let _ = server.wait();
        }
        use std::os::unix::process::CommandExt;
        let server = Command::new(HERDR)
            .args(["--session", &self.session, "server"])
            .env_clear()
            .envs(self.env())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        self.server = Some(server);
        for _ in 0..100 {
            if self.run(&["api", "snapshot"]).1 == 0 {
                if self.panes().is_empty() {
                    self.run(&["workspace", "create", "--focus", "--label", "live"]);
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the private herdr server never came back");
    }

    /// herdr's terminal id of `pane` (the snapshot's keys are sorted: the
    /// first `terminal_id` after the pane's id is its own).
    pub fn terminal(&self, pane: &str) -> Option<String> {
        let (snapshot, _) = self.run(&["api", "snapshot"]);
        let at = snapshot.find(&format!("\"pane_id\":\"{pane}\""))?;
        let rest = &snapshot[at..];
        let key = "\"terminal_id\":\"";
        let start = rest.find(key)? + key.len();
        Some(rest[start..].split('"').next()?.to_owned())
    }

    /// The focused pane, by the snapshot.
    pub fn focused(&self) -> Option<String> {
        let (snapshot, _) = self.run(&["api", "snapshot"]);
        let key = "\"focused_pane_id\":\"";
        let start = snapshot.find(key)? + key.len();
        Some(snapshot[start..].split('"').next()?.to_owned())
    }

    /// The environment every process of this herdr gets.
    pub fn env(&self) -> Vec<(String, String)> {
        let home = self.home.display().to_string();
        vec![
            ("HOME".to_owned(), home.clone()),
            ("ZDOTDIR".to_owned(), home.clone()),
            (
                "PATH".to_owned(),
                format!("{home}/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"),
            ),
            ("SHELL".to_owned(), "/bin/zsh".to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
        ]
    }

    /// What a connection's herdr path is: the binary with the private
    /// session, so every command the client builds carries it.
    pub fn custom_path(&self) -> String {
        format!("{HERDR} --session {}", self.session)
    }

    /// One private herdr command: its merged output and status.
    pub fn run(&self, args: &[&str]) -> (String, i32) {
        let output = Command::new(HERDR)
            .args(["--session", &self.session])
            .args(args)
            .env_clear()
            .envs(self.env())
            .output();
        match output {
            Ok(output) => (
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ),
                output.status.code().unwrap_or(-1),
            ),
            Err(error) => (error.to_string(), -1),
        }
    }

    /// What a pane shows.
    pub fn read(&self, pane: &str) -> String {
        self.run(&["pane", "read", pane, "--format", "text"]).0
    }

    /// The panes, by id, from the snapshot.
    pub fn panes(&self) -> Vec<String> {
        let (snapshot, _) = self.run(&["api", "snapshot"]);
        let mut panes = Vec::new();
        let mut rest = snapshot.as_str();
        while let Some(at) = rest.find("\"pane_id\":\"") {
            rest = &rest[at + 11..];
            if let Some(end) = rest.find('"') {
                let pane = rest[..end].to_owned();
                if !panes.contains(&pane) {
                    panes.push(pane);
                }
                rest = &rest[end..];
            }
        }
        panes
    }
}

impl Drop for PrivateHerdr {
    fn drop(&mut self) {
        self.run(&["server", "stop"]);
        if let Some(mut server) = self.server.take() {
            for _ in 0..50 {
                if let Ok(Some(_)) = server.try_wait() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let group = format!("-{}", server.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).output();
            let _ = server.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
