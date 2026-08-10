import Foundation
import os

/// Central logging facade over `os.Logger`.
///
/// One namespace, a fixed set of categories, and a single subsystem so that
/// Console.app / `log stream` can filter Moshpit's output cleanly:
///
/// ```
/// log stream --predicate 'subsystem == "com.cluas.moshpit"'
/// log stream --predicate 'subsystem == "com.cluas.moshpit" AND category == "ssh"'
/// ```
///
/// Each category is a lazily-created, shared `Logger`. Use them the same way
/// you would use any `os.Logger` — interpolation is redacted by default, mark
/// values you want to keep in the clear with `privacy: .public`:
///
/// ```swift
/// Log.ssh.info("connecting to \(host, privacy: .public):\(port, privacy: .public)")
/// Log.ssh.error("handshake failed: \(error.localizedDescription, privacy: .public)")
/// Log.mosh.debug("udp datagram \(bytes.count) bytes")
/// ```
///
/// This is just the utility — wiring it into existing call sites is out of
/// scope here.
enum Log {

    /// The subsystem all Moshpit loggers share. Matches the app's bundle id so
    /// system log tools group Moshpit's output under one heading.
    private static let subsystem = "com.cluas.moshpit"

    /// SSH transport: connection lifecycle, auth, channel and host-key events.
    static let ssh = Logger(subsystem: subsystem, category: "ssh")

    /// Mosh transport: UDP session bring-up, roaming, and predictive echo.
    static let mosh = Logger(subsystem: subsystem, category: "mosh")
}
