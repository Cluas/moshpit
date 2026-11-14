#if DEBUG
import Foundation
import SwiftTerm

/// `-MOSHPIT_PREDICT_PROBE adaptive|always|never|experimental` — a bench for
/// predictive echo on a real link. Once a mosh session has been live for a
/// few seconds it types a short script through the real keyboard path
/// (``SessionHub/ActiveSession/sendInput(_:)``) at a fast typist's pace,
/// timing how long each key takes to show, then checks that the screen
/// settles to exactly what the server drew and that a no-echo prompt never
/// shows a typed character. Results: `Documents/predict-probe.log` in the
/// app container.
@MainActor
enum MoshPredictionProbe {

    static var requestedMode: PredictMode? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_PREDICT_PROBE"), i + 1 < args.count else { return nil }
        return PredictMode(rawValue: args[i + 1])
    }

    private static var started = false

    static func startIfRequested(_ session: SessionHub.ActiveSession) {
        guard let mode = requestedMode, !started else { return }
        started = true
        Task { await run(session, mode: mode) }
    }

    /// Keys a fast typist sends: ~6 a second, well inside one 500ms round trip.
    private static let cadence: UInt64 = 170_000_000

    private static func run(_ session: SessionHub.ActiveSession, mode: PredictMode) async {
        let log = Log()
        log.line("probe mode=\(mode.rawValue)")

        // Live, painted, and the prompt has settled.
        for _ in 0..<300 where !(session.viewModel.connState == .live && session.coordinator.swiftTermView != nil) {
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
        // SwiftTerm's engine writes into SwiftTerm's cells; the core's
        // predictions are measured by CoreTerminalViewTests.
        guard let view = session.coordinator.swiftTermView else { log.line("FAIL no SwiftTerm terminal"); return }
        try? await Task.sleep(nanoseconds: 6_000_000_000)

        let srtt = session.coordinator.prediction?.srttMs ?? 0
        let engine = MoshPrediction(mode: mode, trail: AppSettings.shared.trailOnPredict)
        engine.srttMs = srtt
        session.coordinator.attachPrediction(engine)
        log.line("srtt=\(Int(srtt))ms trail=\(engine.trail)")

        let terminal = view.getTerminal()
        func cursor() -> (x: Int, y: Int) { (terminal.buffer.x, terminal.buffer.y) }
        func text(row: Int, from col: Int, count: Int) -> String {
            guard let line = terminal.getLine(row: row) else { return "" }
            return String((col..<min(col + count, terminal.cols)).map { col -> Character in
                let c = terminal.getCharacter(for: line[col])
                return c == "\u{0}" ? " " : c
            })
        }
        func rowText(_ row: Int) -> String {
            var s = text(row: row, from: 0, count: terminal.cols)
            while s.last == " " { s.removeLast() }
            return s
        }

        // MARK: 1. Edit a command line: type, fix a typo, walk the cursor.

        let origin = cursor()
        log.line("prompt row=\(origin.y) col=\(origin.x) text=\(rowText(origin.y).debugDescription)")
        var line = ""
        var col = 0
        var latencies: [(key: String, ms: Int)] = []

        func step(_ key: String, _ bytes: String, line newLine: String, col newCol: Int) async {
            line = newLine
            col = newCol
            let sentAt = DispatchTime.now().uptimeNanoseconds
            session.sendInput(Data(bytes.utf8))
            // Shown = the line reads right and the caret sits where it should.
            var shown: Int?
            for _ in 0..<600 {
                if text(row: origin.y, from: origin.x, count: line.count) == line,
                   cursor() == (origin.x + col, origin.y) {
                    shown = Int((DispatchTime.now().uptimeNanoseconds - sentAt) / 1_000_000)
                    break
                }
                try? await Task.sleep(nanoseconds: 5_000_000)
            }
            latencies.append((key, shown ?? -1))
            let elapsed = DispatchTime.now().uptimeNanoseconds - sentAt
            if elapsed < cadence { try? await Task.sleep(nanoseconds: cadence - elapsed) }
        }

        /// Each cell of the line as "char:fg/dim", shown and as the server has it.
        func dumpLine(_ label: String) {
            func cells() -> String {
                guard let row = terminal.getLine(row: origin.y) else { return "" }
                return (origin.x..<min(origin.x + line.count + 1, terminal.cols)).map { col in
                    let cell = row[col]
                    let dim = cell.attribute.style.contains(.dim) ? "/dim" : ""
                    return "\(terminal.getCharacter(for: cell)):\(cell.attribute.fg)\(dim)"
                }.joined(separator: " ")
            }
            log.line("\(label) shown: \(cells())")
            guard let prediction = session.coordinator.prediction, prediction.withdraw(from: terminal) else { return }
            log.line("\(label) server: \(cells())")
            prediction.apply(to: terminal)
        }

        for (i, ch) in "echo moshpit-predikt".enumerated() {
            await step(String(ch), String(ch), line: line + String(ch), col: col + 1)
            if i == 6 || i == 15 { dumpLine("after \(line.debugDescription)") }
        }
        for _ in 0..<2 {
            await step("⌫", "\u{7f}", line: String(line.dropLast()), col: col - 1)
        }
        for ch in "ct" {
            await step(String(ch), String(ch), line: line + String(ch), col: col + 1)
        }
        for _ in 0..<3 { await step("←", "\u{1b}[D", line: line, col: col - 1) }
        for _ in 0..<3 { await step("→", "\u{1b}[C", line: line, col: col + 1) }

        let shownAll = latencies.map(\.ms)
        log.line("keys: " + latencies.map { "\($0.key)=\($0.ms)" }.joined(separator: " "))
        let tail = shownAll.dropFirst(3).filter { $0 >= 0 }.sorted()
        if !tail.isEmpty {
            log.line("latency after the first 3 keys: median=\(tail[tail.count / 2])ms max=\(tail.last!)ms n=\(tail.count)")
        }

        session.sendInput(Data("\r".utf8))
        try? await Task.sleep(nanoseconds: 4_000_000_000)
        let applied = session.coordinator.prediction?.isApplied ?? false
        let typed = text(row: origin.y, from: origin.x, count: line.count)
        let output = rowText(origin.y + 1)
        log.line("settled: overlayApplied=\(applied) line=\(typed.debugDescription) next=\(output.debugDescription)")
        let editOK = !applied && typed == "echo moshpit-predict" && output == "moshpit-predict"
        log.line(editOK ? "PASS edit" : "FAIL edit")

        // MARK: 2. A prompt that doesn't echo.

        for ch in "stty -echo; read x; stty echo; echo \"[${#x}]\"" {
            session.sendInput(Data(String(ch).utf8))
            try? await Task.sleep(nanoseconds: cadence)
        }
        session.sendInput(Data("\r".utf8))
        try? await Task.sleep(nanoseconds: 4_000_000_000)
        let quiet = cursor()
        var leaks: [String] = []
        for ch in "hunter2" {
            session.sendInput(Data(String(ch).utf8))
            let sentAt = DispatchTime.now().uptimeNanoseconds
            while DispatchTime.now().uptimeNanoseconds - sentAt < cadence {
                let shown = text(row: quiet.y, from: quiet.x, count: 8).trimmingCharacters(in: .whitespaces)
                if !shown.isEmpty, leaks.last != shown { leaks.append(shown) }
                try? await Task.sleep(nanoseconds: 5_000_000)
            }
        }
        for _ in 0..<200 {   // the guesses' judgement lands a round trip later
            let shown = text(row: quiet.y, from: quiet.x, count: 8).trimmingCharacters(in: .whitespaces)
            if !shown.isEmpty, leaks.last != shown { leaks.append(shown) }
            try? await Task.sleep(nanoseconds: 5_000_000)
        }
        session.sendInput(Data("\r".utf8))
        try? await Task.sleep(nanoseconds: 4_000_000_000)
        // Echo off swallowed the Return too: the count lands on the read's own row.
        let echoed = rowText(quiet.y)
        log.line("no-echo: leaks=\(leaks) after=\(echoed.debugDescription)")
        log.line(leaks.isEmpty && echoed == "[7]" ? "PASS no-echo" : "FAIL no-echo")
        log.line("done")
    }

    private struct Log {
        private let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("predict-probe.log")

        init() { try? Data().write(to: url) }

        func line(_ text: String) {
            let stamped = "\(Date().timeIntervalSince1970) \(text)\n"
            guard let handle = try? FileHandle(forWritingTo: url) else { return }
            handle.seekToEndOfFile()
            handle.write(Data(stamped.utf8))
            try? handle.close()
        }
    }
}
#endif
