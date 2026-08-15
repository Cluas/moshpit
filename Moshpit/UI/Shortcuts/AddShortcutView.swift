import SwiftUI

/// Screen 7 — Add Shortcut modal. Live preview, Key Combo / Text / Command,
/// modifier toggles, payload, display chip config, optional host scope.
struct AddShortcutView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(ShortcutStore.self) private var store
    @Environment(ConnectionStoreHolder.self) private var connections

    var existing: TerminalShortcut?

    @State private var kind: ShortcutKind = .keyCombo
    @State private var modifiers: Set<ShortcutModifier> = []
    @State private var key = ""
    @State private var payload = ""
    @State private var appendReturn = false
    @State private var repeatOnHold = true
    @State private var chipLabel = ""
    @State private var summary = ""
    @State private var colorId = "accent"
    @State private var scopeHosts: Set<String> = []
    @State private var onlyInTmux = false

    init(existing: TerminalShortcut? = nil) {
        self.existing = existing
        if let sc = existing {
            _kind = State(initialValue: sc.kind)
            _modifiers = State(initialValue: sc.modifiers)
            _key = State(initialValue: sc.key)
            _payload = State(initialValue: sc.payload)
            _appendReturn = State(initialValue: sc.appendReturn)
            _repeatOnHold = State(initialValue: sc.repeatOnHold)
            _chipLabel = State(initialValue: sc.chipLabel)
            _summary = State(initialValue: sc.summary)
            _colorId = State(initialValue: sc.colorId)
            _scopeHosts = State(initialValue: sc.scopeHosts)
            _onlyInTmux = State(initialValue: sc.onlyInTmux)
        }
    }

    private var canSave: Bool {
        !chipLabel.isEmpty && (kind == .keyCombo ? (!key.isEmpty || !payload.isEmpty) : !payload.isEmpty)
    }

    private var comboCap: String {
        (modifiers.sorted().map(\.symbol) + [key.uppercased()])
            .filter { !$0.isEmpty }
            .joined(separator: " ")
    }

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(
                            title: "PREVIEW",
                            footer: "Live preview of your input below. Saved shortcuts land in the Custom group of the Shortcuts toolbar."
                        ) {
                            previewRow
                        }

                        FormGroup(
                            title: "TYPE",
                            footer: "Key Combo: sends modifiers + key. Text: types a string as-is. Command: submits one command with Return."
                        ) {
                            PillSegmentedControl(
                                items: [
                                    SegItem(value: ShortcutKind.keyCombo, label: "Key Combo", systemImage: "keyboard"),
                                    SegItem(value: ShortcutKind.text, label: "Text", systemImage: "text.alignleft"),
                                    SegItem(value: ShortcutKind.command, label: "Command", systemImage: "terminal"),
                                ],
                                selection: $kind)
                        }

                        if kind == .keyCombo {
                            FormGroup(
                                title: "QUICK KEYS",
                                footer: "Tap a preset to fill the trigger — tmux prefixes, control chords, special & navigation keys, F-keys. Then tweak the chip label / color below."
                            ) {
                                quickKeys
                            }

                            FormGroup(
                                title: "TRIGGER",
                                footer: "Tap modifiers to toggle them and enter a single character as the main key; the mobile keyboard triggers the raw scancode automatically."
                            ) {
                                Text("Modifiers")
                                    .font(Face.text(14))
                                    .foregroundStyle(Ink.primary)
                                    .frame(minHeight: 32, alignment: .leading)
                                modifierKeys
                                HStack {
                                    TextField("", text: $key, prompt: Text("Main key").foregroundStyle(Ink.placeholder))
                                        .font(Face.text(14))
                                        .foregroundStyle(Ink.primary)
                                        .autocorrectionDisabled()
                                        .textInputAutocapitalization(.never)
                                        .accessibilityIdentifier("shortcut-key")
                                        .onChange(of: key) { _, newValue in
                                            if newValue.count > 6 { key = String(newValue.prefix(6)) }
                                        }
                                    Spacer()
                                    if !comboCap.isEmpty {
                                        Text(comboCap)
                                            .font(Face.mono(11, .semibold))
                                            .foregroundStyle(Ink.primary)
                                            .padding(EdgeInsets(top: 5, leading: 9, bottom: 5, trailing: 9))
                                            .background(Ink.chipNeutralBG, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                                    }
                                }
                                .frame(minHeight: Metrics.cellMinHeight)
                            }
                        }

                        FormGroup(
                            title: "ACTION",
                            footer: "In Key Combo mode the payload is the escape sequence sent to the PTY, honoring the transport transcription rules."
                        ) {
                            TextEditor(text: $payload)
                                .font(Face.mono(13))
                                .foregroundStyle(Ink.primary)
                                .scrollContentBackground(.hidden)
                                .frame(minHeight: 64)
                                .padding(8)
                                .background(Ink.terminalBG,
                                            in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                                .overlay(
                                    RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                        .strokeBorder(Ink.groupBorder, lineWidth: 1))
                                .padding(.vertical, 6)
                                .overlay(alignment: .topLeading) {
                                    if payload.isEmpty {
                                        Text("Payload (text or command)")
                                            .font(Face.mono(13))
                                            .foregroundStyle(Ink.placeholder)
                                            .padding(.top, 14)
                                            .padding(.leading, 5)
                                            .allowsHitTesting(false)
                                    }
                                }
                            ToggleRow(label: "Append Return", subtitle: "Automatically appends ⏎ to submit", isOn: $appendReturn)
                            ToggleRow(label: "Repeat on hold", subtitle: "Repeats at 30/s after a 0.4s hold", isOn: $repeatOnHold)
                        }

                        FormGroup(
                            title: "DISPLAY",
                            footer: "Chip Label is capped at 6 characters; Description appears in the edit list to keep shortcuts recognizable."
                        ) {
                            HStack {
                                Text("Chip Label").font(Face.text(14)).foregroundStyle(Ink.primary)
                                Spacer()
                                TextField("", text: $chipLabel, prompt: Text("⌘B").foregroundStyle(Ink.placeholder))
                                    .font(Face.mono(14))
                                    .foregroundStyle(Ink.primary)
                                    .multilineTextAlignment(.trailing)
                                    .frame(width: 120)
                                    .accessibilityIdentifier("shortcut-chip-label")
                                    .onChange(of: chipLabel) { _, newValue in
                                        if newValue.count > 6 { chipLabel = String(newValue.prefix(6)) }
                                    }
                            }
                            .frame(minHeight: Metrics.cellMinHeight)
                            HStack {
                                Text("Description").font(Face.text(14)).foregroundStyle(Ink.primary)
                                Spacer()
                                TextField("", text: $summary, prompt: Text("multiplexer prefix").foregroundStyle(Ink.placeholder))
                                    .font(Face.text(14))
                                    .foregroundStyle(Ink.primary)
                                    .multilineTextAlignment(.trailing)
                                    .accessibilityIdentifier("shortcut-description")
                            }
                            .frame(minHeight: Metrics.cellMinHeight)
                            HStack {
                                Text("Color").font(Face.text(14)).foregroundStyle(Ink.primary)
                                Spacer()
                                ColorSwatchRow(
                                    colors: [
                                        ("gray", Ink.chipNeutralBG),
                                        ("accent", Ink.customChipText),
                                        ("mosh", Ink.mosh),
                                        ("amber", Ink.termAmber),
                                    ],
                                    selection: $colorId,
                                    diameter: 26)
                            }
                            .frame(minHeight: Metrics.cellMinHeight)
                        }

                        FormGroup(
                            title: "SCOPE",
                            titleSuffix: "(optional)",
                            footer: "Available everywhere when nothing is selected. When bound, it appears only in the toolbar of the selected hosts, saving slots in the 12-slot budget."
                        ) {
                            HostChipsRow(hosts: hostNames, selected: $scopeHosts)
                            ToggleRow(label: "Only in a multiplexer", subtitle: "Show only while the session runs tmux or herdr", isOn: $onlyInTmux)
                        }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle(existing == nil ? "Add Shortcut" : "Edit Shortcut")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .font(Face.text(15))
                        .foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save() }
                        .font(Face.text(15, .semibold))
                        .foregroundStyle(canSave ? Ink.accent : Ink.disabledNav)
                        .disabled(!canSave)
                }
            }
        }
        .preferredColorScheme(.dark)
    }

    private var hostNames: [String] {
        connections.store.connections.map(\.displayName)
    }

    // MARK: Preview row

    private var previewRow: some View {
        HStack(spacing: 10) {
            ShortcutChip(label: chipLabel.isEmpty ? "—" : chipLabel, custom: true, size: 12)
            Text("→").font(Face.mono(11)).foregroundStyle(Ink.meta)
            Text(previewSummary)
                .font(Face.mono(11.5))
                .foregroundStyle(Ink.secondary)
                .lineLimit(1)
            Spacer()
            Text("SLOT \(store.toolbarCount)/\(ShortcutStore.toolbarLimit)")
                .font(Face.mono(9.5))
                .kerning(1.14)
                .foregroundStyle(Ink.meta)
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
        .frame(minHeight: Metrics.cellMinHeight)
        .background(Ink.groupRaised, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.cardBorder, lineWidth: 1))
        .padding(.vertical, 8)
    }

    private var previewSummary: String {
        let name = summary.isEmpty ? String(localized: "unnamed") : summary
        switch kind {
        case .keyCombo:
            return comboCap.isEmpty ? name : String(localized: "\(name) · sends \(comboCap)")
        case .text:
            return payload.isEmpty ? name : String(localized: "\(name) · types \"\(payload)\"")
        case .command:
            return payload.isEmpty ? name : String(localized: "\(name) · runs \(payload)")
        case .dpad, .arrows, .paste, .scroll, .ctrl, .mic, .image:
            return name   // not user-creatable; here only for exhaustiveness
        }
    }

    // MARK: Modifier keys

    private var modifierKeys: some View {
        HStack(spacing: 6) {
            ForEach(ShortcutModifier.allCases, id: \.self) { mod in
                let on = modifiers.contains(mod)
                Button {
                    if on { modifiers.remove(mod) } else { modifiers.insert(mod) }
                } label: {
                    Text(mod.symbol)
                        .font(Face.mono(13, .semibold))
                        .foregroundStyle(on ? Ink.modkeyOnText : Ink.secondary)
                        .frame(width: 38, height: 32)
                        .background(
                            on ? Ink.modkeyOnBG : Ink.shortcutKeyBG,
                            in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                .strokeBorder(
                                    on ? Ink.modkeyOnBorder : Ink.groupBorder,
                                    lineWidth: 1))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.bottom, 8)
    }

    // MARK: Quick keys (Attach-style preset picker)

    /// One tappable preset that fills the trigger fields in one tap.
    private struct KeyPreset: Identifiable {
        let label: String          // button text
        let chip: String           // chip label to fill
        var mods: Set<ShortcutModifier> = []
        var key: String = ""
        var payload: String = ""   // escape sequence (`\e…` honored by unescape)
        var summary: String = ""
        var id: String { label }
    }

    private static let presetGroups: [(String, [KeyPreset])] = [
        ("PREFIX", [
            KeyPreset(label: "PREFIX", chip: "C-b", mods: [.ctrl], key: "b", summary: "multiplexer prefix"),
            KeyPreset(label: "C-c", chip: "^C", mods: [.ctrl], key: "c", summary: "Interrupt"),
            KeyPreset(label: "C-d", chip: "^D", mods: [.ctrl], key: "d", summary: "EOF"),
            KeyPreset(label: "C-z", chip: "^Z", mods: [.ctrl], key: "z", summary: "Suspend"),
            KeyPreset(label: "C-\\", chip: "^\\", mods: [.ctrl], key: "\\", summary: "Quit"),
            KeyPreset(label: "CLEAR", chip: "^L", mods: [.ctrl], key: "l", summary: "Clear"),
        ]),
        ("CONTROL", [
            KeyPreset(label: "C-a", chip: "^A", mods: [.ctrl], key: "a", summary: "Line start"),
            KeyPreset(label: "C-e", chip: "^E", mods: [.ctrl], key: "e", summary: "Line end"),
            KeyPreset(label: "C-r", chip: "^R", mods: [.ctrl], key: "r", summary: "Search"),
            KeyPreset(label: "C-w", chip: "^W", mods: [.ctrl], key: "w", summary: "Del word"),
            KeyPreset(label: "C-u", chip: "^U", mods: [.ctrl], key: "u", summary: "Del line"),
            KeyPreset(label: "C-k", chip: "^K", mods: [.ctrl], key: "k", summary: "Kill EOL"),
            KeyPreset(label: "C-y", chip: "^Y", mods: [.ctrl], key: "y", summary: "Yank"),
            KeyPreset(label: "C-p", chip: "^P", mods: [.ctrl], key: "p", summary: "Prev"),
            KeyPreset(label: "C-n", chip: "^N", mods: [.ctrl], key: "n", summary: "Next"),
        ]),
        ("SPECIAL", [
            KeyPreset(label: "TAB", chip: "tab", key: "tab", summary: "Tab"),
            KeyPreset(label: "S-Tab", chip: "⇤", payload: "\\e[Z", summary: "Back-tab"),
            KeyPreset(label: "Enter", chip: "⏎", key: "return", summary: "Return"),
            KeyPreset(label: "Esc", chip: "esc", key: "esc", summary: "Escape"),
            KeyPreset(label: "Del", chip: "⌦", payload: "\\e[3~", summary: "Forward delete"),
            KeyPreset(label: "BSpace", chip: "⌫", payload: "\u{7f}", summary: "Backspace"),
        ]),
        ("NAVIGATION", [
            KeyPreset(label: "←", chip: "←", key: "left", summary: "Left"),
            KeyPreset(label: "↓", chip: "↓", key: "down", summary: "Down"),
            KeyPreset(label: "↑", chip: "↑", key: "up", summary: "Up"),
            KeyPreset(label: "→", chip: "→", key: "right", summary: "Right"),
            KeyPreset(label: "Home", chip: "Home", payload: "\\e[H", summary: "Home"),
            KeyPreset(label: "End", chip: "End", payload: "\\e[F", summary: "End"),
            KeyPreset(label: "PgUp", chip: "PgUp", payload: "\\e[5~", summary: "Page up"),
            KeyPreset(label: "PgDn", chip: "PgDn", payload: "\\e[6~", summary: "Page down"),
        ]),
        ("F-KEYS", [
            KeyPreset(label: "F1", chip: "F1", payload: "\\eOP"), KeyPreset(label: "F2", chip: "F2", payload: "\\eOQ"),
            KeyPreset(label: "F3", chip: "F3", payload: "\\eOR"), KeyPreset(label: "F4", chip: "F4", payload: "\\eOS"),
            KeyPreset(label: "F5", chip: "F5", payload: "\\e[15~"), KeyPreset(label: "F6", chip: "F6", payload: "\\e[17~"),
            KeyPreset(label: "F7", chip: "F7", payload: "\\e[18~"), KeyPreset(label: "F8", chip: "F8", payload: "\\e[19~"),
            KeyPreset(label: "F9", chip: "F9", payload: "\\e[20~"), KeyPreset(label: "F10", chip: "F10", payload: "\\e[21~"),
            KeyPreset(label: "F11", chip: "F11", payload: "\\e[23~"), KeyPreset(label: "F12", chip: "F12", payload: "\\e[24~"),
        ]),
    ]

    @ViewBuilder
    private var quickKeys: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Self.presetGroups, id: \.0) { group in
                VStack(alignment: .leading, spacing: 8) {
                    Text(group.0)
                        .font(Face.mono(10, .semibold)).kerning(1.4)
                        .foregroundStyle(Ink.meta)
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 4), spacing: 8) {
                        ForEach(group.1) { preset in
                            Button { apply(preset) } label: {
                                Text(preset.label)
                                    .font(Face.mono(12, .semibold))
                                    .foregroundStyle(Ink.primary)
                                    .lineLimit(1)
                                    .frame(maxWidth: .infinity, minHeight: 40)
                                    .background(Ink.shortcutKeyBG,
                                                in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                                    .overlay(RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                        .strokeBorder(Ink.groupBorder, lineWidth: 1))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
        .padding(.vertical, 8)
    }

    private func apply(_ preset: KeyPreset) {
        modifiers = preset.mods
        key = preset.key
        payload = preset.payload
        chipLabel = preset.chip
        if summary.isEmpty { summary = preset.summary }
    }

    // MARK: Save

    private func save() {
        var sc = existing ?? TerminalShortcut()
        sc.kind = kind
        sc.modifiers = modifiers
        sc.key = key
        sc.payload = payload
        sc.appendReturn = appendReturn
        sc.repeatOnHold = repeatOnHold
        sc.chipLabel = chipLabel
        sc.summary = summary.isEmpty ? chipLabel : summary
        sc.colorId = colorId
        sc.scopeHosts = scopeHosts
        sc.onlyInTmux = onlyInTmux
        if existing == nil {
            store.add(sc)
        } else {
            store.update(sc)
        }
        dismiss()
    }
}

/// Tiny environment wrapper so sheets can reach the ConnectionStore without
/// threading it through every initializer.
@Observable
final class ConnectionStoreHolder {
    let store: ConnectionStore
    init(store: ConnectionStore) { self.store = store }
}
