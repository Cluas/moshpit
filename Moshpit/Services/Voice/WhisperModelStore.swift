import Foundation
import WhisperKit

// MARK: - Catalog

/// One Whisper variant offered under Settings ▸ Voice Input ▸ Model.
struct WhisperModelOption: Identifiable, Equatable, Sendable {
    /// WhisperKit variant name — also the folder name inside the model repo
    /// and the value persisted in `AppSettings.whisperModelId`.
    let id: String
    /// Short display name ("Large v3 Turbo").
    let name: String
    /// One line of guidance for choosing, shown under the name.
    let summary: String
    /// Download size. **Approximate** — the real figure is only known once
    /// the files are on disk, so the UI marks this with a "≈".
    let approximateBytes: Int64
    /// Exact bytes on disk, or nil when the variant isn't installed.
    let installedBytes: Int64?
    /// Bytes of an interrupted download waiting in the resume cache. Zero when
    /// there's nothing to carry on from.
    let partialBytes: Int64

    var isInstalled: Bool { installedBytes != nil }
    /// Started but not finished — the row offers Resume rather than Download.
    var isPartial: Bool { !isInstalled && partialBytes > 0 }
}

enum WhisperModelFailure: Error, Equatable, LocalizedError {
    /// The variant finished "downloading" but the three CoreML bundles the
    /// runtime needs aren't all there.
    case incomplete(String)
    case notInstalled(String)
    case download(String)

    var errorDescription: String? {
        switch self {
        case .incomplete(let name):
            return String(localized: "The \(name) speech model didn't download completely. Remove it and try again.")
        case .notInstalled(let name):
            return String(localized: "The \(name) speech model isn't downloaded yet.")
        case .download(let detail):
            return String(localized: "Couldn't download the speech model: \(detail)")
        }
    }
}

// MARK: - Languages

/// The language list for the Whisper picker — around a hundred of them.
///
/// Sourced from WhisperKit's own table rather than hand-maintained, so it
/// can't drift from what the model was trained on. Unlike Apple's catalog
/// nothing here depends on the device — one multilingual model covers all of
/// them, and there is no per-language download.
enum WhisperLanguageCatalog {
    struct Option: Identifiable, Equatable, Sendable {
        /// Whisper language code ("zh", "en", …) — stored in settings.
        let id: String
        /// Localized display name.
        let name: String
    }

    static func options() -> [Option] {
        let ui = Locale.current
        return Set(Constants.languages.values)
            .map { Option(id: $0, name: ui.localizedString(forLanguageCode: $0) ?? $0) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// The handful to show above the full alphabetical list.
    ///
    /// Derived from the device's own language settings rather than a fixed
    /// set: with a hundred entries the list needs a shortcut, but hard-coding
    /// which languages deserve the top says the feature is aimed at the people
    /// who speak them. `candidates` is `VoiceLanguageResolver`'s reading of
    /// what this user speaks, so the shortcut is theirs.
    ///
    /// English is appended regardless — in a terminal the command names are
    /// English whoever you are, so it's a plausible second choice for
    /// everyone. Nil when there's nothing but English to suggest, so a
    /// monolingual English user doesn't get a one-row section above an
    /// alphabetical list that starts with it anyway.
    static func suggested(for candidates: [String]) -> [Option] {
        let all = options()
        var codes: [String] = []
        for identifier in candidates {
            guard let code = VoiceLanguageResolver.whisperCode(forIdentifier: identifier),
                  !codes.contains(code), all.contains(where: { $0.id == code })
            else { continue }
            codes.append(code)
        }
        guard codes.contains(where: { $0 != "en" }) else { return [] }
        if !codes.contains("en") { codes.append("en") }
        return codes.compactMap { code in all.first(where: { $0.id == code }) }
    }

    static func displayName(for code: String) -> String {
        guard !code.isEmpty else { return String(localized: "Auto-detect") }
        return Locale.current.localizedString(forLanguageCode: code) ?? code
    }
}

// MARK: - Store

/// Owns everything about local Whisper models: which variants this device is
/// offered, downloading and deleting their weights, and the one loaded
/// `WhisperKit` instance that actually transcribes.
///
/// An actor rather than a plain type because `WhisperKit` is a non-Sendable
/// class holding CoreML state that must not be touched from two tasks at once,
/// and because loading a model is expensive enough (seconds, hundreds of MB)
/// that it has to be shared across dictation sessions instead of rebuilt per
/// session. Nothing hands the instance out — callers ask the store to
/// transcribe, so the model never escapes the actor.
actor WhisperModelStore {
    static let shared = WhisperModelStore()

    /// Variants worth offering, best first.
    ///
    /// Deliberately **multilingual only**. The repo also publishes `.en`
    /// variants and the `distil-whisper` family, which are English-only —
    /// offering those would silently defeat the entire reason Whisper is here
    /// (a sentence that doesn't stay in one language). `recommendedModels()`
    /// then trims this to what the chip can actually run.
    private static let curated: [(id: String, name: String, summary: String, bytes: Int64)] = [
        ("openai_whisper-large-v3_turbo_954MB",
         String(localized: "Large v3 Turbo"),
         String(localized: "Most accurate, especially on mixed-language speech. Needs a recent chip."),
         954 * 1_000_000),
        ("openai_whisper-small",
         String(localized: "Small"),
         String(localized: "Good balance of accuracy and speed."),
         470 * 1_000_000),
        ("openai_whisper-base",
         String(localized: "Base"),
         String(localized: "Fast and light. Noticeably weaker outside English."),
         145 * 1_000_000),
        ("openai_whisper-tiny",
         String(localized: "Tiny"),
         String(localized: "Fastest, lowest accuracy. For older devices."),
         75 * 1_000_000),
    ]

    /// The three CoreML bundles `WhisperKit.loadModels` requires. A folder
    /// missing any of them is a half-finished download, not an install.
    private static let requiredModels = ["MelSpectrogram", "AudioEncoder", "TextDecoder"]

    private static let repo = "argmaxinc/whisperkit-coreml"

    /// Currently loaded model, keyed by variant so switching models in
    /// Settings drops the old one instead of stacking two in memory.
    private var loaded: (variant: String, kit: WhisperKit)?

    // MARK: Locations

    /// Weights live in Application Support, not Documents (WhisperKit's own
    /// default): they're re-downloadable caches, so they must not show up in
    /// the Files app next to the user's own data, and must not be uploaded to
    /// iCloud — a 954 MB model in a device backup is indefensible.
    nonisolated static var downloadBase: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("SpeechModels", isDirectory: true)
        if !FileManager.default.fileExists(atPath: base.path) {
            try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
            var url = base
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? url.setResourceValues(values)
        }
        return base
    }

    /// Where `HubApi` puts a repo: `<downloadBase>/models/<repo-id>`. Mirrors
    /// `HubApi.localRepoLocation`, which is internal to WhisperKit — if that
    /// layout ever changes, installed models read as missing and re-download
    /// rather than corrupting anything.
    nonisolated static func folder(for variant: String) -> URL {
        downloadBase
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent(repo, isDirectory: true)
            .appendingPathComponent(variant, isDirectory: true)
    }

    /// True only when every required CoreML bundle is present — an interrupted
    /// download leaves the folder there with some of them.
    nonisolated static func isInstalled(_ variant: String) -> Bool {
        let folder = folder(for: variant)
        return requiredModels.allSatisfy { name in
            let compiled = folder.appendingPathComponent("\(name).mlmodelc")
            let package = folder.appendingPathComponent("\(name).mlpackage")
            return FileManager.default.fileExists(atPath: compiled.path)
                || FileManager.default.fileExists(atPath: package.path)
        }
    }

    // MARK: Catalog

    /// The curated list trimmed to what this chip supports, annotated with
    /// install state. Synchronous and offline — `recommendedModels()` reads a
    /// device table compiled into WhisperKit, so the Settings screen renders
    /// without touching the network.
    nonisolated static func catalog() -> [WhisperModelOption] {
        let supported = Set(WhisperKit.recommendedModels().supported)
        var offered = curated.filter { supported.contains($0.id) }
        // A chip missing from WhisperKit's table (or a table that disagrees
        // with our curation) must not produce an empty screen — fall back to
        // the two variants every supported device can run.
        if offered.isEmpty {
            offered = curated.filter { $0.id == "openai_whisper-base" || $0.id == "openai_whisper-tiny" }
        }
        return offered.map { entry in
            WhisperModelOption(
                id: entry.id,
                name: entry.name,
                summary: entry.summary,
                approximateBytes: entry.bytes,
                installedBytes: isInstalled(entry.id) ? directorySize(folder(for: entry.id)) : nil,
                partialBytes: isInstalled(entry.id) ? 0 : partialBytes(for: entry.id))
        }
    }

    /// What a fresh install should download: the best variant this device is
    /// offered, which the curated ordering already puts first.
    nonisolated static func recommendedVariant() -> String {
        catalog().first?.id ?? "openai_whisper-base"
    }

    /// The variant to actually use — the stored choice when it's installed,
    /// otherwise any other installed one, otherwise nil. Keeps a session from
    /// dying because Settings points at a model the user deleted.
    nonisolated static func resolvedVariant(preferring stored: String) -> String? {
        if !stored.isEmpty, isInstalled(stored) { return stored }
        return catalog().first(where: { $0.isInstalled })?.id
    }

    nonisolated static func displayName(for variant: String) -> String {
        curated.first(where: { $0.id == variant })?.name ?? variant
    }

    /// Bytes of a half-finished download sitting in the resume cache.
    ///
    /// The Hub downloader appends to `<file>.<etag>.incomplete` and resumes with
    /// a `Range: bytes=N-` header, so an interrupted transfer costs nothing but
    /// the time already spent — provided nothing deletes these files. Surfacing
    /// the number is what turns "it broke, start over" into "carry on".
    nonisolated static func partialBytes(for variant: String) -> Int64 {
        // Both halves, because a repo is fetched file by file: the one in
        // flight sits in the resume cache as `.incomplete`, and each finished
        // file MOVES out of that cache into the model folder. Counting only the
        // cache made the figure fall back every time a file completed — progress
        // appearing to go backwards is worse than no figure at all.
        logicalSize(of: resumeCache(for: variant)) { $0.pathExtension == "incomplete" }
            + logicalSize(of: folder(for: variant)) { _ in true }
    }

    /// Sum of logical file sizes under `url`, for files passing `include`.
    ///
    /// Logical, not allocated: this feeds a download-progress readout, and
    /// allocated blocks overstate it (measured 157 MB against 140 MB actually
    /// transferred). `directorySize` deliberately keeps using allocated,
    /// because that one answers "how much storage is this costing me".
    private nonisolated static func logicalSize(
        of url: URL, include: (URL) -> Bool) -> Int64 {
        guard let walker = FileManager.default.enumerator(
            at: url, includingPropertiesForKeys: [.fileSizeKey]) else { return 0 }
        var total: Int64 = 0
        for case let file as URL in walker where include(file) {
            let values = try? file.resourceValues(forKeys: [.fileSizeKey])
            total += Int64(values?.fileSize ?? 0)
        }
        return total
    }

    /// Where the Hub downloader keeps this variant's partial files + metadata.
    nonisolated static func resumeCache(for variant: String) -> URL {
        downloadBase
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent(repo, isDirectory: true)
            .appendingPathComponent(".cache/huggingface/download", isDirectory: true)
            .appendingPathComponent(variant, isDirectory: true)
    }

    nonisolated static func directorySize(_ url: URL) -> Int64 {
        guard let walker = FileManager.default.enumerator(
            at: url, includingPropertiesForKeys: [.totalFileAllocatedSizeKey, .fileAllocatedSizeKey])
        else { return 0 }
        var total: Int64 = 0
        for case let file as URL in walker {
            let values = try? file.resourceValues(forKeys: [.totalFileAllocatedSizeKey, .fileAllocatedSizeKey])
            total += Int64(values?.totalFileAllocatedSize ?? values?.fileAllocatedSize ?? 0)
        }
        return total
    }

    // MARK: Install / remove

    /// Fetch a variant's weights, then load it once.
    ///
    /// The load is part of *installing* on purpose. WhisperKit pulls the
    /// tokenizer from a different Hugging Face repo the first time a model is
    /// loaded, and CoreML compiles the bundles on first use — leaving both to
    /// the first dictation would mean tapping the mic and waiting on the
    /// network with the overlay already open. Doing it here means the download
    /// screen is the only place that ever needs a connection.
    ///
    /// `onProgress` reports 0…1 across the whole operation: the byte download
    /// occupies the first 90%, the load the last 10% (it has no progress of
    /// its own, so it lands as a single step rather than a fake ramp).
    func install(variant: String, onProgress: @escaping @Sendable (Double) -> Void) async throws {
        if !Self.isInstalled(variant) {
            // Two attempts, because the first one after a hard kill reliably
            // fails and the second reliably works.
            //
            // The Hub downloader's background-session identifier is hardcoded
            // in the library, so it is process-wide and outlives the app. When
            // iOS relaunches after a kill, the new session adopts the dead
            // one's state ("will reconnect to existing state by requesting
            // pending callbacks") and the in-flight task dies with
            // NSURLErrorBackgroundSessionWasDisconnected (-997). Measured: the
            // immediate retry gets a clean session and resumes from the
            // `.incomplete` file, so this costs a moment rather than a restart.
            var lastError: Error?
            for attempt in 1 ... 2 {
                do {
                    try await downloadOnce(variant: variant, onProgress: onProgress)
                    lastError = nil
                    break
                } catch is CancellationError {
                    throw CancellationError()
                } catch {
                    lastError = error
                    if attempt == 1 {
                        Log.voice.error(
                            "whisper download attempt 1 failed, retrying: \(error.localizedDescription)")
                    }
                }
            }
            if let lastError {
                throw WhisperModelFailure.download(lastError.localizedDescription)
            }
            guard Self.isInstalled(variant) else {
                throw WhisperModelFailure.incomplete(Self.displayName(for: variant))
            }
        }
        onProgress(0.9)
        _ = try await kit(for: variant)
        onProgress(1)
    }

    private func downloadOnce(variant: String,
                              onProgress: @escaping @Sendable (Double) -> Void) async throws {
        _ = try await WhisperKit.download(
            variant: variant,
            downloadBase: Self.downloadBase,
            // Deliberately NOT a background session, despite that being the
            // obvious fix for "keep downloading while the user is elsewhere".
            //
            // The Hub downloader hardcodes its background-session identifier,
            // so the session is process-wide and outlives the app. Measured: if
            // the app is killed mid-download, the next launch's session adopts
            // the dead one's state, the first task dies with
            // NSURLErrorBackgroundSessionWasDisconnected (-997), and every
            // subsequent task on that identifier *hangs indefinitely* rather
            // than failing — no progress, no error, nothing to retry. A wedged
            // download that reports nothing is worse than one that stopped.
            //
            // What makes stopping survivable instead is the resume cache: the
            // partial `.incomplete` file persists, `WhisperDownloadCenter`
            // outlives the screen, and Resume continues from the exact byte via
            // a `Range` request. Interruption costs the seconds since the last
            // byte, not the download.
            useBackgroundSession: false,
            from: Self.repo,
            progressCallback: { progress in
                onProgress(min(0.9, progress.fractionCompleted * 0.9))
            })
    }

    /// Delete a variant's weights. Unloads it first when it's the live model,
    /// so CoreML isn't holding files that just vanished underneath it.
    ///
    /// Also clears the resume cache. Remove means remove: leaving the
    /// `.incomplete` files behind would keep charging the user disk for a
    /// download they just discarded, and Storage would under-report it.
    func remove(variant: String) throws {
        if loaded?.variant == variant { loaded = nil }
        for folder in [Self.folder(for: variant), Self.resumeCache(for: variant)]
        where FileManager.default.fileExists(atPath: folder.path) {
            try FileManager.default.removeItem(at: folder)
        }
    }

    /// Throw away a half-finished download without touching installed models.
    func discardPartial(variant: String) throws {
        let cache = Self.resumeCache(for: variant)
        guard FileManager.default.fileExists(atPath: cache.path) else { return }
        try FileManager.default.removeItem(at: cache)
    }

    /// Drop the loaded model. Called when dictation is disabled or the user
    /// picks a different engine — several hundred MB is too much to keep
    /// resident for a feature that isn't in use.
    func unload() {
        loaded = nil
    }

    /// Make sure a variant is loaded and ready to transcribe. Cheap after the
    /// first call in a process; the first call is where CoreML compilation and
    /// tokenizer setup are paid for.
    func warmUp(variant: String) async throws {
        _ = try await kit(for: variant)
    }

    // MARK: Transcription

    /// Transcribe 16 kHz mono samples. `language` is a Whisper code ("zh",
    /// "en", …); nil asks Whisper to detect it per window.
    ///
    /// Note the asymmetry with Apple's engines: this is a whole-utterance
    /// call, not a stream. Whisper has no notion of partial results — every
    /// pass re-reads all the audio it's given and may revise earlier words,
    /// which is exactly why it handles a sentence that switches language
    /// halfway through, and exactly why the caller treats its output as a
    /// replaceable hypothesis rather than an append-only transcript.
    func transcribe(samples: [Float], language: String?, variant: String) async throws -> String {
        guard samples.count >= Self.minimumSamples else { return "" }
        let kit = try await kit(for: variant)
        let options = DecodingOptions(
            verbose: false,
            task: .transcribe,
            language: language,
            temperature: 0,
            temperatureFallbackCount: 2,
            usePrefillPrompt: true,
            detectLanguage: language == nil,
            skipSpecialTokens: true,
            withoutTimestamps: true,
            wordTimestamps: false,
            promptTokens: promptTokens(for: language, kit: kit),
            compressionRatioThreshold: 2.4,
            logProbThreshold: -1.0,
            noSpeechThreshold: 0.6,
            chunkingStrategy: .vad)
        let results = try await kit.transcribe(audioArray: samples, decodeOptions: options)
        return results.map(\.text).joined(separator: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Whisper needs at least a window's worth of audio to say anything
    /// useful; below ~0.6 s it reliably hallucinates a stock phrase — an
    /// invented sentence is far worse here than an empty one, because the
    /// user is about to paste it into a shell. Enforced on both sides of the
    /// boundary: here so no caller can get a hallucination out of the model,
    /// and in the engine so a stray tap never reaches this far.
    static let minimumSamples = 9_600 // 0.6 s at 16 kHz

    /// A short primer in the target language, fed as Whisper's "previous
    /// text" context.
    ///
    /// This is the one lever that fixes two real defects for free. Whisper
    /// asked for `zh` emits **Traditional** characters about as often as
    /// Simplified, and a Simplified primer flips that; the primer also
    /// establishes that Latin words appearing mid-sentence are expected, which
    /// keeps it from transliterating "git rebase" into Chinese characters.
    private func promptTokens(for language: String?, kit: WhisperKit) -> [Int]? {
        guard let tokenizer = kit.tokenizer else { return nil }
        let primer: String
        switch language {
        case "zh":
            primer = "以下是普通话的转写，其中夹杂英文技术词汇，例如 git commit、Docker、SSH。"
        case "yue":
            primer = "以下是粵語嘅轉寫，其中夾雜英文技術詞彙，例如 git commit、Docker、SSH。"
        case "ja":
            primer = "以下は日本語の書き起こしで、git commit や Docker などの英語の技術用語が混在します。"
        default:
            return nil
        }
        let tokens = tokenizer.encode(text: primer)
        return tokens.isEmpty ? nil : tokens
    }

    // MARK: Loading

    private func kit(for variant: String) async throws -> WhisperKit {
        if let loaded, loaded.variant == variant { return loaded.kit }
        guard Self.isInstalled(variant) else {
            throw WhisperModelFailure.notInstalled(Self.displayName(for: variant))
        }
        // Release the previous model before building the next — two large
        // variants resident at once is an OOM on a phone.
        loaded = nil
        let config = WhisperKitConfig(
            model: variant,
            downloadBase: Self.downloadBase,
            modelRepo: Self.repo,
            modelFolder: Self.folder(for: variant).path,
            verbose: false,
            logLevel: .error,
            prewarm: false,
            load: true,
            download: false)
        let kit = try await WhisperKit(config)
        loaded = (variant, kit)
        Log.voice.info("whisper model loaded: \(variant, privacy: .public)")
        return kit
    }
}
