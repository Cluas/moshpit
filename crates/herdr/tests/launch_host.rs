//! The mosh raw-attach renderer's command lines run for real: the loop
//! ([`launch::raw_attach_loop_command`]) under `/bin/sh` with a private
//! `$HOME`, steered by the retarget and watched by the watch, against a fake
//! `herdr` (a shell script) whose `terminal attach` behaves as herdr 0.9.5
//! was measured to: it paints, holds the terminal until killed, and ends on
//! its own — exit 1, its last words on stderr — when another client takes
//! the terminal over, the pane closes or the server stops; and a plain one
//! (no `--takeover`) to a terminal another client holds is refused ~260 ms
//! in, while one with `--takeover` takes it from that client.
//!
//! Each case runs a few hundred milliseconds of real time (the loop's own
//! sleeps); every wait is bounded.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use moshpit_herdr::launch::{self, RendererStop, StopKind};

const KEY: &str = "C0FFEE00-0000-4000-8000-000000000001-n0nce";

/// The fake herdr. `terminal attach <tid> [--takeover]`: logs `<tid> <pid>
/// <--takeover|plain>` to `attaches`; refused when another client holds the
/// terminal (`held-<tid>`) and it is plain, else takes it; paints `PAINT
/// <tid>`, then holds until killed or until a file says why it ends on its
/// own, with herdr's words on stderr.
const FAKE_HERDR: &str = r#"#!/bin/sh
dir="$FAKE_HERDR_DIR"
if [ "$1 $2" = "terminal attach" ]; then
    tid="$3"
    echo "$tid $$ ${4:-plain}" >> "$dir/attaches"
    if [ -e "$dir/refuse-$tid" ] && [ "$4" != "--takeover" ]; then
        echo "herdr: server shut down: terminal attach failed: terminal $tid already has an attached client; retry with --takeover" >&2
        exit 1
    fi
    if [ -e "$dir/held-$tid" ]; then
        if [ "$4" != "--takeover" ]; then
            sleep 0.26
            echo "herdr: server shut down: terminal attach failed: terminal $tid already has an attached client; retry with --takeover" >&2
            exit 1
        fi
        rm -f "$dir/held-$tid"
        echo "$tid" >> "$dir/taken"
    fi
    printf 'PAINT %s\n' "$tid"
    if grep -qx "$tid" "$dir/missing" 2>/dev/null; then
        sleep 0.27
        echo "herdr: server shut down: terminal attach failed: terminal $tid not found" >&2
        exit 1
    fi
    while :; do
        if [ -e "$dir/evict-$tid" ]; then
            rm -f "$dir/evict-$tid"
            echo "herdr: server shut down: terminal attach taken over" >&2
            exit 1
        fi
        if [ -e "$dir/close-$tid" ]; then
            echo "herdr: server shut down: terminal attach ended: terminal $tid not found" >&2
            exit 1
        fi
        if [ -e "$dir/server-stop" ]; then
            echo "herdr: server shut down: server is shutting down" >&2
            exit 1
        fi
        sleep 0.02
    done
fi
if [ "$1 $2" = "api snapshot" ]; then
    if [ -e "$dir/no-server" ]; then
        echo '{"error":{"code":"server_not_running","message":"no server"}}'
        exit 1
    fi
    printf '{"id":"cli:api:snapshot","result":{"snapshot":{"panes":['
    sep=
    while read -r t; do printf '%s{"terminal_id":"%s"}' "$sep" "$t"; sep=,; done < "$dir/listed"
    printf ']}}}\n'
    exit 0
fi
exit 2
"#;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// How long something that has to happen is waited for before the test
/// says it never did — by the rig, and by the commands it runs on the
/// host, whose own waits are this long here (a retarget's wait for the next
/// attach, a watch's): each ends the moment it has its answer, so only a
/// failing test waits it out, and it is long. With the suite at background
/// QoS beside a dozen busy loops (load 35–100) a shell has been seen to
/// write nothing for over 60 s (T9: the 10 s it was fell short far sooner).
const EVENTUALLY: Duration = Duration::from_secs(300);

/// A private host: `$HOME`, the fake herdr, and the loop running in it.
struct Host {
    home: PathBuf,
    events: PathBuf,
    loop_shell: Option<Child>,
    /// The loop's own pid, as it wrote it when it started.
    loop_pid: Option<String>,
}

impl Host {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!(
            "moshpit-launch-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&home);
        let events = home.join("events");
        fs::create_dir_all(&events).unwrap();
        let herdr = home.join("herdr");
        fs::write(&herdr, FAKE_HERDR).unwrap();
        // The terminals the server lists.
        fs::write(events.join("listed"), "term_a\nterm_b\n").unwrap();
        Command::new("chmod")
            .arg("755")
            .arg(&herdr)
            .status()
            .unwrap();
        Host {
            home,
            events,
            loop_shell: None,
            loop_pid: None,
        }
    }

    fn herdr(&self) -> String {
        self.home.join("herdr").display().to_string()
    }

    fn sh(&self, script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(script)
            .env_clear()
            .env("HOME", &self.home)
            .env("FAKE_HERDR_DIR", &self.events)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        command
    }

    /// Types the loop into "the login shell": `/bin/sh -c '<loop line>;
    /// echo LOGIN-SHELL'` — what follows the loop's line runs only once the
    /// loop has ended and the shell has the terminal back.
    fn boot(&mut self) {
        let line = launch::raw_attach_loop_command(KEY, Some(&self.herdr()));
        let screen = fs::File::create(self.home.join("screen")).unwrap();
        // A process group of its own: whatever the loop has running when the
        // test ends goes with it (`Drop`), its pid file written or not.
        let child = self
            .sh(&format!("{line}; echo LOGIN-SHELL"))
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(screen.try_clone().unwrap())
            .stderr(screen)
            .spawn()
            .unwrap();
        self.loop_shell = Some(child);
        self.wait("the loop's pid", || self.state("loop").is_some());
        self.loop_pid = self.state("loop").map(|pid| pid.trim().to_owned());
    }

    /// An exec on the sidecar: its output.
    fn exec(&self, command: &str) -> Vec<u8> {
        self.sh(command)
            .stdin(Stdio::null())
            .output()
            .expect("sh")
            .stdout
    }

    /// [`Host::retarget`], its wait for the loop's next attach `wait` long:
    /// what it said and when, or `None` when it hadn't said anything after
    /// `give_up` (it is killed then).
    fn retarget_within(
        &self,
        terminal: &str,
        wait: Duration,
        give_up: Duration,
    ) -> Option<(Vec<u8>, Duration)> {
        let began = Instant::now();
        let mut child = self
            .sh(&launch::retarget_command_within(
                terminal,
                KEY,
                Some(&self.herdr()),
                false,
                wait,
                launch::RETARGET_SETTLE,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("sh");
        while child.try_wait().unwrap().is_none() {
            if began.elapsed() > give_up {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            sleep(Duration::from_millis(20));
        }
        let took = began.elapsed();
        let mut said = Vec::new();
        std::io::Read::read_to_end(&mut child.stdout.take().unwrap(), &mut said).unwrap();
        Some((said, took))
    }

    /// A switch that follows the focus: a plain attach. The loop is given
    /// [`EVENTUALLY`] to come round to it (the app's 2 s is a budget for a
    /// host, not for this machine at any load); the attach is watched the
    /// app's settle for a refusal, which nothing here is set to make.
    fn retarget(&self, terminal: &str) -> Vec<u8> {
        self.exec(&launch::retarget_command_within(
            terminal,
            KEY,
            Some(&self.herdr()),
            false,
            EVENTUALLY,
            launch::RETARGET_SETTLE,
        ))
    }

    /// [`Host::retarget`] for a switch herdr is set to refuse: the attach is
    /// watched until it is refused, however late (it ends the moment it is),
    /// rather than the app's settle, after which a late refusal reads as a
    /// landing.
    fn retarget_refused(&self, terminal: &str) -> Vec<u8> {
        self.exec(&launch::retarget_command_within(
            terminal,
            KEY,
            Some(&self.herdr()),
            false,
            EVENTUALLY,
            EVENTUALLY,
        ))
    }

    /// The person's pick, or their take back: `--takeover`.
    fn take(&self, terminal: &str) -> Vec<u8> {
        self.exec(&launch::retarget_command_within(
            terminal,
            KEY,
            Some(&self.herdr()),
            true,
            EVENTUALLY,
            launch::RETARGET_SETTLE,
        ))
    }

    /// The watch armed after a landing, looking for [`EVENTUALLY`].
    fn watch_command(&self) -> String {
        launch::raw_attach_watch_command_within(KEY, EVENTUALLY)
    }

    /// Each attach the loop made: its terminal and whether it took over.
    fn attach_flags(&self) -> Vec<(String, String)> {
        fs::read_to_string(self.events.join("attaches"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let words: Vec<&str> = line.split(' ').collect();
                (
                    words.first().copied().unwrap_or("").to_owned(),
                    words.get(2).copied().unwrap_or("").to_owned(),
                )
            })
            .collect()
    }

    fn state(&self, name: &str) -> Option<String> {
        fs::read_to_string(
            self.home
                .join(".moshpit")
                .join(format!("mosh-{KEY}.{name}")),
        )
        .ok()
    }

    fn screen(&self) -> String {
        fs::read_to_string(self.home.join("screen")).unwrap_or_default()
    }

    fn attaches(&self) -> Vec<String> {
        fs::read_to_string(self.events.join("attaches"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.split(' ').next().unwrap_or("").to_owned())
            .collect()
    }

    fn event(&self, name: &str) {
        fs::write(self.events.join(name), "").unwrap();
    }

    fn wait(&self, what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + EVENTUALLY;
        while !done() {
            assert!(
                Instant::now() < deadline,
                "never: {what}\nscreen: {}",
                self.screen()
            );
            sleep(Duration::from_millis(10));
        }
    }

    fn loop_running(&mut self) -> bool {
        self.loop_shell
            .as_mut()
            .is_some_and(|child| child.try_wait().unwrap().is_none())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // The loop shell, then its attach.
        let target = self
            .home
            .join(".moshpit")
            .join(format!("mosh-{KEY}.target"));
        let _ = fs::remove_file(target);
        for pid in [self.loop_pid.clone(), self.state("pid")]
            .into_iter()
            .flatten()
        {
            let _ = Command::new("kill")
                .arg("-9")
                .arg(pid.trim())
                .stderr(Stdio::null())
                .status();
        }
        if let Some(mut child) = self.loop_shell.take() {
            let deadline = Instant::now() + Duration::from_secs(3);
            while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
                sleep(Duration::from_millis(20));
            }
            // The loop's whole group: a loop that never wrote its pid (its
            // boot outwaited) or lost its pid file would otherwise outlive
            // the test, orphaned, spinning on a home that is gone.
            let _ = Command::new("kill")
                .args(["-9", "--", &format!("-{}", child.id())])
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn path_has(home: &Path, name: &str) -> bool {
    home.join(".moshpit")
        .join(format!("mosh-{KEY}.{name}"))
        .exists()
}

#[test]
fn a_takeover_parks_the_loop_and_says_why_and_nothing_reaches_the_login_shell() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the first paint", || host.screen().contains("PAINT term_a"));
    let watch = {
        let command = host.watch_command();
        let host_sh = host.sh(&command);
        std::thread::spawn(move || {
            let mut host_sh = host_sh;
            host_sh.stdin(Stdio::null()).output().unwrap().stdout
        })
    };
    sleep(Duration::from_millis(100));

    // Another client takes the terminal over: the attach ends on its own.
    host.event("evict-term_a");
    let said = watch.join().unwrap();
    assert_eq!(
        launch::renderer_stop(&said),
        Some(RendererStop::Parked {
            status: 1,
            terminal: "term_a".to_owned(),
            said: "herdr: server shut down: terminal attach taken over".to_owned(),
        }),
        "the watch said {:?}",
        String::from_utf8_lossy(&said)
    );
    assert_eq!(
        launch::renderer_stop_kind(
            1,
            "herdr: server shut down: terminal attach taken over",
            true,
            true
        ),
        StopKind::TakenOver
    );
    // Parked, not ended: the login shell never got the terminal back, and
    // the pane isn't taken back on the loop's own.
    sleep(Duration::from_millis(400));
    assert!(host.loop_running(), "the loop ended: {}", host.screen());
    assert!(!host.screen().contains("LOGIN-SHELL"));
    assert_eq!(host.attaches(), ["term_a"], "taken back on its own");
    assert!(
        !path_has(&host.home, "pid"),
        "a pid file outlived its attach"
    );

    // The person's take back: the same terminal, and it comes round.
    let said = host.take("term_a");
    assert!(
        launch::retarget_landed(&said),
        "{}",
        String::from_utf8_lossy(&said)
    );
    // The marker comes once the wrapper has written its pid; the attach
    // itself starts a moment later.
    host.wait("the attach taken back", || host.attaches().len() == 2);
    assert_eq!(host.attaches(), ["term_a", "term_a"]);
    assert!(
        !path_has(&host.home, "stop"),
        "the stop record outlived the park"
    );
}

#[test]
fn a_closed_pane_or_a_stopped_server_parks_the_loop_until_it_is_pointed_elsewhere() {
    for (event, words) in [
        (
            "close-term_a",
            "terminal attach ended: terminal term_a not found",
        ),
        ("server-stop", "server is shutting down"),
    ] {
        let mut host = Host::new();
        host.boot();
        assert!(launch::retarget_landed(&host.retarget("term_a")));
        host.wait("the attach", || host.state("pid").is_some());
        host.event(event);
        host.wait("the park", || host.state("stop").is_some());
        let said = host.exec(&host.watch_command());
        let Some(RendererStop::Parked { status, said, .. }) = launch::renderer_stop(&said) else {
            panic!("no stop: {}", String::from_utf8_lossy(&said));
        };
        assert!(said.contains(words), "{said}");
        assert_eq!(
            launch::renderer_stop_kind(status, &said, true, true),
            StopKind::Gone
        );
        assert!(host.loop_running(), "the loop ended after {event}");
        if event == "server-stop" {
            fs::remove_file(host.events.join("server-stop")).unwrap();
        }
        // The app points it at the focused pane: it comes round.
        assert!(launch::retarget_landed(&host.retarget("term_b")));
        host.wait("the new pane's paint", || {
            host.screen().contains("PAINT term_b")
        });
        assert!(!host.screen().contains("LOGIN-SHELL"));
    }
}

#[test]
fn a_switch_bounces_the_attach_without_parking_and_the_watch_steps_aside() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    let command = host.watch_command();
    let watch = {
        let mut sh = host.sh(&command);
        std::thread::spawn(move || sh.stdin(Stdio::null()).output().unwrap().stdout)
    };
    sleep(Duration::from_millis(100));
    assert!(launch::retarget_landed(&host.retarget("term_b")));
    // The attach it watched was replaced: it says nothing and ends.
    let said = watch.join().unwrap();
    assert_eq!(
        launch::renderer_stop(&said),
        None,
        "{}",
        String::from_utf8_lossy(&said)
    );
    assert!(!path_has(&host.home, "stop"), "a kill is no park");
    host.wait("the new attach", || host.attaches().len() == 2);
    assert_eq!(host.attaches(), ["term_a", "term_b"]);
}

#[test]
fn a_parked_loop_goes_when_its_target_is_removed() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    host.event("evict-term_a");
    host.wait("the park", || host.state("stop").is_some());
    // A later connect's cleanup of this (orphaned) generation.
    host.exec(&launch::stale_renderer_cleanup_command(
        KEY.rsplit_once('-').unwrap().0,
    ));
    let deadline = Instant::now() + EVENTUALLY;
    while host.loop_running() {
        assert!(
            Instant::now() < deadline,
            "the parked loop outlived its target"
        );
        sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_retarget_says_the_loop_is_gone_at_once_when_it_was_killed() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    // The loop shell itself is killed (its attach goes with the session).
    let looped = host.state("loop").unwrap();
    Command::new("kill")
        .arg("-9")
        .arg(looped.trim())
        .status()
        .unwrap();
    if let Some(pid) = host.state("pid") {
        let _ = Command::new("kill").arg("-9").arg(pid.trim()).status();
    }
    // "At once" is told from "when the wait ran out" by a wait the test
    // never sees the end of, and a limit far below it that no load stretches
    // the exec past: the time anything that has to happen is given (T9: a
    // 1.5 s bound on the 2 s wait failed at load 68–93; at load 60 under
    // background QoS the exec took 10.9 s, a shell's start over 60 s).
    // Detection left for the end of the wait fails at the limit.
    let wait = Duration::from_secs(3600);
    let limit = EVENTUALLY;
    let Some((said, took)) = host.retarget_within("term_b", wait, limit) else {
        panic!("nothing said after {limit:?}: it is waiting out the {wait:?} wait");
    };
    assert_eq!(
        launch::renderer_stop(&said),
        Some(RendererStop::LoopGone),
        "said after {took:?}: {}",
        String::from_utf8_lossy(&said)
    );
    assert!(!launch::retarget_landed(&said));
    let watched = host.exec(&host.watch_command());
    assert_eq!(
        launch::renderer_stop(&watched),
        Some(RendererStop::LoopGone)
    );
}

/// A follow herdr refuses before the retarget has seen its attach alive
/// (a refusal faster than the retarget's looks: a loaded host, a herdr
/// that answers at once) used to be said as nothing at all: the attach's
/// pid came and went between two looks, the stop record was there, and the
/// retarget ran out its wait. The app took it for a switch that didn't
/// land and asked again, never saying the pane is another client's.
#[test]
fn a_follow_refused_before_its_attach_is_seen_is_still_said() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    host.event("refuse-term_b");
    for round in 0..5 {
        let said = host.retarget_refused("term_b");
        let Some(RendererStop::Parked {
            terminal,
            said: words,
            ..
        }) = launch::renderer_stop(&said)
        else {
            panic!(
                "round {round}: not said: {:?}",
                String::from_utf8_lossy(&said)
            );
        };
        assert_eq!(terminal, "term_b");
        assert!(words.contains("already has an attached client"), "{words}");
    }
}

/// A stale pid file names whatever has the pid now: the loop it belonged to
/// died with its mosh-server (or the host rebooted) without removing it.
#[test]
fn the_connect_time_cleanup_kills_only_a_renderer_never_what_reused_its_pid() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    let attach: u32 = host.state("pid").unwrap().trim().parse().unwrap();

    // An unrelated process holds the pid an earlier generation recorded.
    // Lives far longer than the test can take at any load (T9: a 30 s one
    // ran out on its own when the suite took 391 s at load 95, and read as
    // killed); killed at the end.
    let mut decoy = Command::new("/bin/sleep")
        .arg("3600")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let conn = KEY.rsplit_once('-').unwrap().0;
    let moshpit = host.home.join(".moshpit");
    fs::write(
        moshpit.join(format!("mosh-{conn}-old.pid")),
        format!("{}\n", decoy.id()),
    )
    .unwrap();
    fs::write(moshpit.join(format!("mosh-{conn}-old.target")), "term_gone").unwrap();
    // And one names a pid nobody has, and one holds no number at all.
    fs::write(moshpit.join(format!("mosh-{conn}-dead.pid")), "999999\n").unwrap();
    fs::write(moshpit.join(format!("mosh-{conn}-dead.target")), "term_x").unwrap();
    fs::write(
        moshpit.join(format!("mosh-{conn}-junk.pid")),
        "1; echo pwned\n",
    )
    .unwrap();

    // The next connect's cleanup: every earlier generation of this
    // connection, the live loop here among them.
    let said = host.exec(&launch::stale_renderer_cleanup_command(conn));
    assert!(said.is_empty(), "{}", String::from_utf8_lossy(&said));
    sleep(Duration::from_millis(300));

    let ended = decoy.try_wait().unwrap();
    assert!(
        ended.is_none(),
        "the cleanup killed an unrelated process that had reused a renderer's pid: {ended:?}"
    );
    // Killed is gone once its parent, the loop's shell, has reaped it,
    // which a loaded machine can put off past any fixed pause (until then
    // `kill -0` finds it); one the cleanup missed never goes.
    let alive = || {
        Command::new("kill")
            .arg("-0")
            .arg(attach.to_string())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let deadline = Instant::now() + EVENTUALLY;
    while alive() {
        assert!(
            Instant::now() < deadline,
            "a renderer's attach outlived the cleanup"
        );
        sleep(Duration::from_millis(20));
    }
    let left: Vec<String> = fs::read_dir(&moshpit)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(conn))
        .collect();
    assert!(left.is_empty(), "state files left: {left:?}");
    let _ = decoy.kill();
    let _ = decoy.wait();
}

/// A switch to a terminal the server no longer lists (its pane closed since
/// the app last read the tree) used to write the target and kill the attach
/// on screen; the marker came as the next wrapper wrote its pid, and herdr
/// refused that attach ~350 ms later.
#[test]
fn a_switch_to_a_terminal_the_server_no_longer_lists_is_refused_and_the_loop_left_alone() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    let attach = host.state("pid").unwrap();

    let said = host.retarget("term_gone");
    assert!(
        !launch::retarget_landed(&said),
        "a switch to a closed pane counted"
    );
    assert!(
        launch::retarget_missing(&said),
        "{}",
        String::from_utf8_lossy(&said)
    );
    sleep(Duration::from_millis(300));
    assert_eq!(
        host.state("pid"),
        Some(attach),
        "the attach on screen was killed"
    );
    assert_eq!(host.state("target").as_deref(), Some("term_a"));
    assert_eq!(host.attaches(), ["term_a"]);
    assert!(!path_has(&host.home, "stop"), "the renderer went down");

    // A server that doesn't answer is no answer: the switch goes ahead.
    host.event("no-server");
    assert!(launch::retarget_landed(&host.retarget("term_b")));
}

/// The cover over a switch lifts once the screen has been quiet a moment;
/// what the loop itself prints between the kill and the new pane's paint
/// counted as that paint. bash, macOS's sh (and Linux's where sh is bash),
/// reports the killed attach — "Killed: 9" — about 10 ms after the kill,
/// while herdr repaints about 250 ms later: the cover lifted on the previous
/// pane for the difference.
#[test]
fn a_switch_prints_nothing_of_the_loops_own_between_the_panes_but_a_clear_screen() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("A's paint", || host.screen().contains("PAINT term_a"));
    let before = host.screen().len();
    assert!(launch::retarget_landed(&host.retarget("term_b")));
    host.wait("B's paint", || host.screen().contains("PAINT term_b"));
    sleep(Duration::from_millis(100));
    let screen = host.screen();
    let between = &screen[before..screen.find("PAINT term_b").unwrap()];
    assert!(
        !between.contains("Killed"),
        "the loop's shell reported the kill on the screen: {between:?}"
    );
    // The pane being left is wiped as soon as it is killed: whenever a
    // cover lifts, it is never onto the previous pane's screen.
    assert_eq!(
        between, "\u{1b}[0m\u{1b}[H\u{1b}[2J",
        "the loop's own output between the panes"
    );
}

/// Following the shared focus always took over: the loop ran every attach
/// with `--takeover`, so a second device lost its pane whenever this one
/// followed the focus onto it. A follow is a plain attach now, which herdr
/// refuses while another client holds the terminal; the loop parks on it,
/// and the retarget says so as the watch would. The person's pick takes it.
#[test]
fn a_follow_never_takes_a_pane_another_client_holds_and_the_persons_pick_does() {
    let mut host = Host::new();
    host.boot();
    assert!(launch::retarget_landed(&host.retarget("term_a")));
    host.wait("the attach", || host.state("pid").is_some());
    host.wait("A's paint", || host.screen().contains("PAINT term_a"));

    // Another device's client holds term_b; the shared focus moves there.
    host.event("held-term_b");
    let said = host.retarget_refused("term_b");
    assert!(
        !launch::retarget_landed(&said),
        "a follow took the pane from the device that has it: {:?}",
        host.attach_flags()
    );
    let Some(RendererStop::Parked {
        status,
        terminal,
        said: words,
    }) = launch::renderer_stop(&said)
    else {
        panic!("not said: {}", String::from_utf8_lossy(&said));
    };
    assert_eq!(terminal, "term_b");
    assert!(words.contains("already has an attached client"), "{words}");
    assert_eq!(
        launch::renderer_stop_kind(status, &words, true, true),
        StopKind::TakenOver
    );
    assert!(
        host.events.join("held-term_b").exists(),
        "the other client lost the pane"
    );
    assert_eq!(
        host.attach_flags(),
        [
            ("term_a".to_owned(), "plain".to_owned()),
            ("term_b".to_owned(), "plain".to_owned())
        ]
    );
    // Parked: nothing typed reaches a shell, and nothing retries on its own.
    sleep(Duration::from_millis(400));
    assert!(host.loop_running(), "the loop ended: {}", host.screen());
    assert!(!host.screen().contains("LOGIN-SHELL"));
    assert_eq!(host.attach_flags().len(), 2, "attached again on its own");

    // "Open here": the person's word takes it, and the loop comes round.
    let said = host.take("term_b");
    assert!(
        launch::retarget_landed(&said),
        "{}",
        String::from_utf8_lossy(&said)
    );
    host.wait("the take", || host.attach_flags().len() == 3);
    assert_eq!(
        host.attach_flags()[2],
        ("term_b".to_owned(), "--takeover".to_owned())
    );
    host.wait("B's paint", || host.screen().contains("PAINT term_b"));
    assert!(
        !host.events.join("held-term_b").exists(),
        "the other client still holds it"
    );
}
