//! Host care against real programs on this machine's loopback: host care, the
//! Host Setup screen and push pairing (`moshpit-hostcare`) driven over a
//! Rust session's own SSH connection (`Sessions::borrow_ssh`, as the app's
//! platform reaches it), against the in-tree relay.
//!
//! Private every one: an sshd of our own whose sessions live in a scratch
//! home (`SetEnv HOME`, so the scripts, the manifest and the agent configs
//! land there) with tmux's default socket in it too (`SetEnv TMUX_TMPDIR`,
//! so the hook test's `tmux display-message` reaches our server and never
//! the account's own); a tmux server on that socket; `push-relay` built
//! from this tree and run in dry-run mode on a loopback port (it logs the
//! APNs payload instead of sending it). The self-test push is opened from
//! that log with the phone's secret, exactly as the notification service
//! opens one, and its nonce handed to the machine. Nothing touches the
//! real relay, APNs, `~/.ssh`, the account's home or its tmux.
//!
//! The cycle: care on a fresh host asks for consent and writes nothing; a
//! yes installs Claude Code's hooks and the scripts and pairs (minted at the
//! relay, the conf on the host the phone's byte for byte); the hook test
//! fires the stamp into the private tmux; the push test goes host → relay
//! and back; care again changes nothing; the push conf and the sender
//! deleted by hand are put back (B315); a stale script is put right; hooks
//! taken out of the agent's config by hand are reported, not put back; the
//! screen removes the hooks and unpairs, and the host is left clean.
//!
//! ```sh
//! CARGO_BUILD_JOBS=4 cargo test -p moshpit-session --test live_hostcare_borrowed -- --ignored --nocapture
//! ```

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use moshpit_hostcare::driver::{Driver, Platform};
use moshpit_hostcare::{
    Change, Clock, Config, Conn, HostCare, HttpReply, PairingStore, Proof, PushSupport, Settings,
};
use moshpit_install::{InstallComponent, InstallManifest};
use moshpit_seal::foundation::json::{self, Dialect};
use moshpit_seal::{Date, Language, PushFinishesFile, PushPairing, PushStandingFile, Uuid};
use moshpit_session::{Sessions, Timing};
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEVICE_TOKEN: &str = "abababababababababababababababababababababababababababababababab";

/// `push-relay`, built from this tree, in dry-run mode on a loopback port.
struct GoRelay {
    child: Child,
    port: u16,
    log: PathBuf,
}

impl Drop for GoRelay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl GoRelay {
    fn available() -> bool {
        Command::new("go").arg("version").output().is_ok()
    }

    async fn start(dir: &Scratch) -> GoRelay {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let binary = dir.join("push-relay");
        let built = Command::new("go")
            .args(["build", "-o"])
            .arg(&binary)
            .arg(".")
            .current_dir(repo.join("push-relay"))
            .status()
            .expect("go build");
        assert!(built.success(), "push-relay didn't build");
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        let log = dir.join("relay.log");
        let mut child = Command::new(&binary)
            .env("MOSHPIT_RELAY_DRY_RUN", "1")
            .env("MOSHPIT_RELAY_ADDR", format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return GoRelay { child, port, log };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Not left running behind the failure.
        let _ = child.kill();
        let _ = child.wait();
        panic!("the relay didn't start");
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The payloads it would have sent to APNs.
    fn payloads(&self) -> Vec<String> {
        self.log()
            .lines()
            .filter_map(|line| line.split_once(" payload=").map(|(_, p)| p.to_owned()))
            .collect()
    }
}

/// One HTTP/1.1 POST to a loopback relay (the app's `URLSession`).
async fn post(url: &str, body: &str) -> Result<HttpReply, String> {
    let rest = url.strip_prefix("http://").ok_or("not http")?;
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_owned()), |(a, p)| (a, format!("/{p}")));
    let mut stream = tokio::net::TcpStream::connect(authority)
        .await
        .map_err(|e| e.to_string())?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut reply = Vec::new();
    stream
        .read_to_end(&mut reply)
        .await
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&reply);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no body")?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("no status")?;
    Ok(HttpReply {
        status,
        body: body.as_bytes().to_vec(),
    })
}

/// The platform: commands on the session's own SSH connection, borrowed
/// for each (as the app's `HostCare` reaches `BorrowedSsh`); the relay over
/// loopback HTTP; what changed, counted.
struct Phone {
    sessions: Sessions,
    session: u64,
    connection: Uuid,
    commands: Mutex<Vec<String>>,
    /// The slowest command the host answered, and how long it took.
    slowest: Mutex<(Duration, String)>,
    changes: Mutex<Vec<Change>>,
}

#[async_trait]
impl Platform for Phone {
    async fn exec(&self, connection: Uuid, command: String) -> Result<Vec<u8>, String> {
        assert_eq!(connection, self.connection);
        self.commands.lock().unwrap().push(command.clone());
        let started = std::time::Instant::now();
        let ssh = self
            .sessions
            .borrow_ssh(self.session)
            .await
            .map_err(|_| "no session".to_owned())?
            .map_err(|f| f.log)?;
        let output = ssh.run(&command).await.map_err(|e| e.to_string())?;
        let took = started.elapsed();
        {
            let mut slowest = self.slowest.lock().unwrap();
            if took > slowest.0 {
                *slowest = (took, command.chars().take(60).collect());
            }
        }
        match (output.closed, output.status()) {
            (true, Some(0)) => Ok(output.stdout()),
            (_, status) => Err(format!(
                "exit {status:?}: {}",
                String::from_utf8_lossy(&output.stderr())
            )),
        }
    }

    async fn post(&self, url: String, body: String, _timeout: u32) -> Result<HttpReply, String> {
        post(&url, &body).await
    }

    fn register_for_push(&self) {}
    fn remember_declined(&self, _connection: Uuid) {}

    fn remember_unpaired(&self, _connection: Uuid, _unpaired: bool) {}

    fn changed(&self, change: Change) {
        self.changes.lock().unwrap().push(change);
    }
}

/// The phone's pairings, where the test can read them.
#[derive(Clone, Default)]
struct Pairings(Arc<Mutex<Vec<PushPairing>>>);

impl PairingStore for Pairings {
    fn read(&self) -> Vec<PushPairing> {
        self.0.lock().unwrap().clone()
    }
    fn upsert(&mut self, pairing: &PushPairing) -> bool {
        let mut all = self.0.lock().unwrap();
        all.retain(|p| p.connection_id != pairing.connection_id);
        all.push(pairing.clone());
        true
    }
    fn remove(&mut self, connection: Uuid) -> bool {
        self.0
            .lock()
            .unwrap()
            .retain(|p| p.connection_id != connection);
        true
    }
}

async fn until(what: &str, seconds: u64, mut done: impl FnMut() -> bool) {
    for _ in 0..seconds * 20 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never: {what}");
}

fn read(home: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(home.join(path.trim_start_matches("$HOME/"))).ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, tmux and go; see the module docs"]
async fn install_care_and_uninstall_against_a_private_host_and_relay() {
    if !Sshd::available() || !GoRelay::available() {
        eprintln!("skipped: needs {SSHD} and go");
        return;
    }
    let home = Scratch::new("borrow-home");
    let home_path = home.0.clone();
    // Short: a socket's path has to fit in 104 bytes.
    let tmux_dir = PathBuf::from(format!("/tmp/borrow-{}", moshpit_seal::random::hex(4)));
    std::fs::create_dir_all(&tmux_dir).unwrap();
    // Our tmux server and its socket go when the test ends, passed or not.
    struct Gone(PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .env("TMUX_TMPDIR", &self.0)
                .env_remove("TMUX")
                .arg("kill-server")
                .output();
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _gone = Gone(tmux_dir.clone());
    let tmux_dir_text = tmux_dir.display().to_string();
    let home_text = home_path.display().to_string();
    // tmux's default socket, in the scratch home: the hook test's probe
    // reaches this server and no other.
    let tmux = |args: &[&str]| {
        Command::new("tmux")
            .env("TMUX_TMPDIR", &tmux_dir)
            .env_remove("TMUX")
            .args(["-f", "/dev/null"])
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        tmux(&["new-session", "-d", "-s", "borrow", "-x", "80", "-y", "24"])
            .status
            .success()
    );
    let pane = String::from_utf8(tmux(&["list-panes", "-F", "#{pane_id}"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    // tmux where the sessions look for it: a non-interactive shell's PATH
    // has no Homebrew (a host like that reports the hook test unavailable,
    // the app's Swift did the same).
    let tmux_bin = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v tmux"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let path = format!(
        "{}:/usr/bin:/bin:/usr/sbin:/sbin",
        Path::new(tmux_bin.trim()).parent().unwrap().display()
    );
    let sshd = Sshd::start_configured(
        &[
            ("HOME", &home_text),
            ("ZDOTDIR", &home_text),
            ("TMUX_TMPDIR", &tmux_dir_text),
            ("PATH", &path),
        ],
        &[],
    )
    .await;
    let relay_dir = Scratch::new("borrow-relay");
    let relay = GoRelay::start(&relay_dir).await;

    let mut bench = Bench::with(sshd.addr(), Timing::default());
    let session = bench.sessions.connect(spec(sshd.port));
    bench.connected(session).await;
    let connection = Uuid::new_v4();
    let phone = Arc::new(Phone {
        sessions: bench.sessions.clone(),
        session,
        connection,
        commands: Mutex::new(Vec::new()),
        slowest: Mutex::new((Duration::ZERO, String::new())),
        changes: Mutex::new(Vec::new()),
    });
    let pairings = Pairings::default();
    let machine = HostCare::new(
        Config {
            push: PushSupport::Apns {
                environment: "sandbox".into(),
            },
            autocare_off: false,
            budgets: Default::default(),
        },
        Box::new(pairings.clone()),
        Clock {
            ms: 0,
            date: Date::now(),
        },
    );
    let care = Driver::new(machine, phone.clone(), tokio::runtime::Handle::current());
    let conn = Conn {
        id: connection,
        label: "borrow-host".into(),
    };
    care.with(|m, k| {
        m.set_settings(
            Settings {
                relay_url: relay.url(),
                ..Settings::default()
            },
            k,
        );
        m.set_saved(&[connection], Some(&[connection]), k);
        m.restore_device_token(Some(DEVICE_TOKEN.into()), k);
    });

    // 1. Care on a fresh host asks, and writes nothing.
    care.with(|m, k| m.tend(conn.clone(), k));
    until("care asks for consent", 20, || {
        care.read(|m| m.consent().is_some())
    })
    .await;
    until("the pass ends", 20, || {
        !care.read(|m| m.is_tending(connection))
    })
    .await;
    assert!(
        !home_path.join(".moshpit").exists(),
        "care wrote before a yes"
    );

    // 2. A yes: hooks, scripts, a pairing minted at the relay.
    care.with(|m, k| m.grant_consent(conn.clone(), k));
    until("the consented pass ends", 30, || {
        !care.read(|m| m.is_tending(connection)) && !pairings.read().is_empty()
    })
    .await;
    until("the pairing lands on the host", 30, || {
        read(&home_path, &format!(".moshpit/push.d/{connection}.conf")).is_some()
    })
    .await;
    let phone_pairing = pairings.read().pop().unwrap();
    assert!(!phone_pairing.send_token.is_empty(), "minted at the relay");
    assert!(relay.log().contains("mint"), "{}", relay.log());
    let conf = read(&home_path, &format!(".moshpit/push.d/{connection}.conf")).unwrap();
    assert_eq!(
        conf,
        moshpit_install::push_conf(&phone_pairing),
        "the host holds another pairing"
    );
    let settings = read(&home_path, ".claude/settings.json").expect("Claude Code's config");
    assert!(settings.contains("moshpit-stamp.sh"));
    let manifest = InstallManifest::from_text(&read(&home_path, ".moshpit/manifest.json").unwrap());
    for component in [
        InstallComponent::Stamp,
        InstallComponent::Sender,
        InstallComponent::Hooks {
            agent: "claude".into(),
        },
    ] {
        assert!(
            manifest.get(&component).is_some(),
            "{component:?} not recorded"
        );
    }
    assert!(manifest.pairing_entry(&connection.to_string()).is_some());
    let mode = std::fs::metadata(home_path.join(format!(".moshpit/push.d/{connection}.conf")))
        .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777)
        .unwrap();
    assert_eq!(mode, 0o600, "the pairing's secret is readable by others");

    // 3. The Host Setup screen proves the hooks: the stamp fired into our
    // tmux pane and read back.
    let page = care.with(|m, k| m.open_setup(conn.clone(), Some(pane.clone()), k));
    until("the screen reads the host", 20, || {
        care.read(|m| m.setup_view(page).unwrap().phase == moshpit_hostcare::Phase::Idle)
    })
    .await;
    let view = care.read(|m| m.setup_view(page).unwrap());
    assert_eq!(
        view.hooks,
        moshpit_install::installer::ComponentStatus::Current
    );
    assert_eq!(
        view.pairing,
        moshpit_install::installer::ComponentStatus::Current
    );
    care.with(|m, k| m.setup_prove_hooks(page, k));
    until("the hook test ends", 20, || {
        care.read(|m| !matches!(m.setup_view(page).unwrap().hooks_proof, Proof::Proving))
    })
    .await;
    let proof = care.read(|m| m.setup_view(page).unwrap().hooks_proof);
    assert!(matches!(proof, Proof::Proven(_)), "{proof:?}");

    // 4. The push test: host → relay; the payload opened with the phone's
    // secret, as the notification service does, and its nonce handed back.
    care.with(|m, k| m.setup_prove_push(page, k));
    until("the relay is asked to push", 20, || {
        !relay.payloads().is_empty()
    })
    .await;
    let payload = relay.payloads().pop().unwrap();
    let value = json::parse(payload.as_bytes(), Dialect::Decoder).unwrap();
    let standing = PushStandingFile::new(relay_dir.join("standing.json"));
    let finishes = PushFinishesFile::new(relay_dir.join("rung-finishes.json"));
    let received = moshpit_seal::receive(
        &value,
        &pairings.read(),
        &standing,
        &finishes,
        Default::default(),
        Language::En,
        Date::now(),
    );
    let moshpit_seal::Delivery::Card(card) = received.delivery else {
        panic!(
            "the phone couldn't open the host's push: {:?}",
            received.diagnostics
        );
    };
    let nonce = card.self_test_nonce().expect("a self-test push").to_owned();
    care.with(|m, k| m.self_test_arrived(nonce, k));
    until("the push test ends", 20, || {
        care.read(|m| !matches!(m.setup_view(page).unwrap().push_proof, Proof::Proving))
    })
    .await;
    let proof = care.read(|m| m.setup_view(page).unwrap().push_proof);
    assert!(matches!(proof, Proof::Proven(_)), "{proof:?}");

    // 5. Care again, a session later: nothing to put right.
    let before = phone.commands.lock().unwrap().len();
    care.with(|m, k| {
        m.forget(connection, k);
        m.tend(conn.clone(), k);
    });
    until("the pass ends", 20, || {
        !care.read(|m| m.is_tending(connection))
    })
    .await;
    let again: Vec<String> = phone.commands.lock().unwrap()[before..].to_vec();
    assert!(
        again.iter().all(|c| !c.contains("base64 -d >")),
        "care rewrote a current host: {again:?}"
    );

    // 6. Deleted by hand on the host (B315): this phone's push conf and the
    // sender. The manifest still says both are there; care looks and
    // puts them back, the conf the phone's byte for byte.
    std::fs::remove_file(home_path.join(format!(".moshpit/push.d/{connection}.conf"))).unwrap();
    std::fs::remove_file(home_path.join(".moshpit/moshpit-push.sh")).unwrap();
    let mints = relay.log().matches("mint").count();
    care.with(|m, k| {
        m.forget(connection, k);
        m.tend(conn.clone(), k);
    });
    until("the pass ends", 20, || {
        !care.read(|m| m.is_tending(connection))
    })
    .await;
    let phone_pairing = pairings.read().pop().unwrap();
    assert_eq!(
        read(&home_path, &format!(".moshpit/push.d/{connection}.conf")).as_deref(),
        Some(moshpit_install::push_conf(&phone_pairing).as_str()),
        "the conf deleted by hand wasn't written again"
    );
    assert_eq!(
        read(&home_path, ".moshpit/moshpit-push.sh").as_deref(),
        moshpit_install::scripts::body(&InstallComponent::Sender),
        "the sender deleted by hand wasn't installed again"
    );
    assert_eq!(
        relay.log().matches("mint").count(),
        mints,
        "the phone's pairing was still good"
    );

    // 7. A stale stamp script (an app update shipped a new one) is put right.
    std::fs::write(
        home_path.join(".moshpit/moshpit-stamp.sh"),
        "#!/bin/sh\n# an older build\n",
    )
    .unwrap();
    care.with(|m, k| {
        m.forget(connection, k);
        m.tend(conn.clone(), k);
    });
    until("the pass ends", 20, || {
        !care.read(|m| m.is_tending(connection))
    })
    .await;
    assert_eq!(
        read(&home_path, ".moshpit/moshpit-stamp.sh").as_deref(),
        moshpit_install::scripts::body(&InstallComponent::Stamp)
    );
    let fixed = InstallManifest::from_text(&read(&home_path, ".moshpit/manifest.json").unwrap());
    assert_eq!(
        fixed
            .get(&InstallComponent::Stamp)
            .and_then(|c| c.digest.clone()),
        moshpit_install::scripts::digest(&InstallComponent::Stamp)
    );

    // 8. Hooks taken out of Claude Code's config by hand: the screen says
    // so, care doesn't put them back and asks again. Then put back by hand.
    let hooked_config = read(&home_path, ".claude/settings.json").unwrap();
    std::fs::write(
        home_path.join(".claude/settings.json"),
        "{\"model\":\"opus\"}",
    )
    .unwrap();
    care.with(|m, k| m.setup_inspect(page, k));
    until("the screen reads the host", 20, || {
        care.read(|m| m.setup_view(page).unwrap().phase == moshpit_hostcare::Phase::Idle)
    })
    .await;
    assert_eq!(
        care.read(|m| m.setup_view(page).unwrap().hooks),
        moshpit_install::installer::ComponentStatus::Absent
    );
    care.with(|m, k| {
        m.forget(connection, k);
        m.tend(conn.clone(), k);
    });
    until("the pass ends", 20, || {
        !care.read(|m| m.is_tending(connection))
    })
    .await;
    assert_eq!(
        read(&home_path, ".claude/settings.json").as_deref(),
        Some("{\"model\":\"opus\"}"),
        "care put back hooks taken out by hand"
    );
    assert!(care.read(|m| m.consent().is_some()), "nobody was asked");
    care.with(|m, k| m.dismiss_consent(k));
    std::fs::write(home_path.join(".claude/settings.json"), &hooked_config).unwrap();
    care.with(|m, k| m.setup_inspect(page, k));
    until("the screen reads the host", 20, || {
        care.read(|m| {
            let view = m.setup_view(page).unwrap();
            view.phase == moshpit_hostcare::Phase::Idle
                && view.hooks == moshpit_install::installer::ComponentStatus::Current
        })
    })
    .await;

    // 9. The screen takes it all back out: hooks, then the pairing.
    care.with(|m, k| m.setup_remove_hooks(page, k));
    until("the hooks come out", 20, || {
        care.read(|m| m.setup_view(page).unwrap().phase == moshpit_hostcare::Phase::Idle)
            && !read(&home_path, ".claude/settings.json")
                .unwrap_or_default()
                .contains("moshpit-stamp.sh")
    })
    .await;
    care.with(|m, k| m.setup_unpair(page, k));
    until("the host is unpaired", 20, || {
        read(&home_path, &format!(".moshpit/push.d/{connection}.conf")).is_none()
            && care.read(|m| m.setup_view(page).unwrap().phase == moshpit_hostcare::Phase::Idle)
    })
    .await;
    assert!(pairings.read().is_empty(), "the phone kept the secret");
    let after = InstallManifest::from_text(&read(&home_path, ".moshpit/manifest.json").unwrap());
    assert!(after.pairing_entry(&connection.to_string()).is_none());
    assert!(
        after
            .get(&InstallComponent::Hooks {
                agent: "claude".into()
            })
            .is_none()
    );
    let view = care.read(|m| m.setup_view(page).unwrap());
    assert_eq!(view.error, None);
    assert_eq!(
        view.pairing,
        moshpit_install::installer::ComponentStatus::Absent
    );

    // What a command takes here, against what the machine waits for one:
    // the budgets must leave a slow host's link a wide margin.
    let (slowest, which) = phone.slowest.lock().unwrap().clone();
    eprintln!("live borrow: slowest command {slowest:?}: {which}");
    let budgets = moshpit_hostcare::Budgets::default();
    assert!(
        slowest < Duration::from_millis(budgets.care_exec_ms.min(budgets.setup_exec_ms) / 10),
        "a command took {slowest:?} on loopback: the budgets leave no margin"
    );

    // 10. A host that stops answering (every shell it starts hangs): care's
    // pass gives up on the command at its deadline, not when, or if, the
    // host answers.
    std::fs::write(home_path.join(".zshenv"), "sleep 6\n").unwrap();
    std::fs::write(home_path.join(".bashrc"), "sleep 6\n").unwrap();
    let hung = Driver::new(
        HostCare::new(
            Config {
                push: PushSupport::Apns {
                    environment: "sandbox".into(),
                },
                autocare_off: false,
                budgets: moshpit_hostcare::Budgets {
                    care_exec_ms: 1_500,
                    ..Default::default()
                },
            },
            Box::new(pairings.clone()),
            Clock {
                ms: 0,
                date: Date::now(),
            },
        ),
        phone.clone(),
        tokio::runtime::Handle::current(),
    );
    hung.with(|m, k| {
        m.set_settings(
            Settings {
                relay_url: relay.url(),
                ..Settings::default()
            },
            k,
        );
        m.set_saved(&[connection], Some(&[connection]), k);
    });
    let started = std::time::Instant::now();
    hung.with(|m, k| m.tend(conn.clone(), k));
    assert!(hung.read(|m| m.is_tending(connection)));
    until("the pass gives up on the host", 5, || {
        !hung.read(|m| m.is_tending(connection))
    })
    .await;
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(1_400) && waited < Duration::from_secs(5),
        "waited {waited:?}: not the deadline"
    );
    assert!(
        hung.read(|m| m.consent().is_none()),
        "the pass read the host: it never hung"
    );
    std::fs::remove_file(home_path.join(".zshenv")).unwrap();
    std::fs::remove_file(home_path.join(".bashrc")).unwrap();

    let commands = phone.commands.lock().unwrap().len();
    eprintln!(
        "live borrow: {commands} commands on one borrowed connection, {} sign-in(s); relay: {}",
        sshd.accepted(),
        relay
            .log()
            .lines()
            .filter(|l| l.contains("POST") || l.contains("mint") || l.contains("DRY RUN"))
            .count()
    );
    assert_eq!(sshd.accepted(), 1, "host care signed in beside the session");
    care.with(|m, k| m.close_setup(page, k));
    let _ = bench.sessions.disconnect(session).await;
}
