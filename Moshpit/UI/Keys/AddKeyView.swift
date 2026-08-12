import SwiftUI
import UniformTypeIdentifiers

/// Screen 9 — Add Key modal. Generate / Import / Hardware methods,
/// algorithm choice with REC badge, passphrase strength, Face ID + Secure
/// Enclave toggles, optional host binding with fingerprint preview.
struct AddKeyView: View {
    enum Method: String, CaseIterable { case generate, importKey, hardware }
    /// Which text field a picked file's contents should land in — one
    /// `.fileImporter` shared by both "Import File…" buttons rather than two
    /// independent ones.
    private enum ImportTarget { case privateKey, publicKey }

    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ConnectionStoreHolder.self) private var connections
    @Environment(KeychainServiceHolder.self) private var keychainHolder

    /// Fired with the new record right before this view dismisses itself —
    /// lets a caller (e.g. `KeyPickerSheet`) auto-select a key generated
    /// inline instead of leaving the picker on whatever it showed before.
    var onCreated: ((SSHKeyRecord) -> Void)? = nil

    @State private var method: Method = .generate
    @State private var name = ""
    @State private var algorithm: SSHKeyAlgorithm = .ed25519
    @State private var comment = ""
    @State private var passphrase = ""
    @State private var confirmPassphrase = ""
    @State private var requireFaceID = true
    @State private var storeInSecureEnclave = false
    @State private var boundHosts: Set<String> = []
    @State private var importPEM = ""
    @State private var importPublicLine = ""
    @State private var generatedFingerprint: String?
    @State private var working = false
    @State private var errorMessage: String?
    /// Presentation and destination are deliberately SEPARATE pieces of state.
    ///
    /// Driving `.fileImporter(isPresented:)` off `importTarget != nil` looks
    /// tidier and silently loses every file: SwiftUI sets `isPresented` false
    /// as it dismisses the picker, which ran the binding's setter and cleared
    /// the target BEFORE the completion handler read it — so the file was read
    /// and then thrown away with no destination and no error. Keeping the flag
    /// separate means the target cannot be clobbered by the dismissal, and the
    /// handler has no "nowhere to put this" case left to swallow.
    @State private var isImportingFile = false
    @State private var importTarget: ImportTarget = .privateKey

    private var effectiveAlgorithm: SSHKeyAlgorithm {
        storeInSecureEnclave ? .seP256 : algorithm
    }

    private var canSubmit: Bool {
        guard !name.isEmpty, !working else { return false }
        switch method {
        case .generate:
            return passphrase == confirmPassphrase
        case .importKey:
            return !importPEM.isEmpty
        case .hardware:
            return false
        }
    }

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(
                            title: "METHOD",
                            footer: "Generates the key pair on device. The private key can optionally be held in the Secure Enclave — used only for signing, never leaving the chip."
                        ) {
                            PillSegmentedControl(
                                items: [
                                    SegItem(value: Method.generate, label: "Generate", systemImage: "plus.circle"),
                                    SegItem(value: Method.importKey, label: "Import", systemImage: "doc"),
                                    SegItem(value: Method.hardware, label: "Hardware", systemImage: "lock"),
                                ],
                                selection: $method)
                        }

                        switch method {
                        case .generate: generateGroups
                        case .importKey: importGroups
                        case .hardware:
                            FormGroup(title: "HARDWARE KEY") {
                                Text("ECDSA-sk keys live on hardware tokens such as a YubiKey. iOS cannot enumerate them directly yet; paste the public key line (sk-ecdsa-sha2-nistp256@openssh.com …) under Import.")
                                    .font(Face.text(12))
                                    .foregroundStyle(Ink.tertiary)
                                    .lineSpacing(3)
                                    .padding(.vertical, 12)
                            }
                        }

                        bindGroup
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle("Add Key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .font(Face.text(15))
                        .foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(method == .importKey ? "Import" : "Generate") {
                        Task { await submit() }
                    }
                    .font(Face.text(15, .semibold))
                    .foregroundStyle(canSubmit ? Ink.accent : Ink.disabledNav)
                    .disabled(!canSubmit)
                }
            }
            .moshpitCard(isPresented: Binding(
                get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } }
            )) {
                MoshpitNoticeCard(icon: "key.slash.fill", title: "Key error",
                                  message: errorMessage ?? "") {
                    errorMessage = nil
                }
            }
            .fileImporter(
                isPresented: $isImportingFile,
                allowedContentTypes: [.data]
            ) { result in
                handleImportedFile(result)
            }
        }
        .preferredColorScheme(.dark)
    }

    /// Reads a file picked via either "Import File…" button into the field
    /// `importTarget` names. `.fileImporter` hands back a security-scoped
    /// URL — SwiftUI has no `asCopy`-style option, so the access window has
    /// to be opened and closed by hand around the read.
    private func handleImportedFile(_ result: Result<URL, Error>) {
        do {
            let url = try result.get()
            guard url.startAccessingSecurityScopedResource() else {
                throw SSHKeyFactory.KeyError.unsupported("couldn't access the selected file")
            }
            defer { url.stopAccessingSecurityScopedResource() }
            let text = try SSHKeyFactory.decodeImportedText(try Data(contentsOf: url))
            switch importTarget {
            case .privateKey: importPEM = text
            case .publicKey: importPublicLine = text
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: Generate

    @ViewBuilder
    private var generateGroups: some View {
        FormGroup(
            title: "KEY DETAILS",
            footer: "ED25519 recommended: 32-byte keys, fast signatures, no practical break to date."
        ) {
            FieldRow(placeholder: "Name", text: $name)
            Text("Algorithm")
                .font(Face.text(14))
                .foregroundStyle(Ink.primary)
                .frame(minHeight: 32, alignment: .leading)
            PillSegmentedControl(
                items: [
                    SegItem(value: SSHKeyAlgorithm.ed25519, label: "ED25519", trailingBadge: "REC"),
                    SegItem(value: SSHKeyAlgorithm.ecdsaSK, label: "ECDSA-sk"),
                    SegItem(value: SSHKeyAlgorithm.rsa4096, label: "RSA-4096"),
                ],
                selection: $algorithm)
            FieldRow(placeholder: "Comment", text: $comment, mono: true)
        }

        FormGroup(
            title: "PROTECTION",
            footer: "Secure Enclave keys are limited to ECDSA-P256; the algorithm adjusts automatically when SE is enabled."
        ) {
            FieldRow(placeholder: "Passphrase", text: $passphrase, secure: true)
            FieldRow(placeholder: "Confirm Passphrase", text: $confirmPassphrase, secure: true)
            StrengthMeter(strength: SSHKeyFactory.passphraseStrength(passphrase))
            ToggleRow(label: "Require Face ID", subtitle: "Face ID confirmation before every signature", isOn: $requireFaceID)
            ToggleRow(label: "Store in Secure Enclave", subtitle: "Hardware-backed · private key cannot be exported", isOn: $storeInSecureEnclave)
        }
    }

    // MARK: Import

    @ViewBuilder
    private var importGroups: some View {
        FormGroup(title: "KEY DETAILS") {
            FieldRow(placeholder: "Name", text: $name)
            FieldRow(placeholder: "Comment", text: $comment, mono: true)
        }
        FormGroup(
            title: "PRIVATE KEY",
            footer: "Paste an OpenSSH / PEM private key; the public key line is used to compute the fingerprint (optional)."
        ) {
            TextEditor(text: $importPEM)
                .font(Face.mono(10.5))
                .foregroundStyle(Ink.primary)
                .scrollContentBackground(.hidden)
                .frame(minHeight: 96)
                .padding(8)
                .background(Ink.terminalBG,
                            in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                        .strokeBorder(Ink.groupBorder, lineWidth: 1))
                .padding(.vertical, 6)
                .overlay(alignment: .topLeading) {
                    if importPEM.isEmpty {
                        Text(verbatim: "-----BEGIN OPENSSH PRIVATE KEY-----")
                            .font(Face.mono(10.5))
                            .foregroundStyle(Ink.placeholder)
                            .padding(.top, 14)
                            .padding(.leading, 5)
                            .allowsHitTesting(false)
                    }
                }
            Button {
                importTarget = .privateKey
                isImportingFile = true
            } label: {
                Label("Import File…", systemImage: "folder")
                    .font(Face.text(13))
            }
            .buttonStyle(.plain)
            .foregroundStyle(Ink.accent)
            .padding(.bottom, 6)

            HStack(spacing: 8) {
                FieldRow(placeholder: "Public key line (optional)", text: $importPublicLine, mono: true)
                Button {
                    importTarget = .publicKey
                    isImportingFile = true
                } label: {
                    Image(systemName: "folder")
                        .font(.system(size: 13))
                }
                .buttonStyle(.plain)
                .foregroundStyle(Ink.accent)
            }
            ToggleRow(label: "Require Face ID", subtitle: "Face ID confirmation before every signature", isOn: $requireFaceID)
        }
    }

    // MARK: Bind to hosts

    private var bindGroup: some View {
        FormGroup(
            title: "BIND TO HOSTS",
            titleSuffix: "(optional)",
            footer: "When bound, this key is used only for the selected hosts by default; change it anytime in the SSH Keys detail page."
        ) {
            HostChipsRow(
                hosts: connections.store.connections.map(\.displayName),
                selected: $boundHosts)
            VStack(alignment: .leading, spacing: 4) {
                Text("PREVIEW · SHA256")
                    .font(Face.mono(9.5))
                    .kerning(1.14)
                    .foregroundStyle(Ink.meta)
                Text(generatedFingerprint ?? String(localized: "—:—:—:—:—:—:—:—:—:—  (shown after generation)"))
                    .font(Face.mono(10.5))
                    .foregroundStyle(Ink.fpPreviewText)
                    .lineLimit(1)
            }
            .padding(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Ink.moshPillBG, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                    .strokeBorder(Ink.moshPillBorder, lineWidth: 1))
            .padding(.vertical, 10)
        }
    }

    // MARK: Submit

    private func submit() async {
        working = true
        defer { working = false }
        do {
            let generated: SSHKeyFactory.Generated
            if method == .importKey {
                generated = try SSHKeyFactory.importKey(
                    privatePEM: importPEM,
                    publicKeyLine: importPublicLine.isEmpty ? nil : importPublicLine,
                    comment: comment)
            } else {
                generated = try SSHKeyFactory.generate(
                    algorithm: effectiveAlgorithm,
                    comment: comment.isEmpty ? name : comment)
            }
            generatedFingerprint = generated.fingerprint

            let keychain = keychainHolder.service
            let ref = keychain.generateRef()
            try await keychain.saveSecret(
                generated.privateBlob,
                forRef: ref,
                requireBiometry: requireFaceID)

            let record = SSHKeyRecord(
                name: name,
                algorithm: generated.algorithm,
                comment: comment,
                fingerprint: generated.fingerprint,
                publicKey: generated.publicKeyLine,
                source: method == .importKey
                    ? .imported
                    : (storeInSecureEnclave ? .secureEnclave : .generated),
                boundHosts: Array(boundHosts),
                keychainRef: ref,
                requireBiometry: requireFaceID)
            keyStore.add(record)
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

/// Environment wrapper for the KeychainService actor.
@Observable
final class KeychainServiceHolder {
    let service: KeychainService
    init(service: KeychainService) { self.service = service }
}
