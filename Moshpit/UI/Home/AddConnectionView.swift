import SwiftUI

/// Screen 1 — Add Connection modal form. Also doubles as the editor for an
/// existing connection (Home card → Edit).
struct AddConnectionView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @Environment(SSHKeyStore.self) private var keyStore

    let store: ConnectionStore
    let keychain: KeychainService
    var existing: ServerConnection?

    @State private var name = ""
    @State private var host = ""
    @State private var port = "22"
    @State private var username = ""
    @State private var authMethod: AuthMethod = .password
    @State private var password = ""
    @State private var privateKeyPEM = ""
    /// Managed key picked from SSH Keys; nil = paste-a-PEM mode.
    @State private var selectedKeyId: UUID?
    @State private var useMosh = false
    @State private var predictMode: PredictMode = .adaptive
    @State private var roamOnCellular = true
    /// Defaults to tmux: every connection saved before this picker existed ran
    /// tmux (the form hardcoded it), so a new connection behaving the same way
    /// is the least surprising thing.
    @State private var multiplexer: Multiplexer = .tmux
    @State private var tmuxPath = ""
    @State private var herdrPath = ""
    // Empty by default: a non-nil path SKIPS the capability probe (the user
    // "vouches" for it), so prefilling the Apple-Silicon Homebrew path made
    // every Linux host bypass the graceful mosh degrade and die on a raw
    // "command not found". Empty → `mosh-server` on PATH + probe intact.
    @State private var moshServerPath = ""
    @State private var compressOutput = false
    @State private var saveError: String?

    init(store: ConnectionStore, keychain: KeychainService, existing: ServerConnection? = nil) {
        self.store = store
        self.keychain = keychain
        self.existing = existing
        if let c = existing {
            _name = State(initialValue: c.name)
            _host = State(initialValue: c.host)
            _port = State(initialValue: String(c.port))
            _username = State(initialValue: c.username)
            _authMethod = State(initialValue: c.authMethod)
            _selectedKeyId = State(initialValue: c.sshKeyId)
            _useMosh = State(initialValue: c.connectionProtocol == .mosh)
            _predictMode = State(initialValue: c.predictMode)
            _roamOnCellular = State(initialValue: c.roamOnCellular ?? true)
            _multiplexer = State(initialValue: c.multiplexer)
            _tmuxPath = State(initialValue: c.tmuxPath ?? "")
            _herdrPath = State(initialValue: c.herdrPath ?? "")
            _moshServerPath = State(initialValue: c.moshServerPath ?? "")
            _compressOutput = State(initialValue: c.compression)
        }
    }

    /// Keys that can actually authenticate: everything except hardware-token
    /// ECDSA-sk (its private half lives on the YubiKey).
    private var connectableKeys: [SSHKeyRecord] {
        keyStore.keys.filter { $0.algorithm != .ecdsaSK }
    }

    private var selectedKey: SSHKeyRecord? {
        selectedKeyId.flatMap { id in keyStore.keys.first { $0.id == id } }
    }


    private var canSave: Bool { !name.isEmpty && !host.isEmpty }

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(title: "CONNECTION") {
                            FieldRow(placeholder: "Name", text: $name)
                            FieldRow(placeholder: "Host", text: $host)
                            HStack {
                                Text("Port").font(Face.text(14)).foregroundStyle(Ink.primary)
                                Spacer()
                                TextField("22", text: $port)
                                    .keyboardType(.numberPad)
                                    .font(Face.text(14))
                                    .foregroundStyle(Ink.fixedValue)
                                    .multilineTextAlignment(.trailing)
                                    .frame(width: 80)
                            }
                            .frame(minHeight: Metrics.cellMinHeight)
                            FieldRow(placeholder: "Username", text: $username)
                        }

                        FormGroup(title: "AUTHENTICATION") {
                            PillSegmentedControl(
                                items: [
                                    SegItem(value: AuthMethod.password, label: "Password"),
                                    SegItem(value: AuthMethod.key, label: "SSH Key"),
                                ],
                                selection: $authMethod)
                            if authMethod == .password {
                                FieldRow(placeholder: "Password", text: $password, secure: true)
                            } else {
                                if !connectableKeys.isEmpty {
                                    Menu {
                                        ForEach(connectableKeys) { key in
                                            Button {
                                                selectedKeyId = key.id
                                            } label: {
                                                Label {
                                                    Text(verbatim: "\(key.name) · \(key.badgeText)")
                                                } icon: {
                                                    if selectedKeyId == key.id {
                                                        Image(systemName: "checkmark")
                                                    }
                                                }
                                            }
                                        }
                                        Button {
                                            selectedKeyId = nil
                                        } label: {
                                            Label {
                                                Text("Paste a PEM instead…")
                                            } icon: {
                                                if selectedKeyId == nil {
                                                    Image(systemName: "checkmark")
                                                }
                                            }
                                        }
                                    } label: {
                                        HStack(spacing: 10) {
                                            Text("Key").font(Face.text(14)).foregroundStyle(Ink.primary)
                                            Spacer()
                                            Text(selectedKey.map { "\($0.name) · \($0.badgeText)" } ?? String(localized: "Paste PEM"))
                                                .font(Face.text(14))
                                                .foregroundStyle(Ink.meta)
                                                .lineLimit(1)
                                            MiniChevron()
                                        }
                                        .frame(minHeight: Metrics.cellMinHeight)
                                        .contentShape(Rectangle())
                                    }
                                }
                                if selectedKey == nil {
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text("Private Key (PEM)")
                                            .font(Face.text(11))
                                            .foregroundStyle(Ink.meta)
                                        TextEditor(text: $privateKeyPEM)
                                            .font(Face.mono(11))
                                            .foregroundStyle(Ink.primary)
                                            .scrollContentBackground(.hidden)
                                            .frame(minHeight: 84)
                                            .padding(8)
                                            .background(Ink.terminalBG,
                                                        in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                                            .overlay(
                                                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                                    .strokeBorder(Ink.groupBorder, lineWidth: 1))
                                            .accessibilityIdentifier("private-key-editor")
                                    }
                                    .padding(.vertical, 8)
                                }
                            }
                        }

                        FormGroup(
                            title: "ROAMING · MOSH",
                            footer: "Mosh keeps the session alive across Wi-Fi / 5G handoff and sleep/wake. UDP must be open server-side; the client picks an unused port within range. With tmux, Moshpit attaches to your existing sessions and never creates or restyles them; only sessions you create through Moshpit get its native look (status bar hidden, restored on disconnect)."
                        ) {
                            ToggleRow(
                                label: "Use Mosh",
                                subtitle: "Wrap SSH with mosh-server (UDP)",
                                isOn: $useMosh)
                            // The tuning rows only exist when Mosh does. With
                            // the toggle off they were pure expert noise — and
                            // "Roam on Cellular: ON" under a disabled Mosh was
                            // a lie about what would happen.
                            if useMosh {
                                ValueRow(
                                    label: "UDP Port Range",
                                    value: "\(settings.udpRangeStart) – \(settings.udpRangeEnd)")
                                Menu {
                                    ForEach(PredictMode.allCases, id: \.self) { mode in
                                        Button {
                                            predictMode = mode
                                        } label: {
                                            Label {
                                                Text(mode.label)
                                            } icon: {
                                                if predictMode == mode {
                                                    Image(systemName: "checkmark")
                                                }
                                            }
                                        }
                                    }
                                } label: {
                                    HStack(spacing: 10) {
                                        Text("Predict Mode").font(Face.text(14)).foregroundStyle(Ink.primary)
                                        Spacer()
                                        Text(predictMode.label).font(Face.text(14)).foregroundStyle(Ink.meta)
                                        MiniChevron()
                                    }
                                    .frame(minHeight: Metrics.cellMinHeight)
                                    .contentShape(Rectangle())
                                }
                                ToggleRow(
                                    label: "Roam on Cellular",
                                    subtitle: "Reconnect over 5G/LTE when Wi-Fi drops",
                                    isOn: $roamOnCellular)
                                FieldRow(placeholder: "mosh-server path",
                                         text: $moshServerPath)
                            }
                        }
                        .animation(Motion.settle, value: useMosh)

                        FormGroup(
                            title: "ADVANCED",
                            footer: "tmux and herdr hold separate, unrelated sessions. If the host doesn't have the one you pick, Moshpit says so and drops to a plain shell — it never quietly attaches the other. With Mosh, herdr runs its own terminal UI; native rendering needs SSH."
                        ) {
                            Menu {
                                ForEach(Multiplexer.allCases, id: \.self) { mux in
                                    Button {
                                        multiplexer = mux
                                    } label: {
                                        // Checkmark on the current choice — the
                                        // menu used to give no clue which one
                                        // was active.
                                        Label {
                                            Text(mux.label)
                                            Text(mux.subtitle)
                                        } icon: {
                                            if multiplexer == mux {
                                                Image(systemName: "checkmark")
                                            }
                                        }
                                    }
                                }
                            } label: {
                                HStack(spacing: 10) {
                                    Text("Multiplexer").font(Face.text(14)).foregroundStyle(Ink.primary)
                                    Spacer()
                                    Text(multiplexer.label).font(Face.text(14)).foregroundStyle(Ink.meta)
                                    MiniChevron()
                                }
                                .frame(minHeight: Metrics.cellMinHeight)
                                .contentShape(Rectangle())
                            }
                            .accessibilityIdentifier("multiplexer-picker")
                            // The path field follows the choice: it's the one
                            // that skips the capability probe, so offering the
                            // wrong binary's path would be a trap.
                            switch multiplexer {
                            case .tmux:
                                FieldRow(placeholder: "Custom tmux Path", text: $tmuxPath)
                            case .herdr:
                                FieldRow(placeholder: "Custom herdr Path", text: $herdrPath)
                            case .none:
                                EmptyView()
                            }
                            ToggleRow(label: "Compress Output", isOn: $compressOutput)
                        }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle(existing == nil ? "Add Connection" : "Edit Connection")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .font(Face.text(15))
                        .foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
                        .font(Face.text(15, .semibold))
                        .foregroundStyle(canSave ? Ink.accent : Ink.disabledNav)
                        .disabled(!canSave)
                        .accessibilityIdentifier("save-connection")
                }
            }
            .moshpitCard(isPresented: Binding(
                get: { saveError != nil }, set: { if !$0 { saveError = nil } }
            )) {
                MoshpitNoticeCard(title: "Could not save", message: saveError ?? "") {
                    saveError = nil
                }
            }
        }
        .preferredColorScheme(.dark)
    }

    private func save() async {
        var connection = existing ?? ServerConnection()
        connection.name = name
        connection.host = host
        connection.port = Int(port) ?? 22
        connection.sshPort = Int(port) ?? 22
        connection.username = username
        connection.authMethod = authMethod
        connection.connectionProtocol = useMosh ? .mosh : .ssh
        connection.moshPortRangeStart = settings.udpRangeStart
        connection.moshPortRangeEnd = settings.udpRangeEnd
        // Per-connection mosh-server (`--server`). Empty → nil → bootstrap uses
        // `mosh-server` on PATH (more portable than a hardcoded absolute path).
        connection.moshServerPath = moshServerPath.isEmpty ? nil : moshServerPath
        connection.predictMode = predictMode
        connection.roamOnCellular = roamOnCellular
        connection.tmuxPath = tmuxPath.isEmpty ? nil : tmuxPath
        connection.herdrPath = herdrPath.isEmpty ? nil : herdrPath
        connection.compression = compressOutput
        // Also keeps the legacy `useTmux` flag in sync — see `multiplexer`.
        connection.multiplexer = multiplexer

        do {
            if authMethod == .key, let key = selectedKey {
                // Managed key: point at the key's own keychain blob; the raw
                // algorithm tells SSHService how to interpret it (SE keys are
                // dataRepresentation handles, not PEM).
                connection.keychainRef = key.keychainRef
                connection.sshKeyId = key.id
                connection.sshKeyAlgorithmRaw = key.algorithm.rawValue
            } else {
                connection.sshKeyId = nil
                connection.sshKeyAlgorithmRaw = nil
                let secret = authMethod == .password ? password : privateKeyPEM
                if !secret.isEmpty {
                    let ref = connection.keychainRef ?? keychain.generateRef()
                    if authMethod == .password {
                        try await keychain.savePassword(secret, forRef: ref, requireBiometry: false)
                    } else {
                        try await keychain.savePrivateKey(secret, forRef: ref, requireBiometry: false)
                    }
                    connection.keychainRef = ref
                }
            }
            // Drop any cached (possibly stale) credential so the new one is used.
            await SSHService.shared.clearCachedSecret(for: connection.id)
        } catch {
            saveError = String(describing: error)
            return
        }

        if existing == nil {
            store.add(connection)
        } else {
            store.update(connection)
        }
        dismiss()
    }
}
