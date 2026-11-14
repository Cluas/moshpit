import Foundation
import Network
@testable import Moshpit

/// A mosh session brought up by the hub's real `start()` → `startMosh()` over
/// fakes: the bootstrap connection is a ``FakeSSHHost`` that reports a
/// mosh-server and starts one (pid ``pid``), the transport runs over a fake
/// datagram channel, and the connections the session dials on the side (the
/// reaper's `kill`, a sidecar) all reach ``reaper``.
@MainActor
final class MoshSessionRig {
    nonisolated static let pid = 4242
    /// mosh-server prints the key as 22 unpadded base64 characters.
    nonisolated static let keyText = "AAAAAAAAAAAAAAAAAAAAAA"
    nonisolated static let key = Data(repeating: 0, count: 16)
    nonisolated static let probeOutput = "/usr/bin/mosh-server\n::Linux::/usr/bin/apt-get\n"
    nonisolated static let bootstrapOutput = "MOSH CONNECT 60001 \(keyText)\n[mosh-server detached, pid = \(pid)]\n"

    let connection: ServerConnection
    let host: FakeSSHHost
    let reaper = FakeSSHHost()
    let channel = MoshTransportStateMachineTests.FakeDatagramChannel()
    let session: SessionHub.ActiveSession
    /// Every set of credentials the session built a transport from.
    private(set) var built: [MoshCredentials] = []

    /// `transport` builds the session's transport over the rig's channel
    /// (defaults: the production timings).
    init(multiplexer: Multiplexer = .none,
         peer: String? = nil,
         answer: (@Sendable (String) -> FakeSSHHost.Reply)? = nil,
         transport: ((MoshCredentials, DatagramChannel) throws -> MoshTransport)? = nil) {
        var connection = ServerConnection(
            name: "mosh-rig",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .mosh
        connection.multiplexer = multiplexer
        self.connection = connection
        let host = FakeSSHHost(peer: peer, answer: answer ?? { MoshSessionRig.answer($0) })
        self.host = host
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        self.session = session
        session.probeTimeout = 0.2
        let reaper = self.reaper
        session.sidecarConnect = { connection in SSHSession(connection: connection, transport: reaper) }
        let channel = self.channel
        let make = transport ?? { credentials, channel in try MoshTransport(credentials: credentials, channel: channel) }
        session.makeMoshTransport = { [weak self] credentials in
            self?.built.append(credentials)
            return try make(credentials, channel)
        }
    }

    /// The host as a mosh host: the probe finds mosh-server, `mosh-server
    /// new` starts one.
    nonisolated static func answer(_ command: String) -> FakeSSHHost.Reply {
        if command.contains("command -v tmux mosh-server") { return .output(probeOutput) }
        if command.contains("mosh-server new") { return .output(bootstrapOutput) }
        return .output("")
    }

    func start() async {
        await session.start(theme: .githubDark, fontSize: 14)
    }

    /// Whether the side connections were asked to kill `pid`.
    func killed(_ pid: Int = MoshSessionRig.pid) -> Bool {
        reaper.commands.contains { $0.contains("kill \(pid)") }
    }

    /// A datagram from the server, sealed with the session key: what a real
    /// server's first frame looks like to the transport.
    static func serverDatagram(seq: UInt64 = 0, newNum: UInt64 = 1) throws -> Data {
        var instruction = TransportInstruction()
        instruction.oldNum = 0
        instruction.newNum = newNum
        instruction.ackNum = 0
        instruction.diff = Data()
        let compressed = try MoshCompression.compress(instruction.encoded())
        let fragment = TransportFragment.fragment(payload: compressed, id: 1, mtu: 1300)[0]
        let message = MoshCrypto.Message(timestamp: 0, timestampReply: MoshTransport.noTimestamp,
                                         payload: fragment.encoded())
        return try MoshCrypto(key: key).seal(message, seq: seq, direction: .toClient)
    }

    func eventually(_ timeout: Double = 5, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }
}
