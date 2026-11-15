import Foundation
import Observation

/// The user-created app-chrome themes (accent colors), as the views watch
/// them. The Rust store (`crates/store/src/app_themes.rs`, through
/// ``AppThemeStoreBacking``) does every read and write — only the accent
/// stored, themes this build can't decode carried as stored, built-ins
/// refused, names made unique, the sealed-launch rule.
///
/// A singleton, unlike ``ThemeStore``, because ``AppThemeCatalog/current`` — the
/// thing every `Ink.accent` call site resolves through — is a static computed
/// property with no view context to inject into. Views still read the same
/// instance via `@Environment` so mutations publish. That instance, `shared`,
/// is made with the app's other stores (``AppStores``).
@Observable
final class AppThemeStore {
    /// User-created themes, newest last.
    private(set) var customThemes: [AppTheme] = []

    /// True once `customThemes` came from preferences known to be readable
    /// and anything the read skipped is safe; until then nothing here is
    /// written back.
    @ObservationIgnored private(set) var isAuthoritative = false

    @ObservationIgnored private let backing: AppThemeStoreBacking

    /// The Rust store over `defaults`, readable as `readiness` says (tests
    /// and scratch suites; the app's is ``shared``).
    convenience init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        let host = RustPreferencesHost(defaults: defaults, available: readiness.protectedDataAvailable)
        self.init(backing: RustAppThemeStore(preferences: host, mode: .system,
                                             untitled: String(localized: "Untitled")))
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe.
    init(backing: AppThemeStoreBacking) {
        self.backing = backing
        adopt()
    }

    /// Take what the backing holds now, changing only what changed.
    private func adopt() {
        let held = backing.customThemes
        if held != customThemes { customThemes = held }
        isAuthoritative = backing.isAuthoritative
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    func reloadIfNeeded() {
        backing.reloadIfNeeded()
        adopt()
    }

    /// Insert or replace by id; a built-in is refused.
    func save(_ theme: AppTheme) {
        backing.save(theme)
        adopt()
    }

    func delete(id: String) {
        backing.delete(id: id)
        adopt()
    }

    func removeAllCustom() {
        backing.removeAllCustom()
        adopt()
    }

    func isCustom(_ id: String) -> Bool { customThemes.contains { $0.id == id } }

    /// A draft starting from the current accent, so the editor opens on
    /// something already coherent rather than an arbitrary color.
    func makeDraft(basedOn theme: AppTheme) -> AppTheme {
        backing.makeDraft(basedOn: theme)
    }
}

/// What an ``AppThemeStore`` reads and writes through: the Rust store
/// (crates/store, through crates/ffi; ``RustAppThemeStore``), or a test's
/// stand-in. Each member keeps the contract of the store's member of the
/// same name — only the accent stored, built-ins refused, names made
/// unique.
protocol AppThemeStoreBacking: AnyObject {
    var customThemes: [AppTheme] { get }
    var isAuthoritative: Bool { get }
    func reloadIfNeeded()
    func save(_ theme: AppTheme)
    func delete(id: String)
    func removeAllCustom()
    func makeDraft(basedOn theme: AppTheme) -> AppTheme
}
