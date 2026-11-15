import SwiftUI
import UIKit

/// The theme editor drawn by NSH (`ui/src/theme_editor.rs`) in the sheet's
/// own stack. The draft is the host's (``ThemeEditorModel``): the page
/// tells it every change and draws the preview from its own copy; Save hands
/// the draft to `onSave`, and Copy as JSON puts it
/// on the pasteboard as the store exports it. Each color well is UIKit's
/// (`moshpit/color-well`): a tap presents the system's color picker. If its
/// screen can't run, the page says so (``NSHPageFailed``).
struct ThemeEditorNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(ThemeStore.self) private var themes
    @State private var model: ThemeEditorModel
    @State private var failed = false
    private let onSave: (TerminalTheme) -> Void

    init(theme: TerminalTheme, onSave: @escaping (TerminalTheme) -> Void) {
        _model = State(initialValue: ThemeEditorModel(theme: theme))
        self.onSave = onSave
    }

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NavigationStack {
                NSHPage(
                    screen: .themeEditor,
                    attach: { model.attach($0) },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
            }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "cancel":
            dismiss()
        case "save":
            onSave(model.draft)
            dismiss()
        case "copy-json":
            if let json = try? themes.exportJSON(model.draft) { UIPasteboard.general.string = json }
        default:
            model.apply(message)
        }
    }
}

/// The theme the NSH editor edits: the shared model's editor
/// (`crates/app/src/looks.rs`'s `ThemeEditor`, through ``ThemeEditorHandle``,
/// the one Android runs), owned by the host so a surface made again starts
/// from it (``NSHPage``'s third rule); the message that carries it
/// (`{"op": "theme", …}`); and the page's changes.
@MainActor
final class ThemeEditorModel {
    private let editor: ThemeEditorHandle
    private weak var surface: NSHSurface?

    init(theme: TerminalTheme) {
        editor = ThemeEditorHandle(theme: ThemeRecord(theme))
    }

    /// The theme as edited so far.
    var draft: TerminalTheme { TerminalTheme(editor.draft()) }

    /// The bright rows show: from the start when the theme overrides one.
    var overrideBright: Bool { editor.overrideBright() }

    /// `surface` edits the draft from now on, starting from it as it is.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        surface.post(Self.message(draft, overrideBright: overrideBright))
    }

    /// Applies a change the page reports (`name`, `color`,
    /// `override-bright`). false for one the model does not read.
    @discardableResult
    func apply(_ message: [String: Any]) -> Bool {
        guard JSONSerialization.isValidJSONObject(message),
              let data = try? JSONSerialization.data(withJSONObject: message) else { return false }
        return editor.handle(messageJson: String(decoding: data, as: UTF8.self)) == .changed
    }

    /// The screen's message: the theme, its bright overrides (null where
    /// derived), and whether the bright rows show.
    nonisolated static func message(_ theme: TerminalTheme, overrideBright: Bool) -> [String: Any] {
        let json = themeEditorMessage(theme: ThemeRecord(theme), overrideBright: overrideBright)
        return (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}
