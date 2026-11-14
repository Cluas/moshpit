//! Trust on first use against the trusted-key map, over real key exchanges:
//! new hosts asked about, changed keys refused, a locked map deciding
//! nothing, fingerprints as `ssh-keygen` prints them, and the host shown
//! the same key it showed the app.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use moshpit_ssh::{
    Auth, ConnectOptions, HOST_KEY_ORDER, KnownHosts, KnownHostsStore, Ssh, SshError, StoreError,
    Target, TrustOnFirstUse,
};
use russh::keys::{EcdsaCurve, PrivateKey};
use support::*;

fn tofu(
    known: &Arc<KnownHosts>,
    prompt: Arc<dyn moshpit_ssh::HostKeyPrompt>,
) -> Arc<TrustOnFirstUse> {
    Arc::new(TrustOnFirstUse::new(known.clone(), prompt))
}

async fn connect_with(
    target: &Target,
    policy: Arc<TrustOnFirstUse>,
    options: &ConnectOptions,
) -> Result<Ssh, SshError> {
    Ssh::connect(target, Auth::key(fixture_key("ed25519")), policy, options).await
}

fn entry(host: &Host) -> String {
    format!("127.0.0.1:{}", host.port())
}

#[tokio::test]
async fn a_new_host_is_asked_about_once_and_remembered() {
    let host = Host::start(Bench::default()).await;
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let person = Person::answering(&[true]);
    let ssh = connect_with(&host.target(), tofu(&known, person.clone()), &options())
        .await
        .unwrap();
    let fingerprint = host.host_keys[0].fingerprint();
    assert_eq!(ssh.host_key().fingerprint(), fingerprint);
    assert_eq!(
        person.asked(),
        vec![Asked {
            host: "127.0.0.1".into(),
            port: host.port(),
            fingerprint: fingerprint.clone(),
            expected: None,
        }]
    );
    assert_eq!(
        known.snapshot(),
        BTreeMap::from([(entry(&host), fingerprint)])
    );

    // Known now: nobody is asked.
    let nobody = Person::answering(&[]);
    connect_with(&host.target(), tofu(&known, nobody.clone()), &options())
        .await
        .unwrap();
    assert!(nobody.asked().is_empty());
}

#[tokio::test]
async fn a_declined_host_gets_no_credentials_and_no_trust() {
    let host = Host::start(Bench::default()).await;
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let err = connect_with(
        &host.target(),
        tofu(&known, Person::answering(&[false])),
        &options(),
    )
    .await
    .err()
    .unwrap();
    match err {
        SshError::HostKeyNotTrusted {
            fingerprint, port, ..
        } => {
            assert_eq!(fingerprint, host.host_keys[0].fingerprint());
            assert_eq!(port, host.port());
        }
        other => panic!("{other:?}"),
    }
    assert!(known.snapshot().is_empty());
    assert!(host.seen().key_algorithms.is_empty());
}

#[tokio::test]
async fn a_changed_key_is_refused_without_asking_by_default() {
    let host = Host::start(Bench::default()).await;
    let old = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".to_owned();
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::from([(
        entry(&host),
        old.clone(),
    )])));
    let person = Person::answering(&[true]);
    let err = connect_with(
        &host.target(),
        tofu(&known, Arc::new(NewHostsOnly(person.clone()))),
        &options(),
    )
    .await
    .err()
    .unwrap();
    match err {
        SshError::HostKeyChanged {
            presented,
            expected,
            ..
        } => {
            assert_eq!(presented, host.host_keys[0].fingerprint());
            assert_eq!(expected, old);
        }
        other => panic!("{other:?}"),
    }
    assert!(person.asked().is_empty(), "a change is not a new host");
    assert_eq!(known.snapshot()[&entry(&host)], old);
    assert!(host.seen().key_algorithms.is_empty());
}

#[tokio::test]
async fn a_change_the_person_accepts_is_trusted_from_then_on() {
    let host = Host::start(Bench::default()).await;
    let old = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".to_owned();
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::from([(
        entry(&host),
        old.clone(),
    )])));
    let person = Person::answering(&[true]);
    connect_with(&host.target(), tofu(&known, person.clone()), &options())
        .await
        .unwrap();
    let asked = person.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].expected.as_deref(), Some(old.as_str()));
    assert_eq!(
        known.snapshot()[&entry(&host)],
        host.host_keys[0].fingerprint()
    );
}

struct Locked;

impl KnownHostsStore for Locked {
    fn read(&self) -> Result<BTreeMap<String, String>, StoreError> {
        Err(StoreError::Locked)
    }
    fn write(&self, _: &BTreeMap<String, String>) -> Result<(), StoreError> {
        Err(StoreError::Locked)
    }
}

/// B22: while the device is locked nothing true can be asked.
#[tokio::test]
async fn a_locked_map_refuses_without_asking() {
    let host = Host::start(Bench::default()).await;
    let known = Arc::new(KnownHosts::new(Arc::new(Locked)));
    let person = Person::answering(&[true]);
    let err = connect_with(&host.target(), tofu(&known, person.clone()), &options())
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SshError::HostKeysLocked { .. }), "{err:?}");
    assert!(person.asked().is_empty());
}

/// The map is keyed by the host as typed: a name and its address are two
/// hosts, as they are in the app.
#[tokio::test]
async fn trust_is_keyed_by_the_host_as_typed() {
    let host = Host::start(Bench::default()).await;
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let person = Person::answering(&[true, true]);
    connect_with(&host.target(), tofu(&known, person.clone()), &options())
        .await
        .unwrap();
    let by_name = Target::new(USER, "localhost", host.port());
    connect_with(&by_name, tofu(&known, person.clone()), &options())
        .await
        .unwrap();
    assert_eq!(person.asked().len(), 2);
    assert_eq!(
        known.snapshot().keys().cloned().collect::<Vec<_>>(),
        vec![entry(&host), format!("localhost:{}", host.port())]
    );
}

async fn presented(keys: Vec<PrivateKey>) -> (String, String) {
    let host = Host::start(Bench {
        host_keys: keys,
        ..Default::default()
    })
    .await;
    let ssh = connect(&host).await;
    (ssh.host_key().algorithm(), ssh.host_key().fingerprint())
}

/// What `HostKeyValidator` stored for a host is `ssh-keygen -l`'s
/// fingerprint of the key it presented.
#[tokio::test]
async fn fingerprints_are_what_ssh_keygen_prints() {
    for name in ["ed25519", "ecdsa256", "ecdsa384", "ecdsa521", "rsa2048"] {
        let (_, fingerprint) = presented(vec![fixture_private(name)]).await;
        assert_eq!(fingerprint, keygen_fingerprint(name), "{name}");
    }
}

/// A host with several keys shows each client the first of the client's
/// list it has. Listing them in NIOSSH's order means the host shows this
/// client the key the app trusted — any other order would greet a user
/// with "the host key changed" on the day the transport switches.
#[tokio::test]
async fn hosts_show_the_key_niossh_saw() {
    let p256 = || ecdsa(EcdsaCurve::NistP256);
    let p384 = || ecdsa(EcdsaCurve::NistP384);
    let p521 = || ecdsa(EcdsaCurve::NistP521);
    let rsa = || fixture_private("rsa2048");

    let cases: Vec<(Vec<PrivateKey>, &str)> = vec![
        (vec![rsa(), p256(), ed25519()], "ssh-ed25519"),
        (vec![p256(), p384()], "ecdsa-sha2-nistp384"),
        (vec![p521(), p256()], "ecdsa-sha2-nistp256"),
        (vec![p521(), rsa()], "ecdsa-sha2-nistp521"),
        // The app never accepted RSA host keys; now it can, as a last resort.
        (vec![rsa()], "ssh-rsa"),
    ];
    for (keys, expected) in cases {
        let (algorithm, _) = presented(keys).await;
        assert_eq!(algorithm, expected);
    }
}

#[test]
fn the_host_key_order_is_niosshs_then_rsa() {
    // SSHKeyExchangeStateMachine.bundledServerHostKeyAlgorithms in the
    // swift-nio-ssh the app pins (Wellz26 fork, a05e6bbe).
    let names: Vec<&str> = HOST_KEY_ORDER.iter().map(|a| a.as_str()).collect();
    assert_eq!(
        names,
        [
            "ssh-ed25519",
            "ecdsa-sha2-nistp384",
            "ecdsa-sha2-nistp256",
            "ecdsa-sha2-nistp521",
            "rsa-sha2-512",
            "rsa-sha2-256",
        ]
    );
}

/// Citadel gave the whole login ten seconds, the New Host card included.
#[tokio::test]
async fn the_time_a_person_takes_to_decide_does_not_count() {
    let host = Host::start(Bench::default()).await;
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let person = Person::thinking(Duration::from_millis(1500), &[true]);
    let options = ConnectOptions {
        handshake_timeout: Duration::from_millis(1000),
        ..options()
    };
    connect_with(&host.target(), tofu(&known, person), &options)
        .await
        .unwrap();
}

/// A connect given up while the card is up takes the card with it.
#[tokio::test]
async fn giving_up_on_a_connect_withdraws_the_question() {
    let host = Host::start(Bench::default()).await;
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let person = Person::thinking(Duration::from_secs(600), &[true]);
    let gave_up = tokio::time::timeout(
        Duration::from_millis(500),
        connect_with(&host.target(), tofu(&known, person.clone()), &options()),
    )
    .await;
    assert!(gave_up.is_err());
    assert_eq!(person.asked().len(), 1);
    within(5, async {
        while person.walked_away.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(known.snapshot().is_empty());
}
