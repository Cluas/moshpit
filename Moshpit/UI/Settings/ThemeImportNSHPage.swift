import SwiftUI

// MARK: - Import sheet

/// Import JSON…'s sheet, drawn by NSH (`ui/src/theme_import.rs`), which
/// the NSH Theme page presents: the text is the page's; the host imports
/// it (`ThemeStore.importThemes`) and posts why it couldn't
/// (`{"op": "themeImport", "error"}`). If its screen can't run, the sheet
/// says so.
struct ThemeImportNSHPage: View {
    /// Why the last import didn't take.
    @Binding var error: String?
    /// Imports `text`; true when it took (the sheet closes).
    let onImport: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var surface: NSHSurface?
    @State private var failed = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NSHPage(
                screen: .themeImport,
                attach: { surface = $0; $0.post(json: Self.message(error: error)) },
                onMessage: handle,
                onFailure: { failed = true })
                .ignoresSafeArea(.all)
                .background(Ink.pageBG)
                .onChange(of: error) { _, new in surface?.post(json: Self.message(error: new)) }
        }
    }

    /// The page's `{"op": "themeImport", …}`.
    nonisolated static func message(error: String?) -> String {
        var fields: [String: Any] = ["op": "themeImport"]
        if let error { fields["error"] = error }
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "import":
            if let text = message["text"] as? String { onImport(text) }
        case "cancel":
            dismiss()
        default:
            break
        }
    }
}

#if DEBUG
/// Where a capture run has Settings › Theme scrolled to
/// (`-MOSHPIT_SEED_SCROLL custom`): the user's own themes, their first row
/// just under the bar (``ThemeGalleryNSHPage``'s reveal).
enum ThemeGalleryAnchor {
    static var requested: Bool {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_SEED_SCROLL"), i + 1 < args.count else { return false }
        return args[i + 1] == "custom"
    }
}
#endif

// MARK: - Optional-item sheet helper

extension View {
    /// `sheet(item:)` for a plain `Identifiable` optional binding — the same
    /// shape as `sheet(item:)` on newer SDKs, kept local so the call sites read
    /// the same on the deployment target.
    func sheet<Item: Identifiable, Content: View>(
        item: Binding<Item?>,
        @ViewBuilder content: @escaping (Item) -> Content
    ) -> some View {
        sheet(isPresented: Binding(
            get: { item.wrappedValue != nil },
            set: { if !$0 { item.wrappedValue = nil } }
        )) {
            if let value = item.wrappedValue {
                content(value)
            }
        }
    }
}
