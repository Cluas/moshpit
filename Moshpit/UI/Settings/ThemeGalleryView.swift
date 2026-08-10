import SwiftUI

/// Terminal theme picker. Every row previews the palette it is offering — a
/// list of theme *names* can't answer the only question being asked here
/// ("what will my terminal look like?"), which is why this replaced the
/// dropdown that used to live in Settings.
///
/// Built-ins are read-only; the swipe/tap affordances for editing appear only
/// on the user's own themes, with "Duplicate" as the bridge from one to the
/// other.
struct ThemeGalleryView: View {
    @Environment(AppSettings.self) private var settings
    @Environment(ThemeStore.self) private var themes

    @State private var editing: TerminalTheme?
    @State private var deleting: TerminalTheme?
    @State private var showImport = false
    @State private var importText = ""
    @State private var importError: String?

    /// The id actually in effect, which is not always `settings.themeId`: a
    /// stored id can outlive the theme it names (a custom theme deleted from
    /// another device, or by the DEBUG reset seam), in which case the terminal
    /// falls back to the default. Ticking the row by the *resolved* id keeps the
    /// gallery honest — comparing raw ids showed no selection at all while the
    /// terminal was quite visibly using a theme.
    private var selectedID: String { themes.theme(id: settings.themeId).id }

    var body: some View {
        ZStack {
            MoshpitBackground()

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    FormGroup(title: "BUILT-IN") {
                        ForEach(TerminalTheme.builtIns) { theme in
                            ThemeRow(theme: theme,
                                     isSelected: theme.id == selectedID,
                                     onSelect: { select(theme) })
                                .contextMenu {
                                    Button {
                                        editing = themes.duplicate(theme)
                                    } label: {
                                        Label("Duplicate & Edit", systemImage: "plus.square.on.square")
                                    }
                                }
                        }
                    }

                    FormGroup(
                        title: "MY THEMES",
                        footer: "Duplicate a built-in theme to start from its palette, or import one as JSON. Long-press a theme for more."
                    ) {
                        if themes.customThemes.isEmpty {
                            Text("No custom themes yet.")
                                .font(Face.text(13))
                                .foregroundStyle(Ink.meta)
                                .frame(minHeight: Metrics.cellMinHeight, alignment: .leading)
                        } else {
                            ForEach(themes.customThemes) { theme in
                                ThemeRow(theme: theme,
                                         isSelected: theme.id == selectedID,
                                         isEditable: true,
                                         onSelect: { select(theme) },
                                         onEdit: { editing = theme })
                                    .contextMenu {
                                        Button { editing = theme } label: {
                                            Label("Edit", systemImage: "slider.horizontal.3")
                                        }
                                        Button { editing = themes.duplicate(theme) } label: {
                                            Label("Duplicate", systemImage: "plus.square.on.square")
                                        }
                                        Button { copyJSON(theme) } label: {
                                            Label("Copy JSON", systemImage: "doc.on.doc")
                                        }
                                        Button(role: .destructive) { deleting = theme } label: {
                                            Label("Delete", systemImage: "trash")
                                        }
                                    }
                            }
                        }
                    }
                }
                .padding(.bottom, 28)
            }
        }
        .navigationTitle("Theme")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button {
                        editing = themes.makeDraft()
                    } label: {
                        Label("New Theme", systemImage: "plus")
                    }
                    Button {
                        importText = ""
                        importError = nil
                        showImport = true
                    } label: {
                        Label("Import JSON…", systemImage: "square.and.arrow.down")
                    }
                } label: {
                    Image(systemName: "plus")
                        .foregroundStyle(Ink.accent)
                        .accessibilityLabel("Add theme")
                }
                .accessibilityIdentifier("theme-add")
            }
        }
        // `item:` (not `isPresented:`) so the sheet always opens against the
        // theme that was tapped — a shared bool + separate state can present
        // stale content when two rows are tapped in quick succession.
        .sheet(item: $editing) { theme in
            ThemeEditorView(theme: theme) { saved in
                themes.save(saved)
                settings.themeId = saved.id      // editing implies wanting it
            }
        }
        .sheet(isPresented: $showImport) {
            ThemeImportView(text: $importText, error: $importError) {
                runImport()
            }
        }
        .moshpitCard(item: $deleting) { theme in
            MoshpitModalCard(
                icon: "trash.fill", tone: .danger,
                title: "Delete \(theme.name)?",
                message: Text("This theme will be removed. Built-in themes are unaffected."),
                buttons: [
                    ModalButton("Cancel", kind: .secondary) { deleting = nil },
                    ModalButton("Delete", kind: .danger) {
                        remove(theme)
                        deleting = nil
                    },
                ]
            ) { EmptyView() }
        }
    }

    // MARK: - Actions

    private func select(_ theme: TerminalTheme) {
        Haptics.tap()
        settings.themeId = theme.id
    }

    /// Deleting the selected theme has to move the selection too, or the
    /// terminal would fall back to the default with no visible reason why.
    private func remove(_ theme: TerminalTheme) {
        let wasSelected = settings.themeId == theme.id
        themes.delete(id: theme.id)
        if wasSelected { settings.themeId = TerminalTheme.fallback.id }
    }

    private func copyJSON(_ theme: TerminalTheme) {
        guard let json = try? themes.exportJSON(theme) else { return }
        UIPasteboard.general.string = json
        Haptics.tap()
    }

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
}

// MARK: - Row

private struct ThemeRow: View {
    let theme: TerminalTheme
    let isSelected: Bool
    var isEditable: Bool = false
    let onSelect: () -> Void
    var onEdit: (() -> Void)?

    var body: some View {
        HStack(spacing: 11) {
            Button(action: onSelect) {
                HStack(spacing: 11) {
                    ThemePreviewTile(theme: theme)
                    VStack(alignment: .leading, spacing: 5) {
                        Text(theme.name)
                            .font(Face.text(14))
                            .foregroundStyle(Ink.primary)
                            .lineLimit(1)
                        ThemeSwatchStrip(theme: theme, swatch: 9)
                    }
                    Spacer(minLength: 4)
                    if isSelected {
                        Image(systemName: "checkmark")
                            .font(.system(size: 13, weight: .bold))
                            .foregroundStyle(Ink.accent)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            // Identifier belongs on the Button, not the enclosing HStack:
            // SwiftUI doesn't publish an accessibility element for a plain
            // stack, so an identifier there is invisible to XCUITest (and to
            // VoiceOver) — the row was unreachable from the UI tests.
            .accessibilityIdentifier("theme-row-\(theme.id)")

            if isEditable, let onEdit {
                Button(action: onEdit) {
                    Image(systemName: "slider.horizontal.3")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Ink.meta)
                        .frame(width: 30, height: 30)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Edit \(theme.name)")
            }
        }
        .frame(minHeight: 52)
    }
}

// MARK: - Previews shared with Settings

/// A miniature terminal: the theme's own background with a prompt line in its
/// own foreground/accent colors. Small enough for a list row, but it shows the
/// two colors that actually decide legibility.
struct ThemePreviewTile: View {
    let theme: TerminalTheme
    var side: CGFloat = 44

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: side * 0.2, style: .continuous)
                .fill(theme.background)

            VStack(alignment: .leading, spacing: side * 0.09) {
                HStack(spacing: side * 0.05) {
                    Text("❯")
                        .foregroundStyle(theme.green)
                    Rectangle()
                        .fill(theme.foreground.opacity(0.75))
                        .frame(width: side * 0.34, height: max(1, side * 0.055))
                }
                HStack(spacing: side * 0.05) {
                    Rectangle().fill(theme.blue).frame(width: side * 0.2, height: max(1, side * 0.055))
                    Rectangle().fill(theme.yellow).frame(width: side * 0.14, height: max(1, side * 0.055))
                }
                HStack(spacing: side * 0.05) {
                    Rectangle().fill(theme.magenta).frame(width: side * 0.12, height: max(1, side * 0.055))
                    Rectangle()
                        .fill(theme.cursor)
                        .frame(width: side * 0.1, height: max(2, side * 0.1))
                }
            }
            .font(.system(size: side * 0.2, weight: .bold, design: .monospaced))
            .padding(side * 0.16)
        }
        .frame(width: side, height: side)
        .overlay(
            RoundedRectangle(cornerRadius: side * 0.2, style: .continuous)
                .strokeBorder(Ink.groupBorder, lineWidth: 1))
        .accessibilityHidden(true)
    }
}

/// The eight base ANSI colors as a compact strip — the palette's fingerprint,
/// which is what makes two similar-looking dark themes distinguishable at a
/// glance.
struct ThemeSwatchStrip: View {
    let theme: TerminalTheme
    var swatch: CGFloat = 8

    var body: some View {
        HStack(spacing: 2.5) {
            ForEach(TerminalTheme.ANSISlot.allCases) { slot in
                RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                    .fill(theme.ansi(slot))
                    .frame(width: swatch, height: swatch)
            }
        }
        .accessibilityHidden(true)
    }
}

// MARK: - Import sheet

private struct ThemeImportView: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var text: String
    @Binding var error: String?
    let onImport: () -> Void

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        Text("Paste a theme exported from Moshpit, or any JSON object with \"background\", \"foreground\" and the eight ANSI color names (\"black\", \"red\", …). Bright colors are optional.")
                            .font(Face.text(13))
                            .foregroundStyle(Ink.secondary)
                            .fixedSize(horizontal: false, vertical: true)

                        TextEditor(text: $text)
                            .font(Face.mono(12))
                            .foregroundStyle(Ink.primary)
                            .scrollContentBackground(.hidden)
                            .frame(minHeight: 240)
                            .padding(10)
                            .background(Ink.terminalBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                    .strokeBorder(Ink.groupBorder, lineWidth: 1))
                            .accessibilityIdentifier("theme-import-editor")

                        if let error {
                            Text(error)
                                .font(Face.text(12))
                                .foregroundStyle(Ink.danger)
                                .fixedSize(horizontal: false, vertical: true)
                        }

                        Button {
                            if let pasted = UIPasteboard.general.string { text = pasted }
                        } label: {
                            Label("Paste from Clipboard", systemImage: "doc.on.clipboard")
                                .font(Face.text(13))
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(Ink.accent)
                    }
                    .padding(16)
                }
            }
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
