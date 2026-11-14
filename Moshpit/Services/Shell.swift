import Foundation

/// Command lines for a remote shell.
enum Shell {

    /// `value` as one shell word, whatever it holds, in any login shell — sh,
    /// dash, bash, zsh and fish: wrapped in single quotes, every `'` in it
    /// closed, escaped and reopened (`'\''`), and every `\` likewise
    /// (`'\\'`). Inside the quotes nothing is left that any of them reads
    /// specially — not `$`, a backtick, a newline or a glob — so the command
    /// carries exactly these bytes.
    ///
    /// The backslash goes outside too (B138): fish takes `\'` and `\\` as
    /// escapes inside single quotes (3.7.1 and 4.6.0 alike), so a value ending
    /// in `\` never closed fish's quote and the next word ran on, and `\\`
    /// came out as `\`. Outside quotes `\\` is one backslash in all five.
    ///
    /// The app's one shell quoter. `HostCommands.quote`,
    /// `ExecUploadCommands.quote`, `ActiveSession.shellQuote` and the image
    /// paths pasted at a prompt are all this function; none of them may grow
    /// its own escaping again. (A name in a tmux control-mode line is not a
    /// shell word: see `TmuxSessionController.tmuxNameArgument`.)
    ///
    /// By Unicode scalar, never by `Character`. The six copies this replaced
    /// each did `replacingOccurrences(of: "'", with: …)`, which compares
    /// grapheme clusters: an apostrophe followed by a combining mark (U+0301),
    /// a variation selector (U+FE0F), an emoji modifier, a zero-width joiner
    /// or a spacing mark — or preceded by a prepended concatenation mark — is
    /// one cluster that is not "'", so it was left unescaped, closed the
    /// quote, and the rest of the value ran as commands on the host. The
    /// values include what the host and other clients choose: pane working
    /// directories from herdr's snapshot, worktree paths, agent names,
    /// workspace and tab labels, tmux session names, file names in a listing.
    /// A shell reads bytes, and `'` is one byte of UTF-8 that is never part of
    /// another character, so escaping the scalar escapes exactly what the
    /// shell reads.
    ///
    /// A NUL can't be part of a word: a command line is a C string, so it ends
    /// there — inside the quotes, which are then unterminated, and the shell
    /// refuses the whole line. Nothing runs, the right outcome for a value no
    /// path or label can hold.
    static func quote(_ value: String) -> String {
        var quoted = String.UnicodeScalarView()
        quoted.append("'")
        for scalar in value.unicodeScalars {
            switch scalar {
            case "'": quoted.append(contentsOf: #"'\''"#.unicodeScalars)
            case "\\": quoted.append(contentsOf: #"'\\'"#.unicodeScalars)
            default: quoted.append(scalar)
            }
        }
        quoted.append("'")
        return String(quoted)
    }

    /// `script`, a POSIX sh script, as a command line any login shell runs as
    /// that script: `sh -c <script quoted>` (B138).
    ///
    /// sshd runs an exec request as `<login shell> -c <command>`, and the
    /// app's commands are sh. fish (3.7.1, 4.6.0) refuses `$?` and `$$`
    /// outright — "$? is not the exit status. In fish, please use $status."
    /// — and has no `while … do … done`: every herdr command (its exit-status
    /// trailer) and the install preflight (`${SHELL:-…}`) failed before
    /// anything ran, a path-probe value ending in `\` left fish's quote open,
    /// and zsh failed the cleanup loops whose globs matched nothing (its
    /// NOMATCH). Handed one quoted word, the login shell
    /// passes it to sh whole; `SSHExec` sends every exec request so, and the
    /// lines the app types into a terminal use the same where they loop.
    /// moshpit-ssh's `shell::portable` is the Rust side; its
    /// `harness/shells.py` runs every line the app builds through sh, dash,
    /// bash, zsh and fish 3 and 4.
    static func portable(_ script: String) -> String {
        "sh -c " + quote(script)
    }
}
