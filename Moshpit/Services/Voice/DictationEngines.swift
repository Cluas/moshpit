import AVFoundation
import Foundation
import Speech

// MARK: - Engine selection

/// Builds the ordered list of engines a session should try.
///
/// With Whisper selected the local model leads and Apple's engines stay
/// behind it as a fallback — a model that failed to load should cost accuracy,
/// not the ability to dictate at all. With Apple selected, the preference is:
///
/// 1. **SpeechTranscriber** (iOS 26+) — Apple's large on-device speech model
///    (the SpeechAnalyzer stack). Best accuracy, fully local, no speech-
///    recognition permission needed; assets download on first use.
/// 2. **DictationTranscriber** (iOS 26+) — the on-device keyboard-dictation
///    model, covering many locales the large model doesn't yet.
/// 3. **SFSpeechRecognizer** (iOS 18–25) — *on-device only*. The server
///    variant is deliberately never used: the mic/speech purpose strings and
///    the privacy label promise audio never leaves the device, so a locale
///    without on-device support reads as unavailable rather than silently
///    shipping audio to Apple.
enum DictationEngineFactory {
    /// All engines that can serve the request, best first. The controller
    /// works down the list at prepare/start time: a transcriber whose locale
    /// is *listed* can still fail to obtain assets in a given environment
    /// (Simulators can't install the large-model assets at all, and a
    /// device can hit the reserved-locales cap), and the session should
    /// degrade to the next engine rather than die with a daemon error.
    ///
    /// `spokenLanguages` is the caller's ordered guess at what the user
    /// speaks — see `VoiceLanguageResolver`. It only matters when the stored
    /// language is Automatic.
    static func candidates(request: DictationRequest,
                           spokenLanguages: [String]) async throws -> [DictationEngine] {
        var engines: [DictationEngine] = []
        if request.engine == .whisper,
           let variant = WhisperModelStore.resolvedVariant(preferring: request.whisperModelId) {
            let language = VoiceLanguageResolver.whisperCode(stored: request.whisperLanguage)
            engines.append(WhisperDictationEngine(
                store: WhisperModelStore.shared,
                variant: variant,
                language: language,
                label: whisperLabel(variant: variant, language: language)))
        }
        let locale = await appleLocale(request: request, spokenLanguages: spokenLanguages)
        engines.append(contentsOf: await appleEngines(locale: locale))
        guard !engines.isEmpty else {
            if request.engine == .whisper, request.whisperModelId.isEmpty,
               WhisperModelStore.resolvedVariant(preferring: "") == nil {
                throw DictationFailure.noWhisperModel
            }
            throw DictationFailure.unsupportedLanguage(
                Locale.current.localizedString(forIdentifier: locale.identifier) ?? locale.identifier)
        }
        return engines
    }

    /// Apple's transcribers for a locale, best first. Empty when none of them
    /// can do that language on this device.
    private static func appleEngines(locale: Locale) async -> [DictationEngine] {
        var engines: [DictationEngine] = []
        if #available(iOS 26.0, *) {
            if let match = await SpeechTranscriber.supportedLocale(equivalentTo: locale) {
                engines.append(ModernDictationEngine(module: .advanced(SpeechTranscriber(
                    locale: match,
                    transcriptionOptions: [],
                    reportingOptions: [.volatileResults],
                    attributeOptions: []))))
            }
            if let match = await DictationTranscriber.supportedLocale(equivalentTo: locale) {
                engines.append(ModernDictationEngine(module: .dictation(DictationTranscriber(
                    locale: match, preset: .progressiveLongDictation))))
            }
        }
        if let recognizer = SFSpeechRecognizer(locale: locale),
           recognizer.supportsOnDeviceRecognition {
            engines.append(LegacyDictationEngine(recognizer: recognizer))
        }
        return engines
    }

    /// Resolve Automatic against what Apple can actually transcribe here.
    ///
    /// The check is a real capability probe rather than a lookup table: asking
    /// each candidate language whether *some* engine claims it is the only way
    /// to avoid picking a language the device would then refuse.
    private static func appleLocale(request: DictationRequest,
                                    spokenLanguages: [String]) async -> Locale {
        if !request.appleLocaleId.isEmpty { return Locale(identifier: request.appleLocaleId) }
        for identifier in spokenLanguages {
            let locale = Locale(identifier: identifier)
            if await !appleEngines(locale: locale).isEmpty { return locale }
        }
        return spokenLanguages.first.map(Locale.init(identifier:)) ?? Locale.current
    }

    private static func whisperLabel(variant: String, language: String?) -> String {
        let model = WhisperModelStore.displayName(for: variant)
        let languageName = language.flatMap {
            Locale.current.localizedString(forLanguageCode: $0)
        } ?? String(localized: "Auto")
        return "\(model) · \(languageName)"
    }
}

// MARK: - SpeechAnalyzer engine (iOS 26+)

/// The SpeechAnalyzer pipeline: mic buffers → format conversion →
/// `AnalyzerInput` stream → transcriber module → results. Model assets are
/// system-managed (`AssetInventory`) and shared across apps, so the first use
/// of a language may download once and every later session is instant.
@available(iOS 26.0, *)
final class ModernDictationEngine: DictationEngine {
    /// Which on-device module is transcribing. Both flavors expose the same
    /// result shape but are unrelated concrete types, hence the enum rather
    /// than a generic (the module's Result associated type doesn't carry
    /// `text` at the protocol level).
    enum Module {
        case advanced(SpeechTranscriber)
        case dictation(DictationTranscriber)

        var speechModule: any SpeechModule {
            switch self {
            case .advanced(let transcriber): return transcriber
            case .dictation(let transcriber): return transcriber
            }
        }
    }

    let updates: AsyncThrowingStream<DictationUpdate, Error>

    var label: String {
        let name = selectedLocale.flatMap {
            Locale.current.localizedString(forIdentifier: $0.identifier(.bcp47))
        } ?? String(localized: "Unknown")
        switch module {
        case .advanced: return String(localized: "Apple · \(name)")
        case .dictation: return String(localized: "Apple Dictation · \(name)")
        }
    }

    private let module: Module
    private let updateContinuation: AsyncThrowingStream<DictationUpdate, Error>.Continuation
    private let converter = DictationBufferConverter()
    private var analyzer: SpeechAnalyzer?
    private var inputContinuation: AsyncStream<AnalyzerInput>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var resultsTask: Task<Void, Never>?
    /// Set by `cancel()` so a torn-down results stream ends quietly instead
    /// of surfacing a cancellation as a user-visible error.
    private var cancelled = false

    /// Transcript assembly (results-task context only): finals accumulate,
    /// the volatile hypothesis is replaced by every non-final result.
    private var finalized = ""

    init(module: Module) {
        self.module = module
        (updates, updateContinuation) = AsyncThrowingStream.makeStream()
    }

    /// The locale the module was built for — needed to reserve its assets.
    private var selectedLocale: Locale? {
        switch module {
        case .advanced(let transcriber): return transcriber.selectedLocales.first
        case .dictation(let transcriber): return transcriber.selectedLocales.first
        }
    }

    func prepare(onProgress: @escaping @Sendable (DictationPreparation) -> Void) async throws {
        if await AssetInventory.status(forModules: [module.speechModule]) == .installed {
            return
        }
        // The app must hold a reservation ("subscription") on the locale
        // before it may even ask about its assets — skipping this fails
        // with "not subscribed to transcription.<lang>". Reservations are
        // capped (maximumReservedLocales), so a failed reserve isn't fatal
        // here: let the install request report the real story.
        if let locale = selectedLocale {
            let reserved = await AssetInventory.reservedLocales
            if !reserved.contains(where: { $0.identifier(.bcp47) == locale.identifier(.bcp47) }) {
                do {
                    try await AssetInventory.reserve(locale: locale)
                } catch {
                    Log.voice.error("locale reservation failed: \(error.localizedDescription)")
                }
            }
        }
        // nil request = assets already installed. Otherwise this is the
        // one-time per-language model download (system-managed, shared).
        guard let request = try await AssetInventory.assetInstallationRequest(
            supporting: [module.speechModule]) else { return }
        let progress = request.progress
        let poll = Task {
            while !Task.isCancelled {
                onProgress(.downloading(progress: progress.fractionCompleted))
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
        defer { poll.cancel() }
        try await request.downloadAndInstall()
    }

    func start() async throws {
        let analyzer = SpeechAnalyzer(modules: [module.speechModule])
        self.analyzer = analyzer
        analyzerFormat = await SpeechAnalyzer.bestAvailableAudioFormat(
            compatibleWith: [module.speechModule])
        let (inputSequence, inputContinuation) = AsyncStream<AnalyzerInput>.makeStream()
        self.inputContinuation = inputContinuation
        // Subscribe to results BEFORE starting analysis so nothing is missed.
        resultsTask = Task { [weak self] in await self?.consumeResults() }
        try await analyzer.start(inputSequence: inputSequence)
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        guard let inputContinuation else { return }
        do {
            let converted: AVAudioPCMBuffer
            if let analyzerFormat {
                converted = try converter.convert(buffer, to: analyzerFormat)
            } else {
                converted = buffer
            }
            inputContinuation.yield(AnalyzerInput(buffer: converted))
        } catch {
            // A malformed buffer isn't fatal to the session — drop it.
            Log.voice.error("buffer conversion failed: \(error.localizedDescription)")
        }
    }

    func finishAudio() async {
        inputContinuation?.finish()
        inputContinuation = nil
        // Flush the tail of the audio into a final result; the results
        // stream then ends, which ends `updates` via consumeResults().
        try? await analyzer?.finalizeAndFinishThroughEndOfInput()
        await resultsTask?.value
    }

    func cancel() {
        cancelled = true
        inputContinuation?.finish()
        inputContinuation = nil
        resultsTask?.cancel()
        updateContinuation.finish()
        let analyzer = analyzer
        Task { await analyzer?.cancelAndFinishNow() }
    }

    private func consumeResults() async {
        do {
            switch module {
            case .advanced(let transcriber):
                for try await result in transcriber.results {
                    ingest(text: String(result.text.characters), isFinal: result.isFinal)
                }
            case .dictation(let transcriber):
                for try await result in transcriber.results {
                    ingest(text: String(result.text.characters), isFinal: result.isFinal)
                }
            }
            updateContinuation.finish()
        } catch {
            if cancelled || error is CancellationError {
                updateContinuation.finish()
            } else {
                updateContinuation.finish(throwing: error)
            }
        }
    }

    private func ingest(text: String, isFinal: Bool) {
        var volatileText = ""
        if isFinal {
            finalized = DictationTranscript.join(finalized, text)
        } else {
            volatileText = text
        }
        updateContinuation.yield(DictationUpdate(
            finalizedText: finalized, volatileText: volatileText))
    }
}

/// Converts mic-native buffers (typically 48 kHz float) to the analyzer's
/// preferred format (typically 16 kHz). One converter instance is reused
/// across buffers; `primeMethod = .none` avoids the converter swallowing the
/// first frames of real-time audio as priming.
final class DictationBufferConverter {
    private var converter: AVAudioConverter?

    func convert(_ buffer: AVAudioPCMBuffer, to format: AVAudioFormat) throws -> AVAudioPCMBuffer {
        guard buffer.format != format else { return buffer }
        if converter == nil
            || converter?.inputFormat != buffer.format
            || converter?.outputFormat != format {
            converter = AVAudioConverter(from: buffer.format, to: format)
            converter?.primeMethod = .none
        }
        guard let converter else {
            throw DictationFailure.engine(String(localized: "Audio format conversion unavailable."))
        }
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 16
        guard let converted = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else {
            throw DictationFailure.engine(String(localized: "Audio buffer allocation failed."))
        }
        var consumed = false
        var conversionError: NSError?
        let status = converter.convert(to: converted, error: &conversionError) { _, inputStatus in
            if consumed {
                inputStatus.pointee = .noDataNow
                return nil
            }
            consumed = true
            inputStatus.pointee = .haveData
            return buffer
        }
        if let conversionError { throw conversionError }
        guard status != .error else {
            throw DictationFailure.engine(String(localized: "Audio format conversion failed."))
        }
        return converted
    }
}

// MARK: - SFSpeechRecognizer engine (iOS 18–25 fallback)

/// The pre-iOS-26 Speech API. Unlike SpeechAnalyzer it needs the user's
/// speech-recognition authorization, and it re-reports the whole utterance on
/// every partial callback (so no accumulation here — the latest callback IS
/// the transcript). Recognition is ALWAYS forced on-device — the factory only
/// mints this engine when the device supports it, keeping the "audio never
/// leaves the device" promise unconditional.
final class LegacyDictationEngine: DictationEngine {
    let updates: AsyncThrowingStream<DictationUpdate, Error>

    var label: String {
        let name = Locale.current.localizedString(forIdentifier: recognizer.locale.identifier)
            ?? recognizer.locale.identifier
        return String(localized: "Apple Speech · \(name)")
    }

    private let recognizer: SFSpeechRecognizer
    private let updateContinuation: AsyncThrowingStream<DictationUpdate, Error>.Continuation
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var cancelled = false

    init(recognizer: SFSpeechRecognizer) {
        self.recognizer = recognizer
        (updates, updateContinuation) = AsyncThrowingStream.makeStream()
    }

    func prepare(onProgress _: @escaping @Sendable (DictationPreparation) -> Void) async throws {
        let status = await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: $0) }
        }
        guard status == .authorized else { throw DictationFailure.speechRecognitionDenied }
    }

    func start() async throws {
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.taskHint = .dictation
        request.addsPunctuation = true
        // Unconditional — never fall back to Apple's server (see class doc).
        request.requiresOnDeviceRecognition = true
        self.request = request
        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            guard let self else { return }
            if let result {
                let text = result.bestTranscription.formattedString
                if result.isFinal {
                    updateContinuation.yield(DictationUpdate(finalizedText: text, volatileText: ""))
                    updateContinuation.finish()
                    return
                }
                updateContinuation.yield(DictationUpdate(finalizedText: "", volatileText: text))
            }
            if let error {
                // "No speech detected" / cancellation are clean ends of a
                // session, not failures worth an alert.
                let code = (error as NSError).code
                if cancelled || code == 216 || code == 1110 || code == 301 {
                    updateContinuation.finish()
                } else {
                    updateContinuation.finish(throwing: error)
                }
            }
        }
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        request?.append(buffer)
    }

    func finishAudio() async {
        // The task delivers its final result after endAudio(), which ends
        // `updates` in the handler above — the controller awaits that.
        request?.endAudio()
    }

    func cancel() {
        cancelled = true
        request?.endAudio()
        task?.cancel()
        updateContinuation.finish()
    }
}

// MARK: - Microphone capture

/// AVAudioEngine wrapper feeding raw input-format buffers to the engine and a
/// 0…1 loudness to the overlay's level meter. Session category stays
/// `.playAndRecord` so a terminal bell can still sound during dictation.
final class MicrophoneAudioSource: DictationAudioSource {
    private let audioEngine = AVAudioEngine()
    private var tapInstalled = false
    private var interruptionObserver: NSObjectProtocol?

    func requestPermission() async -> Bool {
        switch AVAudioApplication.shared.recordPermission {
        case .granted: return true
        case .denied: return false
        default: return await AVAudioApplication.requestRecordPermission()
        }
    }

    func start(onBuffer: @escaping @Sendable (AVAudioPCMBuffer) -> Void,
               onLevel: @escaping @Sendable (Float) -> Void,
               onInterruption: @escaping @Sendable () -> Void) throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .spokenAudio,
                                options: [.duckOthers, .defaultToSpeaker, .allowBluetoothHFP])
        try session.setActive(true, options: [])

        let input = audioEngine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw DictationFailure.audioCapture(String(localized: "No audio input is available."))
        }
        if tapInstalled { input.removeTap(onBus: 0) }
        input.installTap(onBus: 0, bufferSize: 4096, format: format) { buffer, _ in
            onBuffer(buffer)
            onLevel(Self.loudness(of: buffer))
        }
        tapInstalled = true
        audioEngine.prepare()
        try audioEngine.start()

        interruptionObserver = NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification,
            object: session, queue: nil
        ) { note in
            let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt
            if raw.flatMap(AVAudioSession.InterruptionType.init(rawValue:)) == .began {
                onInterruption()
            }
        }
    }

    func stop() {
        if let interruptionObserver {
            NotificationCenter.default.removeObserver(interruptionObserver)
            self.interruptionObserver = nil
        }
        if tapInstalled {
            audioEngine.inputNode.removeTap(onBus: 0)
            tapInstalled = false
        }
        if audioEngine.isRunning { audioEngine.stop() }
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    /// RMS mapped through dB so the meter moves in speech's dynamic range
    /// (-50 dB floor → 0, full scale → 1) instead of hugging zero.
    private static func loudness(of buffer: AVAudioPCMBuffer) -> Float {
        guard let channel = buffer.floatChannelData?[0] else { return 0 }
        let frames = Int(buffer.frameLength)
        guard frames > 0 else { return 0 }
        var sum: Float = 0
        for i in 0 ..< frames { sum += channel[i] * channel[i] }
        let rms = sqrt(sum / Float(frames))
        let db = 20 * log10(max(rms, 1e-7))
        return max(0, min(1, (db + 50) / 50))
    }
}

// MARK: - Language catalog (Settings picker)

/// Every locale some engine on this device can transcribe, deduplicated
/// across the three engines and sorted by display name. Async because the
/// iOS 26 module lists are.
enum VoiceLocaleCatalog {
    struct Option: Identifiable, Equatable {
        /// BCP-47 identifier ("en-US", "zh-CN") — stored in settings.
        let id: String
        /// Localized display name in the user's UI language.
        let name: String
    }

    static func options() async -> [Option] {
        var ids = Set<String>()
        if #available(iOS 26.0, *) {
            for locale in await SpeechTranscriber.supportedLocales {
                ids.insert(locale.identifier(.bcp47))
            }
            for locale in await DictationTranscriber.supportedLocales {
                ids.insert(locale.identifier(.bcp47))
            }
        }
        for locale in SFSpeechRecognizer.supportedLocales() {
            // Only locales the legacy engine can do ON-DEVICE — the server
            // path is never used (see DictationEngineFactory), so a locale
            // that would need it must not be offered.
            guard SFSpeechRecognizer(locale: locale)?.supportsOnDeviceRecognition == true else {
                continue
            }
            ids.insert(locale.identifier(.bcp47))
        }
        let ui = Locale.current
        return ids
            .map { Option(id: $0, name: ui.localizedString(forIdentifier: $0) ?? $0) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}
