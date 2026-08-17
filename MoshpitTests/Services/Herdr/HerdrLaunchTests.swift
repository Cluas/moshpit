import Foundation
import Testing
@testable import Moshpit

/// The mosh raw-attach renderer's command builders: a loop that keeps
/// `herdr terminal attach <terminal_id> --takeover` pointed at a target
/// file, and the sidecar command that redirects it.
@Suite("herdr raw-attach commands")
struct HerdrLaunchTests {

    private let id = UUID(uuidString: "6F9619FF-8B86-D011-B42D-00C04FC964FF")!
    private var key: String { HerdrLaunch.moshRendererKey(connectionId: id, nonce: "abc12345") }

    @Test("The loop is one line, waits for a target, and records the attach pid")
    func loopShape() {
        let line = HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: nil)
        // Typed into a live shell — a raw newline would submit half a loop.
        #expect(!line.contains("\n"))
        // Waits for the sidecar to publish the first target rather than
        // failing: no ordering constraint between renderer and control plane.
        #expect(line.contains("if [ -n \"$tid\" ]"))
        #expect(line.contains("sleep 0.5"))
        // The attach must be the tty's FOREGROUND process (a backgrounded
        // one renders but never receives keys), so the pid the retarget
        // kills is captured by a wrapper writing its own $$ before exec'ing
        // into the attach — same pid, still foreground.
        #expect(line.contains("sh -c \"echo \\$\\$ >"))
        #expect(line.contains("exec herdr terminal attach \\\"\\$0\\\" --takeover"))
        #expect(!line.contains("& echo $!"))
        #expect(line.contains("mosh-\(key).pid"))
        #expect(line.contains("mosh-\(key).target"))
        // The probe's PATH trick rides along for Homebrew installs,
        // exported because an assignment prefix does not survive exec.
        #expect(line.contains("export PATH="))
        // Exit-status discrimination — the anti-storm rule: a retarget's
        // kill (signal death, ≥128) re-attaches; an attach that exits on
        // its own (evicted by --takeover, server gone) BREAKS instead of
        // grabbing the pane back. Orphan loops lose once and stay down.
        #expect(line.contains("[ $? -ge 128 ] || break"))
    }

    @Test("Cleanup retires every previous generation of this connection, and only this connection")
    func cleanupShape() {
        let cmd = HerdrLaunch.staleRendererCleanupCommand(connectionId: id)
        // All nonces of THIS connection id — wildcard after the id…
        #expect(cmd.contains("mosh-\(id.uuidString)-\""))
        #expect(cmd.contains("*.pid"))
        #expect(cmd.contains("rm -f"))
        // …kill whatever attach each generation still holds.
        #expect(cmd.contains("kill $(cat \"$f\""))
        // Never fails the bootstrap channel it runs on.
        #expect(cmd.hasSuffix("true"))
    }

    @Test("A custom herdr path is trusted verbatim in the loop")
    func loopCustomPath() {
        let line = HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: "/opt/herdr")
        #expect(line.contains("exec /opt/herdr terminal attach"))
        #expect(!line.contains("export PATH="))
    }

    @Test("Retarget publishes the terminal id single-quoted and bounces the attach")
    func retargetShape() {
        let cmd = HerdrLaunch.retargetCommand(terminalId: "term_abc123", rendererKey: key)
        #expect(cmd.contains("printf '%s' 'term_abc123' >"))
        #expect(cmd.contains("mosh-\(key).target"))
        // -9, not TERM: herdr exits gracefully (<128) on TERM, which the
        // loop's discrimination reads as an eviction and stays down.
        #expect(cmd.contains("kill -9 $(cat"))
        // A dead pid file must not fail the exec — the loop may be between
        // attaches (or the pane vanished) and the write alone still lands.
        #expect(cmd.hasSuffix("|| true"))
        // Untrusted input is quoted — ids come from the server.
        let hostile = HerdrLaunch.retargetCommand(terminalId: "a'; rm -rf /", rendererKey: key)
        #expect(hostile.contains(#"'a'\'''"#) || !hostile.contains("rm -rf /; "))
    }

    @Test("The probe prints the marker only when terminal attach exists")
    func probeShape() {
        let probe = HerdrLaunch.rawAttachProbeCommand(customPath: nil)
        #expect(probe.contains("terminal attach --help"))
        #expect(probe.contains("&& echo MOSHPIT_RAW_ATTACH_OK"))
        #expect(probe.contains(">/dev/null 2>&1"))
    }
}
