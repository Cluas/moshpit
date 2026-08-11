import AVFoundation
import Foundation
import Observation

/// One incremental update from a dictation engine: the complete transcript
/// state so far, not a delta. Engines own the assembly because the two
/// recognition APIs report differently — SpeechAnalyzer emits *segments*
/// (finalized text accumulates, the volatile hypothesis is replaced), while
/// SFSpeechRecognizer re-reports the *whole utterance* on every callback.
/// Normalizing here keeps the controller and UI dumb.
struct DictationUpdate: Equatable, Sendable {
    /// Text the engine has committed — it will not change anymore.
    var finalizedText: String
    /// The engine's current in-flight hypothesis for the audio after
    /// `finalizedText`. Rendered dimmer in the UI; replaced on every update.
    var volatileText: String
}

/// What `prepare()` is currently doing, so the overlay can say which of the
/// two very different waits the user is looking at — fetching hundreds of
/// megabytes, or loading bytes that are already on disk.
enum DictationPreparation: Equatable, Sendable {
    /// Fetching model assets. 0…1 where the source reports progress.
    case downloading(progress: Double)
    /// Assets are local; CoreML or the speech daemon is loading them.
    case loading
}

/// Where a dictation session currently is. Drives the overlay's status line
/// and which controls make sense (Insert is only offered while listening).
enum DictationPhase: Equatable {
    case idle
    /// Permission checks + engine selection underway (sub-second normally).
    case starting
    /// First use of an on-device model locale: the system is downloading its
    /// speech assets. `progress` is 0…1 where known.
    case downloadingModel(progress: Double)
    /// Assets are on disk and being loaded into memory. Distinct from
    /// `downloadingModel` because it needs no network and is bounded — a
    /// Whisper model can spend several seconds here on first load while
    /// CoreML compiles it, and "Downloading 100%" sitting still is alarming.
    case loadingModel
    case listening
    /// Mic stopped; waiting for the engine to finalize the tail of the audio.
    case finishing
    /// The system took the microphone (call, Siri). Whatever was heard is
    /// kept — Insert still works; only new audio is off the table.
    case interrupted
    case failed(DictationFailure)
}

enum DictationFailure: Equatable, Error {
    case microphoneDenied
    case speechRecognitionDenied
    /// No engine on this device/OS can transcribe the chosen language.
    case unsupportedLanguage(String)
    /// Whisper is selected but no model is on disk. Recoverable, and the fix
    /// is a specific place in Settings — so it says where.
    case noWhisperModel
    case audioCapture(String)
    case engine(String)

    /// User-facing message for the overlay's failed state.
    var message: String {
        switch self {
        case .microphoneDenied:
            return String(localized: "Microphone access is off. Enable it in Settings → Privacy → Microphone.")
        case .speechRecognitionDenied:
            return String(localized: "Speech recognition is off. Enable it in Settings → Privacy → Speech Recognition.")
        case .unsupportedLanguage(let name):
            return String(localized: "Dictation isn't available for \(name) on this device.")
        case .noWhisperModel:
            return String(localized: "No Whisper model is downloaded yet. Pick one in Settings → Voice Input → Model.")
        case .audioCapture(let detail):
            return String(localized: "Couldn't start the microphone: \(detail)")
        case .engine(let detail):
            return detail
        }
    }
}

/// Everything a session needs to choose an engine and a language. Assembled
/// from `AppSettings` at the moment the mic key is tapped, so changing any of
/// it in Settings takes effect on the next session with no extra plumbing.
struct DictationRequest: Equatable, Sendable {
    var engine: VoiceEngineKind = .apple
    /// BCP-47 for the Apple engines; "" means Automatic.
    var appleLocaleId: String = ""
    /// Whisper language code ("zh", "en", …); "" means let Whisper detect.
    var whisperLanguage: String = ""
    /// Whisper variant; "" means use the best installed one.
    var whisperModelId: String = ""
}

extension DictationFailure: LocalizedError {
    /// So a failure that leaks into a generic `localizedDescription` still
    /// reads as its real message, not "error 4".
    var errorDescription: String? { message }
}

/// A speech-to-text engine the controller can drive. Two implementations:
/// `ModernDictationEngine` (iOS 26+, SpeechAnalyzer's on-device models) and
/// `LegacyDictationEngine` (SFSpeechRecognizer, iOS 18–25). Tests inject a
/// scripted fake through `VoiceDictationController`'s factory.
protocol DictationEngine: AnyObject {
    /// Complete-state updates, finishing when the engine has delivered its
    /// final result (after `finishAudio()`) or throwing on recognition error.
    var updates: AsyncThrowingStream<DictationUpdate, Error> { get }
    /// Short label naming the engine and language actually in play — shown on
    /// the overlay so a session running in the wrong language is visible
    /// *before* the user talks to it for a minute and gets nonsense back.
    var label: String { get }
    /// How long `finishAudio()` may take before the controller gives up and
    /// keeps whatever transcript exists. Engines that stream have delivered
    /// nearly everything by then; Whisper does all its work here and needs
    /// room proportional to the utterance.
    var finalizeTimeout: Duration { get }
    /// Authorization + model assets. May be slow on first use of a locale
    /// (model download, then load) — progress lands in `onProgress`.
    func prepare(onProgress: @escaping @Sendable (DictationPreparation) -> Void) async throws
    /// Begin accepting audio. Only valid after `prepare()`.
    func start() async throws
    /// Feed one microphone buffer. Called on the audio tap's thread.
    func append(_ buffer: AVAudioPCMBuffer)
    /// No more audio is coming — finalize and end `updates`.
    func finishAudio() async
    /// Abandon the session; `updates` ends without further results.
    func cancel()
}

extension DictationEngine {
    /// Streaming engines have essentially finished by the time the mic stops.
    var finalizeTimeout: Duration { .seconds(5) }
}

/// Microphone capture, abstracted so controller tests never touch real audio
/// hardware (Simulator CI has no deterministic mic).
protocol DictationAudioSource: AnyObject {
    /// Ask for (or verify) record permission. False = denied.
    func requestPermission() async -> Bool
    /// Start capturing. `onBuffer` fires on the audio thread with raw mic
    /// buffers; `onLevel` fires with a 0…1 loudness for the level meter;
    /// `onInterruption` fires when the system takes the mic away (phone call).
    func start(onBuffer: @escaping @Sendable (AVAudioPCMBuffer) -> Void,
               onLevel: @escaping @Sendable (Float) -> Void,
               onInterruption: @escaping @Sendable () -> Void) throws
    func stop()
}

/// Drives one dictation session: mic permission → engine prep (model
/// download) → live transcription → insert-or-cancel. Owned by the terminal
/// screen while the overlay is up; thrown away afterwards.
///
/// The transcript is buffered here and only *sent to the remote* when the
/// user taps Insert — dictating straight into a shell would make every
/// mis-hearing an executed keystroke. (Blink's Prompt Mode and Termius's
/// voice typing made the same call: compose first, commit once.)
@Observable
@MainActor
final class VoiceDictationController {
    private(set) var phase: DictationPhase = .idle
    private(set) var finalizedText = ""
    private(set) var volatileText = ""
    /// Mic loudness 0…1 for the overlay's level meter.
    private(set) var level: Float = 0
    /// Engine + language of the running session, for the overlay's chip.
    /// Empty until an engine has been selected.
    private(set) var engineLabel = ""

    /// Everything the user has said so far, as it would be inserted.
    var transcript: String {
        DictationTranscript.join(finalizedText, volatileText)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var isListening: Bool { phase == .listening }

    @ObservationIgnored
    private let makeEngines: (DictationRequest, [String]) async throws -> [DictationEngine]
    @ObservationIgnored private let audioSource: DictationAudioSource
    @ObservationIgnored private var engine: DictationEngine?
    @ObservationIgnored private var consumeTask: Task<Void, Never>?

    init(engineFactory: @escaping (DictationRequest, [String]) async throws -> [DictationEngine]
             = DictationEngineFactory.candidates,
         audioSource: DictationAudioSource = MicrophoneAudioSource()) {
        makeEngines = engineFactory
        self.audioSource = audioSource
    }

    /// Kick off a session. Safe to call only from `.idle`/`.failed`; anything
    /// else is ignored.
    func start(_ request: DictationRequest) async {
        switch phase {
        case .idle, .failed: break
        default: return
        }
        phase = .starting
        finalizedText = ""
        volatileText = ""
        level = 0
        engineLabel = ""

        guard await audioSource.requestPermission() else {
            phase = .failed(.microphoneDenied)
            return
        }

        // Work down the candidate list: an engine whose locale looked fine
        // can still fail to prepare (asset install unavailable, reservation
        // cap) — degrade to the next one and only fail when all did.
        let onProgress: @Sendable (DictationPreparation) -> Void = { [weak self] step in
            Task { @MainActor [weak self] in
                // Don't regress the phase if listening already started
                // (progress callbacks can trail the install completing).
                guard let self, phase == .starting || isPreparing else { return }
                switch step {
                case .downloading(let progress): phase = .downloadingModel(progress: progress)
                case .loading: phase = .loadingModel
                }
            }
        }
        // Resolved here rather than inside the factory: reading the user's
        // language ranking and installed keyboards is main-actor work, and
        // this is the main actor.
        let spoken = VoiceLanguageResolver.spokenLanguageCandidates()
        var selected: DictationEngine?
        var lastError: Error?
        do {
            for candidate in try await makeEngines(request, spoken) {
                do {
                    try await candidate.prepare(onProgress: onProgress)
                    try await candidate.start()
                    selected = candidate
                    break
                } catch let failure as DictationFailure where failure == .speechRecognitionDenied {
                    // Denial is a user decision, not an engine defect —
                    // falling through would just re-ask. Surface it.
                    throw failure
                } catch {
                    lastError = error
                    candidate.cancel()
                    Log.voice.error("engine unavailable, trying next: \(error.localizedDescription)")
                }
            }
        } catch let failure as DictationFailure {
            phase = .failed(failure)
            return
        } catch {
            phase = .failed(.engine(error.localizedDescription))
            return
        }
        guard let engine = selected else {
            if let failure = lastError as? DictationFailure {
                phase = .failed(failure)
            } else if let lastError {
                phase = .failed(.engine(lastError.localizedDescription))
            } else {
                phase = .failed(.engine(String(localized: "Speech recognition is unavailable.")))
            }
            return
        }
        self.engine = engine
        engineLabel = engine.label

        consumeTask = Task { [weak self] in
            do {
                for try await update in engine.updates {
                    guard let self else { return }
                    finalizedText = update.finalizedText
                    volatileText = update.volatileText
                }
            } catch {
                guard let self, phase == .listening else { return }
                // A DictationFailure travels as itself; anything else (an
                // Apple recognition error) keeps its own message.
                phase = .failed((error as? DictationFailure) ?? .engine(error.localizedDescription))
                stopAudio()
            }
        }

        do {
            try audioSource.start(
                onBuffer: { [weak engine] buffer in engine?.append(buffer) },
                onLevel: { [weak self] level in
                    Task { @MainActor [weak self] in self?.level = level }
                },
                onInterruption: { [weak self] in
                    Task { @MainActor [weak self] in await self?.handleInterruption() }
                })
        } catch {
            engine.cancel()
            self.engine = nil
            phase = .failed(.audioCapture(error.localizedDescription))
            return
        }
        phase = .listening
    }

    /// Stop the mic, let the engine finalize, and return the transcript
    /// (nil when nothing usable was heard). Ends in `.idle`.
    func finish() async -> String? {
        await stopListening()
        let text = transcript
        reset()
        return text.isEmpty ? nil : text
    }

    /// Abandon the session and discard the transcript.
    func cancel() {
        stopAudio()
        engine?.cancel()
        consumeTask?.cancel()
        reset()
    }

    /// Between "go" and "listening": fetching or loading model assets.
    private var isPreparing: Bool {
        if case .downloadingModel = phase { return true }
        return phase == .loadingModel
    }

    /// The system claimed the mic mid-session. Finalize what was heard and
    /// park in `.interrupted` so the user can still insert it.
    private func handleInterruption() async {
        guard phase == .listening || isPreparing else { return }
        await stopListening()
        phase = .interrupted
    }

    /// Mic off + engine finalization (bounded — a wedged recognizer must not
    /// hold the overlay hostage; whatever transcript exists is kept).
    private func stopListening() async {
        guard phase == .listening || isPreparing else { return }
        phase = .finishing
        stopAudio()
        guard let engine else { return }
        let consumeTask = consumeTask
        // The budget is the engine's, not a fixed 5 s: a streaming engine has
        // already delivered nearly everything, while Whisper hasn't started —
        // decoding a minute of speech happens entirely inside finishAudio(),
        // and cutting it off at 5 s would throw away the whole transcript.
        let budget = engine.finalizeTimeout
        let finalized = await withTaskGroup(of: Bool.self) { group in
            group.addTask {
                await engine.finishAudio()
                await consumeTask?.value
                return true
            }
            group.addTask {
                try? await Task.sleep(for: budget)
                return false
            }
            let first = await group.next() ?? false
            group.cancelAll()
            return first
        }
        if !finalized { engine.cancel() }
    }

    private func stopAudio() {
        audioSource.stop()
        level = 0
    }

    private func reset() {
        engine = nil
        consumeTask = nil
        phase = .idle
    }
}

/// Joining rule for transcript pieces: English-like segments need a space
/// between them, CJK must not grow spaces mid-sentence. Pure + tested.
enum DictationTranscript {
    /// ASCII marks that end a clause — a following ASCII word wants a space
    /// ("Hello." + "How" → "Hello. How"). Their CJK counterparts (。，) are
    /// full-width and never take one.
    private static let clauseEnders: Set<Unicode.Scalar> = [".", ",", "!", "?", ";", ":", ")"]

    /// Concatenate two transcript pieces, inserting a single space only at an
    /// ASCII word/clause boundary — "hello"+"world" → "hello world",
    /// "wait,"+"go" → "wait, go" — while "你好"+"世界" → "你好世界" and
    /// pieces already separated ("hello "+"world") pass through untouched.
    static func join(_ head: String, _ tail: String) -> String {
        guard !head.isEmpty else { return tail }
        guard !tail.isEmpty else { return head }
        guard let last = head.unicodeScalars.last, let first = tail.unicodeScalars.first,
              last.isASCII, first.isASCII else {
            return head + tail
        }
        let wordlike = CharacterSet.alphanumerics
        guard wordlike.contains(first) else { return head + tail }
        let needsSpace = wordlike.contains(last) || clauseEnders.contains(last)
        return needsSpace ? head + " " + tail : head + tail
    }
}
