import Foundation

// Capture rig for the "white cursor-sized blocks" render-divergence bug.
//
// The app renders mosh's screen-diff bytes STRAIGHT into SwiftTerm with no
// local framebuffer. If SwiftTerm's cell state ever diverges from
// mosh-server's own emulator model, the divergence is permanent: the server
// only ever sends the cells it believes changed. The reported trigger is a
// tmux PANE SWITCH (`select-pane -Z`, issued out-of-band by the app's -CC
// sidecar), whose zoom relayout makes mosh emit a big incremental repaint.
//
// This CLI drives the CLIENT half of that with the app's REAL SSP client
// (MoshTransport, compiled in by capture-mosh-switch-bytes.sh):
//
//   phase 0  connect + initial sync   → phase0.bin
//   phase N  run switch script N      → phaseN.bin   (the repaint burst)
//
// Every phase ends when the host stream has been quiet for `quiet` seconds,
// so the byte split is deterministic rather than time-boxed. The switch
// scripts are plain /bin/sh — they own the tmux side (select-pane -Z) and
// also snapshot tmux's own idea of the screen (`capture-pane -p -e`) as the
// ground truth to diff SwiftTerm against.
//
// usage: capture-mosh-switch-bytes <host> <port> <key> <cols> <rows> <outDir> [switchScript...]

/// One transport's host bytes, with the timestamp of the last arrival so a
/// phase can end on quiescence instead of a fixed sleep.
actor ByteLog {
    private var pending = Data()
    private var lastAppend = Date()
    private var everReceived = false
    private(set) var totalBytes = 0

    func append(_ data: Data) {
        pending.append(data)
        lastAppend = Date()
        everReceived = true
        totalBytes += data.count
    }

    /// Take everything accumulated so far, leaving the quiescence clock alone.
    func drain() -> Data {
        let out = pending
        pending = Data()
        return out
    }

    func quietSeconds() -> Double { Date().timeIntervalSince(lastAppend) }
    func sawAnything() -> Bool { everReceived }
}

@main
struct CaptureMoshSwitchBytes {

    static func main() async {
        let args = CommandLine.arguments
        guard args.count >= 7 else {
            fail("usage: capture-mosh-switch-bytes <host> <port> <key> <cols> <rows> <outDir> [switchScript...]")
        }
        let host = args[1]
        guard let port = UInt16(args[2]), let key = decodeKey(args[3]),
              let cols = Int(args[4]), let rows = Int(args[5]) else {
            fail("bad port/key/cols/rows")
        }
        let outDir = URL(fileURLWithPath: args[6], isDirectory: true)
        let switchScripts = Array(args.dropFirst(7))
        try? FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

        let log = ByteLog()
        let credentials = MoshCredentials(host: host, udpPort: port, key: key)
        guard let transport = try? MoshTransport(credentials: credentials) else {
            fail("MoshTransport init failed")
        }
        let pump = Task {
            for await data in transport.hostStream { await log.append(data) }
        }
        await transport.start(cols: cols, rows: rows)

        // Phase 0: the initial sync. Nothing is typed — the screen the user
        // lands on after connect, which the bug report says is CLEAN.
        var settled = await settle(log, quiet: 1.5, minWait: 1.0, maxWait: 25, needData: true)
        var phase = await log.drain()
        write(phase, to: outDir.appendingPathComponent("phase0.bin"))
        print("phase0: \(phase.count) bytes (settled=\(settled))")
        if phase.isEmpty { fail("no initial sync — mosh-server never painted") }

        // Phases 1..N: one out-of-band tmux switch each, exactly like the
        // app's sidecar issuing `select-pane -Z` over its separate -CC lane.
        for (index, script) in switchScripts.enumerated() {
            let n = index + 1
            runScript(script)
            settled = await settle(log, quiet: 1.5, minWait: 1.5, maxWait: 20, needData: false)
            phase = await log.drain()
            write(phase, to: outDir.appendingPathComponent("phase\(n).bin"))
            print("phase\(n): \(phase.count) bytes (settled=\(settled)) via \(URL(fileURLWithPath: script).lastPathComponent)")
        }

        // Final phase: the app's own self-heal (the diagnostics popover's
        // "repaint" button) — ack display state 0 so the server re-diffs from
        // a BLANK baseline and paints its whole framebuffer.
        //
        // This is the reference the tmux ground truth can't be: `capture-pane`
        // trims trailing SPACES from every line regardless of their
        // attributes, so an inverse-video space parked at the end of a row —
        // exactly the white block being hunted — is invisible in it. The full
        // repaint, fed into a FRESH terminal, is mosh-server's own framebuffer
        // rendered from scratch, attributes and all. Incremental screen ≠ full
        // repaint IS the bug, and it is the same comparison the user makes
        // when they tap repaint and watch the blocks vanish.
        await transport.requestFullRedraw()
        settled = await settle(log, quiet: 1.5, minWait: 1.5, maxWait: 20, needData: false)
        phase = await log.drain()
        write(phase, to: outDir.appendingPathComponent("redraw.bin"))
        print("redraw: \(phase.count) bytes (settled=\(settled))")

        let diagnostics = await transport.currentDiagnostics
        print("diagnostics: datagrams=\(diagnostics.datagramsReceived) applied=\(diagnostics.appliedHostNum) "
            + "gaps=\(diagnostics.gapEvents) parseFailures=\(diagnostics.parseFailures) total=\(await log.totalBytes)")
        await transport.close()
        pump.cancel()
        exit(0)
    }

    /// Wait for the host stream to go quiet for `quiet` seconds.
    static func settle(_ log: ByteLog, quiet: Double, minWait: Double,
                       maxWait: Double, needData: Bool) async -> Bool {
        let start = Date()
        let end = start.addingTimeInterval(maxWait)
        while Date() < end {
            try? await Task.sleep(nanoseconds: 100_000_000)
            if Date().timeIntervalSince(start) < minWait { continue }
            if needData, !(await log.sawAnything()) { continue }
            if await log.quietSeconds() >= quiet { return true }
        }
        return false
    }

    static func runScript(_ path: String) {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/bin/sh")
        proc.arguments = [path]
        do {
            try proc.run()
            proc.waitUntilExit()
            if proc.terminationStatus != 0 {
                FileHandle.standardError.write(Data("switch script \(path) exited \(proc.terminationStatus)\n".utf8))
            }
        } catch {
            FileHandle.standardError.write(Data("switch script \(path) failed: \(error)\n".utf8))
        }
    }

    static func write(_ data: Data, to url: URL) {
        do { try data.write(to: url) } catch { fail("write \(url.path): \(error)") }
    }

    /// mosh prints the key as 22 unpadded base64 chars = 16 bytes.
    static func decodeKey(_ token: String) -> Data? {
        var b64 = token
        while b64.count % 4 != 0 { b64.append("=") }
        guard let key = Data(base64Encoded: b64), key.count == 16 else { return nil }
        return key
    }

    static func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FATAL: \(message)\n".utf8))
        exit(2)
    }
}
