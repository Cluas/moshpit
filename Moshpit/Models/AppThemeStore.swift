import Foundation
import Observation

/// Persistence for user-created app-chrome themes (accent colors).
///
/// A singleton, unlike ``ThemeStore``, because ``AppThemeCatalog/current`` — the
/// thing every `Ink.accent` call site resolves through — is a static computed
/// property with no view context to inject into. That indirection predates this
/// feature; custom themes just have to live where the catalog can reach them.
/// Views still read the same instance via `@Environment` so mutations publish.
/// That instance, `shared`, is made where the app reads its storage switch
/// (`RustStoreSwitch.swift`): on this class's own code, or on the Rust store.
@Observable
final class AppThemeStore {
    /// User-created themes, newest last.
    private(set) var customThemes: [AppTheme] = []

    @ObservationIgnored private let defaults: UserDefaults
    /// Read one theme at a time — see ``StoredList``.
    private static let list = StoredList<Stored>(key: "moshpit.settings.customAppThemes")

    /// One custom theme as persisted: only the accent is stored, because
    /// everything else is derived (see ``AppTheme/custom(id:name:accentHex:)``).
    /// Storing the derived values would mean a future tweak to the derivation
    /// silently not applying to themes already saved.
    private struct Stored: Codable {
        var id: String
        var name: String
        var accent: String
    }

    @ObservationIgnored private let readiness: DefaultsReadiness
    /// True once `customThemes` came from defaults known to be readable
    /// (``DefaultsReadiness``) and anything the read skipped is safe; until
    /// then nothing here is written back.
    @ObservationIgnored private(set) var isAuthoritative = false
    /// Stored themes this build could not decode, written back as stored.
    @ObservationIgnored private var carried: [Any] = []

    /// What reads and writes the themes when it isn't the code below: the
    /// Rust store, behind the `MOSHPIT_RUST_STORE` switch
    /// (``AppThemeStoreBacking``). nil: the code below, as ever.
    @ObservationIgnored private let backing: AppThemeStoreBacking?

    init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        self.defaults = defaults
        self.readiness = readiness
        backing = nil
        load()
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe. `defaults` and `readiness` go unused.
    init(backing: AppThemeStoreBacking) {
        defaults = .standard
        readiness = .always
        self.backing = backing
        adopt(backing)
    }

    /// Take what `backing` holds now, changing only what changed.
    private func adopt(_ backing: AppThemeStoreBacking) {
        let held = backing.customThemes
        if held != customThemes { customThemes = held }
        isAuthoritative = backing.isAuthoritative
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    func reloadIfNeeded() {
        if let backing {
            backing.reloadIfNeeded()
            return adopt(backing)
        }
        if !isAuthoritative { load() }
    }

    private func load() {
        let readable = readiness.isReadable(defaults)
        let contents = Self.list.read(from: defaults)
        customThemes = contents.elements.map {
            AppTheme.custom(id: $0.id, name: $0.name, accentHex: $0.accent)
        }
        carried = contents.carried
        isAuthoritative = readable && Self.list.secure(contents, in: defaults)
        if readable { readiness.markReadable(defaults) }
    }

    // MARK: - Mutation

    func save(_ theme: AppTheme) {
        if let backing {
            backing.save(theme)
            return adopt(backing)
        }
        guard !theme.isBuiltIn else { return }
        reloadIfNeeded()
        let rebuilt = AppTheme.custom(id: theme.id,
                                      name: uniqueName(theme.name, excluding: theme.id),
                                      accentHex: theme.accentHex)
        if let index = customThemes.firstIndex(where: { $0.id == rebuilt.id }) {
            customThemes[index] = rebuilt
        } else {
            customThemes.append(rebuilt)
        }
        persist()
    }

    func delete(id: String) {
        if let backing {
            backing.delete(id: id)
            return adopt(backing)
        }
        reloadIfNeeded()
        customThemes.removeAll { $0.id == id }
        persist()
    }

    /// Only used by the DEBUG `-MOSHPIT_RESET` launch seam — these live in
    /// UserDefaults and would otherwise survive a simulator reinstall.
    func removeAllCustom() {
        if let backing {
            backing.removeAllCustom()
            return adopt(backing)
        }
        reloadIfNeeded()
        guard !customThemes.isEmpty else { return }
        customThemes.removeAll()
        persist()
    }

    func isCustom(_ id: String) -> Bool { customThemes.contains { $0.id == id } }

    /// A draft starting from the current accent, so the editor opens on
    /// something already coherent rather than an arbitrary color.
    func makeDraft(basedOn theme: AppTheme) -> AppTheme {
        if let backing { return backing.makeDraft(basedOn: theme) }
        return AppTheme.custom(id: "app-\(UUID().uuidString.prefix(8).lowercased())",
                        name: uniqueName("My Accent", excluding: ""),
                        accentHex: theme.accentHex)
    }

    // MARK: - Private

    private func uniqueName(_ name: String, excluding id: String) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let base = trimmed.isEmpty ? String(localized: "Untitled") : trimmed
        let taken = Set((AppThemeCatalog.builtIns + customThemes)
            .filter { $0.id != id }.map(\.name))
        guard taken.contains(base) else { return base }
        var suffix = 2
        while taken.contains("\(base) \(suffix)") { suffix += 1 }
        return "\(base) \(suffix)"
    }

    private func persist() {
        guard isAuthoritative else { return }
        let stored = customThemes.map { Stored(id: $0.id, name: $0.name, accent: $0.accentHex) }
        guard let data = Self.list.encode(stored, carrying: carried) else { return }
        defaults.set(data, forKey: Self.list.key)
    }
}

/// What an ``AppThemeStore`` reads and writes through when it isn't its own
/// Swift code: the Rust store (crates/store, through crates/ffi), behind the
/// `MOSHPIT_RUST_STORE` switch (`RustStoreSwitch`). Each member keeps the
/// contract of the store's member of the same name — only the accent
/// stored, built-ins refused, names made unique.
protocol AppThemeStoreBacking: AnyObject {
    var customThemes: [AppTheme] { get }
    var isAuthoritative: Bool { get }
    func reloadIfNeeded()
    func save(_ theme: AppTheme)
    func delete(id: String)
    func removeAllCustom()
    func makeDraft(basedOn theme: AppTheme) -> AppTheme
}
