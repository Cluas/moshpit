import SwiftUI

/// Editor for a custom terminal theme.
///
/// The preview at the top is the point of the screen: it renders a fake session
/// using the *draft* palette, so every color change is judged in context
/// (against real prompt/diff/error colors on the real background) instead of as
/// an abstract swatch. Colors are only committed on Save, so backing out leaves
/// the live terminal untouched.
struct ThemeEditorView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(ThemeStore.self) private var themes

    /// Working copy. The caller passes either a fresh draft, a duplicate, or an
    /// existing custom theme.
    @State private var draft: TerminalTheme
    @State private var showBright: Bool
    @State private var exported: String?

    private let onSave: (TerminalTheme) -> Void

    init(theme: TerminalTheme, onSave: @escaping (TerminalTheme) -> Void) {
        _draft = State(initialValue: theme)
        // Start expanded when the theme already overrides bright colors —
        // otherwise the screen would hide settings the user deliberately set.
        _showBright = State(initialValue: TerminalTheme.ANSISlot.allCases.contains { theme.hasBrightOverride($0) })
        self.onSave = onSave
    }

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ThemeLivePreview(theme: draft)
                            .padding(.horizontal, 16)
                            .padding(.top, 8)
                            .padding(.bottom, 4)

                        FormGroup(title: "NAME") {
                            FieldRow(placeholder: "Theme name", text: $draft.name)
                        }

                        FormGroup(title: "TERMINAL") {
                            colorRow("Background", hex: $draft.backgroundHex)
                            colorRow("Foreground", hex: $draft.foregroundHex)
                            colorRow("Cursor", hex: $draft.cursorHex)
                        }

                        FormGroup(
                            title: "ANSI COLORS",
                            footer: "These eight are what shells, diffs and TUIs paint with."
                        ) {
                            ForEach(TerminalTheme.ANSISlot.allCases) { slot in
                                colorRow(slot.label, hex: ansiBinding(slot))
                            }
                        }

                        FormGroup(
                            title: "BRIGHT COLORS",
                            footer: "Bright slots are derived from the eight above by default. Override them only if you want exact control — many tools paint dim text with bright black, so keeping it distinct from black matters."
                        ) {
                            ToggleRow(
                                label: "Override bright colors",
                                subtitle: showBright ? nil : "Currently derived automatically",
                                isOn: Binding(
                                    get: { showBright },
                                    set: { on in
                                        showBright = on
                                        if !on { clearBrightOverrides() }
                                    })
                            )
                            if showBright {
                                ForEach(TerminalTheme.ANSISlot.allCases) { slot in
                                    colorRow("Bright " + slot.label, hex: brightBinding(slot))
                                }
                            }
                        }
                    }
                    .padding(.bottom, 28)
                }
            }
            .navigationTitle(draft.name.isEmpty ? "New Theme" : draft.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button {
                            exported = try? themes.exportJSON(draft)
                            if let exported { UIPasteboard.general.string = exported }
                        } label: {
                            Label("Copy as JSON", systemImage: "doc.on.doc")
                        }
                    } label: {
                        Image(systemName: "ellipsis.circle").foregroundStyle(Ink.accent)
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        onSave(draft)
                        dismiss()
                    }
                    .accessibilityIdentifier("theme-editor-save")
                }
            }
        }
    }

    // MARK: - Rows

    private func colorRow(_ label: String, hex: Binding<String>) -> some View {
        HStack(spacing: 10) {
            Text(label).font(Face.text(14)).foregroundStyle(Ink.primary)
            Spacer(minLength: 6)
            Text("#" + hex.wrappedValue)
                .font(Face.mono(12))
                .foregroundStyle(Ink.meta)
            // `supportsOpacity: false` — a terminal palette entry has no alpha,
            // and letting one through would render as an unexplained wash.
            ColorPicker("", selection: colorBinding(hex), supportsOpacity: false)
                .labelsHidden()
                .frame(width: 34)
        }
        .frame(minHeight: Metrics.cellMinHeight)
    }

    // MARK: - Bindings

    /// Bridges `ColorPicker`'s `Color` to the theme's hex storage. Writing back
    /// through hex (rather than keeping a `Color`) is what keeps the draft
    /// encodable and the preview, swatches and SwiftTerm palette in agreement.
    private func colorBinding(_ hex: Binding<String>) -> Binding<Color> {
        Binding(
            get: { Color(hex: hex.wrappedValue) },
            set: { hex.wrappedValue = $0.hexString }
        )
    }

    private func ansiBinding(_ slot: TerminalTheme.ANSISlot) -> Binding<String> {
        Binding(
            get: { draft.ansiHex(slot) },
            set: { draft.setAnsi($0, for: slot) }
        )
    }

    private func brightBinding(_ slot: TerminalTheme.ANSISlot) -> Binding<String> {
        Binding(
            get: { draft.brightHex(slot) },              // derived value until overridden
            set: { draft.setBrightOverride($0, for: slot) }
        )
    }

    private func clearBrightOverrides() {
        for slot in TerminalTheme.ANSISlot.allCases {
            draft.setBrightOverride(nil, for: slot)
        }
    }
}

// MARK: - Live preview

/// A fake terminal session rendered in the draft palette. The lines are chosen
/// to exercise the colors that decide whether a theme is usable: prompt green,
/// path blue, a diff's red/green pair, a warning yellow, and dim (bright-black)
/// secondary text.
struct ThemeLivePreview: View {
    let theme: TerminalTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            line {
                Text("❯ ").foregroundStyle(theme.green)
                Text("git ").foregroundStyle(theme.foreground)
                Text("status").foregroundStyle(theme.cyan)
            }
            line {
                Text("On branch ").foregroundStyle(theme.foreground)
                Text("main").foregroundStyle(theme.blue)
            }
            line {
                Text("+ ").foregroundStyle(theme.green)
                Text("MoshTransport.swift").foregroundStyle(theme.green)
            }
            line {
                Text("- ").foregroundStyle(theme.red)
                Text("MoshpitMark.swift").foregroundStyle(theme.red)
            }
            line {
                Text("warning: ").foregroundStyle(theme.yellow)
                Text("2 files changed").foregroundStyle(theme.foreground)
            }
            line {
                Text("hint: use --staged").foregroundStyle(theme.bright(.black))
            }
            line {
                Text("❯ ").foregroundStyle(theme.green)
                Text("█").foregroundStyle(theme.cursor)
            }
        }
        .font(.system(size: 12, weight: .regular, design: .monospaced))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(EdgeInsets(top: 12, leading: 13, bottom: 12, trailing: 13))
        .background(theme.background, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                .strokeBorder(Ink.groupBorder, lineWidth: 1))
        .accessibilityLabel("Theme preview")
    }

    private func line<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 0) { content() }
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
