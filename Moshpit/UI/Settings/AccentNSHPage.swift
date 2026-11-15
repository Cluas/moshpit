import SwiftUI
import UIKit

/// Settings › Accent drawn by NSH (`ui/src/accent.rs`): the built-in and the
/// user's own accents, from the host (``AccentModel``). A pick is ticked at
/// once and applied as the page goes away (``AccentModel/commit(_:)``,
/// ``AccentModel/pendingID`` says why); New and a custom accent's edit
/// button open the editor sheet
/// (``AccentEditorNSHPage``); a custom accent's
/// row has the same context menu (UIKit's, on the NSH surface:
/// ``AccentRowMenus``); Delete asks with the card the NSH page draws, for
/// the accent the model names.
struct AccentNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @Environment(AppThemeStore.self) private var store
    @State private var model = AccentModel()
    @State private var menus = AccentRowMenus()
    @State private var failed = false
    @State private var editing: AppTheme?

    var body: some View {
        if failed {
            NSHPageFailed { failed = false }
        } else {
            NSHPage(
                screen: .accent,
                attach: { surface in
                    model.attach(surface, settings: settings, store: store)
                    model.commitOnLeaving(settings)
                    menus.editable = { [store] in store.customThemes }
                    menus.onEdit = { editing = $0 }
                    menus.onDelete = { model.confirmDelete($0.id, settings: settings, store: store) }
                    menus.install(on: surface)
                    #if DEBUG
                    seedCapture()
                    #endif
                },
                onEvent: { if case .accent(let event) = $0 { handle(event) } },
                onFailure: { failed = true })
                .nshPage()
                // The title the next page's back button would show.
                .navigationTitle("Accent")
                .onDisappear { model.commit(settings) }
                .appSheet(item: $editing) { theme in
                    AccentEditorNSHPage(theme: theme) { saved in
                        // Persisted at once (the user pressed Save), picked
                        // like any other accent.
                        store.save(saved)
                        model.pick(saved.id, settings: settings, store: store)
                    }
                }
        }
    }

    /// What the screen tells the host.
    private func handle(_ event: AccentEvent) {
        switch event {
        case .back:
            dismiss()
        case .select(let id):
            model.select(id, settings: settings, store: store)
        case .add:
            editing = store.makeDraft(basedOn: AppThemeCatalog.theme(for: settings.appThemeId))
        case .delete(let id):
            if let theme = store.customThemes.first(where: { $0.id == id }) {
                model.remove(theme, settings: settings, store: store)
            } else {
                model.cancelDelete(settings: settings, store: store)
            }
        case .cancelDelete:
            model.cancelDelete(settings: settings, store: store)
        }
    }

    #if DEBUG
    /// A capture run's editor or delete card, over the first custom accent.
    private func seedCapture() {
        if CaptureScreen.deleteRequested == false, let first = store.customThemes.first {
            model.confirmDelete(first.id, settings: settings, store: store)
        }
        if CaptureScreen.editorRequested == false {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                editing = store.customThemes.first
                    ?? store.makeDraft(basedOn: AppThemeCatalog.theme(for: settings.appThemeId))
            }
        }
    }
    #endif
}

/// What the NSH Accent page shows, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule), and the state that carries it
/// (``AccentPageState``, the screen's own through the page bridge).
@MainActor
final class AccentModel {
    /// The accent picked here and not yet applied: ticked at once, written to
    /// the settings as the page goes away. Writing `appThemeId` at once would
    /// recolor the app at once, but the root view rebuilds its whole subtree
    /// on that change (`Ink`'s tokens are static computed properties that
    /// Observation can't track), and the rebuild takes this pushed page down
    /// with it: a tap bounced the user back to Settings, so comparing two
    /// accents meant navigating back each time.
    private(set) var pendingID: String?
    /// The custom accent the delete card asks about, while it shows.
    private(set) var deleting: String?
    private weak var surface: NSHSurface?

    /// The accent to tick: the pending pick, else the one in effect (a stored
    /// id can outlive its theme, so it is resolved, as
    /// `AppThemeCatalog.theme(for:)` does, among `store`'s accents).
    func selectedID(_ settings: AppSettings, store: AppThemeStore) -> String {
        if let pendingID { return pendingID }
        let id = settings.appThemeId
        let known = AppThemeCatalog.builtIns.contains { $0.id == id } || store.customThemes.contains { $0.id == id }
        return known ? id : AppThemeCatalog.signalRoom.id
    }

    /// `surface` shows the accents from now on, starting with the current
    /// state.
    func attach(_ surface: NSHSurface, settings: AppSettings, store: AppThemeStore) {
        self.surface = surface
        post(settings: settings, store: store)
    }

    /// The user picked accent `id`.
    func select(_ id: String, settings: AppSettings, store: AppThemeStore) {
        guard id != selectedID(settings, store: store) else { return }
        pick(id, settings: settings, store: store)
    }

    /// Accent `id` is the pick (also a custom accent just saved).
    func pick(_ id: String, settings: AppSettings, store: AppThemeStore) {
        pendingID = id
        post(settings: settings, store: store)
    }

    /// The observer that commits the pick as the app leaves (B609), and
    /// where it is registered.
    nonisolated(unsafe) private var leaving: (NSObjectProtocol, NotificationCenter)?

    deinit {
        if let leaving { leaving.1.removeObserver(leaving.0) }
    }

    /// The pick applies when the app goes to the background too (B609): it
    /// waited for the page to go away, which a phone that ends the app in
    /// the background never lets happen — the accent picked was the old one
    /// at the next launch. Registered once.
    func commitOnLeaving(_ settings: AppSettings, center: NotificationCenter = .default) {
        guard leaving == nil else { return }
        let observer = center.addObserver(forName: UIApplication.didEnterBackgroundNotification,
                                          object: nil, queue: nil) { [weak self, weak settings] _ in
            // Posted on the main thread.
            MainActor.assumeIsolated {
                if let settings { self?.commit(settings) }
            }
        }
        leaving = (observer, center)
    }

    /// The page goes away: the pick applies.
    func commit(_ settings: AppSettings) {
        guard let pendingID, pendingID != settings.appThemeId else { return }
        withAnimation(.easeInOut(duration: 0.28)) {
            settings.appThemeId = pendingID
        }
    }

    /// Delete was picked in custom accent `id`'s menu: the card asks.
    func confirmDelete(_ id: String, settings: AppSettings, store: AppThemeStore) {
        deleting = id
        post(settings: settings, store: store)
    }

    /// The card's Cancel.
    func cancelDelete(settings: AppSettings, store: AppThemeStore) {
        guard deleting != nil else { return }
        deleting = nil
        post(settings: settings, store: store)
    }

    /// Deletes custom accent `theme` (the card's Delete); one that was
    /// picked or in effect gives way to Signal Room.
    func remove(_ theme: AppTheme, settings: AppSettings, store: AppThemeStore) {
        deleting = nil
        let wasSelected = selectedID(settings, store: store) == theme.id
        store.delete(id: theme.id)
        if wasSelected {
            pendingID = AppThemeCatalog.signalRoom.id
            if settings.appThemeId == theme.id {
                settings.appThemeId = AppThemeCatalog.signalRoom.id
            }
        }
        post(settings: settings, store: store)
    }

    private func post(settings: AppSettings, store: AppThemeStore) {
        surface?.post(.accent(state: Self.state(
            builtIns: AppThemeCatalog.builtIns, custom: store.customThemes,
            selected: selectedID(settings, store: store), deleting: deleting)))
    }

    /// What the page shows: the built-in accents, the user's own (newest
    /// last), the one to tick, and the one the delete card asks about.
    nonisolated static func state(builtIns: [AppTheme], custom: [AppTheme], selected: String,
                                  deleting: String? = nil) -> AccentPageState {
        AccentPageState(builtIn: builtIns.map(accent), custom: custom.map(accent),
                        selected: selected, deleting: deleting)
    }

    /// One accent as the screen takes it: its name and the two colors its
    /// row's swatch shows.
    nonisolated static func accent(_ theme: AppTheme) -> AccentEntry {
        AccentEntry(id: theme.id, name: theme.name, accentHex: "#" + theme.accentHex,
                    screenBgHex: NSHScreenParams.hex(theme.screenBG))
    }
}

/// The context menu of a custom accent's row (Edit, Delete), as the SwiftUI
/// page's `.contextMenu`: UIKit's own, on the NSH surface, for the row under
/// the finger. The screen marks each custom accent's row `accent-cell-<id>`
/// (its view's accessibility identifier); the menu lifts that row.
@MainActor
final class AccentRowMenus: NSObject, UIContextMenuInteractionDelegate {
    /// The accents whose rows have the menu: the user's own.
    var editable: () -> [AppTheme] = { [] }
    var onEdit: (AppTheme) -> Void = { _ in }
    var onDelete: (AppTheme) -> Void = { _ in }
    private weak var surface: UIView?
    private var interaction: UIContextMenuInteraction?

    /// Puts the menu on `surface` (again: a new surface replaces the old).
    func install(on surface: UIView) {
        if let interaction { interaction.view?.removeInteraction(interaction) }
        let interaction = UIContextMenuInteraction(delegate: self)
        surface.addInteraction(interaction)
        self.interaction = interaction
        self.surface = surface
    }

    /// The custom accent whose row is at `point` (in the surface), and that
    /// row's view.
    func row(at point: CGPoint) -> (theme: AppTheme, view: UIView)? {
        guard let surface else { return nil }
        for theme in editable() {
            guard let view = Self.view(identified: Self.cellID(theme.id), in: surface), view.window != nil else {
                continue
            }
            if view.bounds.contains(view.convert(point, from: surface)) {
                return (theme, view)
            }
        }
        return nil
    }

    /// The menu's actions for `theme`.
    func actions(for theme: AppTheme) -> [UIAction] {
        [
            UIAction(title: String(localized: "Edit"), image: UIImage(systemName: "slider.horizontal.3")) {
                [weak self] _ in self?.onEdit(theme)
            },
            UIAction(title: String(localized: "Delete"), image: UIImage(systemName: "trash"),
                     attributes: .destructive) { [weak self] _ in self?.onDelete(theme) },
        ]
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        guard let (theme, _) = row(at: location) else { return nil }
        let actions = actions(for: theme)
        return UIContextMenuConfiguration(identifier: theme.id as NSString, previewProvider: nil) { _ in
            UIMenu(children: actions)
        }
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configuration: UIContextMenuConfiguration,
                                highlightPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
        preview(for: configuration)
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configuration: UIContextMenuConfiguration,
                                dismissalPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
        preview(for: configuration)
    }

    /// The row the menu is for, lifted as a list's row is: its own rounded
    /// shape.
    private func preview(for configuration: UIContextMenuConfiguration) -> UITargetedPreview? {
        guard let id = configuration.identifier as? String, let surface,
              let view = Self.view(identified: Self.cellID(id), in: surface), view.window != nil
        else { return nil }
        let parameters = UIPreviewParameters()
        parameters.visiblePath = UIBezierPath(roundedRect: view.bounds, cornerRadius: 10)
        return UITargetedPreview(view: view, parameters: parameters)
    }

    /// The accessibility identifier of accent `id`'s row on the NSH screen.
    nonisolated static func cellID(_ id: String) -> String { "accent-cell-\(id)" }

    /// The view under `root` with accessibility identifier `id`.
    static func view(identified id: String, in root: UIView) -> UIView? {
        if root.accessibilityIdentifier == id { return root }
        for child in root.subviews {
            if let hit = view(identified: id, in: child) { return hit }
        }
        return nil
    }
}
