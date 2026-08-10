import Foundation
import SwiftUI
import Testing
@testable import Moshpit

/// Covers the app-chrome theme model: the derivation that makes a custom accent
/// cheap (one color in, four out), the store's persistence, and the icon
/// migration that keeps a pre-split install's home-screen icon.
@Suite("App themes and icons")
struct AppThemeTests {

    // MARK: Derivation

    @Test("a custom accent derives its pressed state and background wash")
    func customDerivation() {
        let theme = AppTheme.custom(id: "app-x", name: "Mine", accentHex: "#FF8800")
        #expect(theme.accentHex == "FF8800")
        #expect(!theme.isBuiltIn)

        // Pressed is darker than the accent but not black.
        let pressed = theme.accentPressed.hexString
        #expect(pressed != "FF8800")
        #expect(pressed != "000000")
        #expect(Color.rgbComponents(hex: pressed).0 < Color.rgbComponents(hex: "FF8800").0)

        // Backgrounds are near-black — a mood cue, not a colored app.
        for hex in [theme.screenBG.hexString, theme.terminalBG.hexString] {
            let (r, g, b) = Color.rgbComponents(hex: hex)
            #expect(r < 0.09 && g < 0.09 && b < 0.09, "\(hex) should stay near-black")
        }
        // …and the terminal is the darker of the two.
        #expect(Color.rgbComponents(hex: theme.terminalBG.hexString).0
                <= Color.rgbComponents(hex: theme.screenBG.hexString).0)
    }

    @Test("equality tracks the accent value, not just the id")
    func equalityTracksValue() {
        // The root view's rebuild key relies on this: editing a custom accent
        // keeps its id, and an id-only comparison left the app painted in the
        // pre-edit color.
        let a = AppTheme.custom(id: "app-x", name: "Mine", accentHex: "FF8800")
        let b = AppTheme.custom(id: "app-x", name: "Mine", accentHex: "00FF88")
        #expect(a != b)
        #expect(a == AppTheme.custom(id: "app-x", name: "Renamed", accentHex: "FF8800"))
    }

    @Test("hex blending stays in range at both extremes")
    func blending() {
        #expect(Color.blend("FF0000", toward: "000000", by: 0) == "FF0000")
        #expect(Color.blend("FF0000", toward: "000000", by: 1) == "000000")
        #expect(Color.blend("FF0000", toward: "FFFFFF", by: 1) == "FFFFFF")
        // Out-of-range factors clamp rather than overflow.
        #expect(Color.blend("FF0000", toward: "000000", by: 5) == "000000")
        #expect(Color.blend("FF0000", toward: "000000", by: -3) == "FF0000")
    }

    // MARK: Store

    private func makeStore(_ name: String = UUID().uuidString) -> AppThemeStore {
        AppThemeStore(defaults: UserDefaults(suiteName: "appthemetests.\(name)")!)
    }

    @Test("saving a custom accent stores it; built-ins are refused")
    func storeSave() {
        let store = makeStore()
        #expect(store.customThemes.isEmpty)

        store.save(AppTheme.custom(id: "app-1", name: "Mine", accentHex: "FF8800"))
        #expect(store.customThemes.count == 1)
        #expect(store.isCustom("app-1"))

        // A built-in would resurrect from code on next launch.
        store.save(AppThemeCatalog.signalRoom)
        #expect(store.customThemes.count == 1)
    }

    @Test("names are de-duplicated against built-ins too")
    func storeUniqueNames() {
        let store = makeStore()
        store.save(AppTheme.custom(id: "app-1", name: "Signal Room", accentHex: "FF8800"))
        #expect(store.customThemes[0].name == "Signal Room 2")
    }

    @Test("only the accent is persisted, so the derivation stays live")
    func storePersistence() {
        let suite = UUID().uuidString
        let defaults = UserDefaults(suiteName: "appthemetests.\(suite)")!
        let store = AppThemeStore(defaults: defaults)
        store.save(AppTheme.custom(id: "app-1", name: "Mine", accentHex: "3366FF"))

        let reloaded = AppThemeStore(defaults: defaults)
        #expect(reloaded.customThemes.count == 1)
        #expect(reloaded.customThemes[0].accentHex == "3366FF")
        // Derived values come back identical because they're recomputed, not stored.
        #expect(reloaded.customThemes[0].accentPressed.hexString
                == AppTheme.custom(id: "app-1", name: "Mine", accentHex: "3366FF").accentPressed.hexString)
    }

    @Test("a deleted or unknown id resolves to the default theme")
    func catalogFallback() {
        #expect(AppThemeCatalog.theme(for: "does-not-exist").id == AppThemeCatalog.signalRoom.id)
    }

    // MARK: Icons

    @Test("every icon option names bundled artwork, and only the primary has no alternate")
    func iconCatalogShape() {
        let all = AppIconCatalog.all
        #expect(all.count >= 6)
        #expect(all.filter(\.isPrimary).count == 1, "exactly one primary icon")
        #expect(all.first?.isPrimary == true)
        // Ids and asset names are unique — a duplicate would make the picker
        // ambiguous and the stored selection unresolvable.
        #expect(Set(all.map(\.id)).count == all.count)
        #expect(Set(all.map(\.previewAsset)).count == all.count)
        for option in all.dropFirst() {
            #expect(option.alternateName?.hasPrefix("AppIcon-") == true)
        }
    }

    @Test("pre-split installs migrate to the icon their app theme implied")
    func iconMigration() {
        // Before the split, picking a theme also swapped the icon; those users
        // must keep the icon already on their home screen.
        #expect(AppIconCatalog.migratedID(fromAppThemeID: "moshpit-classic") == "teal")
        #expect(AppIconCatalog.migratedID(fromAppThemeID: "terminal-green") == "green")
        #expect(AppIconCatalog.migratedID(fromAppThemeID: "amber-console") == "amber")
        #expect(AppIconCatalog.migratedID(fromAppThemeID: "signal-room") == "default")
        // A custom accent has no implied icon → the primary one.
        #expect(AppIconCatalog.migratedID(fromAppThemeID: "app-abc123") == "default")
    }

    @Test("an unknown icon id resolves to the primary icon")
    func iconFallback() {
        #expect(AppIconCatalog.option(for: "nope").id == AppIconCatalog.primary.id)
        #expect(AppIconCatalog.option(for: "hail").alternateName == "AppIcon-Hail")
    }
}
