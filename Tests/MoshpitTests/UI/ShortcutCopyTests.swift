import Foundation
import Testing
@testable import Moshpit

/// B234: what the built-in shortcuts are called was English in every
/// language. Read here from the app's own compiled catalog, one language at
/// a time. (The editor's quick keys are `ui/src/add_shortcut.rs`'s, tested
/// there.)
@Suite("Built-in shortcuts' names in every language (B234)")
struct ShortcutCopyTests {

    private static let languages = ["zh-Hans", "zh-Hant", "ja", "ko", "de", "es", "fr", "pt-BR"]

    private static func bundle(_ language: String) throws -> Bundle {
        let path = try #require(Bundle.main.path(forResource: language, ofType: "lproj"),
                                "no \(language).lproj in the app")
        return try #require(Bundle(path: path))
    }

    /// The built-ins' names.
    private static var english: [String] {
        ShortcutStore.builtins.map(\.summary)
    }

    @Test("every built-in's name is in each language", arguments: languages)
    func describedInEveryLanguage(language: String) throws {
        let bundle = try Self.bundle(language)
        // Proper names of keys a language may keep as they are.
        let kept: Set<String> = ["Home", "End", "Escape", "Return"]
        // Words a language spells exactly as English does, by its catalog
        // entry (the German and French NAVIGATION, the Spanish CONTROL, the
        // key name Control in several): the same text is the translation.
        let same: [String: Set<String>] = [
            "de": ["Control", "NAVIGATION"], "es": ["Control", "CONTROL"],
            "fr": ["NAVIGATION"], "ko": ["Control"], "pt-BR": ["Control"],
        ]
        let alike = same[language] ?? []
        for text in Set(Self.english) {
            let shown = ShortcutCopy.describe(text, bundle: bundle)
            #expect(!shown.isEmpty)
            if !kept.contains(text), !alike.contains(text) {
                #expect(shown != text, "\(language): \"\(text)\" is still English")
            }
        }
    }

    @Test("Chinese reads as Chinese")
    func chinese() throws {
        let zh = try Self.bundle("zh-Hans")
        #expect(ShortcutCopy.describe("Interrupt (SIGINT)", bundle: zh) == "中断（SIGINT）")
        #expect(ShortcutCopy.describe("Tab", bundle: zh) == "Tab 键", "the key, not the multiplexer's tab")
        #expect(ShortcutCopy.describe("Clear line before cursor", bundle: zh) == "删掉光标前的内容")
    }

    @Test("a built-in is shown translated and stored as it was; the user's own words are their own")
    func displayOnly() {
        var builtin = ShortcutStore.builtins[0]
        #expect(builtin.summary == "Escape")
        builtin.isBuiltin = true
        #expect(builtin.displaySummary == ShortcutCopy.describe("Escape"))
        var mine = TerminalShortcut()
        mine.summary = "Escape"
        mine.isBuiltin = false
        #expect(mine.displaySummary == "Escape")
        // The NSH page gets the built-in's name in the app's language.
        let message = ShortcutsModel.message(ShortcutsSnapshot(shortcuts: [builtin]))
        let listed = ((message["toolbar"] as? [[String: Any]]) ?? []) + ((message["available"] as? [[String: Any]]) ?? [])
        #expect(listed.first?["summary"] as? String == builtin.displaySummary)
    }
}
