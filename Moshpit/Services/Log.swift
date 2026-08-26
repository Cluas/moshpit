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

    /// Voice input: engine selection, model downloads, capture failures.
    static let voice = Logger(subsystem: subsystem, category: "voice")

    /// Keyboard input reaching the terminal. Added for one question that could
    /// not be answered by reading code: when hold-to-repeat backspace "stops
    /// working", did UIKit stop delivering the repeats, or did we send them and
    /// the screen not follow? Those have opposite fixes. The answer was the
    /// former (see SwiftTerm fork patch 12 in `docs/PATCHES.md`), and the
    /// counter stays because it is how that answer is re-checked: a hold should
    /// tick steadily for as long as the key is down, whatever put the text on
    /// the line.
    ///
    /// Deliberately `info`, not `debug`: it has to survive in a Release
    /// TestFlight build, which is where these get reproduced.
    static let input = Logger(subsystem: subsystem, category: "input")

    /// Remote push: device-token registration, relay sync, pairing decode.
    ///
    /// This whole path fails SILENTLY by nature — an unregistered device is not
    /// an error anywhere, it is simply a phone that never buzzes — and every
    /// step of it happens with no UI attached: at launch, in the background, or
    /// inside an extension. The first attempt to bring it up on a simulator
    /// produced exactly that: no prompt, no token, no registration, and nothing
    /// anywhere saying which of the three guards had returned early. Hence
    /// `info`, not `debug`, and hence logging the decisions rather than only the
    /// failures.
    ///
    /// Never log a pairing secret or a whole send token. Device tokens are
    /// logged as a short prefix — enough to correlate with the relay's own
    /// fingerprints, not enough to be a credential.
    static let push = Logger(subsystem: subsystem, category: "push")

    /// Vibe Island: which agent states were seen, and what was announced.
    ///
    /// Added because a reconnect re-notifying was reported from a phone and
    /// could not be observed anywhere — the decision left no trace, so the only
    /// evidence was a person counting buzzes. `postAttention` now says who it is
    /// announcing and for which episode, which is what makes the difference
    /// between "it announced again" and "iOS re-delivered" visible at all.
    static let island = Logger(subsystem: subsystem, category: "island")
}
