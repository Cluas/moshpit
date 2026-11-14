//! What the platform gives the sessions: credentials, the trusted host
//! keys, a little memory per connection and the UDP network.

use std::sync::Arc;

use moshpit_install::HostCapabilities;
use moshpit_ssh::{Auth, KnownHosts};
use moshpit_tmux::model::Selection;

use crate::mosh_udp::{BoxFuture, SystemUdp, UdpNetwork};

/// Why the platform has no credential to give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// The device is locked: the keychain item can't be read until it is
    /// unlocked (a launch before the first unlock). The attempt fails
    /// retryably and the session redials on the unlock (B201).
    Locked,
    /// The person cancelled (Face ID, the passphrase card).
    Cancelled,
    /// Nothing usable: the item is gone (a re-install), unreadable, or not a
    /// key this build can sign in with. For the log; never shown.
    Unavailable(String),
}

/// How to sign in to a connection.
pub trait Credentials: Send + Sync {
    /// The password or key from the keychain (a biometric prompt may come
    /// first), or the hardware key's signer. Called once per attempt; an
    /// error fails the attempt with [`crate::FailureKind::Credentials`], or
    /// [`crate::FailureKind::CredentialsLocked`] for a locked device.
    fn auth<'a>(&'a self, connection_id: &'a str) -> BoxFuture<'a, Result<Auth, CredentialError>>;
}

/// What a session remembers about a connection between attempts and app
/// launches — the slice of the store (`crates/store`, W5) it needs, as a
/// trait until that crate lands.
pub trait SessionMemory: Send + Sync {
    /// Where the person was in tmux (`TmuxSelectionStore`): a reconnect, a
    /// relaunch, a protocol switch lands back there.
    fn tmux_selection(&self, connection_id: &str) -> Option<Selection> {
        let _ = connection_id;
        None
    }

    fn save_tmux_selection(&self, connection_id: &str, selection: &Selection) {
        let _ = (connection_id, selection);
    }

    /// What the host could run when last probed (`HostCapabilityCache`):
    /// a known host is decided on what is known and re-probed behind it.
    fn capabilities(&self, connection_id: &str) -> Option<HostCapabilities> {
        let _ = connection_id;
        None
    }

    fn save_capabilities(&self, connection_id: &str, capabilities: &HostCapabilities) {
        let _ = (connection_id, capabilities);
    }
}

/// Remembers nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMemory;

impl SessionMemory for NoMemory {}

/// Everything the sessions use from the platform.
#[derive(Clone)]
pub struct Services {
    pub credentials: Arc<dyn Credentials>,
    /// The trusted host keys (the keychain map, `crates/ssh`).
    pub known_hosts: Arc<KnownHosts>,
    pub memory: Arc<dyn SessionMemory>,
    pub udp: Arc<dyn UdpNetwork>,
    /// What tmux's size leases call this device ("iPhone", "iPad").
    pub device_label: String,
}

impl Services {
    /// The system's UDP, no memory.
    pub fn new(credentials: Arc<dyn Credentials>, known_hosts: Arc<KnownHosts>) -> Self {
        Self {
            credentials,
            known_hosts,
            memory: Arc::new(NoMemory),
            udp: Arc::new(SystemUdp),
            device_label: "iPhone".to_owned(),
        }
    }
}
