//! Whether the Local Network permission may be why a host didn't answer
//! (B740, B801): iOS asks on the first connect to a host on the local
//! network, and denied (or not yet answered) the connect fails as if the
//! host weren't there. One rule for the Home card and the app's alert (the
//! FFI's `local_network_may_be_why`), which had its own copy in Swift.

use std::net::{Ipv4Addr, Ipv6Addr};

use moshpit_session::UnreachableWhy;

/// Whether the permission may be why `host` didn't answer for `why`: a
/// refusal came from the host itself, so never; a name that resolved to
/// nothing only when it is a `.local` name (looked up on the local network
/// itself); anything else when the host is on the local network.
pub fn may_be_why(host: &str, why: UnreachableWhy) -> bool {
    match why {
        UnreachableWhy::Refused => false,
        UnreachableWhy::NoAddress => is_multicast_name(host),
        UnreachableWhy::TimedOut | UnreachableWhy::NoRoute | UnreachableWhy::Other => {
            on_local_network(host)
        }
    }
}

/// Whether `host`, as typed, is on the local network as iOS's permission
/// counts it: a private or link-local address (10/8, 172.16/12,
/// 192.168/16, 169.254/16, fe80::/10, fc00::/7), a `.local` name, or a
/// one-word name the network's own resolver answers. A tailnet address
/// (100.64/10), a public one or `localhost` goes out a way iOS doesn't ask
/// about.
pub fn on_local_network(host: &str) -> bool {
    let name = normalized(host);
    if let Ok(v4) = name.parse::<Ipv4Addr>() {
        let [a, b, ..] = v4.octets();
        return a == 10
            || (a == 192 && b == 168)
            || (a == 169 && b == 254)
            || (a == 172 && (16..=31).contains(&b));
    }
    if let Ok(v6) = name.parse::<Ipv6Addr>() {
        let first = v6.segments()[0];
        return first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00;
    }
    if is_multicast_name(&name) {
        return true;
    }
    !name.is_empty() && !name.contains('.') && !name.contains(':') && name != "localhost"
}

/// A name looked up with multicast DNS, on the local network itself.
pub fn is_multicast_name(host: &str) -> bool {
    normalized(host).ends_with(".local")
}

/// Lowercased, without the brackets, zone or trailing dot a host may be
/// typed with.
fn normalized(host: &str) -> String {
    let mut name = host.trim().to_lowercase();
    if name.starts_with('[') && name.ends_with(']') && name.len() >= 2 {
        name = name[1..name.len() - 1].to_owned();
    }
    if let Some(zone) = name.find('%') {
        name.truncate(zone);
    }
    while name.ends_with('.') {
        name.pop();
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts_are_the_ones_ios_asks_about() {
        for host in [
            "192.168.1.20",
            "10.0.0.5",
            "172.20.1.1",
            "169.254.3.4",
            "fe80::1",
            "[fe80::1%en0]",
            "fd12:3456::1",
            "studio.local",
            "Studio.Local.",
            "studio",
        ] {
            assert!(on_local_network(host), "{host}");
        }
        for host in [
            "203.0.113.5",
            "box.example.com",
            "100.64.0.7",
            "172.32.0.1",
            "2001:db8::1",
            "localhost",
            "",
        ] {
            assert!(!on_local_network(host), "{host}");
        }
    }

    #[test]
    fn the_permission_may_be_why_only_where_it_could_be() {
        for why in [
            UnreachableWhy::NoRoute,
            UnreachableWhy::TimedOut,
            UnreachableWhy::Other,
        ] {
            assert!(may_be_why("192.168.1.20", why), "{why:?}");
            assert!(!may_be_why("box.example.com", why), "{why:?}");
        }
        // The host answered: never the permission.
        assert!(!may_be_why("192.168.1.20", UnreachableWhy::Refused));
        assert!(!may_be_why("studio.local", UnreachableWhy::Refused));
        // A name that resolved to nothing: only a `.local` one.
        assert!(may_be_why("studio.local", UnreachableWhy::NoAddress));
        assert!(!may_be_why("studio", UnreachableWhy::NoAddress));
        assert!(!may_be_why("box.example.com", UnreachableWhy::NoAddress));
    }
}
