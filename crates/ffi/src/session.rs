//! The session layer's API for the hosts (Swift, Kotlin), through UniFFI:
//! `moshpit-session`'s registry behind one object, [`CoreSessions`], and
//! what a platform gives it behind four traits —
//!
//! - [`CredentialHost`]: a connection's password or key from the keychain
//!   (Face ID first, when the item asks for it), and signatures by a
//!   hardware key whose private half never leaves the Secure Enclave
//!   (`crates/ssh`'s signer callback);
//! - [`KeychainHost`] (the store's): the trusted host keys, W5's keychain
//!   item `moshpit.knownHosts`, the same one the Swift transport reads;
//! - [`PreferencesHost`] (the store's): what a session remembers between
//!   attempts and launches — where the person was in tmux, what the host
//!   could run — under the app's own keys (`moshpit_store::session_memory`);
//! - [`SessionEventSink`]: every session's events, pumped by one task into
//!   the sink in the order the sessions produced them, a batch at a time.
//!
//! Every method returns at once; the work runs on the core's runtime
//! (`moshpit-runtime`). What takes a while answers as an event: a
//! disconnect's teardown ([`SessionEventKind::Closed`]), a flush on the way
//! to the background ([`SessionEventKind::Flushed`]), a herdr request
//! ([`SessionEventKind::HerdrAnswer`]), a delivery
//! ([`SessionEventKind::Delivered`]). Borrowing the session's SSH for what
//! runs beside the terminal is async ([`CoreSessions::borrow_ssh`],
//! `crate::borrow`).
//!
//! Nothing here is a sentence for a person (B202): failures, notices and
//! outcomes cross as kinds with their fields, and the host words them in
//! the person's language. The `log` fields are English for the log only.
//!
//! Not a hot path: these are called per person's action and per state
//! change. A pane's bytes go through its byte channel's C ABI
//! (`crates/stream/include/moshpit_stream.h`), and what its view must do
//! between two bytes rides in that channel as a mark
//! (`moshpit_session::view_marks`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use moshpit_install::{HostCapabilities, Multiplexer};
use moshpit_session::{
    self as core, Cause, ConnectionSpec, CredentialError, Credentials, CredentialsWhy, Degrade,
    Discarded, EndReason, EventKind, Failure, FailureKind, KeyWhy, MoshWhy, PaneInfo, Protocol,
    ProxyWhy, ResumeOutcome, SessionMemory, SessionState, Sessions, Timing, TmuxAction, TmuxView,
    Transport, UdpFlow, UdpNetwork, UnreachableWhy,
};
use moshpit_ssh::{
    Auth, BoxFuture, HardwareKey, HostSignature, KeySigner, KnownHosts, LinkFault, SignerError,
    Socks5Proxy, Target,
};
use moshpit_store::known_hosts::KeychainKnownHosts;
use moshpit_store::session_memory::{self, TmuxSelection};
use moshpit_store::{Defaults, Readiness};
use moshpit_tmux::model::Selection;
use tokio::sync::mpsc;

use crate::store::{HostDefaults, HostKeychain, KeychainHost, PreferencesHost};

// MARK: - What the platform gives

/// How to sign in to a connection, as the keychain holds it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Credential {
    Password {
        password: String,
    },
    /// An unencrypted `openssh-key-v1` private key, as text (the app
    /// converts every other format when the key is first used).
    Key {
        openssh: String,
    },
    /// A key whose private half stays in the hardware (the Secure Enclave):
    /// its public half as an `authorized_keys` line, and what the platform
    /// signs with it by ([`CredentialHost::sign`]'s `key_ref`).
    Hardware {
        public_key: String,
        key_ref: String,
    },
}

/// A signature as the platform makes it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PlatformSignature {
    /// ECDSA as ASN.1 DER (CryptoKit's `derRepresentation`).
    Der { bytes: Vec<u8> },
    /// Fixed width: ECDSA `r ‖ s`, or Ed25519's 64 bytes.
    Raw { bytes: Vec<u8> },
}

/// Why the platform has no credential or signature to give.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum CredentialFailure {
    /// The person cancelled (Face ID, a passphrase card).
    Canceled,
    /// The device is locked: the keychain item can't be read until it is
    /// unlocked (a launch before the first unlock). Retried on the unlock
    /// ([`CoreSessions::device_unlocked`]) and the foreground return (B201).
    Locked,
    /// Anything else, for the log; never a secret.
    Unavailable { message: String },
}

impl std::fmt::Display for CredentialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Canceled => f.write_str("authentication was cancelled"),
            Self::Locked => f.write_str("the keychain can't be read while the device is locked"),
            Self::Unavailable { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CredentialFailure {}

impl From<uniffi::UnexpectedUniFFICallbackError> for CredentialFailure {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Unavailable {
            message: error.reason,
        }
    }
}

/// The keychain's side of signing in. Both calls may wait on the person
/// (Face ID, a passphrase card); that time doesn't count against a
/// connection's handshake.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait CredentialHost: Send + Sync {
    /// The credential of connection `connection_id`, read for one attempt.
    async fn credential(&self, connection_id: String) -> Result<Credential, CredentialFailure>;

    /// `data` signed by the hardware key `key_ref` names (ECDSA P-256 with
    /// SHA-256, as the Secure Enclave signs).
    async fn sign(
        &self,
        key_ref: String,
        data: Vec<u8>,
    ) -> Result<PlatformSignature, CredentialFailure>;
}

/// Where the sessions' events go: called on a core thread, one batch at a
/// time, in order; it must return quickly (hop to the UI's thread and go).
#[uniffi::export(with_foreign)]
pub trait SessionEventSink: Send + Sync {
    fn deliver(&self, events: Vec<SessionEvent>);
}

// MARK: - What a connection is

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SessionMultiplexer {
    None,
    Tmux,
    Herdr,
}

/// One saved connection, as far as a session is concerned.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionSpec {
    /// The app's connection id (`ServerConnection.id.uuidString`): what the
    /// session's memory is keyed by.
    pub connection_id: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    /// A SOCKS5 proxy for the SSH leg.
    pub proxy_host: Option<String>,
    pub proxy_port: Option<u16>,
    pub mosh: bool,
    pub multiplexer: SessionMultiplexer,
    pub tmux_path: Option<String>,
    pub mosh_server_path: Option<String>,
    pub mosh_port_low: Option<i64>,
    pub mosh_port_high: Option<i64>,
    pub herdr_path: Option<String>,
    /// The locale mosh-server starts the shell in.
    pub locale: String,
    /// The terminal's grid before the view has reported one.
    pub cols: u16,
    pub rows: u16,
}

impl SessionSpec {
    fn core(&self) -> ConnectionSpec {
        let mut spec = ConnectionSpec::ssh(
            self.connection_id.clone(),
            Target::new(self.user.clone(), self.host.clone(), self.port),
        );
        spec.proxy = self
            .proxy_host
            .as_ref()
            .filter(|host| !host.is_empty())
            .map(|host| Socks5Proxy::new(host.clone(), self.proxy_port.unwrap_or(1080)));
        spec.protocol = if self.mosh {
            Protocol::Mosh
        } else {
            Protocol::Ssh
        };
        spec.multiplexer = match self.multiplexer {
            SessionMultiplexer::None => Multiplexer::None,
            SessionMultiplexer::Tmux => Multiplexer::Tmux,
            SessionMultiplexer::Herdr => Multiplexer::Herdr,
        };
        let path = |p: &Option<String>| p.clone().filter(|p| !p.trim().is_empty());
        spec.tmux_path = path(&self.tmux_path);
        spec.mosh_server_path = path(&self.mosh_server_path);
        spec.herdr_path = path(&self.herdr_path);
        spec.mosh_port_range = (self.mosh_port_low, self.mosh_port_high);
        if !self.locale.is_empty() {
            spec.locale = self.locale.clone();
        }
        spec.initial_grid = (self.cols.max(1), self.rows.max(1));
        spec
    }
}

// MARK: - What happens

/// One thing that happened to a session (`session` 0: to all of them).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionEvent {
    pub session: u64,
    pub kind: SessionEventKind,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SessionEventKind {
    State {
        state: SessionStateValue,
    },
    /// The panes with terminals for attach number `attach`; keep showing
    /// the previous attach's views until [`SessionEventKind::FirstFrame`]
    /// for this one.
    Panes {
        attach: u64,
        panes: Vec<PaneValue>,
    },
    FirstFrame {
        attach: u64,
    },
    /// tmux's tree, or what the chrome draws around it, changed:
    /// [`CoreSessions::tmux_view`].
    TreeChanged,
    PaneActivity {
        pane: String,
    },
    PaneBell {
        pane: String,
    },
    AgentHooksUpdated,
    AttachStalled,
    TmuxActionsDropped,
    HostKeyPrompt {
        prompt: u64,
        host: String,
        port: u16,
        fingerprint: String,
        /// The trusted fingerprint, when the key CHANGED.
        previous: Option<String>,
    },
    HostKeyPromptWithdrawn {
        prompt: u64,
    },
    Degraded {
        degrade: DegradeValue,
    },
    ReturnPathDead,
    ReturnPathAlive,
    MoshMetrics {
        srtt_ms: f64,
        roaming: bool,
    },
    /// mosh's protocol counters (the transport pill's long-press popover),
    /// at most twice a second while they move.
    MoshDiagnostics {
        datagrams_received: u64,
        parse_failures: u64,
        gap_events: u64,
        applied_host_num: u64,
        packets_sent: u64,
        last_user_num: u64,
        server_acked_user_num: u64,
        /// The UDP flow's state: `setup`, `preparing`, `waiting`, `ready`,
        /// `failed` or `cancelled`.
        channel_state: String,
        input_held: bool,
    },
    InputDiscarded {
        stream: u64,
        bytes: u64,
        why: DiscardedValue,
    },
    /// The way back from the background asked the connection that was live
    /// when the app left, and heard (resume in place): what the device's
    /// diagnostics log.
    ResumeChecked {
        away_ms: u64,
        /// 0 when there was nothing to ask (the connection was gone before
        /// the return).
        waited_ms: u64,
        outcome: ResumeOutcomeValue,
        /// Asked on tmux's control channel (else the SSH keepalive, or
        /// nothing).
        through_tmux: bool,
        /// Writes typed meanwhile: sent when kept, dropped otherwise.
        held: u32,
    },
    /// [`CoreSessions::disconnect`] has torn the session down.
    Closed,
    /// [`CoreSessions::flush`]'s answer.
    Flushed {
        token: u64,
    },
    /// What a herdr request ([`CoreSessions::herdr`]) came back with.
    HerdrAnswer {
        token: u64,
        answer: crate::herdr::HerdrAnswerValue,
    },
    /// [`CoreSessions::deliver`]'s answer: whether the content went out.
    Delivered {
        token: u64,
        delivered: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SessionStateValue {
    Connecting,
    Connected,
    Reconnecting,
    Ended {
        reason: EndValue,
    },
    Failed {
        kind: FailureKindValue,
        /// What the host words its card from.
        cause: FailureCauseValue,
        /// English, for the log only.
        log: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum EndValue {
    ShellExited { status: Option<u32> },
    MoshServerEnded,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FailureKindValue {
    Unreachable,
    Handshake,
    HostKey,
    HostKeysLocked,
    Credentials,
    CredentialsLocked,
    Proxy,
    Channel,
    MoshBootstrap,
    Lost,
}

/// Why nothing answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnreachableValue {
    Refused,
    TimedOut,
    NoAddress,
    NoRoute,
    Other,
}

/// Why the SOCKS5 proxy didn't carry the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProxyFailureValue {
    Unreachable,
    NeedsPassword,
    NotSocks5,
    Refused { code: u8 },
    NameTooLong,
    Closed,
}

/// Why a saved key can't sign in.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyWhyValue {
    NotOpenSsh,
    Encrypted,
    Malformed,
    Unsupported { algorithm: String },
    BadPublicKey,
}

/// Why signing in didn't happen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CredentialsWhyValue {
    Rejected { user: String },
    MethodRejected { method: String },
    KeyUnusable { why: KeyWhyValue },
    SignerFailed,
    Unavailable,
    Cancelled,
    Locked,
    InteractivePrompts,
}

/// Why mosh-server gave no session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MoshWhyValue {
    TimedOut { seconds: u64 },
    Unreadable,
}

/// The structured cause of a failure (B202): what happened, for the host to
/// say in the person's language.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FailureCauseValue {
    Plain,
    Unreachable {
        host: String,
        port: u16,
        why: UnreachableValue,
    },
    Proxy {
        proxy: String,
        why: ProxyFailureValue,
    },
    Handshake {
        host: String,
        port: u16,
        timed_out: bool,
    },
    HostKey {
        host: String,
        port: u16,
        changed: bool,
    },
    HostKeysLocked {
        host: String,
        port: u16,
    },
    Credentials {
        why: CredentialsWhyValue,
    },
    Channel {
        timed_out: bool,
    },
    MoshBootstrap {
        why: MoshWhyValue,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DegradeValue {
    MoshServerMissing,
    TmuxMissing,
    HerdrMissing,
    HerdrUnavailable,
}

/// What the way back from the background found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ResumeOutcomeValue {
    /// It answered: kept.
    Kept,
    /// No answer by the deadline: redialled.
    Silent,
    /// Closed or reset, before the answer or already before the return:
    /// redialled.
    Dead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiscardedValue {
    NotLive,
    PaneGone,
    SessionEnded,
    /// Typed at a herdr pane whose attach isn't reading yet (B185).
    NotAttached,
    /// Typed at a plain shell while the way back found its connection dead
    /// (B396).
    Reconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransportValue {
    Ssh,
    Mosh,
}

/// A pane with a terminal: what a view attaches to.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PaneValue {
    /// tmux's `%N`, herdr's pane id, or `shell` for a session without a
    /// multiplexer (and mosh, whose screen is the shell's).
    pub pane: String,
    /// The byte channel's id.
    pub stream: u64,
    pub active: bool,
}

/// What the UI reads about a session at any time.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionReadModel {
    pub state: SessionStateValue,
    pub attach: u64,
    pub panes: Vec<PaneValue>,
    pub degrade: Option<DegradeValue>,
    pub transport: Option<TransportValue>,
    pub return_path_dead: bool,
    pub attach_stalled: bool,
    pub mosh_srtt_ms: Option<f64>,
    pub mosh_roaming: bool,
    pub mosh_server_pid: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TmuxSessionValue {
    pub id: String,
    pub name: String,
    pub is_attached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TmuxWindowValue {
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub index: i64,
    pub layout: String,
    pub is_active: bool,
    pub pane_count: i64,
    pub is_zoomed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TmuxPaneValue {
    pub id: String,
    pub window_id: String,
    pub index: i64,
    pub command: String,
    /// The working directory of what runs in the pane (tmux's
    /// `#{pane_current_path}`, herdr's pane cwd); `None` when unknown.
    #[uniffi(default = None)]
    pub path: Option<String>,
    pub width: i64,
    pub height: i64,
    pub is_active: bool,
}

/// One pane's agent stamp.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentHookValue {
    pub pane: String,
    pub state: Option<String>,
    pub agent: Option<String>,
    /// Unix seconds.
    pub since: Option<f64>,
    pub command: Option<String>,
    pub title: Option<String>,
}

/// tmux as the terminal chrome draws it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TmuxReadModel {
    pub sessions: Vec<TmuxSessionValue>,
    pub windows: Vec<TmuxWindowValue>,
    pub panes: Vec<TmuxPaneValue>,
    pub active_session_id: Option<String>,
    pub active_window_id: Option<String>,
    pub active_pane_id: Option<String>,
    pub is_attached: bool,
    pub ever_attached: bool,
    pub last_switch_forward: bool,
    pub has_swiped_since_attach: bool,
    /// A tmux command's complaint, for a few seconds.
    pub notice: Option<String>,
    pub refreshing: bool,
    /// The size-lease follower's veil: the active window is sized for
    /// another device (`size_follow_window`, its `size_follow_owner`).
    pub size_follow_window: Option<String>,
    pub size_follow_owner: Option<String>,
    pub agent_hooks: Vec<AgentHookValue>,
    pub agent_hooks_loaded: bool,
    /// The active pane's program wants the mouse / is on the alternate
    /// screen (unknown: discovery's `alternate_on`): where a scroll goes.
    pub wants_mouse: bool,
    pub alt_screen: bool,
    /// The program scrolls itself: a scroll is always its wheel (an
    /// alternate-screen mouse app, Claude with the mouse; B363).
    pub scrolls_itself: bool,
    /// Claude on the primary screen with no flags read yet: a scroll is
    /// dropped rather than guessed (B363).
    pub holds_ticks: bool,
    /// The server has no session at all, asked over exec once the attach
    /// would have stalled (B228, `TmuxView::no_session`): it waits for the
    /// person's first, made from the empty state. Until something
    /// attaches.
    pub no_session: bool,
}

/// What the person (or the UI) asks of tmux.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum TmuxIntent {
    SelectSession {
        id: String,
    },
    SelectWindow {
        id: String,
    },
    SelectPane {
        id: String,
    },
    SwitchPaneOrWindow {
        forward: bool,
    },
    NewWindow {
        name: Option<String>,
    },
    NewWindowInSession {
        session: String,
        name: Option<String>,
    },
    NewPane,
    NewPaneInWindow {
        window: String,
    },
    NewSession {
        name: Option<String>,
    },
    RenameSession {
        session: String,
        name: String,
    },
    KillSession {
        id: String,
    },
    RenameWindow {
        window: String,
        name: String,
    },
    KillWindow {
        id: String,
    },
    KillPane {
        id: String,
    },
    Refresh,
    TakeSizeOwnership,
    Paste {
        pane: String,
        text: String,
    },
    ScrollLines {
        lines: i64,
    },
    Click {
        col: i64,
        row: i64,
    },
    ViewRevealed {
        pane: String,
    },
    ReturnSent {
        pane: String,
    },
    InputModeChanged {
        pane: String,
    },
    ScrollBegan,
    PollAgentHooks,
    DeferResize {
        pane: String,
        cols: u16,
        rows: u16,
    },
}

/// Why the core refused what was asked of tmux, before sending anything:
/// the host says it in the person's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum TmuxRefused {
    /// A name typed for a session or a window holds a line break, a tab or
    /// another control character (B139, B250).
    NameHasControlCharacters,
}

impl std::fmt::Display for TmuxRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameHasControlCharacters => f.write_str("a name with a control character"),
        }
    }
}

impl std::error::Error for TmuxRefused {}

impl From<core::TmuxRefusal> for TmuxRefused {
    fn from(refusal: core::TmuxRefusal) -> Self {
        match refusal {
            core::TmuxRefusal::NameHasControlCharacters => Self::NameHasControlCharacters,
        }
    }
}

/// A broken network, injected by a debug build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InjectedFault {
    /// The session's SSH link reset.
    CutTcp,
    /// The session's SSH link silent for good.
    StallTcp,
    /// Every mosh datagram dropped, both ways, until turned off.
    MoshBlackhole { on: bool },
}

// MARK: - The registry

/// Every session the app has, on the core's runtime.
#[derive(uniffi::Object)]
pub struct CoreSessions {
    sessions: Sessions,
    out: mpsc::UnboundedSender<SessionEvent>,
    tokens: AtomicU64,
    blackhole: Arc<AtomicBool>,
    /// The trusted host keys the sessions decide by: Home's marks are made
    /// from them (a host's key's face).
    known_hosts: Arc<KnownHosts>,
}

impl CoreSessions {
    /// The core's registry, for the crate's other entry points over it
    /// (Home's record, `home`).
    pub(crate) fn registry(&self) -> &Sessions {
        &self.sessions
    }

    /// The fingerprint trusted for `host` on `port`, if any.
    pub(crate) fn trusted(&self, host: &str, port: u16) -> Option<String> {
        self.known_hosts.trusted(host, port)
    }
}

#[uniffi::export]
impl CoreSessions {
    /// The registry, with the app in the foreground. `device_label` is what
    /// tmux's size leases call this device ("iPhone", "iPad").
    #[uniffi::constructor]
    pub fn new(
        credentials: Arc<dyn CredentialHost>,
        keychain: Arc<dyn KeychainHost>,
        preferences: Arc<dyn PreferencesHost>,
        events: Arc<dyn SessionEventSink>,
        device_label: String,
    ) -> Arc<Self> {
        let runtime = moshpit_runtime::handle();
        let blackhole = Arc::new(AtomicBool::new(false));
        // The sealed-launch rule for the trusted keys (B226): "no item" read
        // before the first unlock is not an empty map.
        let unlocked = HostDefaults(preferences.clone());
        let known_hosts = Arc::new(KnownHosts::new(Arc::new(
            KeychainKnownHosts::new(Arc::new(HostKeychain(keychain)))
                .with_unlock_probe(Arc::new(move || unlocked.protected_data_available())),
        )));
        let mut services =
            core::Services::new(Arc::new(HostCredentials(credentials)), known_hosts.clone());
        let timing = app_timing();
        services.memory = Arc::new(StoreMemory {
            defaults: Arc::new(HostDefaults(preferences)),
            readiness: Readiness::System,
        });
        services.udp = Arc::new(FaultableUdp {
            blackhole: blackhole.clone(),
        });
        if !device_label.is_empty() {
            services.device_label = device_label;
        }
        let (sessions, mut core_events) = Sessions::new(runtime.clone(), services, timing);
        let (out, mut pending) = mpsc::unbounded_channel::<SessionEvent>();
        let forward = out.clone();
        runtime.spawn(async move {
            while let Some(event) = core_events.recv().await {
                if let Some(event) = event_value(event)
                    && forward.send(event).is_err()
                {
                    return;
                }
            }
        });
        runtime.spawn(async move {
            while let Some(first) = pending.recv().await {
                let mut batch = vec![first];
                while batch.len() < 256 {
                    match pending.try_recv() {
                        Ok(next) => batch.push(next),
                        Err(_) => break,
                    }
                }
                events.deliver(batch);
            }
        });
        Arc::new(Self {
            sessions,
            out,
            tokens: AtomicU64::new(0),
            blackhole,
            known_hosts,
        })
    }

    /// Opens a session to `spec` and connects it; its id.
    pub fn connect(&self, spec: SessionSpec) -> u64 {
        self.sessions.connect(spec.core())
    }

    /// The person's "try again".
    pub fn reconnect(&self, session: u64) {
        self.sessions.reconnect(session);
    }

    /// The session's connection was saved while it runs (B329): its next
    /// connect goes to `spec`'s address, user and proxy.
    pub fn respec(&self, session: u64, spec: SessionSpec) {
        self.sessions.respec(session, &spec.core());
    }

    /// Closes a session for good; [`SessionEventKind::Closed`] once it is
    /// torn down (tmux's pins handed back, a mosh-server stopped).
    pub fn disconnect(&self, session: u64) {
        let done = self.sessions.disconnect(session);
        let out = self.out.clone();
        moshpit_runtime::handle().spawn(async move {
            let _ = done.await;
            let _ = out.send(SessionEvent {
                session,
                kind: SessionEventKind::Closed,
            });
        });
    }

    pub fn switch_to_ssh(&self, session: u64) {
        self.sessions.switch_to_ssh(session);
    }

    /// The mosh diagnostics' Repaint: mosh-server redraws the whole screen
    /// (B338).
    pub fn repaint(&self, session: u64) {
        self.sessions.repaint(session);
    }

    pub fn answer_host_key(&self, session: u64, prompt: u64, trust: bool) {
        self.sessions.answer_host_key(session, prompt, trust);
    }

    /// Something for the session's tmux to do; refused at once, with
    /// nothing sent, when a name in it can't be used as typed.
    pub fn tmux(&self, session: u64, intent: TmuxIntent) -> Result<(), TmuxRefused> {
        self.sessions
            .tmux(session, intent.core())
            .map_err(TmuxRefused::from)
    }

    /// Something for the session's herdr to do; its answer comes as
    /// [`SessionEventKind::HerdrAnswer`] with the token this returns (every
    /// intent is answered once, the ones with nothing to report with
    /// `Done`).
    pub fn herdr(&self, session: u64, intent: crate::herdr::HerdrIntent) -> u64 {
        self.sessions.herdr(session, intent.core())
    }

    /// The New agent task form went away while the task asked for under
    /// `token` was starting.
    pub fn herdr_cancel_task(&self, session: u64, token: u64) {
        self.sessions.herdr_cancel_task(session, token);
    }

    /// herdr's tree and what is said around it, when herdr runs.
    pub fn herdr_view(&self, session: u64) -> Option<crate::herdr::HerdrReadModel> {
        self.sessions
            .herdr_view(session)
            .map(|view| crate::herdr::read_model(&view))
    }

    /// A paste into `pane` (the terminal on screen): one bracketed paste
    /// tmux and herdr fit to the pane's real mode. Dropped while not live.
    pub fn paste(&self, session: u64, pane: String, text: String) {
        self.sessions.paste(session, &pane, text);
    }

    /// Content for one pane on purpose: a notification's reply, a share.
    /// Waits out a reconnect (bounded), and herdr switches to the pane and
    /// holds it for the pane's first frame; [`SessionEventKind::Delivered`]
    /// with the token this returns says whether it went out.
    pub fn deliver(&self, session: u64, pane: String, bytes: Vec<u8>, paste: bool) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::Relaxed) + 1;
        let delivered = self.sessions.deliver(session, &pane, bytes, paste);
        let out = self.out.clone();
        moshpit_runtime::handle().spawn(async move {
            let delivered = delivered.await.unwrap_or(false);
            let _ = out.send(SessionEvent {
                session,
                kind: SessionEventKind::Delivered { token, delivered },
            });
        });
        token
    }

    /// The device was unlocked: sessions that failed on a locked keychain
    /// dial again (B201).
    pub fn device_unlocked(&self) {
        self.sessions.device_unlocked();
    }

    /// The session's SSH, lent to what runs beside its terminal (a file
    /// preview, an upload, file transfer, host care): exec and SFTP
    /// channels on the connection it already has (`crate::borrow`).
    pub async fn borrow_ssh(
        &self,
        session: u64,
    ) -> Result<Arc<crate::borrow::BorrowedSsh>, crate::borrow::BorrowFailure> {
        crate::borrow::borrow(self.sessions.clone(), session).await
    }

    /// The app came to the foreground (`true`) or left it.
    pub fn set_foreground(&self, foreground: bool) {
        self.sessions.set_foreground(foreground);
    }

    /// Asks every session to say when what it was asked to send its server
    /// so far has gone out (bounded): [`SessionEventKind::Flushed`] with the
    /// token this returns.
    pub fn flush(&self) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::Relaxed) + 1;
        let done = self.sessions.flushed();
        let out = self.out.clone();
        moshpit_runtime::handle().spawn(async move {
            let _ = done.await;
            let _ = out.send(SessionEvent {
                session: 0,
                kind: SessionEventKind::Flushed { token },
            });
        });
        token
    }

    /// The device's network path changed.
    pub fn network_changed(&self) {
        self.sessions.network_changed();
    }

    /// The session whose terminal is on screen (`None` on Home).
    pub fn set_visible(&self, session: Option<u64>) {
        self.sessions.set_visible(session);
    }

    pub fn view(&self, session: u64) -> Option<SessionReadModel> {
        self.sessions.view(session).map(|view| SessionReadModel {
            state: state_value(&view.state),
            attach: view.attach,
            panes: view.panes.iter().map(pane_value).collect(),
            degrade: view.degrade.map(degrade_value),
            transport: view.transport.map(|t| match t {
                Transport::Ssh => TransportValue::Ssh,
                Transport::Mosh => TransportValue::Mosh,
            }),
            return_path_dead: view.return_path_dead,
            attach_stalled: view.attach_stalled,
            mosh_srtt_ms: view.mosh_srtt_ms,
            mosh_roaming: view.mosh_roaming,
            mosh_server_pid: view.mosh_server_pid,
        })
    }

    pub fn tmux_view(&self, session: u64) -> Option<TmuxReadModel> {
        self.sessions
            .tmux_view(session)
            .map(|view| tmux_value(&view))
    }

    /// A debug build's broken network: the session reconnects as it would
    /// after the real thing.
    pub fn inject_fault(&self, session: u64, fault: InjectedFault) {
        match fault {
            InjectedFault::CutTcp => self.sessions.inject_fault(session, LinkFault::Cut),
            InjectedFault::StallTcp => self.sessions.inject_fault(session, LinkFault::Stall),
            InjectedFault::MoshBlackhole { on } => self.blackhole.store(on, Ordering::SeqCst),
        }
    }
}

// MARK: - The platform behind the core's traits

struct HostCredentials(Arc<dyn CredentialHost>);

impl Credentials for HostCredentials {
    fn auth<'a>(&'a self, connection_id: &'a str) -> BoxFuture<'a, Result<Auth, CredentialError>> {
        Box::pin(async move {
            let credential = self.0.credential(connection_id.to_owned()).await.map_err(
                |failure| match failure {
                    CredentialFailure::Canceled => CredentialError::Cancelled,
                    CredentialFailure::Locked => CredentialError::Locked,
                    CredentialFailure::Unavailable { message } => {
                        CredentialError::Unavailable(message)
                    }
                },
            )?;
            Ok(match credential {
                Credential::Password { password } => Auth::password(password),
                Credential::Key { openssh } => Auth::key(openssh),
                Credential::Hardware {
                    public_key,
                    key_ref,
                } => {
                    let signer: Arc<dyn KeySigner> = Arc::new(HostSigner(self.0.clone()));
                    Auth::Hardware(
                        HardwareKey::new(&public_key, key_ref, signer)
                            .map_err(|error| CredentialError::Unavailable(error.to_string()))?,
                    )
                }
            })
        })
    }
}

struct HostSigner(Arc<dyn CredentialHost>);

impl KeySigner for HostSigner {
    fn sign<'a>(
        &'a self,
        key_ref: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<HostSignature, SignerError>> {
        Box::pin(async move {
            match self.0.sign(key_ref.to_owned(), data.to_vec()).await {
                Ok(PlatformSignature::Der { bytes }) => Ok(HostSignature::Der(bytes)),
                Ok(PlatformSignature::Raw { bytes }) => Ok(HostSignature::Raw(bytes)),
                Err(failure) => Err(signer_error(failure)),
            }
        })
    }
}

/// What the platform's signer said, as the SSH layer reads it (B227): a
/// key the locked device refused is the lock — the sign-in is tried again on
/// the unlock, as a keychain read refused for the lock is (B211) — and a
/// cancelled Face ID the person's. Anything else failed the signature.
fn signer_error(failure: CredentialFailure) -> SignerError {
    match failure {
        CredentialFailure::Locked => SignerError::Locked,
        CredentialFailure::Canceled => SignerError::Cancelled,
        CredentialFailure::Unavailable { message } => SignerError::Failed(message),
    }
}

/// What a session remembers, in the app's preferences under the app's keys.
struct StoreMemory {
    defaults: Arc<dyn Defaults>,
    readiness: Readiness,
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
            &self.readiness,
            &session_memory::tmux_selection_key(connection_id),
            &stored,
        );
    }

    fn capabilities(&self, connection_id: &str) -> Option<HostCapabilities> {
        session_memory::read(
            self.defaults.as_ref(),
            &session_memory::host_capabilities_key(connection_id),
        )
    }

    fn save_capabilities(&self, connection_id: &str, capabilities: &HostCapabilities) {
        session_memory::write(
            self.defaults.as_ref(),
            &self.readiness,
            &session_memory::host_capabilities_key(connection_id),
            capabilities,
        );
    }
}

/// The system's UDP, which a debug build can blackhole (both directions)
/// to stand in for a network that stopped carrying mosh.
struct FaultableUdp {
    blackhole: Arc<AtomicBool>,
}

impl UdpNetwork for FaultableUdp {
    fn open(&self, server: SocketAddr) -> BoxFuture<'static, std::io::Result<Box<dyn UdpFlow>>> {
        let blackhole = self.blackhole.clone();
        Box::pin(async move {
            let flow = core::SystemUdp.open(server).await?;
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

// MARK: - Conversions

impl TmuxIntent {
    fn core(self) -> TmuxAction {
        match self {
            TmuxIntent::SelectSession { id } => TmuxAction::SelectSession(id),
            TmuxIntent::SelectWindow { id } => TmuxAction::SelectWindow(id),
            TmuxIntent::SelectPane { id } => TmuxAction::SelectPane(id),
            TmuxIntent::SwitchPaneOrWindow { forward } => {
                TmuxAction::SwitchPaneOrWindow { forward }
            }
            TmuxIntent::NewWindow { name } => TmuxAction::NewWindow { name },
            TmuxIntent::NewWindowInSession { session, name } => {
                TmuxAction::NewWindowInSession { session, name }
            }
            TmuxIntent::NewPane => TmuxAction::NewPane,
            TmuxIntent::NewPaneInWindow { window } => TmuxAction::NewPaneInWindow(window),
            TmuxIntent::NewSession { name } => TmuxAction::NewSession { name },
            TmuxIntent::RenameSession { session, name } => {
                TmuxAction::RenameSession { session, name }
            }
            TmuxIntent::KillSession { id } => TmuxAction::KillSession(id),
            TmuxIntent::RenameWindow { window, name } => TmuxAction::RenameWindow { window, name },
            TmuxIntent::KillWindow { id } => TmuxAction::KillWindow(id),
            TmuxIntent::KillPane { id } => TmuxAction::KillPane(id),
            TmuxIntent::Refresh => TmuxAction::Refresh,
            TmuxIntent::TakeSizeOwnership => TmuxAction::TakeSizeOwnership,
            TmuxIntent::Paste { pane, text } => TmuxAction::Paste { pane, text },
            TmuxIntent::ScrollLines { lines } => TmuxAction::ScrollLines(lines),
            TmuxIntent::Click { col, row } => TmuxAction::Click { col, row },
            TmuxIntent::ViewRevealed { pane } => TmuxAction::ViewRevealed { pane },
            TmuxIntent::ReturnSent { pane } => TmuxAction::ReturnSent { pane },
            TmuxIntent::InputModeChanged { pane } => TmuxAction::InputModeChanged { pane },
            TmuxIntent::ScrollBegan => TmuxAction::ScrollBegan,
            TmuxIntent::PollAgentHooks => TmuxAction::PollAgentHooks,
            TmuxIntent::DeferResize { pane, cols, rows } => {
                TmuxAction::DeferResize { pane, cols, rows }
            }
        }
    }
}

/// The event as the host sees it; `None` for what rides elsewhere (a
/// pane view's actions are marks in its byte channel).
fn event_value(event: core::Event) -> Option<SessionEvent> {
    let kind = match event.kind {
        EventKind::State(state) => SessionEventKind::State {
            state: state_value(&state),
        },
        EventKind::Panes { attach, panes } => SessionEventKind::Panes {
            attach,
            panes: panes.iter().map(pane_value).collect(),
        },
        EventKind::FirstFrame { attach } => SessionEventKind::FirstFrame { attach },
        EventKind::View { .. } => return None,
        EventKind::TreeChanged => SessionEventKind::TreeChanged,
        EventKind::PaneActivity { pane } => SessionEventKind::PaneActivity { pane },
        EventKind::PaneBell { pane } => SessionEventKind::PaneBell { pane },
        EventKind::AgentHooksUpdated => SessionEventKind::AgentHooksUpdated,
        EventKind::AttachStalled => SessionEventKind::AttachStalled,
        EventKind::TmuxActionsDropped => SessionEventKind::TmuxActionsDropped,
        EventKind::HostKeyPrompt {
            prompt,
            host,
            port,
            fingerprint,
            previous,
        } => SessionEventKind::HostKeyPrompt {
            prompt,
            host,
            port,
            fingerprint,
            previous,
        },
        EventKind::HostKeyPromptWithdrawn { prompt } => {
            SessionEventKind::HostKeyPromptWithdrawn { prompt }
        }
        EventKind::Degraded(degrade) => SessionEventKind::Degraded {
            degrade: degrade_value(degrade),
        },
        EventKind::ReturnPathDead => SessionEventKind::ReturnPathDead,
        EventKind::ReturnPathAlive => SessionEventKind::ReturnPathAlive,
        EventKind::MoshMetrics { srtt_ms, roaming } => {
            SessionEventKind::MoshMetrics { srtt_ms, roaming }
        }
        EventKind::MoshDiagnostics(d) => SessionEventKind::MoshDiagnostics {
            datagrams_received: d.datagrams_received,
            parse_failures: d.parse_failures,
            gap_events: d.gap_events,
            applied_host_num: d.applied_host_num,
            packets_sent: d.packets_sent,
            last_user_num: d.last_user_num,
            server_acked_user_num: d.server_acked_user_num,
            channel_state: match d.channel_state {
                moshpit_session::MoshChannelState::Setup => "setup",
                moshpit_session::MoshChannelState::Preparing => "preparing",
                moshpit_session::MoshChannelState::Waiting => "waiting",
                moshpit_session::MoshChannelState::Ready => "ready",
                moshpit_session::MoshChannelState::Failed => "failed",
                moshpit_session::MoshChannelState::Cancelled => "cancelled",
            }
            .to_owned(),
            input_held: d.input_held,
        },
        EventKind::ResumeChecked {
            away_ms,
            waited_ms,
            outcome,
            through_tmux,
            held,
        } => SessionEventKind::ResumeChecked {
            away_ms,
            waited_ms,
            outcome: match outcome {
                ResumeOutcome::Kept => ResumeOutcomeValue::Kept,
                ResumeOutcome::Silent => ResumeOutcomeValue::Silent,
                ResumeOutcome::Dead => ResumeOutcomeValue::Dead,
            },
            through_tmux,
            held: u32::try_from(held).unwrap_or(u32::MAX),
        },
        EventKind::InputDiscarded { stream, bytes, why } => SessionEventKind::InputDiscarded {
            stream,
            bytes: bytes as u64,
            why: match why {
                Discarded::NotLive => DiscardedValue::NotLive,
                Discarded::PaneGone => DiscardedValue::PaneGone,
                Discarded::SessionEnded => DiscardedValue::SessionEnded,
                Discarded::NotAttached => DiscardedValue::NotAttached,
                Discarded::Reconnected => DiscardedValue::Reconnected,
            },
        },
        EventKind::HerdrAnswer { token, answer } => SessionEventKind::HerdrAnswer {
            token,
            answer: crate::herdr::answer_value(answer),
        },
    };
    Some(SessionEvent {
        session: event.session,
        kind,
    })
}

fn state_value(state: &SessionState) -> SessionStateValue {
    match state {
        SessionState::Connecting => SessionStateValue::Connecting,
        SessionState::Connected => SessionStateValue::Connected,
        SessionState::Reconnecting => SessionStateValue::Reconnecting,
        SessionState::Ended(reason) => SessionStateValue::Ended {
            reason: match reason {
                EndReason::ShellExited { status } => EndValue::ShellExited { status: *status },
                EndReason::MoshServerEnded => EndValue::MoshServerEnded,
                EndReason::Disconnected => EndValue::Disconnected,
            },
        },
        SessionState::Failed(failure) => failure_value(failure),
    }
}

fn failure_value(failure: &Failure) -> SessionStateValue {
    SessionStateValue::Failed {
        kind: match failure.kind {
            FailureKind::Unreachable => FailureKindValue::Unreachable,
            FailureKind::Handshake => FailureKindValue::Handshake,
            FailureKind::HostKey => FailureKindValue::HostKey,
            FailureKind::HostKeysLocked => FailureKindValue::HostKeysLocked,
            FailureKind::Credentials => FailureKindValue::Credentials,
            FailureKind::CredentialsLocked => FailureKindValue::CredentialsLocked,
            FailureKind::Proxy => FailureKindValue::Proxy,
            FailureKind::Channel => FailureKindValue::Channel,
            FailureKind::MoshBootstrap => FailureKindValue::MoshBootstrap,
            FailureKind::Lost => FailureKindValue::Lost,
        },
        cause: cause_value(&failure.cause),
        log: failure.log.clone(),
        retryable: failure.retryable,
    }
}

pub(crate) fn cause_value(cause: &Cause) -> FailureCauseValue {
    match cause {
        Cause::Plain => FailureCauseValue::Plain,
        Cause::Unreachable { host, port, why } => FailureCauseValue::Unreachable {
            host: host.clone(),
            port: *port,
            why: match why {
                UnreachableWhy::Refused => UnreachableValue::Refused,
                UnreachableWhy::TimedOut => UnreachableValue::TimedOut,
                UnreachableWhy::NoAddress => UnreachableValue::NoAddress,
                UnreachableWhy::NoRoute => UnreachableValue::NoRoute,
                UnreachableWhy::Other => UnreachableValue::Other,
            },
        },
        Cause::Proxy { proxy, why } => FailureCauseValue::Proxy {
            proxy: proxy.clone(),
            why: match why {
                ProxyWhy::Unreachable => ProxyFailureValue::Unreachable,
                ProxyWhy::NeedsPassword => ProxyFailureValue::NeedsPassword,
                ProxyWhy::NotSocks5 => ProxyFailureValue::NotSocks5,
                ProxyWhy::Refused { code } => ProxyFailureValue::Refused { code: *code },
                ProxyWhy::NameTooLong => ProxyFailureValue::NameTooLong,
                ProxyWhy::Closed => ProxyFailureValue::Closed,
            },
        },
        Cause::Handshake {
            host,
            port,
            timed_out,
        } => FailureCauseValue::Handshake {
            host: host.clone(),
            port: *port,
            timed_out: *timed_out,
        },
        Cause::HostKey {
            host,
            port,
            changed,
        } => FailureCauseValue::HostKey {
            host: host.clone(),
            port: *port,
            changed: *changed,
        },
        Cause::HostKeysLocked { host, port } => FailureCauseValue::HostKeysLocked {
            host: host.clone(),
            port: *port,
        },
        Cause::Credentials(why) => FailureCauseValue::Credentials {
            why: match why {
                CredentialsWhy::Rejected { user } => {
                    CredentialsWhyValue::Rejected { user: user.clone() }
                }
                CredentialsWhy::MethodRejected { method } => CredentialsWhyValue::MethodRejected {
                    method: method.clone(),
                },
                CredentialsWhy::KeyUnusable(key) => CredentialsWhyValue::KeyUnusable {
                    why: match key {
                        KeyWhy::NotOpenSsh => KeyWhyValue::NotOpenSsh,
                        KeyWhy::Encrypted => KeyWhyValue::Encrypted,
                        KeyWhy::Malformed => KeyWhyValue::Malformed,
                        KeyWhy::Unsupported { algorithm } => KeyWhyValue::Unsupported {
                            algorithm: algorithm.clone(),
                        },
                        KeyWhy::BadPublicKey => KeyWhyValue::BadPublicKey,
                    },
                },
                CredentialsWhy::InteractivePrompts => CredentialsWhyValue::InteractivePrompts,
                CredentialsWhy::SignerFailed => CredentialsWhyValue::SignerFailed,
                CredentialsWhy::Unavailable => CredentialsWhyValue::Unavailable,
                CredentialsWhy::Cancelled => CredentialsWhyValue::Cancelled,
                CredentialsWhy::Locked => CredentialsWhyValue::Locked,
            },
        },
        Cause::Channel { timed_out } => FailureCauseValue::Channel {
            timed_out: *timed_out,
        },
        Cause::MoshBootstrap(why) => FailureCauseValue::MoshBootstrap {
            why: match why {
                MoshWhy::TimedOut { seconds } => MoshWhyValue::TimedOut { seconds: *seconds },
                MoshWhy::Unreadable => MoshWhyValue::Unreadable,
            },
        },
    }
}

fn degrade_value(degrade: Degrade) -> DegradeValue {
    match degrade {
        Degrade::MoshServerMissing => DegradeValue::MoshServerMissing,
        Degrade::TmuxMissing => DegradeValue::TmuxMissing,
        Degrade::HerdrMissing => DegradeValue::HerdrMissing,
        Degrade::HerdrUnavailable => DegradeValue::HerdrUnavailable,
    }
}

fn pane_value(pane: &PaneInfo) -> PaneValue {
    PaneValue {
        pane: pane.pane.clone(),
        stream: pane.stream,
        active: pane.active,
    }
}

fn tmux_value(view: &TmuxView) -> TmuxReadModel {
    let snapshot = &view.snapshot;
    let mut hooks: Vec<AgentHookValue> = view
        .agent_hooks
        .iter()
        .map(|(pane, hook)| AgentHookValue {
            pane: pane.clone(),
            state: hook.state.clone(),
            agent: hook.agent.clone(),
            since: hook.since,
            command: hook.command.clone(),
            title: hook.title.clone(),
        })
        .collect();
    hooks.sort_by(|a, b| a.pane.cmp(&b.pane));
    TmuxReadModel {
        sessions: snapshot
            .sessions
            .values()
            .map(|s| TmuxSessionValue {
                id: s.id.clone(),
                name: s.name.clone(),
                is_attached: s.is_attached,
            })
            .collect(),
        windows: snapshot
            .windows
            .values()
            .map(|w| TmuxWindowValue {
                id: w.id.clone(),
                session_id: w.session_id.clone(),
                name: w.name.clone(),
                index: w.index,
                layout: w.layout.clone(),
                is_active: w.is_active,
                pane_count: w.pane_count,
                is_zoomed: w.is_zoomed,
            })
            .collect(),
        panes: snapshot
            .panes
            .values()
            .map(|p| TmuxPaneValue {
                id: p.id.clone(),
                window_id: p.window_id.clone(),
                index: p.index,
                command: p.command.clone(),
                path: p.path.clone(),
                width: p.width,
                height: p.height,
                is_active: p.is_active,
            })
            .collect(),
        active_session_id: snapshot.active_session_id.clone(),
        active_window_id: snapshot.active_window_id.clone(),
        active_pane_id: snapshot.active_pane_id.clone(),
        is_attached: snapshot.is_attached,
        ever_attached: snapshot.ever_attached,
        last_switch_forward: snapshot.last_switch_forward,
        has_swiped_since_attach: snapshot.has_swiped_since_attach,
        notice: view.notice.clone(),
        refreshing: view.refreshing,
        size_follow_window: view.size_follow.as_ref().map(|f| f.window_id.clone()),
        size_follow_owner: view.size_follow.as_ref().map(|f| f.owner_label.clone()),
        agent_hooks: hooks,
        agent_hooks_loaded: view.agent_hooks_loaded,
        wants_mouse: view.input.wants_mouse,
        alt_screen: view.input.alt_screen,
        scrolls_itself: view.input.scrolls_itself,
        holds_ticks: view.input.holds_ticks,
        no_session: view.no_session,
    }
}

/// The sessions' clock: the core's (`Timing::default()`), resuming in
/// place on the way back from the background (`Timing::resume_in_place`): a
/// connection that survived the suspension is asked one round trip and kept
/// when it answers, instead of being redialled. Every Canary build ran it
/// behind a switch in the app's preferences; the switch went with the app's
/// Swift paths (F0), so the core decides it alone.
fn app_timing() -> Timing {
    Timing {
        resume_in_place: true,
        ..Timing::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B337: the session layer's word that the server has no session
    /// reaches the app, where the terminal shows the empty state with its
    /// Create over mosh's waiting renderer.
    #[test]
    fn a_tmux_server_with_no_session_says_so_to_the_app() {
        let mut view = TmuxView {
            snapshot: Default::default(),
            notice: None,
            refreshing: false,
            size_follow: None,
            agent_hooks: Default::default(),
            agent_hooks_loaded: true,
            input: Default::default(),
            no_session: true,
        };
        let model = tmux_value(&view);
        assert!(model.no_session);
        assert!(!model.is_attached && model.sessions.is_empty());
        view.no_session = false;
        assert!(!tmux_value(&view).no_session);
    }

    #[test]
    fn the_sessions_resume_in_place() {
        let timing = app_timing();
        assert!(timing.resume_in_place);
        assert_eq!(
            Timing {
                resume_in_place: false,
                ..timing
            },
            Timing::default(),
            "nothing else changes"
        );
    }

    #[test]
    fn what_the_way_back_found_reaches_the_host() {
        for (outcome, value) in [
            (ResumeOutcome::Kept, ResumeOutcomeValue::Kept),
            (ResumeOutcome::Silent, ResumeOutcomeValue::Silent),
            (ResumeOutcome::Dead, ResumeOutcomeValue::Dead),
        ] {
            let event = core::Event {
                session: 4,
                kind: EventKind::ResumeChecked {
                    away_ms: 312_000,
                    waited_ms: 61,
                    outcome,
                    through_tmux: true,
                    held: 2,
                },
            };
            assert_eq!(
                event_value(event),
                Some(SessionEvent {
                    session: 4,
                    kind: SessionEventKind::ResumeChecked {
                        away_ms: 312_000,
                        waited_ms: 61,
                        outcome: value,
                        through_tmux: true,
                        held: 2,
                    }
                })
            );
        }
    }

    #[test]
    fn a_spec_becomes_the_cores() {
        let spec = SessionSpec {
            connection_id: "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11".into(),
            user: "me".into(),
            host: "box.local".into(),
            port: 2222,
            proxy_host: Some("proxy".into()),
            proxy_port: None,
            mosh: true,
            multiplexer: SessionMultiplexer::Tmux,
            tmux_path: Some("  ".into()),
            mosh_server_path: Some("/opt/bin/mosh-server".into()),
            mosh_port_low: Some(60001),
            mosh_port_high: None,
            herdr_path: None,
            locale: String::new(),
            cols: 0,
            rows: 40,
        };
        let core = spec.core();
        assert_eq!(core.connection_id, spec.connection_id);
        assert_eq!(core.target, Target::new("me", "box.local", 2222));
        assert_eq!(core.proxy, Some(Socks5Proxy::new("proxy", 1080)));
        assert_eq!(core.protocol, Protocol::Mosh);
        assert_eq!(core.multiplexer, Multiplexer::Tmux);
        assert_eq!(core.tmux_path, None, "a blank path is no path");
        assert_eq!(
            core.mosh_server_path.as_deref(),
            Some("/opt/bin/mosh-server")
        );
        assert_eq!(core.mosh_port_range, (Some(60001), None));
        assert_eq!(core.locale, "en_US.UTF-8", "the default locale");
        assert_eq!(core.initial_grid, (1, 40));
    }

    #[test]
    fn view_actions_ride_the_byte_channel_not_the_events() {
        let event = core::Event {
            session: 3,
            kind: EventKind::View {
                stream: 9,
                action: moshpit_tmux::controller::ViewAction::Reveal,
            },
        };
        assert_eq!(event_value(event), None);
        let event = core::Event {
            session: 3,
            kind: EventKind::State(SessionState::Failed(Failure::new(
                FailureKind::HostKey,
                Cause::HostKey {
                    host: "box".into(),
                    port: 22,
                    changed: true,
                },
                "refused",
            ))),
        };
        assert_eq!(
            event_value(event),
            Some(SessionEvent {
                session: 3,
                kind: SessionEventKind::State {
                    state: SessionStateValue::Failed {
                        kind: FailureKindValue::HostKey,
                        cause: FailureCauseValue::HostKey {
                            host: "box".into(),
                            port: 22,
                            changed: true
                        },
                        log: "refused".into(),
                        retryable: false
                    }
                }
            })
        );
    }

    /// The platform's keychain, signing: what each failure crosses as.
    struct Keychain(CredentialFailure);

    #[async_trait::async_trait]
    impl CredentialHost for Keychain {
        async fn credential(&self, _: String) -> Result<Credential, CredentialFailure> {
            Err(self.0.clone())
        }

        async fn sign(
            &self,
            _: String,
            _: Vec<u8>,
        ) -> Result<PlatformSignature, CredentialFailure> {
            Err(self.0.clone())
        }
    }

    #[test]
    fn a_secure_enclave_key_the_locked_device_refuses_crosses_as_the_lock() {
        // B227: every failure crossed as SignerError::Failed, and the
        // session read it as SignerFailed, never retried.
        let sign = |failure: CredentialFailure| {
            let signer = HostSigner(Arc::new(Keychain(failure)));
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(signer.sign("moshpit.secret.enclave", b"data"))
        };
        assert_eq!(sign(CredentialFailure::Locked), Err(SignerError::Locked));
        assert_eq!(
            sign(CredentialFailure::Canceled),
            Err(SignerError::Cancelled)
        );
        assert_eq!(
            sign(CredentialFailure::Unavailable {
                message: "no Secure Enclave key".into()
            }),
            Err(SignerError::Failed("no Secure Enclave key".into()))
        );
    }

    #[test]
    fn the_memory_is_the_apps_preferences() {
        let defaults = Arc::new(moshpit_store::MemoryDefaults::new());
        let memory = StoreMemory {
            defaults: defaults.clone(),
            readiness: Readiness::Always,
        };
        let id = "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11";
        assert_eq!(memory.tmux_selection(id), None);
        let selection = Selection {
            session: "$1".into(),
            window: Some("@2".into()),
            pane: None,
        };
        memory.save_tmux_selection(id, &selection);
        assert_eq!(memory.tmux_selection(id), Some(selection));
        let caps = HostCapabilities::unknown();
        memory.save_capabilities(id, &caps);
        assert_eq!(memory.capabilities(id), Some(caps));
        assert!(defaults.get(&format!("hostcaps.{id}")).is_some());
    }
}
