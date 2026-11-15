//! The TCP leg through a SOCKS5 proxy without authentication: host names
//! resolved by the proxy, addresses as addresses, and every way a proxy can
//! say no.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use moshpit_ssh::{
    Auth, ConnectOptions, KnownHosts, ProxyError, Socks5Proxy, Socks5Reply, Ssh, SshError, Target,
    TrustOnFirstUse,
};
use support::*;

fn via(proxy: &Proxy) -> ConnectOptions {
    ConnectOptions {
        proxy: Some(Socks5Proxy::new("127.0.0.1", proxy.port)),
        ..options()
    }
}

async fn connect_via(target: &Target, options: &ConnectOptions) -> Result<Ssh, SshError> {
    Ssh::connect(
        target,
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        options,
    )
    .await
}

#[tokio::test]
async fn a_host_name_is_resolved_by_the_proxy() {
    let host = Host::start(Bench::default()).await;
    let proxy = Proxy::start(ProxyMode::Tunnel).await;
    let target = Target::new(USER, "localhost", host.port());
    let ssh = connect_via(&target, &via(&proxy)).await.unwrap();
    assert!(ssh.run("echo through").await.unwrap().success());
    assert_eq!(
        *proxy.targets.lock().unwrap(),
        vec![(3, "localhost".to_owned(), host.port())]
    );
}

#[tokio::test]
async fn an_address_goes_as_an_address() {
    let host = Host::start(Bench::default()).await;
    let proxy = Proxy::start(ProxyMode::Tunnel).await;
    connect_via(&host.target(), &via(&proxy)).await.unwrap();
    assert_eq!(
        *proxy.targets.lock().unwrap(),
        vec![(1, "127.0.0.1".to_owned(), host.port())]
    );
}

/// Where a mosh session's datagrams go: the address a direct connection
/// reached; through a proxy that address is the proxy's, so none.
#[tokio::test]
async fn the_peer_address_is_known_only_when_direct() {
    let host = Host::start(Bench::default()).await;
    let direct = connect(&host).await;
    assert_eq!(
        direct.peer_addr(),
        Some(std::net::SocketAddr::from(([127, 0, 0, 1], host.port())))
    );
    let proxy = Proxy::start(ProxyMode::Tunnel).await;
    let proxied = connect_via(&host.target(), &via(&proxy)).await.unwrap();
    assert_eq!(proxied.peer_addr(), None);
}

#[tokio::test]
async fn the_host_key_is_the_hosts_not_the_proxys() {
    let host = Host::start(Bench::default()).await;
    let proxy = Proxy::start(ProxyMode::Tunnel).await;
    let ssh = connect_via(&host.target(), &via(&proxy)).await.unwrap();
    assert_eq!(ssh.host_key(), &host.host_keys[0]);
}

/// Signing in through the proxy ends as it ends directly: the right
/// password runs a command, a wrong one is a failed sign-in, and a host key
/// the person won't trust is that refusal, with nothing trusted — never an
/// unreachable server (the Swift transport's SOCKSSignInTests, gone with it).
#[tokio::test]
async fn signing_in_through_the_proxy_ends_as_it_ends_directly() {
    let host = Host::start(Bench::default()).await;
    let proxy = Proxy::start(ProxyMode::Tunnel).await;
    let sign_in = |auth: Auth| {
        let target = host.target();
        let options = via(&proxy);
        async move { Ssh::connect(&target, auth, Arc::new(AcceptAny), &options).await }
    };

    let ssh = sign_in(Auth::password(PASSWORD)).await.unwrap();
    let out = ssh.run("echo hello").await.unwrap();
    assert_eq!(out.stdout(), b"hello\n");
    assert!(out.success());

    let wrong = sign_in(Auth::password("hunter2")).await.err().unwrap();
    assert!(matches!(wrong, SshError::AuthFailed { .. }), "{wrong:?}");

    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let declined = Ssh::connect(
        &host.target(),
        Auth::password(PASSWORD),
        Arc::new(TrustOnFirstUse::new(
            known.clone(),
            Person::answering(&[false]),
        )),
        &via(&proxy),
    )
    .await
    .err()
    .unwrap();
    assert!(
        matches!(declined, SshError::HostKeyNotTrusted { .. }),
        "{declined:?}"
    );
    assert!(known.snapshot().is_empty());
}

async fn refused(mode: ProxyMode) -> ProxyError {
    let host = Host::start(Bench::default()).await;
    let proxy = Proxy::start(mode).await;
    match connect_via(&host.target(), &via(&proxy))
        .await
        .err()
        .unwrap()
    {
        SshError::Proxy {
            proxy: name,
            reason,
        } => {
            assert_eq!(name, format!("127.0.0.1:{}", proxy.port));
            reason
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_proxy_that_refuses_the_tunnel_says_why() {
    assert_eq!(
        refused(ProxyMode::Refuse(5)).await,
        ProxyError::Refused(Socks5Reply(5))
    );
    assert_eq!(Socks5Reply(5).to_string(), "connection refused");
    assert_eq!(
        refused(ProxyMode::Refuse(2)).await,
        ProxyError::Refused(Socks5Reply(2))
    );
}

#[tokio::test]
async fn a_proxy_that_wants_a_password_is_refused() {
    assert_eq!(
        refused(ProxyMode::WantsPassword).await,
        ProxyError::AuthRequired
    );
}

#[tokio::test]
async fn a_proxy_that_is_not_socks5_is_named() {
    assert_eq!(refused(ProxyMode::NotSocks).await, ProxyError::NotSocks5);
}

/// The connect timeout covers the proxy's handshake too: a proxy that takes
/// the connection and never answers is a black hole like any other (NIO's
/// SOCKS handler waited forever).
#[tokio::test]
async fn a_silent_proxy_times_out() {
    let host = Host::start(Bench::default()).await;
    let proxy = Proxy::start(ProxyMode::Silent).await;
    let options = ConnectOptions {
        connect_timeout: Duration::from_millis(300),
        ..via(&proxy)
    };
    let err = connect_via(&host.target(), &options).await.err().unwrap();
    match err {
        SshError::ConnectTimeout { host, port, .. } => {
            assert_eq!((host.as_str(), port), ("127.0.0.1", proxy.port));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_unreachable_proxy_is_named() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let options = ConnectOptions {
        proxy: Some(Socks5Proxy::new("127.0.0.1", port)),
        ..options()
    };
    let err = connect_via(&Target::new(USER, "example.com", 22), &options)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(err, SshError::Unreachable { port: p, .. } if p == port),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_name_too_long_for_socks5_is_refused_before_sending() {
    let proxy = Proxy::start(ProxyMode::Tunnel).await;
    let name = format!("{}.example", "a".repeat(250));
    let err = connect_via(&Target::new(USER, name, 22), &via(&proxy))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            SshError::Proxy {
                reason: ProxyError::NameTooLong,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(proxy.targets.lock().unwrap().is_empty());
}
