//! Why a connection, a sign-in or a channel failed.
//!
//! The variants follow what the app tells a person today (`SSHError` in
//! `Moshpit/Services/SSHService.swift`): unreachable, handshake, host key,
//! credentials, the server refusing a channel. Each keeps what a log needs;
//! none carries a password or a key.

use std::fmt;
use std::time::Duration;

/// A connection, sign-in or channel failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SshError {
    /// Nothing answered: the name didn't resolve, the port refused, the
    /// network is unreachable. For a proxied connection, the proxy's address.
    #[error("could not reach {host}:{port}: {source}")]
    Unreachable {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },
    /// The TCP leg (with the proxy's handshake, when there is one) took longer
    /// than [`crate::ConnectOptions::connect_timeout`]: most often a VPN
    /// address with the tunnel down.
    #[error("no answer from {host}:{port} within {timeout:?}")]
    ConnectTimeout {
        host: String,
        port: u16,
        timeout: Duration,
    },
    /// The SOCKS5 proxy wouldn't open the tunnel.
    #[error("the SOCKS5 proxy {proxy} {reason}")]
    Proxy { proxy: String, reason: ProxyError },
    /// The jump host ([`crate::Ssh::connect_through`]) wouldn't open the
    /// way to the host: it couldn't reach it, or doesn't forward.
    #[error("the jump host {via} could not open the way to {host}:{port}: {reason}")]
    Tunnel {
        /// The jump host, as its connection names it.
        via: String,
        host: String,
        port: u16,
        reason: TunnelRefusal,
    },
    /// The SSH negotiation and sign-in took longer than
    /// [`crate::ConnectOptions::handshake_timeout`], not counting the time a
    /// person spent deciding on a host key or a hardware key spent signing.
    #[error("{host}:{port} stopped answering during the SSH handshake ({timeout:?})")]
    HandshakeTimeout {
        host: String,
        port: u16,
        timeout: Duration,
    },
    /// The other end isn't an SSH server, closed the connection while it was
    /// being set up, or shares no algorithm with us.
    #[error("the SSH handshake with {host}:{port} failed: {reason}")]
    Handshake {
        host: String,
        port: u16,
        reason: String,
    },
    /// The server and this client have no algorithm of a kind in common
    /// (key exchange, host key, cipher, MAC): a server that only offers an
    /// `ssh-rsa` host key, or SHA-1 key exchange. Nothing changes on a
    /// retry (B561).
    #[error("{host}:{port} offers no {what} algorithm this client uses (it offers {theirs:?})")]
    Incompatible {
        host: String,
        port: u16,
        /// "key exchange", "host key", "cipher", "MAC" or "compression".
        what: &'static str,
        /// What the server offered.
        theirs: Vec<String>,
    },
    /// A host this device never trusted, and the person (or the default
    /// policy) said no.
    #[error("the host key of {host}:{port} ({fingerprint}) was not trusted")]
    HostKeyNotTrusted {
        host: String,
        port: u16,
        fingerprint: String,
    },
    /// The host presents a key other than the one trusted for it. Refused
    /// unless the person accepted the change.
    #[error(
        "the host key of {host}:{port} changed: it presents {presented}, {expected} was trusted"
    )]
    HostKeyChanged {
        host: String,
        port: u16,
        presented: String,
        expected: String,
    },
    /// The trusted host keys can't be read until the device is unlocked, so
    /// this host's key could not be checked. Nothing was asked.
    #[error("the trusted host keys can't be read while the device is locked ({host}:{port})")]
    HostKeysLocked { host: String, port: u16 },
    /// The server turned the credentials down.
    #[error("{host} rejected the credentials for {user}")]
    AuthFailed {
        user: String,
        host: String,
        /// The methods the server would still take.
        remaining: Vec<String>,
    },
    /// The server doesn't take this kind of credential at all, e.g. a
    /// password on a key-only server.
    #[error("{host} does not accept {method} sign-in (it takes {offered:?})")]
    AuthMethodRejected {
        host: String,
        method: &'static str,
        offered: Vec<String>,
    },
    /// The server takes a password only by asking its own questions
    /// (keyboard-interactive), and asked something the saved password can't
    /// answer: a prompt shown as typed, or a second hidden one (a one-time
    /// code). Only the person could answer it (B393).
    #[error("{host} asks questions at sign-in that only a person can answer")]
    InteractivePrompts { host: String },
    /// The person declined what the server asked at sign-in (B563).
    #[error("signing in to {host} was declined at the server's question")]
    SignInDeclined { host: String },
    #[error(transparent)]
    Key(#[from] KeyError),
    /// The hardware key's signer failed or answered something unusable.
    #[error("the hardware key could not sign: {0}")]
    Signer(#[from] SignerError),
    /// The host refused a channel or a request on one: a session, a PTY, a
    /// command, a subsystem.
    #[error("{host} refused to {what}")]
    Refused { host: String, what: String },
    /// The host didn't answer a channel request in time (a half-open
    /// connection, most often).
    #[error("{host} didn't answer within {timeout:?} when asked to {what}")]
    Unresponsive {
        host: String,
        what: String,
        timeout: Duration,
    },
    /// The connection is gone.
    #[error("the connection to {host} is closed")]
    Closed { host: String },
    #[error("ssh: {0}")]
    Protocol(#[from] russh::Error),
}

/// Why a jump host refused a `direct-tcpip` channel (RFC 4254 §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TunnelRefusal {
    /// It doesn't forward (sshd's `AllowTcpForwarding no`, `PermitOpen`).
    Prohibited,
    /// It couldn't connect: the name doesn't resolve there, nothing
    /// listens, no route.
    ConnectFailed,
    /// Anything else it said, as it said it.
    Other(String),
}

impl fmt::Display for TunnelRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prohibited => f.write_str("it doesn't forward connections"),
            Self::ConnectFailed => f.write_str("it couldn't connect"),
            Self::Other(said) => f.write_str(said),
        }
    }
}

/// A private or public key that can't be used to sign in.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// Not an `openssh-key-v1` key. PEM, PKCS#8 and PuTTY keys are converted
    /// when the app imports them; what reaches a connection is OpenSSH's own.
    #[error("not an OpenSSH private key")]
    NotOpenSsh,
    /// Encrypted with a passphrase. The keychain holds keys decrypted; the
    /// app asks for the passphrase once, at import.
    #[error("the private key is encrypted")]
    Encrypted,
    #[error("the private key can't be read: {0}")]
    Malformed(String),
    /// A kind of key that can't sign in from here: security keys (`sk-`),
    /// DSA, or one this build doesn't know.
    #[error("{0} keys can't be used to sign in")]
    Unsupported(String),
    #[error("the public key can't be read: {0}")]
    BadPublicKey(String),
}

/// A hardware signer that couldn't produce a usable signature.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SignerError {
    /// What the platform said: the key is gone, unreadable.
    #[error("{0}")]
    Failed(String),
    /// The key can't be used yet: the device is locked (the Secure Enclave
    /// key's keychain item is kept until the first unlock, or behind Face ID
    /// where no prompt can show). The sign-in is worth trying again on the
    /// unlock (B227), as a keychain read refused for the lock is (B211).
    #[error("the hardware key can't be used while the device is locked")]
    Locked,
    /// The person cancelled (Face ID).
    #[error("signing was cancelled")]
    Cancelled,
    /// The bytes aren't a signature of the kind the key makes.
    #[error("the signature is malformed: {0}")]
    Malformed(String),
    /// A well-formed signature that doesn't verify against the public key
    /// the connection offered: the stored public half and the hardware key
    /// are not a pair.
    #[error("the signature does not match the public key")]
    Mismatch,
    /// Hardware RSA and security keys aren't wired up.
    #[error("{0} hardware keys are not supported")]
    Unsupported(String),
}

/// Why a SOCKS5 proxy didn't open the tunnel.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProxyError {
    /// The proxy wants a username and password; only proxies without
    /// authentication are supported, as before.
    #[error("requires a username and password")]
    AuthRequired,
    #[error("does not speak SOCKS5")]
    NotSocks5,
    #[error("refused the tunnel: {0}")]
    Refused(Socks5Reply),
    /// SOCKS5 carries at most 255 bytes of host name.
    #[error("can't carry a host name longer than 255 bytes")]
    NameTooLong,
    #[error("closed the connection during the handshake")]
    Closed,
}

/// A SOCKS5 reply code other than "succeeded" (RFC 1928 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socks5Reply(pub u8);

impl fmt::Display for Socks5Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self.0 {
            1 => "general failure",
            2 => "not allowed by its rules",
            3 => "network unreachable",
            4 => "host unreachable",
            5 => "connection refused",
            6 => "TTL expired",
            7 => "command not supported",
            8 => "address type not supported",
            _ => return write!(f, "reply code {}", self.0),
        };
        f.write_str(text)
    }
}

/// What an SFTP operation ran into.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SftpError {
    /// The host has no SFTP subsystem, or refused it: the caller falls back
    /// to exec (`RemoteFileCommands`, the exec upload).
    #[error("SFTP is not available: {0}")]
    Unavailable(String),
    #[error("no such file: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Something with the target name is already there.
    #[error("the name is taken: {0}")]
    NameTaken(String),
    /// Any other status the server answered with.
    #[error("the server refused: {message}")]
    Failure { message: String },
    /// The channel went away or a request went unanswered.
    #[error("the SFTP channel failed: {0}")]
    Transport(String),
    /// The progress callback asked to stop.
    #[error("cancelled")]
    Cancelled,
    /// The file changed size or time while it was being read (B814): what
    /// came down is neither version, and isn't handed out.
    #[error("the file changed while it was read: {0}")]
    Changed(String),
    /// The local side of a transfer.
    #[error("local file: {0}")]
    Local(#[from] std::io::Error),
    #[error(transparent)]
    Ssh(#[from] SshError),
}
