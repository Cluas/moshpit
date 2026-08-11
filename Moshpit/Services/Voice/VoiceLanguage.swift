import Foundation
import UIKit

// MARK: - Engine choice

/// Which speech engine a dictation session runs on. Persisted in
/// `AppSettings.voiceEngineId`.
enum VoiceEngineKind: String, CaseIterable, Identifiable, Sendable {
    /// Apple's on-device transcribers (SpeechAnalyzer on iOS 26, otherwise
    /// SFSpeechRecognizer). Nothing to download, but each session is locked to
    /// exactly one locale — Apple exposes no code-switching mode.
    case apple
    /// Local Whisper through WhisperKit. A single multilingual model covers
    /// ~100 languages at once, which is the only way a sentence that starts in
    /// one language and ends in "git rebase --onto" survives intact. Costs a
    /// several-hundred-MB download and more time per utterance.
    case whisper

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .apple: return String(localized: "Apple (built in)")
        case .whisper: return String(localized: "Whisper (local model)")
        }
    }

    var summary: String {
        switch self {
        case .apple:
            return String(localized: "No download. One language per session.")
        case .whisper:
            return String(localized: "Downloads a model. Around 100 languages, and copes with a sentence that mixes two.")
        }
    }
}

// MARK: - Language resolution

/// Works out what language to transcribe in, and — the part that matters —
/// what "Automatic" should mean.
///
/// The naive reading of Automatic is `Locale.current`, i.e. the language the
/// *interface* is in. That is wrong for a large share of this app's users: a
/// developer running an English iPhone who dictates Chinese into a terminal
/// gets an English acoustic model and a transcript of nonsense, with nothing
/// on screen explaining why. The UI language is evidence about what someone
/// reads, not about what they say.
enum VoiceLanguageResolver {
    /// Language identifiers the user plausibly speaks, best guess first.
    ///
    /// Two sources, in order of how much they say about *speech*:
    ///
    /// 1. `Locale.preferredLanguages` — the ranked list from Settings ▸
    ///    General ▸ Language & Region. It's a ranking the user made
    ///    themselves, so it leads.
    /// 2. Installed keyboards. Someone who installed a Pinyin keyboard types
    ///    Chinese, and an English-interface phone with a Chinese keyboard is
    ///    an extremely common setup among exactly the bilingual users this
    ///    exists for. It's weaker evidence than an explicit ranking, so it
    ///    only breaks ties the first list left unresolved.
    ///
    /// Deduplicated by language code, order preserved.
    @MainActor
    static func spokenLanguageCandidates() -> [String] {
        var seen = Set<String>()
        var ordered: [String] = []
        func add(_ identifier: String) {
            let code = Locale(identifier: identifier).language.languageCode?.identifier
            guard let code, !code.isEmpty, seen.insert(code).inserted else { return }
            ordered.append(identifier)
        }
        Locale.preferredLanguages.forEach(add)
        UITextInputMode.activeInputModes.compactMap(\.primaryLanguage).forEach(add)
        // `Locale.current` last: on a device with no preferred languages set
        // it's the only thing left, and it can't make the ordering worse.
        add(Locale.current.identifier)
        return ordered
    }

    /// The locale an Apple dictation session should use.
    ///
    /// `stored` is `AppSettings.voiceInputLocaleId` — empty means Automatic,
    /// which walks the candidates above and takes the first one some engine on
    /// this device can actually transcribe. `isSupported` is the engine
    /// catalog's answer; when nothing matches we hand back the top candidate
    /// anyway so the failure the user sees names a real language rather than
    /// silently substituting another one.
    @MainActor
    static func appleLocale(stored: String, isSupported: (Locale) -> Bool) -> Locale {
        if !stored.isEmpty { return Locale(identifier: stored) }
        let candidates = spokenLanguageCandidates()
        for identifier in candidates {
            let locale = Locale(identifier: identifier)
            if isSupported(locale) { return locale }
        }
        return candidates.first.map(Locale.init(identifier:)) ?? Locale.current
    }

    /// The Whisper language code for a session.
    ///
    /// Returns nil for Automatic, and nil is meaningful: it puts Whisper into
    /// its own language detection, which is a genuinely better Automatic than
    /// anything derived from system settings — the model listens to the audio
    /// instead of guessing from configuration.
    static func whisperCode(stored: String) -> String? {
        guard !stored.isEmpty else { return nil }
        return whisperCode(forIdentifier: stored)
    }

    /// Maps a BCP-47 identifier onto Whisper's language codes, which are bare
    /// ISO-639-1 with a handful of exceptions.
    static func whisperCode(forIdentifier identifier: String) -> String? {
        let locale = Locale(identifier: identifier)
        guard let code = locale.language.languageCode?.identifier else { return nil }
        guard code == "zh" else { return code }
        // Cantonese is its own Whisper language, and `zh` alone doesn't say
        // which is meant. Region is NOT enough to decide: `zh-Hans-HK` is a
        // Simplified-Chinese (i.e. Mandarin) speaker who happens to be in Hong
        // Kong — an extremely common setup — and calling that Cantonese would
        // hand them the wrong model on a signal that says nothing about how
        // they talk. Require the tag to actually mean Cantonese: an explicit
        // `yue`, or Traditional script in a Cantonese-speaking region.
        let script = locale.language.script?.identifier
        let region = locale.language.region?.identifier
        if script == "Hant", region == "HK" || region == "MO" { return "yue" }
        return "zh"
    }

    /// The language to preselect the first time someone switches to Whisper.
    ///
    /// The user's top **non-English** language, when their device settings
    /// name one; otherwise empty, meaning auto-detect.
    ///
    /// Two reasons it isn't just auto-detect for everyone. Naming a language
    /// beats per-window detection on exactly the code-switched speech this
    /// engine is for — detection can flip mid-sentence. And English is the
    /// one language that needs no help here: shell commands are English
    /// whoever is speaking, so an English-only speaker gains nothing from
    /// pinning it, while someone who speaks anything else gains a lot.
    @MainActor
    static func initialWhisperLanguage() -> String {
        for identifier in spokenLanguageCandidates() {
            if let code = whisperCode(forIdentifier: identifier), code != "en" {
                return code
            }
        }
        return ""
    }
}
