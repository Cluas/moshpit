import Foundation
import Observation

/// The terminal themes available to the app, as the views watch them: the
/// built-in set plus any the user created. The Rust store
/// (`crates/store/src/themes.rs`, through ``ThemeStoreBacking``) does every
/// read and write — the stored form, themes this build can't decode carried
/// as stored, built-ins refused, names made unique, ids minted, import and
/// export, the sealed-launch rule — and this object keeps what the views
/// observe and resolves ids.
@Observable
final class ThemeStore {
    /// User-created themes, newest last. Built-ins are not in here.
    private(set) var customThemes: [TerminalTheme] = []

    /// True once `customThemes` came from preferences known to be readable
    /// and anything the read skipped is safe; until then nothing here is
    /// written back.
    @ObservationIgnored private(set) var isAuthoritative = false

    @ObservationIgnored private let backing: ThemeStoreBacking

    /// The Rust store over `defaults`, readable as `protectedData` says (the core's readiness rule) (tests
    /// and scratch suites; the app's is ``AppStores/themes()``).
    convenience init(defaults: UserDefaults = .standard, protectedData: @escaping () -> Bool = { true }) {
        let host = RustPreferencesHost(defaults: defaults, available: protectedData)
        self.init(backing: RustThemeStore(preferences: host, mode: .system,
                                          untitled: String(localized: "Untitled")))
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe.
    init(backing: ThemeStoreBacking) {
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

    /// Insert or replace by id. Built-in themes are refused: a built-in is
    /// compiled in and would silently resurrect on next launch, so the gallery
    /// routes "edit a built-in" through ``duplicate(_:)`` instead.
    func save(_ theme: TerminalTheme) {
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

    /// An editable copy of any theme, with a fresh id and a "Copy" name. This is
    /// the only way to get from a built-in to something editable.
    func duplicate(_ theme: TerminalTheme) -> TerminalTheme {
        backing.duplicate(theme)
    }

    /// A blank-slate theme for "new from scratch" — starts from the default
    /// palette so every slot has a sane value and the preview is legible before
    /// the user has touched anything.
    func makeDraft() -> TerminalTheme {
        backing.makeDraft()
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
        backing.exportJSON(theme)
    }

    /// Decode one theme or an array of them, saving each as a custom theme
    /// with a fresh id (a payload's ids are never trusted). Returns what was
    /// imported so the caller can select it.
    @discardableResult
    func importThemes(json: String) throws -> [TerminalTheme] {
        defer { adopt() }
        return try backing.importThemes(json: json)
    }
}

/// What a ``ThemeStore`` reads and writes through: the Rust store
/// (crates/store, through crates/ffi; ``RustThemeStore``), or a test's
/// stand-in. Each member keeps the contract of the store's member of the
/// same name — built-ins refused,
/// names made unique, ids minted, an import's errors as
/// ``ThemeStore/ImportError``.
protocol ThemeStoreBacking: AnyObject {
    var customThemes: [TerminalTheme] { get }
    var isAuthoritative: Bool { get }
    func reloadIfNeeded()
    func save(_ theme: TerminalTheme)
    func delete(id: String)
    func removeAllCustom()
    func duplicate(_ theme: TerminalTheme) -> TerminalTheme
    func makeDraft() -> TerminalTheme
    func exportJSON(_ theme: TerminalTheme) -> String
    func importThemes(json: String) throws -> [TerminalTheme]
}
