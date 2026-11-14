// Harness-only command-line front end for the app's tmux -CC parser.
//
// Built by harness/build-swift.sh together with the UNCHANGED app source
// Moshpit/Services/Tmux/TmuxControlClient.swift (macOS, no UIKit). It reads
// a raw control-mode byte stream from a file, feeds it to TmuxControlClient
// in the chunks a plan dictates, and prints every callback as one JSON line.
// crates/tmux/examples/cc_events.rs prints the same shape for the Rust port,
// byte for byte; harness/equivalence.py diffs the two.
//
//   cc-events-swift events <capture> <plan> [<out>]
//   cc-events-swift bench <capture> <chunk-size|whole> <iterations>
//
// Plans (identical in cc_events.rs, SplitMix64 drives every random choice):
//   whole            one feed
//   bytes            one byte per feed
//   fixed:N          N bytes per feed
//   rand:SEED:MAX    feed sizes uniform in 1...MAX
//   inside:SEED      cuts only inside UTF-8 sequences and octal escapes
//   reset:SEED:MAX   like rand, plus reset() after ~1 in 64 feeds
// The last line, {"ev":"end",...}, counts the cuts (and those inside UTF-8
// sequences and octal escapes) and carries a hash of the plan, so a diff
// also proves both parsers saw the same chunk boundaries.

import Foundation

// MARK: - Plan

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

enum Step {
    case feed(Range<Int>)
    case reset
}

func isOctal(_ byte: UInt8) -> Bool { byte >= 0x30 && byte <= 0x37 }

func makePlan(_ spec: String, _ data: [UInt8]) -> [Step] {
    let fields = spec.split(separator: ":").map(String.init)
    let count = data.count
    var steps: [Step] = []
    func sized(_ next: () -> Int) {
        var at = 0
        while at < count {
            let end = min(count, at + next())
            steps.append(.feed(at..<end))
            at = end
        }
    }
    switch fields[0] {
    case "whole":
        if count > 0 { steps.append(.feed(0..<count)) }
    case "bytes":
        sized { 1 }
    case "fixed":
        let size = Int(fields[1])!
        sized { size }
    case "rand":
        var rng = SplitMix64(state: UInt64(fields[1])!)
        let max = UInt64(fields[2])!
        sized { Int(1 + rng.next() % max) }
    case "reset":
        var rng = SplitMix64(state: UInt64(fields[1])!)
        let max = UInt64(fields[2])!
        var at = 0
        while at < count {
            let end = min(count, at + Int(1 + rng.next() % max))
            steps.append(.feed(at..<end))
            at = end
            if rng.next() % 64 == 0 { steps.append(.reset) }
        }
    case "inside":
        var rng = SplitMix64(state: UInt64(fields[1])!)
        var start = 0
        var cut = 1
        while cut < count {
            let byte = data[cut]
            let insideUTF8 = byte >= 0x80 && byte <= 0xBF
            let afterBackslash = data[cut - 1] == 0x5C
            let afterOneDigit = cut >= 2 && data[cut - 2] == 0x5C && isOctal(data[cut - 1])
            let afterTwoDigits = cut >= 3 && data[cut - 3] == 0x5C
                && isOctal(data[cut - 2]) && isOctal(data[cut - 1])
            if insideUTF8 || afterBackslash || afterOneDigit || afterTwoDigits,
               rng.next() % 3 == 0 {
                steps.append(.feed(start..<cut))
                start = cut
            }
            cut += 1
        }
        if start < count { steps.append(.feed(start..<count)) }
    default:
        fatalError("unknown plan \(spec)")
    }
    return steps
}

/// Where the feeds end: how many cuts fall inside a UTF-8 sequence (before
/// a continuation byte) and inside a backslash escape (after the backslash
/// or one or two of its octal digits).
func cutStats(_ steps: [Step], _ data: [UInt8]) -> (cuts: Int, utf8: Int, octal: Int) {
    var cuts = 0, utf8 = 0, octal = 0
    for step in steps {
        guard case .feed(let range) = step, range.upperBound < data.count else { continue }
        let cut = range.upperBound
        cuts += 1
        if data[cut] >= 0x80 && data[cut] <= 0xBF { utf8 += 1 }
        if data[cut - 1] == 0x5C
            || (cut >= 2 && data[cut - 2] == 0x5C && isOctal(data[cut - 1]))
            || (cut >= 3 && data[cut - 3] == 0x5C && isOctal(data[cut - 2]) && isOctal(data[cut - 1])) {
            octal += 1
        }
    }
    return (cuts, utf8, octal)
}

/// FNV-1a over every step: a feed hashes its length (u64 little endian), a
/// reset hashes u64.max.
func planHash(_ steps: [Step]) -> UInt64 {
    var hash: UInt64 = 0xCBF2_9CE4_8422_2325
    for step in steps {
        var word: UInt64
        switch step {
        case .feed(let range): word = UInt64(range.count)
        case .reset: word = UInt64.max
        }
        for _ in 0..<8 {
            hash ^= word & 0xFF
            hash &*= 0x0000_0100_0000_01B3
            word >>= 8
        }
    }
    return hash
}

// MARK: - JSON lines

struct JSONLine {
    var bytes: [UInt8] = []

    mutating func raw(_ text: String) { bytes.append(contentsOf: text.utf8) }

    /// `"`, `\` and C0 controls escaped (controls always as \u00xx, lower
    /// hex), everything else verbatim UTF-8.
    mutating func string(_ text: String) {
        bytes.append(0x22)
        for byte in text.utf8 {
            switch byte {
            case 0x22: bytes.append(contentsOf: [0x5C, 0x22])
            case 0x5C: bytes.append(contentsOf: [0x5C, 0x5C])
            case 0x00..<0x20:
                bytes.append(contentsOf: Array("\\u00".utf8))
                bytes.append(hexDigit(byte >> 4))
                bytes.append(hexDigit(byte & 0x0F))
            default: bytes.append(byte)
            }
        }
        bytes.append(0x22)
    }

    mutating func hex(_ data: Data) {
        bytes.append(0x22)
        for byte in data {
            bytes.append(hexDigit(byte >> 4))
            bytes.append(hexDigit(byte & 0x0F))
        }
        bytes.append(0x22)
    }

    private func hexDigit(_ nibble: UInt8) -> UInt8 {
        nibble < 10 ? 0x30 + nibble : 0x61 + nibble - 10
    }
}

/// Collects callback output. Callbacks only run inside `await client.feed`,
/// which `main` awaits one at a time, so access is already serialized.
final class Sink: @unchecked Sendable {
    var out: [UInt8] = []

    func emit(_ build: (inout JSONLine) -> Void) {
        var line = JSONLine()
        build(&line)
        out.append(contentsOf: line.bytes)
        out.append(0x0A)
    }
}

func install(_ sink: Sink, on client: TmuxControlClient) async {
    await client.setCallbacks(
        onPaneOutput: { pane, data in
            sink.emit { $0.raw(#"{"ev":"output","pane":"#); $0.string(pane)
                $0.raw(#","hex":"#); $0.hex(data); $0.raw("}") }
        },
        onLayoutChange: { window, layout, zoomed in
            sink.emit { $0.raw(#"{"ev":"layout-change","window":"#); $0.string(window)
                $0.raw(#","layout":"#); $0.string(layout)
                $0.raw(#","zoomed":"#); $0.raw(zoomed ? "true" : "false"); $0.raw("}") }
        },
        onWindowAdd: { window in
            sink.emit { $0.raw(#"{"ev":"window-add","window":"#); $0.string(window); $0.raw("}") }
        },
        onWindowClose: { window in
            sink.emit { $0.raw(#"{"ev":"window-close","window":"#); $0.string(window); $0.raw("}") }
        },
        onWindowRenamed: { window, name in
            sink.emit { $0.raw(#"{"ev":"window-renamed","window":"#); $0.string(window)
                $0.raw(#","name":"#); $0.string(name); $0.raw("}") }
        },
        onSessionChanged: { session, name in
            sink.emit { $0.raw(#"{"ev":"session-changed","session":"#); $0.string(session)
                $0.raw(#","name":"#); $0.string(name); $0.raw("}") }
        },
        onSessionRenamed: { session, name in
            sink.emit { $0.raw(#"{"ev":"session-renamed","session":"#); $0.string(session)
                $0.raw(#","name":"#); $0.string(name); $0.raw("}") }
        },
        onSessionWindowChanged: { session, window in
            sink.emit { $0.raw(#"{"ev":"session-window-changed","session":"#); $0.string(session)
                $0.raw(#","window":"#); $0.string(window); $0.raw("}") }
        },
        onSessionsChanged: {
            sink.emit { $0.raw(#"{"ev":"sessions-changed"}"#) }
        },
        onUnlinkedWindowAdd: { window in
            sink.emit { $0.raw(#"{"ev":"unlinked-window-add","window":"#); $0.string(window); $0.raw("}") }
        },
        onUnlinkedWindowClose: { window in
            sink.emit { $0.raw(#"{"ev":"unlinked-window-close","window":"#); $0.string(window); $0.raw("}") }
        },
        onUnlinkedWindowRenamed: { window, name in
            sink.emit { $0.raw(#"{"ev":"unlinked-window-renamed","window":"#); $0.string(window)
                $0.raw(#","name":"#); $0.string(name); $0.raw("}") }
        },
        onClientSessionChanged: { client, session, name in
            sink.emit { $0.raw(#"{"ev":"client-session-changed","client":"#); $0.string(client)
                $0.raw(#","session":"#); $0.string(session)
                $0.raw(#","name":"#); $0.string(name); $0.raw("}") }
        },
        onActivePaneChanged: { window, pane in
            sink.emit { $0.raw(#"{"ev":"window-pane-changed","window":"#); $0.string(window)
                $0.raw(#","pane":"#); $0.string(pane); $0.raw("}") }
        },
        onPause: { pane in
            sink.emit { $0.raw(#"{"ev":"pause","pane":"#); $0.string(pane); $0.raw("}") }
        },
        onContinue: { pane in
            sink.emit { $0.raw(#"{"ev":"continue","pane":"#); $0.string(pane); $0.raw("}") }
        },
        onClientDetached: { payload in
            sink.emit { $0.raw(#"{"ev":"client-detached","payload":"#); $0.string(payload); $0.raw("}") }
        },
        onExit: { reason in
            sink.emit { line in
                line.raw(#"{"ev":"exit","reason":"#)
                if let reason { line.string(reason) } else { line.raw("null") }
                line.raw("}")
            }
        },
        onCommandResponse: { response in
            sink.emit { line in
                line.raw(#"{"ev":"response","id":"#); line.raw(String(response.commandId))
                line.raw(#","num":"#); line.raw(String(response.commandNum))
                line.raw(#","error":"#); line.raw(response.isError ? "true" : "false")
                line.raw(#","lines":["#)
                for (index, text) in response.lines.enumerated() {
                    if index > 0 { line.raw(",") }
                    line.string(text)
                }
                line.raw("]}")
            }
        },
        onProtocolError: { error in
            sink.emit { line in
                line.raw(#"{"ev":"protocol-error","message":"#)
                switch error {
                case .malformedLine(let text): line.string(text)
                }
                line.raw("}")
            }
        }
    )
}

// MARK: - Modes

func readCapture(_ path: String) -> [UInt8] {
    guard let data = FileManager.default.contents(atPath: path) else {
        FileHandle.standardError.write(Data("cannot read \(path)\n".utf8))
        exit(2)
    }
    return [UInt8](data)
}

func runEvents(capture: String, plan spec: String, out: String?) async {
    let data = readCapture(capture)
    let steps = makePlan(spec, data)
    let chunks: [Data?] = steps.map { step in
        if case .feed(let range) = step { return Data(data[range]) }
        return nil
    }
    let client = TmuxControlClient()
    let sink = Sink()
    await install(sink, on: client)
    var feeds = 0
    var resets = 0
    for chunk in chunks {
        if let chunk {
            await client.feed(chunk)
            feeds += 1
        } else {
            await client.reset()
            resets += 1
        }
    }
    sink.emit { line in
        line.raw(#"{"ev":"end","plan":"#); line.string(spec)
        line.raw(#","feeds":"#); line.raw(String(feeds))
        line.raw(#","resets":"#); line.raw(String(resets))
        line.raw(#","bytes":"#); line.raw(String(data.count))
        let stats = cutStats(steps, data)
        line.raw(#","cuts":"#); line.raw(String(stats.cuts))
        line.raw(#","in_utf8":"#); line.raw(String(stats.utf8))
        line.raw(#","in_octal":"#); line.raw(String(stats.octal))
        line.raw(#","plan_hash":""#)
        line.raw(String(format: "%016llx", planHash(steps)))
        line.raw("\"}")
    }
    let output = Data(sink.out)
    if let out {
        FileManager.default.createFile(atPath: out, contents: output)
    } else {
        FileHandle.standardOutput.write(output)
    }
}

/// Counts events without formatting them, so the bench measures the parser.
final class Counter: @unchecked Sendable {
    var events = 0
    var outputBytes = 0
}

func runBench(capture: String, chunk: String, iterations: Int) async {
    let data = readCapture(capture)
    let steps = makePlan(chunk == "whole" ? "whole" : "fixed:\(chunk)", data)
    let chunks: [Data] = steps.compactMap { step in
        if case .feed(let range) = step { return Data(data[range]) }
        return nil
    }
    let counter = Counter()
    let clock = ContinuousClock()
    var best = Duration.seconds(1_000_000)
    var total = Duration.zero
    for _ in 0..<iterations {
        let client = TmuxControlClient()
        await client.setCallbacks(
            onPaneOutput: { _, data in counter.events += 1; counter.outputBytes += data.count },
            onLayoutChange: { _, _, _ in counter.events += 1 },
            onWindowAdd: { _ in counter.events += 1 },
            onWindowClose: { _ in counter.events += 1 },
            onWindowRenamed: { _, _ in counter.events += 1 },
            onSessionChanged: { _, _ in counter.events += 1 },
            onSessionRenamed: { _, _ in counter.events += 1 },
            onSessionWindowChanged: { _, _ in counter.events += 1 },
            onSessionsChanged: { counter.events += 1 },
            onUnlinkedWindowAdd: { _ in counter.events += 1 },
            onUnlinkedWindowClose: { _ in counter.events += 1 },
            onUnlinkedWindowRenamed: { _, _ in counter.events += 1 },
            onClientSessionChanged: { _, _, _ in counter.events += 1 },
            onActivePaneChanged: { _, _ in counter.events += 1 },
            onPause: { _ in counter.events += 1 },
            onContinue: { _ in counter.events += 1 },
            onClientDetached: { _ in counter.events += 1 },
            onExit: { _ in counter.events += 1 },
            onCommandResponse: { _ in counter.events += 1 },
            onProtocolError: { _ in counter.events += 1 })
        let start = clock.now
        for piece in chunks { await client.feed(piece) }
        let elapsed = clock.now - start
        total += elapsed
        if elapsed < best { best = elapsed }
    }
    func seconds(_ d: Duration) -> Double {
        Double(d.components.seconds) + Double(d.components.attoseconds) / 1e18
    }
    let megabytes = Double(data.count) / 1_000_000
    let mean = seconds(total) / Double(iterations)
    print(String(format: #"{"impl":"swift","capture":"%@","chunk":"%@","iterations":%d,"bytes":%d,"#
                 + #""feeds":%d,"events":%d,"output_bytes":%d,"best_s":%.6f,"mean_s":%.6f,"#
                 + #""best_mb_s":%.1f,"mean_mb_s":%.1f}"#,
                 (capture as NSString).lastPathComponent, chunk, iterations, data.count, chunks.count,
                 counter.events / iterations, counter.outputBytes / iterations,
                 seconds(best), mean, megabytes / seconds(best), megabytes / mean))
}

@main
struct CCEventsSwift {
    static func main() async {
        let args = CommandLine.arguments
        guard args.count >= 4 else {
            FileHandle.standardError.write(Data("""
                usage: cc-events-swift events <capture> <plan> [<out>]
                       cc-events-swift bench <capture> <chunk-size|whole> <iterations>

                """.utf8))
            exit(2)
        }
        switch args[1] {
        case "events":
            await runEvents(capture: args[2], plan: args[3], out: args.count > 4 ? args[4] : nil)
        case "bench":
            await runBench(capture: args[2], chunk: args[3], iterations: args.count > 4 ? Int(args[4])! : 5)
        default:
            FileHandle.standardError.write(Data("unknown mode \(args[1])\n".utf8))
            exit(2)
        }
    }
}
