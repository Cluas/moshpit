//! The live sessions: `moshpit-session`'s registry on the core's runtime,
//! what the platform gives it (credentials from the Keystore-wrapped
//! keychain, hardware keys that sign in the Keystore, the trusted host keys,
//! the session memory), and each connection's session as the screens read
//! it.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_session::{
    ConnectionSpec, CredentialError, Credentials, Degrade, Event, EventKind, Events, FailureKind,
    HerdrAction, HerdrView, LinkFault, Mark, PaneInfo, Protocol, Services, SessionId,
    SessionMemory, SessionState, Sessions, StreamId, SystemUdp, Timing, TmuxAction, TmuxView,
    UdpFlow, UdpNetwork,
};
use moshpit_ssh::{
    Auth, BoxFuture, HardwareKey, HostSignature, KeySigner, KnownHosts, SignerError, Socks5Proxy,
    Target,
};
use moshpit_store::connections::{
    AuthMethod, ConnectionProtocol, ConnectionStore, Multiplexer, ServerConnection,
};
use moshpit_store::known_hosts::KeychainKnownHosts;
use moshpit_store::session_memory::{self, TmuxSelection};
use moshpit_store::ssh_keys::{SshKeySource, SshKeyStore};
use moshpit_store::{Defaults, Keychain, Readiness};
use moshpit_tmux::model::Selection;

use crate::app::{App, Core, Options, PageKind};
use crate::platform::{Ask, Platform};

/// A host key the person is asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyAsk {
    pub prompt: u64,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub previous: Option<String>,
    /// `ssh-rsa` when the server offers no better host-key algorithm
    /// (B564): the question says so, as iOS's card does.
    pub older_algorithm: Option<String>,
}

/// One connection's session, as the screens read it.
#[derive(Debug, Clone)]
pub struct Conn {
    pub id: SessionId,
    pub state: SessionState,
    /// The current attach's panes.
    pub attach: u64,
    pub panes: Vec<PaneInfo>,
    /// The attach whose panes the terminal shows (its first frame came).
    pub shown: u64,
    pub shown_panes: Vec<PaneInfo>,
    /// When it connected (Unix seconds).
    pub connected_at: Option<f64>,
    pub degrade: Option<Degrade>,
    pub return_path_dead: bool,
    pub attach_stalled: bool,
    pub roaming: bool,
    pub mosh: bool,
    pub disconnecting: bool,
    pub tmux: Option<TmuxView>,
    /// herdr's tree and what is said around it (W2b's client on the
    /// session: a herdr connection over SSH).
    pub herdr: Option<HerdrView>,
    /// The mosh-server this session started (the tests stop only theirs).
    pub mosh_server_pid: Option<i64>,
    /// It has been live once (a reconnect veils the kept frame; before
    /// that, the poster).
    pub ever_live: bool,
    /// mosh's smoothed round trip, when it says.
    pub srtt_ms: Option<f64>,
}

/// The registry and what each connection has.
#[derive(Default)]
pub struct Live {
    sessions: Option<Sessions>,
    /// The trusted host keys the sessions decide by: what Home's marks are
    /// made from (a host's key's face).
    known_hosts: Option<Arc<KnownHosts>>,
    pub(crate) by_connection: HashMap<Uuid, Conn>,
    /// Every mosh datagram dropped, both ways, while set (a debug build's
    /// fault injection: the UDP silence test).
    blackhole: Arc<AtomicBool>,
}

impl Live {
    /// Starts the registry; its events, for the controller's pump.
    pub fn start(
        &mut self,
        platform: &Arc<dyn Platform>,
        defaults: &Arc<dyn Defaults>,
        keychain: &Arc<dyn Keychain>,
        options: &Options,
    ) -> Option<Events> {
        if self.sessions.is_some() {
            return None;
        }
        let known_hosts = Arc::new(KnownHosts::new(Arc::new(KeychainKnownHosts::new(
            keychain.clone(),
        ))));
        let credentials = Arc::new(KeystoreCredentials {
            platform: platform.clone(),
            defaults: defaults.clone(),
            keychain: keychain.clone(),
        });
        self.known_hosts = Some(known_hosts.clone());
        let mut services = Services::new(credentials, known_hosts);
        services.memory = Arc::new(StoreMemory {
            defaults: defaults.clone(),
        });
        services.device_label = options.device_label.clone();
        services.udp = Arc::new(FaultableUdp {
            blackhole: self.blackhole.clone(),
        });
        let (sessions, events) =
            Sessions::new(moshpit_runtime::handle(), services, Timing::default());
        self.sessions = Some(sessions);
        Some(events)
    }

    pub fn sessions(&self) -> Option<&Sessions> {
        self.sessions.as_ref()
    }

    pub fn conn(&self, connection: Uuid) -> Option<&Conn> {
        self.by_connection.get(&connection)
    }

    /// The fingerprint trusted for `host` on `port`; none before the
    /// registry starts, or for a host never trusted.
    pub fn trusted(&self, host: &str, port: i64) -> Option<String> {
        let port = u16::try_from(port).ok()?;
        self.known_hosts.as_ref()?.trusted(host, port)
    }

    fn connection_of(&self, session: SessionId) -> Option<Uuid> {
        self.by_connection
            .iter()
            .find(|(_, c)| c.id == session)
            .map(|(id, _)| *id)
    }

    /// Connects `connection` unless a session is up or on its way; the
    /// session's id. `udp` is Settings' UDP range (B430).
    pub fn connect(
        &mut self,
        connection: &ServerConnection,
        options: &Options,
        udp: (i64, i64),
    ) -> Option<SessionId> {
        let sessions = self.sessions.as_ref()?;
        if let Some(conn) = self.by_connection.get(&connection.id) {
            match conn.state {
                SessionState::Ended(_) | SessionState::Failed(_) => {
                    sessions.reconnect(conn.id);
                    let conn = self.by_connection.get_mut(&connection.id)?;
                    conn.state = SessionState::Connecting;
                    return Some(conn.id);
                }
                _ => return Some(conn.id),
            }
        }
        let id = sessions.connect(spec(connection, options, udp));
        log::info!(target: "moshpit", "session {id}: connecting {}", connection.display_name());
        self.by_connection.insert(
            connection.id,
            Conn {
                id,
                state: SessionState::Connecting,
                attach: 0,
                panes: Vec::new(),
                shown: 0,
                shown_panes: Vec::new(),
                connected_at: None,
                degrade: None,
                return_path_dead: false,
                attach_stalled: false,
                roaming: false,
                mosh: connection.connection_protocol == ConnectionProtocol::Mosh,
                disconnecting: false,
                tmux: None,
                herdr: None,
                mosh_server_pid: None,
                ever_live: false,
                srtt_ms: None,
            },
        );
        Some(id)
    }

    /// Closes `connection`'s session for good; `on_closed` once it is torn
    /// down (tmux's pins handed back, a mosh-server stopped).
    pub fn disconnect(&mut self, connection: Uuid, on_closed: impl FnOnce() + Send + 'static) {
        let (Some(sessions), Some(conn)) = (
            self.sessions.as_ref(),
            self.by_connection.get_mut(&connection),
        ) else {
            return;
        };
        conn.disconnecting = true;
        let done = sessions.disconnect(conn.id);
        moshpit_runtime::handle().spawn(async move {
            let _ = done.await;
            on_closed();
        });
    }

    /// `connection`'s session asks tmux for `action`.
    pub fn tmux(&self, connection: Uuid, action: TmuxAction) {
        if let (Some(sessions), Some(conn)) =
            (self.sessions.as_ref(), self.by_connection.get(&connection))
            && let Err(refusal) = sessions.tmux(conn.id, action)
        {
            log::info!(target: "moshpit", "session {}: tmux refused: {refusal:?}", conn.id);
        }
    }

    /// `connection`'s session asks herdr for `action` (its answer comes as
    /// an event; nothing waits for it here).
    pub fn herdr(&self, connection: Uuid, action: HerdrAction) {
        if let (Some(sessions), Some(conn)) =
            (self.sessions.as_ref(), self.by_connection.get(&connection))
        {
            sessions.herdr(conn.id, action);
        }
    }

    /// Whether `connection`'s session runs herdr's client now.
    pub fn is_herdr(&self, connection: Uuid) -> bool {
        self.by_connection
            .get(&connection)
            .is_some_and(|c| c.herdr.is_some())
    }

    pub fn reconnect(&self, connection: Uuid) {
        if let (Some(sessions), Some(conn)) =
            (self.sessions.as_ref(), self.by_connection.get(&connection))
        {
            sessions.reconnect(conn.id);
        }
    }

    pub fn switch_to_ssh(&self, connection: Uuid) {
        if let (Some(sessions), Some(conn)) =
            (self.sessions.as_ref(), self.by_connection.get(&connection))
        {
            sessions.switch_to_ssh(conn.id);
        }
    }

    /// Breaks `connection`'s link the way a network would (the test hooks).
    pub fn inject(&self, connection: Uuid, fault: &str) -> bool {
        let (Some(sessions), Some(conn)) =
            (self.sessions.as_ref(), self.by_connection.get(&connection))
        else {
            return false;
        };
        match fault {
            "cut-tcp" => sessions.inject_fault(conn.id, LinkFault::Cut),
            "stall-tcp" => sessions.inject_fault(conn.id, LinkFault::Stall),
            "mosh-blackhole-on" => self.blackhole.store(true, Ordering::SeqCst),
            "mosh-blackhole-off" => self.blackhole.store(false, Ordering::SeqCst),
            _ => return false,
        }
        true
    }

    pub fn set_visible(&self, connection: Option<Uuid>) {
        if let Some(sessions) = self.sessions.as_ref() {
            let id = connection
                .and_then(|c| self.by_connection.get(&c))
                .map(|c| c.id);
            sessions.set_visible(id);
        }
    }

    pub fn set_foreground(&self, foreground: bool) {
        if let Some(sessions) = self.sessions.as_ref() {
            sessions.set_foreground(foreground);
        }
    }

    pub fn network_changed(&self) {
        if let Some(sessions) = self.sessions.as_ref() {
            sessions.network_changed();
        }
    }

    pub fn device_unlocked(&self) {
        if let Some(sessions) = self.sessions.as_ref() {
            sessions.device_unlocked();
        }
    }

    /// Types `text` into `connection`'s terminal on screen: `None` when
    /// there is none live.
    pub fn deliver(&self, connection: Uuid, text: &str) -> Option<()> {
        let sessions = self.sessions.as_ref()?;
        let conn = self.by_connection.get(&connection)?;
        let pane = active_pane(conn)?;
        sessions.paste(conn.id, &pane.pane, text.to_owned());
        Some(())
    }

    /// The stream the terminal of `connection` shows: the active pane of the
    /// attach on screen.
    pub fn stream(&self, connection: Uuid) -> Option<StreamId> {
        let conn = self.by_connection.get(&connection)?;
        active_pane(conn).map(|p| p.stream)
    }
}

/// The pane the terminal shows: tmux's active pane when it is among the
/// panes on screen (herdr's: the pane its frame channel shows, else its
/// focus), else the one the core marks active, else the first.
pub(crate) fn active_pane(conn: &Conn) -> Option<&PaneInfo> {
    let panes = if conn.shown_panes.is_empty() {
        &conn.panes
    } else {
        &conn.shown_panes
    };
    let tmux_active = conn
        .tmux
        .as_ref()
        .and_then(|t| t.snapshot.active_pane_id.clone());
    let herdr_active = conn.herdr.as_ref().and_then(|h| {
        h.frame_target
            .clone()
            .or_else(|| h.attached_pane.clone())
            .or_else(|| h.snapshot.active_pane_id.clone())
    });
    tmux_active
        .or(herdr_active)
        .and_then(|id| panes.iter().find(|p| p.pane == id))
        .or_else(|| panes.iter().find(|p| p.active))
        .or_else(|| panes.first())
}

/// What a session dials for `connection`. mosh-server's UDP ports are
/// Settings' (`udp`): the one source, "set in Settings, for every
/// connection". The connection's stored range is only what the setting was
/// at its last save, kept for older builds (J1): dialled from, a narrowed
/// setting never reached a saved connection (B430).
pub fn spec(connection: &ServerConnection, options: &Options, udp: (i64, i64)) -> ConnectionSpec {
    let port = u16::try_from(connection.port).unwrap_or(22);
    let mut spec = ConnectionSpec::ssh(
        connection.id.to_string(),
        Target::new(connection.username.clone(), connection.host.clone(), port),
    );
    if connection.use_socks_proxy == Some(true)
        && let Some(host) = connection
            .socks_proxy_host
            .clone()
            .filter(|h| !h.is_empty())
    {
        let proxy_port = connection
            .socks_proxy_port
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(1080);
        spec.proxy = Some(Socks5Proxy::new(host, proxy_port));
    }
    spec.protocol = match connection.connection_protocol {
        ConnectionProtocol::Mosh => Protocol::Mosh,
        ConnectionProtocol::Ssh => Protocol::Ssh,
    };
    spec.multiplexer = match connection.multiplexer() {
        Multiplexer::None => moshpit_session::Multiplexer::None,
        Multiplexer::Tmux => moshpit_session::Multiplexer::Tmux,
        Multiplexer::Herdr => moshpit_session::Multiplexer::Herdr,
    };
    let path = |p: &Option<String>| p.clone().filter(|p| !p.trim().is_empty());
    spec.tmux_path = path(&connection.tmux_path);
    spec.herdr_path = path(&connection.herdr_path);
    spec.mosh_server_path = path(&connection.mosh_server_path);
    spec.mosh_port_range = (Some(udp.0), Some(udp.1));
    spec.locale = options.shell_locale.clone();
    spec
}

impl App {
    /// One event of the sessions, on the pump's task.
    pub(crate) fn on_session_event(&mut self, core: &Core, event: Event) {
        let Some(connection) = self.sessions.connection_of(event.session) else {
            return;
        };
        let view = self.sessions.sessions().and_then(|s| s.view(event.session));
        let tmux = self
            .sessions
            .sessions()
            .and_then(|s| s.tmux_view(event.session));
        let herdr = self
            .sessions
            .sessions()
            .and_then(|s| s.herdr_view(event.session));
        let mut released = Vec::new();
        let mut host_key = None;
        {
            let Some(conn) = self.sessions.by_connection.get_mut(&connection) else {
                return;
            };
            if let Some(view) = &view {
                conn.degrade = view.degrade;
                conn.return_path_dead = view.return_path_dead;
                conn.attach_stalled = view.attach_stalled;
                conn.roaming = view.mosh_roaming;
                conn.mosh_server_pid = view.mosh_server_pid.or(conn.mosh_server_pid);
                conn.srtt_ms = view.mosh_srtt_ms.or(conn.srtt_ms);
            }
            conn.tmux = tmux;
            conn.herdr = herdr;
            match event.kind {
                EventKind::State(state) => {
                    log::info!(target: "moshpit", "session {}: {state:?}", conn.id);
                    conn.ever_live |= state.is_live();
                    if state.is_live() && conn.connected_at.is_none() {
                        conn.connected_at = Some(Date::now().0 + 978_307_200.0);
                    }
                    if matches!(state, SessionState::Ended(_) | SessionState::Failed(_)) {
                        conn.connected_at = None;
                    }
                    conn.state = state;
                }
                EventKind::Panes { attach, panes } => {
                    log::info!(target: "moshpit", "session {}: attach {attach}: {} panes", conn.id, panes.len());
                    conn.attach = attach;
                    conn.panes = panes;
                    if conn.shown_panes.is_empty() {
                        conn.shown = attach;
                        conn.shown_panes = conn.panes.clone();
                    }
                }
                EventKind::FirstFrame { attach } => {
                    if attach == conn.attach && conn.shown != attach {
                        released = conn
                            .shown_panes
                            .iter()
                            .filter(|old| !conn.panes.iter().any(|p| p.stream == old.stream))
                            .map(|p| p.stream)
                            .collect();
                        conn.shown = attach;
                        conn.shown_panes = conn.panes.clone();
                    }
                }
                EventKind::HostKeyPrompt {
                    prompt,
                    host,
                    port,
                    fingerprint,
                    previous,
                    older_algorithm,
                } => {
                    host_key = Some(HostKeyAsk {
                        prompt,
                        host,
                        port,
                        fingerprint,
                        previous,
                        older_algorithm,
                    });
                }
                EventKind::InputDiscarded { stream, bytes, why } => {
                    log::info!(target: "moshpit", "stream {stream}: {bytes} B not sent: {why:?}");
                }
                EventKind::HerdrAnswer { token, answer } => {
                    log::info!(target: "moshpit", "session {}: herdr {token}: {answer:?}", conn.id);
                }
                _ => {}
            }
        }
        for stream in released {
            self.platform.release_stream(stream);
        }
        if let Some(ask) = host_key {
            self.ask_host_key(connection, ask);
        }
        self.refresh_home();
        self.refresh_terminals(connection);
        let _ = core;
    }

    fn ask_host_key(&mut self, connection: Uuid, ask: HostKeyAsk) {
        let (title, mut message) = match &ask.previous {
            None => (
                "Trust this host?".to_owned(),
                format!(
                    "{}:{} offers a host key this device has not seen.\n\n{}",
                    ask.host, ask.port, ask.fingerprint
                ),
            ),
            Some(previous) => (
                "Host key changed".to_owned(),
                format!(
                    "{}:{} offers a different host key than the one trusted.\n\nNow: {}\nTrusted: {}",
                    ask.host, ask.port, ask.fingerprint, previous
                ),
            ),
        };
        if let Some(older) = &ask.older_algorithm {
            message.push_str(&format!(
                "\n\nThis server uses an older host-key algorithm ({older}, SHA-1)."
            ));
        }
        let destructive = ask.previous.is_some();
        let prompt = ask.prompt;
        self.ask(super::Question::TrustHostKey(connection, prompt), |token| {
            Ask::Confirm {
                token,
                title,
                message,
                ok: "Trust".to_owned(),
                destructive,
            }
        });
    }

    /// The person answered the host-key question.
    pub(crate) fn answer_host_key(&mut self, connection: Uuid, prompt: u64, trust: bool) {
        if let (Some(sessions), Some(conn)) = (
            self.sessions.sessions(),
            self.sessions.by_connection.get(&connection),
        ) {
            sessions.answer_host_key(conn.id, prompt, trust);
        }
    }

    /// A mark a pane's view read in its output: a view lifting its cover
    /// tells tmux's controller (the scroll hold waits for it).
    pub fn view_mark(&mut self, stream: StreamId, mark: Mark) {
        if mark.code != moshpit_session::view_marks::REVEAL {
            return;
        }
        let found = self.sessions.by_connection.iter().find_map(|(c, conn)| {
            conn.panes
                .iter()
                .chain(conn.shown_panes.iter())
                .find(|p| p.stream == stream)
                .map(|p| (*c, p.pane.clone()))
        });
        if let Some((connection, pane)) = found {
            self.sessions
                .tmux(connection, TmuxAction::ViewRevealed { pane });
        }
    }

    /// Connects `connection` (Home's head, the terminal's retry).
    pub(crate) fn connect(&mut self, connection: Uuid) {
        let Some(saved) = self
            .connections
            .connections()
            .iter()
            .find(|c| c.id == connection)
            .cloned()
        else {
            return;
        };
        let udp = self.settings.udp_range();
        self.sessions.connect(&saved, &self.options, udp);
        self.refresh_home();
    }

    /// Opens `connection`'s terminal (connecting it first when it has no
    /// session).
    pub(crate) fn enter(&mut self, core: &Core, connection: Uuid) {
        self.connect(connection);
        let top_is_it = matches!(self.pages.last().map(|p| &p.kind), Some(PageKind::Terminal(t)) if t.connection == connection);
        if !top_is_it {
            let page = crate::app::terminal::TerminalPage::new(connection);
            self.open(core, PageKind::Terminal(Box::new(page)), true);
        }
        self.sessions.set_visible(Some(connection));
    }

    /// Disconnects `connection` and forgets its session once it is down.
    pub(crate) fn disconnect(&mut self, core: &Core, connection: Uuid) {
        let weak = core.me.clone();
        self.sessions.disconnect(connection, move || {
            if let Some(core) = weak.upgrade() {
                let mut app = core.lock();
                app.forget_session(connection);
                app.refresh_home();
                app.refresh_terminals(connection);
            }
        });
        self.refresh_home();
    }

    /// `connection`'s session is down for good: its record goes, and every
    /// pane's view with it.
    pub(crate) fn forget_session(&mut self, connection: Uuid) {
        if let Some(conn) = self.sessions.by_connection.remove(&connection) {
            let streams: Vec<_> = conn
                .panes
                .iter()
                .chain(conn.shown_panes.iter())
                .map(|p| p.stream)
                .collect();
            for stream in streams {
                self.platform.release_stream(stream);
            }
        }
    }

    /// Why the last attempt failed, in words for the terminal's banner.
    pub(crate) fn failure_text(state: &SessionState) -> Option<String> {
        let SessionState::Failed(failure) = state else {
            return None;
        };
        Some(
            match failure.kind {
                FailureKind::Unreachable => "The host did not answer.",
                FailureKind::Handshake => "The SSH handshake failed.",
                FailureKind::HostKey => "The host key was not trusted.",
                FailureKind::HostKeysLocked => "Unlock the device to read the trusted host keys.",
                FailureKind::Credentials => "The host refused the password or key.",
                FailureKind::CredentialsLocked => "Unlock the device to read the saved key.",
                FailureKind::Proxy => "The SOCKS proxy refused the connection.",
                FailureKind::Channel => "The host refused a terminal.",
                FailureKind::MoshBootstrap => "mosh-server did not start.",
                FailureKind::Lost => "The connection was lost.",
            }
            .to_owned(),
        )
    }
}

// MARK: - What the sessions get from the platform

/// Credentials from the keychain, read for one attempt; hardware keys sign
/// in the Keystore.
struct KeystoreCredentials {
    platform: Arc<dyn Platform>,
    defaults: Arc<dyn Defaults>,
    keychain: Arc<dyn Keychain>,
}

impl Credentials for KeystoreCredentials {
    fn auth<'a>(&'a self, connection_id: &'a str) -> BoxFuture<'a, Result<Auth, CredentialError>> {
        Box::pin(async move { self.read(connection_id) })
    }
}

impl KeystoreCredentials {
    fn read(&self, connection_id: &str) -> Result<Auth, CredentialError> {
        let unavailable = |why: &str| CredentialError::Unavailable(why.to_owned());
        let id = Uuid::parse(connection_id).ok_or_else(|| unavailable("not a connection id"))?;
        let mut store = ConnectionStore::new(self.defaults.clone(), Readiness::System);
        store.load();
        let connection = store
            .connections()
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| unavailable("no such connection"))?;
        let load = |account: &str| -> Result<String, CredentialError> {
            let bytes = self
                .keychain
                .load(account, "")
                .map_err(|e| CredentialError::Unavailable(e.to_string()))?;
            String::from_utf8(bytes).map_err(|_| unavailable("not text"))
        };
        match connection.auth_method {
            AuthMethod::Password => {
                let account = connection
                    .keychain_ref
                    .as_deref()
                    .ok_or_else(|| unavailable("no password saved"))?;
                Ok(Auth::password(load(account)?))
            }
            AuthMethod::Key => {
                let mut keys = SshKeyStore::new(self.defaults.clone(), Readiness::System);
                keys.load();
                let record = connection
                    .ssh_key_id
                    .and_then(|id| keys.keys().iter().find(|k| k.id == id).cloned());
                if let Some(record) = &record
                    && record.source == SshKeySource::SecureEnclave
                {
                    let alias = record
                        .keychain_ref
                        .clone()
                        .ok_or_else(|| unavailable("the hardware key has no alias"))?;
                    let signer: Arc<dyn KeySigner> =
                        Arc::new(KeystoreSigner(self.platform.clone()));
                    let key = HardwareKey::new(&record.public_key, alias, signer)
                        .map_err(|e| CredentialError::Unavailable(e.to_string()))?;
                    return Ok(Auth::Hardware(key));
                }
                let account = record
                    .as_ref()
                    .and_then(|r| r.keychain_ref.clone())
                    .or(connection.keychain_ref.clone())
                    .ok_or_else(|| unavailable("no key saved"))?;
                let text = zeroize_string(load(&account)?);
                // A key stored as pasted (a PEM, Windows line endings) is
                // converted for this attempt, as iOS converts it (B125).
                use moshpit_ssh::keyfile::{Assessment, assess};
                match assess(&text, None) {
                    Ok(Assessment::Usable) => Ok(Auth::key(text.to_string())),
                    Ok(Assessment::Converted(converted)) => Ok(Auth::key(converted.to_string())),
                    Ok(Assessment::NeedsPassphrase) => Err(CredentialError::Unavailable(
                        moshpit_ssh::keyfile::ImportError::NeedsPassphrase.to_string(),
                    )),
                    Err(e) => Err(CredentialError::Unavailable(e.to_string())),
                }
            }
        }
    }
}

fn zeroize_string(text: String) -> zeroize_shim::Wiped {
    zeroize_shim::Wiped(text)
}

mod zeroize_shim {
    //! A string wiped when dropped (the key text read for one attempt).
    pub struct Wiped(pub String);

    impl std::ops::Deref for Wiped {
        type Target = String;
        fn deref(&self) -> &String {
            &self.0
        }
    }

    impl Drop for Wiped {
        fn drop(&mut self) {
            // SAFETY: zeroes are valid UTF-8.
            unsafe { self.0.as_bytes_mut() }.fill(0);
        }
    }
}

/// Signs with a Keystore key: SHA256withECDSA, ASN.1 DER.
struct KeystoreSigner(Arc<dyn Platform>);

impl KeySigner for KeystoreSigner {
    fn sign<'a>(
        &'a self,
        key_ref: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<HostSignature, SignerError>> {
        let platform = self.0.clone();
        let alias = key_ref.to_owned();
        let data = data.to_vec();
        Box::pin(async move {
            let signed = tokio::task::spawn_blocking(move || platform.hardware_sign(&alias, &data))
                .await
                .map_err(|e| SignerError::Failed(e.to_string()))?;
            signed.map(HostSignature::Der).map_err(SignerError::Failed)
        })
    }
}

/// What a session remembers, in the preferences under the app's keys.
struct StoreMemory {
    defaults: Arc<dyn Defaults>,
}

impl SessionMemory for StoreMemory {
    fn tmux_selection(&self, connection_id: &str) -> Option<Selection> {
        let stored: TmuxSelection = session_memory::read(
            self.defaults.as_ref(),
            &session_memory::tmux_selection_key(connection_id),
        )?;
        Some(Selection {
            session: stored.session,
            window: stored.window,
            pane: stored.pane,
        })
    }

    fn save_tmux_selection(&self, connection_id: &str, selection: &Selection) {
        let stored = TmuxSelection {
            session: selection.session.clone(),
            window: selection.window.clone(),
            pane: selection.pane.clone(),
        };
        session_memory::write(
            self.defaults.as_ref(),
            &Readiness::System,
            &session_memory::tmux_selection_key(connection_id),
            &stored,
        );
    }

    fn capabilities(&self, connection_id: &str) -> Option<moshpit_install::HostCapabilities> {
        session_memory::read(
            self.defaults.as_ref(),
            &session_memory::host_capabilities_key(connection_id),
        )
    }

    fn save_capabilities(
        &self,
        connection_id: &str,
        capabilities: &moshpit_install::HostCapabilities,
    ) {
        session_memory::write(
            self.defaults.as_ref(),
            &Readiness::System,
            &session_memory::host_capabilities_key(connection_id),
            capabilities,
        );
    }
}

/// The system's UDP, which drops every datagram while told to.
struct FaultableUdp {
    blackhole: Arc<AtomicBool>,
}

impl UdpNetwork for FaultableUdp {
    fn open(&self, server: SocketAddr) -> BoxFuture<'static, std::io::Result<Box<dyn UdpFlow>>> {
        let blackhole = self.blackhole.clone();
        Box::pin(async move {
            let flow = SystemUdp.open(server).await?;
            Ok(Box::new(FaultableFlow { flow, blackhole }) as Box<dyn UdpFlow>)
        })
    }
}

struct FaultableFlow {
    flow: Box<dyn UdpFlow>,
    blackhole: Arc<AtomicBool>,
}

impl UdpFlow for FaultableFlow {
    fn send<'a>(&'a self, datagram: &'a [u8]) -> BoxFuture<'a, std::io::Result<()>> {
        if self.blackhole.load(Ordering::SeqCst) {
            return Box::pin(async { Ok(()) });
        }
        self.flow.send(datagram)
    }

    fn recv<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, std::io::Result<usize>> {
        Box::pin(async move {
            loop {
                let n = self.flow.recv(buf).await?;
                if !self.blackhole.load(Ordering::SeqCst) {
                    return Ok(n);
                }
            }
        })
    }
}
