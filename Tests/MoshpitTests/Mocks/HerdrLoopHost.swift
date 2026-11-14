import Foundation
@testable import Moshpit

/// A herdr host for the SSH frame channel that runs what the channel's lines
/// ask for, after the Rust client's rig (`crates/herdr/src/client/rig.rs`,
/// herdr 0.9.5 as recorded and probed): the login shell, the reader loop the
/// boot line starts, one attach at a time, and herdr's own ways —
///
///  - an attach is up once it has printed its first frame (``attachDelay``
///    after its line); a line that reaches it before that is LOST (probed on
///    a PTY: 9 of 10 lines written right behind the attach line, 0 of 10
///    written after its first frame);
///  - a release is answered by ONE `terminal.closed`, however many were
///    written: the attach that took the first is gone, and the loop's `read`
///    drops a line opening with `{`;
///  - a line a live attach can't parse gets `input ignored`, and the attach
///    goes on;
///  - another client's `--takeover` ends the attach with a close of its own,
///    and that client holds the pane from then on;
///  - a plain attach (no `--takeover`) to a pane another client holds is
///    refused with a close of its own ("… already has an attached client;
///    retry with --takeover", herdr 0.9.5); one with `--takeover` takes it.
///
/// Execs: the capability probe (herdr installed; python3 only when asked,
/// so no push pump dials out otherwise), `api snapshot` of one tab with two
/// panes whose focus follows `agent focus`. Real time: the session under
/// test runs its own clocks.
final class HerdrLoopHost: SSHClientTransport, @unchecked Sendable {
    private enum Reader: Equatable {
        case shell
        case loop
        /// `up`: the attach printed its first frame.
        case attach(pane: String, generation: Int, up: Bool)
    }

    private let lock = NSLock()
    private var _output: (@Sendable (Data) -> Void)?
    private var reader: Reader = .shell
    private var generation = 0
    private var seq = 0
    private var pendingBytes = Data()
    private var _focused = "w1:p1"
    private var _lines: [String] = []
    private var _attaches: [String] = []
    private var _typed: [String: Data] = [:]
    private var _lost: [String] = []
    private var _received: [String: [String]] = [:]
    private var _swallowNext = false
    /// Panes another client (another device) holds.
    private var _held: Set<String> = []
    private var _takenFromOthers: [String] = []
    private var _tookOver: [Bool] = []
    /// The loop takes over whatever it attaches (the boot line before round
    /// twenty-eight).
    private var _loopTakesOver = false

    /// From an attach line to that attach's first (full) frame.
    let attachDelay: Duration
    /// The probe says python3 is there (the push pump dials out).
    let python3: Bool
    /// Panes whose attach never comes up: no frame, every line it reads lost.
    let stuck: Set<String>

    init(attachDelay: Duration = .milliseconds(150), python3: Bool = false, stuck: Set<String> = []) {
        self.attachDelay = attachDelay
        self.python3 = python3
        self.stuck = stuck
    }

    // MARK: What the test reads

    /// Every line written into the PTY, in order.
    var lines: [String] { lock.withLock { _lines } }
    /// The panes the shell or the loop attached, in order.
    var attaches: [String] { lock.withLock { _attaches } }
    /// Lines that reached an attach before it was up, and were lost.
    var lost: [String] { lock.withLock { _lost } }
    /// The pane the server has focused.
    var focused: String { lock.withLock { _focused } }
    /// What each pane received as typed input, as text.
    func typed(_ pane: String) -> String {
        String(decoding: lock.withLock { _typed[pane] ?? Data() }, as: UTF8.self)
    }
    /// The message types each pane's attach acted on, in order.
    func received(_ pane: String) -> [String] { lock.withLock { _received[pane] ?? [] } }
    /// The pane whose attach is reading the channel, once it is up.
    var reading: String? {
        lock.withLock {
            if case .attach(let pane, _, true) = reader { return pane }
            return nil
        }
    }
    /// How many releases were written.
    var releases: Int { lines.filter { $0 == HerdrFrameCommand.release }.count }
    /// Panes another client holds.
    var held: Set<String> {
        get { lock.withLock { _held } }
        set { lock.withLock { _held = newValue } }
    }
    /// Panes our `--takeover` took from that client, in order.
    var takenFromOthers: [String] { lock.withLock { _takenFromOthers } }
    /// Whether each attach in ``attaches`` asked for `--takeover`.
    var tookOver: [Bool] { lock.withLock { _tookOver } }

    /// Another client moves the server's focus (its person picked a pane).
    func focus(_ pane: String) { lock.withLock { _focused = pane } }

    /// The bytes of the frames a pane's attach printed, in order.
    static func screen(_ pane: String, _ seq: Int) -> Data { Data("[\(pane) #\(seq)]".utf8) }

    // MARK: What the test makes happen

    /// The attach reading the channel prints a frame (output in its pane).
    func paint(full: Bool = false) {
        let line: String? = lock.withLock {
            guard case .attach(let pane, _, true) = reader else { return nil }
            seq += 1
            return Self.frameLine(pane: pane, seq: seq, full: full)
        }
        if let line { emit(line) }
    }

    /// The loop's `read` loses the next pane it is asked for: a target
    /// line eaten on its way (by an attach still dying, a shell not ready).
    func swallowNextTarget() { lock.withLock { _swallowNext = true } }

    /// Another client takes the attach over, and holds the pane from then on.
    func takeOver() {
        let ended: Bool = lock.withLock {
            guard case .attach(let pane, _, _) = reader else { return false }
            _held.insert(pane)
            reader = .loop
            return true
        }
        if ended { emit(#"{"reason":"terminal attach taken over","type":"terminal.closed"}"# + "\n") }
    }

    /// Bytes the host prints, as they are.
    func emit(_ text: String) {
        let output = lock.withLock { _output }
        output?(Data(text.utf8))
    }

    // MARK: SSHClientTransport

    func run(_ command: String) async throws -> Data {
        if command == HostCapabilities.probeCommand {
            let python = python3 ? "/usr/bin/python3\n" : ""
            return Data("/usr/local/bin/herdr\n\(python)::Linux::/usr/bin/apt-get\n".utf8)
        }
        if let target = HerdrCommandOrderTests.FocusServer.focusTarget(command) {
            lock.withLock { _focused = target }
            return Data()
        }
        guard command.contains("api snapshot") else { return Data() }
        return Data(HerdrCommandOrderTests.FocusServer.snapshot(focused: focused).utf8)
    }

    final class Writer: SSHPTYWriter, @unchecked Sendable {
        private weak var host: HerdrLoopHost?
        init(host: HerdrLoopHost) { self.host = host }
        func write(_ data: Data) async throws { host?.received(data) }
        func resize(cols: Int, rows: Int) async throws {}
    }

    func openPTY(rows: Int, cols: Int,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        try await openPTY(rows: rows, cols: cols, command: nil, onOutput: onOutput, onEnd: onEnd)
    }

    /// The channel's program (nil: the login shell): the reader loop runs
    /// at once, says it reads, and waits for a target (B267).
    func openPTY(rows: Int, cols: Int, command: String?,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        let loop = command?.contains("moshpit-herdr") == true
        lock.withLock {
            _output = onOutput
            _programs.append(command)
            if loop {
                reader = .loop
                _loopTakesOver = false
            }
        }
        if loop {
            Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(20))
                self?.emit(HerdrFrameCommand.readyMarker + "\r\n")
            }
        }
        return Writer(host: self)
    }

    /// Each PTY's program, in order (nil: the login shell).
    var programs: [String?] { lock.withLock { _programs } }
    private var _programs: [String?] = []

    func shutdown() async {}

    // MARK: The channel

    static func frameLine(pane: String, seq: Int, full: Bool) -> String {
        HerdrFrameHost.frameLine(seq: seq, bytes: screen(pane, seq), full: full)
    }

    private func received(_ data: Data) {
        var complete: [String] = []
        lock.withLock {
            pendingBytes.append(data)
            while let newline = pendingBytes.firstIndex(of: UInt8(ascii: "\n")) {
                let line = String(decoding: pendingBytes[pendingBytes.startIndex..<newline], as: UTF8.self)
                pendingBytes.removeSubrange(pendingBytes.startIndex...newline)
                complete.append(line.trimmingCharacters(in: CharacterSet(charactersIn: "\r")))
            }
        }
        for line in complete { take(line) }
    }

    /// One line reaching whatever reads the channel now.
    private func take(_ line: String) {
        var out: [String] = []
        var attachTo: String?
        var takeover = false
        lock.withLock {
            _lines.append(line)
            switch reader {
            case .attach(let pane, _, let up):
                guard up else {
                    _lost.append(line)
                    return
                }
                guard line.hasPrefix("{"),
                      let object = HerdrSnapshot.firstJSONObject(in: line),
                      let type = object["type"] as? String else {
                    out.append("herdr: terminal session control input ignored: invalid json command: "
                               + "expected value at line 1 column 1\r\n")
                    return
                }
                _received[pane, default: []].append(type)
                switch type {
                case "terminal.input":
                    if let encoded = object["bytes"] as? String, let bytes = Data(base64Encoded: encoded) {
                        _typed[pane, default: Data()].append(bytes)
                    }
                case "terminal.release":
                    reader = .loop
                    out.append(#"{"reason":"detached","type":"terminal.closed"}"# + "\n")
                default:
                    break
                }
            case .loop:
                // The loop's `read`: our own commands meant for an attach
                // that is gone are dropped, a target is attached.
                let words = line.split(separator: " ")
                guard !line.hasPrefix("{"), let target = words.first else { return }
                guard !_swallowNext else {
                    _swallowNext = false
                    return
                }
                attachTo = String(target)
                // `<pane> <cols> <rows> [take]`.
                takeover = _loopTakesOver || (words.count >= 4 && words[3] == "take")
            case .shell:
                if line.contains("moshpit-herdr") {
                    out.append(HerdrFrameCommand.readyMarker + "\r\n")
                    reader = .loop
                    attachTo = Self.quoted(after: "moshpit-herdr ", in: line)
                    _loopTakesOver = !line.contains("${4:+--takeover}")
                    takeover = _loopTakesOver || line.hasSuffix(" take")
                } else if line.contains("terminal session control") {
                    attachTo = Self.quoted(after: "terminal session control ", in: line)
                    takeover = line.contains("--takeover")
                }
            }
        }
        for text in out { emit(text) }
        if let attachTo { attach(attachTo, takeover: takeover) }
    }

    /// An attach to `pane`: up once its first full frame is out — unless
    /// another client holds the pane and it is plain, then refused.
    private func attach(_ pane: String, takeover: Bool) {
        let refused: Bool = lock.withLock {
            _attaches.append(pane)
            _tookOver.append(takeover)
            guard _held.contains(pane) else { return false }
            if takeover {
                _held.remove(pane)
                _takenFromOthers.append(pane)
                return false
            }
            // The loop reads its next line; the one-shot line's shell its prompt.
            reader = reader == .shell ? .shell : .loop
            return true
        }
        if refused {
            emit(#"{"reason":"terminal attach failed: terminal \#(pane) already has an attached client; retry with --takeover","type":"terminal.closed"}"# + "\n")
            return
        }
        let mine: Int = lock.withLock {
            generation += 1
            reader = .attach(pane: pane, generation: generation, up: false)
            return generation
        }
        guard !stuck.contains(pane) else { return }
        let delay = attachDelay
        Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard let self else { return }
            let line: String? = self.lock.withLock {
                guard case .attach(let pane, mine, false) = self.reader else { return nil }
                self.reader = .attach(pane: pane, generation: mine, up: true)
                self.seq += 1
                return Self.frameLine(pane: pane, seq: self.seq, full: true)
            }
            if let line { self.emit(line) }
        }
    }

    private static func quoted(after marker: String, in line: String) -> String? {
        guard let start = line.range(of: marker + "'") else { return nil }
        return String(line[start.upperBound...].prefix { $0 != "'" })
    }
}
