import Foundation
import Testing
@testable import Moshpit

/// W14b: on a Rust session, the features beside the terminal borrow the
/// session's own connection (``RustSideChannel``) — never a Swift dial —
/// wait for a reconnect under way when the person asked, say so when the
/// session isn't there, and never hang. A fake core lends a fake connection
/// that answers after a link's delay.
@MainActor
@Suite("Side features borrow the Rust session's connection")
struct RustSideChannelTests {
    private static let look = RustTmuxController.Appearance(
        theme: .dracula, fontSize: 13, fontName: "system", cursorShape: .block,
        cursorColorId: "teal", cursorBlink: true)

    private static let sweep = SessionHub.ActiveSession.uploadCleanupCommand(
        days: AppSettings.shared.uploadRetentionDays)

    /// Counts from any thread.
    final class Count: @unchecked Sendable {
        private let lock = NSLock()
        private var n = 0
        func add() { lock.lock(); n += 1; lock.unlock() }
        func reset() { lock.lock(); n = 0; lock.unlock() }
        var value: Int { lock.lock(); defer { lock.unlock() }; return n }
    }

    struct Rig {
        let session: SessionHub.ActiveSession
        let driver: RustSessionDriver
        let fake: FakeCoreSessions
        let borrows: Count
    }

    /// A session on a fake core, connected, lending `ssh`. There is no
    /// Swift dial beside it any more (E4-SSH): what isn't borrowed fails.
    private func rig(_ connection: ServerConnection = ServerConnection(host: "h", username: "u"),
                     lending ssh: FakeLentSSH) async -> Rig {
        let fake = FakeCoreSessions()
        let borrows = Count()
        fake.lend = { _ in borrows.add(); return ssh }
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        let session = SessionHub.ActiveSession(connection: connection)
        let driver = RustSessionDriver(session: session, core: core, appearance: Self.look)
        session.rust = driver
        driver.side.reconnectWait = .milliseconds(600)
        driver.side.pollInterval = .milliseconds(20)
        driver.side.pingTimeout = 0.3
        let task = Task { await driver.start(appearance: Self.look, automatic: false) }
        for _ in 0..<100 where fake.connected.isEmpty { await Task.yield() }
        driver.handle(.state(state: .connected))
        await task.value
        return Rig(session: session, driver: driver, fake: fake, borrows: borrows)
    }

    /// Waits (bounded) for the upload sweep a connect starts to have run.
    private func swept(_ ssh: FakeLentSSH, times: Int = 1) async {
        for _ in 0..<200 where ssh.ran.filter({ $0 == Self.sweep }).count < times {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    private static let unreachable = SessionStateValue.failed(
        kind: .unreachable, cause: .unreachable(host: "h", port: 22, why: .noRoute),
        log: "no route", retryable: true)

    // MARK: The session's connection, never a dial

    @Test("preview, upload, maintenance and the probe borrow the session's connection; nothing dials")
    func borrowsNeverDials() async throws {
        let ssh = FakeLentSSH()
        ssh.file("/work/a.png", Data("PNG".utf8))
        ssh.commands = { command in
            command.contains("moshpit-probe") ? .init(stdout: Data("0\tf\t/work/a.png\n".utf8)) : .init()
        }
        let rig = await rig(lending: ssh)
        await swept(ssh)

        let host = try await rig.session.acquireFileTransferSSH()
        #expect(host is Lent)
        #expect(try await host.remoteStat("/work/a.png").size == 3)
        let path = try await host.uploadToUploadsDirectory(Data("img".utf8), named: "IMG-1.jpg", progress: nil)
        #expect(path == "/home/dev/.moshpit/uploads/IMG-1.jpg")
        // What host care runs a command on (``HostCare``'s platform).
        #expect(try await host.executeCommand("echo hi") == Data())
        _ = try await rig.session.acquireFileTransferSSH()

        let candidate = TerminalPathScanner.Candidate(token: "a.png", offset: 0, length: 5, directoryOnly: false)
        let hits = await rig.session.probeRemotePaths(cwd: "/work", batch: [candidate])
        #expect(hits?[0]?.path == "/work/a.png")

        #expect(rig.fake.connected.count == 1, "one session in the core, its one connection")
        #expect(rig.borrows.value >= 5)
    }

    @Test("a tapped path previews over the borrowed connection, and says why when it can't")
    func previewThroughTheSession() async throws {
        let ssh = FakeLentSSH()
        let chart = Data((0..<4096).map { UInt8($0 % 256) })
        let name = "w14b-\(UUID().uuidString).png"
        ssh.file("/work/\(name)", chart)
        let rig = await rig(lending: ssh)
        rig.session.openRemotePath("/work/\(name)")
        var phase: RemotePreviewModel.Phase?
        for _ in 0..<300 {
            phase = rig.session.remotePreview?.phase
            if case .ready = phase { break }
            if case .failed = phase { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        guard case .ready(let url)? = phase else {
            Issue.record("the preview didn't land: \(String(describing: phase))")
            return
        }
        #expect(try Data(contentsOf: url) == chart)
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())

        // The session gone: the card says so, in so many words.
        rig.driver.handle(.state(state: Self.unreachable))
        rig.session.openRemotePath("/work/\(name)")
        for _ in 0..<200 {
            phase = rig.session.remotePreview?.phase
            if case .failed = phase { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(phase == .failed(HostConnectionError.notConnected.errorDescription ?? ""))
    }

    // MARK: Reconnects

    @Test("a call the person made waits for a reconnect under way, then goes")
    func waitsForTheReconnect() async throws {
        let ssh = FakeLentSSH()
        let rig = await rig(lending: ssh)
        rig.driver.handle(.state(state: .reconnecting))
        let started = ContinuousClock.now
        let call = Task { try await rig.session.acquireFileTransferSSH().remoteHomeDirectory() }
        try await Task.sleep(for: .milliseconds(150))
        rig.driver.handle(.state(state: .connected))
        #expect(try await call.value == "/home/dev")
        #expect(ContinuousClock.now - started >= .milliseconds(150))
    }

    @Test("a reconnect that takes too long is said so — bounded, never a hang")
    func saysItIsReconnecting() async throws {
        let rig = await rig(lending: FakeLentSSH())
        rig.driver.handle(.state(state: .reconnecting))
        let started = ContinuousClock.now
        await #expect(throws: HostConnectionError.reconnecting) {
            _ = try await rig.session.acquireFileTransferSSH()
        }
        let waited = ContinuousClock.now - started
        #expect(waited >= .milliseconds(550) && waited < .seconds(2), "\(waited)")
    }

    @Test("a session that failed, ended or is closing says so at once")
    func notConnected() async throws {
        let rig = await rig(lending: FakeLentSSH())
        for state in [Self.unreachable, .ended(reason: .disconnected)] {
            rig.driver.handle(.state(state: state))
            let started = ContinuousClock.now
            await #expect(throws: HostConnectionError.notConnected) {
                _ = try await rig.session.acquireFileTransferSSH()
            }
            #expect(ContinuousClock.now - started < .milliseconds(200))
        }
        rig.driver.handle(.state(state: .connected))
        rig.session.isStopping = true
        await #expect(throws: HostConnectionError.notConnected) {
            _ = try await rig.session.acquireFileTransferSSH()
        }
        #expect(await rig.session.recheckCapabilities() == nil)
    }

    @Test("the path probe never waits: no session, no answer, at once")
    func probeNeverWaits() async throws {
        let rig = await rig(lending: FakeLentSSH())
        rig.driver.handle(.state(state: .reconnecting))
        let started = ContinuousClock.now
        await #expect(throws: HostConnectionError.reconnecting) {
            _ = try await rig.session.fileTransferSSH?.executeCommand("true")
        }
        #expect(ContinuousClock.now - started < .milliseconds(150))
        rig.driver.handle(.state(state: Self.unreachable))
        let candidate = TerminalPathScanner.Candidate(token: "a.png", offset: 0, length: 5, directoryOnly: false)
        let asked = ContinuousClock.now
        #expect(await rig.session.probeRemotePaths(cwd: "/work", batch: [candidate]) == nil)
        #expect(ContinuousClock.now - asked < .milliseconds(300))
    }

    @Test("the core a step ahead of its events: the borrow is asked again until the state catches up")
    func coreAheadOfItsEvents() async throws {
        let ssh = FakeLentSSH()
        let rig = await rig(lending: ssh)
        await swept(ssh)
        let refusals = Count()
        rig.fake.lend = { _ in
            if refusals.value < 3 {
                refusals.add()
                throw BorrowFailure.NotConnected
            }
            return ssh
        }
        #expect(try await rig.session.acquireFileTransferSSH().remoteHomeDirectory() == "/home/dev")
        #expect(refusals.value == 3)
    }

    // MARK: A connection that may be dead

    @Test("a connection quiet for a while is asked `true` first; one that answered lately isn't")
    func pingsWhenStale() async throws {
        let ssh = FakeLentSSH()
        let rig = await rig(lending: ssh)
        await swept(ssh)
        let pings = { ssh.ran.filter { $0 == "true" }.count }
        let afterConnect = pings()
        #expect(afterConnect == 1, "the sweep's borrow asked once")
        for _ in 0..<3 { _ = try await rig.session.acquireFileTransferSSH().remoteHomeDirectory() }
        #expect(pings() == afterConnect, "it answered lately")
        rig.driver.side.freshFor = .zero
        _ = try await rig.session.acquireFileTransferSSH().remoteHomeDirectory()
        #expect(pings() == afterConnect + 2, "the acquire and the call each asked")
        // A reconnect: what answered before speaks for nothing.
        rig.driver.side.freshFor = .seconds(10)
        rig.driver.handle(.state(state: .reconnecting))
        rig.driver.handle(.state(state: .connected))
        await swept(ssh, times: 2)
        let reconnected = pings()
        _ = try await rig.session.acquireFileTransferSSH().remoteHomeDirectory()
        #expect(pings() == reconnected, "the new connection's sweep just answered")
    }

    @Test("a connection that doesn't answer `true` is said so within the ping's bound")
    func deadConnection() async throws {
        let ssh = FakeLentSSH()
        let rig = await rig(lending: ssh)
        await swept(ssh)
        rig.driver.side.freshFor = .zero
        ssh.hang("true")
        let started = ContinuousClock.now
        await #expect(throws: HostConnectionError.notAnswering) {
            _ = try await rig.session.acquireFileTransferSSH()
        }
        let waited = ContinuousClock.now - started
        #expect(waited >= .milliseconds(250) && waited < .seconds(2), "\(waited)")
    }

    @Test("a read cut off by the connection's drop is tried once more; a change never is")
    func retriesReadsOnly() async throws {
        let healthy = FakeLentSSH()
        healthy.folder("/work")
        let rig = await rig(lending: healthy)
        await swept(healthy)
        // The first borrow gets a connection that dropped under the call;
        // the next, the session's new one.
        let broken = FakeLentSSH()
        broken.failure = .Sftp(why: .transport, path: "/work", log: "the channel went away")
        let lent = Count()
        rig.fake.lend = { _ in
            lent.add()
            return lent.value == 1 ? broken : healthy
        }
        let host = rig.driver.side.waiting
        #expect(try await host.listRemoteDirectory("/work").isEmpty)
        #expect(lent.value == 2, "the read went again, on the next connection")

        lent.reset()
        await #expect(throws: HostConnectionError.notAnswering) {
            try await host.createRemoteDirectory("/work/new")
        }
        #expect(lent.value == 1, "a change is never sent twice")
    }

    // MARK: Around the connect

    @Test("the upload sweep runs on every connect over SSH; plain mosh doesn't dial for it")
    func uploadSweep() async throws {
        let ssh = FakeLentSSH()
        let rig = await rig(lending: ssh)
        await swept(ssh)
        #expect(ssh.ran.contains(Self.sweep))
        rig.driver.handle(.state(state: .reconnecting))
        rig.driver.handle(.state(state: .connected))
        await swept(ssh, times: 2)
        #expect(ssh.ran.filter { $0 == Self.sweep }.count == 2)

        var mosh = ServerConnection(host: "h", username: "u")
        mosh.connectionProtocol = .mosh
        let quiet = FakeLentSSH()
        let moshRig = await self.rig(mosh, lending: quiet)
        try await Task.sleep(for: .milliseconds(200))
        #expect(moshRig.borrows.value == 0, "no side connection dialled for a sweep nobody asked for")
    }

    @Test("tmux panes on the core get host-path links; a pane's directory is asked of tmux over the session")
    func tmuxPanes() async throws {
        let ssh = FakeLentSSH()
        ssh.commands = { command in
            command.contains("display-message -p -t %1 '#{pane_current_path}'")
                ? .init(stdout: Data("/work/proj \n".utf8)) : .init()
        }
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let rig = await rig(connection, lending: ssh)
        let tmux = try #require(rig.session.tmuxController as? RustTmuxController)
        #expect(tmux.pathLinksFactory != nil, "wired as the Swift path's controllers are")
        rig.driver.handle(.panes(attach: 1, panes: [PaneValue(pane: "%1", stream: 0, active: true)]))
        #expect(tmux.coordinator(for: "%1")?.pathLinks != nil)
        // The directory as tmux printed it: a trailing space is the name's (B57).
        #expect(await tmux.panePath("%1") == "/work/proj ")
        let asked = try #require(ssh.ran.first { $0.contains("display-message") })
        #expect(asked.hasPrefix("PATH=\"$PATH:"), "tmux found where the probe looks, on an exec channel")
        // Only a pane id is spliced into the command (B146).
        #expect(await tmux.panePath("%1; rm -rf ~") == nil)
        #expect(!ssh.ran.contains { $0.contains("rm -rf") })
        rig.driver.handle(.state(state: Self.unreachable))
        #expect(await tmux.panePath("%1") == nil, "no session to ask: no answer, at once")
    }

    /// The folder the preview's sheet opened on, once it has (bounded).
    @MainActor
    private func openedFolder(_ session: SessionHub.ActiveSession) async throws -> String? {
        for _ in 0..<300 {
            switch session.remotePreview?.phase {
            case .folder(let entry)?: return entry.path
            case .failed?: return nil
            default: try await Task.sleep(for: .milliseconds(10))
            }
        }
        return nil
    }

    /// 1.0.8 (1)'s note (B743): the shortcut bar's folder key opens the
    /// folder sheet at the focused pane's working directory, or the home
    /// folder when nothing reports one.
    @Test("the folder key opens the focused pane's working directory, else the home folder")
    func folderKey() async throws {
        let ssh = FakeLentSSH()
        for folder in ["/work/proj", "/home/dev", "/work/osc"] { ssh.folder(folder) }
        ssh.commands = { command in
            command.contains("display-message -p -t %1 '#{pane_current_path}'")
                ? .init(stdout: Data("/work/proj\n".utf8)) : .init()
        }

        // tmux: its active pane's directory, as tmux says it.
        var tmuxed = ServerConnection(host: "h", username: "u")
        tmuxed.multiplexer = .tmux
        let rig = await rig(tmuxed, lending: ssh)
        let tmux = try #require(rig.session.tmuxController)
        tmux.mirror(RustSessionTests.model(active: "%1"))
        rig.session.openWorkingDirectory()
        #expect(try await openedFolder(rig.session) == "/work/proj")

        // A plain shell that reported nowhere: the home folder; once it
        // reports one (OSC 7), there.
        let plain = await self.rig(lending: ssh)
        plain.session.openWorkingDirectory()
        #expect(try await openedFolder(plain.session) == "/home/dev")
        plain.session.coordinator.onHostDirectoryChange?("file://h/work/osc")
        for _ in 0..<50 { await Task.yield() }
        plain.session.openWorkingDirectory()
        #expect(try await openedFolder(plain.session) == "/work/osc")
    }

    @Test("Install Assist's re-check reads the host over the session's connection")
    func recheck() async throws {
        let ssh = FakeLentSSH()
        ssh.commands = { command in
            command == HostCapabilities.probeCommand
                ? .init(stdout: Data("/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n".utf8)) : .init()
        }
        let rig = await rig(lending: ssh)
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps.\(rig.session.connection.id.uuidString)") }
        let caps = try #require(await rig.session.recheckCapabilities())
        #expect(caps.hasTmux)
        #expect(!caps.hasMoshServer)
    }
}
