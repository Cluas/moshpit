//! The key reader against real keys in every form people paste, and every
//! test the app's Swift key code had (SSHKeyImportTests, SSHKeyFactoryTests,
//! StoredKeyTests, PastedKeyStorageTests and the key parts of
//! SSHKeyRecordTests, RSASHA2SigningTests and KeychainReferencesTests),
//! ported. Where the answer changed on purpose the test says why (B126:
//! P-384 and P-521 sign in; ChaCha20-Poly1305 keys open).

use std::path::{Path, PathBuf};

use russh::keys::ssh_key::private::KeypairData;
use serde_json::Value;

use super::*;
use crate::auth::{KeyKind, load_private_key};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keys")
}

fn fixture(name: &str) -> String {
    let path = fixtures().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// One of the app's Swift test keys (`SSHKeyFixtures.swift`, extracted to
/// `fixtures/keys/app` with its manifest): `ssh-keygen`'s and OpenSSL's
/// own, with their `.pub` line and `ssh-keygen -l` fingerprint.
struct AppKey {
    name: String,
    text: String,
    encrypted: bool,
    public_key: String,
    fingerprint: String,
    algorithm: String,
    bits: i64,
}

const PASSPHRASE: &str = "correct horse battery";

fn app_keys() -> Vec<AppKey> {
    let manifest: Value = serde_json::from_str(&fixture("app/manifest.json")).unwrap();
    assert_eq!(manifest["passphrase"], PASSPHRASE);
    manifest["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| AppKey {
            name: k["name"].as_str().unwrap().to_owned(),
            text: fixture(&format!("app/{}", k["file"].as_str().unwrap())),
            encrypted: k["encrypted"].as_bool().unwrap(),
            public_key: k["publicKey"].as_str().unwrap().to_owned(),
            fingerprint: k["fingerprint"].as_str().unwrap().to_owned(),
            algorithm: k["algorithm"].as_str().unwrap().to_owned(),
            bits: k["bits"].as_i64().unwrap(),
        })
        .collect()
}

fn app_key(name: &str) -> AppKey {
    app_keys().into_iter().find(|k| k.name == name).unwrap()
}

impl AppKey {
    fn passphrase(&self) -> Option<&'static str> {
        self.encrypted.then_some(PASSPHRASE)
    }

    /// The key blob of its `.pub` line, as ssh-keygen wrote it.
    fn keygen_blob(&self) -> Vec<u8> {
        armor::strict_base64(self.public_key.split(' ').nth(1).unwrap())
            .unwrap()
            .to_vec()
    }
}

fn blob_of(text: &str) -> Vec<u8> {
    read(text, None).unwrap().public_blob()
}

fn algorithm_name(stored: &StoredKey) -> &'static str {
    match stored.record_algorithm() {
        SshKeyAlgorithm::Ed25519 => "ed25519",
        SshKeyAlgorithm::Rsa4096 => "rsa4096",
        SshKeyAlgorithm::Ecdsa => "ecdsa",
        other => panic!("{other:?}"),
    }
}

fn ssh_string(data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(data);
    out
}

fn mpint(raw: &[u8]) -> Vec<u8> {
    let mut bytes = der::magnitude(raw).to_vec();
    if bytes.first().is_some_and(|b| b & 0x80 != 0) {
        bytes.insert(0, 0);
    }
    ssh_string(&bytes)
}

/// An openssh-key-v1 container around `public_blob`, armoured as
/// `ssh-keygen` writes it. The private section defaults to a stand-in,
/// for keys refused on what their public half says.
fn openssh_pem(public_blob: &[u8], section: Option<&[u8]>, cipher: &str) -> String {
    let mut payload = b"openssh-key-v1\0".to_vec();
    payload.extend(ssh_string(cipher.as_bytes()));
    if cipher == "none" {
        payload.extend(ssh_string(b"none"));
        payload.extend(ssh_string(b""));
    } else {
        payload.extend(ssh_string(b"bcrypt"));
        let mut options = ssh_string(&[7; 16]);
        options.extend_from_slice(&[0, 0, 0, 16]);
        payload.extend(ssh_string(&options));
    }
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend(ssh_string(public_blob));
    payload.extend(ssh_string(section.unwrap_or(&[0; 16])));
    let body = armor::base64_encode(&payload);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!("{OPENSSH_BEGIN}\n{}\n{OPENSSH_END}\n", lines.join("\n"))
}

/// A real, unencrypted OpenSSH ECDSA P-256 key laid out the way
/// `ssh-keygen -t ecdsa` writes one — the scalar a minimal mpint, as
/// OpenSSH (and the app's Swift import) wrote it — and its public blob.
fn openssh_p256(scalar: &[u8], comment: &str) -> (String, Vec<u8>) {
    let KeypairData::Ecdsa(pair) = material::ecdsa(EcdsaCurve::P256, scalar).unwrap() else {
        unreachable!()
    };
    let point = pair.public_key_bytes().to_vec();
    let mut public = ssh_string(b"ecdsa-sha2-nistp256");
    public.extend(ssh_string(b"nistp256"));
    public.extend(ssh_string(&point));
    let mut section = vec![9u8; 8];
    section.extend(ssh_string(b"ecdsa-sha2-nistp256"));
    section.extend(ssh_string(b"nistp256"));
    section.extend(ssh_string(&point));
    section.extend(mpint(scalar));
    section.extend(ssh_string(comment.as_bytes()));
    let mut pad = 1u8;
    while !section.len().is_multiple_of(8) {
        section.push(pad);
        pad += 1;
    }
    (openssh_pem(&public, Some(&section), "none"), public)
}

// MARK: - Every form converts to one that connects (SSHKeyImportTests)

#[test]
fn every_form_ssh_keygen_and_openssl_write_imports_into_a_key_the_connect_path_loads() {
    let keys = app_keys();
    assert_eq!(keys.len(), 16);
    for key in keys {
        let imported = import(&key.text, key.passphrase(), None, "")
            .unwrap_or_else(|e| panic!("{}: {e}", key.name));
        assert_eq!(algorithm_name(&imported), key.algorithm, "{}", key.name);
        assert_eq!(
            imported.info.fingerprint, key.fingerprint,
            "{}: not the key ssh-keygen made",
            key.name
        );
        let two = |line: &str| line.split(' ').take(2).collect::<Vec<_>>().join(" ");
        assert_eq!(
            two(&imported.info.public_key),
            two(&key.public_key),
            "{}",
            key.name
        );
        assert!(
            imported
                .openssh
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n")
        );
        assert!(
            !is_encrypted(&imported.openssh),
            "{}: stored still behind its passphrase",
            key.name
        );
        assert!(reads_as_stored(&imported.openssh), "{}", key.name);
        load_private_key(&imported.openssh)
            .unwrap_or_else(|e| panic!("{}: the connect path can't load it: {e}", key.name));
        if key.algorithm == "rsa4096" {
            assert_eq!(imported.bits, Some(key.bits), "{}", key.name);
        }
    }
}

/// Before B125 none of these reached the server: the connect path reads an
/// unencrypted openssh-key-v1 file only, and converts the rest on first use.
#[test]
fn stored_as_pasted_the_connect_paths_reader_cant_read_these() {
    for key in app_keys()
        .into_iter()
        .filter(|k| k.encrypted || !k.text.contains("OPENSSH PRIVATE KEY"))
    {
        assert!(!reads_as_stored(&key.text), "{}", key.name);
        assert!(load_private_key(&key.text).is_err(), "{}", key.name);
    }
}

#[test]
fn a_key_whose_line_breaks_didnt_survive_the_paste_imports_into_one_that_loads() {
    let key = app_key("Ed25519 · OpenSSH");
    let crlf = key.text.replace('\n', "\r\n");
    let one_line = key.text.replace('\n', " ");
    for pasted in [&crlf, &one_line, &format!("\n  {}  \n\n", key.text)] {
        let imported = import(pasted, None, None, "").unwrap();
        assert_eq!(imported.info.fingerprint, key.fingerprint);
        assert!(load_private_key(&imported.openssh).is_ok());
    }
    assert!(!reads_as_stored(&crlf));
    assert!(!reads_as_stored(&one_line));
}

#[test]
fn import_knows_which_pasted_keys_need_a_passphrase() {
    for key in app_keys() {
        assert_eq!(is_encrypted(&key.text), key.encrypted, "{}", key.name);
    }
}

#[test]
fn a_key_behind_a_passphrase_isnt_imported_without_it() {
    for key in app_keys().into_iter().filter(|k| k.encrypted) {
        for none in [None, Some("")] {
            assert_eq!(
                import(&key.text, none, None, "").unwrap_err(),
                ImportError::NeedsPassphrase,
                "{}",
                key.name
            );
        }
    }
}

/// Every encryption scheme says a wrong passphrase is wrong — the three
/// the Swift test named, and every other encrypted fixture.
#[test]
fn a_wrong_passphrase_is_refused_as_wrong_in_each_encryption_scheme() {
    for key in app_keys().into_iter().filter(|k| k.encrypted) {
        assert_eq!(
            import(&key.text, Some("correct horse"), None, "").unwrap_err(),
            ImportError::WrongPassphrase,
            "{}",
            key.name
        );
    }
}

#[test]
fn keys_that_cant_sign_in_are_refused_at_import_saying_why() {
    // B126: P-384 (and P-521) now sign in, so the P-384 key the Swift test
    // refused imports, to ssh-keygen's fingerprint.
    let p384 = import(&fixture("app/ecdsa-p384"), None, None, "").unwrap();
    assert_eq!(p384.info.kind, KeyKind::EcdsaP384);
    assert_eq!(p384.record_algorithm(), SshKeyAlgorithm::Ecdsa);
    // A security key's file holds a handle; the private half is on the token.
    let mut blob = ssh_string(b"sk-ecdsa-sha2-nistp256@openssh.com");
    blob.extend(ssh_string(b"nistp256"));
    blob.extend(ssh_string(&[4; 65]));
    blob.extend(ssh_string(b"ssh:"));
    assert_eq!(
        import(&openssh_pem(&blob, None, "none"), None, None, "").unwrap_err(),
        ImportError::SecurityKey
    );
    assert!(
        !is_encrypted(&openssh_pem(&blob, None, "aes256-ctr")),
        "no passphrase to ask for when the key is refused anyway"
    );
    // ChaCha20-Poly1305 (`ssh-keygen -Z chacha20-poly1305@openssh.com`) the
    // Swift reader couldn't undo (CommonCrypto has no ChaCha20); it opens.
    let chacha = import(&fixture("app/chacha-encrypted"), Some(PASSPHRASE), None, "").unwrap();
    assert_eq!(chacha.info.kind, KeyKind::Ed25519);
    assert_eq!(
        import(
            &fixture("app/chacha-encrypted"),
            Some("correct horse"),
            None,
            ""
        )
        .unwrap_err(),
        ImportError::WrongPassphrase
    );
    assert_eq!(
        import(&fixture("app/pkcs12-encrypted"), Some(PASSPHRASE), None, "").unwrap_err(),
        ImportError::UnsupportedEncryption("PKCS #12 PBE".to_owned())
    );
    let key = app_key("RSA 2048 · OpenSSH");
    assert_eq!(
        import(&key.public_key, None, None, "").unwrap_err(),
        ImportError::Unrecognized
    );
    let lines: Vec<&str> = key.text.trim_end().split('\n').collect();
    let cut_short = [&lines[..lines.len() / 2], &[*lines.last().unwrap()]]
        .concat()
        .join("\n");
    assert_eq!(
        import(&cut_short, None, None, "").unwrap_err(),
        ImportError::Damaged
    );
    for refusal in [
        ImportError::SecurityKey,
        ImportError::Unsupported("DSA".to_owned()),
        ImportError::NeedsPassphrase,
        ImportError::Unrecognized,
        ImportError::Damaged,
        ImportError::WrongPassphrase,
        ImportError::UnsupportedEncryption("x".to_owned()),
    ] {
        assert!(!refusal.to_string().is_empty());
    }
}

#[test]
fn the_public_key_line_carries_the_comment_typed_else_the_pasted_pubs_else_the_keys_own() {
    let key = app_key("Ed25519 · OpenSSH");
    let type_and_blob = key
        .public_key
        .split(' ')
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    let typed = import(&key.text, None, Some(&key.public_key), "work laptop").unwrap();
    assert_eq!(
        typed.info.public_key,
        format!("{type_and_blob} work laptop")
    );
    assert!(read(&typed.openssh, None).unwrap().comment == "work laptop");
    let pasted = import(
        &key.text,
        None,
        Some(&format!("{type_and_blob} from-the-pub\n")),
        "",
    )
    .unwrap();
    assert_eq!(
        pasted.info.public_key,
        format!("{type_and_blob} from-the-pub")
    );
    // Nothing typed, no .pub: the comment ssh-keygen wrote into the key.
    let own = import(&key.text, None, None, "").unwrap();
    assert_eq!(own.info.public_key, key.public_key);
}

// MARK: - SSHKeyFactoryTests

#[test]
fn fingerprints_are_deterministic_sha256_prefixed_and_unpadded() {
    let fingerprint = fingerprint_of_blob(b"some-public-key-blob");
    assert!(fingerprint.starts_with("SHA256:"));
    assert_eq!(fingerprint, fingerprint_of_blob(b"some-public-key-blob"));
    assert!(!fingerprint.contains('='));
    assert_ne!(fingerprint_of_blob(b"a"), fingerprint_of_blob(b"b"));
}

#[test]
fn a_generated_ed25519_key_reads_back_as_itself() {
    let key = generate(Generate::Ed25519, "alice@laptop").unwrap();
    assert_eq!(key.info.kind, KeyKind::Ed25519);
    assert_eq!(key.record_algorithm(), SshKeyAlgorithm::Ed25519);
    assert!(key.info.public_key.starts_with("ssh-ed25519 "));
    assert!(key.info.public_key.ends_with(" alice@laptop"));
    assert!(key.info.fingerprint.starts_with("SHA256:"));
    assert!(key.openssh.contains(OPENSSH_BEGIN) && key.openssh.contains(OPENSSH_END));
    assert!(!key.info.encrypted);
    assert_eq!(key.bits, None);
    let again = import(
        &key.openssh,
        None,
        Some(&key.info.public_key),
        "alice@laptop",
    )
    .unwrap();
    assert_eq!(again.info, key.info);
    assert_eq!(import(&key.openssh, None, None, "").unwrap().info, key.info);
    let other = generate(Generate::Ed25519, "alice@laptop").unwrap();
    assert_ne!(
        other.info.public_key, key.info.public_key,
        "fresh key material each time"
    );
    assert_ne!(other.info.fingerprint, key.info.fingerprint);
}

/// RSA keys take a while to make unoptimised: one, of the smallest size
/// the app makes, with its size read back (4096 in `generated_rsa_4096`).
#[test]
fn a_generated_rsa_key_knows_its_size() {
    let key = generate(Generate::Rsa(2048), "server").unwrap();
    assert_eq!(key.info.kind, KeyKind::Rsa);
    assert_eq!(key.record_algorithm(), SshKeyAlgorithm::Rsa4096);
    assert_eq!(key.bits, Some(2048));
    assert!(key.openssh.contains(OPENSSH_BEGIN));
    assert!(!key.openssh.contains("-----BEGIN RSA PRIVATE KEY-----"));
    let fields: Vec<&str> = key.info.public_key.split(' ').collect();
    assert_eq!(fields.len(), 3);
    assert!(armor::strict_base64(fields[1]).is_some());
    assert_eq!(
        moshpit_store::ssh_keys::rsa_bits_of_public_key_line(&key.info.public_key),
        Some(2048)
    );
    assert_eq!(import(&key.openssh, None, None, "").unwrap().info, key.info);
}

#[test]
#[ignore = "an RSA-4096 key takes a minute to make unoptimised; run with --release"]
fn generated_rsa_4096() {
    let key = generate(Generate::Rsa(4096), "server").unwrap();
    assert_eq!(key.bits, Some(4096));
    assert_eq!(
        moshpit_store::ssh_keys::rsa_bits_of_public_key_line(&key.info.public_key),
        Some(4096)
    );
}

#[test]
fn a_secure_enclave_key_line_comes_from_its_point_or_its_spki() {
    // The P-256 generator point, a valid public key.
    let point: Vec<u8> = [
        "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
        "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
    ]
    .concat()
    .as_bytes()
    .chunks(2)
    .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
    .collect();
    let (line, fingerprint) = hardware_public_key(&point, "phone").unwrap();
    assert!(line.starts_with("ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY"));
    assert!(line.ends_with(" phone"));
    assert!(fingerprint.starts_with("SHA256:"));
    let (bare, _) = hardware_public_key(&point, "").unwrap();
    assert_eq!(bare.split(' ').count(), 2, "no comment, no trailing space");
    let mut spki = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    spki.extend_from_slice(&point);
    assert_eq!(hardware_public_key(&spki, "phone").unwrap().0, line);
    assert!(hardware_public_key(&spki[1..], "x").is_err());
    assert!(hardware_public_key(&[2; 33], "x").is_err(), "compressed");
}

#[test]
fn import_refuses_text_that_isnt_a_private_key() {
    for text in ["just some pasted text", "hello", "  ", ""] {
        assert_eq!(
            import(text, None, None, "").unwrap_err(),
            ImportError::Unrecognized
        );
    }
    assert_eq!(
        import("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI x", None, None, "").unwrap_err(),
        ImportError::Unrecognized
    );
}

/// These used to import — RSA and ECDSA named from the header alone — and
/// then fail every connection.
#[test]
fn a_pem_body_with_no_key_in_it_is_refused_not_named_from_its_header() {
    for label in [
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
    ] {
        let pem = format!("-----BEGIN {label}-----\nAAAA\n-----END {label}-----");
        assert_eq!(
            import(&pem, None, None, "").unwrap_err(),
            ImportError::Damaged,
            "{label}"
        );
    }
}

#[test]
fn a_public_key_line_cant_stand_in_for_a_key_body() {
    let pem = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----";
    let line = format!("ssh-rsa {} comment", armor::base64_encode(b"rsa-blob"));
    assert_eq!(
        import(pem, None, Some(&line), "").unwrap_err(),
        ImportError::Damaged
    );
}

#[test]
fn import_ignores_whitespace_around_the_pasted_key() {
    let generated = generate(Generate::Ed25519, "c").unwrap();
    let imported = import(
        &format!("\n\n  {}  \n", generated.openssh.trim()),
        None,
        None,
        "",
    )
    .unwrap();
    assert_eq!(imported.info.fingerprint, generated.info.fingerprint);
    assert_eq!(imported.info.public_key, generated.info.public_key);
}

#[test]
fn pasted_keys_lose_their_indents_and_crlf() {
    let key = generate(Generate::Ed25519, "x").unwrap();
    let mangled = key
        .openssh
        .lines()
        .map(|l| format!("   {l}  "))
        .collect::<Vec<_>>()
        .join("\r\n");
    assert_eq!(import(&mangled, None, None, "").unwrap().info, key.info);
}

#[test]
fn an_openssh_ecdsa_key_is_ecdsa_with_or_without_its_pub() {
    let scalar =
        <p256::SecretKey as p256::elliptic_curve::Generate>::generate_from_rng(&mut rand::rng())
            .to_bytes();
    let (pem, public) = openssh_p256(&scalar, "p256@host");
    let alone = import(&pem, None, None, "").unwrap();
    assert_eq!(alone.record_algorithm(), SshKeyAlgorithm::Ecdsa);
    assert_eq!(alone.info.fingerprint, fingerprint_of_blob(&public));
    let line = format!(
        "ecdsa-sha2-nistp256 {} p256@host",
        armor::base64_encode(&public)
    );
    let with_pub = import(&pem, None, Some(&line), "").unwrap();
    assert_eq!(with_pub.record_algorithm(), SshKeyAlgorithm::Ecdsa);
    assert_eq!(with_pub.info.public_key, line);
}

/// A P-256 scalar whose first byte is zero and second below 0x80 is
/// written by OpenSSH (and was by the app's Swift import) as a 31-byte
/// mpint: no sign byte is needed. ssh-key's own reader wants at least 32
/// bytes, so before the connect path read keys with this reader, one P-256
/// key in 512 imported fine and could never sign in on the Rust path. Both
/// read it now, and it converts to nothing else.
#[test]
fn a_p256_key_whose_scalar_starts_with_a_zero_byte_reads_and_signs() {
    let mut rng = rand::rng();
    let scalar = loop {
        let secret =
            <p256::SecretKey as p256::elliptic_curve::Generate>::generate_from_rng(&mut rng);
        let bytes = secret.to_bytes();
        if bytes[0] == 0 && bytes[1] < 0x80 {
            break bytes;
        }
    };
    let (pem, public) = openssh_p256(&scalar, "short");
    assert!(
        PrivateKey::from_openssh(&pem).is_err(),
        "ssh-key reads it after all: the test no longer shows the bug"
    );
    let key = load_private_key(&pem).unwrap();
    assert_eq!(material::public_blob(key.key_data()).unwrap(), public);
    assert_eq!(assess(&pem, None).unwrap(), Assessment::Usable);
    let stored = import(&pem, None, None, "").unwrap();
    assert_eq!(stored.info.fingerprint, fingerprint_of_blob(&public));
    match signing_key(&pem).unwrap() {
        SigningKey::Ecdsa { curve, scalar: got } => {
            assert_eq!(curve, EcdsaCurve::P256);
            assert_eq!(&got[..], &scalar[..], "the full 32-byte scalar");
        }
        other => panic!("{other:?}"),
    }
}

/// The Swift test refused P-384 and P-521 by their public half alone; B126
/// signs in with both, so a stand-in container is refused only as what it
/// is — damaged — and real keys of both curves import.
#[test]
fn openssh_ecdsa_keys_on_the_larger_curves_import() {
    for (key_type, curve) in [
        ("ecdsa-sha2-nistp384", "nistp384"),
        ("ecdsa-sha2-nistp521", "nistp521"),
    ] {
        let mut blob = ssh_string(key_type.as_bytes());
        blob.extend(ssh_string(curve.as_bytes()));
        blob.extend(ssh_string(&[4; 97]));
        assert_eq!(
            import(&openssh_pem(&blob, None, "none"), None, None, "").unwrap_err(),
            ImportError::Damaged,
            "{key_type}"
        );
    }
    let fingerprints = fixture("../fingerprints.txt");
    for (name, kind) in [
        ("ecdsa384", KeyKind::EcdsaP384),
        ("ecdsa521", KeyKind::EcdsaP521),
    ] {
        let stored = import(&fixture(name), None, None, "").unwrap();
        assert_eq!(stored.info.kind, kind);
        assert!(
            fingerprints.contains(&format!("{name} "))
                && fingerprints.contains(&stored.info.fingerprint)
        );
        assert_eq!(refused_type(&stored.info.public_key), None);
    }
}

#[test]
fn an_openssh_key_of_a_type_the_app_cant_sign_with_is_refused() {
    let mut blob = ssh_string(b"ssh-dss");
    blob.extend(ssh_string(&[1; 20]));
    assert_eq!(
        import(&openssh_pem(&blob, None, "none"), None, None, "").unwrap_err(),
        ImportError::Unsupported("DSA".to_owned())
    );
    let mut sk = ssh_string(b"sk-ssh-ed25519@openssh.com");
    sk.extend(ssh_string(&[1; 32]));
    sk.extend(ssh_string(b"ssh:"));
    assert_eq!(
        import(&openssh_pem(&sk, None, "none"), None, None, "").unwrap_err(),
        ImportError::SecurityKey
    );
}

#[test]
fn the_comment_typed_on_import_ends_the_public_key_line() {
    let generated = generate(Generate::Ed25519, "generated").unwrap();
    let imported = import(&generated.openssh, None, None, "work laptop").unwrap();
    let fields: Vec<&str> = generated.info.public_key.split(' ').collect();
    assert_eq!(
        imported.info.public_key,
        format!("{} {} work laptop", fields[0], fields[1])
    );
    assert_eq!(imported.info.fingerprint, generated.info.fingerprint);
}

#[test]
fn with_no_comment_typed_the_pasted_pub_keeps_its_own_without_the_files_newline() {
    let generated = generate(Generate::Ed25519, "alice@laptop").unwrap();
    // What "Import File…" puts in the field: the .pub file, newline and all.
    let imported = import(
        &generated.openssh,
        None,
        Some(&format!("{}\n", generated.info.public_key)),
        "",
    )
    .unwrap();
    assert_eq!(imported.info.public_key, generated.info.public_key);
    // A .pub line whose blob isn't base64 contributes nothing.
    let own = import(&generated.openssh, None, Some("ssh-ed25519 !!! x"), "").unwrap();
    assert_eq!(own.info.public_key, generated.info.public_key);
}

// MARK: - StoredKeyTests

#[test]
fn every_form_a_key_was_pasted_in_comes_out_as_an_openssh_key_v1_file_of_the_same_key() {
    for key in app_keys() {
        let expected = key.keygen_blob();
        let mut assessment = assess(&key.text, None).unwrap();
        if key.encrypted {
            assert_eq!(assessment, Assessment::NeedsPassphrase, "{}", key.name);
            assessment = assess(&key.text, key.passphrase()).unwrap();
        }
        match assessment {
            Assessment::Usable => {
                assert!(reads_as_stored(&key.text), "{}", key.name);
                assert_eq!(blob_of(&key.text), expected, "{}", key.name);
            }
            Assessment::Converted(converted) => {
                assert!(converted.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
                assert!(reads_as_stored(&converted), "{}", key.name);
                assert!(!is_encrypted(&converted));
                assert_eq!(
                    blob_of(&converted),
                    expected,
                    "{}: not the same key",
                    key.name
                );
                assert!(
                    !reads_as_stored(&key.text) || key.encrypted,
                    "{}: read as stored, converted anyway",
                    key.name
                );
            }
            Assessment::NeedsPassphrase => panic!("{}: still wants a passphrase", key.name),
        }
    }
}

#[test]
fn windows_line_endings_and_indented_lines_convert_to_the_same_key() {
    let key = app_key("Ed25519 · OpenSSH");
    let crlf = |t: &str| t.replace('\n', "\r\n");
    let indented = |t: &str| {
        t.split('\n')
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for stored in [
        crlf(&key.text),
        indented(&key.text),
        crlf(&indented(&key.text)),
    ] {
        assert!(!reads_as_stored(&stored));
        let Assessment::Converted(converted) = assess(&stored, None).unwrap() else {
            panic!("not converted");
        };
        assert!(reads_as_stored(&converted));
        assert_eq!(blob_of(&converted), key.keygen_blob());
    }
}

#[test]
fn a_stored_key_that_cant_sign_in_says_which_kind_it_is() {
    // B126: the P-384 key the Swift test refused here is usable as stored.
    assert_eq!(
        assess(&fixture("app/ecdsa-p384"), None).unwrap(),
        Assessment::Usable
    );
    let key = app_key("RSA 2048 · OpenSSH · passphrase");
    assert_eq!(
        assess(&key.text, Some("not it")).unwrap_err(),
        ImportError::WrongPassphrase
    );
    let mut blob = ssh_string(b"ssh-dss");
    blob.extend(ssh_string(&[1; 20]));
    assert_eq!(
        assess(&openssh_pem(&blob, None, "none"), None).unwrap_err(),
        ImportError::Unsupported("DSA".to_owned())
    );
}

#[test]
fn the_picker_can_tell_a_key_that_cant_sign_in_from_its_public_key_line() {
    assert_eq!(refused_type("ssh-dss AAAA").as_deref(), Some("DSA"));
    // B126: no longer refused.
    assert_eq!(refused_type("ecdsa-sha2-nistp384 AAAA x"), None);
    assert_eq!(refused_type("ecdsa-sha2-nistp521 AAAA"), None);
    for usable in [
        "ssh-ed25519 AAAA",
        "ssh-rsa AAAA c",
        "ecdsa-sha2-nistp256 AAAA",
        "",
        "garbage",
    ] {
        assert_eq!(refused_type(usable), None, "{usable}");
    }
}

#[test]
fn a_key_that_already_signs_in_is_left_exactly_as_stored() {
    for name in ["ed25519", "rsa2048", "ecdsa256", "ecdsa384", "ecdsa521"] {
        let text = fixture(name);
        assert_eq!(assess(&text, None).unwrap(), Assessment::Usable, "{name}");
        assert_eq!(*storable(&text), text, "{name}");
    }
}

// MARK: - PastedKeyStorageTests (the connection form, B133)

#[test]
fn mangled_in_the_paste_it_is_stored_as_the_key_the_connect_path_reads() {
    let key = app_key("Ed25519 · OpenSSH").text;
    let indented = |t: &str| {
        t.split('\n')
            .map(|l| format!("\t  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for pasted in [
        key.replace('\n', "\r\n"),
        indented(&key),
        key.replace('\n', " "),
        format!("\n\n{}\r\n\r\n", indented(&key.replace('\n', "\r\n"))),
    ] {
        assert!(!reads_as_stored(&pasted), "read as pasted after all");
        let stored = storable(&pasted);
        assert!(reads_as_stored(&stored));
        assert_eq!(blob_of(&stored), blob_of(&key));
    }
}

#[test]
fn a_pem_paste_is_stored_converted_a_clean_paste_exactly_as_pasted() {
    let pem = app_key("RSA 2048 · PEM (ssh-keygen -m PEM)").text;
    let stored = storable(&pem);
    assert!(stored.starts_with(OPENSSH_BEGIN));
    assert_eq!(blob_of(&stored), blob_of(&pem));
    let clean = app_key("Ed25519 · OpenSSH").text;
    assert_eq!(*storable(&clean), clean);
}

#[test]
fn behind_a_passphrase_or_not_a_key_at_all_kept_as_pasted() {
    let encrypted = app_key("Ed25519 · OpenSSH · passphrase").text;
    assert_eq!(*storable(&encrypted), encrypted);
    assert_eq!(*storable("not a key"), "not a key");
    assert_eq!(*storable(""), "");
    let p384 = fixture("app/ecdsa-p384");
    assert_eq!(*storable(&p384), p384, "clean: kept as it is");
}

#[test]
fn imported_with_every_line_indented_stored_as_the_key_the_connect_path_reads() {
    let key = app_key("Ed25519 · OpenSSH");
    let indented = key
        .text
        .split('\n')
        .map(|l| format!("\t  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let imported = import(&indented, None, None, "").unwrap();
    assert_eq!(imported.info.fingerprint, key.fingerprint);
    assert!(reads_as_stored(&imported.openssh));
}

// MARK: - SSHKeyRecordTests, RSASHA2SigningTests, KeychainReferencesTests

#[test]
fn an_rsa_keys_size_is_its_own() {
    let imported = import(&app_key("RSA 2048 · OpenSSH").text, None, None, "").unwrap();
    assert_eq!(imported.record_algorithm(), SshKeyAlgorithm::Rsa4096);
    assert_eq!(imported.bits, Some(2048));
    assert_eq!(
        import(&fixture("rsa4096"), None, None, "").unwrap().bits,
        Some(4096)
    );
}

/// The numbers the Swift connect path signs RSA with (rsa-sha2-512/256,
/// B129) are the key's: `n` and `e` make its public blob, `p·q = n`.
#[test]
fn an_rsa_keys_numbers_make_its_public_key() {
    let key = app_key("RSA 2048 · OpenSSH");
    let SigningKey::Rsa { n, e, d, p, q } = signing_key(&key.text).unwrap() else {
        panic!("not RSA");
    };
    let mut blob = ssh_string(b"ssh-rsa");
    blob.extend(mpint(&e));
    blob.extend(mpint(&n));
    assert_eq!(blob, key.keygen_blob());
    assert!(
        n[0] != 0 && d[0] != 0 && p[0] != 0 && q[0] != 0,
        "no leading zeros"
    );
    let to = |b: &[u8]| rsa::BoxedUint::from_be_slice_vartime(b);
    rsa::RsaPrivateKey::from_components(to(&n), to(&e), to(&d), vec![to(&p), to(&q)])
        .expect("a key whose numbers agree");
    for name in ["ed25519", "ecdsa384", "ecdsa521"] {
        let parts = signing_key(&fixture(name)).unwrap();
        match (name, parts) {
            ("ed25519", SigningKey::Ed25519 { seed }) => assert_eq!(seed.len(), 32),
            ("ecdsa384", SigningKey::Ecdsa { curve, scalar }) => {
                assert_eq!((curve, scalar.len()), (EcdsaCurve::P384, 48));
            }
            ("ecdsa521", SigningKey::Ecdsa { curve, scalar }) => {
                assert_eq!((curve, scalar.len()), (EcdsaCurve::P521, 66));
            }
            (name, other) => panic!("{name}: {other:?}"),
        }
    }
}

/// An orphaned keychain item is described from the key alone; a password,
/// or a key behind a passphrase, is no key.
#[test]
fn a_stored_key_is_described_from_the_key_alone() {
    let lost = generate(Generate::Ed25519, "lost").unwrap();
    let described = describe_stored(&lost.openssh).unwrap();
    assert_eq!(described.info.fingerprint, lost.info.fingerprint);
    assert_eq!(described.info.public_key, lost.info.public_key);
    assert_eq!(described.record_algorithm(), SshKeyAlgorithm::Ed25519);
    assert!(describe_stored("hunter2").is_none());
    assert!(describe_stored("pw").is_none());
    assert!(describe_stored(&app_key("Ed25519 · OpenSSH · passphrase").text).is_none());
    let pem = app_key("ECDSA P-256 · PEM (ssh-keygen -m PEM)");
    assert_eq!(
        describe_stored(&pem.text).unwrap().info.fingerprint,
        pem.fingerprint
    );
}

// MARK: - The fixture keys, cut short (B137) and every other way

/// Every fixture key — OpenSSH, PKCS#8 EC, PKCS#1 RSA, the encrypted one
/// with its passphrase — imports to the fingerprint `ssh-keygen -l` gives
/// its public half.
#[test]
fn the_fixture_keys_import_to_their_fingerprints() {
    let listing = fixture("../fingerprints.txt");
    let mut seen = 0;
    for line in listing.lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (name, bits, fingerprint) = (fields[0], fields[1], fields[2]);
        let text = fixture(name);
        let passphrase = name.ends_with("-encrypted").then_some("fixture");
        if passphrase.is_some() {
            assert_eq!(
                import(&text, None, None, "").unwrap_err(),
                ImportError::NeedsPassphrase,
                "{name}"
            );
            assert_eq!(
                import(&text, Some("not it"), None, "").unwrap_err(),
                ImportError::WrongPassphrase,
                "{name}"
            );
        }
        let stored = import(&text, passphrase, None, "").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(stored.info.fingerprint, fingerprint, "{name}");
        assert!(!stored.info.encrypted, "{name}");
        assert!(
            stored
                .openssh
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n")
        );
        if name.starts_with("rsa") {
            assert_eq!(stored.bits, Some(bits.parse().unwrap()), "{name}");
        }
        seen += 1;
    }
    assert_eq!(seen, 9);
}

/// B137: a key whose text was cut short (a paste that lost its end, a file
/// copied half-way) is refused at import, in every form, with or without
/// its armour's last line: the private half is read, not just the public
/// half at the front, which is all a cut OpenSSH key still has whole.
#[test]
fn a_key_cut_short_is_refused_at_import() {
    let names = [
        "ed25519",
        "ed25519-encrypted",
        "ecdsa256",
        "ecdsa256-pkcs8",
        "ecdsa384",
        "ecdsa521",
        "rsa2048",
        "rsa2048-pem",
        "rsa4096",
    ];
    let mut cases = 0;
    for name in names {
        let text = fixture(name);
        let passphrase = name.ends_with("-encrypted").then_some("fixture");
        let lines: Vec<&str> = text.trim().lines().collect();
        let (first, body, last) = (lines[0], &lines[1..lines.len() - 1], lines[lines.len() - 1]);
        let joined = body.concat();
        for keep in [joined.len() / 2, joined.len() * 9 / 10, joined.len() - 8] {
            // Cut on a base64 quantum, so it still decodes.
            let keep = keep - keep % 4;
            let cut = &joined[..keep];
            for armoured in [
                format!("{first}\n{cut}\n{last}\n"),
                format!("{first}\n{cut}\n"),
            ] {
                let result = import(&armoured, passphrase, None, "").map(|k| k.info.fingerprint);
                // Said as broken: not a passphrase to ask for or retry.
                assert_eq!(
                    result,
                    Err(ImportError::Damaged),
                    "{name} cut to {keep} of {}",
                    joined.len()
                );
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 54);
}

/// One byte flipped anywhere in a key's body never imports a different key:
/// it is refused, or (a flip in the comment, the padding or a check that
/// still matches) reads as the same key.
#[test]
fn a_garbled_key_never_imports_as_another_key() {
    for name in [
        "ed25519",
        "ecdsa256",
        "ecdsa384",
        "rsa2048",
        "rsa2048-pem",
        "ecdsa256-pkcs8",
    ] {
        let text = fixture(name);
        let expected = blob_of(&text);
        let armor = Armor::find(&text).unwrap();
        let label = armor.label.clone();
        let step = (armor.body.len() / 97).max(1);
        for at in (0..armor.body.len()).step_by(step) {
            let mut body = armor.body.to_vec();
            body[at] ^= 0x5a;
            let garbled = format!(
                "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
                armor::base64_encode(&body)
            );
            if let Ok(key) = read(&garbled, None) {
                assert_eq!(key.public_blob(), expected, "{name} byte {at}");
            }
        }
    }
}

#[test]
fn putty_keys_import_and_their_passphrase_is_asked_for() {
    let ed = import(&fixture("putty/id_ed25519.ppk"), None, None, "").unwrap();
    assert_eq!(ed.info.kind, KeyKind::Ed25519);
    assert!(ed.info.public_key.ends_with(" user@example.com"));
    let encrypted = fixture("putty/id_ed25519_enc.ppk");
    assert!(is_encrypted(&encrypted));
    assert_eq!(
        import(&encrypted, None, None, "").unwrap_err(),
        ImportError::NeedsPassphrase
    );
    assert_eq!(
        import(&encrypted, Some("124"), None, "").unwrap_err(),
        ImportError::WrongPassphrase
    );
    assert_eq!(
        import(&encrypted, Some("123"), None, "")
            .unwrap()
            .info
            .public_key,
        ed.info.public_key
    );
    assert_eq!(
        import(&fixture("putty/id_ecdsa_p256.ppk"), None, None, "")
            .unwrap()
            .info
            .kind,
        KeyKind::EcdsaP256
    );
    let rsa = import(&fixture("putty/id_rsa_3072.ppk2"), None, None, "").unwrap();
    assert_eq!(rsa.bits, Some(3072));
    assert_eq!(
        import(
            &fixture("putty/id_rsa_3072_enc.ppk2"),
            Some("123"),
            None,
            ""
        )
        .unwrap()
        .info,
        rsa.info
    );
    assert_eq!(
        import(&fixture("putty/id_dsa_1024.ppk"), None, None, "").unwrap_err(),
        ImportError::Unsupported("DSA".to_owned())
    );
}

/// Keys generated, imported and converted here are what ssh-keygen writes:
/// 70-column base64, `\n` endings, no trailing whitespace — the form
/// 1.0.8's readers take.
#[test]
fn what_is_stored_is_written_the_way_ssh_keygen_writes_it() {
    let mut texts = vec![
        generate(Generate::Ed25519, "a").unwrap().openssh,
        generate(Generate::Rsa(2048), "b").unwrap().openssh,
    ];
    for key in app_keys() {
        texts.push(
            import(&key.text, key.passphrase(), None, "")
                .unwrap()
                .openssh,
        );
    }
    for text in texts {
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], OPENSSH_BEGIN);
        assert_eq!(lines[lines.len() - 2], OPENSSH_END);
        assert_eq!(lines[lines.len() - 1], "", "one final newline");
        let body = &lines[1..lines.len() - 2];
        assert!(body[..body.len() - 1].iter().all(|l| l.len() == 70));
        assert!(body.last().unwrap().len() <= 70);
        assert!(!text.contains('\r'));
        assert!(body.iter().all(|l| !l.contains(' ') && !l.contains('\t')));
        assert!(matches!(
            read(&text, None).unwrap().key.key_data(),
            KeypairData::Ecdsa(_) | KeypairData::Ed25519(_) | KeypairData::Rsa(_)
        ));
    }
}

/// How the matrix's manifest names an import's outcome.
fn outcome(result: Result<StoredKey, ImportError>) -> String {
    match result {
        Ok(stored) => stored.info.fingerprint,
        Err(ImportError::Unsupported(what)) => format!("error Unsupported {what}"),
        Err(ImportError::UnsupportedEncryption(what)) => {
            format!("error UnsupportedEncryption {what}")
        }
        Err(other) => format!("error {other:?}"),
    }
}

/// Every key in the matrix (`harness/make_key_matrix.py`: ssh-keygen's and
/// OpenSSL's every form, cipher and PRF, and the kinds refused) comes to
/// what the tools say: the fingerprint of the key they made, or the refusal
/// that names it. Behind a passphrase, none is asked for first and a wrong
/// one is said to be wrong.
#[test]
fn every_key_in_the_matrix_comes_to_what_the_tools_say() {
    let manifest = fixture("matrix/manifest.tsv");
    let (mut imported, mut refused) = (0, 0);
    for line in manifest.lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        let (name, passphrase, expected) = (fields[0], fields[1], fields[2]);
        let passphrase = (passphrase != "-").then_some(passphrase);
        let text = fixture(&format!("matrix/{name}"));
        assert_eq!(
            outcome(import(&text, passphrase, None, "")),
            expected,
            "{name}"
        );
        if expected.starts_with("error") {
            refused += 1;
            continue;
        }
        imported += 1;
        assert_eq!(is_encrypted(&text), passphrase.is_some(), "{name}");
        if passphrase.is_some() {
            assert_eq!(
                outcome(import(&text, None, None, "")),
                "error NeedsPassphrase",
                "{name}"
            );
            assert_eq!(
                outcome(import(&text, Some("not the pass"), None, "")),
                "error WrongPassphrase",
                "{name}"
            );
        }
        let stored = import(&text, passphrase, None, "").unwrap();
        assert!(reads_as_stored(&stored.openssh), "{name}");
        load_private_key(&stored.openssh).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    assert_eq!((imported, refused), (113, 12));
}

// MARK: - What older builds read (the format promise)

/// Keys as this reader writes them, committed where the app's Swift tests
/// read them (`Tests/MoshpitTests/Fixtures/keys-written-by-rust`): there
/// `StoredKeyCompatibilityTests` reads each with 1.0.8's own readers — the
/// Citadel revision 1.0.8 pins and a verbatim copy of its P-256 reader —
/// on the device. Each is every app fixture key imported, the matrix's
/// P-384 and P-521, a P-256 key whose scalar starts with a zero byte (the
/// mpint width 1.0.8 has to cope with), and generated Ed25519 and RSA-4096.
fn rust_written() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../Tests/MoshpitTests/Fixtures/keys-written-by-rust")
}

#[test]
#[ignore = "writes Tests/MoshpitTests/Fixtures/keys-written-by-rust; run with --release when the format changes"]
fn write_keys_for_older_builds() {
    let dir = rust_written();
    std::fs::create_dir_all(&dir).unwrap();
    let mut manifest = String::from(
        "# Written by keyfile::tests::write_keys_for_older_builds: file, fingerprint, record algorithm, comment.\n",
    );
    let mut write = |name: &str, key: StoredKey| {
        std::fs::write(dir.join(name), key.openssh.as_bytes()).unwrap();
        manifest.push_str(&format!(
            "{name}\t{}\t{}\t{}\n",
            key.info.fingerprint,
            key.record_algorithm().raw_value(),
            key.info.comment
        ));
    };
    for key in app_keys() {
        let file = key
            .name
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        write(
            &format!("imported-{file}"),
            import(&key.text, key.passphrase(), None, "").unwrap(),
        );
    }
    for name in [
        "keygen-ecdsa384-openssh-chacha20-poly1305",
        "openssl-p521-pkcs8-explicit",
    ] {
        let passphrase = name.contains("chacha").then_some("matrix pass");
        let text = fixture(&format!("matrix/{name}"));
        write(
            &format!("imported-{name}"),
            import(&text, passphrase, None, "").unwrap(),
        );
    }
    let mut rng = rand::rng();
    let scalar = loop {
        let secret =
            <p256::SecretKey as p256::elliptic_curve::Generate>::generate_from_rng(&mut rng);
        if secret.to_bytes()[0] == 0 {
            break secret.to_bytes();
        }
    };
    let (pem, _) = openssh_p256(&scalar, "zero-led scalar");
    write(
        "imported-p256-zero-led-scalar",
        import(&pem, None, None, "").unwrap(),
    );
    write(
        "generated-ed25519",
        generate(Generate::Ed25519, "made here").unwrap(),
    );
    write(
        "generated-rsa4096",
        generate(Generate::Rsa(4096), "made here").unwrap(),
    );
    std::fs::write(dir.join("manifest.tsv"), manifest).unwrap();
}

/// The committed keys still read as what they were written as, and as
/// stored: a key the keychain holds in this form is never rewritten.
#[test]
fn keys_written_for_older_builds_read_back_as_stored() {
    let dir = rust_written();
    let manifest = std::fs::read_to_string(dir.join("manifest.tsv")).unwrap();
    let mut seen = 0;
    for line in manifest.lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        let text = std::fs::read_to_string(dir.join(fields[0])).unwrap();
        let stored = import(&text, None, None, "").unwrap();
        assert_eq!(stored.info.fingerprint, fields[1], "{}", fields[0]);
        assert_eq!(
            stored.record_algorithm().raw_value(),
            fields[2],
            "{}",
            fields[0]
        );
        assert_eq!(
            read(&text, None).unwrap().comment,
            fields[3],
            "{}",
            fields[0]
        );
        assert_eq!(
            assess(&text, None).unwrap(),
            Assessment::Usable,
            "{}",
            fields[0]
        );
        assert_eq!(*storable(&text), text);
        load_private_key(&text).unwrap();
        seen += 1;
    }
    assert_eq!(seen, 21);
}

/// A key file that asks for more work than anyone gives a key — a byte
/// garbled in its bcrypt rounds or PBKDF2 count, or made to — is refused at
/// once, not opened over hours (the Swift reader hung on these).
#[test]
fn a_key_asking_for_endless_key_derivation_is_refused_at_once() {
    let text = fixture("ed25519-encrypted");
    let armor = Armor::find(&text).unwrap();
    let mut body = armor.body.to_vec();
    // magic, "aes256-ctr", "bcrypt", then the options: salt, rounds.
    let rounds_at = 15 + 4 + 10 + 4 + 6 + 4 + 4 + 16;
    // ssh-keygen 10.3's default: 24 rounds.
    assert_eq!(&body[rounds_at..rounds_at + 4], &24u32.to_be_bytes());
    body[rounds_at] ^= 0x5a;
    let garbled = format!(
        "{OPENSSH_BEGIN}\n{}\n{OPENSSH_END}\n",
        armor::base64_encode(&body)
    );
    let started = std::time::Instant::now();
    assert_eq!(
        import(&garbled, Some("fixture"), None, "").unwrap_err(),
        ImportError::UnsupportedEncryption(format!("bcrypt with {} rounds", 0x5a00_0018u32))
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    // And the most a key may ask for still opens.
    const { assert!(openssh::MAX_BCRYPT_ROUNDS >= 1024 && pkcs::MAX_PBKDF2_ROUNDS >= 1 << 20) };
}

/// The other way: keys as the app's Swift code stored them, before the
/// move (`fixtures/keys/swift-written`, written by the Swift harness's
/// `write-keys` over that code as of 81c121a1 — imports of every app test
/// key and the matrix's samples, and keys it generated) read back here as
/// exactly what their records say, and as stored: a key already in a
/// person's keychain is never rewritten, and shows as it always did.
#[test]
fn keys_the_swift_code_stored_read_back_unchanged() {
    let manifest = fixture("swift-written/manifest.tsv");
    let mut seen = 0;
    for line in manifest.lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        let (name, fingerprint, algorithm, public_line, bits) =
            (fields[0], fields[1], fields[2], fields[3], fields[4]);
        let text = fixture(&format!("swift-written/{name}"));
        assert_eq!(assess(&text, None).unwrap(), Assessment::Usable, "{name}");
        assert!(reads_as_stored(&text), "{name}");
        load_private_key(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        for stored in [
            import(&text, None, None, "").unwrap(),
            describe_stored(&text).unwrap(),
        ] {
            assert_eq!(stored.info.fingerprint, fingerprint, "{name}");
            assert_eq!(stored.record_algorithm().raw_value(), algorithm, "{name}");
            assert_eq!(stored.info.public_key, public_line, "{name}");
            assert_eq!(
                stored.bits.map_or("-".to_owned(), |b| b.to_string()),
                bits,
                "{name}"
            );
        }
        seen += 1;
    }
    assert!(seen >= 24, "{seen}");
}

/// B780: a stored key no connect path signs with says what kind it is, by
/// name — a brainpool curve's, not its OID — so the key list and the
/// picker can say so before a sign-in fails. A key that signs, one behind a
/// passphrase and a password say nothing.
#[test]
fn a_stored_key_that_cant_sign_says_what_kind_it_is() {
    assert_eq!(
        refused_kind(&fixture("matrix/openssl-brainpool256-sec1")).as_deref(),
        Some("ECDSA brainpoolP256r1")
    );
    assert_eq!(
        refused_kind(&fixture("matrix/openssl-dsa-traditional")).as_deref(),
        Some("DSA")
    );
    assert_eq!(
        refused_kind(&fixture("matrix/openssl-secp256k1-sec1")).as_deref(),
        Some("ECDSA secp256k1")
    );
    for signs in [
        "matrix/openssl-p256-sec1",
        "matrix/keygen-rsa2048-pkcs8",
        "ed25519",
    ] {
        assert_eq!(refused_kind(&fixture(signs)), None, "{signs}");
    }
    assert_eq!(
        refused_kind(&fixture("matrix/keygen-ecdsa256-pem-encrypted")),
        None
    );
    assert_eq!(refused_kind("hunter2"), None);
}
