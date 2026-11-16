import Foundation
import Testing
import CryptoKit
@testable import Moshpit

/// A registry that records what it was asked and answers from what the test
/// set: the Rust session path's Swift side, without the core.
final class FakeCoreSessions: CoreSessionsProtocol, @unchecked Sendable {
    var connected: [SessionSpec] = []
    var intents: [(UInt64, TmuxIntent)] = []
    var answers: [(UInt64, UInt64, Bool)] = []
    var disconnected: [UInt64] = []
    var reconnected: [UInt64] = []
    var foreground: [Bool] = []
    var visible: [UInt64?] = []
    var faults: [(UInt64, InjectedFault)] = []
    var flushes: UInt64 = 0
    var tmux: TmuxReadModel?
    var herdr: HerdrReadModel?
    var herdrIntents: [(UInt64, HerdrIntent)] = []
    var delivered: [(UInt64, String, Data, Bool)] = []
    var pasted: [(UInt64, String, String)] = []
    var unlocked = 0
    var nextId: UInt64 = 7
    var nextToken: UInt64 = 100

    func answerHostKey(session: UInt64, prompt: UInt64, trust: Bool) { answers.append((session, prompt, trust)) }
    var questionAnswers: [(UInt64, UInt64, String?)] = []
    func answerQuestion(session: UInt64, prompt: UInt64, answer: String?) {
        questionAnswers.append((session, prompt, answer))
    }
    func connect(spec: SessionSpec) -> UInt64 { connected.append(spec); return nextId }
    func disconnect(session: UInt64) { disconnected.append(session) }
    func flush() -> UInt64 { flushes += 1; return flushes }
    func injectFault(session: UInt64, fault: InjectedFault) { faults.append((session, fault)) }
    func networkChanged() {}
    func reconnect(session: UInt64) { reconnected.append(session) }
    /// The specs a session was handed as its connection was saved (B329).
    var respecs: [(UInt64, SessionSpec)] = []
    func respec(session: UInt64, spec: SessionSpec) { respecs.append((session, spec)) }
    func setForeground(foreground: Bool) { self.foreground.append(foreground) }
    func setVisible(session: UInt64?) { visible.append(session) }
    func setPet(pet: PetMachine?) {}
    func switchToSsh(session: UInt64) {}
    var repainted: [UInt64] = []
    func repaint(session: UInt64) { repainted.append(session) }
    /// What the core refuses of tmux, as the core would (crates/session
    /// `TmuxAction::refusal`): nothing is recorded as sent then.
    var refuseTmux: ((TmuxIntent) -> TmuxRefused?)?
    func tmux(session: UInt64, intent: TmuxIntent) throws {
        if let refused = refuseTmux?(intent) { throw refused }
        intents.append((session, intent))
    }
    func tmuxView(session: UInt64) -> TmuxReadModel? { tmux }
    func view(session: UInt64) -> SessionReadModel? { nil }
    /// Home's record: what the page asked with, and what to answer.
    var homeAsked: [(connections: [HomeConnection], agents: Bool, version: String, sidebar: Bool,
                     locale: String)] = []
    var home = #"{"op":"home","connections":[],"version":"v"}"#
    func homeMessage(connections: [HomeConnection], agents: AgentMachine?, version: String,
                     sidebar: Bool, locale: String) -> String {
        homeAsked.append((connections, agents != nil, version, sidebar, locale))
        return home
    }
    func herdr(session: UInt64, intent: HerdrIntent) -> UInt64 {
        herdrIntents.append((session, intent)); nextToken += 1; return nextToken
    }
    func herdrCancelTask(session: UInt64, token: UInt64) {}
    func herdrView(session: UInt64) -> HerdrReadModel? { herdr }
    func paste(session: UInt64, pane: String, text: String) { pasted.append((session, pane, text)) }
    func deliver(session: UInt64, pane: String, bytes: Data, paste: Bool) -> UInt64 {
        delivered.append((session, pane, bytes, paste)); nextToken += 1; return nextToken
    }
    func deviceUnlocked() { unlocked += 1 }
    /// What a borrow gets (W14b's tests lend a fake connection); refused
    /// as not connected when unset.
    var lend: (@Sendable (UInt64) async throws -> BorrowedSsh)?
    func borrowSsh(session: UInt64) async throws -> BorrowedSsh {
        guard let lend else { throw BorrowFailure.NotConnected }
        return try await lend(session)
    }
}

/// A listener that keeps what it heard.
@MainActor
final class Heard: RustSessionListener {
    var events: [SessionEventKind] = []
    func handle(_ event: SessionEventKind) { events.append(event) }
}

@MainActor
@Suite("Rust sessions: the app's side")
struct RustSessionTests {

    // MARK: Hosts

    @Test("property-list values cross to the store and back as they were")
    func plistNodesRoundTrip() throws {
        let date = Date(timeIntervalSinceReferenceDate: 784_000_000.5)
        let object: [String: Any] = [
            "data": Data([1, 2, 3]), "text": "héllo", "yes": true, "n": 42, "x": 1.5,
            "when": date, "list": [1, "two", ["nested": false]],
        ]
        let value = try #require(PlistNodes.value(object))
        let back = try #require(PlistNodes.object(value) as? NSDictionary)
        #expect(back["data"] as? Data == Data([1, 2, 3]))
        #expect(back["text"] as? String == "héllo")
        #expect(back["yes"] as? Bool == true)
        #expect((back["n"] as? NSNumber)?.int64Value == 42)
        #expect((back["x"] as? NSNumber)?.doubleValue == 1.5)
        #expect(back["when"] as? Date == date)
        let list = try #require(back["list"] as? [Any])
        #expect((list[0] as? NSNumber)?.intValue == 1)
        #expect(list[1] as? String == "two")
        #expect((list[2] as? NSDictionary)?["nested"] as? Bool == false)
        // A Bool stays a Bool, an Int an Int: the store compares them as
        // Foundation does.
        guard case .bool(true)? = PlistNodes.value(true)?.nodes.first else {
            Issue.record("a Bool crossed as something else"); return
        }
        guard case .int(7)? = PlistNodes.value(7)?.nodes.first else {
            Issue.record("an Int crossed as something else"); return
        }
    }

    @Test("the preferences host reads and writes UserDefaults")
    func preferencesHost() throws {
        let name = "w14.rust.prefs.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let host = RustPreferencesHost(defaults: defaults, protectedData: ProtectedDataFlag(fixed: false))
        #expect(host.get(key: "hostcaps.X") == nil)
        host.set(key: "hostcaps.X", value: PlistNodes.value(Data("{}".utf8)))
        #expect(defaults.data(forKey: "hostcaps.X") == Data("{}".utf8))
        #expect(host.get(key: "hostcaps.X") == PlistNodes.value(Data("{}".utf8)))
        host.set(key: "hostcaps.X", value: nil)
        #expect(defaults.object(forKey: "hostcaps.X") == nil)
        #expect(!host.protectedDataAvailable())
    }

    @Test("the keychain host speaks the store's failures")
    func keychainHost() throws {
        // Its own launch table: what a seed trusts for this process is no
        // part of the item this test reads back.
        let host = RustKeychainHost(backend: InMemoryKeychainBackend(seed: [:]), launchTrust: LaunchTrust.Table())
        #expect(throws: KeychainFailure.NotFound) { try host.load(account: "nope", prompt: "") }
        try host.save(account: "moshpit.knownHosts", secret: Data("{\"a:22\":\"SHA256:a\"}".utf8),
                      requireBiometry: false)
        #expect(host.exists(account: "moshpit.knownHosts"))
        let read = try host.load(account: "moshpit.knownHosts", prompt: "")
        let map = try JSONSerialization.jsonObject(with: read) as? [String: String]
        #expect(map == ["a:22": "SHA256:a"])
        try host.delete(account: "moshpit.knownHosts")
        #expect(!host.exists(account: "moshpit.knownHosts"))
    }

    @Test("a host trusted for this launch is read but never written")
    func launchTrustOverlay() throws {
        let backend = InMemoryKeychainBackend(seed: [:])
        // The test's own table: the process's (``LaunchTrust/shared``) never
        // learns `rig.test:2299`, so no later reader sees it (T7).
        let trusted = LaunchTrust.Table()
        let host = RustKeychainHost(backend: backend, launchTrust: trusted)
        trusted.trust(host: "rig.test", port: 2299, fingerprint: "SHA256:rig")
        // Nothing stored yet: the overlay alone.
        let first = try host.load(account: "moshpit.knownHosts", prompt: "")
        #expect((try JSONSerialization.jsonObject(with: first) as? [String: String])?["rig.test:2299"] == "SHA256:rig")
        // A write with the seed in it (the core trusting another host)
        // stores only the other host.
        let written = try JSONSerialization.data(withJSONObject: ["rig.test:2299": "SHA256:rig",
                                                                  "b:22": "SHA256:b"])
        try host.save(account: "moshpit.knownHosts", secret: written, requireBiometry: false)
        let stored = try backend.load(account: "moshpit.knownHosts", prompt: "")
        #expect(try JSONSerialization.jsonObject(with: stored) as? [String: String] == ["b:22": "SHA256:b"])
        #expect(LaunchTrust.fingerprint(host: "rig.test", port: 2299) == nil, "the trust leaked to the process")
    }

    @Test("a connection becomes the core's spec")
    func spec() {
        var connection = ServerConnection(host: "box.local", port: 2222, username: "me",
                                          connectionProtocol: .mosh, moshServerPath: "/opt/mosh-server",
                                          tmuxPath: "/opt/tmux")
        connection.multiplexer = .tmux
        connection.useSOCKSProxy = true
        connection.socksProxyHost = "proxy"
        let spec = RustSessionDriver.spec(for: connection, grid: (cols: 70, rows: 40), udp: (60001, 60999))
        #expect(spec.connectionId == connection.id.uuidString)
        #expect(spec.user == "me" && spec.host == "box.local" && spec.port == 2222)
        #expect(spec.mosh && spec.multiplexer == .tmux)
        #expect(spec.proxyHost == "proxy" && spec.proxyPort == nil)
        #expect(spec.tmuxPath == "/opt/tmux" && spec.moshServerPath == "/opt/mosh-server")
        #expect(spec.moshPortLow == 60001 && spec.moshPortHigh == 60999)
        #expect(spec.cols == 70 && spec.rows == 40)
        connection.useSOCKSProxy = false
        #expect(RustSessionDriver.spec(for: connection, grid: (80, 24)).proxyHost == nil)
    }

    /// B430: Settings › UDP ports said "for every connection", but a
    /// connect asked mosh-server for the range the connection stored at its
    /// last save — a narrowed range never reached a saved connection, and
    /// mosh landed outside the firewall: connected, black.
    @Test("a mosh connect asks for Settings' UDP ports, not the ones the connection last saved")
    func specTakesSettingsUDPRange() {
        var connection = ServerConnection(host: "box.local", username: "me", connectionProtocol: .mosh)
        connection.moshPortRangeStart = 60001
        connection.moshPortRangeEnd = 60999
        let spec = RustSessionDriver.spec(for: connection, grid: (80, 24), udp: (60000, 60010))
        let ports = (spec.moshPortLow, spec.moshPortHigh)
        #expect(ports.0 == 60000)
        #expect(ports.1 == 60010)
    }

    @Test("a Secure Enclave key's public half is an authorized_keys line")
    func openSSHLine() throws {
        let key = P256.Signing.PrivateKey()
        let line = SSHService.openSSHLine(of: key.publicKey)
        #expect(line.hasPrefix("ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABB"))
        let blob = try #require(Data(base64Encoded: String(line.dropFirst("ecdsa-sha2-nistp256 ".count))))
        #expect(blob.count == 4 + 19 + 4 + 8 + 4 + 65)
        #expect(blob.suffix(65) == key.publicKey.x963Representation)
    }

    // MARK: Routing

    @Test("events reach their session in order; a flush answers its caller")
    func routing() {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        let one = Heard(), two = Heard()
        core.listen(1, one)
        core.listen(2, two)
        var flushed = 0
        core.flush { flushed += 1 }
        core.deliver([
            SessionEvent(session: 1, kind: .state(state: .connecting)),
            SessionEvent(session: 2, kind: .treeChanged),
            SessionEvent(session: 1, kind: .state(state: .connected)),
            SessionEvent(session: 0, kind: .flushed(token: 1)),
            SessionEvent(session: 3, kind: .attachStalled),
        ])
        #expect(one.events == [.state(state: .connecting), .state(state: .connected)])
        #expect(two.events == [.treeChanged])
        #expect(flushed == 1)
        core.forget(1)
        core.deliver([SessionEvent(session: 1, kind: .attachStalled)])
        #expect(one.events.count == 2)
    }

    // MARK: The driver

    private static let look = RustTmuxController.Appearance(
        theme: .dracula, fontSize: 13, fontName: "system", cursorShape: .block,
        cursorColorId: "teal", cursorBlink: true)

    private func driver(_ connection: ServerConnection) -> (RustSessionDriver, SessionHub.ActiveSession, FakeCoreSessions) {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        let session = SessionHub.ActiveSession(connection: connection)
        let driver = RustSessionDriver(session: session, core: core, appearance: Self.look)
        session.rust = driver
        return (driver, session, fake)
    }

    /// Starts the driver the way the hub does, up to the core's connect.
    private func started(_ driver: RustSessionDriver, _ fake: FakeCoreSessions,
                         automatic: Bool = false) async -> Task<Void, Never> {
        let calls = fake.connected.count + fake.reconnected.count
        let task = Task { await driver.start(appearance: Self.look, automatic: automatic) }
        for _ in 0..<100 where fake.connected.count + fake.reconnected.count == calls {
            await Task.yield()
        }
        return task
    }

    /// B396: a plain shell's keys typed while the way back found the
    /// connection dead weren't sent — said in the banner once the session is
    /// live again, for a moment; other reasons for unsent input say nothing
    /// on screen, and a notice herdr shows is never covered.
    @Test("keys a return couldn't send are said once the session is back, briefly")
    func inputLostNotice() async {
        let wait = SessionHub.ActiveSession.inputLostWait
        let shown = SessionHub.ActiveSession.inputLostShown
        SessionHub.ActiveSession.inputLostShown = .milliseconds(300)
        defer {
            SessionHub.ActiveSession.inputLostWait = wait
            SessionHub.ActiveSession.inputLostShown = shown
        }
        let lost = String(localized: "What you just typed wasn't sent: the connection dropped, and it's back now")
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        for why: DiscardedValue in [.notLive, .paneGone, .sessionEnded, .notAttached] {
            driver.handle(.inputDiscarded(stream: 1, bytes: 3, why: why))
        }
        try? await Task.sleep(for: .milliseconds(150))
        let quiet = session.herdrNotice
        #expect(quiet == nil, "only a return's lost keys are said")

        driver.handle(.state(state: .reconnecting))
        driver.handle(.inputDiscarded(stream: 1, bytes: 5, why: .reconnected))
        try? await Task.sleep(for: .milliseconds(150))
        let early = session.herdrNotice
        #expect(early == nil, "not while it is still reconnecting")
        driver.handle(.state(state: .connected))
        let said = await eventually { session.herdrNotice == lost }
        #expect(said)
        let cleared = await eventually { session.herdrNotice == nil }
        #expect(cleared, "it clears itself")

        session.rustSetHerdrNotice("herdr says", action: nil)
        driver.handle(.inputDiscarded(stream: 1, bytes: 5, why: .reconnected))
        try? await Task.sleep(for: .milliseconds(200))
        let kept = session.herdrNotice
        #expect(kept == "herdr says", "herdr's notice is never covered")
    }

    /// Whether `predicate` holds within `seconds`.
    private func eventually(seconds: Double = 3, _ predicate: () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }

    @Test("the core's states drive the view model; a reconnect keeps its frame")
    func states() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let first = await started(driver, fake)
        #expect(fake.connected.count == 1)
        #expect(session.viewModel.status == .connecting)
        driver.handle(.state(state: .connected))
        await first.value
        #expect(session.viewModel.connState == .live)
        driver.handle(.state(state: .reconnecting))
        #expect(session.viewModel.connState == .reconnecting)
        // An automatic attempt that fails: offline, and no modal.
        driver.handle(.state(state: .failed(kind: .unreachable,
                                            cause: .unreachable(host: "h", port: 22, why: .noRoute),
                                            log: "no route", retryable: true)))
        #expect(session.viewModel.connState == .offline)
        #expect(session.viewModel.errorMessage == nil)
        // The person's own try again fails: the card.
        let retry = await started(driver, fake)
        #expect(fake.reconnected == [7])
        driver.handle(.state(state: .connecting))
        driver.handle(.state(state: .failed(kind: .credentials,
                                            cause: .credentials(why: .rejected(user: "u")),
                                            log: "refused", retryable: false)))
        await retry.value
        // The app's own words, never the core's log.
        #expect(session.viewModel.errorMessage == SSHError.authenticationFailed.description)
    }

    @Test("an automatic redial that ends in a failure nothing will retry says why (B163, B335)")
    func failureNothingRetries() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        // The host's password changed while the phone was away: the redial
        // is refused, and the core won't try again on its own.
        driver.handle(.state(state: .reconnecting))
        driver.handle(.state(state: .failed(kind: .credentials,
                                            cause: .credentials(why: .rejected(user: "u")),
                                            log: "refused", retryable: false)))
        #expect(session.viewModel.connState == .offline)
        #expect(session.viewModel.errorMessage == SSHError.authenticationFailed.description)
        // One the core retries stays behind the redial, with no card.
        let (quietDriver, quiet, quietFake) = self.driver(ServerConnection(host: "h", username: "u"))
        let quietTask = await started(quietDriver, quietFake)
        quietDriver.handle(.state(state: .connected))
        await quietTask.value
        quietDriver.handle(.state(state: .reconnecting))
        quietDriver.handle(.state(state: .failed(kind: .unreachable,
                                                 cause: .unreachable(host: "h", port: 22, why: .timedOut),
                                                 log: "timed out", retryable: true)))
        #expect(quiet.viewModel.errorMessage == nil)
    }

    /// B513: an error nobody dismissed (a redial that failed behind Home)
    /// belongs to that attempt; the next connect starts without it.
    @Test("a new connect starts without the last attempt's error (B513)")
    func newConnectDropsTheLastError() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        driver.handle(.state(state: .reconnecting))
        driver.handle(.state(state: .failed(kind: .credentials,
                                            cause: .credentials(why: .rejected(user: "u")),
                                            log: "refused", retryable: false)))
        #expect(session.viewModel.errorMessage == SSHError.authenticationFailed.description)
        let retry = await started(driver, fake)
        #expect(fake.reconnected.count == 1)
        #expect(session.viewModel.errorMessage == nil, "the last attempt's error on the new one")
        driver.handle(.state(state: .connected))
        await retry.value
        #expect(session.viewModel.errorMessage == nil)
    }

    /// B533: Settings' predictive echo and its trail reach a live mosh
    /// session at once; they used to wait for its next connect.
    @Test("Settings' predictive echo reaches a live mosh session at once (B533)")
    func predictionFollowsSettings() async {
        let settings = AppSettings.shared
        let (mode, trail) = (settings.predictMode, settings.trailOnPredict)
        defer {
            settings.predictMode = mode
            settings.trailOnPredict = trail
        }
        var connection = ServerConnection(host: "h", username: "u")
        connection.connectionProtocol = .mosh
        let (driver, session, fake) = driver(connection)
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        settings.predictMode = mode == .always ? .never : .always
        let moded = await eventually { session.coordinator.prediction?.mode == settings.predictMode }
        #expect(moded, "the live session kept the mode it connected with")
        settings.trailOnPredict = !trail
        let trailed = await eventually { session.coordinator.prediction?.trail == !trail }
        #expect(trailed, "the live session kept the trail it connected with")
    }

    @Test("what the core's probe found is the session's: Install Assist has its command (B336)")
    func capabilitiesPublished() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let key = "hostcaps.\(connection.id.uuidString)"
        defer { UserDefaults.standard.removeObject(forKey: key) }
        // An older copy in the app's cache, then the core's own probe written
        // behind it, as crates/store writes it: JSON in Data, the same key.
        HostCapabilityCache.shared.store(
            HostCapabilities(hasTmux: true, hasMoshServer: true, os: "", packageManager: nil),
            for: connection.id)
        let probed = HostCapabilities(hasTmux: false, hasMoshServer: true, os: "Linux",
                                      packageManager: .aptGet)
        UserDefaults.standard.set(try JSONEncoder().encode(probed), forKey: key)
        let (driver, session, fake) = driver(connection)
        let task = await started(driver, fake)
        #expect(session.capabilities == nil)
        driver.handle(.degraded(degrade: .tmuxMissing))
        driver.handle(.state(state: .connected))
        await task.value
        #expect(session.capabilities == probed, "the core's, not the cache's older copy")
        let plan = InstallAssistPlan(session: session, packages: ["tmux"])
        #expect(plan.packageManager == .aptGet)
        #expect(plan.command != nil, "the one-line install, as on the Swift path")
    }

    @Test("mosh + tmux on a server with no session: the terminal's empty state, not a black screen (B228, B337)")
    func moshTmuxNoSession() async {
        var connection = ServerConnection(host: "h", username: "u")
        connection.connectionProtocol = .mosh
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        #expect(session.moshControl != nil, "the sidecar's surface")
        #expect(session.tmuxController == nil, "mosh's renderer is on screen")
        // The core asked the server: nothing on it.
        fake.tmux = Self.emptyServer()
        driver.handle(.treeChanged)
        #expect(session.awaitingFirstSession)
        let (content, empty) = TerminalScreen.paneContent(of: session, isReconnect: false, connState: .live)
        #expect(content == .empty)
        #expect(empty == TerminalNSHChrome.Empty(mux: .tmux, missing: false, settled: true))
        // Mid-reconnect it stays behind the veil.
        let (dropped, _) = TerminalScreen.paneContent(of: session, isReconnect: true, connState: .reconnecting)
        #expect(dropped == .terminal)
        // The person's first session attaches: the renderer's screen again.
        fake.tmux = Self.model(active: "%0")
        driver.handle(.treeChanged)
        #expect(!session.awaitingFirstSession)
        let (attached, none) = TerminalScreen.paneContent(of: session, isReconnect: false, connState: .live)
        #expect(attached == .terminal)
        #expect(none == nil)
    }

    @Test("SSH + tmux on a server the core found empty: the answer at once, no blank wait (B789)")
    func sshTmuxNoSessionIsKnown() async {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        #expect(session.tmuxController != nil)
        // Not asked yet: the attach may be on its way.
        let (waiting, unknown) = TerminalScreen.paneContent(of: session, isReconnect: false, connState: .live)
        #expect(waiting == .empty)
        #expect(unknown?.settled == false)
        // The core asked the server: nothing on it.
        fake.tmux = Self.emptyServer()
        driver.handle(.treeChanged)
        let (content, empty) = TerminalScreen.paneContent(of: session, isReconnect: false, connState: .live)
        #expect(content == .empty)
        #expect(empty == TerminalNSHChrome.Empty(mux: .tmux, missing: false, settled: true))
    }

    @Test("the mosh diagnostics' Repaint reaches the core's mosh (B338)")
    func repaint() async {
        var connection = ServerConnection(host: "h", username: "u")
        connection.connectionProtocol = .mosh
        let (driver, session, fake) = driver(connection)
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        session.requestRepaint()
        #expect(fake.repainted == [7])
    }

    @Test("the shell's end ends the session, as the Swift path does")
    func ended() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        var ended = false
        session.endedSink = { ended = true }
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        driver.handle(.state(state: .ended(reason: .shellExited(status: 0))))
        #expect(session.viewModel.endedByServer)
        #expect(ended)
    }

    @Test("a host-key card goes to the core with the person's answer; a no ends the connect quietly (B151, B334)")
    func hostKeyCard() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        var retired = false
        session.endedSink = { retired = true }
        let task = await started(driver, fake)
        driver.handle(.hostKeyPrompt(prompt: 3, host: "h", port: 22, fingerprint: "SHA256:x", previous: nil,
                                     olderAlgorithm: nil, conflicting: false, connectedBefore: false))
        #expect(session.viewModel.hostKeyPrompt?.fingerprint == "SHA256:x")
        #expect(session.viewModel.hostKeyPrompt?.olderAlgorithm == nil)
        session.viewModel.hostKeyPrompt?.decide(false)
        #expect(session.viewModel.hostKeyPrompt == nil)
        #expect(fake.answers.count == 1)
        #expect(fake.answers.first.map { $0.0 == 7 && $0.1 == 3 && !$0.2 } == true)
        // The refusal's failure is not news on a card: the connect ends as
        // the Swift path ends it — disconnected, the terminal leaves
        // (`declinedHostKey`), and the hub takes the session out, so Home
        // shows no dead card for a connect the person chose not to make.
        driver.handle(.state(state: .failed(kind: .hostKey,
                                            cause: .hostKey(host: "h", port: 22, changed: false),
                                            log: "not trusted", retryable: false)))
        await task.value
        #expect(session.viewModel.errorMessage == nil)
        #expect(session.viewModel.connState == .offline)
        #expect(session.viewModel.status == .disconnected)
        #expect(session.viewModel.declinedHostKey)
        #expect(retired, "the declined session is taken out")
    }

    /// B563: what the server asks at sign-in beyond the password goes on a
    /// card in the server's words; the person's answer, or their no, goes
    /// back to the core; a question the core withdraws closes its card.
    @Test("a server's sign-in question is asked on a card, and the answer reaches the core")
    func signInQuestionCard() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.signInQuestion(prompt: 4, host: "h", name: "", instructions: "",
                                      text: "Verification code: ", echo: false))
        let question = session.viewModel.signInQuestion
        #expect(question?.words == "Verification code:")
        #expect(question?.echo == false)
        question?.decide("424242")
        #expect(session.viewModel.signInQuestion == nil)
        #expect(fake.questionAnswers.count == 1)
        #expect(fake.questionAnswers.first.map { $0.0 == 7 && $0.1 == 4 && $0.2 == "424242" } == true)

        driver.handle(.signInQuestion(prompt: 5, host: "h", name: "", instructions: "",
                                      text: "Token: ", echo: false))
        session.viewModel.signInQuestion?.decide(nil)
        #expect(fake.questionAnswers.last.map { $0.1 == 5 && $0.2 == nil } == true)

        driver.handle(.signInQuestion(prompt: 6, host: "h", name: "", instructions: "",
                                      text: "Token: ", echo: false))
        driver.handle(.signInQuestionWithdrawn(prompt: 6))
        #expect(session.viewModel.signInQuestion == nil)
        #expect(fake.questionAnswers.count == 2, "a withdrawn question isn't answered")
        driver.handle(.state(state: .connected))
        await task.value
    }

    /// B564: a host key that came by ssh-rsa says so on the New Host card.
    @Test("a host key by the older algorithm carries it to the card")
    func olderHostKeyCard() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.hostKeyPrompt(prompt: 3, host: "h", port: 22, fingerprint: "SHA256:x", previous: nil,
                                     olderAlgorithm: "ssh-rsa", conflicting: false, connectedBefore: false))
        #expect(session.viewModel.hostKeyPrompt?.olderAlgorithm == "ssh-rsa")
        session.viewModel.hostKeyPrompt?.decide(true)
        driver.handle(.olderHostKeyAlgorithm(algorithm: "ssh-rsa"))
        driver.handle(.state(state: .connected))
        await task.value
    }

    @Test("the server's words are shown as sent, without terminal escapes")
    func serverWords() {
        #expect(RustSessionDriver.serverWords(name: "Duo", instructions: "\u{1B}[1mPick one\u{1B}[0m\n",
                                              prompt: "Passcode: ")
                == "Duo\nPick one\nPasscode:")
        #expect(RustSessionDriver.serverWords(name: "", instructions: "\u{1B}]0;title\u{07}Hi\u{07}",
                                              prompt: "Code:\r") == "Hi\nCode:")
        #expect(RustSessionDriver.serverWords(name: "", instructions: "", prompt: "Code: ") == "Code:")
        #expect(RustSessionDriver.serverWords(name: "", instructions: "", prompt: String(repeating: "x", count: 900)).count == 600)
    }

    /// The passphrase card of a key kept behind one (B125): the core's
    /// credential call asks through the terminal's card, and what the person
    /// types unlocks the key the core is handed.
    @Test("a key's passphrase is asked on the terminal's card, and the answer reaches the core")
    func passphraseCard() async throws {
        let key = try #require(SSHKeyFixtures.all.first { $0.name == "Ed25519 · OpenSSH · passphrase" })
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let connection = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                          authMethod: .key, keychainRef: ref)
        let fake = FakeCoreSessions()
        let credentials = RustCredentialHost(service: SSHService(keychain: keychain, protectedDataAvailable: { true }))
        let core = RustCore(sessions: fake, credentials: credentials)
        let session = SessionHub.ActiveSession(connection: connection)
        let driver = RustSessionDriver(session: session, core: core, appearance: Self.look)
        session.rust = driver
        let task = await started(driver, fake)

        let asked = Task { try await credentials.credential(connectionId: connection.id.uuidString) }
        var waited = 0
        while session.viewModel.keyPassphrasePrompt == nil, waited < 300 {
            waited += 1
            try await Task.sleep(for: .milliseconds(10))
        }
        let prompt = try #require(session.viewModel.keyPassphrasePrompt)
        #expect(prompt.host == "192.0.2.1")
        #expect(!prompt.wrong)
        prompt.decide(SSHKeyFixtures.passphrase)
        guard case .key(let openssh) = try await asked.value else {
            Issue.record("not a key")
            return
        }
        #expect(!SSHKeyFactory.isEncrypted(openssh))
        #expect(session.viewModel.keyPassphrasePrompt == nil)
        driver.handle(.state(state: .connected))
        await task.value
    }

    /// B631: two reads of a key behind a passphrase at once (the dial and a
    /// side connection) share one card and its one answer; the card is
    /// answered once, whatever else answers it after (Cancel racing
    /// Unlock: a continuation resumed twice trapped).
    @Test("reads asking for the passphrase at once share one card, answered once")
    func passphraseAnsweredOnce() async throws {
        let key = try #require(SSHKeyFixtures.all.first { $0.name == "Ed25519 · OpenSSH · passphrase" })
        let keychain = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await keychain.savePrivateKey(key.privateKey, forRef: ref, requireBiometry: false)
        let connection = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                          authMethod: .key, keychainRef: ref)
        let fake = FakeCoreSessions()
        let credentials = RustCredentialHost(service: SSHService(keychain: keychain, protectedDataAvailable: { true }))
        let core = RustCore(sessions: fake, credentials: credentials)
        let session = SessionHub.ActiveSession(connection: connection)
        let driver = RustSessionDriver(session: session, core: core, appearance: Self.look)
        session.rust = driver
        let task = await started(driver, fake)

        let first = Task { try await credentials.credential(connectionId: connection.id.uuidString) }
        let second = Task { try await credentials.credential(connectionId: connection.id.uuidString) }
        var waited = 0
        while driver.passphraseWaiting < 2, waited < 300 {
            waited += 1
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(driver.passphraseWaiting == 2, "the second read didn't wait on the card")
        let prompt = try #require(session.viewModel.keyPassphrasePrompt)
        prompt.decide(SSHKeyFixtures.passphrase)
        prompt.decide(nil)
        for read in [first, second] {
            guard case .key(let openssh) = try await read.value else {
                Issue.record("not a key")
                return
            }
            #expect(!SSHKeyFactory.isEncrypted(openssh))
        }
        #expect(session.viewModel.keyPassphrasePrompt == nil)
        #expect(driver.passphraseWaiting == 0)
        driver.handle(.state(state: .connected))
        await task.value
    }

    /// B631: a host-key sheet on its way out after the core withdrew its
    /// prompt and asked the next answers late: its answer goes to the core
    /// (which takes it only while that prompt is asked) and clears nothing
    /// of the next one, which used to vanish and leave the dial waiting.
    @Test("a late host-key answer leaves the next prompt up")
    func lateHostKeyAnswer() async throws {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.hostKeyPrompt(prompt: 3, host: "h", port: 22, fingerprint: "SHA256:a", previous: nil,
                                     olderAlgorithm: nil, conflicting: false, connectedBefore: false))
        let first = try #require(session.viewModel.hostKeyPrompt)
        driver.handle(.hostKeyPromptWithdrawn(prompt: 3))
        driver.handle(.hostKeyPrompt(prompt: 4, host: "h", port: 22, fingerprint: "SHA256:b", previous: nil,
                                     olderAlgorithm: nil, conflicting: false, connectedBefore: false))
        first.decide(false)
        #expect(session.viewModel.hostKeyPrompt?.fingerprint == "SHA256:b")
        #expect(fake.answers.last.map { $0.1 == 3 && !$0.2 } == true)
        session.viewModel.hostKeyPrompt?.decide(true)
        #expect(session.viewModel.hostKeyPrompt == nil)
        #expect(fake.answers.last.map { $0.1 == 4 && $0.2 } == true)
        driver.handle(.state(state: .connected))
        await task.value
        #expect(!session.viewModel.declinedHostKey, "the late no read as the person's refusal")
    }

    /// B631: the same for the server's sign-in question.
    @Test("a late answer to a sign-in question leaves the next one up")
    func lateQuestionAnswer() async throws {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        let task = await started(driver, fake)
        driver.handle(.signInQuestion(prompt: 5, host: "h", name: "", instructions: "",
                                      text: "Verification code: ", echo: false))
        let first = try #require(session.viewModel.signInQuestion)
        driver.handle(.signInQuestionWithdrawn(prompt: 5))
        driver.handle(.signInQuestion(prompt: 6, host: "h", name: "", instructions: "",
                                      text: "Token: ", echo: false))
        first.decide(nil)
        #expect(session.viewModel.signInQuestion?.words == "Token:")
        #expect(fake.questionAnswers.last.map { $0.1 == 5 && $0.2 == nil } == true)
        session.viewModel.signInQuestion?.decide("424242")
        #expect(session.viewModel.signInQuestion == nil)
        #expect(fake.questionAnswers.last.map { $0.1 == 6 && $0.2 == "424242" } == true)
        driver.handle(.state(state: .connected))
        await task.value
    }

    @Test("a host-key failure the person didn't decline is still a failure, with its card")
    func hostKeyFailureNotDeclined() async {
        let (driver, session, fake) = driver(ServerConnection(host: "h", username: "u"))
        var retired = false
        session.endedSink = { retired = true }
        let task = await started(driver, fake)
        driver.handle(.state(state: .connecting))
        driver.handle(.state(state: .failed(kind: .hostKey,
                                            cause: .hostKey(host: "h", port: 22, changed: true),
                                            log: "changed", retryable: false)))
        await task.value
        #expect(!session.viewModel.declinedHostKey)
        #expect(!retired)
        #expect(session.viewModel.errorMessage?.isEmpty == false)
    }

    @Test("tmux: a surface per attach, the old one frozen until the new one paints")
    func tmuxAttaches() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        fake.tmux = Self.model(active: "%1")
        let task = await started(driver, fake)
        #expect(session.awaitingFirstFrame)
        driver.handle(.state(state: .connected))
        await task.value
        let first = try #require(session.tmuxController as? RustTmuxController)
        #expect(!first.snapshot.isAttached, "nothing mirrored before the attach's own tree")
        driver.handle(.treeChanged)
        #expect(session.awaitingFirstFrame)
        #expect(first.snapshot.activePaneId == "%1")
        #expect(first.snapshot.panes["%1"]?.path == "/srv/work", "the pane's working directory")
        #expect(first.snapshot.isAttached)
        driver.handle(.panes(attach: 1, panes: [PaneValue(pane: "%1", stream: 0, active: true)]))
        #expect(first.attach == 1)
        driver.handle(.firstFrame(attach: 1))
        #expect(!session.awaitingFirstFrame)
        #expect(first.paintedSinceAttach)

        // A drop: the painted surface stays, frozen; the new attach gets a
        // new one, which takes over once it paints.
        driver.handle(.state(state: .reconnecting))
        #expect(session.retiredTmuxController === first)
        driver.handle(.state(state: .connected))
        let second = try #require(session.tmuxController as? RustTmuxController)
        #expect(second !== first)
        driver.handle(.panes(attach: 2, panes: [PaneValue(pane: "%1", stream: 0, active: true)]))
        #expect(second.attach == 2)
        #expect(session.retiredTmuxController === first)
        driver.handle(.firstFrame(attach: 2))
        #expect(session.retiredTmuxController == nil)
    }

    @Test("tmux degraded to a shell: no tmux surface, the plain pane paints")
    func tmuxDegraded() async {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        let task = await started(driver, fake)
        driver.handle(.degraded(degrade: .tmuxMissing))
        driver.handle(.state(state: .connected))
        await task.value
        #expect(session.tmuxController == nil)
        #expect(session.degrade?.missing == .tmux)
        driver.handle(.panes(attach: 1, panes: [PaneValue(pane: "shell", stream: 0, active: true)]))
        #expect(session.tmuxController == nil)
        #expect(!session.awaitingFirstFrame)
    }

    @Test("the tmux surface mirrors the core and asks it for everything")
    func tmuxSurface() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        var model = Self.model(active: "%1")
        model.notice = "can't find window"
        model.sizeFollowWindow = "@1"
        model.sizeFollowOwner = "iPad"
        model.agentHooks = [AgentHookValue(pane: "%1", state: "attention", agent: "claude",
                                           since: 1_800_000_000, command: "claude", title: "t")]
        model.agentHooksLoaded = true
        fake.tmux = model
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        driver.handle(.treeChanged)
        let tmux = try #require(session.tmuxController)
        #expect(tmux.notice == "can't find window")
        #expect(tmux.sizeFollow == RustTmuxController.SizeFollow(windowId: "@1", ownerLabel: "iPad"))
        #expect(tmux.agentHooks["%1"]?.state == "attention")
        #expect(tmux.agentHooks["%1"]?.since == Date(timeIntervalSince1970: 1_800_000_000))
        #expect(tmux.agentHooksLoaded)
        #expect(tmux.snapshot.windows["@1"]?.name == "main")
        #expect(tmux.snapshot.sortedSessions.map(\.name) == ["work"])

        tmux.selectWindow("@1")
        tmux.takeSizeOwnership()
        tmux.newSession(named: "x")
        tmux.killPane("%1")
        tmux.pollAgentHooks()
        session.sendPaste("hi")
        session.createFirstTmuxSession()
        #expect(fake.intents.map(\.1) == [
            .selectWindow(id: "@1"), .takeSizeOwnership, .newSession(name: "x"),
            .killPane(id: "%1"), .pollAgentHooks, .paste(pane: "%1", text: "hi"),
            .newSession(name: nil),
        ])
        #expect(fake.intents.allSatisfy { $0.0 == 7 })
        // The terminal for the pane on screen is minted on demand, once, on
        // the core.
        let view = tmux.surface(for: "%1")
        #expect(view === tmux.surface(for: "%1"))
        #expect(view is CoreTerminalView)
        #expect(tmux.activeCoordinator === tmux.coordinator(for: "%1"))
        // A disconnect asks the core to close the session.
        let stopping = Task { await session.stop() }
        await Task.yield()
        driver.handle(.closed)
        await stopping.value
        #expect(fake.disconnected == [7])
    }

    /// B250: the core refuses a typed name with a control character in it
    /// — it made the first session with tmux's own name instead, silently —
    /// and the surface says why, in the app's words, as the Swift
    /// controller does; tmux's own notice still wins while it shows.
    @Test("a name the core refuses is said as the notice")
    func tmuxRefusalIsSaid() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        fake.refuseTmux = { intent in
            if case .newSession(let name) = intent, name?.contains("\n") == true {
                return .NameHasControlCharacters
            }
            if case .renameWindow(_, let name) = intent, name.contains("\t") {
                return .NameHasControlCharacters
            }
            return nil
        }
        fake.tmux = Self.model(active: "%1")
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        driver.handle(.treeChanged)
        let tmux = try #require(session.tmuxController)
        #expect(tmux.notice == nil)

        tmux.newSession(named: "two\nlines")
        let words = String(localized: "Names can't contain line breaks or other control characters.")
        #expect(tmux.notice == words)
        #expect(fake.intents.isEmpty, "a refused intent was sent")
        // A tree change in the meantime doesn't take the notice away.
        driver.handle(.treeChanged)
        #expect(tmux.notice == words)

        tmux.renameWindow("@1", to: "a\tb")
        #expect(tmux.notice == words)
        tmux.newSession(named: "fine")
        #expect(fake.intents.map(\.1) == [.newSession(name: "fine")])

        var model = Self.model(active: "%1")
        model.notice = "duplicate session: fine"
        fake.tmux = model
        driver.handle(.treeChanged)
        #expect(tmux.notice == "duplicate session: fine", "tmux's own notice comes first")
    }

    @Test("foreground, background and the terminal on screen reach the core")
    func appLifecycle() async {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        core.setForeground(false)
        #expect(fake.foreground == [false])
        #expect(fake.flushes == 1, "the background is held until the release went out")
        core.setForeground(false)
        #expect(fake.foreground == [false], "told once")
        core.setForeground(true)
        #expect(fake.foreground == [false, true])
        core.setVisible(7)
        core.setVisible(nil)
        #expect(fake.visible == [7, nil])
        core.deliver([SessionEvent(session: 0, kind: .flushed(token: 1))])
    }

    // MARK: W10b

    @Test("a failure is worded by the app, from the core's kind and cause (B202)")
    func failureCopy() {
        #expect(RustCopy.failure(kind: .credentialsLocked, cause: .credentials(why: .locked))
                .contains("locked"))
        // A host with a domain (B740: a one-word name is a local one, worded
        // with the Local Network permission).
        #expect(RustCopy.failure(kind: .unreachable,
                                 cause: .unreachable(host: "box.example.com", port: 22, why: .other))
                == SSHError.connectionFailed.description)
        #expect(RustCopy.failure(kind: .proxy, cause: .proxy(proxy: "p:1080", why: .unreachable))
                == SSHError.proxyUnreachable("p:1080").description)
        #expect(RustCopy.failure(kind: .moshBootstrap, cause: .moshBootstrap(why: .timedOut(seconds: 15)))
                .contains("15"))
        // No English from the core reaches the card: the log is not the copy.
        let copy = RustCopy.failure(kind: .handshake,
                                    cause: .handshake(host: "h", port: 22, timedOut: true))
        #expect(copy == SSHError.handshakeFailed.description)
        // B393: a server whose questions only the person can answer — the
        // rejected method's words, and why.
        let rejected = SSHError.authMethodRejected.description
        let prompts = RustCopy.failure(kind: .credentials, cause: .credentials(why: .interactivePrompts))
        #expect(prompts.hasPrefix(rejected))
        #expect(prompts != rejected)
    }

    /// B787: tmux on the host gone under mosh's screen says so; mosh-server's
    /// own end keeps its line, which blamed the shell.
    @Test("each end says its own line: tmux gone apart from mosh-server's end (B787)")
    func endCopy() {
        #expect(RustCopy.ended(.tmuxGone) == "Session ended: tmux on the host stopped.")
        #expect(RustCopy.ended(.moshServerEnded)
                == "The session ended: the remote shell exited or mosh-server was stopped.")
        #expect(RustCopy.ended(.shellExited(status: 0)) == "The session ended: the remote shell exited.")
        #expect(RustCopy.ended(.disconnected) == nil)
    }

    /// B740: a first connection that reaches nothing says why — a port that
    /// turned it away, a name that resolves to nothing, a host that never
    /// answered, no route — instead of one sentence for all of them. A host
    /// on the local network adds the one cause the person can't see: iOS
    /// asked whether Moshpit may use the local network, and the answer was
    /// no (or the question is still up). A denial there fails the connect
    /// like a host that isn't there; the person's way out is a switch in the
    /// Settings app.
    @Test("an unreachable host is worded by why, and a local one points at Local Network (B740)")
    func unreachableSaysWhy() {
        func copy(_ host: String, _ why: UnreachableValue, port: UInt16 = 22) -> String {
            RustCopy.failure(kind: .unreachable, cause: .unreachable(host: host, port: port, why: why))
        }
        let generic = SSHError.connectionFailed.description
        let allow = "Local Network"

        // Each reason has its own words, with the host (and the port, for a
        // refusal) the person typed.
        let refused = copy("box.example.com", .refused, port: 2222)
        #expect(refused.contains("box.example.com") && refused.contains("2222"), "\(refused)")
        let missing = copy("box.exmaple.com", .noAddress)
        #expect(missing.contains("box.exmaple.com"), "\(missing)")
        let silent = copy("box.example.com", .timedOut)
        let unrouted = copy("203.0.113.5", .noRoute)
        #expect(Set([refused, missing, silent, unrouted, generic]).count == 5,
                "one sentence per reason: \([refused, missing, silent, unrouted])")

        // A host on the local network: no route, no answer or anything else
        // may be the local network permission.
        let lan = ["192.168.1.20", "10.0.0.5", "172.20.1.1", "169.254.3.4",
                   "fe80::1", "fd12:3456::1", "studio.local", "Studio.Local.", "studio"]
        for host in lan {
            for why in [UnreachableValue.noRoute, .timedOut, .other] {
                #expect(copy(host, why).contains(allow), "\(host) \(why): \(copy(host, why))")
                #expect(copy(host, why).contains(host), "\(host) \(why)")
            }
        }
        // A `.local` name is looked up on the local network itself.
        #expect(copy("studio.local", .noAddress).contains(allow))

        // Elsewhere — the internet, a tailnet address, a refusal (the host
        // answered), a dotted name that resolves to nothing — the permission
        // isn't the cause, and isn't named.
        let elsewhere: [(String, UnreachableValue)] = [
            ("203.0.113.5", .noRoute), ("box.example.com", .timedOut),
            ("100.64.0.7", .timedOut), ("172.32.0.1", .noRoute), ("2001:db8::1", .other),
            ("192.168.1.20", .refused), ("studio.local", .refused), ("box.example.com", .noAddress),
            ("studio", .noAddress),
        ]
        for (host, why) in elsewhere {
            #expect(!copy(host, why).contains(allow), "\(host) \(why): \(copy(host, why))")
        }
    }

    @Test("a credential the keychain won't give while the device is locked is 'locked' (B201)")
    func lockedCredentials() async {
        // No keychain item behind this connection: unreadable either way —
        // but only a locked device makes that worth retrying on the unlock.
        let connection = ServerConnection(host: "h", username: "u")
        for (available, locked) in [(false, true), (true, false)] {
            let host = RustCredentialHost(protectedData: { available })
            host.register(connection) { _ in nil }
            do {
                _ = try await host.credential(connectionId: connection.id.uuidString)
                Issue.record("no credential without a keychain item")
            } catch CredentialFailure.Locked {
                #expect(locked)
            } catch {
                #expect(!locked, "\(error)")
            }
        }
    }

    @Test("a Secure Enclave signature the locked device refuses is 'locked', retried on the unlock (B227)")
    func lockedSigner() {
        // Every failure crossed as Unavailable — SignerFailed in the core,
        // never retried.
        #expect(RustCredentialHost.signFailure(SSHError.credentialLocked, protectedDataAvailable: true)
                == .Locked)
        #expect(RustCredentialHost.signFailure(SSHError.underlying(CocoaError(.featureUnsupported)),
                                               protectedDataAvailable: false) == .Locked)
        guard case .Unavailable = RustCredentialHost.signFailure(SSHError.credentialUnavailable,
                                                                 protectedDataAvailable: true) else {
            Issue.record("a key that is gone is no lock")
            return
        }
    }

    @Test("agent stamps are held for the pace they come at: tmux's sweep, herdr's poll, push or not (B346)")
    func stampIntervals() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .herdr
        let (driver, session, fake) = driver(connection)
        var model = Self.herdrModel()
        model.pushActive = false
        fake.herdr = model
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        let herdr = try #require(session.herdrControl as? RustHerdrController)
        #expect(AgentActivityMonitor.stampInterval(of: herdr) == 8)
        // Push comes up mid-session: the next stamps carry its pace.
        model.pushActive = true
        fake.herdr = model
        driver.handle(.agentHooksUpdated)
        #expect(AgentActivityMonitor.stampInterval(of: herdr) == 45)
        model.pushActive = false
        fake.herdr = model
        driver.handle(.treeChanged)
        #expect(AgentActivityMonitor.stampInterval(of: herdr) == 8)
        // tmux's: the sweep that polls them.
        let tmux = AgentStandingTests.FakeMux()
        #expect(AgentActivityMonitor.stampInterval(of: tmux) == AgentActivityMonitor.sweepInterval)
    }

    @Test("herdr: the surface mirrors the core, asks it for everything, waits for its answers")
    func herdrSurface() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .herdr
        let (driver, session, fake) = driver(connection)
        fake.herdr = Self.herdrModel()
        let task = await started(driver, fake)
        #expect(session.awaitingFirstFrame)
        driver.handle(.state(state: .connected))
        await task.value
        let herdr = try #require(session.herdrControl as? RustHerdrController)
        #expect(herdr.snapshot.activePaneId == "w1:p1")
        #expect(herdr.workingDirectory(ofPane: "w1:p1") == "/r")
        // Each pane's working directory, on the pane (UI-DESIGN §10.5).
        #expect(herdr.snapshot.panes["w1:p1"]?.path == "/r")
        #expect(herdr.snapshot.panes["w1:p2"]?.path == nil)
        #expect(herdr.agentHooks["w1:p1"]?.state == "working")
        #expect(session.herdrNotice == String(localized: "This pane is open on another device"))
        #expect(session.herdrNoticeAction == String(localized: "Open here"))
        // The focused pane's channel paints the session's terminal.
        driver.handle(.panes(attach: 1, panes: [PaneValue(pane: "w1:p1", stream: 0, active: true)]))
        driver.handle(.firstFrame(attach: 1))
        #expect(!session.awaitingFirstFrame)
        // Paintings of a pane herdr emulates: the terminal answers no query
        // into it (B575).
        #expect(!session.coordinator.answersQueries)
        herdr.selectPane("w1:p2")
        herdr.newPane()
        #expect(fake.herdrIntents.map(\.1) == [.selectPane(id: "w1:p2"), .newPane])
        // A task waits for the core's answer to its token.
        let start = Task {
            await herdr.startAgentTask(AgentTaskRequest(repoPath: "/r", branch: "b", agent: "claude",
                                                        prompt: "go"))
        }
        for _ in 0..<100 where fake.herdrIntents.count < 3 { await Task.yield() }
        let token = fake.nextToken
        driver.handle(.herdrAnswer(token: token, answer: .task(outcome: .incomplete(
            incomplete: .agentNotReady(agent: "claude")))))
        let outcome = await start.value
        #expect(outcome == .incomplete(String(localized: "The task is running, but claude wasn't ready for the first message in time.")))
        // A notification's reply goes to the core and waits for its answer.
        let reply = Task { await session.deliverInput(Data("yes\r".utf8), toPane: "w1:p2") }
        for _ in 0..<100 where fake.delivered.isEmpty { await Task.yield() }
        #expect(fake.delivered.first?.1 == "w1:p2")
        driver.handle(.delivered(token: fake.nextToken, delivered: true))
        #expect(await reply.value)
    }

    @Test("mosh + tmux: the sidecar's tree as the mosh control plane, mosh's screen on the session's terminal")
    func moshSidecar() async throws {
        var connection = ServerConnection(host: "h", username: "u")
        connection.connectionProtocol = .mosh
        connection.multiplexer = .tmux
        let (driver, session, fake) = driver(connection)
        fake.tmux = Self.model(active: "%3")
        let task = await started(driver, fake)
        driver.handle(.state(state: .connected))
        await task.value
        #expect(session.tmuxController == nil, "no pane hosts over mosh")
        let sidecar = try #require(session.moshControl as? RustTmuxController)
        #expect(sidecar.sidecar)
        driver.handle(.panes(attach: 1, panes: [PaneValue(pane: "shell", stream: 0, active: true)]))
        driver.handle(.treeChanged)
        #expect(sidecar.snapshot.activePaneId == "%3")
        #expect(session.coordinator.answersQueries, "a shell's program is answered (B575)")
        #expect(session.moshControl === sidecar, "a shell pane under mosh keeps the sidecar")
        sidecar.selectSession("$0")
        #expect(fake.intents.map(\.1) == [.selectSession(id: "$0")])
        // A reconnect mirrors into the same surface.
        driver.handle(.state(state: .reconnecting))
        driver.handle(.state(state: .connected))
        #expect(session.moshControl === sidecar)
    }

    static func herdrModel() -> HerdrReadModel {
        HerdrReadModel(
            sessions: [TmuxSessionValue(id: "w1", name: "live", isAttached: true)],
            windows: [TmuxWindowValue(id: "w1:t1", sessionId: "w1", name: "1", index: 0, layout: "",
                                      isActive: true, paneCount: 2, isZoomed: false)],
            panes: [TmuxPaneValue(id: "w1:p1", windowId: "w1:t1", index: 0, command: "zsh", path: "/r",
                                  width: 80, height: 24, isActive: true),
                    TmuxPaneValue(id: "w1:p2", windowId: "w1:t1", index: 1, command: "zsh",
                                  width: 80, height: 24, isActive: false)],
            activeSessionId: "w1", activeWindowId: "w1:t1", activePaneId: "w1:p1",
            isAttached: true, everAttached: true,
            agentHooks: [AgentHookValue(pane: "w1:p1", state: "working", agent: "claude",
                                        since: nil, command: nil, title: nil)],
            agentHooksLoaded: true, notice: .onAnotherDevice, noticeAction: .openHere, refreshing: false,
            restarting: false, serverNotRunning: false, protocolMismatch: nil, skewClient: nil,
            skewServer: nil, serverVersion: "0.9.5", pushActive: true, paneSwitchInFlight: false,
            paneCwds: ["w1:p1": "/r"], worktreeRepos: [:], openRepoGuesses: ["/r"],
            frameTarget: "w1:p1", attachedPane: "w1:p1", retryWaitMs: nil)
    }

    static func model(active: String) -> TmuxReadModel {
        TmuxReadModel(
            sessions: [TmuxSessionValue(id: "$0", name: "work", isAttached: true)],
            windows: [TmuxWindowValue(id: "@1", sessionId: "$0", name: "main", index: 0, layout: "",
                                      isActive: true, paneCount: 1, isZoomed: false)],
            panes: [TmuxPaneValue(id: active, windowId: "@1", index: 0, command: "zsh", path: "/srv/work",
                                  width: 80, height: 24, isActive: true)],
            activeSessionId: "$0", activeWindowId: "@1", activePaneId: active,
            isAttached: true, everAttached: true, lastSwitchForward: true,
            hasSwipedSinceAttach: false, notice: nil, refreshing: false,
            sizeFollowWindow: nil, sizeFollowOwner: nil, paneFollowPane: nil, agentHooks: [],
            agentHooksLoaded: false, wantsMouse: false, altScreen: false, scrollsItself: false,
            holdsTicks: false, noSession: false)
    }

    /// A tmux server with nothing on it, as the core reads it once it asked.
    static func emptyServer() -> TmuxReadModel {
        TmuxReadModel(
            sessions: [], windows: [], panes: [], activeSessionId: nil, activeWindowId: nil,
            activePaneId: nil, isAttached: false, everAttached: false, lastSwitchForward: true,
            hasSwipedSinceAttach: false, notice: nil, refreshing: false, sizeFollowWindow: nil,
            sizeFollowOwner: nil, paneFollowPane: nil, agentHooks: [], agentHooksLoaded: true, wantsMouse: false,
            altScreen: false, scrollsItself: false, holdsTicks: false, noSession: true)
    }
}

/// B329: a connection saved while its session runs reaches the session's
/// next connect when the dial or the credential changed — not for a new
/// name or a last-connected stamp.
@Suite("A connection saved while its session runs")
struct SavedWhileRunningTests {
    @Test("a new address, user, proxy or credential dials differently; a new name does not")
    @MainActor
    func whatADialUses() {
        var started = ServerConnection(name: "box", host: "box.example.com", username: "dev",
                                       keychainRef: "moshpit.secret.own")
        started.authMethod = .password
        var renamed = started
        renamed.name = "renamed"
        #expect(RustSessionDriver.dialsTheSame(started, renamed))
        var moved = started
        moved.host = "other.example.com"
        #expect(!RustSessionDriver.dialsTheSame(started, moved))
        var user = started
        user.username = "root"
        #expect(!RustSessionDriver.dialsTheSame(started, user))
        var key = started
        key.authMethod = .key
        key.keychainRef = "moshpit.secret.key-k"
        key.sshKeyId = UUID()
        #expect(!RustSessionDriver.dialsTheSame(started, key))
        var proxied = started
        proxied.useSOCKSProxy = true
        proxied.socksProxyHost = "127.0.0.1"
        #expect(!RustSessionDriver.dialsTheSame(started, proxied))
    }
}
