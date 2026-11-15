import Foundation
import MoshpitFFI
import QuartzCore
import Testing
import UIKit
@testable import Moshpit

/// Gate T1's device measurements that need no live session
/// (docs/nsh/TERMINAL-CORE.md §8), the core view on the device the tests
/// run on (the SwiftTerm baseline went with SwiftTerm, batch B). Results
/// print as `[t2-bench] …` lines; the asserts only guard what the gate
/// calls correctness (nothing lost, nothing reordered).
@Suite("core terminal bench", .serialized)
@MainActor
struct CoreTerminalBenchTests {
    static func say(_ line: String) {
        print("[t2-bench] \(line)")
        NSLog("[t2-bench] %@", line)
    }

    private static let font = UIFont.monospacedSystemFont(ofSize: 9, weight: .regular)

    /// Sink for what a view sends.
    final class Loop: CoreTerminalViewDelegate {
        var onSend: ((Data) -> Void)?
        func surface(_ surface: CoreTerminalView, send data: Data) { onSend?(data) }
        func surface(_ surface: CoreTerminalView, sizeChanged cols: Int, rows: Int) {}
        func surface(_ surface: CoreTerminalView, titleChanged title: String) {}
        func surfaceBell(_ surface: CoreTerminalView) {}
        func surface(_ surface: CoreTerminalView, openLink link: String) {}
        func surface(_ surface: CoreTerminalView, paste text: String) {}
        func surfaceClipboardRequest(_ surface: CoreTerminalView) {}
        func surfaceScrollWillBegin(_ surface: CoreTerminalView) {}
        func surface(_ surface: CoreTerminalView, scrollPixels dy: CGFloat) -> CGFloat { surface.scrollLocal(pixels: dy) }
        func surfaceScrollDidEnd(_ surface: CoreTerminalView) {}
        func surface(_ surface: CoreTerminalView, click col: Int, row: Int) {}
        var surfaceScrollsElsewhere: Bool { false }
        var surfaceCanSwitch: Bool { false }
        func surface(_ surface: CoreTerminalView, switchForward forward: Bool) {}
        func surface(_ surface: CoreTerminalView, fontSizeCommitted size: Double) {}
    }

    // MARK: Workloads

    static func seq(_ n: Int) -> Data { Data((1...n).map { "\($0)\r\n" }.joined().utf8) }

    static func colours(_ lines: Int) -> Data {
        var s = ""
        for i in 0..<lines {
            for j in 0..<16 { s += "\u{1b}[38;5;\((i + j) % 256)m\u{1b}[48;2;\(j * 9);\(i % 255);40mwd\(j)" }
            s += "\u{1b}[0m\r\n"
        }
        return Data(s.utf8)
    }

    static func cjk(_ lines: Int) -> Data {
        var s = ""
        for i in 0..<lines { s += "第\(i)行 中文测试 日本語 한국어 😀👨‍👩‍👧 é ✓ ⚠️ 終わり\r\n" }
        return Data(s.utf8)
    }

    /// Claude Code's inline redraws: up, erase, rewrite a block of lines.
    static func agentFrames(_ frames: Int) -> Data {
        var s = ""
        for f in 0..<frames {
            s += "\u{1b}[?2026h\u{1b}[8A"
            for l in 0..<8 { s += "\u{1b}[2K\u{1b}[38;5;\(f % 200)m● thinking \(f) line \(l) — the agent is writing code\u{1b}[0m\r\n" }
            s += "\u{1b}[?2026l"
        }
        return Data(s.utf8)
    }

    static func footprint() -> Int {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return kr == KERN_SUCCESS ? Int(info.phys_footprint) : 0
    }

    private func window() -> UIWindow {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let w = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: UIScreen.main.bounds)
        w.frame = UIScreen.main.bounds
        w.rootViewController = UIViewController()
        w.isHidden = false
        return w
    }

    private func coreView(cols: Int, rows: Int, in window: UIWindow? = nil) -> (CoreTerminalView, Loop) {
        let cell = TerminalCellGeometry.measuredCell(font: Self.font, scale: UITraitCollection.current.displayScale)
        let v = CoreTerminalView(frame: CGRect(x: 0, y: 0, width: cell.width * CGFloat(cols), height: cell.height * CGFloat(rows)),
                                 font: Self.font)
        let loop = Loop()
        v.surfaceDelegate = loop
        var s = v.style
        s.cursorBlink = false
        v.style = s
        window?.rootViewController?.view.addSubview(v)
        return (v, loop)
    }

    // MARK: T1d: parse throughput on the device

    @Test func parseThroughput() {
        let loads: [(String, Data)] = [("seq 200000", Self.seq(200_000)), ("colours", Self.colours(4_000)),
                                       ("cjk+emoji", Self.cjk(20_000)), ("agent frames", Self.agentFrames(6_000))]
        var total = 0, coreTime = 0.0
        for (name, data) in loads {
            // The core, as its pane thread feeds it (4 KB reads), best of 3.
            var best = Double.infinity
            for _ in 0..<3 {
                guard let t = mp_term_new(80, 24, 10_000) else { continue }
                let start = CACurrentMediaTime()
                data.withUnsafeBytes { raw in
                    var at = 0
                    let base = raw.baseAddress!.assumingMemoryBound(to: UInt8.self)
                    while at < raw.count {
                        let n = min(4096, raw.count - at)
                        mp_term_feed(t, base + at, n)
                        at += n
                    }
                }
                best = min(best, CACurrentMediaTime() - start)
                mp_term_close(t)
            }
            total += data.count
            coreTime += best
            Self.say(String(format: "parse %@ %.1f MB: core %.1f MB/s", name,
                            Double(data.count) / 1e6, Double(data.count) / 1e6 / best))
        }
        Self.say(String(format: "parse all: core %.1f MB/s (gate T1d >= 10)",
                        Double(total) / 1e6 / coreTime))
    }

    // MARK: T1g: memory per pane

    @Test func memoryPerPane() {
        let lines = Data((0..<10_000).map { "\($0) " + String(repeating: "x", count: 70) + "\r\n" }.joined().utf8)
        var views: [CoreTerminalView] = []
        let before = Self.footprint()
        for _ in 0..<4 {
            let (v, _) = coreView(cols: 80, rows: 24)
            v.push(lines)
            _ = v.settle(timeout: 10) { $0.historyLines >= 9_900 }
            views.append(v)
        }
        let core = Double(Self.footprint() - before) / 4 / 1e6
        views.removeAll()
        Self.say(String(format: "memory per pane, 10 000 lines × 80: core %.1f MB (gate T1g <= 8)", core))
    }

    // MARK: T1c: a 50 MB burst through the view

    /// Pushed from the main thread in the reads the byte channel gives
    /// (32 KB), as the app's feeder does; the main thread's longest stall is
    /// the longest gap between display-link ticks while it runs. Every line
    /// arrives, in order.
    @Test func burstKeepsTheMainThreadFree() async {
        let w = window()
        defer { w.isHidden = true }
        let (v, _) = coreView(cols: 90, rows: 50, in: w)
        let lineCount = 600_000
        var payload = Data()
        payload.reserveCapacity(52_000_000)
        let tail = String(repeating: "abcdefgh", count: 10) + "\r\n"
        for i in 0..<lineCount { payload.append(contentsOf: ("\(i) " + tail).utf8) }
        let probe = TickProbe()
        probe.start()
        let start = CACurrentMediaTime()
        var at = 0
        while at < payload.count {
            let n = min(32 * 1024, payload.count - at)
            v.push(payload.subdata(in: at..<at + n))
            at += n
            // The feeder's 4 ms budget per turn, then the run loop.
            if at % (32 * 1024 * 16) == 0 { try? await Task.sleep(for: .milliseconds(1)) }
        }
        let pushed = CACurrentMediaTime() - start
        let last = "\(lineCount - 1) "
        var done = false
        for _ in 0..<2_000 where !done {
            try? await Task.sleep(for: .milliseconds(10))
            v.takeFrame()
            done = v.rowText(v.rows.gridRows - 2).hasPrefix(last)
        }
        let elapsed = CACurrentMediaTime() - start
        probe.stop()
        #expect(done, "the burst's last line never showed")
        // Order: the history's tail counts up by one.
        var ordered = true
        var previous = -1
        for r in 0..<(v.rows.gridRows - 1) {
            guard let n = Int(v.rowText(r).split(separator: " ").first ?? "") else { continue }
            if previous >= 0, n != previous + 1 { ordered = false }
            previous = n
        }
        #expect(ordered, "lines out of order or lost")
        Self.say(String(format: "burst %.1f MB: pushed in %.2f s, shown in %.2f s (%.1f MB/s); main thread longest stall %.1f ms (gate < 32), frames %d, longest frame %.2f ms (%@)",
                        Double(payload.count) / 1e6, pushed, elapsed, Double(payload.count) / 1e6 / elapsed,
                        probe.longestGap * 1000, v.framesTaken, v.longestFrameMs, v.frameCosts.summary))
    }

    /// T4: a backlog — bytes arriving faster than the core parses them, as
    /// when a long capture, a big `cat` or the output of a spell away lands
    /// at once. Pushed as the feeder pushes (a 4 ms budget a turn, the run
    /// loop in between), and nothing here takes a frame: what shows is what
    /// the core's wakes bring. The screen must keep moving (target ≥ 30
    /// frames a second) and every line arrive, in order.
    @Test func backlogKeepsTheScreenMoving() async {
        let w = window()
        defer { w.isHidden = true }
        let (v, _) = coreView(cols: 90, rows: 50, in: w)
        let lineCount = 600_000
        var payload = Data()
        payload.reserveCapacity(52_000_000)
        let tail = String(repeating: "abcdefgh", count: 10) + "\r\n"
        for i in 0..<lineCount { payload.append(contentsOf: ("\(i) " + tail).utf8) }
        let probe = TickProbe()
        // Building the payload is not the backlog's.
        try? await Task.sleep(for: .milliseconds(300))
        probe.start()
        var shown: [CFTimeInterval] = []
        var encoded = v.rowsEncoded
        v.onFrame = { [weak v] in
            guard let v, v.rowsEncoded != encoded else { return }
            encoded = v.rowsEncoded
            shown.append(CACurrentMediaTime())
        }
        let start = CACurrentMediaTime()
        var at = 0
        while at < payload.count {
            let turn = CACurrentMediaTime()
            while at < payload.count, CACurrentMediaTime() - turn < 0.004 {
                let n = min(32 * 1024, payload.count - at)
                v.push(payload.subdata(in: at..<at + n))
                at += n
            }
            await Task.yield()
        }
        let pushed = CACurrentMediaTime() - start
        let last = "\(lineCount - 1) "
        var done = false
        for _ in 0..<6_000 where !done {
            try? await Task.sleep(for: .milliseconds(5))
            done = (0..<v.rows.gridRows).contains { v.rowText($0).hasPrefix(last) }
        }
        let end = CACurrentMediaTime()
        probe.stop()
        v.onFrame = nil
        #expect(done, "the backlog's last line never showed")
        var ordered = true
        var previous = -1
        for r in 0..<(v.rows.gridRows - 1) {
            guard let n = Int(v.rowText(r).split(separator: " ").first ?? "") else { continue }
            if previous >= 0, n != previous + 1 { ordered = false }
            previous = n
        }
        #expect(ordered, "lines out of order or lost")
        let marks = [start] + shown + [end]
        let still = zip(marks.dropFirst(), marks).map { ($0 - $1) * 1000 }.max() ?? 0
        let fps = Double(shown.count) / (end - start)
        Self.say(String(format: "backlog %.1f MB: pushed in %.2f s, shown in %.2f s (%.1f MB/s); %d frames changed the screen (%.0f/s), longest still %.0f ms; main thread longest stall %.1f ms, longest frame %.2f ms (%@)",
                        Double(payload.count) / 1e6, pushed, end - start, Double(payload.count) / 1e6 / (end - start),
                        shown.count, fps, still, probe.longestGap * 1000, v.longestFrameMs, v.frameCosts.summary))
    }

    // MARK: T1c: echo, view to view

    /// A keystroke's echo, with the network taken out: the bytes a key sends
    /// come straight back as output; timed from the key to the frame that
    /// shows it drawn. Main thread per echo: what the view itself costs.
    @Test func loopbackEcho() async {
        let w = window()
        defer { w.isHidden = true }
        let (v, loop) = coreView(cols: 70, rows: 72, in: w)
        v.push(Data("$ ".utf8))
        _ = v.settle { $0.rowText(0) == "$" }
        loop.onSend = { [weak v] data in v?.push(data) }
        var samples: [Double] = []
        var mainCost: [Double] = []
        let letters = Array("abcdefghijklmnopqrstuvwxyz")
        for i in 0..<200 {
            if i > 0, i % 60 == 0 {
                v.push(Data("\r\n$ ".utf8))
                _ = v.settle { $0.rowText(Int($0.rows.info.cursor.row)) == "$" }
            }
            let ch = letters[i % letters.count]
            let col = Int(v.rows.info.cursor.col), row = Int(v.rows.info.cursor.row)
            var shownAt: CFTimeInterval = 0
            var cost = 0.0
            v.onFrame = { [weak v] in
                guard let v, shownAt == 0, row < v.rows.rows.count, col < v.rows.rows[row].cells.count,
                      v.rows.rows[row].cells[col].ch == ch.unicodeScalars.first!.value else { return }
                shownAt = CACurrentMediaTime()
            }
            let start = CACurrentMediaTime()
            v.sendText(String(ch))
            cost = (CACurrentMediaTime() - start) * 1000
            for _ in 0..<400 where shownAt == 0 { try? await Task.sleep(for: .microseconds(250)) }
            v.onFrame = nil
            if shownAt > 0 {
                samples.append((shownAt - start) * 1000)
                mainCost.append(cost)
            }
            try? await Task.sleep(for: .milliseconds(20))
        }
        samples.sort()
        func q(_ a: [Double], _ p: Double) -> Double { a.isEmpty ? .nan : a[min(a.count - 1, Int(Double(a.count - 1) * p))] }
        // The frame's main-thread work per echo: take + encode (+ render
        // submit), measured inside the view.
        Self.say(String(format: "loopback echo core n=%d: key→frame p50 %.3f ms p95 %.3f ms max %.3f; longest frame %.3f ms",
                        samples.count, q(samples, 0.5), q(samples, 0.95), samples.last ?? .nan, v.longestFrameMs))

        #expect(samples.count >= 190, "echoes lost")
    }

    // MARK: Scroll smoothness

    /// Flinging through local history by the pixel: the main thread's cost
    /// of each step (frame + encode + render submit), as a 120 Hz display
    /// would ask for them.
    @Test func pixelScrollFrameCost() async {
        let w = window()
        defer { w.isHidden = true }
        let (v, _) = coreView(cols: 70, rows: 60, in: w)
        v.push(Self.colours(3_000))
        _ = v.settle(timeout: 10) { $0.historyLines > 2_900 }
        var costs: [Double] = []
        let probe = TickProbe()
        probe.start()
        for i in 0..<600 {
            let dy: CGFloat = i < 300 ? 7.3 : -7.3
            let start = CACurrentMediaTime()
            v.scrollViewport(by: dy)
            costs.append((CACurrentMediaTime() - start) * 1000)
            try? await Task.sleep(for: .milliseconds(8))
        }
        probe.stop()
        costs.sort()
        func q(_ p: Double) -> Double { costs[min(costs.count - 1, Int(Double(costs.count - 1) * p))] }
        let gaps = probe.gaps.sorted()
        func g(_ p: Double) -> Double { gaps.isEmpty ? .nan : gaps[min(gaps.count - 1, Int(Double(gaps.count - 1) * p))] * 1000 }
        Self.say(String(format: "pixel scroll step on main: p50 %.3f ms p95 %.3f ms max %.3f ms; display frame intervals while scrolling p50 %.1f p95 %.1f max %.1f ms (%d refreshes), renders %d",
                        q(0.5), q(0.95), q(1), g(0.5), g(0.95), g(1), gaps.count, v.renders))
    }
}

/// Measures the longest gap between display-link ticks.
@MainActor
final class TickProbe: NSObject {
    private var link: CADisplayLink?
    private var last: CFTimeInterval = 0
    private(set) var longestGap: CFTimeInterval = 0

    func start() {
        let link = CADisplayLink(target: self, selector: #selector(tick(_:)))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    private(set) var gaps: [CFTimeInterval] = []

    @objc private func tick(_ link: CADisplayLink) {
        if last > 0 {
            longestGap = max(longestGap, link.timestamp - last)
            gaps.append(link.timestamp - last)
        }
        last = link.timestamp
    }

    func stop() {
        link?.invalidate()
        link = nil
    }
}
