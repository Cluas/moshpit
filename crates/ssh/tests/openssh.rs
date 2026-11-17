//! The transport against real OpenSSH. Ignored by default; two benches,
//! each used only when it is set up:
//!
//! - The machine's own sshd, with an account and key that already work
//!   (`ssh localhost`): commands, terminals, `exit` against a real drop,
//!   SFTP in a scratch folder under /tmp, and the host key against
//!   `/etc/ssh/ssh_host_ed25519_key.pub`. Nothing outside that folder is
//!   written.
//!
//!   ```sh
//!   MOSHPIT_SSH_TEST_TARGET=me@localhost MOSHPIT_SSH_TEST_KEY=~/.ssh/id_ed25519 \
//!     cargo test -p moshpit-ssh --test openssh -- --ignored --nocapture
//!   ```
//!
//! - A private sshd these tests start as the current user, on a free
//!   loopback port, with its own host keys, `authorized_keys` and pid file in
//!   a temp directory (the machine's sshd and `~/.ssh` are left alone):
//!   every client key type, every host key type, the SHA-1 `ssh-rsa`
//!   signature Citadel sends being refused, and one private sshd reached
//!   through another (`ssh -J`).
//!
//!   ```sh
//!   MOSHPIT_SSH_TEST_SSHD=/usr/sbin/sshd \
//!     cargo test -p moshpit-ssh --test openssh -- --ignored --nocapture
//!   ```

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use moshpit_ssh::{
    Auth, ConnectionEnd, Event, Exit, Jump, KnownHosts, PtyRequest, SftpError, Sig, Ssh, Target,
    TrustOnFirstUse,
};
use support::*;

fn expand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap()),
        None => path.to_owned(),
    }
}

/// The machine's sshd, when `MOSHPIT_SSH_TEST_TARGET` and
/// `MOSHPIT_SSH_TEST_KEY` say how to reach it.
fn machine() -> Option<(Target, String)> {
    let target = Target::parse(&std::env::var("MOSHPIT_SSH_TEST_TARGET").ok()?)?;
    let key = std::fs::read_to_string(expand(&std::env::var("MOSHPIT_SSH_TEST_KEY").ok()?)).ok()?;
    Some((target, key))
}

macro_rules! machine_or_skip {
    () => {
        match machine() {
            Some(found) => found,
            None => {
                eprintln!("skipped: set MOSHPIT_SSH_TEST_TARGET and MOSHPIT_SSH_TEST_KEY");
                return;
            }
        }
    };
}

async fn on_machine() -> Ssh {
    let (target, key) = machine().unwrap();
    Ssh::connect(&target, Auth::key(key), Arc::new(AcceptAny), &options())
        .await
        .unwrap()
}

/// A scratch folder on the host, removed afterwards.
struct RemoteScratch {
    ssh: Ssh,
    path: String,
}

impl RemoteScratch {
    async fn new(ssh: &Ssh) -> RemoteScratch {
        let out = ssh
            .run("mktemp -d /tmp/moshpit-ssh-test.XXXXXX")
            .await
            .unwrap();
        let path = String::from_utf8(out.stdout()).unwrap().trim().to_owned();
        assert!(path.starts_with("/tmp/moshpit-ssh-test."), "{path}");
        RemoteScratch {
            ssh: ssh.clone(),
            path,
        }
    }

    fn at(&self, name: &str) -> String {
        format!("{}/{name}", self.path)
    }

    async fn remove(self) {
        let out = self
            .ssh
            .run(&format!("rm -rf '{}'", self.path))
            .await
            .unwrap();
        assert!(out.success());
    }
}

#[tokio::test]
#[ignore = "needs an OpenSSH server; see the module docs"]
async fn openssh_runs_commands_with_their_status() {
    let _ = machine_or_skip!();
    let ssh = on_machine().await;
    let out = ssh.run("uname -s && id -un").await.unwrap();
    eprintln!("stdout: {:?}", String::from_utf8_lossy(&out.stdout()));
    assert!(out.success());
    assert_eq!(out.stdout().iter().filter(|b| **b == b'\n').count(), 2);

    let out = ssh.run("echo oops 1>&2; exit 3").await.unwrap();
    assert_eq!(out.stderr(), b"oops\n");
    assert_eq!(out.status(), Some(3));
    assert!(out.closed);

    let out = ssh
        .run("printf a; printf b 1>&2; sleep 0.1; printf c")
        .await
        .unwrap();
    assert_eq!(out.merged(), b"abc");
}

#[tokio::test]
#[ignore = "needs an OpenSSH server; see the module docs"]
async fn openssh_takes_stdin_and_signals() {
    let _ = machine_or_skip!();
    let ssh = on_machine().await;
    let mut cat = ssh.exec("cat").await.unwrap();
    cat.write(b"ping\n").await.unwrap();
    assert!(
        within(10, read_until(&mut cat, "ping\n"))
            .await
            .contains("ping")
    );
    cat.close_stdin().await.unwrap();
    let (events, _) = within(10, drain(&mut cat)).await;
    assert!(events.contains(&Event::Exited(Exit::Status(0))));
    assert_eq!(events.last(), Some(&Event::Closed));

    let mut sleeper = ssh.exec("exec sleep 30").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    sleeper.signal(Sig::TERM).await.unwrap();
    let (events, _) = within(10, drain(&mut sleeper)).await;
    eprintln!("after TERM: {events:?}");
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Exited(Exit::Signal { signal, .. }) if signal == "TERM"
    )));
}

#[tokio::test]
#[ignore = "needs an OpenSSH server; see the module docs"]
async fn openssh_terminal_follows_resizes_and_speaks_utf8() {
    let _ = machine_or_skip!();
    let ssh = on_machine().await;
    let mut request = PtyRequest::new(80, 24);
    request.command = Some("echo LANG=$LANG; stty size; sleep 0.5; stty size".into());
    let mut term = ssh.terminal(&request).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    term.resize(132, 40).await.unwrap();
    let (events, stdout) = within(10, drain(&mut term)).await;
    let text = String::from_utf8_lossy(&stdout).replace("\r\n", "\n");
    eprintln!("terminal said: {text:?}");
    assert!(text.contains("LANG=en_US.UTF-8\n"), "AcceptEnv LANG");
    assert!(text.contains("24 80\n"), "the size asked for first");
    assert!(text.contains("40 132"), "the size after the resize");
    assert_eq!(events.last(), Some(&Event::Closed));
}

/// B103 on a real host: `exit` in a login shell closes the terminal with a
/// status and leaves the connection up; killing the connection's sshd
/// process is a drop.
#[tokio::test]
#[ignore = "needs an OpenSSH server; see the module docs"]
async fn openssh_exit_is_not_a_drop_and_a_drop_is() {
    let _ = machine_or_skip!();
    let ssh = on_machine().await;
    let mut shell = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    shell.write(b"exit\r").await.unwrap();
    let (events, _) = within(10, drain(&mut shell)).await;
    let tail: Vec<&Event> = events
        .iter()
        .filter(|e| !matches!(e, Event::Stdout(_) | Event::Stderr(_)))
        .collect();
    eprintln!("after exit: {tail:?}");
    assert!(tail.contains(&&Event::Exited(Exit::Status(0))));
    assert_eq!(tail.last(), Some(&&Event::Closed));
    assert_eq!(ssh.end(), None);
    assert!(ssh.run("true").await.unwrap().success());

    // The process serving this connection, killed: no goodbye.
    let mut shell = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    let _ = ssh.exec("kill -9 $PPID").await;
    let (events, _) = within(10, drain(&mut shell)).await;
    assert_eq!(events.last(), Some(&Event::Lost));
    let end = within(10, ssh.ended()).await;
    eprintln!("after the kill: {end:?}");
    assert!(end.is_lost(), "{end:?}");
}

#[tokio::test]
#[ignore = "needs an OpenSSH server; see the module docs"]
async fn openssh_host_key_is_what_ssh_keygen_prints_and_tofu_keeps_it() {
    let (target, key) = machine_or_skip!();
    let known = Arc::new(KnownHosts::in_memory(BTreeMap::new()));
    let person = Person::answering(&[true]);
    let policy = Arc::new(TrustOnFirstUse::new(known.clone(), person.clone()));
    let ssh = Ssh::connect(&target, Auth::key(key.clone()), policy, &options())
        .await
        .unwrap();
    eprintln!(
        "{}:{} presents {} {}",
        target.host,
        target.port,
        ssh.host_key().algorithm(),
        ssh.host_key().fingerprint()
    );
    // NIOSSH's order puts Ed25519 first; OpenSSH hosts have one.
    assert_eq!(ssh.host_key().algorithm(), "ssh-ed25519");
    if target.host == "localhost" || target.host == "127.0.0.1" {
        let printed = keygen(Path::new("/etc/ssh/ssh_host_ed25519_key.pub"));
        assert_eq!(ssh.host_key().fingerprint(), printed);
    }
    assert_eq!(person.asked().len(), 1);
    assert_eq!(
        known.snapshot(),
        BTreeMap::from([(
            format!("{}:{}", target.host, target.port),
            ssh.host_key().fingerprint()
        )])
    );
    let nobody = Person::answering(&[]);
    let policy = Arc::new(TrustOnFirstUse::new(known.clone(), nobody.clone()));
    Ssh::connect(&target, Auth::key(key), policy, &options())
        .await
        .unwrap();
    assert!(nobody.asked().is_empty());
}

/// `ssh-keygen -lf`'s fingerprint of a public key file.
fn keygen(path: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .arg("-lf")
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

#[tokio::test]
#[ignore = "needs an OpenSSH server; see the module docs"]
async fn openssh_sftp_does_what_the_preview_and_the_sheet_need() {
    let _ = machine_or_skip!();
    let ssh = on_machine().await;
    let scratch = RemoteScratch::new(&ssh).await;
    let sftp = ssh.sftp().await.unwrap();

    let home = String::from_utf8(ssh.run("printf %s \"$HOME\"").await.unwrap().stdout()).unwrap();
    assert_eq!(sftp.home().await.unwrap(), home);

    let local = Scratch::new("openssh-sftp");
    let bytes: Vec<u8> = (0..1_500_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(local.join("clip.bin"), &bytes).unwrap();
    sftp.upload_new(
        &local.join("clip.bin"),
        &scratch.at("clip.bin"),
        &mut |_| true,
    )
    .await
    .unwrap();
    // OpenSSH's own exclusive create: a taken name.
    assert!(matches!(
        sftp.upload_new(
            &local.join("clip.bin"),
            &scratch.at("clip.bin"),
            &mut |_| true
        )
        .await,
        Err(SftpError::NameTaken(_))
    ));

    let entry = sftp.stat(&scratch.at("clip.bin")).await.unwrap();
    assert_eq!(entry.size, Some(bytes.len() as u64));
    assert!(!entry.is_directory);

    let file = sftp.open(&scratch.at("clip.bin")).await.unwrap();
    assert_eq!(file.size(), bytes.len() as u64);
    assert_eq!(
        file.read(1_000_000, 4096).await.unwrap(),
        bytes[1_000_000..1_004_096]
    );
    assert!(file.read(bytes.len() as u64, 10).await.unwrap().is_empty());
    file.close().await;

    let mut fractions = 0;
    let copied = sftp
        .download(
            &scratch.at("clip.bin"),
            &local.join("back.bin"),
            &mut |_| {
                fractions += 1;
                true
            },
        )
        .await
        .unwrap();
    assert_eq!(copied, bytes.len() as u64);
    assert!(std::fs::read(local.join("back.bin")).unwrap() == bytes);
    assert!(fractions > 1);

    sftp.create_dir(&scratch.at("folder")).await.unwrap();
    let mut listing: Vec<(String, bool)> = sftp
        .list(&scratch.path)
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.path, e.is_directory))
        .collect();
    listing.sort();
    assert_eq!(
        listing,
        vec![
            (scratch.at("clip.bin"), false),
            (scratch.at("folder"), true)
        ]
    );

    assert!(matches!(
        sftp.rename(&scratch.at("folder"), &scratch.at("clip.bin"))
            .await,
        Err(SftpError::NameTaken(_))
    ));
    sftp.rename(&scratch.at("clip.bin"), &scratch.at("folder/clip.bin"))
        .await
        .unwrap();
    sftp.remove_file(&scratch.at("folder/clip.bin"))
        .await
        .unwrap();
    assert!(matches!(
        sftp.stat(&scratch.at("folder/clip.bin")).await,
        Err(SftpError::NotFound(_))
    ));
    scratch.remove().await;
}

// ---------------------------------------------------------------------------
// A private sshd.

/// An sshd of our own, killed when dropped.
struct PrivateSshd {
    port: u16,
    dir: Scratch,
    child: Child,
}

impl Drop for PrivateSshd {
    fn drop(&mut self) {
        reap_sshd_tree(&mut self.child, &self.dir.join("sshd.log"));
    }
}

/// Kill the sshd listener process group first, then sweep any detached session
/// children (which call `setsid` and leave the group) still holding `sshd.log`
/// open before `Scratch::drop` deletes the directory (B870).
fn reap_sshd_tree(child: &mut Child, log_path: &Path) {
    let listener = child.id();
    let _ = child.kill();
    let group = format!("-{listener}");
    for signal in ["-TERM", "-KILL"] {
        let _ = Command::new("kill")
            .args([signal, "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();
    }
    for _ in 0..2 {
        let holders = pids_holding_file(log_path);
        if holders.is_empty() {
            break;
        }
        for signal in ["-TERM", "-KILL"] {
            for pid in &holders {
                let _ = Command::new("kill")
                    .args([signal, &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .output();
            }
            if signal == "-TERM" {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let _ = child.wait();
}

impl PrivateSshd {
    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("sshd.log")).unwrap_or_default()
    }

    fn target(&self) -> Target {
        Target::new(whoami(), "127.0.0.1", self.port)
    }
}

fn whoami() -> String {
    let out = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn sshd_binary() -> Option<PathBuf> {
    std::env::var("MOSHPIT_SSH_TEST_SSHD")
        .ok()
        .map(PathBuf::from)
}

macro_rules! sshd_or_skip {
    () => {
        match sshd_binary() {
            Some(found) => found,
            None => {
                eprintln!("skipped: set MOSHPIT_SSH_TEST_SSHD to an sshd binary");
                return;
            }
        }
    };
}

/// Starts an sshd with the fixture keys `host_keys` as its host keys and
/// every fixture public key authorized.
async fn private_sshd(binary: &Path, host_keys: &[&str]) -> PrivateSshd {
    private_sshd_authorizing(binary, host_keys, "").await
}

/// [`private_sshd`], with the `authorized_keys` lines `more` authorized too.
async fn private_sshd_authorizing(binary: &Path, host_keys: &[&str], more: &str) -> PrivateSshd {
    let dir = Scratch::new("sshd");
    let mut config = String::new();
    for name in host_keys {
        let path = dir.join(&format!("host_{name}"));
        std::fs::write(&path, fixture_key(name)).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        config.push_str(&format!("HostKey {}\n", path.display()));
    }
    let mut authorized = String::new();
    for name in [
        "ed25519", "rsa2048", "rsa4096", "ecdsa256", "ecdsa384", "ecdsa521",
    ] {
        authorized.push_str(&fixture_key(&format!("{name}.pub")));
    }
    authorized.push_str(more);
    std::fs::write(dir.join("authorized_keys"), authorized).unwrap();
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let sftp = [
        "/usr/libexec/sftp-server",
        "/usr/lib/openssh/sftp-server",
        "/usr/lib/ssh/sftp-server",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
    .unwrap_or("internal-sftp");
    config.push_str(&format!(
        "Port {port}\nListenAddress 127.0.0.1\nPidFile {pid}\nAuthorizedKeysFile {keys}\n\
         StrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n\
         Subsystem sftp {sftp}\nLogLevel VERBOSE\n",
        pid = dir.join("sshd.pid").display(),
        keys = dir.join("authorized_keys").display(),
    ));
    use std::os::unix::process::CommandExt;
    std::fs::write(dir.join("sshd_config"), config).unwrap();
    let child = Command::new(binary)
        .args(["-D", "-f"])
        .arg(dir.join("sshd_config"))
        .arg("-E")
        .arg(dir.join("sshd.log"))
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let sshd = PrivateSshd { port, dir, child };
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return sshd;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("sshd didn't start: {}", sshd.log());
}

/// Every key type the app can hold signs in to real OpenSSH — RSA with a
/// SHA-2 signature, ECDSA on all three curves.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_takes_every_client_key_type() {
    let binary = sshd_or_skip!();
    let sshd = private_sshd(&binary, &["ed25519"]).await;
    for name in [
        "ed25519", "rsa2048", "rsa4096", "ecdsa256", "ecdsa384", "ecdsa521",
    ] {
        let ssh = Ssh::connect(
            &sshd.target(),
            Auth::key(fixture_key(name)),
            Arc::new(AcceptAny),
            &options(),
        )
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}\n{}", sshd.log()));
        assert!(ssh.run("true").await.unwrap().success(), "{name}");
        ssh.close().await;
        assert_eq!(ssh.end(), Some(ConnectionEnd::ClosedByUs));
    }
    let log = sshd.log();
    for fingerprint in ["ed25519", "rsa2048", "ecdsa521"].map(keygen_fingerprint) {
        assert!(
            log.contains(&format!("Accepted publickey for {}", whoami())),
            "{log}"
        );
        assert!(log.contains(&fingerprint), "{fingerprint} not in {log}");
    }
}

/// A private sshd reached through another (F1): real OpenSSH opens the
/// `direct-tcpip` channel, the target's session runs in it — a command and
/// SFTP — and closing the target says goodbye on both.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_through_another() {
    let binary = sshd_or_skip!();
    let bastion = private_sshd(&binary, &["ed25519"]).await;
    let target = private_sshd(&binary, &["ecdsa256"]).await;
    let jump = Ssh::connect(
        &bastion.target(),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}\n{}", bastion.log()));
    let ssh = Ssh::connect_through(
        Jump::new(jump.clone()),
        &target.target(),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        None,
        &options(),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}\n{}\n{}", bastion.log(), target.log()));
    let out = ssh.run("echo behind").await.unwrap();
    assert_eq!(out.stdout(), b"behind\n");
    assert_eq!(ssh.host_key().algorithm(), "ecdsa-sha2-nistp256");
    let sftp = ssh.sftp().await.unwrap();
    assert!(sftp.list("/").await.unwrap().iter().any(|e| e.is_directory));
    ssh.close().await;
    assert_eq!(ssh.end(), Some(ConnectionEnd::ClosedByUs));
    assert_eq!(
        within(5, jump.ended()).await,
        ConnectionEnd::ClosedByUs,
        "{}",
        bastion.log()
    );
}

/// Each host key type, presented alone, fingerprints as ssh-keygen prints
/// it; presented together, the host shows the key NIOSSH's order asks for.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_host_keys_fingerprint_and_order_as_the_app_saw_them() {
    let binary = sshd_or_skip!();
    for (name, algorithm) in [
        ("ed25519", "ssh-ed25519"),
        ("ecdsa256", "ecdsa-sha2-nistp256"),
        ("ecdsa384", "ecdsa-sha2-nistp384"),
        ("ecdsa521", "ecdsa-sha2-nistp521"),
        ("rsa2048", "ssh-rsa"),
    ] {
        let sshd = private_sshd(&binary, &[name]).await;
        let ssh = Ssh::connect(
            &sshd.target(),
            Auth::key(fixture_key("ed25519")),
            Arc::new(AcceptAny),
            &options(),
        )
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}\n{}", sshd.log()));
        assert_eq!(ssh.host_key().algorithm(), algorithm, "{name}");
        assert_eq!(
            ssh.host_key().fingerprint(),
            keygen_fingerprint(name),
            "{name}"
        );
    }
    for (keys, shown) in [
        (
            &["rsa2048", "ecdsa256", "ecdsa384", "ed25519"][..],
            "ed25519",
        ),
        (&["ecdsa256", "ecdsa384"][..], "ecdsa384"),
        (&["ecdsa521", "ecdsa256", "rsa2048"][..], "ecdsa256"),
    ] {
        let sshd = private_sshd(&binary, keys).await;
        let ssh = Ssh::connect(
            &sshd.target(),
            Auth::key(fixture_key("ed25519")),
            Arc::new(AcceptAny),
            &options(),
        )
        .await
        .unwrap();
        assert_eq!(
            ssh.host_key().fingerprint(),
            keygen_fingerprint(shown),
            "{keys:?}"
        );
    }
}

/// What Citadel does with an RSA key — `ssh-rsa`, a SHA-1 signature — real
/// OpenSSH (8.8 and later) refuses; the same key as rsa-sha2-512 is in.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_refuses_the_sha1_signature_citadel_sends() {
    use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
    let binary = sshd_or_skip!();
    let sshd = private_sshd(&binary, &["ed25519"]).await;

    struct Any;
    impl russh::client::Handler for Any {
        type Error = russh::Error;
        async fn check_server_key(
            &mut self,
            _: &russh::keys::PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    for (hash, accepted) in [(None, false), (Some(HashAlg::Sha512), true)] {
        let mut handle = russh::client::connect(
            Arc::new(russh::client::Config::default()),
            ("127.0.0.1", sshd.port),
            Any,
        )
        .await
        .unwrap();
        let key = Arc::new(fixture_private("rsa2048"));
        let result = handle
            .authenticate_publickey(whoami(), PrivateKeyWithHashAlg::new(key, hash))
            .await
            .unwrap();
        assert_eq!(result.success(), accepted, "{hash:?}");
    }
    let log = sshd.log();
    eprintln!("{log}");
    assert!(
        log.contains("ssh-rsa") && log.contains("not in PubkeyAcceptedAlgorithms"),
        "{log}"
    );
}

/// B241, B242 on the core's path, against a real `sftp-server`: SFTP says
/// why a path can't be read — no such file, not this account's — and the
/// app's lent host (`BorrowedHost`) reports that, where the Swift path took
/// any SFTP failure for "no SFTP", ran the exec form and showed a generic
/// error. Only a host without the subsystem is `Unavailable`. A command's
/// stderr stays out of its stdout, which the parsers read (the Swift path
/// merges them).
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_sftp_says_why_and_a_commands_stderr_stays_apart() {
    use std::os::unix::fs::PermissionsExt;
    let binary = sshd_or_skip!();
    let sshd = private_sshd(&binary, &["ed25519"]).await;
    let ssh = Ssh::connect(
        &sshd.target(),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}\n{}", sshd.log()));

    let out = ssh.run("echo out; echo err 1>&2; exit 3").await.unwrap();
    assert_eq!(out.stdout(), b"out\n");
    assert_eq!(out.stderr(), b"err\n");
    assert_eq!(out.status(), Some(3));

    let scratch = Scratch::new("sftp-why");
    let locked = scratch.join("locked.txt");
    std::fs::write(&locked, b"secret").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let closed = scratch.join("closed");
    std::fs::create_dir(&closed).unwrap();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).unwrap();
    let path = |p: &Path| p.to_str().unwrap().to_owned();

    let sftp = ssh.sftp().await.unwrap();
    let missing = sftp.stat(&path(&scratch.join("missing.txt"))).await;
    let unreadable = sftp.open(&path(&locked)).await.map(|_| ());
    let unlisted = sftp.list(&path(&closed)).await.map(|_| ());
    let download = sftp
        .download(&path(&locked), &scratch.join("copy"), &mut |_| true)
        .await
        .map(|_| ());
    // Put the modes back before asserting, so the scratch folder goes.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(missing, Err(SftpError::NotFound(_))),
        "{missing:?}"
    );
    assert!(
        matches!(unreadable, Err(SftpError::PermissionDenied(_))),
        "{unreadable:?}"
    );
    assert!(
        matches!(unlisted, Err(SftpError::PermissionDenied(_))),
        "{unlisted:?}"
    );
    assert!(
        matches!(download, Err(SftpError::PermissionDenied(_))),
        "{download:?}"
    );
    ssh.close().await;
}

/// r42 against a real `sftp-server`: a rename that only changes case goes
/// through. On a disk that ignores case (a Mac's /tmp), OpenSSH's own rename
/// refuses it — it links the new name first, and the old answers for it —
/// so the core moves the file to a free name and then to the new one.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_renames_to_another_case_of_the_same_name() {
    let binary = sshd_or_skip!();
    let sshd = private_sshd(&binary, &["ed25519"]).await;
    let ssh = Ssh::connect(
        &sshd.target(),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}\n{}", sshd.log()));
    let scratch = Scratch::new("sftp-case");
    let path = |name: &str| scratch.join(name).to_str().unwrap().to_owned();
    let names = || {
        let mut names: Vec<String> = std::fs::read_dir(scratch.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    };
    std::fs::write(scratch.join("a.png"), b"a").unwrap();
    std::fs::create_dir(scratch.join("drafts")).unwrap();
    std::fs::write(scratch.join("b.png"), b"b").unwrap();

    let sftp = ssh.sftp().await.unwrap();
    sftp.rename(&path("a.png"), &path("A.png")).await.unwrap();
    sftp.rename(&path("drafts"), &path("Drafts")).await.unwrap();
    let taken = sftp.rename(&path("b.png"), &path("A.png")).await;
    assert!(matches!(taken, Err(SftpError::NameTaken(_))), "{taken:?}");
    assert_eq!(names(), ["A.png", "Drafts", "b.png"]);
    assert_eq!(std::fs::read(scratch.join("A.png")).unwrap(), b"a");
    assert_eq!(std::fs::read(scratch.join("b.png")).unwrap(), b"b");
    ssh.close().await;
}

/// B126 and every import path, end to end: a key in each form the app
/// imports — ssh-keygen's own (each cipher), its PEM and PKCS #8, OpenSSL's
/// SEC1, PKCS #1 and PKCS #8 (traditional and PBES2 encryption, explicit
/// curves), PuTTY's, and keys made here — goes through
/// `keyfile::import` with its passphrase, and the stored text it gives
/// signs in to real OpenSSH. Ed25519, RSA (rsa-sha2), and ECDSA on P-256,
/// P-384 and P-521.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_takes_a_key_imported_in_every_form() {
    use moshpit_ssh::keyfile::{self, Generate};
    let binary = sshd_or_skip!();
    let read = |path: &str| fixture_key(path);
    let mut keys = Vec::new();
    for (path, passphrase) in [
        (
            "app/ed25519-openssh-passphrase",
            Some("correct horse battery"),
        ),
        (
            "app/ed25519-pkcs-8-openssl-genpkey-passphrase",
            Some("correct horse battery"),
        ),
        ("app/chacha-encrypted", Some("correct horse battery")),
        (
            "app/rsa-2048-pem-ssh-keygen-m-pem-passphrase",
            Some("correct horse battery"),
        ),
        (
            "app/rsa-2048-pkcs-8-ssh-keygen-m-pkcs8-passphrase",
            Some("correct horse battery"),
        ),
        (
            "app/ecdsa-p-256-pem-ssh-keygen-m-pem-passphrase",
            Some("correct horse battery"),
        ),
        ("app/ecdsa-p-256-pkcs-8-ssh-keygen-m-pkcs8", None),
        ("app/ecdsa-p384", None),
        ("matrix/keygen-ecdsa384-pem-encrypted", Some("matrix pass")),
        (
            "matrix/keygen-ecdsa521-pkcs8-encrypted",
            Some("matrix pass"),
        ),
        (
            "matrix/keygen-ecdsa521-openssh-aes256-gcm",
            Some("matrix pass"),
        ),
        (
            "matrix/openssl-p384-sec1-traditional-aes256",
            Some("matrix pass"),
        ),
        ("matrix/openssl-p521-pkcs8-explicit", None),
        (
            "matrix/openssl-rsa2048-pkcs1-traditional-des3",
            Some("matrix pass"),
        ),
        ("putty/id_ecdsa_p256.ppk", None),
        ("putty/id_rsa_3072_enc.ppk2", Some("123")),
    ] {
        let stored = keyfile::import(&read(path), passphrase, None, "")
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        keys.push((path.to_owned(), stored));
    }
    keys.push((
        "generated ed25519".to_owned(),
        keyfile::generate(Generate::Ed25519, "made").unwrap(),
    ));
    keys.push((
        "generated rsa".to_owned(),
        keyfile::generate(Generate::Rsa(2048), "made").unwrap(),
    ));
    let authorized: String = keys
        .iter()
        .map(|(_, k)| format!("{}\n", k.info.public_key))
        .collect();
    let sshd = private_sshd_authorizing(&binary, &["ed25519"], &authorized).await;
    let mut kinds = std::collections::BTreeSet::new();
    for (name, stored) in &keys {
        let ssh = Ssh::connect(
            &sshd.target(),
            Auth::key(stored.openssh.to_string()),
            Arc::new(AcceptAny),
            &options(),
        )
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}\n{}", sshd.log()));
        assert!(ssh.run("true").await.unwrap().success(), "{name}");
        ssh.close().await;
        assert!(
            sshd.log().contains(&stored.info.fingerprint),
            "{name}: {} never signed in",
            stored.info.fingerprint
        );
        kinds.insert(stored.info.kind.ssh_name().to_owned());
        eprintln!("signed in: {name} ({})", stored.info.kind.ssh_name());
    }
    assert_eq!(
        kinds.into_iter().collect::<Vec<_>>(),
        [
            "ecdsa-sha2-nistp256",
            "ecdsa-sha2-nistp384",
            "ecdsa-sha2-nistp521",
            "ssh-ed25519",
            "ssh-rsa"
        ]
    );
}

fn pids_holding_file(path: &Path) -> Vec<u32> {
    let self_pid = std::process::id();
    #[cfg(target_os = "linux")]
    {
        let deleted = format!("{} (deleted)", path.display());
        let mut pids = Vec::new();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return pids;
        };
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if pid == self_pid {
                continue;
            }
            let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(target) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                if target == path || target.to_string_lossy() == deleted {
                    pids.push(pid);
                    break;
                }
            }
        }
        pids.sort_unstable();
        pids.dedup();
        pids
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = ["lsof", "/usr/sbin/lsof"]
            .into_iter()
            .find_map(|bin| Command::new(bin).args(["-t", "--"]).arg(path).output().ok());
        let Some(out) = out else {
            return Vec::new();
        };
        let mut pids: Vec<u32> = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|s| s.parse::<u32>().ok())
            .filter(|&pid| pid != self_pid)
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// B870: dropping a private sshd while a session is open (a forked session
/// worker having called `setsid` and still holding `sshd.log`) must reap the
/// whole tree, not leave detached workers alive behind `Drop`.
#[tokio::test]
#[ignore = "needs an sshd binary; see the module docs"]
async fn private_sshd_drop_reaps_detached_session_children() {
    let binary = sshd_or_skip!();
    let sshd = private_sshd(&binary, &["ed25519"]).await;
    let listener_pid = sshd.child.id();
    let log_path = sshd.dir.join("sshd.log");
    let ssh = Ssh::connect(
        &sshd.target(),
        Auth::key(fixture_key("ed25519")),
        Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap();
    let mut request = PtyRequest::new(80, 24);
    request.command = Some("sleep 30".to_owned());
    let _proc = ssh.terminal(&request).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let holders = pids_holding_file(&log_path);
    let workers: Vec<u32> = holders
        .iter()
        .copied()
        .filter(|&pid| pid != listener_pid)
        .collect();
    assert!(
        !workers.is_empty(),
        "expected sshd session worker holding {}; holders={holders:?}",
        log_path.display()
    );
    drop(sshd);
    let survivors: Vec<u32> = workers.into_iter().filter(|&pid| pid_alive(pid)).collect();
    if !survivors.is_empty() {
        for &pid in &survivors {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .output();
        }
        panic!("B870: sshd workers survived Drop: {survivors:?}");
    }
}
