import Citadel
import Foundation
import NIOCore
import NIOSSH

/// One command over an exec channel of its own, its output (stdout and
/// stderr as they came) handed back only when the command is known to have
/// finished: an exit status of 0.
///
/// Citadel's `executeCommand` can't tell an exit status of 0 from none at
/// all: a channel that closed without one — the command killed by a signal
/// (sshd then sends `exit-signal`, which Citadel passes over), or the server
/// dropping the channel mid-command — returned whatever output had arrived,
/// as success (B136). Every caller took that for the whole answer: a herdr
/// command cut short read as "no JSON, exit status lost", a preview or an
/// upload's last step as done. Here the channel's ending is kept, and only a
/// 0 is success; a dropped connection still fails as it did (NIOSSH reports
/// `tcpShutdown` on every channel of a connection that went away).
enum SSHExec {
    /// How a command's channel ended, when it wasn't with an exit status of 0.
    enum Failure: Error, Equatable, CustomStringConvertible {
        /// The command exited with this (non-zero) status, having printed
        /// `output` (stdout and stderr as they came): an edit's exec form
        /// says "taken" and exits 1, and the marker is the answer (B240).
        case exited(status: Int, output: Data)
        /// A signal ended it ("KILL", "TERM", …).
        case killed(signal: String)
        /// The channel closed with no exit status at all: what came is cut
        /// short.
        case endedWithoutStatus
        /// The server refused to run it.
        case refused

        var description: String {
            switch self {
            case .exited(let status, _): return "command exited with status \(status)"
            case .killed(let signal): return "command killed by SIG\(signal)"
            case .endedWithoutStatus: return "command's channel closed without an exit status"
            case .refused: return "server refused to run the command"
            }
        }
    }

    /// Runs `command`, a POSIX sh script, on a channel of `connection` — the
    /// SSH connection's own TCP channel, whose pipeline holds the
    /// NIOSSHHandler — and returns its output once it exited with status 0.
    ///
    /// The account's shell gets it as `sh -c <command>` (``Shell/portable(_:)``,
    /// B138): a fish login shell otherwise parses the app's sh — and refuses
    /// the `$?` every herdr command ends with.
    static func run(_ command: String, on connection: Channel) async throws -> Data {
        let loop = connection.eventLoop
        let done = loop.makePromise(of: Data.self)
        let child: Channel
        do {
            child = try await loop.flatSubmit { () -> EventLoopFuture<Channel> in
                let created = loop.makePromise(of: Channel.self)
                do {
                    try connection.pipeline.syncOperations.handler(type: NIOSSHHandler.self)
                        .createChannel(created, channelType: .session) { child, _ in
                            child.eventLoop.makeCompletedFuture {
                                try child.pipeline.syncOperations.addHandler(ExecReader(done: done))
                            }
                        }
                } catch {
                    created.fail(error)
                }
                // As Citadel's exec: a channel the server never opens fails.
                loop.scheduleTask(in: .seconds(15)) {
                    created.fail(CitadelError.channelCreationFailed)
                }
                return created.futureResult
            }.get()
        } catch {
            done.fail(error)
            throw error
        }
        do {
            try await child.triggerUserOutboundEvent(
                SSHChannelRequestEvent.ExecRequest(command: Shell.portable(command), wantReply: true)).get()
        } catch {
            child.close(promise: nil)
            done.fail(error)
        }
        return try await done.futureResult.get()
    }

    /// Collects one exec channel's output and how it ended; answers when the
    /// channel is gone.
    private final class ExecReader: ChannelDuplexHandler {
        typealias InboundIn = SSHChannelData
        typealias OutboundIn = SSHChannelData
        typealias OutboundOut = SSHChannelData

        private let done: EventLoopPromise<Data>
        private var output = Data()
        private var status: Int?
        private var signal: String?
        private var refused = false
        private var error: Error?

        init(done: EventLoopPromise<Data>) {
            self.done = done
        }

        func handlerAdded(context: ChannelHandlerContext) {
            // The server's EOF comes before its exit status; the channel has
            // to stay open for it.
            context.channel.setOption(ChannelOptions.allowRemoteHalfClosure, value: true).whenFailure { _ in }
        }

        func channelRead(context: ChannelHandlerContext, data: NIOAny) {
            let data = unwrapInboundIn(data)
            guard case .byteBuffer(let bytes) = data.data else { return }
            switch data.type {
            case .channel, .stdErr:
                output.append(contentsOf: bytes.readableBytesView)
            default:
                break
            }
        }

        func userInboundEventTriggered(context: ChannelHandlerContext, event: Any) {
            switch event {
            case let exit as SSHChannelRequestEvent.ExitStatus:
                status = exit.exitStatus
            case let exit as SSHChannelRequestEvent.ExitSignal:
                signal = exit.signalName
            case is ChannelFailureEvent:
                refused = true
                context.close(promise: nil)
            case is ChannelSuccessEvent:
                break
            default:
                context.fireUserInboundEventTriggered(event)
            }
        }

        func errorCaught(context: ChannelHandlerContext, error: Error) {
            if self.error == nil { self.error = error }
        }

        func handlerRemoved(context: ChannelHandlerContext) {
            if let error {
                done.fail(error)
            } else if refused {
                done.fail(Failure.refused)
            } else if let status {
                if status == 0 {
                    done.succeed(output)
                } else {
                    done.fail(Failure.exited(status: status, output: output))
                }
            } else if let signal {
                done.fail(Failure.killed(signal: signal))
            } else {
                done.fail(Failure.endedWithoutStatus)
            }
        }
    }
}
