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
            Form {
                FormGroup(title: "SAVED KEYS") {
                    if keys.isEmpty {
                        emptyState
                    } else {
                        ForEach(keys) { key in
                            pickerRow(
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
                    pickerRow(
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
            .moshpitForm()
            .navigationTitle("Select Key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
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
        .appSheet(isPresented: $showAddKey) {
            AddKeyView(onCreated: { record in
                selectedKeyId = record.id
                showAddKey = false
                dismiss()
            })
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        Text("No keys yet — generate or import one with ＋")
            .font(.footnote)
            .foregroundStyle(Ink.meta)
        Button {
            showAddKey = true
        } label: {
            Label("Generate SSH Key", systemImage: "plus.circle")
                .font(.body)
        }
        .foregroundStyle(Ink.accent)
        .accessibilityIdentifier("key-picker-empty-generate")
    }

    /// A native Form row: icon, name, meta, and a checkmark on the selection.
    /// The terminal sheets keep `SheetListRow`, whose pill chrome belongs to
    /// the compact sheet; inside an inset-grouped Form the row itself is the
    /// chrome.
    private func pickerRow(icon: String, name: String, meta: String, isActive: Bool,
                           action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon)
                    .font(.body)
                    .foregroundStyle(isActive ? Ink.accent : Ink.secondary)
                    .frame(width: 28)
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).font(.body).foregroundStyle(Ink.primary)
                    Text(meta).font(.footnote).foregroundStyle(Ink.meta)
                }
                Spacer(minLength: 8)
                if isActive {
                    Image(systemName: "checkmark")
                        .font(.body.weight(.semibold))
                        .foregroundStyle(Ink.accent)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityAddTraits(isActive ? .isSelected : [])
    }
}
