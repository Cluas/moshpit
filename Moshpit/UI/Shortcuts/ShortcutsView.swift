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
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(
                            title: "PREVIEW",
                            footer: "Drag ≡ to reorder; the red − removes it from the toolbar. 12 items max."
                        ) {
                            previewStrip
                        }

                        FormGroup(title: "IN TOOLBAR · \(store.toolbarCount)/\(ShortcutStore.toolbarLimit)") {
                            toolbarList
                        }

                        FormGroup(
                            title: "CUSTOM",
                            footer: "Tap ＋ in the top right to create a custom shortcut — key combos, text snippets, and command chains."
                        ) {
                            if store.custom.isEmpty {
                                Text("No custom shortcuts yet")
                                    .font(Face.text(12))
                                    .foregroundStyle(Ink.tertiary)
                                    .frame(minHeight: Metrics.cellMinHeight)
                            } else {
                                ForEach(store.custom) { shortcut in
                                    ShortcutEditRow(
                                        shortcut: shortcut,
                                        accessory: .remove,
                                        onAction: { store.remove(id: shortcut.id) },
                                        onTap: { editing = shortcut })
                                }
                            }
                        }

                        FormGroup(title: "AVAILABLE") {
                            ForEach(store.available) { shortcut in
                                ShortcutEditRow(
                                    shortcut: shortcut,
                                    accessory: .add,
                                    onAction: { store.addToToolbar(id: shortcut.id) },
                                    onTap: nil)
                            }
                        }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle("Shortcuts")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .font(Face.text(15))
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
        .sheet(isPresented: $showAdd) {
            AddShortcutView()
        }
        .sheet(item: $editing) { shortcut in
            AddShortcutView(existing: shortcut)
        }
    }

    // MARK: Preview strip

    private var previewStrip: some View {
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
        .padding(.vertical, 8)
    }

    // MARK: Toolbar list (drag to reorder)

    private var toolbarList: some View {
        VStack(spacing: 0) {
            ForEach(store.toolbar) { shortcut in
                ShortcutEditRow(
                    shortcut: shortcut,
                    accessory: .remove,
                    showsHandle: true,
                    onAction: { store.removeFromToolbar(id: shortcut.id) },
                    onTap: shortcut.isBuiltin ? nil : { editing = shortcut })
                // Modern drag-reorder (replaces the flaky NSItemProvider +
                // DropDelegate that hovered-reordered asynchronously).
                .draggable(shortcut.id.uuidString) {
                    ShortcutEditRow(shortcut: shortcut, accessory: .remove,
                                    onAction: {}, onTap: nil)
                        .frame(width: 220)
                        .padding(8)
                        .background(Ink.group)
                }
                .dropDestination(for: String.self) { items, _ in
                    guard let first = items.first, let id = UUID(uuidString: first) else { return false }
                    store.moveInToolbar(id, before: shortcut.id)
                    return true
                }
                if shortcut.id != store.toolbar.last?.id {
                    Rectangle().fill(Ink.hairline).frame(height: 1)
                }
            }
        }
    }
}

// MARK: - Row (§3.5)

struct ShortcutEditRow: View {
    enum Accessory { case remove, add }

    let shortcut: TerminalShortcut
    let accessory: Accessory
    var showsHandle: Bool = false
    let onAction: () -> Void
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
        HStack(spacing: 10) {
            Button(action: onAction) {
                ZStack {
                    Circle()
                        .fill(accessory == .remove ? Ink.danger : Ink.success)
                        .frame(width: 22, height: 22)
                    Image(systemName: accessory == .remove ? "minus" : "plus")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundStyle(Ink.screenBG)
                }
                .frame(width: 40, height: 40)
            }
            .buttonStyle(.plain)

            glyph
                .frame(width: 56, alignment: .leading)

            Text(shortcut.summary)
                .font(Face.text(14))
                .foregroundStyle(Ink.primary)

            Spacer()

            if showsHandle {
                VStack(spacing: 3) {
                    ForEach(0..<3, id: \.self) { _ in
                        RoundedRectangle(cornerRadius: 1)
                            .fill(Ink.meta)
                            .frame(width: 18, height: 2)
                    }
                }
            }
        }
        .frame(minHeight: Metrics.cellMinHeight)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .onTapGesture { onTap?() }
    }
}
