import Foundation
@testable import Moshpit

/// A host behind a fake SSH connection, for session-layer tests that run the
/// hub's real start, teardown and delivery paths with no network: execs
/// answered by the test's script (or held until the test lets them go), a
/// PTY whose writes are kept and whose output the test writes — and, when
/// ``tmux`` is set, a tmux server on that PTY that answers `-CC attach` and
/// the controller's commands the way tmux 3.6a does, with one session
/// (`$0 main`), one window (`@0`) and one pane (`%0`).
final class FakeSSHHost: SSHClientTransport, @unchecked Sendable {

    /// How the host answers an exec.
    enum Reply: Sendable {
        case output(String)
        case fail
        /// Held until ``release(_:with:)`` (or forever).
        case held(String)
    }

    /// The tmux on this host's PTY.
    struct Tmux: Sendable {
        /// A server with a session; without one, `-CC attach` is refused.
        var hasSession = true
        /// Whether `paste-buffer` succeeds (false: the pane is gone).
        var pasteSucceeds = true
        /// How long tmux takes to answer the boot line (attach or refusal).
        /// Zero answers inside the write itself — before the app has even
        /// returned from writing it, which no real link does: the reply
        /// takes at least a round trip, and a tmux client to start.
        var answerDelay: Duration = .zero
        /// `list-clients` rows in the format the mosh sidecar asks for
        /// (`TmuxClientRow.format`: `control|pid|session|name`); nil answers
        /// a bare client name, as before.
        var clients: [String]?
    }

    private let lock = NSLock()
    private var answer: @Sendable (String) -> Reply
    private var _commands: [String] = []
    private var _written: [String] = []
    private var _programs: [String?] = []
    private var _ptyOpened = 0
    private var _shutdowns = 0
    private var onOutput: (@Sendable (Data) -> Void)?
    private var onEnd: (@Sendable (SSHChannelEnd) -> Void)?
    private var held: [String: [CheckedContinuation<Reply, Never>]] = [:]
    private var _tmux: Tmux?
    private var tmuxAttached = false
    /// The mosh sidecar's boot chain is retrying `-CC attach` on a server
    /// with no session (its `while ! … -CC attach … sleep 2` loop).
    private var attachRetrying = false
    private var pending = ""
    private var replyNumber = 1000
    private var _writeFails = false
    private var _heldSurviveShutdown = false
    private var _ptyHold: String?

    /// The address the connection reached (``SSHClientTransport/peerAddress``).
    let peerAddress: String?

    init(tmux: Tmux? = nil, peer: String? = nil,
         answer: @escaping @Sendable (String) -> Reply = { _ in .output("") }) {
        self._tmux = tmux
        self.peerAddress = peer
        self.answer = answer
    }

    // MARK: What the test sets and reads

    /// How execs are answered from now on.
    func answer(_ script: @escaping @Sendable (String) -> Reply) { lock.withLock { answer = script } }

    var tmux: Tmux? {
        get { lock.withLock { _tmux } }
        set { lock.withLock { _tmux = newValue } }
    }

    /// Every exec, in order.
    var commands: [String] { lock.withLock { _commands } }
    /// Every PTY write, decoded, in order.
    var written: [String] { lock.withLock { _written } }
    /// Everything written to the PTY as one string.
    var typed: String { written.joined() }
    /// Each PTY's program, in order (nil: the login shell).
    var programs: [String?] { lock.withLock { _programs } }
    var ptyOpened: Int { lock.withLock { _ptyOpened } }
    var shutdowns: Int { lock.withLock { _shutdowns } }
    /// Whether tmux on this PTY is attached (answered `-CC attach`).
    var attached: Bool { lock.withLock { tmuxAttached } }
    /// Whether a boot chain on this PTY is retrying `-CC attach`, waiting
    /// for a session.
    var retryingAttach: Bool { lock.withLock { attachRetrying } }

    /// A session appears on the server — the person made one, here or on
    /// another client. A boot chain retrying `-CC attach` lands on it with
    /// its next try (after ``Tmux/answerDelay``), as the real loop does.
    func sessionAppears() {
        let (retrying, delay): (Bool, Duration) = lock.withLock {
            _tmux?.hasSession = true
            defer { attachRetrying = false }
            guard attachRetrying else { return (false, .zero) }
            tmuxAttached = true
            return (true, _tmux?.answerDelay ?? .zero)
        }
        guard retrying else { return }
        answerBoot("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n", after: delay)
    }

    /// PTY writes fail from now on (a dead channel).
    func failWrites() { lock.withLock { _writeFails = true } }

    /// Held execs (and a held PTY) still answer after the connection is shut
    /// down: the reply was already on its way when the session closed it.
    var heldSurviveShutdown: Bool {
        get { lock.withLock { _heldSurviveShutdown } }
        set { lock.withLock { _heldSurviveShutdown = newValue } }
    }

    /// The next PTY request is held under this key until ``release(_:with:)``.
    var holdPTY: String? {
        get { lock.withLock { _ptyHold } }
        set { lock.withLock { _ptyHold = newValue } }
    }

    /// The PTY prints `text`.
    func say(_ text: String) {
        lock.withLock { onOutput }?(Data(text.utf8))
    }

    /// The PTY channel ends, once.
    func end(_ how: SSHChannelEnd) {
        let handler: (@Sendable (SSHChannelEnd) -> Void)? = lock.withLock {
            defer { onEnd = nil }
            return onEnd
        }
        handler?(how)
    }

    /// Execs held under `key` (``Reply/held(_:)``) answer `output` now.
    func release(_ key: String, with output: String) {
        let waiting = lock.withLock { held.removeValue(forKey: key) ?? [] }
        for continuation in waiting { continuation.resume(returning: .output(output)) }
    }

    /// Whether an exec is held under `key`.
    func isHolding(_ key: String) -> Bool { lock.withLock { !(held[key] ?? []).isEmpty } }

    // MARK: SSHClientTransport

    func run(_ command: String) async throws -> Data {
        let script = lock.withLock { () -> @Sendable (String) -> Reply in
            _commands.append(command)
            return answer
        }
        switch script(command) {
        case .output(let text):
            return Data(text.utf8)
        case .fail:
            throw SSHError.sessionClosed
        case .held(let key):
            let outcome = await withCheckedContinuation { (continuation: CheckedContinuation<Reply, Never>) in
                lock.withLock { held[key, default: []].append(continuation) }
            }
            if case .output(let text) = outcome { return Data(text.utf8) }
            throw SSHError.sessionClosed
        }
    }

    final class Writer: SSHPTYWriter, @unchecked Sendable {
        private let host: FakeSSHHost
        init(host: FakeSSHHost) { self.host = host }
        func write(_ data: Data) async throws { try host.received(data) }
        func resize(cols: Int, rows: Int) async throws {}
    }

    func openPTY(rows: Int, cols: Int,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        try await openPTY(rows: rows, cols: cols, command: nil, onOutput: onOutput, onEnd: onEnd)
    }

    /// The PTY with `command` its program (nil: the login shell): tmux on
    /// this host runs what the program asks, as it does a line typed into
    /// the shell — the program is never typed (``programs``, ``typed``).
    func openPTY(rows: Int, cols: Int, command: String?,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        let writer = try await openShellPTY(onOutput: onOutput, onEnd: onEnd)
        lock.withLock { _programs.append(command) }
        if let command, lock.withLock({ _tmux != nil }) { tmuxHears(command) }
        return writer
    }

    private func openShellPTY(onOutput: @escaping @Sendable (Data) -> Void,
                              onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        if let key = lock.withLock({ () -> String? in defer { _ptyHold = nil }; return _ptyHold }) {
            let outcome = await withCheckedContinuation { (continuation: CheckedContinuation<Reply, Never>) in
                lock.withLock { held[key, default: []].append(continuation) }
            }
            if case .fail = outcome { throw SSHError.channelOpenFailed }
        }
        lock.withLock {
            _ptyOpened += 1
            self.onOutput = onOutput
            self.onEnd = onEnd
        }
        return Writer(host: self)
    }

    /// Shutting the connection ends the PTY's read loop, as Citadel's does,
    /// and fails every exec still waiting.
    func shutdown() async {
        let waiting = lock.withLock { () -> [CheckedContinuation<Reply, Never>] in
            _shutdowns += 1
            if _heldSurviveShutdown { return [] }
            defer { held.removeAll() }
            return held.values.flatMap { $0 }
        }
        for continuation in waiting { continuation.resume(returning: .fail) }
        end(.dropped)
    }

    // MARK: The PTY and its tmux

    private func received(_ data: Data) throws {
        let text = String(decoding: data, as: UTF8.self)
        let lines: [String] = try lock.withLock {
            if _writeFails { throw SSHError.sessionClosed }
            _written.append(text)
            guard _tmux != nil else { return [] }
            pending += text
            var lines: [String] = []
            while let end = pending.firstIndex(where: { $0 == "\n" || $0 == "\r" }) {
                lines.append(String(pending[..<end]))
                pending = String(pending[pending.index(after: end)...])
            }
            return lines
        }
        for line in lines where !line.isEmpty { tmuxHears(line) }
    }

    private func tmuxHears(_ line: String) {
        let (tmux, attached) = lock.withLock { (_tmux, tmuxAttached) }
        guard let tmux else { return }
        if !attached {
            guard line.contains("-CC attach") || line.contains("-CC new") else { return }
            guard tmux.hasSession || line.contains("-CC new") else {
                if line.contains("while ! ") {
                    // The sidecar's chain: its refusals go to /dev/null and
                    // it tries again every 2 s until there is a session
                    // (``sessionAppears()``).
                    lock.withLock { attachRetrying = true }
                    return
                }
                if line.contains("until ") {
                    // The SSH control client's program: refused once, then
                    // it waits for a session and attaches it (B267).
                    lock.withLock { attachRetrying = true }
                    answerBoot("%begin 1 1 0\nno sessions\n%error 1 1 0\n%exit\n", after: tmux.answerDelay)
                    return
                }
                answerBoot("no sessions\r\n", after: tmux.answerDelay)
                return
            }
            lock.withLock { tmuxAttached = true }
            // The attach command's own reply block, then the switch onto the
            // session: what tmux 3.6a sends a fresh control client.
            answerBoot("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n", after: tmux.answerDelay)
            return
        }
        say(block(for: line, tmux: tmux))
    }

    private func answerBoot(_ text: String, after delay: Duration) {
        guard delay > .zero else {
            say(text)
            return
        }
        Task { [self] in
            try? await Task.sleep(for: delay)
            say(text)
        }
    }

    /// tmux's answer to one control command, FIFO like tmux's own.
    private func block(for command: String, tmux: Tmux) -> String {
        let number = lock.withLock { () -> Int in
            replyNumber += 1
            return replyNumber
        }
        var body: [String] = []
        var failed = false
        if command.hasPrefix("list-sessions") {
            body = ["$0 1 main"]
        } else if command.hasPrefix("list-windows") {
            body = ["$0 @0 0 81x24,0,0,0 1 1 main"]
        } else if command.hasPrefix("list-panes") {
            body = ["%0 @0 0 80 24 1 0 0 bash"]
        } else if command.hasPrefix("display-message -p '#{client_name}'") {
            body = ["/dev/ttys009"]
        } else if command.hasPrefix("list-clients") {
            body = tmux.clients ?? ["/dev/ttys009"]
        } else if command.hasPrefix("paste-buffer") {
            failed = !tmux.pasteSucceeds
            if failed { body = ["can't find pane: %0"] }
        }
        let lines = body.isEmpty ? "" : body.joined(separator: "\n") + "\n"
        let end = failed ? "%error" : "%end"
        return "%begin \(number) \(number) 1\n\(lines)\(end) \(number) \(number) 1\n"
    }
}

/// A fresh ``FakeSSHHost`` per dial — a reconnect's new connection gets its
/// own PTY, and a tmux that answers a new attach — every one kept, in dial
/// order. `make` says what the next dial finds.
final class FakeSSHHosts: @unchecked Sendable {
    private let lock = NSLock()
    private var list: [FakeSSHHost] = []
    private var _make: @Sendable () -> FakeSSHHost

    init(_ make: @escaping @Sendable () -> FakeSSHHost) { _make = make }

    var make: @Sendable () -> FakeSSHHost {
        get { lock.withLock { _make } }
        set { lock.withLock { _make = newValue } }
    }

    func next() -> FakeSSHHost {
        let host = make()
        lock.withLock { list.append(host) }
        return host
    }

    var all: [FakeSSHHost] { lock.withLock { list } }
    var last: FakeSSHHost? { lock.withLock { list.last } }

    /// A dialer handing out the next host each time.
    var script: DialScript {
        DialScript { [self] connection in SSHSession(connection: connection, transport: next()) }
    }
}
