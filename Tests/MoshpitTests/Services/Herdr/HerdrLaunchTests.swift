import Foundation
import Testing
@testable import Moshpit

/// The mosh raw-attach renderer's command builders: a loop that keeps
/// `herdr terminal attach <terminal_id>` pointed at a target file —
/// `--takeover` only for the terminal the person chose — and the sidecar
/// command that redirects it.
@Suite("herdr raw-attach commands")
struct HerdrLaunchTests {

    private let id = UUID(uuidString: "6F9619FF-8B86-D011-B42D-00C04FC964FF")!
    private var key: String { HerdrLaunch.moshRendererKey(connectionId: id, nonce: "abc12345") }

    @Test("The loop is one line, waits for a target, and records the attach pid")
    func loopShape() throws {
        let typed = HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: nil)
        // Typed into a live shell — a raw newline would submit half a loop.
        #expect(!typed.contains("\n"))
        // One word for the login shell, the loop for sh (B138): typed into
        // fish, `while … do … done` was a syntax error.
        let line = try #require(PosixShell.shPayload(typed))
        // Waits for the sidecar to publish the first target rather than
        // failing: no ordering constraint between renderer and control plane.
        #expect(line.contains("if [ -n \"$tid\" ]"))
        #expect(line.contains("sleep 0.5"))
        // The attach must be the tty's FOREGROUND process (a backgrounded
        // one renders but never receives keys), so the pid the retarget
        // kills is captured by a wrapper writing its own $$ before exec'ing
        // into the attach — same pid, still foreground.
        #expect(line.contains("sh -c \"echo \\$\\$ >"))
        #expect(line.contains("exec herdr terminal attach \\\"\\$0\\\" \\$k 2>\\\"$HOME/.moshpit/mosh-\(key).err\\\""))
        // `--takeover` only for the terminal the take file names — the one
        // the person chose. Following the focus attached with it too, and a
        // second device lost its pane whenever this one followed the focus
        // onto it.
        let takeover = "k=; [ \\\"\\$(cat \\\"$HOME/.moshpit/mosh-\(key).take\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] && k=--takeover; "
        let decide = try #require(line.range(of: takeover), "no take file read: \(line)")
        #expect(!line.contains("\\$0\\\" --takeover"), "takes over whatever it attaches")
        // B208: once its pid is in the file, the wrapper reads the target
        // again and goes round (137, as if killed) if it has moved on. A
        // switch landing between the loop's read and the pid write killed a
        // pid already dead, and the loop attached the pane being LEFT.
        let recheck = "[ \\\"\\$(cat \\\"$HOME/.moshpit/mosh-\(key).target\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] || exit 137"
        let pidWrite = try #require(line.range(of: "echo \\$\\$ >"))
        let reread = try #require(line.range(of: recheck), "no second read of the target: \(line)")
        let attach = try #require(line.range(of: "exec herdr terminal attach"))
        #expect(pidWrite.upperBound <= reread.lowerBound && reread.upperBound <= attach.lowerBound,
                "the pid first, then the second read, then the attach")
        #expect(reread.upperBound <= decide.lowerBound && decide.upperBound <= attach.lowerBound,
                "the take file read for the target the wrapper read")
        #expect(!line.contains("& echo $!"))
        #expect(line.contains("mosh-\(key).pid"))
        #expect(line.contains("mosh-\(key).target"))
        // The probe's PATH trick rides along for Homebrew installs,
        // exported because an assignment prefix does not survive exec.
        #expect(line.contains("export PATH="))
        // Exit-status discrimination — the anti-storm rule: a retarget's
        // kill (signal death, ≥128) re-attaches; an attach that exits on
        // its own (evicted by --takeover, its pane closed, the server gone)
        // never grabs the pane back. It used to BREAK the loop, leaving the
        // login shell under mosh to run what was typed next as a command
        // line; now the loop PARKS — the stop record written, the screen
        // blanked — until a retarget's go file wakes it, and goes when its
        // target is removed (an orphan's cleanup).
        #expect(line.contains("s=$?; rm -f \"$HOME/.moshpit/mosh-\(key).pid\"; "
            + "printf \"\\033[0m\\033[H\\033[2J\"; if [ $s -lt 128 ]; then echo \"$s $tid\" > \"$HOME/.moshpit/mosh-\(key).stop\";"))
        #expect(line.contains("while [ ! -e \"$HOME/.moshpit/mosh-\(key).go\" ]; do "
            + "[ -e \"$HOME/.moshpit/mosh-\(key).target\" ] || exit 0; sleep 0.1; done"))
        #expect(!line.contains("|| break"), "an attach that ended on its own still ends the loop")
        // The go file is consumed only past the second read, by the attach
        // it is for: a parked loop never finds a stale one.
        let consume = try #require(line.range(of: "rm -f \\\"$HOME/.moshpit/mosh-\(key).go\\\""))
        #expect(reread.upperBound <= consume.lowerBound && consume.upperBound <= attach.lowerBound)
        // The loop's own pid, for the watch; and what was typed while no
        // pane showed is read away, raw and non-blocking, before the next
        // attach (and the first).
        #expect(line.contains("echo $$ > \"$HOME/.moshpit/mosh-\(key).loop\"; f=1;"))
        #expect(line.contains("stty -icanon min 0 time 0 2>/dev/null && cat >/dev/null"))
        // Nothing of the loop's own between two panes but a clear screen: its
        // stderr goes nowhere — bash's "Killed: 9" ~10 ms after a switch's
        // kill lifted the veil on the pane being left — and the pane being
        // left is wiped as its attach ends.
        #expect(line.hasPrefix("mkdir -p \"$HOME/.moshpit\"; exec 2>/dev/null; "))
        #expect(line.contains("s=$?; rm -f \"$HOME/.moshpit/mosh-\(key).pid\"; printf \"\\033[0m\\033[H\\033[2J\"; if [ $s -lt 128 ]"))
        // …and the pid file goes with its attach (B208): between attaches,
        // and once the loop has ended, a retarget finds no pid to kill — not
        // a dead one the system may since have given to another process.
        // The re-attach pause is switch latency the user watches the PREVIOUS
        // pane through, so it stays small — but not zero: this line is only
        // reached after a signal death, and one we didn't send would respawn
        // `herdr` as fast as it can die.
        #expect(line.contains("sleep 0.05"))
        #expect(!line.contains("sleep 0.3"))
    }

    @Test("Cleanup retires every previous generation, including the immortal 360/361 layout")
    func cleanupShape() throws {
        let cmd = HerdrLaunch.staleRendererCleanupCommand(connectionId: id)
        // Both layouts: nonce-less (360/361 — loops with no lose-gracefully
        // logic, immortal fighters) and the nonced generations.
        #expect(cmd.contains("mosh-\(id.uuidString)\".target"))
        #expect(cmd.contains("mosh-\(id.uuidString)-\"*.target"))
        #expect(cmd.contains("mosh-\(id.uuidString)\".pid"))
        #expect(cmd.contains("mosh-\(id.uuidString)-\"*.pid"))
        // Targets are removed BEFORE the kill: the orphan loop must wake to
        // "no target" and idle, not re-read its old pane and re-attach.
        let rmTargets = try #require(cmd.range(of: ".target"))
        let kill = try #require(cmd.range(of: "kill -9"))
        #expect(rmTargets.lowerBound < kill.lowerBound)
        // Never fails the bootstrap channel it runs on.
        #expect(cmd.hasSuffix("true"))
        // Only a renderer is killed: a pid file outlives a loop that died
        // with its mosh-server, and the system gives the pid to anything.
        // The command line must be `terminal attach` and the generation's
        // terminal id — read from /proc on Linux, `ps -o args=` elsewhere.
        #expect(cmd.contains("case \"$a\" in *\"terminal attach\"*\"$t\"*) kill -9 \"$p\""))
        #expect(cmd.contains("[ -r \"/proc/$p/cmdline\" ]") && cmd.contains("ps -o args= -p \"$p\""))
        #expect(cmd.contains("case \"$p\" in ''|*[!0-9]*) continue ;; esac"), "not a number: no kill")
        #expect(cmd.contains("[ -n \"$t\" ] || continue"), "no target to check against: no kill")
        #expect(!cmd.contains("kill -9 $(cat"), "a pid killed unseen")
    }

    @Test("A custom herdr path is trusted verbatim in the loop")
    func loopCustomPath() throws {
        let line = try #require(PosixShell.shPayload(
            HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: "/opt/herdr")))
        #expect(line.contains("exec /opt/herdr terminal attach"))
        #expect(line.contains("&& k=--takeover; exec /opt/herdr terminal attach \\\"\\$0\\\" \\$k 2>"))
        #expect(!line.contains("export PATH="))
    }

    @Test("Retarget publishes the terminal id single-quoted, bounces the attach, and says when the loop came round")
    func retargetShape() throws {
        let cmd = HerdrLaunch.retargetCommand(terminalId: "term_abc123", rendererKey: key, customPath: nil,
                                              takeover: true)
        // A terminal the server no longer lists is refused before the loop
        // is touched — asked of the herdr the loop runs — and a server that
        // doesn't answer lets the switch go ahead.
        #expect(cmd.hasPrefix("t='term_abc123'; s=$(PATH="))
        let check = try #require(cmd.range(of: "case \"$s\" in *'\"result\"'*) case \"$s\" in *\"\\\"$t\\\"\"*) ;; "
            + "*) echo \(HerdrLaunch.retargetMissingMarker); exit 0 ;; esac ;; esac"))
        let write = try #require(cmd.range(of: "printf '%s'"))
        #expect(check.upperBound <= write.lowerBound, "the check before the target is written")
        #expect(HerdrLaunch.retargetCommand(terminalId: "t", rendererKey: key, customPath: "/opt/herdr",
                                            takeover: false)
            .contains("s=$(/opt/herdr api snapshot 2>/dev/null)"))
        #expect(HerdrLaunch.retargetMissing(Data("MOSHPIT_RETARGET_MISSING\n".utf8)))
        #expect(!HerdrLaunch.retargetMissing(Data("MOSHPIT_RETARGETED\n".utf8)))
        #expect(cmd.contains("printf '%s' 'term_abc123' >"))
        // The go file wakes a loop that parked; one attached is woken by the kill.
        #expect(cmd.contains(": > \"$HOME/.moshpit/mosh-\(key).go\""))
        // A loop that is gone is said at once; one that never wrote its pid
        // at the end of the wait.
        #expect(cmd.contains("[ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && { echo \(HerdrLaunch.rendererGoneMarker); break; }"))
        #expect(cmd.contains("[ -s \"$HOME/.moshpit/mosh-\(key).loop\" ] || echo \(HerdrLaunch.rendererGoneMarker)"))
        // The kill only takes an attach: a loop that died with its shell
        // left its pid file, the pid free for the system to give away.
        #expect(cmd.contains("case \"$a\" in *\"terminal attach\"*) kill -9 \"$old\""))
        #expect(!cmd.contains("kill -9 $old"), "a pid killed unseen")
        #expect(cmd.contains("mosh-\(key).target"))
        // -9, not TERM: herdr exits gracefully (<128) on TERM, which the
        // loop's discrimination reads as an eviction and stays down.
        #expect(cmd.contains("old=$(cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null); case \"$old\" in"))
        // B208: the exec coming back is no proof the loop shows the target —
        // a loop that is gone (evicted, its pane closed) leaves a bare shell
        // under mosh. The marker is printed only once the loop has started
        // its next attach: a pid in the file other than the one killed, and
        // alive. Bounded: 40 looks, 50 ms apart.
        #expect(cmd.contains("[ -n \"$new\" ] && [ \"$new\" != \"$old\" ] && kill -0 \"$new\""))
        #expect(cmd.contains("echo \(HerdrLaunch.retargetLandedMarker)"))
        #expect(cmd.contains("[ $i -lt 40 ]"))
        #expect(cmd.contains("sleep 0.05"))
        let kill = try #require(cmd.range(of: "kill -9 \"$old\""))
        let look = try #require(cmd.range(of: "while [ $i -lt 40 ]"))
        #expect(kill.upperBound <= look.lowerBound, "the kill first, then the wait for the next attach")
        // A dead pid file or a loop that never comes round must not fail the
        // exec — the caller reads the marker, not the status.
        #expect(cmd.hasSuffix("; true"))
        #expect(HerdrLaunch.retargetLanded(Data("MOSHPIT_RETARGETED\n".utf8)))
        #expect(!HerdrLaunch.retargetLanded(Data()))
        #expect(!HerdrLaunch.retargetLanded(Data("kill: usage\n".utf8)))
        // Untrusted input is quoted — ids come from the server.
        let hostile = HerdrLaunch.retargetCommand(terminalId: "a'; rm -rf /", rendererKey: key, customPath: nil,
                                                  takeover: true)
        #expect(hostile.contains(#"'a'\'''"#) || !hostile.contains("rm -rf /; "))
    }

    @Test("A retarget takes the pane over only on the person's word; a follow's attach is plain and waits out herdr's refusal")
    func retargetTakesOverOnlyOnThePersonsWord() throws {
        let take = "\"$HOME/.moshpit/mosh-\(key).take\""
        let chosen = HerdrLaunch.retargetCommand(terminalId: "term_abc123", rendererKey: key, customPath: nil,
                                                 takeover: true)
        let follow = HerdrLaunch.retargetCommand(terminalId: "term_abc123", rendererKey: key, customPath: nil,
                                                 takeover: false)
        // The person's pick names its terminal in the take file; a follow
        // removes it. Written before the target: a wrapper that reads the
        // new target reads the take file that goes with it.
        let named = try #require(chosen.range(of: "printf '%s' 'term_abc123' > \(take); "))
        let removed = try #require(follow.range(of: "rm -f \(take); "))
        #expect(named.upperBound <= chosen.range(of: "mosh-\(key).target")!.lowerBound)
        #expect(removed.upperBound <= follow.range(of: "mosh-\(key).target")!.lowerBound)
        // A follow counts once herdr has had its moment to refuse the plain
        // attach (~260 ms, measured): parked by then, the stop record is
        // printed as the watch prints it, never the landing.
        let settle = "j=0; while [ $j -lt 12 ]; do if [ -s \"$HOME/.moshpit/mosh-\(key).stop\" ]; then "
            + "echo \(HerdrLaunch.rendererStoppedMarker); cat \"$HOME/.moshpit/mosh-\(key).stop\" "
            + "\"$HOME/.moshpit/mosh-\(key).err\" 2>/dev/null; exit 0; fi; sleep 0.05; j=$((j+1)); done; "
            + "echo \(HerdrLaunch.retargetLandedMarker)"
        #expect(follow.contains(settle))
        #expect(!chosen.contains("$j"), "the person's pick waits for nothing more")
        // herdr's refusal of a plain attach to a held terminal reads as the
        // pane being on another device, as a takeover does.
        let held = "herdr: server shut down: terminal attach failed: terminal term_1 already has an attached "
            + "client; retry with --takeover"
        #expect(HerdrLaunch.rendererStopKind(status: 1, said: held, terminalListed: true, serverUp: true) == .takenOver)
        #expect(HerdrLaunch.rendererStopKind(status: 1, said: held, terminalListed: false, serverUp: true) == .gone)
        let record = Data("MOSHPIT_RENDERER_STOPPED\n1 term_1\n\(held)\n".utf8)
        #expect(HerdrLaunch.rendererStop(record) == .parked(status: 1, terminal: "term_1", said: held))
        // The cleanup takes the take files with the rest.
        #expect(HerdrLaunch.staleRendererCleanupCommand(connectionId: id).contains("\"*.take; true"))
    }

    @Test("The check of the pane on screen reads the target and the attach's pid, and kills nothing")
    func checkShape() {
        let cmd = HerdrLaunch.rawAttachCheckCommand(terminalId: "term_abc123", rendererKey: key)
        // B208: a reply for the pane already on screen asks whether the loop
        // is still on it — the target file names it, the attach in the pid
        // file is alive (the file goes with its attach) — and never bounces
        // it: a retarget would take the pane back from whoever holds it.
        #expect(cmd.contains("[ \"$(cat \"$HOME/.moshpit/mosh-\(key).target\" 2>/dev/null)\" = 'term_abc123' ]"))
        #expect(cmd.contains("kill -0 \"$(cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null)\""))
        #expect(cmd.contains("echo \(HerdrLaunch.rawAttachUpMarker)"))
        #expect(!cmd.contains("kill -9"))
        #expect(!cmd.contains("printf"))
        #expect(cmd.hasSuffix("; true"))
        // When it isn't up because the loop parked, it says so as the watch
        // would: a reply that beat the watch to a takeover must not read as
        // a switch that never landed (the next poll took the pane back).
        #expect(cmd.contains("if [ -s \"$HOME/.moshpit/mosh-\(key).stop\" ]; then echo \(HerdrLaunch.rendererStoppedMarker);"))
        #expect(HerdrLaunch.rawAttachUp(Data("MOSHPIT_ATTACHED\n".utf8)))
        #expect(!HerdrLaunch.rawAttachUp(Data()))
        // Ids come from the server: quoted.
        let hostile = HerdrLaunch.rawAttachCheckCommand(terminalId: "a'; rm -rf /", rendererKey: key)
        #expect(hostile.contains("= 'a'\\''; rm -rf /' ]"))
    }

    @Test("The watch waits for the loop to park or go, says why, and steps aside for a switch")
    func watchShape() throws {
        let cmd = HerdrLaunch.rawAttachWatchCommand(rendererKey: key)
        #expect(cmd.hasPrefix("w=; read w < \"$HOME/.moshpit/mosh-\(key).pid\""))
        #expect(cmd.contains("if [ -s \"$HOME/.moshpit/mosh-\(key).stop\" ]; then echo MOSHPIT_RENDERER_STOPPED; "
            + "cat \"$HOME/.moshpit/mosh-\(key).stop\" \"$HOME/.moshpit/mosh-\(key).err\""))
        #expect(cmd.contains("echo MOSHPIT_RENDERER_GONE"))
        // A switch put another attach in the pid file: nothing to say.
        #expect(cmd.contains("[ -n \"$p\" ] && [ \"$p\" != \"$w\" ] && break"))
        // Bounded: 300 looks, 200 ms apart.
        #expect(cmd.contains("[ $i -lt 300 ]") && cmd.contains("sleep 0.2"))
        #expect(!cmd.contains("kill -9") && !cmd.contains("rm "), "reads only")
        #expect(cmd.hasSuffix("; true"))
    }

    @Test("A stop record reads back as the loop wrote it; herdr's words decide who brings the renderer back")
    func stopRecords() {
        let taken = "herdr: server shut down: terminal attach taken over"
        let said = Data("MOSHPIT_RENDERER_STOPPED\n1 term_65ffe2a36e7ee1\n\(taken)\n".utf8)
        #expect(HerdrLaunch.rendererStop(said) == .parked(status: 1, terminal: "term_65ffe2a36e7ee1", said: taken))
        #expect(HerdrLaunch.rendererStop(Data("MOSHPIT_RENDERER_GONE\n".utf8)) == .loopGone)
        #expect(HerdrLaunch.rendererStop(Data()) == nil)
        #expect(HerdrLaunch.rendererStop(Data("MOSHPIT_RETARGETED\n".utf8)) == nil)
        // herdr 0.9.5's words, measured live.
        let kind = { (status: Int, said: String, listed: Bool) in
            HerdrLaunch.rendererStopKind(status: status, said: said, terminalListed: listed, serverUp: true)
        }
        #expect(kind(1, taken, true) == .takenOver)
        #expect(kind(1, taken, false) == .gone, "taken over, then closed")
        #expect(kind(1, "herdr: server shut down: terminal attach ended: terminal t not found", true) == .gone)
        #expect(kind(1, "herdr: server shut down: terminal attach failed: terminal t not found", true) == .gone)
        #expect(kind(1, "herdr: server shut down: server is shutting down", true) == .gone)
        // A shell that couldn't run herdr is no pane closing.
        #expect(kind(127, "sh: 1: exec: herdr: not found", true) == .refused)
        #expect(kind(1, "herdr: something new", true) == .refused)
        #expect(kind(1, "herdr: something new", false) == .gone)
        #expect(HerdrLaunch.rendererStopKind(status: 1, said: "x", terminalListed: true, serverUp: false) == .gone)
    }

    @Test("The switch veil lifts on the new pane's repaint, never on the loop's own noise or the pane being left")
    func switchCoverWaitsForTheRepaint() {
        let t0 = Date(timeIntervalSinceReferenceDate: 1_000)
        let at = { (ms: Double) in t0.addingTimeInterval(ms / 1000) }
        // Never on a boundary: a `Date` is a Double, and 1000.42 − 1000.27
        // is 0.14999999999997726 — short of the 150 ms by a rounding.
        var cover = MoshSwitchCover(since: t0)
        // bash's "Killed: 9" ~10 ms after the kill, the pane being left
        // still printing at +50: before the landing, they don't count.
        cover.painted(at: at(10))
        cover.painted(at: at(50))
        #expect(!cover.lifts(at: at(200)), "lifted on the loop's own noise (the old rule: +160 ms)")
        cover.landed(at: at(120))
        #expect(!cover.lifts(at: at(250)))
        cover.painted(at: at(250))   // herdr's repaint
        cover.painted(at: at(270))
        #expect(!cover.lifts(at: at(415)))
        #expect(cover.lifts(at: at(425)), "150 ms after the repaint's last bytes")

        // A repaint that beat the landing's exec back: the quiet counts from
        // the landing.
        var early = MoshSwitchCover(since: t0)
        early.painted(at: at(80))
        early.landed(at: at(120))
        #expect(!early.lifts(at: at(265)))
        #expect(early.lifts(at: at(275)))

        // Nothing painting under it at all: the cap.
        let idle = MoshSwitchCover(since: t0)
        #expect(!idle.lifts(at: at(2_990)))
        #expect(idle.lifts(at: at(3_010)))
    }

    @Test("The probe prints the marker only when terminal attach exists")
    func probeShape() {
        let probe = HerdrLaunch.rawAttachProbeCommand(customPath: nil)
        #expect(probe.contains("terminal attach --help"))
        #expect(probe.contains("&& echo MOSHPIT_RAW_ATTACH_OK"))
        #expect(probe.contains(">/dev/null 2>&1"))
    }
}
