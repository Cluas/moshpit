//! The bench: an in-process russh server that plays an OpenSSH host closely
//! enough for the client — commands with exit codes and signals, a shell on
//! a PTY, an SFTP subsystem over a scratch directory, `sh -c` for the exec
//! upload — plus a TCP relay that can be cut (a dropped connection) and a
//! small SOCKS5 proxy. No sshd, no network beyond the loopback.

#![allow(dead_code)]

use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moshpit_ssh::{
    Auth, BoxFuture, ConnectOptions, HostKey, HostKeyPolicy, HostKeyPrompt, HostKeyRefusal, Ssh,
    Target, Waiting,
};
use russh::keys::{Algorithm, EcdsaCurve, PrivateKey, PublicKey};
use russh::server::{self, Auth as ServerAuth, Msg, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Sig};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub const USER: &str = "moshpit";
pub const PASSWORD: &str = "correct horse";
/// What a keyboard-interactive prompt naming a "code" takes.
pub const ONE_TIME_CODE: &str = "424242";

/// A key from `tests/fixtures/keys`.
pub fn fixture_key(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/keys")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn fixture_private(name: &str) -> PrivateKey {
    PrivateKey::from_openssh(fixture_key(name)).unwrap()
}

pub fn fixture_public(name: &str) -> PublicKey {
    PublicKey::from_openssh(fixture_key(&format!("{name}.pub")).trim()).unwrap()
}

/// `ssh-keygen -lf`'s fingerprint of a fixture key.
pub fn keygen_fingerprint(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fingerprints.txt");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let fields: Vec<&str> = l.split_whitespace().collect();
            (fields[0] == name).then(|| fields[2].to_owned())
        })
        .unwrap_or_else(|| panic!("no fingerprint for {name}"))
}

pub fn random_key(algorithm: Algorithm) -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), algorithm).unwrap()
}

pub fn ed25519() -> PrivateKey {
    random_key(Algorithm::Ed25519)
}

pub fn ecdsa(curve: EcdsaCurve) -> PrivateKey {
    random_key(Algorithm::Ecdsa { curve })
}

/// A directory that goes away with the value.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "moshpit-ssh-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    pub fn path(&self) -> &Path {
        &self.0
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

/// What the server saw.
#[derive(Debug, Default)]
pub struct Seen {
    /// The signature algorithm of every accepted public-key sign-in.
    pub key_algorithms: Vec<String>,
    /// The signature algorithm of every signed public-key request, taken
    /// or not, in order.
    pub key_attempts: Vec<String>,
    /// The answers to each keyboard-interactive round, in order.
    pub kbd_answers: Vec<Vec<String>>,
    pub password_attempts: usize,
    /// `none` sign-ins asked, taken or not.
    pub none_attempts: usize,
    pub env: Vec<(String, String)>,
    /// `(term, cols, rows)`.
    pub ptys: Vec<(String, u32, u32)>,
    pub window_changes: Vec<(u32, u32)>,
    pub signals: Vec<String>,
    /// What each exec request ran: the payload of its `sh -c` (B138), as
    /// the account's shell hands it to sh.
    pub commands: Vec<String>,
    /// Each exec request exactly as it came.
    pub requests: Vec<String>,
    pub connections: usize,
    /// SFTP file and directory handles open right now.
    pub open_handles: usize,
    /// The next this many SFTP opens for writing are refused (permission
    /// denied): an upload's failure of its own, not the subsystem's.
    pub sftp_write_failures: usize,
    /// Session channels open right now, across connections: one closes
    /// when the client's CHANNEL_CLOSE arrives (B720).
    pub open_channels: usize,
    /// Every `direct-tcpip` open asked for (`ssh -J`'s tunnel), forwarded
    /// or not: where to.
    pub tunnels: Vec<(String, u32)>,
}

/// How the bench's host behaves.
pub struct Bench {
    pub host_keys: Vec<PrivateKey>,
    pub authorized: Vec<PublicKey>,
    pub password: Option<String>,
    /// The sign-in methods the server advertises.
    pub methods: Vec<MethodKind>,
    pub sftp: bool,
    /// What the account's shell prints before the SFTP server starts (an
    /// rc file's `echo`, which sshd's `$SHELL -c sftp-server` runs first).
    pub sftp_noise: Vec<u8>,
    /// What the account's shell prints ahead of every exec command's own
    /// output (an rc file's `echo`, for sshd's `$SHELL -c`).
    pub exec_noise: Vec<u8>,
    /// Unknown commands run in `sh -c` with `$HOME` the scratch home.
    pub shell_commands: bool,
    /// The server's host-key preference (it presents the first that the
    /// client also lists... the client's order decides). Also what its
    /// `server-sig-algs` lists: one naming no RSA signature reads, to the
    /// client, as a server that said nothing.
    pub preferred_keys: Option<Vec<Algorithm>>,
    /// The ciphers the host offers (`None`: russh's).
    pub ciphers: Option<Vec<russh::cipher::Name>>,
    /// The RSA signatures the host takes (`None`: any). An OpenSSH before
    /// 7.2 takes only `ssh-rsa`.
    pub rsa_signatures: Option<Vec<&'static str>>,
    /// What keyboard-interactive asks, round by round: each prompt and
    /// whether it is shown as typed. Taken when every hidden prompt was
    /// answered with the password, or, for one naming a "code", with
    /// [`ONE_TIME_CODE`]. (Offered only when `methods` names it.)
    pub keyboard_interactive: Vec<Vec<(&'static str, bool)>>,
    /// A key is only the first factor, keyboard-interactive the second
    /// (`AuthenticationMethods publickey,keyboard-interactive`).
    pub key_then_keyboard_interactive: bool,
    /// A key is only the first factor, the password the second
    /// (`AuthenticationMethods publickey,password`).
    pub key_then_password: bool,
    /// Session channels a connection may hold open at once, beyond which
    /// opening one is refused (sshd's `MaxSessions`; `None`: no limit).
    /// One closes when the client's CHANNEL_CLOSE arrives.
    pub max_sessions: Option<usize>,
    /// How long the host takes to accept a session channel (`None`: at
    /// once): a client can give up on an open still on its way.
    pub open_delay: Option<Duration>,
    /// The places a `direct-tcpip` open reaches, by the name and port the
    /// client asks for (a jump host's view of its network): a name only
    /// the host knows stands for a host only it can reach. Any other is
    /// refused as sshd refuses one it can't connect to.
    pub tunnels: Vec<(&'static str, u16, SocketAddr)>,
    /// The account signs in on `none`, nothing asked (Tailscale SSH: the
    /// tailnet vouched for it).
    pub none: bool,
}

impl Default for Bench {
    fn default() -> Self {
        Self {
            host_keys: vec![ed25519()],
            authorized: vec![fixture_public("ed25519")],
            password: Some(PASSWORD.to_owned()),
            methods: vec![MethodKind::PublicKey, MethodKind::Password],
            sftp: true,
            sftp_noise: Vec::new(),
            exec_noise: Vec::new(),
            shell_commands: false,
            preferred_keys: None,
            ciphers: None,
            rsa_signatures: None,
            keyboard_interactive: Vec::new(),
            key_then_keyboard_interactive: false,
            key_then_password: false,
            max_sessions: None,
            open_delay: None,
            tunnels: Vec::new(),
            none: false,
        }
    }
}

struct Shared {
    authorized: Vec<PublicKey>,
    rsa_signatures: Option<Vec<&'static str>>,
    keyboard_interactive: Vec<Vec<(&'static str, bool)>>,
    key_then_keyboard_interactive: bool,
    key_then_password: bool,
    password: Option<String>,
    methods: Vec<MethodKind>,
    sftp: bool,
    sftp_noise: Vec<u8>,
    exec_noise: Vec<u8>,
    shell_commands: bool,
    max_sessions: Option<usize>,
    open_delay: Option<Duration>,
    tunnels: Vec<(&'static str, u16, SocketAddr)>,
    none: bool,
    home: PathBuf,
    seen: Arc<Mutex<Seen>>,
}

/// A bench host, listening.
pub struct Host {
    pub addr: SocketAddr,
    pub seen: Arc<Mutex<Seen>>,
    pub home: Scratch,
    pub host_keys: Vec<HostKey>,
    task: JoinHandle<()>,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Host {
    pub async fn start(bench: Bench) -> Host {
        let home = Scratch::new("home");
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mut config = server::Config {
            keys: bench.host_keys.clone(),
            methods: MethodSet::from(&bench.methods[..]),
            auth_rejection_time: Duration::from_millis(5),
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        };
        if let Some(keys) = &bench.preferred_keys {
            config.preferred.key = keys.clone().into();
        }
        if let Some(ciphers) = &bench.ciphers {
            config.preferred.cipher = ciphers.clone().into();
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut server = Server {
            shared: Arc::new(Shared {
                authorized: bench.authorized.clone(),
                rsa_signatures: bench.rsa_signatures.clone(),
                keyboard_interactive: bench.keyboard_interactive.clone(),
                key_then_keyboard_interactive: bench.key_then_keyboard_interactive,
                key_then_password: bench.key_then_password,
                password: bench.password.clone(),
                methods: bench.methods.clone(),
                sftp: bench.sftp,
                sftp_noise: bench.sftp_noise.clone(),
                exec_noise: bench.exec_noise.clone(),
                shell_commands: bench.shell_commands,
                max_sessions: bench.max_sessions,
                open_delay: bench.open_delay,
                tunnels: bench.tunnels.clone(),
                none: bench.none,
                home: home.path().to_owned(),
                seen: seen.clone(),
            }),
        };
        let config = Arc::new(config);
        let task = tokio::spawn(async move {
            let _ = server.run_on_socket(config, &listener).await;
        });
        Host {
            addr,
            seen,
            home,
            host_keys: bench
                .host_keys
                .iter()
                .map(|k| HostKey::from(k.public_key().clone()))
                .collect(),
            task,
        }
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn target(&self) -> Target {
        Target::new(USER, "127.0.0.1", self.port())
    }

    pub fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().unwrap()
    }
}

#[derive(Clone)]
struct Server {
    shared: Arc<Shared>,
}

impl server::Server for Server {
    type Handler = Connection;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Connection {
        self.shared.seen.lock().unwrap().connections += 1;
        Connection {
            shared: self.shared.clone(),
            channels: HashMap::new(),
            kinds: HashMap::new(),
            kbd_given: Vec::new(),
            key_taken: false,
            sizes: HashMap::new(),
            env: HashMap::new(),
            open_sessions: 0,
            tunnels: HashSet::new(),
        }
    }
}

enum Kind {
    Cat,
    Shell(Vec<u8>),
    Sleep,
    WatchSize,
}

struct Connection {
    shared: Arc<Shared>,
    channels: HashMap<ChannelId, Channel<Msg>>,
    kinds: HashMap<ChannelId, Kind>,
    sizes: HashMap<ChannelId, (u32, u32)>,
    env: HashMap<ChannelId, HashMap<String, String>>,
    /// Session channels open now (against `max_sessions`).
    open_sessions: usize,
    /// The `direct-tcpip` channels open now (not sessions).
    tunnels: HashSet<ChannelId>,
    /// The keyboard-interactive answers of this sign-in so far.
    kbd_given: Vec<Vec<String>>,
    /// The key, the first factor, was taken.
    key_taken: bool,
}

impl Connection {
    /// As OpenSSH does: a failed attempt leaves every method on offer (russh
    /// would drop the one just tried).
    fn reject(&self) -> ServerAuth {
        ServerAuth::Reject {
            proceed_with_methods: Some(MethodSet::from(&self.shared.methods[..])),
            partial_success: false,
        }
    }
}

fn finish(session: &mut Session, channel: ChannelId, status: u32) -> Result<(), russh::Error> {
    session.exit_status_request(channel, status)?;
    session.eof(channel)?;
    session.close(channel)
}

impl server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_none(&mut self, user: &str) -> Result<ServerAuth, Self::Error> {
        self.shared.seen.lock().unwrap().none_attempts += 1;
        if self.shared.none && user == USER {
            Ok(ServerAuth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<ServerAuth, Self::Error> {
        let algorithm = key.algorithm.as_str().to_owned();
        self.shared
            .seen
            .lock()
            .unwrap()
            .key_attempts
            .push(algorithm.clone());
        let known = self
            .shared
            .authorized
            .iter()
            .any(|k| k.key_data() == key.public_key.key_data());
        let signature_taken = match (&key.algorithm, &self.shared.rsa_signatures) {
            (Algorithm::Rsa { .. }, Some(taken)) => taken.contains(&algorithm.as_str()),
            _ => true,
        };
        if user == USER && known && signature_taken {
            self.shared
                .seen
                .lock()
                .unwrap()
                .key_algorithms
                .push(key.algorithm.as_str().to_owned());
            if self.shared.key_then_keyboard_interactive {
                self.key_taken = true;
                return Ok(ServerAuth::Reject {
                    proceed_with_methods: Some(MethodSet::from(
                        &[MethodKind::KeyboardInteractive][..],
                    )),
                    partial_success: true,
                });
            }
            if self.shared.key_then_password {
                self.key_taken = true;
                return Ok(ServerAuth::Reject {
                    proceed_with_methods: Some(MethodSet::from(&[MethodKind::Password][..])),
                    partial_success: true,
                });
            }
            Ok(ServerAuth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<ServerAuth, Self::Error> {
        if !self.shared.methods.contains(&MethodKind::Password) {
            return Ok(self.reject());
        }
        self.shared.seen.lock().unwrap().password_attempts += 1;
        if self.shared.key_then_password && !self.key_taken {
            return Ok(self.reject());
        }
        if user == USER && self.shared.password.as_deref() == Some(password) {
            Ok(ServerAuth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<ServerAuth, Self::Error> {
        let rounds = self.shared.keyboard_interactive.clone();
        if !self
            .shared
            .methods
            .contains(&MethodKind::KeyboardInteractive)
            || rounds.is_empty()
            || (self.shared.key_then_keyboard_interactive && !self.key_taken)
        {
            return Ok(self.reject());
        }
        match response {
            None => self.kbd_given.clear(),
            Some(response) => {
                let answers: Vec<String> = response
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .collect();
                self.shared
                    .seen
                    .lock()
                    .unwrap()
                    .kbd_answers
                    .push(answers.clone());
                self.kbd_given.push(answers);
            }
        }
        if let Some(prompts) = rounds.get(self.kbd_given.len()) {
            let prompts: Vec<(Cow<'static, str>, bool)> = prompts
                .iter()
                .map(|(prompt, echo)| (Cow::Borrowed(*prompt), *echo))
                .collect();
            return Ok(ServerAuth::Partial {
                name: Cow::Borrowed(""),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(prompts),
            });
        }
        let password = self.shared.password.as_deref();
        let answered = rounds
            .iter()
            .zip(&self.kbd_given)
            .all(|(prompts, answers)| {
                prompts.len() == answers.len()
                    && prompts.iter().zip(answers).all(|((prompt, echo), answer)| {
                        *echo
                            || if prompt.to_lowercase().contains("code") {
                                answer == ONE_TIME_CODE
                            } else {
                                Some(answer.as_str()) == password
                            }
                    })
            });
        if user == USER && answered {
            Ok(ServerAuth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self
            .shared
            .max_sessions
            .is_some_and(|most| self.open_sessions >= most)
        {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        if let Some(delay) = self.shared.open_delay {
            tokio::time::sleep(delay).await;
        }
        self.open_sessions += 1;
        self.shared.seen.lock().unwrap().open_channels += 1;
        // Kept only until the channel turns out not to be SFTP's: russh
        // also queues every message for this handle, and a full queue
        // nobody reads stalls the whole server.
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    /// `ssh -J`'s tunnel: the place asked for, if it is one of the
    /// bench's, connected and its bytes carried both ways until either
    /// side closes.
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
        self.shared
            .seen
            .lock()
            .unwrap()
            .tunnels
            .push((host_to_connect.to_owned(), port_to_connect));
        let to = self
            .shared
            .tunnels
            .iter()
            .find(|(host, port, _)| *host == host_to_connect && u32::from(*port) == port_to_connect)
            .map(|(_, _, to)| *to);
        let Some(Ok(mut tcp)) = (match to {
            Some(to) => Some(TcpStream::connect(to).await),
            None => None,
        }) else {
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        self.tunnels.insert(channel.id());
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        });
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.tunnels.remove(&channel) {
            return Ok(());
        }
        self.open_sessions = self.open_sessions.saturating_sub(1);
        {
            let mut seen = self.shared.seen.lock().unwrap();
            seen.open_channels = seen.open_channels.saturating_sub(1);
        }
        self.channels.remove(&channel);
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        value: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared
            .seen
            .lock()
            .unwrap()
            .env
            .push((name.to_owned(), value.to_owned()));
        self.env
            .entry(channel)
            .or_default()
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared
            .seen
            .lock()
            .unwrap()
            .ptys
            .push((term.to_owned(), cols, rows));
        self.sizes.insert(channel, (cols, rows));
        self.channels.remove(&channel);
        session.channel_success(channel)
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared
            .seen
            .lock()
            .unwrap()
            .window_changes
            .push((cols, rows));
        self.sizes.insert(channel, (cols, rows));
        if let Some(Kind::WatchSize) = self.kinds.get(&channel) {
            session.data(channel, format!("{rows} {cols}\r\n").into_bytes())?;
        }
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.kinds.insert(channel, Kind::Shell(Vec::new()));
        self.channels.remove(&channel);
        session.channel_success(channel)?;
        session.data(channel, &b"$ "[..])
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let request = String::from_utf8_lossy(data).into_owned();
        // The client wraps every command as `sh -c <word>` (B138); a host's
        // shell would hand sh the word.
        let command = sh_payload(&request).unwrap_or_else(|| request.clone());
        self.channels.remove(&channel);
        {
            let mut seen = self.shared.seen.lock().unwrap();
            seen.commands.push(command.clone());
            seen.requests.push(request);
        }
        if command == "refuse" {
            return session.channel_failure(channel);
        }
        if command == "hang" {
            // Never answers the request.
            return Ok(());
        }
        session.channel_success(channel)?;
        let (word, rest) = command.split_once(' ').unwrap_or((command.as_str(), ""));
        match word {
            "echo" => {
                session.data(channel, format!("{rest}\n").into_bytes())?;
                finish(session, channel, 0)
            }
            "fail" => {
                session.extended_data(channel, 1, &b"nope\n"[..])?;
                finish(session, channel, rest.parse().unwrap_or(1))
            }
            "interleave" => {
                session.extended_data(channel, 1, &b"a"[..])?;
                session.data(channel, &b"b"[..])?;
                session.extended_data(channel, 1, &b"c"[..])?;
                session.data(channel, &b"d"[..])?;
                finish(session, channel, 0)
            }
            "noise" => {
                let n: usize = rest.parse().unwrap_or(0);
                let bytes: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
                for chunk in bytes.chunks(16 * 1024) {
                    session.data(channel, chunk.to_vec())?;
                }
                finish(session, channel, 0)
            }
            "cat" => {
                self.kinds.insert(channel, Kind::Cat);
                Ok(())
            }
            "sleep" => {
                self.kinds.insert(channel, Kind::Sleep);
                Ok(())
            }
            "watch-size" => {
                self.kinds.insert(channel, Kind::WatchSize);
                Ok(())
            }
            "size" => {
                let (cols, rows) = self.sizes.get(&channel).copied().unwrap_or((0, 0));
                session.data(channel, format!("{rows} {cols}\n").into_bytes())?;
                finish(session, channel, 0)
            }
            "env" => {
                let value = self
                    .env
                    .get(&channel)
                    .and_then(|env| env.get(rest))
                    .cloned()
                    .unwrap_or_default();
                session.data(channel, format!("{value}\n").into_bytes())?;
                finish(session, channel, 0)
            }
            "no-status" => {
                session.data(channel, &b"partial"[..])?;
                session.eof(channel)?;
                session.close(channel)
            }
            "disconnect" => session.disconnect(
                russh::Disconnect::ByApplication,
                "the host is going down",
                "en",
            ),
            _ if self.shared.shell_commands => {
                let output = tokio::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&command)
                    .env("HOME", &self.shared.home)
                    .current_dir(&self.shared.home)
                    .output()
                    .await
                    .unwrap();
                let stdout = [self.shared.exec_noise.clone(), output.stdout].concat();
                if !stdout.is_empty() {
                    session.data(channel, stdout)?;
                }
                if !output.stderr.is_empty() {
                    session.extended_data(channel, 1, output.stderr)?;
                }
                finish(session, channel, output.status.code().unwrap_or(255) as u32)
            }
            _ => {
                session.extended_data(
                    channel,
                    1,
                    format!("{word}: command not found\n").into_bytes(),
                )?;
                finish(session, channel, 127)
            }
        }
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let name = format!("{signal:?}");
        self.shared.seen.lock().unwrap().signals.push(name);
        if let Some(Kind::Sleep | Kind::WatchSize) = self.kinds.remove(&channel) {
            session.exit_signal_request(channel, signal, false, "killed", "en")?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match self.kinds.get_mut(&channel) {
            Some(Kind::Cat) => session.data(channel, data.to_vec()),
            Some(Kind::Shell(line)) => {
                // Echo, as a terminal would, and run a line on Enter.
                session.data(channel, data.to_vec())?;
                for &byte in data {
                    if byte == b'\r' || byte == b'\n' {
                        let typed = String::from_utf8_lossy(line).into_owned();
                        line.clear();
                        if typed == "exit" {
                            self.kinds.remove(&channel);
                            session.data(channel, &b"\r\nlogout\r\n"[..])?;
                            return finish(session, channel, 0);
                        }
                        session.data(channel, &b"\r\n$ "[..])?;
                    } else {
                        line.push(byte);
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(Kind::Cat) = self.kinds.remove(&channel) {
            finish(session, channel, 0)?;
        }
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" || !self.shared.sftp {
            return session.channel_failure(channel);
        }
        let Some(stream) = self.channels.remove(&channel) else {
            return session.channel_failure(channel);
        };
        session.channel_success(channel)?;
        let files = Files::new(self.shared.home.clone(), self.shared.seen.clone());
        let mut stream = stream.into_stream();
        if !self.shared.sftp_noise.is_empty() {
            use tokio::io::AsyncWriteExt;
            let _ = stream.write_all(&self.shared.sftp_noise).await;
        }
        russh_sftp::server::run(stream, files).await;
        Ok(())
    }
}

/// An SFTP server over a directory, with OpenSSH's habits: `.` and `..` in
/// listings, exclusive creates and renames over a taken name answered with
/// a bare failure, reads past the end answered with EOF.
struct Files {
    home: PathBuf,
    seen: Arc<Mutex<Seen>>,
    next: usize,
    files: HashMap<String, std::fs::File>,
    dirs: HashMap<String, Option<Vec<File>>>,
}

impl Files {
    fn new(home: PathBuf, seen: Arc<Mutex<Seen>>) -> Self {
        Self {
            home,
            seen,
            next: 0,
            files: HashMap::new(),
            dirs: HashMap::new(),
        }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        if path.starts_with('/') {
            PathBuf::from(path)
        } else {
            self.home.join(path)
        }
    }

    fn handle(&mut self) -> String {
        self.next += 1;
        self.seen.lock().unwrap().open_handles += 1;
        format!("h{}", self.next)
    }
}

fn status_of(e: std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn attrs_of(meta: &std::fs::Metadata) -> FileAttributes {
    use std::os::unix::fs::MetadataExt;
    FileAttributes {
        size: Some(meta.len()),
        uid: Some(meta.uid()),
        gid: Some(meta.gid()),
        permissions: Some(meta.mode()),
        atime: Some(meta.atime() as u32),
        mtime: Some(meta.mtime() as u32),
        ..FileAttributes::empty()
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en-US".to_owned(),
    }
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let real = self.resolve(&path).canonicalize().map_err(status_of)?;
        Ok(Name {
            id,
            files: vec![File::dummy(real.to_string_lossy().into_owned())],
        })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options
            .read(pflags.contains(OpenFlags::READ))
            .write(pflags.contains(OpenFlags::WRITE))
            .append(pflags.contains(OpenFlags::APPEND))
            .truncate(pflags.contains(OpenFlags::TRUNCATE));
        if pflags.contains(OpenFlags::EXCLUDE) {
            options.create_new(true);
        } else if pflags.contains(OpenFlags::CREATE) {
            options.create(true);
        }
        options.mode(attrs.permissions.unwrap_or(0o666) & 0o777);
        if pflags.contains(OpenFlags::WRITE) {
            let mut seen = self.seen.lock().unwrap();
            if seen.sftp_write_failures > 0 {
                seen.sftp_write_failures -= 1;
                return Err(StatusCode::PermissionDenied);
            }
        }
        let file = options.open(self.resolve(&filename)).map_err(status_of)?;
        let handle = self.handle();
        self.files.insert(handle.clone(), file);
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        if self.files.remove(&handle).is_none() && self.dirs.remove(&handle).is_none() {
            return Err(StatusCode::Failure);
        }
        self.seen.lock().unwrap().open_handles -= 1;
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        use std::os::unix::fs::FileExt;
        let file = self.files.get(&handle).ok_or(StatusCode::Failure)?;
        let mut buf = vec![0u8; len.min(255 * 1024) as usize];
        let n = file.read_at(&mut buf, offset).map_err(status_of)?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        use std::os::unix::fs::FileExt;
        let file = self.files.get(&handle).ok_or(StatusCode::Failure)?;
        file.write_all_at(&data, offset).map_err(status_of)?;
        Ok(ok(id))
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let file = self.files.get(&handle).ok_or(StatusCode::Failure)?;
        let meta = file.metadata().map_err(status_of)?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&meta),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = std::fs::metadata(self.resolve(&path)).map_err(status_of)?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&meta),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = std::fs::symlink_metadata(self.resolve(&path)).map_err(status_of)?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&meta),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let dir = self.resolve(&path);
        let mut entries = vec![
            File::new(".", attrs_of(&std::fs::metadata(&dir).map_err(status_of)?)),
            File::new("..", FileAttributes::empty()),
        ];
        for entry in std::fs::read_dir(&dir).map_err(status_of)? {
            let entry = entry.map_err(status_of)?;
            let meta = entry.metadata().map_err(status_of)?;
            entries.push(File::new(
                entry.file_name().to_string_lossy().into_owned(),
                attrs_of(&meta),
            ));
        }
        let handle = self.handle();
        self.dirs.insert(handle.clone(), Some(entries));
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.dirs.get_mut(&handle) {
            Some(slot @ Some(_)) => Ok(Name {
                id,
                files: slot.take().unwrap(),
            }),
            Some(None) => Err(StatusCode::Eof),
            None => Err(StatusCode::Failure),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        std::fs::remove_file(self.resolve(&filename)).map_err(status_of)?;
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(attrs.permissions.unwrap_or(0o777) & 0o777)
            .create(self.resolve(&path))
            .map_err(status_of)?;
        Ok(ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let to = self.resolve(&newpath);
        if std::fs::symlink_metadata(&to).is_ok() {
            return Err(StatusCode::Failure);
        }
        std::fs::rename(self.resolve(&oldpath), to).map_err(status_of)?;
        Ok(ok(id))
    }
}

/// A TCP relay in front of a host whose connections can be cut with a reset:
/// a dropped connection, as a phone sees one.
pub struct Relay {
    pub port: u16,
    state: tokio::sync::watch::Sender<Link>,
    task: JoinHandle<()>,
}

/// What the relay's line is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Up,
    /// Nothing moves either way, as on a link that went dead without a
    /// word: the client's socket buffers fill and stay full.
    Stalled,
    Cut,
}

impl Relay {
    pub async fn start(to: SocketAddr) -> Relay {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (state, _) = tokio::sync::watch::channel(Link::Up);
        let states = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let states = states.clone();
                tokio::spawn(async move {
                    let Ok(server) = TcpStream::connect(to).await else {
                        return;
                    };
                    let (client_read, client_write) = client.into_split();
                    let (server_read, server_write) = server.into_split();
                    let up = tokio::spawn(pipe(client_read, server_write, states.subscribe()));
                    let down = tokio::spawn(pipe(server_read, client_write, states.subscribe()));
                    let (up, down) = (up.await, down.await);
                    // A reset, not a goodbye: zero linger, then drop. Doesn't block.
                    if let (Ok(Some((r1, w1))), Ok(Some((r2, w2)))) = (up, down) {
                        for half in [r1.reunite(w2), r2.reunite(w1)].into_iter().flatten() {
                            #[allow(deprecated)]
                            let _ = half.set_linger(Some(Duration::ZERO));
                        }
                    }
                });
            }
        });
        Relay { port, state, task }
    }

    /// Resets every connection through the relay.
    pub fn cut(&self) {
        self.state.send_replace(Link::Cut);
    }

    /// Stops moving bytes, both ways, without closing anything.
    pub fn stall(&self) {
        self.state.send_replace(Link::Stalled);
    }

    /// Moves bytes again after a stall: what queued up meanwhile goes
    /// through.
    pub fn resume(&self) {
        self.state.send_replace(Link::Up);
    }
}

/// Copies one direction until EOF or a cut; on a cut, hands both halves
/// back to be reset.
async fn pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    mut state: tokio::sync::watch::Receiver<Link>,
) -> Option<(
    tokio::net::tcp::OwnedReadHalf,
    tokio::net::tcp::OwnedWriteHalf,
)> {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        if *state.borrow_and_update() == Link::Stalled {
            match state.wait_for(|s| *s != Link::Stalled).await.map(|s| *s) {
                Ok(Link::Cut) => return Some((from, to)),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        tokio::select! {
            read = from.read(&mut buf) => match read {
                Ok(0) | Err(_) => {
                    let _ = to.shutdown().await;
                    return None;
                }
                Ok(n) => {
                    if to.write_all(&buf[..n]).await.is_err() {
                        return None;
                    }
                }
            },
            changed = state.changed() => {
                if changed.is_err() {
                    return None;
                }
                if *state.borrow() == Link::Cut {
                    return Some((from, to));
                }
            }
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// How the bench's SOCKS5 proxy behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyMode {
    Tunnel,
    /// Answers the CONNECT with this reply code.
    Refuse(u8),
    /// Takes only username/password.
    WantsPassword,
    /// Accepts TCP and never says a word.
    Silent,
    /// Answers like an HTTP server.
    NotSocks,
}

/// A SOCKS5 proxy (RFC 1928, no authentication) for the bench.
pub struct Proxy {
    pub port: u16,
    /// Every CONNECT target: address type, host, port.
    pub targets: Arc<Mutex<Vec<(u8, String, u16)>>>,
    task: JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    pub async fn start(mode: ProxyMode) -> Proxy {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let targets = Arc::new(Mutex::new(Vec::new()));
        let seen = targets.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let seen = seen.clone();
                tokio::spawn(async move {
                    let _ = proxy_one(client, mode, seen).await;
                });
            }
        });
        Proxy {
            port,
            targets,
            task,
        }
    }
}

async fn proxy_one(
    mut client: TcpStream,
    mode: ProxyMode,
    seen: Arc<Mutex<Vec<(u8, String, u16)>>>,
) -> std::io::Result<()> {
    match mode {
        ProxyMode::Silent => {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            return Ok(());
        }
        ProxyMode::NotSocks => {
            client
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .await?;
            return Ok(());
        }
        _ => {}
    }
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    let mut methods = vec![0u8; head[1] as usize];
    client.read_exact(&mut methods).await?;
    if mode == ProxyMode::WantsPassword || !methods.contains(&0) {
        client.write_all(&[5, 0xff]).await?;
        return Ok(());
    }
    client.write_all(&[5, 0]).await?;
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    let host = match request[3] {
        1 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        4 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return Ok(()),
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    seen.lock().unwrap().push((request[3], host.clone(), port));
    if let ProxyMode::Refuse(code) = mode {
        client.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    }
    let mut upstream = TcpStream::connect((host.as_str(), port)).await?;
    // Bound to a domain name, as some proxies answer.
    let mut reply = vec![5, 0, 0, 3, 9];
    reply.extend_from_slice(b"localhost");
    reply.extend_from_slice(&port.to_be_bytes());
    client.write_all(&reply).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Accepts any host key, without asking.
pub struct AcceptAny;

impl HostKeyPolicy for AcceptAny {
    fn check<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
        _key: &'a HostKey,
        _waiting: &'a Waiting,
    ) -> BoxFuture<'a, Result<(), HostKeyRefusal>> {
        Box::pin(async { Ok(()) })
    }
}

/// A person at the prompt, answering from a script after thinking for
/// `think`, and remembering what was asked.
#[derive(Default)]
pub struct Person {
    pub answers: Mutex<VecDeque<bool>>,
    pub asked: Mutex<Vec<Asked>>,
    pub think: Duration,
    /// Set when a question is dropped before it was answered.
    pub walked_away: Arc<AtomicUsize>,
    /// What each key asked about said of its algorithm
    /// ([`HostKey::older_algorithm`]), in order.
    pub older: Mutex<Vec<Option<&'static str>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    /// `Some` for a changed key: what was trusted.
    pub expected: Option<String>,
    /// Asked because an earlier build stored two different keys (B725).
    pub conflicting: bool,
}

impl Person {
    pub fn answering(answers: &[bool]) -> Arc<Person> {
        Arc::new(Person {
            answers: Mutex::new(answers.iter().copied().collect()),
            ..Default::default()
        })
    }

    pub fn thinking(think: Duration, answers: &[bool]) -> Arc<Person> {
        Arc::new(Person {
            answers: Mutex::new(answers.iter().copied().collect()),
            think,
            ..Default::default()
        })
    }

    pub fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }

    async fn answer(&self, asked: Asked) -> bool {
        self.asked.lock().unwrap().push(asked);
        let guard = WalkAway(self.walked_away.clone(), true);
        tokio::time::sleep(self.think).await;
        let mut guard = guard;
        guard.1 = false;
        self.answers.lock().unwrap().pop_front().unwrap_or(false)
    }
}

struct WalkAway(Arc<AtomicUsize>, bool);

impl Drop for WalkAway {
    fn drop(&mut self) {
        if self.1 {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl HostKeyPrompt for Person {
    fn trust_new_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> BoxFuture<'a, bool> {
        self.older.lock().unwrap().push(key.older_algorithm());
        Box::pin(self.answer(Asked {
            host: host.to_owned(),
            port,
            fingerprint: key.fingerprint(),
            expected: None,
            conflicting: false,
        }))
    }

    fn accept_changed_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
        expected: &'a str,
    ) -> BoxFuture<'a, bool> {
        self.older.lock().unwrap().push(key.older_algorithm());
        Box::pin(self.answer(Asked {
            host: host.to_owned(),
            port,
            fingerprint: key.fingerprint(),
            expected: Some(expected.to_owned()),
            conflicting: false,
        }))
    }

    fn accept_conflicting_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> BoxFuture<'a, bool> {
        Box::pin(self.answer(Asked {
            host: host.to_owned(),
            port,
            fingerprint: key.fingerprint(),
            expected: None,
            conflicting: true,
        }))
    }
}

/// A person who never gets to change a key: the default prompt behaviour.
pub struct NewHostsOnly(pub Arc<Person>);

impl HostKeyPrompt for NewHostsOnly {
    fn trust_new_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> BoxFuture<'a, bool> {
        self.0.trust_new_host(host, port, key)
    }
}

pub fn options() -> ConnectOptions {
    ConnectOptions {
        connect_timeout: Duration::from_secs(5),
        handshake_timeout: Duration::from_secs(10),
        ..ConnectOptions::default()
    }
}

/// Signs in as the bench user with the fixture Ed25519 key, any host key.
pub async fn connect(host: &Host) -> Ssh {
    Ssh::connect(
        &host.target(),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap()
}

/// Reads a process to its end, returning everything and the last event.
pub async fn drain(process: &mut moshpit_ssh::Process) -> (Vec<moshpit_ssh::Event>, Vec<u8>) {
    let mut events = Vec::new();
    let mut stdout = Vec::new();
    while let Some(event) = process.next().await {
        if let moshpit_ssh::Event::Stdout(bytes) = &event {
            stdout.extend_from_slice(bytes);
        }
        events.push(event);
    }
    (events, stdout)
}

/// Reads stdout until `needle` shows up (or the process ends).
pub async fn read_until(process: &mut moshpit_ssh::Process, needle: &str) -> String {
    let mut seen = Vec::new();
    while !String::from_utf8_lossy(&seen).contains(needle) {
        match process.next().await {
            Some(moshpit_ssh::Event::Stdout(bytes)) => seen.extend_from_slice(&bytes),
            Some(moshpit_ssh::Event::Closed | moshpit_ssh::Event::Lost) | None => break,
            Some(_) => {}
        }
    }
    String::from_utf8_lossy(&seen).into_owned()
}

pub async fn within<F: std::future::Future>(seconds: u64, work: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(seconds), work)
        .await
        .expect("timed out")
}

/// The script of `sh -c <one word>`, the word read as a POSIX shell reads
/// it: single quotes literal, a backslash outside them taking the next
/// character. `None` for anything else.
pub fn sh_payload(request: &str) -> Option<String> {
    let word = request.strip_prefix("sh -c ")?;
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '\'') => quoted = false,
            (true, c) => out.push(c),
            (false, '\'') => quoted = true,
            (false, '\\') => out.push(chars.next()?),
            (false, _) => return None,
        }
    }
    (!quoted).then_some(out)
}
