//! The SSH transport of Moshpit's Rust core, on russh: what
//! `Moshpit/Services/SSHService.swift` (Citadel over swift-nio-ssh) does for
//! the app today, for iOS behind a switch and for Android.
//!
//! - [`Ssh::connect`]: TCP straight to the host or through a SOCKS5 proxy
//!   (Happy Eyeballs over the host's addresses, 15 s), the host key checked
//!   by a [`HostKeyPolicy`], then a password, a private key or a hardware
//!   key ([`Auth`]). Never a reconnect: the session layer owns those.
//! - Host keys ([`host_key`]): trust on first use per `host:port`, keyed and
//!   fingerprinted exactly as the Swift transport's `HostKeyValidator`
//!   stored them, so every build reads and writes the same keychain map. The
//!   host-key algorithms go out in NIOSSH's order, so a host shows this
//!   client the key the app already trusted. A new host is the caller's
//!   question ([`HostKeyPrompt`]); a changed key is refused unless the
//!   caller accepts it; a locked map refuses without asking.
//! - Keys ([`Auth`], [`inspect_private_key`]): unencrypted
//!   `openssh-key-v1` text, Ed25519, RSA (rsa-sha2-512/256, never the SHA-1
//!   `ssh-rsa` Citadel signs with) and ECDSA P-256/384/521, wiped after
//!   use; a hardware key signs through the platform ([`KeySigner`]) and
//!   never enters Rust. [`keyfile`] is the one key reader every platform
//!   shares: import in every form people have keys in (OpenSSH with any
//!   cipher, PEM, PKCS #8, PuTTY), generation, the stored form and what to
//!   do with a key kept in another; the connect path reads with it too.
//! - Channels ([`Process`]): commands with their exit status, terminals
//!   with resize, stdin EOF, signals; any number per connection. A program
//!   that finished ([`Event::Closed`]) is told apart from a connection that
//!   went away under it ([`Event::Lost`], [`ConnectionEnd`]).
//! - SFTP ([`Sftp`]): stat, list, range reads, download, upload, mkdir,
//!   rename, remove — and the uploads directory with its exec fallback.
//!
//! The time a person spends deciding on a host key, or a hardware key on
//! Face ID, never counts against the handshake ([`Waiting`]). Channel
//! requests are bounded ([`ConnectOptions::channel_timeout`]); writes give
//! up when the connection dies.
//!
//! Tests: an in-process russh host for every path (`tests/`), real OpenSSH
//! when pointed at it (`tests/openssh.rs`), and the app's own Swift for
//! fingerprints, key detection, key import and the exec upload
//! (`harness/`).

mod auth;
mod channel;
mod connection;
mod dial;
mod error;
#[doc(hidden)]
pub mod harness;
pub mod host_key;
pub mod keyfile;
mod questions;
mod sftp;
pub mod shell;
mod target;
mod upload;
mod waiting;

pub use auth::{
    Auth, HardwareKey, HostSignature, KeyInfo, KeyKind, KeySigner, inspect_private_key,
    public_key_blob,
};
pub use channel::{Event, Exit, Output, Process, ProcessReader, ProcessWriter, PtyRequest, Stream};
pub use connection::{ConnectionEnd, HOST_KEY_ORDER, LinkFault, OLDER_HOST_KEY, Ssh};
pub use dial::unbracketed;
pub use error::{KeyError, ProxyError, SftpError, SignerError, Socks5Reply, SshError};
pub use host_key::{
    BoxFuture, Decision, DenyAll, HostKey, HostKeyPolicy, HostKeyPrompt, HostKeyRefusal,
    KnownHosts, KnownHostsStore, MemoryKnownHosts, StoreError, TrustOnFirstUse,
};
pub use questions::{SignInQuestion, SignInQuestions, reads_as_password};
pub use russh::Sig;
pub use sftp::{RemoteEntry, RemoteFile, Sftp};
pub use target::{ConnectOptions, Keepalive, RECEIVE_WINDOW, Socks5Proxy, Target};
/// How a caller gives up on an operation in flight ([`Sftp::cancelled_by`],
/// [`Ssh::run_until`], [`Ssh::upload_to_uploads_directory_until`]).
pub use tokio_util::sync::CancellationToken;
pub use upload::{ExecUploadPlan, quote};
pub use waiting::Waiting;
