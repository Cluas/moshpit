import Foundation
import Observation

/// Owns the terminal themes available to the app: the built-in set plus any the
/// user created, and the persistence + id resolution around them.
///
/// Custom themes live in `UserDefaults` as a single JSON array. That keeps them
/// in the same store as the `themeId` selection that points at them, so a
/// selection can never be restored from a snapshot newer than the themes it
/// refers to — and it means "export" is the same encoder the store already uses.
@Observable
final class ThemeStore {
    /// User-created themes, newest last. Built-ins are not in here.
    private(set) var customThemes: [TerminalTheme] = []

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let storageKey = "moshpit.settings.customThemes"

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        self.customThemes = Self.load(from: defaults, key: storageKey)
    }

    // MARK: - Lookup

    /// Built-ins first, then custom — the order the gallery renders in.
    var allThemes: [TerminalTheme] { TerminalTheme.builtIns + customThemes }

    /// Resolve a stored id. Falls back to the default theme when the id names a
    /// custom theme that has since been deleted (or a built-in that a future
    /// version dropped), so a stale selection degrades to "looks normal" rather
    /// than to a black-on-black terminal.
    func theme(id: String) -> TerminalTheme {
        allThemes.first { $0.id == id } ?? .fallback
    }

    func isCustom(_ id: String) -> Bool {
        customThemes.contains { $0.id == id }
    }

    // MARK: - Mutation

    /// Insert or replace by id. Built-in themes are rejected: a built-in is
    /// compiled in and would silently resurrect on next launch, so the gallery
    /// routes "edit a built-in" through ``duplicate(_:)`` instead.
    func save(_ theme: TerminalTheme) {
        guard !theme.isBuiltIn else { return }
        var updated = theme
        updated.name = uniqueName(updated.name, excluding: updated.id)
        if let index = customThemes.firstIndex(where: { $0.id == updated.id }) {
            customThemes[index] = updated
        } else {
            customThemes.append(updated)
        }
        persist()
    }

    func delete(id: String) {
        customThemes.removeAll { $0.id == id }
        persist()
    }

    /// Drop every custom theme. Only used by the DEBUG `-MOSHPIT_RESET` launch
    /// seam: custom themes live in `UserDefaults` and therefore survive an app
    /// reinstall in the simulator, so without this a UI test's "fresh install"
    /// still sees themes left behind by the previous run.
    func removeAllCustom() {
        guard !customThemes.isEmpty else { return }
        customThemes.removeAll()
        persist()
    }

    /// An editable copy of any theme, with a fresh id and a "Copy" name. This is
    /// the only way to get from a built-in to something editable.
    func duplicate(_ theme: TerminalTheme) -> TerminalTheme {
        var copy = theme
        copy.id = Self.mintID()
        copy.isBuiltIn = false
        copy.name = uniqueName("\(theme.name) Copy", excluding: copy.id)
        return copy
    }

    /// A blank-slate theme for "new from scratch" — starts from the default
    /// palette so every slot has a sane value and the preview is legible before
    /// the user has touched anything.
    func makeDraft() -> TerminalTheme {
        var draft = TerminalTheme.fallback
        draft.id = Self.mintID()
        draft.isBuiltIn = false
        draft.name = uniqueName("My Theme", excluding: draft.id)
        return draft
    }

    // MARK: - Import / export

    enum ImportError: LocalizedError {
        case notJSON
        case noThemes

        var errorDescription: String? {
            switch self {
            case .notJSON:
                return String(localized: "That doesn't look like theme JSON. Paste an exported theme, or an object with \"background\", \"foreground\" and the eight ANSI color names.")
            case .noThemes:
                return String(localized: "No themes found in that JSON.")
            }
        }
    }

    /// Pretty-printed JSON for one theme, with keys in a stable order so a
    /// re-export produces a reviewable diff rather than a reshuffle.
    func exportJSON(_ theme: TerminalTheme) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return String(decoding: try encoder.encode(theme), as: UTF8.self)
    }

    /// Decode one theme or an array of them, saving each as a custom theme.
    /// Returns what was imported so the caller can select it.
    ///
    /// Ids from the payload are deliberately **not** trusted: importing a theme
    /// whose id collides with a built-in (or with an existing custom theme the
    /// user still wants) would either be rejected or silently overwrite. Every
    /// import gets a fresh id.
    @discardableResult
    func importThemes(json: String) throws -> [TerminalTheme] {
        guard let data = json.data(using: .utf8), !json.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw ImportError.notJSON
        }
        let decoder = JSONDecoder()
        var decoded: [TerminalTheme]
        if let many = try? decoder.decode([TerminalTheme].self, from: data) {
            decoded = many
        } else if let one = try? decoder.decode(TerminalTheme.self, from: data) {
            decoded = [one]
        } else {
            throw ImportError.notJSON
        }
        guard !decoded.isEmpty else { throw ImportError.noThemes }

        var imported: [TerminalTheme] = []
        for var theme in decoded {
            theme.id = Self.mintID()
            theme.isBuiltIn = false
            save(theme)
            // Re-read: save() may have de-duplicated the name.
            imported.append(customThemes.last ?? theme)
        }
        return imported
    }

    // MARK: - Private

    /// Appends " 2", " 3", … until the name is free. Names are what the user
    /// navigates by, so two themes called "My Theme" is a worse outcome than a
    /// slightly adjusted name.
    private func uniqueName(_ name: String, excluding id: String) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let base = trimmed.isEmpty ? String(localized: "Untitled") : trimmed
        let taken = Set(allThemes.filter { $0.id != id }.map(\.name))
        guard taken.contains(base) else { return base }
        var suffix = 2
        while taken.contains("\(base) \(suffix)") { suffix += 1 }
        return "\(base) \(suffix)"
    }

    private static func mintID() -> String {
        "custom-\(UUID().uuidString.prefix(8).lowercased())"
    }

    private func persist() {
        let encoder = JSONEncoder()
        guard let data = try? encoder.encode(customThemes) else { return }
        defaults.set(data, forKey: storageKey)
    }

    private static func load(from defaults: UserDefaults, key: String) -> [TerminalTheme] {
        guard let data = defaults.data(forKey: key),
              let decoded = try? JSONDecoder().decode([TerminalTheme].self, from: data)
        else { return [] }
        // Decoded themes are custom by construction (`isBuiltIn` is not encoded),
        // but be explicit — a persisted `true` would make the theme uneditable
        // and undeletable with no way back.
        return decoded.map { theme in
            var t = theme
            t.isBuiltIn = false
            return t
        }
    }
}
