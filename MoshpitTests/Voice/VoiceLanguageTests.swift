import Foundation
import Testing
@testable import Moshpit

/// Language resolution — the layer that decides what "Automatic" means and
/// translates between Apple's BCP-47 locales and Whisper's bare codes.
@Suite("Voice language resolution")
struct VoiceLanguageTests {

    @Test("A stored language wins over anything inferred")
    func storedWins() {
        #expect(VoiceLanguageResolver.whisperCode(stored: "zh") == "zh")
        #expect(VoiceLanguageResolver.whisperCode(stored: "en-US") == "en")
        #expect(VoiceLanguageResolver.whisperCode(stored: "ja-JP") == "ja")
    }

    @Test("Automatic maps to nil so Whisper detects from the audio")
    func automaticIsDetection() {
        // Not a guess from system configuration: nil puts the model's own
        // language detection in charge, which is the only source that has
        // actually heard the speaker.
        #expect(VoiceLanguageResolver.whisperCode(stored: "") == nil)
    }

    @Test("Regional Chinese variants land on the right Whisper language")
    func chineseVariants() {
        // Whisper treats Cantonese as its own language, so a `zh` tag has to
        // be resolved to one or the other.
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "zh-Hans-CN") == "zh")
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "zh-Hant-TW") == "zh")
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "zh-Hant-HK") == "yue")
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "yue") == "yue")
    }

    @Test("Hong Kong region alone does not mean Cantonese")
    func simplifiedInHongKongIsMandarin() {
        // Caught on a real device configuration: preferred languages of
        // ["en-US", "zh-Hans-HK", "en"] — an English interface with a
        // Simplified-Chinese speaker living in Hong Kong. Deciding on region
        // handed them the Cantonese model. The script is the signal that
        // actually tracks the spoken variety; the region does not.
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "zh-Hans-HK") == "zh")
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "zh-Hans-MO") == "zh")
    }

    @Test("A nonsense identifier resolves to nothing rather than English")
    func unknownIdentifier() {
        // Silently substituting a language is the failure mode this whole
        // area exists to prevent.
        #expect(VoiceLanguageResolver.whisperCode(forIdentifier: "") == nil)
    }

    @MainActor
    @Test("Automatic candidates are ordered, deduplicated, and non-empty")
    func candidateOrdering() {
        let candidates = VoiceLanguageResolver.spokenLanguageCandidates()
        #expect(!candidates.isEmpty)

        // One entry per language: a phone with en-US preferred and an en-GB
        // keyboard must not spend two slots on English before reaching the
        // language the user actually speaks.
        let codes = candidates.compactMap { Locale(identifier: $0).language.languageCode?.identifier }
        #expect(codes.count == Set(codes).count)
        #expect(codes.count == candidates.count)
    }

    @MainActor
    @Test("Automatic for Apple honours an explicit choice and probes support otherwise")
    func appleLocaleResolution() {
        // Explicit setting is passed through untouched.
        let explicit = VoiceLanguageResolver.appleLocale(stored: "zh-CN", isSupported: { _ in false })
        #expect(explicit.language.languageCode?.identifier == "zh")

        // Automatic skips languages no engine can transcribe rather than
        // stopping at the first preferred one — the bug that left a Chinese
        // speaker on an English phone with an English acoustic model.
        let chinese = VoiceLanguageResolver.appleLocale(stored: "") { locale in
            locale.language.languageCode?.identifier == "zh"
        }
        #expect(chinese.language.languageCode?.identifier == "zh")

        // Nothing supported: still returns a real locale so the error message
        // can name a language instead of substituting one.
        let fallback = VoiceLanguageResolver.appleLocale(stored: "", isSupported: { _ in false })
        #expect(fallback.language.languageCode != nil)
    }

    @Test("Whisper's language catalog spans far more than the CJK set")
    func whisperCatalog() {
        let options = WhisperLanguageCatalog.options()
        let ids = Set(options.map(\.id))
        // A spread across scripts and families — the model is multilingual,
        // and the picker must not read as though it were built for two
        // languages with the rest bolted on.
        for code in ["zh", "en", "yue", "ja", "ko", "ar", "hi", "de", "es", "ru", "pt", "tr"] {
            #expect(ids.contains(code), "catalog missing \(code)")
        }
        #expect(options.count > 90)
        // Sorted by localized name so the full list is navigable.
        #expect(options.map(\.name) == options.map(\.name)
            .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending })
    }

    @Test("Suggested languages come from the device, not a hard-coded set")
    func suggestedFollowsTheDevice() {
        // A Japanese speaker must not be shown a Chinese shortcut list, and
        // vice versa — the shortcut above 100 alphabetical rows has to be the
        // user's own languages or it is just a statement about who the
        // feature was built for.
        let japanese = WhisperLanguageCatalog.suggested(for: ["ja-JP", "en-US"])
        #expect(japanese.map(\.id) == ["ja", "en"])

        let chinese = WhisperLanguageCatalog.suggested(for: ["en-US", "zh-Hans-HK", "en"])
        #expect(chinese.map(\.id) == ["en", "zh"])

        // English is appended even when unlisted: shell command names are
        // English whoever is speaking.
        let arabic = WhisperLanguageCatalog.suggested(for: ["ar-EG"])
        #expect(arabic.map(\.id) == ["ar", "en"])
    }

    @Test("An English-only device gets no suggested section at all")
    func suggestedSkippedForEnglishOnly() {
        // One row above a list that already starts near English is noise.
        #expect(WhisperLanguageCatalog.suggested(for: ["en-US", "en-GB"]).isEmpty)
        #expect(WhisperLanguageCatalog.suggested(for: []).isEmpty)
    }

    @Test("Suggested entries are deduplicated and real catalog entries")
    func suggestedIsClean() {
        let ids = Set(WhisperLanguageCatalog.options().map(\.id))
        let suggested = WhisperLanguageCatalog.suggested(
            for: ["zh-Hans-CN", "zh-Hant-TW", "en-US", "en-GB", "de-DE"])
        // zh appears twice in the input; it must occupy one row, not two.
        #expect(suggested.map(\.id) == ["zh", "en", "de"])
        #expect(suggested.allSatisfy { ids.contains($0.id) })
    }

    @Test("Empty language reads as Auto-detect, not as a blank")
    func displayNames() {
        #expect(WhisperLanguageCatalog.displayName(for: "") == "Auto-detect")
        #expect(!WhisperLanguageCatalog.displayName(for: "zh").isEmpty)
    }
}

/// The model store's pure parts — catalog shape, path layout, and the
/// fallback that keeps a deleted model from breaking a session. Nothing here
/// downloads or loads anything.
@Suite("Whisper model store")
struct WhisperModelStoreTests {

    @Test("Catalog is non-empty and offers only multilingual models")
    func catalogIsMultilingualOnly() {
        let options = WhisperModelStore.catalog()
        #expect(!options.isEmpty)

        for option in options {
            // `.en` and distil-whisper variants are English-only. Offering one
            // would silently defeat the reason Whisper is here at all.
            #expect(!option.id.hasSuffix(".en"), "\(option.id) is English-only")
            #expect(!option.id.contains("distil"), "\(option.id) is English-only")
            #expect(option.approximateBytes > 0)
            #expect(!option.name.isEmpty)
            #expect(!option.summary.isEmpty)
        }
    }

    @Test("Recommended variant is one the catalog actually offers")
    func recommendedIsOffered() {
        let recommended = WhisperModelStore.recommendedVariant()
        #expect(WhisperModelStore.catalog().contains { $0.id == recommended })
    }

    @Test("Model paths sit under Application Support, not Documents")
    func modelPathLocation() {
        let folder = WhisperModelStore.folder(for: "openai_whisper-small")
        let path = folder.path
        // Documents is user-visible in the Files app and gets backed up —
        // neither is acceptable for a re-downloadable 950 MB cache.
        #expect(path.contains("Application Support"))
        #expect(!path.contains("/Documents/"))
        #expect(path.hasSuffix("models/argmaxinc/whisperkit-coreml/openai_whisper-small"))
    }

    @Test("A model that was never downloaded is not reported as installed")
    func notInstalledByDefault() {
        #expect(!WhisperModelStore.isInstalled("openai_whisper-nonexistent-variant"))

        // Deliberately not asserting nil here. The store answers from the real
        // Application Support container, which this test process shares with the
        // app, so on a device or simulator where a model HAS been downloaded the
        // fallback correctly hands back that one — asserting nil made the test
        // pass only on a machine that had never used the feature. What has to
        // hold either way is the invariant the fallback exists for: it never
        // returns a variant that isn't on disk.
        let resolved = WhisperModelStore.resolvedVariant(preferring: "openai_whisper-nonexistent-variant")
        #expect(resolved != "openai_whisper-nonexistent-variant")
        if let resolved {
            #expect(WhisperModelStore.isInstalled(resolved))
        }
    }

    @Test("Display names fall back to the raw variant rather than going blank")
    func displayNameFallback() {
        #expect(WhisperModelStore.displayName(for: "openai_whisper-small") == "Small")
        #expect(WhisperModelStore.displayName(for: "mystery") == "mystery")
    }
}
