import Foundation
import SwiftUI
import Testing
@testable import Ringdown

/// Covers the theme model's storage/encoding contract and the store's
/// persistence + id resolution — the parts a broken change would surface as
/// "my custom theme vanished" or "the terminal went black on black".
@Suite("Terminal themes")
struct ThemeTests {

    // MARK: Hex parsing

    @Test("hex forms normalize to bare uppercase RRGGBB")
    func hexNormalization() {
        #expect(Color.normalizedHex("#0d1117") == "0D1117")
        #expect(Color.normalizedHex("0D1117") == "0D1117")
        #expect(Color.normalizedHex("0x0D1117") == "0D1117")
        #expect(Color.normalizedHex("  #0d1117  ") == "0D1117")
        // 3-digit shorthand expands by doubling each nibble.
        #expect(Color.normalizedHex("#f0a") == "FF00AA")
        // 8-digit drops alpha rather than failing.
        #expect(Color.normalizedHex("#0D1117FF") == "0D1117")
    }

    @Test("unparseable hex degrades to black instead of failing")
    func hexFallback() {
        // A theme with one typo'd color must still load — it is fixable in the
        // editor, whereas a decode failure would lose the whole theme.
        #expect(Color.normalizedHex("nope") == "000000")
        #expect(Color.normalizedHex("") == "000000")
    }

    @Test("Color round-trips through hexString")
    func colorRoundTrip() {
        for hex in ["0D1117", "FF5555", "FFFFFF", "000000", "5FE3D8"] {
            #expect(Color(hex: hex).hexString == hex, "round trip failed for \(hex)")
        }
    }

    // MARK: Bright derivation

    @Test("derived bright colors lift toward white and keep black distinct")
    func brightDerivation() {
        let theme = TerminalTheme.githubDark
        // No overrides on a built-in, so every bright slot is derived.
        for slot in TerminalTheme.ANSISlot.allCases {
            #expect(!theme.hasBrightOverride(slot))
            #expect(theme.brightHex(slot) != theme.ansiHex(slot),
                    "\(slot.key): bright must differ from base — duplicating them is what made dim text invisible")
        }
        // Pure black lifts to a visible grey (bright black is what many tools
        // paint dim text with).
        #expect(TerminalTheme.derivedBright(from: "000000") == "525252")
        // Pure white stays white rather than overflowing.
        #expect(TerminalTheme.derivedBright(from: "FFFFFF") == "FFFFFF")
    }

    @Test("an explicit bright override wins, and clearing it restores derivation")
    func brightOverride() {
        var theme = TerminalTheme.githubDark
        let derived = theme.brightHex(.red)

        theme.setBrightOverride("#123456", for: .red)
        #expect(theme.hasBrightOverride(.red))
        #expect(theme.brightHex(.red) == "123456")
        // Other slots keep deriving.
        #expect(!theme.hasBrightOverride(.blue))

        theme.setBrightOverride(nil, for: .red)
        #expect(!theme.hasBrightOverride(.red))
        #expect(theme.brightHex(.red) == derived)
        // Fully cleared overrides collapse back to nil so the theme encodes
        // without a dead `bright` array.
        #expect(theme.brightHex == nil)
    }

    @Test("the SwiftTerm palette is 16 distinct entries, base then bright")
    func paletteShape() {
        let theme = TerminalTheme.dracula
        let slots = TerminalTheme.ANSISlot.allCases
        let base = slots.map { theme.ansiHex($0) }
        let bright = slots.map { theme.brightHex($0) }
        #expect(base.count == 8)
        #expect(bright.count == 8)
        #expect(Set(base + bright).count > 8, "bright half must not duplicate the base half")
    }

    // MARK: Codable

    @Test("a theme round-trips through JSON with human-readable keys")
    func codableRoundTrip() throws {
        var original = TerminalTheme.nord
        original.id = "custom-abc"
        original.isBuiltIn = false
        original.setBrightOverride("#ABCDEF", for: .cyan)

        let data = try JSONEncoder().encode(original)
        let json = String(decoding: data, as: UTF8.self)
        // Keys a person can read and hand-edit.
        #expect(json.contains("\"background\""))
        #expect(json.contains("\"brightCyan\""))
        // Slots without an override are not written out.
        #expect(!json.contains("\"brightRed\""))

        let decoded = try JSONDecoder().decode(TerminalTheme.self, from: data)
        #expect(decoded.id == original.id)
        #expect(decoded.name == original.name)
        #expect(decoded.backgroundHex == original.backgroundHex)
        #expect(decoded.ansiHex == original.ansiHex)
        #expect(decoded.brightHex(.cyan) == "ABCDEF")
        #expect(decoded.brightHex(.red) == original.brightHex(.red))
    }

    @Test("decoding never yields a built-in — anything imported is the user's")
    func decodedThemesAreCustom() throws {
        let data = try JSONEncoder().encode(TerminalTheme.githubDark)
        let decoded = try JSONDecoder().decode(TerminalTheme.self, from: data)
        // Otherwise an imported theme would be uneditable and undeletable.
        #expect(!decoded.isBuiltIn)
    }

    @Test("a hand-written palette from another emulator decodes")
    func lenientImport() throws {
        // Shorthand keys and no id — the shape someone pastes from a gist.
        let json = """
        { "name": "Pasted", "bg": "#1e1e1e", "fg": "#d4d4d4",
          "black": "#000000", "red": "#cd3131", "green": "#0dbc79",
          "yellow": "#e5e510", "blue": "#2472c8", "magenta": "#bc3fbc",
          "cyan": "#11a8cd", "white": "#e5e5e5" }
        """
        let theme = try JSONDecoder().decode(TerminalTheme.self, from: Data(json.utf8))
        #expect(theme.name == "Pasted")
        #expect(theme.backgroundHex == "1E1E1E")
        #expect(theme.foregroundHex == "D4D4D4")
        // No cursor given → falls back to the foreground rather than to black,
        // which would make the caret invisible.
        #expect(theme.cursorHex == "D4D4D4")
        #expect(theme.ansiHex(.red) == "CD3131")
        #expect(!theme.id.isEmpty)
    }

    // MARK: Store

    /// A store on its own defaults domain so tests never touch real settings.
    private func makeStore(_ name: String = UUID().uuidString) -> ThemeStore {
        ThemeStore(defaults: UserDefaults(suiteName: "themetests.\(name)")!)
    }

    @Test("built-ins are listed and resolvable, custom themes append after them")
    func storeListing() {
        let store = makeStore()
        #expect(store.allThemes.count == TerminalTheme.builtIns.count)
        #expect(store.theme(id: "dracula").id == "dracula")

        let draft = store.makeDraft()
        store.save(draft)
        #expect(store.customThemes.count == 1)
        #expect(store.allThemes.last?.id == draft.id)
        #expect(store.isCustom(draft.id))
        #expect(!store.isCustom("dracula"))
    }

    @Test("an unknown or deleted id resolves to the fallback, not to black")
    func storeFallback() {
        let store = makeStore()
        #expect(store.theme(id: "nope").id == TerminalTheme.fallback.id)

        let draft = store.makeDraft()
        store.save(draft)
        store.delete(id: draft.id)
        #expect(store.theme(id: draft.id).id == TerminalTheme.fallback.id)
    }

    @Test("saving a built-in is refused — it would resurrect on next launch")
    func storeRejectsBuiltIns() {
        let store = makeStore()
        store.save(TerminalTheme.dracula)
        #expect(store.customThemes.isEmpty)
    }

    @Test("duplicate produces an editable copy with a fresh id and unique name")
    func storeDuplicate() {
        let store = makeStore()
        let copy = store.duplicate(.dracula)
        #expect(copy.id != TerminalTheme.dracula.id)
        #expect(!copy.isBuiltIn)
        #expect(copy.name == "Dracula Copy")
        #expect(copy.ansiHex == TerminalTheme.dracula.ansiHex)

        store.save(copy)
        // A second duplicate can't collide by name.
        let second = store.duplicate(.dracula)
        store.save(second)
        #expect(Set(store.customThemes.map(\.name)).count == 2)
    }

    @Test("names collide-proof against built-ins too")
    func storeUniqueNames() {
        let store = makeStore()
        var draft = store.makeDraft()
        draft.name = "Dracula"
        store.save(draft)
        #expect(store.customThemes[0].name == "Dracula 2")
    }

    @Test("custom themes survive a fresh store on the same defaults")
    func storePersistence() {
        let suite = UUID().uuidString
        let defaults = UserDefaults(suiteName: "themetests.\(suite)")!
        let store = ThemeStore(defaults: defaults)
        var draft = store.makeDraft()
        draft.name = "Persisted"
        draft.backgroundHex = "112233"
        store.save(draft)

        let reloaded = ThemeStore(defaults: defaults)
        #expect(reloaded.customThemes.count == 1)
        #expect(reloaded.customThemes[0].name == "Persisted")
        #expect(reloaded.customThemes[0].backgroundHex == "112233")
        #expect(!reloaded.customThemes[0].isBuiltIn)
    }

    @Test("import assigns fresh ids so a payload can't overwrite an existing theme")
    func importMintsIDs() throws {
        let store = makeStore()
        // A payload claiming to be a built-in.
        let json = try store.exportJSON(TerminalTheme.dracula)
        let imported = try store.importThemes(json: json)

        #expect(imported.count == 1)
        #expect(imported[0].id != "dracula")
        #expect(store.customThemes.count == 1)
        // The real built-in is untouched and still resolvable.
        #expect(store.theme(id: "dracula").isBuiltIn)
    }

    @Test("import accepts an array of themes")
    func importArray() throws {
        let store = makeStore()
        let one = try store.exportJSON(.nord)
        let two = try store.exportJSON(.monokai)
        let imported = try store.importThemes(json: "[\(one),\(two)]")
        #expect(imported.count == 2)
        #expect(store.customThemes.count == 2)
        #expect(Set(store.customThemes.map(\.id)).count == 2)
    }

    @Test("import rejects junk with a message instead of crashing")
    func importRejectsJunk() {
        let store = makeStore()
        #expect(throws: (any Error).self) { try store.importThemes(json: "not json at all") }
        #expect(throws: (any Error).self) { try store.importThemes(json: "") }
        #expect(store.customThemes.isEmpty)
    }
}
