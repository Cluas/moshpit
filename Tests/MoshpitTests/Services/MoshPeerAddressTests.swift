import Foundation
import NIOCore
import Testing
@testable import Moshpit

/// B171: mosh's UDP goes to the address the SSH connection reached.
///
/// It went to a fresh resolution of the host NAME. `mosh-server -s` binds to
/// the address SSH came in on, and a name with several addresses can resolve
/// to another one: `localhost` gives `::1` first, and a server bound to
/// 127.0.0.1 never hears anything sent there — connected, and black
/// (scripts/verify/mosh-live reach shows it against a real mosh-server).
/// crates/ssh's `peer_addr`: the address the TCP leg reached, none through a
/// proxy.
@Suite("mosh peer address", .serialized)
@MainActor
struct MoshPeerAddressTests {

    private static func service() -> SSHService {
        SSHService(keychain: .inMemory(), hostKeyValidator: .inMemory())
    }

    @Test("a real connection knows the address it reached; a proxied one doesn't claim the proxy's")
    func theConnectionKnowsWhereItWent() async throws {
        // The test server listens on 127.0.0.1 only; `localhost` resolves to
        // ::1 as well, and the dial lands where something answers.
        let server = try await LoopbackSSHServer.start()
        let connection = ServerConnection(name: "peer", host: "localhost", port: server.port,
                                          username: "me", authMethod: .password, keychainRef: nil)
        let session = try await Self.service().connect(connection, overrideSecret: "right",
                                                       onUnknownHost: { _, _, _ in true })
        #expect(session.peerAddress == "127.0.0.1")
        await session.close()
        await server.stop()

        let proxy = try await LoopbackSSHServer.start(.init(socks: true))
        var proxied = ServerConnection(name: "peer", host: "ssh.example.com", port: 22,
                                       username: "me", authMethod: .password, keychainRef: nil)
        proxied.useSOCKSProxy = true
        proxied.socksProxyHost = "127.0.0.1"
        proxied.socksProxyPort = proxy.port
        let throughProxy = try await Self.service().connect(proxied, overrideSecret: "right",
                                                            onUnknownHost: { _, _, _ in true })
        #expect(throughProxy.peerAddress == nil, "the channel's peer is the proxy, not the host")
        await throughProxy.close()
        await proxy.stop()
    }

    @Test("mosh's UDP goes where the bootstrap's SSH connection went")
    func moshTargetsThePeer() async throws {
        let rig = MoshSessionRig(peer: "198.51.100.7")
        await rig.start()
        #expect(rig.session.viewModel.status == .connected)
        #expect(rig.built.map(\.host) == ["198.51.100.7"],
                "UDP to \(rig.built.map(\.host)), not where SSH went")
        await rig.session.stop(forReconnect: true)

        // No address to go by (a proxy, an unknown peer): the name.
        let named = MoshSessionRig()
        await named.start()
        #expect(named.built.map(\.host) == [named.connection.host])
        await named.session.stop(forReconnect: true)
    }

    @Test("a link-local peer keeps its interface")
    func linkLocalKeepsItsScope() throws {
        var v6 = sockaddr_in6()
        v6.sin6_len = UInt8(MemoryLayout<sockaddr_in6>.size)
        v6.sin6_family = sa_family_t(AF_INET6)
        v6.sin6_port = in_port_t(22).bigEndian
        inet_pton(AF_INET6, "fe80::1", &v6.sin6_addr)
        v6.sin6_scope_id = if_nametoindex("lo0")
        let scoped = try #require(CitadelClientTransport.numericHost(.init(v6, host: "fe80::1")))
        #expect(scoped == "fe80::1%lo0")
        let plain = try SocketAddress(ipAddress: "192.0.2.9", port: 22)
        #expect(CitadelClientTransport.numericHost(plain) == "192.0.2.9")
        #expect(CitadelClientTransport.numericHost(nil) == nil)
    }
}
