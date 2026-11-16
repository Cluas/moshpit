//! A host reached through another (`ssh -J`, F1): the target's SSH
//! session runs inside a `direct-tcpip` channel of the jump host's, the
//! target's name resolved by the jump host alone, each host's key checked
//! as its own; the jump host going takes the target with it, and the
//! target going alone leaves the jump host up for a redial through it.

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use moshpit_ssh::{
    Auth, ConnectionEnd, Event, Jump, KnownHosts, MemoryKnownHosts, Ssh, SshError, Target,
    TrustOnFirstUse, TunnelRefusal,
};
use support::*;

/// A name only the jump host can resolve: the client can't reach the
/// target but through it.
const BEHIND: &str = "node01.invalid";

/// A jump host forwarding [`BEHIND`]`:22` to `target`, and the client's
/// connection to it (any host key).
async fn bastion_to(target: &Host) -> (Host, Ssh) {
    let bastion = Host::start(Bench {
        tunnels: vec![(BEHIND, 22, target.addr)],
        ..Bench::default()
    })
    .await;
    let jump = connect(&bastion).await;
    (bastion, jump)
}

/// The target through `jump`, held by nothing else.
async fn through(jump: &Ssh, target: &Target) -> Result<Ssh, SshError> {
    through_hold(Jump::new(jump.clone()), target).await
}

async fn through_hold(jump: Jump, target: &Target) -> Result<Ssh, SshError> {
    Ssh::connect_through(
        jump,
        target,
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        None,
        &options(),
    )
    .await
}

#[tokio::test]
async fn a_host_behind_a_jump_host_is_reached_through_it() {
    let target = Host::start(Bench::default()).await;
    let (bastion, jump) = bastion_to(&target).await;
    let ssh = within(10, through(&jump, &Target::new(USER, BEHIND, 22)))
        .await
        .unwrap();
    let out = ssh.run("echo behind").await.unwrap();
    assert!(out.success());
    assert_eq!(out.stdout(), b"behind\n");
    assert_eq!(bastion.seen().tunnels, vec![(BEHIND.to_owned(), 22)]);
    // No address of the target's own: a mosh session has nowhere to send
    // its datagrams.
    assert_eq!(ssh.peer_addr(), None);
    assert_eq!(ssh.target().host, BEHIND);
    // The jump host ran nothing: it only carried the way.
    assert!(bastion.seen().commands.is_empty());
}

/// Previews, uploads and the file browser borrow the terminal's
/// connection: SFTP goes through the jump as the terminal does.
#[tokio::test]
async fn sftp_goes_through_the_jump_host() {
    let target = Host::start(Bench::default()).await;
    std::fs::write(target.home.join("notes.txt"), b"through the bastion").unwrap();
    let (_bastion, jump) = bastion_to(&target).await;
    let ssh = through(&jump, &Target::new(USER, BEHIND, 22))
        .await
        .unwrap();
    let sftp = ssh.sftp().await.unwrap();
    let names: Vec<String> = sftp
        .list(".")
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert!(names.iter().any(|p| p.ends_with("notes.txt")), "{names:?}");
}

/// Each host's key is its own question, the jump host's first, and each is
/// kept under the name and port it was asked by.
#[tokio::test]
async fn each_host_key_is_asked_for_in_order() {
    let target = Host::start(Bench::default()).await;
    let bastion = Host::start(Bench {
        tunnels: vec![(BEHIND, 22, target.addr)],
        ..Bench::default()
    })
    .await;
    let known = Arc::new(KnownHosts::new(Arc::new(MemoryKnownHosts::default())));
    let person = Person::answering(&[true, true]);
    let policy = Arc::new(TrustOnFirstUse::new(known.clone(), person.clone()));
    let jump = Ssh::connect(
        &bastion.target(),
        Auth::key(fixture_key("ed25519")),
        policy.clone(),
        &options(),
    )
    .await
    .unwrap();
    Ssh::connect_through(
        Jump::new(jump),
        &Target::new(USER, BEHIND, 22),
        Auth::key(fixture_key("ed25519")),
        policy,
        None,
        &options(),
    )
    .await
    .unwrap();
    let asked: Vec<(String, u16, String)> = person
        .asked()
        .into_iter()
        .map(|a| (a.host, a.port, a.fingerprint))
        .collect();
    assert_eq!(
        asked,
        vec![
            (
                "127.0.0.1".to_owned(),
                bastion.port(),
                bastion.host_keys[0].fingerprint()
            ),
            (BEHIND.to_owned(), 22, target.host_keys[0].fingerprint()),
        ]
    );
}

/// A place the jump host can't reach is the jump host's answer, said as
/// such: not the target refusing, not a timeout.
#[tokio::test]
async fn a_place_the_jump_host_cannot_reach_says_so() {
    let target = Host::start(Bench::default()).await;
    let (_bastion, jump) = bastion_to(&target).await;
    let error = through(&jump, &Target::new(USER, "elsewhere.invalid", 22))
        .await
        .err()
        .unwrap();
    match error {
        SshError::Tunnel {
            via,
            host,
            port,
            reason: TunnelRefusal::ConnectFailed,
        } => {
            assert_eq!(via, "127.0.0.1");
            assert_eq!((host.as_str(), port), ("elsewhere.invalid", 22));
        }
        other => panic!("{other:?}"),
    }
}

/// The jump host's link dies: the target's connection ends with it, at
/// once, and says why in one reason — the jump host's — rather than a
/// broken channel of its own.
#[tokio::test]
async fn the_jump_host_going_takes_the_target_with_it() {
    let target = Host::start(Bench::default()).await;
    let bastion = Host::start(Bench {
        tunnels: vec![(BEHIND, 22, target.addr)],
        ..Bench::default()
    })
    .await;
    let relay = Relay::start(bastion.addr).await;
    let jump = Ssh::connect(
        &Target::new(USER, "127.0.0.1", relay.port),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap();
    let ssh = through(&jump, &Target::new(USER, BEHIND, 22))
        .await
        .unwrap();
    let mut cat = ssh.exec("cat").await.unwrap();
    relay.cut();
    let end = within(5, ssh.ended()).await;
    match &end {
        ConnectionEnd::Lost { error } => assert!(
            error.contains("jump host 127.0.0.1"),
            "the reason names the jump host: {error}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(end.is_lost(), "a reconnect is what it wants");
    assert_eq!(within(5, cat.next()).await, Some(Event::Lost));
    assert!(jump.is_closed());
}

/// Closing the target closes the jump host's connection it came through:
/// nothing is left signed in on the bastion.
#[tokio::test]
async fn closing_the_target_closes_the_jump() {
    let target = Host::start(Bench::default()).await;
    let (_bastion, jump) = bastion_to(&target).await;
    let ssh = through(&jump, &Target::new(USER, BEHIND, 22))
        .await
        .unwrap();
    ssh.close().await;
    assert_eq!(within(5, ssh.ended()).await, ConnectionEnd::ClosedByUs);
    assert_eq!(within(5, jump.ended()).await, ConnectionEnd::ClosedByUs);
}

/// Dropped without a close (a session torn down mid-dial), the target
/// takes the jump host's connection with it all the same.
#[tokio::test]
async fn dropping_the_target_lets_the_jump_go() {
    let target = Host::start(Bench::default()).await;
    let (_bastion, jump) = bastion_to(&target).await;
    let ssh = through(&jump, &Target::new(USER, BEHIND, 22))
        .await
        .unwrap();
    drop(ssh);
    within(5, async {
        while !jump.is_closed() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
}

/// A dial through the jump host that fails (the target turns the key
/// down) closes the jump connection it was given.
#[tokio::test]
async fn a_failed_dial_through_lets_the_jump_go() {
    let target = Host::start(Bench {
        authorized: Vec::new(),
        password: None,
        ..Bench::default()
    })
    .await;
    let (_bastion, jump) = bastion_to(&target).await;
    let error = through(&jump, &Target::new(USER, BEHIND, 22))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, SshError::AuthFailed { ref host, .. } if host == BEHIND),
        "{error:?}"
    );
    within(5, async {
        while !jump.is_closed() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
}

/// The target's leg alone drops (the host behind restarted; the jump
/// host's link is fine): its end is its own, the jump host's connection
/// outlives it, and the redial goes through that again — no second
/// sign-in on the jump host, where a one-time code would be asked again.
#[tokio::test]
async fn a_redial_goes_through_a_jump_host_still_up() {
    let target = Host::start(Bench::default()).await;
    let behind = Relay::start(target.addr).await;
    let bastion = Host::start(Bench {
        tunnels: vec![(BEHIND, 22, SocketAddr::from(([127, 0, 0, 1], behind.port)))],
        ..Bench::default()
    })
    .await;
    let jump = connect(&bastion).await;
    let first = through(&jump, &Target::new(USER, BEHIND, 22))
        .await
        .unwrap();
    behind.cut();
    let end = within(5, first.ended()).await;
    assert!(end.is_lost(), "{end:?}");
    assert!(
        !format!("{end:?}").contains("jump host"),
        "its own reason: {end:?}"
    );
    let kept = first.jump().unwrap().clone();
    assert!(kept.is_alive());
    drop(first);
    assert!(
        kept.is_alive(),
        "held by the redial, not by the dead target"
    );
    behind.resume();
    let second = within(10, through_hold(kept, &Target::new(USER, BEHIND, 22)))
        .await
        .unwrap();
    assert!(second.run("echo again").await.unwrap().success());
    assert_eq!(
        bastion.seen().connections,
        1,
        "signed in to the jump host once"
    );
    assert_eq!(bastion.seen().tunnels.len(), 2);
    // The new target is now the jump host's only hold: closing it closes
    // the jump host's connection.
    second.close().await;
    assert_eq!(within(5, jump.ended()).await, ConnectionEnd::ClosedByUs);
}

/// A host behind a jump host is trusted under its name as every host is
/// (B630): lower case, no trailing dot, whatever case it was typed in —
/// and the jump host's entry the same, each its own.
#[tokio::test]
async fn host_keys_through_a_jump_are_kept_like_any_others() {
    const TYPED: &str = "Node01.INVALID.";
    let target = Host::start(Bench::default()).await;
    let bastion = Host::start(Bench {
        tunnels: vec![(TYPED, 22, target.addr)],
        ..Bench::default()
    })
    .await;
    let known = Arc::new(KnownHosts::new(Arc::new(MemoryKnownHosts::default())));
    let policy = Arc::new(TrustOnFirstUse::new(
        known.clone(),
        Person::answering(&[true, true]),
    ));
    let jump = Ssh::connect(
        &bastion.target(),
        Auth::key(fixture_key("ed25519")),
        policy.clone(),
        &options(),
    )
    .await
    .unwrap();
    Ssh::connect_through(
        Jump::new(jump),
        &Target::new(USER, TYPED, 22),
        Auth::key(fixture_key("ed25519")),
        policy,
        None,
        &options(),
    )
    .await
    .unwrap();
    let keys: Vec<String> = known.snapshot().into_keys().collect();
    assert_eq!(
        keys,
        vec![
            format!("127.0.0.1:{}", bastion.port()),
            "node01.invalid:22".to_owned()
        ]
    );
}
