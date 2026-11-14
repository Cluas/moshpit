//! W10c against real programs on this machine's loopback, private every
//! one: an sshd of our own (temp keys and config, key auth only), the
//! mosh-server it starts (stopped by pid afterwards; nobody else's is
//! touched), a herdr 0.9.5 server on a private session under a temp HOME
//! (`--session mw10b-…`, every herdr command carries it). herdr over mosh on
//! the Rust core: mosh-server runs the raw-attach loop, the control client
//! steers it from beside mosh on the kept bootstrap connection.
//!
//! - one device: the loop on the focused pane, echo through mosh and the
//!   attach, a pick (the switch's time to the loop's word and to the veil
//!   lifting), a reply to an unfocused pane, the pane on screen closed, herdr
//!   restarted, a reply over a silent UDP path;
//! - two devices on one herdr: following the focus never takes a pane,
//!   "Open here" does;
//! - a switch storm: typing reaches only the pane asked for;
//! - the connect's cleanup kills only a renderer.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_w10c -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_herdr::client::{Notice, NoticeAction};
use moshpit_session::{HerdrAction, HerdrView, Multiplexer, Protocol, SessionId, view_marks};
use support::*;

macro_rules! need {
    () => {
        if !Sshd::available() || !PrivateHerdr::available() {
            eprintln!("skipped: no sshd or herdr");
            return;
        }
    };
}

async fn eventually(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    check()
}

fn mosh_herdr_spec(port: u16, herdr: &PrivateHerdr) -> moshpit_session::ConnectionSpec {
    let mut spec = spec(port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Herdr;
    spec.herdr_path = Some(herdr.custom_path());
    spec
}

/// Two panes, A focused (the second made by a split).
fn two_panes(herdr: &PrivateHerdr) -> (String, String) {
    let a = herdr.panes()[0].clone();
    herdr.run(&["pane", "split", &a, "--direction", "right"]);
    let b = herdr
        .panes()
        .into_iter()
        .find(|pane| *pane != a)
        .expect("the split's pane");
    focus(herdr, &a);
    (a, b)
}

/// The server's focus on `pane`, as another client would move it.
fn focus(herdr: &PrivateHerdr, pane: &str) {
    if herdr.run(&["agent", "focus", pane]).1 != 0 || herdr.focused().as_deref() != Some(pane) {
        herdr.run(&["pane", "zoom", "--pane", pane, "--on"]);
        herdr.run(&["pane", "zoom", "--pane", pane, "--off"]);
    }
}

fn view(bench: &Bench, id: SessionId) -> HerdrView {
    bench
        .sessions
        .view(id)
        .and_then(|view| view.herdr)
        .expect("herdr's view")
}

fn on_screen(bench: &Bench, id: SessionId) -> Option<String> {
    bench
        .sessions
        .view(id)
        .and_then(|v| v.herdr)
        .and_then(|v| v.attached_pane)
}

fn notice(bench: &Bench, id: SessionId) -> Option<(Notice, Option<NoticeAction>)> {
    bench
        .sessions
        .view(id)
        .and_then(|v| v.herdr)
        .and_then(|v| v.notice)
}

/// The shell's channel: what mosh painted, and the marks among it.
struct Screen {
    stream: u64,
    bytes: Vec<u8>,
    marks: Vec<(Instant, u32)>,
    /// Where in the bytes each mark came.
    at: Vec<(usize, u32)>,
    rung: std::sync::Arc<tokio::sync::Notify>,
}

impl Screen {
    fn attach(stream: u64) -> Screen {
        let rung = std::sync::Arc::new(tokio::sync::Notify::new());
        let bell = rung.clone();
        assert!(moshpit_stream::view::attach(stream, move || bell.notify_one()));
        Screen {
            stream,
            bytes: Vec::new(),
            marks: Vec::new(),
            at: Vec::new(),
            rung,
        }
    }

    fn read(&mut self) {
        let mut buf = [0u8; 16 * 1024];
        loop {
            let (n, mark) = moshpit_stream::view::read_marked(self.stream, &mut buf);
            self.bytes.extend_from_slice(&buf[..n]);
            if let Some(mark) = mark {
                self.marks.push((Instant::now(), mark.code));
                self.at.push((self.bytes.len(), mark.code));
                continue;
            }
            if n < buf.len() {
                return;
            }
        }
    }

    fn write(&self, bytes: &[u8]) {
        assert!(moshpit_stream::view::write(self.stream, bytes));
    }

    /// Waits for `needle` among the bytes after `from`.
    async fn wait_from(&mut self, from: usize, needle: &[u8], within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            self.read();
            if find(&self.bytes[from.min(self.bytes.len())..], needle).is_some() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let _ = tokio::time::timeout(left.min(Duration::from_millis(50)), self.rung.notified())
                .await;
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        moshpit_stream::view::detach(self.stream);
    }
}

fn ms(since: Instant) -> u128 {
    since.elapsed().as_millis()
}

/// Connects one device and waits for its loop to show `pane`.
async fn device(
    bench: &mut Bench,
    sshd: &Sshd,
    herdr: &PrivateHerdr,
    pane: &str,
) -> (SessionId, Screen, u128, Reaper) {
    let started = Instant::now();
    let id = bench.sessions.connect(mosh_herdr_spec(sshd.port, herdr));
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let reaper = reaper(bench, id);
    let (_, panes) = bench.panes(id, 10).await;
    let screen = Screen::attach(panes[0].stream);
    assert!(
        eventually(20, || on_screen(bench, id).as_deref() == Some(pane)).await,
        "the loop on {pane}: {:?}",
        bench
            .sessions
            .view(id)
            .and_then(|v| v.herdr)
            .map(|v| (v.attached_pane, v.notice))
    );
    (id, screen, ms(started), reaper)
}

/// The session's mosh-server, stopped when the test ends however it ends
/// (it daemonizes: sshd's process group doesn't reach it).
fn reaper(bench: &Bench, id: SessionId) -> Reaper {
    Reaper(bench.sessions.view(id).and_then(|v| v.mosh_server_pid))
}

// MARK: one device

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and herdr; see the module docs"]
async fn herdr_over_mosh_on_the_core() {
    need!();
    let mut herdr = PrivateHerdr::start();
    let (a, b) = two_panes(&herdr);
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let (id, mut screen, connect_ms, _reaper) = device(&mut bench, &sshd, &herdr, &a).await;
    // The tree beside it, and the pane's prompt on mosh's screen.
    let tree = view(&bench, id);
    assert!(tree.snapshot.panes.contains_key(&a) && tree.snapshot.panes.contains_key(&b));
    assert!(
        screen.wait_from(0, b"%", Duration::from_secs(10)).await,
        "{}",
        screen.text()
    );
    let painted_ms = connect_ms;

    // Echo: a keystroke down mosh into the attach, back as the pane's
    // repaint.
    let mut echo = Vec::new();
    for c in b"qwertyuiopasdfghjklz" {
        let from = screen.bytes.len();
        let typed = Instant::now();
        screen.write(&[*c]);
        assert!(screen.wait_from(from, &[*c], Duration::from_secs(5)).await);
        echo.push(typed.elapsed().as_micros());
    }
    screen.write(b"\x15");
    let (e50, e95, _, emax) = percentiles(&mut echo);
    screen.write(b"echo one-$((6*7))\r");
    assert!(
        eventually(10, || herdr.read(&a).contains("one-42")).await,
        "{}",
        herdr.read(&a)
    );

    // The person picks B: the loop takes it on its word; the veil lifts
    // after B's repaint (its own words on its screen, before the reveal in
    // the channel).
    herdr.run(&["pane", "run", &b, "echo BEE-PANE-$((6*7))"]);
    assert!(eventually(10, || herdr.read(&b).contains("BEE-PANE-42")).await);
    screen.read();
    let bytes_before = screen.bytes.len();
    let at_before = screen.at.len();
    let marks_before = screen.marks.len();
    let picked = Instant::now();
    bench.sessions.herdr(id, HerdrAction::SelectPane(b.clone()));
    assert!(eventually(10, || on_screen(&bench, id).as_deref() == Some(b.as_str())).await);
    let landed_ms = ms(picked);
    let revealed = eventually(5, || {
        screen.read();
        screen.marks[marks_before..]
            .iter()
            .any(|(_, code)| *code == view_marks::REVEAL)
    })
    .await;
    assert!(revealed, "{:?}", screen.marks);
    let veil_at = screen.marks[marks_before..]
        .iter()
        .find(|(_, code)| *code == view_marks::VEIL)
        .map(|(at, _)| *at)
        .expect("veiled");
    let reveal_at = screen.marks[marks_before..]
        .iter()
        .find(|(_, code)| *code == view_marks::REVEAL)
        .map(|(at, _)| *at)
        .unwrap();
    let reveal_ms = reveal_at.duration_since(picked).as_millis();
    assert!(veil_at <= reveal_at);
    let painted_b = find(&screen.bytes[bytes_before..], b"BEE-PANE-42")
        .map(|at| bytes_before + at)
        .expect("B's repaint");
    let revealed_after = screen.at[at_before..]
        .iter()
        .find(|(_, code)| *code == view_marks::REVEAL)
        .map(|(at, _)| *at)
        .unwrap();
    assert!(
        revealed_after > painted_b,
        "the veil lifted (byte {revealed_after}) before B's repaint (byte {painted_b})"
    );
    screen.write(b"echo two-$((6*7))\r");
    assert!(
        eventually(10, || herdr.read(&b).contains("two-42")).await,
        "{}",
        herdr.read(&b)
    );
    assert!(!herdr.read(&a).contains("two-42"));

    // A reply to A, which isn't on screen: switched to, sent, said so.
    let asked = Instant::now();
    let reply = bench
        .sessions
        .deliver(id, &a, b"echo three-$((6*7))\r".to_vec(), false);
    let sent = tokio::time::timeout(Duration::from_secs(25), reply)
        .await
        .expect("answered")
        .expect("answered");
    let reply_ms = ms(asked);
    assert!(sent);
    assert!(eventually(10, || herdr.read(&a).contains("three-42")).await);
    assert!(!herdr.read(&b).contains("three-42"));
    assert_eq!(on_screen(&bench, id).as_deref(), Some(a.as_str()));

    // The pane on screen closes: back to the focused pane by itself.
    bench.sessions.herdr(id, HerdrAction::SelectPane(b.clone()));
    assert!(eventually(10, || on_screen(&bench, id).as_deref() == Some(b.as_str())).await);
    let closed = Instant::now();
    herdr.run(&["pane", "close", &b]);
    assert!(
        eventually(15, || on_screen(&bench, id).as_deref() == Some(a.as_str())).await,
        "{:?}",
        view(&bench, id).notice
    );
    let back_ms = ms(closed);
    assert_eq!(notice(&bench, id), None);
    screen.write(b"echo four-$((6*7))\r");
    assert!(eventually(10, || herdr.read(&a).contains("four-42")).await);

    // herdr restarts: back on the new focused pane by itself, nothing typed
    // meanwhile run by a shell.
    let restarted = Instant::now();
    herdr.restart();
    screen.write(b"echo lost-$((6*7))\r");
    let mut fresh = None;
    assert!(
        eventually(30, || {
            fresh = herdr.focused();
            fresh.is_some() && on_screen(&bench, id) == fresh
        })
        .await,
        "{:?} vs {:?}",
        on_screen(&bench, id),
        herdr.focused()
    );
    let restart_ms = ms(restarted);
    let fresh = fresh.unwrap();
    screen.write(b"echo five-$((6*7))\r");
    assert!(eventually(10, || herdr.read(&fresh).contains("five-42")).await);
    assert!(!screen.text().contains("not found"), "a shell ran a line");

    // UDP goes silent: a reply is never said sent; nothing of it lands once
    // the path heals after mosh's keystroke lifetime.
    bench.udp.faults.blackhole(true);
    let asked = Instant::now();
    let reply = bench
        .sessions
        .deliver(id, &fresh, b"echo silent-$((6*7))\r".to_vec(), false);
    let sent = tokio::time::timeout(Duration::from_secs(25), reply)
        .await
        .expect("answered")
        .expect("answered");
    let silent_ms = ms(asked);
    assert!(!sent, "said sent over a silent path");
    tokio::time::sleep(Duration::from_secs(12)).await;
    bench.udp.faults.blackhole(false);
    tokio::time::sleep(Duration::from_secs(4)).await;
    screen.write(b"echo healed-$((6*7))\r");
    assert!(eventually(15, || herdr.read(&fresh).contains("healed-42")).await);
    let silent_landed = herdr.read(&fresh).contains("silent-42");

    eprintln!(
        "herdr over mosh (Rust core): loop on the focused pane {connect_ms} ms after connect \
         (prompt painted by {painted_ms} ms); echo p50 {} ms p95 {} ms max {} ms; \
         pick → loop's word {landed_ms} ms, → veil lifted {reveal_ms} ms; reply to an \
         unfocused pane answered sent in {reply_ms} ms; pane closed → back on the focused \
         pane {back_ms} ms; herdr restart → back {restart_ms} ms; reply over silent UDP \
         answered not sent in {silent_ms} ms (landed after the heal: {silent_landed})",
        e50 / 1000,
        e95 / 1000,
        emax / 1000
    );
    assert!(!silent_landed, "a reply said not sent landed later");
    let _ = bench.sessions.disconnect(id).await;
}

// MARK: two devices

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and herdr; see the module docs"]
async fn two_devices_following_the_focus_never_takes_and_open_here_does() {
    need!();
    let herdr = PrivateHerdr::start();
    let (a, b) = two_panes(&herdr);
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    // The phone on A.
    let (phone, _phone_screen, _, _phone_reaper) = device(&mut bench, &sshd, &herdr, &a).await;
    // The iPad follows the focus onto A too: herdr refuses its plain attach
    // (the phone holds A), and it says so; the phone keeps A.
    let ipad = bench.sessions.connect(mosh_herdr_spec(sshd.port, &herdr));
    bench.connected(ipad).await;
    let _ipad_reaper = reaper(&bench, ipad);
    let (_, panes) = bench.panes(ipad, 10).await;
    let ipad_screen = Screen::attach(panes[0].stream);
    assert!(
        eventually(20, || notice(&bench, ipad)
            == Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere))))
        .await,
        "{:?}",
        notice(&bench, ipad)
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(on_screen(&bench, phone).as_deref(), Some(a.as_str()));
    assert_eq!(notice(&bench, phone), None, "the phone lost A to a follow");

    // The iPad's person picks B: theirs. The server's focus is B now; the
    // phone follows it — refused, said, and the iPad keeps B.
    bench
        .sessions
        .herdr(ipad, HerdrAction::SelectPane(b.clone()));
    assert!(
        eventually(10, || on_screen(&bench, ipad).as_deref()
            == Some(b.as_str()))
        .await
    );
    let follow = Instant::now();
    assert!(
        eventually(20, || notice(&bench, phone)
            == Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere))))
        .await,
        "{:?} on {:?}",
        notice(&bench, phone),
        on_screen(&bench, phone)
    );
    let follow_ms = ms(follow);
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_eq!(
        on_screen(&bench, ipad).as_deref(),
        Some(b.as_str()),
        "the follow took B"
    );
    assert_eq!(notice(&bench, ipad), None);
    ipad_screen.write(b"echo ipad-$((6*7))\r");
    assert!(eventually(10, || herdr.read(&b).contains("ipad-42")).await);

    // "Open here" on the phone takes B; the iPad says so in turn.
    let opened = Instant::now();
    bench.sessions.herdr(phone, HerdrAction::NoticeAction);
    assert!(
        eventually(10, || on_screen(&bench, phone).as_deref()
            == Some(b.as_str()))
        .await
    );
    let open_ms = ms(opened);
    assert_eq!(notice(&bench, phone), None);
    assert!(
        eventually(10, || notice(&bench, ipad)
            == Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere))))
        .await
    );
    eprintln!(
        "two devices (Rust core): a follow onto the other device's pane said so in {follow_ms} ms \
         and took nothing; Open here took it in {open_ms} ms"
    );
    let _ = bench.sessions.disconnect(phone).await;
    let _ = bench.sessions.disconnect(ipad).await;
}

// MARK: a switch storm

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and herdr; see the module docs"]
async fn a_switch_storm_types_only_into_the_pane_asked_for() {
    need!();
    let herdr = PrivateHerdr::start();
    let (a, b) = two_panes(&herdr);
    herdr.run(&["pane", "split", &b, "--direction", "down"]);
    let c = herdr
        .panes()
        .into_iter()
        .find(|pane| *pane != a && *pane != b)
        .unwrap();
    focus(&herdr, &a);
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let (id, screen, _, _reaper) = device(&mut bench, &sshd, &herdr, &a).await;
    let panes = [b.clone(), c.clone(), a.clone()];
    let mut picks = Vec::new();
    let mut seen = Vec::new();
    let started = Instant::now();
    for round in 0..15usize {
        let pane = panes[(round * 7 + round / 3) % 3].clone();
        bench
            .sessions
            .herdr(id, HerdrAction::SelectPane(pane.clone()));
        tokio::time::sleep(Duration::from_millis(60 + (round as u64 % 4) * 45)).await;
        let now = view(&bench, id);
        seen.push((
            round,
            pane.clone(),
            now.frame_target,
            now.attached_pane,
            ms(started),
        ));
        screen.write(format!("<q{round:02}z> ").as_bytes());
        picks.push(pane);
    }
    let last = picks.last().unwrap().clone();
    assert!(
        eventually(15, || on_screen(&bench, id).as_deref()
            == Some(last.as_str()))
        .await
    );
    let settled_ms = ms(started);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut landed = 0;
    for (round, pane) in picks.iter().enumerate() {
        let probe = format!("<q{round:02}z>");
        for other in [&a, &b, &c] {
            if herdr.read(other).contains(&probe) {
                landed += 1;
                assert_eq!(
                    other, pane,
                    "{probe} typed after picking {pane} reached {other}: {seen:?}"
                );
            }
        }
    }
    screen.write(b"\x15echo calm-$((6*7))\r");
    assert!(eventually(10, || herdr.read(&last).contains("calm-42")).await);
    eprintln!(
        "switch storm (Rust core): 15 picks in {} ms settled on the last at {settled_ms} ms; \
         {landed}/15 probes typed mid-storm went out, each into the pane asked for",
        picks.len() * 120
    );
    let _ = bench.sessions.disconnect(id).await;
}

// MARK: the connect's cleanup

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and herdr; see the module docs"]
async fn the_connects_cleanup_kills_only_a_renderer() {
    need!();
    let herdr = PrivateHerdr::start();
    let a = herdr.panes()[0].clone();
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let spec = mosh_herdr_spec(sshd.port, &herdr);
    // An earlier generation's files, its pid file naming a process that
    // isn't a renderer (the pid went to something else).
    let mut bystander = std::process::Command::new("/bin/sleep")
        .arg("120")
        .spawn()
        .unwrap();
    let dir = herdr.home.join(".moshpit");
    std::fs::create_dir_all(&dir).unwrap();
    let base = format!("mosh-{}-0BSOLETE", spec.connection_id);
    std::fs::write(dir.join(format!("{base}.pid")), bystander.id().to_string()).unwrap();
    std::fs::write(dir.join(format!("{base}.target")), "term_old").unwrap();
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let _reaper = reaper(&bench, id);
    assert!(eventually(20, || on_screen(&bench, id).as_deref() == Some(a.as_str())).await);
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "the cleanup killed a bystander"
    );
    assert!(!dir.join(format!("{base}.pid")).exists());
    assert!(!dir.join(format!("{base}.target")).exists());
    let _ = bystander.kill();
    let _ = bystander.wait();
    let _ = bench.sessions.disconnect(id).await;
}

// MARK: herdr over SSH under a slow login shell (the Canary report)

/// herdr over SSH on the core, the account's login shell a zsh whose rc
/// takes seconds (oh-my-zsh's) and flushes what was typed meanwhile
/// (Powerlevel10k's instant prompt), the network path flapping every 5 s.
/// The frame channel's boot line, typed into that shell, never ran; every
/// re-ask typed a release and a one-shot line into it too: a blinking cursor,
/// forever. The loop is the channel's own program now.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and herdr; see the module docs"]
async fn herdr_over_ssh_attaches_under_a_slow_flushing_login_shell() {
    need!();
    slow_login_shell(
        "a 3 s, flushing zsh rc",
        "sleep 3\nprint -r -- '[oh-my-zsh] plugins loading'\n\
         python3 -c 'import termios; termios.tcflush(0, termios.TCIFLUSH)' 2>/dev/null\n\
         PROMPT='%% '\nunsetopt BEEP\n"
            .to_owned(),
    )
    .await;
}

/// The same under oh-my-zsh with zsh-autosuggestions and
/// zsh-syntax-highlighting (the Canary report's shell): this machine's own
/// `~/.oh-my-zsh`, read only — its update check off, its cache and
/// completion dump in the temp HOME.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, herdr and ~/.oh-my-zsh; see the module docs"]
async fn herdr_over_ssh_attaches_under_oh_my_zsh() {
    need!();
    let omz = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".oh-my-zsh");
    if !omz.join("oh-my-zsh.sh").exists() {
        eprintln!("skipped: no ~/.oh-my-zsh");
        return;
    }
    let custom = omz.join("custom/plugins");
    let plugins: Vec<&str> = [
        "git",
        "zsh-autosuggestions",
        "zsh-syntax-highlighting",
        "zsh-completions",
    ]
    .into_iter()
    .filter(|p| *p == "git" || custom.join(p).exists() || omz.join("plugins").join(p).exists())
    .collect();
    slow_login_shell(
        "oh-my-zsh",
        format!(
            "export ZSH={}\nDISABLE_AUTO_UPDATE=true\nzstyle ':omz:update' mode disabled\n\
             ZSH_DISABLE_COMPFIX=true\nZSH_CACHE_DIR=$HOME/.omz-cache\n\
             ZSH_COMPDUMP=$HOME/.zcompdump\nmkdir -p $ZSH_CACHE_DIR\n\
             ZSH_THEME=robbyrussell\nplugins=({})\nsource $ZSH/oh-my-zsh.sh\n",
            omz.display(),
            plugins.join(" ")
        ),
    )
    .await;
}

/// herdr over SSH on the core with the account's login shell a zsh whose rc
/// is `rc`, the network path flapping every 5 s.
async fn slow_login_shell(label: &str, rc: String) {
    let herdr = PrivateHerdr::start();
    let a = herdr.panes()[0].clone();
    std::fs::write(herdr.home.join(".zshrc"), rc).unwrap();
    // A pane whose zsh started on the slow rc flushes what is typed too:
    // past its start before anything is typed there.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Herdr;
    spec.herdr_path = Some(herdr.custom_path());
    // The account's login shell, as in the app.
    spec.pty_command = None;
    let started = Instant::now();
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    let flapping = {
        let sessions = bench.sessions.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                sessions.network_changed();
            }
        })
    };
    bench.connected(id).await;
    let (attach, panes) = bench.panes(id, 20).await;
    bench
        .until(id, 30, |kind| match kind {
            moshpit_session::EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
            _ => None,
        })
        .await;
    let first_frame_ms = ms(started);
    let pane = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
    assert_eq!(pane.pane, a);
    let mut term = Term::attach(pane.stream);
    term.write(b"echo slow-$((6*7))\r");
    assert!(term.wait_for("slow-42", 15).await, "{}", term.text());
    assert!(herdr.read(&a).contains("slow-42"));
    // Still attached after a few flaps of the path.
    tokio::time::sleep(Duration::from_secs(12)).await;
    term.write(b"echo still-$((6*7))\r");
    assert!(term.wait_for("still-42", 15).await, "{}", term.text());
    assert_eq!(
        bench.sessions.herdr_view(id).and_then(|v| v.notice),
        None,
        "a notice over a working channel"
    );
    flapping.abort();
    eprintln!(
        "herdr over SSH under {label}, the path flapping every 5 s: \
         first frame {first_frame_ms} ms after connect"
    );
    let _ = bench.sessions.disconnect(id).await;
}
