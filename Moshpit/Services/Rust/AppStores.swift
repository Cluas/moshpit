import Foundation
import MoshpitFFI
import MoshpitKit

/// The app's stores, on the Rust core's store (crates/store through
/// crates/ffi, docs/nsh/DESIGN.md §3.5–3.6): the connections, the SSH key
/// records, the shortcut bar, the settings, the terminal themes and
/// accents, the trusted host keys and the push pairings. The same keys,
/// formats and items the Swift stores used, so nothing migrates from a
/// build that ran them (the store harness's 1.0.8 phases hold it). Each is
/// the store class the views and services know.
///
/// The Swift stores' own code is gone (batch C): there is no other side to
/// switch to.
enum AppStores {
    /// The preferences the stores read and write: the app's own,
    /// `UserDefaults.standard`, with the device's protected-data state for
    /// the sealed-launch rule.
    static let preferences = RustPreferencesHost(defaults: .standard, protectedData: .app)

    /// Before anything reads a push pairing: the pairings go through the
    /// Rust store (``PushPairingStore/backing``). Without an App Group
    /// container (an unsigned build) nothing can be paired, and nothing
    /// changes.
    static func installPushPairings() {
        guard PushPairingStore.backing == nil, let url = PushPairingStore.fileURL else { return }
        PushPairingStore.backing = RustPushPairings(url: url)
    }

    static func connections() -> ConnectionStore {
        ConnectionStore(backing: RustConnectionStore(preferences: preferences, mode: .system))
    }

    static func sshKeys() -> SSHKeyStore {
        SSHKeyStore(backing: RustSSHKeyStore(preferences: preferences, mode: .system))
    }

    static func shortcuts() -> ShortcutStore {
        ShortcutStore(backing: RustShortcutStore(preferences: preferences, mode: .system))
    }

    static func themes() -> ThemeStore {
        ThemeStore(backing: RustThemeStore(preferences: preferences, mode: .system,
                                           untitled: String(localized: "Untitled")))
    }
}

extension AppSettings {
    /// The app-wide settings.
    static let shared: AppSettings = {
        // The two switches the notification service reads, mirrored into
        // the App Group's preferences (PushPrefs).
        let mirror = UserDefaults(suiteName: PushPrefs.suite).map {
            RustPreferencesHost(defaults: $0, protectedData: .app)
        }
        #if DEBUG
        let debugBuild = true
        #else
        let debugBuild = false
        #endif
        return AppSettings(backing: RustSettings(preferences: AppStores.preferences, mirror: mirror,
                                                 mode: .system, debugBuild: debugBuild))
    }()
}

extension AppThemeStore {
    /// The app-wide accent themes.
    static let shared = AppThemeStore(backing: RustAppThemeStore(preferences: AppStores.preferences,
                                                                  mode: .system,
                                                                  untitled: String(localized: "Untitled")))
}
