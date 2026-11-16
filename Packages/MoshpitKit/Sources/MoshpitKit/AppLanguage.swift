import Foundation

/// The language Moshpit shows — one answer for every process that writes
/// Moshpit's words: the app, the Live Activity and the widget
/// (MoshpitIsland), and the cards the notification service extension
/// (MoshpitPush) writes. UI-DESIGN.md §10.
///
/// Each of those used to resolve its strings in its own process, against
/// its own languages. The app's are the ones the user picked for it (iOS
/// applies Settings → Moshpit → Language to the app's process); the
/// extensions' are the system's. With Moshpit set to English on a phone in
/// Chinese, the app and the cards it posts read English, while the island,
/// the widget and every pushed card read Chinese — and a pushed card
/// replaces the local copy of the same event, so one card changed language
/// depending on which copy landed last.
///
/// The app records its resolved language in the App Group at launch (iOS
/// relaunches the app whenever its language changes, per-app or the
/// system's), and every process writes in it. Nothing recorded — a phone
/// that hasn't launched this build, or a read before the first unlock —
/// falls back to the process's own languages, as before.
public enum AppLanguage {
    /// Where the app records it, in the App Group's defaults.
    public static let key = "moshpit.app.language"

    /// The localization this process's own bundle resolves to. In the app,
    /// the language Moshpit shows.
    public static var own: String { Bundle.main.preferredLocalizations.first ?? "en" }

    /// Record the app's language. The app calls this at launch.
    public static func record(_ language: String = own, suiteName: String = AppGroup.id) {
        guard !language.isEmpty, let defaults = UserDefaults(suiteName: suiteName) else { return }
        if defaults.string(forKey: key) != language {
            defaults.set(language, forKey: key)
        }
    }

    /// What the app recorded, if anything.
    public static func stored(suiteName: String = AppGroup.id) -> String? {
        guard let value = UserDefaults(suiteName: suiteName)?.string(forKey: key), !value.isEmpty
        else { return nil }
        return value
    }

    /// The language this process writes in: the app's, as recorded; this
    /// process's own when nothing is.
    public static func current(suiteName: String = AppGroup.id) -> String {
        stored(suiteName: suiteName) ?? own
    }

    /// `key`, in `language`, from `bundle`'s strings — the catalog the
    /// calling process ships (the extension's own, inside an extension).
    public static func string(_ key: String.LocalizationValue,
                              language: String = current(),
                              in bundle: Bundle = .main,
                              comment: StaticString? = nil) -> String {
        let (strings, table) = lookup(language, in: bundle)
        return String(localized: key, table: table, bundle: strings, locale: locale(language),
                      comment: comment)
    }

    /// The locale numbers, units and durations are written in for `language`.
    public static func locale(_ language: String = current()) -> Locale {
        Locale(identifier: language)
    }

    /// Where `language`'s strings are in `bundle`, and under which table.
    ///
    /// * `bundle` has `<language>.lproj`: that folder, opened as a bundle of
    ///   its own, so lookups go to that localization whatever this
    ///   process's languages are.
    /// * `language` is the one the keys are written in (English), with no
    ///   folder of its own — the push extension's catalog has none, its
    ///   English being the keys themselves: a table that doesn't exist, so
    ///   the lookup returns the key. Falling back to `bundle` here would
    ///   answer in the system's language, which is the bug.
    /// * Anything else `bundle` doesn't have: `bundle` itself, and this
    ///   process's own choice.
    static func lookup(_ language: String, in bundle: Bundle) -> (Bundle, String?) {
        if let path = bundle.path(forResource: language, ofType: "lproj"),
           let folder = Bundle(path: path) {
            return (folder, nil)
        }
        if language == (bundle.developmentLocalization ?? "en") {
            return (bundle, sourceTable)
        }
        return (bundle, nil)
    }

    /// No table by this name exists in any bundle: looking a key up in it
    /// returns the key, which is the source language's text.
    static let sourceTable = "MoshpitSourceLanguage"
}
