//! W14b against real programs on this machine's loopback: what runs beside
//! a terminal — the app's remote preview and folder sheet, image uploads,
//! the path probe, host maintenance — on the session's own SSH, borrowed
//! (`Sessions::borrow_ssh`; the app reaches it through `crates/ffi`'s
//! `BorrowedSsh`, whose methods are the calls made here). Private every
//! one: an sshd of our own whose sessions live in a scratch home
//! (`SetEnv HOME`) and whose SFTP starts there (`Subsystem sftp
//! internal-sftp -d`), so nothing lands in the account's real home;
//! mosh-server through it; a tmux server on a private socket. Everything
//! started here is stopped when the test ends.
//!
//! - Sign-ins: a session's first borrows the moment it is live (the app's
//!   maintenance pass and upload sweep), then a preview, a stream, a folder
//!   listing, an upload and a probe: SSH, SSH + tmux and mosh + tmux sign in
//!   once; plain mosh twice (the side connection it keeps, its bootstrap
//!   being closed once mosh-server runs).
//! - Latency: the same calls on the borrowed connection against a
//!   connection dialled for each feature (the app before W14b, where each
//!   feature beside a Rust session signed in on its own).
//! - Failure: what is in flight when the session's connection drops fails
//!   at once; a borrow while it reconnects is refused at once; the next
//!   borrow gets the new connection.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_w14b -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moshpit_session::{ConnectionSpec, LinkFault, Multiplexer, Protocol, SessionState, Timing};
use moshpit_ssh::{Auth, DenyAll, SftpError, Ssh, TrustOnFirstUse};
use support::*;

macro_rules! need {
    ($($what:expr),*) => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
        $(if !$what {
            eprintln!("skipped: {}", stringify!($what));
            return;
        })*
    };
}

const CHART_BYTES: usize = 1 << 20;
const VIDEO_BYTES: usize = 4 << 20;
const UPLOAD_BYTES: usize = 256 << 10;

/// Bytes nobody compresses: xorshift from `seed`.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// The scratch home the sessions live in, with what an agent leaves
/// behind: a chart, a video, a folder of notes.
struct Home {
    dir: Scratch,
    chart: Vec<u8>,
}

impl Home {
    fn new() -> Home {
        let dir = Scratch::new("home");
        let chart = noise(CHART_BYTES, 7);
        std::fs::write(dir.join("chart.png"), &chart).unwrap();
        std::fs::write(dir.join("clip.mp4"), noise(VIDEO_BYTES, 11)).unwrap();
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::write(dir.join("notes/plan.md"), b"# plan\n").unwrap();
        Home { dir, chart }
    }

    fn path(&self) -> String {
        self.dir.0.display().to_string()
    }

    /// An sshd whose sessions and SFTP start in this home.
    async fn sshd(&self) -> Sshd {
        let home = self.path();
        Sshd::start_configured(
            &[("HOME", &home), ("ZDOTDIR", &home)],
            &[&format!("Subsystem sftp internal-sftp -d {home}")],
        )
        .await
    }
}

fn tmux_spec(port: u16, tmux: &Tmux, protocol: Protocol) -> ConnectionSpec {
    let mut spec = spec(port);
    spec.protocol = protocol;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    spec
}

fn mosh_spec(port: u16) -> ConnectionSpec {
    let mut spec = spec(port);
    spec.protocol = Protocol::Mosh;
    spec
}

/// The app's uploads sweep (`ActiveSession.uploadCleanupCommand(days: 7)`).
const SWEEP: &str = "find \"$HOME/.moshpit/uploads\" -type f -mtime +7 -delete 2>/dev/null || true";

/// A path probe's question, in the shape `PathProbe.command` asks it.
const PROBE: &str = "sh -c 'for p in \"$@\"; do if [ -e \"$HOME/$p\" ]; then printf \"%s\\n\" \"$p\"; fi; done' moshpit-probe chart.png notes missing.txt";

/// A command that must have succeeded (exit status 0, the channel closed
/// in order): what the app's `executeCommand` requires (B136).
async fn run_ok(ssh: &Ssh, command: &str) -> Vec<u8> {
    let output = ssh.run(command).await.expect("the command ran");
    assert!(output.success(), "{command}: {output:?}");
    output.stdout()
}

/// What a side feature does with its connection, timed per step: the calls
/// the app's `BorrowedHost` makes for a probe, a preview (stat, download),
/// a stream's first megabyte (open, sixteen reads in flight), a folder
/// listing and an image upload into the uploads directory.
async fn side_features(ssh: &Ssh, home: &Home, out: &Path) -> Vec<(&'static str, Duration)> {
    let mut steps = Vec::new();
    let started = Instant::now();
    let probed = run_ok(ssh, PROBE).await;
    assert_eq!(String::from_utf8_lossy(&probed), "chart.png\nnotes\n");
    steps.push(("probe", started.elapsed()));

    let started = Instant::now();
    let sftp = ssh.sftp().await.expect("SFTP");
    let chart = format!("{}/chart.png", home.path());
    let entry = sftp.stat(&chart).await.expect("stat");
    assert_eq!(entry.size, Some(CHART_BYTES as u64));
    let local = out.join("chart.png");
    let copied = sftp
        .download(&chart, &local, &mut |_| true)
        .await
        .expect("download");
    assert_eq!(copied, CHART_BYTES as u64);
    assert!(
        std::fs::read(&local).unwrap() == home.chart,
        "the copy is the file"
    );
    steps.push(("preview", started.elapsed()));

    let started = Instant::now();
    let file = Arc::new(
        sftp.open(&format!("{}/clip.mp4", home.path()))
            .await
            .expect("open"),
    );
    assert_eq!(file.size(), VIDEO_BYTES as u64);
    let mut reads = tokio::task::JoinSet::new();
    for chunk in 0..16u64 {
        let file = file.clone();
        reads.spawn(async move { file.read(chunk * 65536, 65536).await.map(|b| b.len()) });
    }
    let mut first_megabyte = 0;
    while let Some(read) = reads.join_next().await {
        first_megabyte += read.unwrap().expect("a read");
    }
    assert_eq!(first_megabyte, 1 << 20);
    if let Ok(file) = Arc::try_unwrap(file) {
        file.close().await;
    }
    steps.push(("stream 1 MiB", started.elapsed()));

    let started = Instant::now();
    let listed = sftp.list(&home.path()).await.expect("list");
    assert!(
        listed
            .iter()
            .any(|e| e.path.ends_with("/notes") && e.is_directory)
    );
    steps.push(("folder", started.elapsed()));

    let started = Instant::now();
    let name = format!("IMG-{}.jpg", unique("w14b"));
    let image = noise(UPLOAD_BYTES, 13);
    let path = ssh
        .upload_to_uploads_directory(&image, &name, &mut |_| true)
        .await
        .expect("upload");
    assert_eq!(path, format!("{}/.moshpit/uploads/{name}", home.path()));
    assert!(
        std::fs::read(&path).unwrap() == image,
        "the upload is the image"
    );
    steps.push(("upload 256 KiB", started.elapsed()));
    steps
}

/// The first borrows a session gets the moment it is live — the app's
/// maintenance pass and its upload sweep, both started on `Connected` —
/// then the side features on whatever it lends.
async fn borrow_through(bench: &mut Bench, id: u64, home: &Home, out: &Path) {
    let first = bench.sessions.borrow_ssh(id);
    let second = bench.sessions.borrow_ssh(id);
    let (first, second) = (
        first.await.unwrap().expect("a connection"),
        second.await.unwrap().expect("a connection"),
    );
    run_ok(&first, SWEEP).await;
    run_ok(&second, HostProbe::PREFLIGHT).await;
    let lent = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("a connection");
    side_features(&lent, home, out).await;
}

/// The maintenance pass's first question (`HostCommands.preflight`'s shape).
struct HostProbe;
impl HostProbe {
    const PREFLIGHT: &'static str = "printf 'home=%s\\n' \"$HOME\"; for t in tmux base64; do if command -v \"$t\" >/dev/null 2>&1; then printf '%s=yes\\n' \"$t\"; fi; done";
}

async fn connected_now(bench: &mut Bench, id: u64) {
    bench.connected(id).await;
}

/// Each kind of session signs in once for its terminal and everything
/// beside it — plain mosh once more, for the side connection it keeps.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn side_features_sign_in_once_per_session() {
    need!();
    let home = Home::new();
    let out = Scratch::new("out");
    let tmux = Tmux::new();
    tmux.new_session("w14b");
    for (label, mosh, multiplexer, expected) in [
        ("ssh", false, Multiplexer::None, 1),
        ("ssh + tmux", false, Multiplexer::Tmux, 1),
        ("mosh + tmux", true, Multiplexer::Tmux, 1),
        ("mosh", true, Multiplexer::None, 2),
    ] {
        let sshd = home.sshd().await;
        let mut bench = Bench::with(sshd.addr(), Timing::default());
        let spec = match (mosh, multiplexer) {
            (false, Multiplexer::None) => spec(sshd.port),
            (true, Multiplexer::None) => mosh_spec(sshd.port),
            (false, _) => tmux_spec(sshd.port, &tmux, Protocol::Ssh),
            (true, _) => tmux_spec(sshd.port, &tmux, Protocol::Mosh),
        };
        let id = bench.sessions.connect(spec);
        bench.sessions.set_visible(Some(id));
        connected_now(&mut bench, id).await;
        let _reaper = Reaper(bench.sessions.view(id).and_then(|v| v.mosh_server_pid));
        borrow_through(&mut bench, id, &home, &out.0).await;
        // A second round, as a second preview and upload would.
        let lent = bench
            .sessions
            .borrow_ssh(id)
            .await
            .unwrap()
            .expect("a connection");
        side_features(&lent, &home, &out.0).await;
        let signed_in = sshd.accepted();
        eprintln!("{label}: {signed_in} sign-in(s) for the terminal and every side feature");
        assert_eq!(signed_in, expected, "{label}: {}", sshd.log());
        let _ = bench.sessions.disconnect(id).await;
    }
}

/// The side features on the borrowed connection against a connection
/// dialled and signed in for each (what each feature beside a Rust
/// session did before W14b: `acquireFileTransferSSH`'s on-demand dial).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn borrowed_against_a_dial_per_feature() {
    need!();
    let home = Home::new();
    let out = Scratch::new("out");
    let sshd = home.sshd().await;
    let mut bench = Bench::with(sshd.addr(), Timing::default());
    let id = bench.sessions.connect(spec(sshd.port));
    connected_now(&mut bench, id).await;
    let target = moshpit_ssh::Target::new(whoami(), "127.0.0.1", sshd.port);
    let options = Timing::default().connect_options(None);
    let rounds = 12;
    let mut borrowed: Vec<Vec<(&'static str, Duration)>> = Vec::new();
    let mut dialled: Vec<Vec<(&'static str, Duration)>> = Vec::new();
    let mut sign_ins = Vec::new();
    for _ in 0..rounds {
        let started = Instant::now();
        let lent = bench
            .sessions
            .borrow_ssh(id)
            .await
            .unwrap()
            .expect("a connection");
        let mut steps = vec![("borrow", started.elapsed())];
        steps.extend(side_features(&lent, &home, &out.0).await);
        borrowed.push(steps);

        let started = Instant::now();
        let policy = Arc::new(TrustOnFirstUse::new(
            trusting(sshd.addr()),
            Arc::new(DenyAll),
        ));
        let fresh = Ssh::connect(&target, Auth::key(fixture("ed25519")), policy, &options)
            .await
            .expect("a dial");
        sign_ins.push(started.elapsed());
        let mut steps = vec![("sign in", started.elapsed())];
        steps.extend(side_features(&fresh, &home, &out.0).await);
        fresh.close().await;
        dialled.push(steps);
    }
    let report = |label: &str, runs: &[Vec<(&'static str, Duration)>]| {
        for step in 0..runs[0].len() {
            let mut samples: Vec<u128> = runs.iter().map(|r| r[step].1.as_micros()).collect();
            let (p50, p95, _, max) = percentiles(&mut samples);
            eprintln!(
                "{label:>9} {:>15}: p50 {:>6.2} ms  p95 {:>6.2} ms  max {:>6.2} ms",
                runs[0][step].0,
                p50 as f64 / 1000.0,
                p95 as f64 / 1000.0,
                max as f64 / 1000.0
            );
        }
        let mut totals: Vec<u128> = runs
            .iter()
            .map(|r| r.iter().map(|s| s.1.as_micros()).sum())
            .collect();
        let (p50, p95, _, _) = percentiles(&mut totals);
        eprintln!(
            "{label:>9} {:>15}: p50 {:>6.2} ms  p95 {:>6.2} ms",
            "all of it",
            p50 as f64 / 1000.0,
            p95 as f64 / 1000.0
        );
    };
    report("borrowed", &borrowed);
    report("dialled", &dialled);
    assert_eq!(
        sshd.accepted(),
        1 + rounds,
        "the borrowed rounds signed in no more: {}",
        sshd.log()
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// A download on the borrowed connection when the link under the session
/// is cut fails at once (the session closes the connection it lent as it
/// reconnects); borrowing while it reconnects is refused at once; once it
/// is back the next borrow gets the new connection, and nothing else
/// signed in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn a_borrow_follows_the_session_through_a_reconnect() {
    need!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let home = Home::new();
    let out = Scratch::new("out");
    // A file big enough to be mid-way when the cut lands.
    std::fs::write(home.dir.join("big.bin"), noise(64 << 20, 17)).unwrap();
    let sshd = home.sshd().await;
    let relay = Relay::start(sshd.addr()).await;
    let relay_addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
    // A borrow refused "at once" while the session reconnects is told from
    // one that sat out a dial by dials far longer than load can stretch the
    // refusal to (T9).
    let dial = Duration::from_secs(30);
    let mut bench = Bench::with(
        relay_addr,
        Timing {
            dial_timeout: dial,
            handshake_timeout: dial,
            channel_timeout: dial,
            ..quick()
        },
    );
    let id = bench.sessions.connect(spec(relay.port));
    connected_now(&mut bench, id).await;
    let lent = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("a connection");
    let big = format!("{}/big.bin", home.path());
    let local = out.join("big.bin");
    let download = tokio::spawn(async move {
        let mut moved = 0.0;
        // A cut before SFTP started fails the download all the same.
        let result = match lent.sftp().await {
            Ok(sftp) => {
                sftp.download(&big, &local, &mut |fraction| {
                    moved = fraction;
                    true
                })
                .await
            }
            Err(e) => Err::<u64, SftpError>(e),
        };
        (result, moved)
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let cut = Instant::now();
    relay.cut();
    // B277: a download cut early sometimes hung here for a minute (the
    // SFTP request timeout, `REQUEST_TIMEOUT_SECS`), on the close it sends
    // its handle after the failed read.
    let request_timeout = Duration::from_secs(60);
    let (result, moved) = tokio::time::timeout(request_timeout * 3 / 4, download)
        .await
        .expect("the download ends")
        .unwrap();
    let failed_after = cut.elapsed();
    assert!(result.is_err(), "a cut download fails: {result:?}");
    eprintln!(
        "cut at {:.0}% → the download failed after {failed_after:?}: {:?}",
        moved * 100.0,
        result.err()
    );
    assert!(
        failed_after < request_timeout / 3,
        "failed after {failed_after:?}: a request waited out its timeout"
    );

    // While it reconnects: refused at once.
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    let asked = Instant::now();
    let refused = bench.sessions.borrow_ssh(id).await.unwrap();
    if refused.is_err() {
        eprintln!(
            "borrowed while reconnecting: refused after {:?}",
            asked.elapsed()
        );
        assert!(
            asked.elapsed() < dial / 3,
            "refused after {:?}: the borrow sat out a dial",
            asked.elapsed()
        );
    }
    bench.state(id, 20, |s| *s == SessionState::Connected).await;
    let again = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("the new connection");
    side_features(&again, &home, &out.0).await;
    assert_eq!(
        sshd.accepted(),
        2,
        "the session's dial and its redial, nothing beside them: {}",
        sshd.log()
    );

    // A stall: the session finds the silence and redials; the lent
    // connection is closed with the old one.
    let lent = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("a connection");
    bench.sessions.inject_fault(id, LinkFault::Stall);
    bench
        .state(id, 20, |s| *s == SessionState::Reconnecting)
        .await;
    let asked = Instant::now();
    let output = lent.run("true").await;
    eprintln!(
        "a command on the stalled connection, after the session redialled: {:?} after {:?}",
        output.as_ref().map(|o| o.success()),
        asked.elapsed()
    );
    assert!(!matches!(output, Ok(ref o) if o.success()));
    let _ = bench.sessions.disconnect(id).await;
}
