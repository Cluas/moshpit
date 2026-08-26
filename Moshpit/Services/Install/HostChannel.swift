import CryptoKit
import Foundation

/// A way to run one command on a host and read its output.
///
/// One method, on purpose. Everything the installer needs — writing files,
/// reading configs, probing for tools — is built on top of it, so there is
/// exactly one thing to implement against a real connection and exactly one
/// thing to fake in a test.
///
/// What it deliberately is NOT: typing into a pane. The install flow this
/// replaces sent 6.5 KB of shell into whatever pane happened to be active,
/// which meant it could not run while an agent held the pane (there was a guard
/// refusing exactly that), left secrets in `~/.bash_history`, and had no way to
/// see whether any of it worked. An exec channel has none of those problems, and
/// the app already had one: `SessionHub.ActiveSession.acquireFileTransferSSH()`,
/// the same channel image attachment rides — in-band SSH when there is one, the
/// mosh sidecar otherwise, an on-demand dial for pure mosh, from the in-memory
/// secret cache so it never re-prompts Face ID.
protocol HostChannel: Sendable {
    /// Run `command` through a fresh exec channel and return its stdout.
    /// Throws if the channel itself fails.
    func run(_ command: String) async throws -> Data
}

extension HostChannel {
    /// stdout as text, trailing newline trimmed.
    func runText(_ command: String) async throws -> String {
        String(decoding: try await run(command), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// The commands the installer sends, as pure functions.
///
/// Every one of these is a `static` returning a String, and every one is pinned
/// by a test. That is the same discipline `SessionHub.uploadCleanupCommand`
/// already follows, for the reason its comment gives: a quoting slip in a
/// command that runs on someone else's server is not a bug to discover in the
/// field. Here the stakes are the same — these lines write files into `$HOME`
/// and edit agent configs.
enum HostCommands {

    /// Where everything Moshpit installs lives.
    static let dir = "$HOME/.moshpit"

    // MARK: - Writing a file

    /// Land `content` at `path` with mode `mode`.
    ///
    /// The content rides as base64 for one reason that matters: the base64
    /// alphabet is `A-Za-z0-9+/=`, so the payload CANNOT contain a quote, a
    /// dollar, a backtick or a newline. Single-quoting it is therefore total —
    /// there is no escaping to get wrong, and no shell metacharacter can survive
    /// from a config file, a secret, or a script into the command line.
    ///
    /// `umask` before the write rather than `chmod` after: a chmod leaves a
    /// window in which a file holding a pairing secret is world-readable on a
    /// shared box. The chmod is still there to fix an existing file's mode.
    static func writeFile(path: String, mode: String, base64 payload: String) -> String {
        let dir = (path as NSString).deletingLastPathComponent
        return "set -e; mkdir -p \"\(dir)\"; umask 077; " +
               "printf %s '\(payload)' | base64 -d > \"\(path)\"; " +
               "chmod \(mode) \"\(path)\""
    }

    /// Read a file, or print nothing if it does not exist.
    ///
    /// Absent and empty are deliberately the same answer: every caller treats
    /// "no config yet" and "empty config" identically, and a command that fails
    /// on a missing file would make the common first-install path an error path.
    static func readFile(path: String) -> String {
        "cat \"\(path)\" 2>/dev/null || true"
    }

    /// SHA-256 of a file, lowercase hex, or empty.
    ///
    /// Three implementations because there is no one portable tool: `sha256sum`
    /// on most Linux, `shasum` on macOS, and `openssl dgst` as the backstop —
    /// which the sender already requires, so a host that can push at all can
    /// checksum. `awk '{print $1}'` normalises the three different output shapes
    /// to just the digest.
    static func sha256(path: String) -> String {
        "{ sha256sum \"\(path)\" 2>/dev/null || shasum -a 256 \"\(path)\" 2>/dev/null || " +
        "openssl dgst -sha256 -r \"\(path)\" 2>/dev/null; } | awk '{print $1}'"
    }

    // MARK: - Preflight

    /// One round trip that answers every "is this host able to do this?"
    /// question, as `key=value` lines.
    ///
    /// Deliberately one command rather than one per tool: each exec is a channel,
    /// a fork on the server and a couple of round trips (see
    /// `SSHService.executeCommand`), and over a 500 ms link five of those is a
    /// visible stall in a sheet the user is staring at.
    static let preflight = """
    printf 'home=%s\\n' "$HOME"; \
    printf 'shell=%s\\n' "${SHELL:-unknown}"; \
    printf 'uname=%s\\n' "$(uname -s 2>/dev/null || echo unknown)"; \
    for t in openssl curl jq tmux python3 base64; do \
      if command -v "$t" >/dev/null 2>&1; then printf '%s=yes\\n' "$t"; \
      else printf '%s=no\\n' "$t"; fi; \
    done
    """

    // MARK: - Agent configs

    /// Keep ONE copy of the config as it was before Moshpit ever touched it.
    ///
    /// Deliberately not the old behaviour, which wrote
    /// `<config>.moshpit.bak.<epoch>` on every run and never removed one — a
    /// host where hooks had been reinstalled a dozen times carried a dozen
    /// copies of the user's settings forever. `.moshpit.orig` is written once
    /// and never overwritten, so it always means "before Moshpit", which is the
    /// only version worth keeping.
    static func backupOnce(configPath: String) -> String {
        "if [ -f \"\(configPath)\" ] && [ ! -f \"\(configPath).moshpit.orig\" ]; then " +
        "cp \"\(configPath)\" \"\(configPath).moshpit.orig\"; fi"
    }

    /// Where the tmux server this host is running keeps its socket.
    ///
    /// Needed by the hooks self-test: the stamp script refuses to run outside
    /// tmux, and an exec channel is outside it. Empty output means either no
    /// server or one on a non-default socket — in which case the self-test is
    /// reported unavailable rather than guessed at.
    static let tmuxSocketPath = "tmux display-message -p '#{socket_path}' 2>/dev/null || true"

    // MARK: - Self-test

    /// Fire the stamp script by hand, exactly as an agent hook would.
    ///
    /// This is what makes install verification honest. The flow this replaces
    /// could only say "run an agent turn in any pane, then re-check" — it had no
    /// way to distinguish "not installed" from "installed and idle", so it
    /// resolved on ANY pane carrying ANY stamp and called that success. Invoking
    /// the script directly proves the real thing: it exists, it is executable,
    /// it can talk to tmux, and — for `done` — it hands off to the push sender,
    /// which is a notification the phone either receives or does not.
    ///
    /// `TMUX`/`TMUX_PANE` are passed explicitly because an exec channel is not
    /// inside tmux and the script exits early without them.
    static func selfTest(state: String, pane: String, tmuxSocket: String) -> String {
        "TMUX=\(quote(tmuxValue(socket: tmuxSocket))) TMUX_PANE=\(quote(pane)) " +
        "sh \"\(dir)/moshpit-stamp.sh\" \(quote(state)) \(quote(selfTestAgent)) < /dev/null"
    }

    /// `$TMUX` as tmux itself formats it: socket path, server pid, session
    /// index. Only the socket matters to the CLI, and only non-emptiness matters
    /// to the stamp script, but a well-formed value avoids depending on which of
    /// those two facts is true this year.
    static func tmuxValue(socket: String) -> String { "\(socket),0,0" }

    /// The agent label a self-test stamps with — defined with the push contract,
    /// because the extension and the notification delegate must recognise it and
    /// neither of them links this engine.
    static var selfTestAgent: String { PushRemoteNotification.selfTestAgent }

    /// Read back what a pane is stamped with, without needing the control
    /// channel — so the self-test works on a connection whose multiplexer is
    /// herdr, or none.
    static func readStamp(pane: String, tmuxSocket: String) -> String {
        "TMUX=\(quote(tmuxValue(socket: tmuxSocket))) tmux display-message -p -t \(quote(pane)) " +
        "'#{@moshpit_state}|#{@moshpit_agent}|#{@moshpit_title}' 2>/dev/null || true"
    }

    /// Clear a self-test's stamps so nothing lingers.
    static func clearStamp(pane: String, tmuxSocket: String) -> String {
        "TMUX=\(quote(tmuxValue(socket: tmuxSocket))) sh -c 'tmux set -pu -t \(quote(pane)) @moshpit_state; " +
        "tmux set -pu -t \(quote(pane)) @moshpit_agent; " +
        "tmux set -pu -t \(quote(pane)) @moshpit_title' 2>/dev/null || true"
    }

    // MARK: - Removal

    /// Delete one installed file.
    static func removeFile(path: String) -> String {
        "rm -f \"\(path)\""
    }

    /// Prune the timestamped backups the old installer left behind, keeping the
    /// newest `keep`.
    ///
    /// The flow this replaces wrote `<config>.moshpit.bak.<epoch>` on every run
    /// and never removed one, so a host where hooks had been reinstalled a dozen
    /// times carried a dozen copies of the user's settings forever.
    static func pruneBackups(configPath: String, keep: Int) -> String {
        let dir = (configPath as NSString).deletingLastPathComponent
        let name = (configPath as NSString).lastPathComponent
        return "ls -t \"\(dir)/\(name).moshpit.bak.\"* 2>/dev/null | tail -n +\(keep + 1) | " +
               "while IFS= read -r f; do rm -f \"$f\"; done; true"
    }

    // MARK: - Quoting

    /// Is this a path we are willing to interpolate into a command?
    ///
    /// Paths reach the installer from the MANIFEST, which lives on the host and
    /// is therefore not ours to trust. They cannot be single-quoted, because
    /// `$HOME` in them has to expand — so they are validated instead, and a path
    /// carrying a quote, a backtick, a `$(` or a `;` is refused rather than
    /// executed.
    static func isSafePath(_ path: String) -> Bool {
        guard !path.isEmpty, path.count <= 512, !path.contains("$(") else { return false }
        let allowed = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-/$~+")
        return path.allSatisfy { allowed.contains($0) }
    }

    /// POSIX single-quote a value for the remote shell.
    ///
    /// The only correct way to do this: close the quote, emit an escaped quote,
    /// reopen. Everything else — backslash-escaping, blacklisting characters —
    /// is a bug waiting for a filename with an apostrophe in it.
    static func quote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

/// SHA-256 of some content, lowercase hex — the identity used throughout the
/// install manifest.
///
/// Content-addressed rather than a hand-maintained version number, because the
/// failure this exists to prevent is precisely a forgotten bump: the stamp
/// script gained a push hand-off, every already-installed copy silently stopped
/// being current, and nothing in the app could tell. A digest cannot be
/// forgotten.
enum ContentDigest {
    static func of(_ text: String) -> String {
        Data(SHA256.hash(data: Data(text.utf8))).moshpitHexString
    }
}
