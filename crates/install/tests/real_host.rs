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
        let output = std::process::Command::new("sh")
            .args(["-c", command])
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", path)
            .output();
        std::future::ready(match output {
            Ok(out) if out.status.success() => Ok(out.stdout),
            Ok(out) => Err(ChannelError(
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )),
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

/// A real fire marks the agent as seen on the host (hs1's Codex trust
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
    assert!(!home.0.join(".moshpit/.lock").exists(), "the lock was left");
}

/// A lock someone holds now: the change waits, then gives up and says why,
/// touching nothing — the holder's lock included. A lock whose holder never
/// came back (older than a minute) is broken.
#[test]
fn a_held_lock_is_waited_for_and_a_dead_one_broken() {
    let home = home();
    let shell = Shell::at(&home.0);
    let installer = HostInstaller::new(&shell);
    let lock = home.0.join(".moshpit/.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(lock.join("at"), format!("{now}.1")).unwrap();
    let started = std::time::Instant::now();
    let error = block_on(installer.update_manifest(pairing_record(FIRST))).unwrap_err();
    assert!(error.0.contains("another Moshpit"), "{error:?}");
    assert!(started.elapsed() >= std::time::Duration::from_secs(4));
    assert!(
        !home.0.join(".moshpit/manifest.json").exists(),
        "written under another's lock"
    );
    assert_eq!(
        std::fs::read_to_string(lock.join("at")).unwrap(),
        format!("{now}.1"),
        "another's lock was let go"
    );

    std::fs::write(lock.join("at"), format!("{}.1", now - 120)).unwrap();
    block_on(installer.update_manifest(pairing_record(FIRST))).expect("a dead lock is broken");
    assert!(!lock.exists());
    let manifest = block_on(installer.inspect()).unwrap().manifest;
    assert!(manifest.pairing_entry(FIRST).is_some());
}

/// B448: the next holder of the lock takes it with `mkdir` and writes its
/// `at` a moment later. A waiter that had waited four tries and read the
/// lock in that moment, with no `at` yet, broke the fresh lock and took it
/// too: two writers at once, and one's change was lost. Here the next
/// holder's moment lasts a poll, at the waiter's fourth try: the waiter
/// takes the lock only once it is let go.
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

/// B448: a lock left with no `at` for good — a holder gone between its
/// `mkdir` and its `at` — is still broken, once four tries found it so.
#[test]
fn a_lock_left_with_no_at_is_broken_after_four_tries() {
    let home = home();
    let lock = home.0.join(".moshpit/.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let started = std::time::Instant::now();
    block_on(Shell::at(&home.0).run(&moshpit_install::commands::lock("waiter0123456789"))).expect("broken and taken");
    assert!(started.elapsed() >= std::time::Duration::from_secs(3), "broken too soon");
    assert!(std::fs::read_to_string(lock.join("at")).unwrap().ends_with(".waiter0123456789"));
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
    assert!(!home.0.join(".moshpit/.lock").exists());
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
