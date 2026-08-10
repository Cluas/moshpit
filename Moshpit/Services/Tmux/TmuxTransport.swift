import Foundation

/// Narrow transport surface ``TmuxSessionController`` needs from whatever
/// connection layer is feeding it — only "give me an async byte stream" and
/// "send these bytes back". `SSHSession` conforms naturally; tests can plug
/// in an in-memory `MockTmuxTransport` instead of standing up a real SSH
/// channel.
///
/// Keeping the protocol this narrow means the controller can't accidentally
/// reach for SSH-specific behaviour (PTY size requests, host keys, etc.) and
/// stays testable through the entire controller lifecycle.
protocol TmuxTransport: Sendable {
    /// Stream of bytes coming out of the remote shell. The controller iterates
    /// this exactly once per attach; the protocol does not guarantee a stream
    /// can be re-subscribed after it finishes.
    nonisolated var dataStream: AsyncStream<Data> { get }

    /// Send bytes toward the remote shell. May throw if the transport has
    /// been torn down — the controller swallows write errors so a closed
    /// channel doesn't crash the session.
    func write(_ data: Data) async throws
}

extension SSHSession: TmuxTransport {}

// NOTE: MoshTransport intentionally does NOT conform. tmux -CC control mode
// needs a raw, line-framed byte pipe; mosh transmits rendered screen diffs,
// which destroy the framing (verified: `tmux -CC` runs server-side but no
// %begin/%output ever returns). Native tmux sheets require SSH; mosh+tmux
// instead runs the plain full-screen tmux TUI.
