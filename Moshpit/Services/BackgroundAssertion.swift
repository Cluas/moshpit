import Foundation
import UIKit

/// A scoped `beginBackgroundTask` assertion: tens of seconds of guaranteed
/// runtime after the app leaves the foreground.
///
/// This is emphatically **not** a keepalive. iOS offers no sanctioned way to
/// hold a socket open in the background — the modes that keep an app running
/// (`audio`, `voip`, `location`) all require actually doing that thing, and
/// faking one is App Review guideline 2.5.4. What an assertion buys is the
/// chance to *finish something*, which is a different and achievable goal.
///
/// Moshpit needs exactly that. Backgrounding queues server-side teardown —
/// handing tmux window pins back so a desktop attaching later isn't stranded at
/// the phone grid, taking our clients out of `window-size latest` — and every
/// one of those is a control-mode command with a round trip. Without an
/// assertion the app can be suspended between queueing them and their bytes
/// reaching the socket, and then, as `TmuxSessionController.releaseWindowPins`
/// puts it, "once iOS suspends/kills us we never get another chance to run
/// code". The teardown was already written as if it completed synchronously;
/// this is what makes that true.
///
/// The assertion MUST be ended. One the app never ends is not a longer grace
/// period — it is a termination with `0xdead10cc`, so `end()` is idempotent and
/// the expiration handler calls it too.
@MainActor
final class BackgroundAssertion {
    private var id: UIBackgroundTaskIdentifier = .invalid

    init(name: String) {
        id = UIApplication.shared.beginBackgroundTask(withName: name) { [weak self] in
            // Documented to run on the main thread, synchronously, while the
            // app's suspension waits — so this is the one place where ending
            // *now* matters more than hopping actors, and where hopping could
            // lose the race we are trying to win.
            MainActor.assumeIsolated { self?.end() }
        }
    }

    /// Idempotent — safe to call from both the completion path and the
    /// expiration handler without tracking which got there first.
    func end() {
        guard id != .invalid else { return }
        UIApplication.shared.endBackgroundTask(id)
        id = .invalid
    }

    deinit {
        // A dropped assertion is a termination waiting to happen, so this is a
        // backstop rather than the normal path: callers end it explicitly when
        // the work they took it for finishes.
        if id != .invalid {
            let leaked = id
            Task { @MainActor in UIApplication.shared.endBackgroundTask(leaked) }
        }
    }
}
