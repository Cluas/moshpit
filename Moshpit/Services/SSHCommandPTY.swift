import Citadel
import Foundation
import NIOCore
import NIOSSH

/// A terminal channel whose program is a command: a PTY request, then an exec
/// request — what `ssh -t host <command>` asks for — instead of a shell
/// request and a line typed into the shell (B267).
///
/// Every program the app starts on a terminal — tmux's control client, the
/// `-CC` sidecar's attach chain, herdr's frame loop, the push pump — was a
/// line typed into the interactive login shell the channel opened. A shell
/// still starting swallowed it: a startup that flushes pending input
/// (Powerlevel10k's instant prompt; a private sshd with a 3 s zsh rc that
/// flushes: tmux never attached, push never came up), or a line editor whose
/// plugins chewed on it (oh-my-zsh with autosuggestions and syntax
/// highlighting, the B266 report). As the channel's command it runs at once,
/// under no interactive shell, and nothing reads it as typing.
///
/// Citadel opens a PTY only together with a shell request, and keeps its
/// stdin writer's initializer to itself, so the channel is opened here on
/// the connection's NIOSSHHandler, as ``SSHExec`` opens its own.
enum SSHCommandPTY {
    /// Opens the channel and returns its writer once the server has granted
    /// the PTY and started `command` (as `sh -c`, ``Shell/portable(_:)``).
    /// `onOutput` then gets the channel's bytes in order, on the connection's
    /// event loop; `onEnd` fires once when the channel is gone.
    static func open(_ command: String, rows: Int, cols: Int, on connection: Channel,
                     environment: [(String, String)],
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        let loop = connection.eventLoop
        let granted = loop.makePromise(of: Void.self)
        let reader = Reader(granted: granted, onOutput: onOutput, onEnd: onEnd)
        let child: Channel
        do {
            child = try await loop.flatSubmit { () -> EventLoopFuture<Channel> in
                let created = loop.makePromise(of: Channel.self)
                do {
                    try connection.pipeline.syncOperations.handler(type: NIOSSHHandler.self)
                        .createChannel(created, channelType: .session) { child, _ in
                            child.eventLoop.makeCompletedFuture {
                                try child.pipeline.syncOperations.addHandler(reader)
                            }
                        }
                } catch {
                    created.fail(error)
                }
                // As Citadel's: a channel the server never opens fails.
                loop.scheduleTask(in: .seconds(15)) {
                    created.fail(CitadelError.channelCreationFailed)
                }
                return created.futureResult
            }.get()
        } catch {
            granted.fail(error)
            throw error
        }
        do {
            for (name, value) in environment {
                // Without a reply: a server that refuses it (no `AcceptEnv`)
                // starts the program all the same.
                try await child.triggerUserOutboundEvent(
                    SSHChannelRequestEvent.EnvironmentRequest(wantReply: false, name: name, value: value))
            }
            try await child.triggerUserOutboundEvent(
                SSHChannelRequestEvent.PseudoTerminalRequest(
                    wantReply: true,
                    term: "xterm-256color",
                    terminalCharacterWidth: cols,
                    terminalRowHeight: rows,
                    terminalPixelWidth: 0,
                    terminalPixelHeight: 0,
                    terminalModes: SSHTerminalModes([:])))
            try await child.triggerUserOutboundEvent(
                SSHChannelRequestEvent.ExecRequest(command: Shell.portable(command), wantReply: true))
            try await granted.futureResult.get()
        } catch {
            child.close(promise: nil)
            throw error
        }
        return Writer(channel: child)
    }

    /// The channel's reader: the two requests' answers, its bytes in order,
    /// and how it ended. Every call is on the channel's event loop.
    private final class Reader: ChannelInboundHandler, @unchecked Sendable {
        typealias InboundIn = SSHChannelData

        private let granted: EventLoopPromise<Void>
        private let onOutput: @Sendable (Data) -> Void
        private let onEnd: @Sendable (SSHChannelEnd) -> Void
        /// Answers still owed: the PTY request's, then the exec's.
        private var owed = 2
        private var opened = false
        private var status: Int?
        private var error: Error?

        init(granted: EventLoopPromise<Void>,
             onOutput: @escaping @Sendable (Data) -> Void,
             onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) {
            self.granted = granted
            self.onOutput = onOutput
            self.onEnd = onEnd
        }

        func handlerAdded(context: ChannelHandlerContext) {
            // The server's EOF comes before its exit status; the channel has
            // to stay open for it.
            context.channel.setOption(ChannelOptions.allowRemoteHalfClosure, value: true).whenFailure { _ in }
        }

        func channelRead(context: ChannelHandlerContext, data: NIOAny) {
            let data = unwrapInboundIn(data)
            guard case .byteBuffer(let bytes) = data.data, bytes.readableBytes > 0 else { return }
            switch data.type {
            case .channel, .stdErr:
                // Synchronously, in order: the event loop reads one message
                // at a time (the byte path takes no unordered hop).
                onOutput(Data(bytes.readableBytesView))
            default:
                break
            }
        }

        func userInboundEventTriggered(context: ChannelHandlerContext, event: Any) {
            switch event {
            case is ChannelSuccessEvent:
                guard owed > 0 else { return }
                owed -= 1
                if owed == 0 {
                    opened = true
                    granted.succeed(())
                }
            case is ChannelFailureEvent:
                guard owed > 0 else { return }
                owed = 0
                granted.fail(SSHExec.Failure.refused)
                context.close(promise: nil)
            case let exit as SSHChannelRequestEvent.ExitStatus:
                status = exit.exitStatus
            default:
                context.fireUserInboundEventTriggered(event)
            }
        }

        func errorCaught(context: ChannelHandlerContext, error: Error) {
            if self.error == nil { self.error = error }
        }

        func handlerRemoved(context: ChannelHandlerContext) {
            guard opened else {
                granted.fail(error ?? SSHExec.Failure.endedWithoutStatus)
                return
            }
            onEnd(Self.end(status: status, error: error))
        }

        /// As ``CitadelClientTransport/channelEnd(after:)``: closed in order
        /// is the program's end (its status when not 0); anything that took
        /// the channel down with it — the connection gone (NIOSSH reports
        /// `tcpShutdown` on each of its channels), an I/O error — a drop.
        static func end(status: Int?, error: Error?) -> SSHChannelEnd {
            if error != nil { return .dropped }
            return .exited(status: status == 0 ? nil : status)
        }
    }

    private struct Writer: SSHPTYWriter {
        let channel: Channel

        func write(_ data: Data) async throws {
            try await channel.writeAndFlush(SSHChannelData(type: .channel, data: .byteBuffer(ByteBuffer(bytes: data))))
        }

        func resize(cols: Int, rows: Int) async throws {
            try await channel.triggerUserOutboundEvent(
                SSHChannelRequestEvent.WindowChangeRequest(
                    terminalCharacterWidth: cols,
                    terminalRowHeight: rows,
                    terminalPixelWidth: 0,
                    terminalPixelHeight: 0))
        }
    }
}
