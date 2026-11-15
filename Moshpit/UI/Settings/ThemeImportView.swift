import SwiftUI

// MARK: - Import sheet

/// Import JSON…'s sheet, which the NSH Theme page presents.
struct ThemeImportView: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var text: String
    @Binding var error: String?
    let onImport: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextEditor(text: $text)
                        .font(.system(.footnote, design: .monospaced))
                        .foregroundStyle(Ink.primary)
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: 240)
                        .accessibilityIdentifier("theme-import-editor")
                } header: {
                    SectionKicker(title: "THEME JSON")
                } footer: {
                    Text("Paste a theme exported from Moshpit, or any JSON object with \"background\", \"foreground\" and the eight ANSI color names (\"black\", \"red\", …). Bright colors are optional.")
                        .moshpitFooter()
                }
                .listRowBackground(Ink.terminalBG)

                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.circle.fill")
                            .font(.footnote)
                            .foregroundStyle(Ink.danger)
                    }
                    .listRowBackground(Color.clear)
                }

                Section {
                    Button {
                        if let pasted = UIPasteboard.general.string { text = pasted }
                    } label: {
                        Label("Paste from Clipboard", systemImage: "doc.on.clipboard")
                    }
                    .foregroundStyle(Ink.accent)
                    .moshpitRows()
                }
            }
            .moshpitForm()
            .navigationTitle("Import Theme")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import") { onImport() }
                        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
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
