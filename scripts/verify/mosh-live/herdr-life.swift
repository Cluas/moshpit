// mosh-live herdr-life (round twenty-seven): what becomes of the app's
// mosh+herdr raw-attach renderer when its attach ends without the app —
// another client takes the pane over, the pane closes, herdr restarts — and
// when a switch names a terminal that is gone; and what the connect-time
// cleanup kills. A real herdr behind a real mosh-server; the loop, the
// retarget, the watch and the cleanup are the app's (HerdrLaunch, built by
// build.sh from the tree or from a commit); the panes record every byte
// typed into them (run.sh's recorder).
//
//   mosh-live herdr-life <ssh host> <port> <user> <key> <known_hosts>
//       <herdr> <pane A> <terminal A> <pane B> <terminal B> <log A> <log B>
//       <trials> <scenario>...
//
// Scenarios:
//   takeover  another client takes pane A over (`$HERDR_EVICT`, 1.5 s); a
//             line is typed 400 ms later, raw down mosh (what the app sends
//             before it knows: the old app never knew) — where did it go?;
//             then the app's own way back: the watch's word (where the tree
//             has one), and after the other client left, the take back (a
//             retarget to A): does A paint again, and does a probe reach it?
//             Also: did the loop take A back on its own?
//   close     pane B (on screen) closes; a line typed 400 ms later; then the
//             app follows the focus to A (a retarget): does A come back?
//   restart   herdr's server stops and starts again; a line typed 400 ms
//             after the stop; then the app follows the focused pane's new
//             terminal: does it come back?
//   missing   a switch to a terminal that no longer exists: what the
//             retarget says (landed or not), and — when the app takes it as
//             landed — where a probe sent then goes; then A again.
//   cover     a switch A → B, every mosh frame timed from the retarget's
//             start: the first one after the kill (the loop's own "Killed:
//             9" on a macOS host?), B's first paint, and when the switch
//             cover would lift — by the old rule (150 ms quiet after the
//             first bytes) and by the app's (bytes count from the landing).
//   cleanup   a stale pid file of an earlier generation naming an unrelated
//             process (a `sleep` started for the purpose): does the app's
//             connect-time cleanup kill it? And the live loop's attach,
//             as an orphan generation's: still killed?
//   follow    round twenty-eight, two devices on one herdr: this session
//             and a second mosh session with a loop of its own (another
//             phone). The other device picks pane B (`--takeover`, the
//             person's tap); this one follows the shared focus onto B, as
//             the app does on the poll's report. Does the other device keep
//             B (a probe typed there reaches B), what does this one's
//             retarget say, and does "Open here" (a retarget with
//             `--takeover`) then take B?
//   retype    round twenty-eight: a login shell still starting runs the
//             first copy of the start line while a second copy (the app's
//             retype) waits in the tty. In the app's order — both copies
//             acknowledged by mosh-server, the loop seen running, only then
//             the target — the loop's read of the tty before its first
//             attach takes the second copy away. Against it, the hazard the
//             order prevents (B170's for the tmux attach line): the target
//             published first, the second copy typed once the attach runs —
//             it reaches the pane as keystrokes.
//
// A typed line is `p<run>-<scenario>-<trial>\r`: found in a pane's log, it
// reached that pane; on the mosh screen as zsh's "command not found", it ran
// in the login shell; nowhere, it was discarded.

import Foundation

#if MOSHPIT_RETARGET_TAKEOVER
/// The app's retarget: `--takeover` only on the person's word (round
/// twenty-eight on); a follow is a plain attach.
func retargetLine(_ terminal: String, _ key: String, _ herdr: String, takeover: Bool = true) -> String {
    HerdrLaunch.retargetCommand(terminalId: terminal, rendererKey: key, customPath: herdr, takeover: takeover)
}
let takeoverRule = "the app's (round 28: --takeover only on the person's word)"
#elseif MOSHPIT_RETARGET_CHECKS
/// The app's retarget: it asks the herdr the loop runs whether the terminal
/// is still there (round twenty-seven on). Every attach takes over.
func retargetLine(_ terminal: String, _ key: String, _ herdr: String, takeover: Bool = true) -> String {
    HerdrLaunch.retargetCommand(terminalId: terminal, rendererKey: key, customPath: herdr)
}
let takeoverRule = "before round 28 (every attach --takeover)"
#else
func retargetLine(_ terminal: String, _ key: String, _ herdr: String, takeover: Bool = true) -> String {
    HerdrLaunch.retargetCommand(terminalId: terminal, rendererKey: key)
}
let takeoverRule = "before round 28 (every attach --takeover)"
#endif

#if MOSHPIT_RENDERER_PARKS
let lifeRule = "the app's (the loop parks; HerdrLaunch.rawAttachWatchCommand says why)"
func watchCommand(_ key: String) -> String? { HerdrLaunch.rawAttachWatchCommand(rendererKey: key) }
func watchSays(_ output: Data) -> String {
    switch HerdrLaunch.rendererStop(output) {
    case .parked(let status, let terminal, let said): return "parked (\(status) \(terminal): \(said))"
    case .loopGone: return "the loop is gone"
    case nil: return "nothing"
    }
}
#else
let lifeRule = "before round twenty-seven (an attach that ends on its own ends the loop; no watch)"
func watchCommand(_ key: String) -> String? { nil }
func watchSays(_ output: Data) -> String { "" }
#endif

func herdrLife(ssh: SSHSession, _ args: [String]) async {
    guard args.count >= 9 else {
        fail("usage: herdr-life … <herdr> <pane A> <terminal A> <pane B> <terminal B> <log A> <log B> <trials> <scenario>...")
    }
    let herdr = args[0]
    var panes = ["A": args[1], "B": args[3]]
    var terminals = ["A": args[2], "B": args[4]]
    let logs = ["A": args[5], "B": args[6]]
    guard let trials = Int(args[7]) else { fail("trials: \(args[7])") }
    let scenarios = Array(args.dropFirst(8))
    guard herdr.contains(" --session ") else { fail("refused: \(herdr) is not a private session") }
    say("renderer: \(lifeRule)")
    let here = ProcessInfo.processInfo.environment["MOSH_LIVE_HERE"] ?? "."

    let session = try! await Session.start(ssh: ssh, udpHost: "127.0.0.1")
    // The shim's exec blocks its actor for as long as the command runs; the
    // app's channels don't wait on each other. The watch and the other
    // client get connections of their own.
    let watcher = SSHSession(host: ssh.host, port: ssh.port, user: ssh.user, key: ssh.key, knownHosts: ssh.knownHosts)
    let other = SSHSession(host: ssh.host, port: ssh.port, user: ssh.user, key: ssh.key, knownHosts: ssh.knownHosts)
    func bail(_ line: String) async -> Never {
        await session.transport.close()
        await session.stopServer()
        fail(line)
    }
    guard await session.ask("echo ready-$((6*7))\r", expecting: "ready-42", seconds: 10) != nil else {
        await bail("the shell never answered: \(session.text.suffix(200))")
    }
    let connection = UUID()
    var key = HerdrLaunch.moshRendererKey(connectionId: connection, nonce: String(UUID().uuidString.prefix(8)))
    /// A new loop in the login shell; the one there (if any) killed first,
    /// so the shell has the terminal to read the line.
    func boot() async {
        _ = try? await ssh.executeCommand(Shell.portable(
            "l=$(cat \"$HOME/.moshpit/mosh-\(key).loop\" 2>/dev/null); [ -n \"$l\" ] && kill -9 $l; "
            + "p=$(cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null); "
            + "[ -n \"$p\" ] && q=$(ps -o ppid= -p $p) && kill -9 $q $p; true"))
        try? await Task.sleep(for: .milliseconds(300))
        key = HerdrLaunch.moshRendererKey(connectionId: connection, nonce: String(UUID().uuidString.prefix(8)))
        let line = "\u{15}" + HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: herdr) + "\r"
        await session.transport.send(Data(line.utf8))
        try? await Task.sleep(for: .milliseconds(300))
    }
    await boot()

    func exec(_ command: String) async -> Data {
        (try? await ssh.executeCommand(Shell.portable(command))) ?? Data()
    }
    /// `takeover`: the person's pick (and the setup's); a follow is plain.
    func retarget(_ terminal: String, takeover: Bool = true) async -> (ms: Int, output: Data) {
        let began = Date()
        let output = await exec(retargetLine(terminal, key, herdr, takeover: takeover))
        return (Int(Date().timeIntervalSince(began) * 1000), output)
    }
    func landed(_ output: Data) -> Bool { HerdrLaunch.retargetLanded(output) }
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
    /// Where a typed probe went: a pane, the login shell, or nowhere.
    func whereDid(_ probe: String, seconds: Double = 2) async -> String {
        let want = Data(probe.utf8)
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            for pane in ["A", "B"] where received(pane).range(of: want) != nil { return "pane \(pane)" }
            if session.text.contains("not found: \(probe)") { return "THE LOGIN SHELL (ran as a command)" }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return "nowhere (discarded)"
    }
    /// Put the loop on `pane` and see its paint — or, when it was on it
    /// already (mosh sends nothing for a repaint of the same cells), see
    /// that the last pane painted is this one.
    func settle(_ pane: String) async -> Bool {
        let mark = session.paintCount
        let (_, output) = await retarget(terminals[pane]!)
        guard landed(output) else { return false }
        if await session.waitForPaint(containing: marker(pane), after: mark, seconds: 3) != nil { return true }
        let text = session.text
        let mine = text.range(of: marker(pane), options: .backwards)?.lowerBound
        let other = text.range(of: marker(pane == "A" ? "B" : "A"), options: .backwards)?.lowerBound
        guard let mine else { return false }
        return other.map { mine > $0 } ?? true
    }
    /// Is the loop running at all (its shell, or the attach)?
    func loopAlive() async -> Bool {
        let said = await exec("p=$(cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null); "
            + "[ -n \"$p\" ] && kill -0 \"$p\" 2>/dev/null && echo ATTACHED; "
            + "[ -s \"$HOME/.moshpit/mosh-\(key).stop\" ] && echo PARKED; true")
        return !said.isEmpty
    }
    /// The loop's attach, by its pid file.
    func attachPid() async -> Int32? {
        Int32(String(decoding: await exec("cat \"$HOME/.moshpit/mosh-\(key).pid\" 2>/dev/null"), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines))
    }
    /// Until the attach `pid` has ended (this host is the test's own), then
    /// 100 ms: a line typed right after the renderer stopped. When it ended.
    @discardableResult
    func ended(_ pid: Int32?) async -> Date {
        guard let pid else { return Date() }
        for _ in 0..<300 where alive(pid) { try? await Task.sleep(for: .milliseconds(5)) }
        let at = Date()
        try? await Task.sleep(for: .milliseconds(100))
        return at
    }
    func recorder(_ pane: String) async {
        _ = await exec("\(herdr) pane run \(panes[pane]!) \"python3 -I '\(here)/herdr-recorder.py' \(pane) '\(logs[pane]!)'\"")
        try? await Task.sleep(for: .milliseconds(800))
    }
    func snapshotPanes() async -> [(pane: String, terminal: String)] {
        let output = await exec("\(herdr) api snapshot")
        guard let json = try? JSONSerialization.jsonObject(with: output) as? [String: Any],
              let result = json["result"] as? [String: Any],
              let snapshot = result["snapshot"] as? [String: Any],
              let list = snapshot["panes"] as? [[String: Any]] else { return [] }
        return list.compactMap { entry in
            guard let pane = entry["pane_id"] as? String, let terminal = entry["terminal_id"] as? String else { return nil }
            return (pane, terminal)
        }
    }
    let run = String(key.suffix(4))
    guard await settle("A") else { await bail("the loop never painted A: \(session.text.suffix(300))") }
    say("takeover: \(takeoverRule)")

    // The other device (follow): a second mosh session with a loop of its
    // own, steered over a connection of its own.
    var device2: (session: Session, ssh: SSHSession, key: String)?
    if scenarios.contains("follow") {
        let ssh2 = SSHSession(host: ssh.host, port: ssh.port, user: ssh.user, key: ssh.key, knownHosts: ssh.knownHosts)
        guard let session2 = try? await Session.start(ssh: ssh2, udpHost: "127.0.0.1") else {
            await bail("the other device's mosh session never started")
        }
        guard await session2.ask("echo ready-$((6*7))\r", expecting: "ready-42", seconds: 10) != nil else {
            await bail("the other device's shell never answered")
        }
        let key2 = HerdrLaunch.moshRendererKey(connectionId: UUID(), nonce: String(UUID().uuidString.prefix(8)))
        let line = "\u{15}" + HerdrLaunch.rawAttachLoopCommand(rendererKey: key2, customPath: herdr) + "\r"
        await session2.transport.send(Data(line.utf8))
        try? await Task.sleep(for: .milliseconds(300))
        device2 = (session2, ssh2, key2)
    }
    /// The other device's own retarget: always the person's pick there.
    func otherPick(_ terminal: String) async -> Data {
        guard let device2 else { return Data() }
        return (try? await device2.ssh.executeCommand(Shell.portable(
            retargetLine(terminal, device2.key, herdr, takeover: true)))) ?? Data()
    }
    /// Whether the other device's loop parked (its attach ended on its own).
    func otherParked() async -> String? {
        guard let device2 else { return nil }
        let said = (try? await device2.ssh.executeCommand(Shell.portable(
            "cat \"$HOME/.moshpit/mosh-\(device2.key).stop\" \"$HOME/.moshpit/mosh-\(device2.key).err\" 2>/dev/null; true")))
            ?? Data()
        let text = String(decoding: said, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    for scenario in scenarios {
        var tally: [String: Int] = [:]
        for trial in 0..<trials {
            try? await Task.sleep(for: .milliseconds(500))
            let probe = "p\(run)-\(scenario)-\(trial)"
            switch scenario {
            case "takeover":
                guard await settle("A") else { await bail("A never came back before the trial") }
                // The app's watch, armed on this landing, where the tree has one.
                let watchStarted = Date()
                let watching: Task<(Data, Date), Never>? = watchCommand(key).map { command in
                    Task { ((try? await watcher.executeCommand(Shell.portable(command))) ?? Data(), Date()) }
                }
                try? await Task.sleep(for: .milliseconds(200))
                guard let evict = ProcessInfo.processInfo.environment["HERDR_EVICT"] else { await bail("HERDR_EVICT") }
                let evicted = Date()
                let attach = await attachPid()
                let taker = Task { (try? await other.executeCommand("\(evict) 1.5 \(herdr) \(terminals["A"]!)")) ?? Data() }
                // Typed right after the attach ended (or 3 s on).
                let attachEnded = await ended(attach)
                await session.transport.send(Data("\(probe)\r".utf8))
                let went = await whereDid(probe)
                tally["typed → \(went)", default: 0] += 1
                var watched = "no watch"
                if let watching {
                    let (said, at) = await watching.value
                    watched = String(format: "the watch: %@ %.0f ms after the attach ended",
                                     watchSays(said), at.timeIntervalSince(attachEnded) * 1000)
                    _ = watchStarted
                }
                _ = await taker.value
                // The other client is gone: did the loop take A back on its own?
                try? await Task.sleep(for: .milliseconds(1000))
                let mark = session.paintCount
                try? await Task.sleep(for: .milliseconds(800))
                let selfRetook = session.firstPaint(containing: marker("A"), after: mark) != nil
                tally[selfRetook ? "took A back ON ITS OWN" : "left A alone", default: 0] += 1
                // The person's take back.
                let (ms, output) = await retarget(terminals["A"]!)
                var back = "never came back (the retarget said nothing)"
                if landed(output) {
                    let paint = await session.waitForPaint(containing: marker("A"), after: mark, seconds: 5)
                    await session.transport.send(Data("\(probe)b\r".utf8))
                    let reached = await whereDid("\(probe)b")
                    back = "back in \(ms) ms\(paint == nil ? " (no paint)" : ""), a probe → \(reached)"
                }
                tally["take back: " + (landed(output) ? "landed" : "not landed"), default: 0] += 1
                say("takeover #\(trial): typed 100 ms after the attach ended → \(went); \(watched); \(selfRetook ? "took A back on its own" : "left A alone"); take back \(back)")
                if !landed(output) { await boot() }
            case "close":
                guard await settle("B") else { await bail("B never painted before the trial") }
                let watching: Task<(Data, Date), Never>? = watchCommand(key).map { command in
                    Task { ((try? await watcher.executeCommand(Shell.portable(command))) ?? Data(), Date()) }
                }
                try? await Task.sleep(for: .milliseconds(200))
                let closed = Date()
                let attach = await attachPid()
                _ = await exec("\(herdr) pane close \(panes["B"]!)")
                let attachEnded = await ended(attach)
                await session.transport.send(Data("\(probe)\r".utf8))
                let went = await whereDid(probe)
                tally["typed → \(went)", default: 0] += 1
                var watched = "no watch"
                if let watching {
                    let (said, at) = await watching.value
                    watched = String(format: "the watch: %@ %.0f ms after the attach ended",
                                     watchSays(said), at.timeIntervalSince(attachEnded) * 1000)
                    _ = closed
                }
                // The app follows the focus to A.
                let mark = session.paintCount
                let (ms, output) = await retarget(terminals["A"]!, takeover: false)
                let paint = landed(output) ? await session.waitForPaint(containing: marker("A"), after: mark, seconds: 5) : nil
                let back = paint != nil
                tally[back ? "A back on screen" : "A NEVER BACK", default: 0] += 1
                say("close #\(trial): typed 100 ms after the attach ended → \(went); \(watched); A \(back ? "back in \(ms) ms" : "never back")")
                if !back { await boot(); _ = await settle("A") }
                // A new pane B for the next trial.
                _ = await exec("\(herdr) pane split \(panes["A"]!) --direction right")
                let now = await snapshotPanes()
                if let fresh = now.first(where: { $0.pane != panes["A"]! }) {
                    panes["B"] = fresh.pane
                    terminals["B"] = fresh.terminal
                    await recorder("B")
                }
            case "restart":
                guard await settle("A") else { await bail("A never painted before the trial") }
                let watching: Task<(Data, Date), Never>? = watchCommand(key).map { command in
                    Task { ((try? await watcher.executeCommand(Shell.portable(command))) ?? Data(), Date()) }
                }
                try? await Task.sleep(for: .milliseconds(200))
                let stopped = Date()
                let attach = await attachPid()
                _ = await exec("\(herdr) server stop")
                let attachEnded = await ended(attach)
                await session.transport.send(Data("\(probe)\r".utf8))
                let went = await whereDid(probe, seconds: 1.5)
                tally["typed → \(went)", default: 0] += 1
                var watched = "no watch"
                if let watching {
                    let (said, at) = await watching.value
                    watched = String(format: "the watch: %@ %.0f ms after the attach ended",
                                     watchSays(said), at.timeIntervalSince(attachEnded) * 1000)
                    _ = stopped
                }
                _ = await exec("nohup \(herdr) server >/dev/null 2>&1 </dev/null &")
                var fresh: [(pane: String, terminal: String)] = []
                for _ in 0..<50 {
                    fresh = await snapshotPanes()
                    if !fresh.isEmpty { break }
                    try? await Task.sleep(for: .milliseconds(200))
                }
                guard let a = fresh.first(where: { $0.pane == panes["A"]! }) ?? fresh.first else {
                    await bail("the server never came back")
                }
                panes["A"] = a.pane
                terminals["A"] = a.terminal
                if let b = fresh.first(where: { $0.pane != a.pane }) {
                    panes["B"] = b.pane
                    terminals["B"] = b.terminal
                }
                await recorder("A")
                await recorder("B")
                // The app follows the focused pane's new terminal.
                let mark = session.paintCount
                let (ms, output) = await retarget(terminals["A"]!, takeover: false)
                let paint = landed(output) ? await session.waitForPaint(containing: marker("A"), after: mark, seconds: 5) : nil
                tally[paint != nil ? "A back on screen" : "A NEVER BACK", default: 0] += 1
                say("restart #\(trial): typed 100 ms after the attach ended → \(went); \(watched); A \(paint != nil ? "back in \(ms) ms" : "never back")")
                if paint == nil { await boot(); _ = await settle("A") }
            case "missing":
                guard await settle("A") else { await bail("A never painted before the trial") }
                let gone = "term_\(String(UUID().uuidString.prefix(14)).lowercased().replacingOccurrences(of: "-", with: ""))"
                let (ms, output) = await retarget(gone)
                var went = "not sent (the switch didn't count)"
                if landed(output) {
                    await session.transport.send(Data("\(probe)\r".utf8))
                    went = await whereDid(probe)
                }
                tally[landed(output) ? "TAKEN AS LANDED, probe → \(went)" : "not landed", default: 0] += 1
                try? await Task.sleep(for: .milliseconds(600))
                let alive = await loopAlive()
                let back = await settle("A")
                tally[back ? "A back after" : "A NEVER BACK", default: 0] += 1
                say("missing #\(trial): the retarget took \(ms) ms, said \(landed(output) ? "landed" : String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines).prefix(40).description); probe → \(went); loop \(alive ? "still there" : "GONE"); A \(back ? "back" : "never back")")
                if !back { await boot(); _ = await settle("A") }
            case "cover":
                guard await settle("A") else { await bail("A never painted before the trial") }
                try? await Task.sleep(for: .milliseconds(400))
                let mark = session.paintCount
                let began = Date()
                let (ms, output) = await retarget(terminals["B"]!)
                try? await Task.sleep(for: .milliseconds(1500))
                let frames = session.frames(after: mark).map { (at: $0.at.timeIntervalSince(began) * 1000, text: $0.text) }
                let paintB = frames.first { $0.text.contains(marker("B")) }?.at
                let killed = frames.first { $0.text.contains("Killed") }?.at
                // The screen wiped before B's paint (the loop's clear after the
                // kill): mosh sends a cleared screen as erases, not a 2J.
                let wiped = frames.first {
                    !$0.text.contains(marker("B")) && !$0.text.contains(marker("A"))
                        && ($0.text.contains("[2J") || $0.text.contains("[J") || $0.text.contains("[K"))
                }?.at
                if ProcessInfo.processInfo.environment["HERDR_LIFE_DUMP"] != nil, let first = frames.first {
                    say("first frame: " + first.text.unicodeScalars.map { $0.value < 32 ? "^" + String(UnicodeScalar($0.value + 64)!) : String($0) }.joined().prefix(300))
                }
                /// When the cover lifts: sampled every 60 ms (as the app's
                /// loop does), 150 ms of quiet after the last bytes that
                /// count — any bytes (old rule), or bytes from the landing on,
                /// the quiet counted from the landing (the app's rule).
                func lift(countingFrom start: Double?) -> Double {
                    var t = 0.0
                    while t < 3000 {
                        t += 60
                        let counted = frames.filter { $0.at <= t && $0.at >= (start ?? 0) }
                        let last = start.map { max($0, counted.last?.at ?? 0) } ?? counted.last?.at
                        if let last, start.map({ t >= $0 }) ?? true, t - last >= 150 { return t }
                    }
                    return 3000
                }
                let old = lift(countingFrom: nil)
                let app = lift(countingFrom: Double(ms))
                let landedNote = landed(output) ? "landed at \(ms) ms" : "NOT landed"
                say(String(format: "cover #%d: %@; first bytes %@; \"Killed\" %@; wiped %@; B paints %@; cover lifts (old rule) %.0f ms%@, (app's) %.0f ms%@",
                           trial, landedNote,
                           frames.first.map { String(format: "%+.0f ms", $0.at) } ?? "none",
                           killed.map { String(format: "%+.0f ms", $0) } ?? "never",
                           wiped.map { String(format: "%+.0f ms", $0) } ?? "never",
                           paintB.map { String(format: "%+.0f ms", $0) } ?? "never",
                           old, paintB.map { old < $0 ? String(format: " — %.0f ms BEFORE B's paint", $0 - old) : "" } ?? "",
                           app, paintB.map { app < $0 ? String(format: " — %.0f ms before B's paint", $0 - app) : "" } ?? ""))
                tally[killed != nil ? "\"Killed\" on the screen" : "no kill noise", default: 0] += 1
                tally[wiped != nil ? "the pane left wiped before B paints" : "the pane left stays until B paints", default: 0] += 1
                if let paintB {
                    tally[old < paintB ? "old rule lifts before B paints" : "old rule waits for B", default: 0] += 1
                    tally[app < paintB ? "app's rule lifts before B paints" : "app's rule waits for B", default: 0] += 1
                }
            case "follow":
                guard let device2 else { await bail("no other device") }
                // This device on A; the other device's person picks B.
                guard await settle("A") else { await bail("A never painted before the trial") }
                let mark2 = device2.session.paintCount
                guard landed(await otherPick(terminals["B"]!)) else { await bail("the other device never reached B") }
                _ = await device2.session.waitForPaint(containing: marker("B"), after: mark2, seconds: 3)
                try? await Task.sleep(for: .milliseconds(300))
                // The shared focus is on B: this device follows it.
                let (ms, output) = await retarget(terminals["B"]!, takeover: false)
                let stop = HerdrLaunch.rendererStop(output)
                var mine: String
                if landed(output) {
                    mine = "LANDED on B (\(ms) ms)"
                } else if case .parked(_, let terminal, let said)? = stop {
                    mine = "said held: \(terminal) \(said.suffix(60)) (\(ms) ms)"
                } else {
                    mine = "said \(String(decoding: output, as: UTF8.self).prefix(60))"
                }
                tally[landed(output) ? "this device TOOK B by following the focus" : (stop != nil ? "this device said B is held" : "this device: neither"), default: 0] += 1
                try? await Task.sleep(for: .milliseconds(400))
                let parked = await otherParked()
                await device2.session.transport.send(Data("\(probe)o\r".utf8))
                let went = await whereDid("\(probe)o")
                tally[went == "pane B" ? "the other device kept B (its probe → pane B)" : "the other device LOST B (its probe → \(went))", default: 0] += 1
                // This device's "Open here": the person's word.
                let mark = session.paintCount
                let (backMs, back) = await retarget(terminals["B"]!, takeover: true)
                var openHere = "not landed"
                if landed(back) {
                    _ = await session.waitForPaint(containing: marker("B"), after: mark, seconds: 3)
                    await session.transport.send(Data("\(probe)m\r".utf8))
                    let reached = await whereDid("\(probe)m")
                    openHere = "landed in \(backMs) ms, a probe → \(reached)"
                    tally["Open here took B (probe → \(reached))", default: 0] += 1
                } else {
                    tally["Open here NOT landed", default: 0] += 1
                }
                try? await Task.sleep(for: .milliseconds(400))
                let parkedAfter = await otherParked()
                say("follow #\(trial): this device's follow → \(mine); the other device \(parked.map { "PARKED (\($0.replacingOccurrences(of: "\n", with: " ").suffix(70)))" } ?? "still on B"), its probe → \(went); Open here \(openHere); the other device then \(parkedAfter.map { "parked (\($0.replacingOccurrences(of: "\n", with: " ").suffix(50)))" } ?? "still on B")")
            case "retype":
                for order in ["app", "hazard"] {
                    // A fresh generation: the loop there now killed, its
                    // start line typed (the first copy, which the shell runs).
                    await boot()
                    let line = "\u{15}" + HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: herdr) + "\r"
                    /// mosh-server acknowledged what was typed (at most 2 s).
                    func acked() async {
                        let done = AckFlag()
                        Task { await session.transport.waitUntilDelivered(); done.set() }
                        for _ in 0..<40 where !done.value { try? await Task.sleep(for: .milliseconds(50)) }
                    }
                    let before = received("A").count
                    if order == "app" {
                        // The app's retype: a second copy behind the loop that
                        // started, acknowledged before anything else.
                        await session.transport.send(Data(line.utf8))
                        await acked()
                        #if MOSHPIT_LOOP_UP_CHECK
                        let up = HerdrLaunch.loopUp(await exec(HerdrLaunch.rawAttachLoopUpCommand(rendererKey: key)))
                        #else
                        let up = !(await exec("cat \"$HOME/.moshpit/mosh-\(key).loop\" 2>/dev/null")).isEmpty
                        #endif
                        tally[up ? "app order: the loop seen running" : "app order: NO LOOP", default: 0] += 1
                        if !up { say("no loop: " + String(session.text.suffix(300)).replacingOccurrences(of: "\u{1b}", with: "^[")) }
                        _ = await retarget(terminals["A"]!)
                    } else {
                        // The hazard: the target first, the copy once the attach runs.
                        _ = await retarget(terminals["A"]!)
                        try? await Task.sleep(for: .milliseconds(600))
                        await session.transport.send(Data(line.utf8))
                        await acked()
                    }
                    try? await Task.sleep(for: .milliseconds(800))
                    let typed = String(decoding: received("A").dropFirst(before), as: UTF8.self)
                    let leaked = typed.contains("moshpit") || typed.contains("while :")
                    tally["\(order) order: " + (leaked ? "the second copy REACHED pane A" : "nothing reached pane A"), default: 0] += 1
                    await session.transport.send(Data("\(probe)\(order)\r".utf8))
                    let went = await whereDid("\(probe)\(order)")
                    tally["\(order) order: a probe after → \(went)", default: 0] += 1
                    say("retype #\(trial) \(order): \(leaked ? "the second copy reached pane A (\(typed.count) bytes)" : "nothing of the line reached pane A"); a probe → \(went)")
                }
            case "cleanup":
                guard await settle("A") else { await bail("A never painted before the trial") }
                // A process that has nothing to do with Moshpit, its pid in an
                // earlier generation's pid file (that loop died with its
                // mosh-server; the system reused the pid).
                let decoy = String(decoding: await exec("nohup sleep 300 >/dev/null 2>&1 </dev/null & echo $!"), as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                guard Int(decoy) != nil else { await bail("no decoy: \(decoy)") }
                let stale = "\(connection.uuidString)-stale\(trial)"
                _ = await exec("mkdir -p \"$HOME/.moshpit\"; echo \(decoy) > \"$HOME/.moshpit/mosh-\(stale).pid\"; "
                    + "printf '%s' term_gone > \"$HOME/.moshpit/mosh-\(stale).target\"")
                let attach = String(decoding: await exec("cat \"$HOME/.moshpit/mosh-\(key).pid\""), as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                // This loop's shell: the cleanup takes its target (an earlier
                // generation to it), so it is killed before the next boot.
                let shell = String(decoding: await exec("ps -o ppid= -p \(attach)"), as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                _ = await exec(HerdrLaunch.staleRendererCleanupCommand(connectionId: connection))
                try? await Task.sleep(for: .milliseconds(300))
                let decoyAlive = !(await exec("kill -0 \(decoy) 2>/dev/null && echo alive")).isEmpty
                let attachAlive = !(await exec("kill -0 \(attach) 2>/dev/null && echo alive")).isEmpty
                tally[decoyAlive ? "unrelated process left alone" : "UNRELATED PROCESS KILLED", default: 0] += 1
                tally[attachAlive ? "orphan attach LEFT RUNNING" : "orphan attach killed", default: 0] += 1
                say("cleanup #\(trial): unrelated `sleep` \(decoy) \(decoyAlive ? "alive" : "KILLED"); the loop's attach \(attach) (an earlier generation to this cleanup) \(attachAlive ? "still running" : "killed")")
                _ = await exec("kill \(decoy) 2>/dev/null; kill -9 \(shell) 2>/dev/null; true")
                // The cleanup took this loop's target: a new generation.
                await boot()
            default:
                await bail("scenario \(scenario)")
            }
        }
        say("== \(scenario): " + tally.sorted { $0.key < $1.key }.map { "\($0.value)/\(trials) \($0.key)" }.joined(separator: "; "))
    }
    if let device2 {
        _ = try? await device2.ssh.executeCommand(Shell.portable(
            "p=$(cat \"$HOME/.moshpit/mosh-\(device2.key).pid\" 2>/dev/null); "
            + "l=$(cat \"$HOME/.moshpit/mosh-\(device2.key).loop\" 2>/dev/null); "
            + "rm -f \"$HOME/.moshpit/mosh-\(device2.key)\".*; "
            + "[ -n \"$p\" ] && kill -9 $p; [ -n \"$l\" ] && kill -9 $l; true"))
        await device2.session.transport.close()
        await device2.session.stopServer()
    }
    _ = await exec("rm -f \"$HOME/.moshpit/mosh-\(connection.uuidString)\"*")
    _ = await exec("for f in \"$HOME/.moshpit/mosh-\(connection.uuidString)\"*.pid; do kill -9 $(cat \"$f\" 2>/dev/null) 2>/dev/null; done; true")
    await session.transport.close()
    await session.stopServer()
}

/// Whether mosh-server acknowledged what was typed (`retype`).
final class AckFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var _value = false
    var value: Bool { lock.withLock { _value } }
    func set() { lock.withLock { _value = true } }
}
