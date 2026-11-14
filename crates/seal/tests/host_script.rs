//! The real host script against the Rust port, live: `scripts/host/
//! moshpit-push.sh` runs under `sh` with whatever `openssl` this machine has
//! (LibreSSL on macOS, OpenSSL on Linux) and a stub `curl` that keeps the
//! request; Rust opens the envelope and checks the routing the relay would.
//! And back: an envelope Rust seals must open with the openssl CLI.
//!
//! Needs `sh` and `openssl`; set `MOSHPIT_NO_OPENSSL=1` to skip on a machine
//! without them. `harness/interop.py` runs the same against every shell and
//! openssl build it can find, and against Go and Swift.

use std::path::{Path, PathBuf};
use std::process::Command;

use moshpit_seal::foundation::json::{self, Dialect, Value};
use moshpit_seal::{Envelope, envelope, token};

const STUB_CURL: &str = r#"#!/bin/sh
body=""; config=""; w=""
while [ $# -gt 0 ]; do
  case "$1" in
    --data-binary) body="$2"; shift 2 ;;
    --config) config="$2"; shift 2 ;;
    -w) w="$2"; shift 2 ;;
    *) shift ;;
  esac
done
f=$(mktemp "$MOSHPIT_CAPTURE/req.XXXXXX")
printf '%s' "$body" > "$f"
cat "$config" > "$f.config"
[ -n "$w" ] && printf '200'
exit 0
"#;

const MASTER: &[u8] = b"moshpit-host-script-test-master-0123456789";
const SECRET: &str = "00112233445566778899AABBCCDDEEFF00112233445566778899aabbccddeeff";
const CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

fn openssl() -> Option<PathBuf> {
    let output = Command::new("sh")
        .args(["-c", "command -v openssl"])
        .output()
        .ok()?;
    let path = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn skip() -> bool {
    if openssl().is_some() {
        return false;
    }
    assert!(
        std::env::var_os("MOSHPIT_NO_OPENSSL").is_some(),
        "no openssl on PATH: install it, or set MOSHPIT_NO_OPENSSL=1 to skip the live host-script tests"
    );
    true
}

struct Run {
    dir: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A `tmux` that answers every question with `desk`: a session the
/// script must not name unless it is in that tmux.
const STUB_TMUX: &str = "#!/bin/sh\necho desk\n";

/// One run of the host script; returns the requests curl was given, each
/// with the bearer config it was handed.
fn run_script(args: &[&str], pane: Option<&str>) -> Vec<(Value, String)> {
    let env: Vec<(&str, &str)> = pane.map(|pane| vec![("TMUX_PANE", pane)]).unwrap_or_default();
    run_script_in(args, &env)
}

/// [`run_script`] with `env` set for the script.
fn run_script_in(args: &[&str], env: &[(&str, &str)]) -> Vec<(Value, String)> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/host/moshpit-push.sh");
    let run = Run {
        dir: std::env::temp_dir().join(format!("seal-host-{}", moshpit_seal::random::hex(8))),
    };
    let bin = run.dir.join("bin");
    let capture = run.dir.join("capture");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&capture).unwrap();
    let curl = bin.join("curl");
    std::fs::write(&curl, STUB_CURL).unwrap();
    let tmux = bin.join("tmux");
    std::fs::write(&tmux, STUB_TMUX).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for stub in [&curl, &tmux] {
            std::fs::set_permissions(stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let iat = 1_762_800_000;
    let apns = "AB".repeat(32);
    let send_token = token::mint(MASTER, &apns, CONN, iat);
    let conf = run.dir.join("push.conf");
    std::fs::write(
        &conf,
        format!(
            "RELAY_URL=https://push.example.org\nSEND_TOKEN={send_token}\nSECRET={SECRET}\nCONN={CONN}\n\
             APNS_TOKEN={apns}\nAPNS_ENV=sandbox\nSEND_IAT={iat}\n"
        ),
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new("sh");
    command
        .arg(&script)
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &run.dir)
        .env("MOSHPIT_CAPTURE", &capture)
        .env("MOSHPIT_PUSH_CONF", &conf)
        .env("MOSHPIT_PUSH_HOST", "m1-pro");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("sh runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut requests = Vec::new();
    for entry in std::fs::read_dir(&capture).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "config") {
            continue;
        }
        let body = json::parse(&std::fs::read(&path).unwrap(), Dialect::Decoder)
            .expect("the body is JSON");
        let mut config_path = path.clone().into_os_string();
        config_path.push(".config");
        let config = std::fs::read_to_string(config_path).expect("the bearer config");
        requests.push((body, config));
    }
    assert!(
        token::verify(
            MASTER,
            &send_token,
            &apns,
            CONN,
            iat,
            iat,
            token::DEFAULT_TTL_SECONDS
        ),
        "the conf's token is the relay's"
    );
    requests
}

fn field<'a>(body: &'a Value, key: &str) -> &'a Value {
    body.as_object()
        .and_then(|o| o.get(key))
        .unwrap_or_else(|| panic!("no {key}"))
}

#[test]
fn the_host_script_seals_what_rust_opens() {
    if skip() {
        return;
    }
    let title = "Bash: rm -rf \"build\" 构建\tand\nmore \\ 'quoted' $(id)";
    let requests = run_script(&["attention", "claude", title], Some("%7"));
    assert_eq!(requests.len(), 1);
    let (body, config) = &requests[0];
    let envelope: Envelope = json::decode_value(field(body, "env")).expect("an envelope");
    let status = envelope::open(&envelope, SECRET).expect("Rust opens what the host sealed");
    assert_eq!(status.conn, CONN);
    assert_eq!(status.host, "m1-pro");
    assert_eq!(status.sess.as_deref(), Some(""));
    assert_eq!(status.pane, "%7");
    assert_eq!(status.agent.as_deref(), Some("claude"));
    assert_eq!(status.state, "attention");
    assert_eq!(
        status.title.as_deref(),
        Some("Bash: rm -rf \"build\" 构建 and more \\ 'quoted' $(id)")
    );
    assert_eq!(status.dur, None);

    // The routing the relay checks, and the bearer kept out of argv.
    let send_token = token::mint(MASTER, &"AB".repeat(32), CONN, 1_762_800_000);
    assert_eq!(
        config,
        &format!("header = \"authorization: Bearer {send_token}\"\n")
    );
    assert_eq!(field(body, "cat").as_str(), Some("attention"));
    assert_eq!(
        field(body, "thread").as_str(),
        Some(format!("moshpit.attention.{CONN}").as_str())
    );
    assert_eq!(field(body, "tokEnv").as_str(), Some("sandbox"));
    let tok = field(body, "tok").as_str().unwrap();
    let iat = field(body, "iat").as_swift_int().unwrap();
    assert!(token::verify(
        MASTER,
        &send_token,
        tok,
        field(body, "conn").as_str().unwrap(),
        iat,
        iat,
        3600
    ));
}

#[test]
fn a_finished_turn_carries_its_duration_and_a_per_pane_thread() {
    if skip() {
        return;
    }
    let requests = run_script(&["done", "codex", "the build", "185"], None);
    let (body, _) = &requests[0];
    let envelope: Envelope = json::decode_value(field(body, "env")).unwrap();
    let status = envelope::open(&envelope, SECRET).unwrap();
    assert_eq!(status.state, "done");
    assert_eq!(status.dur, Some(185));
    assert_eq!(status.pane, "%0");
    assert_eq!(
        field(body, "thread").as_str(),
        Some(format!("moshpit.done.{CONN}.%0").as_str())
    );

    // A self-test is a done from the reserved agent, its nonce the title.
    let requests = run_script(&["--test", "selftest-abc123", CONN], None);
    let envelope: Envelope = json::decode_value(field(&requests[0].0, "env")).unwrap();
    let status = envelope::open(&envelope, SECRET).unwrap();
    assert_eq!(
        status.agent.as_deref(),
        Some(moshpit_seal::notification::SELF_TEST_AGENT)
    );
    assert_eq!(status.title.as_deref(), Some("selftest-abc123"));

    // `working` never pushes.
    assert!(run_script(&["working", "claude", "x"], None).is_empty());
}

#[test]
fn openssl_opens_what_rust_seals() {
    if skip() {
        return;
    }
    let openssl = openssl().unwrap();
    let plaintext = "{\"conn\":\"C\",\"host\":\"h\",\"pane\":\"%1\",\"state\":\"done\",\"title\":\"字 \\\"x\\\"\",\"ts\":1}";
    let sealed = envelope::seal(plaintext.as_bytes(), SECRET, None).unwrap();
    let keys = envelope::derive_keys(SECRET).unwrap();
    // The phone's half, as the host's shell would write it.
    let script = r#"
        TAG=$(printf 'v1|%s|%s' "$IV" "$CT" | "$OPENSSL" dgst -sha256 -hmac "$KM" -binary | "$OPENSSL" base64 -A)
        [ "$TAG" = "$MAC" ] || { echo "MAC mismatch" >&2; exit 1; }
        printf '%s' "$CT" | "$OPENSSL" enc -d -aes-256-cbc -K "$KE" -iv "$IV" -a -A
    "#;
    let output = Command::new("sh")
        .args(["-c", script])
        .env("OPENSSL", &openssl)
        .env("IV", &sealed.iv)
        .env("CT", &sealed.ct)
        .env("MAC", &sealed.mac)
        .env("KE", &keys.enc_hex)
        .env("KM", &keys.mac_hex)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), plaintext);

    // And the subkeys are openssl's own.
    let derive = |info: &str| {
        let out = Command::new(&openssl)
            .args(["dgst", "-sha256", "-hmac", &SECRET.to_lowercase()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(info.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .last()
            .unwrap()
            .to_owned()
    };
    assert_eq!(derive("moshpit-push-enc-v1"), keys.enc_hex);
    assert_eq!(derive("moshpit-push-mac-v1"), keys.mac_hex);
}

/// A stub `curl` that holds its first request until `$MOSHPIT_CAPTURE/go`
/// exists (so the test can signal the script mid-send), answers every
/// other at once, and logs each call with the mode of the bearer file it
/// was handed.
const BLOCKING_CURL: &str = r#"#!/bin/sh
config=""
while [ $# -gt 0 ]; do
  case "$1" in --config) config="$2"; shift 2 ;; *) shift ;; esac
done
mode=$(ls -l "$config" 2>/dev/null | cut -c1-10)
printf '%s\n' "$mode" >> "$MOSHPIT_CAPTURE/calls"
if [ ! -e "$MOSHPIT_CAPTURE/held" ]; then
  : > "$MOSHPIT_CAPTURE/held"
  while [ ! -e "$MOSHPIT_CAPTURE/go" ]; do sleep 0.05; done
fi
printf '200'
exit 0
"#;

/// Runs the host script over two paired phones, sends `signal` to it while
/// the first phone's request is in flight, and returns the calls curl
/// logged (one line each: the bearer file's mode) and what was left in the
/// script's TMPDIR.
fn signalled(signal: &str) -> (Vec<String>, Vec<String>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/host/moshpit-push.sh");
    let run = Run {
        dir: std::env::temp_dir().join(format!("seal-sig-{}", moshpit_seal::random::hex(8))),
    };
    let (bin, capture, tmp) = (
        run.dir.join("bin"),
        run.dir.join("capture"),
        run.dir.join("tmp"),
    );
    let confs = run.dir.join(".moshpit/push.d");
    for dir in [&bin, &capture, &tmp, &confs] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let curl = bin.join("curl");
    std::fs::write(&curl, BLOCKING_CURL).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let iat = 1_762_800_000;
    for (i, apns) in ["AB".repeat(32), "CD".repeat(32)].into_iter().enumerate() {
        let conn = format!("3F2504E0-4F89-11D3-9A0C-0305E82C330{i}");
        let send_token = token::mint(MASTER, &apns, &conn, iat);
        std::fs::write(
            confs.join(format!("{conn}.conf")),
            format!(
                "RELAY_URL=https://push.example.org\nSEND_TOKEN={send_token}\nSECRET={SECRET}\nCONN={conn}\n\
                 APNS_TOKEN={apns}\nAPNS_ENV=sandbox\nSEND_IAT={iat}\n"
            ),
        )
        .unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new("sh")
        .arg(&script)
        .args(["attention", "claude", "may I?"])
        .env_clear()
        .env("PATH", path)
        .env("HOME", &run.dir)
        .env("TMPDIR", &tmp)
        .env("MOSHPIT_CAPTURE", &capture)
        .env("MOSHPIT_PUSH_HOST", "m1-pro")
        .spawn()
        .expect("sh runs");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !capture.join("held").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "curl was never called"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let sent = Command::new("kill")
        .args([&format!("-{signal}"), &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    // Signalled while its request was in flight: let that request answer.
    std::thread::sleep(std::time::Duration::from_millis(200));
    std::fs::write(capture.join("go"), "").unwrap();
    let _ = child.wait();
    let calls = std::fs::read_to_string(capture.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let left = std::fs::read_dir(&tmp)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    (calls, left)
}

/// B350: told to stop (TERM, INT) mid-send, the script only removed its
/// bearer file and went on to the next phone, writing that phone's send
/// token into a file it recreated with the default mode (0644 under the
/// usual umask) — readable by every local user. Told to hang up (the pane
/// or session closing under a grace fork), it died with the token file
/// left in /tmp. It stops now, and leaves nothing.
#[test]
fn told_to_stop_mid_send_it_stops_and_leaves_no_token_behind() {
    if skip() {
        return;
    }
    for signal in ["TERM", "INT", "HUP"] {
        let (calls, left) = signalled(signal);
        assert_eq!(
            calls.len(),
            1,
            "{signal}: it went on to the next phone: {calls:?}"
        );
        assert!(
            calls.iter().all(|mode| mode == "-rw-------"),
            "{signal}: a bearer file others can read: {calls:?}"
        );
        assert!(left.is_empty(), "{signal}: left in TMPDIR: {left:?}");
    }
}

/// Runs the sender in hook mode over `confs` (file name, contents) in a
/// private HOME with the stub curl; the requests it sent and the files left
/// in HOME's root.
fn over_confs(confs: &[(&str, String)]) -> (Vec<(Value, String)>, Vec<String>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/host/moshpit-push.sh");
    let run = Run {
        dir: std::env::temp_dir().join(format!("seal-conf-{}", moshpit_seal::random::hex(8))),
    };
    let (bin, capture) = (run.dir.join("bin"), run.dir.join("capture"));
    let push_d = run.dir.join(".moshpit/push.d");
    for dir in [&bin, &capture, &push_d] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let curl = bin.join("curl");
    std::fs::write(&curl, STUB_CURL).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (name, body) in confs {
        std::fs::write(push_d.join(name), body).unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new("sh")
        .arg(&script)
        .args(["done", "claude", "finished"])
        .current_dir(&run.dir)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &run.dir)
        .env("MOSHPIT_CAPTURE", &capture)
        .env("MOSHPIT_PUSH_HOST", "m1-pro")
        .output()
        .expect("sh runs");
    assert!(output.status.success());
    let mut requests = Vec::new();
    for entry in std::fs::read_dir(&capture).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with(".config") {
            continue;
        }
        let body = std::fs::read_to_string(&path).unwrap();
        let config = std::fs::read_to_string(format!("{}.config", path.display())).unwrap();
        requests.push((
            json::parse(body.as_bytes(), Dialect::Decoder).expect("a JSON body"),
            config,
        ));
    }
    let mut left: Vec<String> = std::fs::read_dir(&run.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !matches!(name.as_str(), "bin" | "capture" | ".moshpit"))
        .collect();
    left.sort();
    (requests, left)
}

fn conf_with_token(conn: &str, apns: &str, send_token: &str) -> String {
    format!(
        "RELAY_URL=https://push.example.org\nSEND_TOKEN={send_token}\nSECRET={SECRET}\nCONN={conn}\n\
         APNS_TOKEN={apns}\nAPNS_ENV=sandbox\nSEND_IAT=1762800000\n"
    )
}

/// B357: the sender sourced each conf, so whatever a value held ran as
/// shell on the host at every push — and the send token is the relay's own
/// answer. A conf is read as text now; a token that isn't hex (which could
/// add options to curl's config) skips that pairing, and the next one is
/// still sent.
#[test]
fn a_conf_is_read_never_run() {
    if skip() {
        return;
    }
    let good = "3F2504E0-4F89-11D3-9A0C-0305E82C3309";
    let apns = "AB".repeat(32);
    let other = "CD".repeat(32);
    let token = token::mint(MASTER, &other, good, 1_762_800_000);
    let mut confs = vec![(
        format!("{good}.conf"),
        conf_with_token(good, &other, &token),
    )];
    for (i, evil) in [
        format!("{};touch PWNED-semicolon", &token[..8]),
        format!("$(touch PWNED-substitution){}", &token[..8]),
        format!("`touch PWNED-backtick`{}", &token[..8]),
        format!("{}\ntouch PWNED-newline", &token[..8]),
        format!("{}\" output = \"PWNED-curl", &token[..8]),
    ]
    .into_iter()
    .enumerate()
    {
        let conn = format!("3F2504E0-4F89-11D3-9A0C-0305E82C330{i}");
        confs.push((format!("{conn}.conf"), conf_with_token(&conn, &apns, &evil)));
    }
    let named: Vec<(&str, String)> = confs.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (requests, left) = over_confs(&named);
    assert!(left.is_empty(), "a conf ran as shell: {left:?}");
    // The good pairing is sent with its token. Of the rest only the one
    // whose SEND_TOKEN line is hex on its own is tried (the line after it
    // is text, not a command; the relay refuses the short token); the
    // others' tokens aren't hex, and nothing reaches curl's config but a
    // bearer header.
    let mut sent: Vec<(String, String)> = requests
        .iter()
        .map(|(body, config)| {
            (
                field(body, "conn").as_str().unwrap().to_owned(),
                config.clone(),
            )
        })
        .collect();
    sent.sort();
    assert_eq!(
        sent,
        [
            (
                "3F2504E0-4F89-11D3-9A0C-0305E82C3303".to_owned(),
                format!("header = \"authorization: Bearer {}\"\n", &token[..8])
            ),
            (
                good.to_owned(),
                format!("header = \"authorization: Bearer {token}\"\n")
            ),
        ]
    );
}

/// B434: every push from outside tmux named the pane `%0`, which no herdr
/// pane is. On the phone its waiting entry never matched a pane a poll
/// names, so its card wasn't taken down once answered and the next
/// question inside fifteen minutes came in silent; every herdr agent on
/// the host shared one entry and one finish card. A herdr pane's push
/// names it by herdr's own id — also inside a $TMUX herdr's server
/// inherited, whose session it doesn't ask for — and an id no collapse id
/// could carry falls back to `%0`.
#[test]
fn a_herdr_panes_push_names_it_by_herdrs_own_id() {
    if skip() {
        return;
    }
    let opened = |body: &Value| -> moshpit_seal::Status {
        let envelope: Envelope = json::decode_value(field(body, "env")).expect("an envelope");
        envelope::open(&envelope, SECRET).expect("Rust opens what the host sealed")
    };
    let requests = run_script_in(
        &["done", "claude", "fix the build", "240"],
        &[("HERDR_PANE_ID", "w1:p2")],
    );
    assert_eq!(requests.len(), 1);
    assert_eq!(opened(&requests[0].0).pane, "w1:p2");
    assert_eq!(
        field(&requests[0].0, "thread").as_str(),
        Some(format!("moshpit.done.{CONN}.w1:p2").as_str())
    );

    let inherited = run_script_in(
        &["attention", "claude", "Bash: make"],
        &[
            ("HERDR_PANE_ID", "w1:p2"),
            ("TMUX", "/tmp/desk.sock,1,0"),
            ("TMUX_PANE", "%5"),
        ],
    );
    let status = opened(&inherited[0].0);
    assert_eq!(status.pane, "w1:p2");
    assert_eq!(status.sess.as_deref(), Some(""), "the inherited tmux was asked");

    let odd = run_script_in(&["done", "claude"], &[("HERDR_PANE_ID", "w1 p2")]);
    assert_eq!(opened(&odd[0].0).pane, "%0");
}

/// B450: an attention push carries when its question was asked (the
/// stamp's episode, the pane's `@moshpit_since`) as `ts`, the question's
/// identity on the phone; without one, and for a finish, the send time.
#[test]
fn an_attention_push_is_dated_by_its_question() {
    if skip() {
        return;
    }
    let opened = |body: &Value| -> moshpit_seal::Status {
        let envelope: Envelope = json::decode_value(field(body, "env")).expect("an envelope");
        envelope::open(&envelope, SECRET).expect("Rust opens what the host sealed")
    };
    let asked = run_script(&["attention", "claude", "Bash: make", "", "1790000000"], Some("%3"));
    assert_eq!(opened(&asked[0].0).ts, 1_790_000_000);
    let undated = run_script(&["attention", "claude", "Bash: make"], Some("%3"));
    assert!(opened(&undated[0].0).ts > 1_790_000_000, "the send time");
    let done = run_script(&["done", "claude", "fix", "600", "1790000000"], Some("%3"));
    assert!(opened(&done[0].0).ts > 1_790_000_000, "a finish keeps its send time");
}
