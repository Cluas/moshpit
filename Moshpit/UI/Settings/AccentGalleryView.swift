import SwiftUI

/// Accent-color picker for the app chrome, with custom colors.
///
/// A theme here is one accent; `accentPressed` and the two faint background
/// tints are derived from it (``AppTheme/custom(id:name:accentHex:)``), which is
/// why "custom" needs a single color well rather than a full editor. The app
/// icon is chosen separately — see ``AppIconGalleryView``.
struct AccentGalleryView: View {
    @Environment(AppSettings.self) private var settings
    @Environment(AppThemeStore.self) private var store

    @State private var editing: AppTheme?
    @State private var deleting: AppTheme?

    /// The row the user has picked but that hasn't been written to settings yet.
    ///
    /// Writing `appThemeId` immediately would recolor the app instantly — which
    /// is nice — but the root view forces a full subtree rebuild on that change
    /// (`Ink`'s tokens are static computed properties that Observation can't
    /// track), and the rebuild takes this pushed screen down with it: tapping an
    /// accent bounced the user back to Settings, so comparing two accents meant
    /// re-navigating each time. Ticking the row locally and committing on the
    /// way out keeps the gallery usable, and the recolor still lands the moment
    /// the user leaves — the same sequence the old in-Settings list used when it
    /// dismissed before applying.
    @State private var pendingID: String?

    /// The row to tick: the pending pick if there is one, else the theme
    /// actually in effect. A stored id can outlive the theme it names (a custom
    /// accent deleted while selected), so this resolves rather than comparing
    /// raw ids.
    private var selectedID: String {
        pendingID ?? AppThemeCatalog.theme(for: settings.appThemeId).id
    }

    var body: some View {
        ZStack {
            MoshpitBackground()

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    FormGroup(title: "BUILT-IN") {
                        ForEach(AppThemeCatalog.builtIns) { theme in
                            AccentRow(theme: theme,
                                      isSelected: theme.id == selectedID,
                                      onSelect: { select(theme) })
                        }
                    }

                    FormGroup(
                        title: "MY ACCENTS",
                        footer: "A custom accent tints controls, highlights and the faint background wash. Status colors (warning, success, error) stay fixed so they never get mistaken for the accent."
                    ) {
                        if store.customThemes.isEmpty {
                            Text("No custom accents yet.")
                                .font(Face.text(13))
                                .foregroundStyle(Ink.meta)
                                .frame(minHeight: Metrics.cellMinHeight, alignment: .leading)
                        } else {
                            ForEach(store.customThemes) { theme in
                                AccentRow(theme: theme,
                                          isSelected: theme.id == selectedID,
                                          isEditable: true,
                                          onSelect: { select(theme) },
                                          onEdit: { editing = theme })
                                    .contextMenu {
                                        Button { editing = theme } label: {
                                            Label("Edit", systemImage: "slider.horizontal.3")
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
        .navigationTitle("Accent")
        .navigationBarTitleDisplayMode(.inline)
        .onDisappear { commit() }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    editing = store.makeDraft(basedOn: AppThemeCatalog.theme(for: settings.appThemeId))
                } label: {
                    Image(systemName: "plus").foregroundStyle(Ink.accent)
                }
                // Both modifiers go on the Button: an accessibilityLabel on the
                // inner Image makes that image its own element, and the
                // identifier on the Button then isn't what the label resolves
                // to — the button became unreachable from the UI tests.
                .accessibilityLabel("New accent")
                .accessibilityIdentifier("accent-add")
            }
        }
        .sheet(item: $editing) { theme in
            AccentEditorView(theme: theme) { saved in
                // Persist immediately (the user pressed Save), but route the
                // selection through `pendingID` like any other pick.
                store.save(saved)
                pendingID = saved.id
            }
        }
        .moshpitCard(item: $deleting) { theme in
            MoshpitModalCard(
                icon: "trash.fill", tone: .danger,
                title: "Delete \(theme.name)?",
                message: Text("This accent will be removed. Built-in accents are unaffected."),
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

    private func select(_ theme: AppTheme) {
        guard theme.id != selectedID else { return }
        Haptics.select()
        pendingID = theme.id
    }

    /// Apply the pick as the screen goes away — see ``pendingID``.
    private func commit() {
        guard let pendingID, pendingID != settings.appThemeId else { return }
        withAnimation(.easeInOut(duration: 0.28)) {
            settings.appThemeId = pendingID
        }
    }

    private func remove(_ theme: AppTheme) {
        let wasSelected = selectedID == theme.id
        store.delete(id: theme.id)
        if wasSelected {
            pendingID = AppThemeCatalog.signalRoom.id
            // The live setting has to move too if it pointed at the deleted
            // theme — otherwise it resolves to the fallback anyway, but the
            // stored id would stay dangling.
            if settings.appThemeId == theme.id {
                settings.appThemeId = AppThemeCatalog.signalRoom.id
            }
        }
    }
}

// MARK: - Row

private struct AccentRow: View {
    let theme: AppTheme
    let isSelected: Bool
    var isEditable: Bool = false
    let onSelect: () -> Void
    var onEdit: (() -> Void)?

    var body: some View {
        HStack(spacing: 11) {
            Button(action: onSelect) {
                HStack(spacing: 12) {
                    // The swatch shows the accent over the background wash it
                    // derives, so the row previews the whole mood, not one dot.
                    ZStack {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .fill(theme.screenBG)
                        Circle()
                            .fill(theme.accent)
                            .frame(width: 16, height: 16)
                    }
                    .frame(width: 34, height: 30)
                    .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(Color.white.opacity(0.12), lineWidth: 1))

                    Text(theme.name).font(Face.text(14)).foregroundStyle(Ink.primary).lineLimit(1)
                    Spacer(minLength: 4)
                    Text("#" + theme.accentHex)
                        .font(Face.mono(11))
                        .foregroundStyle(Ink.meta)
                    if isSelected {
                        Image(systemName: "checkmark")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(theme.accent)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("accent-row-\(theme.id)")

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
        .frame(minHeight: 46)
    }
}

// MARK: - Editor

/// One color well plus a name, previewed on a mock of the app's own chrome.
private struct AccentEditorView: View {
    @Environment(\.dismiss) private var dismiss

    @State private var name: String
    @State private var accentHex: String
    private let id: String
    private let onSave: (AppTheme) -> Void

    init(theme: AppTheme, onSave: @escaping (AppTheme) -> Void) {
        self.id = theme.id
        _name = State(initialValue: theme.name)
        _accentHex = State(initialValue: theme.accentHex)
        self.onSave = onSave
    }

    /// Rebuilt on every change so the preview shows the derived values too.
    private var draft: AppTheme {
        AppTheme.custom(id: id, name: name, accentHex: accentHex)
    }

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        AccentPreview(theme: draft)
                            .padding(.horizontal, 16)
                            .padding(.top, 8)

                        FormGroup(title: "NAME") {
                            FieldRow(placeholder: "Accent name", text: $name)
                        }

                        FormGroup(
                            title: "COLOR",
                            footer: "The pressed state and the background wash are derived from this one color."
                        ) {
                            HStack(spacing: 10) {
                                Text("Accent").font(Face.text(14)).foregroundStyle(Ink.primary)
                                Spacer(minLength: 6)
                                Text("#" + accentHex)
                                    .font(Face.mono(12))
                                    .foregroundStyle(Ink.meta)
                                ColorPicker("", selection: Binding(
                                    get: { Color(hex: accentHex) },
                                    set: { accentHex = $0.hexString }
                                ), supportsOpacity: false)
                                .labelsHidden()
                                .frame(width: 34)
                            }
                            .frame(minHeight: Metrics.cellMinHeight)
                        }
                    }
                    .padding(.bottom, 28)
                }
            }
            .navigationTitle(name.isEmpty ? "New Accent" : name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        onSave(draft)
                        dismiss()
                    }
                    .accessibilityIdentifier("accent-editor-save")
                }
            }
        }
    }
}

/// A mock of the app's chrome in the draft accent — a button, a pill, a
/// selected row — so the color is judged where it will actually appear rather
/// than as a swatch.
private struct AccentPreview: View {
    let theme: AppTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Text("CONNECT")
                    .font(Face.mono(11, .bold))
                    .kerning(0.8)
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .background(theme.accent, in: Capsule())
                HStack(spacing: 5) {
                    Circle().fill(theme.accent).frame(width: 5, height: 5)
                    Text("MOSH").font(Face.mono(9.5, .bold)).kerning(0.6)
                        .foregroundStyle(theme.accent)
                }
                .padding(.horizontal, 7).padding(.vertical, 3)
                .background(theme.accent.opacity(0.13), in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .strokeBorder(theme.accent.opacity(0.3), lineWidth: 1))
                Spacer()
            }

            HStack(spacing: 9) {
                Image(systemName: "checkmark")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(theme.accent)
                Text("selected row").font(Face.text(13)).foregroundStyle(Ink.primary)
                Spacer()
                // Status colors are fixed on purpose — the preview shows them
                // next to the accent so a clash is visible before saving.
                Text("warn").font(Face.mono(10)).foregroundStyle(Ink.warn)
                Text("ok").font(Face.mono(10)).foregroundStyle(Ink.success)
                Text("err").font(Face.mono(10)).foregroundStyle(Ink.danger)
            }
        }
        .padding(14)
        .background(theme.screenBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
            .strokeBorder(Ink.groupBorder, lineWidth: 1))
        .accessibilityLabel("Accent preview")
    }
}
