//! The installer against a real `sh` in a private HOME, not a simulated
//! host: what its commands actually do to files a person already has.

#![cfg(unix)]

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use moshpit_install::installer::{ChannelError, HookProof, HostChannel, HostInstaller};
use moshpit_install::{HookAgent, InstallComponent};
use moshpit_seal::pairing::PushPairing;
use moshpit_seal::{Date, Uuid};

/// A host that is this machine's `sh`, with `home` as `$HOME`. A command
/// that exits non-zero fails, as the app's exec channel does.
struct Shell {
    home: PathBuf,
    /// Searched before the system's directories: stand-ins for a program.
    bin: Option<PathBuf>,
}

impl Shell {
    fn at(home: &Path) -> Self {
        Shell {
            home: home.to_owned(),
            bin: None,
        }
    }
}

impl HostChannel for Shell {
    fn run(&self, command: &str) -> impl Future<Output = Result<Vec<u8>, ChannelError>> {
        let system = "/usr/bin:/bin:/usr/sbin:/sbin";
        let path = match &self.bin {
            Some(bin) => format!("{}:{system}", bin.display()),
            None => system.to_owned(),
        };
        // No pipes (B566's test, macOS only; the app's commands run over
        // SSH, not here). On macOS a pipe is made close-on-exec only after
        // it is made, so a child another thread starts in that moment
        // inherits it, and this command isn't done until that child exits:
        // with several writers at once, a waiter for the manifest lock held
        // its holder's command open, the holder couldn't go on to let go,
        // and the waiter gave up. So the output goes to files (opened
        // close-on-exec at once, and an inherited one holds nobody), and
        // `sh` is named by its path: a program looked up on a PATH the
        // child is given is started by fork and exec, which reports a
        // failed exec over a pipe of its own; posix_spawn needs none.
        let scratch = |name: &str| {
            std::env::temp_dir().join(format!("install-{name}-{}", moshpit_seal::random::hex(8)))
        };
        let (out_path, err_path) = (scratch("out"), scratch("err"));
        let status = (|| {
            std::process::Command::new("/bin/sh")
                .args(["-c", command])
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", path)
                // The host scripts put the system's tools first, and the
                // stand-ins (a stub curl) only where named outright.
                .env(
                    "MOSHPIT_TEST_BIN",
                    self.bin.as_deref().map(Path::as_os_str).unwrap_or_default(),
                )
                .stdin(std::process::Stdio::null())
                .stdout(std::fs::File::create(&out_path)?)
                .stderr(std::fs::File::create(&err_path)?)
                .status()
        })();
        let stdout = std::fs::read(&out_path).unwrap_or_default();
        let stderr = std::fs::read(&err_path).unwrap_or_default();
        let _ = std::fs::remove_file(&out_path);
        let _ = std::fs::remove_file(&err_path);
        std::future::ready(match status {
            Ok(status) if status.success() => Ok(stdout),
            Ok(_) => Err(ChannelError(String::from_utf8_lossy(&stderr).into_owned())),
            Err(error) => Err(ChannelError(error.to_string())),
        })
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    match pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the shell answers at once"),
    }
}

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home() -> Home {
    let dir = std::env::temp_dir().join(format!("install-home-{}", moshpit_seal::random::hex(6)));
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    Home(dir)
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn claude() -> HookAgent {
    HookAgent::named("claude").expect("claude")
}

/// Installs, then uninstalls, Claude Code's hooks against `home`.
fn install_and_remove(home: &Path) {
    let shell = Shell::at(home);
    let installer = HostInstaller::new(&shell);
    let now = Date::from_unix(1_762_800_000.0);
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.install_hooks(&claude(), &state, now)).expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.uninstall_hooks(&claude(), &state)).expect("uninstall");
    assert!(report.succeeded(), "{:?}", report.failures());
}

/// B352: installing or removing the hooks set the person's agent config to
/// 644 whatever it was. These configs hold secrets (an `env` block with an
/// API key, an MCP server's token), and one kept 600 became readable by
/// every local user on a shared host. It keeps its own mode now.
#[test]
fn a_private_agent_config_stays_private_through_install_and_removal() {
    let home = home();
    let config = home.0.join(".claude/settings.json");
    std::fs::write(&config, "{\"env\": {\"ANTHROPIC_API_KEY\": \"sk-test\"}}\n").unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let report =
        block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_800_000.0)))
            .expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
    assert_eq!(
        mode(&config),
        0o600,
        "installing the hooks opened the config up"
    );
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.uninstall_hooks(&claude(), &state)).expect("uninstall");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(
        !std::fs::read_to_string(&config)
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
    assert_eq!(
        mode(&config),
        0o600,
        "removing the hooks opened the config up"
    );
}

/// A config kept in dotfiles: the link stays a link, and what it points at
/// keeps its mode.
#[test]
fn a_linked_agent_config_stays_a_link_and_keeps_its_mode() {
    let home = home();
    let real = home.0.join("dotfiles-settings.json");
    std::fs::write(&real, "{}\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config = home.0.join(".claude/settings.json");
    std::os::unix::fs::symlink(&real, &config).unwrap();
    install_and_remove(&home.0);
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(mode(&real), 0o600);
}

/// A config the hooks create is made as the agents make theirs: 644.
#[test]
fn a_config_the_install_creates_is_made_644() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let report =
        block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_800_000.0)))
            .expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(mode(&home.0.join(".claude/settings.json")), 0o644);
}

/// A stand-in for `curl` that keeps each request's body in `capture` and
/// answers 200.
fn stub_curl(bin: &Path, capture: &Path) {
    std::fs::create_dir_all(bin).unwrap();
    std::fs::create_dir_all(capture).unwrap();
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\nbody=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    \
             --data-binary) body=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\n\
             printf '%s' \"$body\" > \"$(mktemp '{}/req.XXXXXX')\"\nprintf 200\n",
            capture.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The connection each request the stub kept was routed for.
fn requested(capture: &Path) -> Vec<String> {
    let mut conns: Vec<String> = std::fs::read_dir(capture)
        .unwrap()
        .map(|entry| {
            let body = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let start = body.find("\"conn\":\"").expect("a conn") + "\"conn\":\"".len();
            body[start..][..36].to_owned()
        })
        .collect();
    conns.sort();
    conns
}

/// One phone's pairing of one connection, as the app mints it.
fn pairing(conn: &str, phone: &str) -> PushPairing {
    let now = Date::from_unix(1_762_800_000.0);
    PushPairing {
        connection_id: Uuid::parse(conn).expect("a uuid"),
        host_label: "m1-pro".into(),
        secret_hex: "ab".repeat(32),
        send_token: "cd".repeat(32),
        relay_url: "https://push.example.org".into(),
        created_at: now,
        apns_token: Some(phone.into()),
        apns_env: Some("sandbox".into()),
        send_token_issued_at: Some(now),
    }
}

/// A host whose `curl` is the stub, with each of `pairings` installed by
/// the real installer.
fn paired_host(home: &Path, pairings: &[PushPairing]) -> (Shell, PathBuf) {
    let capture = home.join("capture");
    stub_curl(&home.join("bin"), &capture);
    let shell = Shell {
        home: home.to_owned(),
        bin: Some(home.join("bin")),
    };
    for pairing in pairings {
        let installer = HostInstaller::new(&shell);
        let state = block_on(installer.inspect()).expect("inspect");
        let report =
            block_on(installer.install_pairing(pairing, &state, Date::from_unix(1_762_800_000.0)))
                .expect("pair");
        assert!(report.succeeded(), "{:?}", report.failures());
    }
    (shell, capture)
}

const PHONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_PHONE: &str = "2222222222222222222222222222222222222222222222222222222222222222";
/// Sorts first: this phone's other connection to the host.
const FIRST: &str = "10000000-0000-4000-8000-000000000001";
/// The other phone's connection.
const SECOND: &str = "50000000-0000-4000-8000-000000000005";
/// The connection a test proves: sorts last.
const PROVED: &str = "90000000-0000-4000-8000-000000000009";

/// B353: the push test went out on every pairing the host held. Another
/// phone paired with it got the test, and showed it on its lock screen as
/// a finished `moshpit-selftest` turn; and a phone with two connections to
/// the host had it go out on whichever pairing sorted first, so a test of
/// one connection passed on the other's pairing. It goes out on the pairing
/// of the connection it proves, and nowhere else.
#[test]
fn the_push_test_goes_out_on_the_tested_connections_pairing_alone() {
    let home = home();
    let (shell, capture) = paired_host(
        &home.0,
        &[
            pairing(FIRST, PHONE),
            pairing(SECOND, OTHER_PHONE),
            pairing(PROVED, PHONE),
        ],
    );
    let said = block_on(HostInstaller::new(&shell).self_test_push("selftest-ab12", PROVED))
        .expect("the test runs");
    assert_eq!(said, "");
    assert_eq!(requested(&capture), [PROVED]);
}

/// B664: a phone with two saved connections to one host holds two pairings
/// here and gets each push once — on whichever pairing sorted first, so the
/// card, its tap and its withdrawal named the connection the phone may not
/// use. The app's look over the host (the drift check, each time it tends
/// it) marks this device's conf, and the push goes out on the one the phone
/// used last.
#[test]
fn a_phone_with_two_connections_here_is_pushed_on_the_one_it_used_last() {
    let home = home();
    let (shell, capture) = paired_host(&home.0, &[pairing(FIRST, PHONE), pairing(PROVED, PHONE)]);
    let push_d = home.0.join(".moshpit/push.d");
    let confs: Vec<PathBuf> = std::fs::read_dir(&push_d)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(confs.len(), 2, "{confs:?}");
    let push = || {
        for request in std::fs::read_dir(&capture).unwrap() {
            std::fs::remove_file(request.unwrap().path()).unwrap();
        }
        let status = std::process::Command::new("sh")
            .arg(home.0.join(".moshpit/moshpit-push.sh"))
            .args(["attention", "claude", "waiting"])
            .stdin(std::process::Stdio::null())
            .env_clear()
            .env("HOME", &home.0)
            .env("PATH", format!("{}:/usr/bin:/bin", home.0.join("bin").display()))
            .env("MOSHPIT_TEST_BIN", home.0.join("bin"))
            .status()
            .unwrap();
        assert!(status.success());
        requested(&capture)
    };
    let used = |conn: &str| {
        // Both confs an hour old, as from an earlier run of the app.
        let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for conf in &confs {
            std::fs::File::options().write(true).open(conf).unwrap().set_modified(earlier).unwrap();
        }
        let installer = HostInstaller::new(&shell);
        let state = block_on(installer.inspect()).expect("inspect");
        let (_, drifts) = block_on(installer.verify(state, Some(conn))).expect("the look");
        assert!(drifts.is_empty(), "{drifts:?}");
    };
    // The phone uses the connection that sorts last.
    used(PROVED);
    assert_eq!(push(), [PROVED]);
    // Now the other.
    used(FIRST);
    assert_eq!(push(), [FIRST]);
}

/// B700: the phone's notifications off, the app writes `NOTIFY=off` into
/// its conf and the sender sends that phone nothing — not on its other
/// pairing here either, which would have taken the push instead. A push
/// test still goes: the person asked for it.
#[test]
fn a_muted_phone_is_sent_nothing_but_its_push_test() {
    let home = home();
    let (shell, capture) = paired_host(&home.0, &[pairing(FIRST, PHONE), pairing(PROVED, PHONE)]);
    let push_d = home.0.join(".moshpit/push.d");
    let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for entry in std::fs::read_dir(&push_d).unwrap() {
        let conf = entry.unwrap().path();
        let text = std::fs::read_to_string(&conf).unwrap();
        if text.contains(&format!("CONN={PROVED}")) {
            // The one the phone used last, muted.
            std::fs::write(&conf, format!("{text}NOTIFY=off\n")).unwrap();
        } else {
            std::fs::File::options().write(true).open(&conf).unwrap().set_modified(earlier).unwrap();
        }
    }
    let status = std::process::Command::new("sh")
        .arg(home.0.join(".moshpit/moshpit-push.sh"))
        .args(["attention", "claude", "waiting"])
        .stdin(std::process::Stdio::null())
        .env_clear()
        .env("HOME", &home.0)
        .env("PATH", format!("{}:/usr/bin:/bin", home.0.join("bin").display()))
        .env("MOSHPIT_TEST_BIN", home.0.join("bin"))
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(requested(&capture), Vec::<String>::new());
    let said = block_on(HostInstaller::new(&shell).self_test_push("selftest-ab12", PROVED))
        .expect("the test runs");
    assert_eq!(said, "");
    assert_eq!(requested(&capture), [PROVED]);
}

/// A test for a connection the host holds no pairing for says so at once,
/// instead of leaving the phone to wait out its timeout for a push that was
/// never sent.
#[test]
fn a_push_test_with_no_pairing_to_send_with_says_so() {
    let home = home();
    let (shell, capture) = paired_host(&home.0, &[pairing(FIRST, PHONE)]);
    let said = block_on(HostInstaller::new(&shell).self_test_push("selftest-ab12", PROVED))
        .expect("the test runs");
    assert!(said.contains("no usable pairing"), "{said:?}");
    assert_eq!(requested(&capture), Vec::<String>::new());

    // A host with the sender and no pairing at all.
    let empty = self::home();
    let (shell, _) = paired_host(&empty.0, &[]);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let step = block_on(installer.install_script(&InstallComponent::Sender, &state))
        .expect("the sender lands");
    assert!(!step.is_failure());
    let said = block_on(HostInstaller::new(&shell).self_test_push("selftest-ab12", PROVED))
        .expect("the test runs");
    assert!(said.contains("no usable pairing"), "{said:?}");
}

/// B439: the push test runs outside any pane, so it named `%0` — tmux's
/// first pane, where an agent often waits — and the phone's notification
/// service cleared that pane's waiting entry on the test's "done". It names
/// a pane no tmux or herdr pane can be, even where a herdr pane id is in
/// the environment.
#[test]
fn the_push_test_names_a_pane_no_real_pane_can_be() {
    use moshpit_seal::foundation::json::{self, Dialect};
    let home = home();
    let (shell, capture) = paired_host(&home.0, &[pairing(PROVED, PHONE)]);
    let pane_sent = |capture: &Path| -> String {
        let request = std::fs::read_dir(capture).unwrap().next().unwrap().unwrap().path();
        let body = json::parse(&std::fs::read(&request).unwrap(), Dialect::Decoder).expect("JSON");
        std::fs::remove_file(request).unwrap();
        let env = body.as_object().and_then(|o| o.get("env")).expect("an envelope");
        let envelope: moshpit_seal::Envelope = json::decode_value(env).expect("an envelope");
        let status = moshpit_seal::envelope::open(&envelope, &"ab".repeat(32)).expect("it opens");
        assert_eq!(status.agent.as_deref(), Some("moshpit-selftest"));
        status.pane
    };
    let said = block_on(HostInstaller::new(&shell).self_test_push("selftest-ab12", PROVED))
        .expect("the test runs");
    assert_eq!(said, "");
    assert_eq!(pane_sent(&capture), moshpit_install::commands::SELF_TEST_PANE);
    // A shell whose environment names a herdr pane and a tmux one.
    let inherited = format!(
        "export HERDR_PANE_ID=w1:p2 TMUX_PANE=%5; {}",
        moshpit_install::commands::self_test_push("selftest-ab12", PROVED)
    );
    block_on(shell.run(&inherited)).expect("the test runs");
    assert_eq!(pane_sent(&capture), moshpit_install::commands::SELF_TEST_PANE);
}

/// B459: a phone views a session through a group of its own
/// (`<session>~mp-<device>-<tag>`, marked `@moshpit_group_of`), and a hook
/// inside the session's pane asks tmux for `#S`: tmux names the session it
/// finds best for the pane, the phone's group while the phone is on it. The
/// lock screen said "m1-pro · t~mp-iPhone-x7k". The push names the session.
#[test]
fn a_push_names_the_session_not_the_phones_group() {
    use moshpit_seal::foundation::json::{self, Dialect};
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let (shell, capture) = paired_host(&home.0, &[pairing(PROVED, PHONE)]);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let step = block_on(installer.install_script(&InstallComponent::Sender, &state))
        .expect("the sender lands");
    assert!(!step.is_failure());
    let session = tmux.run(&["display-message", "-p", "-t", "t", "#{session_id}"]);
    tmux.run(&["new-session", "-d", "-t", "t", "-s", "t~mp-iPhone-x7k"]);
    tmux.run(&[
        "set-option",
        "-t",
        "=t~mp-iPhone-x7k:",
        "@moshpit_group_of",
        &session,
    ]);
    // The phone's client, on its group.
    let mut phone = std::process::Command::new(tmux.bin.join("tmux"))
        .args(["-C", "attach", "-t", "=t~mp-iPhone-x7k"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let pane = tmux.pane();
    let status = std::process::Command::new("sh")
        .arg(home.0.join(".moshpit/moshpit-push.sh"))
        .args(["attention", "claude", "waiting"])
        .stdin(std::process::Stdio::null())
        .env_clear()
        .env("HOME", &home.0)
        .env("PATH", format!("{}:/usr/bin:/bin", tmux.bin.display()))
        .env("MOSHPIT_TEST_BIN", &tmux.bin)
        .env("TMUX", format!("{},1,0", tmux.socket.display()))
        .env("TMUX_PANE", &pane)
        .status()
        .unwrap();
    let _ = phone.kill();
    let _ = phone.wait();
    assert!(status.success());
    let request = std::fs::read_dir(&capture)
        .unwrap()
        .next()
        .expect("a push")
        .unwrap()
        .path();
    let body = json::parse(&std::fs::read(&request).unwrap(), Dialect::Decoder).expect("JSON");
    let env = body
        .as_object()
        .and_then(|o| o.get("env"))
        .expect("an envelope");
    let envelope: moshpit_seal::Envelope = json::decode_value(env).expect("an envelope");
    let status = moshpit_seal::envelope::open(&envelope, &"ab".repeat(32)).expect("it opens");
    assert_eq!(status.sess.as_deref(), Some("t"));
}

/// B354: which of the host's pairings push to a phone, read with the
/// host's own `sh` and `sed`. Only the confs carrying that phone's APNs
/// token, named for a connection; no secret or send token is ever printed.
#[test]
fn the_pairings_that_push_to_a_phone_are_read_without_their_secrets() {
    let home = home();
    let (shell, _) = paired_host(
        &home.0,
        &[
            pairing(FIRST, PHONE),
            pairing(SECOND, OTHER_PHONE),
            pairing(PROVED, &PHONE.to_uppercase()),
        ],
    );
    let push_d = home.0.join(".moshpit/push.d");
    // A conf from before the relay routed by token, and files that aren't
    // a connection's conf.
    std::fs::write(
        push_d.join("70000000-0000-4000-8000-000000000007.conf"),
        "RELAY_URL=r\nSEND_TOKEN=t\nSECRET=s\n",
    )
    .unwrap();
    std::fs::write(push_d.join("notes.conf"), format!("APNS_TOKEN={PHONE}\n")).unwrap();
    std::fs::write(push_d.join("x.conf.bak"), format!("APNS_TOKEN={PHONE}\n")).unwrap();
    std::fs::write(
        home.0.join(".moshpit/push.conf"),
        format!("APNS_TOKEN={PHONE}\n"),
    )
    .unwrap();

    let listing = block_on(shell.run(moshpit_install::commands::PAIRING_PHONES)).unwrap();
    let listing = String::from_utf8(listing).unwrap();
    for secret in ["ab".repeat(32), "cd".repeat(32)] {
        assert!(!listing.contains(&secret), "{listing}");
    }

    let installer = HostInstaller::new(&shell);
    let conns = |phone: &str| {
        let mut conns: Vec<String> = block_on(installer.pairings_reaching(phone))
            .unwrap()
            .into_iter()
            .map(|component| match component {
                InstallComponent::Pairing { conn, install: None } => conn,
                other => panic!("not a legacy pairing: {other:?}"),
            })
            .collect();
        conns.sort();
        conns
    };
    assert_eq!(conns(PHONE), [FIRST, PROVED]);
    assert_eq!(conns(OTHER_PHONE), [SECOND]);
    assert!(
        block_on(installer.pairings_reaching(""))
            .unwrap()
            .is_empty()
    );

    // An empty push.d, and none at all, read as nothing.
    let bare = self::home();
    let shell = Shell::at(&bare.0);
    let installer = HostInstaller::new(&shell);
    assert!(
        block_on(installer.pairings_reaching(PHONE))
            .unwrap()
            .is_empty()
    );
    std::fs::create_dir_all(bare.0.join(".moshpit/push.d")).unwrap();
    assert!(
        block_on(installer.pairings_reaching(PHONE))
            .unwrap()
            .is_empty()
    );
}

/// A tmux server of the test's own: a `tmux` on the shell's `PATH` that
/// always names this server's socket, so nothing reaches any other tmux on
/// this machine. `None` without a tmux to run.
struct Tmux {
    bin: PathBuf,
    socket: PathBuf,
}

impl Tmux {
    fn start(home: &Path) -> Option<Tmux> {
        let found = std::process::Command::new("sh")
            .args(["-c", "command -v tmux"])
            .output()
            .ok()?;
        let real = String::from_utf8(found.stdout).ok()?.trim().to_owned();
        if real.is_empty() {
            assert!(
                std::env::var_os("MOSHPIT_NO_TMUX").is_some(),
                "no tmux on PATH: install it, or set MOSHPIT_NO_TMUX=1 to skip"
            );
            return None;
        }
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let socket = home.join("tmux.sock");
        let wrapper = bin.join("tmux");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec '{real}' -S '{}' -f /dev/null \"$@\"\n",
                socket.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tmux = Tmux { bin, socket };
        tmux.run(&[
            "new-session",
            "-d",
            "-s",
            "t",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 600",
        ]);
        Some(tmux)
    }

    fn run(&self, args: &[&str]) -> String {
        let out = std::process::Command::new(self.bin.join("tmux"))
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    }

    fn pane(&self) -> String {
        self.run(&["display-message", "-p", "-t", "t", "#{pane_id}"])
    }

    /// The five stamps, `-` for one that isn't set.
    fn stamps(&self, pane: &str) -> Vec<String> {
        ["state", "agent", "since", "prompt", "title"]
            .iter()
            .map(|o| {
                let set = self.run(&["show-options", "-p", "-t", pane, &format!("@moshpit_{o}")]);
                if set.is_empty() {
                    "-".to_owned()
                } else {
                    self.run(&["show-options", "-pv", "-t", pane, &format!("@moshpit_{o}")])
                }
            })
            .collect()
    }
}

impl Drop for Tmux {
    fn drop(&mut self) {
        self.run(&["kill-server"]);
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The installer over a host whose `tmux` is `tmux`'s own server, with
/// the stamp script installed.
fn stamp_host(home: &Path, tmux: &Tmux) -> Shell {
    let shell = Shell {
        home: home.to_owned(),
        bin: Some(tmux.bin.clone()),
    };
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let step = block_on(installer.install_script(&InstallComponent::Stamp, &state))
        .expect("the stamp script lands");
    assert!(!step.is_failure());
    shell
}

/// B355: the hooks' self-test fires into the session's active pane, where
/// the person's agent usually runs, and then cleared the pane's stamps:
/// the agent waiting on a question lost its state (so the island dropped
/// it, and the grace push that re-reads the state never went), its name,
/// the prompt its finish card names, and its title, and the pane kept a
/// `since` of the test's. The test still proves the hook, and the pane says
/// what it said before.
#[test]
fn the_hooks_test_leaves_a_live_agents_pane_as_it_found_it() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let shell = stamp_host(&home.0, &tmux);
    let pane = tmux.pane();
    for (option, value) in [
        ("state", "attention"),
        ("agent", "claude"),
        ("since", "1762800000"),
        ("prompt", "fix the flaky build"),
        ("title", "Bash: rm -rf build | tee log"),
    ] {
        tmux.run(&[
            "set",
            "-p",
            "-t",
            &pane,
            &format!("@moshpit_{option}"),
            value,
        ]);
    }
    let before = tmux.stamps(&pane);
    let proof = block_on(HostInstaller::new(&shell).self_test_stamp(&pane)).expect("the test runs");
    assert_eq!(proof, moshpit_install::installer::StampProof::Stamped);
    assert_eq!(tmux.stamps(&pane), before);
}

/// A pane with nothing stamped is left with nothing stamped, `since`
/// included.
#[test]
fn the_hooks_test_leaves_an_unstamped_pane_unstamped() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let shell = stamp_host(&home.0, &tmux);
    let pane = tmux.pane();
    let proof = block_on(HostInstaller::new(&shell).self_test_stamp(&pane)).expect("the test runs");
    assert_eq!(proof, moshpit_install::installer::StampProof::Stamped);
    assert_eq!(tmux.stamps(&pane), ["-", "-", "-", "-", "-"]);
}

/// B356, through the real installer: a hook the person added to our `Stop`
/// group survives the refresh care makes when the scripts change, and the
/// hooks' removal.
#[test]
fn a_hook_the_person_added_to_our_group_survives_a_refresh_and_removal() {
    let home = home();
    let config = home.0.join(".claude/settings.json");
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let report =
        block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_800_000.0)))
            .expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    // The person adds a hook of their own to the Stop group that is there.
    let ours = "\"command\" : \"sh ~/.moshpit/moshpit-stamp.sh done claude\"";
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains(ours), "{text}");
    let edited = text.replacen(
        ours,
        &format!("{ours}\n          }},\n          {{\n            \"command\" : \"say done\",\n            \"type\" : \"command\""),
        1,
    );
    assert!(edited.contains("say done"));
    std::fs::write(&config, &edited).unwrap();

    let state = block_on(installer.inspect()).expect("inspect");
    let report =
        block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_900_000.0)))
            .expect("refresh");
    assert!(report.succeeded(), "{:?}", report.failures());
    let refreshed = std::fs::read_to_string(&config).unwrap();
    assert!(
        refreshed.contains("say done"),
        "the refresh took the person's hook: {refreshed}"
    );
    assert_eq!(refreshed.matches("moshpit-stamp.sh done claude").count(), 1);

    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.uninstall_hooks(&claude(), &state)).expect("uninstall");
    assert!(report.succeeded(), "{:?}", report.failures());
    let removed = std::fs::read_to_string(&config).unwrap();
    assert!(
        removed.contains("say done"),
        "removal took the person's hook: {removed}"
    );
    assert!(!removed.contains("moshpit-stamp.sh"), "{removed}");
}

/// Runs the installed stamp script in `pane` as an agent's hook would, with
/// `grace` seconds before an attention push.
fn stamp(home: &Path, tmux: &Tmux, pane: &str, state: &str, grace: &str) {
    let status = std::process::Command::new("sh")
        .arg(home.join(".moshpit/moshpit-stamp.sh"))
        .args([state, "claude"])
        .stdin(std::process::Stdio::null())
        .env_clear()
        .env("HOME", home)
        .env("PATH", format!("{}:/usr/bin:/bin", tmux.bin.display()))
        .env("MOSHPIT_TEST_BIN", &tmux.bin)
        .env("TMUX", format!("{},1,0", tmux.socket.display()))
        .env("TMUX_PANE", pane)
        .env("MOSHPIT_NOTIFY_GRACE", grace)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Waits up to five seconds for `path`.
fn appears(path: &Path) -> Option<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            return Some(text);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

/// B358: the stamp script ran the sender only if `-x` said it could, and
/// exec'd it. A home mounted noexec fails both: every push was dropped
/// without a word, while the setup screen's push test — which runs the
/// sender with `sh` — said it worked. A sender without its exec bit stands
/// in for the mount here (same `-x`); the finish push and the question's
/// push after its grace both go out.
#[test]
fn the_stamp_script_hands_off_to_a_sender_it_cannot_exec() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let _ = stamp_host(&home.0, &tmux);
    let sender = home.0.join(".moshpit/moshpit-push.sh");
    let pushed = home.0.join("pushed");
    std::fs::write(
        &sender,
        format!("#!/bin/sh\necho \"$1 $2\" >> '{}'\n", pushed.display()),
    )
    .unwrap();
    std::fs::set_permissions(&sender, std::fs::Permissions::from_mode(0o644)).unwrap();
    let pane = tmux.pane();

    stamp(&home.0, &tmux, &pane, "working", "0");
    stamp(&home.0, &tmux, &pane, "done", "0");
    assert_eq!(
        appears(&pushed).as_deref(),
        Some("done claude\n"),
        "the finish push"
    );

    std::fs::remove_file(&pushed).unwrap();
    stamp(&home.0, &tmux, &pane, "working", "0");
    stamp(&home.0, &tmux, &pane, "attention", "0");
    assert_eq!(
        appears(&pushed).as_deref(),
        Some("attention claude\n"),
        "the question's push"
    );
}

/// B666: the pushed finish was timed from the turn's last episode alone: a
/// twenty-minute task that asked one question at minute nineteen read
/// "1 min" and rang nothing. The pane keeps its turn's start
/// (`@moshpit_turn`), set by the first `working` after it was idle or done
/// and kept through its questions; the finish is timed from it.
#[test]
fn the_pushed_finish_is_timed_from_the_turns_start() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let _ = stamp_host(&home.0, &tmux);
    let sender = home.0.join(".moshpit/moshpit-push.sh");
    let pushed = home.0.join("pushed");
    std::fs::write(&sender, format!("#!/bin/sh\necho \"$1 $4\" >> '{}'\n", pushed.display())).unwrap();
    let pane = tmux.pane();
    let turn = || tmux.run(&["display-message", "-p", "-t", &pane, "#{@moshpit_turn}"]).trim().to_owned();
    // The finish push's duration, once it has gone out (it is detached).
    let finish = || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let text = std::fs::read_to_string(&pushed).unwrap_or_default();
            if let Some(line) = text.lines().find(|line| line.starts_with("done ")) {
                return line["done ".len()..].trim().parse::<i64>().expect(line);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("no finish push")
    };

    stamp(&home.0, &tmux, &pane, "working", "0");
    let began = turn();
    assert!(began.parse::<i64>().is_ok(), "{began:?}");
    // Twenty minutes ago, as the turn would have begun.
    let twenty_ago = (began.parse::<i64>().unwrap() - 1200).to_string();
    tmux.run(&["set", "-p", "-t", &pane, "@moshpit_turn", &twenty_ago]);
    // A question and its answer keep the turn.
    stamp(&home.0, &tmux, &pane, "attention", "0");
    stamp(&home.0, &tmux, &pane, "working", "0");
    assert_eq!(turn(), twenty_ago);
    stamp(&home.0, &tmux, &pane, "done", "0");
    let duration = finish();
    assert!((1200..1260).contains(&duration), "{duration}");

    // The next turn starts its own.
    let _ = std::fs::remove_file(&pushed);
    stamp(&home.0, &tmux, &pane, "working", "0");
    assert_ne!(turn(), twenty_ago);
    stamp(&home.0, &tmux, &pane, "done", "0");
    let duration = finish();
    assert!(duration < 60, "{duration}");
}

/// Runs the installed stamp script as an agent's hook would, with `env`.
fn fire(home: &Path, state: &str, agent: &str, env: &[(&str, &str)]) {
    let mut command = std::process::Command::new("sh");
    command
        .arg(home.join(".moshpit/moshpit-stamp.sh"))
        .args([state, agent])
        .stdin(std::process::Stdio::null())
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin");
    for (key, value) in env {
        command.env(key, value);
    }
    assert!(command.status().unwrap().success());
}

/// B380: an agent's own hook leaves its mark the first time it runs here,
/// in tmux or not — the Host Setup page reads it to tell Codex's trusted
/// hooks from ones Codex still skips. The page's own test leaves none, and
/// no label reaches a path outside the marks' directory.
#[test]
fn a_real_hook_fire_leaves_the_agents_mark_and_the_pages_test_leaves_none() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let step = block_on(installer.install_script(&InstallComponent::Stamp, &state))
        .expect("the stamp script lands");
    assert!(!step.is_failure());
    let seen = home.0.join(".moshpit/seen");
    // Outside tmux: no stamp to set, but the mark is left all the same.
    fire(&home.0, "working", "codex", &[]);
    assert!(seen.join("codex").exists(), "Codex's hook left no mark");
    fire(&home.0, "working", "gemini", &[("MOSHPIT_SELFTEST", "1")]);
    assert!(!seen.join("gemini").exists(), "the page's test left a mark");
    fire(&home.0, "working", "../escape", &[]);
    fire(&home.0, "working", "moshpit-selftest", &[]);
    assert!(!home.0.join(".moshpit/escape").exists());
    assert!(!seen.join("moshpit-selftest").exists());
}

/// B380: the page reads the marks with one read-only command.
#[test]
fn the_probe_reads_which_agents_hooks_have_run() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let none = block_on(installer.agents()).expect("the probe runs");
    assert!(none.seen.is_empty());
    let seen = home.0.join(".moshpit/seen");
    std::fs::create_dir_all(&seen).unwrap();
    for agent in ["codex", "claude"] {
        std::fs::write(seen.join(agent), "").unwrap();
    }
    let agents = block_on(installer.agents()).expect("the probe runs");
    assert!(agents.seen("codex") && agents.seen("claude") && !agents.seen("gemini"));
}

/// B380: a changed Codex registration hasn't been trusted yet, so its mark
/// goes; a reinstall that writes the same block keeps it.
#[test]
fn a_codex_install_that_changes_its_hooks_forgets_its_mark_and_one_that_doesnt_keeps_it() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let codex = HookAgent::named("codex").expect("codex");
    let now = Date::from_unix(1_762_800_000.0);
    let mark = home.0.join(".moshpit/seen/codex");
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, "").unwrap();
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.install_hooks(&codex, &state, now)).expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(!mark.exists(), "a new registration kept the old one's mark");
    std::fs::write(&mark, "").unwrap();
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.install_hooks(&codex, &state, now)).expect("reinstall");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(mark.exists(), "an unchanged registration lost its mark");
}

/// B380: the page's test runs what the agent's config registers, in a real
/// tmux pane: it proves the registration and the script, labels its stamp
/// as the test, leaves the pane as it was and leaves no mark. A config
/// missing one of Moshpit's events fails, naming the event.
#[test]
fn the_hooks_test_runs_the_registered_command_in_a_real_pane() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let shell = Shell {
        home: home.0.clone(),
        bin: Some(tmux.bin.clone()),
    };
    let installer = HostInstaller::new(&shell);
    let now = Date::from_unix(1_762_800_000.0);
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.install_hooks(&claude(), &state, now)).expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    let pane = tmux.pane();
    let before = tmux.stamps(&pane);
    let proof = block_on(installer.self_test_registration(
        &claude(),
        "$HOME/.claude/settings.json",
        &pane,
    ))
    .expect("the test runs");
    assert_eq!(proof, HookProof::Stamped);
    assert_eq!(tmux.stamps(&pane), before);
    assert!(!home.0.join(".moshpit/seen/claude").exists());

    // Someone moved Moshpit's Notification hook under another event.
    let config = home.0.join(".claude/settings.json");
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("\"Notification\""));
    std::fs::write(&config, text.replace("\"Notification\"", "\"Elsewhere\"")).unwrap();
    let proof = block_on(installer.self_test_registration(
        &claude(),
        "$HOME/.claude/settings.json",
        &pane,
    ))
    .expect("the test runs");
    assert_eq!(
        proof,
        HookProof::NotRegistered {
            event: "Notification".into()
        }
    );
}

/// B374: the preflight finds an agent by its config directory or by its
/// CLI on PATH, with this machine's `sh`.
#[test]
fn the_preflight_finds_the_agents_a_host_has() {
    let home = home();
    // `home()` made ~/.claude. Codex has a config directory; Factory's CLI
    // (`droid`) is on PATH.
    std::fs::create_dir_all(home.0.join(".codex")).unwrap();
    let bin = home.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("droid"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(bin.join("droid"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let shell = Shell {
        home: home.0.clone(),
        bin: Some(bin),
    };
    let facts = block_on(HostInstaller::new(&shell).preflight()).expect("preflight");
    let found: Vec<&str> = HookAgent::ALL
        .iter()
        .filter(|a| facts.has_agent(a.id))
        .map(|a| a.id)
        .collect();
    assert_eq!(found, ["claude", "codex", "factory"]);
    assert_eq!(
        facts.agents.len(),
        HookAgent::ALL.len(),
        "{:?}",
        facts.agents
    );
    assert!(facts.tools.keys().all(|k| !k.starts_with("agent.")));
    assert!(facts.has_base64());
}

/// A host with the stamp script installed by the real installer and, as
/// its sender, one that writes `<state> <agent>` to `pushed` a line per
/// push. No tmux anywhere.
fn untmuxed_host(home: &Path) -> PathBuf {
    let shell = Shell::at(home);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).expect("inspect");
    let step = block_on(installer.install_script(&InstallComponent::Stamp, &state))
        .expect("the stamp script lands");
    assert!(!step.is_failure());
    let pushed = home.join("pushed");
    std::fs::write(
        home.join(".moshpit/moshpit-push.sh"),
        format!("#!/bin/sh\necho \"$1 $2\" >> '{}'\n", pushed.display()),
    )
    .unwrap();
    pushed
}

/// Runs the installed stamp script as a hook would outside tmux: no
/// `$TMUX`, the hook's JSON (`session`'s, when given) on stdin.
fn stamp_untmuxed(home: &Path, state: &str, grace: &str, session: Option<&str>) {
    use std::io::Write;
    let mut child = std::process::Command::new("sh")
        .arg(home.join(".moshpit/moshpit-stamp.sh"))
        .args([state, "claude"])
        .stdin(std::process::Stdio::piped())
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("MOSHPIT_NOTIFY_GRACE", grace)
        .spawn()
        .unwrap();
    let json = session.map_or(String::new(), |id| {
        format!("{{\"session_id\":\"{id}\",\"hook_event_name\":\"Stop\"}}")
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(json.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

fn pushes(pushed: &Path) -> Vec<String> {
    std::fs::read_to_string(pushed)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// B370: outside tmux (herdr, a plain ssh or mosh shell) the stamp script
/// exited before anything, so no agent there ever pushed. A finish pushes
/// now, and a question pushes once its grace has passed — unless it was
/// answered in time, or it was only an idle reminder.
#[test]
fn outside_tmux_a_finish_and_a_question_still_push() {
    let home = home();
    let pushed = untmuxed_host(&home.0);

    stamp_untmuxed(&home.0, "working", "0", None);
    stamp_untmuxed(&home.0, "done", "0", None);
    assert_eq!(
        appears(&pushed).as_deref(),
        Some("done claude\n"),
        "the finish"
    );

    std::fs::remove_file(&pushed).unwrap();
    stamp_untmuxed(&home.0, "working", "0", None);
    stamp_untmuxed(&home.0, "attention", "0", None);
    assert_eq!(
        appears(&pushed).as_deref(),
        Some("attention claude\n"),
        "the question"
    );

    // Answered at the desk within the grace: no push.
    std::fs::remove_file(&pushed).unwrap();
    stamp_untmuxed(&home.0, "working", "0", None);
    stamp_untmuxed(&home.0, "attention", "2", None);
    stamp_untmuxed(&home.0, "working", "0", None);
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert_eq!(
        pushes(&pushed),
        Vec::<String>::new(),
        "an answered question"
    );

    // An idle reminder after a finish: not a question.
    stamp_untmuxed(&home.0, "done", "0", None);
    assert_eq!(appears(&pushed).as_deref(), Some("done claude\n"));
    std::fs::remove_file(&pushed).unwrap();
    stamp_untmuxed(&home.0, "attention", "0", None);
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert_eq!(pushes(&pushed), Vec::<String>::new(), "an idle reminder");

    // What stands in for the pane is private.
    let run = std::fs::read_dir(home.0.join(".moshpit/run"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(mode(&run), 0o700, "{}", run.display());
    assert_eq!(mode(&run.join("state")), 0o600);
}

/// Two agent sessions outside tmux keep apart, when jq can read their ids:
/// one's finish doesn't answer the other's question.
#[test]
fn outside_tmux_two_sessions_keep_apart() {
    let has_jq = std::path::Path::new("/usr/bin/jq").exists();
    if !has_jq {
        return;
    }
    let home = home();
    let pushed = untmuxed_host(&home.0);
    stamp_untmuxed(&home.0, "working", "0", Some("one"));
    stamp_untmuxed(&home.0, "working", "0", Some("two"));
    stamp_untmuxed(&home.0, "attention", "1", Some("one"));
    stamp_untmuxed(&home.0, "done", "0", Some("two"));
    std::thread::sleep(std::time::Duration::from_secs(3));
    let mut got = pushes(&pushed);
    got.sort();
    assert_eq!(got, ["attention claude", "done claude"]);
}

/// A real fire marks the agent as seen on the host (the Codex trust
/// step reads it); Moshpit's own hooks test never does.
#[test]
fn a_real_fire_marks_the_agent_seen_and_the_hooks_test_does_not() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let shell = stamp_host(&home.0, &tmux);
    let pane = tmux.pane();
    let proof = block_on(HostInstaller::new(&shell).self_test_stamp(&pane)).expect("the test");
    assert_eq!(proof, moshpit_install::installer::StampProof::Stamped);
    assert!(
        !home.0.join(".moshpit/seen").exists(),
        "the test marked something seen"
    );
    stamp(&home.0, &tmux, &pane, "working", "0");
    assert!(home.0.join(".moshpit/seen/claude").exists());
}

/// B385: droid reads `~/.factory/hooks.json` once it exists, and the hooks
/// in its settings.json only while it doesn't. With the person's own
/// hooks file there, ours go into it beside theirs (and come out of
/// settings.json, where droid no longer reads them); removal takes ours
/// out of it and leaves theirs. Without one, settings.json as before.
#[test]
fn factorys_hooks_go_where_droid_reads_them() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let factory = HookAgent::named("factory").expect("factory");
    let now = Date::from_unix(1_762_800_000.0);
    let dir = home.0.join(".factory");
    std::fs::create_dir_all(&dir).unwrap();
    // Without a hooks file: its settings.json.
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.install_hooks(&factory, &state, now)).expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    let settings = std::fs::read_to_string(dir.join("settings.json")).unwrap();
    assert!(settings.contains("moshpit-stamp.sh"), "{settings}");
    // The person saves a hook through droid's /hooks: its hooks file now.
    let theirs = "{\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"say done\"}]}]}\n";
    std::fs::write(dir.join("hooks.json"), theirs).unwrap();
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.install_hooks(&factory, &state, now)).expect("install");
    assert!(report.succeeded(), "{:?}", report.failures());
    let hooks = std::fs::read_to_string(dir.join("hooks.json")).unwrap();
    assert!(hooks.contains("moshpit-stamp.sh working factory"), "{hooks}");
    assert!(hooks.contains("say done"), "the person's hook: {hooks}");
    assert!(!hooks.contains("\"hooks\" : {"), "an event map, no hooks key: {hooks}");
    let settings = std::fs::read_to_string(dir.join("settings.json")).unwrap();
    assert!(!settings.contains("moshpit-stamp.sh"), "{settings}");
    let recorded = report
        .manifest
        .as_ref()
        .and_then(|m| {
            m.get(&InstallComponent::Hooks {
                agent: "factory".into(),
            })
        })
        .and_then(|c| c.config_path.clone());
    assert_eq!(recorded.as_deref(), Some("$HOME/.factory/hooks.json"));
    let state = block_on(installer.inspect()).expect("inspect");
    let report = block_on(installer.uninstall_hooks(&factory, &state)).expect("uninstall");
    assert!(report.succeeded(), "{:?}", report.failures());
    let hooks = std::fs::read_to_string(dir.join("hooks.json")).unwrap();
    assert!(!hooks.contains("moshpit-stamp.sh"), "{hooks}");
    assert!(hooks.contains("say done"), "{hooks}");
}

/// A manifest component a test can tell apart.
fn pairing_record(conn: &str) -> impl FnOnce(&mut moshpit_install::InstallManifest) {
    let conn = conn.to_owned();
    move |manifest| {
        manifest.set(
            &InstallComponent::Pairing { conn: conn.clone(), install: None },
            Some(moshpit_install::manifest::Component {
                connection_id: Some(conn),
                ..moshpit_install::manifest::Component::new(Date::from_unix(1_762_800_000.0))
            }),
        )
    }
}

/// The host's manifest lock, as a holder leaves it (B566): a symlink to
/// `<unix time>.<token>`.
fn hold(lock: &Path, stamp: &str) {
    std::os::unix::fs::symlink(stamp, lock).unwrap();
}

/// What the lock names, if it is there.
fn held(lock: &Path) -> Option<String> {
    std::fs::read_link(lock)
        .ok()
        .map(|target| target.to_string_lossy().into_owned())
}

/// Whether the lock is there at all (a symlink to nothing reads as missing
/// to `exists`).
fn locked(lock: &Path) -> bool {
    lock.symlink_metadata().is_ok()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// B373: a manifest change is made under the host's lock, read afresh;
/// the lock is gone afterwards.
#[test]
fn a_manifest_change_takes_the_lock_and_lets_it_go() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    block_on(installer.update_manifest(pairing_record(FIRST))).expect("first");
    block_on(installer.update_manifest(pairing_record(SECOND))).expect("second");
    let manifest = block_on(installer.inspect()).unwrap().manifest;
    assert!(manifest.pairing_entry(FIRST).is_some());
    assert!(manifest.pairing_entry(SECOND).is_some());
    assert!(!locked(&home.0.join(".moshpit/.lock")), "the lock was left");
}

/// A lock someone holds now: the change waits about ten seconds (B566),
/// then gives up and says why, touching nothing — the holder's lock
/// included. A lock whose holder never came back (older than a minute) is
/// broken.
#[test]
fn a_held_lock_is_waited_for_and_a_dead_one_broken() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    std::fs::create_dir_all(home.0.join(".moshpit")).unwrap();
    let lock = home.0.join(".moshpit/.lock");
    let stamp = format!("{}.1", unix_now());
    hold(&lock, &stamp);
    let started = std::time::Instant::now();
    let error = block_on(installer.update_manifest(pairing_record(FIRST))).unwrap_err();
    assert!(error.0.contains("another Moshpit"), "{error:?}");
    assert!(started.elapsed() >= std::time::Duration::from_secs(8));
    assert!(
        !home.0.join(".moshpit/manifest.json").exists(),
        "written under another's lock"
    );
    assert_eq!(held(&lock), Some(stamp), "another's lock was let go");

    std::fs::remove_file(&lock).unwrap();
    hold(&lock, &format!("{}.1", unix_now() - 120));
    block_on(installer.update_manifest(pairing_record(FIRST))).expect("a dead lock is broken");
    assert!(!locked(&lock));
    let manifest = block_on(installer.inspect()).unwrap().manifest;
    assert!(manifest.pairing_entry(FIRST).is_some());
}

/// B566: an older Moshpit's lock, a directory with its stamp in `at`, is
/// waited for and, once its holder is a minute gone, broken — and nothing
/// is ever made inside it.
#[test]
fn an_older_moshpits_lock_is_waited_for_and_a_dead_one_broken() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let lock = home.0.join(".moshpit/.lock");
    std::fs::create_dir_all(&lock).unwrap();
    std::fs::write(lock.join("at"), format!("{}.1", unix_now())).unwrap();
    let error = block_on(installer.update_manifest(pairing_record(FIRST))).unwrap_err();
    assert!(error.0.contains("another Moshpit"), "{error:?}");
    let inside: Vec<_> = std::fs::read_dir(&lock)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        inside,
        vec![std::ffi::OsString::from("at")],
        "made inside another's lock"
    );

    std::fs::write(lock.join("at"), format!("{}.1", unix_now() - 120)).unwrap();
    block_on(installer.update_manifest(pairing_record(FIRST))).expect("a dead lock is broken");
    assert!(!locked(&lock));
    let manifest = block_on(installer.inspect()).unwrap().manifest;
    assert!(manifest.pairing_entry(FIRST).is_some());
}

/// B448: an older Moshpit takes the lock with `mkdir` and writes its `at`
/// a moment later. A waiter that read the lock in that moment, with no
/// `at` yet, broke the fresh lock and took it too: two writers at once,
/// and one's change was lost. Here that moment lasts more than a second:
/// the waiter takes the lock only once it is let go. (This Moshpit's own
/// lock has no such moment, B566.)
#[test]
fn a_lock_taken_a_moment_ago_is_not_broken_for_its_missing_at() {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let home = home();
    let lock = home.0.join(".moshpit/.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let now = || SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    std::fs::write(lock.join("at"), format!("{}.held", now())).unwrap();
    let start = Instant::now();
    let waiter = {
        let home = home.0.clone();
        std::thread::spawn(move || {
            let result = block_on(Shell::at(&home).run(&moshpit_install::commands::lock("waiter0123456789")));
            (result, Instant::now())
        })
    };
    let until = |at: f64| {
        let wake = start + Duration::from_secs_f64(at);
        std::thread::sleep(wake.saturating_duration_since(Instant::now()));
    };
    // The holder lets go; the next one has made the directory, not yet `at`.
    until(2.5);
    std::fs::remove_file(lock.join("at")).unwrap();
    // Its `at` lands; it holds the lock a little longer, then lets go.
    until(3.8);
    std::fs::write(lock.join("at"), format!("{}.next", now())).unwrap();
    until(4.5);
    std::fs::remove_dir_all(&lock).unwrap();
    let released = Instant::now();
    let (result, taken) = waiter.join().unwrap();
    result.expect("the waiter takes the lock");
    assert!(taken >= released, "the waiter broke a live lock {:?} before it was let go", released - taken);
}

/// B448: an older Moshpit's lock left with no `at` for good — a holder
/// gone between its `mkdir` and its `at` — is still broken, once it has
/// been so for about four seconds.
#[test]
fn a_lock_left_with_no_at_is_broken_after_four_tries() {
    let home = home();
    let lock = home.0.join(".moshpit/.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let started = std::time::Instant::now();
    block_on(Shell::at(&home.0).run(&moshpit_install::commands::lock("waiter0123456789"))).expect("broken and taken");
    assert!(started.elapsed() >= std::time::Duration::from_secs(3), "broken too soon");
    assert!(held(&lock).unwrap().ends_with(".waiter0123456789"));
}

/// Two phones changing one host at once: both changes land.
#[test]
fn two_changes_at_once_both_land() {
    let home = home();
    let threads: Vec<_> = [FIRST, SECOND, PROVED]
        .into_iter()
        .map(|conn| {
            let home = home.0.clone();
            std::thread::spawn(move || {
                let shell = Shell::at(&home);
                block_on(HostInstaller::new(&shell).update_manifest(pairing_record(conn)))
                    .expect("a change");
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let shell = Shell::at(&home.0);
    let manifest = block_on(HostInstaller::new(&shell).inspect())
        .unwrap()
        .manifest;
    for conn in [FIRST, SECOND, PROVED] {
        assert!(manifest.pairing_entry(conn).is_some(), "{conn} was lost");
    }
}

/// B566: many phones changing one host at once, round after round, all
/// starting together: every change lands, and nobody gives up. The lock
/// was taken in two steps (`mkdir`, then its stamp), so a waiter could
/// find it with no stamp and break it — two waiters together, the second
/// removing the first's fresh lock: two holders, a change lost (Linux).
/// And waiters that polled once a second, all in step, gave up after six
/// tries while the others took their turns (macOS). They showed in 2 of 15
/// runs of this suite with three writers; with eight, every round.
#[test]
fn many_changes_at_once_all_land() {
    const WRITERS: usize = 8;
    const ROUNDS: usize = 6;
    let conn = |writer: usize, round: usize| format!("{writer:08}-0000-4000-8000-{round:012}");
    let home = home();
    for round in 0..ROUNDS {
        let start = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));
        let threads: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let home = home.0.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    let shell = Shell::at(&home);
                    let conn = conn(writer, round);
                    start.wait();
                    block_on(HostInstaller::new(&shell).update_manifest(pairing_record(&conn)))
                        .unwrap_or_else(|e| panic!("{conn}: {}", e.0));
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
    let shell = Shell::at(&home.0);
    let manifest = block_on(HostInstaller::new(&shell).inspect())
        .unwrap()
        .manifest;
    let lost: Vec<String> = (0..WRITERS)
        .flat_map(|writer| (0..ROUNDS).map(move |round| conn(writer, round)))
        .filter(|conn| manifest.pairing_entry(conn).is_none())
        .collect();
    assert!(lost.is_empty(), "lost: {lost:?}");
    assert!(!locked(&home.0.join(".moshpit/.lock")), "the lock was left");
}

/// A host with Claude Code's hooks installed by the real installer, Codex's
/// config calling the stamp script with no record of it (a manifest another
/// writer lost), and this phone's pairing.
fn fully_installed(home: &Path) -> (Shell, PathBuf) {
    let (shell, capture) = paired_host(home, &[pairing(PROVED, PHONE)]);
    std::fs::write(
        home.join(".claude/settings.json"),
        "{\"model\": \"opus\", \"env\": {\"KEY\": \"secret\"}}\n",
    )
    .unwrap();
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let report =
        block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_800_000.0)))
            .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/config.toml"),
        format!(
            "model = \"gpt-5\"\n\n{}",
            moshpit_install::hooks::merged_toml("", "codex")
        ),
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".moshpit/seen")).unwrap();
    std::fs::write(home.join(".moshpit/seen/claude"), "").unwrap();
    (shell, capture)
}

/// B633: two phones on different builds paired with one host rewrote its
/// scripts, and the hooks calling them, back and forth at every session's
/// start. A newer build's scripts are now left as they are — the look reads
/// their version off the host, in the host's `sh` — and an older build's
/// (no version line) are brought up to this one's.
#[test]
fn a_newer_builds_scripts_are_left_and_an_older_ones_replaced() {
    use moshpit_install::installer::ComponentStatus;
    use moshpit_install::scripts;
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let now = Date::from_unix(1_762_800_000.0);
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.install_hooks(&claude(), &state, now)).unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());

    // Another phone's newer build puts its stamp script there.
    let stamp = home.0.join(".moshpit/moshpit-stamp.sh");
    let newer = format!(
        "#!/bin/sh\n# moshpit-scripts: {}\nexit 0",
        scripts::SCRIPTS_VERSION + 1
    );
    std::fs::write(&stamp, &newer).unwrap();
    let state = block_on(installer.inspect()).unwrap();
    let (state, _) = block_on(installer.verify(state, None)).unwrap();
    assert_eq!(
        state
            .manifest
            .get(&InstallComponent::Stamp)
            .unwrap()
            .scripts_version,
        Some(scripts::SCRIPTS_VERSION + 1)
    );
    assert_eq!(
        state.status(&InstallComponent::Stamp),
        ComponentStatus::Current
    );
    let report = block_on(installer.install_hooks(&claude(), &state, now)).unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(
        std::fs::read_to_string(&stamp).unwrap(),
        newer,
        "a newer build's script was replaced"
    );

    // An older build's, which says no version, is brought up to this one's.
    std::fs::write(&stamp, "#!/bin/sh\nexit 0").unwrap();
    let state = block_on(installer.inspect()).unwrap();
    let (state, _) = block_on(installer.verify(state, None)).unwrap();
    assert_ne!(
        state.status(&InstallComponent::Stamp),
        ComponentStatus::Current
    );
    let report = block_on(installer.install_hooks(&claude(), &state, now)).unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(std::fs::read_to_string(&stamp).unwrap(), scripts::stamp());
}

/// B375: nothing took Moshpit off a host — Unpair and Remove left the
/// scripts, the manifest, the backups and Moshpit's directories, and a
/// hook in a config the manifest didn't record. Remove everything does,
/// when this phone is the only one paired there; the person's configs keep
/// everything of theirs.
#[test]
fn remove_everything_leaves_nothing_of_moshpits_on_a_host_only_this_phone_used() {
    let home = home();
    let (shell, _) = fully_installed(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.remove_everything(PROVED, &state)).expect("removed");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(
        !home.0.join(".moshpit").exists(),
        "~/.moshpit is still there"
    );
    let claude = std::fs::read_to_string(home.0.join(".claude/settings.json")).unwrap();
    assert!(!claude.contains("moshpit-stamp.sh"), "{claude}");
    assert!(
        claude.contains("\"secret\"") && claude.contains("opus"),
        "{claude}"
    );
    let codex = std::fs::read_to_string(home.0.join(".codex/config.toml")).unwrap();
    assert_eq!(codex, "model = \"gpt-5\"\n");
    let left: Vec<String> = std::fs::read_dir(home.0.join(".claude"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["settings.json"], "a backup was left");
}

/// With another phone paired there too, that phone's conf, the scripts and
/// the manifest stay; this phone's pairing and every agent's hooks go.
#[test]
fn remove_everything_leaves_another_phones_pairing_and_what_it_needs() {
    let home = home();
    let (shell, _) = fully_installed(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.install_pairing(
        &pairing(SECOND, OTHER_PHONE),
        &state,
        Date::from_unix(1_762_800_000.0),
    ))
    .unwrap();
    assert!(report.succeeded());
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.remove_everything(PROVED, &state)).expect("removed");
    assert!(report.succeeded(), "{:?}", report.failures());
    let push_d = home.0.join(".moshpit/push.d");
    assert!(
        !push_d.join(format!("{PROVED}.conf")).exists(),
        "this phone's conf stayed"
    );
    assert!(
        push_d.join(format!("{SECOND}.conf")).exists(),
        "another phone's conf went"
    );
    assert!(home.0.join(".moshpit/moshpit-push.sh").exists());
    assert!(home.0.join(".moshpit/moshpit-stamp.sh").exists());
    let manifest = block_on(installer.inspect()).unwrap().manifest;
    assert!(manifest.pairing_entry(SECOND).is_some());
    assert!(manifest.pairing_entry(PROVED).is_none());
    assert!(moshpit_install::installer::installed_hook_agents(&manifest).is_empty());
    for config in [".claude/settings.json", ".codex/config.toml"] {
        let text = std::fs::read_to_string(home.0.join(config)).unwrap();
        assert!(!text.contains("moshpit-stamp.sh"), "{config}: {text}");
    }
}

/// `shell`'s commands under the umask most hosts give a login shell, 022,
/// whatever this test process's is.
struct Umask022<'a>(&'a Shell);

impl HostChannel for Umask022<'_> {
    fn run(&self, command: &str) -> impl Future<Output = Result<Vec<u8>, ChannelError>> {
        std::future::ready(block_on(self.0.run(&format!("umask 022; {command}"))))
    }
}

/// B637: Moshpit's directories were made with the host's umask, 0755 on
/// most hosts, so any local user could list `~/.moshpit/push.d` — which
/// connections this host pushes on, and which one each phone used last —
/// and read the manifest. The pairing files were 0600, under directories
/// everyone could read. Both directories are the person's alone now,
/// whichever install makes them.
#[test]
fn moshpits_directories_are_the_persons_alone() {
    let now = Date::from_unix(1_762_800_000.0);
    for pairing_first in [true, false] {
        let home = home();
        let shell = Shell::at(&home.0);
        let host = Umask022(&shell);
        let installer = HostInstaller::new(&host);
        let pair = || {
            let state = block_on(installer.inspect()).expect("inspect");
            let report = block_on(installer.install_pairing(&pairing(PROVED, PHONE), &state, now))
                .expect("pair");
            assert!(report.succeeded(), "{:?}", report.failures());
        };
        let hooks = || {
            let state = block_on(installer.inspect()).expect("inspect");
            let report = block_on(installer.install_hooks(&claude(), &state, now)).expect("hooks");
            assert!(report.succeeded(), "{:?}", report.failures());
        };
        if pairing_first {
            pair();
            hooks();
        } else {
            hooks();
            pair();
        }
        let moshpit = home.0.join(".moshpit");
        assert_eq!(
            mode(&moshpit),
            0o700,
            "~/.moshpit, pairing first: {pairing_first}"
        );
        assert_eq!(
            mode(&moshpit.join("push.d")),
            0o700,
            "push.d, pairing first: {pairing_first}"
        );
        assert_eq!(mode(&moshpit.join(format!("push.d/{PROVED}.conf"))), 0o600);
    }
}

/// An install from before keeps the directories it made open until the
/// phone next looks the host over, or changes anything on it.
#[test]
fn an_install_from_before_is_made_private_when_the_phone_next_looks() {
    let home = home();
    let (shell, _) = paired_host(&home.0, &[pairing(PROVED, PHONE)]);
    let moshpit = home.0.join(".moshpit");
    let open_up = || {
        for dir in [moshpit.clone(), moshpit.join("push.d")] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    };
    open_up();
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let _ = block_on(installer.verify(state, Some(PROVED))).unwrap();
    assert_eq!(mode(&moshpit), 0o700, "the look left ~/.moshpit open");
    assert_eq!(
        mode(&moshpit.join("push.d")),
        0o700,
        "the look left push.d open"
    );

    open_up();
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.install_pairing(
        &pairing(SECOND, OTHER_PHONE),
        &state,
        Date::from_unix(1_762_800_000.0),
    ))
    .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(mode(&moshpit), 0o700, "a change left ~/.moshpit open");
    assert_eq!(
        mode(&moshpit.join("push.d")),
        0o700,
        "a change left push.d open"
    );
}

/// The single pairing file the first builds wrote, for `conn`, 0600: as
/// they wrote it, with no phone token — nothing can send with it since the
/// relay keeps no registry (inert).
fn first_builds_pairing_file(home: &Path, conn: &str) -> PathBuf {
    pairing_file_at_the_old_place(home, conn, None)
}

/// A single `push.conf` for `conn`, carrying `phone`'s token when there is
/// one: one the sender can still push with (live).
fn pairing_file_at_the_old_place(home: &Path, conn: &str, phone: Option<&str>) -> PathBuf {
    let legacy = home.join(".moshpit/push.conf");
    let live = phone
        .map(|phone| format!("APNS_TOKEN={phone}\nAPNS_ENV=sandbox\nSEND_IAT=1762800000\n"))
        .unwrap_or_default();
    std::fs::write(
        &legacy,
        format!(
            "RELAY_URL=https://push.example.org\nSEND_TOKEN={}\nSECRET={}\nCONN={conn}\n{live}",
            "cd".repeat(32),
            "ab".repeat(32)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o600)).unwrap();
    legacy
}

/// B638: the single `~/.moshpit/push.conf` the first builds wrote holds a
/// pairing's secret and send token, and nothing took it off the host —
/// not Unpair, not Remove everything. While it was there, Remove
/// everything read it as another phone's pairing, and left the scripts,
/// the manifest and Moshpit's directories behind. It goes with the pairing
/// of the connection it names, however its id is written.
#[test]
fn the_first_builds_pairing_file_goes_with_its_connections_pairing() {
    let home = home();
    let (shell, _) = fully_installed(&home.0);
    let legacy = first_builds_pairing_file(&home.0, &PROVED.to_lowercase());
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.remove_everything(PROVED, &state)).expect("removed");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(!legacy.exists(), "push.conf stayed");
    assert!(
        !home.0.join(".moshpit").exists(),
        "~/.moshpit is still there"
    );
}

/// Unpair takes it too.
#[test]
fn unpairing_takes_the_first_builds_pairing_file_of_that_connection() {
    let home = home();
    let (shell, _) = paired_host(&home.0, &[pairing(PROVED, PHONE)]);
    let legacy = first_builds_pairing_file(&home.0, PROVED);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let mine = InstallComponent::Pairing {
        conn: PROVED.to_owned(),
        install: None,
    };
    block_on(installer.uninstall(&[mine], &state)).expect("unpaired");
    assert!(!legacy.exists(), "push.conf stayed");
}

/// Another connection's stays while it can still push — and so does what
/// it needs.
#[test]
fn another_connections_live_first_pairing_file_stays() {
    let home = home();
    let (shell, _) = fully_installed(&home.0);
    let legacy = pairing_file_at_the_old_place(&home.0, SECOND, Some(OTHER_PHONE));
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.remove_everything(PROVED, &state)).expect("removed");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(legacy.exists(), "another connection's live push.conf went");
    assert!(home.0.join(".moshpit/moshpit-push.sh").exists());
}

/// B638 (the lead's call): one that can't push — no phone token, as the
/// first builds wrote every one — is Moshpit's and of no use to anyone:
/// Remove everything takes it whichever connection it names, and with it
/// gone takes everything else too. Unpair alone leaves another
/// connection's.
#[test]
fn an_inert_first_pairing_file_goes_with_remove_everything_whoever_it_names() {
    let home = home();
    let (shell, _) = fully_installed(&home.0);
    let legacy = first_builds_pairing_file(&home.0, SECOND);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let mine = InstallComponent::Pairing {
        conn: PROVED.to_owned(),
        install: None,
    };
    block_on(installer.uninstall(std::slice::from_ref(&mine), &state)).expect("unpaired");
    assert!(
        legacy.exists(),
        "unpairing took another connection's push.conf"
    );
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.remove_everything(PROVED, &state)).expect("removed");
    assert!(report.succeeded(), "{:?}", report.failures());
    assert!(!legacy.exists(), "an inert push.conf stayed");
    assert!(
        !home.0.join(".moshpit").exists(),
        "~/.moshpit is still there"
    );
}

/// B375: installing and removing the hooks wrote the person's whole config
/// back as Foundation formats it — keys sorted, lines re-indented, numbers
/// rewritten. Through the real installer, a config written by hand comes
/// back byte for byte after an install and a removal, and the install
/// changed nothing but Moshpit's part; a second install changes nothing.
#[test]
fn a_hand_written_config_keeps_its_layout_through_install_and_removal() {
    let home = home();
    let config = home.0.join(".claude/settings.json");
    let original = "{\n    \"model\": \"opus\",\n    \"env\": { \"A\": \"1\" },\n    \"cleanupPeriodDays\": 30.0\n}\n";
    std::fs::write(&config, original).unwrap();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let state = block_on(installer.inspect()).unwrap();
    let report =
        block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_800_000.0)))
            .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    let installed = std::fs::read_to_string(&config).unwrap();
    assert!(
        installed.starts_with(
            "{\n    \"model\": \"opus\",\n    \"env\": { \"A\": \"1\" },\n    \"cleanupPeriodDays\": 30.0,\n    \"hooks\": {\n        \""
        ),
        "{installed}"
    );
    let state = block_on(installer.inspect()).unwrap();
    block_on(installer.install_hooks(&claude(), &state, Date::from_unix(1_762_900_000.0))).unwrap();
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        installed,
        "a refresh changed the file"
    );
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.uninstall_hooks(&claude(), &state)).unwrap();
    assert!(report.succeeded());
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
}

/// B376: the platform sends a command once more after a dropped connection.
/// The manifest lock, taken by the first run whose answer was lost, made the
/// second wait five seconds on itself and fail. The same holder's command
/// takes it again at once; another holder still waits.
#[test]
fn the_manifest_lock_taken_twice_by_one_holder_is_taken_at_once() {
    let home = home();
    let shell = Shell::at(&home.0);
    block_on(shell.run(&moshpit_install::commands::lock("00112233aabbccdd"))).expect("taken");
    let started = std::time::Instant::now();
    block_on(shell.run(&moshpit_install::commands::lock("00112233aabbccdd"))).expect("taken again");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "waited on itself"
    );
    let error =
        block_on(shell.run(&moshpit_install::commands::lock("99887766aabbccdd"))).unwrap_err();
    assert!(error.0.contains("another Moshpit"), "{error:?}");
    block_on(shell.run(&moshpit_install::commands::unlock("00112233aabbccdd"))).unwrap();
    assert!(!locked(&home.0.join(".moshpit/.lock")));
}

/// B376: the hooks test sent again after its first run had fired and lost
/// the connection before putting the stamps back: the second run used to
/// take the test's own stamps for the pane's and put those back. It puts
/// back what the pane said before the first run, and leaves nothing behind.
#[test]
fn the_hooks_test_sent_twice_puts_back_what_the_pane_said() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let shell = stamp_host(&home.0, &tmux);
    let pane = tmux.pane();
    let nonce = "0123456789abcdef";
    let socket = tmux.run(&["display-message", "-p", "#{socket_path}"]);
    // What the first run did before the connection dropped: it kept the
    // pane's stamps (a waiting agent's) and fired the test.
    let run = home.0.join(".moshpit/run");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(
        run.join(format!("selftest-{nonce}")),
        "attention\nclaude\n1762800000\nfix the build\nBash: make\n",
    )
    .unwrap();
    for (option, value) in [
        ("state", "working"),
        ("agent", "moshpit-selftest"),
        ("since", "1794620000"),
    ] {
        tmux.run(&[
            "set",
            "-p",
            "-t",
            &pane,
            &format!("@moshpit_{option}"),
            value,
        ]);
    }
    let read = block_on(shell.run(&moshpit_install::commands::stamp_test(
        &pane, &socket, nonce,
    )))
    .expect("the test runs again");
    assert!(String::from_utf8_lossy(&read).starts_with("working|moshpit-selftest"));
    assert_eq!(
        tmux.stamps(&pane),
        [
            "attention",
            "claude",
            "1762800000",
            "fix the build",
            "Bash: make"
        ]
    );
    assert!(
        !run.join(format!("selftest-{nonce}")).exists(),
        "the kept stamps were left"
    );
}

/// B375 × B385: Factory's `~/.factory/hooks.json` is an event map at its
/// top level, and goes through the same layout-keeping edit as any config.
/// A hooks file written by hand keeps its indentation and the person's
/// hook through an install; a refresh changes nothing; removal gives the
/// file back byte for byte — the events that held only Moshpit's hooks go
/// with them, and the event the person shares with us keeps theirs alone.
#[test]
fn a_hand_written_factory_hooks_file_keeps_its_layout_through_install_and_removal() {
    let home = home();
    let dir = home.0.join(".factory");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hooks.json");
    let original = "{\n  \"Stop\": [\n    { \"hooks\": [ { \"type\": \"command\", \"command\": \"say done\", \"timeout\": 5.0 } ] }\n  ]\n}\n";
    std::fs::write(&file, original).unwrap();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let factory = HookAgent::named("factory").expect("factory");
    let state = block_on(installer.inspect()).unwrap();
    let report =
        block_on(installer.install_hooks(&factory, &state, Date::from_unix(1_762_800_000.0)))
            .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    let installed = std::fs::read_to_string(&file).unwrap();
    assert!(
        installed.starts_with(
            "{\n  \"Stop\": [\n    { \"hooks\": [ { \"type\": \"command\", \"command\": \"say done\", \"timeout\": 5.0 } ] },\n"
        ),
        "the person's group moved or was rewritten: {installed}"
    );
    for event in ["UserPromptSubmit", "PreToolUse", "Notification"] {
        assert!(installed.contains(&format!("\n  \"{event}\": [")), "{event}: {installed}");
    }
    assert!(installed.contains("moshpit-stamp.sh done factory"), "{installed}");
    let state = block_on(installer.inspect()).unwrap();
    let report =
        block_on(installer.install_hooks(&factory, &state, Date::from_unix(1_762_900_000.0)))
            .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        installed,
        "a refresh changed the file"
    );
    let state = block_on(installer.inspect()).unwrap();
    let report = block_on(installer.uninstall_hooks(&factory, &state)).unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}

/// B434: herdr's panes inherit a $TMUX (and $TMUX_PANE) when its server
/// was started inside tmux. The stamp script took that for its own pane:
/// a herdr agent's stamps landed on whichever tmux pane the variable named,
/// on an island that has nothing to do with it. A herdr pane keeps its
/// stamps as outside tmux, under its own pane's directory, and still
/// pushes.
#[test]
fn a_herdr_pane_with_an_inherited_tmux_keeps_its_stamps_out_of_that_tmux() {
    let home = home();
    let Some(tmux) = Tmux::start(&home.0) else {
        return;
    };
    let pushed = untmuxed_host(&home.0);
    let pane = tmux.pane();
    for state in ["working", "done"] {
        let status = std::process::Command::new("sh")
            .arg(home.0.join(".moshpit/moshpit-stamp.sh"))
            .args([state, "claude"])
            .stdin(std::process::Stdio::null())
            .env_clear()
            .env("HOME", &home.0)
            .env("PATH", format!("{}:/usr/bin:/bin", tmux.bin.display()))
            .env("MOSHPIT_TEST_BIN", &tmux.bin)
            .env("TMUX", format!("{},1,0", tmux.socket.display()))
            .env("TMUX_PANE", &pane)
            .env("HERDR_PANE_ID", "w1:p2")
            .env("MOSHPIT_NOTIFY_GRACE", "0")
            .status()
            .unwrap();
        assert!(status.success());
    }
    assert_eq!(appears(&pushed).as_deref(), Some("done claude\n"));
    assert_eq!(tmux.stamps(&pane), ["-", "-", "-", "-", "-"], "stamped the inherited tmux's pane");
    let run = home.0.join(".moshpit/run/s-w1_p2");
    assert_eq!(std::fs::read_to_string(run.join("state")).unwrap(), "done");
}

/// B450: the question's push is handed the episode it is about — the
/// pane's stamped `since` — as its fifth argument, so the phone can tell
/// the pushed copy and its own copy of one question from a second question.
#[test]
fn the_question_s_push_is_handed_its_episode() {
    let home = home();
    let pushed = untmuxed_host(&home.0);
    std::fs::write(
        home.0.join(".moshpit/moshpit-push.sh"),
        format!("#!/bin/sh\necho \"$1 $5\" >> '{}'\n", pushed.display()),
    )
    .unwrap();
    stamp_untmuxed(&home.0, "working", "0", None);
    stamp_untmuxed(&home.0, "attention", "0", None);
    let line = appears(&pushed).expect("the question pushed");
    let run = std::fs::read_dir(home.0.join(".moshpit/run")).unwrap().next().unwrap().unwrap().path();
    let since = std::fs::read_to_string(run.join("since")).unwrap();
    assert_eq!(line.trim(), format!("attention {since}"));
}
