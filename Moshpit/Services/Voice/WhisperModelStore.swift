import Foundation
import WhisperKit
import MoshpitKit

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

// MARK: - Model seam

/// One loaded Whisper model, as the store drives it: the real one is
/// `WhisperKit` (``WhisperModelLoader/whisperKit``), tests stand in a scripted
/// one, since the real thing is hundreds of megabytes of CoreML.
///
/// Not Sendable, and never handed out: the store keeps it and runs one
/// transcription on it at a time.
protocol WhisperLoadedModel: AnyObject {
    /// Transcribe 16 kHz mono samples. `language` nil = let Whisper detect.
    /// `hotwords`: words the pane says to expect, for a model that takes them
    /// (Qwen3-ASR); Whisper's decoding ignores them.
    func transcribe(samples: [Float], language: String?, hotwords: [String]) async throws -> String
}

/// Where the store's models come from.
struct WhisperModelLoader: Sendable {
    /// Every CoreML bundle the variant needs is on disk.
    var isInstalled: @Sendable (String) -> Bool
    /// Build the variant's model from disk. Seconds, and hundreds of MB.
    var load: @Sendable (String) async throws -> any WhisperLoadedModel

    static let whisperKit = WhisperModelLoader(
        isInstalled: { WhisperModelStore.isInstalled($0) },
        load: { variant in
            switch WhisperModelStore.family(of: variant) {
            case .whisper: return try await WhisperKitModel.load(variant: variant)
            case .qwen3: return try await Qwen3ASRModel.load(folder: WhisperModelStore.folder(for: variant))
            }
        })
}

/// What runs a local model. The store, its downloads, the model page and the
/// dictation engine treat both alike; only fetching, telling whether a
/// download is complete, and loading differ.
enum LocalModelFamily: Sendable, Equatable {
    /// A WhisperKit (Core ML) variant from argmaxinc/whisperkit-coreml.
    case whisper
    /// Qwen3-ASR on sherpa-onnx (``Qwen3ASR``).
    case qwen3
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
///
/// Being an actor is not, on its own, what keeps those two promises. An actor
/// runs one call at a time only BETWEEN awaits: at every await — a
/// transcription running, a model loading — the next call gets in. So a
/// cancelled live preview still decoding ran side by side with the final pass
/// on the same instance, and tapping the mic while a just-downloaded model was
/// still loading loaded it a second time. Both are now explicit: transcriptions
/// take turns (``transcribe(samples:language:variant:)``), and a load in flight
/// is waited for rather than repeated (`model(for:)`).
actor WhisperModelStore {
    static let shared = WhisperModelStore()

    private let loader: WhisperModelLoader

    /// How long the model stays after its last use (B678).
    private let idleRelease: Duration

    init(loader: WhisperModelLoader = .whisperKit, idleRelease: Duration = .seconds(30)) {
        self.loader = loader
        self.idleRelease = idleRelease
    }

    /// Variants worth offering, best first.
    ///
    /// Deliberately **multilingual only**. The repo also publishes `.en`
    /// variants and the `distil-whisper` family, which are English-only —
    /// offering those would silently defeat the entire reason Whisper is here
    /// (a sentence that doesn't stay in one language). `recommendedModels()`
    /// then trims this to what the chip can actually run.
    private struct CuratedModel {
        let id: String
        let name: String
        let summary: String
        let bytes: Int64
        var family: LocalModelFamily = .whisper
    }

    /// Qwen3-ASR leads. On synthetic Chinese speech carrying English terms,
    /// with the pane's hotwords, it got 75% of the terms right where Large v3
    /// Turbo (language set to Chinese, as the app sets it) got 68%, at 10%
    /// mixed error against 16%; on 50 real code-switched clips (ASCEND) 13%
    /// against 30%. It dropped no sentence and fell into no repetition loop
    /// in either, and needs no language set to get an English sentence right
    /// (docs/nsh/ASR-2026-11.md).
    private static let curated: [CuratedModel] = [
        CuratedModel(id: Qwen3ASR.variant, name: "Qwen3-ASR 0.6B",
         summary: String(localized: "Best for Chinese with English terms in it. Listens for the file names and branches on screen."),
         bytes: Qwen3ASR.totalBytes, family: .qwen3),
        CuratedModel(id: "openai_whisper-large-v3_turbo_954MB", name: String(localized: "Large v3 Turbo"),
         summary: String(localized: "Most accurate, especially on mixed-language speech. Needs a recent chip."),
         bytes: 954 * 1_000_000),
        CuratedModel(id: "openai_whisper-small", name: String(localized: "Small"),
         summary: String(localized: "Good balance of accuracy and speed."),
         bytes: 470 * 1_000_000),
        CuratedModel(id: "openai_whisper-base", name: String(localized: "Base"),
         summary: String(localized: "Fast and light. Noticeably weaker outside English."),
         bytes: 145 * 1_000_000),
        CuratedModel(id: "openai_whisper-tiny", name: String(localized: "Tiny"),
         summary: String(localized: "Fastest, lowest accuracy. For older devices."),
         bytes: 75 * 1_000_000),
    ]

    /// The three CoreML bundles `WhisperKit.loadModels` requires. A folder
    /// missing any of them is a half-finished download, not an install.
    private static let requiredModels = ["MelSpectrogram", "AudioEncoder", "TextDecoder"]

    private static let repo = "argmaxinc/whisperkit-coreml"

    /// Currently loaded model, keyed by variant so switching models in
    /// Settings drops the old one instead of stacking two in memory.
    private var loaded: (variant: String, model: any WhisperLoadedModel)?

    /// The variant being loaded right now, if any. Anyone else who needs a
    /// model meanwhile waits in `loadWaiters` and looks again once it settles.
    private var loading: String?
    private var loadWaiters: [CheckedContinuation<Void, Never>] = []

    /// A transcription holds the model. The rest wait in `turnWaiters`,
    /// first come first served.
    private var transcribing = false
    private var turnWaiters: [CheckedContinuation<Void, Never>] = []

    /// Which dictation sessions are using `loaded`, and whether it should go
    /// once they're done.
    private var sessions = WhisperSessionLedger()

    /// The idle release waiting to fire, and which arming it belongs to: a
    /// session or a load since then moves `idleArming`, and a stale timer
    /// does nothing.
    private var idleTimer: Task<Void, Never>?
    private var idleArming = 0

    /// How many times each variant was removed: a load of a variant removed
    /// while it was loading keeps nothing once it lands (B124).
    private var removals: [String: Int] = [:]

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
        switch family(of: variant) {
        case .whisper:
            return downloadBase
                .appendingPathComponent("models", isDirectory: true)
                .appendingPathComponent(repo, isDirectory: true)
                .appendingPathComponent(variant, isDirectory: true)
        case .qwen3:
            return downloadBase
                .appendingPathComponent("sherpa", isDirectory: true)
                .appendingPathComponent(variant, isDirectory: true)
        }
    }

    /// Which runtime a catalog id belongs to. Anything not Qwen3-ASR's is a
    /// WhisperKit variant, as every stored id was before Qwen3-ASR.
    nonisolated static func family(of variant: String) -> LocalModelFamily {
        variant == Qwen3ASR.variant ? .qwen3 : .whisper
    }

    /// True only when every required CoreML bundle is on disk in full. For
    /// Qwen3-ASR: every pinned file in place at its size.
    nonisolated static func isInstalled(_ variant: String) -> Bool {
        let folder = folder(for: variant)
        if family(of: variant) == .qwen3 { return Qwen3ASR.isComplete(folder) }
        return isComplete(folder)
    }

    /// Every required bundle in `folder` downloaded in full (B671).
    ///
    /// The bundles' folders are no evidence: the Hub downloader fetches a
    /// variant one file at a time, in name order, and a bundle's folder
    /// appears with its first, smallest file. Large v3 Turbo's three folders
    /// all exist from the TextDecoder's first kilobyte on, while its 591 MB
    /// of weights — the longest stretch of the download, so the likeliest
    /// moment for a pause, a kill or a trip to the background — are still to
    /// come. Read as installed, the half-download offered no Resume, was
    /// picked as the model in use, failed to load, and dictation fell back to
    /// Apple's engine without a word.
    ///
    /// A bundle's last file is `weights/weight.bin`, and the downloader moves
    /// each file into place only once it is whole, so the weights being there
    /// means the bundle is. A bundle with no `model.mil` (a neural-network
    /// model, none of the curated ones) is judged by its `coremldata.bin`.
    nonisolated static func isComplete(_ folder: URL) -> Bool {
        let files = FileManager.default
        return requiredModels.allSatisfy { name in
            let compiled = folder.appendingPathComponent("\(name).mlmodelc")
            guard files.fileExists(atPath: compiled.path) else {
                return files.fileExists(atPath: folder.appendingPathComponent("\(name).mlpackage").path)
            }
            if files.fileExists(atPath: compiled.appendingPathComponent("model.mil").path) {
                return files.fileExists(atPath: compiled.appendingPathComponent("weights/weight.bin").path)
            }
            return files.fileExists(atPath: compiled.appendingPathComponent("coremldata.bin").path)
        }
    }

    // MARK: Catalog

    /// The curated list trimmed to what this chip supports, annotated with
    /// install state. Synchronous and offline — `recommendedModels()` reads a
    /// device table compiled into WhisperKit, so the Settings screen renders
    /// without touching the network.
    nonisolated static func catalog() -> [WhisperModelOption] {
        let supported = Set(WhisperKit.recommendedModels().supported)
        var offered = curated.filter { $0.family == .whisper && supported.contains($0.id) }
        // A chip missing from WhisperKit's table (or a table that disagrees
        // with our curation) must not produce an empty screen — fall back to
        // the two variants every supported device can run.
        if offered.isEmpty {
            offered = curated.filter { $0.id == "openai_whisper-base" || $0.id == "openai_whisper-tiny" }
        }
        // Qwen3-ASR by memory rather than WhisperKit's chip table; once on
        // the device it stays listed, so it can always be removed.
        if Qwen3ASR.isSupportedDevice || isInstalled(Qwen3ASR.variant) || partialBytes(for: Qwen3ASR.variant) > 0 {
            offered.insert(contentsOf: curated.filter { $0.family == .qwen3 }, at: 0)
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
        // Qwen3-ASR keeps its partial file beside the finished ones
        // (`<file>.part`): the folder is the whole story.
        if family(of: variant) == .qwen3 {
            return logicalSize(of: folder(for: variant)) { _ in true }
        }
        // Both halves, because a repo is fetched file by file: the one in
        // flight sits in the resume cache as `.incomplete`, and each finished
        // file MOVES out of that cache into the model folder. Counting only the
        // cache made the figure fall back every time a file completed — progress
        // appearing to go backwards is worse than no figure at all.
        return logicalSize(of: resumeCache(for: variant)) { $0.pathExtension == "incomplete" }
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
    /// Qwen3-ASR's partial files sit in its own folder (``ModelFileDownloader``).
    nonisolated static func resumeCache(for variant: String) -> URL {
        if family(of: variant) == .qwen3 { return folder(for: variant) }
        return downloadBase
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
        if !loader.isInstalled(variant) {
            // Two attempts. They date from when this downloaded on a background
            // session: its identifier is hardcoded in the Hub downloader, so
            // after a hard kill the relaunched app adopted the dead session and
            // the first task died with NSURLErrorBackgroundSessionWasDisconnected
            // (-997), while the immediate retry went through. Downloads no
            // longer use a background session (`downloadOnce` says why); the
            // retry stays because it costs a moment, not a restart — it resumes
            // from the `.incomplete` file.
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
            guard loader.isInstalled(variant) else {
                throw WhisperModelFailure.incomplete(Self.displayName(for: variant))
            }
        }
        onProgress(0.9)
        _ = try await model(for: variant)
        onProgress(1)
    }

    private func downloadOnce(variant: String,
                              onProgress: @escaping @Sendable (Double) -> Void) async throws {
        if Self.family(of: variant) == .qwen3 {
            try await ModelFileDownloader.fetch(
                Qwen3ASR.files, into: Self.folder(for: variant), url: Qwen3ASR.url(for:),
                onProgress: { onProgress(min(0.9, $0 * 0.9)) })
            return
        }
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
        // Still loading, it lands after this: `model(for:)` sees the count
        // moved and does not keep it.
        removals[variant, default: 0] += 1
        for folder in [Self.folder(for: variant), Self.resumeCache(for: variant)]
        where FileManager.default.fileExists(atPath: folder.path) {
            try FileManager.default.removeItem(at: folder)
        }
    }

    /// Throw away a half-finished download without touching installed models.
    func discardPartial(variant: String) throws {
        // Qwen3-ASR's resume cache is its folder: only a download that never
        // finished may go.
        if Self.family(of: variant) == .qwen3, loader.isInstalled(variant) { return }
        let cache = Self.resumeCache(for: variant)
        guard FileManager.default.fileExists(atPath: cache.path) else { return }
        try FileManager.default.removeItem(at: cache)
    }

    /// Drop the loaded model — several hundred MB is too much to keep
    /// resident for a feature that isn't in use. `WhisperModelResidency`
    /// calls this when voice input is switched off or moved to Apple's
    /// engine, and on a memory warning. A dictation in progress keeps the
    /// model until its session ends; the next one loads it again.
    func unload() {
        if sessions.requestUnload() { dropModel() }
    }

    /// A dictation session starts using the model: until `endSession`, an
    /// unload waits instead of pulling the model out from under it.
    func beginSession() -> UUID {
        disarmIdleRelease()
        return sessions.open()
    }

    func endSession(_ id: UUID) {
        if sessions.close(id) {
            dropModel()
        } else {
            armIdleRelease()
        }
    }

    /// The variant in memory, if any.
    var loadedVariant: String? { loaded?.variant }

    private func dropModel() {
        disarmIdleRelease()
        guard let variant = loaded?.variant else { return }
        loaded = nil
        Log.voice.info("voice unload: model=\(variant, privacy: .public)\(ProcessMemory.logSuffix, privacy: .public)")
    }

    // MARK: Idle release

    /// The model goes `idleRelease` after its last use (B678): once the last
    /// dictation ends, or once a load nobody is dictating with — an
    /// install's warm-up — lands. It used to stay until a memory warning or
    /// a settings change: up to 1.2 GB beside the live terminals for the rest
    /// of the app's life, making the jetsam kill that takes every session
    /// with it that much likelier. A dictation inside the window takes the
    /// model as it is; one after it loads it again (seconds).
    private func armIdleRelease() {
        disarmIdleRelease()
        guard loaded != nil, sessions.openSessions.isEmpty else { return }
        let arming = idleArming
        let delay = idleRelease
        idleTimer = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            await self?.idleExpired(arming)
        }
    }

    private func disarmIdleRelease() {
        idleArming += 1
        idleTimer?.cancel()
        idleTimer = nil
    }

    private func idleExpired(_ arming: Int) {
        guard arming == idleArming, sessions.openSessions.isEmpty, loading == nil, !transcribing else { return }
        Log.voice.info("voice idle: releasing the model after \(Self.seconds(self.idleRelease), privacy: .public)s unused")
        unload()
    }

    /// Make sure a variant is loaded and ready to transcribe. Cheap after the
    /// first call in a process; the first call is where CoreML compilation and
    /// tokenizer setup are paid for.
    func warmUp(variant: String) async throws {
        _ = try await model(for: variant)
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
    ///
    /// One at a time, in the order asked. A dictation's live preview is
    /// cancelled when the mic stops, but a pass already decoding runs on until
    /// WhisperKit next checks for cancellation — so the final pass waits for
    /// it here instead of decoding on the same instance beside it. A pass
    /// cancelled while it waits never starts.
    func transcribe(samples: [Float], language: String?, hotwords: [String] = [],
                    variant: String) async throws -> String {
        guard samples.count >= Self.minimumSamples else { return "" }
        let model = try await model(for: variant)
        await takeTurn()
        defer { passTurn() }
        try Task.checkCancellation()
        let started = ContinuousClock.now
        defer {
            // B677: every pass, previews included; the audio against the
            // time the model took on it, not the wait for a turn.
            let audio = Double(samples.count) / 16_000
            let (whole, attoseconds) = (ContinuousClock.now - started).components
            let decode = Double(whole) + Double(attoseconds) / 1e18
            let line = String(format: "audio=%.2fs decode=%.2fs rtf=%.2f", audio, decode, decode / max(audio, 0.01))
            Log.voice.info(
                "voice decode: model=\(variant, privacy: .public) \(line, privacy: .public) hotwords=\(hotwords.count, privacy: .public)")
        }
        // A decoder stuck repeating itself — Whisper's temperature fallback
        // doesn't always get it out, and Fun-ASR-Nano did it too in the
        // eval — is kept to one copy (B670).
        let text = try await model.transcribe(samples: samples, language: language, hotwords: hotwords)
        return DictationTranscript.collapsingRepeats(text)
    }

    /// A duration in seconds, two decimals, for log lines.
    nonisolated static func seconds(_ duration: Duration) -> String {
        let (whole, attoseconds) = duration.components
        return String(format: "%.2f", Double(whole) + Double(attoseconds) / 1e18)
    }

    /// Whisper needs at least a window's worth of audio to say anything
    /// useful; below ~0.6 s it reliably hallucinates a stock phrase — an
    /// invented sentence is far worse here than an empty one, because the
    /// user is about to paste it into a shell. Enforced on both sides of the
    /// boundary: here so no caller can get a hallucination out of the model,
    /// and in the engine so a stray tap never reaches this far.
    static let minimumSamples = 9_600 // 0.6 s at 16 kHz

    private func takeTurn() async {
        guard transcribing else {
            transcribing = true
            return
        }
        // Resumed by `passTurn`, which hands the turn over without letting
        // it go — nobody can slip in between.
        await withCheckedContinuation { turnWaiters.append($0) }
    }

    private func passTurn() {
        if turnWaiters.isEmpty {
            transcribing = false
        } else {
            turnWaiters.removeFirst().resume()
        }
    }

    // MARK: Loading

    /// The loaded model for `variant`, loading it if it isn't.
    ///
    /// A load already in flight is waited for, not repeated: the mic tapped
    /// while a just-downloaded model was still loading used to start a second
    /// load of it — two copies in memory, and the first thrown away.
    private func model(for variant: String) async throws -> any WhisperLoadedModel {
        // Whatever is loading may be this very variant; once it settles, look
        // again (another waiter may have started a load by then, too).
        while loading != nil {
            await withCheckedContinuation { loadWaiters.append($0) }
        }
        if let loaded, loaded.variant == variant { return loaded.model }
        guard loader.isInstalled(variant) else {
            throw WhisperModelFailure.notInstalled(Self.displayName(for: variant))
        }
        // Release the previous model before building the next — two large
        // variants resident at once is an OOM on a phone.
        loaded = nil
        loading = variant
        defer {
            loading = nil
            let waiting = loadWaiters
            loadWaiters.removeAll()
            for waiter in waiting { waiter.resume() }
        }
        let generation = sessions.generation
        let removal = removals[variant, default: 0]
        let started = ContinuousClock.now
        let model = try await loader.load(variant)
        // B677: what a device check reads, numbers only.
        let loadSeconds = Self.seconds(ContinuousClock.now - started)
        Log.voice.info(
            "voice load: model=\(variant, privacy: .public) seconds=\(loadSeconds, privacy: .public)\(ProcessMemory.logSuffix, privacy: .public)")
        // An unload, or a removal of this variant, that landed while this
        // loaded wins: the caller gets the model for this one use, but it is
        // not kept.
        guard sessions.generation == generation, removals[variant, default: 0] == removal else { return model }
        loaded = (variant, model)
        Log.voice.info("whisper model loaded: \(variant, privacy: .public)")
        // Loaded with nobody dictating (an install's warm-up): it goes
        // unless a dictation takes it in time.
        armIdleRelease()
        return model
    }

    /// Builds a variant's WhisperKit from the downloaded folder.
    fileprivate static func loadWhisperKit(variant: String) async throws -> WhisperKit {
        let config = WhisperKitConfig(
            model: variant,
            downloadBase: downloadBase,
            modelRepo: repo,
            modelFolder: folder(for: variant).path,
            verbose: false,
            logLevel: .error,
            prewarm: false,
            load: true,
            download: false)
        return try await WhisperKit(config)
    }
}

/// WhisperKit, decoding the way dictation wants it.
private final class WhisperKitModel: WhisperLoadedModel {
    private let kit: WhisperKit

    private init(kit: WhisperKit) {
        self.kit = kit
    }

    static func load(variant: String) async throws -> WhisperKitModel {
        WhisperKitModel(kit: try await WhisperModelStore.loadWhisperKit(variant: variant))
    }

    func transcribe(samples: [Float], language: String?, hotwords _: [String]) async throws -> String {
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
            promptTokens: promptTokens(for: language),
            compressionRatioThreshold: 2.4,
            logProbThreshold: -1.0,
            noSpeechThreshold: 0.6,
            chunkingStrategy: .vad)
        let results = try await kit.transcribe(audioArray: samples, decodeOptions: options)
        let text = results.map(\.text).joined(separator: " ")
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// A short primer in the target language, fed as Whisper's "previous
    /// text" context, for a pinned Cantonese or Japanese: it establishes that
    /// Latin words mid-sentence are expected, so "git rebase" isn't
    /// transliterated.
    ///
    /// Not for Chinese any more (B670). On 50 real Mandarin–English clips
    /// the Mandarin primer took Whisper from 15.8% to 30.2% mixed error —
    /// English words rewritten ("friends" → "French、French、French"), two
    /// sentences dropped, one looped — with no English word gained, and to
    /// 34.4% under auto-detect. Its other job, Simplified rather than
    /// Traditional characters, is done by converting the output to the
    /// user's script (`DictationTranscript.converting`). The Cantonese and
    /// Japanese primers were not measured.
    private func promptTokens(for language: String?) -> [Int]? {
        guard let tokenizer = kit.tokenizer else { return nil }
        let primer: String
        switch language {
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
}
