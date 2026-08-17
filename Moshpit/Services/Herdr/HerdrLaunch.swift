import Foundation

/// How Moshpit starts herdr on the remote host.
///
/// Phase 0 runs herdr as a full-screen TUI inside whatever shell the transport
/// already gave us (mosh, or SSH with a PTY) — exactly the shape mosh+tmux has
/// today, with Moshpit acting purely as the renderer. The native control
/// plane (`herdr api snapshot`) and per-pane frame channel
/// (`herdr terminal session control`) come later; see
/// `docs/design/herdr-multiplexer.md`.
enum HerdrLaunch {

    /// Command that attaches the user's herdr session, for writing into an
    /// interactive remote shell (append a `\r` at the call site).
    ///
    /// Two things worth knowing:
    ///
    /// 1. **PATH.** herdr's official installer drops the binary in
    ///    `~/.local/bin` and deliberately does not touch any shell rc file, so
    ///    an interactive login shell often cannot see a herdr the capability
    ///    probe just found. We therefore boot with the same PATH prefix the
    ///    probe searched — otherwise the very command Install Assist hands the
    ///    user yields "herdr: command not found" on the next connect. Same
    ///    trick `MoshBootstrap` uses for `mosh-server`.
    /// 2. **A custom path is trusted verbatim.** It bypasses the probe (the
    ///    user vouched for it), so it must bypass the PATH prefix too.
    ///
    /// Unlike the tmux path, bare `herdr` CREATES a workspace when no server
    /// is running rather than reporting "no sessions". That breaks Moshpit's
    /// "never create a session on the user's behalf" rule, but Phase 0 has no
    /// control plane to detect emptiness with, and the terminal here IS
    /// herdr's own UI — the user sees exactly what typing `herdr` would do.
    /// Phase 1 adds the snapshot probe and the explicit empty state.
    static func attachCommand(customPath: String?) -> String {
        if let customPath, !customPath.isEmpty { return customPath }
        return "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" herdr"
    }

    /// A `herdr <subcommand>` invocation for the control channel (see
    /// ``HerdrControlClient``), carrying the same PATH treatment as
    /// ``attachCommand(customPath:)``.
    static func command(_ subcommand: String, customPath: String?) -> String {
        "\(attachCommand(customPath: customPath)) \(subcommand)"
    }

    /// Start a headless herdr server on the host, detached.
    ///
    /// This is the cold-host bootstrap. Moshpit only ever runs it from an
    /// explicit "create a session" tap — the same rule the tmux path follows,
    /// where a server with no sessions gets an empty state rather than one
    /// Moshpit quietly created. (Phase 0 booted herdr's TUI, which started a
    /// server as a side effect; native rendering has no such accident, which
    /// makes the rule easier to honor rather than harder.)
    ///
    /// Starting the server is only half the job: one with no persisted state
    /// comes up with zero workspaces, so the caller has to create one (see
    /// `HerdrControlClient.bootstrapServer`).
    ///
    /// All three descriptors are redirected and `nohup` applied because this
    /// runs on a one-shot exec channel: an inherited stdout would hold the
    /// channel open for the server's whole life, and the channel closing would
    /// otherwise HUP the server we just started.
    static func daemonCommand(customPath: String?) -> String {
        if let customPath, !customPath.isEmpty {
            return "nohup \(customPath) server >/dev/null 2>&1 </dev/null &"
        }
        return "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" "
            + "nohup herdr server >/dev/null 2>&1 </dev/null &"
    }

    // MARK: mosh raw-attach renderer (docs/design/roaming-transport.md)

    /// `herdr terminal attach <terminal_id> --takeover` renders ONE pane's
    /// raw stream straight to the tty — no sidebar, no header, and the pane
    /// PTY sized to THIS client alone (verified: `stty size` inside reports
    /// the phone grid; the desktop layout is untouched). It is the herdr
    /// analogue of the mosh+tmux renderer's zoomed attach, and strictly
    /// better than driving the shared TUI: nothing about the desktop user's
    /// view changes. Requires a herdr with the `terminal attach` subcommand
    /// (0.8.0 has it) — ``rawAttachProbeCommand`` decides, and hosts without
    /// it keep the full-TUI + immersive-zoom fallback.
    ///
    /// The renderer is a LOOP around the attach: the target lives in a file,
    /// and switching panes means writing the file and bouncing the current
    /// attach (``retargetCommand``) — the loop re-reads and re-attaches.
    /// The loop also self-heals an eviction (another client's --takeover):
    /// the attach exits, the loop attaches again a beat later.

    static func moshTargetPath(connectionId: UUID) -> String {
        "$HOME/.moshpit/mosh-\(connectionId.uuidString).target"
    }

    static func moshPidPath(connectionId: UUID) -> String {
        "$HOME/.moshpit/mosh-\(connectionId.uuidString).pid"
    }

    /// The renderer loop, typed into the mosh shell (append `\r` at the call
    /// site). Single line; waits for the sidecar to publish the first target.
    ///
    /// The attach MUST run in the foreground: backgrounding it (`… & wait`)
    /// leaves the loop shell as the tty's foreground process, and every
    /// keystroke lands there instead of in the pane — output kept flowing
    /// while typing died (found exactly that way, on-device shape). The pid
    /// the retarget needs is captured by a wrapper that writes its own `$$`
    /// and then `exec`s into the attach: same pid, still foreground.
    static func rawAttachLoopCommand(connectionId: UUID, customPath: String?) -> String {
        let target = moshTargetPath(connectionId: connectionId)
        let pid = moshPidPath(connectionId: connectionId)
        // Inner wrapper body, double-quoted for the OUTER shell: $HOME/… in
        // the pid path expands out there; `\$\$` and `\$0` survive into the
        // wrapper. `$0` carries the terminal id so it never re-enters shell
        // parsing. exec drops the wrapper; PATH rides an `export` because an
        // assignment prefix does not survive `exec`.
        let launch: String
        if let customPath, !customPath.isEmpty {
            launch = "exec \(customPath) terminal attach \\\"\\$0\\\" --takeover"
        } else {
            launch = "export PATH=\\\"$PATH:\(HostCapabilities.extraPathDirs)\\\"; "
                + "exec herdr terminal attach \\\"\\$0\\\" --takeover"
        }
        return "mkdir -p \"$HOME/.moshpit\"; while :; do "
            + "tid=$(cat \"\(target)\" 2>/dev/null); "
            + "if [ -n \"$tid\" ]; then "
            + "sh -c \"echo \\$\\$ > \\\"\(pid)\\\"; \(launch)\" \"$tid\"; "
            + "else sleep 0.5; fi; sleep 0.3; done"
    }

    /// Point the loop at a new terminal: publish the target, bounce the
    /// running attach. Runs on the sidecar's exec channel. Idempotence is the
    /// caller's job (skip when the target hasn't changed) — the bounce itself
    /// costs a visible reattach flicker.
    static func retargetCommand(terminalId: String, connectionId: UUID) -> String {
        let target = moshTargetPath(connectionId: connectionId)
        let pid = moshPidPath(connectionId: connectionId)
        return "printf '%s' \(quote(terminalId)) > \"\(target)\"; "
            + "kill $(cat \"\(pid)\" 2>/dev/null) 2>/dev/null || true"
    }

    /// Does this herdr know `terminal attach`? Prints the marker on yes;
    /// anything else (old herdr, no herdr) prints nothing and the mosh
    /// renderer falls back to the full TUI + immersive zoom.
    static func rawAttachProbeCommand(customPath: String?) -> String {
        "\(attachCommand(customPath: customPath)) terminal attach --help >/dev/null 2>&1 "
            + "&& echo MOSHPIT_RAW_ATTACH_OK"
    }

    /// POSIX single-quote a value destined for the remote shell.
    ///
    /// Workspace and tab labels can come from the user or from another client
    /// entirely, so they are untrusted: an unescaped `'` would close the quote
    /// and let the rest of the string run as commands. Close, escape, reopen —
    /// `'\''`. Same treatment `SessionHub` gives tmux session names.
    static func quote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}
