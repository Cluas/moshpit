// Harness-only command-line front end for the app's Swift tmux controller.
//
// Built by harness/controller/build-swift.sh (SwiftPM, macOS) around the
// app's TmuxSessionController.swift as generate.py writes it out. Runs one
// script of crates/tmux/src/scenario.rs's format and prints the same log as
// the Rust runner (examples/controller_harness.rs), byte for byte where the
// two controllers agree.
//
//   controller-swift run <script> [<out>]
//
// Each step runs to quiescence before the next: every event the bytes make
// has been handled (a shadow parser counts them; `handle(_:)` counts what
// the controller handled), every task it started has had its turn, every
// command has reached the mock transport.

import Foundation

actor HarnessTransport: TmuxTransport {
    nonisolated let dataStream: AsyncStream<Data>
    nonisolated private let continuation: AsyncStream<Data>.Continuation
    private(set) var writes: [String] = []

    init() {
        var captured: AsyncStream<Data>.Continuation!
        dataStream = AsyncStream<Data>(bufferingPolicy: .unbounded) { captured = $0 }
        continuation = captured
    }

    func write(_ data: Data) async throws {
        var text = String(decoding: data, as: UTF8.self)
        if text.hasSuffix("\n") { text.removeLast() }
        writes.append(text)
    }

    nonisolated func push(_ data: Data) { continuation.yield(data) }
    nonisolated func finish() { continuation.finish() }
    func count() -> Int { writes.count }
    func all() -> [String] { writes }
}

final class EventCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func bump() { lock.withLock { value += 1 } }
    var count: Int { lock.withLock { value } }
}

struct Rule {
    var prefix: String
    var body: [String]?
    var error: Bool
    var after: String
}

func unescape(_ text: String) -> Data {
    let bytes = Array(text.utf8)
    var out: [UInt8] = []
    var at = 0
    while at < bytes.count {
        if bytes[at] != 0x5C || at + 1 >= bytes.count {
            out.append(bytes[at]); at += 1; continue
        }
        switch bytes[at + 1] {
        case UInt8(ascii: "n"): out.append(0x0A)
        case UInt8(ascii: "r"): out.append(0x0D)
        case UInt8(ascii: "t"): out.append(0x09)
        case UInt8(ascii: "e"): out.append(0x1B)
        case UInt8(ascii: "s"): out.append(0x20)
        case 0x5C: out.append(0x5C)
        case UInt8(ascii: "x") where at + 3 < bytes.count:
            if let byte = UInt8(String(decoding: bytes[(at + 2)...(at + 3)], as: UTF8.self), radix: 16) {
                out.append(byte); at += 4; continue
            }
            out.append(UInt8(ascii: "x"))
        default: out.append(bytes[at + 1])
        }
        at += 2
    }
    return Data(out)
}

func unhex(_ text: String) -> Data {
    let bytes = Array(text.utf8)
    var out: [UInt8] = []
    var at = 0
    while at + 1 < bytes.count {
        if let byte = UInt8(String(decoding: bytes[at...(at + 1)], as: UTF8.self), radix: 16) { out.append(byte) }
        at += 2
    }
    return Data(out)
}

/// One reply block, flags 1: a command from this client's own input, as
/// tmux writes them (the Rust rig's `block`).
func block(_ num: Int, _ lines: [String], _ error: Bool) -> String {
    var text = "%begin \(num) \(num) 1\n"
    for line in lines { text += line + "\n" }
    text += "\(error ? "%error" : "%end") \(num) \(num) 1\n"
    return text
}

@MainActor
final class Runner {
    var out = ""
    var controller: TmuxSessionController?
    let transport = HarnessTransport()
    let shadow = TmuxControlClient(backend: .swift)
    let counter = EventCounter()
    var renders = true
    var settings: [String: String] = [:]
    var answered = 0
    var printed = 0
    var rules: [Rule] = []
    var nextBlock = 1000
    var nextRequest = 0
    var detached = false
    var closed = false

    init() async {
        let c = counter
        await shadow.setCallbacks(
            onPaneOutput: { _, _ in c.bump() }, onLayoutChange: { _, _, _ in c.bump() },
            onWindowAdd: { _ in c.bump() }, onWindowClose: { _ in c.bump() },
            onWindowRenamed: { _, _ in c.bump() }, onSessionChanged: { _, _ in c.bump() },
            onSessionRenamed: { _, _ in c.bump() }, onSessionWindowChanged: { _, _ in c.bump() },
            onSessionsChanged: { c.bump() }, onUnlinkedWindowAdd: { _ in c.bump() },
            onUnlinkedWindowClose: { _ in c.bump() }, onUnlinkedWindowRenamed: { _, _ in c.bump() },
            onClientSessionChanged: { _, _, _ in c.bump() }, onActivePaneChanged: { _, _ in c.bump() },
            onPause: { _ in c.bump() }, onContinue: { _ in c.bump() },
            onClientDetached: { _ in c.bump() }, onExit: { _ in c.bump() },
            onCommandResponse: { _ in c.bump() })
    }

    func make() -> TmuxSessionController {
        if let controller { return controller }
        let made = TmuxSessionController(sshSession: transport, rendersOutput: renders)
        made.pathLinksFactory = { RemotePathLinks(paneId: $0) }
        made.onPaneActivity = { HarnessLog.item("note activity \($0)") }
        made.onPaneBell = { HarnessLog.item("note bell \($0)") }
        made.onAgentHooksUpdated = { HarnessLog.item("note hooks") }
        made.onFirstFramePainted = { HarnessLog.item("note first-frame") }
        made.onChannelSilent = { HarnessLog.item("note dead") }
        controller = made
        apply(made)
        return made
    }

    func apply(_ c: TmuxSessionController) {
        for (key, value) in settings {
            let f = Double(value) ?? 0
            switch key {
            case "label": c.sizeOwnerLabel = value
            case "steal": c.foregroundStealAfter = f
            case "stamp": c.leaseStampRefreshInterval = f
            case "probe": c.foregroundProbeTimeout = f
            case "quiet": c.foreignQuietRepinDelay = f
            case "streaming": c.streamingWindow = f
            default: break
            }
        }
    }

    // MARK: Quiescence

    func settleEvents() async {
        let deadline = Date().addingTimeInterval(10)
        while HarnessProbe.handled < counter.count, !detached, Date() < deadline {
            await Task.yield()
            try? await Task.sleep(nanoseconds: 20_000)
        }
    }

    func signature() async -> [Int] {
        [Int(HarnessClock.shared.registrations), await transport.count(), HarnessProbe.handled, HarnessLog.items.count]
    }

    func quiesce() async {
        var stable = 0
        var last = await signature()
        while stable < 6 {
            for _ in 0..<3 { await Task.yield() }
            if let controller { await controller.flushPendingWrites() }
            await settleEvents()
            let now = await signature()
            if now == last { stable += 1 } else { stable = 0; last = now }
        }
    }

    func recv(_ data: Data) async {
        await shadow.feed(data)
        transport.push(data)
        await settleEvents()
        await quiesce()
    }

    func wait(_ ms: UInt64) async {
        let target = HarnessClock.shared.now + ms
        while let continuation = HarnessClock.shared.takeDue(by: target) {
            continuation.resume()
            await quiesce()
        }
        HarnessClock.shared.standAt(target)
        await quiesce()
    }

    func respond() async -> Int {
        var count = 0
        let commands = await transport.all()
        var at = answered
        while at < commands.count {
            let command = commands[at]
            at += 1
            nextBlock += 1
            var text: String
            if let rule = rules.first(where: { command.hasPrefix($0.prefix) }) {
                text = block(nextBlock, rule.body ?? [], rule.error) + rule.after
            } else {
                text = block(nextBlock, ["0 0"], false)
            }
            answered = at
            await recv(Data(text.utf8))
            count += 1
        }
        return count
    }

    func settle() async {
        for _ in 0..<64 {
            if await respond() == 0, await transport.count() == answered { break }
        }
    }

    // MARK: Steps

    func print(_ line: String) { HarnessLog.item("=" + line) }

    func coordinator(_ pane: String) -> SwiftTerminalView.Coordinator? {
        controller?.coordinator(for: pane)
    }

    func activeCoordinator() -> SwiftTerminalView.Coordinator? {
        guard let c = controller, let pane = c.snapshot.activePaneId ?? c.snapshot.activePanes.first?.id else { return nil }
        return c.coordinator(for: pane)
    }

    func request() -> Int { nextRequest += 1; return nextRequest }

    func step(_ index: Int, _ line: String) async {
        let (op, arg): (String, String) = {
            guard let space = line.firstIndex(of: " ") else { return (line, "") }
            return (String(line[..<space]), String(line[line.index(after: space)...]))
        }()
        let words = arg.split(separator: " ").map(String.init)
        func num(_ i: Int) -> Int { i < words.count ? Int(words[i]) ?? 0 : 0 }
        func float(_ i: Int) -> Double { i < words.count ? Double(words[i]) ?? 0 : 0 }
        func split2(_ s: String) -> (String, String) {
            guard let space = s.firstIndex(of: " ") else { return (s, "") }
            return (String(s[..<space]), String(s[s.index(after: space)...]))
        }
        if op == "config" {
            for word in words {
                let kv = word.split(separator: "=", maxSplits: 1).map(String.init)
                guard kv.count == 2 else { continue }
                if kv[0] == "renders" { renders = kv[1] == "1" } else { settings[kv[0]] = kv[1] }
            }
            if let controller { apply(controller) }
            return
        }
        out += "@\(index) \(line)\n"
        let c = make()
        switch op {
        case "init-size": c.setInitialClientSize(cols: num(0), rows: num(1))
        case "client-name": c.ourClientName = arg
        case "restore":
            func part(_ i: Int) -> String? { i < words.count && words[i] != "-" ? words[i] : nil }
            c.pendingRestore = TmuxSelection(session: words.first ?? "", window: part(1), pane: part(2))
        case "inherit-pins":
            c.inheritPinLedger(.init(resizedWindows: Set(arg.split(separator: ",").map(String.init))))
        case "attach": await c.attach()
        case "begin": await c.beginControlMode()
        case "detach":
            await c.detach()
            await shadow.reset()
            detached = true
        case "close":
            transport.finish()
            closed = true
            let deadline = Date().addingTimeInterval(5)
            while !HarnessLog.items.contains("note dead"), Date() < deadline {
                try? await Task.sleep(nanoseconds: 1_000_000)
            }
        case "recv": await recv(unescape(arg))
        case "rule":
            let (kind, rest) = split2(arg)
            let parts = rest.components(separatedBy: " => ")
            let prefix = String(decoding: unescape(parts[0]), as: UTF8.self)
            let bodyText = parts.count > 1 ? parts[1] : "-"
            let body: [String]? = bodyText == "-" ? nil
                : String(decoding: unescape(bodyText), as: UTF8.self).components(separatedBy: "\n")
            let after = parts.count > 2 ? String(decoding: unescape(parts[2]), as: UTF8.self) : ""
            rules.append(Rule(prefix: prefix, body: body, error: kind == "err", after: after))
        case "rules-clear": rules.removeAll()
        case "skip": answered = await transport.count()
        case "respond": _ = await respond()
        case "settle": await settle()
        case "wait": await wait(UInt64(num(0)))
        case "waitsettle":
            let target = HarnessClock.shared.now + UInt64(num(0))
            let step = UInt64(max(1, num(1)))
            while HarnessClock.shared.now < target {
                await wait(min(step, target - HarnessClock.shared.now))
                await settle()
            }
        case "select-window": c.selectWindow(arg)
        case "select-pane": c.selectPane(arg)
        case "select-session": c.selectSession(arg)
        case "switch": c.switchPaneOrWindow(forward: arg == "next")
        case "new-window": c.newWindow(named: arg.isEmpty ? nil : arg)
        case "new-window-in":
            let (session, name) = split2(arg)
            c.newWindow(inSession: session, named: name.isEmpty ? nil : name)
        case "new-pane": c.newPane()
        case "new-pane-in": c.newPane(inWindow: arg)
        case "new-session": c.newSession(named: arg.isEmpty ? nil : arg)
        case "rename-session", "rename-window":
            let (id, name) = split2(arg)
            let decoded = String(decoding: unescape(name), as: UTF8.self)
            if op == "rename-session" { c.renameSession(id, to: decoded) } else { c.renameWindow(id, to: decoded) }
        case "kill-session": c.killSession(arg)
        case "kill-window": c.killWindow(arg)
        case "kill-pane": c.killPane(arg)
        case "refresh": c.refresh()
        case "run": c.runCommand(arg)
        case "resize":
            let changed = c.resizeClient(rows: num(1), cols: num(0))
            print(" resize \(changed ? 1 : 0)")
        case "input":
            let (pane, hex) = split2(arg)
            c.sendInput(unhex(hex), paneId: pane)
        case "paste":
            let (pane, text) = split2(arg)
            c.sendPaste(String(decoding: unescape(text), as: UTF8.self), paneId: pane)
        case "resync": c.resyncActivePane(reveal: arg == "1")
        case "release": c.releaseWindowPins()
        case "repin": c.repinActiveWindow()
        case "take": c.takeSizeOwnership()
        case "sync-mosh": c.syncMoshWindow(cols: num(0), rows: num(1))
        case "renderer-pid": Task { @MainActor in _ = await c.adoptRenderer(pid: num(0)) }
        case "poll": c.pollAgentHooks()
        case "count":
            let id = request()
            c.countNonControlClients { HarnessLog.item("note count \(id) \($0)") }
        case "path":
            let id = request()
            Task { @MainActor in
                let path = await c.panePath(arg)
                HarnessLog.item("note path \(id) \(path.map { harnessEscape($0) } ?? "-")")
            }
        case "prefix":
            let id = request()
            c.queryPrefixKey { HarnessLog.item("note prefix \(id) \(harnessEscape($0))") }
        case "cancel-copy": c.cancelStaleCopyMode()
        case "mouse": c.refreshActivePaneMouse()
        case "scroll": c.scroll(lines: num(0))
        case "scroll-px":
            HarnessView.cellHeight = CGFloat(float(1))
            let consumed = c.scroll(pixels: CGFloat(float(0)))
            print(" px \(String(format: "%.3f", Double(consumed)))")
        case "scroll-end": c.scrollDidEnd()
        case "scroll-begin": activeCoordinator()?.onScrollBegin?()
        case "click": activeCoordinator()?.onClick?(num(0), num(1))
        case "restore-status":
            let id = request()
            Task { @MainActor in
                await c.restoreSuppressedStatusAndFlush()
                HarnessLog.item("note flushed \(id)")
            }
        case "ledger":
            let ledger = c.takePinLedger()
            let pins = ledger.pinnedWindows.sorted(by: TmuxSnapshot.idPrecedes).joined(separator: ",")
            print(" ledger pins=\(pins) empty=\(ledger.isEmpty ? 1 : 0)")
        case "lease-release": for command in c.sizeLeaseReleaseCommands() { print(" \(command)") }
        case "layout-restore": for command in c.pristineLayoutRestoreCommands() { print(" \(command)") }
        case "mint": _ = c.surface(for: arg)
        case "live":
            let live = arg == "1"
            c.transportIsLive = { live }
        case "view-grid", "view-size":
            let pane = words.first ?? ""
            if let coordinator = coordinator(pane) {
                if let terminal = coordinator.terminal {
                    terminal.terminal.cols = num(1)
                    terminal.terminal.rows = num(2)
                }
                if op == "view-grid" { coordinator.onGridReport?(num(1), num(2)) } else { coordinator.onSizeChange?(num(1), num(2)) }
            }
        case "view-defer":
            let pane = words.first ?? ""
            var held = false
            if let coordinator = coordinator(pane) {
                held = coordinator.shouldDeferResize?(num(1), num(2)) ?? false
                if held {
                    coordinator.heldGrid = (num(1), num(2))
                } else if let terminal = coordinator.terminal {
                    terminal.terminal.cols = num(1)
                    terminal.terminal.rows = num(2)
                }
            }
            print(" defer \(held ? 1 : 0)")
        case "view-input":
            let (pane, hex) = split2(arg)
            coordinator(pane)?.onInput?(unhex(hex))
        case "view-mode": coordinator(arg)?.onInputModeChange?()
        case "view-return": coordinator(arg)?.onReturnSend?()
        case "view-bell": coordinator(arg)?.onBell?()
        case "view-reveal": coordinator(arg)?.reveal()
        case "dump": for line in dump(c) { print(" " + line) }
        default: print(" unknown step \(op)")
        }
        await quiesce()
        await flushStep()
    }

    func flushStep() async {
        let items = HarnessLog.items
        HarnessLog.items.removeAll()
        for item in items where !item.hasPrefix("=") { out += ". \(item)\n" }
        let commands = await transport.all()
        if printed < commands.count {
            for command in commands[printed...] { out += "> \(command)\n" }
            printed = commands.count
        }
        for item in items where item.hasPrefix("=") { out += "\(item)\n" }
    }

    func dump(_ c: TmuxSessionController) -> [String] {
        let s = c.snapshot
        func opt(_ v: String?) -> String { v.map { harnessEscape($0) } ?? "-" }
        func b(_ v: Bool) -> Int { v ? 1 : 0 }
        var lines = ["attached=\(b(s.isAttached)) ever=\(b(s.everAttached)) session=\(opt(s.activeSessionId)) window=\(opt(s.activeWindowId)) pane=\(opt(s.activePaneId)) fwd=\(b(s.lastSwitchForward)) swiped=\(b(s.hasSwipedSinceAttach))"]
        for session in s.sortedSessions {
            lines.append("s \(session.id) att=\(b(session.isAttached)) name=\(harnessEscape(session.name))")
        }
        for id in s.windows.keys.sorted(by: TmuxSnapshot.idPrecedes) {
            let w = s.windows[id]!
            lines.append("w \(w.id) s=\(opt(w.sessionId)) idx=\(w.index) act=\(b(w.isActive)) panes=\(w.paneCount) zoom=\(b(w.isZoomed)) name=\(harnessEscape(w.name)) layout=\(harnessEscape(w.layout))")
        }
        for id in s.panes.keys.sorted(by: TmuxSnapshot.idPrecedes) {
            let p = s.panes[id]!
            lines.append("p \(p.id) w=\(opt(p.windowId)) idx=\(p.index) act=\(b(p.isActive)) \(p.width)x\(p.height) cmd=\(harnessEscape(p.command))")
        }
        let follow = c.sizeFollow.map { "\($0.windowId)/\(harnessEscape($0.ownerLabel))" } ?? "-"
        let resized = c.resizedWindows.sorted(by: TmuxSnapshot.idPrecedes).joined(separator: ",")
        let renderer = c.rendererClient.map { "\($0.pid)/\(harnessEscape($0.name))" } ?? "-"
        let owned = c.ownedSessionIds.sorted(by: TmuxSnapshot.idPrecedes).joined(separator: ",")
        lines.append("follow=\(follow) notice=\(opt(c.notice)) refreshing=\(b(c.isRefreshing)) resized=\(resized) mouse=\(b(c.activePaneWantsMouse)) alt=\(b(c.activePaneOnAltScreen)) client=\(opt(c.ourClientName)) hooks-loaded=\(b(c.agentHooksLoaded)) painted=\(b(c.hasPaintedSinceAttach)) renderer=\(renderer) renderer-session=\(opt(c.rendererSessionId)) owned=\(owned)")
        for pane in c.agentHooks.keys.sorted(by: TmuxSnapshot.idPrecedes) {
            let h = c.agentHooks[pane]!
            let since = h.since.map { String(format: "%.3f", $0.timeIntervalSince1970) } ?? "-"
            lines.append("h \(harnessEscape(pane)) state=\(opt(h.state)) agent=\(opt(h.agent)) since=\(since) cmd=\(opt(h.command)) title=\(opt(h.title))")
        }
        return lines
    }
}

@main
struct Main {
    @MainActor
    static func main() async {
        let args = CommandLine.arguments
        guard args.count >= 3, args[1] == "run",
              let script = try? String(contentsOfFile: args[2], encoding: .utf8) else {
            FileHandle.standardError.write(Data("usage: controller-swift run <script> [<out>]\n".utf8))
            exit(2)
        }
        let runner = await Runner()
        for (index, raw) in script.components(separatedBy: "\n").enumerated() {
            let line = raw.hasSuffix("\r") ? String(raw.dropLast()) : raw
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty || trimmed.hasPrefix("#") { continue }
            await runner.step(index, line)
        }
        if args.count >= 4 {
            try? runner.out.write(toFile: args[3], atomically: true, encoding: .utf8)
        } else {
            FileHandle.standardOutput.write(Data(runner.out.utf8))
        }
        exit(0)
    }
}
