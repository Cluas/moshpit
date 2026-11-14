import Foundation
import Testing
@testable import Moshpit

/// B131: through a SOCKS proxy, the connect waits for the sign-in and reports
/// what a direct connect does. Citadel's `connect(on:authenticationMethod:…)`,
/// which the proxy path used, returned before host key check and sign-in — a
/// wrong password surfaced at the first channel as "can't reach the server" —
/// and built its session off the event loop, a precondition failure that
/// stopped the app on every proxied connect that got that far. These run the
/// app's real connect path (`SSHService` → Citadel → NIOSSH over TCP) against
/// an SSH server in the test process, its listener a SOCKS5 proxy first.
@Suite("Sign-in through a SOCKS proxy", .serialized)
struct SOCKSSignInTests {

    private static func service() -> SSHService {
        SSHService(keychain: .inMemory(), hostKeyValidator: .inMemory())
    }

    private static func connection(to server: LoopbackSSHServer, viaProxy: Bool) -> ServerConnection {
        var connection = ServerConnection(name: "t", host: viaProxy ? "ssh.example.com" : "127.0.0.1",
                                          port: viaProxy ? 22 : server.port, username: "me",
                                          authMethod: .password, keychainRef: nil)
        if viaProxy {
            connection.useSOCKSProxy = true
            connection.socksProxyHost = "127.0.0.1"
            connection.socksProxyPort = server.port
        }
        return connection
    }

    /// The error a connect throws, or nil when it connected (the session closed).
    private static func outcome(_ connection: ServerConnection, password: String,
                                trust: Bool = true) async -> SSHError? {
        do {
            let session = try await service().connect(connection, overrideSecret: password,
                                                      onUnknownHost: { _, _, _ in trust })
            await session.close()
            return nil
        } catch let error as SSHError {
            return error
        } catch {
            return .underlying(error)
        }
    }

    @Test("The right password signs in through the proxy, and a command runs")
    func signsInThroughTheProxy() async throws {
        let server = try await LoopbackSSHServer.start(.init(socks: true,
                                                              execs: ["echo hi": .exits(output: "hi\n", status: 0)]))
        let session = try await Self.service().connect(Self.connection(to: server, viaProxy: true),
                                                       overrideSecret: "right",
                                                       onUnknownHost: { _, _, _ in true })
        let output = try await session.executeCommand("echo hi")
        #expect(String(decoding: output, as: UTF8.self) == "hi\n")
        await session.close()
        await server.stop()
    }

    @Test("A wrong password through the proxy is a failed sign-in, as it is directly")
    func wrongPasswordReadsTheSame() async throws {
        let proxied = try await LoopbackSSHServer.start(.init(socks: true))
        let direct = try await LoopbackSSHServer.start()
        let throughProxy = await Self.outcome(Self.connection(to: proxied, viaProxy: true), password: "wrong")
        let directly = await Self.outcome(Self.connection(to: direct, viaProxy: false), password: "wrong")
        guard case .authenticationFailed? = throughProxy else {
            Issue.record("through the proxy: \(throughProxy?.diagnostic ?? "connected")")
            return
        }
        guard case .authenticationFailed? = directly else {
            Issue.record("directly: \(directly?.diagnostic ?? "connected")")
            return
        }
        await proxied.stop()
        await direct.stop()
    }

    @Test("A host key the user won't trust ends the proxied connect as it ends a direct one")
    func refusedHostKeyReadsTheSame() async throws {
        let proxied = try await LoopbackSSHServer.start(.init(socks: true))
        let direct = try await LoopbackSSHServer.start()
        let throughProxy = await Self.outcome(Self.connection(to: proxied, viaProxy: true), password: "right", trust: false)
        let directly = await Self.outcome(Self.connection(to: direct, viaProxy: false), password: "right", trust: false)
        #expect(throughProxy != nil, "connected though the host key was refused")
        #expect(throughProxy?.diagnostic == directly?.diagnostic,
                "proxy: \(throughProxy?.diagnostic ?? "-"), direct: \(directly?.diagnostic ?? "-")")
        if case .connectionFailed? = throughProxy { Issue.record("reads as an unreachable server") }
        await proxied.stop()
        await direct.stop()
    }
}
