import Foundation
import Testing
@testable import Moshpit

/// The mosh+tmux renderer's leak hygiene: the attach line must leave a
/// killable pidfile trail and die with its tmux client, and the boot-time
/// cleanup must kill previous connects' stacks by that trail.
///
/// The incident (2026-08-19): every mosh+tmux connect typed a plain
/// `tmux attach` into the mosh shell, and mosh-server survives disconnects
/// by design — so shell and tmux client outlived every session. NINE
/// phone-sized zombie clients were found attached, contesting
/// `window-size latest` against the desktop client and flooding redraws —
/// the "SSH 乱码" resize war.
@Suite("mosh+tmux renderer hygiene")
@MainActor
struct TmuxRendererHygieneTests {

    private let connId = UUID(uuidString: "AAAAAAAA-BBBB-CCCC-DDDD-EEEEFFFF0000")!

    @Test("the renderer program records its pid pre-exec and execs the client on the remembered session, else the most recent")
    func rendererProgramShape() throws {
        let program = SessionHub.ActiveSession.tmuxRendererProgram(
            tmux: "tmux", remembered: "$3", connectionId: connId, nonce: "ab12cd34")
        // mosh-server's own program (B267): nothing typed, no ^U, no CR.
        #expect(!program.contains("\u{15}") && !program.contains("\r"))
        #expect(program.hasPrefix("/bin/sh -c '"))
        let script = try #require(PosixShell.shPayload(String(program.dropFirst("/bin/".count))))
        #expect(script == "mkdir -p \"$HOME/.moshpit\"; "
                + "echo $$ > \"$HOME/.moshpit/tmuxr-\(connId.uuidString)-ab12cd34.pid\"; "
                + "if tmux has-session -t '$3' 2>/dev/null; then exec tmux attach -t '$3'; fi; "
                + "until tmux has-session 2>/dev/null; do sleep 1; done; exec tmux attach", """
                the pid recorded BEFORE exec — exec preserves it, so the file names the tmux client; \
                exec ties the renderer's life to the tmux client; the remembered session by its id, \
                gated on has-session, else the most recent, else a wait for the first
                """)
        for word in ["new-session", "-CC new", "tmux new"] { #expect(!script.contains(word)) }
    }

    /// A remembered session is named by its `$N` id only (tmux never reuses
    /// one while its server lives); anything else posing as one — a name, a
    /// window id, an injection — is left out and the most recent is taken.
    @Test("the renderer program splices in only a real session id",
          arguments: ["dev", "@3", "$3;reboot", "$", ""])
    func rendererProgramIdOnly(_ remembered: String) throws {
        let program = SessionHub.ActiveSession.tmuxRendererProgram(
            tmux: "tmux", remembered: remembered, connectionId: connId, nonce: "n")
        let script = try #require(PosixShell.shPayload(String(program.dropFirst("/bin/".count))))
        #expect(!script.contains("has-session -t"), "\(script)")
        #expect(script.hasSuffix("until tmux has-session 2>/dev/null; do sleep 1; done; exec tmux attach"))
    }

    /// B267: the sidecar's chain and the SSH control client are their
    /// terminal channel's own program — tmux found as the probe finds it,
    /// the login shell after.
    @Test("the control client and the sidecar are their channel's program")
    func channelPrograms() {
        let path = "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\""
        let after = "exec \"${SHELL:-/bin/sh}\" -l"
        #expect(SessionHub.ActiveSession.tmuxControlAttachProgram(tmux: "tmux")
                == "\(path); tmux -CC attach || { until tmux has-session 2>/dev/null; do sleep 1; done; "
                + "exec tmux -CC attach; }; \(after)",
                "attach; with no session, wait for the person's first (made by exec), then attach it")
        #expect(SessionHub.ActiveSession.tmuxSidecarAttachProgram(
            tmux: "tmux", preferred: .init(id: "$3", name: "dev"))
                == "\(path); tmux -CC attach -t '$3' 2>/dev/null || tmux -CC attach -t '=dev' 2>/dev/null || "
                + "while ! tmux -CC attach 2>/dev/null; do sleep 2; done; \(after)")
        for program in [SessionHub.ActiveSession.tmuxControlAttachProgram(tmux: "tmux"),
                        SessionHub.ActiveSession.tmuxSidecarAttachProgram(tmux: "tmux", preferred: nil)] {
            #expect(!program.contains("\r") && !program.contains("\n") && !program.contains("\u{15}"))
            for word in ["new-session", "-CC new", "set -g", "set-option", "history-limit"] {
                #expect(!program.contains(word))
            }
        }
    }

    /// By id first: a session renamed while the app was suspended is not
    /// found by the name the sidecar knew (`-t '=dev'` fails once `dev` is
    /// `ops`, tmux 3.6a), and the rebuild fell through to the most recent
    /// session. tmux never reuses an id while its server lives.
    @Test("the sidecar goes back to its session by id, then by exact name, else to the most recent one")
    func sidecarChainIsExact() {
        typealias Return = SessionHub.ActiveSession.SidecarReturn
        // B138: one word for the login shell, the chain for sh — typed into
        // fish, the `while … do … done` was a syntax error.
        func chain(_ preferred: Return?) -> String? {
            PosixShell.shPayload(SessionHub.ActiveSession.tmuxSidecarAttachChain(tmux: "tmux", preferred: preferred))
        }
        #expect(chain(Return(id: "$3", name: "dev")) == "tmux -CC attach -t '$3' 2>/dev/null || "
                + "tmux -CC attach -t '=dev' 2>/dev/null || "
                + "while ! tmux -CC attach 2>/dev/null; do sleep 2; done",
                "the id is single-quoted: unquoted, the shell reads `$3` as an argument")
        #expect(chain(Return(id: "$3", name: nil))
                == "tmux -CC attach -t '$3' 2>/dev/null || while ! tmux -CC attach 2>/dev/null; do sleep 2; done")
        #expect(SessionHub.ActiveSession.tmuxSidecarAttachChain(tmux: "tmux", preferred: nil)
                == "sh -c 'while ! tmux -CC attach 2>/dev/null; do sleep 2; done'")
        // Only a session id is spliced in, never anything else posing as one.
        #expect(!SessionHub.ActiveSession.tmuxSidecarAttachChain(
            tmux: "tmux", preferred: Return(id: "$3;reboot", name: "dev")).contains("reboot"))
        #expect(!SessionHub.ActiveSession.tmuxSidecarAttachChain(
            tmux: "tmux", preferred: Return(id: "@3", name: "dev")).contains("@3"))
    }

    @Test("cleanup kills by pidfile for THIS connection only, then removes the trail")
    func cleanupShape() {
        let cmd = SessionHub.ActiveSession.tmuxRendererCleanupCommand(connectionId: connId)

        #expect(cmd.contains("\"$HOME/.moshpit/tmuxr-\(connId.uuidString)-\"*.pid"),
                "scoped to this connection's pidfiles — never someone else's clients")
        #expect(cmd.contains("kill $(cat \"$f\" 2>/dev/null) 2>/dev/null"))
        #expect(cmd.contains("rm -f"))
        #expect(cmd.hasSuffix("true"), "a host with no stale renderers owes nobody an error")
    }
}
