//! Who to connect to, through what, and how long to wait.

use std::time::Duration;

/// A host and the account on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub user: String,
    /// As the person typed it: the trusted-host map is keyed by it, never by
    /// what it resolves to.
    pub host: String,
    pub port: u16,
}

impl Target {
    pub fn new(user: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            user: user.into(),
            host: host.into(),
            port,
        }
    }

    /// Parses `user@host[:port]`; the port defaults to 22. IPv6 hosts go in
    /// brackets: `user@[::1]:2222`.
    pub fn parse(spec: &str) -> Option<Self> {
        let (user, rest) = spec.split_once('@')?;
        if user.is_empty() || rest.is_empty() {
            return None;
        }
        let (host, port) = if let Some(rest) = rest.strip_prefix('[') {
            let (host, after) = rest.split_once(']')?;
            match after.strip_prefix(':') {
                Some(port) => (host, port.parse().ok()?),
                None if after.is_empty() => (host, 22),
                None => return None,
            }
        } else {
            match rest.rsplit_once(':') {
                Some((host, port)) => (host, port.parse().ok()?),
                None => (rest, 22),
            }
        };
        if host.is_empty() {
            return None;
        }
        Some(Self::new(user, host, port))
    }
}

/// A SOCKS5 proxy without authentication, for the TCP leg only: a mosh
/// session's UDP goes direct once SSH has started it, as it always has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socks5Proxy {
    pub host: String,
    pub port: u16,
}

impl Socks5Proxy {
    /// What the connection form assumes when no port is given.
    pub const DEFAULT_PORT: u16 = 1080;

    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

/// Keepalive probes on an idle connection: after `interval` without a word
/// from the server, ask; after `max_unanswered` unanswered asks, the
/// connection is lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keepalive {
    pub interval: Duration,
    pub max_unanswered: usize,
}

/// How a connection is made.
///
/// There is no reconnect here: a connection that ends stays ended, and the
/// session layer decides whether and when to dial again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectOptions {
    /// The TCP connect, to the host or to the proxy, plus the proxy's
    /// handshake. 15 s, as the app's `connectTimeout` has been: a SYN nobody
    /// answers in 15 s is a black hole, and the sooner it fails the sooner a
    /// retry lands on the path the person has since fixed.
    pub connect_timeout: Duration,
    /// The SSH negotiation and the sign-in, with the clock stopped while a
    /// person decides on a host key or a hardware key signs (a Face ID
    /// prompt).
    pub handshake_timeout: Duration,
    /// Opening a channel and the host's answer to its request (a command,
    /// a terminal, SFTP): 15 s, as Citadel bounded channel creation. A
    /// half-open connection fails here instead of hanging.
    pub channel_timeout: Duration,
    pub proxy: Option<Socks5Proxy>,
    /// None by default: the app has never sent SSH keepalives (the session
    /// layer probes with exec channels instead).
    pub keepalive: Option<Keepalive>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            handshake_timeout: Duration::from_secs(15),
            channel_timeout: Duration::from_secs(15),
            proxy: None,
            keepalive: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Target;

    #[test]
    fn target_parses_user_host_and_port() {
        assert_eq!(Target::parse("me@box"), Some(Target::new("me", "box", 22)));
        assert_eq!(
            Target::parse("me@box:2222"),
            Some(Target::new("me", "box", 2222))
        );
        assert_eq!(
            Target::parse("me@[::1]:2222"),
            Some(Target::new("me", "::1", 2222))
        );
        assert_eq!(
            Target::parse("me@[fe80::1]"),
            Some(Target::new("me", "fe80::1", 22))
        );
    }

    #[test]
    fn target_rejects_what_is_not_an_address() {
        for bad in [
            "box",
            "@box",
            "me@",
            "me@box:",
            "me@box:x",
            "me@[::1",
            "me@[::1]x",
            "me@box:70000",
        ] {
            assert_eq!(Target::parse(bad), None, "{bad}");
        }
    }
}
