//! The commands the installer sends to a host, as pure functions
//! (`HostCommands`). Every one returns a string that runs in someone
//! else's shell, so every one is pinned by a test and, through
//! `harness/equivalence.py`, by the Swift original's bytes.

use moshpit_seal::foundation::text;

/// Where everything Moshpit installs lives.
pub const DIR: &str = "$HOME/.moshpit";

/// The agent label a self-test stamps and pushes with
/// (`PushRemoteNotification.selfTestAgent`).
pub const SELF_TEST_AGENT: &str = moshpit_seal::notification::SELF_TEST_AGENT;

/// `(path as NSString).deletingLastPathComponent`: repeated and trailing
/// slashes folded away, `/` for the root, `""` for a lone component.
pub fn deleting_last_path_component(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    components.pop();
    let joined = components.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// `(path as NSString).lastPathComponent`.
pub fn last_path_component(path: &str) -> String {
    match path.split('/').rfind(|c| !c.is_empty()) {
        Some(last) => last.to_owned(),
        None if path.starts_with('/') => "/".to_owned(),
        None => String::new(),
    }
}

/// Land a base64 payload at `path` with mode `mode` (`writeFile`). The
/// payload's alphabet cannot hold a quote, a dollar, a backtick or a
/// newline, so single-quoting it is total; `umask` before the write leaves
/// no moment where a file of secrets is world-readable.
///
/// Fix (atomic-write, B351): the Swift original decodes straight into
/// `path` (`HostChannel.swift`'s `writeFile`), so a write that fails part
/// way — a full disk, a quota — has already truncated it: an installed
/// script left half written (a stamp script every agent hook then fails
/// on), or the person's own agent config cut short, every edit since
/// Moshpit first backed it up gone. The bytes now land in a file of their
/// own beside it (`.moshpit-new.<pid>`: two phones tending one host at
/// once never share one), which replaces `path` only once whole; a failure
/// removes it and leaves `path` as it was. A `path` that is a link (a
/// config kept in someone's dotfiles) stays one: see [`RESOLVE`].
pub fn write_file(path: &str, mode: &str, base64: &str) -> String {
    let dir = deleting_last_path_component(path);
    format!(
        "set -e; mkdir -p \"{dir}\"; umask 077; p=\"{path}\"; {RESOLVE} t=\"$p{NEW}$$\"; trap 'rm -f \"$t\"' EXIT; \
         printf %s '{base64}' | base64 -d > \"$t\"; chmod {mode} \"$t\"; mv -f \"$t\" \"$p\""
    )
}

/// Follows `$p` through every link to the file it names, so a write lands
/// beside that file and replaces it whole; the links are left as they are.
///
/// Part of atomic-write (B379): B351's write copied the bytes into the
/// file a link points at (`cat "$t" > "$link"`), which truncates it first —
/// so a copy failing part way still cut short exactly the configs people
/// keep in dotfiles. Each link is read with plain `readlink` (macOS before 12.3
/// has no `readlink -f`), a relative one taken from its own directory, and
/// a cycle stops after 40 steps.
pub const RESOLVE: &str = "i=0; while [ -L \"$p\" ] && [ $i -lt 40 ]; do l=$(readlink \"$p\"); \
     case \"$l\" in /*) p=$l ;; *) p=$(dirname \"$p\")/$l ;; esac; i=$((i+1)); done;";

/// [`write_file`] for a file the person may already have (an agent's
/// config), keeping its mode; `new_mode` only for a file this makes.
///
/// Fix (config-mode, B352): the Swift original writes a config with
/// `writeFile(path, "644", …)`, which set the person's agent config to 644
/// whatever it was — and these hold secrets (an `env` block with an API
/// key, an MCP server's token): one kept 600 became readable by every
/// local user. The new bytes now start from a copy of the file (`cp -p`
/// carries its mode), so the file keeps the mode it had — through a link,
/// the mode of the file it points at ([`RESOLVE`], B379).
pub fn write_keeping_mode(path: &str, new_mode: &str, base64: &str) -> String {
    let dir = deleting_last_path_component(path);
    format!(
        "set -e; mkdir -p \"{dir}\"; umask 077; p=\"{path}\"; {RESOLVE} t=\"$p{NEW}$$\"; trap 'rm -f \"$t\"' EXIT; \
         n=; if [ -f \"$p\" ]; then cp -p \"$p\" \"$t\"; else n=1; fi; \
         printf %s '{base64}' | base64 -d > \"$t\"; [ -z \"$n\" ] || chmod {new_mode} \"$t\"; mv -f \"$t\" \"$p\""
    )
}

/// The suffix of the file a write lands in before it replaces its target.
const NEW: &str = ".moshpit-new.";

/// The target and the base64 payload of a [`write_file`] (or
/// [`write_keeping_mode`]) command, for the
/// simulated hosts the tests and the harness run.
pub fn written(command: &str) -> Option<(String, String)> {
    if !command.contains(&format!("t=\"$p{NEW}$$\"")) {
        return None;
    }
    let path = between(command, "; p=\"", "\"; ")?;
    let payload = between(command, "printf %s '", "' | base64 -d")?;
    Some((path.to_owned(), payload.to_owned()))
}

fn between<'a>(text: &'a str, from: &str, to: &str) -> Option<&'a str> {
    let start = text.find(from)? + from.len();
    let end = start + text[start..].find(to)?;
    Some(&text[start..end])
}

/// Read a file, or print nothing if it does not exist (`readFile`).
pub fn read_file(path: &str) -> String {
    format!("cat \"{path}\" 2>/dev/null || true")
}

/// SHA-256 of a file, lowercase hex, or empty — whichever of `sha256sum`,
/// `shasum` and `openssl` the host has (`sha256`).
pub fn sha256(path: &str) -> String {
    format!(
        "{{ sha256sum \"{path}\" 2>/dev/null || shasum -a 256 \"{path}\" 2>/dev/null || \
         openssl dgst -sha256 -r \"{path}\" 2>/dev/null; }} | awk '{{print $1}}'"
    )
}

/// Every "can this host do it?" question in one round trip, as
/// `key=value` lines (`preflight`).
pub const PREFLIGHT: &str = concat!(
    r#"printf 'home=%s\n' "$HOME"; "#,
    r#"printf 'shell=%s\n' "${SHELL:-unknown}"; "#,
    r#"printf 'uname=%s\n' "$(uname -s 2>/dev/null || echo unknown)"; "#,
    r#"for t in openssl curl jq tmux python3 base64; do "#,
    r#"  if command -v "$t" >/dev/null 2>&1; then printf '%s=yes\n' "$t"; "#,
    r#"  else printf '%s=no\n' "$t"; fi; "#,
    "done; ",
    // Which agents the host has (B374): `agent.<id>=yes` when its config
    // directory is there or one of its CLI's names is on PATH. One entry
    // per `HookAgent::ALL`, as `<id> <config dir> <command>…` (a test holds
    // the two together).
    r#"for a in 'claude .claude claude' 'codex .codex codex' 'gemini .gemini gemini' 'qwen .qwen qwen' "#,
    r#"'qoder .qoder qodercli qoder' 'factory .factory droid factory' 'codebuddy .codebuddy codebuddy cbc'; do "#,
    r#"set -- $a; i=$1; f=no; [ -d "$HOME/$2" ] && f=yes; shift 2; "#,
    r#"for c in "$@"; do command -v "$c" >/dev/null 2>&1 && f=yes; done; "#,
    r#"printf 'agent.%s=%s\n' "$i" "$f"; done"#
);

/// Keep one copy of an agent config as it was before Moshpit touched it
/// (`backupOnce`).
pub fn backup_once(config_path: &str) -> String {
    format!(
        "if [ -f \"{config_path}\" ] && [ ! -f \"{config_path}.moshpit.orig\" ]; then \
         cp \"{config_path}\" \"{config_path}.moshpit.orig\"; fi"
    )
}

/// Where the host's tmux server keeps its socket (`tmuxSocketPath`).
pub const TMUX_SOCKET_PATH: &str = "tmux display-message -p '#{socket_path}' 2>/dev/null || true";

/// `$TMUX` as tmux itself formats it (`tmuxValue(socket:)`).
pub fn tmux_value(socket: &str) -> String {
    format!("{socket},0,0")
}

/// The hook self-test, in one command: fire the stamp script into `pane`
/// as an agent hook would, print what the pane then says
/// (`state|agent|title`), and put back what it said before.
///
/// Fix (stamp-test-restores, B355): the Swift original takes three
/// commands — fire (`selfTest`), read back (`readStamp`), then clear the
/// pane's stamps (`clearStamp`). The pane is the session's active one,
/// where the person's agent usually runs, so the test wiped that agent's
/// state, name, remembered prompt and title, and left a `since` of its own:
/// the island lost the agent until its next hook, its next finish card had
/// nothing to say what finished, and a question it was waiting on never
/// reached the phone (the grace push re-reads the state, and it was gone).
/// Now the five stamps are read first and put back after, in the same
/// command, so only a hook of the agent's own landing in those few
/// milliseconds could be lost. The pane always reaches the inner shell as
/// its one argument, which also covers what clear-stamp-quoting fixed (the
/// Swift `clearStamp` left the pane unquoted in both shells).
///
/// B376: sent again after a dropped connection (the platform retries a
/// command once), a run that had fired but not yet put the stamps back read
/// the test's own stamps as the pane's and put those back. What the pane
/// said is kept in `~/.moshpit/run/selftest-<nonce>` until the stamps are
/// back, and a second run of the same command reads it from there.
pub fn stamp_test(pane: &str, tmux_socket: &str, nonce: &str) -> String {
    stamp_test_running(
        &format!("sh \"{DIR}/moshpit-stamp.sh\" working {SELF_TEST_AGENT}"),
        pane,
        tmux_socket,
        nonce,
    )
}

/// [`stamp_test`] firing `fired` in the pane (`TMUX_PANE` set, and
/// `MOSHPIT_SELFTEST=1`, which the stamp script reads as "a test: label it
/// so, leave no mark") instead of the stamp script by hand. The Host Setup
/// page's test (B380) fires an agent's own registered command. `fired` goes
/// inside the inner shell's single quotes: it is only ever [`crate::hooks::command`]'s
/// text (checked equal before it gets here), which holds no quote. `nonce`
/// names where the pane's stamps wait to be put back (B376).
pub fn stamp_test_running(fired: &str, pane: &str, tmux_socket: &str, nonce: &str) -> String {
    debug_assert!(!fired.contains('\''), "a quote would end the inner script");
    // The nonce names a file in the inner script's single quotes: hex only.
    assert!(nonce.bytes().all(|b| b.is_ascii_hexdigit()), "a hex nonce");
    format!(
        "TMUX={} sh -c 'p=$1; f=\"{DIR}/run/selftest-{nonce}\"; \
         g() {{ tmux show-options -pv -t \"$p\" \"@moshpit_$1\" 2>/dev/null; }}; \
         r() {{ if [ -n \"$2\" ]; then tmux set -p -t \"$p\" \"@moshpit_$1\" \"$2\"; \
         else tmux set -pu -t \"$p\" \"@moshpit_$1\"; fi 2>/dev/null; }}; \
         if [ -f \"$f\" ]; then {{ IFS= read -r s; IFS= read -r a; IFS= read -r w; IFS= read -r m; IFS= read -r t; }} < \"$f\"; \
         else s=$(g state); a=$(g agent); w=$(g since); m=$(g prompt); t=$(g title); \
         (umask 077; mkdir -p \"{DIR}/run\" && printf \"%s\\n\" \"$s\" \"$a\" \"$w\" \"$m\" \"$t\" > \"$f\") 2>/dev/null; fi; \
         MOSHPIT_SELFTEST=1 TMUX_PANE=$p {fired} < /dev/null; \
         tmux display-message -p -t \"$p\" \"#{{@moshpit_state}}|#{{@moshpit_agent}}|#{{@moshpit_title}}\" 2>/dev/null; \
         r state \"$s\"; r agent \"$a\"; r since \"$w\"; r prompt \"$m\"; r title \"$t\"; rm -f \"$f\"' sh {} \
         2>/dev/null || true",
        quote(&tmux_value(tmux_socket)),
        quote(pane)
    )
}

/// Which coding agents have run Moshpit's hooks on this host: the names of
/// the marks the stamp script leaves (`~/.moshpit/seen/<agent>`), on one
/// `seen=` line. Read-only.
pub const AGENTS_PROBE: &str =
    "printf 'seen=%s\\n' \"$(ls \"$HOME/.moshpit/seen\" 2>/dev/null | tr '\\n' ' ')\"";

/// `yes` when the file at `path` exists, nothing otherwise (Factory's
/// hooks file, B385). Read-only.
pub fn file_exists(path: &str) -> String {
    format!("[ -f \"{path}\" ] && echo yes || true")
}

/// Forget that `agent`'s hooks have run here: its registration just
/// changed, and Codex skips a changed hook until it is trusted again.
pub fn forget_seen(agent: &str) -> String {
    format!("rm -f \"{DIR}/seen/{agent}\"")
}

/// Delete one installed file (`removeFile`).
pub fn remove_file(path: &str) -> String {
    format!("rm -f \"{path}\"")
}

/// Prune the timestamped backups the old installer left behind, keeping the
/// newest `keep` (`pruneBackups`).
pub fn prune_backups(config_path: &str, keep: i64) -> String {
    let dir = deleting_last_path_component(config_path);
    let name = last_path_component(config_path);
    format!(
        "ls -t \"{dir}/{name}.moshpit.bak.\"* 2>/dev/null | tail -n +{} | while IFS= read -r f; do rm -f \"$f\"; done; true",
        keep + 1
    )
}

/// Which of `configs` (agent config paths, as `HookAgent::config_path`)
/// still call the stamp script: the index of each, a line each (B375).
pub fn configs_calling_stamp(configs: &[&str]) -> String {
    let quoted: Vec<String> = configs.iter().map(|c| format!("\"{c}\"")).collect();
    format!(
        "i=0; for f in {}; do if grep -q 'moshpit-stamp.sh' \"$f\" 2>/dev/null; then echo $i; fi; \
         i=$((i+1)); done; true",
        quoted.join(" ")
    )
}

/// Whatever pairing is left on the host: each conf in push.d, and the old
/// single push.conf (B375). Empty when none is.
pub const PAIRINGS_LEFT: &str = "ls -A \"$HOME/.moshpit/push.d\" 2>/dev/null; \
     [ -e \"$HOME/.moshpit/push.conf\" ] && echo push.conf; true";

/// Everything else Moshpit put on the host, once no pairing is left there
/// (B375): the two scripts, the manifest, what stands in for tmux panes and
/// the agents seen, the backups beside each of `configs`, and the
/// directories once empty.
pub fn remove_the_rest(configs: &[&str]) -> String {
    let backups: Vec<String> = configs
        .iter()
        .map(|c| format!("\"{c}.moshpit.orig\" \"{c}.moshpit.bak.\"*"))
        .collect();
    format!(
        "rm -f \"{DIR}/moshpit-stamp.sh\" \"{DIR}/moshpit-push.sh\" \"{DIR}/manifest.json\" {}; \
         rm -rf \"{DIR}/run\" \"{DIR}/seen\"; rmdir \"{DIR}/push.d\" 2>/dev/null; true",
        backups.join(" ")
    )
}

/// `~/.moshpit` itself, if nothing is left in it.
pub const REMOVE_DIR_IF_EMPTY: &str = "rmdir \"$HOME/.moshpit\" 2>/dev/null; true";

/// Take the host's manifest lock (B373) as `token`'s holder: two phones,
/// or two screens of one, each read the manifest, changed their part and
/// wrote all of it back, so the later write erased what the other had just
/// recorded.
///
/// B566: the lock is a symlink, `~/.moshpit/.lock -> <unix time>.<token>`,
/// made with `ln -s`: taken and stamped in one atomic step. It was a
/// directory (`mkdir`) with its stamp written into it after, so a waiter
/// could find a lock with no stamp yet. B448 waited four tries before
/// breaking one, but waiters polled in step, a second apart: two of them
/// broke the same lock together, the second removing the first's fresh
/// one, and two writers held it at once — one's change lost (Linux). And
/// waiters gave up after six one-second tries while others took turns, so
/// a third phone's change failed (macOS).
///
/// - Waiting is on a deadline, about ten seconds, polling every 50–200 ms
///   (spread by the process id, so waiters don't move in step).
/// - Only a lock whose stamp is older than 60 s is broken: a holder that
///   never came back. It is renamed aside first and removed only if it is
///   still the one read; a fresh one taken in that moment is put back.
/// - A lock that already names `token` is this holder's (B376): the same
///   command sent again after a dropped connection.
/// - An older Moshpit's lock — a directory with its stamp in `at` — is
///   waited for the same way, and broken when its stamp is older than 60 s
///   or it has had none for about four seconds. No link is made while it
///   is there (`ln -s` would make one inside it); one made as it appeared
///   is read as not taken, and undone.
pub fn lock(token: &str) -> String {
    format!(
        "t={}; d=\"$HOME/.moshpit/.lock\"; mkdir -p \"$HOME/.moshpit\" || exit 1; \
         link_of() {{ readlink \"$1\" 2>/dev/null || ls -ld \"$1\" 2>/dev/null | sed -n 's/.* -> //p'; }}; \
         end=$(($(date +%s) + 10)); e=; n=0; \
         while :; do s=$(date +%s); \
         if [ ! -d \"$d\" ] && ln -s \"$s.$t\" \"$d\" 2>/dev/null; then [ \"$(link_of \"$d\")\" = \"$s.$t\" ] && break; rm -f \"$d/$s.$t\"; fi; \
         if [ -L \"$d\" ]; then a=$(link_of \"$d\"); elif [ -d \"$d\" ]; then a=$(cat \"$d/at\" 2>/dev/null); else a=; fi; \
         [ \"${{a#*.}}\" = \"$t\" ] && break; b=; \
         case \"$a\" in \
         *.*) e=; x=${{a%%.*}}; case \"$x\" in ''|*[!0-9]*) b=1 ;; *) [ $((s - x)) -le 60 ] || b=1 ;; esac ;; \
         '') if [ -L \"$d\" ] || [ -e \"$d\" ]; then [ -n \"$e\" ] || e=$s; [ $((s - e)) -lt 4 ] || b=1; fi ;; \
         *) b=1 ;; esac; \
         if [ -n \"$b\" ]; then g=\"$d.$$.$n\"; if mv \"$d\" \"$g\" 2>/dev/null; then \
         if [ -L \"$g\" ]; then c=$(link_of \"$g\"); else c=$(cat \"$g/at\" 2>/dev/null); fi; \
         if [ \"$c\" = \"$a\" ]; then rm -rf \"$g\"; elif [ -L \"$g\" ]; then ln -s \"$c\" \"$d\" 2>/dev/null; rm -f \"$g\"; \
         else mv \"$g\" \"$d\" 2>/dev/null || rm -rf \"$g\"; fi; fi; fi; \
         [ \"$s\" -lt \"$end\" ] || {{ echo 'another Moshpit is changing this host right now: try again in a moment' >&2; exit 75; }}; \
         n=$((n + 1)); if [ -L \"$d\" ] || [ -e \"$d\" ]; then \
         case $((($$ + n) % 4)) in 0) z=0.05 ;; 1) z=0.1 ;; 2) z=0.15 ;; *) z=0.2 ;; esac; \
         sleep $z 2>/dev/null || sleep 1; fi; done",
        quote(token)
    )
}

/// Let go of the manifest lock `token` holds — only if it still does (one
/// broken as stale may be another's by now).
pub fn unlock(token: &str) -> String {
    format!(
        "d=\"$HOME/.moshpit/.lock\"; a=$(readlink \"$d\" 2>/dev/null || ls -ld \"$d\" 2>/dev/null | sed -n 's/.* -> //p'); \
         [ \"${{a#*.}}\" = {} ] && rm -f \"$d\"; true",
        quote(token)
    )
}

/// Each pairing's conf on the host and the phone it pushes to: a line
/// `<file name> <APNs token>` per `push.d/*.conf`, the token empty for a
/// conf without one. Read-only, and nothing secret leaves the host: the
/// pairing secret and the send token are never printed.
pub const PAIRING_PHONES: &str = "for f in \"$HOME/.moshpit/push.d\"/*.conf; do [ -f \"$f\" ] || continue; \
     printf '%s %s\\n' \"${f##*/}\" \"$(sed -n 's/^APNS_TOKEN=//p' \"$f\" | head -n 1)\"; done; true";

/// Run the sender's self-test with a nonce, for connection `conn`'s
/// pairing only (`HostInstaller.selfTestPush`).
///
/// Fix (self-test-scope, B353): the Swift original passes the nonce alone,
/// so the sender tests every pairing on the host. Every other phone paired
/// with it got the test push, and that phone isn't waiting for it, so
/// unless Moshpit was in the foreground there it showed on the lock screen
/// as a finished `moshpit-selftest` turn. A phone with two connections to
/// the host had the test go out on whichever pairing sorted first, so
/// proving one connection could pass on the other's pairing. The sender has
/// long taken the connection as its third argument; it now gets it.
///
/// The test names the pane [`SELF_TEST_PANE`] (B439). Run outside any pane,
/// it named `%0` — tmux's first pane, where an agent often runs — and the
/// phone's notification service took the test's "done" for that pane's:
/// a question waiting there left the waiting set, and rang again as a new
/// one. No herdr pane either: an inherited `HERDR_PANE_ID` would name one.
pub fn self_test_push(nonce: &str, conn: &str) -> String {
    format!(
        "HERDR_PANE_ID= TMUX_PANE={SELF_TEST_PANE} sh \"{DIR}/moshpit-push.sh\" --test {} {} 2>&1",
        quote(nonce),
        quote(conn)
    )
}

/// The pane a push test names: one no tmux pane (`%N`) or herdr pane
/// (`w1:p2`) can be, short enough for the relay's 64-byte collapse id.
pub const SELF_TEST_PANE: &str = "selftest";

/// Is this a path we are willing to interpolate into a command
/// (`isSafePath`)? Paths reach the installer from the host's manifest and
/// cannot be single-quoted (`$HOME` in them must expand), so they are
/// validated: at most 512 of `A-Za-z0-9._-/$~+`, and no `$(`.
pub fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && text::character_count(path) <= 512
        && !path.contains("$(")
        && path.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/' | b'$' | b'~' | b'+')
        })
}

/// POSIX single-quote a value for the remote shell (`quote`): close the
/// quote, emit an escaped quote, reopen.
///
/// Fix (quote-combining): Swift's `replacingOccurrences(of: "'", …)`
/// compares grapheme clusters, so an apostrophe followed by a combining mark
/// is not replaced (`HostChannel.swift:217-219`): `quote("it'\u{301}s")` is
/// `'it'\u{301}s'`, which closes the quote early — a value like
/// `x'\u{301}; touch /tmp/p; '` runs `touch` on the host. Here every
/// apostrophe byte is escaped.
///
/// B138 (the app's `Shell.quote` the same): every `\` goes outside the
/// quotes too (`'\\'`), because fish takes `\'` and `\\` as escapes inside
/// single quotes; the word then reads the same in sh, dash, bash, zsh and
/// fish.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str(r"'\''"),
            '\\' => out.push_str(r"'\\'"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HostInstallerTests.quoting`.
    #[test]
    fn remote_values_are_single_quoted_the_only_way_that_is_correct() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("$HOME `whoami`"), "'$HOME `whoami`'");
        assert_eq!(quote(""), "''");
        // Fix (quote-combining).
        assert_eq!(quote("it'\u{301}s"), "'it'\\''\u{301}s'");
    }

    /// `HostInstallerTests.checksumPortability`.
    #[test]
    fn the_checksum_command_copes_with_all_three_tools_a_host_might_have() {
        let cmd = sha256("$HOME/.moshpit/x");
        for tool in ["sha256sum", "shasum -a 256", "openssl dgst -sha256"] {
            assert!(cmd.contains(tool));
        }
        assert!(cmd.contains("awk '{print $1}'"));
        assert_eq!(
            cmd,
            "{ sha256sum \"$HOME/.moshpit/x\" 2>/dev/null || shasum -a 256 \"$HOME/.moshpit/x\" 2>/dev/null || \
             openssl dgst -sha256 -r \"$HOME/.moshpit/x\" 2>/dev/null; } | awk '{print $1}'"
        );
    }

    #[test]
    fn writing_a_file_never_leaves_it_readable() {
        let write = write_file("$HOME/.moshpit/push.d/X.conf", "600", "QUJD");
        assert_eq!(
            write,
            "set -e; mkdir -p \"$HOME/.moshpit/push.d\"; umask 077; p=\"$HOME/.moshpit/push.d/X.conf\"; \
             i=0; while [ -L \"$p\" ] && [ $i -lt 40 ]; do l=$(readlink \"$p\"); \
             case \"$l\" in /*) p=$l ;; *) p=$(dirname \"$p\")/$l ;; esac; i=$((i+1)); done; \
             t=\"$p.moshpit-new.$$\"; trap 'rm -f \"$t\"' EXIT; printf %s 'QUJD' | base64 -d > \"$t\"; \
             chmod 600 \"$t\"; mv -f \"$t\" \"$p\""
        );
        assert!(write.find("umask 077").unwrap() < write.find("base64 -d >").unwrap());
        assert_eq!(
            written(&write),
            Some(("$HOME/.moshpit/push.d/X.conf".to_owned(), "QUJD".to_owned()))
        );
    }

    #[test]
    fn the_preflight_is_one_line_of_shell() {
        assert_eq!(
            PREFLIGHT,
            "printf 'home=%s\\n' \"$HOME\"; printf 'shell=%s\\n' \"${SHELL:-unknown}\"; printf 'uname=%s\\n' \
             \"$(uname -s 2>/dev/null || echo unknown)\"; for t in openssl curl jq tmux python3 base64; do   if \
             command -v \"$t\" >/dev/null 2>&1; then printf '%s=yes\\n' \"$t\";   else printf '%s=no\\n' \"$t\"; \
             fi; done; for a in 'claude .claude claude' 'codex .codex codex' 'gemini .gemini gemini' 'qwen .qwen qwen' \
             'qoder .qoder qodercli qoder' 'factory .factory droid factory' 'codebuddy .codebuddy codebuddy cbc'; do \
             set -- $a; i=$1; f=no; [ -d \"$HOME/$2\" ] && f=yes; shift 2; for c in \"$@\"; do command -v \"$c\" \
             >/dev/null 2>&1 && f=yes; done; printf 'agent.%s=%s\\n' \"$i\" \"$f\"; done"
        );
        assert!(!PREFLIGHT.contains('\n'));
    }

    /// The preflight asks after every agent the installer knows, by its
    /// config directory and its CLI's names (B374).
    #[test]
    fn the_preflight_asks_after_every_agent() {
        let listed: Vec<String> = crate::HookAgent::ALL
            .iter()
            .map(|a| format!("'{} {} {}'", a.id, a.config_dir(), a.commands.join(" ")))
            .collect();
        assert!(
            PREFLIGHT.contains(&format!("for a in {}; do", listed.join(" "))),
            "{listed:?}"
        );
    }

    #[test]
    fn paths_as_nsstring_splits_them() {
        for (path, dir, last) in [
            (
                "$HOME/.moshpit/push.d/X.conf",
                "$HOME/.moshpit/push.d",
                "X.conf",
            ),
            ("/a/b/", "/a", "b"),
            ("a", "", "a"),
            ("/a", "/", "a"),
            ("/", "/", "/"),
            ("a/", "", "a"),
            ("a//b", "a", "b"),
            ("//a//b//", "/a", "b"),
            ("~/x", "~", "x"),
            ("", "", ""),
            ("a/b/../c", "a/b/..", "c"),
            ("./x", ".", "x"),
        ] {
            assert_eq!(deleting_last_path_component(path), dir, "{path}");
            assert_eq!(last_path_component(path), last, "{path}");
        }
    }

    #[test]
    fn only_inert_paths_are_safe() {
        assert!(is_safe_path("$HOME/.claude/settings.json"));
        assert!(is_safe_path("~/dotfiles/claude+1.json"));
        for bad in [
            "",
            "$(rm -rf ~)",
            "a b",
            "a'b",
            "a;b",
            "a`b`",
            "é",
            &"a".repeat(513),
        ] {
            assert!(!is_safe_path(bad), "{bad}");
        }
        assert!(is_safe_path(&"a".repeat(512)));
    }

    #[test]
    fn the_self_test_commands() {
        assert_eq!(
            stamp_test("%3", "/private/tmp/tmux-501/default", "0123456789abcdef"),
            "TMUX='/private/tmp/tmux-501/default,0,0' sh -c 'p=$1; \
             f=\"$HOME/.moshpit/run/selftest-0123456789abcdef\"; \
             g() { tmux show-options -pv -t \"$p\" \"@moshpit_$1\" 2>/dev/null; }; \
             r() { if [ -n \"$2\" ]; then tmux set -p -t \"$p\" \"@moshpit_$1\" \"$2\"; \
             else tmux set -pu -t \"$p\" \"@moshpit_$1\"; fi 2>/dev/null; }; \
             if [ -f \"$f\" ]; then { IFS= read -r s; IFS= read -r a; IFS= read -r w; IFS= read -r m; IFS= read -r t; } \
             < \"$f\"; else s=$(g state); a=$(g agent); w=$(g since); m=$(g prompt); t=$(g title); \
             (umask 077; mkdir -p \"$HOME/.moshpit/run\" && printf \"%s\\n\" \"$s\" \"$a\" \"$w\" \"$m\" \"$t\" > \"$f\") \
             2>/dev/null; fi; \
             MOSHPIT_SELFTEST=1 TMUX_PANE=$p sh \"$HOME/.moshpit/moshpit-stamp.sh\" working moshpit-selftest < /dev/null; \
             tmux display-message -p -t \"$p\" \"#{@moshpit_state}|#{@moshpit_agent}|#{@moshpit_title}\" 2>/dev/null; \
             r state \"$s\"; r agent \"$a\"; r since \"$w\"; r prompt \"$m\"; r title \"$t\"; rm -f \"$f\"' \
             sh '%3' 2>/dev/null || true"
        );
        assert_eq!(
            self_test_push("selftest-ab12", "3F2504E0-4F89-11D3-9A0C-0305E82C3301"),
            "HERDR_PANE_ID= TMUX_PANE=selftest sh \"$HOME/.moshpit/moshpit-push.sh\" --test 'selftest-ab12' \
             '3F2504E0-4F89-11D3-9A0C-0305E82C3301' 2>&1"
        );
    }

    /// A private directory with `file` holding `old`, and a `base64` on
    /// `PATH` that writes the first `cut` bytes it decodes and then fails
    /// (a disk full part way); runs `write_file` there.
    #[cfg(unix)]
    fn write_into(
        old: Option<&str>,
        link: bool,
        cut: Option<usize>,
        text: &str,
    ) -> (std::path::PathBuf, bool) {
        write_with(old, link, cut, None, text, |path, payload| {
            write_file(path, "644", payload)
        })
    }

    /// [`write_into`] with the command `make` builds from the path and the
    /// payload. With `link`, `file` is a relative link to a link to
    /// `dotfiles/real.json`, as a dotfiles manager leaves one. `copy_cut`:
    /// a `cat` that copies that many bytes and fails (a disk full under
    /// the file a link points at).
    #[cfg(unix)]
    fn write_with(
        old: Option<&str>,
        link: bool,
        cut: Option<usize>,
        copy_cut: Option<usize>,
        text: &str,
        make: impl Fn(&str, &str) -> String,
    ) -> (std::path::PathBuf, bool) {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("install-write-{}", moshpit_seal::random::hex(6)));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        let target = dir.join("dotfiles/real.json");
        if let Some(old) = old {
            std::fs::write(&target, old).unwrap();
        }
        if link {
            std::os::unix::fs::symlink("real.json", dir.join("dotfiles/hop")).unwrap();
            std::os::unix::fs::symlink("dotfiles/hop", dir.join("file")).unwrap();
        } else if old.is_some() {
            std::fs::rename(&target, dir.join("file")).unwrap();
        }
        // Stand-ins that do part of the job and fail, ahead of the real
        // tools on PATH.
        let fakes = [
            cut.map(|n| ("base64", format!("/usr/bin/base64 -d | head -c {n}"))),
            copy_cut.map(|n| ("cat", format!("head -c {n} \"$@\""))),
        ];
        for (name, body) in fakes.iter().flatten() {
            let fake = dir.join("bin").join(name);
            std::fs::write(&fake, format!("#!/bin/sh\n{body}\nexit 1\n")).unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:/usr/bin:/bin", dir.join("bin").display());
        let command = make(
            dir.join("file").to_str().unwrap(),
            &moshpit_seal::foundation::base64::encode(text.as_bytes()),
        );
        let ok = std::process::Command::new("sh")
            .args(["-c", &command])
            .env_clear()
            .env("PATH", path)
            .status()
            .unwrap()
            .success();
        (dir, ok)
    }

    /// Whatever a write left beside its target besides the files the test
    /// made.
    #[cfg(unix)]
    fn left_over(dir: &std::path::Path) -> Vec<String> {
        [dir.to_owned(), dir.join("dotfiles")]
            .iter()
            .flat_map(|d| std::fs::read_dir(d).unwrap())
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !["bin", "dotfiles", "file", "hop", "real.json"].contains(&n.as_str()))
            .collect()
    }

    /// B351: a write that failed part way — a full disk or quota — had
    /// already truncated the file: an installed script left half written
    /// (a stamp script that every agent hook then failed on), or the
    /// person's own agent config cut short, with every edit since Moshpit
    /// first backed it up gone. The file keeps what it held until the new
    /// bytes are all down.
    #[cfg(unix)]
    #[test]
    fn a_write_that_fails_part_way_leaves_the_file_as_it_was() {
        let (dir, ok) = write_into(Some("{\"mine\": true}\n"), false, Some(3), "{\"new\": 1}\n");
        assert!(!ok, "the failed write said it worked");
        assert_eq!(
            std::fs::read_to_string(dir.join("file")).unwrap(),
            "{\"mine\": true}\n",
            "the file was cut short"
        );
        assert!(left_over(&dir).is_empty(), "{:?}", left_over(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_write_lands_whole_with_its_mode_and_leaves_nothing_beside_it() {
        use std::os::unix::fs::PermissionsExt;
        for old in [None, Some("old\n")] {
            let (dir, ok) = write_into(old, false, None, "new bytes\n");
            assert!(ok);
            let file = dir.join("file");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "new bytes\n");
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o644
            );
            assert!(left_over(&dir).is_empty(), "{:?}", left_over(&dir));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A config kept in someone's dotfiles is a link: it stays one (and so
    /// does every link on the way), and what it points at gets the new
    /// bytes. Both writes.
    #[cfg(unix)]
    #[test]
    fn a_write_through_a_link_keeps_the_link() {
        let keeping = |path: &str, payload: &str| write_keeping_mode(path, "644", payload);
        for (dir, ok) in [
            write_into(Some("old\n"), true, None, "new bytes\n"),
            write_with(Some("old\n"), true, None, None, "new bytes\n", keeping),
        ] {
            assert!(ok);
            for link in ["file", "dotfiles/hop"] {
                assert!(
                    std::fs::symlink_metadata(dir.join(link))
                        .unwrap()
                        .file_type()
                        .is_symlink(),
                    "{link}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(dir.join("dotfiles/real.json")).unwrap(),
                "new bytes\n"
            );
            assert!(left_over(&dir).is_empty(), "{:?}", left_over(&dir));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// B379: through a link, B351's write copied the bytes into the file it
    /// points at (`cat "$t" > "$link"`), truncating it first — a copy
    /// failing part way (the disk under that file full) still cut short
    /// exactly the configs people keep in dotfiles. The file a link points
    /// at now holds all of the old bytes or all of the new, never part, and
    /// nothing is left beside it.
    #[cfg(unix)]
    #[test]
    fn a_write_through_a_link_that_fails_part_way_leaves_the_file_whole() {
        let keeping = |path: &str, payload: &str| write_keeping_mode(path, "644", payload);
        let plain = |path: &str, payload: &str| write_file(path, "644", payload);
        let (old, text) = ("{\"mine\": true}\n", "{\"new\": 1}\n");
        for (failing, (dir, ok)) in [
            (
                "decode",
                write_with(Some(old), true, Some(3), None, text, plain),
            ),
            (
                "copy",
                write_with(Some(old), true, None, Some(3), text, plain),
            ),
            (
                "decode",
                write_with(Some(old), true, Some(3), None, text, keeping),
            ),
            (
                "copy",
                write_with(Some(old), true, None, Some(3), text, keeping),
            ),
        ] {
            let held = std::fs::read_to_string(dir.join("dotfiles/real.json")).unwrap();
            if failing == "decode" {
                assert!(!ok, "the failed write said it worked");
            }
            assert_eq!(
                held,
                if ok { text } else { old },
                "{failing}: the linked file was cut short"
            );
            assert!(
                std::fs::symlink_metadata(dir.join("file"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert!(left_over(&dir).is_empty(), "{:?}", left_over(&dir));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A pane, whatever it holds, reaches the inner shell as its one
    /// argument; run it to see it stays one word in every tmux call.
    #[cfg(unix)]
    #[test]
    fn an_odd_pane_reaches_tmux_as_one_word() {
        let command = stamp_test("%3; touch x 'q'", "/tmp/s", "0123456789abcdef");
        assert!(
            command.ends_with("sh '%3; touch x '\\''q'\\''' 2>/dev/null || true"),
            "{command}"
        );
        let dir =
            std::env::temp_dir().join(format!("install-quote-{}", moshpit_seal::random::hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let tmux = dir.join("tmux");
        std::fs::write(&tmux, "#!/bin/sh\nprintf '[%s]' \"$4\" >> \"$OUT\"\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let out = dir.join("out");
        let status = std::process::Command::new("sh")
            .args(["-c", &command])
            .current_dir(&dir)
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("HOME", &dir)
            .env("OUT", &out)
            .status()
            .unwrap();
        assert!(status.success());
        // Five reads, the read-back, five restores; the stamp script isn't
        // in this HOME, which the command rides over.
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "[%3; touch x 'q']".repeat(11)
        );
        assert!(!dir.join("x").exists(), "part of the pane ran as a command");
        let _ = std::fs::remove_dir_all(dir);
    }
}
