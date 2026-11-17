//! F1 against real programs: sessions to a private OpenSSH sshd that the
//! client reaches only through a jump host — an in-process russh bastion
//! forwarding a name only it knows ([`BEHIND`]) to the sshd, so a dial
//! that went direct couldn't even resolve it. (The bastion is in-process
//! because one test wants a password as its second factor, which a private
//! OpenSSH run as this user can't check.)
//!
//! Ignored by default (they need sshd). Nothing outside the temp
//! directories is touched; `~/.ssh` and the machine's sshd are left alone.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_f1 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use moshpit_session::{
    Cause, ConnectionSpec, EventKind, FailureKind, Hop, HopBroken, SessionId, SessionState,
    TunnelWhy, Via,
};
use moshpit_ssh::{KnownHosts, Target};
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth as ServerAuth, Msg, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, MethodKind, MethodSet};
use support::*;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The name the target goes by on the bastion's side.
const BEHIND: &str = "node01.invalid";
/// The bastion's second factor, when it asks one.
const SECOND_FACTOR: &str = "correct horse";

macro_rules! need_sshd {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
}

/// A jump host on the loopback: takes the fixture key (then, with
/// `second_factor`, a password), forwards [`BEHIND`]`:22` to `to`, and
/// counts its sign-ins and tunnels.
struct Bastion {
    addr: SocketAddr,
    seen: Arc<Seen>,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct Seen {
    signed_in: AtomicUsize,
    tunnels: AtomicUsize,
}

impl Drop for Bastion {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Bastion {
    async fn start(to: SocketAddr, second_factor: bool) -> Bastion {
        let config = Arc::new(server::Config {
            keys: vec![PrivateKey::from_openssh(fixture("ed25519")).unwrap()],
            methods: MethodSet::from(&[MethodKind::PublicKey, MethodKind::Password][..]),
            auth_rejection_time: Duration::from_millis(5),
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Seen::default());
        let mut server = BastionServer {
            to,
            second_factor,
            seen: seen.clone(),
        };
        let task = tokio::spawn(async move {
            let _ = server.run_on_socket(config, &listener).await;
        });
        Bastion { addr, seen, task }
    }

    fn signed_in(&self) -> usize {
        self.seen.signed_in.load(Ordering::SeqCst)
    }

    fn tunnels(&self) -> usize {
        self.seen.tunnels.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct BastionServer {
    to: SocketAddr,
    second_factor: bool,
    seen: Arc<Seen>,
}

impl server::Server for BastionServer {
    type Handler = BastionConnection;

    fn new_client(&mut self, _: Option<SocketAddr>) -> BastionConnection {
        BastionConnection {
            server: self.clone(),
            key_taken: false,
        }
    }
}

struct BastionConnection {
    server: BastionServer,
    key_taken: bool,
}

impl server::Handler for BastionConnection {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<ServerAuth, Self::Error> {
        let fixture = PublicKey::from_openssh(fixture("ed25519.pub").trim()).unwrap();
        if key.public_key.key_data() != fixture.key_data() {
            return Ok(ServerAuth::reject());
        }
        if self.server.second_factor {
            self.key_taken = true;
            return Ok(ServerAuth::Reject {
                proceed_with_methods: Some(MethodSet::from(&[MethodKind::Password][..])),
                partial_success: true,
            });
        }
        self.server.seen.signed_in.fetch_add(1, Ordering::SeqCst);
        Ok(ServerAuth::Accept)
    }

    async fn auth_password(
        &mut self,
        _user: &str,
        password: &str,
    ) -> Result<ServerAuth, Self::Error> {
        if self.key_taken && password == SECOND_FACTOR {
            self.server.seen.signed_in.fetch_add(1, Ordering::SeqCst);
            return Ok(ServerAuth::Accept);
        }
        Ok(ServerAuth::reject())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let reached = if (host_to_connect, port_to_connect) == (BEHIND, 22) {
            TcpStream::connect(self.server.to).await.ok()
        } else {
            None
        };
        let Some(mut tcp) = reached else {
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        self.server.seen.tunnels.fetch_add(1, Ordering::SeqCst);
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        });
        Ok(())
    }
}

/// A shell session to [`BEHIND`] through the bastion at `bastion`.
fn spec_through(bastion: SocketAddr) -> ConnectionSpec {
    let mut spec = ConnectionSpec::ssh(unique("hpc"), Target::new(whoami(), BEHIND, 22));
    spec.pty_command = Some("/bin/sh".to_owned());
    spec.initial_grid = (100, 30);
    spec.hop = Hop::Through(Box::new(Via {
        connection_id: unique("bastion"),
        name: "bastion".to_owned(),
        target: Target::new(whoami(), "127.0.0.1", bastion.port()),
        proxy: None,
    }));
    spec
}

/// Both hosts' keys trusted: the bastion's at `bastion`, the target's
/// under the name the bastion knows it by.
fn trusting_both(bastion: SocketAddr) -> Arc<KnownHosts> {
    Arc::new(KnownHosts::in_memory(BTreeMap::from([
        (
            format!("127.0.0.1:{}", bastion.port()),
            fingerprint("ed25519"),
        ),
        (format!("{BEHIND}:22"), fingerprint("ed25519")),
    ])))
}

fn bench_trusting(known: Arc<KnownHosts>) -> Bench {
    let mut services = moshpit_session::Services::new(Arc::new(Keys), known);
    let udp = ProxiedUdp::new();
    services.udp = udp.clone();
    let mut bench = Bench::new(services, quick());
    bench.udp = udp;
    bench
}

/// Waits until `id` is back on a transport after one dropped.
async fn back(bench: &mut Bench, id: SessionId) {
    bench
        .state(id, 20, |s| *s == SessionState::Reconnecting)
        .await;
    bench.state(id, 30, |s| *s == SessionState::Connected).await;
}

/// A session through the jump host: its terminal, a borrowed SFTP read,
/// and a redial after the host behind restarted (the bastion's link fine)
/// sign in to the bastion once — the redial goes through the connection
/// the session holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn f1_a_session_signs_in_to_its_jump_host_once() {
    need_sshd!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start_configured(&[], &["Subsystem sftp internal-sftp"]).await;
    let behind = Relay::start(sshd.addr()).await;
    let bastion = Bastion::start(SocketAddr::from(([127, 0, 0, 1], behind.port)), false).await;
    let mut bench = bench_trusting(trusting_both(bastion.addr));
    let id = bench.sessions.connect(spec_through(bastion.addr));
    bench.connected(id).await;

    let lent = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("a connection");
    let sftp = lent.sftp().await.unwrap();
    assert!(!sftp.list("/").await.unwrap().is_empty());
    drop(sftp);
    drop(lent);

    behind.cut();
    tokio::time::sleep(Duration::from_millis(300)).await;
    behind.heal();
    back(&mut bench, id).await;

    assert_eq!(bastion.signed_in(), 1, "one sign-in on the jump host");
    assert_eq!(
        bastion.tunnels(),
        2,
        "the terminal's tunnel, then the redial's"
    );
    assert_eq!(sshd.accepted(), 2);
    bench.sessions.disconnect(id).await.unwrap();
}

/// The bastion's link dies: the session redials both legs, and comes back
/// on a new jump host connection (a second sign-in there).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn f1_the_jump_host_going_redials_both_legs() {
    need_sshd!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let bastion = Bastion::start(sshd.addr(), false).await;
    let front = Relay::start(bastion.addr).await;
    let mut bench = bench_trusting(trusting_both(SocketAddr::from((
        [127, 0, 0, 1],
        front.port,
    ))));
    let id = bench
        .sessions
        .connect(spec_through(SocketAddr::from(([127, 0, 0, 1], front.port))));
    bench.connected(id).await;
    front.cut();
    tokio::time::sleep(Duration::from_millis(300)).await;
    front.heal();
    back(&mut bench, id).await;
    assert_eq!(bastion.signed_in(), 2);
    assert_eq!(bastion.tunnels(), 2);
    bench.sessions.disconnect(id).await.unwrap();
}

/// The bastion asks a password after the key: asked of the person once,
/// naming the bastion; the target's own redial after it restarted asks
/// nothing again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn f1_the_jump_hosts_second_factor_is_asked_once() {
    need_sshd!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let behind = Relay::start(sshd.addr()).await;
    let bastion = Bastion::start(SocketAddr::from(([127, 0, 0, 1], behind.port)), true).await;
    let mut bench = bench_trusting(trusting_both(bastion.addr));
    let id = bench.sessions.connect(spec_through(bastion.addr));
    let (prompt, host) = bench
        .until(id, 20, |kind| match kind {
            EventKind::SignInQuestion { prompt, host, .. } => Some((*prompt, host.clone())),
            _ => None,
        })
        .await;
    assert_eq!(host, "127.0.0.1", "the bastion asks, by its name");
    bench
        .sessions
        .answer_question(id, prompt, Some(SECOND_FACTOR.to_owned()));
    bench.connected(id).await;

    behind.cut();
    tokio::time::sleep(Duration::from_millis(300)).await;
    behind.heal();
    back(&mut bench, id).await;
    let asked = bench
        .seen
        .iter()
        .filter(|e| e.session == id && matches!(e.kind, EventKind::SignInQuestion { .. }))
        .count();
    assert_eq!(asked, 1, "asked once");
    assert_eq!(bastion.signed_in(), 1);
    bench.sessions.disconnect(id).await.unwrap();
}

/// Neither host trusted yet: the bastion's key is asked first, then the
/// target's under the name the bastion knows it by; each kept as its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn f1_both_host_keys_are_asked_in_order() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let bastion = Bastion::start(sshd.addr(), false).await;
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let mut bench = bench_trusting(known.clone());
    let id = bench.sessions.connect(spec_through(bastion.addr));
    let mut asked = Vec::new();
    for _ in 0..2 {
        let (prompt, host, port) = bench
            .until(id, 20, |kind| match kind {
                EventKind::HostKeyPrompt {
                    prompt, host, port, ..
                } => Some((*prompt, host.clone(), *port)),
                _ => None,
            })
            .await;
        asked.push((host, port));
        bench.sessions.answer_host_key(id, prompt, true);
    }
    bench.connected(id).await;
    assert_eq!(
        asked,
        vec![
            ("127.0.0.1".to_owned(), bastion.addr.port()),
            (BEHIND.to_owned(), 22)
        ]
    );
    let keys: Vec<String> = known.snapshot().into_keys().collect();
    assert_eq!(
        keys,
        vec![
            format!("127.0.0.1:{}", bastion.addr.port()),
            format!("{BEHIND}:22")
        ]
    );
    bench.sessions.disconnect(id).await.unwrap();
}

/// A jump host that was deleted (or would be a second hop): the session
/// fails at once, says so, isn't retried — and never dials the host
/// directly instead, though it answers right there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn f1_a_deleted_jump_host_never_falls_back_to_a_direct_dial() {
    need_sshd!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let front = Relay::start(sshd.addr()).await;
    for why in [HopBroken::Deleted, HopBroken::Chained] {
        let mut bench = Bench::with(SocketAddr::from(([127, 0, 0, 1], front.port)), quick());
        // The host itself, reachable directly: the address a fallback
        // would take.
        let mut spec = spec(front.port);
        spec.hop = Hop::Broken(why);
        let id = bench.sessions.connect(spec);
        let failed = bench
            .state(id, 10, |s| matches!(s, SessionState::Failed(_)))
            .await;
        let SessionState::Failed(failure) = failed else {
            unreachable!()
        };
        assert_eq!(failure.kind, FailureKind::Route);
        assert_eq!(failure.cause, Cause::Route(why));
        assert!(!failure.retryable);
        // The network changing, the app coming back: nothing dials.
        bench.sessions.network_changed();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(front.connections(), 0, "never dialled direct");
        bench.sessions.disconnect(id).await.unwrap();
    }
    assert_eq!(sshd.accepted(), 0);
}

/// Failures on the jump host's leg say so: its key refused there is the
/// bastion's credentials, and a host it can't reach is its answer — not
/// the device's network.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn f1_a_failure_on_the_jump_hosts_leg_names_it() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let bastion = Bastion::start(sshd.addr(), false).await;
    // The bastion can't reach this name.
    let mut bench = bench_trusting(trusting_both(bastion.addr));
    let mut spec = spec_through(bastion.addr);
    spec.target = Target::new(whoami(), "elsewhere.invalid", 22);
    let id = bench.sessions.connect(spec);
    let failed = bench
        .state(id, 20, |s| matches!(s, SessionState::Failed(_)))
        .await;
    let SessionState::Failed(failure) = failed else {
        unreachable!()
    };
    assert_eq!(failure.leg.as_deref(), Some("bastion"));
    assert_eq!(failure.kind, FailureKind::Unreachable);
    assert_eq!(
        failure.cause,
        Cause::Tunnel {
            host: "elsewhere.invalid".to_owned(),
            port: 22,
            why: TunnelWhy::ConnectFailed
        }
    );
    bench.sessions.disconnect(id).await.unwrap();
}
