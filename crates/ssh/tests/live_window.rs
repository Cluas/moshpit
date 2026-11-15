//! B790 against this Mac's own sshd (Remote Login on, `~/.ssh/id_ed25519`
//! authorized), through a relay that holds the link to a rate and a delay:
//! what the receive window costs and buys. A flood (`yes` on a PTY) with a
//! ^C into it: how long the bytes already on their way take to arrive (the
//! screen settles), at 0.5 and 2 MB/s. And an SFTP download of 64 MB at 20
//! and 100 ms round trips. Each at 2 MB (russh's, and ours), 1.5 MB and
//! 1 MB (B790: measured, kept at 2 MB; for when russh can set a window per
//! channel).
//!
//! ```sh
//! cargo test -p moshpit-ssh --release --test live_window -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use moshpit_ssh::{ConnectOptions, Event, PtyRequest, Target};
use support::AcceptAny;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// A link: what one direction carries at most (bytes a second; None: as
/// fast as it goes) and how long a byte takes one way.
#[derive(Clone, Copy)]
struct Link {
    down: Option<u64>,
    one_way: Duration,
}

/// A relay to 127.0.0.1:22 over `link`; its port.
async fn relay(link: Link) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let server = TcpStream::connect("127.0.0.1:22").await.unwrap();
            let _ = client.set_nodelay(true);
            let _ = server.set_nodelay(true);
            let (client_read, client_write) = client.into_split();
            let (server_read, server_write) = server.into_split();
            tokio::spawn(pump(client_read, server_write, None, link.one_way));
            tokio::spawn(pump(server_read, client_write, link.down, link.one_way));
        }
    });
    port
}

/// Carries one direction: each read is delivered `delay` later, no faster
/// than `rate` (a queue with room for anything: the SSH window is what
/// bounds what is on its way).
async fn pump(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    rate: Option<u64>,
    delay: Duration,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match from.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send((Instant::now() + delay, buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut free_at = Instant::now();
    while let Some((due, bytes)) = rx.recv().await {
        let start = due.max(free_at);
        tokio::time::sleep_until(start.into()).await;
        if let Some(rate) = rate {
            free_at = start + Duration::from_secs_f64(bytes.len() as f64 / rate as f64);
            tokio::time::sleep_until(free_at.into()).await;
        }
        if to.write_all(&bytes).await.is_err() {
            break;
        }
    }
}

fn user() -> String {
    std::env::var("USER").unwrap()
}

fn key() -> String {
    let home = std::env::var("HOME").unwrap();
    std::fs::read_to_string(format!("{home}/.ssh/id_ed25519")).expect("~/.ssh/id_ed25519")
}

async fn connect(port: u16, window: u32) -> moshpit_ssh::Ssh {
    let options = ConnectOptions {
        receive_window: window,
        ..ConnectOptions::default()
    };
    moshpit_ssh::Ssh::connect(
        &Target::new(user(), "127.0.0.1", port),
        moshpit_ssh::Auth::key(key()),
        Arc::new(AcceptAny),
        &options,
    )
    .await
    .expect("signs in to this Mac's sshd")
}

/// A flood, then a ^C: how long until its last byte, and how many came.
async fn settle(rate: u64, window: u32) -> (Duration, usize) {
    let port = relay(Link {
        down: Some(rate),
        one_way: Duration::from_millis(25),
    })
    .await;
    let ssh = connect(port, window).await;
    let mut request = PtyRequest::new(120, 40);
    request.command = Some("yes".to_owned());
    let process = ssh.terminal(&request).await.unwrap();
    let (mut reader, writer) = process.split();
    // Flooding long enough for the window to be full on the way.
    let flooding = Instant::now();
    while flooding.elapsed() < Duration::from_secs(4) {
        if reader.next().await.is_none() {
            break;
        }
    }
    let pressed = Instant::now();
    writer.write(&[0x03]).await.unwrap();
    let mut after = 0;
    let mut last = pressed;
    loop {
        match tokio::time::timeout(Duration::from_secs(30), reader.next()).await {
            Ok(Some(Event::Stdout(bytes))) | Ok(Some(Event::Stderr(bytes))) => {
                after += bytes.len();
                last = Instant::now();
            }
            Ok(Some(Event::Closed)) | Ok(Some(Event::Lost)) | Ok(None) | Err(_) => break,
            Ok(Some(_)) => {}
        }
    }
    (last - pressed, after)
}

/// A 64 MB download over a link of `rtt`: MB/s.
async fn sftp_rate(rtt: Duration, window: u32, file: &str) -> f64 {
    let port = relay(Link {
        down: None,
        one_way: rtt / 2,
    })
    .await;
    let ssh = connect(port, window).await;
    let sftp = ssh.sftp().await.unwrap();
    let out = std::env::temp_dir().join(format!("moshpit-b790-{}.out", std::process::id()));
    let started = Instant::now();
    let bytes = sftp.download(file, &out, &mut |_| true).await.unwrap();
    let took = started.elapsed();
    let _ = std::fs::remove_file(&out);
    bytes as f64 / 1e6 / took.as_secs_f64()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs this Mac's sshd and ~/.ssh/id_ed25519; see the module docs"]
async fn the_window_against_a_flood_and_a_download() {
    for rate in [512 * 1024, 2 * 1024 * 1024] {
        for window in [2 << 20, 1536 << 10, 1 << 20] {
            let (took, bytes) = settle(rate, window).await;
            eprintln!(
                "flood at {:.1} MB/s, window {} KB: the last byte {:?} after the ^C, {} KB after it",
                rate as f64 / 1048576.0,
                window / 1024,
                took,
                bytes / 1024
            );
        }
    }
    let file = std::env::temp_dir().join(format!("moshpit-b790-{}.bin", std::process::id()));
    std::fs::write(&file, vec![0x5au8; 64 << 20]).unwrap();
    let path = file.to_string_lossy().into_owned();
    for rtt in [Duration::from_millis(20), Duration::from_millis(100)] {
        for window in [2 << 20, 1536 << 10, 1 << 20] {
            let rate = sftp_rate(rtt, window, &path).await;
            eprintln!("sftp at {rtt:?} round trip, window {} KB: {rate:.1} MB/s", window / 1024);
        }
    }
    let _ = std::fs::remove_file(&file);
}
