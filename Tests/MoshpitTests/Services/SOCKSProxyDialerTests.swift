import Foundation
import NIOCore
import NIOPosix
import Testing
@testable import Moshpit

/// B130: through a SOCKS proxy, every way the proxy can fail ends the dial —
/// promptly, or at the budget for one that never answers. The dialler used to
/// wait for the proxy's success and nothing else: a refused CONNECT, a proxy
/// that wants a password, one that isn't SOCKS, one that closes or one that
/// says nothing all left the session "connecting" for good. These proxies are
/// fakes on 127.0.0.1, scripted byte for byte from RFC 1928.
@Suite("SOCKS proxy dial: failures end it", .serialized)
struct SOCKSProxyDialerTests {

    /// How the fake proxy answers the client's greeting and request.
    enum Script: Sendable {
        /// Takes no-auth, then answers the CONNECT with this reply code
        /// (0 = succeeded, 5 = connection refused, …).
        case reply(UInt8)
        /// Selects username/password (0x02), which the client didn't offer.
        case wantsPassword
        /// "No acceptable methods" (0xFF).
        case noAcceptableMethods
        /// Answers like a web server.
        case notSOCKS
        /// Closes the connection after the greeting.
        case hangUp
        /// Reads, and never answers.
        case silent
    }

    /// The dial's outcome within `seconds`: "connected", "failed: …", or nil
    /// for a dial still waiting then.
    private func dial(_ script: Script, budget: TimeAmount = .seconds(15),
                      within seconds: Double = 5) async throws -> String? {
        let proxy = try await FakeSOCKSProxy.start(script)
        let outcome = await withTimeoutValue(seconds) {
            do {
                let channel = try await SOCKSProxyDialer.connect(
                    proxyHost: "127.0.0.1", proxyPort: proxy.port,
                    targetHost: "ssh.example.com", targetPort: 22, budget: budget)
                try? await channel.close()
                return "connected"
            } catch {
                return "failed: \(error)"
            }
        }
        await proxy.stop()
        return outcome
    }

    @Test("A proxy that opens the connection hands it back")
    func success() async throws {
        #expect(try await dial(.reply(0)) == "connected")
    }

    @Test("A refused CONNECT, a password demand, no usable method, a web server, a hang-up: each fails at once",
          arguments: [Script.reply(5), .reply(1), .wantsPassword, .noAcceptableMethods, .notSOCKS, .hangUp])
    func failuresEndTheDial(_ script: Script) async throws {
        let started = Date()
        let outcome = try await dial(script)
        #expect(outcome?.hasPrefix("failed: SOCKS proxy failed") == true, "\(String(describing: outcome))")
        #expect(Date().timeIntervalSince(started) < 3, "took \(Date().timeIntervalSince(started)) s")
    }

    @Test("A proxy that never answers fails when the budget runs out")
    func silenceEndsAtTheBudget() async throws {
        let started = Date()
        let outcome = try await dial(.silent, budget: .milliseconds(700), within: 5)
        #expect(outcome?.hasPrefix("failed: SOCKS proxy failed") == true, "\(String(describing: outcome))")
        let took = Date().timeIntervalSince(started)
        #expect(took >= 0.6 && took < 3, "took \(took) s")
    }

    @Test("A proxy nobody listens for is unreachable, not a hang")
    func unreachableProxy() async throws {
        // A port that was free a moment ago: bound, then closed.
        let proxy = try await FakeSOCKSProxy.start(.silent)
        let port = proxy.port
        await proxy.stop()
        let outcome = await withTimeoutValue(5) {
            do {
                _ = try await SOCKSProxyDialer.connect(proxyHost: "127.0.0.1", proxyPort: port,
                                                       targetHost: "ssh.example.com", targetPort: 22)
                return "connected"
            } catch {
                return "failed: \(error)"
            }
        }
        #expect(outcome?.hasPrefix("failed: SOCKS proxy 127.0.0.1:\(port) unreachable") == true,
                "\(String(describing: outcome))")
    }

    @Test("Proxy failures read as the proxy's, not the server's")
    func failuresAreNamed() {
        let unreachable = SSHError.map(SOCKSProxyDialer.DialError.proxyUnreachable(host: "p", port: 1080, reason: "x"))
        guard case .proxyUnreachable("p:1080") = unreachable else {
            Issue.record("\(unreachable.diagnostic)")
            return
        }
        guard case .proxyFailed = SSHError.map(SOCKSProxyDialer.DialError.proxyFailed(reason: "x")) else {
            Issue.record("not proxyFailed")
            return
        }
        #expect(unreachable.description.contains("p:1080"))
        #expect(!SSHError.proxyFailed.description.isEmpty)
    }
}

/// A SOCKS5 proxy on 127.0.0.1 that answers by `script`.
final class FakeSOCKSProxy: @unchecked Sendable {
    let port: Int
    private let channel: Channel

    private init(port: Int, channel: Channel) {
        self.port = port
        self.channel = channel
    }

    static func start(_ script: SOCKSProxyDialerTests.Script) async throws -> FakeSOCKSProxy {
        let channel = try await ServerBootstrap(group: MultiThreadedEventLoopGroup.singleton)
            .serverChannelOption(ChannelOptions.socketOption(.so_reuseaddr), value: 1)
            .childChannelInitializer { child in
                child.eventLoop.makeCompletedFuture {
                    try child.pipeline.syncOperations.addHandler(Responder(script))
                }
            }
            .bind(host: "127.0.0.1", port: 0).get()
        return FakeSOCKSProxy(port: channel.localAddress!.port!, channel: channel)
    }

    func stop() async {
        try? await channel.close()
    }

    private final class Responder: ChannelInboundHandler {
        typealias InboundIn = ByteBuffer
        typealias OutboundOut = ByteBuffer

        private let script: SOCKSProxyDialerTests.Script
        private var buffer = ByteBuffer()
        private var greeted = false

        init(_ script: SOCKSProxyDialerTests.Script) { self.script = script }

        func channelRead(context: ChannelHandlerContext, data: NIOAny) {
            var incoming = unwrapInboundIn(data)
            buffer.writeBuffer(&incoming)
            if !greeted {
                // VER, NMETHODS, METHODS…
                guard buffer.readableBytes >= 2,
                      let count = buffer.getInteger(at: buffer.readerIndex + 1, as: UInt8.self),
                      buffer.readableBytes >= 2 + Int(count) else { return }
                buffer.moveReaderIndex(forwardBy: 2 + Int(count))
                greeted = true
                switch script {
                case .reply: send(context, [0x05, 0x00])
                case .wantsPassword: send(context, [0x05, 0x02])
                case .noAcceptableMethods: send(context, [0x05, 0xFF])
                case .notSOCKS: send(context, Array("HTTP/1.1 400 Bad Request\r\n\r\n".utf8))
                case .hangUp: context.close(promise: nil)
                case .silent: break
                }
                return
            }
            // VER CMD RSV ATYP(3) LEN DOMAIN PORT
            guard case .reply(let code) = script, buffer.readableBytes >= 5,
                  let length = buffer.getInteger(at: buffer.readerIndex + 4, as: UInt8.self),
                  buffer.readableBytes >= 5 + Int(length) + 2 else { return }
            buffer.moveReaderIndex(forwardBy: 5 + Int(length) + 2)
            send(context, [0x05, code, 0x00, 0x01, 127, 0, 0, 1, 0, 22])
            if code != 0 { context.close(promise: nil) }
        }

        private func send(_ context: ChannelHandlerContext, _ bytes: [UInt8]) {
            context.writeAndFlush(wrapOutboundOut(ByteBuffer(bytes: bytes)), promise: nil)
        }
    }
}
