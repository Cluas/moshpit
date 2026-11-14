// A SOCKS5 proxy on 127.0.0.1 that answers by a script, byte for byte from
// RFC 1928 — the same fake Tests/MoshpitTests/Services/SOCKSProxyDialerTests
// runs on the device.

import Foundation
import NIOCore
import NIOPosix

enum FakeSOCKSScript: String, CaseIterable {
    /// Takes no-auth, then opens the connection (reply 0).
    case success
    /// Takes no-auth, then refuses the CONNECT (reply 5).
    case refused
    /// Selects username/password, which the client didn't offer.
    case wantsPassword = "wants-password"
    /// "No acceptable methods" (0xFF).
    case noAcceptableMethods = "no-acceptable-methods"
    /// Answers like a web server.
    case notSOCKS = "not-socks"
    /// Closes the connection after the greeting.
    case hangUp = "hang-up"
    /// Reads, and never answers.
    case silent
}

final class FakeSOCKSProxy: @unchecked Sendable {
    let port: Int
    private let channel: Channel

    private init(port: Int, channel: Channel) {
        self.port = port
        self.channel = channel
    }

    static func start(_ script: FakeSOCKSScript) async throws -> FakeSOCKSProxy {
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

        private let script: FakeSOCKSScript
        private var buffer = ByteBuffer()
        private var greeted = false

        init(_ script: FakeSOCKSScript) { self.script = script }

        func channelRead(context: ChannelHandlerContext, data: NIOAny) {
            var incoming = unwrapInboundIn(data)
            buffer.writeBuffer(&incoming)
            if !greeted {
                guard buffer.readableBytes >= 2,
                      let count = buffer.getInteger(at: buffer.readerIndex + 1, as: UInt8.self),
                      buffer.readableBytes >= 2 + Int(count) else { return }
                buffer.moveReaderIndex(forwardBy: 2 + Int(count))
                greeted = true
                switch script {
                case .success, .refused: send(context, [0x05, 0x00])
                case .wantsPassword: send(context, [0x05, 0x02])
                case .noAcceptableMethods: send(context, [0x05, 0xFF])
                case .notSOCKS: send(context, Array("HTTP/1.1 400 Bad Request\r\n\r\n".utf8))
                case .hangUp: context.close(promise: nil)
                case .silent: break
                }
                return
            }
            guard script == .success || script == .refused, buffer.readableBytes >= 5,
                  let length = buffer.getInteger(at: buffer.readerIndex + 4, as: UInt8.self),
                  buffer.readableBytes >= 5 + Int(length) + 2 else { return }
            buffer.moveReaderIndex(forwardBy: 5 + Int(length) + 2)
            let code: UInt8 = script == .success ? 0 : 5
            send(context, [0x05, code, 0x00, 0x01, 127, 0, 0, 1, 0, 22])
            if code != 0 { context.close(promise: nil) }
        }

        private func send(_ context: ChannelHandlerContext, _ bytes: [UInt8]) {
            context.writeAndFlush(wrapOutboundOut(ByteBuffer(bytes: bytes)), promise: nil)
        }
    }
}
