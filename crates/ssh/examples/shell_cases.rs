//! Every command line Moshpit hands a login shell, for harness/shells.py
//! (B138): one JSON object per line — `group`, `name`, the `line` a shell
//! gets, the `reference` `sh` must agree with, and for a quoted word the
//! `value` it must print. `old` marks the forms the app sent before, kept
//! for the record of what fish made of them.
//!
//!   cargo run --example shell_cases -p moshpit-ssh

use moshpit_herdr::{control, frame, launch, socket};
use moshpit_install::commands;
use moshpit_ssh::shell;
use moshpit_tmux::boot::{self, SidecarReturn};
use serde_json::{Value, json};

/// Values a quoted word must carry through every shell: fish's escapes
/// inside single quotes (`\'`, `\\`), a value ending in a backslash, and
/// what any shell would expand if it got out of its quotes.
const VALUES: &[&str] = &[
    "",
    "plain",
    "it's",
    "'",
    "''",
    "\\",
    "a\\",
    "\\'",
    "'\\",
    "x'\\''y",
    "a\\\\b",
    "c:\\path\\to\\",
    "a\\nb",
    "$HOME `whoami` $(echo INJECTED) $status $?",
    "*",
    "~root",
    "#comment",
    "-n",
    "--",
    "{a,b}",
    "a b\tc",
    "line one\nline two\n",
    "\r\n",
    "\"double\"",
    "; echo INJECTED",
    "x'; echo INJECTED; '",
    "x\\'; echo INJECTED; '",
    "it'\u{301}s",
    "中文 路径/文件 😀 👍🏽 👨‍👩‍👧",
    "\u{FEFF}bom",
    "\u{1B}[201~",
    "%s %d %%",
];

fn emit(group: &str, name: &str, line: &str, reference: &str, extra: Value) {
    let mut case = json!({"group": group, "name": name, "line": line, "reference": reference});
    if let (Value::Object(case), Value::Object(extra)) = (&mut case, extra) {
        case.extend(extra);
    }
    println!("{case}");
}

/// A command the app runs on an exec channel: the login shell gets
/// `sh -c <payload>`; `sh` reading the payload itself is the reference.
fn exec(name: &str, payload: &str, files: Value) {
    emit(
        "exec",
        name,
        &shell::portable(payload),
        payload,
        json!({"files": files}),
    );
    emit(
        "exec (as it was)",
        name,
        payload,
        payload,
        json!({"files": files, "old": true}),
    );
}

/// A line typed into the login shell (without its `\r`, and the ^U the
/// renderer line starts with: both are the line editor's, not the shell's).
fn typed(name: &str, line: &str, files: Value) {
    let line = line.trim_start_matches('\u{15}').trim_end_matches('\r');
    emit("typed", name, line, line, json!({"files": files}));
}

fn typed_old(name: &str, line: &str, files: Value) {
    let line = line.trim_start_matches('\u{15}').trim_end_matches('\r');
    emit(
        "typed (as it was)",
        name,
        line,
        line,
        json!({"files": files, "old": true}),
    );
}

fn main() {
    // Quoted words: every quoter the app's commands are built with.
    for (index, value) in VALUES.iter().enumerate() {
        let quoted = shell::quote(value);
        assert_eq!(launch::quote(value), quoted, "herdr's quote of {value:?}");
        assert_eq!(
            commands::quote(value),
            quoted,
            "install's quote of {value:?}"
        );
        let line = format!("printf %s {quoted}");
        emit(
            "quoted word",
            &format!("word {index}"),
            &line,
            &line,
            json!({"value": value}),
        );
    }

    // Exec requests.
    let none = json!({});
    exec(
        "herdr snapshot",
        &control::command_line(control::SNAPSHOT, None),
        none.clone(),
    );
    exec(
        "herdr, custom path",
        &control::command_line("workspace list", Some("/opt/bin/herdr")),
        none.clone(),
    );
    exec("herdr server", &launch::daemon_command(None), none.clone());
    exec(
        "herdr probe",
        &launch::raw_attach_probe_command(None),
        none.clone(),
    );
    exec(
        "herdr retarget",
        &launch::retarget_command("term'\\x", "c1-n0", None, false),
        none.clone(),
    );
    exec(
        "herdr retarget, the person's pick",
        &launch::retarget_command("term'\\x", "c1-n0", None, true),
        none.clone(),
    );
    // A loop that parked: the watch says so at its first look.
    exec(
        "herdr watch",
        &launch::raw_attach_watch_command("c1-n0"),
        json!({".moshpit/mosh-c1-n0.stop": "1 term_abc\n", ".moshpit/mosh-c1-n0.err": "herdr: x\n"}),
    );
    exec(
        "herdr check",
        &launch::raw_attach_check_command("term'\\x", "c1-n0"),
        none.clone(),
    );
    // The loop's start line, seen running (its pid file names a live pid).
    exec(
        "herdr loop up",
        &launch::raw_attach_loop_up_command("c1-n0"),
        json!({".moshpit/mosh-c1-n0.loop": "1\n"}),
    );
    exec(
        "herdr cleanup",
        &launch::stale_renderer_cleanup_command("c1"),
        none.clone(),
    );
    exec("install preflight", commands::PREFLIGHT, none.clone());
    exec(
        "install write",
        &commands::write_file("$HOME/.moshpit/x.sh", "700", "ZWNobyBoaQo="),
        none.clone(),
    );
    exec(
        "install read",
        &commands::read_file("$HOME/.moshpit/x.sh"),
        none.clone(),
    );
    exec(
        "install sha",
        &commands::sha256("$HOME/.moshpit/x.sh"),
        none.clone(),
    );
    exec(
        "install stamp test",
        &commands::stamp_test("%3", "/tmp/tmux-1/d'\\x", "0123456789abcdef"),
        none.clone(),
    );
    exec(
        "install stamp test, odd pane",
        &commands::stamp_test("a b'c\\", "/tmp/tmux-1/default", "0123456789abcdef"),
        none.clone(),
    );
    exec(
        "install push test",
        &commands::self_test_push("selftest-ab12", "3F2504E0-4F89-11D3-9A0C-0305E82C3301"),
        none.clone(),
    );
    exec(
        "install pairing phones",
        commands::PAIRING_PHONES,
        none.clone(),
    );
    exec(
        "tmux renderer cleanup",
        &boot::renderer_cleanup_command("c1"),
        none.clone(),
    );
    exec(
        "status of the last command",
        "false; echo \"rc=$?\"",
        none.clone(),
    );
    exec(
        "a script over several lines",
        "set -- a 'b c'\nfor x in \"$@\"; do\n  printf '%s|\\n' \"$x\"\ndone\ncase $# in 2) echo two ;; esac",
        none.clone(),
    );

    // Lines typed into a terminal's shell.
    let target = json!({".moshpit/mosh-c1-n0.target": "term_abc"});
    let preferred = SidecarReturn {
        id: "$3".to_owned(),
        name: Some("dev'\\x".to_owned()),
    };
    // A terminal channel's own program (B267), run as `exec` runs commands.
    exec(
        "tmux control attach program",
        &boot::control_attach_program("tmux", "exec /bin/sh"),
        none.clone(),
    );
    exec(
        "tmux sidecar program",
        &boot::sidecar_attach_program("tmux", Some(&preferred), "exec /bin/sh"),
        none.clone(),
    );
    exec(
        "tmux sidecar program, no session",
        &boot::sidecar_attach_program("tmux", None, "exec /bin/sh"),
        none.clone(),
    );
    exec(
        "herdr push pump program",
        &socket::pump_boot_line(),
        none.clone(),
    );
    typed(
        "tmux renderer",
        &boot::renderer_attach_line("tmux", "dev'\\x", "c1", "n0"),
        none.clone(),
    );
    typed("herdr attach", &launch::attach_command(None), none.clone());
    typed(
        "herdr raw-attach loop",
        &launch::raw_attach_loop_command("c1-n0", None),
        target.clone(),
    );
    typed(
        "herdr frame boot",
        &frame::boot("term'\\x", 80, 24, None, false, "C1"),
        none.clone(),
    );
    // The person's pick, and a dropped connection's ghost to stop first.
    typed(
        "herdr frame boot, the person's pick",
        &frame::boot("term'\\x", 80, 24, None, true, "C1"),
        json!({".moshpit/herdr-frame-C1.pid": "999999\n"}),
    );
    typed(
        "herdr frame start",
        &frame::start("term_x", 80, 24, None, false),
        none.clone(),
    );
    typed(
        "herdr frame start, the person's pick",
        &frame::start("term_x", 80, 24, None, true),
        none.clone(),
    );

    // What the app typed before, for the record.
    typed_old("tmux control attach", "tmux -CC attach", none.clone());
    typed_old(
        "tmux sidecar chain",
        "tmux -CC attach -t '$3' 2>/dev/null || while ! tmux -CC attach 2>/dev/null; do sleep 2; done",
        none.clone(),
    );
    typed_old(
        "tmux renderer",
        "mkdir -p \"$HOME/.moshpit\"; echo $$ > \"$HOME/.moshpit/tmuxr-c1-n0.pid\"; \
         tmux has-session -t '=dev' 2>/dev/null && exec tmux attach -t '=dev'",
        none,
    );
}
