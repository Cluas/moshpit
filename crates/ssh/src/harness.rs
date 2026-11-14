//! The answers the Swift cross-check compares (harness/equivalence.py,
//! tests/swift_equivalence.rs), in the Swift side's words: one function per
//! harness command, each output exactly as harness/swift/main.swift prints
//! its own.

use std::time::Duration;

use crate::auth::{KeyKind, inspect_private_key};
use crate::error::KeyError;
use crate::host_key::HostKey;
use crate::target::{ConnectOptions, Socks5Proxy};
use crate::upload::ExecUploadPlan;

/// `fingerprint <line>`: `<algorithm> <fingerprint>`, or `error`.
pub fn fingerprint(line: &str) -> String {
    match HostKey::from_openssh(line) {
        Some(key) => describe(&key),
        None => "error".to_owned(),
    }
}

pub fn describe(key: &HostKey) -> String {
    format!("{} {}", key.algorithm(), key.fingerprint())
}

/// Citadel's five key types; anything else it calls unsupported.
const CITADEL_TYPES: &[&str] = &[
    "ssh-rsa",
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
];

/// `keytype <file>`: the key's type as `SSHKeyDetection.detectPrivateKeyType`
/// reports it (`type <name>` or `error <case>`).
pub fn keytype(text: &str) -> String {
    match inspect_private_key(text) {
        Ok(info) if CITADEL_TYPES.contains(&info.kind.ssh_name()) => {
            format!("type {}", info.kind.ssh_name())
        }
        Ok(info) => format!("error unsupportedKeyType {}", info.kind.ssh_name()),
        Err(KeyError::NotOpenSsh | KeyError::Malformed(_)) => {
            "error invalidPrivateKeyFormat".to_owned()
        }
        Err(e) => format!("error {e}"),
    }
}

/// `import <file>`: `<label>\t<fingerprint>\t<type> <base64> [<comment>]`,
/// as `SSHKeyFactory.importKey` answers for a key pasted without its
/// `.pub`: the app's label for the type (`SSHKeyAlgorithm`, RSA of any size
/// being `RSA-4096`), the fingerprint, the `authorized_keys` line with the
/// key's own comment (as `ssh-keygen -y` prints it; the app keeps it since
/// it reads the whole container, B29); `error` for what it refuses.
pub fn import(text: &str) -> String {
    let Ok(info) = inspect_private_key(text) else {
        return "error".to_owned();
    };
    let label = match info.kind {
        KeyKind::Ed25519 => "ED25519",
        KeyKind::Rsa => "RSA-4096",
        KeyKind::EcdsaP256 | KeyKind::EcdsaP384 | KeyKind::EcdsaP521 => "ECDSA",
        KeyKind::SkEcdsaP256 => "ECDSA-sk",
        _ => return "error".to_owned(),
    };
    format!("{label}\t{}\t{}", info.fingerprint, info.public_key)
}

/// `upload <home> <filename>`: the plan's four strings, one per line.
pub fn upload(home: &str, filename: &str) -> String {
    let plan = ExecUploadPlan::new(home, filename);
    format!(
        "begin\t{}\nappend\t{}\nfinish\t{}\nfinal\t{}",
        plan.begin,
        plan.append("QUJD"),
        plan.finish,
        plan.final_path
    )
}

/// `socks <proxy port> <host> <port>`: what the TCP leg through a proxy on
/// the loopback comes to — `connected`, or `error` — within `patience`
/// (`hung` past it), the connect timeout set well inside it.
pub async fn socks(proxy_port: u16, host: &str, port: u16, patience: Duration) -> String {
    let options = ConnectOptions {
        connect_timeout: patience / 2,
        proxy: Some(Socks5Proxy::new("127.0.0.1", proxy_port)),
        ..ConnectOptions::default()
    };
    match tokio::time::timeout(patience, crate::dial::dial(host, port, &options)).await {
        Ok(Ok(_)) => "connected".to_owned(),
        Ok(Err(_)) => "error".to_owned(),
        Err(_) => "hung".to_owned(),
    }
}

/// The key reader's answer to one case of the key cross-check
/// (`harness/keys_crosscheck.py`, `keys` in harness/swift/main.swift):
/// `id <TAB> op <TAB> file <TAB> passphrase|- <TAB> public line file|- <TAB> comment`.
pub fn key_case(fields: &[&str]) -> String {
    use crate::keyfile::{self, Assessment, ImportError};
    fn error(e: ImportError) -> String {
        match e {
            ImportError::Unsupported(what) => format!("error Unsupported {what}"),
            ImportError::UnsupportedEncryption(what) => {
                format!("error UnsupportedEncryption {what}")
            }
            other => format!("error {other:?}"),
        }
    }
    fn read_back(text: &str) -> String {
        match keyfile::read(text, None) {
            Ok(contents) => format!(
                "{} [{}]",
                crate::host_key::fingerprint_of_blob(&contents.public_blob()),
                contents.comment
            ),
            Err(_) => "unreadable".to_owned(),
        }
    }
    let (op, path) = (fields[1], fields[2]);
    let passphrase = (fields[3] != "-").then_some(fields[3]);
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let text = String::from_utf8_lossy(&bytes);
    match op {
        "import" => {
            let line =
                (fields[4] != "-").then(|| std::fs::read_to_string(fields[4]).unwrap_or_default());
            match keyfile::import(&text, passphrase, line.as_deref(), fields[5]) {
                Ok(key) => [
                    "ok".to_owned(),
                    key.record_algorithm().raw_value().to_owned(),
                    key.info.fingerprint.clone(),
                    key.info.public_key.clone(),
                    key.bits.map_or("-".to_owned(), |b| b.to_string()),
                    read_back(&key.openssh),
                ]
                .join("\t"),
                Err(e) => error(e),
            }
        }
        "isencrypted" => keyfile::is_encrypted(&text).to_string(),
        "assess" => match keyfile::assess(&text, passphrase) {
            Ok(Assessment::Usable) => "usable".to_owned(),
            Ok(Assessment::NeedsPassphrase) => "needsPassphrase".to_owned(),
            Ok(Assessment::Converted(converted)) => format!("converted {}", read_back(&converted)),
            Err(e) => error(e),
        },
        "describe" => match std::str::from_utf8(&bytes)
            .ok()
            .and_then(keyfile::describe_stored)
        {
            Some(key) => [
                key.record_algorithm().raw_value().to_owned(),
                key.info.fingerprint.clone(),
                key.info.public_key.clone(),
                key.bits.map_or("-".to_owned(), |b| b.to_string()),
            ]
            .join("\t"),
            None => "none".to_owned(),
        },
        other => panic!("no key op {other}"),
    }
}

/// One harness command, given its arguments (file arguments already read).
pub fn answer(command: &str, args: &[&str]) -> Option<String> {
    Some(match (command, args) {
        ("fingerprint", [line]) => fingerprint(line),
        ("keytype", [text]) => keytype(text),
        ("import", [text]) => import(text),
        ("upload", [home, filename]) => upload(home, filename),
        _ => return None,
    })
}
