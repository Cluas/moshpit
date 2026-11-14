import Foundation
import Network
import Testing
@testable import Moshpit

/// B165: a mosh session that goes quiet mid-session keeps its server.
///
/// The transport reports a dead return path twice over: a server that never
/// answered at all (8 s after the flow came up), and a session that HAD heard
/// from its server going silent through three flow rebuilds (about 15 s of
/// UDP silence). The hub took both for the first and killed the server — the
/// user's shell, with whatever ran in it, gone after a short dead spot on the
/// network. crates/session's rule: only a server never contacted is reaped;
/// a contacted one stays, the session stays connected, SSH is offered.
@Suite("mosh return path and the server", .serialized)
@MainActor
struct MoshReturnPathReapTests {

    @Test("the transport says whether the server was ever heard from")
    func transportSaysWhetherItHeard() async throws {
        // Silence after contact: three liveness rebuilds, then the report.
        let clock = ManualMoshClock(1_000)
        let channel = MoshTransportStateMachineTests.FakeDatagramChannel()
        let transport = try MoshTransport(
            credentials: MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: MoshSessionRig.key),
            channel: channel, livenessInterval: .milliseconds(20), clock: clock.read)
        var heard: [Bool] = []
        transport.onReturnPathDead = { heard.append($0) }
        await transport.start(cols: 80, rows: 24)
        channel.onStateChange?(.ready)
        channel.deliver(try MoshSessionRig.serverDatagram())
        #expect(await eventually { await transport.currentDiagnostics.datagramsReceived == 1 })
        clock.now += 10_000   // ten seconds without a word back
        #expect(await eventually { !heard.isEmpty })
        #expect(heard.first == true, "silence after contact was reported as a server never reached")
        await transport.close()

        // Nothing back from the start: the watchdog's report.
        let silent = MoshTransportStateMachineTests.FakeDatagramChannel()
        let fresh = try MoshTransport(
            credentials: MoshCredentials(host: "127.0.0.1", udpPort: 60001, key: MoshSessionRig.key),
            channel: silent, returnPathDeadlineNanos: 50_000_000)
        var said: Bool?
        fresh.onReturnPathDead = { said = $0 }
        await fresh.start(cols: 80, rows: 24)
        silent.onStateChange?(.ready)
        #expect(await eventually { said != nil })
        #expect(said == false)
        await fresh.close()
    }

    @Test("a server the session has talked to survives a silence: connected, SSH offered, nothing killed")
    func contactedServerSurvives() async throws {
        let clock = ManualMoshClock(1_000)
        let rig = MoshSessionRig(transport: { credentials, channel in
            try MoshTransport(credentials: credentials, channel: channel,
                              livenessInterval: .milliseconds(20), clock: clock.read)
        })
        await rig.start()
        #expect(rig.session.viewModel.status == .connected)
        let transport = try #require(rig.session.moshTransport)
        rig.channel.onStateChange?(.ready)
        rig.channel.deliver(try MoshSessionRig.serverDatagram())
        #expect(await eventually { await transport.currentDiagnostics.datagramsReceived == 1 })

        clock.now += 10_000   // the path goes quiet mid-session
        #expect(await rig.eventually { rig.session.moshReturnPathDead }, "the banner offering SSH")
        try await Task.sleep(for: .milliseconds(300))   // room for a reaper to dial
        #expect(!rig.killed(), "the server holding the user's shell was killed for a silence")
        #expect(rig.reaper.commands.isEmpty, "nothing should even have dialled")
        #expect(rig.session.viewModel.status == .connected)
        #expect(rig.session.moshTransport === transport, "the session keeps its transport")
        await rig.session.stop(forReconnect: true)
    }

    @Test("a server that never heard from the session is still reaped")
    func unreachedServerIsReaped() async throws {
        let rig = MoshSessionRig(transport: { credentials, channel in
            try MoshTransport(credentials: credentials, channel: channel, returnPathDeadlineNanos: 50_000_000)
        })
        await rig.start()
        #expect(rig.session.viewModel.status == .connected)
        rig.channel.onStateChange?(.ready)
        #expect(await rig.eventually { rig.session.moshReturnPathDead })
        #expect(await rig.eventually { rig.killed() }, "the unreached server waits forever otherwise")
        #expect(rig.session.viewModel.status == .connected)
        await rig.session.stop(forReconnect: true)
    }

    @Test("the rule itself")
    func rule() {
        #expect(MoshServerReap.onReturnPathDead(heardFromServer: false))
        #expect(!MoshServerReap.onReturnPathDead(heardFromServer: true))
    }

    private func eventually(_ timeout: Double = 5, _ predicate: () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return await predicate()
    }
}
