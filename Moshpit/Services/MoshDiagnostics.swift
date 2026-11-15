import Foundation

/// Running protocol counters surfaced for on-device diagnosis of a black
/// screen (cursor + input work, but no content ever renders) — a failure
/// mode this app's tests can't reproduce, since it needs real packet
/// loss/reordering or a specific remote shell config that loopback testing
/// never exercises. Screenshotting this (see the terminal pill's
/// long-press) is the only practical way to see which branch a live,
/// unreproducible session actually hit. Filled from the core's counters
/// (crates/mosh `Diagnostics`, the session's `moshDiagnostics` event).
struct MoshDiagnostics: Hashable {
    /// Total UDP datagrams received (before decrypt/parse — a low count
    /// here despite a "connected" pill points at network/NAT, not us).
    var datagramsReceived: UInt64 = 0
    /// Host message parse failures on an otherwise-applicable diff.
    /// Before e61a9b3 these were silently acked (permanent content loss);
    /// now they fall through to re-diff instead — but a high count still
    /// means something about this remote's output isn't decoding.
    var parseFailures: UInt64 = 0
    /// Diffs received with a gap (newNum > appliedHostNum but oldNum
    /// doesn't match) — expected occasionally on lossy links; a count that
    /// keeps climbing without `appliedHostNum` ever advancing means we're
    /// stuck re-diffing and never landing a diff we can apply.
    var gapEvents: UInt64 = 0
    /// Highest display state actually applied. Stuck at 0 with datagrams
    /// arriving means every diff so far has hit the gap or parse-failure
    /// branch — the server has content queued that we've never accepted.
    var appliedHostNum: UInt64 = 0

    // Send side — the input path's vitals, added for the "typing stopped
    // working over mosh" report (2026-08-17) that no loopback rig
    // reproduces. Screenshot the popover while it's happening:
    /// Encrypted datagrams sent. Frozen while typing = flush isn't reaching
    /// the socket (channel wedged / send loop broken).
    var packetsSent: UInt64 = 0
    /// Newest queued user state vs the newest the server acked. `lastUserNum`
    /// climbing with keystrokes while `serverAckedUserNum` stands still =
    /// input is leaving the app and dying on the wire (or the server's acks
    /// are dying on the way back). Idle, both climb by one every 3 s
    /// (the core's idle state).
    var lastUserNum: UInt64 = 0
    var serverAckedUserNum: UInt64 = 0
    /// The UDP flow's state, verbatim. Anything but "ready" while the pill
    /// says live is the story.
    var channelState: String = "?"
    /// Expired keystrokes the server may already hold are unacked, so new
    /// input waits for the server's answer (the core's held input, divergences.json `ttl-rewrite`).
    var inputHeld = false
}
