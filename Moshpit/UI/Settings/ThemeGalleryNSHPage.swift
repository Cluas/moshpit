import SwiftUI
import UIKit

/// Settings › Theme drawn by NSH (`ui/src/theme_gallery.rs`): the built-in
/// and the user's own terminal themes, from the host (``ThemeGalleryModel``).
/// A pick applies at once; New Theme, Duplicate and a custom theme's edit
/// button open the editor sheet (``ThemeEditorNSHPage``); Import JSON…
/// opens the import sheet (``ThemeImportView``); each row has a long-press
/// menu (UIKit's, on the NSH surface: ``NSHContextMenus``); Delete asks with
/// the card the NSH page draws, for the theme the model names.
struct ThemeGalleryNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @Environment(ThemeStore.self) private var themes
    @State private var model = ThemeGalleryModel()
    @State private var menus = NSHContextMenus()
    @State private var failed = false
    @State private var editing: TerminalTheme?
    @State private var showImport = false
    @State private var importText = ""
    @State private var importError: String?

    var body: some View {
        if failed {
            NSHPageFailed { failed = false }
        } else {
            let snapshot = ThemeGallerySnapshot(themes: themes, settings: settings)
            NSHPage(
                screen: .theme,
                attach: { surface in
                    model.attach(surface, current: snapshot)
                    menus.install(on: surface)
                    installMenus(for: snapshot)
                    #if DEBUG
                    seedCapture(snapshot)
                    #endif
                },
                onMessage: handle,
                onFailure: { failed = true })
                .nshPage()
                // The title the next page's back button would show.
                .navigationTitle("Theme")
                .onAppear { model.update(snapshot) }
                .onChange(of: snapshot) { _, new in
                    model.update(new)
                    installMenus(for: new)
                }
                // `item:`, so the sheet opens on the theme asked for.
                .appSheet(item: $editing) { theme in
                    ThemeEditorNSHPage(theme: theme) { saved in
                        themes.save(saved)
                        settings.themeId = saved.id      // editing implies wanting it
                    }
                }
                .appSheet(isPresented: $showImport) {
                    ThemeImportView(text: $importText, error: $importError) { runImport() }
                }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        let id = message["id"] as? String
        switch message["ev"] as? String {
        case "back":
            dismiss()
        case "select":
            if let id { settings.themeId = id }
        case "edit":
            if let id, let theme = themes.customThemes.first(where: { $0.id == id }) { editing = theme }
        case "new":
            editing = themes.makeDraft()
        case "import":
            importText = ""
            importError = nil
            showImport = true
        case "delete":
            if let id { model.remove(id, settings: settings, themes: themes) }
        case "cancel-delete":
            model.cancelDelete()
        default:
            break
        }
    }

    /// Each row's long-press menu:
    /// Duplicate & Edit on a built-in theme; Edit, Duplicate, Copy JSON and
    /// Delete on the user's own. The menu finds the theme as it is when it
    /// opens.
    private func installMenus(for snapshot: ThemeGallerySnapshot) {
        var parts: [String: () -> [UIMenuElement]] = [:]
        for theme in TerminalTheme.builtIns {
            parts[ThemeGalleryModel.cellID(theme.id)] = {
                [UIAction(title: String(localized: "Duplicate & Edit"),
                          image: UIImage(systemName: "plus.square.on.square")) { _ in
                    editing = themes.duplicate(theme)
                }]
            }
        }
        for theme in snapshot.custom {
            let id = theme.id
            parts[ThemeGalleryModel.cellID(id)] = {
                guard let theme = themes.customThemes.first(where: { $0.id == id }) else { return [] }
                return ThemeGalleryModel.customActions(
                    edit: { editing = theme },
                    duplicate: { editing = themes.duplicate(theme) },
                    copyJSON: { ThemeGalleryModel.copyJSON(theme, themes: themes) },
                    delete: { model.confirmDelete(id) })
            }
        }
        menus.menus = parts
        menus.cornerRadius = Dictionary(uniqueKeysWithValues: parts.keys.map { ($0, 10) })
    }

    /// The SwiftUI page's import, the first theme imported picked.
    private func runImport() {
        do {
            let imported = try themes.importThemes(json: importText)
            if let first = imported.first { settings.themeId = first.id }
            showImport = false
            importError = nil
        } catch {
            importError = error.localizedDescription
        }
    }

    #if DEBUG
    /// A capture run's editor or delete card, over the first custom theme,
    /// and its scroll (``ThemeGalleryAnchor``): once the page is up, and
    /// again once the text has settled, so the shot finds the rows where
    /// the final layout has them.
    private func seedCapture(_ snapshot: ThemeGallerySnapshot) {
        if ThemeGalleryAnchor.requested {
            for delay in [0.9, 2.0] {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { model.reveal() }
            }
        }
        if CaptureScreen.deleteRequested == true, let first = snapshot.custom.first {
            model.confirmDelete(first.id)
        }
        if CaptureScreen.editorRequested == true {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                editing = themes.customThemes.first ?? themes.makeDraft()
            }
        }
    }
    #endif
}

/// What the NSH Theme page shows: the user's themes and the one in effect.
struct ThemeGallerySnapshot: Equatable {
    var custom: [TerminalTheme]
    /// The theme in effect: a stored id that names no theme resolves to the
    /// default, as the terminal does.
    var selected: String

    @MainActor
    init(themes: ThemeStore, settings: AppSettings) {
        custom = themes.customThemes
        selected = themes.theme(id: settings.themeId).id
    }

    init(custom: [TerminalTheme], selected: String) {
        self.custom = custom
        self.selected = selected
    }
}

/// What the NSH Theme page shows, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule): the themes, the one in effect
/// and the theme the delete card asks about; the message that carries it
/// (`{"op": "themes", …}`, `ui/src/theme_gallery.rs`'s `Message`); and what
/// the page's intents do.
@MainActor
final class ThemeGalleryModel {
    /// The custom theme the delete card asks about, while it shows.
    private(set) var deleting: String?
    private(set) var snapshot: ThemeGallerySnapshot?
    private weak var surface: NSHSurface?

    /// `surface` shows the themes from now on, starting with the current
    /// ones.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        post()
    }

    /// ``attach(_:)``, the model filled from `current` (the page's themes as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current snapshot: ThemeGallerySnapshot) {
        if self.snapshot == nil {
            self.surface = nil
            update(snapshot)
        }
        attach(surface)
    }

    /// The themes as they are now. A card asking about a theme that is gone
    /// goes with it.
    func update(_ snapshot: ThemeGallerySnapshot) {
        guard snapshot != self.snapshot else { return }
        self.snapshot = snapshot
        if let deleting, !snapshot.custom.contains(where: { $0.id == deleting }) {
            self.deleting = nil
        }
        post()
    }

    /// Delete was picked in custom theme `id`'s menu: the card asks.
    func confirmDelete(_ id: String) {
        deleting = id
        post()
    }

    /// The card's Cancel.
    func cancelDelete() {
        guard deleting != nil else { return }
        deleting = nil
        post()
    }

    /// The card's Delete: removes the theme, the selection
    /// moving to the default when it was the one deleted.
    func remove(_ id: String, settings: AppSettings, themes: ThemeStore) {
        deleting = nil
        let wasSelected = settings.themeId == id
        themes.delete(id: id)
        if wasSelected { settings.themeId = TerminalTheme.fallback.id }
        snapshot = ThemeGallerySnapshot(themes: themes, settings: settings)
        post()
    }

    /// Scrolls the user's own themes to the top (a capture run's).
    func reveal() {
        surface?.post(["op": "reveal"])
    }

    private func post() {
        guard let snapshot else { return }
        surface?.post(Self.message(builtIns: TerminalTheme.builtIns, custom: snapshot.custom,
                                   selected: snapshot.selected, deleting: deleting))
    }

    /// The screen's message.
    nonisolated static func message(builtIns: [TerminalTheme], custom: [TerminalTheme], selected: String,
                                    deleting: String?) -> [String: Any] {
        var message: [String: Any] = [
            "op": "themes",
            "builtIn": builtIns.map(palette),
            "custom": custom.map(palette),
            "selected": selected,
        ]
        if let deleting { message["deleting"] = deleting }
        return message
    }

    /// One theme as the screen takes it: its name and the colors its row
    /// previews.
    nonisolated static func palette(_ theme: TerminalTheme) -> [String: Any] {
        [
            "id": theme.id,
            "name": theme.name,
            "background": "#" + theme.backgroundHex,
            "foreground": "#" + theme.foregroundHex,
            "cursor": "#" + theme.cursorHex,
            "ansi": TerminalTheme.ANSISlot.allCases.map { "#" + theme.ansiHex($0) },
        ]
    }

    /// The accessibility identifier of theme `id`'s row on the NSH screen
    /// (its long-press menu's part).
    nonisolated static func cellID(_ id: String) -> String { "theme-cell-\(id)" }

    /// A custom theme's menu.
    static func customActions(edit: @escaping () -> Void, duplicate: @escaping () -> Void,
                              copyJSON: @escaping () -> Void, delete: @escaping () -> Void) -> [UIMenuElement] {
        [
            UIAction(title: String(localized: "Edit"), image: UIImage(systemName: "slider.horizontal.3")) { _ in
                edit()
            },
            UIAction(title: String(localized: "Duplicate"), image: UIImage(systemName: "plus.square.on.square")) { _ in
                duplicate()
            },
            UIAction(title: String(localized: "Copy JSON"), image: UIImage(systemName: "doc.on.doc")) { _ in
                copyJSON()
            },
            UIAction(title: String(localized: "Delete"), image: UIImage(systemName: "trash"),
                     attributes: .destructive) { _ in delete() },
        ]
    }

    /// Copy JSON: the theme as the store exports it, on the pasteboard.
    static func copyJSON(_ theme: TerminalTheme, themes: ThemeStore,
                         pasteboard: UIPasteboard = .general) {
        guard let json = try? themes.exportJSON(theme) else { return }
        pasteboard.string = json
        Haptics.tap()
    }
}
