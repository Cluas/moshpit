import SwiftUI

/// Screen 8 — SSH Keys list. Secure Enclave device keys on top, imported
/// keys below, each with algo badge + SHA256 fingerprint + host chips.
struct SSHKeysView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ConnectionStoreHolder.self) private var connections
    @Environment(KeychainServiceHolder.self) private var keychainHolder

    @State private var showAdd = false
    /// The key whose delete is awaiting confirmation. Deleting is not
    /// undoable — a Secure Enclave key's private half never existed anywhere
    /// else — so a long-press menu item alone is too easy to hit.
    @State private var deleting: SSHKeyRecord?
    /// Why a key wasn't deleted: connections still use it, or the keychain
    /// kept it (``SSHKeyDeletion``).
    @State private var notice: KeyDeletionNotice?

    var body: some View {
        NavigationStack {
            Form {
                FormGroup(
                    title: "THIS DEVICE · SECURE ENCLAVE",
                    footer: "Hardware-backed keys cannot be exported; every signature triggers Face ID."
                ) {
                    if keyStore.deviceKeys.isEmpty {
                        Text("No device key yet — generate one with ＋")
                            .font(Face.text(12))
                            .foregroundStyle(Ink.tertiary)
                    } else {
                        ForEach(keyStore.deviceKeys) { key in
                            SSHKeyRow(record: key)
                                .contextMenu { keyRowMenu(key) { requestDelete(key) } }
                                .accessibilityIdentifier(SSHKeysModel.rowID(key.id))
                        }
                    }
                }

                FormGroup(title: "IMPORTED") {
                    if keyStore.importedKeys.isEmpty {
                        Text("No imported keys")
                            .font(Face.text(12))
                            .foregroundStyle(Ink.tertiary)
                    } else {
                        ForEach(keyStore.importedKeys) { key in
                            SSHKeyRow(record: key)
                                .contextMenu { keyRowMenu(key) { requestDelete(key) } }
                                .accessibilityIdentifier(SSHKeysModel.rowID(key.id))
                        }
                    }
                }
            }
            .moshpitForm()
            .navigationTitle("SSH Keys")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .foregroundStyle(Ink.accent)
                        .accessibilityIdentifier("ssh-keys-done")
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
                    .accessibilityLabel("Add Key")
                    .accessibilityIdentifier("ssh-keys-add")
                }
            }
            .moshpitCard(item: $deleting) { key in
                MoshpitModalCard(
                    icon: "trash.fill", tone: .danger,
                    title: "Delete \(key.name)?",
                    message: Text("Its private key is erased from this device for good; Moshpit keeps no other copy. Servers that only trust this key will need another one."),
                    buttons: [
                        ModalButton("Cancel", kind: .secondary, identifier: "ssh-key-delete-cancel") { deleting = nil },
                        ModalButton("Delete", kind: .danger, identifier: "ssh-key-delete-confirm") {
                            deleting = nil
                            delete(key)
                        },
                    ]
                ) { EmptyView() }
            }
            .moshpitCard(item: $notice) { notice in
                MoshpitModalCard(
                    icon: notice.failure.isInUse ? "link" : "exclamationmark.triangle.fill", tone: .danger,
                    title: notice.failure.isInUse ? "Key in use" : "Couldn't delete the key",
                    message: Text(notice.failure.message),
                    buttons: [
                        ModalButton("OK", kind: .secondary, identifier: "ssh-key-notice-ok") { self.notice = nil },
                    ]
                ) { EmptyView() }
            }
            #if DEBUG
            .task {
                // A capture run's keys are in memory, named by no connection:
                // the confirmation, as for any key nothing uses.
                if let id = KeysCapture.deleteRequested(nsh: false, in: SSHKeysSnapshot(store: keyStore)) {
                    deleting = keyStore.keys.first { $0.id == id }
                }
            }
            #endif
        }
        .darkOnlyPage()
        .appSheet(isPresented: $showAdd) {
            AddKeyPage()
        }
    }

    /// Delete Key, from a row's menu: the confirmation when nothing uses the
    /// key, or why it can't go (connections still sign in with it).
    private func requestDelete(_ key: SSHKeyRecord) {
        switch SSHKeyDeletion.check(key, connections: connections.store, keys: keyStore) {
        case .free: deleting = key
        case .inUse(let names): notice = KeyDeletionNotice(.inUse(names))
        case .unknown: notice = KeyDeletionNotice(.unknown)
        }
    }

    /// The confirmed delete: the private key, then the record.
    private func delete(_ key: SSHKeyRecord) {
        let keychain = keychainHolder.service
        Task {
            do {
                try await SSHKeyDeletion.delete(key, from: keyStore, connections: connections.store) {
                    try await keychain.delete(forRef: $0)
                }
            } catch let failure as SSHKeyDeletion.Failure {
                notice = KeyDeletionNotice(failure)
            } catch {
                notice = KeyDeletionNotice(.keychain(String(describing: error)))
            }
        }
    }
}

/// A key that wasn't deleted, and why: the card that says so.
struct KeyDeletionNotice: Identifiable {
    let id = UUID()
    let failure: SSHKeyDeletion.Failure

    init(_ failure: SSHKeyDeletion.Failure) {
        self.failure = failure
    }
}

extension SSHKeyDeletion.Failure {
    var isInUse: Bool {
        if case .inUse = self { return true }
        return false
    }
}

// MARK: - Key row (§3.4)

/// Shared long-press menu for a key row. The public key was previously
/// UNREACHABLE anywhere in the app — you could generate a device key but
/// never install it on a server.
@MainActor
private func keyRowMenu(_ key: SSHKeyRecord, onDelete: @escaping () -> Void) -> some View {
    Group {
        Button {
            UIPasteboard.general.string = key.publicKey
        } label: {
            Label("Copy Public Key", systemImage: "doc.on.doc")
        }
        .disabled(key.publicKey.isEmpty)
        Button {
            UIPasteboard.general.string = key.fingerprint
        } label: {
            Label("Copy Fingerprint", systemImage: "number")
        }
        .disabled(key.fingerprint.isEmpty)
        ShareLink(item: key.publicKey) {
            Label("Share Public Key", systemImage: "square.and.arrow.up")
        }
        Button(role: .destructive, action: onDelete) {
            Label("Delete Key", systemImage: "trash")
        }
    }
}

struct SSHKeyRow: View {
    let record: SSHKeyRecord

    private var avatar: (gradient: LinearGradient, icon: String) {
        switch record.source {
        case .secureEnclave: return (Ink.avatarSE, "checkmark.shield.fill")
        case .hardware: return (Ink.avatarYubikey, "mediastick")
        default:
            return record.algorithm == .rsa4096 && isStale
                ? (Ink.avatarGray, "key.fill")
                : (Ink.avatarImported, "key.fill")
        }
    }

    private var isStale: Bool {
        guard let lastUsed = record.lastUsedAt else { return false }
        return Date().timeIntervalSince(lastUsed) > 90 * 86400
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .fill(avatar.gradient)
                .frame(width: 36, height: 36)
                .overlay(
                    Image(systemName: avatar.icon)
                        .font(.system(size: 15))
                        .foregroundStyle(isStale ? Ink.primary : Ink.screenBG))

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(record.name)
                        .font(Face.text(14, .medium))
                        .foregroundStyle(Ink.primary)
                        .lineLimit(1)
                    badge
                }
                if !record.fingerprint.isEmpty {
                    Text(record.fingerprint)
                        .font(Face.mono(10.5))
                        .kerning(0.1)
                        .foregroundStyle(Ink.meta)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                if !record.boundHosts.isEmpty {
                    HStack(spacing: 4) {
                        ForEach(record.boundHosts, id: \.self) { host in
                            Text(host)
                                .font(Face.mono(9.5))
                                .foregroundStyle(Ink.keyChipText)
                                .padding(EdgeInsets(top: 1, leading: 6, bottom: 1, trailing: 6))
                                .background(Ink.keyChipBG, in: Capsule())
                        }
                    }
                } else if let lastUsed = record.lastUsedAt, isStale {
                    Text("unused since \(lastUsed.formatted(.dateTime.month(.abbreviated).year()))")
                        .font(Face.text(10.5).italic())
                        .foregroundStyle(Ink.meta)
                }
            }

            Spacer()
            MiniChevron(color: Ink.meta)
                .padding(.top, 10)
        }
        .padding(.vertical, 10)
        .frame(minHeight: 60)
    }

    private var badge: some View {
        let isSE = record.source == .secureEnclave
        return Text(record.badgeText)
            .font(Face.mono(9.5, .bold))
            .kerning(0.57)
            .foregroundStyle(isSE ? Ink.seBadgeText : Ink.secondary)
            .padding(EdgeInsets(top: 2, leading: 6, bottom: 2, trailing: 6))
            .background(isSE ? Ink.seBadgeBG : Ink.chipNeutralBG, in: Capsule())
            .overlay(
                Capsule().strokeBorder(
                    isSE ? Ink.seBadgeBorder : Ink.groupBorder, lineWidth: 1))
    }
}
