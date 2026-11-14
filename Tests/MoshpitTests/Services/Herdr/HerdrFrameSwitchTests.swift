import Foundation
import Testing
@testable import Moshpit

/// The SSH frame channel against a host that runs what its lines ask for
/// (``HerdrLoopHost``, herdr 0.9.5's ways as the Rust client's rig models
/// them), driven through a real `ActiveSession.start()`: what each pane
/// received, what was lost on the way, which attaches were asked for. The
/// Rust client (`crates/herdr/src/client/attach.rs`) is the reference;
/// `crates/herdr/harness/client/divergences.json` names each case.
@Suite("herdr frame channel against herdr's own ways", .serialized)
@MainActor
struct HerdrFrameSwitchTests {

    private func connection() -> ServerConnection {
        var connection = ServerConnection(
            name: "herdr-switch",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .ssh
        connection.multiplexer = .herdr
        // The user vouches for the binary: no probe decides the multiplexer.
        connection.herdrPath = "/usr/local/bin/herdr"
        return connection
    }

    private func waitUntil(_ timeout: Duration = .seconds(10), _ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    /// Started over `host`, attached to the focused pane `w1:p1`, and its
    /// first frame on the terminal. The probe's result is forgotten when
    /// the test ends (it lands in the shared capability cache).
    private func attached(_ host: HerdrLoopHost, _ connection: ServerConnection) async throws -> SessionHub.ActiveSession {
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        #expect(await waitUntil { host.reading == "w1:p1" }, "the boot line never attached the focused pane")
        #expect(await waitUntil { session.coordinator.unattachedFeed.contains(HerdrLoopHost.screen("w1:p1", 1)) },
                "the first frame never reached the terminal")
        return session
    }

    private func forgetProbe(_ connection: ServerConnection) {
        UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString)
    }

    /// The screens fed to the terminal so far (`[<pane> #<seq>]`), in order.
    private func screens(_ session: SessionHub.ActiveSession) -> [String] {
        session.coordinator.unattachedFeed
            .map { String(decoding: $0, as: UTF8.self) }
            .filter { $0.hasPrefix("[w") }
    }

    // MARK: - B267

    /// The reader loop was a boot line typed into the channel's interactive
    /// login shell, and where it didn't answer, a one-shot `terminal session
    /// control` line was typed after it. A shell still starting swallowed
    /// them — one that flushes pending input (Powerlevel10k's instant
    /// prompt), or oh-my-zsh's plugins chewing on every character (B266 on
    /// the core): a blank terminal for good. The loop is the channel's own
    /// program, and its first pane is one word to it, once it reads.
    @Test("The frame loop is the channel's program: nothing is typed into a login shell")
    func theLoopIsTheChannelsProgram() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)
        #expect(host.programs == [HerdrFrameCommand.loopProgram(customPath: connection.herdrPath,
                                                                connectionId: connection.id.uuidString)])
        #expect(host.lines.first?.hasPrefix("w1:p1 ") == true, "the first line wasn't a pane for the loop: \(host.lines)")
        #expect(!host.lines.contains { $0.contains("moshpit-herdr") || $0.contains("terminal session control")
                                       || $0.contains("stty") },
                "a start line typed into the channel: \(host.lines)")
        await session.stop()
    }

    /// A loop that never says it reads leaves nothing to paint: said, with a
    /// Retry that redials, instead of a blank terminal — and nothing typed
    /// in its place (the one-shot line this replaced went to a shell).
    @Test("A loop that never says it reads is said, with a Retry that redials; nothing is typed")
    func aSilentLoopIsSaid() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrFrameHost(snapshot: HerdrFrameHost.onePane, silentLoop: true)
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        #expect(await waitUntil(.seconds(8)) { session.herdrNoticeAction == String(localized: "Retry") },
                "a loop that never read left a silent blank terminal")
        #expect(session.herdrNotice == String(localized: "Not attached to the pane yet — input wasn't sent"))
        #expect(host.writtenText.isEmpty, "written into a channel whose loop never read: \(host.writtenText)")
        session.runHerdrNoticeAction()
        #expect(await waitUntil { script.dials >= 2 }, "Retry didn't redial")
        await session.stop()
    }

    // MARK: - B117

    /// herdr turns away one line it can't parse — a resize past u16, a mouse
    /// event to a herdr before 0.9.2, an attach line read by an attach still
    /// alive — and the attach goes on painting. Every fault used to be read
    /// as a refused attach: the pane dropped, its words up as a banner, and
    /// every attach held off for 30 s, typing dropped meanwhile.
    @Test("A line herdr turns away leaves the attach alone: no banner, no backoff, typing goes on")
    func inputIgnoredLeavesTheAttachAlone() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)

        // What herdr 0.9.5 prints for a line it can't parse (probed live).
        host.emit("herdr: terminal session control input ignored: invalid json command: "
                  + "expected value at line 1 column 1\r\n")
        // The pump's turn (no frame after it: a frame takes any banner down).
        try await Task.sleep(for: .milliseconds(300))
        #expect(session.herdrNotice == nil, "herdr's words went up as a refused attach")

        session.sendInput(Data("still here".utf8))
        #expect(await waitUntil { host.typed("w1:p1") == "still here" },
                "typing was dropped: the live attach was taken for refused and backed off from")
        host.paint()
        #expect(await waitUntil { session.coordinator.unattachedFeed.contains(HerdrLoopHost.screen("w1:p1", 2)) })
        #expect(host.attaches == ["w1:p1"])
        await session.stop()
    }

    // MARK: - B185

    /// herdr 0.9.5 drops input that reaches an attach before its first frame
    /// (probed: 9 of 10 lines written right behind the attach line lost).
    /// Typed at the pick, it used to be dropped; typed once the attach line
    /// was out, it was written right behind it and lost without a word.
    @Test("Typing at the pane just picked waits for its attach and reaches it, in order")
    func typingAtThePickFollowsTheAttach() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)
        let control = try #require(session.herdrControl)

        control.selectPane("w1:p2")
        session.sendInput(Data("a".utf8))            // at the pick
        #expect(await waitUntil { host.lines.contains { $0.hasPrefix("w1:p2 ") } })
        session.sendInput(Data("b".utf8))            // its line out, its attach not up
        #expect(await waitUntil { host.typed("w1:p2") == "ab" },
                "typed at the pane just picked, dropped or lost: \(host.typed("w1:p2")) lost=\(host.lost)")
        #expect(host.typed("w1:p1") == "", "typing reached the pane being left")
        #expect(host.lost.isEmpty, "lines reached an attach that wasn't up: \(host.lost)")
        #expect(session.herdrNotice == nil)
        await session.stop()
    }

    /// The lock screen's Allow, a reply typed in a notification: the app
    /// switched to the agent's pane and wrote the reply right behind the
    /// attach line, where herdr drops it — and said sent.
    @Test("A reply to another pane's agent arrives, and is said sent only once it went out")
    func aReplyToAnotherPaneArrives() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)

        let sent = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p2")
        #expect(sent)
        #expect(await waitUntil { host.typed("w1:p2") == "y\r" },
                "the reply was lost behind the attach line: \(host.lost)")
        #expect(host.typed("w1:p1") == "")
        #expect(host.lost.isEmpty, "lines reached an attach that wasn't up: \(host.lost)")
        await session.stop()
    }

    @Test("A reply whose pane never comes up is said not sent, and nothing is lost on the way")
    func aReplyThatCantArriveSaysSo() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost(stuck: ["w1:p2"])
        let session = try await attached(host, connection)

        let began = ContinuousClock.now
        let sent = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p2")
        #expect(!sent, "reported sent while the reply never reached the pane")
        #expect(ContinuousClock.now - began < .seconds(9))
        #expect(host.typed("w1:p2") == "")
        #expect(!host.lost.contains { $0.contains("terminal.input") },
                "the reply was written into an attach that wasn't up: \(host.lost)")
        await session.stop()
    }

    // MARK: - B186

    /// Two picks inside one release's round trip (A → B → A). Each wrote a
    /// release; herdr answers both with one close, so the app stayed owed a
    /// close, and the next close nobody asked for — another client's
    /// takeover here — was taken for it: the pane counted as attached with
    /// nothing but the loop's `read` behind it, never asked for again, the
    /// screen frozen and typing gone.
    @Test("Quick switches write one release, and a takeover after them is seen, said, and taken back on the person's word")
    func quickSwitchesWriteOneRelease() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)
        let control = try #require(session.herdrControl)

        control.selectPane("w1:p2")
        control.selectPane("w1:p1")
        #expect(await waitUntil { host.attaches.count == 2 && host.reading == "w1:p1" },
                "the switch never landed: \(host.attaches)")
        #expect(host.releases == 1, "a release per switch: herdr answers both with one close")
        // The attach back on w1:p1 painted (the host's second frame).
        #expect(await waitUntil { screens(session).contains("[w1:p1 #2]") })

        // Another client takes the pane over: seen as such — never taken for
        // our own release's close — and said.
        let frames = screens(session).count
        host.takeOver()
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") },
                "the takeover was taken for our own release's close")
        #expect(host.attaches.count == 2, "taken back on its own")
        // "Open here": asked for again, and it paints.
        session.runHerdrNoticeAction()
        #expect(await waitUntil { host.attaches.count == 3 })
        #expect(await waitUntil { screens(session).count > frames }, "the pane never painted again")
        session.sendInput(Data("back".utf8))
        #expect(await waitUntil { host.typed("w1:p1") == "back" }, "typing went nowhere after the takeover")
        #expect(host.lost.isEmpty, "lines reached an attach that wasn't up: \(host.lost)")
        await session.stop()
    }

    // MARK: - Round twenty-eight: a pane another device has

    /// Another device's `--takeover` ended the attach. The app asked for the
    /// pane again on the poll's next report — with `--takeover`, taking it
    /// from that device — three times in 30 s before a backoff and "retrying
    /// shortly", then again: the two devices fought over the pane.
    @Test("A pane another device takes over is said with Open here, and never taken back on its own")
    func takenOverIsSaidAndLeftToTheOtherDevice() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)
        let control = try #require(session.herdrControl)

        host.takeOver()
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") },
                "nothing said the pane is on another device: \(String(describing: session.herdrNotice))")
        #expect(session.herdrNoticeAction == String(localized: "Open here"))
        // The poll repeats the focus, and the other device moves it: nothing
        // is asked of the channel, and typing goes nowhere.
        for _ in 0..<3 { await control.pollNowForTesting() }
        host.focus("w1:p2")
        for _ in 0..<3 { await control.pollNowForTesting() }
        try await Task.sleep(for: .milliseconds(300))
        #expect(host.attaches == ["w1:p1"], "taken back on its own: \(host.attaches)")
        session.sendInput(Data("rm -rf build\r".utf8))
        try await Task.sleep(for: .milliseconds(200))
        #expect(host.typed("w1:p1") == "")

        // "Open here": the person's word takes it.
        host.focus("w1:p1")
        session.runHerdrNoticeAction()
        #expect(await waitUntil { host.attaches.count == 2 && host.reading == "w1:p1" })
        #expect(host.tookOver == [false, true])
        #expect(host.takenFromOthers == ["w1:p1"])
        #expect(await waitUntil { session.herdrNotice == nil }, "the notice outlived the take")
        session.sendInput(Data("y".utf8))
        #expect(await waitUntil { host.typed("w1:p1") == "y" })
        await session.stop()
    }

    /// The shared focus moved onto a pane another device holds (that
    /// device's person picked it): following it attached with `--takeover`
    /// and took it. A follow's attach is plain now, and herdr refuses it.
    @Test("Following the focus onto a pane another device holds takes nothing, and says so")
    func followOntoAHeldPaneTakesNothing() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)
        let control = try #require(session.herdrControl)

        host.held = ["w1:p2"]
        host.focus("w1:p2")
        await control.pollNowForTesting()
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") },
                "\(host.attaches) \(host.tookOver)")
        #expect(host.takenFromOthers.isEmpty, "following the focus took the pane from the device that has it")
        #expect(host.attaches == ["w1:p1", "w1:p2"])
        #expect(host.tookOver == [false, false])
        for _ in 0..<3 { await control.pollNowForTesting() }
        try await Task.sleep(for: .milliseconds(200))
        #expect(host.attaches.count == 2, "the poll asked again: \(host.attaches)")

        session.runHerdrNoticeAction()
        #expect(await waitUntil { host.takenFromOthers == ["w1:p2"] })
        #expect(await waitUntil { host.reading == "w1:p2" })
        #expect(await waitUntil { session.herdrNotice == nil })
        await session.stop()
    }

    @Test("A reply for the pane another device has is refused without taking it; one for another pane takes that pane")
    func repliesAndThePaneAnotherDeviceHas() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)

        host.takeOver()
        #expect(await waitUntil { session.herdrNotice == String(localized: "This pane is open on another device") })
        let toTheHeldPane = await session.deliverInput(Data("y\r".utf8), toPane: "w1:p1")
        #expect(!toTheHeldPane, "reported sent into a pane another device has")
        try await Task.sleep(for: .milliseconds(300))
        #expect(host.attaches == ["w1:p1"], "taken back for a reply")
        #expect(host.typed("w1:p1") == "")

        // A reply for another pane is the person's pick of it: taken, sent.
        host.held.insert("w1:p2")
        let toAnother = await session.deliverInput(Data("n\r".utf8), toPane: "w1:p2")
        #expect(toAnother)
        #expect(await waitUntil { host.typed("w1:p2") == "n\r" })
        #expect(host.takenFromOthers == ["w1:p2"])
        await session.stop()
    }

    // MARK: - B187

    /// The pane being left prints once more between the switch and its
    /// release's close. Its frame used to be painted over the screen wiped
    /// for the new pane and counted as the new pane's paint; with the new
    /// attach line then lost, the poll's repeated focus report — which
    /// re-asks only an attach that never painted — never asked for it again.
    @Test("The pane being left paints nothing of the new one, and a lost attach line is asked again")
    func framesFromThePaneBeingReleased() async throws {
        let connection = connection()
        defer { forgetProbe(connection) }
        let host = HerdrLoopHost()
        let session = try await attached(host, connection)
        let control = try #require(session.herdrControl)

        host.swallowNextTarget()
        control.selectPane("w1:p2")
        // w1:p1 paints once more before its release reaches the host.
        host.paint()
        #expect(await waitUntil { host.lines.contains { $0.hasPrefix("w1:p2 ") } },
                "the switch never named its pane")
        try await Task.sleep(for: .milliseconds(200))
        #expect(!screens(session).contains("[w1:p1 #2]"),
                "the pane being left painted into the screen wiped for the new one")

        // That line was lost. Past the attach timeout the pane is asked for
        // again, and attaches.
        #expect(await waitUntil(.seconds(15)) { host.reading == "w1:p2" },
                "the lost attach line was never asked for again: \(host.attaches)")
        #expect(await waitUntil { screens(session).last?.hasPrefix("[w1:p2 #") == true })
        await session.stop()
    }
}
