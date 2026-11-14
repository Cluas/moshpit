// store-swift's stores: the app's store classes on their own Swift code —
// what the app runs with the MOSHPIT_RUST_STORE switch off. main.swift makes
// every store through these; adapters/Stores.swift has the same functions
// over the Rust-store adapters (store-swift-rust).

import Foundation

func makeConnectionStore(_ defaults: UserDefaults, _ phone: Phone) -> ConnectionStore {
    ConnectionStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
}

func makeKeyStore(_ defaults: UserDefaults, _ phone: Phone) -> SSHKeyStore {
    SSHKeyStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
}

func makeShortcutStore(_ defaults: UserDefaults, _ phone: Phone) -> ShortcutStore {
    ShortcutStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
}

/// `phone` nil: always readable (the export op's scratch store).
func makeThemeStore(_ defaults: UserDefaults, _ phone: Phone?) -> ThemeStore {
    ThemeStore(defaults: defaults, readiness: DefaultsReadiness { phone?.unlocked ?? true })
}

func makeAppThemeStore(_ defaults: UserDefaults, _ phone: Phone) -> AppThemeStore {
    AppThemeStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
}

func makeSettings(_ defaults: UserDefaults, _ phone: Phone) -> AppSettings {
    AppSettings(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
}

func makeKnownHosts(_ keychain: TestKeychain) -> KnownHostsStore {
    KeychainKnownHostsStore(backend: keychain)
}

func readinessCheck(_ defaults: UserDefaults, _ phone: Phone) -> Bool {
    DefaultsReadiness { phone.unlocked }.isReadable(defaults)
}

func readinessMark(_ defaults: UserDefaults, _ phone: Phone) {
    DefaultsReadiness { phone.unlocked }.markReadable(defaults)
}

/// No commands but `run` here.
func runOtherCommand(_ args: [String]) -> Bool { false }

// The app makes its two singletons where it reads its storage switch
// (Moshpit/Services/Rust/RustStoreSwitch.swift, not compiled here);
// UI/AppTheme.swift names them. On the Swift stores, as the switch off does.
extension AppSettings {
    static let shared = AppSettings(readiness: .system)
}

extension AppThemeStore {
    static let shared = AppThemeStore(readiness: .system)
}
