import SwiftUI

/// Settings › Voice Input › Language: the SwiftUI page
/// (``VoiceLanguageView``), or the NSH one while its switch is on
/// (``NSHScreen/voiceLanguage``). With the switch off the SwiftUI page is all
/// there is: no NSH code runs.
struct VoiceLanguagePage: View {
    var body: some View {
        if NSHScreen.voiceLanguage.isOn {
            VoiceLanguageNSHPage()
        } else {
            VoiceLanguageView()
        }
    }
}

/// Settings › Voice Input › Language drawn by NSH
/// (`ui/src/voice_language.rs`): the languages the engine in use offers and
/// the one picked come from the host (``VoiceLanguageModel``), which reads
/// them as the SwiftUI page does (the device's speech locales for Apple's
/// engines, Whisper's own list for Whisper); a pick goes back to it.
struct VoiceLanguageNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @State private var model = VoiceLanguageModel()
    @State private var failed = false

    var body: some View {
        if failed {
            VoiceLanguageView()
        } else {
            NSHPage(
                screen: .voiceLanguage,
                attach: { model.attach($0, settings: settings) },
                onMessage: { message in
                    switch message["ev"] as? String {
                    case "back":
                        dismiss()
                    case "select":
                        if let id = message["id"] as? String { model.select(id, settings: settings) }
                    default:
                        break
                    }
                },
                onFailure: { failed = true })
                .nshPage()
                // The title the next page's back button would show.
                .navigationTitle("Voice Language")
                .task { await model.load(settings: settings) }
        }
    }
}

/// What the NSH Language page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule), and the message that
/// carries it (`{"op": "languages", …}`, `ui/src/voice_language.rs`'s
/// `Message`).
@MainActor
final class VoiceLanguageModel {
    /// One language the page offers.
    struct Option: Equatable {
        let id: String
        let name: String
    }

    /// The device's speech locales for Apple's engines, once read (nil:
    /// still reading).
    private(set) var appleOptions: [Option]?
    private weak var surface: NSHSurface?
    /// Reads the device's speech locales: the catalog's, or a test's.
    var readAppleOptions: () async -> [Option] = {
        await VoiceLocaleCatalog.options().map { Option(id: $0.id, name: $0.name) }
    }

    /// `surface` shows the languages from now on, starting with the current
    /// state.
    func attach(_ surface: NSHSurface, settings: AppSettings) {
        self.surface = surface
        post(settings)
    }

    /// Reads Apple's languages once, as the SwiftUI page does, when Apple's
    /// engine is in use.
    func load(settings: AppSettings) async {
        guard settings.voiceEngine != .whisper, appleOptions == nil else { return }
        let options = await readAppleOptions()
        appleOptions = options
        post(settings)
    }

    /// The user picked language `id` ("": Automatic, or Whisper's
    /// auto-detect) for the engine in use.
    func select(_ id: String, settings: AppSettings) {
        if settings.voiceEngine == .whisper {
            settings.whisperLanguage = id
        } else {
            settings.voiceInputLocaleId = id
        }
        post(settings)
    }

    private func post(_ settings: AppSettings) {
        if settings.voiceEngine == .whisper {
            let candidates = VoiceLanguageResolver.spokenLanguageCandidates()
            surface?.post(Self.whisperMessage(
                suggested: WhisperLanguageCatalog.suggested(for: candidates).map { Option(id: $0.id, name: $0.name) },
                all: WhisperLanguageCatalog.options().map { Option(id: $0.id, name: $0.name) },
                selected: settings.whisperLanguage))
        } else {
            surface?.post(Self.appleMessage(
                automatic: Self.automaticDetail(),
                options: appleOptions,
                selected: settings.voiceInputLocaleId))
        }
    }

    /// The screen's message for Apple's engines: what Automatic resolves to,
    /// the device's languages (nil while they are read) and the pick.
    nonisolated static func appleMessage(automatic: String, options: [Option]?, selected: String) -> [String: Any] {
        var message: [String: Any] = [
            "op": "languages",
            "engine": VoiceEngineKind.apple.rawValue,
            "automatic": automatic,
            "selected": selected,
        ]
        if let options {
            message["options"] = options.map(option)
        }
        return message
    }

    /// The screen's message for Whisper: the languages to show first (the
    /// user's own), all of Whisper's, and the pick.
    nonisolated static func whisperMessage(suggested: [Option], all: [Option], selected: String) -> [String: Any] {
        [
            "op": "languages",
            "engine": VoiceEngineKind.whisper.rawValue,
            "suggested": suggested.map(option),
            "options": all.map(option),
            "selected": selected,
        ]
    }

    nonisolated static func option(_ option: Option) -> [String: Any] {
        ["id": option.id, "name": option.name]
    }

    /// What Automatic resolves to right now, spelled out, as the SwiftUI page
    /// says it (``VoiceLanguageView``'s `automaticDetail`).
    static func automaticDetail() -> String {
        let candidates = VoiceLanguageResolver.spokenLanguageCandidates()
        guard let first = candidates.first else { return Locale.current.identifier }
        let language = Locale(identifier: first).language
        let id = [language.languageCode?.identifier, language.region?.identifier]
            .compactMap { $0 }
            .joined(separator: "-")
        guard !id.isEmpty else { return first }
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }
}
