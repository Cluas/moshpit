import SwiftUI

/// Screen 8 — SSH Keys list. Secure Enclave device keys on top, imported
/// keys below, each with algo badge + SHA256 fingerprint + host chips.
struct SSHKeysView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore

    @State private var showAdd = false

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(
                            title: "THIS DEVICE · SECURE ENCLAVE",
                            footer: "Hardware-backed keys cannot be exported; every signature triggers Face ID."
                        ) {
                            if keyStore.deviceKeys.isEmpty {
                                Text("No device key yet — generate one with ＋")
                                    .font(Face.text(12))
                                    .foregroundStyle(Ink.tertiary)
                                    .frame(minHeight: Metrics.cellMinHeight)
                            } else {
                                ForEach(keyStore.deviceKeys) { key in
                                    SSHKeyRow(record: key)
                                        .contextMenu { keyRowMenu(key, store: keyStore) }
                                }
                            }
                        }

                        FormGroup(title: "IMPORTED") {
                            if keyStore.importedKeys.isEmpty {
                                Text("No imported keys")
                                    .font(Face.text(12))
                                    .foregroundStyle(Ink.tertiary)
                                    .frame(minHeight: Metrics.cellMinHeight)
                            } else {
                                ForEach(keyStore.importedKeys) { key in
                                    SSHKeyRow(record: key)
                                        .contextMenu { keyRowMenu(key, store: keyStore) }
                                }
                            }
                        }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle("SSH Keys")
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
            AddKeyView()
        }
    }
}

// MARK: - Key row (§3.4)

/// Shared long-press menu for a key row. The public key was previously
/// UNREACHABLE anywhere in the app — you could generate a device key but
/// never install it on a server.
@MainActor
private func keyRowMenu(_ key: SSHKeyRecord, store: SSHKeyStore) -> some View {
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
        Button(role: .destructive) {
            store.remove(id: key.id)
        } label: {
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
