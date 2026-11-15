import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// Settings › Theme, the theme editor, Accent and the accent editor in NSH
/// (R14): their switches, the data path between the host
/// and each screen — every state the Swift side posts is a message the Rust
/// screen takes (`ui/src/theme_gallery.rs`, `theme_editor.rs`,
/// `accent_editor.rs` and `accent.rs` test the screens, `ui/tests/` run
/// them), what the screens tell the host is applied as the SwiftUI pages
/// apply it — real surfaces on the device that show UIKit's color wells,
/// menus and switches with the posted values and report what the user does.
@Suite("NSH Theme and the editors")
@MainActor
struct ThemeNSHTests {

    /// Settings and stores of their own, so a test writes nothing the app
    /// keeps.
    private static func scratch() -> (AppSettings, ThemeStore, AppThemeStore) {
        let defaults = UserDefaults(suiteName: "w11d.nsh.\(UUID().uuidString)")!
        return (AppSettings(defaults: defaults), ThemeStore(defaults: defaults), AppThemeStore(defaults: defaults))
    }

    /// A custom theme with a bright override.
    private static func harbor(id: String = "custom-1a2b3c4d") -> TerminalTheme {
        TerminalTheme(id: id, name: "Harbor Night", background: "0B1420", foreground: "D6E2F0", cursor: "F2C14E",
                      ansi: ["1B2633", "F07178", "9BD67E", "F2C14E", "6CB6FF", "C39BFF", "5FD7D7", "E6EDF3"],
                      bright: ["7A7468", "", "", "", "", "", "", ""])
    }

    /// The text of the text field in or under `view` (NSH gives an
    /// input's identifier to its container).
    private static func text(in view: UIView) -> String? {
        if let field = view as? UITextField { return field.text }
        if let textView = view as? UITextView { return textView.text }
        for subview in view.subviews {
            if let text = text(in: subview) { return text }
        }
        return nil
    }

    /// `color` as the app stores a color.
    private static func hex(_ color: UIColor?) -> String? {
        guard let color else { return nil }
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        color.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        return Color.hexString(r: Double(red), g: Double(green), b: Double(blue))
    }

    // MARK: Screens

    @Test("each page runs the Rust screen of its name")
    func names() {
        let screens: [NSHScreen] = [.theme, .themeEditor, .accentEditor]
        #expect(screens.map(\.name) == ["theme", "theme-editor", "accent-editor"])
        #expect(SettingsPage(rawValue: "theme") == .theme, "the NSH Settings root's Theme row")
    }

    // MARK: Theme

    @Test("every Theme state is a message the Rust screen takes; bad ones are refused")
    func themeMessages() throws {
        let screen = NSHScreen.theme.name
        let mine = Self.harbor()
        for deleting in [nil, mine.id] {
            let state = ThemeGalleryModel.state(builtIns: TerminalTheme.builtIns, custom: [mine],
                                                selected: mine.id, deleting: deleting)
            #expect(state.deleting == deleting)
            #expect(NSHSurface.check(screen: screen, json: try pageMessage(message: .themeGallery(state: state))))
        }
        var palette = ThemeGalleryModel.palette(mine)
        #expect(palette.backgroundHex == "#0B1420")
        #expect(palette.ansiHex.count == 8)
        #expect(NSHSurface.check(screen: screen, json: try pageMessage(message: .themeGalleryReveal)))
        palette.ansiHex = ["#000000"]
        #expect(throws: PageError.self) {
            try pageMessage(message: .themeGallery(state: ThemeGalleryPageState(
                builtIn: [], custom: [palette], selected: mine.id, deleting: nil)))
        }
        #expect(!NSHSurface.check(screen: screen, message: ["op": "reveal", "anchor": "custom"]))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "accents", "selected": "x"]))
    }

    @Test("the card asks about a custom theme; Delete falls back to the default theme as the SwiftUI page does")
    func themeCard() {
        let (settings, themes, _) = Self.scratch()
        let mine = Self.harbor()
        themes.save(mine)
        settings.themeId = mine.id
        let model = ThemeGalleryModel()
        model.update(ThemeGallerySnapshot(themes: themes, settings: settings))
        #expect(model.snapshot?.selected == mine.id)
        model.confirmDelete(mine.id)
        #expect(model.deleting == mine.id)
        model.cancelDelete()
        #expect(model.deleting == nil)
        model.confirmDelete(mine.id)
        model.remove(mine.id, settings: settings, themes: themes)
        #expect(themes.customThemes.isEmpty)
        #expect(settings.themeId == TerminalTheme.fallback.id)
        #expect(model.deleting == nil)
        #expect(model.snapshot?.custom.isEmpty == true)
        // Deleting one that is not in effect leaves the setting alone; a
        // theme gone from the store takes its card with it.
        let other = Self.harbor(id: "custom-ffff0000")
        themes.save(other)
        settings.themeId = "dracula"
        model.update(ThemeGallerySnapshot(themes: themes, settings: settings))
        model.confirmDelete(other.id)
        themes.delete(id: other.id)
        model.update(ThemeGallerySnapshot(themes: themes, settings: settings))
        #expect(model.deleting == nil)
        #expect(settings.themeId == "dracula")
        // A stored id that names no theme ticks the default, as the terminal
        // falls back to it.
        settings.themeId = "deleted-long-ago"
        #expect(ThemeGallerySnapshot(themes: themes, settings: settings).selected == TerminalTheme.fallback.id)
    }

    @Test("a custom theme's menu is the SwiftUI page's; Copy JSON copies the store's export")
    func themeMenus() throws {
        var done: [String] = []
        let actions = ThemeGalleryModel.customActions(
            edit: { done.append("edit") }, duplicate: { done.append("duplicate") },
            copyJSON: { done.append("copy") }, delete: { done.append("delete") })
        #expect(actions.map(\.title) == [String(localized: "Edit"), String(localized: "Duplicate"),
                                         String(localized: "Copy JSON"), String(localized: "Delete")])
        #expect((actions.last as? UIAction)?.attributes.contains(.destructive) == true)
        #expect(ThemeGalleryModel.cellID("dracula") == "theme-cell-dracula")
        let (_, themes, _) = Self.scratch()
        let pasteboard = UIPasteboard.withUniqueName()
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        ThemeGalleryModel.copyJSON(Self.harbor(), themes: themes, pasteboard: pasteboard)
        #expect(pasteboard.string == (try themes.exportJSON(Self.harbor())))
    }

    // MARK: Theme editor

    @Test("every editor state is a message the Rust screen takes")
    func editorMessages() {
        let screen = NSHScreen.themeEditor.name
        let (_, themes, _) = Self.scratch()
        let draft = themes.makeDraft()
        #expect(NSHSurface.check(screen: screen, message: ThemeEditorModel.message(draft, overrideBright: false)))
        let message = ThemeEditorModel.message(Self.harbor(), overrideBright: true)
        #expect(NSHSurface.check(screen: screen, message: message), "\(message)")
        let bright = (message["theme"] as? [String: Any])?["bright"] as? [Any]
        // The shared model writes `#RRGGBBAA` (the screen reads either).
        #expect((bright?.first as? String)?.uppercased().hasPrefix("#7A7468") == true)
        #expect(bright?.dropFirst().allSatisfy { $0 is NSNull } == true, "derived slots are null")
        var bad = message
        bad.removeValue(forKey: "overrideBright")
        #expect(!NSHSurface.check(screen: screen, message: bad))
    }

    @Test("the page's changes apply to the draft as the SwiftUI page's bindings do")
    func editorApply() {
        #expect(ThemeEditorModel(theme: Self.harbor()).overrideBright, "an override opens the bright rows")
        var plain = Self.harbor()
        plain.setBrightOverride(nil, for: .black)
        let model = ThemeEditorModel(theme: plain)
        #expect(!model.overrideBright)
        #expect(model.apply(["ev": "name", "value": "Dusk"]))
        #expect(model.draft.name == "Dusk")
        #expect(model.apply(["ev": "color", "slot": "background", "value": "#112233"]))
        #expect(model.draft.backgroundHex == "112233")
        #expect(model.apply(["ev": "color", "slot": "cursor", "value": "#abcdef"]))
        #expect(model.draft.cursorHex == "ABCDEF")
        #expect(model.apply(["ev": "color", "slot": "red", "value": "#FF0000"]))
        #expect(model.draft.ansiHex(.red) == "FF0000")
        #expect(model.apply(["ev": "override-bright", "value": true]))
        #expect(model.apply(["ev": "color", "slot": "brightBlack", "value": "#7A7468"]))
        #expect(model.draft.hasBrightOverride(.black))
        #expect(model.draft.brightHex(.black) == "7A7468")
        #expect(model.apply(["ev": "override-bright", "value": false]))
        #expect(!model.overrideBright)
        #expect(!model.draft.hasBrightOverride(.black), "off drops the overrides")
        #expect(model.draft.brightHex == nil)
        #expect(!model.apply(["ev": "color", "slot": "brightness", "value": "#000000"]))
        #expect(!model.apply(["ev": "color", "slot": "red"]))
        #expect(!model.apply(["ev": "save"]), "the page's own, not a change")
    }

    // MARK: Accent editor and card

    @Test("the accent editor's state is a message the Rust screen takes, and its changes apply")
    func accentEditor() {
        let screen = NSHScreen.accentEditor.name
        let theme = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        let model = AccentEditorModel(theme: theme)
        #expect(NSHSurface.check(screen: screen, message: AccentEditorModel.message(name: "Mint", accentHex: "2FD8A8")))
        #expect(NSHSurface.check(screen: screen, message: AccentEditorModel.message(name: "", accentHex: "2FD8A8")))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "accent", "name": "Mint", "accent": "mint"]))
        #expect(model.apply(["ev": "color", "value": "#ff8800"]))
        #expect(model.apply(["ev": "name", "value": "Ember"]))
        #expect(!model.apply(["ev": "color"]))
        #expect(model.theme == AppTheme.custom(id: "app-1a2b3c4d", name: "Ember", accentHex: "FF8800"))
    }

    @Test("the Accent page's card: asked, cancelled, answered")
    func accentCard() throws {
        let (settings, _, store) = Self.scratch()
        let mine = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        store.save(mine)
        let model = AccentModel()
        model.confirmDelete(mine.id, settings: settings, store: store)
        #expect(model.deleting == mine.id)
        let state = AccentModel.state(builtIns: AppThemeCatalog.builtIns, custom: store.customThemes,
                                      selected: "signal-room", deleting: model.deleting)
        #expect(NSHSurface.check(screen: NSHScreen.accent.name, json: try pageMessage(message: .accent(state: state))))
        model.cancelDelete(settings: settings, store: store)
        #expect(model.deleting == nil)
        model.confirmDelete(mine.id, settings: settings, store: store)
        model.remove(mine, settings: settings, store: store)
        #expect(model.deleting == nil)
        #expect(store.customThemes.isEmpty)
    }

    // MARK: On the device

    @Test("Theme on a real surface: the posted rows, UIKit's add menu, the delete question in the system's alert")
    func themeSurface() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .theme)
        defer { done() }
        let mine = Self.harbor()
        #expect(surface.post(.themeGallery(state: ThemeGalleryModel.state(
            builtIns: TerminalTheme.builtIns, custom: [mine], selected: "dracula", deleting: nil))))
        let dracula = try #require(await SettingsNSHTests.view(identified: "theme-row-dracula", in: surface))
        #expect(dracula.accessibilityTraits.contains(.selected))
        let add = try #require(await SettingsNSHTests.view(identified: "theme-add", in: surface) as? UIButton)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(add.showsMenuAsPrimaryAction)
        #expect(add.menu?.children.map(\.title) == [String(localized: "New…"), String(localized: "Import…")])
        #expect(add.menu?.children.allSatisfy { ($0 as? UIAction)?.image != nil } == true)
        #expect(add.accessibilityLabel == String(localized: "Add theme"))
        #expect(dracula.accessibilityActivate())
        let select = try #require(await heard.next())
        #expect(select["ev"] as? String == "select")
        #expect(select["id"] as? String == "dracula")
        // The long-press menus find each row by its part.
        let cell = try #require(await SettingsNSHTests.view(identified: ThemeGalleryModel.cellID(mine.id), in: surface))
        let menus = NSHContextMenus()
        menus.menus = [ThemeGalleryModel.cellID(mine.id): { [UIAction(title: "Edit") { _ in }] }]
        menus.install(on: surface)
        let interaction = try #require(surface.interactions.compactMap { $0 as? UIContextMenuInteraction }.first)
        let inCell = surface.convert(CGPoint(x: cell.bounds.midX, y: cell.bounds.midY), from: cell)
        let configuration = menus.contextMenuInteraction(interaction, configurationForMenuAtLocation: inCell)
        #expect(configuration?.identifier as? String == ThemeGalleryModel.cellID(mine.id))
        // The question the host asks for: the system's alert, Delete then
        // Cancel, Delete destructive (UI-DESIGN §4.10).
        let presenter = try Self.presenter(for: surface)
        #expect(surface.post(.themeGallery(state: ThemeGalleryModel.state(
            builtIns: TerminalTheme.builtIns, custom: [mine], selected: "dracula", deleting: mine.id))))
        let alert = try #require(await Self.alert(over: presenter))
        #expect(alert.title == String(format: String(localized: "Delete %@?"), mine.name))
        #expect(alert.actions.map(\.title) == [String(localized: "Delete"), String(localized: "Cancel")])
        #expect(alert.actions.first?.style == .destructive)
        presenter.dismiss(animated: false)
    }

    /// A view controller to present over in `surface`'s window, which the
    /// test made without one (NSH shows its alerts over the window's top
    /// controller).
    static func presenter(for surface: UIView) throws -> UIViewController {
        let window = try #require(surface.window)
        let presenter = UIViewController()
        presenter.view.isUserInteractionEnabled = false
        presenter.view.backgroundColor = .clear
        window.rootViewController = presenter
        window.bringSubviewToFront(surface)
        return presenter
    }

    /// The alert presented over `presenter`, waiting up to
    /// ``NSHTestWait/ceiling`` for it.
    static func alert(over presenter: UIViewController) async -> UIAlertController? {
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while Date() < deadline {
            if let alert = presenter.presentedViewController as? UIAlertController { return alert }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return nil
    }

    @Test("the theme editor on a real surface: UIKit's color wells carry the draft, picks and Save reach the host")
    func themeEditorSurface() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .themeEditor)
        defer { done() }
        var draft = Self.harbor()
        draft.setBrightOverride(nil, for: .black)
        #expect(surface.post(ThemeEditorModel.message(draft, overrideBright: false)))
        let background = try #require(await SettingsNSHTests.view(identified: "theme-color-background", in: surface)
            as? UIColorWell)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(Self.hex(background.selectedColor) == "0B1420")
        #expect(!background.supportsAlpha)
        background.selectedColor = UIColor(red: 1, green: 0, blue: 0, alpha: 1)
        background.sendActions(for: .valueChanged)
        let color = try #require(await heard.next())
        #expect(color["ev"] as? String == "color")
        #expect(color["slot"] as? String == "background")
        #expect(color["value"] as? String == "#FF0000")
        let name = try #require(await SettingsNSHTests.view(identified: "theme-editor-name", in: surface))
        #expect(Self.text(in: name) == "Harbor Night")
        let bright = try #require(await SettingsNSHTests.view(identified: "theme-editor-bright", in: surface)
            as? UISwitch)
        #expect(!bright.isOn)
        bright.setOn(true, animated: false)
        bright.sendActions(for: .valueChanged)
        let toggled = try #require(await heard.next())
        #expect(toggled["ev"] as? String == "override-bright")
        #expect(toggled["value"] as? Bool == true)
        let brightBlack = try #require(await SettingsNSHTests.view(identified: "theme-color-brightBlack", in: surface)
            as? UIColorWell)
        #expect(Self.hex(brightBlack.selectedColor) == TerminalTheme.derivedBright(from: "1B2633"))
        let more = try #require(await SettingsNSHTests.view(identified: "theme-editor-more", in: surface) as? UIButton)
        #expect(more.menu?.children.map(\.title) == [String(localized: "Copy as JSON")])
        let save = try #require(await SettingsNSHTests.view(identified: "theme-editor-save", in: surface))
        #expect(save.accessibilityActivate())
        #expect(try #require(await heard.next())["ev"] as? String == "save")
    }

    @Test("the accent editor on a real surface: the well carries the accent, a pick reaches the host")
    func accentEditorSurface() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .accentEditor)
        defer { done() }
        #expect(surface.post(AccentEditorModel.message(name: "Mint", accentHex: "2FD8A8")))
        let well = try #require(await SettingsNSHTests.view(identified: "accent-editor-color", in: surface)
            as? UIColorWell)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(Self.hex(well.selectedColor) == "2FD8A8")
        well.selectedColor = UIColor(red: 1, green: 0.5333, blue: 0, alpha: 1)
        well.sendActions(for: .valueChanged)
        let color = try #require(await heard.next())
        #expect(color["ev"] as? String == "color")
        #expect(color["value"] as? String == "#FF8800")
        let name = try #require(await SettingsNSHTests.view(identified: "accent-editor-name", in: surface))
        #expect(Self.text(in: name) == "Mint")
        let cancel = try #require(await SettingsNSHTests.view(identified: "accent-editor-cancel", in: surface))
        #expect(cancel.accessibilityActivate())
        #expect(try #require(await heard.next())["ev"] as? String == "cancel")
    }

    @Test("Accent on a real surface: the question the host asks for is the system's alert")
    func accentCardSurface() async throws {
        let (surface, _, done) = try SettingsNSHTests.surface(running: .accent)
        defer { done() }
        let mine = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        let presenter = try Self.presenter(for: surface)
        #expect(surface.post(.accent(state: AccentModel.state(
            builtIns: AppThemeCatalog.builtIns, custom: [mine], selected: "signal-room", deleting: mine.id))))
        let alert = try #require(await Self.alert(over: presenter))
        #expect(alert.title == String(format: String(localized: "Delete %@?"), "Mint"))
        #expect(alert.actions.map(\.title) == [String(localized: "Delete"), String(localized: "Cancel")])
        presenter.dismiss(animated: false)
    }

    // MARK: Pictures (R14)

    /// R14's pages at the owner's iPhone's size (402 × 874 pt), dark and
    /// light, each drawn to the app's tmp folder as
    /// `ui-gallery-r14-<page>-<dark|light>.png`: the pictures R14 is checked
    /// by (`ui/tests/` check the pages headless).
    @Test("R14's pages drawn dark and light at the iPhone's size")
    func pictures() async throws {
        let mine = Self.harbor()
        let mint = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        let theme = PageMessage.themeGallery(state: ThemeGalleryModel.state(
            builtIns: TerminalTheme.builtIns, custom: [mine], selected: "dracula", deleting: nil))
        let accent = PageMessage.accent(state: AccentModel.state(
            builtIns: AppThemeCatalog.builtIns, custom: [mint], selected: "signal-room", deleting: nil))
        let pages: [(NSHScreen, String, (NSHSurface) -> Bool, String)] = [
            (.theme, "theme", { $0.post(theme) }, "theme-row-dracula"),
            (.themeEditor, "theme-editor", { $0.post(ThemeEditorModel.message(mine, overrideBright: false)) },
             "theme-editor-name"),
            (.accent, "accent", { $0.post(accent) }, "accent-row-signal-room"),
            (.accentEditor, "accent-editor",
             { $0.post(AccentEditorModel.message(name: "Mint", accentHex: "2FD8A8")) }, "accent-editor-name"),
            (.appIcon, "app-icon",
             { $0.post(AppIconModel.message(selected: AppIconCatalog.all.first?.id ?? "", failure: nil)) },
             "app-icon-grid"),
        ]
        for (screen, name, post, ready) in pages {
            for light in [false, true] {
                let look = light ? "light" : "dark"
                try await Self.picture(screen, ready: ready, light: light, name: "r14-\(name)-\(look)", post: post)
            }
        }
    }

    /// `screen` showing `message` at the iPhone's size, `light` or dark,
    /// drawn once `ready` shows and the page has settled.
    static func picture(_ screen: NSHScreen, _ message: [String: Any], ready: String, light: Bool,
                        name: String) async throws {
        try await picture(screen, ready: ready, light: light, name: name) { $0.post(message) }
    }

    /// ``picture(_:_:ready:light:name:)`` of what `post` posts.
    static func picture(_ screen: NSHScreen, ready: String, light: Bool, name: String,
                        post: (NSHSurface) -> Bool) async throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: NavBarNSHTests.phone))
        window.addSubview(surface)
        window.isHidden = false
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        #expect(surface.run(screen: screen.name, params: UIGallerySurface.params(light: light)), "\(name)")
        #expect(post(surface), "\(name)")
        #expect(await SettingsNSHTests.view(identified: ready, in: surface) != nil, "\(name): \(ready)")
        try? await Task.sleep(for: .milliseconds(600))
        UIGalleryNSHTests.shoot(surface, name)
    }
}
