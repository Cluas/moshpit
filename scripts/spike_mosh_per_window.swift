import Foundation

// Route-B spike: TWO real mosh connections, one per tmux window, using the
// app's own SSP client (MoshTransport + MoshWire + OCB3) compiled into a
// macOS CLI. The mechanism under test is the grouped-session trick
// (`tmux new-session -t base \; select-window -t :N` behind each
// mosh-server) — the substrate a "native tab per tmux window over mosh"
// mode would stand on.
//
// PASS requires, per transport:
//   1. the initial screen sync arrives (host bytes at all);
//   2. a typed command round-trips into ITS OWN window
//      (`echo`'s output comes back on the same screen);
//   3. the sibling's marker never appears (grouped sessions really do keep
//      independent current windows — the whole point).
//
// Driven by scripts/spike-mosh-per-window.sh, which owns the server side.

/// Accumulates one transport's host bytes as best-effort UTF-8.
actor ScreenLog {
    private(set) var text = ""
    func append(_ data: Data) { text += String(decoding: data, as: UTF8.self) }
}

@main
struct SpikeMoshPerWindow {
    static func main() async {
        let args = CommandLine.arguments
        guard args.count == 6 else {
            FileHandle.standardError.write(Data(
                "usage: spike-mosh-per-window <host> <portA> <keyA> <portB> <keyB>\n".utf8))
            exit(2)
        }
        let host = args[1]
        guard let portA = UInt16(args[2]), let portB = UInt16(args[4]),
              let keyA = decodeKey(args[3]), let keyB = decodeKey(args[5]) else {
            FileHandle.standardError.write(Data("bad port/key arguments\n".utf8))
            exit(2)
        }

        async let a = exercise(label: "A", host: host, port: portA, key: keyA)
        async let b = exercise(label: "B", host: host, port: portB, key: keyB)
        let (resultA, resultB) = await (a, b)

        var failures: [String] = []
        if !resultA.sawOwnMarker { failures.append("A never saw MARK-A-OK (input or output path broken)") }
        if !resultB.sawOwnMarker { failures.append("B never saw MARK-B-OK (input or output path broken)") }
        if resultA.screen.contains("MARK-B-OK") { failures.append("A's screen shows B's marker — windows are NOT independent") }
        if resultB.screen.contains("MARK-A-OK") { failures.append("B's screen shows A's marker — windows are NOT independent") }

        print("--- A diagnostics: \(resultA.diagnostics)")
        print("--- B diagnostics: \(resultB.diagnostics)")
        if failures.isEmpty {
            print("SPIKE PASS: two concurrent MoshTransports, one tmux window each, fully independent")
            exit(0)
        }
        for f in failures { print("SPIKE FAIL: \(f)") }
        print("--- A screen tail ---\n\(resultA.screen.suffix(600))")
        print("--- B screen tail ---\n\(resultB.screen.suffix(600))")
        exit(1)
    }

    struct Result {
        let sawOwnMarker: Bool
        let screen: String
        let diagnostics: String
    }

    /// One transport's full lifecycle: connect, wait for the first screen
    /// sync, type a marker command, wait for its output, report.
    static func exercise(label: String, host: String, port: UInt16, key: Data) async -> Result {
        let log = ScreenLog()
        let credentials = MoshCredentials(host: host, udpPort: port, key: key)
        guard let transport = try? MoshTransport(credentials: credentials) else {
            return Result(sawOwnMarker: false, screen: "", diagnostics: "init failed")
        }
        let pump = Task {
            for await data in transport.hostStream { await log.append(data) }
        }
        await transport.start(cols: 80, rows: 24)

        // Wait for the initial sync (shell prompt inside the tmux pane).
        _ = await waitUntil(deadline: 10) { await !log.text.isEmpty }

        // The marker is assembled at the far end so the TYPED ECHO of the
        // command (which also lands on this screen) can't satisfy — or
        // cross-contaminate — the assertion by itself.
        await transport.send(Data("echo MARK-\(label)-\"OK\"\r".utf8))
        let sawOwn = await waitUntil(deadline: 10) { await log.text.contains("MARK-\(label)-OK") }

        // Give any cross-window leakage a moment to show before sampling.
        try? await Task.sleep(nanoseconds: 2_000_000_000)
        let diagnostics = await transport.currentDiagnostics
        await transport.close()
        pump.cancel()
        let screen = await log.text
        return Result(
            sawOwnMarker: sawOwn,
            screen: screen,
            diagnostics: "datagrams=\(diagnostics.datagramsReceived) applied=\(diagnostics.appliedHostNum) "
                + "gaps=\(diagnostics.gapEvents) parseFailures=\(diagnostics.parseFailures)")
    }

    static func waitUntil(deadline seconds: Double, _ check: () async -> Bool) async -> Bool {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            if await check() { return true }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        return await check()
    }

    /// mosh prints the key as 22 unpadded base64 chars = 16 bytes.
    static func decodeKey(_ token: String) -> Data? {
        var b64 = token
        while b64.count % 4 != 0 { b64.append("=") }
        guard let key = Data(base64Encoded: b64), key.count == 16 else { return nil }
        return key
    }
}
