import Foundation

/// ``HostChannel`` over a real SSH connection.
///
/// Thin on purpose: one `executeCommand` per call, each on its own exec channel,
/// which is what makes the installer's failures legible — a command either
/// returns its output or throws, with no pane and no shell history involved.
struct SSHHostChannel: HostChannel {
    let session: SSHSession

    func run(_ command: String) async throws -> Data {
        try await session.executeCommand(command)
    }
}

extension SessionHub.ActiveSession {
    /// An installer bound to this session's best available SSH channel.
    ///
    /// Rides `acquireFileTransferSSH()` — the same channel image attachment uses,
    /// and for the same reasons its comments give: the in-band SSH session when
    /// there is one, the mosh sidecar otherwise, an on-demand dial for pure mosh
    /// (from the in-memory secret cache, so it never re-prompts Face ID), and a
    /// liveness probe rather than trusting `isConnected`, which lies after a
    /// suspension.
    ///
    /// The practical consequence is that installing works the same on SSH, on
    /// mosh, and on a herdr connection — it does not need a tmux control channel
    /// and never touches a pane. The flow this replaces had a "connect to a host
    /// with tmux to run and verify the hooks" dead end; there is no such state
    /// here.
    func hostInstaller() async throws -> HostInstaller {
        HostInstaller(channel: SSHHostChannel(session: try await acquireFileTransferSSH()))
    }
}
