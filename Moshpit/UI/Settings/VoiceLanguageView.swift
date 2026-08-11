import SwiftUI

/// Dictation-language picker. Which list it shows depends on the engine,
/// because the two have genuinely different language models underneath:
///
/// - **Apple** — one locale per session, and only the locales some engine on
///   *this device* can transcribe. The list is engine-derived, so nothing here
///   can be picked that dictation can't actually do.
/// - **Whisper** — one multilingual model covers every language it knows, with
///   no per-language download and no device dependency, so the list is the
///   model's own. Auto-detect is a real option here rather than a guess from
///   system settings: the model decides from the audio.
struct VoiceLanguageView: View {
    @Environment(AppSettings.self) private var settings

    @State private var appleOptions: [VoiceLocaleCatalog.Option] = []
    @State private var loaded = false

    private var isWhisper: Bool { settings.voiceEngine == .whisper }

    /// What Automatic resolves to right now, spelled out.
    ///
    /// Worth the space: Automatic silently picking the interface language is
    /// the single most confusing thing dictation does, and a user who reads
    /// English but speaks Chinese has no way to discover it from a row that
    /// just says "Automatic".
    private var automaticDetail: String {
        let candidates = VoiceLanguageResolver.spokenLanguageCandidates()
        guard let first = candidates.first else { return Locale.current.identifier }
        let language = Locale(identifier: first).language
        let id = [language.languageCode?.identifier, language.region?.identifier]
            .compactMap { $0 }
            .joined(separator: "-")
        guard !id.isEmpty else { return first }
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }

    var body: some View {
        ZStack {
            MoshpitBackground()

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if isWhisper {
                        whisperSections
                    } else {
                        appleSection
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
            guard !isWhisper, !loaded else { return }
            appleOptions = await VoiceLocaleCatalog.options()
            loaded = true
        }
    }

    // MARK: Apple

    @ViewBuilder
    private var appleSection: some View {
        @Bindable var settings = settings
        FormGroup(
            title: "DICTATION LANGUAGE",
            footer: "Languages offered here are the ones this device can transcribe. A language's speech model downloads once on first use, then works offline. Apple's engines handle one language per session — for speech that switches between two, switch Recognition to Whisper."
        ) {
            languageRow(
                name: String(localized: "Automatic"),
                detail: automaticDetail,
                selected: settings.voiceInputLocaleId.isEmpty
            ) { settings.voiceInputLocaleId = "" }

            if loaded {
                ForEach(appleOptions) { option in
                    languageRow(
                        name: option.name,
                        detail: nil,
                        selected: settings.voiceInputLocaleId == option.id
                    ) { settings.voiceInputLocaleId = option.id }
                }
            } else {
                loadingRow
            }
        }
    }

    // MARK: Whisper

    @ViewBuilder
    private var whisperSections: some View {
        @Bindable var settings = settings

        FormGroup(
            title: "DICTATION LANGUAGE",
            footer: "One model covers around 100 languages. Naming yours transcribes it while keeping the foreign words inside a sentence intact — the English command names and library names you say mid-thought survive. Auto-detect reads the language off the audio instead, which can waver on short or heavily mixed phrases."
        ) {
            languageRow(
                name: String(localized: "Auto-detect"),
                detail: String(localized: "Whisper decides from what it hears"),
                selected: settings.whisperLanguage.isEmpty
            ) { settings.whisperLanguage = "" }

            ForEach(suggested) { option in
                languageRow(
                    name: option.name,
                    detail: nil,
                    selected: settings.whisperLanguage == option.id
                ) { settings.whisperLanguage = option.id }
            }
        }

        FormGroup(title: "ALL LANGUAGES") {
            ForEach(WhisperLanguageCatalog.options()) { option in
                languageRow(
                    name: option.name,
                    detail: nil,
                    selected: settings.whisperLanguage == option.id
                ) { settings.whisperLanguage = option.id }
            }
        }
    }

    /// Shortcut rows above the full alphabetical hundred, taken from this
    /// device's own language settings — see `WhisperLanguageCatalog.suggested`
    /// for why they aren't a fixed list.
    private var suggested: [WhisperLanguageCatalog.Option] {
        WhisperLanguageCatalog.suggested(for: VoiceLanguageResolver.spokenLanguageCandidates())
    }

    // MARK: Rows

    private var loadingRow: some View {
        HStack(spacing: 10) {
            ProgressView().controlSize(.small).tint(Ink.accent)
            Text("Checking available languages…")
                .font(Face.text(13))
                .foregroundStyle(Ink.meta)
        }
        .frame(minHeight: Metrics.cellMinHeight, alignment: .leading)
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
