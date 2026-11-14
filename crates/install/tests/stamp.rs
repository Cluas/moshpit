//! The app's `HostStampQuotingTests` and the stamp-script half of
//! `IslandContentTests`, moved here with the Swift they tested: the hook
//! self-test quotes the pane (B109; one command since B355), run through a
//! real shell; the stamp script derives a title from each hook event.

use std::process::Command;

use moshpit_install::{InstallComponent, commands, scripts};

/// The script the self-test runs is the same whatever the pane; the pane
/// reaches it as its one argument (B109, and B355's one command).
#[test]
fn any_pane_reaches_the_inner_shell_as_its_one_argument() {
    let script = |command: &str| command[..command.rfind("' sh ").unwrap()].to_owned();
    let plain = commands::stamp_test("%3", "/tmp/s", "0123456789abcdef");
    assert!(plain.ends_with("' sh '%3' 2>/dev/null || true"), "{plain}");
    for pane in [
        "%3; touch x",
        "",
        "a b",
        "$(id)",
        "`id`",
        "*",
        "%3\n",
        "%3'\u{301}",
        "é",
    ] {
        let command = commands::stamp_test(pane, "/tmp/s", "0123456789abcdef");
        assert_eq!(script(&command), script(&plain), "{pane:?}");
        assert!(
            command.ends_with(&format!(
                "' sh {} 2>/dev/null || true",
                commands::quote(pane)
            )),
            "{pane:?}"
        );
    }
    assert!(
        commands::stamp_test("%3; touch x 'q'", "/tmp/s", "0123456789abcdef")
            .ends_with(r#"sh '%3; touch x '\''q'\''' 2>/dev/null || true"#)
    );
}

/// Run for real against a `tmux` stand-in that records the target it is
/// handed: every call gets the whole pane, and nothing else runs.
#[test]
fn through_a_real_shell_tmux_gets_the_whole_pane_every_time_and_nothing_else_runs() {
    for pane in [
        "%3; touch INJECTED",
        "%3; touch INJECTED 'q'",
        "a b",
        "%3'\u{301}; touch INJECTED; '",
        "%3",
    ] {
        let dir = std::env::temp_dir().join(format!(
            "w15b-stamp-{}-{}",
            std::process::id(),
            moshpit_seal::random::hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tmux = dir.join("tmux");
        std::fs::write(&tmux, "#!/bin/sh\nprintf '[%s]' \"$4\" >> \"$OUT\"\n").unwrap();
        Command::new("chmod").arg("+x").arg(&tmux).status().unwrap();
        let out = dir.join("out");
        let command = commands::stamp_test(pane, "/tmp/s", "0123456789abcdef");
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("cd '{}' && {command}", dir.display()))
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("HOME", &dir)
            .env("OUT", &out)
            .status()
            .unwrap();
        assert!(status.success(), "{pane:?}");
        let seen = std::fs::read_to_string(&out).unwrap_or_default();
        // Five reads, the read-back and five restores (no stamp script in
        // this HOME).
        assert_eq!(seen, format!("[{pane}]").repeat(11), "{pane:?}");
        assert!(!dir.join("INJECTED").exists(), "{pane:?} ran a command");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn the_stamp_script_derives_a_title_from_each_hook_event() {
    let script = scripts::body(&InstallComponent::Stamp).unwrap();
    // One jq branch per event Moshpit registers, plus Codex's own and
    // Gemini CLI's names for the same moments (B381).
    assert!(script.contains("    PreToolUse|PermissionRequest|BeforeTool)"));
    assert!(script.contains("    Notification)"));
    assert!(script.contains("    UserPromptSubmit|BeforeAgent)"));
    // Gemini CLI's prompt is remembered for the finish card too.
    assert!(script.contains("[ \"$EV\" = \"BeforeAgent\" ]"));
    // A title is cleared when none is derivable, so a stale one never
    // lingers: in tmux, by unsetting the pane's option.
    assert!(script.contains("  del title\n"));
    assert!(script.contains("del() { tmux set -pu -t \"$TMUX_PANE\" \"@moshpit_$1\""));
}

/// Marks that join an apostrophe before them into one Swift `Character`
/// (the app's `ShellQuoteCallSiteTests`, B108).
const MARKS: [&str; 5] = ["\u{301}", "\u{FE0F}", "\u{1F3FB}", "\u{200D}", "\u{903}"];

fn hostile(mark: &str) -> String {
    format!("it'{mark}; touch INJECTED; '")
}

#[test]
fn the_quoter_quotes_the_apostrophe_whatever_is_attached_to_it() {
    for mark in MARKS {
        assert_eq!(
            commands::quote(&hostile(mark)),
            format!("'it'\\''{mark}; touch INJECTED; '\\'''"),
            "{mark:?}"
        );
    }
}

#[test]
fn the_self_test_quotes_the_hosts_tmux_socket_path() {
    for mark in MARKS {
        let socket = format!("/tmp/{}", hostile(mark));
        let tmux = format!("'/tmp/it'\\''{mark}; touch INJECTED; '\\'',0,0'");
        assert!(
            commands::stamp_test("%3", &socket, "0123456789abcdef")
                .starts_with(&format!("TMUX={tmux} sh -c ")),
            "{mark:?}"
        );
    }
}
