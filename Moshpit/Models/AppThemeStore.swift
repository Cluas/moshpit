import Foundation
import Observation

/// Persistence for user-created app-chrome themes (accent colors).
///
/// A singleton, unlike ``ThemeStore``, because ``AppThemeCatalog/current`` — the
/// thing every `Ink.accent` call site resolves through — is a static computed
/// property with no view context to inject into. That indirection predates this
/// feature; custom themes just have to live where the catalog can reach them.
/// Views still read the same instance via `@Environment` so mutations publish.
@Observable
final class AppThemeStore {
    static let shared = AppThemeStore()

    /// User-created themes, newest last.
    private(set) var customThemes: [AppTheme] = []

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let storageKey = "moshpit.settings.customAppThemes"

    /// One custom theme as persisted: only the accent is stored, because
    /// everything else is derived (see ``AppTheme/custom(id:name:accentHex:)``).
    /// Storing the derived values would mean a future tweak to the derivation
    /// silently not applying to themes already saved.
    private struct Stored: Codable {
        var id: String
        var name: String
        var accent: String
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        self.customThemes = Self.load(from: defaults, key: storageKey)
    }

    // MARK: - Mutation

    func save(_ theme: AppTheme) {
        guard !theme.isBuiltIn else { return }
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
        customThemes.removeAll { $0.id == id }
        persist()
    }

    /// Only used by the DEBUG `-MOSHPIT_RESET` launch seam — these live in
    /// UserDefaults and would otherwise survive a simulator reinstall.
    func removeAllCustom() {
        guard !customThemes.isEmpty else { return }
        customThemes.removeAll()
        persist()
    }

    func isCustom(_ id: String) -> Bool { customThemes.contains { $0.id == id } }

    /// A draft starting from the current accent, so the editor opens on
    /// something already coherent rather than an arbitrary color.
    func makeDraft(basedOn theme: AppTheme) -> AppTheme {
        AppTheme.custom(id: "app-\(UUID().uuidString.prefix(8).lowercased())",
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
        let stored = customThemes.map { Stored(id: $0.id, name: $0.name, accent: $0.accentHex) }
        guard let data = try? JSONEncoder().encode(stored) else { return }
        defaults.set(data, forKey: storageKey)
    }

    private static func load(from defaults: UserDefaults, key: String) -> [AppTheme] {
        guard let data = defaults.data(forKey: key),
              let stored = try? JSONDecoder().decode([Stored].self, from: data)
        else { return [] }
        return stored.map { AppTheme.custom(id: $0.id, name: $0.name, accentHex: $0.accent) }
    }
}
