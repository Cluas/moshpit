import Foundation
import Network
import Testing
@testable import Moshpit

/// B92, the session layer's half: the remote command exits (`exit` in the
/// shell), mosh-server ends the session with its shutdown state, and the
/// transport acks it and closes (`MoshTransportFixesTests`, B92). The
/// session used to stay in the hub with a frozen screen until the liveness
/// check offered to switch to SSH.
@Suite("mosh session end")
@MainActor
struct MoshSessionEndTests {

    let key = Data(repeating: 5, count: 16)

    /// The session layer's half: a mosh session whose server ended it shows
    /// why and leaves the hub, the way Disconnect takes it — a dead session
    /// left in the hub is redialled by the next keepalive.
    @Test("a mosh session its server ended leaves the hub with the reason, not redialled")
    func theHubRetiresASessionItsServerEnded() async throws {
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        var connection = ServerConnection(name: "ended", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        connection.connectionProtocol = .mosh
        let session = hub.prepare(connection)
        let transport = try MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: SessionLifecycleTests.IdleDatagramChannel())
        session.installForTesting(moshTransport: transport)
        session.moshSessionEnded(transport)

        let reason = String(localized: "The session ended: the remote shell exited or mosh-server was stopped.")
        #expect(session.viewModel.errorMessage == reason)
        let deadline = ContinuousClock.now + .seconds(5)
        while hub.session(for: connection) != nil, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(hub.session(for: connection) == nil, "left in the hub, the keepalive redials it")
        #expect(session.isStopping)
        #expect(session.viewModel.status == .ended(reason), "an end is not a failure")
        #expect(session.moshTransport == nil)

        // A transport that is no longer the session's is not its end.
        let other = hub.prepare(connection)
        let stale = try MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: SessionLifecycleTests.IdleDatagramChannel())
        other.moshSessionEnded(stale)
        #expect(other.viewModel.errorMessage == nil)
        #expect(hub.session(for: connection) === other)
    }
}
