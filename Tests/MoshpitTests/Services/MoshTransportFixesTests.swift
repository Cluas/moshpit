import Foundation
import Network
import Testing
@testable import Moshpit

/// Monotonic milliseconds a test steps by hand, for `MoshTransport`'s
/// `clock`.
final class ManualMoshClock: @unchecked Sendable {
    private let lock = NSLock()
    private var ms: UInt64

    init(_ ms: UInt64 = 0) { self.ms = ms }

    var now: UInt64 {
        get { lock.withLock { ms } }
        set { lock.withLock { ms = newValue } }
    }

    var read: @Sendable () -> UInt64 { { self.now } }
}

/// SplitMix64, the same generator as the Rust harness's.
struct SplitMix64 {
    var state: UInt64

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }

    /// Uniform in `0..<bound`.
    mutating func below(_ bound: UInt64) -> UInt64 { next() % bound }
}

/// Regression tests for the mosh client bugs the Rust port of this client
/// (crates/mosh, `harness/divergences.json`) found, each against a model of
/// what mosh-server 1.4.0 does with the client's datagrams. The same bugs
/// are shown against a real mosh-server by `crates/mosh/harness/live.py`.
@Suite("MoshTransport fixes")
struct MoshTransportFixesTests {

    // MARK: Fixtures

    let key = Data(repeating: 3, count: 16)

    /// Records what the transport sends; never delivers anything by itself.
    final class Channel: DatagramChannel, @unchecked Sendable {
        var state: NWConnection.State = .ready
        var onStateChange: (@Sendable (NWConnection.State) -> Void)?
        var onBetterPath: (@Sendable (Bool) -> Void)?
        private(set) var sent: [Data] = []
        private(set) var cancelled = false
        func start() {}
        func restart() {}
        func cancel() { cancelled = true }
        func send(_ datagram: Data) { sent.append(datagram) }
        func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {}
    }

    /// Reads the client's datagrams the way mosh-server does.
    struct ServerSide {
        let crypto: MoshCrypto
        var assembler = FragmentAssembler()
        var read = 0

        /// Every instruction the client sent since the last call.
        mutating func instructions(from channel: Channel) throws -> [TransportInstruction] {
            var out: [TransportInstruction] = []
            while read < channel.sent.count {
                let message = try crypto.open(channel.sent[read])
                read += 1
                if let payload = assembler.receive(try TransportFragment(parsing: message.payload)) {
                    out.append(try TransportInstruction(parsing: MoshCompression.decompress(payload)))
                }
            }
            return out
        }
    }

    func makeTransport(clock: ManualMoshClock? = nil,
                       instructionBudget: Int = MoshTransport.defaultInstructionBudget)
        throws -> (MoshTransport, Channel, ServerSide) {
        let channel = Channel()
        let transport = try MoshTransport(
            credentials: MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key),
            channel: channel,
            instructionBudget: instructionBudget,
            clock: clock?.read ?? MoshTransport.monotonicMs)
        return (transport, channel, ServerSide(crypto: try MoshCrypto(key: key)))
    }

    /// A server→client datagram as mosh-server encodes it: protobuf, zlib,
    /// one fragment, sealed. 0xFFFF is mosh's "no timestamp".
    func serverDatagram(seq: UInt64, oldNum: UInt64 = 0, newNum: UInt64 = 0, ackNum: UInt64,
                        diff: Data = Data(), timestamp: UInt16 = 0xFFFF,
                        timestampReply: UInt16 = 0xFFFF) throws -> Data {
        var instruction = TransportInstruction()
        instruction.oldNum = oldNum
        instruction.newNum = newNum
        instruction.ackNum = ackNum
        instruction.throwawayNum = oldNum
        instruction.diff = diff
        let compressed = try MoshCompression.compress(instruction.encoded())
        let fragments = TransportFragment.fragment(payload: compressed, id: seq + 1, mtu: 1300)
        #expect(fragments.count == 1)
        return try MoshCrypto(key: key).seal(
            MoshCrypto.Message(timestamp: timestamp, timestampReply: timestampReply,
                               payload: fragments[0].encoded()),
            seq: seq, direction: .toClient)
    }

    // MARK: mosh-server's receiver

    /// mosh-server's view of the client's user stream: `Transport::recv` in
    /// networktransport-impl.h and `UserStream` in user.cc, each keystroke
    /// byte one event as in `UserStream::apply_string`. A newest state that
    /// does not extend the one processed before it is a desync: release
    /// builds misalign or read past the end of the shorter state (a SIGSEGV
    /// in `UserStream::diff_from`), builds with asserts abort.
    final class ServerModel {
        enum Event: Equatable {
            case byte(UInt8)
            case resize(Int, Int)
        }

        private(set) var states: [UInt64: [Event]] = [0: []]
        private var lastProcessed: [Event] = []
        private(set) var desyncs = 0
        /// Every keystroke byte handed to the remote shell, with when.
        private(set) var delivered: [(byte: UInt8, atMs: UInt64)] = []
        private var quenchUntil: UInt64 = 0
        /// States refused because the queue held more than 1024.
        private(set) var quenched = 0
        /// Times a throwaway number left no state at all, where
        /// mosh-server's `fatal_assert` aborts.
        private(set) var emptied = 0

        /// What the server acks: the newest state it holds.
        var ack: UInt64 { states.keys.max() ?? 0 }

        func receive(_ instruction: TransportInstruction, atMs now: UInt64) throws {
            guard states[instruction.newNum] == nil,
                  let reference = states[instruction.oldNum] else { return }
            states = states.filter { $0.key >= instruction.throwawayNum }
            if states.isEmpty { emptied += 1; return }
            if states.count > 1024 {
                if now < quenchUntil { quenched += 1; return }
                quenchUntil = now + 15_000
            }
            var state = reference
            for event in try UserMessage(parsing: instruction.diff).events {
                switch event {
                case .keystroke(let keys): state += keys.map { .byte($0) }
                case .resize(let width, let height): state.append(.resize(width, height))
                }
            }
            let newest = instruction.newNum > ack
            states[instruction.newNum] = state
            guard newest else { return }
            if state.starts(with: lastProcessed) {
                for case .byte(let byte) in state[lastProcessed.count...] {
                    delivered.append((byte, now))
                }
            } else {
                desyncs += 1
            }
            lastProcessed = state
        }
    }

    // MARK: B46 — throwaway_num

    @Test("every instruction lets the server throw away user states below the acked one")
    func throwawayNamesTheAckedUserState() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)          // state 1: the size
        await transport.resume()                          // .ready's first datagram
        _ = try reader.instructions(from: channel)
        var seq: UInt64 = 0
        // A frame apart (250 ms before an RTT sample), each key goes at once.
        for key in UInt8(ascii: "a")...UInt8(ascii: "e") {
            clock.now += 300
            let num = try #require(await transport.send(Data([key])))
            #expect(try reader.instructions(from: channel).count == 1)
            await transport.handleDatagram(try serverDatagram(seq: seq, ackNum: num))
            seq += 1
        }
        clock.now += 300
        await transport.send(Data("z".utf8))
        let last = try #require(try reader.instructions(from: channel).last)
        #expect(last.oldNum == 6)
        #expect(last.throwawayNum == last.oldNum,
                "throwaway_num 0 makes mosh-server keep every user state of the session")
    }

    @Test("typing 2000 keys never fills mosh-server's 1024-state queue")
    func theServerQueueStaysSmall() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        let model = ServerModel()
        await transport.start(cols: 80, rows: 24)
        var typed: [UInt8] = []
        // A frame apart (250 ms before an RTT sample): each key goes alone.
        for index in 0..<2000 {
            let now = UInt64(index) * 250
            clock.now = now
            let byte = UInt8(truncatingIfNeeded: index)
            typed.append(byte)
            await transport.send(Data([byte]))
            // The server's answer acks the newest state it holds, a step
            // behind the input.
            await transport.handleDatagram(try serverDatagram(seq: UInt64(index), ackNum: model.ack))
            for instruction in try reader.instructions(from: channel) {
                try model.receive(instruction, atMs: now)
            }
        }
        #expect(model.quenched == 0,
                "past 1024 held states mosh-server takes one new state per 15 s: typing lags 15 s")
        #expect(model.states.count <= 3, "the server holds \(model.states.count) user states")
        #expect(model.emptied == 0)
        #expect(model.desyncs == 0)
        #expect(model.delivered.map(\.byte) == typed)
        let late = model.delivered.enumerated().filter { $0.element.atMs != UInt64($0.offset) * 250 }
        #expect(late.isEmpty, "\(late.count) keys reached the shell after they were typed")
    }

    // MARK: B47 — expired keystrokes the server may hold

    /// The trigger: the server got the keystrokes, its acks were lost for
    /// longer than the TTL, and the user typed again. Stripping them from
    /// states already sent made the next state shorter than the one the
    /// server processed — mosh-server crashed in `UserStream::diff_from`.
    @Test("an expired keystroke the server may hold pauses input until the server answers, and nothing is lost")
    func heldInputResumesOnAnAck() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        let model = ServerModel()
        await transport.start(cols: 80, rows: 24)
        for (index, key) in "abcde".utf8.enumerated() {
            clock.now = 1 + UInt64(index)
            await transport.send(Data([key]))
            for instruction in try reader.instructions(from: channel) {
                try model.receive(instruction, atMs: clock.now)
            }
        }
        // Every ack is lost for 11 s; the keys go a frame later, then again
        // each retransmission timeout while within 10 s of the first
        // datagram, all arriving (the send timer runs every second).
        for second: UInt64 in 1...11 {
            clock.now = second * 1_000
            await transport.tick()
            for instruction in try reader.instructions(from: channel) {
                try model.receive(instruction, atMs: clock.now)
            }
        }

        // The keys are past their TTL: the next datagram holds input back.
        clock.now = 11_500
        await transport.send(Data("x".utf8))
        #expect(await transport.currentDiagnostics.inputHeld)
        let held = try reader.instructions(from: channel)
        #expect(!held.isEmpty)
        #expect(held.allSatisfy { $0.newNum == $0.oldNum && $0.diff.isEmpty },
                "no user state past the acked one may go out while it is held")
        for instruction in held { try model.receive(instruction, atMs: clock.now) }

        // The return path heals: the server's next datagram acks what it got.
        clock.now = 12_000
        await transport.handleDatagram(try serverDatagram(seq: 0, ackNum: model.ack))
        #expect(await transport.currentDiagnostics.inputHeld == false)
        for instruction in try reader.instructions(from: channel) {
            try model.receive(instruction, atMs: clock.now)
        }
        #expect(model.desyncs == 0, "a desynced mosh-server crashes and takes the remote shell with it")
        #expect(model.delivered.map(\.byte) == Array("abcdex".utf8))
    }

    @Test("held keystrokes are dropped only once an answer comes after the proof delay")
    func heldInputDropsExpiredKeysOnlyAfterTheProofDelay() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        clock.now = 100
        await transport.send(Data("lost".utf8))          // never arrives
        _ = try reader.instructions(from: channel)
        clock.now = 11_000
        await transport.resume()                          // expired: held from 11 s
        _ = try reader.instructions(from: channel)
        #expect(await transport.currentDiagnostics.inputHeld)
        clock.now = 11_100
        await transport.send(Data("new".utf8))            // queued, not sent while held
        #expect(try reader.instructions(from: channel).allSatisfy { $0.diff.isEmpty })

        // An answer 1 s into the hold proves nothing yet.
        clock.now = 12_000
        await transport.handleDatagram(try serverDatagram(seq: 0, oldNum: 0, newNum: 1, ackNum: 0))
        #expect(await transport.currentDiagnostics.inputHeld)
        #expect(try reader.instructions(from: channel).allSatisfy { $0.newNum == $0.oldNum })

        // One 5 s in does: "lost" goes, the resize and the fresh "new" stay.
        clock.now = 16_000
        await transport.handleDatagram(try serverDatagram(seq: 1, oldNum: 1, newNum: 2, ackNum: 0))
        #expect(await transport.currentDiagnostics.inputHeld == false)
        let last = try #require(try reader.instructions(from: channel).last)
        #expect(last.oldNum == 0)
        #expect(last.newNum == 3)
        #expect(try UserMessage(parsing: last.diff).events
                == [.resize(width: 80, height: 24), .keystroke(keys: Data("new".utf8))])
    }

    @Test("keystrokes that expire before they were ever sent are dropped without a hold")
    func unsentKeystrokesExpireWithoutAHold() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        await transport.send(Data("old".utf8))
        clock.now = 10_500
        await transport.resume()                          // held from here
        clock.now = 10_600
        await transport.send(Data("queued".utf8))
        _ = try reader.instructions(from: channel)
        // The server answers only at 31 s; by then "queued" expired unsent.
        clock.now = 31_000
        await transport.handleDatagram(try serverDatagram(seq: 0, oldNum: 0, newNum: 1, ackNum: 1))
        let last = try #require(try reader.instructions(from: channel).last)
        #expect(last.oldNum == 1)
        #expect(last.newNum == 3)
        #expect(last.diff.isEmpty, "both keystrokes expired")
    }

    /// Random loss both ways, random typing, random pauses: mosh-server never
    /// desyncs, never runs a keystroke twice, never runs one older than the
    /// TTL, and once the link heals everything is acked.
    @Test("random loss never desyncs mosh-server nor runs a keystroke twice or late",
          arguments: 1...16 as ClosedRange<UInt64>)
    func theServerNeverDesyncsUnderRandomLoss(seed: UInt64) async throws {
        var rng = SplitMix64(state: seed)
        let clock = ManualMoshClock(1_000)
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        let model = ServerModel()
        var typedAt: [UInt8: UInt64] = [:]
        var nextKey: UInt8 = 0
        var lastTyped: UInt64 = 0
        var seq: UInt64 = 0
        var upOK = true, downOK = true
        await transport.start(cols: 80, rows: 24)
        for _ in 0..<400 {
            clock.now += 50 + rng.below(1_500)
            if rng.below(8) == 0 {
                upOK = rng.below(3) != 0
                downOK = rng.below(3) != 0
            }
            if rng.below(3) == 0, nextKey < 250 {
                typedAt[nextKey] = clock.now
                lastTyped = try #require(await transport.send(Data([nextKey])))
                nextKey += 1
            }
            await transport.tick()                        // the send timer
            if downOK, rng.below(2) == 0 {
                await transport.handleDatagram(try serverDatagram(seq: seq, ackNum: model.ack))
                seq += 1
            }
            // Delivered when sent: the age checked below is the age at
            // transmission, which is what ghost input is about.
            for instruction in try reader.instructions(from: channel) {
                if upOK, rng.below(10) != 0 { try model.receive(instruction, atMs: clock.now) }
            }
        }
        // Heal both ways and let it settle.
        for _ in 0..<40 {
            clock.now += 500
            await transport.tick()
            await transport.handleDatagram(try serverDatagram(seq: seq, ackNum: model.ack))
            seq += 1
            for instruction in try reader.instructions(from: channel) {
                try model.receive(instruction, atMs: clock.now)
            }
        }
        #expect(model.desyncs == 0, "seed \(seed)")
        #expect(model.emptied == 0, "seed \(seed)")
        var seen = Set<UInt8>()
        for (byte, at) in model.delivered {
            #expect(seen.insert(byte).inserted, "seed \(seed): key \(byte) ran twice")
            let age = at - (typedAt[byte] ?? at)
            #expect(age <= MoshTransport.defaultKeystrokeTTLMs, "seed \(seed): key \(byte) ran \(age) ms after it was typed")
        }
        let diagnostics = await transport.currentDiagnostics
        #expect(!diagnostics.inputHeld, "seed \(seed)")
        // Every typed state is acked once the link heals; past it at most the
        // idle state the last 3 s ack added is in flight.
        #expect(diagnostics.serverAckedUserNum >= lastTyped, "seed \(seed)")
        #expect(diagnostics.lastUserNum - diagnostics.serverAckedUserNum <= 1,
                "seed \(seed): everything is acked once the link heals")
    }

    // MARK: B66 — an idle client still sends mosh-server new user states

    /// When mosh-server last heard from its client: the newest user state it
    /// received (mosh-server.cc, `time_since_remote_state`).
    /// MOSH_SERVER_NETWORK_TMOUT reaps the session once that is older than
    /// the timeout, and lost frames are retried at frame rate only while it
    /// is under 10 s.
    struct ServerClock {
        var newest: UInt64 = 0
        var newestAt: UInt64 = 0
        var longestSilence: UInt64 = 0

        mutating func receive(_ instruction: TransportInstruction, atMs now: UInt64) {
            guard instruction.newNum > newest else { return }
            longestSilence = max(longestSilence, now - newestAt)
            newest = instruction.newNum
            newestAt = now
        }
    }

    @Test("an idle client gives mosh-server a new, empty user state every 3 s")
    func anIdleClientGivesTheServerANewStateEveryThreeSeconds() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        var heard = ServerClock()
        func record() throws {
            for instruction in try reader.instructions(from: channel) {
                #expect(instruction.throwawayNum == instruction.oldNum)
                // Idle states add no events: at most the size, until it is acked.
                let events = try UserMessage(parsing: instruction.diff).events
                #expect(events.isEmpty || events == [.resize(width: 80, height: 24)])
                heard.receive(instruction, atMs: clock.now)
            }
        }
        await transport.start(cols: 80, rows: 24)
        await transport.resume()                          // .ready's first datagram
        try record()
        for second: UInt64 in 1...60 {
            clock.now = second * 1_000
            await transport.tick()                        // the send timer
            try record()
            // The server acks the newest state it holds.
            clock.now += 1
            await transport.handleDatagram(try serverDatagram(seq: second, ackNum: heard.newest))
            try record()
        }
        #expect(heard.longestSilence <= 3_000,
                "with TMOUT=8 mosh-server reaped the session at 8 s: \(heard.longestSilence) ms without a new state")
        #expect(heard.newest >= 20)
        #expect(clock.now - heard.newestAt <= 3_000)
    }

    @Test("no idle state while typing, nor while input is held")
    func noIdleStateWhileTypingOrHeld() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        for step: UInt64 in 1...20 {
            clock.now = step * 500
            #expect(await transport.send(Data("k".utf8)) == step + 1, "every state is a keystroke")
            await transport.resume()
        }
        _ = try reader.instructions(from: channel)
        // Nothing acked since: the keystrokes expire at 10.5 s and input is
        // held, where an idle state's diff would carry them past their TTL.
        for second: UInt64 in 11...30 {
            clock.now = second * 1_000
            await transport.resume()
        }
        let diagnostics = await transport.currentDiagnostics
        #expect(diagnostics.inputHeld)
        #expect(diagnostics.lastUserNum == 21)
        #expect(try reader.instructions(from: channel).allSatisfy { $0.newNum == $0.oldNum && $0.diff.isEmpty })
    }

    /// Raised by a waiter task, read by the test.
    actor Flag {
        private(set) var raised = false
        func raise() { raised = true }
    }

    @Test("waitUntilDelivered waits for input, not for an idle state")
    func deliveryIgnoresIdleStates() async throws {
        let clock = ManualMoshClock()
        let (transport, _, _) = try makeTransport(clock: clock)
        await transport.start(cols: 80, rows: 24)
        let typed = try #require(await transport.send(Data("a".utf8)))
        // The key goes out a frame after the size; a server acks only what
        // it was sent (B105).
        clock.now = 300
        await transport.tick()
        await transport.handleDatagram(try serverDatagram(seq: 0, ackNum: typed))
        clock.now = 4_000
        await transport.resume()                          // adds an idle state
        #expect(await transport.currentDiagnostics.lastUserNum == typed + 1)
        let delivered = Flag()
        let waiter = Task {
            await transport.waitUntilDelivered()
            await delivered.raise()
        }
        // The idle state is acked whenever mosh-server next speaks, up to 3 s
        // when it has nothing to say: a herdr key must not wait for that.
        try await Task.sleep(for: .milliseconds(100))
        let raised = await delivered.raised
        await transport.close()                           // never strand the waiter
        await waiter.value
        #expect(raised)
    }

    // MARK: B69 — input past mosh-server's 4 MiB inflate buffer

    /// mosh-server's side of a long paste: it inflates each instruction into
    /// its 4 MiB buffer and drops one that does not fit (a "Crypto exception",
    /// never acked: compressor.cc, `dos_assert`), applies the rest by
    /// `Transport::recv`'s rules, and hands the newest state's new keystroke
    /// bytes to the shell.
    final class ByteServer {
        let crypto: MoshCrypto
        var assembler = FragmentAssembler()
        var read = 0
        /// State number → keystroke bytes since state 0.
        private(set) var states: [UInt64: Data] = [0: Data()]
        private(set) var shell = Data()
        private(set) var refused = 0
        private(set) var mostHeld = 1
        private(set) var largestInstruction = 0

        init(crypto: MoshCrypto) { self.crypto = crypto }

        var ack: UInt64 { states.keys.max() ?? 0 }

        func receive(from channel: Channel) throws {
            while read < channel.sent.count {
                let message = try crypto.open(channel.sent[read])
                read += 1
                guard let payload = assembler.receive(try TransportFragment(parsing: message.payload)) else { continue }
                let bytes: Data
                do {
                    bytes = try MoshCompression.decompress(payload)
                } catch MoshCompression.CompressionError.tooLarge {
                    refused += 1
                    continue
                }
                largestInstruction = max(largestInstruction, bytes.count)
                let instruction = try TransportInstruction(parsing: bytes)
                #expect(instruction.throwawayNum == instruction.oldNum)
                guard states[instruction.newNum] == nil, var state = states[instruction.oldNum] else { continue }
                states = states.filter { $0.key >= instruction.throwawayNum }
                for case .keystroke(let keys) in try UserMessage(parsing: instruction.diff).events {
                    state.append(keys)
                }
                if instruction.newNum > ack {
                    #expect(state.prefix(shell.count) == shell, "desync")
                    shell = state
                }
                states[instruction.newNum] = state
                mostHeld = max(mostHeld, states.count)
            }
        }
    }

    /// Printable bytes, as a paste would be, and not very compressible.
    func pasted(_ count: Int, seed: UInt64) -> Data {
        var rng = SplitMix64(state: seed)
        return Data((0..<count).map { _ in UInt8(0x21 + rng.below(94)) })
    }

    @Test("a paste past mosh-server's 4 MiB buffer arrives whole and in order, in chained instructions")
    func aPastePastTheServersBufferArrivesWhole() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, reader) = try makeTransport(clock: clock)
        let server = ByteServer(crypto: reader.crypto)
        await transport.start(cols: 80, rows: 24)
        await transport.resume()
        try server.receive(from: channel)
        let paste = pasted(4 * 1024 * 1024 + 1_000, seed: 7)
        clock.now = 300                                   // a frame after the size: at once
        let last = try #require(await transport.send(paste))
        try server.receive(from: channel)
        #expect(server.refused == 0, "one instruction for the paste: mosh-server refuses it on every flush")
        #expect(server.ack == last)
        #expect(server.shell == paste)
        // 0, the size and the first run's end; from then on two at a time
        // (each instruction lets the one before it go), not the whole paste.
        #expect(server.mostHeld <= 3, "\(server.mostHeld) states held")
        #expect(server.largestInstruction <= MoshTransport.defaultInstructionBudget + 64)
        // The server's ack ends it: nothing goes again, and the next datagram
        // is a plain ack.
        clock.now = 400
        await transport.handleDatagram(try serverDatagram(seq: 0, ackNum: server.ack))
        #expect(channel.sent.count == server.read)
        await transport.resume()
        var after = reader
        after.read = channel.sent.count - 1
        let plain = try #require(try after.instructions(from: channel).last)
        #expect(plain.oldNum == last && plain.newNum == last)
    }

    @Test("input splits into states that fit; each datagram chains instructions under the budget from a state the server holds")
    func instructionsStayUnderTheBudgetAndChain() async throws {
        let budget = 100
        let clock = ManualMoshClock()
        let channel = Channel()
        // Minutes pass with states unacked; nothing here is about expiry.
        let transport = try MoshTransport(
            credentials: MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: key),
            channel: channel, keystrokeTTLMs: UInt64.max / 4, instructionBudget: budget,
            clock: clock.read)
        var reader = ServerSide(crypto: try MoshCrypto(key: key))
        var rng = SplitMix64(state: 3)
        let chunk = MoshTransport.keystrokeChunk(budget)
        await transport.start(cols: 80, rows: 24)
        // Every user state's events, by number.
        var eventsOf: [UInt64: [UserEvent]] = [1: [.resize(width: 80, height: 24)]]
        var acked: UInt64 = 0, newestSent: UInt64 = 0
        var size = (80, 24)
        for step: UInt64 in 0..<200 {
            // A frame or more apart (250 ms before an RTT sample): input goes
            // at once.
            clock.now += 250 + rng.below(1_000)
            let before = await transport.currentDiagnostics.lastUserNum
            if rng.below(5) == 0 {
                let cols = 60 + Int(rng.below(100)), rows = 20 + Int(rng.below(30))
                guard (cols, rows) != size else { continue }     // no change, nothing sent
                size = (cols, rows)
                await transport.resize(cols: cols, rows: rows)
                eventsOf[before + 1] = [.resize(width: cols, height: rows)]
            } else {
                let keys = pasted(1 + Int(rng.below(400)), seed: step)
                await transport.send(keys)
                var start = 0, num = before
                while start < keys.count {
                    let end = min(start + chunk, keys.count)
                    num += 1
                    eventsOf[num] = [.keystroke(keys: keys.subdata(in: start..<end))]
                    start = end
                }
            }
            let newest = await transport.currentDiagnostics.lastUserNum
            #expect(newest == eventsOf.keys.max())
            let flush = try reader.instructions(from: channel)
            let from = try #require(flush.first, "step \(step): input a frame later goes at once").oldNum
            #expect(from == acked || from == newestSent,
                    "step \(step): from \(from), acked \(acked), newest sent \(newestSent)")
            var old = from
            var events: [UserEvent] = []
            for instruction in flush {
                #expect(instruction.oldNum == old)
                #expect(instruction.throwawayNum == old)
                let these = try UserMessage(parsing: instruction.diff).events
                #expect(instruction.diff.count <= budget || these.count == 1, "\(instruction.diff.count) bytes")
                events += these
                old = instruction.newNum
            }
            #expect(old == newest)
            let expected = eventsOf.keys.sorted().filter { $0 > from }.flatMap { eventsOf[$0] ?? [] }
            #expect(events == expected, "step \(step)")
            newestSent = old
            // Now and then the server acks a state it was sent.
            if rng.below(4) == 0 {
                let to = flush[Int(rng.below(UInt64(flush.count)))].newNum
                await transport.handleDatagram(try serverDatagram(seq: step, ackNum: to))
                #expect(try reader.instructions(from: channel).isEmpty, "an ack sends nothing")
                if to > acked {
                    acked = to
                    eventsOf = eventsOf.filter { $0.key > acked }
                }
            }
        }
    }

    // MARK: B91 — mosh's sending discipline

    // swiftlint:disable large_tuple
    /// A transport on a hand-stepped clock whose channel became ready at 0,
    /// with that first datagram read.
    func readyTransport(clock: ManualMoshClock) async throws -> (MoshTransport, Channel, ServerSide) {
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        await transport.resume()                          // .ready's first datagram
        _ = try reader.instructions(from: channel)
        return (transport, channel, reader)
    }
    // swiftlint:enable large_tuple

    /// A frame of `text` from the server: state `oldNum` → `oldNum + 1`,
    /// acking user state `ackNum`.
    func output(seq: UInt64, oldNum: UInt64, ackNum: UInt64, _ text: String,
                reply: UInt16 = 0xFFFF) throws -> Data {
        try serverDatagram(seq: seq, oldNum: oldNum, newNum: oldNum + 1, ackNum: ackNum,
                           diff: hostDiff([.bytes(text)]), timestampReply: reply)
    }

    func carries(_ instruction: TransportInstruction, _ keys: String) throws -> Bool {
        let wanted = Data(keys.utf8)
        return try UserMessage(parsing: instruction.diff).events.contains { event in
            if case .keystroke(let typed) = event { return typed.range(of: wanted) != nil }
            return false
        }
    }

    /// Every frame used to flush every unacked state, so a key in flight
    /// went once per frame of output until its ack came, and every frame
    /// got an ack of its own.
    @Test("output streaming in neither resends the key in flight nor gets an ack per frame")
    func outputDoesNotResendInputInFlight() async throws {
        let clock = ManualMoshClock()
        var (transport, channel, reader) = try await readyTransport(clock: clock)
        clock.now = 50
        await transport.handleDatagram(try serverDatagram(seq: 0, ackNum: 1))
        clock.now = 300
        await transport.send(Data("k".utf8))
        let first = try reader.instructions(from: channel)
        #expect(first.count == 1)
        #expect(try carries(#require(first.first), "k"))
        // 50 frames 10 ms apart; mosh-server acks the key 100 ms after it.
        var after: [TransportInstruction] = []
        for index: UInt64 in 1...50 {
            clock.now = 300 + 10 * index
            let ack: UInt64 = clock.now >= 400 ? 2 : 1
            await transport.handleDatagram(try output(seq: index, oldNum: index - 1, ackNum: ack, "x"))
            await transport.tick()                        // the send timer
            after += try reader.instructions(from: channel)
        }
        #expect(try after.allSatisfy { try !carries($0, "k") }, "the key went again")
        // One ack ACK_DELAY after the first frame it acks: at 410, 520, 630, 740.
        #expect(after.map(\.ackNum) == [11, 22, 33, 44])
    }

    @Test("unacked input goes again only once its retransmission timeout ran out")
    func unackedInputGoesAgainOnlyAfterTheTimeout() async throws {
        let clock = ManualMoshClock()
        var (transport, channel, reader) = try await readyTransport(clock: clock)
        // One 100 ms sample: SRTT 100, RTTVAR 50, so 300 ms plus ACK_DELAY.
        clock.now = 150
        await transport.handleDatagram(try serverDatagram(
            seq: 0, ackNum: 1, timestampReply: MoshTransport.timestamp16(150) - 100))
        #expect(await transport.currentSRTT == 100)
        clock.now = 1_000
        await transport.send(Data("k".utf8))
        #expect(try reader.instructions(from: channel).count == 1)
        var copies: [UInt64] = []
        for now: UInt64 in 1_001...2_000 {
            clock.now = now
            await transport.tick()
            for instruction in try reader.instructions(from: channel) {
                #expect(try carries(instruction, "k"))
                copies.append(now)
            }
        }
        #expect(copies == [1_400, 1_800])
    }

    @Test("an idle client whose server acks it sends one datagram every 3 s")
    func anIdleClientSendsADatagramEveryThreeSeconds() async throws {
        let clock = ManualMoshClock()
        var (transport, channel, reader) = try await readyTransport(clock: clock)
        var sends: [UInt64] = []
        var acked: UInt64 = 1
        for now in stride(from: UInt64(100), through: 30_000, by: 100) {
            clock.now = now
            await transport.tick()
            for instruction in try reader.instructions(from: channel) {
                acked = instruction.newNum
                sends.append(now)
            }
            // The server acks the newest user state it was sent.
            clock.now = now + 10
            await transport.handleDatagram(try serverDatagram(seq: now, ackNum: acked))
        }
        #expect(sends == (1...10).map { UInt64($0) * 3_000 })
    }

    /// mosh's prophylactic resend: a few bytes in flight go again with new
    /// input (cheaper than doubting them), a long input in flight does not.
    @Test("new input resends a few bytes in flight, but not a long input")
    func newInputResendsALittleInFlightButNotALot() async throws {
        let clock = ManualMoshClock()
        var (transport, channel, reader) = try await readyTransport(clock: clock)
        clock.now = 50
        await transport.handleDatagram(try serverDatagram(seq: 0, ackNum: 1))
        clock.now = 300
        await transport.send(Data("ab".utf8))
        _ = try reader.instructions(from: channel)
        clock.now = 600
        await transport.send(Data("c".utf8))
        let small = try #require(try reader.instructions(from: channel).first)
        #expect(small.oldNum == 1 && small.newNum == 3)
        #expect(try carries(small, "ab") && carries(small, "c"))

        clock.now = 900
        let big = try #require(await transport.send(pasted(1_100, seed: 1)))
        _ = try reader.instructions(from: channel)
        clock.now = 1_200
        await transport.send(Data("d".utf8))
        let next = try reader.instructions(from: channel)
        #expect(next.count == 1)
        #expect(next.first?.oldNum == big && next.first?.newNum == big + 1)
        #expect(try UserMessage(parsing: #require(next.first).diff).events == [.keystroke(keys: Data("d".utf8))])
    }

    /// A 1 MiB paste while the server works through it, acking a state every
    /// 50 ms and printing as it goes. Every frame used to send everything
    /// still unacked again.
    @Test("a long paste goes once while the server acks it")
    func aLongPasteGoesOnceWhileTheServerAcksIt() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, reader) = try makeTransport(clock: clock)
        let server = ByteServer(crypto: reader.crypto)
        await transport.start(cols: 80, rows: 24)
        await transport.resume()                          // .ready's first datagram
        try server.receive(from: channel)
        clock.now = 50
        await transport.handleDatagram(try serverDatagram(seq: 0, ackNum: 1))
        let paste = pasted(1024 * 1024, seed: 5)
        clock.now = 300
        let last = try #require(await transport.send(paste))
        let burst = channel.sent[server.read...].reduce(0) { $0 + $1.count }
        try server.receive(from: channel)
        #expect(server.ack == last)
        #expect(server.shell == paste)
        // The server acks the next state every 50 ms, with a frame of output.
        var later = 0
        var acked: UInt64 = 1
        var index: UInt64 = 0
        while acked < last {
            index += 1
            acked += 1
            clock.now = 300 + 50 * index
            let sentBefore = channel.sent.count
            await transport.handleDatagram(try output(seq: index, oldNum: index - 1, ackNum: acked, "x"))
            await transport.tick()
            later += channel.sent[sentBefore...].reduce(0) { $0 + $1.count }
        }
        #expect(later < 4 * 1_024, "\(later) more bytes after a \(burst)-byte burst: input went again")
    }

    // MARK: B92 — the server's shutdown state ends the session

    /// Counts `onSessionEnded` calls.
    @MainActor final class EndedBox { var fired = 0 }

    /// The client's instructions in the datagrams from index `from` on.
    func instructions(_ channel: Channel, from: Int) throws -> [TransportInstruction] {
        var reader = ServerSide(crypto: try MoshCrypto(key: key))
        reader.read = from
        return try reader.instructions(from: channel)
    }

    /// A frame from state `oldNum` to `newNum` with `text` as output (none if
    /// nil), acking user state `ackNum`.
    func datagram(seq: UInt64, oldNum: UInt64, newNum: UInt64, ackNum: UInt64, text: String?) throws -> Data {
        try serverDatagram(seq: seq, oldNum: oldNum, newNum: newNum, ackNum: ackNum,
                           diff: text.map { hostDiff([.bytes($0)]) } ?? Data())
    }

    @Test("the shutdown state shows the last output, is acked a frame later, and ends the session")
    func theShutdownStateEndsTheSession() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, _) = try makeTransport(clock: clock)
        let box = await EndedBox()
        await MainActor.run { transport.onSessionEnded = { box.fired += 1 } }
        await transport.start(cols: 80, rows: 24)
        await transport.resume()                          // .ready's first datagram
        clock.now = 100
        await transport.handleDatagram(try datagram(seq: 0, oldNum: 0, newNum: 1, ackNum: 1, text: "$ exit"))
        clock.now = 200
        await transport.tick()                            // the output's ack
        let before = channel.sent.count

        clock.now = 300
        await transport.handleDatagram(try datagram(
            seq: 1, oldNum: 1, newNum: MoshTransport.shutdownStateNum, ackNum: 1, text: "logout\r\n"))
        #expect(channel.sent.count == before, "acked a frame after the last datagram, not at once")
        #expect(await transport.sessionEnded == false)
        clock.now = 450                                   // 200 + 250 ms, a frame before any RTT sample
        await transport.tick()
        let acks = try instructions(channel, from: before)
        #expect(acks.count == 1)
        #expect(acks.first?.ackNum == MoshTransport.shutdownStateNum)
        try #require(await transport.sessionEnded, "the session goes on after the server ended it")
        #expect(channel.cancelled)
        try await Task.sleep(for: .milliseconds(50))      // the main-actor hop
        #expect(await MainActor.run { box.fired } == 1)

        // The host stream ends after the last output; nothing goes out again.
        var frames: [Data] = []
        for await frame in transport.hostStream { frames.append(frame.bytes) }
        #expect(frames == [Data("$ exit".utf8), Data("logout\r\n".utf8)])
        let sentAtEnd = channel.sent.count
        clock.now = 600
        await transport.send(Data("x".utf8))
        clock.now = 10_000
        await transport.tick()
        await transport.resume()
        #expect(channel.sent.count == sentAtEnd)
    }

    @Test("a shutdown state the client cannot apply is acked as a gap, and the next try ends the session")
    func aShutdownStateThatDoesNotApplyWaits() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, _) = try makeTransport(clock: clock)
        await transport.start(cols: 80, rows: 24)
        await transport.resume()
        clock.now = 100
        await transport.handleDatagram(try datagram(
            seq: 0, oldNum: 3, newNum: MoshTransport.shutdownStateNum, ackNum: 0, text: "logout"))
        let before = channel.sent.count
        clock.now = 200
        await transport.tick()
        let acks = try instructions(channel, from: before)
        #expect(!acks.isEmpty && acks.allSatisfy { $0.ackNum == 0 })
        #expect(await transport.sessionEnded == false)

        clock.now = 300
        await transport.handleDatagram(try datagram(
            seq: 1, oldNum: 0, newNum: MoshTransport.shutdownStateNum, ackNum: 0, text: "logout"))
        clock.now = 450
        await transport.tick()
        #expect(try instructions(channel, from: before).last?.ackNum == MoshTransport.shutdownStateNum)
        #expect(await transport.sessionEnded)
    }

    // MARK: B48 — full redraw

    /// One instruction of a host diff, as mosh-server's
    /// `Complete::diff_from` writes them (echo ack, resize, output).
    enum HostPiece {
        case echoAck(UInt64)
        case resize(Int, Int)
        case bytes(String)
    }

    func hostDiff(_ pieces: [HostPiece]) -> Data {
        var message = ProtoWriter()
        for piece in pieces {
            var body = ProtoWriter()
            var inner = ProtoWriter()
            switch piece {
            case .echoAck(let num):
                body.appendField(8, varint: num)
                inner.appendField(7, bytes: body.bytes)
            case .resize(let width, let height):
                body.appendField(5, varint: UInt64(bitPattern: Int64(width)))
                body.appendField(6, varint: UInt64(bitPattern: Int64(height)))
                inner.appendField(3, bytes: body.bytes)
            case .bytes(let text):
                body.appendField(4, bytes: Data(text.utf8))
                inner.appendField(2, bytes: body.bytes)
            }
            message.appendField(1, bytes: inner.bytes)
        }
        return message.data
    }

    func frame(seq: UInt64, _ oldNum: UInt64, _ newNum: UInt64, _ pieces: [HostPiece]) throws -> Data {
        try serverDatagram(seq: seq, oldNum: oldNum, newNum: newNum, ackNum: 0, diff: hostDiff(pieces))
    }

    /// Every frame the terminal was handed, once the transport is closed.
    func shownFrames(_ transport: MoshTransport) async -> [MoshHostFrame] {
        await transport.close()
        var frames: [MoshHostFrame] = []
        for await frame in transport.hostStream { frames.append(frame) }
        return frames
    }

    /// The last user event of the last instruction sent since the last call.
    func lastEvent(_ reader: inout ServerSide, _ channel: Channel) throws -> UserEvent? {
        let last = try #require(try reader.instructions(from: channel).last)
        return try UserMessage(parsing: last.diff).events.last
    }

    @Test("a redraw makes mosh-server repaint, and hides the frames drawn at the narrow width")
    func aRedrawHidesFramesUntilTheRepaint() async throws {
        let clock = ManualMoshClock()
        let (transport, _, _) = try makeTransport(clock: clock)
        await transport.start(cols: 80, rows: 24)
        clock.now = 10
        await transport.requestFullRedraw()
        clock.now = 20
        await transport.handleDatagram(try frame(seq: 0, 0, 1, [.echoAck(1), .resize(79, 24), .bytes("narrow")]))
        clock.now = 30
        await transport.handleDatagram(try frame(seq: 1, 1, 2, [.bytes("still narrow")]))
        clock.now = 40
        await transport.handleDatagram(try frame(seq: 2, 2, 3, [.resize(80, 24), .bytes("repaint")]))
        clock.now = 50
        await transport.handleDatagram(try frame(seq: 3, 3, 4, [.bytes("after")]))
        #expect(await transport.currentDiagnostics.appliedHostNum == 4)
        // The echo ack in a hidden frame still lands (predictions settle on it).
        #expect(await shownFrames(transport) == [
            MoshHostFrame(bytes: Data(), echoAck: 1),
            MoshHostFrame(bytes: Data("repaint".utf8), echoAck: 1),
            MoshHostFrame(bytes: Data("after".utf8), echoAck: 1),
        ])
    }

    @Test("the narrower width goes out alone; the real one follows the frame at it")
    func aRedrawRestoresTheWidthAfterTheNarrowFrame() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 100, rows: 30)
        await transport.requestFullRedraw()
        // Two resizes in one state: mosh-server applies both before it
        // renders, sees no size change and sends nothing at all.
        #expect(try lastEvent(&reader, channel) == .resize(width: 99, height: 30))
        // An unrelated frame first: shown, and its ack (within mosh's
        // ACK_DELAY) restores nothing yet.
        await transport.handleDatagram(try frame(seq: 0, 0, 1, [.bytes("output")]))
        clock.now = 100
        await transport.tick()
        #expect(try lastEvent(&reader, channel) == .resize(width: 99, height: 30))
        await transport.handleDatagram(try frame(seq: 1, 1, 2, [.resize(99, 30), .bytes("narrow")]))
        // The real width goes with the ack of the narrow frame.
        clock.now = 200
        await transport.tick()
        #expect(try lastEvent(&reader, channel) == .resize(width: 100, height: 30))
        #expect(await shownFrames(transport) == [MoshHostFrame(bytes: Data("output".utf8), echoAck: 0)])
    }

    @Test("a real resize cancels a redraw: a narrow frame after it is shown")
    func aResizeCancelsARedraw() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        await transport.requestFullRedraw()
        await transport.resize(cols: 120, rows: 40)
        _ = try reader.instructions(from: channel)
        await transport.handleDatagram(try frame(seq: 0, 0, 1, [.resize(79, 24), .bytes("narrow")]))
        // A frame on: the real size goes, and no restore of the old one.
        clock.now = 250
        await transport.tick()
        #expect(try lastEvent(&reader, channel) == .resize(width: 120, height: 40))
        #expect(await shownFrames(transport) == [MoshHostFrame(bytes: Data("narrow".utf8), echoAck: 0)])
    }

    @Test("a redraw hides frames for at most its limit, whatever the server does")
    func aRedrawHidesFramesForABoundedTime() async throws {
        let clock = ManualMoshClock()
        let (transport, _, _) = try makeTransport(clock: clock)
        await transport.start(cols: 80, rows: 24)
        await transport.requestFullRedraw()
        clock.now = 100
        await transport.handleDatagram(try frame(seq: 0, 0, 1, [.resize(79, 24), .bytes("narrow")]))
        clock.now = 100 + MoshTransport.redrawHideLimitMs
        await transport.handleDatagram(try frame(seq: 1, 1, 2, [.bytes("hidden")]))
        clock.now += 1
        await transport.handleDatagram(try frame(seq: 2, 2, 3, [.bytes("late")]))
        #expect(await shownFrames(transport) == [MoshHostFrame(bytes: Data("late".utf8), echoAck: 0)])
    }

    @Test("at two columns, as narrow as it goes, a redraw widens by one")
    func aRedrawAtTwoColumnsWidens() async throws {
        let (transport, channel, server) = try makeTransport()
        var reader = server
        await transport.start(cols: 2, rows: 5)
        await transport.requestFullRedraw()
        #expect(try lastEvent(&reader, channel) == .resize(width: 3, height: 5))
    }

    // MARK: B49 — timestamps (mosh network.cc)

    /// The client's datagrams since the last call, opened.
    func messages(_ reader: inout ServerSide, _ channel: Channel) throws -> [MoshCrypto.Message] {
        var out: [MoshCrypto.Message] = []
        while reader.read < channel.sent.count {
            out.append(try reader.crypto.open(channel.sent[reader.read]))
            reader.read += 1
        }
        return out
    }

    func stamped(seq: UInt64, timestamp: UInt16, reply: UInt16) throws -> Data {
        try serverDatagram(seq: seq, ackNum: 0, timestamp: timestamp, timestampReply: reply)
    }

    @Test("0xFFFF, the server's 'nothing to echo', is not an RTT sample; 0 is a real one")
    func aNoTimestampReplyIsNotASample() async throws {
        // The clock's low 16 bits under 5000: 0xFFFF read as a reply was a
        // sample of now16 + 1 ms, a bogus SRTT for ~5 s of every 65.5 s.
        let clock = ManualMoshClock(3 * 65_536 + 1_000)
        let (transport, _, _) = try makeTransport(clock: clock)
        await transport.handleDatagram(try stamped(seq: 0, timestamp: 500, reply: 0xFFFF))
        #expect(await transport.currentSRTT == 0)
        await transport.handleDatagram(try stamped(
            seq: 1, timestamp: 500, reply: MoshTransport.timestamp16(clock.now) - 40))
        #expect(await transport.currentSRTT == 40)
        clock.now = 4 * 65_536 + 80
        await transport.handleDatagram(try stamped(seq: 2, timestamp: 500, reply: 0))
        #expect(await transport.currentSRTT == 0.875 * 40 + 0.125 * 80)
    }

    @Test("the first RTT sample seeds the estimate even when it is 0 ms")
    func aZeroFirstSampleSeedsTheEstimate() async throws {
        let clock = ManualMoshClock(10_000)
        let (transport, _, _) = try makeTransport(clock: clock)
        await transport.handleDatagram(try stamped(seq: 0, timestamp: 1, reply: MoshTransport.timestamp16(10_000)))
        await transport.handleDatagram(try stamped(seq: 1, timestamp: 1, reply: MoshTransport.timestamp16(10_000) - 80))
        #expect(await transport.currentSRTT == 10)
    }

    @Test("a reordered or replayed datagram feeds neither the RTT nor the echo")
    func anOlderDatagramFeedsNoTimestamps() async throws {
        let clock = ManualMoshClock(19_990)
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        await transport.resume()                          // .ready's first datagram
        _ = try messages(&reader, channel)
        clock.now = 20_000
        await transport.handleDatagram(try stamped(
            seq: 5, timestamp: 1_000, reply: MoshTransport.timestamp16(20_000) - 30))
        #expect(await transport.currentSRTT == 30)
        _ = try messages(&reader, channel)
        clock.now = 20_010
        await transport.handleDatagram(try stamped(
            seq: 4, timestamp: 2_000, reply: MoshTransport.timestamp16(20_010) - 900))
        #expect(await transport.currentSRTT == 30)
        _ = try messages(&reader, channel)
        // The echo is still the in-order datagram's timestamp, advanced 20 ms
        // (the resize goes at once: a frame, 20 ms at this RTT, after the size).
        clock.now = 20_020
        await transport.resize(cols: 81, rows: 24)
        #expect(try messages(&reader, channel).first?.timestampReply == 1_020)
    }

    @Test("the server's timestamp is echoed once, advanced by the time held, only within a second")
    func theEchoIsFreshAndUsedOnce() async throws {
        let clock = ManualMoshClock()
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        clock.now = 1_000
        await transport.resize(cols: 81, rows: 24)
        #expect(try messages(&reader, channel).map(\.timestampReply) == [0xFFFF],
                "before anything to echo the reply is 0xFFFF, never 0")

        clock.now = 2_000
        await transport.handleDatagram(try stamped(seq: 0, timestamp: 7_000, reply: 0xFFFF))
        _ = try messages(&reader, channel)
        // A frame apart (250 ms before an RTT sample), each goes at once.
        clock.now = 2_250
        await transport.resize(cols: 82, rows: 24)
        clock.now = 2_500
        await transport.resize(cols: 83, rows: 24)
        #expect(try messages(&reader, channel).map(\.timestampReply) == [7_250, 0xFFFF])

        clock.now = 3_000
        await transport.handleDatagram(try stamped(seq: 1, timestamp: 9_000, reply: 0xFFFF))
        _ = try messages(&reader, channel)
        clock.now = 4_000                                   // held a full second: too stale
        await transport.resize(cols: 84, rows: 24)
        #expect(try messages(&reader, channel).map(\.timestampReply) == [0xFFFF])
    }

    @Test("our own timestamp skips 0xFFFF, which would read as 'no timestamp'")
    func ourTimestampNeverReadsAsMissing() async throws {
        let clock = ManualMoshClock(65_535)
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        await transport.resize(cols: 81, rows: 24)
        #expect(try messages(&reader, channel).map(\.timestamp) == [0])
        #expect(MoshTransport.timestamp16(65_534) == 65_534)
        #expect(MoshTransport.timestamp16(65_536) == 0)
    }

    @Test("in a fragmented flush only the first datagram carries the echo")
    func aFragmentedFlushEchoesOnce() async throws {
        let clock = ManualMoshClock(0)
        let (transport, channel, server) = try makeTransport(clock: clock)
        var reader = server
        await transport.start(cols: 80, rows: 24)
        await transport.resume()                          // .ready's first datagram
        _ = try messages(&reader, channel)
        clock.now = 10
        await transport.handleDatagram(try stamped(seq: 0, timestamp: 100, reply: 0xFFFF))
        #expect(try messages(&reader, channel).isEmpty)
        var rng = SplitMix64(state: 9)
        clock.now = 260                                   // a frame after the size: at once
        await transport.send(Data((0..<4_000).map { _ in UInt8(truncatingIfNeeded: rng.next()) }))
        let replies = try messages(&reader, channel).map(\.timestampReply)
        #expect(replies == [350, 0xFFFF, 0xFFFF, 0xFFFF], "incompressible input: four fragments")
    }
}
