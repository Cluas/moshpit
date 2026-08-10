import SwiftUI

/// Dictation-language picker: Automatic (system language) on top, then every
/// locale some on-device engine can transcribe. The list is engine-derived —
/// iOS 26 devices see the SpeechAnalyzer + keyboard-dictation locales, older
/// devices the SFSpeechRecognizer set — so nothing here can be picked that
/// dictation can't actually do.
struct VoiceLanguageView: View {
    @Environment(AppSettings.self) private var settings

    @State private var options: [VoiceLocaleCatalog.Option] = []
    @State private var loaded = false

    /// The system language as a display name. Built from language + region
    /// components — the raw `Locale.current.identifier` can carry extension
    /// tags (e.g. `en_US@rg=hkzzzz`) that `localizedString(forIdentifier:)`
    /// refuses to name, which would leak the raw string into the UI.
    private var systemLanguageName: String {
        let language = Locale.current.language
        let id = [language.languageCode?.identifier, language.region?.identifier]
            .compactMap { $0 }
            .joined(separator: "-")
        guard !id.isEmpty else { return Locale.current.identifier }
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }

    var body: some View {
        ZStack {
            MoshpitBackground()

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    FormGroup(
                        title: "DICTATION LANGUAGE",
                        footer: "Languages offered here are the ones this device can transcribe. A language's speech model downloads once on first use, then works offline."
                    ) {
                        languageRow(
                            name: String(localized: "Automatic"),
                            detail: systemLanguageName,
                            selected: settings.voiceInputLocaleId.isEmpty
                        ) { settings.voiceInputLocaleId = "" }

                        if loaded {
                            ForEach(options) { option in
                                languageRow(
                                    name: option.name,
                                    detail: nil,
                                    selected: settings.voiceInputLocaleId == option.id
                                ) { settings.voiceInputLocaleId = option.id }
                            }
                        } else {
                            HStack(spacing: 10) {
                                ProgressView().controlSize(.small).tint(Ink.accent)
                                Text("Checking available languages…")
                                    .font(Face.text(13))
                                    .foregroundStyle(Ink.meta)
                            }
                            .frame(minHeight: Metrics.cellMinHeight, alignment: .leading)
                        }
                    }
                }
                .padding(.horizontal, Metrics.pageHPad)
                .padding(.bottom, 28)
            }
        }
        .navigationTitle("Voice Language")
        .navigationBarTitleDisplayMode(.inline)
        .preferredColorScheme(.dark)
        .task {
            guard !loaded else { return }
            options = await VoiceLocaleCatalog.options()
            loaded = true
        }
    }

    private func languageRow(name: String, detail: String?, selected: Bool,
                             onSelect: @escaping () -> Void) -> some View {
        Button {
            onSelect()
            Haptics.select()
        } label: {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).font(Face.text(14)).foregroundStyle(Ink.primary)
                    if let detail {
                        Text(detail).font(Face.text(11)).foregroundStyle(Ink.meta)
                    }
                }
                Spacer()
                if selected {
                    Image(systemName: "checkmark")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Ink.accent)
                }
            }
            .frame(minHeight: Metrics.cellMinHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
