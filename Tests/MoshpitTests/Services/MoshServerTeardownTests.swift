import Foundation
import Network
import Testing
@testable import Moshpit

/// B164: a mosh-server the user closed is stopped, even one the session
/// talked to.
///
/// `stop()` only ever stopped a server that had never answered, on the
/// belief that mosh-server outlives its client so a later connect can resume
/// it. This app keeps no session key and starts a new server on every
/// connect: nothing ever came back, and every disconnect and protocol switch
/// left a mosh-server and the user's shell running on the host. crates/
/// session's rule: the person's disconnect and switch to SSH stop the
/// server; a drop stops only one never reached; the server's own end leaves
/// nothing to stop.
@Suite("mosh server at teardown", .serialized)
@MainActor
struct MoshServerTeardownTests {

    /// A session up through the hub's real startMosh that has heard from its
    /// server (the hub's own first-contact latch set, the way the transport's
    /// diagnostics set it).
    private func contactedRig() async throws -> MoshSessionRig {
        let rig = MoshSessionRig()
        var seen = MoshDiagnostics()
        rig.session.diagnosticsSink = { seen = $0 }
        await rig.start()
        #expect(rig.session.viewModel.status == .connected)
        rig.channel.onStateChange?(.ready)
        rig.channel.deliver(try MoshSessionRig.serverDatagram())
        #expect(await rig.eventually { seen.datagramsReceived > 0 }, "the server answered")
        return rig
    }

    @Test("disconnecting a mosh session the server answered stops the server")
    func disconnectStopsContactedServer() async throws {
        let rig = try await contactedRig()
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        hub.makeSession = { _ in rig.session }
        #expect(hub.prepare(rig.connection) === rig.session)

        await hub.disconnect(rig.connection.id)   // the user's disconnect, and a protocol switch's first half
        #expect(rig.killed(), "the server and its shell were left running on the host")
        #expect(rig.session.moshTransport == nil)
    }

    @Test("a reconnect leaves a server the session answered alone")
    func reconnectKeepsContactedServer() async throws {
        let rig = try await contactedRig()
        await rig.session.stop(forReconnect: true)
        try await Task.sleep(for: .milliseconds(100))
        #expect(!rig.killed())
        #expect(rig.reaper.commands.isEmpty)
    }

    @Test("a session the server ended is not killed after the fact")
    func serverEndedIsNotKilled() async throws {
        let rig = try await contactedRig()
        let transport = try #require(rig.session.moshTransport)
        rig.session.moshSessionEnded(transport)   // mosh-server sent its goodbye
        #expect(rig.session.viewModel.endedByServer)
        await rig.session.stop()                  // what the hub's endedSink runs
        try await Task.sleep(for: .milliseconds(100))
        #expect(!rig.killed(), "a pid whose server quit may be another process's by now")
    }

    @Test("the rule itself")
    func rule() {
        for contacted in [false, true] {
            #expect(MoshServerReap.onTeardown(.userClosed, contacted: contacted))
            #expect(!MoshServerReap.onTeardown(.ended, contacted: contacted))
        }
        #expect(MoshServerReap.onTeardown(.reconnect, contacted: false))
        #expect(!MoshServerReap.onTeardown(.reconnect, contacted: true))
    }
}
