//! One authenticated connection to a host.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use russh::client::{self, AuthResult, DisconnectReason, KeyboardInteractiveAuthResponse};
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect, MethodKind};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::auth::{self, Auth, PlatformSignError, PlatformSigner};
use crate::channel::{Output, Process, PtyRequest};
use crate::dial;
use crate::error::SshError;
use crate::host_key::{HostKey, HostKeyPolicy, HostKeyRefusal};
use crate::questions::{SignInQuestion, SignInQuestions, reads_as_password};
use crate::sftp::Sftp;
use crate::target::{ConnectOptions, Target};
use crate::upload;
use crate::waiting::Waiting;

/// A broken link, injected ([`Ssh::inject_fault`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFault {
    /// Reset: reads and writes fail from now on.
    Cut,
    /// Silence: nothing moves in either direction again, and nothing says
    /// so (a tunnel that died, a phone that changed networks under a NAT).
    Stall,
}

/// How a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEnd {
    /// We closed it ([`Ssh::close`]).
    ClosedByUs,
    /// The server said goodbye (`SSH_MSG_DISCONNECT`): it ended the
    /// connection on purpose — sshd shutting down, an administrator.
    ServerDisconnected { reason: String, message: String },
    /// It went away without a goodbye: a reset, an EOF, a keepalive nobody
    /// answered, a protocol error. The one a reconnect is for.
    Lost { error: String },
}

impl ConnectionEnd {
    pub fn is_lost(&self) -> bool {
        matches!(self, ConnectionEnd::Lost { .. })
    }
}

/// An authenticated connection. Cheap to clone: every clone shares the
/// connection, and each can open its own channels. Several connections to
/// the same host (the mosh bootstrap and its `-CC` sidecar, a reconnect
/// racing the old one) are simply several `Ssh`s; they share nothing but
/// the trusted-key map.
#[derive(Clone)]
pub struct Ssh {
    inner: Arc<Inner>,
}

struct Inner {
    handle: client::Handle<ClientHandler>,
    target: Target,
    /// The address the TCP leg reached; `None` through a proxy.
    peer: Option<SocketAddr>,
    host_key: HostKey,
    end: watch::Receiver<Option<ConnectionEnd>>,
    closing: Arc<AtomicBool>,
    kill: CancellationToken,
    /// A debug fault froze the link ([`LinkFault::Stall`]).
    stalled: Arc<AtomicBool>,
    /// When the socket last gave us bytes ([`Ssh::last_heard`]).
    heard: Arc<Heard>,
    channel_timeout: Duration,
    /// An upload already fell back to exec and worked there
    /// (`sftpKnownUnavailable`): this connection stops trying SFTP for it.
    pub(crate) sftp_unavailable: AtomicBool,
    /// The host refused the `sftp` subsystem: it has none, and the
    /// connection doesn't ask again (B612).
    pub(crate) sftp_refused: AtomicBool,
    /// The SFTP session file previews read through, kept between reads.
    pub(crate) sftp: tokio::sync::Mutex<Option<Sftp>>,
}

impl Ssh {
    /// Dials (straight or through the proxy), checks the host key with
    /// `policy`, signs in. Never retries: the session layer owns reconnects.
    /// What the server asks at sign-in beyond the password is never
    /// answered ([`Ssh::connect_asking`] asks the person).
    pub async fn connect(
        target: &Target,
        auth: Auth,
        policy: Arc<dyn HostKeyPolicy>,
        options: &ConnectOptions,
    ) -> Result<Ssh, SshError> {
        Self::connect_asking(target, auth, policy, None, options).await
    }

    /// [`Ssh::connect`], with `questions` answering what the server asks at
    /// sign-in that the saved password doesn't (keyboard-interactive: a
    /// one-time code, B563); the time the person takes doesn't count
    /// against the handshake.
    pub async fn connect_asking(
        target: &Target,
        auth: Auth,
        policy: Arc<dyn HostKeyPolicy>,
        questions: Option<Arc<dyn SignInQuestions>>,
        options: &ConnectOptions,
    ) -> Result<Ssh, SshError> {
        let stream = dial::dial(&target.host, target.port, options).await?;
        let peer_of = |stream: &TcpStream| match options.proxy {
            None => stream.peer_addr().ok(),
            Some(_) => None,
        };
        let peer = peer_of(&stream);

        let kill = CancellationToken::new();
        // Whatever happens below — a timeout, an error, the caller dropping
        // this future during a host-key prompt — the half-made connection
        // and any prompt still waiting go with it.
        let abandon = KillOnDrop(Some(kill.clone()));
        let waiting = Waiting::new();
        let decided = Arc::new(Mutex::new(Decided::default()));
        let closing = Arc::new(AtomicBool::new(false));
        // Each dial has its own end: what a dial given up says of itself is
        // not the connection's.
        let handler = |older: Option<&'static str>| {
            let (end, ended) = watch::channel(None);
            let handler = ClientHandler {
                host: target.host.clone(),
                port: target.port,
                policy: policy.clone(),
                older,
                waiting: waiting.clone(),
                decided: decided.clone(),
                closing: closing.clone(),
                end,
                kill: kill.clone(),
            };
            (handler, ended)
        };
        let stalled = Arc::new(AtomicBool::new(false));
        let heard = Arc::new(Heard::new());
        let killable = |stream| Killable::new(stream, kill.clone(), stalled.clone(), heard.clone());

        let handshake = async {
            let (first, ended) = handler(None);
            let opened = client::connect_stream(
                Arc::new(client_config(options, false)),
                killable(stream),
                first,
            )
            .await;
            let (mut handle, peer, ended) = match opened {
                Ok(handle) => (handle, peer, ended),
                Err(e) => match handshake_error(e, target, &decided) {
                    // B564: a server whose host key is signed with SHA-1
                    // only is dialled again, offered that alone — never
                    // chosen over anything better, and known for certain
                    // to be what the key exchange used.
                    SshError::Incompatible {
                        what: "host key",
                        theirs,
                        ..
                    } if theirs.iter().any(|name| name == OLDER_HOST_KEY.as_str()) => {
                        let stream = dial::dial(&target.host, target.port, options).await?;
                        let peer = peer_of(&stream);
                        let (again, ended) = handler(Some(OLDER_HOST_KEY.as_str()));
                        let handle = client::connect_stream(
                            Arc::new(client_config(options, true)),
                            killable(stream),
                            again,
                        )
                        .await
                        .map_err(|e| handshake_error(e, target, &decided))?;
                        (handle, peer, ended)
                    }
                    error => return Err(error),
                },
            };
            sign_in(&mut handle, target, auth, &waiting, questions.as_deref()).await?;
            Ok::<_, SshError>((handle, peer, ended))
        };
        let (handle, peer, end_rx) =
            match waiting.within(options.handshake_timeout, handshake).await {
                Some(result) => result?,
                None => {
                    return Err(SshError::HandshakeTimeout {
                        host: target.host.clone(),
                        port: target.port,
                        timeout: options.handshake_timeout,
                    });
                }
            };
        abandon.disarm();

        let host_key = decided
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .seen
            .clone()
            .ok_or_else(|| SshError::Closed {
                host: target.host.clone(),
            })?;
        Ok(Ssh {
            inner: Arc::new(Inner {
                handle,
                target: target.clone(),
                peer,
                host_key,
                end: end_rx,
                closing,
                kill,
                stalled,
                heard,
                channel_timeout: options.channel_timeout,
                sftp_unavailable: AtomicBool::new(false),
                sftp_refused: AtomicBool::new(false),
                sftp: tokio::sync::Mutex::new(None),
            }),
        })
    }

    pub fn target(&self) -> &Target {
        &self.inner.target
    }

    /// `ssh-rsa` when the server offered no host-key algorithm but that
    /// one (B564): worth saying, never a reason to refuse.
    pub fn older_host_key_algorithm(&self) -> Option<&'static str> {
        self.inner.host_key.older_algorithm()
    }

    /// The address the connection reached, when it went straight to the
    /// host (`None` through a proxy): where a mosh session started over it
    /// sends its datagrams — the address `mosh-server -s` bound to, even
    /// when the name resolves to several.
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.inner.peer
    }

    /// Breaks the link under this connection the way a network does, for a
    /// debug build's fault injection (the app's DEBUG launch arguments; the
    /// session tests' relay does the same from outside). Not a close: no
    /// goodbye is sent, and nothing here says it was on purpose.
    #[doc(hidden)]
    pub fn inject_fault(&self, fault: LinkFault) {
        match fault {
            LinkFault::Cut => self.inner.kill.cancel(),
            LinkFault::Stall => self.inner.stalled.store(true, Ordering::SeqCst),
        }
    }

    /// The key the host presented, as the policy saw it.
    pub fn host_key(&self) -> &HostKey {
        &self.inner.host_key
    }

    /// Starts `command`, a POSIX sh script, through the account's shell,
    /// without a PTY. Returns once the host agreed to run it.
    ///
    /// The shell gets `sh -c <command>` ([`crate::shell::portable`]): a
    /// login shell that is fish parses the app's sh syntax otherwise — and
    /// refuses `$?`, which every herdr command ends with (B138).
    pub async fn exec(&self, command: &str) -> Result<Process, SshError> {
        self.answered("run a command", async {
            let channel = self.open_session().await?;
            channel
                .exec(true, crate::shell::portable(command).as_str())
                .await?;
            Process::opened(channel, self.host(), self.ends(), "run a command").await
        })
        .await
    }

    /// Runs `command` with nothing on its stdin and collects what it did:
    /// the exec the app uses for probes, tmux and herdr commands, the mosh
    /// bootstrap. A command that fails is not an error here — its
    /// [`Output::exit`] says so, and its output is kept.
    pub async fn run(&self, command: &str) -> Result<Output, SshError> {
        let process = self.exec(command).await?;
        process.close_stdin().await?;
        Ok(process.collect().await)
    }

    /// [`Self::run`], given up the moment `cancel` is (B243): `None` then,
    /// at once, and the channel is closed in the background (the command
    /// stops getting a reader; the host is told the channel is done with).
    /// Not run at all if `cancel` already is.
    pub async fn run_until(
        &self,
        command: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<Output>, SshError> {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let process = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(None),
            process = self.exec(command) => process?,
        };
        process.close_stdin().await?;
        let writer = process.writer();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tokio::spawn(async move {
                    let _ = writer.close().await;
                });
                Ok(None)
            }
            output = process.collect() => Ok(Some(output)),
        }
    }

    /// A terminal: a PTY of `request.cols` × `request.rows` running the
    /// login shell, or `request.command` through it. The environment goes
    /// first (`LANG` for a UTF-8 locale; a server that refuses it just
    /// ignores it).
    pub async fn terminal(&self, request: &PtyRequest) -> Result<Process, SshError> {
        self.answered("start a shell", self.open_terminal(request))
            .await
    }

    async fn open_terminal(&self, request: &PtyRequest) -> Result<Process, SshError> {
        let mut channel = self.open_session().await?;
        for (name, value) in &request.env {
            channel
                .set_env(false, name.as_str(), value.as_str())
                .await?;
        }
        channel
            .request_pty(true, &request.term, request.cols, request.rows, 0, 0, &[])
            .await?;
        // The host answers the PTY request before the shell starts.
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) => {
                    // Closed, or it holds a session on the host (B720).
                    let _ = channel.close().await;
                    return Err(SshError::Refused {
                        host: self.host(),
                        what: "open a terminal".to_owned(),
                    });
                }
                Some(_) => continue,
                None => return Err(SshError::Closed { host: self.host() }),
            }
        }
        match &request.command {
            // As `exec`: sh reads the command, whatever the login shell is.
            Some(command) => {
                channel
                    .exec(true, crate::shell::portable(command).as_str())
                    .await?
            }
            None => channel.request_shell(true).await?,
        }
        Process::opened(channel, self.host(), self.ends(), "start a shell").await
    }

    /// The SFTP session file previews and edits go through: opened on first
    /// use, kept between operations, reopened when its channel has died.
    /// A host that refused the subsystem isn't asked again (B612): each ask
    /// is a session channel, and the callers ask on every read.
    pub async fn sftp(&self) -> Result<Sftp, crate::error::SftpError> {
        if self.inner.sftp_refused.load(Ordering::SeqCst) {
            return Err(crate::error::SftpError::Unavailable(
                "the host refused the sftp subsystem".to_owned(),
            ));
        }
        let mut slot = self.inner.sftp.lock().await;
        if let Some(sftp) = slot.as_ref()
            && sftp.is_alive()
        {
            return Ok(sftp.clone());
        }
        let fresh = Sftp::start(self).await?;
        *slot = Some(fresh.clone());
        Ok(fresh)
    }

    /// Uploads `data` into `~/.moshpit/uploads/<filename>` (directory 700,
    /// file 600) and returns the absolute remote path: over SFTP, or over
    /// exec channels on a host without it (`uploadToUploadsDirectory`).
    pub async fn upload_to_uploads_directory(
        &self,
        data: &[u8],
        filename: &str,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
    ) -> Result<String, crate::error::SftpError> {
        upload::to_uploads_directory(self, data, filename, progress, None).await
    }

    /// [`Self::upload_to_uploads_directory`], given up the moment `cancel`
    /// is (B243): [`crate::SftpError::Cancelled`] at once, over SFTP
    /// ([`crate::Sftp::cancelled_by`]) or exec ([`Self::run_until`]).
    pub async fn upload_to_uploads_directory_until(
        &self,
        data: &[u8],
        filename: &str,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
        cancel: &CancellationToken,
    ) -> Result<String, crate::error::SftpError> {
        upload::to_uploads_directory(self, data, filename, progress, Some(cancel)).await
    }

    /// Closes the connection: a goodbye to the server, then the socket.
    /// Open channels end as [`crate::Event::Lost`].
    pub async fn close(&self) {
        self.inner.closing.store(true, Ordering::SeqCst);
        let mut end = self.inner.end.clone();
        // A dead link can't carry the goodbye (and a session stuck on a
        // full socket can't even take it): don't wait for it long.
        let _ = tokio::time::timeout(CLOSE_GRACE, async {
            let _ = self
                .inner
                .handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
            let _ = end.wait_for(Option::is_some).await;
        })
        .await;
        self.inner.kill.cancel();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.handle.is_closed() || self.inner.end.borrow().is_some()
    }

    /// When the host last sent this connection anything at all: a channel's
    /// output, an open confirmed, a window adjust, a ping answered. A link
    /// that has gone dead sends nothing, so a request older than this that
    /// still has no word back is the first sign of one (B276).
    pub fn last_heard(&self) -> Instant {
        self.inner.heard.at()
    }

    /// Asks the host whether the connection lives (`keepalive@openssh.com`,
    /// a reply wanted): `true` once it answered within `timeout`. A global
    /// request sshd answers itself — no channel, nothing run on the host —
    /// so it costs two small packets and works when every session channel
    /// the host allows is in use. A connection that ends meanwhile is
    /// `false`.
    pub async fn ping(&self, timeout: Duration) -> bool {
        if self.is_closed() {
            return false;
        }
        // russh answers the wait with Ok when the connection's task ends
        // too (its reply sender dropped): the end is checked after.
        let answered = tokio::time::timeout(timeout, self.inner.handle.send_ping())
            .await
            .is_ok_and(|sent| sent.is_ok());
        answered && !self.is_closed()
    }

    /// How the connection ended, once it has.
    pub fn end(&self) -> Option<ConnectionEnd> {
        self.inner.end.borrow().clone()
    }

    /// Waits until the connection ends and says how.
    pub async fn ended(&self) -> ConnectionEnd {
        let mut end = self.inner.end.clone();
        match end.wait_for(Option::is_some).await {
            Ok(end) => end.clone().unwrap_or_else(lost_silently),
            Err(_) => lost_silently(),
        }
    }

    pub(crate) fn host(&self) -> String {
        self.inner.target.host.clone()
    }

    /// The connection's end, for a channel to give up on when it comes.
    pub(crate) fn ends(&self) -> watch::Receiver<Option<ConnectionEnd>> {
        self.inner.end.clone()
    }

    /// `work` (opening a channel and the host's answer to its request)
    /// within [`ConnectOptions::channel_timeout`].
    pub(crate) async fn answered<T>(
        &self,
        what: &str,
        work: impl Future<Output = Result<T, SshError>>,
    ) -> Result<T, SshError> {
        let timeout = self.inner.channel_timeout;
        tokio::time::timeout(timeout, work)
            .await
            .unwrap_or_else(|_| {
                Err(SshError::Unresponsive {
                    host: self.host(),
                    what: what.to_owned(),
                    timeout,
                })
            })
    }

    pub(crate) fn sftp_known_unavailable(&self) -> bool {
        self.inner.sftp_unavailable.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_sftp_unavailable(&self) {
        self.inner.sftp_unavailable.store(true, Ordering::SeqCst);
    }

    /// The host refused the `sftp` subsystem on this connection (B612).
    pub(crate) fn sftp_refused(&self) -> bool {
        self.inner.sftp_refused.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_sftp_refused(&self) {
        self.inner.sftp_refused.store(true, Ordering::SeqCst);
    }

    /// A session channel. Opened in a task of its own (B720): a caller given
    /// up while the host is still answering (a cancelled read, a timeout)
    /// left the host's yes to a waiter that was gone, and the channel open
    /// on both sides; the task closes one nobody took.
    pub(crate) async fn open_session(&self) -> Result<russh::Channel<client::Msg>, SshError> {
        let (taken, opened) = tokio::sync::oneshot::channel();
        let ssh = self.clone();
        tokio::spawn(async move {
            let result = ssh.inner.handle.channel_open_session().await;
            if let Err(Ok(channel)) = taken.send(result) {
                let _ = channel.close().await;
            }
        });
        opened
            .await
            .map_err(|_| SshError::Closed { host: self.host() })?
            .map_err(|e| match e {
                russh::Error::ChannelOpenFailure(_) => SshError::Refused {
                    host: self.host(),
                    what: "open a session channel".to_owned(),
                },
                russh::Error::SendError | russh::Error::Disconnect => {
                    SshError::Closed { host: self.host() }
                }
                e => SshError::Protocol(e),
            })
    }
}

fn lost_silently() -> ConnectionEnd {
    ConnectionEnd::Lost {
        error: "the connection's task ended".to_owned(),
    }
}

/// How long [`Ssh::close`] waits for the goodbye to go out.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The client configuration: no compression, no inactivity timeout, and the
/// host-key algorithms in the order NIOSSH offered them, so a host shows
/// this client the same key it showed the app (and the fingerprint trusted
/// then still matches). RSA, which the app never accepted as a host key,
/// comes last, SHA-2 only.
///
/// `older`: the dial again for a server that offers no host-key algorithm
/// but [`OLDER_HOST_KEY`] (B564), which offers that alone.
fn client_config(options: &ConnectOptions, older: bool) -> client::Config {
    let mut preferred = russh::Preferred::DEFAULT.clone();
    preferred.key = if older {
        (&[OLDER_HOST_KEY][..]).into()
    } else {
        HOST_KEY_ORDER.into()
    };
    preferred.compression = (&[russh::compression::NONE][..]).into();
    client::Config {
        preferred,
        inactivity_timeout: None,
        keepalive_interval: options.keepalive.map(|k| k.interval),
        keepalive_max: options.keepalive.map_or(0, |k| k.max_unanswered),
        nodelay: true,
        window_size: options.receive_window,
        ..Default::default()
    }
}

/// The host-key algorithm of a server that offers nothing better: `ssh-rsa`,
/// signed with SHA-1 (an old dropbear, an OpenSSH before 7.2). Used only
/// then, on a dial of its own, and said where the person sees it (B564).
pub const OLDER_HOST_KEY: Algorithm = Algorithm::Rsa { hash: None };

/// `SSHKeyExchangeStateMachine.bundledServerHostKeyAlgorithms`, then RSA.
/// [`OLDER_HOST_KEY`] is not here: it is offered only when the server has
/// nothing in this list.
pub const HOST_KEY_ORDER: &[Algorithm] = &[
    Algorithm::Ed25519,
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP384,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP256,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP521,
    },
    Algorithm::Rsa {
        hash: Some(HashAlg::Sha512),
    },
    Algorithm::Rsa {
        hash: Some(HashAlg::Sha256),
    },
];

async fn sign_in(
    handle: &mut client::Handle<ClientHandler>,
    target: &Target,
    auth: Auth,
    waiting: &Waiting,
    questions: Option<&dyn SignInQuestions>,
) -> Result<(), SshError> {
    let user = target.user.clone();
    let closed = |e: russh::Error| SshError::Handshake {
        host: target.host.clone(),
        port: target.port,
        reason: format!("closed during sign-in: {e}"),
    };
    let asked = Asking {
        host: &target.host,
        questions,
        waiting,
    };
    let (method, result) = match auth {
        Auth::Password(password) => {
            let result = handle
                .authenticate_password(user.clone(), password.as_str())
                .await
                .map_err(closed)?;
            match result {
                // It takes passwords only by asking (B393): FreeBSD's stock
                // sshd, PAM setups.
                AuthResult::Failure {
                    remaining_methods,
                    partial_success: false,
                } if !remaining_methods.contains(&MethodKind::Password)
                    && remaining_methods.contains(&MethodKind::KeyboardInteractive) =>
                {
                    let answered = keyboard_interactive(handle, &user, Some(&password), &asked)
                        .await
                        .map_err(closed)?;
                    (
                        "keyboard-interactive",
                        interactive_result(answered, target)?,
                    )
                }
                result => ("password", result),
            }
        }
        Auth::Key(text) => {
            let key = Arc::new(auth::load_private_key(&text)?);
            drop(text);
            let hashes = if key.algorithm().is_rsa() {
                match rsa_hash(handle).await {
                    Some(listed) => vec![listed],
                    None => UNLISTED_RSA_ORDER.to_vec(),
                }
            } else {
                vec![None]
            };
            // One request per signature, on this connection, until one is
            // taken or the host stops offering keys (B390).
            let mut result = None;
            for hash in hashes {
                let answer = handle
                    .authenticate_publickey(
                        user.clone(),
                        PrivateKeyWithHashAlg::new(key.clone(), hash),
                    )
                    .await
                    .map_err(closed)?;
                let more = match &answer {
                    AuthResult::Success => false,
                    AuthResult::Failure {
                        remaining_methods,
                        partial_success,
                    } => !partial_success && remaining_methods.contains(&MethodKind::PublicKey),
                };
                result = Some(answer);
                if !more {
                    break;
                }
            }
            (
                "publickey",
                result.expect("at least one signature is tried"),
            )
        }
        Auth::Hardware(key) => {
            let public_key = key.public_key.clone();
            let hash = if public_key.algorithm().is_rsa() {
                rsa_hash(handle).await.unwrap_or(Some(HashAlg::Sha512))
            } else {
                None
            };
            let mut signer = PlatformSigner {
                key,
                waiting: waiting.clone(),
            };
            let result = handle
                .authenticate_publickey_with(user.clone(), public_key, hash, &mut signer)
                .await
                .map_err(|e| match e {
                    PlatformSignError::Signer(e) => SshError::Signer(e),
                    PlatformSignError::Ssh(e) => closed(e),
                })?;
            ("publickey", result)
        }
    };
    // B565: a key (or a password) that was only the first factor, with
    // keyboard-interactive the second (`AuthenticationMethods
    // publickey,keyboard-interactive`, the common 2FA setup): every prompt
    // of the second is the person's — the saved password, if any, was the
    // first factor's. It read as a wrong key.
    let (method, result) = match result {
        AuthResult::Failure {
            remaining_methods,
            partial_success: true,
        } if remaining_methods.contains(&MethodKind::KeyboardInteractive) => {
            let answered = keyboard_interactive(handle, &user, None, &asked)
                .await
                .map_err(closed)?;
            (
                "keyboard-interactive",
                interactive_result(answered, target)?,
            )
        }
        // B722: the password as the second factor (`AuthenticationMethods
        // publickey,password`): the person's, asked as keyboard-interactive's
        // second factor is — the saved password, if any, was the first
        // factor's. It read as a wrong key. Declined, it is the person's
        // cancel, not a failure to dial again.
        AuthResult::Failure {
            remaining_methods,
            partial_success: true,
        } if remaining_methods.contains(&MethodKind::Password) => {
            let Some(questions) = asked.questions else {
                return Err(SshError::InteractivePrompts {
                    host: target.host.clone(),
                });
            };
            let question = SignInQuestion {
                name: String::new(),
                instructions: String::new(),
                prompt: "Password: ".to_owned(),
                echo: false,
            };
            let Some(password) = asked
                .waiting
                .on_person(questions.ask(asked.host, &question))
                .await
            else {
                return Err(SshError::SignInDeclined {
                    host: target.host.clone(),
                });
            };
            let result = handle
                .authenticate_password(user.clone(), password.as_str())
                .await
                .map_err(closed)?;
            ("password", result)
        }
        result => (method, result),
    };
    match result {
        AuthResult::Success => Ok(()),
        AuthResult::Failure {
            remaining_methods,
            partial_success,
        } => {
            let offered: Vec<String> = remaining_methods.iter().map(String::from).collect();
            if !partial_success && !offered.is_empty() && !offered.iter().any(|m| m == method) {
                Err(SshError::AuthMethodRejected {
                    host: target.host.clone(),
                    method,
                    offered,
                })
            } else {
                Err(SshError::AuthFailed {
                    user: target.user.clone(),
                    host: target.host.clone(),
                    remaining: offered,
                })
            }
        }
    }
}

/// Who answers a keyboard-interactive round's prompts besides the saved
/// password: the person, through `questions`, when there is one.
struct Asking<'a> {
    host: &'a str,
    questions: Option<&'a dyn SignInQuestions>,
    waiting: &'a Waiting,
}

/// The server's verdict on a keyboard-interactive sign-in, or why there is
/// none.
fn interactive_result(answered: Interactive, target: &Target) -> Result<AuthResult, SshError> {
    match answered {
        Interactive::Done(result) => Ok(result),
        Interactive::Unanswerable => Err(SshError::InteractivePrompts {
            host: target.host.clone(),
        }),
        Interactive::Declined => Err(SshError::SignInDeclined {
            host: target.host.clone(),
        }),
    }
}

/// How a keyboard-interactive sign-in ended.
enum Interactive {
    /// The server's verdict.
    Done(AuthResult),
    /// A prompt only the person can answer, and nobody to ask: the round
    /// is left unanswered.
    Unanswerable,
    /// The person declined a prompt.
    Declined,
}

/// The saved password, given through keyboard-interactive (B393), for a
/// server that takes a password only by asking for it: a hidden prompt
/// that reads as the password's ([`reads_as_password`]) gets it, once. A
/// round asking nothing (PAM's closing one) is answered empty. Every other
/// prompt — a one-time code, a token, a prompt shown as typed, a password
/// asked again — is the person's (B563): asked with the server's words
/// when there is someone to ask, else the round is left unanswered. A
/// one-time code asked before the password used to get the password.
/// Without a saved password (a second factor after a key, B565) every
/// prompt is the person's.
async fn keyboard_interactive(
    handle: &mut client::Handle<ClientHandler>,
    user: &str,
    password: Option<&str>,
    asking: &Asking<'_>,
) -> Result<Interactive, russh::Error> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await?;
    let mut answered = false;
    for _ in 0..KEYBOARD_INTERACTIVE_ROUNDS {
        let answers = match response {
            KeyboardInteractiveAuthResponse::Success => {
                return Ok(Interactive::Done(AuthResult::Success));
            }
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods,
                partial_success,
            } => {
                return Ok(Interactive::Done(AuthResult::Failure {
                    remaining_methods,
                    partial_success,
                }));
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let mut answers = Vec::with_capacity(prompts.len());
                for prompt in &prompts {
                    if let Some(password) = password
                        && !prompt.echo
                        && !answered
                        && reads_as_password(&prompt.prompt)
                    {
                        answered = true;
                        answers.push(password.to_owned());
                        continue;
                    }
                    let Some(questions) = asking.questions else {
                        return Ok(Interactive::Unanswerable);
                    };
                    let question = SignInQuestion {
                        name: name.clone(),
                        instructions: instructions.clone(),
                        prompt: prompt.prompt.clone(),
                        echo: prompt.echo,
                    };
                    match asking
                        .waiting
                        .on_person(questions.ask(asking.host, &question))
                        .await
                    {
                        Some(answer) => answers.push(answer),
                        None => return Ok(Interactive::Declined),
                    }
                }
                answers
            }
        };
        response = handle
            .authenticate_keyboard_interactive_respond(answers)
            .await?;
    }
    Ok(Interactive::Unanswerable)
}

/// How many keyboard-interactive rounds are answered before the server is
/// taken to want more than a password.
const KEYBOARD_INTERACTIVE_ROUNDS: usize = 4;

/// The RSA signature the server says it takes (`server-sig-algs`, RFC
/// 8308): the strongest it lists, plain `ssh-rsa` only when that is all it
/// takes — the SHA-1 signature Citadel sent, refused by OpenSSH since 8.8.
/// `None` when it didn't say (no `server-sig-algs`, none in time, or a
/// list naming no RSA signature).
async fn rsa_hash(handle: &client::Handle<ClientHandler>) -> Option<Option<HashAlg>> {
    handle.best_supported_rsa_hash().await.ok().flatten()
}

/// The RSA signatures to try, one request each on the same connection, when
/// the server didn't say which it takes (B390): SHA-512 first, which every
/// server of the last decade takes; then SHA-256; then `ssh-rsa`, for an
/// OpenSSH before 7.2, which knows neither SHA-2 signature nor
/// `server-sig-algs` (the Swift path's `RSASignatureAlgorithm.fallbackOrder`).
/// Three requests stay within OpenSSH's MaxAuthTries of 6.
const UNLISTED_RSA_ORDER: [Option<HashAlg>; 3] =
    [Some(HashAlg::Sha512), Some(HashAlg::Sha256), None];

fn handshake_error(e: russh::Error, target: &Target, decided: &Mutex<Decided>) -> SshError {
    let host = target.host.clone();
    let port = target.port;
    let decided = decided.lock().unwrap_or_else(|e| e.into_inner());
    if let (russh::Error::UnknownKey, Some(refusal)) = (&e, &decided.refusal) {
        let fingerprint = decided
            .seen
            .as_ref()
            .map(HostKey::fingerprint)
            .unwrap_or_default();
        return match refusal {
            HostKeyRefusal::NotTrusted | HostKeyRefusal::Abandoned => SshError::HostKeyNotTrusted {
                host,
                port,
                fingerprint,
            },
            HostKeyRefusal::Changed { expected } => SshError::HostKeyChanged {
                host,
                port,
                presented: fingerprint,
                expected: expected.clone(),
            },
            HostKeyRefusal::Locked => SshError::HostKeysLocked { host, port },
        };
    }
    if let russh::Error::NoCommonAlgo { kind, theirs, .. } = &e {
        return SshError::Incompatible {
            host,
            port,
            what: match kind {
                russh::AlgorithmKind::Kex => "key exchange",
                russh::AlgorithmKind::Key => "host key",
                russh::AlgorithmKind::Cipher => "cipher",
                russh::AlgorithmKind::Mac => "MAC",
                russh::AlgorithmKind::Compression => "compression",
            },
            theirs: theirs.clone(),
        };
    }
    SshError::Handshake {
        host,
        port,
        reason: e.to_string(),
    }
}

/// What the host-key check saw and said, for `connect` to report.
#[derive(Default)]
struct Decided {
    seen: Option<HostKey>,
    refusal: Option<HostKeyRefusal>,
}

struct ClientHandler {
    host: String,
    port: u16,
    policy: Arc<dyn HostKeyPolicy>,
    /// The older host-key algorithm this dial offers alone, when it does.
    older: Option<&'static str>,
    waiting: Waiting,
    decided: Arc<Mutex<Decided>>,
    closing: Arc<AtomicBool>,
    end: watch::Sender<Option<ConnectionEnd>>,
    kill: CancellationToken,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // No certificate algorithm is advertised, so a server can only
        // present a plain key; read a certificate as the key it certifies.
        let key = HostKey::from(match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => {
                russh::keys::PublicKey::from(cert.public_key().clone())
            }
        })
        .negotiated_as(self.older);
        self.decided.lock().unwrap_or_else(|e| e.into_inner()).seen = Some(key.clone());
        let verdict = tokio::select! {
            verdict = self.policy.check(&self.host, self.port, &key, &self.waiting) => verdict,
            () = self.kill.cancelled() => Err(HostKeyRefusal::Abandoned),
        };
        match verdict {
            Ok(()) => Ok(true),
            Err(refusal) => {
                self.decided
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .refusal = Some(refusal);
                Ok(false)
            }
        }
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        let end = match reason {
            DisconnectReason::ReceivedDisconnect(info) => ConnectionEnd::ServerDisconnected {
                reason: format!("{:?}", info.reason_code),
                message: info.message,
            },
            DisconnectReason::Error(_) if self.closing.load(Ordering::SeqCst) => {
                ConnectionEnd::ClosedByUs
            }
            DisconnectReason::Error(e) => ConnectionEnd::Lost {
                error: e.to_string(),
            },
        };
        log::debug!("{}:{}: connection ended: {end:?}", self.host, self.port);
        self.end.send_replace(Some(end));
        Ok(())
    }
}

/// Cancels the token when dropped, unless disarmed.
struct KillOnDrop(Option<CancellationToken>);

impl KillOnDrop {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.cancel();
        }
    }
}

/// When bytes last came in, as microseconds past the connection's start
/// (shared by the socket wrapper that sees them and the handle that asks).
struct Heard {
    start: Instant,
    micros: AtomicU64,
}

impl Heard {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            micros: AtomicU64::new(0),
        }
    }

    fn now(&self) {
        let micros = self.start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        self.micros.fetch_max(micros, Ordering::Relaxed);
    }

    fn at(&self) -> Instant {
        self.start + Duration::from_micros(self.micros.load(Ordering::Relaxed))
    }
}

/// The socket under the SSH session, with a way to cut it from outside:
/// russh's session task owns the stream, and a connect that gives up (or a
/// close the link can't carry) must not leave that task holding it.
struct Killable<S> {
    inner: S,
    killed: Pin<Box<WaitForCancellationFutureOwned>>,
    stalled: Arc<AtomicBool>,
    heard: Arc<Heard>,
}

impl<S> Killable<S> {
    fn new(
        inner: S,
        token: CancellationToken,
        stalled: Arc<AtomicBool>,
        heard: Arc<Heard>,
    ) -> Self {
        Self {
            inner,
            killed: Box::pin(token.cancelled_owned()),
            stalled,
            heard,
        }
    }

    /// `Err` once killed; `Ok(false)` while a stall holds everything (the
    /// kill still wakes the task: its future was polled first).
    fn check(&mut self, cx: &mut Context<'_>) -> Result<bool, io::Error> {
        match self.killed.as_mut().poll(cx) {
            Poll::Ready(()) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the connection was abandoned",
            )),
            Poll::Pending => Ok(!self.stalled.load(Ordering::SeqCst)),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Killable<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.check(cx) {
            Err(e) => return Poll::Ready(Err(e)),
            Ok(false) => return Poll::Pending,
            Ok(true) => {}
        }
        let before = buf.filled().len();
        let read = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(read, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.heard.now();
        }
        read
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Killable<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.check(cx) {
            Err(e) => return Poll::Ready(Err(e)),
            Ok(false) => return Poll::Pending,
            Ok(true) => {}
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.check(cx) {
            Err(e) => return Poll::Ready(Err(e)),
            Ok(false) => return Poll::Pending,
            Ok(true) => {}
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
