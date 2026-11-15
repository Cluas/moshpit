import Foundation

/// One applied mosh server diff, as the terminal layer consumes it: the
/// core's echo acknowledgement mark (`RustSessionDriver.shellMark`) arrives
/// as one, after the bytes of its frame, for predictive echo to judge its
/// guesses by.
struct MoshHostFrame: Equatable, Sendable {
    /// Terminal output to feed (may be empty when only the echo ack moved).
    var bytes: Data
    /// How far into the typed bytes the server had received at least 50 ms
    /// before this frame: their echo, if the remote echoes at all, is on
    /// screen by now. Monotonic.
    var echoAck: UInt64
}
