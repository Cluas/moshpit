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

    @Test("the attach line records $$ pre-exec, gates on has-session, and execs the client")
    func attachLineShape() {
        let line = SessionHub.ActiveSession.tmuxRendererAttachLine(
            tmux: "tmux", session: "0", connectionId: connId, nonce: "ab12cd34")

        #expect(line.hasPrefix("\u{15}"), "^U first — a half-initialised shell may hold swallowed junk")
        #expect(line.hasSuffix("\r"))
        #expect(line.contains("echo $$ > \"$HOME/.moshpit/tmuxr-\(connId.uuidString)-ab12cd34.pid\""),
                "the pid must be recorded BEFORE exec — exec preserves it, so the file names the tmux client")
        #expect(line.contains("tmux has-session -t '0' 2>/dev/null && exec tmux attach -t '0'"), """
                exec ties the renderer's life to the tmux client (detach kills the stack, ending the \
                immortal-zombie leak), and has-session gates it so a vanished session leaves the shell \
                alive for the retry loop instead of killing the whole mosh connection
                """)
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
