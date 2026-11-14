//! How Moshpit starts herdr on the host: the command lines of
//! `Moshpit/Services/Herdr/HerdrLaunch.swift`.

/// Where herdr's installer and Homebrew put binaries that an interactive
/// login shell's PATH often lacks: `HostCapabilities.extraPathDirs`, which
/// the capability probe searches. Booting with the same PATH prefix keeps
/// "found by the probe" and "runs at the prompt" the same thing.
pub const EXTRA_PATH_DIRS: &str =
    "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin";

fn custom(custom_path: Option<&str>) -> Option<&str> {
    custom_path.filter(|path| !path.is_empty())
}

/// The `herdr` to run: a custom path verbatim (the user vouched for it, so
/// it bypasses the probe and the PATH prefix too), else `herdr` with the
/// probe's PATH dirs appended.
pub fn attach_command(custom_path: Option<&str>) -> String {
    match custom(custom_path) {
        Some(path) => path.to_owned(),
        None => format!("PATH=\"$PATH:{EXTRA_PATH_DIRS}\" herdr"),
    }
}

/// `herdr <subcommand>` for the control channel.
pub fn command(subcommand: &str, custom_path: Option<&str>) -> String {
    format!("{} {subcommand}", attach_command(custom_path))
}

/// Start a headless herdr server on the host, detached: all three
/// descriptors redirected and `nohup`, because this runs on a one-shot exec
/// channel that an inherited stdout would hold open, and whose closing would
/// otherwise HUP the server. Only ever from an explicit "create a session".
pub fn daemon_command(custom_path: Option<&str>) -> String {
    match custom(custom_path) {
        Some(path) => format!("nohup {path} server >/dev/null 2>&1 </dev/null &"),
        None => format!(
            "PATH=\"$PATH:{EXTRA_PATH_DIRS}\" nohup herdr server >/dev/null 2>&1 </dev/null &"
        ),
    }
}

// mosh raw-attach renderer: `herdr terminal attach <terminal_id>` renders ONE
// pane's raw stream to the tty, sized to this client alone, in a loop that
// re-reads its target from a file. `--takeover` only when the person chose
// the pane ([`mosh_take_path`]).

/// State-file key: the connection's id (its `uuidString`: uppercase,
/// hyphenated) and a per-boot nonce. The nonce fences generations: a loop
/// watches only its own target file, so an orphaned loop (mosh-server
/// outlives disconnects by design) idles on a stale file for good.
pub fn mosh_renderer_key(connection_id: &str, nonce: &str) -> String {
    format!("{connection_id}-{nonce}")
}

pub fn mosh_target_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.target")
}

pub fn mosh_pid_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.pid")
}

/// The loop's own pid: whether the loop still runs at all.
pub fn mosh_loop_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.loop")
}

/// Written by the loop when its attach ended on its own and it parked:
/// `<status> <terminal id>`.
pub fn mosh_stop_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.stop")
}

/// The attach's stderr: herdr's last words when it ends on its own.
pub fn mosh_err_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.err")
}

/// Created by every retarget: what wakes a parked loop.
pub fn mosh_go_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.go")
}

/// The terminal the person chose (a tap, a reply for it, "Open here"),
/// written by the retarget that points the loop there: the one attach that
/// may take its pane from another client (`--takeover`). A retarget that
/// follows the shared focus removes it, and its attach is a plain one, which
/// herdr refuses while another client holds the terminal ("… already has an
/// attached client; retry with --takeover", herdr 0.9.5): following the
/// focus never takes a pane from the device that has it.
pub fn mosh_take_path(renderer_key: &str) -> String {
    format!("$HOME/.moshpit/mosh-{renderer_key}.take")
}

/// The renderer loop, typed into the mosh shell (append `\r` at the call
/// site). One line; waits for the sidecar to publish the first target.
///
/// The attach runs in the foreground (a backgrounded one renders but never
/// gets keys); its pid is written by a wrapper that `exec`s into it, so the
/// pid is the attach's, and its stderr — herdr's last words — goes to the
/// err file. A retarget kills it with SIGKILL (status ≥ 128) and the loop
/// attaches again.
///
/// An attach that exits on its own (another client's `--takeover`, its pane
/// closed, the server stopped) never grabs the pane back — that is what
/// ended the takeover storms of orphaned loops. It used to BREAK the loop,
/// and nothing started it again: mosh showed the bare login shell and
/// whatever was typed or dictated next ran there as a command line. The
/// loop now PARKS: it writes `<status> <terminal id>` to the stop file,
/// blanks the screen and waits — still the tty's foreground process, so
/// nothing typed reaches a shell — for a retarget's go file. The app
/// decides ([`raw_attach_watch_command`], [`renderer_stop_kind`]): the pane
/// is on another device → said, taken back only by the person; the pane
/// closed or herdr restarted → the focused pane, by itself. An orphan's go
/// file is never written, so it stays down, and it goes when its target is
/// removed ([`stale_renderer_cleanup_command`]). Leaving the park (and
/// before the first attach), what was typed into the tty meanwhile is
/// discarded (a raw, non-blocking read): it was typed at no pane, and the
/// next attach would read it as its own.
///
/// The wrapper reads the target again once its pid is in the file, and goes
/// round (137, as if killed) when it has moved on (B208): a retarget's kill
/// reads the pid file, which names the attach being left until the next
/// wrapper writes its own, so a switch landing in between killed a pid
/// already dead and the loop attached the pane being LEFT. Any target
/// written after the pid is in the file reaches the wrapper by the kill,
/// any written before by this second read. The go file is consumed only
/// past that read, by the attach it is for: one created later comes with a
/// kill, so a parked loop never finds a stale one. The pid file goes with
/// its attach: between attaches, and once the loop has parked, there is no
/// pid to kill.
///
/// Under `sh -c` (B138): the login shell gets one quoted word, so a fish
/// that has no `while … do … done` (or `$(…)` before 3.4) runs it as sh.
pub fn raw_attach_loop_command(renderer_key: &str, custom_path: Option<&str>) -> String {
    let target = mosh_target_path(renderer_key);
    let pid = mosh_pid_path(renderer_key);
    let looped = mosh_loop_path(renderer_key);
    let stop = mosh_stop_path(renderer_key);
    let err = mosh_err_path(renderer_key);
    let go = mosh_go_path(renderer_key);
    // Double-quoted for the outer shell: $HOME in the paths expands there,
    // `\$\$` and `\$0` reach the wrapper. `$0` carries the terminal id so it
    // never re-enters shell parsing; PATH rides an `export` because an
    // assignment prefix does not survive `exec`.
    // `--takeover` only for the terminal the take file names: the one the
    // person chose. Everything else attaches plainly and is refused while
    // another client holds the terminal; the loop parks on that as on any
    // attach that ended on its own.
    let take = mosh_take_path(renderer_key);
    let takeover = format!(
        "k=; [ \\\"\\$(cat \\\"{take}\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] && k=--takeover; "
    );
    let launch = match custom(custom_path) {
        Some(path) => {
            format!("{takeover}exec {path} terminal attach \\\"\\$0\\\" \\$k 2>\\\"{err}\\\"")
        }
        None => format!(
            "export PATH=\\\"$PATH:{EXTRA_PATH_DIRS}\\\"; {takeover}\
             exec herdr terminal attach \\\"\\$0\\\" \\$k 2>\\\"{err}\\\""
        ),
    };
    let script = format!(
        "mkdir -p \"$HOME/.moshpit\"; exec 2>/dev/null; echo $$ > \"{looped}\"; f=1; while :; do \
         tid=$(cat \"{target}\" 2>/dev/null); \
         if [ -n \"$tid\" ]; then \
         if [ -n \"$f\" ]; then f=; t=$(stty -g 2>/dev/null) \
         && stty -icanon min 0 time 0 2>/dev/null && cat >/dev/null; \
         [ -n \"$t\" ] && stty \"$t\" 2>/dev/null; fi; \
         sh -c \"echo \\$\\$ > \\\"{pid}\\\"; \
         [ \\\"\\$(cat \\\"{target}\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] || exit 137; \
         rm -f \\\"{go}\\\"; \
         {launch}\" \"$tid\"; \
         s=$?; rm -f \"{pid}\"; printf \"\\033[0m\\033[H\\033[2J\"; \
         if [ $s -lt 128 ]; then echo \"$s $tid\" > \"{stop}\"; \
         while [ ! -e \"{go}\" ]; do [ -e \"{target}\" ] || exit 0; sleep 0.1; done; \
         rm -f \"{stop}\"; f=1; fi; \
         else sleep 0.5; fi; sleep 0.05; done"
    );
    format!("sh -c {}", quote(&script))
}

/// Pre-boot cleanup on the bootstrap SSH: for every previous generation of
/// this connection, remove the state files and kill the attach it holds.
/// Both layouts (the nonce-less one of builds 360/361, whose loops never
/// learned to lose, and the nonced one). Each generation's target goes
/// first, then the kill: the orphan wakes to no target and idles (or, parked,
/// goes). SIGKILL, so a graceful exit can't give the old loop a head start
/// on re-reading.
///
/// Only a pid whose command line is that generation's renderer is killed:
/// `terminal attach`, then its target's terminal id (as the attach runs it,
/// and as its wrapper does, the id last). A pid file outlives its loop when
/// the loop dies with its mosh-server (stopped, or the host rebooted), and
/// the system hands the pid to something else: the next connect used to
/// `kill -9` whatever had it now. No target, no number, or a command line
/// that isn't the renderer's: left alone. Linux is read from
/// `/proc/<pid>/cmdline`, other hosts from `ps -o args=`.
pub fn stale_renderer_cleanup_command(connection_id: &str) -> String {
    let base = format!("$HOME/.moshpit/mosh-{connection_id}");
    let args = command_line_of("$p");
    format!(
        "for f in \"{base}\".pid \"{base}-\"*.pid; do \
         [ -f \"$f\" ] || continue; k=${{f%.pid}}; \
         t=$(cat \"$k.target\" 2>/dev/null); rm -f \"$k.target\"; \
         p=$(cat \"$f\" 2>/dev/null); \
         case \"$p\" in ''|*[!0-9]*) continue ;; esac; \
         [ -n \"$t\" ] || continue; \
         {args}\
         case \"$a\" in *\"terminal attach\"*\"$t\"*) kill -9 \"$p\" 2>/dev/null ;; esac; done; \
         rm -f \"{base}\".target \"{base}-\"*.target \"{base}\".pid \"{base}-\"*.pid \
         \"{base}-\"*.go \"{base}-\"*.stop \"{base}-\"*.err \"{base}-\"*.loop \"{base}-\"*.take; true"
    )
}

/// Sets `a` to the command line of process `pid` (a shell word: `$p`), or
/// empty: `/proc` where there is one (Linux, without forking `ps`), `ps -o
/// args=` otherwise (macOS, BSD).
pub fn command_line_of(pid: &str) -> String {
    format!(
        "a=; [ -r \"/proc/{pid}/cmdline\" ] && a=$(tr \"\\000\" \" \" < \"/proc/{pid}/cmdline\" 2>/dev/null); \
         [ -n \"$a\" ] || a=$(ps -o args= -p \"{pid}\" 2>/dev/null); "
    )
}

/// Point the loop at a new terminal: publish the target, bounce the attach
/// (SIGKILL: herdr exits gracefully, below 128, on TERM, which the loop
/// reads as an eviction), and say so once the loop has come round to it
/// ([`retarget_landed`]). Idempotence is the caller's job: each bounce is a
/// visible reattach.
///
/// `takeover`: the person chose this pane (a tap, a reply for it, "Open
/// here"), and its attach takes it from whichever client holds it
/// ([`mosh_take_path`]). Otherwise the retarget follows the shared focus,
/// and its attach is a plain one: following the focus always took over, so
/// a second device lost its pane whenever the other followed the focus onto
/// it. herdr refuses a plain attach to a held terminal ~260 ms after it
/// starts ("… already has an attached client; retry with --takeover",
/// herdr 0.9.5, measured), and the loop parks on it. A plain attach is
/// watched that long past its start before the switch counts: one the loop
/// parked on is said here as the watch would say it ([`renderer_stop`],
/// [`RENDERER_STOPPED_MARKER`] and the stop record), so nothing typed for
/// the pane goes into an attach about to be refused. The take file is
/// written before the target: a wrapper that reads the new target reads the
/// take file that goes with it.
///
/// The exec coming back is no proof of anything (B208): a loop that is gone
/// — its attach evicted by another client's `--takeover`, its pane closed,
/// its server stopped — leaves a bare login shell under mosh, and the write
/// and the kill succeed all the same, so a reply sent "to the pane" was
/// typed into that shell as a command line. The exec waits, at most
/// [`RETARGET_WAIT`] (40 looks, 50 ms apart), for the loop to start its next
/// attach (a pid in the file other than the one killed, alive) and prints
/// the marker only then; with the loop's own second read of the target
/// ([`raw_attach_loop_command`]) that attach is this target's. Fix
/// (retarget-unseen-refusal): an attach that ended before a look saw it alive (refused faster than the looks: a
/// loaded host, a herdr that answers at once) is said by its stop record:
/// one there once the go file is gone is the new attach's, since the loop
/// clears a parked one's before it starts the next, and the next one's
/// wrapper takes the go file. It used to be said as nothing, the stop record
/// there, until the wait ran out.
///
/// A terminal the server no longer lists — its pane closed since the last
/// read, a reply for an agent that is gone — is refused before the loop is
/// touched ([`RETARGET_MISSING_MARKER`]), the attach on screen left as it is.
/// It used to be written and the attach killed, and the marker came as soon
/// as the next wrapper had its pid, ~350 ms before herdr refused that attach
/// ("terminal … not found"): the switch counted, what was typed for the pane
/// went to an attach on its way out, and the renderer was down. Asked of the
/// server with `custom_path`, the herdr the loop runs; a server that doesn't
/// answer (not running, another protocol) is no answer, and the switch goes
/// ahead.
pub fn retarget_command(
    terminal_id: &str,
    renderer_key: &str,
    custom_path: Option<&str>,
    takeover: bool,
) -> String {
    retarget_command_within(
        terminal_id,
        renderer_key,
        custom_path,
        takeover,
        RETARGET_WAIT,
        RETARGET_SETTLE,
    )
}

/// How long a retarget waits for the loop's next attach
/// ([`retarget_command`]).
pub const RETARGET_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a plain attach is watched past its start for herdr to refuse
/// it before the switch counts ([`retarget_command`]).
pub const RETARGET_SETTLE: std::time::Duration = std::time::Duration::from_millis(600);

/// How far apart a retarget looks for the loop's next attach.
const RETARGET_LOOK: std::time::Duration = std::time::Duration::from_millis(50);

/// `span` in whole looks `look` apart, rounded up, at least one.
fn looks(span: std::time::Duration, look: std::time::Duration) -> u128 {
    span.as_millis().div_ceil(look.as_millis()).max(1)
}

/// [`retarget_command`] waiting at most `wait` for the loop's next attach,
/// and watching a plain one `settle` for herdr's refusal (both in whole
/// 50 ms looks, at least one); either ends the moment it has its answer. A
/// test whose answer must not depend on how loaded the machine is makes
/// the windows long: the wait, to tell "said at once" from "said when the
/// wait ran out"; the settle, when it expects a refusal, which then can't
/// come too late to count.
pub fn retarget_command_within(
    terminal_id: &str,
    renderer_key: &str,
    custom_path: Option<&str>,
    takeover: bool,
    wait: std::time::Duration,
    settle: std::time::Duration,
) -> String {
    let looks_for_attach = looks(wait, RETARGET_LOOK);
    let looks_for_refusal = looks(settle, RETARGET_LOOK);
    let target = mosh_target_path(renderer_key);
    let pid = mosh_pid_path(renderer_key);
    let looped = mosh_loop_path(renderer_key);
    let go = mosh_go_path(renderer_key);
    let take = mosh_take_path(renderer_key);
    let stop = mosh_stop_path(renderer_key);
    let err = mosh_err_path(renderer_key);
    // Only a renderer is killed: the pid file is the loop's, but one that
    // died with its shell left it behind, its pid free for the system to
    // give to anything.
    let args = command_line_of("$old");
    // The go file wakes a loop that parked; one still attached is woken by
    // the kill. A loop that is gone — killed, its shell the tty's again — is
    // said at once; one that never wrote its pid (its boot line eaten by a
    // shell still starting) at the end of the wait.
    let snapshot = command("api snapshot", custom_path);
    let id = quote(terminal_id);
    let chosen = if takeover {
        format!("printf '%s' {id} > \"{take}\"; ")
    } else {
        format!("rm -f \"{take}\"; ")
    };
    // A plain attach counts once herdr has had its moment to refuse it
    // (the settle): parked by then, the stop record is printed.
    let settle = if takeover {
        String::new()
    } else {
        format!(
            "j=0; while [ $j -lt {looks_for_refusal} ]; do if [ -s \"{stop}\" ]; then \
             echo {RENDERER_STOPPED_MARKER}; cat \"{stop}\" \"{err}\" 2>/dev/null; exit 0; fi; \
             sleep 0.05; j=$((j+1)); done; "
        )
    };
    format!(
        "t={id}; s=$({snapshot} 2>/dev/null); \
         case \"$s\" in *'\"result\"'*) case \"$s\" in *\"\\\"$t\\\"\"*) ;; \
         *) echo {RETARGET_MISSING_MARKER}; exit 0 ;; esac ;; esac; \
         {chosen}printf '%s' {id} > \"{target}\"; : > \"{go}\"; \
         old=$(cat \"{pid}\" 2>/dev/null); \
         case \"$old\" in ''|*[!0-9]*) ;; *) {args}\
         case \"$a\" in *\"terminal attach\"*) kill -9 \"$old\" 2>/dev/null ;; esac ;; esac; \
         i=0; while [ $i -lt {looks_for_attach} ]; do \
         new=$(cat \"{pid}\" 2>/dev/null); \
         if [ -n \"$new\" ] && [ \"$new\" != \"$old\" ] && kill -0 \"$new\" 2>/dev/null; then \
         {settle}echo {RETARGET_LANDED_MARKER}; break; fi; \
         [ ! -e \"{go}\" ] && [ -s \"{stop}\" ] && {{ echo {RENDERER_STOPPED_MARKER}; \
         cat \"{stop}\" \"{err}\" 2>/dev/null; exit 0; }}; \
         l=; read l < \"{looped}\" 2>/dev/null; \
         [ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && {{ echo {RENDERER_GONE_MARKER}; break; }}; \
         sleep 0.05; i=$((i+1)); done; \
         [ -s \"{looped}\" ] || echo {RENDERER_GONE_MARKER}; true"
    )
}

/// What [`retarget_command`] prints when the server doesn't list the
/// terminal: the loop was not touched.
pub const RETARGET_MISSING_MARKER: &str = "MOSHPIT_RETARGET_MISSING";

/// Whether a retarget's output says its terminal is gone.
pub fn retarget_missing(output: &[u8]) -> bool {
    contains(output, RETARGET_MISSING_MARKER.as_bytes())
}

/// Waits, on the sidecar, for the loop to stop showing the attach it is on
/// now: prints [`RENDERER_STOPPED_MARKER`] and the stop record (`<status>
/// <terminal id>`, then herdr's last words) when the attach ended on its own
/// and the loop parked, [`RENDERER_GONE_MARKER`] when the loop itself is
/// gone. Nothing when a switch moved it on (a new pid in the file — the next
/// landing arms the next watch) or after 60 s (300 looks, 200 ms apart): the
/// caller asks again. Reads only.
///
/// Armed after every landing: a takeover by another device, the pane
/// closing or herdr restarting change nothing the control plane's poll sees
/// in time, and until the app knows, what is typed goes to a loop that
/// shows no pane.
pub fn raw_attach_watch_command(renderer_key: &str) -> String {
    raw_attach_watch_command_within(renderer_key, RAW_ATTACH_WATCH_WAIT)
}

/// How long a watch looks before it gives up and the caller asks again
/// ([`raw_attach_watch_command`]).
pub const RAW_ATTACH_WATCH_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// [`raw_attach_watch_command`] looking at most `wait` (in whole 200 ms
/// looks, at least one); it ends the moment it has its answer.
pub fn raw_attach_watch_command_within(renderer_key: &str, wait: std::time::Duration) -> String {
    let looks = looks(wait, std::time::Duration::from_millis(200));
    let pid = mosh_pid_path(renderer_key);
    let looped = mosh_loop_path(renderer_key);
    let stop = mosh_stop_path(renderer_key);
    let err = mosh_err_path(renderer_key);
    format!(
        "w=; read w < \"{pid}\" 2>/dev/null; i=0; while [ $i -lt {looks} ]; do \
         if [ -s \"{stop}\" ]; then echo {RENDERER_STOPPED_MARKER}; \
         cat \"{stop}\" \"{err}\" 2>/dev/null; break; fi; \
         l=; read l < \"{looped}\" 2>/dev/null; \
         [ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && {{ echo {RENDERER_GONE_MARKER}; break; }}; \
         p=; read p < \"{pid}\" 2>/dev/null; \
         [ -n \"$p\" ] && [ \"$p\" != \"$w\" ] && break; \
         sleep 0.2; i=$((i+1)); done; true"
    )
}

/// What [`raw_attach_watch_command`] prints when the loop parked: its
/// attach ended on its own.
pub const RENDERER_STOPPED_MARKER: &str = "MOSHPIT_RENDERER_STOPPED";

/// What the watch and the retarget print when the loop itself is gone.
pub const RENDERER_GONE_MARKER: &str = "MOSHPIT_RENDERER_GONE";

/// How the loop stopped showing its pane, read from a watch's (or a
/// retarget's) output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererStop {
    /// The attach ended on its own and the loop parked: its exit status, the
    /// terminal it was on, and herdr's last words.
    Parked {
        status: i64,
        terminal: String,
        said: String,
    },
    /// The loop itself is gone.
    LoopGone,
}

/// The stop a watch's or a retarget's output reports, if any. Read by bytes:
/// the record is the line after the marker, `<status> <terminal id>` (a
/// status that isn't 1–9 ASCII digits reads 0), and herdr's words are the
/// rest, ASCII whitespace trimmed.
pub fn renderer_stop(output: &[u8]) -> Option<RendererStop> {
    if let Some(at) = find(output, RENDERER_STOPPED_MARKER.as_bytes()) {
        let mut start = at + RENDERER_STOPPED_MARKER.len();
        while start < output.len() && matches!(output[start], b'\n' | b'\r') {
            start += 1;
        }
        let end = output[start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(output.len(), |at| start + at);
        let record = &output[start..end];
        let space = record.iter().position(|&byte| byte == b' ');
        let head = &record[..space.unwrap_or(record.len())];
        let digits = !head.is_empty() && head.len() <= 9 && head.iter().all(u8::is_ascii_digit);
        let status = if digits {
            String::from_utf8_lossy(head).parse().unwrap_or(0)
        } else {
            0
        };
        let terminal = space.map_or(String::new(), |at| {
            String::from_utf8_lossy(&record[at + 1..]).into_owned()
        });
        let said = if end < output.len() {
            &output[end + 1..]
        } else {
            &[][..]
        };
        let blank = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C);
        let from = said
            .iter()
            .position(|byte| !blank(byte))
            .unwrap_or(said.len());
        let to = said
            .iter()
            .rposition(|byte| !blank(byte))
            .map_or(from, |at| at + 1);
        return Some(RendererStop::Parked {
            status,
            terminal,
            said: String::from_utf8_lossy(&said[from..to]).into_owned(),
        });
    }
    contains(output, RENDERER_GONE_MARKER.as_bytes()).then_some(RendererStop::LoopGone)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Why a parked loop's attach ended, as far as the renderer acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKind {
    /// Another client attached the terminal with `--takeover`, or holds it
    /// and a plain attach (one that followed the focus) was refused: the
    /// pane is on another device, never taken without the person.
    TakenOver,
    /// The pane closed, or the server stopped or restarted (terminal ids are
    /// new after a restart): the renderer goes to the focused pane.
    Gone,
    /// Anything else herdr refused with: said, retried on the person's word.
    Refused,
}

/// herdr 0.9.5's last words, measured live: "terminal attach taken over",
/// "terminal attach failed: terminal … already has an attached client;
/// retry with --takeover" (a plain attach to a terminal another client
/// holds), "terminal attach ended: terminal … not found" (its pane closed),
/// "terminal attach failed: terminal … not found" (no such terminal),
/// "server is shutting down" — each behind "herdr: server shut down: ". A
/// shell that couldn't run herdr at all (126, 127) is refused, whatever it
/// said ("herdr: not found" is no pane closing). Words this doesn't know
/// fall back on the tree: a terminal the server no longer lists, or no
/// server, is gone; one it lists was refused.
pub fn renderer_stop_kind(
    status: i64,
    said: &str,
    terminal_listed: bool,
    server_up: bool,
) -> StopKind {
    if status == 126 || status == 127 {
        return StopKind::Refused;
    }
    if said.contains("taken over") || held_elsewhere(said) {
        return if terminal_listed {
            StopKind::TakenOver
        } else {
            StopKind::Gone
        };
    }
    if said.contains("shutting down")
        || (said.contains("terminal attach") && said.contains("not found"))
    {
        return StopKind::Gone;
    }
    if !server_up || !terminal_listed {
        return StopKind::Gone;
    }
    StopKind::Refused
}

/// Whether herdr's words refuse a plain attach because another client
/// holds the terminal (herdr 0.9.5: "terminal attach failed: terminal …
/// already has an attached client; retry with --takeover"; the frame
/// channel's `terminal.closed` says the same).
pub fn held_elsewhere(said: &str) -> bool {
    said.contains("already has an attached client") || said.contains("retry with --takeover")
}

/// What [`retarget_command`] prints once the loop has started the target's
/// attach.
pub const RETARGET_LANDED_MARKER: &str = "MOSHPIT_RETARGETED";

/// Whether a retarget's output says the loop came round to its target — the
/// only thing that makes the switch the pane on screen.
pub fn retarget_landed(output: &[u8]) -> bool {
    contains(output, RETARGET_LANDED_MARKER.as_bytes())
}

/// Whether the loop is still on `terminal_id`: the target file names it and
/// the attach whose pid is in the file is alive. For input to the pane
/// already on screen, which no retarget has just vouched for (B208): the
/// loop ends under it when another client takes the pane over or the server
/// restarts, mosh then shows the login shell, and the pid file is gone with
/// the attach. Prints [`RAW_ATTACH_UP_MARKER`] when it is; reads only, kills
/// nothing — a retarget would take the pane back from whoever holds it now.
///
/// When it isn't because the loop parked or is gone, it says so as the
/// watch would ([`renderer_stop`]): a reply that beat the watch to a
/// takeover must not read as a switch that never landed, which had the next
/// poll point the loop back at the pane — taking it from the device that
/// has it.
pub fn raw_attach_check_command(terminal_id: &str, renderer_key: &str) -> String {
    let target = mosh_target_path(renderer_key);
    let pid = mosh_pid_path(renderer_key);
    let looped = mosh_loop_path(renderer_key);
    let stop = mosh_stop_path(renderer_key);
    let err = mosh_err_path(renderer_key);
    format!(
        "[ \"$(cat \"{target}\" 2>/dev/null)\" = {} ] && \
         kill -0 \"$(cat \"{pid}\" 2>/dev/null)\" 2>/dev/null && echo {RAW_ATTACH_UP_MARKER}; \
         if [ -s \"{stop}\" ]; then echo {RENDERER_STOPPED_MARKER}; cat \"{stop}\" \"{err}\" 2>/dev/null; \
         else l=; read l < \"{looped}\" 2>/dev/null; \
         [ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && echo {RENDERER_GONE_MARKER}; fi; true",
        quote(terminal_id)
    )
}

/// Whether the loop the start line was typed for is running: its pid file
/// names a live process. Asked before the loop is first given a target (and
/// after a reboot): a login shell still starting can swallow a line typed
/// into it — Powerlevel10k's instant prompt flushes pending input, a startup
/// file reads stdin — and the loop never starts; its first switch then said
/// the loop was gone after 2 s, and only the person's Retry typed the line
/// again. Waits up to 1.5 s (30 looks, 50 ms apart) and prints
/// [`LOOP_UP_MARKER`] once it is up; reads only.
pub fn raw_attach_loop_up_command(renderer_key: &str) -> String {
    let looped = mosh_loop_path(renderer_key);
    format!(
        "i=0; while [ $i -lt 30 ]; do l=; read l < \"{looped}\" 2>/dev/null; \
         [ -n \"$l\" ] && kill -0 \"$l\" 2>/dev/null && {{ echo {LOOP_UP_MARKER}; break; }}; \
         sleep 0.05; i=$((i+1)); done; true"
    )
}

/// What [`raw_attach_loop_up_command`] prints once the loop runs.
pub const LOOP_UP_MARKER: &str = "MOSHPIT_LOOP_UP";

/// Whether a loop-up check's output says the loop runs.
pub fn loop_up(output: &[u8]) -> bool {
    contains(output, LOOP_UP_MARKER.as_bytes())
}

/// What [`raw_attach_check_command`] prints when the loop is on its
/// terminal.
pub const RAW_ATTACH_UP_MARKER: &str = "MOSHPIT_ATTACHED";

/// Whether a check's output says the loop is on its terminal.
pub fn raw_attach_up(output: &[u8]) -> bool {
    contains(output, RAW_ATTACH_UP_MARKER.as_bytes())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Prints `MOSHPIT_RAW_ATTACH_OK` when this herdr knows `terminal attach`;
/// anything else keeps the mosh renderer on the full TUI.
pub fn raw_attach_probe_command(custom_path: Option<&str>) -> String {
    format!(
        "{} terminal attach --help >/dev/null 2>&1 && echo MOSHPIT_RAW_ATTACH_OK",
        attach_command(custom_path)
    )
}

/// Single-quote a value for the remote shell, in any login shell: close,
/// escape, reopen for every `'` (`'\''`) and every `\` (`'\\'`). Labels and
/// ids can come from another client entirely, and working directories from
/// whatever runs on the host.
///
/// Fix (quote-unicode): Swift's `replacingOccurrences(of: "'", …)` searches
/// by composed character sequence and does not see an apostrophe that a
/// combining mark, a variation selector or an emoji modifier is attached to;
/// that apostrophe stayed unescaped, closed the quotes, and the rest of the
/// value ran as shell commands. Every apostrophe byte is escaped here.
///
/// B138 (the app's `Shell.quote` the same): fish takes `\'` and `\\` as
/// escapes inside single quotes, so a backslash stays outside them too — a
/// value ending in `\` no longer leaves fish's quote open.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str("'\\''"),
            '\\' => out.push_str("'\\\\'"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}
