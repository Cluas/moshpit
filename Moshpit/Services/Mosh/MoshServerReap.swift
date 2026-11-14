import Foundation

/// When the app stops a mosh-server it started: crates/session's rules (the
/// mosh leg in `session.rs`). Pure, so the live check
/// (scripts/verify/mosh-live) runs these very rules against a real
/// mosh-server, not a copy of them.
///
/// The app never comes back to a mosh-server: it keeps no session key, and
/// every connect starts a server of its own. A server is stopped when it is
/// waiting for a client that will never come — never while it holds a shell
/// the user may still be using.
enum MoshServerReap {
    /// The transport says nothing comes back from the server
    /// (`MoshTransport.onReturnPathDead`). Only a server that never heard
    /// from us is stopped: it waits forever for a client that isn't coming.
    /// One we have talked to holds the user's shell, and the path that went
    /// quiet may come back: the session stays up and offers SSH. Both used to
    /// be stopped — about 15 s of UDP silence mid-session killed the user's
    /// shell (B165).
    static func onReturnPathDead(heardFromServer: Bool) -> Bool {
        !heardFromServer
    }

    /// Why the session's mosh transport is going away (`stop()`).
    enum Teardown: Equatable {
        /// A reconnect of the same session replaces it.
        case reconnect
        /// The user closed the session: disconnected it, switched it to SSH,
        /// or reconnected it afresh from the terminal (a protocol switch,
        /// Install Assist, the Session-ended card).
        case userClosed
        /// mosh-server ended the session itself (its shell exited, or it was
        /// stopped): there is nothing left to stop.
        case ended
    }

    /// Whether to stop the server as the transport goes away. One the user
    /// closed is stopped whether or not it was ever reached: nothing can
    /// come back to it, and the app used to leave every one it had talked to
    /// running, with its shell, after every disconnect and protocol switch
    /// (B164). A reconnect stops only a server that was never reached.
    static func onTeardown(_ why: Teardown, contacted: Bool) -> Bool {
        switch why {
        case .userClosed: return true
        case .reconnect: return !contacted
        case .ended: return false
        }
    }
}
