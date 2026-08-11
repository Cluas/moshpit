import Foundation
import NIOCore
import NIOPosix
import NIOSOCKS

/// Dials a SOCKS5 proxy and hands back a `Channel` already past the CONNECT
/// handshake — a bare byte pipe to the target host, ready for another
/// protocol (Citadel's SSH handshake, via `SSHClient.connect(on channel:...)`)
/// to run over it directly.
///
/// Only unauthenticated SOCKS5 is supported: `NIOSOCKS.SOCKSClientHandler`
/// always sends `ClientGreeting(methods: [.noneRequired])` — its own source
/// comment says "no authentication currently supported" — so a proxy that
/// requires a username/password will reject the handshake.
enum SOCKSProxyDialer {
    enum DialError: LocalizedError {
        case connectFailed(String)

        var errorDescription: String? {
            switch self {
            case .connectFailed(let why): return "SOCKS proxy connection failed: \(why)"
            }
        }
    }

    private static let socksHandlerName = "moshpit.socks.client"
    private static let waiterHandlerName = "moshpit.socks.waiter"

    static func connect(proxyHost: String, proxyPort: Int, targetHost: String, targetPort: Int) async throws -> Channel {
        // Matches Citadel's own default (`SSHClientSettings.group`) so the
        // proxy path doesn't spin up a second, redundant thread pool.
        let group = MultiThreadedEventLoopGroup.singleton
        let established = group.any().makePromise(of: Channel.self)

        let bootstrap = ClientBootstrap(group: group).channelInitializer { channel in
            let socksHandler = SOCKSClientHandler(targetAddress: .domain(targetHost, port: targetPort))
            let waiter = ProxyEstablishedWaiter(promise: established)
            return channel.pipeline.addHandler(socksHandler, name: socksHandlerName)
                .flatMap { channel.pipeline.addHandler(waiter, name: waiterHandlerName) }
        }

        do {
            _ = try await bootstrap.connect(host: proxyHost, port: proxyPort).get()
        } catch {
            established.fail(DialError.connectFailed(error.localizedDescription))
        }
        return try await established.futureResult.get()
    }

    /// Resolves the dial's promise once `SOCKSProxyEstablishedEvent` fires,
    /// removing both itself and the SOCKS handler first so whatever runs
    /// next (the SSH handshake) sees a clean pipeline with no SOCKS framing
    /// left in front of it.
    private final class ProxyEstablishedWaiter: ChannelInboundHandler {
        typealias InboundIn = ByteBuffer

        private let promise: EventLoopPromise<Channel>

        init(promise: EventLoopPromise<Channel>) {
            self.promise = promise
        }

        func userInboundEventTriggered(context: ChannelHandlerContext, event: Any) {
            guard event is SOCKSProxyEstablishedEvent else {
                context.fireUserInboundEventTriggered(event)
                return
            }
            let channel = context.channel
            let promise = self.promise
            channel.pipeline.removeHandler(name: socksHandlerName).whenComplete { _ in
                channel.pipeline.removeHandler(name: waiterHandlerName).whenComplete { _ in
                    promise.succeed(channel)
                }
            }
        }
    }
}
