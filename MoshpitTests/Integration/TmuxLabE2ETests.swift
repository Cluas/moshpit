import Foundation
import Testing
import SwiftTerm
@testable import Moshpit

/// Full-stack e2e against a REAL tmux server: the production
/// `TmuxSessionController` + SwiftTerm stack drives a real `tmux -CC attach`
/// through a TCP bridge, while a scripted "desktop enemy" client fights it
/// for `window-size latest` — the exact adversary behind the SSH+tmux scroll
/// garble (2026-08-19). The invariant under test is the one no unit test can
/// claim: after any war, the local grid CONVERGES to tmux's screen.
///
/// Run via scripts/tmux-cc-lab/run-e2e.sh, which starts the bridge
/// (`lab.py serve`) and passes the ports in through
/// TEST_RUNNER_MOSHPIT_TMUX_LAB_PORT / _CTL_PORT.
@Suite("TmuxLab e2e (real tmux over TCP bridge)", .tags(.integration), .serialized)
@MainActor
struct TmuxLabE2ETests {
    nonisolated static let dataPort = ProcessInfo.processInfo
        .environment["MOSHPIT_TMUX_LAB_PORT"].flatMap(UInt16.init)
    nonisolated static let ctlPort = ProcessInfo.processInfo
        .environment["MOSHPIT_TMUX_LAB_CTL_PORT"].flatMap(UInt16.init)

    nonisolated static let isConfigured: Bool = {
        let configured = dataPort != nil && ctlPort != nil
        if !configured {
            print("[TmuxLabE2E] SKIPPED: run scripts/tmux-cc-lab/run-e2e.sh "
                + "(sets MOSHPIT_TMUX_LAB_PORT / _CTL_PORT).")
        }
        return configured
    }()

    private func waitUntil(_ timeout: TimeInterval,
                           _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return predicate()
    }

    /// Text of a VISIBLE row — what the glass shows. `getText` addresses the
    /// whole buffer (scrollback included) and the session's early feeds
    /// (backfill seeding, pre-resize frames) push the first screens up into
    /// scrollback, so buffer row 0 is history, not the screen. `yDisp` is the
    /// buffer row at the top of the viewport.
    private func rowText(_ terminal: Terminal, _ row: Int) -> String {
        let top = terminal.buffer.yDisp
        return terminal.getText(start: Position(col: 0, row: top + row),
                                end: Position(col: terminal.cols, row: top + row))
    }

    @Test("a size-war scroll session converges: the local grid ends equal to tmux's screen",
          .enabled(if: isConfigured), .timeLimit(.minutes(2)))
    func sizeWarConverges() async throws {
        let transport = try #require(LabSocketTransport(port: Self.dataPort!),
                                     "cannot reach the lab data bridge")
        let ctl = try #require(LabControl(port: Self.ctlPort!),
                               "cannot reach the lab control channel")

        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 70, rows: 20)
        await controller.attach()
        #expect(await waitUntil(8.0) { controller.snapshot.activePaneId != nil },
                "discovery against real tmux must resolve a pane")
        let paneId = try #require(controller.snapshot.activePaneId)
        let terminal = controller.terminalView(for: paneId).getTerminal()
        // Production terminals are laid out by UIKit to the client grid; a
        // headless view keeps its default (smaller) grid, on which 70-column
        // 20-row frames would wrap and spill.
        terminal.resize(cols: 70, rows: 20)

        // The fresh-attach resync paints the fake TUI's frame.
        #expect(await waitUntil(8.0) { self.rowText(terminal, 0).contains("L0") },
                "the attach resync must paint the TUI's first frame")

        controller.refreshActivePaneMouse()
        _ = await waitUntil(3.0) { controller.activePaneWantsMouse }

        // Three war rounds: scroll (wheel), background (pin handed back),
        // desktop activity wins `window-size latest` and the TUI repaints at
        // 180 columns, foreground re-pins, scroll again.
        for _ in 0..<3 {
            controller.scroll(lines: -3)
            try? await Task.sleep(for: .milliseconds(300))
            controller.releaseWindowPins()
            _ = ctl.request(["cmd": "desktop-keys", "keys": "xxx"])
            try? await Task.sleep(for: .milliseconds(450))
            controller.repinActiveWindow()
            try? await Task.sleep(for: .milliseconds(300))
            controller.scroll(lines: -3)
            try? await Task.sleep(for: .milliseconds(450))
        }
        // Let the settling resyncs land.
        try? await Task.sleep(for: .seconds(2))

        // Diagnostics: identity, buffer mode, and the visible grid.
        print("[e2e] rows=\(terminal.rows) cols=\(terminal.cols) "
            + "alt=\(terminal.isCurrentBufferAlternate) cursor=\(terminal.getCursorLocation()) "
            + "yDisp=\(terminal.buffer.yDisp)")
        for r in 0..<terminal.rows {
            let t = rowText(terminal, r)
            if !t.trimmingCharacters(in: .whitespaces).isEmpty {
                print("[e2e] row\(r)=\(t.prefix(46))")
            }
        }

        // Ground truth: tmux's own view of the pane, plain text.
        let reply = ctl.request(["cmd": "capture"])
        let b64 = (reply?["b64"] as? String) ?? ""
        let truth = String(data: Data(base64Encoded: b64) ?? Data(),
                           encoding: .utf8) ?? ""
        var truthLines = truth.components(separatedBy: "\n").map {
            $0.replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression)
        }
        while let last = truthLines.last, last.isEmpty { truthLines.removeLast() }
        try #require(!truthLines.isEmpty, "empty ground-truth capture")

        var mismatches: [String] = []
        for (row, expected) in truthLines.enumerated() where row < terminal.rows {
            let got = rowText(terminal, row)
                .replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression)
            if got != expected {
                mismatches.append("row \(row):\n  grid: \(got)\n  tmux: \(expected)")
            }
        }
        #expect(mismatches.isEmpty, """
                the local grid diverged from tmux's screen after the size war \
                (\(mismatches.count) rows):\n\(mismatches.prefix(8).joined(separator: "\n"))
                """)
        transport.shutdown()
    }
}

// MARK: - TCP plumbing

/// `TmuxTransport` over a plain TCP socket to the lab bridge — stands in for
/// the SSH channel with byte-for-byte fidelity (the bridge pipes a real
/// `tmux -CC attach` pty).
final class LabSocketTransport: TmuxTransport, @unchecked Sendable {
    nonisolated let dataStream: AsyncStream<Data>
    private let fd: Int32

    init?(port: UInt16) {
        guard let fd = Self.connectLoopback(port: port) else { return nil }
        self.fd = fd
        var captured: AsyncStream<Data>.Continuation!
        dataStream = AsyncStream(bufferingPolicy: .unbounded) { captured = $0 }
        let continuation = captured!
        let readFD = fd
        Thread.detachNewThread {
            var buf = [UInt8](repeating: 0, count: 65536)
            while true {
                let n = read(readFD, &buf, buf.count)
                if n <= 0 { break }
                continuation.yield(Data(buf[0..<n]))
            }
            continuation.finish()
        }
    }

    func write(_ data: Data) async throws {
        var remaining = data
        while !remaining.isEmpty {
            let n = remaining.withUnsafeBytes { raw in
                Darwin.write(fd, raw.baseAddress, raw.count)
            }
            guard n > 0 else { throw POSIXError(.EIO) }
            remaining = remaining.dropFirst(n)
        }
    }

    func shutdown() { close(fd) }

    static func connectLoopback(port: UInt16) -> Int32? {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
        let rc = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { sp in
                Darwin.connect(fd, sp, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard rc == 0 else { close(fd); return nil }
        return fd
    }
}

/// Line-JSON control channel to the lab (`desktop-keys`, `capture`, …).
final class LabControl: @unchecked Sendable {
    private let fd: Int32

    init?(port: UInt16) {
        guard let fd = LabSocketTransport.connectLoopback(port: port) else { return nil }
        self.fd = fd
    }

    func request(_ obj: [String: Any]) -> [String: Any]? {
        guard let payload = try? JSONSerialization.data(withJSONObject: obj) else { return nil }
        var out = payload
        out.append(0x0a)
        _ = out.withUnsafeBytes { raw in Darwin.write(fd, raw.baseAddress, raw.count) }
        var line = Data()
        var byte: UInt8 = 0
        while read(fd, &byte, 1) == 1 {
            if byte == 0x0a { break }
            line.append(byte)
        }
        return (try? JSONSerialization.jsonObject(with: line)) as? [String: Any]
    }
}
