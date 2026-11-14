import SwiftUI

/// Add Connection › Select Key: the SwiftUI page (``KeyPickerView``), or the
/// NSH one while its switch is on (``NSHScreen/keyPicker``). With the switch
/// off the SwiftUI page is all there is: no NSH code runs.
struct KeyPickerSheet: View {
    let keys: [SSHKeyRecord]
    @Binding var selectedKeyId: UUID?

    var body: some View {
        if Self.showsNSH {
            KeyPickerNSHPage(keys: keys, selectedKeyId: $selectedKeyId)
        } else {
            KeyPickerView(keys: keys, selectedKeyId: $selectedKeyId)
        }
    }

    static var showsNSH: Bool {
        #if DEBUG
        if CaptureScreen.requested == .keyPickerNSH { return NSHSurface.available }
        #endif
        return NSHScreen.keyPicker.isOn
    }
}

/// Modal key picker for Add Connection's SSH-Key auth row, replacing a plain
/// system `Menu`. Also the fix for "no keys yet ⇒ paste-only": the empty
/// state offers the same generate/import action as the toolbar "+", so
/// there's always a way in besides pasting a raw PEM.
struct KeyPickerView: View {
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
                            // A key the connect path can't sign with (an
                            // ECDSA P-384 key an older build imported, say)
                            // stays listed, saying so, instead of failing
                            // every connection it is picked for.
                            let refused = StoredKey.refusedType(ofPublicKeyLine: key.publicKey)
                            pickerRow(
                                icon: "key.fill",
                                name: key.name,
                                meta: refused.map { String(localized: "Can't sign in with \($0) keys yet") }
                                    ?? key.badgeText,
                                isActive: key.id == selectedKeyId
                            ) {
                                Haptics.select()
                                selectedKeyId = key.id
                                dismiss()
                            }
                            .disabled(refused != nil && key.id != selectedKeyId)
                            .opacity(refused != nil ? 0.5 : 1)
                            .accessibilityIdentifier(KeyPickerModel.rowID(key.id))
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
                        .accessibilityIdentifier("key-picker-cancel")
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
        .darkOnlyPage()
        .appSheet(isPresented: $showAddKey) {
            AddKeyPage(onCreated: { record in
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
