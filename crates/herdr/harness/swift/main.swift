// Harness-only command-line front end for the app's herdr code.
//
// Built by harness/build-swift.sh together with the UNCHANGED app sources
// (Moshpit/Services/Herdr/*.swift and what they need). It reads a corpus of
// cases, one per line, runs each through the app's own code and prints one
// result line per case, in exactly the format src/harness.rs prints for the
// Rust port; harness/equivalence.py compares the two.
//
//   herdr-swift cases <corpus>
//
// A case is a kind and its arguments separated by spaces: `x:<hex>` is a
// string (its UTF-8 bytes) or raw bytes, `-` is nil, anything else is a
// number or a word. Results print strings as "…" with printable ASCII as
// itself and every other scalar as \u{HEX}, bytes as b:<hex>, nil as nil.

import Foundation

// MARK: - Printing

func quoted(_ value: String) -> String {
    var out = "\""
    for scalar in value.unicodeScalars {
        switch scalar {
        case "\"": out += "\\\""
        case "\\": out += "\\\\"
        case " "..."~": out.unicodeScalars.append(scalar)
        default: out += "\\u{" + String(scalar.value, radix: 16, uppercase: true) + "}"
        }
    }
    return out + "\""
}

func quotedOpt(_ value: String?) -> String { value.map(quoted) ?? "nil" }

func hex<C: Collection>(_ bytes: C) -> String where C.Element == UInt8 {
    bytes.map { String(format: "%02x", $0) }.joined()
}

func unhex(_ text: Substring) -> Data {
    var data = Data(capacity: text.count / 2)
    var index = text.startIndex
    while index < text.endIndex {
        let next = text.index(index, offsetBy: 2)
        data.append(UInt8(text[index..<next], radix: 16)!)
        index = next
    }
    return data
}

func byUTF8(_ a: String, _ b: String) -> Bool {
    Array(a.utf8).lexicographicallyPrecedes(Array(b.utf8))
}

func frameText(_ frame: HerdrFrame) -> String {
    switch frame {
    case .screen(let seq, let width, let height, let full, let bytes):
        return "screen(\(seq),\(width),\(height),\(full),b:\(hex(bytes)))"
    case .closed(let reason): return "closed(\(quotedOpt(reason)))"
    case .loopReady: return "ready"
    case .fault(let message): return "fault(\(quoted(message)))"
    }
}

func valueText(_ value: Any) -> String {
    if value is NSNull { return "null" }
    if let number = value as? NSNumber {
        let kind: String
        if CFGetTypeID(number) == CFBooleanGetTypeID() {
            kind = "bool"
        } else if number is NSDecimalNumber {
            kind = "dec"
        } else {
            switch String(cString: number.objCType) {
            case "q": kind = "int"
            case "Q": kind = "uint"
            case "d": kind = "dbl:" + String(format: "%016llx", number.doubleValue.bitPattern)
            case let other: kind = "other:" + other
            }
        }
        let int = (number as? Int).map(String.init) ?? "-"
        let u64 = (number as? UInt64).map(String.init) ?? "-"
        let bool = (number as? Bool).map(String.init) ?? "-"
        return "n(\(kind),\(int),\(u64),\(bool))"
    }
    if let text = value as? String { return quoted(text) }
    if let items = value as? [Any] { return "[" + items.map(valueText).joined(separator: ",") + "]" }
    if let object = value as? [String: Any] { return objectText(object) }
    return "other(\(type(of: value)))"
}

func objectText(_ object: [String: Any]) -> String {
    "{" + object.keys.sorted(by: byUTF8).map { quoted($0) + ":" + valueText(object[$0]!) }
        .joined(separator: ",") + "}"
}

func snapshotText(_ snapshot: TmuxSnapshot) -> String {
    let sessions = snapshot.sessions.keys.sorted(by: byUTF8).map { id -> String in
        let s = snapshot.sessions[id]!
        return "\(quoted(id))=(\(quoted(s.name)),\(s.isAttached))"
    }
    let windows = snapshot.windows.keys.sorted(by: byUTF8).map { id -> String in
        let w = snapshot.windows[id]!
        return "\(quoted(id))=(\(quoted(w.sessionId)),\(quoted(w.name)),\(w.index),\(quoted(w.layout)),\(w.isActive),\(w.paneCount),\(w.isZoomed))"
    }
    let panes = snapshot.panes.keys.sorted(by: byUTF8).map { id -> String in
        let p = snapshot.panes[id]!
        return "\(quoted(id))=(\(quoted(p.windowId)),\(p.index),\(quoted(p.command)),\(p.width),\(p.height),\(p.isActive))"
    }
    return "sessions[\(sessions.joined(separator: " "))] windows[\(windows.joined(separator: " "))] "
        + "panes[\(panes.joined(separator: " "))] "
        + "active(\(quotedOpt(snapshot.activeSessionId)),\(quotedOpt(snapshot.activeWindowId)),\(quotedOpt(snapshot.activePaneId))) "
        + "attached(\(snapshot.isAttached),\(snapshot.everAttached))"
}

func mapText(_ map: [String: String]) -> String {
    "[" + map.keys.sorted(by: byUTF8).map { "\(quoted($0))=\(quoted(map[$0]!))" }.joined(separator: " ") + "]"
}

func hooksText(_ hooks: [String: AgentHook]) -> String {
    "[" + hooks.keys.sorted(by: byUTF8).map { id -> String in
        let hook = hooks[id]!
        return "\(quoted(id))=(\(quotedOpt(hook.state)),\(quotedOpt(hook.agent)),\(quotedOpt(hook.title)))"
    }.joined(separator: " ") + "]"
}

func decodedText(_ decoded: HerdrSnapshot.Decoded) -> String {
    "\(snapshotText(decoded.snapshot)) hooks\(hooksText(decoded.agentHooks)) cwds\(mapText(decoded.paneCwds)) "
        + "tids\(mapText(decoded.terminalIds)) wt\(mapText(decoded.worktreeRepos))"
}

func reportText(_ report: HerdrVersionReport?) -> String {
    guard let report else { return "nil" }
    return "(\(quoted(report.client)),\(quoted(report.server)),\(report.mismatched),\(report.clientIsOlder),\(quoted(report.notice)))"
}

// MARK: - Plans (identical in src/harness.rs; SplitMix64 drives every random choice)

struct SplitMix64 {
    var state: UInt64
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}

func plan(_ spec: String, _ count: Int) -> [Range<Int>] {
    let fields = spec.split(separator: ":").map(String.init)
    var next: () -> Int
    switch fields[0] {
    case "whole": next = { max(count, 1) }
    case "bytes": next = { 1 }
    case "fixed":
        let size = Int(fields[1])!
        next = { size }
    case "rand":
        var rng = SplitMix64(state: UInt64(fields[1])!)
        let limit = UInt64(fields[2])!
        next = { Int(1 + rng.next() % limit) }
    default: fatalError("bad plan \(spec)")
    }
    var chunks: [Range<Int>] = []
    var at = 0
    while at < count {
        let end = min(count, at + next())
        chunks.append(at..<end)
        at = end
    }
    return chunks
}

// MARK: - Arguments

struct Args {
    var tokens: ArraySlice<Substring>

    mutating func next() -> Substring { tokens.popFirst()! }
    mutating func bytes() -> Data {
        let token = next()
        precondition(token.hasPrefix("x:"), "not bytes: \(token)")
        return unhex(token.dropFirst(2))
    }
    mutating func optBytes() -> Data? {
        if tokens.first == "-" { tokens.removeFirst(); return nil }
        return bytes()
    }
    mutating func string() -> String { String(decoding: bytes(), as: UTF8.self) }
    mutating func optString() -> String? { optBytes().map { String(decoding: $0, as: UTF8.self) } }
    mutating func int() -> Int { Int(next())! }
}

func list(_ text: String) -> [String] {
    text.isEmpty ? [] : text.split(separator: ",", omittingEmptySubsequences: false).map(String.init)
}

// MARK: - Runners for the control client

final class Runner: HerdrCommandRunner, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []
    private let reply: @Sendable (String) -> String

    init(reply: @escaping @Sendable (String) -> String) { self.reply = reply }

    var commands: [String] { lock.withLock { recorded } }

    func run(_ command: String) async throws -> String {
        lock.withLock { recorded.append(command) }
        return reply(command)
    }
}

final class Outputs: @unchecked Sendable {
    private let lock = NSLock()
    private var queue: [String]
    init(_ queue: [String]) { self.queue = queue }
    func next() -> String { lock.withLock { queue.isEmpty ? "" : queue.removeFirst() } }
}

func pollLine(_ customPath: String?) -> String {
    HerdrLaunch.command("api snapshot", customPath: customPath) + "; echo \"\(HerdrControlClient.exitCodeMarker)$?\""
}

// MARK: - Cases

@MainActor
func socketCase(_ args: inout Args) async -> String {
    let spec = String(args.next())
    let requests = args.int()
    let stream = args.bytes()
    final class Box: @unchecked Sendable {
        let lock = NSLock()
        var lines: [Data] = []
        var count: Int { lock.withLock { lines.count } }
    }
    let box = Box()
    let client = HerdrSocketClient(requestTimeout: .seconds(86_400)) { data in
        box.lock.withLock { box.lines.append(data) }
    }
    var tasks: [Task<String, Never>] = []
    for index in 0..<requests {
        tasks.append(Task {
            do {
                return "ok(\(quoted(try await client.request("probe"))))"
            } catch let error as HerdrSocketError {
                switch error {
                case .server(let code, let message): return "server(\(quoted(code)),\(quoted(message)))"
                case .connectionClosed: return "closed"
                case .timeout: return "timeout"
                }
            } catch {
                return "other"
            }
        })
        // One at a time, so the ids go out in order.
        while box.count < index + 1 { try? await Task.sleep(for: .milliseconds(1)) }
    }
    let collector = Task { () -> [String] in
        var kinds: [String] = []
        for await event in client.events { kinds.append(quoted(event.kind)) }
        return kinds
    }
    for chunk in plan(spec, stream.count) { await client.feed(stream.subdata(in: chunk)) }
    let ready = await client.waitForPump(timeout: .zero)
    await client.finishInput()
    let kinds = await collector.value
    var replies: [String] = []
    for (index, task) in tasks.enumerated() { replies.append("mp\(index + 1)=\(await task.value)") }
    return "ready=\(ready) events[\(kinds.joined(separator: " "))] replies[\(replies.joined(separator: " "))]"
}

@MainActor
func requestLine(_ send: @escaping @Sendable (HerdrSocketClient) async -> Void) async -> String {
    final class Box: @unchecked Sendable {
        let lock = NSLock()
        var line: Data?
        var value: Data? { lock.withLock { line } }
    }
    let box = Box()
    let client = HerdrSocketClient(requestTimeout: .seconds(86_400)) { data in
        box.lock.withLock { box.line = data }
    }
    let task = Task { await send(client) }
    while box.value == nil { try? await Task.sleep(for: .milliseconds(1)) }
    await client.finishInput()
    await task.value
    var line = String(decoding: box.value!, as: UTF8.self)
    if line.hasSuffix("\n") { line.removeLast() }
    return quoted(line)
}

@MainActor
func commandCase(_ args: inout Args) async -> String {
    let tree = args.string()
    let path = args.optString()
    let action = String(args.next())
    let manifest = action == "manifests" ? args.string() : ""
    // A healthy host: every other command exits 0. The app sends a
    // sequence's next step only once this one is known done (B121, round
    // 38), so an answer with no exit status stopped every sequence after its
    // first step here (New tab in another workspace listed `workspace focus`
    // alone), where the port's case lists what a healthy host is sent.
    let runner = Runner { command in
        if command.contains("api snapshot") { return tree }
        if command.contains("server agent-manifests") { return manifest }
        return "\(HerdrControlClient.exitCodeMarker)0\n"
    }
    let client = HerdrControlClient(runner: runner, customPath: path)
    await client.pollNowForTesting()
    let before = runner.commands.count
    var suffix = ""
    switch action {
    case "sel-session": client.selectSession(args.string())
    case "sel-window": client.selectWindow(args.string())
    case "sel-pane": client.selectPane(args.string())
    case "sel-pane-imm":
        client.immersiveZoom = true
        client.selectPane(args.string())
    case "kill-session": client.killSession(args.string())
    case "kill-window": client.killWindow(args.string())
    case "kill-pane": client.killPane(args.string())
    case "ren-session":
        let id = args.string()
        client.renameSession(id, to: args.string())
    case "ren-window":
        let id = args.string()
        client.renameWindow(id, to: args.string())
    case "new-session": client.newSession(named: args.optString())
    case "new-window": client.newWindow(named: args.optString())
    case "new-pane": client.newPane()
    case "new-window-in":
        let session = args.string()
        client.newWindow(inSession: session, named: args.optString())
    case "new-pane-in": client.newPane(inWindow: args.string())
    case "keys":
        let pane = args.string()
        await client.sendKeys(list(args.string()), paneId: pane)
    case "remove":
        let id = args.string()
        _ = await client.removeWorktree(sessionId: id, force: args.int() != 0)
    case "manifests":
        let names = await client.agentNames()
        suffix = " names[\(names.map(quoted).joined(separator: " "))]"
    default: return "error unknown action \(action)"
    }
    await client.drainCommandsForTesting()
    let poll = pollLine(path)
    let sent = runner.commands[before...].filter { $0 != poll }
    return "cmds[\(sent.map(quoted).joined(separator: " "))]" + suffix
}

@MainActor
func pollCase(_ args: inout Args) async -> String {
    let status = args.optString() ?? ""
    var outputs: [String] = []
    while !args.tokens.isEmpty { outputs.append(args.string()) }
    let queue = Outputs(outputs)
    let runner = Runner { command in
        if command.contains("herdr status") || command.hasSuffix(" status; echo \"__moshpit_rc=$?\"") {
            return status
        }
        return queue.next()
    }
    let client = HerdrControlClient(runner: runner, customPath: nil)
    var focus: [String] = []
    client.onFocusedPaneChanged = { focus.append($0) }
    var lines: [String] = []
    for _ in outputs {
        focus = []
        await client.pollNowForTesting()
        lines.append("[\(snapshotText(client.snapshot)) hooks\(hooksText(client.agentHooks)) "
            + "loaded=\(client.agentHooksLoaded) snr=\(client.serverNotRunning) "
            + "mm=\(quotedOpt(client.protocolMismatch)) skew=\(reportText(client.versionSkew)) "
            + "iv=\(client.pollInterval.components.seconds) focus[\(focus.map(quoted).joined(separator: " "))]]")
    }
    return lines.joined(separator: " ")
}

@MainActor
func run(_ line: Substring) async -> String {
    let tokens = line.split(separator: " ")
    let kind = String(tokens[0])
    var args = Args(tokens: tokens.dropFirst())
    let out: String
    switch kind {
    // MARK: frame channel
    case "frames":
        let spec = String(args.next())
        let stream = args.bytes()
        var parser = HerdrFrameParser()
        var frames: [String] = []
        for chunk in plan(spec, stream.count) {
            frames += parser.consume(stream.subdata(in: chunk)).map(frameText)
        }
        out = "[" + frames.joined(separator: " ") + "]"
    case "line":
        out = HerdrFrameParser.parse([UInt8](args.bytes())).map(frameText) ?? "nil"
    case "drop":
        out = String(HerdrFrame.isHandshakeDrop(args.string()))
    case "start", "boot":
        // The target, the grid, the herdr, `take` for the person's pick
        // (`--takeover`) or `plain`; a boot line then the connection whose
        // pid file it keeps.
        let target = args.string()
        let cols = args.int(), rows = args.int()
        let path = args.optString()
        let takeover = args.next() == "take"
        if kind == "start" {
            out = quoted(HerdrFrameCommand.start(target: target, cols: cols, rows: rows, customPath: path,
                                                 takeover: takeover))
        } else {
            let connection = args.string()
            out = quoted(HerdrFrameCommand.boot(target: target, cols: cols, rows: rows, customPath: path,
                                                takeover: takeover, connectionId: connection))
        }
    case "retarget":
        let target = args.string()
        let cols = args.int(), rows = args.int()
        let takeover = args.next() == "take"
        out = quoted(HerdrFrameCommand.retarget(target: target, cols: cols, rows: rows, takeover: takeover))
    case "input":
        out = quoted(HerdrFrameCommand.input(args.bytes()))
    case "resize":
        let cols = args.int(), rows = args.int()
        out = quoted(HerdrFrameCommand.resize(cols: cols, rows: rows))
    case "scroll":
        let up = args.int() != 0
        let lines = args.int()
        let source = HerdrFrameCommand.ScrollSource(rawValue: String(args.next()))!
        out = quoted(HerdrFrameCommand.scroll(up: up, lines: lines, source: source))
    case "consts":
        out = "\(quoted(HerdrFrameCommand.release)) \(quoted(HerdrFrameCommand.readyMarker)) \(quoted(HerdrFrameCommand.readyType))"
    case "paste":
        out = "b:" + hex(SessionHubExtract.herdrPaste(args.string()))
    case "arrow":
        out = quotedOpt(SessionHubExtract.arrowKeyName(args.bytes()))
    // MARK: JSON and snapshots
    case "json":
        out = (try? JSONSerialization.jsonObject(with: args.bytes())).map(valueText) ?? "nil"
    case "firstobj":
        out = HerdrSnapshot.firstJSONObject(in: args.string()).map(objectText) ?? "nil"
    case "snapshot":
        out = HerdrSnapshot.decode(args.string()).map(decodedText) ?? "nil"
    case "paneindex":
        out = String(HerdrSnapshot.paneIndex(args.string()))
    case "notrunning":
        out = String(HerdrSnapshot.isServerNotRunning(args.string()))
    case "mismatch":
        out = quotedOpt(HerdrSnapshot.protocolMismatch(args.string()))
    case "skew":
        let message = args.string()
        out = "\(HerdrSnapshot.looksLikeVersionSkew(message)) \(HerdrSnapshot.clientIsOlder(message))"
    // MARK: versions
    case "status":
        out = reportText(HerdrVersionReport.parse(args.string()))
    case "vcompare":
        let a = args.string(), b = args.string()
        switch HerdrVersionReport.compare(a, b) {
        case .orderedAscending: out = "-1"
        case .orderedSame: out = "0"
        case .orderedDescending: out = "1"
        }
    // MARK: socket
    case "socket":
        out = await socketCase(&args)
    case "request":
        let method = args.string()
        out = await requestLine { client in _ = try? await client.request(method) }
    case "subscribe":
        let kinds = list(args.string())
        let scoped = list(args.string()).map { entry -> (kind: String, paneId: String) in
            let parts = entry.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            return (String(parts[0]), parts.count > 1 ? String(parts[1]) : "")
        }
        out = await requestLine { client in try? await client.subscribe(kinds, paneScoped: scoped) }
    case "pumpboot":
        out = quoted(HerdrPushBoot.bootLine())
    case "kinds":
        out = HerdrPushDriver.globalKinds.joined(separator: " ") + " | "
            + HerdrPushDriver.paneScopedKinds.sorted().joined(separator: " ")
    // MARK: launch
    case "launch.attach":
        out = quoted(HerdrLaunch.attachCommand(customPath: args.optString()))
    case "launch.command":
        let sub = args.string()
        out = quoted(HerdrLaunch.command(sub, customPath: args.optString()))
    case "launch.daemon":
        out = quoted(HerdrLaunch.daemonCommand(customPath: args.optString()))
    case "launch.key":
        let id = UUID(uuidString: args.string())!
        out = quoted(HerdrLaunch.moshRendererKey(connectionId: id, nonce: args.string()))
    case "launch.paths":
        let key = args.string()
        out = "\(quoted(HerdrLaunch.moshTargetPath(rendererKey: key))) \(quoted(HerdrLaunch.moshPidPath(rendererKey: key)))"
    case "launch.loop":
        let key = args.string()
        out = quoted(HerdrLaunch.rawAttachLoopCommand(rendererKey: key, customPath: args.optString()))
    case "launch.cleanup":
        out = quoted(HerdrLaunch.staleRendererCleanupCommand(connectionId: UUID(uuidString: args.string())!))
    case "launch.retarget":
        // Whether the person chose the pane (`--takeover`), then the
        // terminal and the key.
        let takeover = args.int() != 0
        let terminal = args.string()
        let key = args.string()
        // The herdr the loop runs, when the case names one.
        let path = args.tokens.isEmpty ? nil : args.optString()
        out = quoted(HerdrLaunch.retargetCommand(terminalId: terminal, rendererKey: key, customPath: path,
                                                 takeover: takeover))
    case "launch.missing":
        out = String(HerdrLaunch.retargetMissing(args.bytes()))
    case "launch.probe":
        out = quoted(HerdrLaunch.rawAttachProbeCommand(customPath: args.optString()))
    case "launch.check":
        let terminal = args.string()
        out = quoted(HerdrLaunch.rawAttachCheckCommand(terminalId: terminal, rendererKey: args.string()))
    case "launch.watch":
        out = quoted(HerdrLaunch.rawAttachWatchCommand(rendererKey: args.string()))
    case "launch.loopup":
        out = quoted(HerdrLaunch.rawAttachLoopUpCommand(rendererKey: args.string()))
    case "launch.isup":
        out = String(HerdrLaunch.loopUp(args.bytes()))
    case "launch.stop":
        switch HerdrLaunch.rendererStop(args.bytes()) {
        case .parked(let status, let terminal, let said):
            out = "parked(\(status),\(quoted(terminal)),\(quoted(said)))"
        case .loopGone: out = "loopGone"
        case nil: out = "nil"
        }
    case "launch.stopkind":
        let status = args.int()
        let said = args.string()
        let listed = args.int() != 0
        let up = args.int() != 0
        switch HerdrLaunch.rendererStopKind(status: status, said: said, terminalListed: listed, serverUp: up) {
        case .takenOver: out = "takenOver"
        case .gone: out = "gone"
        case .refused: out = "refused"
        }
    case "launch.landed":
        out = String(HerdrLaunch.retargetLanded(args.bytes()))
    case "launch.up":
        out = String(HerdrLaunch.rawAttachUp(args.bytes()))
    case "quote":
        out = quoted(HerdrLaunch.quote(args.string()))
    // MARK: control
    case "ctl.parse":
        let result = HerdrControlClient.parse(args.string())
        out = "(\(quoted(result.output)),\(result.exitCode.map(String.init) ?? "nil"))"
    case "ctl.refusal", "ctl.notup":
        let output = args.string()
        let exit = String(args.next())
        let result: HerdrControlClient.CommandResult? = exit == "none" ? nil
            : HerdrControlClient.CommandResult(output: output, exitCode: exit == "-" ? nil : Int(exit)!)
        out = kind == "ctl.refusal"
            ? quotedOpt(HerdrControlClient.refusal(result))
            : String(HerdrControlClient.agentNotUpYet(result!))
    case "ctl.errmsg":
        out = quotedOpt(HerdrSnapshot.firstJSONObject(in: args.string()).flatMap(HerdrControlClient.errorMessage))
    case "ctl.first":
        let message = args.string(), pane = args.string()
        let version = args.optString()
        out = "\(quoted(HerdrControlClient.firstMessageCommand(message, paneId: pane, serverVersion: version))) "
            + "\(HerdrControlClient.typesWithoutSubmitting(serverVersion: version))"
    case "ctl.submit":
        out = quoted(HerdrControlClient.submitCommand(paneId: args.string()))
    case "ctl.homescan":
        out = quoted(HerdrControlClient.homeScanCommand)
    case "ctl.validate":
        let repo = args.string(), branch = args.string(), agent = args.string()
        out = quotedOpt(AgentTaskRequest(repoPath: repo, branch: branch, agent: agent).validationError)
    case "ctl.repolines":
        let output = args.string()
        let runner = Runner { _ in output }
        let repos = await HerdrControlClient(runner: runner).gitRepos(scanBudget: .seconds(3600))
        out = "[" + repos.map(quoted).joined(separator: " ") + "]"
    case "ctl.cmd":
        out = await commandCase(&args)
    case "ctl.poll":
        out = await pollCase(&args)
    default:
        return "error unknown case kind \(kind)"
    }
    return "\(kind) \(out)"
}

/// The parser alone: `iterations` fresh parsers over the stream, cut in
/// `chunk`-byte feeds (or whole); throughput in MB/s and the frame count.
func bench(path: String, chunk: String, iterations: Int) {
    let stream = FileManager.default.contents(atPath: path)!
    let size = chunk == "whole" ? stream.count : Int(chunk)!
    var chunks: [Data] = []
    var at = 0
    while at < stream.count {
        chunks.append(stream.subdata(in: at..<min(stream.count, at + size)))
        at += size
    }
    var frames = 0
    let clock = ContinuousClock()
    let took = clock.measure {
        for _ in 0..<iterations {
            var parser = HerdrFrameParser()
            frames = 0
            for piece in chunks { frames += parser.consume(piece).count }
        }
    }
    let seconds = Double(took.components.seconds) + Double(took.components.attoseconds) / 1e18
    print(String(format: "swift %.1f MB/s frames=%d", Double(stream.count * iterations) / 1e6 / seconds, frames))
}

@main
struct HerdrHarness {
    @MainActor
    static func main() async {
        let arguments = CommandLine.arguments
        if arguments.count == 5, arguments[1] == "bench" {
            bench(path: arguments[2], chunk: arguments[3], iterations: Int(arguments[4])!)
            return
        }
        guard arguments.count == 3, arguments[1] == "cases",
              let corpus = try? String(contentsOfFile: arguments[2], encoding: .utf8) else {
            FileHandle.standardError.write(Data("usage: herdr-swift cases <corpus>\n".utf8))
            exit(2)
        }
        for line in corpus.split(separator: "\n", omittingEmptySubsequences: true) {
            if line.hasPrefix("#") || line.allSatisfy({ $0 == " " || $0 == "\r" }) { continue }
            print(await run(line))
        }
    }
}
