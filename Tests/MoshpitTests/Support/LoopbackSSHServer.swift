import Crypto
import Foundation
import NIOCore
import NIOPosix
import NIOSSH

/// An SSH server inside the test process, on 127.0.0.1: NIOSSH's server role
/// with a fresh Ed25519 host key, one password it accepts, and execs it
/// answers by script — so the app's real connect path (Citadel over TCP) can
/// be driven on the device, with no sshd anywhere.
///
/// With `socks`, the listener is a SOCKS5 proxy first: it answers the greeting
/// and the CONNECT itself, whatever the target, then speaks SSH on the same
/// connection — proxy and server in one.
final class LoopbackSSHServer: @unchecked Sendable {
    /// How the server answers an exec.
    enum Exec: Sendable {
        /// Prints `output`, then sends `status` and closes, as sshd does.
        case exits(output: String, status: Int)
        /// Prints `output`, then closes the channel with no exit status.
        case closes(output: String)
        /// Prints `output`, then reports the command killed by `signal`.
        case killed(output: String, signal: String)
        /// Prints to stdout, stderr, then stdout again, and exits 0.
        case interleaved(stdout: String, stderr: String, more: String)
        /// Prints `output`, then drops the whole connection.
        case dropsConnection(output: String)
    }

    struct Options: Sendable {
        var password = "right"
        var socks = false
        /// Answers by script, keyed by what sh is handed: the client sends
        /// every command as `sh -c <word>` (B138), and a host's shell hands
        /// sh the word.
        var execs: [String: Exec] = [:]
        /// Every exec request exactly as it came, when a test asks.
        var requests: RequestLog?
    }

    /// The exec requests a server saw, as they came.
    final class RequestLog: @unchecked Sendable {
        private let lock = NSLock()
        private var seen: [String] = []
        var all: [String] { lock.withLock { seen } }
        func record(_ request: String) { lock.withLock { seen.append(request) } }
    }

    let port: Int
    let hostKey: NIOSSHPrivateKey
    private let channel: Channel
    private let lock = NSLock()
    private var opened = 0
    private var open = 0

    /// Connections accepted so far.
    var accepted: Int { lock.withLock { opened } }
    /// Connections still open.
    var stillOpen: Int { lock.withLock { open } }

    private init(port: Int, hostKey: NIOSSHPrivateKey, channel: Channel) {
        self.port = port
        self.hostKey = hostKey
        self.channel = channel
    }

    static func start(_ options: Options = Options()) async throws -> LoopbackSSHServer {
        let hostKey = NIOSSHPrivateKey(ed25519Key: Curve25519.Signing.PrivateKey())
        let tally = Tally()
        let channel = try await ServerBootstrap(group: MultiThreadedEventLoopGroup.singleton)
            .serverChannelOption(ChannelOptions.socketOption(.so_reuseaddr), value: 1)
            .childChannelInitializer { child in
                tally.opened()
                child.closeFuture.whenComplete { _ in tally.closed() }
                return child.eventLoop.makeCompletedFuture {
                    if options.socks {
                        try child.pipeline.syncOperations.addHandler(
                            SOCKSFront(then: { Self.sshHandler(hostKey, options, child) }))
                    } else {
                        try child.pipeline.syncOperations.addHandler(Self.sshHandler(hostKey, options, child))
                    }
                }
            }
            .bind(host: "127.0.0.1", port: 0).get()
        let server = LoopbackSSHServer(port: channel.localAddress!.port!, hostKey: hostKey, channel: channel)
        tally.server = server
        return server
    }

    /// The host key's fingerprint, as the app shows and stores it.
    var fingerprint: String {
        var buffer = ByteBuffer()
        hostKey.publicKey.write(to: &buffer)
        let digest = SHA256.hash(data: Data(buffer.readableBytesView))
        return "SHA256:" + Data(digest).base64EncodedString().trimmingCharacters(in: CharacterSet(charactersIn: "="))
    }

    func stop() async {
        try? await channel.close()
    }

    /// Waits up to `seconds` for `condition`; whether it held.
    func waitUntil(_ seconds: Double = 3, _ condition: (LoopbackSSHServer) -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if condition(self) { return true }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return condition(self)
    }

    private final class Tally: @unchecked Sendable {
        weak var server: LoopbackSSHServer?
        private var pendingOpened = 0
        private var pendingClosed = 0
        private let lock = NSLock()

        func opened() {
            lock.withLock {
                if let server {
                    server.lock.withLock { server.opened += 1; server.open += 1 }
                } else {
                    pendingOpened += 1
                }
            }
        }

        func closed() {
            lock.withLock {
                if let server {
                    server.lock.withLock { server.open -= 1 }
                } else {
                    pendingClosed += 1
                }
            }
        }
    }

    private static func sshHandler(_ hostKey: NIOSSHPrivateKey, _ options: Options, _ child: Channel) -> NIOSSHHandler {
        NIOSSHHandler(
            role: .server(SSHServerConfiguration(hostKeys: [hostKey],
                                                 userAuthDelegate: PasswordCheck(password: options.password))),
            allocator: child.allocator,
            inboundChildChannelInitializer: { session, _ in
                session.eventLoop.makeCompletedFuture {
                    try session.pipeline.syncOperations.addHandler(
                        ExecResponder(execs: options.execs, requests: options.requests))
                }
            })
    }

    private final class PasswordCheck: NIOSSHServerUserAuthenticationDelegate {
        let password: String
        init(password: String) { self.password = password }

        var supportedAuthenticationMethods: NIOSSHAvailableUserAuthenticationMethods { .password }

        func requestReceived(request: NIOSSHUserAuthenticationRequest,
                             responsePromise: EventLoopPromise<NIOSSHUserAuthenticationOutcome>) {
            guard case .password(let given) = request.request else {
                responsePromise.succeed(.failure)
                return
            }
            responsePromise.succeed(given.password == password ? .success : .failure)
        }
    }

    /// One session channel: answers an exec by script.
    private final class ExecResponder: ChannelDuplexHandler {
        typealias InboundIn = SSHChannelData
        typealias OutboundIn = SSHChannelData
        typealias OutboundOut = SSHChannelData

        private let execs: [String: Exec]
        private let requests: RequestLog?
        init(execs: [String: Exec], requests: RequestLog?) {
            self.execs = execs
            self.requests = requests
        }

        func handlerAdded(context: ChannelHandlerContext) {
            context.channel.setOption(ChannelOptions.allowRemoteHalfClosure, value: true).whenFailure { _ in }
        }

        func userInboundEventTriggered(context: ChannelHandlerContext, event: Any) {
            // No SFTP here, as on a host with the subsystem turned off: the
            // request is refused and its channel closed.
            if let subsystem = event as? SSHChannelRequestEvent.SubsystemRequest {
                if subsystem.wantReply {
                    context.channel.triggerUserOutboundEvent(ChannelFailureEvent(), promise: nil)
                }
                context.close(promise: nil)
                return
            }
            guard let exec = event as? SSHChannelRequestEvent.ExecRequest else {
                context.fireUserInboundEventTriggered(event)
                return
            }
            let channel = context.channel
            requests?.record(exec.command)
            let script = PosixShell.shPayload(exec.command) ?? exec.command
            let reply = execs[script] ?? .exits(output: "", status: 127)
            let accepted: EventLoopFuture<Void> = exec.wantReply
                ? channel.triggerUserOutboundEvent(ChannelSuccessEvent())
                : channel.eventLoop.makeSucceededVoidFuture()
            func write(_ text: String, _ type: SSHChannelData.DataType = .channel) -> EventLoopFuture<Void> {
                guard !text.isEmpty else { return channel.eventLoop.makeSucceededVoidFuture() }
                return channel.writeAndFlush(SSHChannelData(type: type, data: .byteBuffer(ByteBuffer(string: text))))
            }
            accepted.flatMap { () -> EventLoopFuture<Void> in
                switch reply {
                case .exits(let text, _), .closes(let text), .killed(let text, _), .dropsConnection(let text):
                    return write(text)
                case .interleaved(let out, let err, let more):
                    return write(out).flatMap { write(err, .stdErr) }.flatMap { write(more) }
                }
            }.flatMap { () -> EventLoopFuture<Void> in
                switch reply {
                case .exits(_, let status):
                    return channel.triggerUserOutboundEvent(SSHChannelRequestEvent.ExitStatus(exitStatus: status))
                case .interleaved:
                    return channel.triggerUserOutboundEvent(SSHChannelRequestEvent.ExitStatus(exitStatus: 0))
                case .closes:
                    return channel.eventLoop.makeSucceededVoidFuture()
                case .killed(_, let signal):
                    return channel.triggerUserOutboundEvent(SSHChannelRequestEvent.ExitSignal(
                        signalName: signal, errorMessage: "", language: "", dumpedCore: false))
                case .dropsConnection:
                    // The connection under the channel goes, not the channel.
                    channel.parent?.close(promise: nil)
                    return channel.eventLoop.makeSucceededVoidFuture()
                }
            }.whenComplete { _ in
                channel.close(promise: nil)
            }
        }
    }

    /// The SOCKS5 half of a `socks` server: no-auth, any CONNECT succeeds;
    /// then it hands the connection to SSH.
    private final class SOCKSFront: ChannelInboundHandler, RemovableChannelHandler {
        typealias InboundIn = ByteBuffer
        typealias OutboundOut = ByteBuffer

        private let then: () -> NIOSSHHandler
        private var buffer = ByteBuffer()
        private var greeted = false

        init(then: @escaping () -> NIOSSHHandler) { self.then = then }

        func channelRead(context: ChannelHandlerContext, data: NIOAny) {
            var incoming = unwrapInboundIn(data)
            buffer.writeBuffer(&incoming)
            if !greeted {
                guard buffer.readableBytes >= 2,
                      let count = buffer.getInteger(at: buffer.readerIndex + 1, as: UInt8.self),
                      buffer.readableBytes >= 2 + Int(count) else { return }
                buffer.moveReaderIndex(forwardBy: 2 + Int(count))
                greeted = true
                context.writeAndFlush(wrapOutboundOut(ByteBuffer(bytes: [0x05, 0x00])), promise: nil)
            }
            guard buffer.readableBytes >= 5,
                  let length = buffer.getInteger(at: buffer.readerIndex + 4, as: UInt8.self),
                  buffer.readableBytes >= 5 + Int(length) + 2 else { return }
            buffer.moveReaderIndex(forwardBy: 5 + Int(length) + 2)
            context.writeAndFlush(wrapOutboundOut(ByteBuffer(bytes: [0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 22])),
                                  promise: nil)
            // SSH from here: the server speaks first, as sshd does.
            let ssh = then()
            let pipeline = context.pipeline
            pipeline.syncOperations.removeHandler(context: context).whenSuccess {
                try? pipeline.syncOperations.addHandler(ssh)
            }
        }
    }
}
