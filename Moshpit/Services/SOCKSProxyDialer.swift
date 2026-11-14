import Foundation
import NIOCore
import NIOPosix
import NIOSOCKS

/// Dials a SOCKS5 proxy and hands back a `Channel` already past the CONNECT
/// handshake — a bare byte pipe to the target host, ready for another
/// protocol (Citadel's SSH handshake, via `SSHDial.signIn(on:…)`) to run over
/// it directly. Whatever the target sends before that protocol's handler is
/// in place is held for it (`InboundHold`).
///
/// Only unauthenticated SOCKS5 is supported: `NIOSOCKS.SOCKSClientHandler`
/// always sends `ClientGreeting(methods: [.noneRequired])` — its own source
/// comment says "no authentication currently supported" — so a proxy that
/// requires a username/password will reject the handshake.
///
/// Every way the dial can go wrong ends it, within one budget: the proxy
/// can't be reached, refuses the CONNECT, wants a password, isn't SOCKS,
/// closes, or says nothing. It used to listen for success only — NIOSOCKS
/// reports a failure as an error and closes the channel, which nobody heard —
/// and had no clock, so any of those left the session "connecting" for good
/// (B130).
enum SOCKSProxyDialer {
    enum DialError: Error, Equatable, CustomStringConvertible {
        /// No connection to the proxy itself.
        case proxyUnreachable(host: String, port: Int, reason: String)
        /// The proxy took the connection but didn't open one to the target:
        /// it refused, asked for a password, isn't a SOCKS5 proxy, closed,
        /// or didn't answer within the budget. `reason` is for the log.
        case proxyFailed(reason: String)

        var description: String {
            switch self {
            case .proxyUnreachable(let host, let port, let reason):
                return "SOCKS proxy \(host):\(port) unreachable: \(reason)"
            case .proxyFailed(let reason):
                return "SOCKS proxy failed: \(reason)"
            }
        }
    }

    /// How long a dial may take, proxy connect and SOCKS handshake together:
    /// the 15 s a direct dial gives its TCP connect.
    static let budget: TimeAmount = .seconds(15)

    private static let socksHandlerName = "moshpit.socks.client"
    private static let waiterHandlerName = "moshpit.socks.waiter"

    static func connect(proxyHost: String, proxyPort: Int, targetHost: String, targetPort: Int,
                        budget: TimeAmount = SOCKSProxyDialer.budget) async throws -> Channel {
        let deadline = NIODeadline.now() + budget
        // Matches Citadel's own default (`SSHClientSettings.group`) so the
        // proxy path doesn't spin up a second, redundant thread pool.
        let loop = MultiThreadedEventLoopGroup.singleton.any()
        let established = loop.makePromise(of: Channel.self)

        let bootstrap = ClientBootstrap(group: loop)
            .connectTimeout(budget)
            // As Citadel's own dial sets it: keystrokes go out as typed.
            .channelOption(ChannelOptions.socket(SocketOptionLevel(IPPROTO_TCP), TCP_NODELAY), value: 1)
            .channelInitializer { channel in
                // The initializer runs on the channel's own event loop, so the
                // handlers can go in synchronously. Neither handler is Sendable,
                // and the future-returning `addHandler` insists on that.
                channel.eventLoop.makeCompletedFuture {
                    let socksHandler = SOCKSClientHandler(targetAddress: .domain(targetHost, port: targetPort))
                    let waiter = ProxyEstablishedWaiter(promise: established)
                    try channel.pipeline.syncOperations.addHandler(socksHandler, name: socksHandlerName)
                    try channel.pipeline.syncOperations.addHandler(waiter, name: waiterHandlerName)
                    // The server's first bytes can come with the proxy's
                    // reply; they wait here for the SSH handler.
                    try channel.pipeline.syncOperations.addHandler(InboundHold())
                }
            }

        let channel: Channel
        do {
            channel = try await bootstrap.connect(host: proxyHost, port: proxyPort).get()
        } catch {
            let failure = DialError.proxyUnreachable(host: proxyHost, port: proxyPort,
                                                     reason: String(describing: error))
            established.fail(failure)
            throw failure
        }
        // What is left of the budget is the handshake's.
        let clock = loop.scheduleTask(deadline: deadline) {
            established.fail(DialError.proxyFailed(reason: "no answer within \(budget.nanoseconds / 1_000_000) ms"))
        }
        do {
            let ready = try await established.futureResult.get()
            clock.cancel()
            return ready
        } catch {
            clock.cancel()
            // Nothing may stay open behind a failed dial.
            channel.close(promise: nil)
            throw error
        }
    }

    /// Resolves the dial's promise once `SOCKSProxyEstablishedEvent` fires,
    /// removing both itself and the SOCKS handler first so whatever runs
    /// next (the SSH handshake) sees a clean pipeline with no SOCKS framing
    /// left in front of it — and fails it on the first sign the proxy won't
    /// get there: an error (NIOSOCKS reports a refused CONNECT, a proxy that
    /// wants a password or isn't SOCKS that way, then closes) or the channel
    /// closing.
    private final class ProxyEstablishedWaiter: ChannelInboundHandler {
        typealias InboundIn = ByteBuffer

        private let promise: EventLoopPromise<Channel>
        private var settled = false

        init(promise: EventLoopPromise<Channel>) {
            self.promise = promise
        }

        func userInboundEventTriggered(context: ChannelHandlerContext, event: Any) {
            guard event is SOCKSProxyEstablishedEvent, !settled else {
                context.fireUserInboundEventTriggered(event)
                return
            }
            settled = true
            let channel = context.channel
            let promise = self.promise
            channel.pipeline.removeHandler(name: socksHandlerName).whenComplete { _ in
                channel.pipeline.removeHandler(name: waiterHandlerName).whenComplete { _ in
                    promise.succeed(channel)
                }
            }
        }

        func errorCaught(context: ChannelHandlerContext, error: Error) {
            fail(DialError.proxyFailed(reason: String(describing: error)))
            context.close(promise: nil)
        }

        func channelInactive(context: ChannelHandlerContext) {
            fail(DialError.proxyFailed(reason: "the proxy closed the connection"))
            context.fireChannelInactive()
        }

        private func fail(_ error: Error) {
            guard !settled else { return }
            settled = true
            promise.fail(error)
        }
    }
}
