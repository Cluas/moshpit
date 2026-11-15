//! The TCP leg: straight to the host, or through a SOCKS5 proxy.
//!
//! A name that resolves to several addresses is dialled the way NIO's
//! `ClientBootstrap` did for the app (Happy Eyeballs, RFC 8305): the next
//! address starts 250 ms after the previous one if it hasn't connected yet,
//! families alternating, and the first to connect wins. Trying them one
//! after another would leave a host with a dead IPv6 route to the 15 s
//! timeout while its IPv4 address answers at once.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use crate::error::{ProxyError, Socks5Reply, SshError};
use crate::target::{ConnectOptions, Socks5Proxy};

/// How long one address gets before the next one starts.
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// A TCP stream to `host:port`, through the proxy when there is one.
pub(crate) async fn dial(
    host: &str,
    port: u16,
    options: &ConnectOptions,
) -> Result<TcpStream, SshError> {
    let timeout = options.connect_timeout;
    let attempt = async {
        match &options.proxy {
            None => connect_any(host, port)
                .await
                .map_err(|source| SshError::Unreachable {
                    host: host.to_owned(),
                    port,
                    source,
                }),
            Some(proxy) => {
                let mut stream = connect_any(&proxy.host, proxy.port)
                    .await
                    .map_err(|source| SshError::Unreachable {
                        host: proxy.host.clone(),
                        port: proxy.port,
                        source,
                    })?;
                socks5_connect(&mut stream, host, port)
                    .await
                    .map_err(|reason| SshError::Proxy {
                        proxy: format!("{}:{}", proxy.host, proxy.port),
                        reason,
                    })?;
                Ok(stream)
            }
        }
    };
    let stream = tokio::time::timeout(timeout, attempt).await.map_err(|_| {
        let (host, port) = match &options.proxy {
            Some(Socks5Proxy { host, port }) => (host.clone(), *port),
            None => (host.to_owned(), port),
        };
        SshError::ConnectTimeout {
            host,
            port,
            timeout,
        }
    })??;
    if let Err(e) = stream.set_nodelay(true) {
        log::debug!("set_nodelay failed: {e}");
    }
    Ok(stream)
}

/// Connects to the first of `host`'s addresses that answers. An IPv6
/// address in brackets (`[2001:db8::1]`, as a URL writes it) is the
/// address: the name lookup takes none (B562).
async fn connect_any(host: &str, port: u16) -> io::Result<TcpStream> {
    connect_first(interleave(
        tokio::net::lookup_host((unbracketed(host), port))
            .await?
            .collect(),
    ))
    .await
}

/// `host` without the brackets around an IPv6 address.
pub fn unbracketed(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

/// Races the addresses in order, each starting `ATTEMPT_DELAY` after the
/// one before (at once when the one before failed).
async fn connect_first(addresses: Vec<SocketAddr>) -> io::Result<TcpStream> {
    race(addresses, TcpStream::connect).await
}

/// [`connect_first`] over any way of connecting, for the tests.
async fn race<T, F, Fut>(addresses: Vec<SocketAddr>, connect: F) -> io::Result<T>
where
    T: Send + 'static,
    F: Fn(SocketAddr) -> Fut,
    Fut: std::future::Future<Output = io::Result<T>> + Send + 'static,
{
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the name resolved to no address",
        ));
    }
    let mut attempts = JoinSet::new();
    let mut pending = addresses.into_iter();
    let mut last_error = None;
    loop {
        if let Some(address) = pending.next() {
            attempts.spawn(connect(address));
        }
        let more_to_start = pending.len() > 0;
        // Wait for an answer, but only ATTEMPT_DELAY when another address
        // is waiting its turn.
        let next = if more_to_start {
            match tokio::time::timeout(ATTEMPT_DELAY, attempts.join_next()).await {
                Ok(next) => next,
                Err(_) => continue,
            }
        } else {
            attempts.join_next().await
        };
        match next {
            Some(Ok(Ok(connected))) => return Ok(connected),
            Some(Ok(Err(e))) => last_error = Some(e),
            Some(Err(join)) => last_error = Some(io::Error::other(join.to_string())),
            None if !more_to_start => {
                return Err(last_error.unwrap_or_else(|| io::Error::other("no address answered")));
            }
            None => {}
        }
    }
}

/// Alternates address families, keeping each family's order and starting
/// with the family of the first address (RFC 8305 §4).
fn interleave(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first) = addresses.first() else {
        return addresses;
    };
    let first_is_v6 = first.is_ipv6();
    let (mut lead, mut other): (Vec<_>, Vec<_>) = addresses
        .into_iter()
        .partition(|a| a.is_ipv6() == first_is_v6);
    let mut out = Vec::with_capacity(lead.len() + other.len());
    lead.reverse();
    other.reverse();
    loop {
        match (lead.pop(), other.pop()) {
            (None, None) => return out,
            (a, b) => out.extend(a.into_iter().chain(b)),
        }
    }
}

/// The SOCKS5 handshake without authentication (RFC 1928), on a stream
/// already connected to the proxy: the target as a domain name, which the
/// proxy resolves (a `.local` or MagicDNS name the phone can't resolve may
/// be one the proxy can), or as an address when it is one.
pub(crate) async fn socks5_connect(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
) -> Result<(), ProxyError> {
    let io = |_: io::Error| ProxyError::Closed;

    // Greeting: version 5, one method, "no authentication".
    stream.write_all(&[5, 1, 0]).await.map_err(io)?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await.map_err(io)?;
    match choice {
        [5, 0] => {}
        [5, 0xff] => return Err(ProxyError::AuthRequired),
        [5, _] => return Err(ProxyError::AuthRequired),
        _ => return Err(ProxyError::NotSocks5),
    }

    // CONNECT.
    let mut request = vec![5, 1, 0];
    let bare = unbracketed(host);
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            request.push(1);
            request.extend_from_slice(&v4.octets());
        }
        Ok(IpAddr::V6(v6)) => {
            request.push(4);
            request.extend_from_slice(&v6.octets());
        }
        Err(_) => {
            let name = host.as_bytes();
            let len = u8::try_from(name.len()).map_err(|_| ProxyError::NameTooLong)?;
            request.push(3);
            request.push(len);
            request.extend_from_slice(name);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.map_err(io)?;

    // Reply: version, code, reserved, then the bound address we don't need.
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(io)?;
    if head[0] != 5 {
        return Err(ProxyError::NotSocks5);
    }
    if head[1] != 0 {
        return Err(ProxyError::Refused(Socks5Reply(head[1])));
    }
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(io)?;
            len[0] as usize
        }
        _ => return Err(ProxyError::NotSocks5),
    };
    let mut bound = vec![0u8; skip + 2];
    stream.read_exact(&mut bound).await.map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn families_alternate_from_the_first() {
        let mixed = vec![
            a("[2001:db8::1]:22"),
            a("[2001:db8::2]:22"),
            a("192.0.2.1:22"),
            a("[2001:db8::3]:22"),
            a("192.0.2.2:22"),
        ];
        assert_eq!(
            interleave(mixed),
            vec![
                a("[2001:db8::1]:22"),
                a("192.0.2.1:22"),
                a("[2001:db8::2]:22"),
                a("192.0.2.2:22"),
                a("[2001:db8::3]:22"),
            ]
        );
        assert_eq!(
            interleave(vec![a("192.0.2.1:22"), a("[::1]:22")]),
            vec![a("192.0.2.1:22"), a("[::1]:22")]
        );
        assert!(interleave(vec![]).is_empty());
    }

    /// `localhost` is `::1` and `127.0.0.1` here; only the IPv4 address
    /// listens, so whichever comes first, the connection lands.
    #[tokio::test]
    async fn an_address_in_brackets_is_dialled_as_the_address() {
        // B562: `[::1]` resolves to nothing; `::1` is the address.
        let Ok(live) = tokio::net::TcpListener::bind("[::1]:0").await else {
            eprintln!("skipped: no IPv6 loopback here");
            return;
        };
        let port = live.local_addr().unwrap().port();
        let stream = connect_any("[::1]", port).await.unwrap();
        assert_eq!(stream.peer_addr().unwrap().port(), port);
    }

    #[tokio::test]
    async fn the_address_that_answers_wins() {
        let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = live.local_addr().unwrap().port();
        let stream = connect_any("localhost", port).await.unwrap();
        assert_eq!(stream.peer_addr().unwrap().port(), port);
    }

    /// A fake network: each address answers after its delay, or fails, or
    /// never answers.
    fn network(
        answers: &[(&str, Option<u64>, bool)],
    ) -> impl Fn(
        SocketAddr,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = io::Result<SocketAddr>> + Send>,
    > {
        let answers: Vec<(SocketAddr, Option<u64>, bool)> = answers
            .iter()
            .map(|(addr, delay, ok)| (a(addr), *delay, *ok))
            .collect();
        move |address| {
            let (_, delay, ok) = *answers.iter().find(|(a, _, _)| *a == address).unwrap();
            Box::pin(async move {
                match delay {
                    None => std::future::pending().await,
                    Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                }
                if ok {
                    Ok(address)
                } else {
                    Err(io::Error::from(io::ErrorKind::ConnectionRefused))
                }
            })
        }
    }

    /// An address that never answers (a black-holed route) holds the next
    /// one up for the attempt delay, no longer.
    #[tokio::test(start_paused = true)]
    async fn a_black_hole_first_holds_up_the_next_only_briefly() {
        let start = tokio::time::Instant::now();
        let winner = race(
            vec![a("[2001:db8::1]:22"), a("192.0.2.1:22")],
            network(&[
                ("[2001:db8::1]:22", None, true),
                ("192.0.2.1:22", Some(30), true),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(winner, a("192.0.2.1:22"));
        assert_eq!(start.elapsed(), ATTEMPT_DELAY + Duration::from_millis(30));
    }

    /// A refused address hands over at once.
    #[tokio::test(start_paused = true)]
    async fn a_refusal_starts_the_next_at_once() {
        let start = tokio::time::Instant::now();
        let winner = race(
            vec![a("[::1]:22"), a("127.0.0.1:22")],
            network(&[
                ("[::1]:22", Some(5), false),
                ("127.0.0.1:22", Some(5), true),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(winner, a("127.0.0.1:22"));
        assert_eq!(start.elapsed(), Duration::from_millis(10));
    }

    /// The first to connect wins, even if it started later.
    #[tokio::test(start_paused = true)]
    async fn the_fastest_attempt_wins() {
        let winner = race(
            vec![a("[::1]:22"), a("127.0.0.1:22")],
            network(&[
                ("[::1]:22", Some(2000), true),
                ("127.0.0.1:22", Some(10), true),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(winner, a("127.0.0.1:22"));
    }

    #[tokio::test(start_paused = true)]
    async fn when_every_address_fails_the_last_error_says_why() {
        let err = race(
            vec![a("[::1]:22"), a("127.0.0.1:22")],
            network(&[
                ("[::1]:22", Some(5), false),
                ("127.0.0.1:22", Some(5), false),
            ]),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    }
}
