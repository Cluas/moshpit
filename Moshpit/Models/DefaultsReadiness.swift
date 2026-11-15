import Foundation
import UIKit
import MoshpitKit

/// Whether a `UserDefaults` read can be believed right now.
///
/// iOS launches this app with nobody watching: prewarming, a push, a Live
/// Activity intent. When that happens before the first unlock after a reboot,
/// the app's preferences file is still sealed (Library/Preferences is protected
/// until first user authentication), and every store that loads in `init()`
/// reads NOTHING and believes it. The Home screen then shows the first-run
/// "add a connection" card over a phone that has a dozen hosts, and the
/// shortcut store — which seeds and PERSISTS its built-ins when it finds none —
/// writes the user's custom chips away for good. The report that found this:
/// "opening the app, sometimes every connection is gone". Sometimes, because
/// it takes a reboot and an unattended launch before the unlock.
///
/// Two signals answer the question:
///  * the system's — `isProtectedDataAvailable`: the device is unlocked, so
///    every protection class is readable;
///  * ours — a sentinel written into the same defaults on the first trusted
///    load. A later read that finds it found the FILE, unlocked or not (the
///    preferences stay readable while locked once the device has been unlocked
///    since boot; the sentinel is how a store tells that from a sealed file).
/// Absent both, a store must treat what it read as unknown: show it, never
/// write over it, and read again when the app comes forward
/// (`reloadIfNeeded()` on every store; `MoshpitApp` calls them).
struct DefaultsReadiness {
    static let sentinelKey = "moshpit.defaults.ready"

    /// The system's word on protected data, asked at read time.
    var protectedDataAvailable: () -> Bool

    /// Always readable — tests, and any store handed a scratch suite.
    static let always = DefaultsReadiness { true }

    /// The device's actual state. `UIApplication` answers on the main thread;
    /// stores are built and mutated there (App init, views), and the one
    /// singleton that might be touched elsewhere first hops over.
    static let system = DefaultsReadiness {
        if Thread.isMainThread {
            return MainActor.assumeIsolated { UIApplication.shared.isProtectedDataAvailable }
        }
        return DispatchQueue.main.sync {
            MainActor.assumeIsolated { UIApplication.shared.isProtectedDataAvailable }
        }
    }

    /// The sentinel, written by the stores (crates/store) after their first
    /// trusted read, or the system's word.
    func isReadable(_ defaults: UserDefaults) -> Bool {
        defaults.bool(forKey: Self.sentinelKey) || protectedDataAvailable()
    }
}

extension AppLanguage {
    /// Record the language Moshpit shows (``AppLanguage/record(_:suiteName:)``)
    /// — only once the defaults can be trusted. Returns whether it did.
    ///
    /// The record is a write into the App Group's preferences, which are
    /// sealed until the first unlock just as the app's are. A write into a
    /// domain that reads as empty can replace the file with only that key:
    /// the push switches the notification service reads, the widget's
    /// snapshot, gone with it. So it waits for the readiness the stores wait
    /// for, and the app asks again whenever it comes forward.
    @MainActor @discardableResult
    static func recordIfTrusted(readiness: DefaultsReadiness = .system,
                                defaults: UserDefaults = .standard,
                                language: String = AppLanguage.own,
                                suiteName: String = PushPrefs.suite) -> Bool {
        guard readiness.isReadable(defaults) else { return false }
        record(language, suiteName: suiteName)
        return true
    }
}
