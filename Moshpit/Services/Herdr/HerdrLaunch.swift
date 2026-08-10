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
