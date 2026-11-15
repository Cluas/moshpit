//! The command lines that put a host's tmux in front of Moshpit: the control
//! client's program over SSH, the mosh sidecar's attach chain, and the mosh
//! renderer's attach line with its leak hygiene — the tmux statics of
//! `SessionHub.swift` (`tmuxControlAttachProgram`, `tmuxSidecarAttachChain`,
//! `tmuxRendererAttachLine`, `tmuxRendererCleanupCommand`).
//!
//! Every one of them only attaches: Moshpit never creates a session on its
//! own and sets nothing on the user's server (the `set -g history-limit`
//! these once started with lowered the user's own limit, B101).
//!
//! The control client and the sidecar are their terminal channel's own
//! program, never lines typed into the login shell the channel would open
//! (B267): a shell still starting swallowed them — a startup that flushes
//! pending input (Powerlevel10k's instant prompt) — and nothing attached.

use moshpit_herdr::launch::{EXTRA_PATH_DIRS, quote};

use crate::format::is_valid_id;

/// The PATH the programs run tmux with: what the command gets, then the
/// directories the capability probe searched — a non-interactive shell
/// lacks Homebrew's and `~/.local/bin`, where the probe found tmux.
fn probed_path() -> String {
    format!("PATH=\"$PATH:{EXTRA_PATH_DIRS}\"")
}

/// The SSH terminal channel's program: control mode on the most recent
/// session, then `then` — the channel's own shell (`exec "$SHELL" -l`) —
/// once tmux's client is gone (no session to attach, a detach, the server
/// gone): the channel stays open as it did when the attach was a line
/// typed into that shell. Under `sh -c` (B138).
pub fn control_attach_program(tmux: &str, then: &str) -> String {
    format!(
        "sh -c {}",
        quote(&format!("{} {tmux} -CC attach; {then}", probed_path()))
    )
}

/// A session target that matches `name` and nothing else: a bare name falls
/// back to a prefix, then a pattern (tmux 3.6a: `-t devo` lands on `devops`).
pub fn exact_session(name: &str) -> String {
    format!("={name}")
}

/// The session a rebuilt sidecar goes back to (`SidecarReturn`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarReturn {
    /// tmux's `$N`, stable across a rename.
    pub id: String,
    pub name: Option<String>,
}

/// The sidecar's attach chain: the preferred session by id, then by exact
/// name, else the most recent session, retried every 2 s while there is
/// none. Only a real session id is spliced in.
///
/// Under `sh -c` (B138, `tmuxSidecarAttachChain` the same): typed into a
/// fish login shell the `while … do … done` was a syntax error, and the
/// sidecar never attached.
pub fn sidecar_attach_chain(tmux: &str, preferred: Option<&SidecarReturn>) -> String {
    format!("sh -c {}", quote(&sidecar_chain(tmux, preferred)))
}

/// The sidecar's terminal channel's program: the chain with tmux found as
/// the probe finds it, then `then`, the channel's own shell, as
/// [`control_attach_program`].
pub fn sidecar_attach_program(tmux: &str, preferred: Option<&SidecarReturn>, then: &str) -> String {
    format!(
        "sh -c {}",
        quote(&format!(
            "{}; {}; {then}",
            probed_path(),
            sidecar_chain(tmux, preferred)
        ))
    )
}

fn sidecar_chain(tmux: &str, preferred: Option<&SidecarReturn>) -> String {
    let mut chain = format!("while ! {tmux} -CC attach 2>/dev/null; do sleep 2; done");
    if let Some(name) = preferred.and_then(|p| p.name.as_deref()) {
        chain = format!(
            "{tmux} -CC attach -t {} 2>/dev/null || {chain}",
            quote(&exact_session(name))
        );
    }
    if let Some(id) = preferred.map(|p| p.id.as_str())
        && id.starts_with('$')
        && is_valid_id(id)
    {
        chain = format!("{tmux} -CC attach -t {} 2>/dev/null || {chain}", quote(id));
    }
    chain
}

/// Where the renderer's pid is recorded.
pub fn renderer_pid_path(connection_id: &str, nonce: &str) -> String {
    format!("$HOME/.moshpit/tmuxr-{connection_id}-{nonce}.pid")
}

/// The line typed into the mosh shell to attach the renderer: ^U first (a
/// shell still starting may hold swallowed input), the shell's pid recorded
/// BEFORE `exec` (exec keeps it, so the file names the tmux client), gated on
/// `has-session` so a vanished session leaves the shell for the retry loop.
///
/// The pid is the login shell's as sh sees it from inside, `$PPID`, written
/// under `sh -c` (B138, `tmuxRendererAttachLine` the same): a fish login
/// shell refuses `$$` outright, and the line never attached. The rest reads
/// alike in every shell, and the `exec` stays the login shell's own.
pub fn renderer_attach_line(tmux: &str, session: &str, connection_id: &str, nonce: &str) -> String {
    let pid = renderer_pid_path(connection_id, nonce);
    let target = quote(&exact_session(session));
    let record = quote(&format!(
        "mkdir -p \"$HOME/.moshpit\"; echo $PPID > \"{pid}\""
    ));
    format!(
        "\u{15}sh -c {record}; {tmux} has-session -t {target} 2>/dev/null && exec {tmux} attach -t {target}\r"
    )
}

/// Takes the phone's mosh renderer out of the session, run over an exec
/// channel: `detach-client` on the client tmux still lists with the
/// renderer's pid AND name — either alone may name another client by now
/// (the next terminal on that tty, a recycled pid) — and nothing otherwise
/// (B270, `Controller::renderer_detach_command`).
pub fn renderer_detach_command(tmux: &str, renderer: &crate::clients::ClientIdentity) -> String {
    let row = quote(&format!("{} {}", renderer.pid, renderer.name));
    let name = quote(&renderer.name);
    format!(
        "{tmux} list-clients -F '#{{client_pid}} #{{client_name}}' 2>/dev/null | grep -qxF {row} && {tmux} detach-client -t {name}; true"
    )
}

/// Takes the tmux client mosh-server `server` runs — the phone's renderer,
/// its program — out of the session, by the server's pid, while `server` is
/// still a mosh-server: the session knows that pid from the bootstrap, so
/// this needs no word from the sidecar about which client is the renderer
/// (B270). Run before the windows are handed back on the session's real end.
pub fn renderer_detach_by_server_command(tmux: &str, server: i64) -> String {
    format!(
        "case \"$(ps -o comm= -p {server} 2>/dev/null)\" in *mosh-server*) \
         for p in $(pgrep -P {server} 2>/dev/null || ps -A -o pid= -o ppid= 2>/dev/null | awk '$2 == {server} {{ print $1 }}'); do \
         {tmux} list-clients -F '#{{client_pid}} #{{client_name}}' 2>/dev/null | while read -r cp cn; do \
         [ \"$cp\" = \"$p\" ] && {tmux} detach-client -t \"$cn\"; done; done;; esac; true"
    )
}

/// Gives back the status line the mosh sidecar turned off on `session` (a
/// session Moshpit made), and its stamp (B86), run over an exec channel.
pub fn status_restore_commands(tmux: &str, session: &str) -> [String; 2] {
    let target = quote(session);
    [
        format!("{tmux} set-option -t {target} -u status"),
        format!(
            "{tmux} set-option -t {target} -u {}",
            crate::controller::STATUS_HIDDEN_OPTION
        ),
    ]
}

/// Kill this connection's previous renderer stacks by their pidfiles, then
/// remove the trail.
pub fn renderer_cleanup_command(connection_id: &str) -> String {
    let base = format!("$HOME/.moshpit/tmuxr-{connection_id}");
    format!(
        "for f in \"{base}-\"*.pid; do kill $(cat \"$f\" 2>/dev/null) 2>/dev/null; done; rm -f \"{base}-\"*.pid; true"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONN: &str = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEFFFF0000";

    // TmuxRendererHygieneTests
    #[test]
    fn the_attach_line_records_the_pid_and_execs_the_client() {
        let line = renderer_attach_line("tmux", "0", CONN, "ab12cd34");
        assert!(line.starts_with("\u{15}sh -c 'mkdir -p "));
        assert!(line.ends_with('\r'));
        // B138: the login shell's pid as sh sees it, `$PPID`, under `sh -c`
        // (fish refuses `$$`).
        assert!(line.contains(&format!(
            "; echo $PPID > \"$HOME/.moshpit/tmuxr-{CONN}-ab12cd34.pid\"'; "
        )));
        assert!(!line.contains("$$"));
        assert!(line.contains("tmux has-session -t '=0' 2>/dev/null && exec tmux attach -t '=0'"));
    }

    #[test]
    fn the_renderer_attaches_by_exact_name() {
        for (name, quoted) in [
            ("dev", "'=dev'"),
            ("my project", "'=my project'"),
            ("it's", "'=it'\\''s'"),
            ("d*s", "'=d*s'"),
        ] {
            let line = renderer_attach_line("tmux", name, CONN, "ab12cd34");
            assert!(line.contains(&format!(
                "tmux has-session -t {quoted} 2>/dev/null && exec tmux attach -t {quoted}\r"
            )));
            assert!(!line.contains(&format!("-t '{}", &name[..1])));
        }
    }

    #[test]
    fn the_sidecar_goes_back_by_id_then_exact_name_then_the_most_recent() {
        let preferred = SidecarReturn {
            id: "$3".into(),
            name: Some("dev".into()),
        };
        // B138: one word for the login shell, sh reads the chain.
        let under_sh = |chain: &str| format!("sh -c {}", quote(chain));
        assert_eq!(
            sidecar_attach_chain("tmux", Some(&preferred)),
            under_sh(
                "tmux -CC attach -t '$3' 2>/dev/null || tmux -CC attach -t '=dev' 2>/dev/null || while ! tmux -CC attach 2>/dev/null; do sleep 2; done"
            )
        );
        let id_only = SidecarReturn {
            id: "$3".into(),
            name: None,
        };
        assert_eq!(
            sidecar_attach_chain("tmux", Some(&id_only)),
            under_sh(
                "tmux -CC attach -t '$3' 2>/dev/null || while ! tmux -CC attach 2>/dev/null; do sleep 2; done"
            )
        );
        assert_eq!(
            sidecar_attach_chain("tmux", None),
            "sh -c 'while ! tmux -CC attach 2>/dev/null; do sleep 2; done'"
        );
        for id in ["$3;reboot", "@3"] {
            let bad = SidecarReturn {
                id: id.into(),
                name: Some("dev".into()),
            };
            assert!(!sidecar_attach_chain("tmux", Some(&bad)).contains(id));
        }
    }

    /// A session name someone else chose goes back quoted whole, whatever
    /// mark joins its apostrophe (the Swift `ShellQuoteCallSiteTests` case,
    /// kept when the Swift chain went, E2).
    #[test]
    fn the_sidecar_quotes_a_session_name_whole() {
        for mark in ["\u{301}", "\u{FE0F}", "\u{1F3FB}", "\u{200D}", "\u{903}"] {
            let preferred = SidecarReturn {
                id: "$7".into(),
                name: Some(format!("dev'{mark}; touch INJECTED; '")),
            };
            let chain = sidecar_chain("tmux", Some(&preferred));
            assert!(chain.starts_with("tmux -CC attach -t '$7' 2>/dev/null || "));
            let target = format!(r"'=dev'\''{mark}; touch INJECTED; '\'''");
            assert!(
                chain.contains(&format!(" || tmux -CC attach -t {target} 2>/dev/null || ")),
                "{chain}"
            );
        }
    }

    /// B267: the programs are one word for the login shell (`sh -c`), find
    /// tmux as the probe does, attach, and leave the channel to its shell
    /// after; nothing in them waits on a prompt.
    #[test]
    fn the_programs_attach_then_hand_the_channel_to_its_shell() {
        let then = "exec \"${SHELL:-/bin/sh}\" -l";
        let path = format!("PATH=\"$PATH:{EXTRA_PATH_DIRS}\"");
        assert_eq!(
            control_attach_program("tmux", then),
            format!(
                "sh -c {}",
                quote(&format!("{path} tmux -CC attach; {then}"))
            )
        );
        let preferred = SidecarReturn {
            id: "$3".into(),
            name: Some("dev".into()),
        };
        assert_eq!(
            sidecar_attach_program("tmux", Some(&preferred), then),
            format!(
                "sh -c {}",
                quote(&format!(
                    "{path}; tmux -CC attach -t '$3' 2>/dev/null || tmux -CC attach -t '=dev' 2>/dev/null || while ! tmux -CC attach 2>/dev/null; do sleep 2; done; {then}"
                ))
            )
        );
        for program in [
            control_attach_program("tmux", then),
            sidecar_attach_program("tmux", None, then),
        ] {
            assert!(!program.contains('\r') && !program.contains('\n'));
            assert!(program.starts_with("sh -c '"));
        }
    }

    /// B270: the phone's renderer leaves the session before the windows go
    /// back — the client tmux lists with its pid AND name, or the client
    /// whose parent is the session's mosh-server, while that is one.
    #[test]
    fn the_renderer_is_detached_only_while_it_is_still_that_client() {
        let renderer = crate::clients::ClientIdentity {
            pid: 4481,
            name: "/dev/ttys023".into(),
        };
        assert_eq!(
            renderer_detach_command("tmux", &renderer),
            "tmux list-clients -F '#{client_pid} #{client_name}' 2>/dev/null | grep -qxF '4481 /dev/ttys023' && tmux detach-client -t '/dev/ttys023'; true"
        );
        let by_server = renderer_detach_by_server_command("tmux", 4242);
        assert!(by_server.starts_with(
            "case \"$(ps -o comm= -p 4242 2>/dev/null)\" in *mosh-server*) for p in $(pgrep -P 4242 "
        ));
        assert!(by_server.contains("[ \"$cp\" = \"$p\" ] && tmux detach-client -t \"$cn\""));
        assert!(by_server.ends_with(";; esac; true"));
        for command in [renderer_detach_command("tmux", &renderer), by_server] {
            assert!(!command.contains("kill-session") && !command.contains("kill-server"));
        }
    }

    #[test]
    fn cleanup_kills_this_connections_pidfiles_only() {
        let command = renderer_cleanup_command(CONN);
        assert!(command.contains(&format!("\"$HOME/.moshpit/tmuxr-{CONN}-\"*.pid")));
        assert!(command.contains("kill $(cat \"$f\" 2>/dev/null) 2>/dev/null"));
        assert!(command.contains("rm -f"));
        assert!(command.ends_with("true"));
    }

    // TmuxBootLineTests, at the line level: nothing but an attach.
    #[test]
    fn every_boot_line_only_attaches() {
        let lines = [
            control_attach_program("tmux", "exec \"$SHELL\" -l"),
            sidecar_attach_chain("tmux", None),
            sidecar_attach_program("tmux", None, "exec \"$SHELL\" -l"),
            renderer_attach_line("tmux", "0", CONN, "n"),
        ];
        for line in lines {
            assert!(line.contains("attach"));
            for forbidden in [
                "history-limit",
                "set -g",
                "set-option",
                "new-session",
                " new ",
            ] {
                assert!(!line.contains(forbidden), "{line}");
            }
        }
    }
}
