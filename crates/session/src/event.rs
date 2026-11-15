//! What the UI hears: plain data, keyed by session id and stream id, so the
//! FFI layer can hand it across as is.

use moshpit_ssh::{SignerError, SshError};
use moshpit_stream::StreamId;
use moshpit_tmux::controller::ViewAction;

/// A session's name in the registry. Never 0, never reused.
pub type SessionId = u64;

/// Where a session stands: the one state every surface reads (the
/// terminal's cover, the home card, the transport pill), collapsed the way
/// the app's `TransportConnState` collapses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// A connect the person asked for is on its way.
    Connecting,
    Connected,
    /// A transport that was up went away (or a failed one is being retried
    /// on its own), and the session is dialling again. One state for the
    /// whole automatic recovery, however many attempts it takes. The UI
    /// keeps the last frame under a veil and a capsule — never a poster.
    Reconnecting,
    /// The remote end finished the session, or the person closed it.
    /// Nothing redials it on its own; reconnecting is the person's call.
    Ended(EndReason),
    /// The attempt the person asked for didn't connect, or one that can't
    /// succeed by retrying (credentials, a host key) failed. Retried on the
    /// foreground return and on a network change when
    /// [`Failure::retryable`].
    Failed(Failure),
}

impl SessionState {
    pub fn is_live(&self) -> bool {
        matches!(self, SessionState::Connected)
    }
}

/// Why a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// The shell exited (`exit`, Ctrl-D) and the host closed its channel in
    /// order. Its exit status when the host sent one.
    ShellExited { status: Option<u32> },
    /// mosh-server ended the session: its shell exited, or it was stopped.
    MoshServerEnded,
    /// The person disconnected.
    Disconnected,
}

/// What kind of failure, for the copy and for the retry policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Nothing answered: name, route, refused port, the dial timeout.
    Unreachable,
    /// The SSH handshake failed or timed out, or the connection closed
    /// while it was being set up.
    Handshake,
    /// A new host the person didn't trust, or a changed key they refused.
    HostKey,
    /// The trusted host keys can't be read until the device is unlocked.
    HostKeysLocked,
    /// The server turned the credentials down, or the key can't be used.
    Credentials,
    /// The saved password or key can't be read until the device is unlocked
    /// (a launch before the first unlock, B201): retried on the unlock and
    /// the foreground return, unlike [`FailureKind::Credentials`].
    CredentialsLocked,
    /// The SOCKS5 proxy wouldn't carry the connection.
    Proxy,
    /// The host refused or didn't answer a channel (a shell, a command).
    Channel,
    /// `mosh-server` didn't start, or said something that isn't a session.
    MoshBootstrap,
    /// The connection went away during the attempt.
    Lost,
}

/// Why nothing answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreachableWhy {
    /// The port turned the connection away.
    Refused,
    /// Nothing answered within the dial timeout (a VPN address with the
    /// tunnel down, a host asleep).
    TimedOut,
    /// The name resolves to no address.
    NoAddress,
    /// No route to the network or the host.
    NoRoute,
    Other,
}

/// Why the SOCKS5 proxy didn't carry the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyWhy {
    /// The proxy itself couldn't be reached.
    Unreachable,
    /// It wants a username and password (only proxies without are taken).
    NeedsPassword,
    NotSocks5,
    /// It refused the tunnel with this SOCKS5 reply code.
    Refused {
        code: u8,
    },
    /// The host name is longer than SOCKS5 carries.
    NameTooLong,
    /// It closed the connection during its handshake.
    Closed,
}

/// Why a private key can't sign in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyWhy {
    NotOpenSsh,
    Encrypted,
    Malformed,
    /// A kind of key that can't sign in from here (`sk-`, DSA, …), as named.
    Unsupported {
        algorithm: String,
    },
    BadPublicKey,
}

/// Why signing in didn't happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialsWhy {
    /// The server turned them down.
    Rejected { user: String },
    /// The server doesn't take this kind of sign-in (`password`,
    /// `publickey`).
    MethodRejected { method: String },
    /// The server takes a password only by asking its own questions, and
    /// asked what only the person can answer (a one-time code, B393).
    InteractivePrompts,
    /// The saved key can't be used.
    KeyUnusable(KeyWhy),
    /// The hardware key (the Secure Enclave) didn't sign.
    SignerFailed,
    /// The keychain had nothing usable (an item lost to a re-install).
    Unavailable,
    /// The person cancelled (Face ID, a passphrase card).
    Cancelled,
    /// The device is locked: the item can't be read yet (B201).
    Locked,
}

/// Why mosh-server didn't give a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoshWhy {
    /// It said nothing within the bootstrap timeout (a startup file on the
    /// host waiting for input).
    TimedOut { seconds: u64 },
    /// It finished without a `MOSH CONNECT` line Moshpit can use.
    Unreadable,
}

/// The structured cause a host words its card from (B202): the core says
/// what happened, never how to say it — every sentence a person reads is
/// the host's, in the person's language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// Nothing to add to the kind.
    Plain,
    Unreachable {
        host: String,
        port: u16,
        why: UnreachableWhy,
    },
    Proxy {
        /// `host:port` of the proxy.
        proxy: String,
        why: ProxyWhy,
    },
    Handshake {
        host: String,
        port: u16,
        timed_out: bool,
    },
    HostKey {
        host: String,
        port: u16,
        /// The key CHANGED (else: a new host).
        changed: bool,
    },
    HostKeysLocked {
        host: String,
        port: u16,
    },
    Credentials(CredentialsWhy),
    Channel {
        /// No answer in time, rather than a refusal.
        timed_out: bool,
    },
    MoshBootstrap(MoshWhy),
}

/// A failed attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    /// What the host words its card from.
    pub cause: Cause,
    /// For the log only, in English; never shown to a person, never a
    /// secret.
    pub log: String,
    /// Whether trying again can help on its own: a network that comes back,
    /// a device that gets unlocked. Credentials and refused host keys can't
    /// — retrying them every keepalive tick would re-prompt the host-key
    /// card forever, and send wrong passwords to the server again and again.
    pub retryable: bool,
}

impl Failure {
    pub fn new(kind: FailureKind, cause: Cause, log: impl Into<String>) -> Self {
        let retryable = !matches!(kind, FailureKind::HostKey | FailureKind::Credentials);
        Self {
            kind,
            cause,
            log: log.into(),
            retryable,
        }
    }

    /// A failure that retrying won't fix on its own (the host said
    /// something definitive).
    pub fn permanent(kind: FailureKind, cause: Cause, log: impl Into<String>) -> Self {
        Self {
            retryable: false,
            ..Self::new(kind, cause, log)
        }
    }

    /// The connection went away.
    pub fn lost(log: impl Into<String>) -> Self {
        Self::new(FailureKind::Lost, Cause::Plain, log)
    }

    /// Signing in can't happen, for `why`. Retryable only when the device is
    /// locked (B201): an unlock fixes it; wrong or missing credentials stay
    /// wrong until the person acts.
    pub fn credentials(why: CredentialsWhy, log: impl Into<String>) -> Self {
        let kind = if why == CredentialsWhy::Locked {
            FailureKind::CredentialsLocked
        } else {
            FailureKind::Credentials
        };
        Self::new(kind, Cause::Credentials(why), log)
    }

    /// A failure the device's unlock may fix: the keychain was locked.
    pub fn waits_for_unlock(&self) -> bool {
        matches!(
            self.kind,
            FailureKind::CredentialsLocked | FailureKind::HostKeysLocked
        )
    }
}

impl From<&SshError> for Failure {
    fn from(error: &SshError) -> Self {
        let log = error.to_string();
        let (kind, cause) = match error {
            SshError::Unreachable { host, port, source } => (
                FailureKind::Unreachable,
                Cause::Unreachable {
                    host: host.clone(),
                    port: *port,
                    why: unreachable_why(source),
                },
            ),
            SshError::ConnectTimeout { host, port, .. } => (
                FailureKind::Unreachable,
                Cause::Unreachable {
                    host: host.clone(),
                    port: *port,
                    why: UnreachableWhy::TimedOut,
                },
            ),
            SshError::Proxy { proxy, reason } => {
                let cause = Cause::Proxy {
                    proxy: proxy.clone(),
                    why: proxy_why(reason),
                };
                // B560: what the proxy said about itself or its rules holds
                // until someone changes the proxy or the connection.
                if proxy_is_final(reason) {
                    return Failure::permanent(FailureKind::Proxy, cause, log);
                }
                (FailureKind::Proxy, cause)
            }
            SshError::HandshakeTimeout { host, port, .. } => (
                FailureKind::Handshake,
                Cause::Handshake {
                    host: host.clone(),
                    port: *port,
                    timed_out: true,
                },
            ),
            SshError::Handshake { host, port, .. } => (
                FailureKind::Handshake,
                Cause::Handshake {
                    host: host.clone(),
                    port: *port,
                    timed_out: false,
                },
            ),
            // B561: no algorithm in common holds until one side changes.
            SshError::Incompatible { host, port, .. } => {
                return Failure::permanent(
                    FailureKind::Handshake,
                    Cause::Handshake {
                        host: host.clone(),
                        port: *port,
                        timed_out: false,
                    },
                    log,
                );
            }
            SshError::HostKeyNotTrusted { host, port, .. } => (
                FailureKind::HostKey,
                Cause::HostKey {
                    host: host.clone(),
                    port: *port,
                    changed: false,
                },
            ),
            SshError::HostKeyChanged { host, port, .. } => (
                FailureKind::HostKey,
                Cause::HostKey {
                    host: host.clone(),
                    port: *port,
                    changed: true,
                },
            ),
            SshError::HostKeysLocked { host, port } => (
                FailureKind::HostKeysLocked,
                Cause::HostKeysLocked {
                    host: host.clone(),
                    port: *port,
                },
            ),
            SshError::AuthFailed { user, .. } => {
                return Failure::credentials(CredentialsWhy::Rejected { user: user.clone() }, log);
            }
            SshError::InteractivePrompts { .. } => {
                return Failure::credentials(CredentialsWhy::InteractivePrompts, log);
            }
            // The person's own no to the server's question (B563).
            SshError::SignInDeclined { .. } => {
                return Failure::credentials(CredentialsWhy::Cancelled, log);
            }
            SshError::AuthMethodRejected { method, .. } => {
                return Failure::credentials(
                    CredentialsWhy::MethodRejected {
                        method: (*method).to_owned(),
                    },
                    log,
                );
            }
            SshError::Key(key) => {
                return Failure::credentials(CredentialsWhy::KeyUnusable(key_why(key)), log);
            }
            // B227: a signer the locked device refused is the lock, retried
            // on the unlock — it was SignerFailed, never retried.
            SshError::Signer(SignerError::Locked) => {
                return Failure::credentials(CredentialsWhy::Locked, log);
            }
            SshError::Signer(SignerError::Cancelled) => {
                return Failure::credentials(CredentialsWhy::Cancelled, log);
            }
            SshError::Signer(_) => return Failure::credentials(CredentialsWhy::SignerFailed, log),
            SshError::Refused { .. } => (FailureKind::Channel, Cause::Channel { timed_out: false }),
            SshError::Unresponsive { .. } => {
                (FailureKind::Channel, Cause::Channel { timed_out: true })
            }
            SshError::Closed { .. } => (FailureKind::Lost, Cause::Plain),
            SshError::Protocol(_) => (
                FailureKind::Handshake,
                Cause::Handshake {
                    host: String::new(),
                    port: 0,
                    timed_out: false,
                },
            ),
            _ => (FailureKind::Handshake, Cause::Plain),
        };
        Failure::new(kind, cause, log)
    }
}

fn unreachable_why(error: &std::io::Error) -> UnreachableWhy {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::ConnectionRefused => UnreachableWhy::Refused,
        ErrorKind::TimedOut => UnreachableWhy::TimedOut,
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => UnreachableWhy::NoRoute,
        _ => {
            // A name that resolves to nothing comes back as an `Other` I/O
            // error carrying the resolver's words.
            let text = error.to_string().to_ascii_lowercase();
            if text.contains("nodename") || text.contains("not known") || text.contains("resolve") {
                UnreachableWhy::NoAddress
            } else {
                UnreachableWhy::Other
            }
        }
    }
}

/// Whether the proxy's answer is one no retry changes (B560): it wants a
/// password, isn't SOCKS5, can't take a name this long, or refuses by its
/// rules (RFC 1928 0x02) or the request itself (0x07 command, 0x08 address
/// type). The proxy or the target down or unreachable (0x01, 0x03–0x06), or
/// the proxy hanging up, may pass.
fn proxy_is_final(error: &moshpit_ssh::ProxyError) -> bool {
    use moshpit_ssh::ProxyError;
    match error {
        ProxyError::AuthRequired | ProxyError::NotSocks5 | ProxyError::NameTooLong => true,
        ProxyError::Refused(reply) => matches!(reply.0, 2 | 7 | 8),
        _ => false,
    }
}

fn proxy_why(error: &moshpit_ssh::ProxyError) -> ProxyWhy {
    use moshpit_ssh::ProxyError;
    match error {
        ProxyError::AuthRequired => ProxyWhy::NeedsPassword,
        ProxyError::NotSocks5 => ProxyWhy::NotSocks5,
        ProxyError::Refused(reply) => ProxyWhy::Refused { code: reply.0 },
        ProxyError::NameTooLong => ProxyWhy::NameTooLong,
        ProxyError::Closed => ProxyWhy::Closed,
        _ => ProxyWhy::Closed,
    }
}

fn key_why(error: &moshpit_ssh::KeyError) -> KeyWhy {
    use moshpit_ssh::KeyError;
    match error {
        KeyError::NotOpenSsh => KeyWhy::NotOpenSsh,
        KeyError::Encrypted => KeyWhy::Encrypted,
        KeyError::Malformed(_) => KeyWhy::Malformed,
        KeyError::Unsupported(algorithm) => KeyWhy::Unsupported {
            algorithm: algorithm.clone(),
        },
        KeyError::BadPublicKey(_) => KeyWhy::BadPublicKey,
        _ => KeyWhy::Malformed,
    }
}

/// The session runs at less than the person chose, because the host lacks
/// something. Never the other multiplexer: tmux and herdr hold unrelated
/// sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degrade {
    /// mosh asked for, no `mosh-server` on the host: the session is SSH.
    MoshServerMissing,
    /// tmux asked for, none on the host: a plain shell.
    TmuxMissing,
    /// herdr asked for, none on the host: a plain shell.
    HerdrMissing,
    /// herdr asked for and present, but not over this transport yet (mosh):
    /// a plain shell.
    HerdrUnavailable,
}

/// A pane with a terminal: what the UI mounts a terminal view on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    /// tmux's `%N`, herdr's pane id, or [`SHELL_PANE`] for a session
    /// without a multiplexer.
    pub pane: String,
    /// The byte channel the view attaches to.
    pub stream: StreamId,
    /// The pane input goes to when the UI doesn't say.
    pub active: bool,
}

/// The pane of a session without a multiplexer (a plain shell, mosh).
pub const SHELL_PANE: &str = "shell";

/// Why input was not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discarded {
    /// Typed while the session wasn't live and wasn't coming back on its
    /// own: before the first connect, or after it failed or ended. (Under a
    /// reconnect's veil a pane's keys wait for it, a shell's are
    /// `Reconnected`: B322.)
    NotLive,
    /// Typed into a pane that no longer exists once the session attached.
    PaneGone,
    /// The session ended with it still waiting for its pane.
    SessionEnded,
    /// Typed at a herdr pane whose attach isn't reading yet, and not the
    /// one the person just picked: herdr 0.9.5 would drop it unseen (B185),
    /// so it is dropped here, and the pane's notice says so.
    NotAttached,
    /// Typed at a plain shell while the way back from the background asked
    /// its connection, which turned out dead (B396), or under a reconnect's
    /// veil (B322): the redial is a new shell, and what was typed for the
    /// old one could run out of context there. A multiplexer pane's keys
    /// wait for the pane instead.
    Reconnected,
}

/// One thing that happened to a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub session: SessionId,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    State(SessionState),
    /// The panes with terminals, for attach number `attach` (it changes when
    /// a reconnect re-attaches a multiplexer and its panes get fresh
    /// terminals). The UI keeps showing the previous attach's views — the
    /// last frame, under the veil — until [`EventKind::FirstFrame`] for the
    /// new attach.
    Panes {
        attach: u64,
        panes: Vec<PaneInfo>,
    },
    /// Attach `attach` has painted a pane: switch to its views (and let the
    /// previous attach's go).
    FirstFrame {
        attach: u64,
    },
    /// What a pane's terminal view should do (tmux's cover and scroll
    /// choreography).
    View {
        stream: StreamId,
        action: ViewAction,
    },
    /// The multiplexer's tree (sessions, windows, panes, notice) changed:
    /// read it with [`crate::Sessions::tmux_view`] or
    /// [`crate::Sessions::herdr_view`].
    TreeChanged,
    /// What a herdr request ([`crate::Sessions::herdr`]) came back with.
    HerdrAnswer {
        token: u64,
        answer: crate::herdr::HerdrAnswer,
    },
    PaneActivity {
        pane: String,
    },
    PaneBell {
        pane: String,
    },
    /// A never-shown pane's visible screen, for the overview's picture
    /// (B751): text rows, never the pane's bytes.
    PaneSnapshot {
        pane: String,
        rows: Vec<String>,
    },
    AgentHooksUpdated,
    /// A tmux attach hasn't confirmed within [`crate::Timing::attach_stall`]
    /// after its boot line (a wedged tmux, a host that went to sleep). The
    /// session stays up; the UI offers to retry.
    AttachStalled,
    /// tmux commands the person gave while nothing carried them (mosh's
    /// control plane down, B401; the connection dialled again, B483) didn't
    /// come through: held for it, they waited past their bound. It is back
    /// now. The UI says so.
    TmuxActionsDropped,
    /// Asking the person about a host key; answer with
    /// [`crate::Sessions::answer_host_key`].
    HostKeyPrompt {
        prompt: u64,
        host: String,
        port: u16,
        fingerprint: String,
        /// The trusted fingerprint, when the key CHANGED.
        previous: Option<String>,
        /// `ssh-rsa` when the server offers no better host-key algorithm
        /// (B564): the card says so.
        older_algorithm: Option<String>,
        /// An earlier build stored two different keys for the host, and it
        /// presents one of them (B725): the card asks in its own words.
        conflicting: bool,
        /// A new host this phone connected to before (B783): its trusted
        /// key isn't here any more (a restore, a re-sign); the card says so
        /// instead of "First connection".
        connected_before: bool,
    },
    /// The connection had to use an older host-key algorithm (`ssh-rsa`,
    /// SHA-1, B564): for the diagnostics. Every connect that did.
    OlderHostKeyAlgorithm {
        algorithm: String,
    },
    /// The dial that asked is gone (superseded, given up): close the card.
    HostKeyPromptWithdrawn {
        prompt: u64,
    },
    /// Asking the person what the server asks at sign-in that the saved
    /// password doesn't answer — a one-time code (B563). Shown with the
    /// server's own words; answer with [`crate::Sessions::answer_question`].
    SignInQuestion {
        prompt: u64,
        host: String,
        /// The round's name and instructions, as the server sent them
        /// (often empty).
        name: String,
        instructions: String,
        /// The prompt: "Verification code: ".
        text: String,
        /// Whether the answer is shown as typed (false: a secret).
        echo: bool,
    },
    /// The dial that asked is gone: close the question's card.
    SignInQuestionWithdrawn {
        prompt: u64,
    },
    Degraded(Degrade),
    /// mosh hears nothing back (a VPN or proxy that passes UDP out and
    /// drops it in): offer SSH ([`crate::Sessions::switch_to_ssh`]).
    ReturnPathDead,
    /// mosh hears the server again after [`EventKind::ReturnPathDead`]: the
    /// offer of SSH goes (B400).
    ReturnPathAlive,
    /// mosh's smoothed RTT and whether it is roaming.
    MoshMetrics {
        srtt_ms: f64,
        roaming: bool,
    },
    /// mosh's protocol counters (the pill's long-press popover), at most
    /// every [`crate::mosh_udp::DIAGNOSTICS_INTERVAL`] while they move.
    MoshDiagnostics(moshpit_mosh::Diagnostics),
    /// Input that was not sent, and why.
    InputDiscarded {
        stream: StreamId,
        bytes: usize,
        why: Discarded,
    },
    /// The way back from the background asked the connection that was live
    /// when the app left, and heard (`Timing::resume_in_place`): what a
    /// device's diagnostics log, so a trial can say how often iOS keeps a
    /// suspended socket and what asking it costs.
    ResumeChecked {
        /// How long the app was away.
        away_ms: u64,
        /// How long the question took: 0 when there was nothing to ask
        /// (the connection was gone before the return).
        waited_ms: u64,
        outcome: ResumeOutcome,
        /// Asked on tmux's control channel (else the SSH keepalive, or
        /// nothing).
        through_tmux: bool,
        /// Writes typed while it was out, held for the answer: sent when
        /// the connection was kept; otherwise a multiplexer pane's wait for
        /// the pane on the new attach, a plain shell's are dropped (B396).
        held: usize,
    },
}

/// What the way back from the background found ([`EventKind::ResumeChecked`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// It answered: kept, nothing on screen changed.
    Kept,
    /// No answer by the deadline: redialled.
    Silent,
    /// Closed or reset, before the answer or already before the return:
    /// redialled.
    Dead,
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshpit_ssh::{KeyError, ProxyError, Socks5Reply};

    /// B561: a server with no algorithm in common with this client is a
    /// handshake failure that a retry doesn't change.
    #[test]
    fn no_algorithm_in_common_is_not_retried() {
        let failure = Failure::from(&SshError::Incompatible {
            host: "box".into(),
            port: 22,
            what: "host key",
            theirs: vec!["ssh-rsa".into()],
        });
        assert_eq!(failure.kind, FailureKind::Handshake);
        assert!(!failure.retryable);
        assert!(
            failure.log.contains("ssh-rsa"),
            "the log keeps what it offered"
        );
    }

    /// B560: a SOCKS proxy that can't carry the connection as configured —
    /// it wants a password (Moshpit offers none), it isn't SOCKS5, the name
    /// is too long for SOCKS5, or its rules refuse it (0x02) or the request
    /// (0x07, 0x08) — says so the same way every time: redialled on every
    /// tick, it read "Reconnecting" forever after one card. What a retry
    /// may fix stays retryable: the proxy or the target down or unreachable.
    #[test]
    fn a_proxy_that_cannot_work_as_configured_is_not_retried() {
        use moshpit_ssh::{ProxyError, Socks5Reply};
        let failed = |reason| {
            Failure::from(&SshError::Proxy {
                proxy: "p:1080".into(),
                reason,
            })
        };
        for permanent in [
            ProxyError::AuthRequired,
            ProxyError::NotSocks5,
            ProxyError::NameTooLong,
            ProxyError::Refused(Socks5Reply(2)),
            ProxyError::Refused(Socks5Reply(7)),
            ProxyError::Refused(Socks5Reply(8)),
        ] {
            let failure = failed(permanent);
            assert_eq!(failure.kind, FailureKind::Proxy);
            assert!(!failure.retryable, "{:?}", failure.cause);
        }
        for transient in [1, 3, 4, 5, 6] {
            assert!(
                failed(ProxyError::Refused(Socks5Reply(transient))).retryable,
                "0x{transient:02x}"
            );
        }
        assert!(failed(ProxyError::Closed).retryable);
    }

    #[test]
    fn a_failure_says_what_happened_never_how_to_say_it() {
        let refused = SshError::Unreachable {
            host: "box".into(),
            port: 22,
            source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
        };
        let failure = Failure::from(&refused);
        assert_eq!(failure.kind, FailureKind::Unreachable);
        assert_eq!(
            failure.cause,
            Cause::Unreachable {
                host: "box".into(),
                port: 22,
                why: UnreachableWhy::Refused
            }
        );
        assert!(failure.retryable);
        assert!(failure.log.contains("box:22"), "the log keeps the detail");

        let timeout = SshError::ConnectTimeout {
            host: "box".into(),
            port: 2222,
            timeout: std::time::Duration::from_secs(15),
        };
        assert_eq!(
            Failure::from(&timeout).cause,
            Cause::Unreachable {
                host: "box".into(),
                port: 2222,
                why: UnreachableWhy::TimedOut
            }
        );

        let proxy = SshError::Proxy {
            proxy: "p:1080".into(),
            reason: ProxyError::Refused(Socks5Reply(5)),
        };
        assert_eq!(
            Failure::from(&proxy).cause,
            Cause::Proxy {
                proxy: "p:1080".into(),
                why: ProxyWhy::Refused { code: 5 }
            }
        );

        let changed = SshError::HostKeyChanged {
            host: "box".into(),
            port: 22,
            presented: "SHA256:new".into(),
            expected: "SHA256:old".into(),
        };
        let failure = Failure::from(&changed);
        assert_eq!(failure.kind, FailureKind::HostKey);
        assert_eq!(
            failure.cause,
            Cause::HostKey {
                host: "box".into(),
                port: 22,
                changed: true
            }
        );
        assert!(!failure.retryable);

        let rejected = SshError::AuthFailed {
            user: "me".into(),
            host: "box".into(),
            remaining: vec![],
        };
        let failure = Failure::from(&rejected);
        assert_eq!(failure.kind, FailureKind::Credentials);
        assert_eq!(
            failure.cause,
            Cause::Credentials(CredentialsWhy::Rejected { user: "me".into() })
        );
        assert!(!failure.retryable);

        // B393: a server whose questions only the person can answer.
        let prompts = SshError::InteractivePrompts { host: "box".into() };
        let failure = Failure::from(&prompts);
        assert_eq!(failure.kind, FailureKind::Credentials);
        assert_eq!(
            failure.cause,
            Cause::Credentials(CredentialsWhy::InteractivePrompts)
        );
        assert!(!failure.retryable);

        let key = SshError::Key(KeyError::Unsupported("ssh-dss".into()));
        assert_eq!(
            Failure::from(&key).cause,
            Cause::Credentials(CredentialsWhy::KeyUnusable(KeyWhy::Unsupported {
                algorithm: "ssh-dss".into()
            }))
        );

        let locked = SshError::HostKeysLocked {
            host: "box".into(),
            port: 22,
        };
        let failure = Failure::from(&locked);
        assert!(failure.retryable && failure.waits_for_unlock());

        let silent = SshError::Unresponsive {
            host: "box".into(),
            what: "start a shell".into(),
            timeout: std::time::Duration::from_secs(15),
        };
        assert_eq!(
            Failure::from(&silent).cause,
            Cause::Channel { timed_out: true }
        );
    }

    #[test]
    fn a_hardware_key_the_locked_device_refuses_is_the_lock_and_waits_for_the_unlock() {
        // B227: every signer failure was SignerFailed, never retried — a
        // Secure Enclave key read refused because the device is locked
        // (a reconnect in the background, mid-handshake) gave up for good.
        let locked = Failure::from(&SshError::Signer(SignerError::Locked));
        assert_eq!(locked.kind, FailureKind::CredentialsLocked);
        assert_eq!(locked.cause, Cause::Credentials(CredentialsWhy::Locked));
        assert!(locked.retryable && locked.waits_for_unlock());
        let cancelled = Failure::from(&SshError::Signer(SignerError::Cancelled));
        assert_eq!(
            cancelled.cause,
            Cause::Credentials(CredentialsWhy::Cancelled)
        );
        assert!(!cancelled.retryable);
        // B563: the person's no to the server's question is theirs too.
        let declined = Failure::from(&SshError::SignInDeclined { host: "box".into() });
        assert_eq!(
            declined.cause,
            Cause::Credentials(CredentialsWhy::Cancelled)
        );
        assert!(!declined.retryable);
        for failed in [
            SignerError::Failed("the key is gone".into()),
            SignerError::Mismatch,
        ] {
            let failure = Failure::from(&SshError::Signer(failed.clone()));
            assert_eq!(
                failure.cause,
                Cause::Credentials(CredentialsWhy::SignerFailed),
                "{failed:?}"
            );
            assert!(!failure.retryable, "{failed:?}");
        }
    }

    #[test]
    fn only_a_locked_device_makes_credentials_worth_retrying() {
        assert!(Failure::credentials(CredentialsWhy::Locked, "").retryable);
        for why in [
            CredentialsWhy::Rejected { user: "me".into() },
            CredentialsWhy::MethodRejected {
                method: "password".into(),
            },
            CredentialsWhy::KeyUnusable(KeyWhy::Encrypted),
            CredentialsWhy::SignerFailed,
            CredentialsWhy::Unavailable,
            CredentialsWhy::Cancelled,
        ] {
            let failure = Failure::credentials(why.clone(), "");
            assert_eq!(failure.kind, FailureKind::Credentials, "{why:?}");
            assert!(!failure.retryable, "{why:?}");
        }
    }
}
