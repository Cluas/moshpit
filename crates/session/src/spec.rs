//! What a session connects to, and the clock it keeps: the connection as
//! the app stores it (only the fields a session reads), and the timings the
//! lifecycle runs on.

use std::time::Duration;

use moshpit_install::Multiplexer;
use moshpit_mosh::TransportConfig;
use moshpit_ssh::{ConnectOptions, Socks5Proxy, Target};

/// The transport a connection asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Ssh,
    /// mosh: SSH starts `mosh-server`, then UDP. Falls back to SSH on a host
    /// without `mosh-server`.
    Mosh,
}

/// How a connection reaches its host (F1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Hop {
    /// Straight to the host (through [`ConnectionSpec::proxy`], if set).
    #[default]
    Direct,
    /// Through another saved connection's SSH (`ssh -J`): only ever SSH
    /// (mosh's UDP can't take the way), and [`ConnectionSpec::proxy`]
    /// doesn't apply (the jump host's own does, to its leg).
    Through(Box<Via>),
    /// Through a saved connection that can't serve: the session fails at
    /// once and never dials the host directly instead — a jump host is
    /// often the only way in a network allows.
    Broken(HopBroken),
}

impl Hop {
    pub fn is_direct(&self) -> bool {
        matches!(self, Hop::Direct)
    }
}

/// The jump host a connection goes through: another saved connection,
/// signed in to with its own credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Via {
    /// Its connection id: what its credentials are read by.
    pub connection_id: String,
    /// What the person calls it ("bastion"): what a failure on its leg is
    /// told by ([`crate::Failure::leg`]).
    pub name: String,
    pub target: Target,
    /// Its own SOCKS proxy, for its leg.
    pub proxy: Option<Socks5Proxy>,
}

/// Why a connection's way through another can't be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopBroken {
    /// The connection it went through was deleted.
    Deleted,
    /// More than one hop: the connection it goes through goes through
    /// another itself, or it names itself.
    Chained,
    /// It goes through another, but the dial wasn't told which (a caller
    /// that left the way out): refused, never dialled direct instead.
    Unresolved,
}

/// One saved connection, as far as a session is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSpec {
    /// The app's connection id (`ServerConnection.id`): what the stored
    /// tmux selection, the cached host capabilities and the mosh renderer's
    /// pidfiles are keyed by.
    pub connection_id: String,
    pub target: Target,
    pub proxy: Option<Socks5Proxy>,
    /// Straight there or through another connection's SSH (F1).
    pub hop: Hop,
    pub protocol: Protocol,
    pub multiplexer: Multiplexer,
    /// The tmux command, as typed in the form (a path, or a path with
    /// arguments). `None`: `tmux` on the host's `PATH`. Set, the person
    /// vouches for it: the session doesn't probe for tmux.
    pub tmux_path: Option<String>,
    /// `mosh-server`'s path; set, the host isn't probed for it.
    pub mosh_server_path: Option<String>,
    /// mosh-server's UDP port range (`-p a[:b]`).
    pub mosh_port_range: (Option<i64>, Option<i64>),
    /// herdr's path; set, the host isn't probed for it.
    pub herdr_path: Option<String>,
    /// The locale mosh-server starts the shell in (`-l LANG=…`).
    pub locale: String,
    /// The terminal's grid before the view has reported one: the first PTY
    /// and the first `refresh-client -C` are phone-sized, not 80×24.
    pub initial_grid: (u16, u16),
    /// What the terminal runs instead of the login shell (the PTY's command;
    /// mosh-server's `-- command`). `None` in the app: the login shell, as
    /// the person set it up. Tests use `/bin/sh` so the bench doesn't
    /// depend on anyone's dotfiles.
    #[doc(hidden)]
    pub pty_command: Option<String>,
    /// This phone connected to the host before (the connection's
    /// `lastConnectedAt`, B783): a new host's card says so instead of
    /// "First connection" — its trusted key gone with a restore.
    pub connected_before: bool,
}

impl ConnectionSpec {
    /// A plain SSH connection to `target` with the app's defaults.
    pub fn ssh(connection_id: impl Into<String>, target: Target) -> Self {
        Self {
            connection_id: connection_id.into(),
            target,
            proxy: None,
            hop: Hop::Direct,
            protocol: Protocol::Ssh,
            multiplexer: Multiplexer::None,
            tmux_path: None,
            mosh_server_path: None,
            mosh_port_range: (None, None),
            herdr_path: None,
            locale: "en_US.UTF-8".to_owned(),
            initial_grid: (80, 24),
            pty_command: None,
            connected_before: false,
        }
    }

    /// The transport a dial asks for: [`Self::protocol`], except that a
    /// connection through another is SSH only (F1).
    pub fn dialled_protocol(&self, protocol: Protocol) -> Protocol {
        if self.hop.is_direct() {
            protocol
        } else {
            Protocol::Ssh
        }
    }

    /// The tmux command lines are built with: the custom path or `tmux`.
    pub fn tmux_command(&self) -> &str {
        self.tmux_path.as_deref().unwrap_or("tmux")
    }

    /// Whether a multiplexer is vouched for by a custom path in the form
    /// (`connection.multiplexerPath != nil`): then the probe's verdict on
    /// it doesn't count.
    pub fn vouches_for(&self, multiplexer: Multiplexer) -> bool {
        match multiplexer {
            Multiplexer::None => true,
            Multiplexer::Tmux => self.tmux_path.is_some(),
            Multiplexer::Herdr => self.herdr_path.is_some(),
        }
    }
}

/// The lifecycle's clock. [`Default`] is the app's.
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    /// The TCP leg of a dial, to the host or the proxy (15 s): a SYN nobody
    /// answers in 15 s is a black hole (a VPN address with the tunnel down),
    /// and the sooner it fails the sooner a retry lands on the path the
    /// person has since fixed.
    pub dial_timeout: Duration,
    /// The SSH negotiation and sign-in, not counting the time a person
    /// spends on a host-key card or a Face ID prompt.
    pub handshake_timeout: Duration,
    /// Opening a channel and the host's answer to it.
    pub channel_timeout: Duration,
    /// `mosh-server new` over the bootstrap connection, start to `MOSH
    /// CONNECT`: a shell startup file that waits for input must not leave
    /// the session connecting forever.
    pub bootstrap_timeout: Duration,
    /// The keepalive tick while in the foreground (12 s).
    pub keepalive_interval: Duration,
    /// Bytes this recent are proof of life: no probe (11 s, just under the
    /// tick, so a connection that goes quiet is probed on the next one).
    pub inbound_proof: Duration,
    /// A keepalive probe's deadline (8 s: three round trips of a 700 ms path
    /// plus a retransmit).
    pub probe_timeout: Duration,
    /// The foreground return's probe deadline (4 s): a socket held through a
    /// suspension is dead far more often, and the person is watching.
    pub foreground_probe_timeout: Duration,
    /// Silent keepalive probes in a row before the connection is declared
    /// lost: one silence on a lossy link is routine; an error is proof.
    pub probe_timeouts_to_drop: u32,
    /// Longer than this in the background and the return reconnects
    /// without asking the socket (iOS has almost certainly killed it).
    pub force_reconnect_after: Duration,
    /// How long a dial must have been out before a foreground return
    /// restarts it (2 s): a glance at another app must not cost a handshake.
    pub stalled_dial_grace: Duration,
    /// How old a dial must be for a network change to restart it (1 s): a
    /// younger one most likely began on the new path.
    pub path_redial_after: Duration,
    /// A burst of path changes (a tunnel coming up) is acted on once, this
    /// long after the last.
    pub path_settle: Duration,
    /// A tmux attach that hasn't confirmed this long after its boot line
    /// is reported stalled (22 s).
    pub attach_stall: Duration,
    /// The mosh transport's own timers (the 8 s return-path watchdog, the
    /// 3 s liveness check, …). Keystrokes never expire here (B618): kept
    /// until the server acks them, as mosh-client keeps them, however long
    /// the silence; a flow replaced by another server says what it never
    /// delivered.
    pub mosh: TransportConfig,
    /// The way back from the background without a blind redial (off: the
    /// app's rule, `force_reconnect_after`). On, a live SSH transport is
    /// asked first however long the app was away — one round trip on the
    /// leg itself (tmux's control channel; the SSH keepalive for a shell or
    /// herdr) within [`Timing::resume_probe_timeout`] — and kept when it
    /// answers: nothing on screen changes, no Face ID, no handshake.
    /// Keystrokes typed meanwhile wait for the answer instead of going into
    /// a socket that may be dead. Only an answer that never comes, or a
    /// failure, reconnects.
    pub resume_in_place: bool,
    /// How long the in-place return waits for that round trip (1.5 s: a
    /// slow mobile path's RTT plus a radio waking up). A socket iOS closed
    /// fails at once; this is what a silent one costs before the redial.
    pub resume_probe_timeout: Duration,
}

/// A keystroke lifetime no silence reaches (B618).
pub const KEYS_NEVER_EXPIRE: u64 = u64::MAX;

impl Default for Timing {
    fn default() -> Self {
        Self {
            dial_timeout: Duration::from_secs(15),
            handshake_timeout: Duration::from_secs(15),
            channel_timeout: Duration::from_secs(15),
            bootstrap_timeout: Duration::from_secs(15),
            keepalive_interval: Duration::from_secs(12),
            inbound_proof: Duration::from_secs(11),
            probe_timeout: Duration::from_secs(8),
            foreground_probe_timeout: Duration::from_secs(4),
            probe_timeouts_to_drop: 2,
            force_reconnect_after: Duration::from_secs(20),
            stalled_dial_grace: Duration::from_secs(2),
            path_redial_after: Duration::from_secs(1),
            path_settle: Duration::from_millis(400),
            attach_stall: Duration::from_secs(22),
            mosh: TransportConfig {
                keystroke_ttl_ms: KEYS_NEVER_EXPIRE,
                ..TransportConfig::default()
            },
            resume_in_place: false,
            resume_probe_timeout: Duration::from_millis(1_500),
        }
    }
}

impl Timing {
    /// How the SSH leg of an attempt is dialled: the dial timeout on the
    /// TCP leg, no SSH-level keepalives (the session probes with exec
    /// channels, as the app always has).
    pub fn connect_options(&self, proxy: Option<Socks5Proxy>) -> ConnectOptions {
        ConnectOptions {
            connect_timeout: self.dial_timeout,
            handshake_timeout: self.handshake_timeout,
            channel_timeout: self.channel_timeout,
            proxy,
            keepalive: None,
            receive_window: moshpit_ssh::RECEIVE_WINDOW,
        }
    }
}

pub(crate) fn ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dial_gives_a_silent_host_fifteen_seconds() {
        let timing = Timing::default();
        let options = timing.connect_options(Some(Socks5Proxy::new("proxy", 1080)));
        assert_eq!(options.connect_timeout, Duration::from_secs(15));
        assert_eq!(options.handshake_timeout, Duration::from_secs(15));
        assert_eq!(options.channel_timeout, Duration::from_secs(15));
        assert_eq!(options.keepalive, None);
        assert_eq!(options.proxy, Some(Socks5Proxy::new("proxy", 1080)));
    }

    #[test]
    fn the_app_timings() {
        let timing = Timing::default();
        assert_eq!(timing.keepalive_interval, Duration::from_secs(12));
        assert_eq!(timing.inbound_proof, Duration::from_secs(11));
        assert_eq!(timing.probe_timeout, Duration::from_secs(8));
        assert_eq!(timing.foreground_probe_timeout, Duration::from_secs(4));
        assert_eq!(timing.probe_timeouts_to_drop, 2);
        assert_eq!(timing.force_reconnect_after, Duration::from_secs(20));
        assert_eq!(timing.stalled_dial_grace, Duration::from_secs(2));
        assert_eq!(timing.path_redial_after, Duration::from_secs(1));
        assert_eq!(timing.path_settle, Duration::from_millis(400));
        assert_eq!(timing.attach_stall, Duration::from_secs(22));
        assert_eq!(timing.mosh.return_path_deadline_ms, 8_000);
        assert!(!timing.resume_in_place, "the prototype is off by default");
        assert_eq!(timing.resume_probe_timeout, Duration::from_millis(1_500));
    }

    #[test]
    fn a_custom_path_vouches_for_its_multiplexer() {
        let mut spec = ConnectionSpec::ssh("c", Target::new("me", "box", 22));
        assert!(!spec.vouches_for(Multiplexer::Tmux));
        assert_eq!(spec.tmux_command(), "tmux");
        spec.tmux_path = Some("/opt/tmux/bin/tmux".into());
        assert!(spec.vouches_for(Multiplexer::Tmux));
        assert!(!spec.vouches_for(Multiplexer::Herdr));
        assert!(spec.vouches_for(Multiplexer::None));
        assert_eq!(spec.tmux_command(), "/opt/tmux/bin/tmux");
    }
}
