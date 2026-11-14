import Foundation
@testable import Moshpit

/// The herdr push pump's side of a host (`HerdrPushBoot`): every dial is a
/// fresh connection whose PTY runs the pump — as the channel's program, or
/// once its boot line is typed into the login shell — and the pump says it
/// holds the socket and answers the subscribe, until the test ends it the
/// way a herdr server restart does: the socket closes under the pump, the
/// pump exits, its channel ends.
///
/// `flushingShell`: the account's login shell flushes what was typed while
/// it started (Powerlevel10k's instant prompt, B267) — a line typed into it
/// is lost.
final class HerdrPumpHost: @unchecked Sendable {
    private let lock = NSLock()
    private var _dials = 0
    private var _current: Pump?
    private var _programs: [String?] = []
    let flushingShell: Bool

    init(flushingShell: Bool = false) {
        self.flushingShell = flushingShell
    }

    /// How many connections the pump was dialled on.
    var dials: Int { lock.withLock { _dials } }
    /// Each channel's program, in order (nil: the login shell).
    var programs: [String?] { lock.withLock { _programs } }
    /// What was typed into a login shell, on every connection.
    var typedIntoShells: String {
        lock.withLock { _typed }
    }
    private var _typed = ""

    fileprivate func opened(program: String?) { lock.withLock { _programs.append(program) } }
    fileprivate func typed(_ text: String) { lock.withLock { _typed += text } }

    /// A connection for the next dial.
    func transport() -> Pump {
        let pump = Pump(host: self)
        lock.withLock {
            _dials += 1
            _current = pump
        }
        return pump
    }

    /// The herdr server restarts under the newest pump.
    func endCurrent() {
        let pump = lock.withLock { _current }
        pump?.end()
    }

    final class Pump: SSHClientTransport, @unchecked Sendable {
        private let lock = NSLock()
        private weak var host: HerdrPumpHost?
        private var output: (@Sendable (Data) -> Void)?
        private var onEnd: (@Sendable (SSHChannelEnd) -> Void)?
        private var written = Data()
        private var booted = false
        private var ended = false
        /// The channel is a login shell (no program).
        private var shell = false

        init(host: HerdrPumpHost) { self.host = host }

        func run(_ command: String) async throws -> Data { Data() }

        final class Writer: SSHPTYWriter, @unchecked Sendable {
            private weak var pump: Pump?
            init(pump: Pump) { self.pump = pump }
            func write(_ data: Data) async throws { pump?.received(data) }
            func resize(cols: Int, rows: Int) async throws {}
        }

        func openPTY(rows: Int, cols: Int,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            try await openPTY(rows: rows, cols: cols, command: nil, onOutput: onOutput, onEnd: onEnd)
        }

        func openPTY(rows: Int, cols: Int, command: String?,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            host?.opened(program: command)
            let pump = command?.contains("python3") == true
            lock.withLock {
                output = onOutput
                self.onEnd = onEnd
                shell = command == nil
                booted = pump
            }
            if pump {
                // The program runs at once and says it holds the socket.
                Task { [weak self] in
                    try? await Task.sleep(for: .milliseconds(20))
                    self?.emit(HerdrPushBoot.readyMarker + "\r\n")
                }
            }
            return Writer(pump: self)
        }

        func shutdown() async { end(.dropped) }

        /// The pump exits: its channel ends in order.
        func end(_ how: SSHChannelEnd = .exited(status: nil)) {
            let onEnd: (@Sendable (SSHChannelEnd) -> Void)? = lock.withLock {
                guard !ended else { return nil }
                ended = true
                return self.onEnd
            }
            onEnd?(how)
        }

        private func emit(_ text: String) {
            let output = lock.withLock { ended ? nil : output }
            output?(Data(text.utf8))
        }

        private func received(_ data: Data) {
            var replies: [String] = []
            let typedIntoShell = lock.withLock { shell && !booted }
            if typedIntoShell {
                host?.typed(String(decoding: data, as: UTF8.self))
                // A shell still starting flushes it.
                if host?.flushingShell == true { return }
            }
            lock.withLock {
                written.append(data)
                // The boot line goes out with a carriage return, no newline.
                if !booted, String(decoding: written, as: UTF8.self).contains("python3") {
                    booted = true
                    written.removeAll()
                    replies.append(HerdrPushBoot.readyMarker + "\r\n")
                    return
                }
                while let newline = written.firstIndex(of: UInt8(ascii: "\n")) {
                    let line = String(decoding: written[written.startIndex..<newline], as: UTF8.self)
                    written.removeSubrange(written.startIndex...newline)
                    guard let object = HerdrSnapshot.firstJSONObject(in: line),
                          let id = object["id"] as? String,
                          (object["method"] as? String) == "events.subscribe" else { continue }
                    replies.append(#"{"id":"\#(id)","result":{"type":"subscription_started"}}"# + "\n")
                }
            }
            for reply in replies { emit(reply) }
        }
    }
}
