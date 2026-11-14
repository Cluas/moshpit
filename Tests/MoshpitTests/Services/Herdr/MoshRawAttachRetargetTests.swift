import Foundation
import Network
import Testing
@testable import Moshpit

/// mosh+herdr's raw-attach renderer: one pane's stream on the mosh screen,
/// steered by retarget execs on the sidecar (`HerdrLaunch.retargetCommand`
/// publishes the target, then bounces the attach).
///
/// Each retarget used to be its own `Task { exec }`. Two quick switches were
/// two execs racing; when the first landed second, the loop was left on the
/// older pane — and `moshRawAttachTarget` already named the newer one, so the
/// poll's repeat of the focus was taken for "already showing it" and nothing
/// ever corrected the screen.
@Suite("mosh raw-attach retarget")
@MainActor
struct MoshRawAttachRetargetTests {

    /// The host end of the sidecar: a retarget "lands" — the target file is
    /// written and the attach bounced — when its exec returns, after
    /// `delay(terminal)`. Tracks how many are out at once.
    ///
    /// The renderer loop under mosh is running (`loopRuns`) or gone — its
    /// attach evicted by another client's takeover, its pane closed — when
    /// mosh shows a bare login shell. A running loop comes round to the
    /// target and the retarget says so, as the real one does (B208): it
    /// prints its marker once the loop has started the next attach. A gone
    /// loop never does: the write and the kill succeed all the same, and the
    /// exec comes back with nothing to say.
    final class RetargetHost: SSHClientTransport, @unchecked Sendable {
        private let lock = NSLock()
        private var _landed: [String] = []
        private var _inFlight = 0
        private var _maxInFlight = 0
        private var _failuresLeft: [String: Int]
        private var _loopRuns: Bool
        private var _execs = 0
        private var _checks = 0
        /// The loop parked (its attach ended on its own): the stop record
        /// the watch and the check print, `<status> <terminal>\n<words>`.
        private var _parked: String?
        /// Bumped by every landing: what a watch is armed on.
        private var _attach = 0
        private var _watches = 0
        /// Terminals the server no longer lists: a retarget to one is
        /// refused before the loop is touched.
        private var _missing: Set<String> = []
        /// Terminals another device's client holds: a plain attach to one —
        /// a follow — is refused and the loop parks on it; one with
        /// `--takeover` takes it from that device.
        private var _held: Set<String> = []
        private var _takenFromOthers: [String] = []
        /// Every retarget, with whether it asked for `--takeover`.
        private var _asked: [(terminal: String, takeover: Bool)] = []
        /// Loop-up checks still to find no loop (its start line swallowed
        /// by a login shell still starting).
        private var _bootUnanswered = 0
        /// What was asked, in order: `boot`, `retarget <terminal>`.
        private var _log: [String] = []
        private var _lastRetarget = ""
        private let delay: @Sendable (String) -> Duration

        init(failing failures: [String: Int] = [:], loopRuns: Bool = true,
             delay: @escaping @Sendable (String) -> Duration = { _ in .zero }) {
            _failuresLeft = failures
            _loopRuns = loopRuns
            self.delay = delay
        }

        /// Terminal ids in the order their retargets landed; the last one is
        /// what the loop shows.
        var landed: [String] { lock.withLock { _landed } }
        var maxInFlight: Int { lock.withLock { _maxInFlight } }
        /// Retarget execs run, landed or not.
        var execs: Int { lock.withLock { _execs } }
        /// Checks of the loop run (no retarget among them).
        var checks: Int { lock.withLock { _checks } }
        var loopRuns: Bool {
            get { lock.withLock { _loopRuns } }
            set { lock.withLock { _loopRuns = newValue } }
        }
        /// Watches run.
        var watches: Int { lock.withLock { _watches } }
        var missing: Set<String> {
            get { lock.withLock { _missing } }
            set { lock.withLock { _missing = newValue } }
        }
        var held: Set<String> {
            get { lock.withLock { _held } }
            set { lock.withLock { _held = newValue } }
        }
        /// Terminals our `--takeover` took from the other device, in order.
        var takenFromOthers: [String] { lock.withLock { _takenFromOthers } }
        /// Every retarget run: its terminal, and whether it took over.
        var asked: [(terminal: String, takeover: Bool)] { lock.withLock { _asked } }
        var bootUnanswered: Int {
            get { lock.withLock { _bootUnanswered } }
            set { lock.withLock { _bootUnanswered = newValue } }
        }
        var log: [String] { lock.withLock { _log } }
        /// The last retarget command, as run.
        var lastRetarget: String { lock.withLock { _lastRetarget } }

        /// Whether a retarget asks for `--takeover`: it names its terminal in
        /// the take file (a follow removes it), or — the loop before round
        /// twenty-eight — it has none, and the loop took over whatever it
        /// attached.
        static func takesOver(_ command: String) -> Bool {
            guard let at = command.range(of: ".take\"; ") else { return true }
            let before = command[..<at.lowerBound]
            let start = before.range(of: "; ", options: .backwards)?.upperBound ?? before.startIndex
            return !before[start...].hasPrefix("rm -f ")
        }

        /// Something ends the attach on its own (another device's takeover,
        /// its pane closing, the server stopping): the loop parks with
        /// herdr's last words.
        func park(_ said: String) {
            lock.withLock {
                guard let on = _landed.last, _loopRuns else { return }
                _parked = "1 \(on)\n\(said)"
            }
        }
        /// What mosh shows — where a keystroke going out now lands: the
        /// terminal the loop is attached to, or the login shell once the loop
        /// is gone.
        var onScreen: String {
            lock.withLock {
                guard _loopRuns else { return "the login shell" }
                // Parked: the tty, read by no one (discarded before the next
                // attach) — never a shell.
                return _parked == nil ? (_landed.last ?? "nothing") : "no pane"
            }
        }

        func run(_ command: String) async throws -> Data {
            if command.contains("echo \(Self.loopUpMarker)") {
                // The loop-up check: the host's looks (up to 1.5 s) end at
                // once here, a round trip later.
                let wait = delay("")
                if wait > .zero { try? await Task.sleep(for: wait) }
                return lock.withLock {
                    _log.append("boot")
                    if _bootUnanswered > 0 {
                        _bootUnanswered -= 1
                        return Data()
                    }
                    return Data("\(Self.loopUpMarker)\n".utf8)
                }
            }
            if command.hasPrefix("w=; read w") {
                // The watch: answers once the attach it was armed on stops
                // (the loop parks) or is replaced — its host-side looks are
                // 200 ms apart — or nothing for a long while.
                let armed = lock.withLock { () -> Int in
                    _watches += 1
                    return _attach
                }
                for _ in 0..<1_000 {
                    let said: String? = lock.withLock {
                        if let parked = _parked { return "\(Self.stoppedMarker)\n\(parked)\n" }
                        if _attach != armed { return "" }
                        return nil
                    }
                    if let said { return Data(said.utf8) }
                    try? await Task.sleep(for: .milliseconds(10))
                }
                return Data()
            }
            if let checked = Self.checked(by: command) {
                // Whether the loop is still on a terminal: read, nothing
                // written — answered as the host would, a round trip later.
                lock.withLock { _checks += 1 }
                let wait = delay(checked)
                if wait > .zero { try? await Task.sleep(for: wait) }
                return lock.withLock {
                    let up = _loopRuns && _parked == nil && _landed.last == checked
                    var said = up ? "\(Self.upMarker)\n" : ""
                    if let parked = _parked { said += "\(Self.stoppedMarker)\n\(parked)\n" }
                    return Data(said.utf8)
                }
            }
            guard let terminal = Self.target(of: command) else { return Data() }
            lock.withLock {
                _inFlight += 1
                _maxInFlight = max(_maxInFlight, _inFlight)
                _execs += 1
            }
            defer { lock.withLock { _inFlight -= 1 } }
            let wait = delay(terminal)
            if wait > .zero { try? await Task.sleep(for: wait) }
            let fail: Bool = lock.withLock {
                guard let left = _failuresLeft[terminal], left > 0 else { return false }
                _failuresLeft[terminal] = left - 1
                return true
            }
            if fail { throw SSHError.sessionClosed }
            if lock.withLock({ _missing.contains(terminal) }) {
                return Data("MOSHPIT_RETARGET_MISSING\n".utf8)
            }
            let takeover = Self.takesOver(command)
            let refused: String? = lock.withLock {
                _log.append("retarget \(terminal)")
                _lastRetarget = command
                _asked.append((terminal, takeover))
                guard _loopRuns, _held.contains(terminal) else { return nil }
                if takeover {
                    _held.remove(terminal)
                    _takenFromOthers.append(terminal)
                    return nil
                }
                // herdr refuses the plain attach ~260 ms in; the loop parks
                // on it, and the retarget's settle prints the stop record.
                let record = "1 \(terminal)\nherdr: server shut down: terminal attach failed: terminal "
                    + "\(terminal) already has an attached client; retry with --takeover"
                _parked = record
                _attach += 1
                return "\(Self.stoppedMarker)\n\(record)\n"
            }
            if let refused { return Data(refused.utf8) }
            let loopRuns = lock.withLock { () -> Bool in
                _landed.append(terminal)
                // A parked loop is woken by the retarget's go file.
                if _loopRuns {
                    _parked = nil
                    _attach += 1
                }
                return _loopRuns
            }
            // The marker the command echoes once the loop has come round —
            // only a command that has one, and only a loop that runs.
            guard loopRuns, command.contains("echo \(Self.marker)") else { return Data() }
            return Data("\(Self.marker)\n".utf8)
        }

        /// What a retarget prints once the loop has come round to it.
        static let marker = "MOSHPIT_RETARGETED"
        /// What the loop-up check prints once the loop runs.
        static let loopUpMarker = "MOSHPIT_LOOP_UP"
        /// What a check prints when the loop is on its terminal.
        static let upMarker = "MOSHPIT_ATTACHED"
        /// What the watch (and a check) prints when the loop parked.
        static let stoppedMarker = "MOSHPIT_RENDERER_STOPPED"

        /// The terminal id a check asks about:
        /// `[ "$(cat …target)" = '<id>' ] && kill -0 … && echo MOSHPIT_ATTACHED`.
        static func checked(by command: String) -> String? {
            guard command.contains("echo \(upMarker)"),
                  let start = command.range(of: "\" = '") else { return nil }
            return String(command[start.upperBound...].prefix { $0 != "'" })
        }

        /// The terminal id a retarget command publishes:
        /// `printf '%s' '<id>' > "…target"; kill -9 …`.
        static func target(of command: String) -> String? {
            guard let start = command.range(of: "printf '%s' '") else { return nil }
            return String(command[start.upperBound...].prefix { $0 != "'" })
        }

        final class Writer: SSHPTYWriter, @unchecked Sendable {
            func write(_ data: Data) async throws {}
            func resize(cols: Int, rows: Int) async throws {}
        }

        func openPTY(rows: Int, cols: Int,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            Writer()
        }

        func shutdown() async {}
    }

    /// herdr's snapshot: three panes in one tab, each with its terminal id.
    final class SnapshotRunner: HerdrCommandRunner, @unchecked Sendable {
        func run(_ command: String) async throws -> String {
            guard command.contains("api snapshot") else { return "" }
            return """
            {"id":"cli:api:snapshot","result":{"snapshot":{
              "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
              "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":3}],
              "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true,"terminal_id":"t-1"},
                       {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":false,"terminal_id":"t-2"},
                       {"pane_id":"w1:p3","tab_id":"w1:t1","workspace_id":"w1","focused":false,"terminal_id":"t-3"}],
              "layouts":[],
              "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}
            """
        }
    }

    /// A session that believes it is live mosh+herdr with the raw-attach
    /// loop booted, its sidecar ending at `host`.
    private func makeSession(_ host: RetargetHost) async -> SessionHub.ActiveSession {
        var connection = ServerConnection(
            name: "raw-attach",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .mosh
        connection.multiplexer = .herdr
        let session = SessionHub.ActiveSession(connection: connection)
        let key = Data((0..<16).map { UInt8($0) })
        let mosh = try! MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: SessionLifecycleTests.IdleDatagramChannel())
        let control = HerdrControlClient(runner: SnapshotRunner())
        await control.pollNowForTesting()   // learns the panes' terminal ids
        session.installForTesting(moshTransport: mosh,
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        return session
    }

    private func waitUntil(_ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("Two quick switches leave the loop on the pane picked last")
    func lastSwitchWins() async {
        // The first switch's exec is the slow one, as any real link makes it
        // when the second tap comes right after.
        let host = RetargetHost(delay: { $0 == "t-2" ? .milliseconds(150) : .zero })
        let session = await makeSession(host)
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })

        session.retargetMoshRawAttach(to: "w1:p2")
        try? await Task.sleep(for: .milliseconds(20))   // its exec is out
        session.retargetMoshRawAttach(to: "w1:p3")
        #expect(await waitUntil { host.landed.contains("t-3") })
        try? await Task.sleep(for: .milliseconds(300))   // anything still out lands

        #expect(host.landed.last == "t-3", "the old renderer was left on t-2: \(host.landed)")
        #expect(host.maxInFlight == 1, "one retarget exec at a time")
        await session.stop()
    }

    @Test("A switch queued behind another goes straight to the newest pane")
    func queuedSwitchesCollapseToTheNewest() async {
        let host = RetargetHost(delay: { $0 == "t-1" ? .milliseconds(150) : .zero })
        let session = await makeSession(host)

        session.retargetMoshRawAttach(to: "w1:p1")
        try? await Task.sleep(for: .milliseconds(20))
        session.retargetMoshRawAttach(to: "w1:p2")   // queued behind t-1…
        session.retargetMoshRawAttach(to: "w1:p3")   // …and overtaken before its turn
        #expect(await waitUntil { host.landed.contains("t-3") })
        try? await Task.sleep(for: .milliseconds(100))

        #expect(host.landed == ["t-1", "t-3"], "no bounce through a pane nobody wants any more")
        await session.stop()
    }

    @Test("A retarget whose exec failed is asked for again by the next focus report")
    func failedRetargetIsRetried() async {
        let host = RetargetHost(failing: ["t-2": 1])
        let session = await makeSession(host)
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })

        session.retargetMoshRawAttach(to: "w1:p2")   // the exec fails: still on t-1
        try? await Task.sleep(for: .milliseconds(100))
        #expect(host.landed == ["t-1"])

        // The poll repeats the focus every tick — that has to be a retry,
        // not "already showing it".
        session.retargetMoshRawAttach(to: "w1:p2")
        #expect(await waitUntil { host.landed.last == "t-2" },
                "the old renderer took the repeat for the pane already on screen")
        await session.stop()
    }

    @Test("A retarget the loop never came round to is asked for again by the next focus report")
    func unconfirmedRetargetIsRetried() async {
        // The loop is gone — another client took its pane over — so the
        // exec comes back, a round trip later, with nothing to say.
        let host = RetargetHost(loopRuns: false, delay: { _ in .milliseconds(60) })
        let session = await makeSession(host)
        session.retargetMoshRawAttach(to: "w1:p2")
        #expect(await waitUntil { host.execs == 1 })
        try? await Task.sleep(for: .milliseconds(150))   // its answer is in

        // The poll repeats the focus every tick: that has to ask again, not
        // take the pane for the one on screen (it is the login shell).
        host.loopRuns = true
        session.retargetMoshRawAttach(to: "w1:p2")
        #expect(await waitUntil { host.execs == 2 },
                "an exec that merely came back was taken for the pane on screen")
        await session.stop()
    }

    @Test("A switch to a pane that no longer exists never counts: the pane on screen stays, and takes the typing")
    func missingTerminalLeavesThePaneOnScreen() async throws {
        let host = RetargetHost(delay: { _ in .milliseconds(60) })
        let session = await makeSession(host)
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })
        try? await Task.sleep(for: .milliseconds(100))   // the landing is recorded
        // t-2's pane closed since the tree was read; the stale tree has it.
        host.missing = ["t-2"]
        session.retargetMoshRawAttach(to: "w1:p2")
        #expect(await waitUntil { host.execs == 2 })
        try? await Task.sleep(for: .milliseconds(150))   // its answer is in

        #expect(host.landed == ["t-1"], "the attach on screen was bounced")
        // The poll's stale report of the closed pane: not asked again.
        session.retargetMoshRawAttach(to: "w1:p2")
        try? await Task.sleep(for: .milliseconds(200))
        #expect(host.execs == 2, "the closed pane was asked for again")
        await session.stop()
    }

    @Test("The same focus reported again costs nothing once it is on screen")
    func repeatIsFree() async {
        let host = RetargetHost()
        let session = await makeSession(host)
        session.retargetMoshRawAttach(to: "w1:p2")
        #expect(await waitUntil { host.landed == ["t-2"] })

        for _ in 0..<3 { session.retargetMoshRawAttach(to: "w1:p2") }
        try? await Task.sleep(for: .milliseconds(100))

        #expect(host.landed == ["t-2"], "each repeat would bounce the attach — a visible flicker")
        await session.stop()
    }
}

/// mosh+herdr input for ONE pane — `deliverInput`, which the share sheet's
/// image paths and a notification's Allow / Deny / reply go through. The
/// bytes travel the mosh channel into whatever herdr is typing into, so the
/// pane has to be focused first; the check that it was looked at the
/// client's snapshot, which `selectPane` had already set — it passed before
/// the focus command left, and the reply went to the pane being left.
@Suite("mosh+herdr input for one pane")
@MainActor
struct MoshHerdrDeliverInputTests {

    /// herdr's server: two panes, each with a terminal id. `agent focus` or a
    /// mosh-mode zoom moves the focus when its exec finishes, after
    /// `focusDelay(pane)` — except for a pane in `stuck`, where the exec
    /// "succeeds" and the focus stays put.
    final class FocusingServer: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _focused: String
        private let stuck: Set<String>
        private let focusDelay: @Sendable (String) -> Duration

        init(focused: String, stuck: Set<String> = [],
             focusDelay: @escaping @Sendable (String) -> Duration = { _ in .zero }) {
            _focused = focused
            self.stuck = stuck
            self.focusDelay = focusDelay
        }

        var focused: String { lock.withLock { _focused } }

        /// Someone else — the desktop — moves the focus.
        func focus(_ pane: String) { lock.withLock { _focused = pane } }

        func run(_ command: String) async throws -> String {
            if command.contains("api snapshot") {
                let focused = self.focused
                return """
                {"id":"cli:api:snapshot","result":{"snapshot":{
                  "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
                  "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
                  "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":\(focused == "w1:p1"),"terminal_id":"t-1"},
                           {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":\(focused == "w1:p2"),"terminal_id":"t-2"}],
                  "layouts":[],
                  "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"\(focused)"}}}
                """
            }
            guard let target = HerdrCommandOrderTests.FocusServer.focusTarget(command) else { return "" }
            let delay = focusDelay(target)
            if delay > .zero { try? await Task.sleep(for: delay) }
            if !stuck.contains(target) { lock.withLock { _focused = target } }
            return ""
        }
    }

    /// The UDP side of the mosh transport: every datagram it sends, and what
    /// `probe` said at that moment — where a keystroke going out now lands.
    final class RecordingChannel: DatagramChannel, @unchecked Sendable {
        var state: NWConnection.State = .ready
        var onStateChange: (@Sendable (NWConnection.State) -> Void)?
        var onBetterPath: (@Sendable (Bool) -> Void)?
        private let lock = NSLock()
        private var _sentWhile: [String] = []
        private let probe: @Sendable () -> String

        init(probe: @escaping @Sendable () -> String) { self.probe = probe }

        var sentWhile: [String] { lock.withLock { _sentWhile } }

        func start() {}
        func restart() {}
        func cancel() {}
        func send(_ datagram: Data) {
            let now = probe()
            lock.withLock { _sentWhile.append(now) }
        }
        func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {}
    }

    private func connection() -> ServerConnection {
        var connection = ServerConnection(
            name: "deliver",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .mosh
        connection.multiplexer = .herdr
        return connection
    }

    private func mosh(_ channel: RecordingChannel) throws -> MoshTransport {
        let key = Data((0..<16).map { UInt8($0) })
        return try MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: channel)
    }

    private func waitUntil(_ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("Full-TUI renderer: the reply leaves only once the server has the pane focused")
    func tuiWaitsForTheServer() async throws {
        let server = FocusingServer(focused: "w1:p1",
                                    focusDelay: { $0 == "w1:p2" ? .milliseconds(300) : .zero })
        let control = HerdrControlClient(runner: server)
        control.immersiveZoom = true   // old herdr: mosh shows herdr's own TUI
        await control.pollNowForTesting()
        let channel = RecordingChannel(probe: { server.focused })
        let session = SessionHub.ActiveSession(connection: connection())
        session.installForTesting(moshTransport: try mosh(channel), herdrControl: control)

        let delivered = await session.deliverInput(Data("y".utf8), toPane: "w1:p2")

        #expect(delivered)
        #expect(await waitUntil { !channel.sentWhile.isEmpty }, "nothing was sent at all")
        #expect(channel.sentWhile.first == "w1:p2",
                "the reply went out while the server was still on \(channel.sentWhile.first ?? "?")")
        await session.stop()
    }

    @Test("Raw-attach renderer: the reply leaves only once the loop is on the pane's terminal")
    func rawAttachWaitsForTheRetarget() async throws {
        let server = FocusingServer(focused: "w1:p1")
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()   // learns the terminal ids
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { $0 == "t-2" ? .milliseconds(300) : .zero })
        let channel = RecordingChannel(probe: { host.landed.last ?? "nothing" })
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        // As the control plane wires it: the loop follows the focus.
        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })

        let delivered = await session.deliverInput(Data("y".utf8), toPane: "w1:p2")

        #expect(delivered)
        #expect(await waitUntil { !channel.sentWhile.isEmpty }, "nothing was sent at all")
        #expect(channel.sentWhile.first == "t-2",
                "the reply went out while the loop was still attached to \(channel.sentWhile.first ?? "?")")
        await session.stop()
    }

    @Test("Raw-attach renderer: a reply for a pane the loop never came round to isn't sent, and says so")
    func rawAttachGoneLoopSendsNothing() async throws {
        let server = FocusingServer(focused: "w1:p1")
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()   // learns the terminal ids
        // Every exec answers a round trip later, as over any real link.
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })
        try? await Task.sleep(for: .milliseconds(150))   // the landing is recorded
        // Another client takes the pane over: the loop's attach exits on its
        // own, the loop ends, and mosh shows the login shell.
        host.loopRuns = false

        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p2")
        try? await Task.sleep(for: .milliseconds(100))   // anything still on its way

        #expect(host.landed.last == "t-2", "the switch was asked for")
        #expect(!delivered, "reported as sent")
        #expect(channel.sentWhile.isEmpty,
                "typed into \(channel.sentWhile.first ?? "?") as a command line")
        #expect(session.herdrNotice != nil, "the failure was silent")
        await session.stop()
    }

    /// A session on the raw-attach renderer with the loop on `w1:p1`'s
    /// terminal, its sidecar ending at `host`.
    private func rawAttachOnFirstPane(_ host: MoshRawAttachRetargetTests.RetargetHost,
                                      _ channel: RecordingChannel) async throws -> SessionHub.ActiveSession {
        let server = FocusingServer(focused: "w1:p1")
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()   // learns the terminal ids
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })
        try? await Task.sleep(for: .milliseconds(150))   // the landing is recorded
        return session
    }

    @Test("Raw-attach renderer: a reply for the pane on screen goes once the loop is seen still on it")
    func rawAttachReplyToThePaneOnScreen() async throws {
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await rawAttachOnFirstPane(host, channel)

        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p1")

        #expect(delivered)
        #expect(await waitUntil { !channel.sentWhile.isEmpty }, "nothing was sent at all")
        #expect(channel.sentWhile.first == "t-1")
        #expect(host.checks == 1, "the loop is asked about once")
        #expect(host.landed == ["t-1"], "asked, never re-attached: a retarget would take the pane back")
        await session.stop()
    }

    @Test("Raw-attach renderer: a reply for the pane on screen isn't sent once the loop has ended under it")
    func rawAttachReplyAfterTheLoopEnded() async throws {
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await rawAttachOnFirstPane(host, channel)
        // Another client takes the pane over (or the server restarts): the
        // loop's attach exits on its own, the loop ends, and mosh shows the
        // login shell — with nothing here having switched anywhere.
        host.loopRuns = false

        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p1")
        try? await Task.sleep(for: .milliseconds(100))   // anything still on its way

        #expect(!delivered, "reported as sent")
        #expect(channel.sentWhile.isEmpty,
                "typed into \(channel.sentWhile.first ?? "?") as a command line")
        #expect(session.herdrNotice != nil, "the failure was silent")
        #expect(host.checks == 1, "nothing asked whether the loop was still there")
        #expect(host.landed == ["t-1"], "asked, never re-attached")
        await session.stop()
    }

    @Test("A reply for a herdr pane with no herdr control plane isn't typed into the shell")
    func noControlPlaneSendsNothing() async throws {
        // mosh's control plane never comes up (the raw-attach loop still
        // waits for its first pane): the one shell there is was where the
        // reply went — a command line, or whichever pane attached first —
        // said sent.
        let channel = RecordingChannel(probe: { "the login shell" })
        let session = SessionHub.ActiveSession(connection: connection())
        session.installForTesting(moshTransport: try mosh(channel))
        session.moshSidecarDeliveryWait = 0.4

        let began = ContinuousClock.now
        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p2")
        try? await Task.sleep(for: .milliseconds(100))   // anything still on its way

        #expect(!delivered, "reported as sent")
        #expect(ContinuousClock.now - began < .seconds(3))
        #expect(channel.sentWhile.isEmpty, "typed into \(channel.sentWhile.first ?? "?") as a command line")
        await session.stop()
    }

    /// B248: a reply that comes before herdr's control plane is up — the
    /// first second or two of every mosh connect, and while it is rebuilt
    /// (a notification's Allow sent as the phone reconnects) — was answered
    /// not sent at once. It waits for the control plane, as a tmux pane's
    /// reply waits for the `-CC` sidecar, and goes once the server has its
    /// pane focused.
    @Test("A reply sent before herdr's control plane is up waits for it and reaches its pane")
    func replyWaitsForTheControlPlane() async throws {
        let server = FocusingServer(focused: "w1:p1")
        let channel = RecordingChannel(probe: { server.focused })
        let session = SessionHub.ActiveSession(connection: connection())
        let transport = try mosh(channel)
        session.installForTesting(moshTransport: transport)

        let reply = Task { await session.deliverInput(Data("y".utf8), toPane: "w1:p2") }
        // The control plane comes up a moment later, its tree a poll after.
        try await Task.sleep(for: .milliseconds(300))
        #expect(channel.sentWhile.isEmpty, "the reply went down mosh before anything could place it")
        let control = HerdrControlClient(runner: server)
        control.immersiveZoom = true   // old herdr: mosh shows herdr's own TUI
        session.installForTesting(moshTransport: transport, herdrControl: control)
        await control.pollNowForTesting()

        #expect(await reply.value, "said not sent")
        #expect(await waitUntil { !channel.sentWhile.isEmpty }, "nothing was sent at all")
        #expect(channel.sentWhile.first == "w1:p2",
                "the reply went out while the server was on \(channel.sentWhile.first ?? "?")")
        await session.stop()
    }

    @Test("A reply for a pane herdr's control plane doesn't list is said not sent once it is up")
    func replyForAPaneTheControlPlaneDoesNotList() async throws {
        let server = FocusingServer(focused: "w1:p1")
        let channel = RecordingChannel(probe: { server.focused })
        let session = SessionHub.ActiveSession(connection: connection())
        let transport = try mosh(channel)
        session.installForTesting(moshTransport: transport)

        let reply = Task { await session.deliverInput(Data("y".utf8), toPane: "w1:p9") }
        try await Task.sleep(for: .milliseconds(150))
        let control = HerdrControlClient(runner: server)
        session.installForTesting(moshTransport: transport, herdrControl: control)
        let began = ContinuousClock.now
        await control.pollNowForTesting()

        #expect(!(await reply.value))
        #expect(ContinuousClock.now - began < .seconds(2))
        #expect(channel.sentWhile.isEmpty)
        await session.stop()
    }

    @Test("A reply for a herdr pane on a host with no herdr is refused at once")
    func noHerdrRefusesAtOnce() async throws {
        let channel = RecordingChannel(probe: { "the login shell" })
        let session = SessionHub.ActiveSession(connection: connection())
        session.installForTesting(moshTransport: try mosh(channel))
        session.degrade = DegradeNotice.forMissing(.herdr)

        let began = ContinuousClock.now
        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p2")

        #expect(!delivered)
        #expect(ContinuousClock.now - began < .seconds(1))
        #expect(channel.sentWhile.isEmpty, "typed into the plain shell")
        await session.stop()
    }

    @Test("Raw-attach renderer: a reply for a pane that is gone is answered not sent at once, and the pane on screen keeps its typing")
    func replyForAGonePaneIsRefusedAtOnce() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)
        host.missing = ["t-2"]

        let began = ContinuousClock.now
        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p2")
        #expect(!delivered, "reported as sent")
        #expect(ContinuousClock.now - began < .seconds(2), "waited out the switch's 5 s")
        #expect(host.landed == ["t-1"], "the pane on screen was bounced")
        #expect(channel.sentWhile.isEmpty, "typed into \(channel.sentWhile)")
        session.sendInput(Data("n".utf8))
        #expect(await waitUntil { channel.sentWhile == ["t-1"] }, "the pane on screen lost its typing")
        await session.stop()
    }

    @Test("Raw-attach renderer: typing before the first switch lands goes nowhere — not into whichever pane attaches first")
    func rawAttachTypingBeforeTheFirstSwitchIsDropped() async throws {
        let server = FocusingServer(focused: "w1:p1")
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()   // learns the terminal ids
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        // The loop is booted and shows no pane yet (the sidecar's first
        // report hasn't pointed it anywhere): whatever is typed now went to
        // the tty, and the first attach read it as its own.
        session.sendInput(Data("y\r".utf8))
        try? await Task.sleep(for: .milliseconds(100))
        #expect(channel.sentWhile.isEmpty, "typed into \(channel.sentWhile) before any pane was on screen")
        #expect(session.herdrNotice != nil, "dropped without a word")

        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })
        try? await Task.sleep(for: .milliseconds(100))   // the landing is recorded
        session.sendInput(Data("n".utf8))
        #expect(await waitUntil { channel.sentWhile == ["t-1"] })
        await session.stop()
    }

    // MARK: - The renderer stops: who brings it back

    /// herdr's server with panes that can close and terminal ids that a
    /// restart renews; `agent focus` moves the focus.
    final class TreeServer: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _panes: [(pane: String, terminal: String)]
        private var _focused: String

        init(panes: [(String, String)], focused: String) {
            _panes = panes.map { (pane: $0.0, terminal: $0.1) }
            _focused = focused
        }

        /// The pane closes; herdr focuses the next one.
        func close(_ pane: String) {
            lock.withLock {
                _panes.removeAll { $0.pane == pane }
                if _focused == pane { _focused = _panes.first?.pane ?? "" }
            }
        }

        /// The server restarts: the same panes, new terminal ids.
        func restart() {
            lock.withLock { _panes = _panes.map { (pane: $0.pane, terminal: $0.terminal + "r") } }
        }

        /// The server stops: `api snapshot` answers that none runs.
        func stopServer() { lock.withLock { _running = false } }
        private var _running = true

        func run(_ command: String) async throws -> String {
            if let target = HerdrCommandOrderTests.FocusServer.focusTarget(command) {
                lock.withLock { _focused = target }
                return ""
            }
            guard command.contains("api snapshot") else { return "" }
            if !lock.withLock({ _running }) {
                return #"{"error":{"code":"server_not_running","message":"no herdr server is running"},"id":"cli:api:snapshot"}"#
            }
            return lock.withLock {
                let panes = _panes.map { entry in
                    #"{"pane_id":"\#(entry.pane)","tab_id":"w1:t1","workspace_id":"w1","focused":\#(entry.pane == _focused),"terminal_id":"\#(entry.terminal)"}"#
                }.joined(separator: ",")
                return """
                {"id":"cli:api:snapshot","result":{"snapshot":{
                  "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
                  "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":\(_panes.count)}],
                  "panes":[\(panes)],
                  "layouts":[],
                  "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"\(_focused)"}}}
                """
            }
        }
    }

    /// A session on the raw-attach renderer, wired as the hub wires it, the
    /// loop on `w1:p1`'s terminal and watched.
    private func watchedOnFirstPane(_ server: TreeServer,
                                    _ host: MoshRawAttachRetargetTests.RetargetHost,
                                    _ channel: RecordingChannel) async throws -> SessionHub.ActiveSession {
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()   // learns the terminal ids
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        control.onPanePicked = { [weak session] pane in session?.retargetMoshRawAttach(to: pane, user: true) }
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] && host.watches == 1 })
        return session
    }

    @Test("Raw-attach renderer: a pane another device takes over is said, nothing typed reaches a shell, and only the person takes it back")
    func takenOverIsSaidAndTakenBackOnATap() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)

        host.park("herdr: server shut down: terminal attach taken over")
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") },
                "nothing said the pane is on another device")
        #expect(session.herdrNoticeAction == String(localized: "Open here"))
        // Typed and dictated from here: dropped, never into a tty or a shell.
        session.sendInput(Data("rm -rf build\r".utf8))
        // The poll repeats the focus, and the desktop moves it: neither takes
        // the pane from the device that has it.
        session.retargetMoshRawAttach(to: "w1:p1")
        session.retargetMoshRawAttach(to: "w1:p2")
        try? await Task.sleep(for: .milliseconds(300))
        #expect(host.execs == 1, "a focus report took a pane back: \(host.landed)")
        #expect(channel.sentWhile.isEmpty, "typed into \(channel.sentWhile)")

        // One tap: taken back, the notice gone, and typing reaches the pane.
        session.runHerdrNoticeAction()
        #expect(await waitUntil { host.landed == ["t-1", "t-1"] })
        #expect(await waitUntil { session.herdrNotice == nil }, "the notice outlived the take back")
        session.sendInput(Data("y".utf8))
        #expect(await waitUntil { channel.sentWhile == ["t-1"] })
        await session.stop()
    }

    @Test("Raw-attach renderer: a pane taken over by another device and then closed is left for the focused one, by itself")
    func takenOverThenClosedFollowsTheFocus() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)
        host.park("herdr: server shut down: terminal attach taken over")
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") })

        // The other device closes it; the poll reads the tree without it.
        server.close("w1:p1")
        await session.herdrControl?.pollNowForTesting()
        #expect(await waitUntil { host.landed == ["t-1", "t-2"] },
                "stuck on a pane that no longer exists: \(host.landed)")
        #expect(await waitUntil { session.herdrNotice == nil }, "the takeover notice outlived its pane")
        await session.stop()
    }

    @Test("Raw-attach renderer: the pane on screen closes and the renderer goes to the focused pane by itself")
    func closedPaneFollowsTheFocus() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)

        server.close("w1:p1")
        host.park("herdr: server shut down: terminal attach ended: terminal t-1 not found")
        // Typed before the focused pane is up: dropped, said.
        try? await Task.sleep(for: .milliseconds(250))
        session.sendInput(Data("y".utf8))
        #expect(await waitUntil { host.landed == ["t-1", "t-2"] }, "the renderer never came back: \(host.landed)")
        #expect(await waitUntil { host.watches == 2 })
        session.sendInput(Data("n".utf8))
        #expect(await waitUntil { channel.sentWhile.contains("t-2") })
        #expect(!channel.sentWhile.contains("the login shell"))
        #expect(!channel.sentWhile.contains("no pane"))
        await session.stop()
    }

    @Test("Raw-attach renderer: herdr restarts and the renderer comes back on the focused pane's new terminal")
    func restartFollowsTheNewTerminal() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)

        server.restart()
        host.park("herdr: server shut down: server is shutting down")
        #expect(await waitUntil { host.landed.last == "t-1r" }, "the renderer never came back: \(host.landed)")
        session.sendInput(Data("y".utf8))
        #expect(await waitUntil { channel.sentWhile == ["t-1r"] })
        await session.stop()
    }

    @Test("Raw-attach renderer: a reply that beats the watch to a takeover reads the park and takes nothing back")
    func replyReadsTheParkBeforeTheWatch() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        // The watch's looks are slow here; the check is not.
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)

        host.park("herdr: server shut down: terminal attach taken over")
        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p1")
        #expect(!delivered, "reported as sent")
        // The next report of the same focus asks nothing of the loop.
        session.retargetMoshRawAttach(to: "w1:p1")
        try? await Task.sleep(for: .milliseconds(300))
        #expect(host.landed == ["t-1"], "the pane was taken back from the other device")
        #expect(channel.sentWhile.isEmpty)
        await session.stop()
    }

    // MARK: - Following the focus never takes a pane (round twenty-eight)

    @Test("Raw-attach renderer: following the focus onto a pane another device holds takes nothing — said, and taken only on the person's word")
    func followOntoAHeldPaneTakesNothing() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)
        // The other device's person picked w1:p2: it holds the pane, and
        // the shared focus is there.
        host.held = ["t-2"]
        session.retargetMoshRawAttach(to: "w1:p2")
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") },
                "nothing said the pane is on another device: \(host.asked)")
        #expect(host.takenFromOthers.isEmpty, "following the focus took the pane from the device that has it")
        #expect(host.held == ["t-2"])
        #expect(host.asked.last?.takeover == false)
        #expect(session.herdrNoticeAction == String(localized: "Open here"))
        // Typed from here: dropped. The poll's repeats ask nothing.
        session.sendInput(Data("y\r".utf8))
        let asked = host.asked.count
        session.retargetMoshRawAttach(to: "w1:p2")
        session.retargetMoshRawAttach(to: "w1:p1")
        try? await Task.sleep(for: .milliseconds(300))
        #expect(host.asked.count == asked, "a focus report retried the pane: \(host.asked)")
        #expect(channel.sentWhile.isEmpty, "typed into \(channel.sentWhile)")

        // "Open here": the person's word takes it.
        session.runHerdrNoticeAction()
        #expect(await waitUntil { host.takenFromOthers == ["t-2"] })
        #expect(host.asked.last?.takeover == true)
        #expect(await waitUntil { session.herdrNotice == nil }, "the notice outlived the take")
        #expect(await waitUntil { host.watches == 2 })
        session.sendInput(Data("n".utf8))
        #expect(await waitUntil { channel.sentWhile == ["t-2"] })
        await session.stop()
    }

    @Test("Raw-attach renderer: a tap or a reply takes the pane from the device that has it; following the focus never does")
    func onlyThePersonTakesOver() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2"), ("w1:p3", "t-3")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)
        host.held = ["t-2", "t-3"]
        // A tap: `--takeover`.
        session.herdrControl?.selectPane("w1:p2")
        #expect(await waitUntil { host.takenFromOthers == ["t-2"] }, "a tap didn't take the pane: \(host.asked)")
        #expect(await waitUntil { host.watches == 2 })
        // A reply for another pane: the person's choice too.
        let delivered = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p3")
        #expect(delivered)
        #expect(host.takenFromOthers == ["t-2", "t-3"])
        // The focus follows back to w1:p1, which nobody else holds: plain.
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed.last == "t-1" })
        #expect(host.asked.map(\.takeover) == [false, true, true, false], "\(host.asked)")
        await session.stop()
    }

    @Test("Raw-attach renderer: a tap on the pane a follow is on its way to takes it, though the follow was refused")
    func tapOnThePaneAFollowIsOnItsWayTo() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(80) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)
        host.held = ["t-2"]
        session.retargetMoshRawAttach(to: "w1:p2")
        try? await Task.sleep(for: .milliseconds(20))   // the follow's exec is out
        session.retargetMoshRawAttach(to: "w1:p2", user: true)
        #expect(await waitUntil { host.takenFromOthers == ["t-2"] }, "\(host.asked)")
        #expect(await waitUntil { session.herdrNotice == nil })
        #expect(await waitUntil { host.watches == 2 })
        session.sendInput(Data("y".utf8))
        #expect(await waitUntil { channel.sentWhile == ["t-2"] })
        await session.stop()
    }

    // MARK: - The start line is typed again (round twenty-eight)

    /// A session whose loop's start line was just typed: not seen running yet.
    private func booting(_ server: TreeServer, _ host: MoshRawAttachRetargetTests.RetargetHost,
                         _ channel: RecordingChannel) async throws -> SessionHub.ActiveSession {
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test",
                                  herdrRendererBooted: false)
        // No server acknowledges anything here: the wait for it is short.
        session.moshRendererRetypeAckWait = 0.05
        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        return session
    }

    @Test("Raw-attach renderer: a start line a starting shell swallowed is typed again, and no target goes before the loop runs")
    func startLineSwallowedIsTypedAgain() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(40) })
        host.bootUnanswered = 1
        let channel = RecordingChannel(probe: { "typed" })
        let session = try await booting(server, host, channel)

        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] }, "never landed: \(host.log)")
        #expect(host.log.prefix(3) == ["boot", "boot", "retarget t-1"],
                "a target went before the loop was seen running: \(host.log)")
        #expect(!channel.sentWhile.isEmpty, "the start line was never typed again")
        #expect(session.herdrNotice == nil)
        await session.stop()
    }

    @Test("Raw-attach renderer: a start line never answered goes four times, then waits for the person's Retry — same generation")
    func startLineNeverAnsweredWaitsForThePerson() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(20) })
        host.bootUnanswered = 99
        let channel = RecordingChannel(probe: { "typed" })
        let session = try await booting(server, host, channel)

        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { session.herdrNotice == String(localized: "Not attached to the pane yet — input wasn't sent") },
                "\(host.log)")
        #expect(host.log == ["boot", "boot", "boot", "boot"], "four copies, then nothing asked: \(host.log)")
        #expect(session.herdrNoticeAction == String(localized: "Retry"))
        // Retry: the line once more, the same generation; it runs.
        host.bootUnanswered = 0
        session.runHerdrNoticeAction()
        #expect(await waitUntil { host.landed == ["t-1"] }, "\(host.log)")
        #expect(host.lastRetarget.contains("-test.target"), "a new generation for a loop that never ran")
        await session.stop()
    }

    @Test("When herdr's server is down its old terminal ids go with the tree: a pick asks nothing of the loop with one")
    func staleTerminalIdsGoWithTheTree() async throws {
        let server = TreeServer(panes: [("w1:p1", "t-1"), ("w1:p2", "t-2")], focused: "w1:p1")
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { _ in .milliseconds(60) })
        let channel = RecordingChannel(probe: { host.onScreen })
        let session = try await watchedOnFirstPane(server, host, channel)
        let control = try #require(session.herdrControl)

        server.stopServer()
        await control.pollNowForTesting()
        #expect(control.serverNotRunning)
        #expect(control.terminalIds.isEmpty, "the gone tree's terminal ids: \(control.terminalIds)")
        // A pick of a pane of the gone tree (a stale row, a reply on its
        // way): no terminal to point the loop at.
        session.retargetMoshRawAttach(to: "w1:p2", user: true)
        try? await Task.sleep(for: .milliseconds(300))
        #expect(host.execs == 1, "a stale terminal id reached a retarget: \(host.landed)")
        await session.stop()
    }

    @Test("A snapshot older than the server is not taken for the focus")
    func staleSnapshotIsNotTheFocus() async throws {
        let server = FocusingServer(focused: "w1:p2")
        let control = HerdrControlClient(runner: server)
        control.immersiveZoom = true
        await control.pollNowForTesting()   // the app last saw w1:p2 focused…
        server.focus("w1:p1")                // …and the desktop has moved on since
        let channel = RecordingChannel(probe: { server.focused })
        let session = SessionHub.ActiveSession(connection: connection())
        session.installForTesting(moshTransport: try mosh(channel), herdrControl: control)

        let delivered = await session.deliverInput(Data("y".utf8), toPane: "w1:p2")

        #expect(delivered)
        #expect(await waitUntil { !channel.sentWhile.isEmpty }, "nothing was sent at all")
        #expect(channel.sentWhile.first == "w1:p2",
                "the reply went out while the server was on \(channel.sentWhile.first ?? "?")")
        await session.stop()
    }

    @Test("A pane that never comes into focus gets nothing, and the failure shows")
    func unconfirmedFocusSendsNothing() async throws {
        let server = FocusingServer(focused: "w1:p1", stuck: ["w1:p2"])
        let control = HerdrControlClient(runner: server)
        control.immersiveZoom = true
        await control.pollNowForTesting()
        let channel = RecordingChannel(probe: { server.focused })
        let session = SessionHub.ActiveSession(connection: connection())
        session.installForTesting(moshTransport: try mosh(channel), herdrControl: control)

        let delivered = await session.deliverInput(Data("y".utf8), toPane: "w1:p2")
        try? await Task.sleep(for: .milliseconds(100))   // anything still on its way

        #expect(!delivered, "reported as sent")
        #expect(channel.sentWhile.isEmpty, "typed into \(channel.sentWhile.first ?? "?") instead")
        #expect(session.herdrNotice != nil, "the failure was silent")
        await session.stop()
    }

    // MARK: - Typing during a switch

    /// The mosh side of the SSH rule (`HerdrSSHFrameModeTests`): keys typed
    /// right after a pane switch went down the mosh channel at once, into
    /// whatever herdr was still typing into — the pane being LEFT — until the
    /// retarget (raw attach) or the zoom (full TUI) landed.
    @Test("Raw-attach renderer: typing right after a switch never reaches the pane being left")
    func rawAttachTypingDuringASwitchIsDropped() async throws {
        let server = FocusingServer(focused: "w1:p1")
        let control = HerdrControlClient(runner: server)
        await control.pollNowForTesting()   // learns the terminal ids
        let host = MoshRawAttachRetargetTests.RetargetHost(delay: { $0 == "t-2" ? .milliseconds(300) : .zero })
        let channel = RecordingChannel(probe: { host.landed.last ?? "nothing" })
        let connection = connection()
        let session = SessionHub.ActiveSession(connection: connection)
        session.installForTesting(moshTransport: try mosh(channel),
                                  sidecarSSH: SSHSession(connection: connection, transport: host),
                                  herdrControl: control,
                                  herdrRawAttachRenderer: "\(connection.id.uuidString)-test")
        control.onFocusedPaneChanged = { [weak session] pane in session?.retargetMoshRawAttach(to: pane) }
        session.retargetMoshRawAttach(to: "w1:p1")
        #expect(await waitUntil { host.landed == ["t-1"] })
        try? await Task.sleep(for: .milliseconds(100))   // the landing is recorded

        control.selectPane("w1:p2")
        session.sendInput(Data("y".utf8))       // typed in the same breath
        session.sendInput(Data("\u{1b}[A".utf8))   // an arrow, too
        #expect(await waitUntil { host.landed.last == "t-2" })
        try? await Task.sleep(for: .milliseconds(100))   // anything typed still on its way

        #expect(!channel.sentWhile.contains("t-1"), "typed after the switch, it went into the pane being left")
        #expect(session.herdrNotice != nil, "the keys were dropped without a word")

        // Once the loop is on the new terminal, typing reaches it.
        session.sendInput(Data("n".utf8))
        #expect(await waitUntil { channel.sentWhile.contains("t-2") })
        await session.stop()
    }

    @Test("Full-TUI renderer: typing right after a switch never reaches the pane being left")
    func tuiTypingDuringASwitchIsDropped() async throws {
        let server = FocusingServer(focused: "w1:p1",
                                    focusDelay: { $0 == "w1:p2" ? .milliseconds(300) : .zero })
        let control = HerdrControlClient(runner: server)
        control.immersiveZoom = true   // old herdr: mosh shows herdr's own TUI
        await control.pollNowForTesting()
        let channel = RecordingChannel(probe: { server.focused })
        let session = SessionHub.ActiveSession(connection: connection())
        session.installForTesting(moshTransport: try mosh(channel), herdrControl: control)

        control.selectPane("w1:p2")
        session.sendInput(Data("y".utf8))   // typed before the zoom has moved the focus
        #expect(await waitUntil { server.focused == "w1:p2" })
        await control.drainCommandsForTesting()
        try? await Task.sleep(for: .milliseconds(100))

        #expect(!channel.sentWhile.contains("w1:p1"), "typed after the switch, it went into the pane being left")
        #expect(session.herdrNotice != nil, "the keys were dropped without a word")

        // The zoom has landed: typing reaches the pane it focused.
        session.sendInput(Data("n".utf8))
        #expect(await waitUntil { channel.sentWhile.contains("w1:p2") })
        await session.stop()
    }

    /// B267: the raw-attach loop was a line typed into the mosh shell once
    /// mosh was up, and a shell still starting swallowed it (Powerlevel10k's
    /// instant prompt flushes pending input): seconds of a bare shell until
    /// the check found no loop and typed it again. It is mosh-server's own
    /// program now, the login shell after it, and nothing is typed.
    @Test("the raw-attach loop is mosh-server's program, nothing typed into the mosh shell")
    func theLoopIsMoshServersProgram() async throws {
        let channel = ShellDatagramChannel()
        let rig = MoshSessionRig(multiplexer: .herdr, answer: { command in
            if command.contains("command -v tmux mosh-server") {
                return .output("/usr/bin/mosh-server\n/usr/local/bin/herdr\n::Linux::/usr/bin/apt-get\n")
            }
            if command.contains("terminal attach --help") { return .output("MOSHPIT_RAW_ATTACH_OK\n") }
            return MoshSessionRig.answer(command)
        }, transport: { credentials, _ in
            try MoshTransport(credentials: credentials, channel: channel,
                              returnPathDeadlineNanos: 600 * 1_000_000_000)
        })
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + rig.connection.id.uuidString) }
        await rig.start()
        #expect(rig.session.viewModel.status == .connected)
        let bootstrap = try #require(rig.host.commands.first { $0.contains("mosh-server new") })
        let program = try #require(bootstrap.components(separatedBy: " -- ").last)
        #expect(program.hasPrefix("/bin/sh -c '"))
        let script = try #require(PosixShell.shPayload(String(program.dropFirst("/bin/".count))))
        #expect(script.hasPrefix("trap '' INT QUIT TSTP; "))
        #expect(script.contains("terminal attach"), "\(script)")
        #expect(script.hasSuffix("; trap - INT QUIT TSTP; exec \"${SHELL:-/bin/sh}\" -l"))
        // The last connects' loops retired before this one started.
        let cleanup = try #require(rig.host.commands.firstIndex { $0.contains(".moshpit/mosh-") && $0.contains("kill") })
        let start = try #require(rig.host.commands.firstIndex { $0.contains("mosh-server new") })
        #expect(cleanup < start, "the cleanup ran after the new renderer started")
        try? await Task.sleep(for: .milliseconds(200))
        #expect(channel.typed().isEmpty, "typed into the mosh shell: \(channel.typed().debugDescription)")
        await rig.session.stop()
    }
}
