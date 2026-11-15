import SwiftUI
import UIKit

/// The accent editor drawn by NSH (`ui/src/accent_editor.rs`) in the sheet's
/// own stack. The draft is the host's (``AccentEditorModel``): the page tells
/// it every change and draws the preview from its own copy; Save hands
/// `AppTheme.custom` of the draft to `onSave`. The
/// color well is UIKit's (`moshpit/color-well`). If its screen can't run,
/// the page says so (``NSHPageFailed``).
struct AccentEditorNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model: AccentEditorModel
    @State private var failed = false
    private let onSave: (AppTheme) -> Void

    init(theme: AppTheme, onSave: @escaping (AppTheme) -> Void) {
        _model = State(initialValue: AccentEditorModel(theme: theme))
        self.onSave = onSave
    }

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NavigationStack {
                NSHPage(
                    screen: .accentEditor,
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
            onSave(model.theme)
            dismiss()
        default:
            model.apply(message)
        }
    }
}

/// The accent the NSH editor edits: the shared model's editor
/// (`crates/app/src/looks.rs`'s `AccentEditor`, through
/// ``AccentEditorHandle``, the one Android runs), owned by the host so a
/// surface made again starts from it (``NSHPage``'s third rule); the
/// message that carries it (`{"op": "accent", …}`); and the page's changes.
@MainActor
final class AccentEditorModel {
    private let editor: AccentEditorHandle
    private weak var surface: NSHSurface?

    init(theme: AppTheme) {
        editor = AccentEditorHandle(accent: AccentThemeRecord(theme))
    }

    var name: String { editor.accent().name }
    var accentHex: String { editor.accent().accentHex }

    /// The draft, its pressed shade and background washes derived
    /// (``AppTheme/custom(id:name:accentHex:)``).
    var theme: AppTheme { AppTheme(editor.accent()) }

    /// `surface` edits the draft from now on, starting from it as it is.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        surface.post(Self.message(name: name, accentHex: accentHex))
    }

    /// Applies a change the page reports (`name`, `color`). false for one
    /// the model does not read.
    @discardableResult
    func apply(_ message: [String: Any]) -> Bool {
        guard JSONSerialization.isValidJSONObject(message),
              let data = try? JSONSerialization.data(withJSONObject: message) else { return false }
        return editor.handle(messageJson: String(decoding: data, as: UTF8.self)) == .changed
    }

    /// The screen's message.
    nonisolated static func message(name: String, accentHex: String) -> [String: Any] {
        let json = accentEditorMessage(name: name, accentHex: accentHex)
        return (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}
