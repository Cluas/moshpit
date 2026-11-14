// mosh-live herdr-retarget (B208): the app's mosh+herdr raw-attach renderer
// on a real herdr through a real mosh-server. The loop is
// HerdrLaunch.rawAttachLoopCommand, typed into the mosh shell as the app
// types it (straight onto the transport, `\r` behind it); a switch is
// HerdrLaunch.retargetCommand run as an exec, as the sidecar runs it (one
// `sh -c` word, Shell.portable); a probe line goes down mosh after the
// switch, when the policy under test says, and only if the app would take
// the switch as landed (the rule of the tree built: build.sh). The panes
// record every byte typed into them (run.sh's recorder), so each probe is
// found in the pane it was for, in the other one, or nowhere.
//
//   mosh-live herdr-retarget <ssh host> <port> <user> <key> <known_hosts>
//       <herdr> <terminal A> <terminal B> <log A> <log B> <trials> <policy>...
//
// <herdr> is the custom path the loop runs herdr as (the private session's
// `herdr --session <name>`). Policies, measured from the moment the retarget
// exec came back:
//   landed      at once: typing at a pane just picked (`moshHerdrSwitching`
//               ends there)
//   poll        at the first 120 ms tick after it, ticks counted from the
//               switch: a reply (`deliverInput` samples `waitUntil` so)
//   d<ms>       that long after it
//   paint       after the first screen bytes that arrive after it
//   painted     after the new pane's own paint is on the mosh screen (its
//               marker line: 30 of the pane's letter) — the attach is up
//   back<ms>    two switches: away, then — that long after the first came
//               back — back again; the probe is for the pane switched back
//               to, sent the moment that exec came back (is the loop there?)
//   pair<ms>    the same, in one exec on the host, the second switch that
//               long after the first one's command ended
//   rawpair<ms> the same with the retarget before B208's fix (write the
//               target, kill the pid in the file), against the loop built
//               (<ms> `rand`: a gap anywhere from 40 to 90 ms, each try)
//   stay        no switch: input for the pane on screen, after the app's
//               check that the loop is still on it (where its rule has one)
//   left        the same after another client took that pane over (the
//               loop ended under it; $HERDR_EVICT as for `gone`)
//   gone        another client takes the pane on screen over first
//               ($HERDR_EVICT <seconds> <herdr argv> <terminal>, run on the
//               host): the loop's attach exits on its own and the loop ends,
//               leaving the login shell; then the switch, and the probe
//
// A probe the app would not send (the switch not taken as landed) counts as
// "not sent" — said, and kept by its sender.

import Foundation

/// A stand-in for "the app would send now" on the paths with no exec output.
enum HerdrLaunchMarker { static let landed = "LANDED" }

#if MOSHPIT_RETARGET_LANDED
let switchRule = "the app's (HerdrLaunch.retargetLanded: the loop came round to it; "
    + "the pane on screen: HerdrLaunch.rawAttachCheckCommand)"
func appTakesTheSwitch(_ output: Data) -> Bool { HerdrLaunch.retargetLanded(output) }
/// What the app asks before input for the pane already on screen.
func appCheck(_ terminal: String, _ key: String) -> String? {
    HerdrLaunch.rawAttachCheckCommand(terminalId: terminal, rendererKey: key)
}
func appTakesTheCheck(_ output: Data) -> Bool { HerdrLaunch.rawAttachUp(output) }
#else
let switchRule = "before B208's fix (the exec came back; the pane on screen taken as it is)"
func appTakesTheSwitch(_ output: Data) -> Bool { true }
func appCheck(_ terminal: String, _ key: String) -> String? { nil }
func appTakesTheCheck(_ output: Data) -> Bool { true }
#endif

func herdrRetarget(ssh: SSHSession, _ args: [String]) async {
    guard args.count >= 7 else {
        fail("usage: herdr-retarget … <herdr> <terminal A> <terminal B> <log A> <log B> <trials> <policy>...")
    }
    let herdr = args[0]
    let terminals = ["A": args[1], "B": args[2]]
    let logs = ["A": args[3], "B": args[4]]
    guard let trials = Int(args[5]) else { fail("trials: \(args[5])") }
    let policies = Array(args.dropFirst(6))
    guard herdr.contains(" --session ") else { fail("refused: \(herdr) is not a private session") }
    say("switch taken as landed: \(switchRule)")

    let session = try! await Session.start(ssh: ssh, udpHost: "127.0.0.1")
    /// Fails, stopping the server first: the loop dies with its shell.
    func bail(_ line: String) async -> Never {
        await session.transport.close()
        await session.stopServer()
        fail(line)
    }
    guard await session.ask("echo ready-$((6*7))\r", expecting: "ready-42", seconds: 10) != nil else {
        await bail("the shell never answered: \(session.text.suffix(200))")
    }
    let key = HerdrLaunch.moshRendererKey(connectionId: UUID(), nonce: String(UUID().uuidString.prefix(8)))
    let boot = HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: herdr) + "\r"
    say("loop: \(boot.dropLast())")
    await session.transport.send(Data(boot.utf8))

    /// The sidecar's exec: the retarget, run to its end, and what it said.
    func retarget(_ pane: String) async -> (began: Date, landed: Date, output: Data) {
        let began = Date()
        let command = Shell.portable(retargetLine(terminals[pane]!, key, herdr))
        let output = (try? await ssh.executeCommand(command)) ?? Data()
        return (began, Date(), output)
    }

    /// Everything typed into `pane` so far.
    func received(_ pane: String) -> Data {
        guard let text = try? String(contentsOfFile: logs[pane]!, encoding: .utf8) else { return Data() }
        var bytes = Data()
        for line in text.split(separator: "\n") {
            let fields = line.split(separator: " ")
            guard fields.count == 2 else { continue }
            var hex = Substring(fields[1])
            while hex.count >= 2 {
                if let byte = UInt8(hex.prefix(2), radix: 16) { bytes.append(byte) }
                hex = hex.dropFirst(2)
            }
        }
        return bytes
    }

    let marker = { (pane: String) in String(repeating: pane, count: 30) }

    /// Put the loop on `pane` and see its paint; boot it again first when
    /// it ended (`gone`).
    func settle(on pane: String, reboot: Bool) async {
        if reboot { await session.transport.send(Data(boot.utf8)) }
        let mark = session.paintCount
        _ = await retarget(pane)
        guard await session.waitForPaint(containing: marker(pane), after: mark, seconds: 10) != nil else {
            await bail("the loop never painted \(pane): \(session.text.suffix(300))")
        }
        try? await Task.sleep(for: .seconds(1))
    }

    var on = "A"
    await settle(on: "A", reboot: false)
    let run = String(key.suffix(4))

    for policy in policies {
        var outcome: [String: Int] = ["for": 0, "other": 0, "lost": 0, "held": 0]
        var waits: [Double] = []
        for trial in 0..<trials {
            // `stay` and `left` don't switch: the input is for the pane on
            // screen, which the app checks (if its rule does) instead.
            let staying = policy == "stay" || policy == "left"
            var pane = staying ? on : (on == "A" ? "B" : "A")
            var other = on == "A" ? "B" : "A"
            if !staying { other = on }
            try? await Task.sleep(for: .milliseconds(600))
            var loopEnded = false
            if policy == "gone" || policy == "left" {
                guard let evict = ProcessInfo.processInfo.environment["HERDR_EVICT"] else {
                    await bail("\(policy) needs HERDR_EVICT")
                }
                let said = try? await ssh.executeCommand("\(evict) 1.5 \(herdr) \(terminals[on]!)")
                guard let said, String(decoding: said, as: UTF8.self).contains("evicted") else {
                    await bail("the takeover didn't run")
                }
                loopEnded = true
                try? await Task.sleep(for: .milliseconds(300))
            }
            var mark = session.paintCount
            var began = Date(), landed = Date(), output = Data()
            if staying {
                // The app's check, when its rule has one; the output stands
                // in for a landed switch's.
                if let check = appCheck(terminals[pane]!, key) {
                    let said = (try? await ssh.executeCommand(Shell.portable(check))) ?? Data()
                    output = appTakesTheCheck(said) ? Data("\(HerdrLaunchMarker.landed)\n".utf8) : Data()
                } else {
                    output = Data("\(HerdrLaunchMarker.landed)\n".utf8)
                }
                landed = Date()
            } else {
                (began, landed, output) = await retarget(pane)
            }
            // `rawpair` asks where the loop went, whatever the app's rule.
            var bypassRule = false
            switch policy {
            case "landed", "gone", "stay", "left":
                break
            case "poll":
                // `waitUntil` samples every 120 ms from the switch.
                let since = landed.timeIntervalSince(began)
                let tick = (since / 0.120).rounded(.up) * 0.120
                let wait = max(0, began.addingTimeInterval(tick).timeIntervalSinceNow)
                if wait > 0 { try? await Task.sleep(for: .milliseconds(Int(wait * 1000))) }
            case "paint":
                _ = await session.waitForPaint(containing: nil, after: mark, landedAt: landed, seconds: 5)
            case "painted":
                _ = await session.waitForPaint(containing: marker(pane), after: mark, seconds: 5)
            case _ where policy.hasPrefix("back"):
                guard let gap = Int(policy.dropFirst(4)) else { await bail("policy \(policy)") }
                if gap > 0 { try? await Task.sleep(for: .milliseconds(gap)) }
                (pane, other) = (other, pane)
                mark = session.paintCount
                (began, landed, output) = await retarget(pane)
            case _ where policy.hasPrefix("pair") || policy.hasPrefix("rawpair"):
                // The same two switches in one exec, the second `gap` ms
                // after the first's command ended: the timing without the
                // exec's own jitter. `rawpair`: each switch only writes the
                // target and kills the pid in the file (the retarget before
                // B208's fix), whatever the loop is — the loop's own part.
                let raw = policy.hasPrefix("rawpair")
                bypassRule = raw
                let spec = policy.dropFirst(raw ? 7 : 4)
                // `…rand`: anywhere from 40 to 90 ms — wherever the loop's
                // window sits at this machine's load.
                guard let gap = spec == "rand" ? Int.random(in: 40...90) : Int(spec) else {
                    await bail("policy \(policy)")
                }
                (pane, other) = (other, pane)
                mark = session.paintCount
                began = Date()
                func command(_ pane: String) -> String {
                    guard raw else { return retargetLine(terminals[pane]!, key, herdr) }
                    return "printf '%s' \(Shell.quote(terminals[pane]!)) > \"$HOME/.moshpit/mosh-\(key).target\"; "
                        + "kill -9 $(cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null) 2>/dev/null || true"
                }
                let first = command(other)
                let second = command(pane)
                let both = "\(first) >/dev/null; sleep \(String(format: "%.3f", Double(gap) / 1000)); \(second)"
                output = (try? await ssh.executeCommand(Shell.portable(both))) ?? Data()
                landed = Date()
            default:
                guard policy.hasPrefix("d"), let ms = Int(policy.dropFirst()) else { await bail("policy \(policy)") }
                try? await Task.sleep(for: .milliseconds(ms))
            }
            var found = "held"
            let sent = Date()
            if bypassRule || (staying ? !output.isEmpty : appTakesTheSwitch(output)) {
                let probe = "p\(run)-\(policy)-\(trial)-\(pane)\r"
                await session.transport.send(Data(probe.utf8))
                // Where did it go?
                found = "lost"
                for _ in 0..<40 {
                    try? await Task.sleep(for: .milliseconds(50))
                    let want = Data(probe.dropLast().utf8)
                    if received(pane).range(of: want) != nil { found = "for"; break }
                    if received(other).range(of: want) != nil { found = "other"; break }
                }
                waits.append(sent.timeIntervalSince(landed) * 1000)
            }
            outcome[found, default: 0] += 1
            // The paint the `paint` policies waited for (others go before it).
            let firstPaint = policy.hasPrefix("paint") ? session.firstPaint(after: mark, landedAt: landed).map {
                String(format: "; first bytes %+.0f ms", $0.timeIntervalSince(landed) * 1000)
            } ?? "; no bytes" : ""
            var verdict = ["for": "arrived", "other": "WRONG PANE", "lost": "LOST", "held": "not sent (said)"][found]!
            if found == "lost", session.text.contains("not found: p\(run)-\(policy)-\(trial)-") {
                verdict += " (run in the login shell as a command)"
            }
            say(String(format: "%@ #%d → %@: exec %.0f ms%@; sent %@ → %@",
                       policy, trial, pane, landed.timeIntervalSince(began) * 1000, firstPaint,
                       found == "held" ? "no" : String(format: "%+.0f ms", sent.timeIntervalSince(landed) * 1000),
                       verdict))
            on = pane
            if found != "for" || loopEnded {
                // Where the loop is — or whether it still runs — is unknown:
                // put it on a pane (booting it again after `gone`/`left`) and
                // see it there before the next trial. After `left`, the other
                // one: the pane it ended on is still on mosh's screen, cell
                // for cell, so its repaint would send nothing to see.
                on = staying && loopEnded ? other : pane
                await settle(on: on, reboot: loopEnded)
            }
        }
        waits.sort()
        let median = waits.isEmpty ? 0 : waits[waits.count / 2]
        say("== \(policy): \(outcome["for"]!)/\(trials) arrived, \(outcome["lost"]!) lost, "
            + "\(outcome["other"]!) in the other pane, \(outcome["held"]!) not sent (said); "
            + "sent a median \(Int(median)) ms after the exec came back")
    }
    // The loop stays down once its target file is gone and its attach killed.
    _ = try? await ssh.executeCommand("rm -f \"$HOME/.moshpit/mosh-\(key).target\"; "
        + "kill -9 $(cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null) 2>/dev/null; "
        + "rm -f \"$HOME/.moshpit/mosh-\(key).pid\"")
    await session.transport.close()
    await session.stopServer()
}
