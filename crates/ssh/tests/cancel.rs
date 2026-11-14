//! Giving up on an operation in flight (B243): a host can't cancel the
//! future it awaits, so it trips a token the operation watches. On a link
//! that went silent — where nothing answers and a progress report never
//! comes — the operation answers `Cancelled` at once, and what it holds on
//! the host (a handle, half an upload) is let go once the link moves again.
//! Without the token a stat there waits out the request timeout (60 s) and
//! a transfer its next progress report.

mod support;

use std::sync::Arc;
use std::time::Duration;

use moshpit_ssh::{CancellationToken, SftpError, Target};
use support::*;

fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x9E37_79B9u32;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

async fn through(relay: &Relay) -> moshpit_ssh::Ssh {
    moshpit_ssh::Ssh::connect(
        &Target::new(USER, "127.0.0.1", relay.port),
        moshpit_ssh::Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap()
}

/// Polls `check` every 50 ms for up to `seconds`.
async fn eventually(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    check()
}

/// A stat, a listing and the home on a silent link: given up, each answers
/// at once. The same stat with no token is still waiting a second later.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_on_a_silent_link_answers_cancelled_at_once() {
    let host = Host::start(Bench::default()).await;
    std::fs::create_dir(host.home.join("shots")).unwrap();
    std::fs::write(host.home.join("shots/a.png"), b"png").unwrap();
    let relay = Relay::start(host.addr).await;
    let ssh = through(&relay).await;
    let sftp = ssh.sftp().await.unwrap();
    relay.stall();

    let waiting = tokio::time::timeout(Duration::from_secs(1), sftp.stat("shots")).await;
    assert!(waiting.is_err(), "the stat answered on a silent link");

    let cancel = CancellationToken::new();
    let given_up = sftp.cancelled_by(cancel.clone());
    let calls = (given_up.clone(), given_up.clone(), given_up);
    let stat = tokio::spawn(async move { calls.0.stat("shots").await.map(|_| ()) });
    let list = tokio::spawn(async move { calls.1.list("shots").await.map(|_| ()) });
    let home = tokio::spawn(async move { calls.2.home().await.map(|_| ()) });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!stat.is_finished() && !list.is_finished() && !home.is_finished());

    cancel.cancel();
    for (what, task) in [("stat", stat), ("list", list), ("home", home)] {
        let answer = within(1, task).await.unwrap();
        assert!(
            matches!(answer, Err(SftpError::Cancelled)),
            "{what}: {answer:?}"
        );
    }

    // The listing's folder, opened by the host once its request got there,
    // is closed again.
    relay.resume();
    assert!(
        eventually(5, || host.seen().open_handles == 0).await,
        "{} handles left open",
        host.seen().open_handles
    );
    assert!(sftp.stat("shots").await.unwrap().is_directory);
}

/// A download given up on a silent link stops where it is — no progress
/// report comes to stop it — and its handle on the host is closed once the
/// link moves again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_given_up_on_a_silent_link_stops_and_lets_its_handle_go() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("big.bin"), noise(8 * 1024 * 1024)).unwrap();
    let relay = Arc::new(Relay::start(host.addr).await);
    let ssh = through(&relay).await;
    let cancel = CancellationToken::new();
    let sftp = ssh.sftp().await.unwrap().cancelled_by(cancel.clone());
    let local = Scratch::new("cancelled-download");
    let destination = local.join("big.bin");
    let (stalled, on_stall) = tokio::sync::oneshot::channel::<()>();

    let staller = relay.clone();
    let download = tokio::spawn(async move {
        let mut stalled = Some(stalled);
        sftp.download("big.bin", &destination, &mut |_| {
            // The first window landed: the link goes silent.
            if let Some(stalled) = stalled.take() {
                staller.stall();
                let _ = stalled.send(());
            }
            true
        })
        .await
    });
    within(10, on_stall).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !download.is_finished(),
        "the download ended on a silent link"
    );
    assert_eq!(host.seen().open_handles, 1);

    cancel.cancel();
    let answer = within(1, download).await.unwrap();
    assert!(matches!(answer, Err(SftpError::Cancelled)), "{answer:?}");

    relay.resume();
    assert!(
        eventually(5, || host.seen().open_handles == 0).await,
        "the file's handle is still open on the host"
    );
}

/// An upload given up on a silent link answers at once; once the link
/// moves again, what it wrote is removed and its handle closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_given_up_on_a_silent_link_leaves_nothing_behind() {
    let host = Host::start(Bench::default()).await;
    let local = Scratch::new("cancelled-upload");
    std::fs::write(local.join("big"), noise(8 * 1024 * 1024)).unwrap();
    let relay = Arc::new(Relay::start(host.addr).await);
    let ssh = through(&relay).await;
    let cancel = CancellationToken::new();
    let sftp = ssh.sftp().await.unwrap().cancelled_by(cancel.clone());
    let (stalled, on_stall) = tokio::sync::oneshot::channel::<()>();

    let staller = relay.clone();
    let source = local.join("big");
    let upload = tokio::spawn(async move {
        let mut stalled = Some(stalled);
        sftp.upload_new(&source, "big", &mut |_| {
            if let Some(stalled) = stalled.take() {
                staller.stall();
                let _ = stalled.send(());
            }
            true
        })
        .await
    });
    within(10, on_stall).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!upload.is_finished(), "the upload ended on a silent link");

    cancel.cancel();
    let answer = within(1, upload).await.unwrap();
    assert!(matches!(answer, Err(SftpError::Cancelled)), "{answer:?}");

    relay.resume();
    assert!(
        eventually(5, || !host.home.join("big").exists()
            && host.seen().open_handles == 0)
        .await,
        "half an upload is left on the host (exists: {}, handles: {})",
        host.home.join("big").exists(),
        host.seen().open_handles
    );
}

/// A command given up on a silent link answers at once (`None`); one that
/// was given up before it began is never sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_given_up_answers_at_once() {
    let host = Host::start(Bench::default()).await;
    let relay = Relay::start(host.addr).await;
    let ssh = Arc::new(through(&relay).await);
    let cancel = CancellationToken::new();
    relay.stall();

    let runner = ssh.clone();
    let token = cancel.clone();
    let run = tokio::spawn(async move { runner.run_until("echo hi", &token).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!run.is_finished());
    cancel.cancel();
    let answer = within(1, run).await.unwrap();
    assert!(matches!(answer, Ok(None)), "{answer:?}");

    let sent = host.seen().requests.len();
    assert!(matches!(
        ssh.run_until("echo again", &cancel).await,
        Ok(None)
    ));
    assert_eq!(
        host.seen().requests.len(),
        sent,
        "a cancelled command was sent"
    );
}

/// The uploads directory over SFTP, given up on a silent link: `Cancelled`
/// at once, not the exec fallback.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_to_the_uploads_directory_given_up_answers_at_once() {
    let host = Host::start(Bench::default()).await;
    let relay = Arc::new(Relay::start(host.addr).await);
    let ssh = Arc::new(through(&relay).await);
    // The SFTP session is open before the link goes silent.
    ssh.sftp().await.unwrap();
    let cancel = CancellationToken::new();
    let (stalled, on_stall) = tokio::sync::oneshot::channel::<()>();

    let (uploader, staller, token) = (ssh.clone(), relay.clone(), cancel.clone());
    let upload = tokio::spawn(async move {
        let mut stalled = Some(stalled);
        let image = noise(4 * 1024 * 1024);
        uploader
            .upload_to_uploads_directory_until(
                &image,
                "IMG-1.png",
                &mut |_| {
                    if let Some(stalled) = stalled.take() {
                        staller.stall();
                        let _ = stalled.send(());
                    }
                    true
                },
                &token,
            )
            .await
    });
    within(10, on_stall).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!upload.is_finished());
    cancel.cancel();
    let answer = within(1, upload).await.unwrap();
    assert!(matches!(answer, Err(SftpError::Cancelled)), "{answer:?}");
}
