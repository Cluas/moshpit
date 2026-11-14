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

    /// What the form holds: the shared model's draft (the NSH form's and
    /// Android's too), so all of them save — and decide whether they can —
    /// the same way.
    @State private var draft: ConnectionDraft
    /// The password and the pasted key: this host's, never the model's.
    @State private var secrets = ConnectionSecrets()
    @State private var saveError: String?
    @State private var showKeyPicker = false

    /// Drops a keychain item the saved connection left (B325), as Home
    /// decides (not while its session runs).
    var releaseSecret: (UUID, String) async -> Void = { _, _ in }

    init(store: ConnectionStore, keychain: KeychainService, existing: ServerConnection? = nil,
         releaseSecret: @escaping (UUID, String) async -> Void = { _, _ in }) {
        self.releaseSecret = releaseSecret
        self.store = store
        self.keychain = keychain
        self.existing = existing
        // A new connection starts from the settings, as the NSH form's
        // draft does (B256): the app's one settings object — the
        // environment's too, which an init can't read.
        _draft = State(initialValue: existing.map(ConnectionDraft.init) ?? ConnectionDraft(newWith: .shared))
    }

    /// Keys that can actually authenticate: everything except hardware-token
    /// ECDSA-sk (its private half lives on the YubiKey).
    private var connectableKeys: [SSHKeyRecord] {
        keyStore.keys.filter { $0.algorithm != .ecdsaSK }
    }

    private var selectedKey: SSHKeyRecord? {
        draft.selectedKeyId.flatMap { id in keyStore.keys.first { $0.id == id } }
    }

    private var canSave: Bool { draft.canSave(keys: keyStore.keys, secrets: secrets) }

    /// What a port must be, under a port field whose text is not one.
    private static let portIssue: LocalizedStringKey = "Must be a whole number from 1 to 65535"

    var body: some View {
        NavigationStack {
            Form {
                FormGroup(title: "CONNECTION") {
                    FieldRow(placeholder: "Name", text: $draft.name)
                        .accessibilityIdentifier("connection-form-name")
                    FieldRow(placeholder: "Host", text: $draft.host)
                        .accessibilityIdentifier("connection-form-host")
                    NumberFieldRow(label: "Port", placeholder: "22", text: $draft.port,
                                   issue: draft.portIsWrong ? Self.portIssue : nil)
                        .accessibilityIdentifier("connection-form-port")
                    FieldRow(placeholder: "Username", text: $draft.username)
                        .accessibilityIdentifier("connection-form-username")
                }

                FormGroup(title: "AUTHENTICATION") {
                    PillSegmentedControl(
                        items: [
                            SegItem(value: AuthMethod.password, label: "Password"),
                            SegItem(value: AuthMethod.key, label: "SSH Key"),
                        ],
                        selection: $draft.authMethod)
                    .accessibilityIdentifier("connection-form-auth")
                    if draft.authMethod == .password {
                        FieldRow(placeholder: "Password", text: $secrets.password, secure: true)
                            .accessibilityIdentifier("connection-form-password")
                    } else {
                        // Always rendered — even with zero saved keys — so there's
                        // always a way into KeyPickerSheet's generate/import CTA
                        // instead of paste-PEM being the only option. Kept as an
                        // independent `if` from the TextEditor below (not folded
                        // into an if/else): MainFlowUITest asserts the PEM editor
                        // appears immediately with an empty key store, with no
                        // extra tap first.
                        ChevronRow(
                            label: "Key",
                            value: selectedKey.map { "\($0.name) · \($0.badgeText)" } ?? String(localized: "Paste PEM")
                        ) {
                            showKeyPicker = true
                        }
                        .accessibilityIdentifier("key-picker-row")
                        if selectedKey == nil {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("Private Key (PEM)")
                                    .font(.footnote)
                                    .foregroundStyle(Ink.meta)
                                TextEditor(text: $secrets.privateKeyPEM)
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
                            .padding(.vertical, 6)
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
                        identifier: "toggle-use-mosh",
                        isOn: $draft.useMosh)
                    // The tuning rows only exist when Mosh does. With
                    // the toggle off they were pure expert noise — and
                    // "Roam on Cellular: ON" under a disabled Mosh was
                    // a lie about what would happen.
                    if draft.useMosh {
                        ValueRow(
                            label: "UDP Port Range",
                            value: "\(settings.udpRangeStart) – \(settings.udpRangeEnd)")
                        Menu {
                            ForEach(PredictMode.allCases, id: \.self) { mode in
                                Button {
                                    draft.predictMode = mode
                                } label: {
                                    Label {
                                        Text(mode.label)
                                    } icon: {
                                        if draft.predictMode == mode {
                                            Image(systemName: "checkmark")
                                        }
                                    }
                                }
                            }
                        } label: {
                            HStack(spacing: 10) {
                                Text("Predict Mode").font(.body).foregroundStyle(Ink.primary)
                                Spacer()
                                Text(draft.predictMode.label).font(.body).foregroundStyle(Ink.meta)
                                MenuChevron()
                            }
                            .contentShape(Rectangle())
                        }
                        .accessibilityIdentifier("connection-form-predict")
                        ToggleRow(
                            label: "Roam on Cellular",
                            subtitle: "Reconnect over 5G/LTE when Wi-Fi drops",
                            identifier: "connection-form-roam",
                            isOn: $draft.roamOnCellular)
                        FieldRow(placeholder: "mosh-server path",
                                 text: $draft.moshServerPath)
                            .accessibilityIdentifier("connection-form-mosh-server")
                    }
                }

                FormGroup(
                    title: "PROXY",
                    footer: "Only unauthenticated SOCKS5 proxies are supported. This proxies the SSH connection only — if Mosh is also enabled above, its UDP session connects directly once bootstrapped and is not proxied."
                ) {
                    ToggleRow(
                        label: "Use SOCKS5 Proxy",
                        subtitle: "Route this connection through a local or corporate SOCKS5 proxy",
                        identifier: "connection-form-socks",
                        isOn: $draft.useSocksProxy)
                    if draft.useSocksProxy {
                        FieldRow(placeholder: "Proxy Host", text: $draft.socksProxyHost)
                            .accessibilityIdentifier("connection-form-proxy-host")
                        NumberFieldRow(label: "Proxy Port", placeholder: "1080", text: $draft.socksProxyPort,
                                       issue: draft.proxyPortIsWrong ? Self.portIssue : nil)
                            .accessibilityIdentifier("connection-form-proxy-port")
                    }
                }

                FormGroup(
                    title: "ADVANCED",
                    footer: "tmux and herdr hold separate, unrelated sessions. If the host doesn't have the one you pick, Moshpit says so and drops to a plain shell — it never quietly attaches the other. With Mosh, herdr runs its own terminal UI; native rendering needs SSH."
                ) {
                    Menu {
                        ForEach(Multiplexer.allCases, id: \.self) { mux in
                            Button {
                                draft.multiplexer = mux
                            } label: {
                                // Checkmark on the current choice — the
                                // menu used to give no clue which one
                                // was active.
                                Label {
                                    Text(mux.label)
                                    Text(mux.subtitle)
                                } icon: {
                                    // Checkmark on the active row, the
                                    // multiplexer's own symbol on the
                                    // others: selection stays legible
                                    // (the menu used to give no clue)
                                    // and the rest become recognisable
                                    // without reading two lowercase
                                    // tool names. The collapsed row
                                    // below carries the current one's
                                    // symbol, so it is never hidden.
                                    Image(systemName: draft.multiplexer == mux
                                          ? "checkmark" : mux.symbol)
                                }
                            }
                        }
                    } label: {
                        HStack(spacing: 10) {
                            Text("Multiplexer").font(.body).foregroundStyle(Ink.primary)
                            Spacer()
                            HStack(spacing: 5) {
                                Image(systemName: draft.multiplexer.symbol)
                                    .font(.system(size: 12, weight: .medium))
                                Text(draft.multiplexer.label).font(.body)
                            }
                            .foregroundStyle(Ink.meta)
                            MenuChevron()
                        }
                        .contentShape(Rectangle())
                    }
                    .accessibilityIdentifier("multiplexer-picker")
                    // The path field follows the choice: it's the one
                    // that skips the capability probe, so offering the
                    // wrong binary's path would be a trap.
                    switch draft.multiplexer {
                    case .tmux:
                        FieldRow(placeholder: "Custom tmux Path", text: $draft.tmuxPath)
                            .accessibilityIdentifier("connection-form-tmux-path")
                    case .herdr:
                        FieldRow(placeholder: "Custom herdr Path", text: $draft.herdrPath)
                            .accessibilityIdentifier("connection-form-herdr-path")
                    case .none:
                        EmptyView()
                    }
                }
            }
            .moshpitForm()
            .navigationTitle(existing == nil ? "Add Connection" : "Edit Connection")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .foregroundStyle(Ink.accent)
                        .accessibilityIdentifier("connection-form-cancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
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
            .appSheet(isPresented: $showKeyPicker) {
                KeyPickerSheet(keys: connectableKeys, selectedKeyId: $draft.selectedKeyId)
            }
        }
        .darkOnlyPage()
    }

    private func save() async {
        do {
            let id = existing?.id
            try await draft.save(existing: existing, store: store, keychain: keychain,
                                 settings: settings, keys: keyStore.keys, secrets: secrets,
                                 release: { ref in if let id { await releaseSecret(id, ref) } })
        } catch {
            saveError = String(describing: error)
            return
        }
        dismiss()
    }
}
