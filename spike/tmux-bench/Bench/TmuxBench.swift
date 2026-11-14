// G2's device criterion (docs/nsh/DESIGN.md, the gate table: "真机 CPU 不高于
// 旧实现"): the CPU the app's TmuxControlClient spends parsing the same
// control-mode bytes with its Swift parser and with the Rust one. The client
// is the app's own source file (project.yml lists it), fed and called back
// exactly as TmuxSessionController feeds it: `await client.feed(chunk)` from
// a task, every callback installed.
//
// Inputs:
//   - echo: one keystroke's echo, `%output %0 a\r\n`, fed 100 000 times —
//     the cost of one feed when there is next to nothing to parse;
//   - captures: the committed tmux 3.6a recordings of crates/tmux (plain,
//     CJK and emoji, colours, vim and less, layouts, a session switch,
//     errors, a boot chain, pause-after) and the synthetic recovery stream;
//   - burst: `seq 1 200000` the way tmux -CC writes it (CRLF as \015\012,
//     two or three numbers per %output line, 3.7 MB), synthesised here: the
//     recorded one is not committed.
// Each is cut into fixed-size feeds. A round is one fresh client per parser
// fed the whole input; rounds alternate the parsers, the first round of
// each is a warm-up, and the report gives the medians. CPU is the process's
// (CLOCK_PROCESS_CPUTIME_ID): the actor hops, the parser, the callbacks.

import Foundation
import UIKit

enum TmuxBench {
    struct Case {
        let input: String
        let data: Data
        let chunk: Int
        let rounds: Int
    }

    /// What the callbacks saw: equal for both parsers or the bench is void.
    struct Counts: Equatable {
        var events = 0
        var outputBytes = 0
        var responses = 0
        var responseLines = 0
        var errors = 0
    }

    struct Sample {
        let cpu: UInt64
        let wall: UInt64
    }

    static func run() async -> String {
        var report: [String] = []
        func say(_ line: String) {
            print("BENCH \(line)")
            report.append(line)
        }
        var machine = utsname()
        uname(&machine)
        let model = withUnsafeBytes(of: &machine.machine) { raw in
            String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self)
        }
        let rust = TmuxParserBackend.rustAvailable ? "linked" : "MISSING"
        say("device \(model) iOS \(await UIDevice.current.systemVersion), Rust parser \(rust), "
            + "thermal \(ProcessInfo.processInfo.thermalState.rawValue)")
        say("input chunk feeds | parser cpu/round wall/round MB/s(cpu) ns/feed(cpu) | swift÷rust cpu")

        for benchCase in cases() {
            let chunks = stride(from: 0, to: benchCase.data.count, by: benchCase.chunk).map {
                Data(benchCase.data[$0..<min($0 + benchCase.chunk, benchCase.data.count)])
            }
            var samples: [TmuxParserBackend: [Sample]] = [:]
            var counts: [TmuxParserBackend: Counts] = [:]
            for round in 0...benchCase.rounds {
                for backend in TmuxParserBackend.allCases {
                    let (sample, seen) = await measure(backend, chunks)
                    if round > 0 { samples[backend, default: []].append(sample) }
                    counts[backend] = seen
                }
            }
            let same = counts[.swift] == counts[.rust]
            let swiftCPU = median(samples[.swift, default: []].map(\.cpu))
            let rustCPU = median(samples[.rust, default: []].map(\.cpu))
            for backend in TmuxParserBackend.allCases {
                let cpu = median(samples[backend, default: []].map(\.cpu))
                let wall = median(samples[backend, default: []].map(\.wall))
                let megabytes = Double(benchCase.data.count) / 1_000_000
                say(String(format: "%@ %d %d | %@ %.2fms %.2fms %.1f %.0f",
                           benchCase.input, benchCase.chunk, chunks.count, backend.rawValue,
                           Double(cpu) / 1e6, Double(wall) / 1e6,
                           megabytes / (Double(cpu) / 1e9), Double(cpu) / Double(chunks.count)))
            }
            say(String(format: "%@ %d | swift÷rust %.2fx | same events %@ (%d events, %d output bytes)",
                       benchCase.input, benchCase.chunk, Double(swiftCPU) / Double(max(rustCPU, 1)),
                       same ? "yes" : "NO", counts[.rust]?.events ?? 0, counts[.rust]?.outputBytes ?? 0))
        }
        say("thermal \(ProcessInfo.processInfo.thermalState.rawValue) done")
        return report.joined(separator: "\n")
    }

    static func cases() -> [Case] {
        let echoLine = "%output %0 a\r\n"
        let echo = Data(String(repeating: echoLine, count: 100_000).utf8)
        let captures = recordedCaptures()
        let burst = seqBurst()
        return [
            Case(input: "echo", data: echo, chunk: echoLine.utf8.count, rounds: 5),
            Case(input: "captures", data: captures, chunk: 512, rounds: 15),
            Case(input: "captures", data: captures, chunk: 4096, rounds: 15),
            Case(input: "burst", data: burst, chunk: 4096, rounds: 5),
            Case(input: "burst", data: burst, chunk: 32768, rounds: 5),
        ]
    }

    /// One round: a fresh client of `backend`, every callback installed,
    /// every chunk fed in order.
    static func measure(_ backend: TmuxParserBackend, _ chunks: [Data]) async -> (Sample, Counts) {
        let counter = Counter()
        let client = TmuxControlClient(backend: backend)
        await client.setCallbacks(
            onPaneOutput: { _, data in counter.output(data.count) },
            onLayoutChange: { _, _, _ in counter.event() },
            onWindowAdd: { _ in counter.event() },
            onWindowClose: { _ in counter.event() },
            onWindowRenamed: { _, _ in counter.event() },
            onSessionChanged: { _, _ in counter.event() },
            onSessionRenamed: { _, _ in counter.event() },
            onSessionWindowChanged: { _, _ in counter.event() },
            onSessionsChanged: { counter.event() },
            onUnlinkedWindowAdd: { _ in counter.event() },
            onUnlinkedWindowClose: { _ in counter.event() },
            onUnlinkedWindowRenamed: { _, _ in counter.event() },
            onClientSessionChanged: { _, _, _ in counter.event() },
            onActivePaneChanged: { _, _ in counter.event() },
            onPause: { _ in counter.event() },
            onContinue: { _ in counter.event() },
            onClientDetached: { _ in counter.event() },
            onExit: { _ in counter.event() },
            onCommandResponse: { counter.response($0.lines.count) },
            onProtocolError: { _ in counter.error() })
        let cpu0 = clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID)
        let wall0 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
        for chunk in chunks {
            await client.feed(chunk)
        }
        let wall = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) - wall0
        let cpu = clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID) - cpu0
        return (Sample(cpu: cpu, wall: wall), counter.counts)
    }

    static func median(_ values: [UInt64]) -> UInt64 {
        let sorted = values.sorted()
        return sorted.isEmpty ? 0 : sorted[sorted.count / 2]
    }

    /// The committed recordings, end to end (each starts its own control
    /// session, so the concatenation is a valid stream).
    static func recordedCaptures() -> Data {
        var data = Data()
        for folder in ["captures", "synthetic"] {
            guard let urls = Bundle.main.urls(forResourcesWithExtension: "bin", subdirectory: folder) else {
                continue
            }
            for url in urls.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
                data.append((try? Data(contentsOf: url)) ?? Data())
            }
        }
        return data
    }

    /// `seq 1 200000` as tmux -CC sends it: the boot block, then %output
    /// lines of two or three numbers, each line ending escaped as
    /// \015\012.
    static func seqBurst() -> Data {
        var text = "%begin 1700000000 100 0\r\n%end 1700000000 100 0\r\n"
        text.reserveCapacity(3_800_000)
        var number = 1
        var three = false
        while number <= 200_000 {
            text += "%output %0 "
            for _ in 0..<(three ? 3 : 2) where number <= 200_000 {
                text += "\(number)\\015\\012"
                number += 1
            }
            text += "\r\n"
            three.toggle()
        }
        return Data(text.utf8)
    }
}

/// Counts what the callbacks saw. They all run on the client's actor, one at
/// a time, and the bench reads the counts after the last feed returned.
private final class Counter: @unchecked Sendable {
    private(set) var counts = TmuxBench.Counts()

    func output(_ bytes: Int) {
        counts.events += 1
        counts.outputBytes += bytes
    }

    func event() {
        counts.events += 1
    }

    func response(_ lines: Int) {
        counts.events += 1
        counts.responses += 1
        counts.responseLines += lines
    }

    func error() {
        counts.events += 1
        counts.errors += 1
    }
}
