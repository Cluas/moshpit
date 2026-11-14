//! The Rust side of the ssh cross-check (harness/equivalence.py): the same
//! commands as harness/swift/main.swift, answered by this crate.
//!
//!   ssh_harness fingerprint <openssh public key line>
//!   ssh_harness live <host> <port>        the key a host shows this client
//!   ssh_harness keytype <private key file>
//!   ssh_harness import <private key file> [<public key file>]
//!   ssh_harness upload <home> <filename>
//!   ssh_harness socks <proxy port> <target host> <target port>
//!   ssh_harness keys <cases file>        the key reader, one case per line

use std::sync::{Arc, Mutex};
use std::time::Duration;

use moshpit_ssh::{
    Auth, BoxFuture, ConnectOptions, HostKey, HostKeyPolicy, HostKeyRefusal, Ssh, Target, Waiting,
    harness,
};

/// Notes the host key and refuses it: the handshake goes no further.
struct NoteHostKey(Mutex<Option<HostKey>>);

impl HostKeyPolicy for NoteHostKey {
    fn check<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
        key: &'a HostKey,
        _waiting: &'a Waiting,
    ) -> BoxFuture<'a, Result<(), HostKeyRefusal>> {
        *self.0.lock().unwrap() = Some(key.clone());
        Box::pin(async { Err(HostKeyRefusal::NotTrusted) })
    }
}

async fn live(host: &str, port: u16) -> String {
    let seen = Arc::new(NoteHostKey(Mutex::new(None)));
    let options = ConnectOptions {
        connect_timeout: Duration::from_secs(10),
        handshake_timeout: Duration::from_secs(10),
        ..ConnectOptions::default()
    };
    let _ = Ssh::connect(
        &Target::new("nobody", host, port),
        Auth::password("-"),
        seen.clone(),
        &options,
    )
    .await;
    let key = seen.0.lock().unwrap().clone();
    match key {
        Some(key) => harness::describe(&key),
        None => "kex-failed".to_owned(),
    }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let answer = match args.as_slice() {
        ["live", host, port] => Some(live(host, port.parse().unwrap()).await),
        ["socks", proxy, host, port] => Some(
            harness::socks(
                proxy.parse().unwrap(),
                host,
                port.parse().unwrap(),
                // As the Swift side: past the app's 15 s proxy budget.
                Duration::from_secs(20),
            )
            .await,
        ),
        ["keys", path] => Some(
            read(path)
                .lines()
                .map(|line| {
                    let fields: Vec<&str> = line.split('\t').collect();
                    format!("{}\t{}", fields[0], harness::key_case(&fields))
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        ["keytype", path] => harness::answer("keytype", &[&read(path)]),
        ["import", path] | ["import", path, _] => harness::answer("import", &[&read(path)]),
        [command, rest @ ..] => harness::answer(command, rest),
        [] => None,
    };
    match answer {
        Some(answer) => println!("{answer}"),
        None => {
            eprintln!("usage: ssh_harness fingerprint|live|keytype|import|upload|socks …");
            std::process::exit(2);
        }
    }
}
