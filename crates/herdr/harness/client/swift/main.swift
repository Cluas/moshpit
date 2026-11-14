// Harness-only command-line front end for the app's Swift herdr control
// client (HerdrControlClient.swift, built by harness/client/build-swift.sh).
// Runs one scenario of crates/herdr/src/client/scenario.rs's format against
// the fake host (examples/herdr_host.rs, a co-process: the same host the Rust
// client runs against in-process) and prints the log the Rust runner
// (examples/client_harness.rs) prints.
//
//   client-swift <herdr_host binary> <script>
//
// Each step runs to quiescence before the next: every task the step started
// has had its turn and every command it ran has reached the host.

import Foundation

// MARK: - The host

final class Host: @unchecked Sendable {
    static let shared = Host()
    private var process: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var buffer = Data()
    private(set) var requests = 0

    func start(binary: String, script: String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = [script]
        let stdin = Pipe(), stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        try! process.run()
        self.process = process
        input = stdin.fileHandleForWriting
        output = stdout.fileHandleForReading
    }

    private func readLine() -> String {
        while true {
            if let newline = buffer.firstIndex(of: 0x0A) {
                let line = String(decoding: buffer[buffer.startIndex..<newline], as: UTF8.self)
                buffer.removeSubrange(buffer.startIndex...newline)
                return line
            }
            let chunk = output!.availableData
            if chunk.isEmpty { return "" }
            buffer.append(chunk)
        }
    }

    func request(_ line: String) -> String {
        requests += 1
        input!.write(Data((line + "\n").utf8))
        return readLine()
    }

    func lines(_ line: String) -> [String] {
        requests += 1
        input!.write(Data((line + "\n").utf8))
        var lines: [String] = []
        while true {
            let next = readLine()
            if next == "end" || next.isEmpty { return lines }
            lines.append(next)
        }
    }

    func stop() {
        try? input?.close()
        process?.waitUntilExit()
    }
}

func hex(_ text: String) -> String {
    Data(text.utf8).map { String(format: "%02x", $0) }.joined()
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

/// A command without the PATH prefix and the trailer (the Rust rig's `short`).
func short(_ command: String) -> String {
    var text = Substring(command)
    if let start = text.range(of: "\" herdr ") { text = text[start.upperBound...] }
    if let end = text.range(of: "; echo \"__moshpit_rc=$?\"") { text = text[..<end.lowerBound] }
    return String(text)
}

enum HarnessError: Error { case unreachable }

/// The client's runner: every command goes to the host, which says when it
/// answers; the answer comes at that virtual time.
final class HarnessRunner: HerdrCommandRunner, @unchecked Sendable {
    func run(_ command: String) async throws -> String {
        let plan: [String] = await MainActor.run {
            HarnessLog.item("exec \(short(command))")
            return Host.shared.request("exec \(HarnessClock.shared.elapsed) \(hex(command))")
                .split(separator: " ").map(String.init)
        }
        guard plan.count >= 3 else { throw HarnessError.unreachable }
        let id = plan[1]
        switch plan[2] {
        case "fail":
            throw HarnessError.unreachable
        case "never":
            try await HarnessClock.sleep(for: .seconds(1_000_000_000))
            throw HarnessError.unreachable
        default:
            let ms = Int(plan.count > 3 ? plan[3] : "0") ?? 0
            try await HarnessClock.sleep(for: .milliseconds(ms))
            let reply: String = await MainActor.run {
                Host.shared.request("land \(HarnessClock.shared.elapsed) \(id)")
            }
            return String(decoding: unhex(String(reply.dropFirst("output ".count))), as: UTF8.self)
        }
    }
}

// MARK: - Outcomes as the Rust runner prints them

func taskText(_ outcome: AgentTaskOutcome) -> String {
    switch outcome {
    case .started: return "started"
    case .failed(let text): return "failed \(text)"
    case .incomplete(let text): return "incomplete \(text)"
    }
}

func removalText(_ outcome: HerdrControlClient.WorktreeRemoval) -> String {
    switch outcome {
    case .removed: return "removed"
    case .needsForce: return "needs-force"
    case .failed(let text): return "failed \(text)"
    }
}

func restartText(_ outcome: HerdrControlClient.RestartOutcome) -> String {
    switch outcome {
    case .restarted: return "restarted"
    case .failed: return "failed"
    case .wouldDowngrade(let report): return "would-downgrade \(report.client) \(report.server)"
    case .unneeded: return "unneeded"
    }
}

// MARK: - The runner

@MainActor
final class Runner {
    let client = HerdrControlClient(runner: HarnessRunner(), customPath: nil)
    var lastTask: Task<Void, Never>?

    init() {
        client.onFocusedPaneChanged = { HarnessLog.item("focus \($0)") }
    }

    func perform(_ words: [String]) {
        func word(_ index: Int) -> String { index < words.count ? words[index] : "" }
        func name(_ index: Int) -> String? { let value = word(index); return value == "-" || value.isEmpty ? nil : value }
        let client = self.client
        switch word(0) {
        case "start": client.start()
        case "select-pane": client.selectPane(word(1))
        case "select-session": client.selectSession(word(1))
        case "select-window": client.selectWindow(word(1))
        case "new-session": client.newSession(named: name(1))
        case "new-window": client.newWindow(named: name(1))
        case "new-pane": client.newPane()
        case "new-window-in": client.newWindow(inSession: word(1), named: name(2))
        case "new-pane-in": client.newPane(inWindow: word(1))
        case "rename-session": client.renameSession(word(1), to: word(2))
        case "kill-session": client.killSession(word(1))
        case "rename-window": client.renameWindow(word(1), to: word(2))
        case "kill-window": client.killWindow(word(1))
        case "kill-pane": client.killPane(word(1))
        case "refresh": client.refresh()
        case "task":
            let prompt = words.count > 4 ? words[4...].joined(separator: " ") : ""
            let request = AgentTaskRequest(repoPath: word(1), branch: word(2), agent: word(3),
                                           prompt: prompt == "-" ? "" : prompt)
            lastTask = Task { @MainActor in
                let outcome = await client.startAgentTask(request)
                HarnessLog.item("outcome task \(taskText(outcome))")
            }
        case "cancel-task": lastTask?.cancel()
        case "remove":
            let session = word(1), force = word(2) == "1"
            Task { @MainActor in
                let outcome = await client.removeWorktree(sessionId: session, force: force)
                HarnessLog.item("outcome removal \(removalText(outcome))")
            }
        case "repos":
            Task { @MainActor in
                let repos = await client.gitRepos()
                HarnessLog.item("outcome repos [\(repos.joined(separator: " "))]")
            }
        case "agents":
            Task { @MainActor in
                let names = await client.agentNames()
                HarnessLog.item("outcome agents [\(names.joined(separator: " "))]")
            }
        case "restart":
            Task { @MainActor in
                let outcome = await client.restartServer()
                HarnessLog.item("outcome restart \(restartText(outcome))")
            }
        case "confirm-focus":
            let budget = Int(word(1)) ?? 0
            Task { @MainActor in
                let pane = await client.confirmedFocus(within: .milliseconds(budget))
                HarnessLog.item("outcome confirm \(pane ?? "nil")")
            }
        case "host-focus": _ = Host.shared.request("set focus \(word(1))")
        case "host-running": _ = Host.shared.request("set running \(word(1))")
        case "host-mismatch": _ = Host.shared.request("set mismatch \(words.dropFirst().joined(separator: " "))")
        case "host-rule": _ = Host.shared.request("set rule \(words.dropFirst().joined(separator: " "))")
        case "host-clear": _ = Host.shared.request("set clear")
        default: HarnessLog.item("error unknown action \(word(0))")
        }
    }

    func signature() -> [Int] {
        [Int(HarnessClock.shared.registrations), HarnessLog.items.count, Host.shared.requests]
    }

    func quiesce() async {
        var stable = 0
        var last = signature()
        while stable < 8 {
            for _ in 0..<4 { await Task.yield() }
            let now = signature()
            if now == last { stable += 1 } else { stable = 0; last = now }
        }
    }

    func wait(until elapsed: UInt64) async {
        let target = HarnessClock.startMs + elapsed
        while let continuation = HarnessClock.shared.takeDue(by: target) {
            continuation.resume()
            await quiesce()
        }
        HarnessClock.shared.standAt(target)
        await quiesce()
    }
}

// MARK: - The script

struct Script {
    var steps: [(UInt64, [String])] = []
    var end: UInt64 = 0
}

func parse(_ text: String) -> Script {
    var script = Script()
    for raw in text.components(separatedBy: "\n") {
        let line = (raw.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? "")
            .trimmingCharacters(in: .whitespaces)
        if line.isEmpty { continue }
        let words = line.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
        switch words[0] {
        case "at": script.steps.append((UInt64(words[1]) ?? 0, Array(words.dropFirst(2))))
        case "end": script.end = UInt64(words[1]) ?? 0
        default: break
        }
    }
    script.steps = script.steps.enumerated()
        .sorted { ($0.element.0, $0.offset) < ($1.element.0, $1.offset) }
        .map(\.element)
    return script
}

@main
struct Main {
    @MainActor
    static func main() async {
        let args = CommandLine.arguments
        guard args.count >= 3 else {
            FileHandle.standardError.write(Data("usage: client-swift <herdr_host> <script>\n".utf8))
            exit(2)
        }
        let text = (try? String(contentsOfFile: args[2], encoding: .utf8)) ?? ""
        let script = parse(text)
        Host.shared.start(binary: args[1], script: args[2])
        let runner = Runner()
        for (at, words) in script.steps {
            await runner.wait(until: at)
            runner.perform(words)
            await runner.quiesce()
        }
        await runner.wait(until: script.end)
        var out = HarnessLog.items
        out += Host.shared.lines("landed")
        let server = String(Host.shared.request("focused").dropFirst("focused ".count))
        let snapshot = runner.client.snapshot
        out.append("state focus=\(snapshot.activePaneId ?? "nil") server=\(server) sessions=\(snapshot.sessions.count) notrunning=\(runner.client.serverNotRunning) mismatch=\(runner.client.protocolMismatch != nil)")
        print(out.joined(separator: "\n"))
        Host.shared.stop()
        exit(0)
    }
}
