// Harness-only command line over the app's UNCHANGED mosh sources
// (OCB3, MoshCrypto, MoshCompression, MoshWire, MoshTransport,
// MoshBootstrap), built by harness/build-swift.sh. examples/mosh_harness.rs
// is the same command line over the Rust crate; harness/equivalence.py and
// harness/live.py drive both and compare.
//
//   mosh-swift wire <cases> [<out>]      one result line per case line
//   mosh-swift ssp <scenario> [<out>]    a scenario through MoshTransport
//   mosh-swift bench <what> <size> <iterations>
//   mosh-swift live <host> <port> <key> <ttl-ms> <steps...>
//
// Formats are documented in src/harness.rs, which implements the Rust side.

import Foundation
import Network

// MARK: - Output

final class Output {
    private var handle: FileHandle
    private var buffer = Data()
    init(path: String?) {
        if let path {
            FileManager.default.createFile(atPath: path, contents: nil)
            handle = FileHandle(forWritingAtPath: path)!
        } else {
            handle = FileHandle.standardOutput
        }
    }
    func line(_ text: String) {
        buffer.append(contentsOf: Array(text.utf8))
        buffer.append(0x0A)
        if buffer.count > 1 << 16 { flush() }
    }
    func flush() {
        handle.write(buffer)
        buffer.removeAll(keepingCapacity: true)
    }
}

func hex(_ data: Data) -> String { hex([UInt8](data)) }
func hex(_ bytes: [UInt8]) -> String {
    let digits = Array("0123456789abcdef".utf8)
    var out = [UInt8]()
    out.reserveCapacity(bytes.count * 2)
    for b in bytes {
        out.append(digits[Int(b >> 4)])
        out.append(digits[Int(b & 0xF)])
    }
    return String(decoding: out, as: UTF8.self)
}

func unhex(_ text: Substring) -> Data {
    var out = Data()
    out.reserveCapacity(text.count / 2)
    var index = text.startIndex
    while index < text.endIndex {
        let next = text.index(index, offsetBy: 2)
        out.append(UInt8(text[index..<next], radix: 16)!)
        index = next
    }
    return out
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(2)
}

func wallMs() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

// MARK: - Live: the app's transport against a real mosh-server

/// The app's NWConnectionChannel, counting the receives pending on it at
/// once: one flow must never have two (`live.py rebuild`).
final class CountingChannel: DatagramChannel, @unchecked Sendable {
    private let inner: NWConnectionChannel
    private let lock = NSLock()
    private var pending = 0
    private var mostPending = 0
    var receives: Int { lock.withLock { mostPending } }
    init(host: String, port: NWEndpoint.Port) { inner = NWConnectionChannel(host: host, port: port) }
    var state: NWConnection.State { inner.state }
    var onStateChange: (@Sendable (NWConnection.State) -> Void)? {
        get { inner.onStateChange }
        set { inner.onStateChange = newValue }
    }
    var onBetterPath: (@Sendable (Bool) -> Void)? {
        get { inner.onBetterPath }
        set { inner.onBetterPath = newValue }
    }
    func start() { inner.start() }
    func restart() { inner.restart() }
    func cancel() { inner.cancel() }
    func cancelAfterSends() { inner.cancelAfterSends() }
    func send(_ datagram: Data) { inner.send(datagram) }
    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {
        lock.withLock {
            pending += 1
            mostPending = max(mostPending, pending)
        }
        inner.receive { [self] data, error in
            lock.withLock { pending -= 1 }
            completion(data, error)
        }
    }
}

/// Every flow the live transport opened, in order.
final class Flows: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [CountingChannel] = []
    func add(_ flow: CountingChannel) -> CountingChannel { lock.withLock { all.append(flow) }; return flow }
    var receives: [Int] { lock.withLock { all.map(\.receives) } }
}

/// stdout, one whole line at a time, from any task.
final class LinePrinter: @unchecked Sendable {
    private let lock = NSLock()
    private let out = Output(path: nil)
    func emit(_ text: String) { lock.lock(); out.line(text); out.flush(); lock.unlock() }
}

/// Steps: wait:MS, type:HEX, typeeach:COUNT:INTERVAL_MS (byte i is
/// 0x21 + i % 94), pastefile:PATH (the file's bytes as one input),
/// redraw, resize:COLS:ROWS, mark:LABEL, rebuild (a forced resume: a new
/// flow, as after a long suspension).
func live(_ args: [String]) async {
    guard args.count >= 4, let port = UInt16(args[1]), let ttl = UInt64(args[3]) else {
        fail("usage: live <host> <port> <key> <ttl-ms> <steps...>")
    }
    let host = args[0]
    var b64 = args[2]
    while b64.count % 4 != 0 { b64.append("=") }
    guard let key = Data(base64Encoded: b64), key.count == 16,
          let nwPort = NWEndpoint.Port(rawValue: port) else { fail("bad key or port") }
    let printer = LinePrinter()
    @Sendable func emit(_ text: String) { printer.emit(text) }

    let credentials = MoshCredentials(host: host, udpPort: port, key: key)
    let flows = Flows()
    let transport: MoshTransport
    do {
        transport = try MoshTransport(
            credentials: credentials,
            channel: flows.add(CountingChannel(host: host, port: nwPort)),
            makeChannel: { flows.add(CountingChannel(host: host, port: nwPort)) },
            keystrokeTTLMs: ttl)
    } catch { fail("transport: \(error)") }
    await MainActor.run {
        transport.onSessionEnded = { emit("ended \(wallMs())") }
        transport.onReturnPathDead = { _ in emit("returnpathdead \(wallMs())") }
    }
    let pump = Task {
        for await frame in transport.hostStream {
            emit("frame \(wallMs()) \(frame.echoAck) \(hex(frame.bytes))")
        }
    }
    emit("start \(wallMs())")
    await transport.start(cols: 80, rows: 24)
    for step in args.dropFirst(4) {
        let fields = step.split(separator: ":", omittingEmptySubsequences: false)
        switch fields[0] {
        case "wait":
            // Waits end early once the session ended: nothing else happens.
            var left = UInt64(fields[1])!
            while left > 0, await !transport.sessionEnded {
                let slice = min(left, 50)
                try? await Task.sleep(nanoseconds: slice * 1_000_000)
                left -= slice
            }
        case "type":
            let keys = unhex(fields[1])
            let num = await transport.send(keys)
            emit("sent \(wallMs()) \(num ?? 0) \(hex(keys))")
        case "typeeach":
            let count = Int(fields[1])!, interval = UInt64(fields[2])!
            for i in 0..<count {
                let key = Data([UInt8(0x21 + i % 94)])
                let num = await transport.send(key)
                emit("sent \(wallMs()) \(num ?? 0) \(hex(key))")
                try? await Task.sleep(nanoseconds: interval * 1_000_000)
            }
        case "pastefile":
            guard let data = FileManager.default.contents(atPath: String(fields[1])) else {
                fail("cannot read \(fields[1])")
            }
            // Timed before it goes: `send` hands the datagrams over at once.
            let at = wallMs()
            let num = await transport.send(data)
            emit("paste \(at) \(num ?? 0) \(data.count)")
        case "redraw":
            await transport.requestFullRedraw()
            emit("redraw \(wallMs())")
        case "resize":
            await transport.resize(cols: Int(fields[1])!, rows: Int(fields[2])!)
            emit("resize \(wallMs()) \(fields[1]) \(fields[2])")
        case "mark":
            emit("mark \(wallMs()) \(fields[1])")
        case "rebuild":
            await transport.resume(force: true)
            emit("rebuild \(wallMs())")
        default:
            fail("unknown step \(step)")
        }
    }
    let d = await transport.currentDiagnostics
    emit("diag received=\(d.datagramsReceived) applied=\(d.appliedHostNum) gaps=\(d.gapEvents) parseFailures=\(d.parseFailures)")
    emit("receives \(flows.receives.map(String.init).joined(separator: ","))")
    await transport.close()
    _ = await pump.value
}

// MARK: - Wire cases (format: src/harness/wire.rs)

func errorName(_ error: Error) -> String {
    if let boot = error as? MoshBootstrap.BootstrapError {
        switch boot {
        case .noConnectLine: return "noConnectLine"
        case .unusablePort(let port): return "unusablePort(\(port))"
        case .badKey: return "badKey"
        }
    }
    return String(describing: error)
}

func listOrDash(_ items: [String]) -> String { items.isEmpty ? "-" : items.joined(separator: ",") }

func splitList(_ text: Substring) -> [Substring] {
    text == "-" || text.isEmpty ? [] : text.split(separator: ",", omittingEmptySubsequences: false)
}

func formatEvents(_ events: [UserEvent]) -> String {
    listOrDash(events.map { event in
        switch event {
        case .keystroke(let keys): return "k" + hex(keys)
        case .resize(let width, let height): return "r\(width)x\(height)"
        }
    })
}

func parseEvents(_ text: Substring) -> [UserEvent] {
    splitList(text).map { item in
        let body = item.dropFirst()
        if item.first == "k" { return .keystroke(keys: unhex(body)) }
        let parts = body.split(separator: "x")
        return .resize(width: Int(parts[0])!, height: Int(parts[1])!)
    }
}

func instructionFields(_ i: TransportInstruction) -> String {
    "v=\(i.protocolVersion) old=\(i.oldNum) new=\(i.newNum) ack=\(i.ackNum) thr=\(i.throwawayNum) diff=\(hex(i.diff)) chaff=\(hex(i.chaff))"
}

func hostFields(_ host: HostMessage) -> String {
    let resize = host.resize.map { "\($0.width)x\($0.height)" } ?? "-"
    let echo = host.echoAck.map { String($0) } ?? "-"
    return "bytes=\(hex(host.hostBytes)) resize=\(resize) echo=\(echo)"
}

func receivePipeline(_ crypto: MoshCrypto, _ packets: Substring) -> String {
    var assembler = FragmentAssembler()
    var out: [String] = []
    for item in splitList(packets) {
        let message: MoshCrypto.Message
        do { message = try crypto.open(unhex(item)) } catch {
            out.append("x:" + errorName(error)); continue
        }
        let head = "\(message.seq):\(message.direction == .toServer ? 0 : 1):\(message.timestamp):\(message.timestampReply)"
        let piece: TransportFragment
        do { piece = try TransportFragment(parsing: message.payload) } catch {
            out.append("f:\(head):" + errorName(error)); continue
        }
        guard let payload = assembler.receive(piece) else { out.append("p:\(head)"); continue }
        let bytes: Data
        do { bytes = try MoshCompression.decompress(payload) } catch {
            out.append("z:\(head):" + errorName(error)); continue
        }
        let instruction: TransportInstruction
        do { instruction = try TransportInstruction(parsing: bytes) } catch {
            out.append("i:\(head):" + errorName(error)); continue
        }
        var diff = "-"
        if !instruction.diff.isEmpty {
            if message.direction == .toClient {
                do { diff = "h:" + hostFields(try HostMessage(parsing: instruction.diff)).replacingOccurrences(of: " ", with: "/") }
                catch { diff = "he:" + errorName(error) }
            } else {
                do { diff = "u:" + formatEvents(try UserMessage(parsing: instruction.diff).events) }
                catch { diff = "ue:" + errorName(error) }
            }
        }
        out.append("I:\(head):" + instructionFields(instruction).replacingOccurrences(of: " ", with: "/") + ":" + diff)
    }
    return out.joined(separator: ";")
}

func runWire(_ line: Substring) -> String? {
    if line.isEmpty || line.hasPrefix("#") { return nil }
    let tokens = line.split(separator: " ", omittingEmptySubsequences: false)
    let op = String(tokens[0])
    var fields: [Substring: Substring] = [:]
    for token in tokens.dropFirst() {
        guard let eq = token.firstIndex(of: "=") else { continue }
        fields[token[..<eq]] = token[token.index(after: eq)...]
    }
    func bytes(_ name: Substring) -> Data { unhex(fields[name] ?? "") }
    func number(_ name: Substring) -> UInt64 { UInt64(fields[name]!)! }
    func optional(_ name: Substring) -> Int? { fields[name]! == "-" ? nil : Int(fields[name]!)! }
    func dir(_ n: UInt64) -> MoshCrypto.Direction { n == 0 ? .toServer : .toClient }
    do {
        let result: String
        switch op {
        case "ocb.seal":
            result = "ct=" + hex(try OCB3(key: bytes("key")).seal(plaintext: bytes("pt"), nonce: bytes("nonce"), additionalData: bytes("ad")))
        case "ocb.open":
            result = "pt=" + hex(try OCB3(key: bytes("key")).open(ciphertext: bytes("ct"), nonce: bytes("nonce"), additionalData: bytes("ad")))
        case "crypto.seal":
            let message = MoshCrypto.Message(timestamp: UInt16(number("ts")), timestampReply: UInt16(number("tsr")), payload: bytes("payload"))
            result = "packet=" + hex(try MoshCrypto(key: bytes("key")).seal(message, seq: number("seq"), direction: dir(number("dir"))))
        case "crypto.open":
            let m = try MoshCrypto(key: bytes("key")).open(bytes("packet"))
            result = "seq=\(m.seq) dir=\(m.direction == .toServer ? 0 : 1) ts=\(m.timestamp) tsr=\(m.timestampReply) payload=\(hex(m.payload))"
        case "zlib.compress":
            let data = bytes("data")
            let out = [UInt8](try MoshCompression.compress(data))
            let roundtrip = (try? MoshCompression.decompress(Data(out))) == data
            result = "head=\(hex(Array(out[0..<2]))) tail=\(hex(Array(out[(out.count - 4)...]))) roundtrip=\(roundtrip ? 1 : 0) out=\(hex(out))"
        case "zlib.decompress":
            result = "data=" + hex(try MoshCompression.decompress(bytes("data")))
        case "adler":
            result = String(format: "%08x", MoshCompression.adler32(bytes("data")))
        case "inst.encode":
            let i = TransportInstruction(protocolVersion: UInt32(number("v")), oldNum: number("old"), newNum: number("new"),
                                         ackNum: number("ack"), throwawayNum: number("thr"), diff: bytes("diff"), chaff: bytes("chaff"))
            result = "bytes=" + hex(i.encoded())
        case "inst.decode":
            result = instructionFields(try TransportInstruction(parsing: bytes("data")))
        case "user.encode":
            result = "bytes=" + hex(UserMessage(events: parseEvents(fields["events"]!)).encoded())
        case "user.decode":
            result = "events=" + formatEvents(try UserMessage(parsing: bytes("data")).events)
        case "host.decode":
            result = hostFields(try HostMessage(parsing: bytes("data")))
        case "frag.encode":
            let piece = TransportFragment(id: number("id"), fragmentNum: UInt16(number("num")), isFinal: number("final") != 0, contents: bytes("contents"))
            result = "bytes=" + hex(piece.encoded())
        case "frag.parse":
            let p = try TransportFragment(parsing: bytes("data"))
            result = "id=\(p.id) num=\(p.fragmentNum) final=\(p.isFinal ? 1 : 0) contents=\(hex(p.contents))"
        case "frag.split":
            let pieces = TransportFragment.fragment(payload: bytes("payload"), id: number("id"), mtu: Int(number("mtu")))
            result = "frags=" + listOrDash(pieces.map { hex($0.encoded()) })
        case "frag.assemble":
            var assembler = FragmentAssembler()
            let out = splitList(fields["frags"]!).map { item -> String in
                do {
                    let piece = try TransportFragment(parsing: unhex(item))
                    return assembler.receive(piece).map { "p" + hex($0) } ?? "-"
                } catch { return "e:" + errorName(error) }
            }
            result = "out=" + out.joined(separator: ",")
        case "boot.parse":
            let creds = try MoshBootstrap.parse(output: String(decoding: bytes("output"), as: UTF8.self),
                                                host: String(decoding: bytes("host"), as: UTF8.self))
            result = "port=\(creds.udpPort) key=\(hex(creds.key)) pid=\(creds.serverPid.map { String($0) } ?? "-")"
        case "boot.command":
            let command = MoshBootstrap.command(serverBinary: String(decoding: bytes("bin"), as: UTF8.self),
                                                locale: String(decoding: bytes("locale"), as: UTF8.self),
                                                portRangeStart: optional("start"), portRangeEnd: optional("end"))
            result = hex(Data(command.utf8))
        case "boot.excerpt":
            let excerpt = MoshBootstrap.BootstrapError.sanitizedExcerpt(of: String(decoding: bytes("output"), as: UTF8.self))
            result = hex(Data(excerpt.utf8))
        case "pipe.send":
            let crypto = try MoshCrypto(key: bytes("key"))
            let pieces = TransportFragment.fragment(payload: bytes("zlib"), id: number("id"), mtu: Int(number("mtu")))
            var packets: [String] = []
            for (index, piece) in pieces.enumerated() {
                let message = MoshCrypto.Message(timestamp: UInt16(number("ts")), timestampReply: UInt16(number("tsr")), payload: piece.encoded())
                packets.append(hex(try crypto.seal(message, seq: number("seq") + UInt64(index), direction: .toServer)))
            }
            result = "packets=" + listOrDash(packets)
        case "pipe.recv":
            result = "out=" + receivePipeline(try MoshCrypto(key: bytes("key")), fields["packets"]!)
        default:
            return "\(op) err unknown-op:\(op)"
        }
        return "\(op) ok \(result)"
    } catch {
        return "\(op) err \(errorName(error))"
    }
}

func wire(_ args: [String]) {
    guard let input = args.first, let text = try? String(contentsOfFile: input, encoding: .utf8) else {
        fail("usage: wire <cases> [<out>]")
    }
    let out = Output(path: args.count > 1 ? args[1] : nil)
    for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
        if let result = runWire(line) { out.line(result) }
    }
    out.flush()
}

// MARK: - SSP scenarios (format: src/harness/ssp.rs)

final class FakeChannel: DatagramChannel, @unchecked Sendable {
    var state: NWConnection.State = .ready
    var onStateChange: (@Sendable (NWConnection.State) -> Void)?
    var onBetterPath: (@Sendable (Bool) -> Void)?
    var sent: [Data] = []
    func start() {}
    func restart() {}
    func cancel() {}
    func send(_ datagram: Data) { sent.append(datagram) }
    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {}
}

/// The scenario clock: 1,000,000 ms, moved only by `idle` and `tick` steps.
final class ScenarioClock: @unchecked Sendable {
    var now: UInt64 = 1_000_000
}

func ssp(_ args: [String]) async {
    guard let input = args.first, let text = try? String(contentsOfFile: input, encoding: .utf8) else {
        fail("usage: ssp <scenario> [<out>]")
    }
    let out = Output(path: args.count > 1 ? args[1] : nil)
    var transport: MoshTransport?
    let channel = FakeChannel()
    var observer: MoshCrypto?
    var assembler = FragmentAssembler()
    var seen = 0
    var closed = false
    let clock = ScenarioClock()
    var budget = MoshTransport.defaultInstructionBudget
    let lines = text.split(separator: "\n").filter { !$0.isEmpty }
    for (index, line) in lines.enumerated() {
        let parts = line.split(separator: " ", maxSplits: 1)
        let op = String(parts[0])
        let arg = parts.count > 1 ? parts[1] : ""
        if op == "budget" {
            budget = Int(arg)!
            continue
        }
        if op == "key" {
            let key = unhex(arg)
            // The clock stands still but for idle and tick steps, as the
            // Rust runner's does, and the TTL never runs out.
            transport = try! MoshTransport(credentials: MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key),
                                           channel: channel, keystrokeTTLMs: UInt64.max / 4,
                                           instructionBudget: budget, clock: { clock.now })
            observer = try! MoshCrypto(key: key)
            continue
        }
        guard let transport, let observer else { fail("key first") }
        out.line("step \(index) \(op)")
        let endedBefore = await transport.sessionEnded
        let numbers = { arg.split(separator: " ").map { Int($0)! } }
        switch op {
        case "start": let n = numbers(); await transport.start(cols: n[0], rows: n[1])
        case "send":
            let num = await transport.send(unhex(arg))
            out.line("num \(num.map { String($0) } ?? "-")")
        case "resize": let n = numbers(); await transport.resize(cols: n[0], rows: n[1])
        case "resume": await transport.resume()
        case "resume-force": await transport.resume(force: true)
        case "idle":
            clock.now += UInt64(arg)!
            await transport.resume()
        case "tick":
            clock.now += UInt64(arg)!
            await transport.tick()
        case "redraw": await transport.requestFullRedraw()
        case "recv": await transport.handleDatagram(unhex(arg))
        case "close": await transport.close(); closed = true
        default: fail("unknown step \(op)")
        }
        while seen < channel.sent.count {
            let message = try! observer.open(channel.sent[seen])
            seen += 1
            let piece = try! TransportFragment(parsing: message.payload)
            out.line("tx seq=\(message.seq) id=\(piece.id) num=\(piece.fragmentNum) final=\(piece.isFinal ? 1 : 0) ts=\(message.timestamp) tsr=\(message.timestampReply)")
            if let payload = assembler.receive(piece) {
                let i = try! TransportInstruction(parsing: try! MoshCompression.decompress(payload))
                let events = try! UserMessage(parsing: i.diff).events
                out.line("inst old=\(i.oldNum) new=\(i.newNum) ack=\(i.ackNum) thr=\(i.throwawayNum) events=\(formatEvents(events))")
            }
        }
        if !endedBefore, await transport.sessionEnded { out.line("ended") }
        let d = await transport.currentDiagnostics
        let srtt = await transport.currentSRTT
        out.line("diag received=\(d.datagramsReceived) parseFailures=\(d.parseFailures) gaps=\(d.gapEvents) applied=\(d.appliedHostNum) srtt=\(srtt.bitPattern)")
    }
    if let transport {
        if !closed { await transport.close() }
        for await frame in transport.hostStream {
            out.line("frame \(frame.echoAck) \(hex(frame.bytes))")
        }
    }
    out.flush()
}

// MARK: - Bench

/// Keystroke-like input: shell commands, repeated to `size` bytes.
func benchInput(_ size: Int) -> Data {
    let text = Array("git status && ls -la /usr/local/bin | head -20\r".utf8)
    return Data((0..<size).map { text[$0 % text.count] })
}

/// Terminal-like output: 80-column rows of printable bytes.
func benchOutput(_ size: Int) -> Data {
    Data((0..<size).map { i in i % 80 == 79 ? 0x0A : UInt8(0x20 + (i * 7 + i / 80) % 95) })
}

func bench(_ args: [String]) {
    guard args.count == 3, let size = Int(args[1]), let iterations = Int(args[2]) else {
        fail("usage: bench send|recv|seal <size> <iterations>")
    }
    let key = Data((0..<16).map { UInt8($0) })
    let crypto = try! MoshCrypto(key: key)
    var sink = 0
    let start = DispatchTime.now().uptimeNanoseconds
    switch args[0] {
    case "send":
        // The client's send path: encode, compress, fragment, seal.
        let diff = UserMessage(events: [.keystroke(keys: benchInput(size))]).encoded()
        var seq: UInt64 = 0
        for n in 0..<iterations {
            var instruction = TransportInstruction()
            instruction.oldNum = UInt64(n); instruction.newNum = UInt64(n + 1); instruction.ackNum = 7
            instruction.diff = diff
            let compressed = try! MoshCompression.compress(instruction.encoded())
            for piece in TransportFragment.fragment(payload: compressed, id: UInt64(n), mtu: 1300) {
                let message = MoshCrypto.Message(timestamp: 1, timestampReply: 2, payload: piece.encoded())
                sink &+= try! crypto.seal(message, seq: seq, direction: .toServer).count
                seq += 1
            }
        }
    case "recv":
        // The client's receive path for one server diff: open, reassemble,
        // inflate, decode the instruction and the host output.
        var host = ProtoWriter()
        host.appendField(4, bytes: benchOutput(size))
        var inner = ProtoWriter()
        inner.appendField(2, bytes: host.bytes)
        var message = ProtoWriter()
        message.appendField(1, bytes: inner.bytes)
        var instruction = TransportInstruction()
        instruction.oldNum = 1; instruction.newNum = 2; instruction.diff = message.data
        let compressed = try! MoshCompression.compress(instruction.encoded())
        let packets = TransportFragment.fragment(payload: compressed, id: 1, mtu: 1300).enumerated().map { index, piece in
            try! crypto.seal(MoshCrypto.Message(timestamp: 1, timestampReply: 2, payload: piece.encoded()),
                             seq: UInt64(index), direction: .toClient)
        }
        for _ in 0..<iterations {
            var assembler = FragmentAssembler()
            for packet in packets {
                let opened = try! crypto.open(packet)
                if let payload = assembler.receive(try! TransportFragment(parsing: opened.payload)) {
                    let i = try! TransportInstruction(parsing: try! MoshCompression.decompress(payload))
                    sink &+= try! HostMessage(parsing: i.diff).hostBytes.count
                }
            }
        }
    case "seal":
        let ocb = try! OCB3(key: key)
        let payload = benchInput(size)
        let nonce = Data(repeating: 0, count: 12)
        for _ in 0..<iterations { sink &+= try! ocb.seal(plaintext: payload, nonce: nonce).count }
    default:
        fail("bench send|recv|seal")
    }
    let elapsed = Double(DispatchTime.now().uptimeNanoseconds - start)
    let perOp = elapsed / Double(iterations)
    let mbps = Double(size) / perOp * 1000
    print(String(format: "bench %@ size=%d iterations=%d ns_per_op=%.0f mb_per_s=%.1f sink=%d",
                 args[0], size, iterations, perOp, mbps, sink & 1))
}

// MARK: - Entry

@main
struct MoshSwiftHarness {
    static func main() async {
        let args = Array(CommandLine.arguments.dropFirst())
        guard let mode = args.first else { fail("usage: mosh-swift wire|ssp|bench|live ...") }
        let rest = Array(args.dropFirst())
        switch mode {
        case "live": await live(rest)
        case "wire": wire(rest)
        case "ssp": await ssp(rest)
        case "bench": bench(rest)
        default: fail("unknown mode \(mode)")
        }
    }
}
