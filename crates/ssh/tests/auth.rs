//! Signing in: every key type the app holds, passwords, hardware keys that
//! sign through the platform, and the keys that must be refused before
//! anything is sent.

mod support;

use std::sync::Arc;
use std::time::Duration;

use moshpit_ssh::{
    Auth, BoxFuture, ConnectOptions, HardwareKey, HostSignature, KeyError, KeyKind, KeySigner,
    SignerError, Ssh, SshError, inspect_private_key,
};
use p256::ecdsa::signature::Signer as _;
use russh::MethodKind;
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKey};
use support::*;

async fn sign_in_with(key_text: String, bench: Bench) -> (Host, Result<Ssh, SshError>) {
    let host = Host::start(bench).await;
    let result = Ssh::connect(
        &host.target(),
        Auth::key(key_text),
        Arc::new(AcceptAny),
        &options(),
    )
    .await;
    (host, result)
}

/// Signs in with fixture `key` and checks the signature algorithm the
/// server verified.
async fn signs_in(key: &str, algorithm: &str) {
    let (host, ssh) = sign_in_with(
        fixture_key(key),
        Bench {
            authorized: vec![fixture_public(key)],
            ..Default::default()
        },
    )
    .await;
    let ssh = ssh.unwrap_or_else(|e| panic!("{key}: {e}"));
    assert_eq!(ssh.run("echo in").await.unwrap().stdout(), b"in\n");
    assert_eq!(
        host.seen().key_algorithms,
        vec![algorithm.to_owned()],
        "{key}"
    );
}

#[tokio::test]
async fn an_ed25519_key_signs_in() {
    signs_in("ed25519", "ssh-ed25519").await;
}

/// SHA-2, never the SHA-1 `ssh-rsa` Citadel signed with (which OpenSSH has
/// refused by default since 8.8).
#[tokio::test]
async fn an_rsa_key_signs_in_with_sha512() {
    signs_in("rsa2048", "rsa-sha2-512").await;
}

#[tokio::test]
async fn the_apps_rsa_4096_signs_in() {
    signs_in("rsa4096", "rsa-sha2-512").await;
}

#[tokio::test]
async fn ecdsa_keys_of_every_curve_sign_in() {
    signs_in("ecdsa256", "ecdsa-sha2-nistp256").await;
    // P-384 and P-521 the app could import but never use.
    signs_in("ecdsa384", "ecdsa-sha2-nistp384").await;
    signs_in("ecdsa521", "ecdsa-sha2-nistp521").await;
}

#[tokio::test]
async fn rsa_follows_what_the_server_says_it_takes() {
    let (host, ssh) = sign_in_with(
        fixture_key("rsa2048"),
        Bench {
            authorized: vec![fixture_public("rsa2048")],
            preferred_keys: Some(vec![
                Algorithm::Ed25519,
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha256),
                },
            ]),
            ..Default::default()
        },
    )
    .await;
    ssh.unwrap();
    assert_eq!(host.seen().key_algorithms, vec!["rsa-sha2-256"]);
}

/// What a server's `server-sig-algs` names when it names no RSA signature:
/// to the client, the same as a server that sent none (an OpenSSH before
/// 7.2, which knows neither that extension nor SHA-2 signatures).
fn says_nothing_of_rsa() -> Option<Vec<Algorithm>> {
    Some(vec![Algorithm::Ed25519])
}

/// B390: a host that didn't say which RSA signature it takes, and takes
/// only `ssh-rsa`, is signed in to on the same connection — SHA-512, then
/// SHA-256, then `ssh-rsa`, as the Swift path tries them.
#[tokio::test]
async fn rsa_falls_back_to_ssh_rsa_on_a_server_that_says_nothing() {
    let (host, ssh) = sign_in_with(
        fixture_key("rsa2048"),
        Bench {
            authorized: vec![fixture_public("rsa2048")],
            preferred_keys: says_nothing_of_rsa(),
            rsa_signatures: Some(vec!["ssh-rsa"]),
            ..Default::default()
        },
    )
    .await;
    let ssh = ssh.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ssh.run("echo in").await.unwrap().stdout(), b"in\n");
    let seen = host.seen();
    assert_eq!(
        seen.key_attempts,
        vec!["rsa-sha2-512", "rsa-sha2-256", "ssh-rsa"]
    );
    assert_eq!(seen.key_algorithms, vec!["ssh-rsa"]);
    assert_eq!(seen.connections, 1, "one connection");
}

/// A host that didn't say, and takes SHA-512: one request, as before.
#[tokio::test]
async fn rsa_on_a_server_that_says_nothing_but_takes_sha512_asks_once() {
    let (host, ssh) = sign_in_with(
        fixture_key("rsa2048"),
        Bench {
            authorized: vec![fixture_public("rsa2048")],
            preferred_keys: says_nothing_of_rsa(),
            ..Default::default()
        },
    )
    .await;
    ssh.unwrap();
    assert_eq!(host.seen().key_attempts, vec!["rsa-sha2-512"]);
}

/// A host that lists what it takes gets only that, even when it is refused:
/// the fallback is for a server that said nothing.
#[tokio::test]
async fn rsa_tries_nothing_but_what_a_server_lists() {
    let (host, result) = sign_in_with(
        fixture_key("rsa2048"),
        Bench {
            authorized: vec![fixture_public("rsa2048")],
            preferred_keys: Some(vec![
                Algorithm::Ed25519,
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha256),
                },
            ]),
            rsa_signatures: Some(vec!["ssh-rsa"]),
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(result, Err(SshError::AuthFailed { .. })),
        "{:?}",
        result.err()
    );
    assert_eq!(host.seen().key_attempts, vec!["rsa-sha2-256"]);
}

#[tokio::test]
async fn a_key_the_host_does_not_know_is_rejected() {
    let (_host, result) = sign_in_with(
        fixture_key("ecdsa256"),
        Bench {
            authorized: vec![fixture_public("ed25519")],
            ..Default::default()
        },
    )
    .await;
    match result {
        Err(SshError::AuthFailed {
            user, remaining, ..
        }) => {
            assert_eq!(user, USER);
            assert!(remaining.contains(&"publickey".to_owned()), "{remaining:?}");
        }
        other => panic!("{:?}", other.err()),
    }
}

#[tokio::test]
async fn a_password_signs_in() {
    let host = Host::start(Bench::default()).await;
    let ssh = Ssh::connect(
        &host.target(),
        Auth::password(PASSWORD),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap();
    assert!(ssh.run("echo ok").await.unwrap().success());
    assert_eq!(host.seen().password_attempts, 1);
}

#[tokio::test]
async fn a_wrong_password_is_an_authentication_failure() {
    let host = Host::start(Bench::default()).await;
    let err = Ssh::connect(
        &host.target(),
        Auth::password("hunter2"),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, SshError::AuthFailed { .. }), "{err:?}");
}

/// The case the app words as "the server rejected this sign-in method".
#[tokio::test]
async fn a_password_on_a_key_only_host_is_a_rejected_method() {
    let host = Host::start(Bench {
        methods: vec![MethodKind::PublicKey],
        ..Default::default()
    })
    .await;
    let err = Ssh::connect(
        &host.target(),
        Auth::password(PASSWORD),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .err()
    .unwrap();
    match err {
        SshError::AuthMethodRejected {
            method, offered, ..
        } => {
            assert_eq!(method, "password");
            assert_eq!(offered, vec!["publickey"]);
        }
        other => panic!("{other:?}"),
    }
}

// MARK: keyboard-interactive (B393)

/// What keyboard-interactive asks, round by round: each prompt, and
/// whether it is shown as typed.
type Rounds = Vec<Vec<(&'static str, bool)>>;

/// A host that takes passwords only by asking for them (FreeBSD's stock
/// sshd: password sign-in off, keyboard-interactive on), asking `rounds`.
async fn kbd_host(rounds: Rounds) -> Host {
    Host::start(Bench {
        methods: vec![MethodKind::PublicKey, MethodKind::KeyboardInteractive],
        keyboard_interactive: rounds,
        ..Default::default()
    })
    .await
}

async fn sign_in_by_password(host: &Host, password: &str) -> Result<Ssh, SshError> {
    Ssh::connect(
        &host.target(),
        Auth::password(password),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
}

/// The saved password answers the one hidden "Password:" — and PAM's
/// closing round that asks nothing (red before: "the server rejected this
/// sign-in method").
#[tokio::test]
async fn a_password_signs_in_where_the_host_asks_for_it() {
    for rounds in [
        vec![vec![("Password: ", false)]],
        vec![vec![("Password: ", false)], vec![]],
    ] {
        let host = kbd_host(rounds.clone()).await;
        let ssh = sign_in_by_password(&host, PASSWORD)
            .await
            .unwrap_or_else(|e| panic!("{rounds:?}: {e}"));
        assert_eq!(ssh.run("echo in").await.unwrap().stdout(), b"in\n");
        let mut answered = vec![vec![PASSWORD.to_owned()]];
        if rounds.len() == 2 {
            answered.push(vec![]);
        }
        assert_eq!(host.seen().kbd_answers, answered, "{rounds:?}");
    }
}

/// A wrong password given that way is a wrong password.
#[tokio::test]
async fn a_wrong_password_the_host_asked_for_is_an_authentication_failure() {
    let host = kbd_host(vec![vec![("Password: ", false)]]).await;
    let err = sign_in_by_password(&host, "wrong").await.err().unwrap();
    assert!(matches!(err, SshError::AuthFailed { .. }), "{err:?}");
}

/// What only the person can answer is never answered with the password:
/// a prompt shown as typed, a second hidden one (a one-time code), a round
/// asking for the password and a code at once.
#[tokio::test]
async fn questions_only_the_person_can_answer_are_said_as_such() {
    let cases: [(Rounds, Vec<Vec<String>>); 3] = [
        (vec![vec![("Username: ", true)]], vec![]),
        (
            vec![
                vec![("Password: ", false)],
                vec![("Verification code: ", false)],
            ],
            vec![vec![PASSWORD.to_owned()]],
        ),
        (
            vec![vec![("Password: ", false), ("Verification code: ", false)]],
            vec![],
        ),
    ];
    for (rounds, answered) in cases {
        let host = kbd_host(rounds.clone()).await;
        let err = sign_in_by_password(&host, PASSWORD).await.err().unwrap();
        assert!(
            matches!(err, SshError::InteractivePrompts { .. }),
            "{rounds:?}: {err:?}"
        );
        assert_eq!(host.seen().kbd_answers, answered, "{rounds:?}");
    }
}

/// A host that takes passwords as such never gets one through
/// keyboard-interactive, even when it offers both and the password is
/// wrong.
#[tokio::test]
async fn a_host_that_takes_passwords_is_never_asked_twice() {
    let host = Host::start(Bench {
        methods: vec![
            MethodKind::PublicKey,
            MethodKind::Password,
            MethodKind::KeyboardInteractive,
        ],
        keyboard_interactive: vec![vec![("Password: ", false)]],
        ..Default::default()
    })
    .await;
    let err = sign_in_by_password(&host, "wrong").await.err().unwrap();
    assert!(matches!(err, SshError::AuthFailed { .. }), "{err:?}");
    assert!(host.seen().kbd_answers.is_empty());
}

async fn refused_locally(text: String) -> KeyError {
    let host = Host::start(Bench::default()).await;
    let err = Ssh::connect(
        &host.target(),
        Auth::key(text),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(host.seen().key_algorithms.is_empty());
    match err {
        SshError::Key(e) => e,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn keys_the_app_should_have_converted_are_refused_before_sign_in() {
    assert_eq!(
        refused_locally(fixture_key("ed25519-encrypted")).await,
        KeyError::Encrypted
    );
    assert_eq!(
        refused_locally(fixture_key("rsa2048-pem")).await,
        KeyError::NotOpenSsh
    );
    assert_eq!(
        refused_locally(fixture_key("ecdsa256-pkcs8")).await,
        KeyError::NotOpenSsh
    );
    assert!(matches!(
        refused_locally(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n".into()
        )
        .await,
        KeyError::Malformed(_)
    ));
}

/// A key pasted with Windows line endings, other line lengths and stray
/// spaces is still the key.
#[tokio::test]
async fn a_key_with_crlf_and_odd_wrapping_signs_in() {
    let text = fixture_key("ed25519");
    let body: String = text
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let rewrapped = body
        .as_bytes()
        .chunks(64)
        .map(|c| format!("  {}\r\n", std::str::from_utf8(c).unwrap()))
        .collect::<String>();
    let messy = format!(
        "\r\n-----BEGIN OPENSSH PRIVATE KEY-----\r\n{rewrapped}-----END OPENSSH PRIVATE KEY-----\r\n\r\n"
    );
    let (_host, ssh) = sign_in_with(messy, Bench::default()).await;
    ssh.unwrap();
}

/// The facts the key list shows, against `ssh-keygen -l` and the `.pub`
/// files, for every fixture OpenSSH key — the encrypted one included.
#[test]
fn inspected_keys_match_ssh_keygen() {
    for (name, kind) in [
        ("ed25519", KeyKind::Ed25519),
        ("ed25519-encrypted", KeyKind::Ed25519),
        ("rsa2048", KeyKind::Rsa),
        ("rsa4096", KeyKind::Rsa),
        ("ecdsa256", KeyKind::EcdsaP256),
        ("ecdsa384", KeyKind::EcdsaP384),
        ("ecdsa521", KeyKind::EcdsaP521),
    ] {
        let info = inspect_private_key(&fixture_key(name)).unwrap();
        assert_eq!(info.kind, kind, "{name}");
        assert_eq!(info.fingerprint, keygen_fingerprint(name), "{name}");
        // An encrypted key's comment is in its encrypted half.
        let line = fixture_key(&format!("{name}.pub"));
        let (expected_line, comment) = if info.encrypted {
            (line.rsplit_once(' ').unwrap().0.to_owned(), String::new())
        } else {
            (line.trim().to_owned(), format!("{name}@fixture"))
        };
        assert_eq!(info.public_key, expected_line, "{name}");
        assert_eq!(info.comment, comment, "{name}");
        assert_eq!(info.encrypted, name.contains("encrypted"), "{name}");
    }
    for name in ["rsa2048-pem", "ecdsa256-pkcs8"] {
        assert_eq!(
            inspect_private_key(&fixture_key(name)).err(),
            Some(KeyError::NotOpenSsh)
        );
    }
}

/// The Secure Enclave as the tests know it: a P-256 key in software that
/// answers like CryptoKit (DER or raw), optionally slowly, or not at all.
struct Enclave {
    key: p256::ecdsa::SigningKey,
    raw: bool,
    delay: Duration,
    refuse: bool,
    /// Refused because the device is locked (B227).
    locked: bool,
}

impl Enclave {
    fn new() -> (Enclave, String) {
        let private = PrivateKey::random(
            &mut rand::rng(),
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        )
        .unwrap();
        let scalar = match private.key_data().ecdsa().unwrap() {
            russh::keys::ssh_key::private::EcdsaKeypair::NistP256 { private, .. } => {
                private.as_ref().to_vec()
            }
            _ => unreachable!(),
        };
        let mut public = private.public_key().clone();
        public.set_comment("enclave");
        (
            Enclave {
                key: p256::ecdsa::SigningKey::from_slice(&scalar).unwrap(),
                raw: false,
                delay: Duration::ZERO,
                refuse: false,
                locked: false,
            },
            public.to_openssh().unwrap(),
        )
    }
}

impl KeySigner for Enclave {
    fn sign<'a>(
        &'a self,
        key_ref: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<HostSignature, SignerError>> {
        Box::pin(async move {
            assert_eq!(key_ref, "moshpit.secret.enclave");
            tokio::time::sleep(self.delay).await;
            if self.refuse {
                return Err(SignerError::Failed("the person cancelled Face ID".into()));
            }
            if self.locked {
                return Err(SignerError::Locked);
            }
            let signature: p256::ecdsa::Signature = self.key.sign(data);
            Ok(if self.raw {
                HostSignature::Raw(signature.to_bytes().to_vec())
            } else {
                HostSignature::Der(signature.to_der().as_bytes().to_vec())
            })
        })
    }
}

async fn sign_in_with_enclave(
    enclave: Enclave,
    line: &str,
    authorized_line: &str,
    options: ConnectOptions,
) -> (Host, Result<Ssh, SshError>) {
    let host = Host::start(Bench {
        authorized: vec![russh::keys::PublicKey::from_openssh(authorized_line).unwrap()],
        ..Default::default()
    })
    .await;
    let key = HardwareKey::new(line, "moshpit.secret.enclave", Arc::new(enclave)).unwrap();
    let result = Ssh::connect(
        &host.target(),
        Auth::Hardware(key),
        Arc::new(AcceptAny),
        &options,
    )
    .await;
    (host, result)
}

#[tokio::test]
async fn a_hardware_key_signs_in_through_the_platform_with_der() {
    let (enclave, line) = Enclave::new();
    let (host, ssh) = sign_in_with_enclave(enclave, &line, &line, options()).await;
    assert!(ssh.unwrap().run("echo se").await.unwrap().success());
    assert_eq!(host.seen().key_algorithms, vec!["ecdsa-sha2-nistp256"]);
}

#[tokio::test]
async fn a_hardware_key_may_answer_raw_r_and_s() {
    let (mut enclave, line) = Enclave::new();
    enclave.raw = true;
    let (_host, ssh) = sign_in_with_enclave(enclave, &line, &line, options()).await;
    ssh.unwrap();
}

/// Face ID takes the person's time, not the server's.
#[tokio::test]
async fn a_slow_hardware_signer_does_not_time_the_handshake_out() {
    let (mut enclave, line) = Enclave::new();
    enclave.delay = Duration::from_millis(1500);
    let options = ConnectOptions {
        handshake_timeout: Duration::from_millis(1000),
        ..options()
    };
    let (_host, ssh) = sign_in_with_enclave(enclave, &line, &line, options).await;
    ssh.unwrap();
}

#[tokio::test]
async fn a_cancelled_signature_says_so() {
    let (mut enclave, line) = Enclave::new();
    enclave.refuse = true;
    let (_host, result) = sign_in_with_enclave(enclave, &line, &line, options()).await;
    assert!(
        matches!(result, Err(SshError::Signer(SignerError::Failed(_)))),
        "{:?}",
        result.err()
    );
}

/// B227: a hardware key the locked device refuses ends the sign-in as the
/// lock, which the session retries on the unlock — never as a failed
/// signature.
#[tokio::test]
async fn a_signature_the_locked_device_refuses_says_so() {
    let (mut enclave, line) = Enclave::new();
    enclave.locked = true;
    let (_host, result) = sign_in_with_enclave(enclave, &line, &line, options()).await;
    assert!(
        matches!(result, Err(SshError::Signer(SignerError::Locked))),
        "{:?}",
        result.err()
    );
}

/// A stored public half that isn't the hardware key's: caught before the
/// server could only say "no".
#[tokio::test]
async fn a_signature_from_another_key_is_caught_locally() {
    let (enclave, _) = Enclave::new();
    let (_, other_line) = Enclave::new();
    let (host, result) = sign_in_with_enclave(enclave, &other_line, &other_line, options()).await;
    assert!(
        matches!(result, Err(SshError::Signer(SignerError::Mismatch))),
        "{:?}",
        result.err()
    );
    assert!(host.seen().key_algorithms.is_empty());
}

#[test]
fn hardware_keys_are_ecdsa_or_ed25519() {
    let (enclave, _) = Enclave::new();
    let signer: Arc<dyn KeySigner> = Arc::new(enclave);
    let rsa = fixture_key("rsa2048.pub");
    assert!(matches!(
        HardwareKey::new(&rsa, "r", signer.clone()),
        Err(KeyError::Unsupported(_))
    ));
    assert!(matches!(
        HardwareKey::new("ssh-ed25519 !!!", "r", signer),
        Err(KeyError::BadPublicKey(_))
    ));
}
