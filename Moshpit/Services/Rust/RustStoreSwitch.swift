import Foundation
import MoshpitFFI
import MoshpitKit

/// Whether the app's stores read and write through the Rust core's store
/// (crates/store through crates/ffi, docs/nsh/DESIGN.md §3.5–3.6) instead of
/// their own Swift code: the connections, the SSH key records, the shortcut
/// bar, the settings, the terminal themes and accents, the trusted host keys
/// and the push pairings. Either way the same keys, formats and items: iOS
/// migrates nothing, and the other side reads everything the one wrote.
///
/// The switch is a Bool in `UserDefaults.standard`, set on the hidden Core
/// Switches page, or for one launch with the launch argument
/// `-MOSHPIT_RUST_STORE 1` (the argument domain beats the stored value). Off
/// by default: off is exactly the app without it. It is read ONCE, the
/// first time anything asks — at launch, before the first store is made —
/// and holds for the life of the process: storage never changes under a
/// running app. (A launch before the first unlock reads the preferences as
/// empty, the switch among them, so it takes the registered default: the
/// Swift stores, or the Rust ones in a Canary build, which registers the
/// switch on (``CanaryDefaults``). Both sides keep the sealed-launch rule,
/// so nothing is lost either way.)
enum RustStoreSwitch {
    static let switchKey = "MOSHPIT_RUST_STORE"

    /// Whether this build can run its stores on the Rust core: the library
    /// is linked and is the one the app was compiled against.
    static let available = mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION)

    /// This launch's answer, read once.
    static let isOn: Bool = {
        CanaryDefaults.register()
        let on = available && UserDefaults.standard.bool(forKey: switchKey)
        Log.store.notice("stores: \(on ? "Rust" : "Swift", privacy: .public) this launch")
        return on
    }()

    /// What the stored switch says now: the next launch's answer.
    static var isOnNextLaunch: Bool {
        available && UserDefaults.standard.bool(forKey: switchKey)
    }

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
    /// Rust store when the switch is on (``PushPairingStore/backing``).
    /// Without an App Group container (an unsigned build) nothing can be
    /// paired, and nothing changes.
    static func installPushPairings() {
        guard isOn, PushPairingStore.backing == nil, let url = PushPairingStore.fileURL else { return }
        PushPairingStore.backing = RustPushPairings(url: url)
    }

    /// What the Core Switches page says the switch moves.
    static let note = """
        On: connections, SSH key records (the keys stay in the keychain), the shortcut bar, \
        the settings, terminal themes, accents, trusted host keys and push pairings are read \
        and written by the Rust core's store — the same keys, formats and items, so switching \
        back loses nothing. From the next launch on: storage never changes under a running app.
        """
}

/// The app's stores, made on the side ``RustStoreSwitch`` picked for this
/// launch. Each is the store class the views and services know, its reads
/// and writes its own Swift code's or the Rust store's.
enum AppStores {
    static func connections() -> ConnectionStore {
        guard RustStoreSwitch.isOn else { return ConnectionStore(readiness: .system) }
        return ConnectionStore(backing: RustConnectionStore(preferences: RustStoreSwitch.preferences, mode: .system))
    }

    static func sshKeys() -> SSHKeyStore {
        guard RustStoreSwitch.isOn else { return SSHKeyStore(readiness: .system) }
        return SSHKeyStore(backing: RustSSHKeyStore(preferences: RustStoreSwitch.preferences, mode: .system))
    }

    /// Always the Rust store (W15d): the shortcut bar's Swift copy is gone,
    /// whatever the switch says. The same key and stored form, so nothing
    /// migrates.
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
