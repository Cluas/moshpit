import Foundation
import MoshpitFFI
import MoshpitKit

/// The app's stores read and write through the Rust core's store
/// (crates/store through crates/ffi, docs/nsh/DESIGN.md §3.5–3.6): the
/// connections, the SSH key records, the shortcut bar, the settings, the
/// terminal themes and accents, the trusted host keys and the push
/// pairings. The same keys, formats and items the Swift stores used, so
/// iOS migrates nothing from a build that ran them (the store harness's
/// 1.0.8 phases hold it).
///
/// No switch any more (F0): every build is on the Rust store whenever its
/// library is the one the app was compiled against, as Canary builds were.
/// The Swift stores' own code goes with batch C.
enum RustStoreSwitch {
    /// Whether this build can run its stores on the Rust core: the library
    /// is linked and is the one the app was compiled against.
    static let available = mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION)

    /// This launch's answer, read once.
    static let isOn: Bool = {
        Log.store.notice("stores: \(available ? "Rust" : "Swift (no Rust core in this build)", privacy: .public) this launch")
        return available
    }()

    /// The preferences the Rust stores read and write: the app's own,
    /// `UserDefaults.standard`, with the device's protected-data state for
    /// the sealed-launch rule.
    static let preferences = RustPreferencesHost(defaults: .standard, protectedData: .app)

    /// The keychain the Rust known hosts read and write: the backend the
    /// Swift known hosts use, the same item — read as stored, as the Swift
    /// store reads it: ``HostKeyValidator`` decides a host trusted for this
    /// launch (a DEBUG seed's) itself. Laid over the item, those hosts made
    /// a read before the first unlock a map of them alone instead of
    /// "locked", which the validator would have kept as its table.
    static let keychain = RustKeychainHost(launchTrust: nil)

    /// Before anything reads a push pairing: the pairings go through the
    /// Rust store (``PushPairingStore/backing``).
    /// Without an App Group container (an unsigned build) nothing can be
    /// paired, and nothing changes.
    static func installPushPairings() {
        guard isOn, PushPairingStore.backing == nil, let url = PushPairingStore.fileURL else { return }
        PushPairingStore.backing = RustPushPairings(url: url)
    }
}

/// The app's stores, made on the Rust store (``RustStoreSwitch``). Each is
/// the store class the views and services know.
enum AppStores {
    static func connections() -> ConnectionStore {
        guard RustStoreSwitch.isOn else { return ConnectionStore(readiness: .system) }
        return ConnectionStore(backing: RustConnectionStore(preferences: RustStoreSwitch.preferences, mode: .system))
    }

    static func sshKeys() -> SSHKeyStore {
        guard RustStoreSwitch.isOn else { return SSHKeyStore(readiness: .system) }
        return SSHKeyStore(backing: RustSSHKeyStore(preferences: RustStoreSwitch.preferences, mode: .system))
    }

    /// Always the Rust store (W15d): the shortcut bar's Swift copy is gone.
    /// The same key and stored form, so nothing migrates.
    static func shortcuts() -> ShortcutStore {
        ShortcutStore(backing: RustShortcutStore(preferences: RustStoreSwitch.preferences, mode: .system))
    }

    static func themes() -> ThemeStore {
        guard RustStoreSwitch.isOn else { return ThemeStore(readiness: .system) }
        return ThemeStore(backing: RustThemeStore(preferences: RustStoreSwitch.preferences, mode: .system,
                                                  untitled: String(localized: "Untitled")))
    }
}

extension AppSettings {
    /// The app-wide settings, on the side ``RustStoreSwitch`` picked.
    static let shared: AppSettings = {
        guard RustStoreSwitch.isOn else { return AppSettings(readiness: .system) }
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
        return AppSettings(backing: RustSettings(preferences: RustStoreSwitch.preferences, mirror: mirror,
                                                 mode: .system, debugBuild: debugBuild))
    }()
}

extension AppThemeStore {
    /// The app-wide accent themes, on the side ``RustStoreSwitch`` picked.
    static let shared: AppThemeStore = {
        guard RustStoreSwitch.isOn else { return AppThemeStore(readiness: .system) }
        return AppThemeStore(backing: RustAppThemeStore(preferences: RustStoreSwitch.preferences, mode: .system,
                                                        untitled: String(localized: "Untitled")))
    }()
}

extension AppStores {
    /// The trusted host keys ``HostKeyValidator`` decides from, on the side
    /// ``RustStoreSwitch`` picked: the same keychain item either way, read
    /// by the sealed-launch rule (B226, B251).
    static func knownHosts() -> KnownHostsStore {
        guard RustStoreSwitch.isOn else { return KeychainKnownHostsStore.app() }
        return RustKnownHostsStore(keychain: RustStoreSwitch.keychain, preferences: RustStoreSwitch.preferences)
    }
}
