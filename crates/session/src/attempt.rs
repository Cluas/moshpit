//! One attempt: dial, sign in, decide the route (which multiplexer, what
//! degrades), bring the transport up. It runs as one task per generation
//! and hands what it built to the session, which takes it only if the
//! generation is still current — otherwise it is closed.
//!
//! Nothing here starts reading or writing a pane: the session starts the
//! pane's task once it has accepted the result, so a superseded attempt can
//! never write into a byte channel.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use moshpit_herdr::launch::{self, quote};
use moshpit_install::{HostCapabilities, Multiplexer};
use moshpit_mosh::bootstrap;
use moshpit_ssh::{
    BoxFuture, DenyAll, HostKey, HostKeyPrompt, Process, PtyRequest, SignInQuestions, Ssh,
    SshError, Target, TrustOnFirstUse,
};
use moshpit_tmux::controller::PinLedger;
use moshpit_tmux::format::is_valid_id;

use crate::event::{Cause, CredentialsWhy, Degrade, Failure, FailureKind, MoshWhy, ProxyWhy};
use crate::services::{CredentialError, Services};
use crate::spec::{ConnectionSpec, Protocol, Timing};

/// Where an attempt is headed: the multiplexer it runs and what it lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub multiplexer: Multiplexer,
    pub degrade: Option<Degrade>,
}

/// The degrade matrix (`SessionHub.start`): a multiplexer the host lacks
/// (and no custom path vouches for) becomes a plain shell with a notice —
/// never the other multiplexer, whose sessions are unrelated work. herdr
/// where this build has no client for it (`herdr_client`: mosh without its
/// control connection) is a plain shell too.
pub fn route(spec: &ConnectionSpec, capabilities: &HostCapabilities, herdr_client: bool) -> Route {
    let chosen = spec.multiplexer;
    if !spec.vouches_for(chosen) && !capabilities.has(chosen) {
        let degrade = match chosen {
            Multiplexer::Tmux => Degrade::TmuxMissing,
            Multiplexer::Herdr => Degrade::HerdrMissing,
            Multiplexer::None => unreachable!("a plain shell needs nothing"),
        };
        return Route {
            multiplexer: Multiplexer::None,
            degrade: Some(degrade),
        };
    }
    if chosen == Multiplexer::Herdr && !herdr_client {
        return Route {
            multiplexer: Multiplexer::None,
            degrade: Some(Degrade::HerdrUnavailable),
        };
    }
    Route {
        multiplexer: chosen,
        degrade: None,
    }
}

/// Whether a mosh connection falls back to SSH: no `mosh-server` on the
/// host, and no custom path vouching for one.
pub fn mosh_falls_back(spec: &ConnectionSpec, capabilities: &HostCapabilities) -> bool {
    spec.mosh_server_path.is_none() && !capabilities.has_mosh_server
}

/// What an attempt brought up.
pub(crate) enum Established {
    /// An SSH connection and its terminal channel, not read yet.
    Ssh {
        ssh: Ssh,
        pty: Process,
        multiplexer: Multiplexer,
        degrade: Option<Degrade>,
        /// The host runs python3: herdr's push pump can.
        push: bool,
        /// The older host-key algorithm the connection had to use (B564).
        older_host_key: Option<&'static str>,
    },
    /// mosh-server is up, running tmux's or herdr's own client when one was
    /// asked for.
    Mosh {
        server: SocketAddr,
        key: [u8; 16],
        server_pid: Option<i64>,
        degrade: Option<Degrade>,
        /// The older host-key algorithm the bootstrap had to use (B564).
        older_host_key: Option<&'static str>,
        /// The multiplexer mosh renders; its control plane (tmux's `-CC`
        /// sidecar, herdr's control client) runs beside it on `control`.
        multiplexer: Multiplexer,
        /// The bootstrap connection, kept when a control plane rides it;
        /// closed otherwise.
        control: Option<Ssh>,
        /// herdr over mosh: what mosh-server runs and how it is steered.
        herdr: Option<MoshHerdr>,
        /// tmux over mosh: every window's layout before mosh-server started
        /// the phone's renderer (B270, `note_layouts_before_renderer`).
        tmux_layouts: Option<String>,
        /// Stops the server if no session takes it (B194).
        reaper: ServerReaper,
    },
}

/// herdr over mosh, as the bootstrap started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MoshHerdr {
    /// The raw-attach loop's key (connection id and a per-boot nonce):
    /// mosh-server runs the loop. `None`: the host's herdr has no `terminal
    /// attach`, and mosh-server runs herdr's own TUI.
    pub renderer_key: Option<String>,
    /// The host runs python3: herdr's push pump can.
    pub push: bool,
}

impl Established {
    /// Lets go of what a superseded attempt made: the connection, and a
    /// mosh-server nobody will ever reach (B194: it waited forever, its
    /// shell with it).
    pub async fn close(self) {
        match self {
            Established::Ssh { ssh, .. } => ssh.close().await,
            Established::Mosh {
                control, reaper, ..
            } => {
                reaper.reap_now().await;
                if let Some(control) = control {
                    control.close().await;
                }
            }
        }
    }
}

/// A mosh-server an attempt started, stopped unless a session takes it on
/// ([`ServerReaper::disarm`]): what a superseded, abandoned or interrupted
/// start leaves behind (B194). Dropped armed — the attempt's task aborted
/// mid-way, a result nobody received — it stops the server from a task of
/// its own.
pub(crate) struct ServerReaper {
    pid: Option<i64>,
    on: Option<Box<ReapOn>>,
}

struct ReapOn {
    services: Services,
    spec: ConnectionSpec,
    timing: Timing,
    /// A connection to the host still up, tried first.
    ssh: Option<Ssh>,
}

impl ServerReaper {
    fn new(pid: Option<i64>, services: &Services, spec: &ConnectionSpec, timing: &Timing) -> Self {
        Self {
            pid,
            on: Some(Box::new(ReapOn {
                services: services.clone(),
                spec: spec.clone(),
                timing: timing.clone(),
                ssh: None,
            })),
        }
    }

    /// Reap over this connection while it is up.
    fn over(&mut self, ssh: &Ssh) {
        if let Some(on) = &mut self.on {
            on.ssh = Some(ssh.clone());
        }
    }

    /// The session owns the server now; its pid.
    pub fn disarm(mut self) -> Option<i64> {
        self.on = None;
        self.pid
    }

    /// Stop the server now.
    pub async fn reap_now(mut self) {
        if let (Some(pid), Some(on)) = (self.pid, self.on.take()) {
            reap(on, pid).await;
        }
    }
}

impl Drop for ServerReaper {
    fn drop(&mut self) {
        if let (Some(pid), Some(on)) = (self.pid, self.on.take()) {
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    runtime.spawn(reap(on, pid));
                }
                Err(_) => log::info!("mosh-server {pid} left running: no runtime to stop it"),
            }
        }
    }
}

async fn reap(on: Box<ReapOn>, pid: i64) {
    if let Some(ssh) = on.ssh.as_ref().filter(|ssh| !ssh.is_closed()) {
        let killed = tokio::time::timeout(ON_HOST_BOUND, ssh.run(&reap_command(pid))).await;
        if matches!(killed, Ok(Ok(output)) if output.closed) {
            return;
        }
    }
    let ReapOn {
        services,
        spec,
        timing,
        ..
    } = *on;
    reap_mosh_server(services, spec, timing, vec![pid]).await;
}

/// Stops the mosh-servers `pids` — only while each still is one: a pid
/// reaped late (a reconnect's orphan, B195) may have been recycled by
/// then, and the shell's `kill` must not land on whatever got it (B50). A
/// host without `ps` gets the plain `kill`.
pub(crate) fn reap_command(pid: i64) -> String {
    format!(
        "if command -v ps >/dev/null 2>&1; then case \"$(ps -o comm= -p {pid} 2>/dev/null)\" in *mosh-server*) kill {pid};; esac; else kill {pid}; fi 2>/dev/null; true"
    )
}

/// Everything an attempt needs.
pub(crate) struct Attempt {
    pub spec: ConnectionSpec,
    pub protocol: Protocol,
    pub timing: Timing,
    pub services: Services,
    pub grid: (u16, u16),
    pub prompt: Arc<dyn HostKeyPrompt>,
    /// Who answers the server's sign-in questions (B563).
    pub questions: Option<Arc<dyn SignInQuestions>>,
}

impl Attempt {
    pub async fn run(self) -> Result<Established, Failure> {
        let ssh = dial_asking(
            &self.services,
            &self.spec,
            &self.timing,
            self.prompt.clone(),
            self.questions.clone(),
        )
        .await?;
        let capabilities = self.capabilities(&ssh).await;
        let result = match self.protocol {
            Protocol::Ssh => self.ssh(&ssh, &capabilities, None).await,
            Protocol::Mosh => self.mosh(&ssh, &capabilities).await,
        };
        match result {
            Ok(established) => Ok(established),
            Err(failure) => {
                ssh.close().await;
                Err(failure)
            }
        }
    }

    /// A terminal on the connection, on the route the host allows.
    async fn ssh(
        &self,
        ssh: &Ssh,
        capabilities: &HostCapabilities,
        fell_back: Option<Degrade>,
    ) -> Result<Established, Failure> {
        let mut route = route(&self.spec, capabilities, true);
        if fell_back.is_some() {
            // The mosh notice already points at the install sheet; a second
            // one for a missing multiplexer would only stack.
            route.degrade = fell_back;
        }
        let mut request = PtyRequest::new(u32::from(self.grid.0), u32::from(self.grid.1));
        request.command = match route.multiplexer {
            // herdr's frame channel: its reader loop is the terminal's
            // program, never a line typed into a login shell that may still
            // be starting (the client's `frame_loop`).
            Multiplexer::Herdr => Some(moshpit_herdr::frame::loop_program(
                self.spec.herdr_path.as_deref(),
                &self.spec.connection_id,
            )),
            // tmux's control client is the terminal's program too (B267):
            // typed into the login shell, a shell still starting swallowed
            // it and nothing attached.
            Multiplexer::Tmux => Some(moshpit_tmux::boot::control_attach_program(
                self.spec.tmux_command(),
                &shell_after(&self.spec),
            )),
            Multiplexer::None => self.spec.pty_command.clone(),
        };
        let pty = ssh
            .terminal(&request)
            .await
            .map_err(|e| Failure::from(&e))?;
        Ok(Established::Ssh {
            ssh: ssh.clone(),
            pty,
            multiplexer: route.multiplexer,
            degrade: route.degrade,
            push: capabilities.has_python3,
            older_host_key: ssh.older_host_key_algorithm(),
        })
    }

    /// `mosh-server new` over the connection, then UDP from here — or SSH on
    /// this same connection when the host has no mosh-server. With tmux the
    /// connection stays up: the session runs the `-CC` sidecar on it.
    async fn mosh(
        &self,
        ssh: &Ssh,
        capabilities: &HostCapabilities,
    ) -> Result<Established, Failure> {
        if mosh_falls_back(&self.spec, capabilities) {
            return self
                .ssh(ssh, capabilities, Some(Degrade::MoshServerMissing))
                .await;
        }
        let route = route(&self.spec, capabilities, true);
        let remembered = self
            .services
            .memory
            .tmux_selection(&self.spec.connection_id)
            .map(|selection| selection.session);
        let herdr = if route.multiplexer == Multiplexer::Herdr {
            Some(MoshHerdr {
                renderer_key: self.herdr_renderer(ssh).await,
                push: capabilities.has_python3,
            })
        } else {
            None
        };
        // The windows as they are before the renderer — tmux's client,
        // mosh-server's program — sizes them to the phone (B270).
        let tmux_layouts = if route.multiplexer == Multiplexer::Tmux {
            self.tmux_layouts(ssh).await
        } else {
            None
        };
        let command = mosh_command(
            &self.spec,
            route.multiplexer,
            remembered.as_deref(),
            herdr.as_ref(),
        );
        // The server is ours to stop from the moment it names its pid: an
        // attempt cut short after that — even with the channel still open (a
        // slow link's close) — reaps it on the way out (B194).
        let mut reaper = None;
        let (output, closed) = tokio::time::timeout(
            self.timing.bootstrap_timeout,
            self.bootstrap(ssh, &command, &mut reaper),
        )
        .await
        .map_err(|_| {
            Failure::new(
                FailureKind::MoshBootstrap,
                Cause::MoshBootstrap(MoshWhy::TimedOut {
                    seconds: self.timing.bootstrap_timeout.as_secs(),
                }),
                format!(
                    "mosh-server didn't start within {:?}",
                    self.timing.bootstrap_timeout
                ),
            )
        })?
        .map_err(|e| Failure::from(&e))?;
        let host = &self.spec.target.host;
        let credentials = match bootstrap::parse_bytes(&output, host) {
            Ok(credentials) => credentials,
            // Cut off mid-way: the connection's fault, worth another try.
            Err(_) if !closed => {
                return Err(Failure::lost(
                    "the connection dropped while mosh-server was starting",
                ));
            }
            Err(error) => {
                return Err(Failure::permanent(
                    FailureKind::MoshBootstrap,
                    Cause::MoshBootstrap(MoshWhy::Unreadable),
                    error.to_string(),
                ));
            }
        };
        let reaper = reaper.unwrap_or_else(|| {
            let mut reaper = ServerReaper::new(
                credentials.server_pid,
                &self.services,
                &self.spec,
                &self.timing,
            );
            reaper.over(ssh);
            reaper
        });
        let server = match ssh.peer_addr() {
            Some(peer) => udp_server(peer, credentials.udp_port),
            None => resolve(host, credentials.udp_port, self.timing.dial_timeout).await?,
        };
        let control = if matches!(route.multiplexer, Multiplexer::Tmux | Multiplexer::Herdr) {
            Some(ssh.clone())
        } else {
            // mosh-server has daemonized: the bootstrap connection is done.
            ssh.close().await;
            None
        };
        Ok(Established::Mosh {
            server,
            key: credentials.key,
            server_pid: credentials.server_pid,
            degrade: route.degrade,
            older_host_key: ssh.older_host_key_algorithm(),
            multiplexer: route.multiplexer,
            control,
            herdr,
            tmux_layouts,
            reaper,
        })
    }

    /// Every window's layout (`list-windows -a` in the controller's size
    /// probe format), read on the bootstrap connection before mosh-server
    /// starts the renderer; `None` when the host didn't answer in time.
    async fn tmux_layouts(&self, ssh: &Ssh) -> Option<String> {
        let command = format!(
            "{} list-windows -a -F '{}' 2>/dev/null",
            self.spec.tmux_command(),
            moshpit_tmux::controller::SIZE_PROBE_FORMAT
        );
        let output = tokio::time::timeout(self.timing.channel_timeout, ssh.run(&command))
            .await
            .ok()?
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout()).into_owned())
    }

    /// herdr over mosh, before mosh-server starts: the previous generations'
    /// renderers retired (their targets removed, an attach killed only when
    /// its command line is that renderer's — B220), and whether this herdr
    /// knows `terminal attach` (the raw single-pane loop; otherwise herdr's
    /// own TUI). One exec on the bootstrap connection. The loop's key, or
    /// `None` for the TUI.
    async fn herdr_renderer(&self, ssh: &Ssh) -> Option<String> {
        let custom = self.spec.herdr_path.as_deref();
        let command = format!(
            "{}; {}",
            launch::stale_renderer_cleanup_command(&self.spec.connection_id),
            launch::raw_attach_probe_command(custom)
        );
        let probed = tokio::time::timeout(self.timing.channel_timeout, ssh.run(&command)).await;
        let raw = matches!(&probed, Ok(Ok(output))
            if contains(&output.merged(), b"MOSHPIT_RAW_ATTACH_OK"));
        raw.then(|| launch::mosh_renderer_key(&self.spec.connection_id, &nonce()))
    }

    /// `mosh-server new`, read as it answers: what it printed (merged in
    /// arrival order) and whether the channel closed in order. `reaper` is
    /// armed the moment the output names the server's pid.
    async fn bootstrap(
        &self,
        ssh: &Ssh,
        command: &str,
        reaper: &mut Option<ServerReaper>,
    ) -> Result<(Vec<u8>, bool), SshError> {
        let mut process = ssh.exec(command).await?;
        process.close_stdin().await?;
        let mut output = Vec::new();
        let mut closed = false;
        while let Some(event) = process.next().await {
            match event {
                moshpit_ssh::Event::Stdout(bytes) | moshpit_ssh::Event::Stderr(bytes) => {
                    output.extend_from_slice(&bytes);
                    if reaper.is_none()
                        && let Ok(credentials) =
                            bootstrap::parse_bytes(&output, &self.spec.target.host)
                        && let Some(pid) = credentials.server_pid
                    {
                        let mut armed =
                            ServerReaper::new(Some(pid), &self.services, &self.spec, &self.timing);
                        armed.over(ssh);
                        *reaper = Some(armed);
                    }
                }
                moshpit_ssh::Event::Exited(_) | moshpit_ssh::Event::Eof => {}
                moshpit_ssh::Event::Closed => closed = true,
                moshpit_ssh::Event::Lost => closed = false,
            }
        }
        Ok((output, closed))
    }

    /// The host's capabilities: what is known (re-probed behind it), or a
    /// probe now. A probe that fails says nothing: everything is assumed
    /// present, as before the first probe (a missing tool then fails where
    /// it's used, and the next probe corrects it).
    async fn capabilities(&self, ssh: &Ssh) -> HostCapabilities {
        let id = self.spec.connection_id.clone();
        let memory = self.services.memory.clone();
        if let Some(known) = memory.capabilities(&id) {
            let ssh = ssh.clone();
            let bound = self.timing.channel_timeout;
            tokio::spawn(async move {
                if let Some(fresh) = probe(&ssh, bound).await {
                    memory.save_capabilities(&id, &fresh);
                }
            });
            return known;
        }
        match probe(ssh, self.timing.channel_timeout).await {
            Some(fresh) => {
                memory.save_capabilities(&id, &fresh);
                fresh
            }
            None => HostCapabilities::unknown(),
        }
    }
}

async fn probe(ssh: &Ssh, bound: Duration) -> Option<HostCapabilities> {
    let output = tokio::time::timeout(
        bound,
        ssh.run(&moshpit_install::capabilities::probe_command()),
    )
    .await
    .ok()?
    .ok()?;
    if !output.closed {
        return None;
    }
    Some(HostCapabilities::parse(&String::from_utf8_lossy(
        &output.merged(),
    )))
}

/// The bootstrap command line (`crates/mosh`). With tmux, tmux's own
/// client is the session's program rather than a line typed into a shell
/// that may still be starting (a prompt that flushes pending input eats it;
/// typed twice, the second copy lands in the pane): the remembered session
/// (`remembered`, tmux's `$N`) when it is still there, else the most recent
/// one — and with no session at all it waits for one (the person creates it
/// from the empty state, through the sidecar), never makes one.
///
/// The client is `exec`ed, so it is mosh-server's own child: the sidecar
/// finds the phone's renderer among tmux's clients by that (`pgrep -P`),
/// never by counting clients around an attach (B84). Detaching ends the
/// mosh session, as the app's `exec tmux attach` renderer did.
pub(crate) fn mosh_command(
    spec: &ConnectionSpec,
    multiplexer: Multiplexer,
    remembered: Option<&str>,
    herdr: Option<&MoshHerdr>,
) -> String {
    let mut command = bootstrap::command(
        spec.mosh_server_path.as_deref().unwrap_or("mosh-server"),
        &spec.locale,
        spec.mosh_port_range.0,
        spec.mosh_port_range.1,
    );
    let program = match (multiplexer, herdr) {
        (Multiplexer::Tmux, _) => Some(mosh_tmux_program(spec.tmux_command(), remembered)),
        (Multiplexer::Herdr, Some(herdr)) => Some(mosh_herdr_program(spec, herdr)),
        _ => spec.pty_command.clone(),
    };
    if let Some(program) = program {
        command.push_str(" -- ");
        command.push_str(&program);
    }
    command
}

/// mosh's program with tmux: attach and nothing else, under `sh -c` (any
/// login shell parses it the same, B138).
pub(crate) fn mosh_tmux_program(tmux: &str, remembered: Option<&str>) -> String {
    let mut script = String::new();
    if let Some(id) = remembered.filter(|id| id.starts_with('$') && is_valid_id(id)) {
        let target = quote(id);
        script.push_str(&format!(
            "if {tmux} has-session -t {target} 2>/dev/null; then exec {tmux} attach -t {target}; fi; "
        ));
    }
    script.push_str(&format!(
        "until {tmux} has-session 2>/dev/null; do sleep 1; done; exec {tmux} attach"
    ));
    format!("/bin/sh -c {}", quote(&script))
}

/// mosh's program with herdr: the raw-attach loop (or herdr's own TUI), and
/// the login shell after it, under `sh -c` (B138).
///
/// The loop is the program rather than a line typed into the login shell
/// once mosh is up, as the app's raw-attach renderer typed it: a shell still
/// starting swallows a line typed into it (Powerlevel10k's instant prompt
/// flushes pending input), and the loop then started only after the
/// renderer's check had timed out and the line was typed again (round
/// twenty-eight's retype) — seconds of a bare shell, the long line echoed on
/// screen. As the program it runs at once, under no shell that could take
/// what is typed as a command line.
///
/// Interrupt, quit and suspend are ignored while it runs: a `^C` that came
/// between two attaches (the tty is the loop's in cooked mode then) ended
/// the loop. Should it end all the same (killed), the login shell takes the
/// tty, with those signals back: the renderer says the loop is gone and
/// types it again on the person's word ("Retry").
pub(crate) fn mosh_herdr_program(spec: &ConnectionSpec, herdr: &MoshHerdr) -> String {
    let custom = spec.herdr_path.as_deref();
    let renderer = match &herdr.renderer_key {
        Some(key) => launch::raw_attach_loop_command(key, custom),
        None => launch::attach_command(custom),
    };
    let shell = shell_after(spec);
    let script = format!("trap '' INT QUIT TSTP; {renderer}; trap - INT QUIT TSTP; {shell}");
    format!("/bin/sh -c {}", quote(&script))
}

/// What a channel whose program is ours runs once that program is done:
/// the shell the channel would have had (the spec's command, else the
/// account's login shell), `exec`ed in its place.
pub(crate) fn shell_after(spec: &ConnectionSpec) -> String {
    match &spec.pty_command {
        Some(command) => format!("exec {command}"),
        None => "exec \"${SHELL:-/bin/sh}\" -l".to_owned(),
    }
}

/// Eight hex digits for a renderer generation's key (the app's
/// `UUID().uuidString.prefix(8)`): distinct per boot, not a secret.
fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
    );
    format!("{:08X}", hasher.finish() as u32)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Where mosh's UDP goes when the SSH connection went straight to the host:
/// the address it reached (B171) — the one `mosh-server -s` bound to, where
/// a fresh lookup of a name with several addresses (`localhost`: `::1`
/// first) can find one nothing listens on — at the port the server named.
/// Only the port changes: a link-local peer's interface (its scope) goes
/// with it.
fn udp_server(mut peer: SocketAddr, udp_port: u16) -> SocketAddr {
    peer.set_port(udp_port);
    peer
}

/// The address mosh's UDP flow goes to, when the SSH connection doesn't
/// say (through a proxy), looked up within `bound` (B452): the one step of
/// a dial nothing bounded. A resolver that never answered left the session
/// "Connecting" for good — every other step gives up at its own 15 s, and
/// a dial that is still moving is never cut short.
async fn resolve(host: &str, port: u16, bound: std::time::Duration) -> Result<SocketAddr, Failure> {
    lookup_within(tokio::net::lookup_host((host, port)), bound)
        .await
        .ok_or_else(|| {
            Failure::new(
                FailureKind::Unreachable,
                Cause::Unreachable {
                    host: host.to_owned(),
                    port,
                    why: crate::event::UnreachableWhy::NoAddress,
                },
                format!("{host} resolves to no address for mosh"),
            )
        })
}

/// The first address `lookup` finds within `bound`; none when it finds none,
/// fails, or takes longer.
async fn lookup_within<I: Iterator<Item = SocketAddr>>(
    lookup: impl std::future::Future<Output = std::io::Result<I>>,
    bound: std::time::Duration,
) -> Option<SocketAddr> {
    tokio::time::timeout(bound, lookup)
        .await
        .ok()?
        .ok()
        .and_then(|mut addresses| addresses.next())
}

/// Dials and signs in to `spec`'s host, asking `prompt` about a host key.
///
/// The question is asked inside the handshake: this side's clock stops for
/// it, the server's login grace doesn't (OpenSSH's LoginGraceTime, 120 s by
/// default, less on many hosts). A yes given after the server hung up was
/// stored, but the handshake it answered had nothing left to finish on, and
/// the attempt read as a server that closed the connection during setup —
/// "check the port" — until the next tick dialled again (B391). So a dial
/// that failed that way after a yes is made again at once: the key is
/// trusted now and the credential cached, so nothing is asked twice.
///
/// Nobody answers the server's sign-in questions here: a dial off the
/// person's sight (a side connection) can't ask them.
pub(crate) async fn dial(
    services: &Services,
    spec: &ConnectionSpec,
    timing: &Timing,
    prompt: Arc<dyn HostKeyPrompt>,
) -> Result<Ssh, Failure> {
    dial_asking(services, spec, timing, prompt, None).await
}

/// [`dial`], with `questions` answering what the server asks at sign-in
/// beyond the password (B563).
pub(crate) async fn dial_asking(
    services: &Services,
    spec: &ConnectionSpec,
    timing: &Timing,
    prompt: Arc<dyn HostKeyPrompt>,
    questions: Option<Arc<dyn SignInQuestions>>,
) -> Result<Ssh, Failure> {
    let said_yes = Arc::new(AtomicBool::new(false));
    let noting = Arc::new(NotingYes {
        prompt: prompt.clone(),
        said_yes: said_yes.clone(),
    });
    match dial_once(services, spec, timing, noting, questions.clone()).await {
        Err(failure)
            if said_yes.load(Ordering::SeqCst)
                && matches!(failure.kind, FailureKind::Handshake | FailureKind::Lost) =>
        {
            log::info!(
                "{}: the host hung up while its key was being decided ({}); dialling again",
                target_name(&spec.target),
                failure.log
            );
            dial_once(services, spec, timing, prompt, questions).await
        }
        result => result,
    }
}

/// A host-key prompt that notes a yes (B391).
struct NotingYes {
    prompt: Arc<dyn HostKeyPrompt>,
    said_yes: Arc<AtomicBool>,
}

impl NotingYes {
    fn noted<'a>(&'a self, answer: BoxFuture<'a, bool>) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let yes = answer.await;
            if yes {
                self.said_yes.store(true, Ordering::SeqCst);
            }
            yes
        })
    }
}

impl HostKeyPrompt for NotingYes {
    fn trust_new_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> BoxFuture<'a, bool> {
        self.noted(self.prompt.trust_new_host(host, port, key))
    }

    fn accept_changed_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
        expected: &'a str,
    ) -> BoxFuture<'a, bool> {
        self.noted(self.prompt.accept_changed_host(host, port, key, expected))
    }

    fn accept_conflicting_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> BoxFuture<'a, bool> {
        self.noted(self.prompt.accept_conflicting_host(host, port, key))
    }
}

async fn dial_once(
    services: &Services,
    spec: &ConnectionSpec,
    timing: &Timing,
    prompt: Arc<dyn HostKeyPrompt>,
    questions: Option<Arc<dyn SignInQuestions>>,
) -> Result<Ssh, Failure> {
    let auth = services
        .credentials
        .auth(&spec.connection_id)
        .await
        .map_err(|error| match error {
            CredentialError::Locked => Failure::credentials(
                CredentialsWhy::Locked,
                "the keychain can't be read while the device is locked",
            ),
            CredentialError::Cancelled => {
                Failure::credentials(CredentialsWhy::Cancelled, "signing in was cancelled")
            }
            CredentialError::Unavailable(log) => {
                Failure::credentials(CredentialsWhy::Unavailable, log)
            }
        })?;
    let policy = Arc::new(TrustOnFirstUse::new(services.known_hosts.clone(), prompt));
    let options = timing.connect_options(spec.proxy.clone());
    Ssh::connect_asking(&spec.target, auth, policy, questions, &options)
        .await
        .map_err(|e: SshError| {
            let failure = Failure::from(&e);
            // Through a proxy, a dial nothing answered never got past the
            // proxy: the person checks the proxy, not the server.
            match (&spec.proxy, failure.kind) {
                (Some(proxy), FailureKind::Unreachable) => Failure::new(
                    FailureKind::Proxy,
                    Cause::Proxy {
                        proxy: format!("{}:{}", proxy.host, proxy.port),
                        why: ProxyWhy::Unreachable,
                    },
                    failure.log,
                ),
                _ => failure,
            }
        })
}

/// Stops mosh-servers we started, over a connection of its own: one the
/// session never reached (it waits forever for a client that won't come),
/// one whose session the person ended or abandoned (nothing can ever
/// reattach to it: the key is not kept).
pub(crate) async fn reap_mosh_server(
    services: Services,
    spec: ConnectionSpec,
    timing: Timing,
    pids: Vec<i64>,
) {
    let commands = pids.iter().map(|pid| reap_command(*pid)).collect();
    run_on_host(services, spec, timing, commands, "stop mosh-server").await;
}

/// Pays what a retired tmux controller still owed the server — the window
/// sizes it pinned, a status line the mosh sidecar turned off — when no
/// replacement came to settle it (the redial failed, or landed on a plain
/// shell) and the person closed the session. Without it the desktop's
/// windows stay at the phone's size.
pub(crate) async fn pay_pin_debt(
    services: Services,
    spec: ConnectionSpec,
    timing: Timing,
    ledger: PinLedger,
    renderer_server: Option<i64>,
) {
    let commands = pin_debt_commands(&spec, &ledger, renderer_server);
    if commands.is_empty() {
        return;
    }
    run_on_host(services, spec, timing, commands, "hand tmux windows back").await;
}

/// What paying `ledger` runs, in order: the phone's mosh renderer out of
/// the session first — mosh-server `renderer_server`'s tmux client, and the
/// client the ledger knows for it (B270: still attached, it was the client
/// the windows went back to) — the pins unset, then the layouts the pins
/// squashed put back at their size (B150, `pristine_layout_restore_commands`
/// of a controller holding the ledger), then the status line.
pub(crate) fn pin_debt_commands(
    spec: &ConnectionSpec,
    ledger: &PinLedger,
    renderer_server: Option<i64>,
) -> Vec<String> {
    let tmux = spec.tmux_command();
    let mut renderer: Vec<String> = renderer_server
        .map(|server| moshpit_tmux::boot::renderer_detach_by_server_command(tmux, server))
        .into_iter()
        .collect();
    renderer.extend(
        ledger
            .renderer
            .iter()
            .map(|renderer| moshpit_tmux::boot::renderer_detach_command(tmux, renderer)),
    );
    // The pins and the layouts in one script that reads every window first
    // and puts back only what is still in a layout of ours (round
    // thirty-four), after the renderer is out.
    let mut tally = moshpit_tmux::controller::Controller::new(moshpit_tmux::controller::Config {
        renders_output: false,
        ..Default::default()
    });
    tally.inherit_pin_ledger(ledger.clone());
    let mut commands: Vec<String> = tally
        .pin_hand_back_script(tmux, &renderer)
        .into_iter()
        .collect();
    if let Some(session) = ledger.hidden_status.as_deref().filter(|s| is_valid_id(s)) {
        commands.extend(moshpit_tmux::boot::status_restore_commands(tmux, session));
    }
    commands
}

/// Runs `commands` over a connection of their own, to a host already
/// trusted (nobody is asked about a key), bounded as a whole.
async fn run_on_host(
    services: Services,
    spec: ConnectionSpec,
    timing: Timing,
    commands: Vec<String>,
    what: &str,
) {
    let work = async {
        let prompt: Arc<dyn HostKeyPrompt> = Arc::new(DenyAll);
        if let Ok(ssh) = dial(&services, &spec, &timing, prompt).await {
            for command in &commands {
                let _ = ssh.run(command).await;
            }
            ssh.close().await;
        }
    };
    if tokio::time::timeout(ON_HOST_BOUND, work).await.is_err() {
        log::info!("couldn't reach {} to {what}", target_name(&spec.target));
    }
}

/// How long a teardown's own connection may take as a whole.
const ON_HOST_BOUND: Duration = Duration::from_secs(8);

fn target_name(target: &Target) -> String {
    format!("{}@{}:{}", target.user, target.host, target.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B452: a lookup that never answers gives up at its bound, rather than
    /// leave the dial "Connecting" for good; one that answers is used.
    #[tokio::test(start_paused = true)]
    async fn mosh_s_address_lookup_gives_up_at_its_bound() {
        let bound = std::time::Duration::from_secs(15);
        let hung = std::future::pending::<std::io::Result<std::vec::IntoIter<SocketAddr>>>();
        let started = tokio::time::Instant::now();
        assert_eq!(lookup_within(hung, bound).await, None);
        assert_eq!(started.elapsed(), bound);
        let address: SocketAddr = "192.0.2.7:60001".parse().unwrap();
        let found = std::future::ready(Ok(vec![address].into_iter()));
        assert_eq!(lookup_within(found, bound).await, Some(address));
    }

    /// B171: mosh's datagrams go to the address SSH reached, at the
    /// server's port — and a link-local peer keeps its interface, without
    /// which `fe80::…` has no route (the Swift transport kept it as
    /// `fe80::1%lo0`).
    #[test]
    fn mosh_goes_where_ssh_went_and_a_link_local_peer_keeps_its_interface() {
        let v4: SocketAddr = "198.51.100.7:22".parse().unwrap();
        assert_eq!(udp_server(v4, 60001), "198.51.100.7:60001".parse().unwrap());
        let scoped = SocketAddr::V6(std::net::SocketAddrV6::new(
            "fe80::1".parse().unwrap(),
            22,
            0,
            7,
        ));
        let SocketAddr::V6(server) = udp_server(scoped, 60001) else {
            panic!("a v6 peer became v4");
        };
        assert_eq!(
            server.ip(),
            &"fe80::1".parse::<std::net::Ipv6Addr>().unwrap()
        );
        assert_eq!(server.port(), 60001);
        assert_eq!(
            server.scope_id(),
            7,
            "the interface was dropped: no route to fe80::1"
        );
    }

    fn caps(tmux: bool, mosh: bool, herdr: bool) -> HostCapabilities {
        HostCapabilities {
            has_tmux: tmux,
            has_mosh_server: mosh,
            os: "Linux".into(),
            package_manager: None,
            has_herdr: herdr,
            has_python3: true,
        }
    }

    fn spec(multiplexer: Multiplexer) -> ConnectionSpec {
        let mut spec = ConnectionSpec::ssh("c", Target::new("me", "box", 22));
        spec.multiplexer = multiplexer;
        spec
    }

    #[test]
    fn a_missing_multiplexer_is_a_plain_shell_never_the_other_one() {
        assert_eq!(
            route(&spec(Multiplexer::Tmux), &caps(false, true, true), true),
            Route {
                multiplexer: Multiplexer::None,
                degrade: Some(Degrade::TmuxMissing)
            }
        );
        assert_eq!(
            route(&spec(Multiplexer::Herdr), &caps(true, true, false), true),
            Route {
                multiplexer: Multiplexer::None,
                degrade: Some(Degrade::HerdrMissing)
            }
        );
        assert_eq!(
            route(&spec(Multiplexer::Tmux), &caps(true, false, false), false),
            Route {
                multiplexer: Multiplexer::Tmux,
                degrade: None
            }
        );
        assert_eq!(
            route(&spec(Multiplexer::None), &caps(false, false, false), false),
            Route {
                multiplexer: Multiplexer::None,
                degrade: None
            }
        );
    }

    #[test]
    fn a_custom_path_is_trusted_over_the_probe() {
        let mut vouched = spec(Multiplexer::Tmux);
        vouched.tmux_path = Some("/opt/bin/tmux".into());
        assert_eq!(
            route(&vouched, &caps(false, false, false), false).multiplexer,
            Multiplexer::Tmux
        );
        let mut mosh = spec(Multiplexer::None);
        assert!(mosh_falls_back(&mosh, &caps(false, false, false)));
        assert!(!mosh_falls_back(&mosh, &caps(false, true, false)));
        mosh.mosh_server_path = Some("/opt/bin/mosh-server".into());
        assert!(!mosh_falls_back(&mosh, &caps(false, false, false)));
    }

    #[test]
    fn herdr_without_a_client_is_a_plain_shell_that_says_so() {
        assert_eq!(
            route(&spec(Multiplexer::Herdr), &caps(true, true, true), false),
            Route {
                multiplexer: Multiplexer::None,
                degrade: Some(Degrade::HerdrUnavailable)
            }
        );
        assert_eq!(
            route(&spec(Multiplexer::Herdr), &caps(true, true, true), true).multiplexer,
            Multiplexer::Herdr
        );
    }

    #[test]
    fn the_unprobed_host_is_assumed_to_have_everything() {
        let unknown = HostCapabilities::unknown();
        assert_eq!(
            route(&spec(Multiplexer::Tmux), &unknown, false).multiplexer,
            Multiplexer::Tmux
        );
        assert!(!mosh_falls_back(&spec(Multiplexer::None), &unknown));
    }

    #[test]
    fn the_mosh_command_only_starts_a_server() {
        let mut spec = spec(Multiplexer::None);
        assert_eq!(
            mosh_command(&spec, Multiplexer::None, None, None),
            "PATH=\"$PATH:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin\" mosh-server new -s -c 256 -l LANG=en_US.UTF-8"
        );
        spec.mosh_port_range = (Some(60001), Some(60010));
        spec.pty_command = Some("/bin/sh".into());
        assert!(
            mosh_command(&spec, Multiplexer::None, Some("$1"), None)
                .ends_with(" -p 60001:60010 -l LANG=en_US.UTF-8 -- /bin/sh")
        );
    }

    #[test]
    fn mosh_with_tmux_execs_tmux_attach_and_never_creates_a_session() {
        let mut spec = spec(Multiplexer::Tmux);
        let command = mosh_command(&spec, Multiplexer::Tmux, None, None);
        assert!(
            command.ends_with(
                " -- /bin/sh -c 'until tmux has-session 2>/dev/null; do sleep 1; done; exec tmux attach'"
            ),
            "{command}"
        );
        // The session the person was on, by its id, when it is still there.
        let command = mosh_command(&spec, Multiplexer::Tmux, Some("$3"), None);
        assert!(
            command.ends_with(
                " -- /bin/sh -c 'if tmux has-session -t '\\''$3'\\'' 2>/dev/null; then exec tmux attach -t '\\''$3'\\''; fi; until tmux has-session 2>/dev/null; do sleep 1; done; exec tmux attach'"
            ),
            "{command}"
        );
        // Only a real session id is spliced in.
        for bad in ["$3; reboot", "@3", "work"] {
            let command = mosh_command(&spec, Multiplexer::Tmux, Some(bad), None);
            assert!(!command.contains("has-session -t"), "{command}");
        }
        spec.tmux_path = Some("/opt/my tmux/tmux -L it's".into());
        let command = mosh_command(&spec, Multiplexer::Tmux, None, None);
        assert!(
            command.contains("exec /opt/my tmux/tmux -L it'\\''s attach"),
            "{command}"
        );
        let program = command.split(" -- ").nth(1).unwrap();
        for forbidden in ["new-session", " new ", "set -g", "set-option"] {
            assert!(!program.contains(forbidden), "{program}");
        }
        // A degraded route (no tmux on the host) is a plain shell.
        assert!(!mosh_command(&spec, Multiplexer::None, None, None).contains("attach"));
    }

    #[test]
    fn a_late_reap_only_stops_what_is_still_a_mosh_server() {
        // B50, B195: a pid stopped long after it was reported may belong to
        // something else by then.
        let command = reap_command(4242);
        assert!(
            command.contains(
                "case \"$(ps -o comm= -p 4242 2>/dev/null)\" in *mosh-server*) kill 4242;; esac"
            ),
            "{command}"
        );
        assert!(command.contains("else kill 4242; fi"));
        assert!(command.ends_with("; true"));
    }

    #[test]
    fn a_pin_debt_hands_back_sizes_and_the_status_line() {
        let spec = spec(Multiplexer::Tmux);
        let mut ledger = PinLedger::with_pins(["@1".to_owned(), "@2".to_owned()].into());
        ledger.hidden_status = Some("$4".into());
        assert_eq!(
            pin_debt_commands(&spec, &ledger, None),
            [
                format!(
                    "sh -c {}",
                    quote(
                        "tmux set-option -u -w -t @1 window-size; \
                         tmux set-option -u -w -t @2 window-size; true"
                    )
                ),
                "tmux set-option -t '$4' -u status".to_owned(),
                "tmux set-option -t '$4' -u @moshpit_status_hidden".to_owned(),
            ]
        );
        // Nothing but ids reaches the command line.
        let mut odd = PinLedger::with_pins(["@1;reboot".to_owned()].into());
        odd.hidden_status = Some("x y".into());
        assert!(pin_debt_commands(&spec, &odd, None).is_empty());
    }

    /// B270: mosh + tmux with the phone the only client. The ledger's pins
    /// went back to automatic sizing while the phone's renderer — mosh's
    /// tmux client — was still attached and sizing the session: the windows
    /// went straight back to the phone's grid (and the layouts the pins
    /// squashed were never put back on this path at all). Now the renderer
    /// is taken out first — by its server's pid and by the client the ledger
    /// knows — and each layout is put back at its size before its window
    /// goes back to automatic sizing.
    #[test]
    fn a_pin_debt_takes_the_renderer_out_then_puts_the_layouts_back_at_their_size() {
        let spec = spec(Multiplexer::Tmux);
        let split = "bf8c,200x60,0,0[200x30,0,0,0,200x29,0,31{100x29,0,31,1,99x29,101,31,2}]";
        let mut ledger = PinLedger::with_pins(["@0".to_owned()].into());
        ledger.pristine_layouts.insert(
            "@0".to_owned(),
            moshpit_tmux::controller::PristineLayout {
                layout: split.to_owned(),
                zoomed_pane: None,
                pane_count: 3,
                ours: Vec::new(),
            },
        );
        ledger.renderer = Some(moshpit_tmux::clients::ClientIdentity {
            pid: 4481,
            name: "/dev/ttys023".to_owned(),
        });
        let commands = pin_debt_commands(&spec, &ledger, Some(4242));
        let steps = [
            moshpit_tmux::boot::renderer_detach_by_server_command("tmux", 4242),
            moshpit_tmux::boot::renderer_detach_command("tmux", ledger.renderer.as_ref().unwrap()),
            "tmux set-option -u -w -t @0 window-size".to_owned(),
            "tmux resize-window -t @0 -x 200 -y 60".to_owned(),
            format!("tmux select-layout -t @0 '{split}'"),
            "tmux set-option -u -w -t @0 window-size".to_owned(),
            "true".to_owned(),
        ];
        assert_eq!(
            commands,
            [format!("sh -c {}", quote(&steps.join("; ")))],
            "the renderer out, the pin unset, the layout back at its size"
        );
    }

    /// Round thirty-four: a record that knows the layouts our sizing left
    /// its window in is replayed only while the window is in one of them —
    /// read before the renderer goes and anything is unpinned. A desktop's
    /// rearrangement since is left alone.
    #[test]
    fn a_pin_debt_puts_a_layout_back_only_over_one_of_ours() {
        let spec = spec(Multiplexer::Tmux);
        let split = "bf8c,200x60,0,0[200x30,0,0,0,200x29,0,31{100x29,0,31,1,99x29,101,31,2}]";
        let squashed = "307e,60x20,0,0[60x10,0,0,0,60x9,0,11{30x9,0,11,1,29x9,31,11,2}]";
        let mut ledger = PinLedger::with_pins(["@0".to_owned()].into());
        ledger.pristine_layouts.insert(
            "@0".to_owned(),
            moshpit_tmux::controller::PristineLayout {
                layout: split.to_owned(),
                zoomed_pane: None,
                pane_count: 3,
                ours: vec![squashed.to_owned()],
            },
        );
        let commands = pin_debt_commands(&spec, &ledger, Some(4242));
        let steps = [
            "l0=$(tmux display-message -p -t @0 '#{window_layout}' 2>/dev/null)".to_owned(),
            moshpit_tmux::boot::renderer_detach_by_server_command("tmux", 4242),
            "tmux set-option -u -w -t @0 window-size".to_owned(),
            format!(
                "case \"$l0\" in \"{split}\"|\"{squashed}\") tmux resize-window -t @0 -x 200 -y 60; \
                 tmux select-layout -t @0 '{split}'; tmux set-option -u -w -t @0 window-size;; esac"
            ),
            "true".to_owned(),
        ];
        assert_eq!(commands, [format!("sh -c {}", quote(&steps.join("; ")))]);
    }
}
