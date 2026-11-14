//! The command lines that start herdr on the host. Ported from
//! `Tests/MoshpitTests/Services/Herdr/HerdrLaunchTests.swift` (the mosh
//! raw-attach renderer: a loop that keeps `herdr terminal attach
//! <terminal_id>` pointed at a target file — `--takeover` only for the
//! terminal the person chose — and the sidecar command that redirects it),
//! plus the attach and daemon lines and the quoting every herdr command
//! line relies on.

use std::process::Command;

use moshpit_herdr::launch;

const UUID: &str = "6F9619FF-8B86-D011-B42D-00C04FC964FF";

fn key() -> String {
    launch::mosh_renderer_key(UUID, "abc12345")
}

/// One word as a POSIX shell reads it: single quotes literal, a backslash
/// outside them taking the next character. `None` for anything else.
fn posix_word(word: &str) -> Option<String> {
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '\'') => quoted = false,
            (true, c) => out.push(c),
            (false, '\'') => quoted = true,
            (false, '\\') => out.push(chars.next()?),
            (false, _) => return None,
        }
    }
    (!quoted).then_some(out)
}

/// Whether a quoted word holds a backslash inside its single quotes, which
/// fish would read as an escape (B138).
fn backslash_inside_quotes(word: &str) -> bool {
    let mut quoted = false;
    for c in word.chars() {
        match c {
            '\'' => quoted = !quoted,
            '\\' if quoted => return true,
            _ => {}
        }
    }
    false
}

#[test]
fn the_loop_is_one_line_waits_for_a_target_and_records_the_attach_pid() {
    let key = key();
    let typed = launch::raw_attach_loop_command(&key, None);
    // Typed into a live shell: a raw newline would submit half a loop.
    assert!(!typed.contains('\n'));
    // One word for the login shell, the loop for sh (B138): typed into fish,
    // `while … do … done` was a syntax error.
    let word = typed.strip_prefix("sh -c ").expect("under sh -c");
    assert!(!backslash_inside_quotes(word));
    let line = posix_word(word).expect("one word");
    // Waits for the sidecar to publish the first target.
    assert!(line.contains("if [ -n \"$tid\" ]"));
    assert!(line.contains("sleep 0.5"));
    // The attach must be the tty's FOREGROUND process; the pid comes from a
    // wrapper writing its own $$ before exec'ing into the attach.
    assert!(line.contains("sh -c \"echo \\$\\$ >"));
    assert!(line.contains(&format!(
        "exec herdr terminal attach \\\"\\$0\\\" \\$k 2>\\\"$HOME/.moshpit/mosh-{key}.err\\\""
    )));
    // `--takeover` only for the terminal the take file names, the one the
    // person chose: following the focus took over too, and a second device
    // lost its pane whenever this one followed the focus onto it.
    assert!(line.contains(&format!(
        "k=; [ \\\"\\$(cat \\\"$HOME/.moshpit/mosh-{key}.take\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] && k=--takeover; "
    )));
    assert!(!line.contains("\\$0\\\" --takeover"));
    // B208: once its pid is in the file, the wrapper reads the target again
    // and goes round (137, as if killed) if it has moved on. A switch
    // landing between the loop's read and the pid write killed a pid
    // already dead, and the loop attached the pane being LEFT.
    let recheck = format!(
        "[ \\\"\\$(cat \\\"$HOME/.moshpit/mosh-{key}.target\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] || exit 137"
    );
    let pid_write = line.find("echo \\$\\$ >").expect("the pid write");
    let reread = line
        .find(&recheck)
        .unwrap_or_else(|| panic!("no second read of the target: {line}"));
    let attach = line.find("exec herdr terminal attach").expect("the attach");
    assert!(
        pid_write < reread && reread < attach,
        "the pid first, then the second read, then the attach"
    );
    assert!(!line.contains("& echo $!"));
    assert!(line.contains(&format!("mosh-{key}.pid")));
    assert!(line.contains(&format!("mosh-{key}.target")));
    // The probe's PATH trick rides along, exported (an assignment prefix
    // does not survive exec).
    assert!(line.contains("export PATH="));
    // The anti-storm rule: a signal death re-attaches; an attach that ended
    // on its own never grabs the pane back. It used to break the loop and
    // leave the login shell to run what was typed next; the loop parks now
    // — its stop record written, the screen blanked — until a retarget's go
    // file wakes it, and goes when its target is removed. The pid file goes
    // with its attach (B208).
    assert!(line.contains(&format!(
        "s=$?; rm -f \"$HOME/.moshpit/mosh-{key}.pid\"; printf \"\\033[0m\\033[H\\033[2J\"; \
         if [ $s -lt 128 ]; then echo \"$s $tid\" > \"$HOME/.moshpit/mosh-{key}.stop\";"
    )));
    assert!(line.contains(&format!(
        "while [ ! -e \"$HOME/.moshpit/mosh-{key}.go\" ]; do \
         [ -e \"$HOME/.moshpit/mosh-{key}.target\" ] || exit 0; sleep 0.1; done"
    )));
    assert!(!line.contains("|| break"));
    // The go file is consumed past the second read, by the attach it is for.
    let consume = line
        .find(&format!("rm -f \\\"$HOME/.moshpit/mosh-{key}.go\\\""))
        .expect("the go file consumed");
    assert!(reread < consume && consume < attach);
    // The loop's own pid for the watch, and what was typed while no pane
    // showed read away (raw, non-blocking) before the next attach.
    assert!(line.contains(&format!(
        "echo $$ > \"$HOME/.moshpit/mosh-{key}.loop\"; f=1;"
    )));
    assert!(line.contains("stty -icanon min 0 time 0 2>/dev/null && cat >/dev/null"));
    // Nothing of the loop's own between two panes but a clear screen: its
    // stderr goes nowhere (bash's "Killed: 9" lifted the switch veil on the
    // pane being left), and the pane being left is wiped as its attach ends.
    assert!(line.starts_with("mkdir -p \"$HOME/.moshpit\"; exec 2>/dev/null; "));
    assert!(line.contains(&format!(
        "s=$?; rm -f \"$HOME/.moshpit/mosh-{key}.pid\"; printf \"\\033[0m\\033[H\\033[2J\"; if [ $s -lt 128 ]"
    )));
    // The re-attach pause is small but not zero.
    assert!(line.contains("sleep 0.05"));
    assert!(!line.contains("sleep 0.3"));
}

#[test]
fn cleanup_retires_every_previous_generation_including_the_immortal_360_361_layout() {
    let command = launch::stale_renderer_cleanup_command(UUID);
    assert!(command.contains(&format!("mosh-{UUID}\".target")));
    assert!(command.contains(&format!("mosh-{UUID}-\"*.target")));
    assert!(command.contains(&format!("mosh-{UUID}\".pid")));
    assert!(command.contains(&format!("mosh-{UUID}-\"*.pid")));
    // Targets are removed BEFORE the kill.
    assert!(command.find(".target").unwrap() < command.find("kill -9").unwrap());
    // Never fails the bootstrap channel it runs on.
    assert!(command.ends_with("true"));
    // Only a renderer is killed: its command line `terminal attach` and the
    // generation's terminal id (tests/launch_host.rs runs it).
    assert!(command.contains("case \"$a\" in *\"terminal attach\"*\"$t\"*) kill -9 \"$p\""));
    assert!(!command.contains("kill -9 $(cat"), "a pid killed unseen");
}

#[test]
fn a_custom_herdr_path_is_trusted_verbatim_in_the_loop() {
    let typed = launch::raw_attach_loop_command(&key(), Some("/opt/herdr"));
    let line = posix_word(typed.strip_prefix("sh -c ").unwrap()).unwrap();
    assert!(line.contains("exec /opt/herdr terminal attach"));
    assert!(!line.contains("export PATH="));
}

/// The retarget's wait for the loop's next attach (2 s in the app, 40 looks
/// 50 ms apart) and its settle for a refusal (600 ms, 12 looks), as they
/// always were; any length for a test, never none. The watch's 60 s too.
#[test]
fn the_retargets_wait_and_settle_are_the_apps_unless_asked() {
    let key = key();
    // The number after the first `while [ $<var> -lt `.
    let looks = |command: &str, var: &str| {
        let head = format!("while [ ${var} -lt ");
        let at = command.find(&head).expect("the loop") + head.len();
        command[at..].split(' ').next().unwrap().to_owned()
    };
    let ms = std::time::Duration::from_millis;
    assert_eq!(launch::RETARGET_WAIT, std::time::Duration::from_secs(2));
    assert_eq!(launch::RETARGET_SETTLE, ms(600));
    let app = launch::retarget_command("t", &key, None, false);
    assert_eq!(looks(&app, "i"), "40");
    assert_eq!(looks(&app, "j"), "12");
    for (span, expected) in [
        (30_000, "600"),
        (2_000, "40"),
        (120, "3"),
        (1, "1"),
        (0, "1"),
    ] {
        let command = launch::retarget_command_within("t", &key, None, false, ms(span), ms(span));
        assert_eq!(looks(&command, "i"), expected, "{span} ms");
        assert_eq!(looks(&command, "j"), expected, "{span} ms");
    }
    assert_eq!(
        launch::retarget_command("t", &key, Some("/opt/herdr"), true),
        launch::retarget_command_within(
            "t",
            &key,
            Some("/opt/herdr"),
            true,
            launch::RETARGET_WAIT,
            launch::RETARGET_SETTLE
        )
    );
    assert_eq!(
        launch::RAW_ATTACH_WATCH_WAIT,
        std::time::Duration::from_secs(60)
    );
    assert_eq!(looks(&launch::raw_attach_watch_command(&key), "i"), "300");
    assert_eq!(
        looks(
            &launch::raw_attach_watch_command_within(&key, ms(300_000)),
            "i"
        ),
        "1500"
    );
}

#[test]
fn retarget_publishes_the_terminal_id_bounces_the_attach_and_says_when_the_loop_came_round() {
    let key = key();
    let command = launch::retarget_command("term_abc123", &key, None, true);
    // A terminal the server no longer lists is refused before the loop is
    // touched (the herdr the loop runs is asked), and a server that doesn't
    // answer lets the switch go ahead.
    let check = command
        .find("case \"$s\" in *'\"result\"'*) case \"$s\" in *\"\\\"$t\\\"\"*) ;; *) echo MOSHPIT_RETARGET_MISSING; exit 0 ;; esac ;; esac")
        .expect("the check of the terminal");
    assert!(command.starts_with("t='term_abc123'; s=$(PATH="));
    assert!(command.contains(" herdr api snapshot 2>/dev/null)"));
    assert!(
        check < command.find("printf '%s'").unwrap(),
        "the check before the target is written"
    );
    assert!(
        launch::retarget_command("t", &key, Some("/opt/herdr"), false)
            .contains("s=$(/opt/herdr api snapshot 2>/dev/null)")
    );
    assert!(launch::retarget_missing(b"MOSHPIT_RETARGET_MISSING\n"));
    assert!(!launch::retarget_missing(b"MOSHPIT_RETARGETED\n"));
    assert!(command.contains("printf '%s' 'term_abc123' >"));
    assert!(command.contains(&format!("mosh-{key}.target")));
    // The go file wakes a parked loop; a loop that is gone is said.
    assert!(command.contains(&format!(": > \"$HOME/.moshpit/mosh-{key}.go\"")));
    assert!(command.contains(&format!(
        "[ -s \"$HOME/.moshpit/mosh-{key}.loop\" ] || echo {}",
        launch::RENDERER_GONE_MARKER
    )));
    // -9, not TERM: herdr exits gracefully (<128) on TERM.
    assert!(command.contains(&format!(
        "old=$(cat \"$HOME/.moshpit/mosh-{key}.pid\" 2>/dev/null); case \"$old\" in"
    )));
    // B208: the exec coming back is no proof the loop shows the target — a
    // loop that is gone (evicted, its pane closed) leaves a bare shell under
    // mosh. The marker is printed only once the loop has started its next
    // attach: a pid in the file other than the one killed, and alive.
    // Bounded: 40 looks, 50 ms apart.
    assert!(command.contains("[ -n \"$new\" ] && [ \"$new\" != \"$old\" ] && kill -0 \"$new\""));
    assert!(command.contains(&format!("echo {}", launch::RETARGET_LANDED_MARKER)));
    assert!(command.contains("[ $i -lt 40 ]"));
    assert!(command.contains("sleep 0.05"));
    // Only an attach is killed (a pid file a dead loop left names anything).
    assert!(command.contains("case \"$a\" in *\"terminal attach\"*) kill -9 \"$old\""));
    let kill = command.find("kill -9 \"$old\"").unwrap();
    let look = command.find("while [ $i -lt 40 ]").unwrap();
    assert!(
        kill < look,
        "the kill first, then the wait for the next attach"
    );
    // A dead pid file or a loop that never comes round must not fail the
    // exec: the caller reads the marker, not the status.
    assert!(command.ends_with("; true"));
    assert!(launch::retarget_landed(b"MOSHPIT_RETARGETED\n"));
    assert!(!launch::retarget_landed(b""));
    assert!(!launch::retarget_landed(b"kill: usage\n"));
    // Untrusted input is quoted: ids come from the server.
    let hostile = launch::retarget_command("a'; rm -rf /", &key, None, true);
    assert!(hostile.contains(r"'a'\''; rm -rf /'"));
}

#[test]
fn a_retarget_takes_the_pane_over_only_on_the_persons_word() {
    let key = key();
    let take = format!("\"$HOME/.moshpit/mosh-{key}.take\"");
    let chosen = launch::retarget_command("term_abc123", &key, None, true);
    let follow = launch::retarget_command("term_abc123", &key, None, false);
    // The person's pick names its terminal in the take file, a follow
    // removes it; before the target, so a wrapper that reads the new
    // target reads the take file that goes with it.
    let target = format!("mosh-{key}.target");
    let named = chosen
        .find(&format!("printf '%s' 'term_abc123' > {take}; "))
        .expect("the take file named");
    assert!(named < chosen.find(&target).unwrap());
    let removed = follow
        .find(&format!("rm -f {take}; "))
        .expect("the take file removed");
    assert!(removed < follow.find(&target).unwrap());
    // A follow counts once herdr has had its moment to refuse the plain
    // attach (~260 ms, measured): parked by then, the stop record is
    // printed as the watch prints it, never the landing.
    assert!(follow.contains(&format!(
        "j=0; while [ $j -lt 12 ]; do if [ -s \"$HOME/.moshpit/mosh-{key}.stop\" ]; then \
         echo {}; cat \"$HOME/.moshpit/mosh-{key}.stop\" \"$HOME/.moshpit/mosh-{key}.err\" 2>/dev/null; \
         exit 0; fi; sleep 0.05; j=$((j+1)); done; echo {}",
        launch::RENDERER_STOPPED_MARKER,
        launch::RETARGET_LANDED_MARKER
    )));
    assert!(
        !chosen.contains("$j"),
        "the person's pick waits for nothing more"
    );
    // herdr's refusal of a plain attach to a held terminal reads as the
    // pane being on another device, as a takeover does.
    let held = "herdr: server shut down: terminal attach failed: terminal term_1 already has an attached client; retry with --takeover";
    assert!(launch::held_elsewhere(held));
    assert_eq!(
        launch::renderer_stop_kind(1, held, true, true),
        launch::StopKind::TakenOver
    );
    assert_eq!(
        launch::renderer_stop_kind(1, held, false, true),
        launch::StopKind::Gone
    );
    assert!(launch::stale_renderer_cleanup_command(UUID).contains("\"*.take; true"));
}

#[test]
fn the_check_of_the_pane_on_screen_reads_the_target_and_the_attach_pid_and_kills_nothing() {
    let key = key();
    let command = launch::raw_attach_check_command("term_abc123", &key);
    // B208: a reply for the pane already on screen asks whether the loop is
    // still on it — the target file names it, the attach in the pid file is
    // alive (the file goes with its attach) — and never bounces it: a
    // retarget would take the pane back from whoever holds it.
    assert!(command.contains(&format!(
        "[ \"$(cat \"$HOME/.moshpit/mosh-{key}.target\" 2>/dev/null)\" = 'term_abc123' ]"
    )));
    assert!(command.contains(&format!(
        "kill -0 \"$(cat \"$HOME/.moshpit/mosh-{key}.pid\" 2>/dev/null)\""
    )));
    assert!(command.contains(&format!("echo {}", launch::RAW_ATTACH_UP_MARKER)));
    assert!(!command.contains("kill -9"));
    assert!(!command.contains("printf"));
    assert!(command.ends_with("; true"));
    assert!(launch::raw_attach_up(b"MOSHPIT_ATTACHED\n"));
    assert!(!launch::raw_attach_up(b""));
    // Ids come from the server: quoted.
    let hostile = launch::raw_attach_check_command("a'; rm -rf /", &key);
    assert!(hostile.contains(r"= 'a'\''; rm -rf /' ]"));
}

#[test]
fn the_probe_prints_the_marker_only_when_terminal_attach_exists() {
    let probe = launch::raw_attach_probe_command(None);
    assert!(probe.contains("terminal attach --help"));
    assert!(probe.contains("&& echo MOSHPIT_RAW_ATTACH_OK"));
    assert!(probe.contains(">/dev/null 2>&1"));
}

#[test]
fn the_attach_command_carries_the_probes_path_unless_the_user_named_a_binary() {
    assert_eq!(
        launch::attach_command(None),
        "PATH=\"$PATH:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin\" herdr"
    );
    assert_eq!(launch::attach_command(Some("/opt/herdr")), "/opt/herdr");
    // An empty custom path is no custom path.
    assert_eq!(
        launch::attach_command(Some("")),
        launch::attach_command(None)
    );
    assert!(launch::command("api snapshot", None).ends_with(" herdr api snapshot"));
}

#[test]
fn the_daemon_detaches_from_the_exec_channel_it_runs_on() {
    let daemon = launch::daemon_command(None);
    assert!(daemon.contains("nohup herdr server >/dev/null 2>&1 </dev/null &"));
    assert!(daemon.starts_with("PATH="));
    assert_eq!(
        launch::daemon_command(Some("/opt/herdr")),
        "nohup /opt/herdr server >/dev/null 2>&1 </dev/null &"
    );
}

/// The PATH list is `HostCapabilities.extraPathDirs`, kept in step with the
/// app's (the harness build checks the same).
#[test]
fn the_path_dirs_are_the_apps() {
    let swift = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../Moshpit/Services/HostCapabilities.swift"
    ));
    // Only checked where the app's sources are around (not in an export of
    // the crate alone).
    if let Ok(swift) = swift {
        assert!(swift.contains(&format!("\"{}\"", launch::EXTRA_PATH_DIRS)));
    }
}

/// What a POSIX shell makes of `quote(value)`: one argument, the value.
fn through_sh(value: &str) -> String {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("printf '%s' {}", launch::quote(value)))
        .output()
        .expect("/bin/sh");
    assert!(output.status.success(), "sh failed for {value:?}");
    String::from_utf8(output.stdout).expect("UTF-8")
}

#[test]
fn quoting_survives_the_shell_whatever_the_value() {
    for value in [
        "it's",
        "",
        "plain",
        "''",
        "a'b'c",
        "$(echo pwned)",
        "`id`",
        "; rm -rf ~",
        "x\ny",
        "\\'",
    ] {
        assert_eq!(through_sh(value), value);
    }
}

/// Fix (quote-unicode): Swift's `replacingOccurrences(of: "'")` does a
/// Unicode-aware search that does not see an apostrophe a combining mark, a
/// variation selector or an emoji modifier is attached to, so it isn't
/// escaped and the rest of the value runs as shell commands. Every apostrophe
/// byte is escaped here.
#[test]
fn an_apostrophe_carrying_a_combining_mark_is_still_escaped() {
    for value in [
        "x'\u{301}; echo INJECTED; '",
        "x'\u{fe0f}; echo INJECTED; '",
        "x'\u{1f3fb}; echo INJECTED; '",
        "\u{600}'; echo INJECTED; '",
    ] {
        assert_eq!(
            through_sh(value),
            value,
            "{value:?} broke out of its quotes"
        );
    }
}
