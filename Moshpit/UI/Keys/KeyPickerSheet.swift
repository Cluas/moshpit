import SwiftUI

/// Modal key picker for Add Connection's SSH-Key auth row, replacing a plain
/// system `Menu`. Also the fix for "no keys yet ⇒ paste-only": the empty
/// state offers the same generate/import action as the toolbar "+", so
/// there's always a way in besides pasting a raw PEM.
struct KeyPickerSheet: View {
    let keys: [SSHKeyRecord]
    @Binding var selectedKeyId: UUID?

    @Environment(\.dismiss) private var dismiss
    @State private var showAddKey = false

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(title: "SAVED KEYS") {
                            if keys.isEmpty {
                                emptyState
                            } else {
                                ForEach(keys) { key in
                                    SheetListRow(
                                        icon: "key.fill",
                                        name: key.name,
                                        meta: key.badgeText,
                                        isActive: key.id == selectedKeyId
                                    ) {
                                        Haptics.select()
                                        selectedKeyId = key.id
                                        dismiss()
                                    }
                                }
                            }
                        }

                        FormGroup(title: "OR") {
                            SheetListRow(
                                icon: "doc.text",
                                name: String(localized: "Paste a PEM instead…"),
                                meta: String(localized: "Enter a private key by hand"),
                                isActive: selectedKeyId == nil
                            ) {
                                Haptics.select()
                                selectedKeyId = nil
                                dismiss()
                            }
                            .accessibilityIdentifier("key-picker-paste-pem")
                        }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle("Select Key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .font(Face.text(15))
                        .foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        showAddKey = true
                    } label: {
                        Image(systemName: "plus")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(Ink.accent)
                            .frame(width: 32, height: 32)
                            .background(Ink.accent.opacity(0.11), in: Circle())
                            .overlay(Circle().strokeBorder(Ink.accent.opacity(0.24), lineWidth: 1))
                    }
                    .accessibilityIdentifier("key-picker-add")
                }
            }
        }
        .preferredColorScheme(.dark)
        .sheet(isPresented: $showAddKey) {
            AddKeyView(onCreated: { record in
                selectedKeyId = record.id
                showAddKey = false
                dismiss()
            })
        }
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("No keys yet — generate or import one with ＋")
                .font(Face.text(12))
                .foregroundStyle(Ink.tertiary)
                .frame(minHeight: Metrics.cellMinHeight, alignment: .leading)
            Button {
                showAddKey = true
            } label: {
                Text("Generate SSH Key")
                    .font(Face.text(14, .semibold))
                    .foregroundStyle(Ink.accent)
                    .frame(maxWidth: .infinity, minHeight: 40)
                    .background(Ink.accent.opacity(0.12), in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                            .strokeBorder(Ink.accent.opacity(0.28), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .padding(.bottom, 10)
            .accessibilityIdentifier("key-picker-empty-generate")
        }
    }
}
