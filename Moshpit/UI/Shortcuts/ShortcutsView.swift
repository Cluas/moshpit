import SwiftUI

/// Screen 6 — Shortcuts list editor. PREVIEW strip, IN TOOLBAR n/12 with
/// remove + drag reorder, CUSTOM group, AVAILABLE pool.
struct ShortcutsView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(ShortcutStore.self) private var store

    @State private var showAdd = false
    @State private var editing: TerminalShortcut?

    var body: some View {
        NavigationStack {
            Form {
                FormGroup(
                    title: "PREVIEW",
                    footer: "Drag ≡ to reorder. − takes a shortcut off the toolbar; it stays under AVAILABLE. 12 items max."
                ) {
                    previewStrip
                }

                // The list is permanently in edit mode: the system draws the
                // reorder handles and the − controls, which is what the
                // hand-made handle and red circle were imitating.
                FormGroup(title: "IN TOOLBAR · \(store.toolbarCount)/\(ShortcutStore.toolbarLimit)") {
                    ForEach(store.toolbar) { shortcut in
                        ShortcutEditRow(
                            shortcut: shortcut,
                            onTap: shortcut.isBuiltin ? nil : { editing = shortcut })
                    }
                    .onMove { store.moveInToolbar(fromOffsets: $0, toOffset: $1) }
                    .onDelete { offsets in
                        // Resolve ids before mutating: each removal reindexes.
                        let ids = offsets.map { store.toolbar[$0].id }
                        ids.forEach { store.removeFromToolbar(id: $0) }
                    }
                }

                FormGroup(
                    title: "CUSTOM",
                    footer: "Tap ＋ in the top right to create a custom shortcut — key combos, text snippets, and command chains. Tap one to edit it."
                ) {
                    if store.custom.isEmpty {
                        Text("No custom shortcuts yet")
                            .font(.footnote)
                            .foregroundStyle(Ink.tertiary)
                    } else {
                        ForEach(store.custom) { shortcut in
                            ShortcutEditRow(shortcut: shortcut, onTap: { editing = shortcut })
                        }
                        .onDelete { offsets in
                            let ids = offsets.map { store.custom[$0].id }
                            ids.forEach { store.remove(id: $0) }
                        }
                    }
                }

                FormGroup(title: "AVAILABLE") {
                    ForEach(store.available) { shortcut in
                        ShortcutEditRow(
                            shortcut: shortcut,
                            onAdd: { store.addToToolbar(id: shortcut.id) },
                            onTap: nil)
                    }
                }
            }
            .environment(\.editMode, .constant(.active))
            .moshpitForm()
            .navigationTitle("Shortcuts")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        showAdd = true
                    } label: {
                        Image(systemName: "plus")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(Ink.accent)
                            .frame(width: 32, height: 32)
                            .background(Ink.accent.opacity(0.11), in: Circle())
                            .overlay(Circle().strokeBorder(Ink.accent.opacity(0.24), lineWidth: 1))
                    }
                }
            }
        }
        .preferredColorScheme(.dark)
        .appSheet(isPresented: $showAdd) {
            AddShortcutView()
        }
        .appSheet(item: $editing) { shortcut in
            AddShortcutView(existing: shortcut)
        }
    }

    // MARK: Preview strip

    private var previewStrip: some View {
        // The strip previews the terminal's toolbar, which is keyboard-like
        // chrome and stays clamped at xLarge (TerminalScreen); the preview
        // follows the same clamp so it shows what the bar will actually be.
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 5) {
                ForEach(store.toolbar) { shortcut in
                    Group {
                        if let symbol = ShortcutEditRow.glyphSymbol(for: shortcut.kind) {
                            Image(systemName: symbol)
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundStyle(Ink.primary)
                        } else {
                            Text(shortcut.chipLabel)
                                .font(Face.mono(10.5, .semibold))
                                .foregroundStyle(shortcut.isBuiltin ? Ink.primary : Ink.customChipText)
                        }
                    }
                    .padding(.horizontal, 7)
                    .frame(minWidth: 36, minHeight: 24)
                    .background(
                        shortcut.isBuiltin ? Ink.shortcutKeyBG : Ink.customChipBG,
                        in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .strokeBorder(shortcut.isBuiltin ? Ink.groupBorder : Ink.customChipBorder, lineWidth: 1))
                }
            }
        }
        .dynamicTypeSize(...DynamicTypeSize.xLarge)
        .padding(.vertical, 8)
    }
}

// MARK: - Row (§3.5)

struct ShortcutEditRow: View {
    let shortcut: TerminalShortcut
    /// Present on AVAILABLE rows: the green ＋ that puts the shortcut back on
    /// the toolbar. IN TOOLBAR and CUSTOM rows get their − from edit mode.
    var onAdd: (() -> Void)?
    let onTap: (() -> Void)?

    /// SF Symbol for the kinds the toolbar draws as an icon; nil for kinds
    /// that render as a text chip.
    static func glyphSymbol(for kind: ShortcutKind) -> String? {
        switch kind {
        case .dpad: return "arrow.up.and.down.and.arrow.left.and.right"
        case .scroll: return "arrow.up.and.down"
        case .mic: return "mic"
        case .image: return "photo"
        default: return nil
        }
    }

    /// Glyph-rendered kinds show their symbol (matching the bar) rather than
    /// their text label, so the editor and the terminal look the same.
    @ViewBuilder private var glyph: some View {
        if let symbol = Self.glyphSymbol(for: shortcut.kind) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Ink.primary)
                .padding(EdgeInsets(top: 4, leading: 6, bottom: 4, trailing: 6))
                .frame(minWidth: 36)
                .background(Ink.chipNeutralBG, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        } else {
            ShortcutChip(label: shortcut.chipLabel, custom: !shortcut.isBuiltin)
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            if let onAdd {
                Button(action: onAdd) {
                    Image(systemName: "plus.circle.fill")
                        .font(.title3)
                        .foregroundStyle(Ink.success)
                        .frame(width: 32, height: 32)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Add \(shortcut.summary) to toolbar")
            }

            glyph
                // Minimum, not fixed: the chip's mono label scales with
                // Dynamic Type and must widen rather than wrap "es/c".
                .frame(minWidth: 56, alignment: .leading)

            Text(shortcut.summary)
                .font(.body)
                .foregroundStyle(Ink.primary)

            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
        .onTapGesture { onTap?() }
        // Chips differ in width, so a separator inset to the title would
        // start somewhere else on every row; run it across the whole panel.
        .alignmentGuide(.listRowSeparatorLeading) { $0[.leading] }
    }
}
