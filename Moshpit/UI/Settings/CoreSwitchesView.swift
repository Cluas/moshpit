import SwiftUI

/// The runtime switches between the app's Swift modules and the Rust core's
/// replacements, and between SwiftUI pages and their NSH ones
/// (docs/nsh/DESIGN.md §3.6). Hidden: Settings, tap the version line seven
/// times. Tooling for the developer and the Canary group, not product UI —
/// so its words are English only, kept out of the string catalog with
/// `verbatim:`.
struct CoreSwitchesView: View {
    @Environment(\.dismiss) private var dismiss
    @AppStorage(RustSessionSwitch.switchKey) private var rustSessions = false
    @AppStorage(ResumeInPlaceSwitch.switchKey) private var resumeInPlace = false
    @AppStorage(RustStoreSwitch.switchKey) private var rustStore = false
    @AppStorage(CoreTermSwitch.switchKey) private var coreTerminal = false
    /// The NSH screens' switches (``NSHScreen/switchKey``), as stored.
    @State private var nshScreens = Self.storedSwitches()

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Toggle(isOn: $rustSessions) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "Rust sessions")
                                .font(.body).foregroundStyle(Ink.primary)
                            Text(verbatim: "Connects through the Rust core's session layer instead of the Swift transports. From the next connection on.")
                                .font(.footnote).foregroundStyle(Ink.meta)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .tint(Ink.accent)
                    .disabled(Self.sessionsLocked)
                    .accessibilityIdentifier("core-switch-rust-sessions")
                    Toggle(isOn: $resumeInPlace) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "Resume in place")
                                .font(.body).foregroundStyle(Ink.primary)
                            Text(verbatim: "Back from the background, a Rust session asks its connection once and keeps it when it answers, instead of reconnecting after 20 s away. From the next return to the app on.")
                                .font(.footnote).foregroundStyle(Ink.meta)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .tint(Ink.accent)
                    .disabled(!RustSessionSwitch.available
                              || UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[ResumeInPlaceSwitch.switchKey] != nil)
                    .accessibilityIdentifier("core-switch-resume-in-place")
                } header: {
                    Text(verbatim: "SESSIONS")
                        .font(Face.mono(10.5, .semibold))
                        .kerning(1.2)
                        .foregroundStyle(Ink.sectionTitle)
                } footer: {
                    Text(verbatim: Self.sessionsNote).moshpitFooter()
                }
                .moshpitRows()

                Section {
                    Toggle(isOn: $coreTerminal) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "Rust terminal core")
                                .font(.body).foregroundStyle(Ink.primary)
                            Text(verbatim: "Draws new terminals with the shared Rust core (wezterm-term) instead of SwiftTerm. From the next terminal on.")
                                .font(.footnote).foregroundStyle(Ink.meta)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .tint(Ink.accent)
                    .disabled(!CoreTermSwitch.available || CoreTermSwitch.setByLaunchArgument)
                    .accessibilityIdentifier("core-switch-term-core")
                } header: {
                    Text(verbatim: "TERMINAL")
                        .font(Face.mono(10.5, .semibold))
                        .kerning(1.2)
                        .foregroundStyle(Ink.sectionTitle)
                } footer: {
                    Text(verbatim: Self.terminalNote).moshpitFooter()
                }
                .moshpitRows()

                Section {
                    Toggle(isOn: $rustStore) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "Rust store")
                                .font(.body).foregroundStyle(Ink.primary)
                            Text(verbatim: "Reads and writes what the app keeps through the Rust core's store instead of the Swift stores. From the next launch on.")
                                .font(.footnote).foregroundStyle(Ink.meta)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .tint(Ink.accent)
                    .disabled(Self.storeLocked)
                    .accessibilityIdentifier("core-switch-rust-store")
                } header: {
                    Text(verbatim: "STORAGE")
                        .font(Face.mono(10.5, .semibold))
                        .kerning(1.2)
                        .foregroundStyle(Ink.sectionTitle)
                } footer: {
                    Text(verbatim: Self.storeNote).moshpitFooter()
                }
                .moshpitRows()

                Section {
                    ForEach(NSHScreen.allCases, id: \.self) { screen in
                        Toggle(isOn: nshSwitch(screen)) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(verbatim: screen.switchTitle)
                                    .font(.body).foregroundStyle(Ink.primary)
                                Text(verbatim: "\(screen.page) drawn by NSH instead of SwiftUI. The next time it opens.")
                                    .font(.footnote).foregroundStyle(Ink.meta)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                        .tint(Ink.accent)
                        .disabled(Self.locked(screen))
                        .accessibilityIdentifier("core-switch-nsh-\(screen.rawValue)")
                    }
                } header: {
                    Text(verbatim: "SCREENS · NSH")
                        .font(Face.mono(10.5, .semibold))
                        .kerning(1.2)
                        .foregroundStyle(Ink.sectionTitle)
                } footer: {
                    if let note = Self.nshNote {
                        Text(verbatim: note).moshpitFooter()
                    }
                }
                .moshpitRows()

                Section {
                    NavigationLink {
                        UIGalleryPage()
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "UI gallery")
                                .font(.body).foregroundStyle(Ink.primary)
                            Text(verbatim: "Every component of the redesign's ui layer, in every variant and state, dark or light (docs/nsh/UI-DESIGN.md §4.20).")
                                .font(.footnote).foregroundStyle(Ink.meta)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .disabled(!NSHSurface.available)
                    .accessibilityIdentifier("core-switch-ui-gallery")
                } header: {
                    Text(verbatim: "UI LAYER")
                        .font(Face.mono(10.5, .semibold))
                        .kerning(1.2)
                        .foregroundStyle(Ink.sectionTitle)
                }
                .moshpitRows()
            }
            .moshpitForm()
            .navigationTitle(Text(verbatim: "Core Switches"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .foregroundStyle(Ink.accent)
                }
            }
        }
        .darkOnlyPage()
    }

    /// No Rust core to switch to, or a launch argument holds the switch.
    private static var sessionsLocked: Bool {
        !RustSessionSwitch.available
            || UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[RustSessionSwitch.switchKey] != nil
    }

    /// What the sessions switch covers, and what stays on the Swift path.
    private static var sessionsNote: String {
        if !RustSessionSwitch.available {
            return "This build has no Rust core."
        }
        var note = RustSessionSwitch.swiftPathNote
        if UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[RustSessionSwitch.switchKey] != nil {
            note += "\nSet for this launch by -\(RustSessionSwitch.switchKey)."
        }
        return note
    }

    /// What the terminal core switch covers.
    private static var terminalNote: String {
        if !CoreTermSwitch.available {
            return "This build has no terminal core."
        }
        var note = CoreTermSwitch.note
        if CoreTermSwitch.setByLaunchArgument {
            note += "\nSet for this launch by -\(CoreTermSwitch.switchKey)."
        }
        return note
    }

    /// No Rust store to switch to, or a launch argument holds the switch.
    private static var storeLocked: Bool {
        !RustStoreSwitch.available
            || UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[RustStoreSwitch.switchKey] != nil
    }

    /// What the storage switch covers, and which side this launch runs on
    /// (it changes at the next launch, never under a running app).
    private static var storeNote: String {
        if !RustStoreSwitch.available {
            return "This build has no Rust core."
        }
        var note = RustStoreSwitch.note
        note += "\nThis launch: \(RustStoreSwitch.isOn ? "the Rust store" : "the Swift stores")."
        if UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[RustStoreSwitch.switchKey] != nil {
            note += "\nSet for this launch by -\(RustStoreSwitch.switchKey)."
        }
        return note
    }

    /// The switch of `screen`, written to `UserDefaults.standard` as the
    /// page reads it (``NSHScreen/isOn``).
    private func nshSwitch(_ screen: NSHScreen) -> Binding<Bool> {
        Binding(
            get: { nshScreens[screen] ?? false },
            set: { on in
                nshScreens[screen] = on
                UserDefaults.standard.set(on, forKey: screen.switchKey)
            })
    }

    /// Every NSH screen's switch as stored now.
    private static func storedSwitches() -> [NSHScreen: Bool] {
        Dictionary(uniqueKeysWithValues: NSHScreen.allCases.map {
            ($0, UserDefaults.standard.bool(forKey: $0.switchKey))
        })
    }

    /// No NSH in this build, or a launch argument holds the switch.
    private static func locked(_ screen: NSHScreen) -> Bool {
        !NSHSurface.available || screen.setByLaunchArgument
    }

    /// What the NSH screens' switches do not show by themselves.
    private static var nshNote: String? {
        if !NSHSurface.available {
            return "This build cannot host NSH screens."
        }
        let held = NSHScreen.allCases.filter(\.setByLaunchArgument)
        guard !held.isEmpty else { return nil }
        return held.map { "\($0.switchTitle): set for this launch by -\($0.switchKey)." }
            .joined(separator: "\n")
    }
}
