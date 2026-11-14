import Citadel
import Foundation
import NIOCore
import NIOPosix
import NIOSSH

/// Signing in over a TCP connection this side dialed — directly
/// (``directChannel(host:port:timeout:)``) or through a SOCKS proxy
/// (`SOCKSProxyDialer`) — so that every way a connect can fail closes what it
/// opened.
///
/// Citadel's `SSHClient.connect(on:authenticationMethod:…)` — what the proxy
/// path used — returns before the server has seen a host key check or a
/// sign-in: a wrong password, a refused key, a locked keychain all surfaced at
/// the first channel, as "can't reach the server" (B131). Worse, it builds the
/// session off the channel's event loop, which NIO's loop-bound box refuses
/// with a precondition: every proxied connect that got that far stopped the
/// app. ``signIn(on:authenticationMethod:hostKeyValidator:)`` uses
/// Citadel's other entry, which waits for the sign-in as a direct connect
/// does, and runs it on the channel's event loop (``EventLoopTaskExecutor``),
/// where Citadel's pipeline and session code must run.
enum SSHDial {
    /// A TCP connection to `host`:`port`, set up as Citadel's own dial sets
    /// one up, holding what the server sends for the SSH handler
    /// (``InboundHold``).
    static func directChannel(host: String, port: Int, timeout: TimeAmount) async throws -> Channel {
        try await ClientBootstrap(group: MultiThreadedEventLoopGroup.singleton.any())
            .connectTimeout(timeout)
            .channelOption(ChannelOptions.socket(SocketOptionLevel(SOL_SOCKET), SO_REUSEADDR), value: 1)
            .channelOption(ChannelOptions.socket(SocketOptionLevel(IPPROTO_TCP), TCP_NODELAY), value: 1)
            .channelInitializer { channel in
                channel.eventLoop.makeCompletedFuture {
                    try channel.pipeline.syncOperations.addHandler(InboundHold())
                }
            }
            .connect(host: host, port: port).get()
    }

    /// Sign in over `channel`, a connection to an SSH server, and return the
    /// client once the server has accepted the host key check and the
    /// sign-in. Throws what went wrong, as a direct connect does; a failure
    /// closes the channel.
    ///
    /// The channel must hold what the server sends until the SSH handler is
    /// there to read it — ``InboundHold`` — since sshd speaks first.
    static func signIn(on channel: Channel,
                       authenticationMethod: SSHAuthenticationMethod,
                       hostKeyValidator: SSHHostKeyValidator) async throws -> SSHClient {
        // Host and port are Citadel's for reconnecting, which the app never
        // asks of it; the connection is the channel.
        let settings = SSHClientSettings(host: channel.remoteAddress?.ipAddress ?? "", port: channel.remoteAddress?.port ?? 22,
                                         authenticationMethod: { authenticationMethod },
                                         hostKeyValidator: hostKeyValidator)
        do {
            return try await withTaskExecutorPreference(EventLoopTaskExecutor(channel.eventLoop)) {
                try await SSHClient.connect(on: channel, settings: settings)
            }
        } catch {
            channel.close(promise: nil)
            throw error
        }
    }
}

/// Runs Swift-concurrency jobs on a NIO event loop's own thread, so async code
/// that touches a channel's pipeline synchronously — Citadel's connect — may.
/// Used as a task executor preference: whatever nonisolated async code runs
/// under it runs on the loop, between the loop's own work, never during it.
final class EventLoopTaskExecutor: TaskExecutor, @unchecked Sendable {
    private let eventLoop: EventLoop

    init(_ eventLoop: EventLoop) {
        self.eventLoop = eventLoop
    }

    func enqueue(_ job: consuming ExecutorJob) {
        let job = UnownedJob(job)
        eventLoop.execute {
            job.runSynchronously(on: self.asUnownedTaskExecutor())
        }
    }
}

/// Holds whatever arrives on a fresh connection until the SSH handler is in
/// the pipeline to read it, then hands it all on, in order, and leaves.
///
/// sshd sends its version line the moment the connection is up — through a
/// proxy, in the same read as the proxy's own reply, often. Between the dial
/// and Citadel adding its handlers the pipeline had nobody to give it to, and
/// bytes that reach the end of a pipeline are dropped: the handshake then
/// waited for a version line it had already been sent. The SSH handler's
/// first write (its own version line) says it is there; what was held goes
/// to it once that write has gone out.
final class InboundHold: ChannelDuplexHandler, RemovableChannelHandler {
    typealias InboundIn = ByteBuffer
    typealias InboundOut = ByteBuffer
    typealias OutboundIn = ByteBuffer
    typealias OutboundOut = ByteBuffer

    private var held: [NIOAny] = []
    private var heldReadComplete = false
    private var heldInactive = false
    private var releasing = false

    init() {}

    func channelRead(context: ChannelHandlerContext, data: NIOAny) {
        held.append(data)
    }

    func channelReadComplete(context: ChannelHandlerContext) {
        heldReadComplete = true
    }

    func channelInactive(context: ChannelHandlerContext) {
        // In order: after what came before it.
        heldInactive = true
    }

    func write(context: ChannelHandlerContext, data: NIOAny, promise: EventLoopPromise<Void>?) {
        if !releasing {
            releasing = true
            // Not from inside the SSH handler's own write: once it returns.
            context.eventLoop.execute {
                context.pipeline.syncOperations.removeHandler(context: context, promise: nil)
            }
        }
        context.write(data, promise: promise)
    }

    func removeHandler(context: ChannelHandlerContext, removalToken: ChannelHandlerContext.RemovalToken) {
        let held = self.held
        self.held = []
        for data in held {
            context.fireChannelRead(data)
        }
        if heldReadComplete || !held.isEmpty {
            context.fireChannelReadComplete()
        }
        if heldInactive {
            context.fireChannelInactive()
        }
        context.leavePipeline(removalToken: removalToken)
    }
}
