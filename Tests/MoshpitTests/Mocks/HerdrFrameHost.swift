import Foundation
@testable import Moshpit

/// A host for herdr over SSH, end to end through a real `SSHSession`: it
/// answers the capability probe (herdr installed; no mosh-server, no
/// python3 — so no push pump dials out), serves `herdr api snapshot` from
/// `snapshot` (held until ``releaseSnapshots()`` when asked to), opens a PTY
/// whose output the test drives with ``emit(_:)``, and records every byte
/// written into that PTY.
final class HerdrFrameHost: SSHClientTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var _output: (@Sendable (Data) -> Void)?
    private var _written: [Data] = []
    private let snapshot: String?
    private var holding: Bool
    private var held: [CheckedContinuation<Void, Never>] = []

    /// The frame loop never says it reads (it can't run on this host).
    let silentLoop: Bool

    init(snapshot: String? = nil, holdSnapshots: Bool = false, silentLoop: Bool = false) {
        self.snapshot = snapshot
        self.holding = holdSnapshots
        self.silentLoop = silentLoop
    }

    /// One workspace, one tab, one pane — `w1:p1`, focused.
    static let onePane = """
    {"id":"cli:api:snapshot","result":{"snapshot":{
      "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
      "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],
      "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true}],
      "layouts":[],
      "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}
    """

    /// One line of `herdr terminal session control` output carrying `bytes`.
    static func frameLine(seq: Int, bytes: Data, full: Bool = false) -> String {
        #"{"type":"terminal.frame","seq":\#(seq),"width":80,"height":24,"full":\#(full),"bytes":"\#(bytes.base64EncodedString())"}"#
            + "\n"
    }

    var ptyOpen: Bool { lock.withLock { _output != nil } }
    var written: [Data] { lock.withLock { _written } }
    var writtenText: String { written.map { String(decoding: $0, as: UTF8.self) }.joined() }

    /// Bytes from the remote end of the PTY.
    func emit(_ data: Data) {
        let output = lock.withLock { _output }
        output?(data)
    }

    /// Let every held snapshot read through, and stop holding.
    func releaseSnapshots() {
        let waiting: [CheckedContinuation<Void, Never>] = lock.withLock {
            holding = false
            defer { held = [] }
            return held
        }
        for continuation in waiting { continuation.resume() }
    }

    func run(_ command: String) async throws -> Data {
        if command == HostCapabilities.probeCommand {
            return Data("/usr/local/bin/herdr\n::Linux::/usr/bin/apt-get\n".utf8)
        }
        guard command.contains("api snapshot") else { return Data() }
        if lock.withLock({ holding }) {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                let resumeNow: Bool = lock.withLock {
                    guard holding else { return true }
                    held.append(continuation)
                    return false
                }
                if resumeNow { continuation.resume() }
            }
        }
        return Data((snapshot ?? "").utf8)
    }

    final class Writer: SSHPTYWriter, @unchecked Sendable {
        private weak var host: HerdrFrameHost?
        init(host: HerdrFrameHost) { self.host = host }
        func write(_ data: Data) async throws { host?.record(data) }
        func resize(cols: Int, rows: Int) async throws {}
    }

    private func record(_ data: Data) { lock.withLock { _written.append(data) } }

    func openPTY(rows: Int, cols: Int,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        try await openPTY(rows: rows, cols: cols, command: nil, onOutput: onOutput, onEnd: onEnd)
    }

    /// The channel's program (nil: the login shell). The frame loop, the
    /// SSH channel's program since B267, says it reads as soon as it runs.
    func openPTY(rows: Int, cols: Int, command: String?,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        lock.withLock {
            _output = onOutput
            _programs.append(command)
        }
        if command?.contains("moshpit-herdr") == true, !silentLoop {
            Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(10))
                self?.emit(Data((HerdrFrameCommand.readyMarker + "\r\n").utf8))
            }
        }
        return Writer(host: self)
    }

    private var _programs: [String?] = []
    /// Each PTY's program, in order (nil: the login shell).
    var programs: [String?] { lock.withLock { _programs } }

    func shutdown() async { releaseSnapshots() }
}
