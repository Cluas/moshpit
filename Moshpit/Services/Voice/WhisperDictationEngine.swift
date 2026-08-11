import AVFoundation
import Foundation

/// The slice of `WhisperModelStore` the engine actually needs.
///
/// Exists so the engine can be tested against a scripted transcriber: the real
/// store's answers come from a several-hundred-megabyte CoreML model, which is
/// neither available nor deterministic in CI.
protocol WhisperTranscribing: Sendable {
    /// Load the variant if it isn't already; cheap once warm.
    func warmUp(variant: String) async throws
    /// Transcribe 16 kHz mono samples. `language` nil = let Whisper detect.
    func transcribe(samples: [Float], language: String?, variant: String) async throws -> String
}

extension WhisperModelStore: WhisperTranscribing {}

/// Dictation backed by a local Whisper model.
///
/// Structurally unlike the Apple engines, and the difference is inherent to
/// Whisper rather than a shortcut: Whisper is not a streaming recognizer. It
/// takes a span of audio and returns a transcript of the whole span, free to
/// revise any word in it, because that is how a sequence-to-sequence model
/// hears a sentence whose language changes halfway through. So this engine
/// buffers the microphone and *re-transcribes*:
///
/// - While listening it runs a pass over everything heard so far, at most one
///   at a time, and publishes the result as the volatile hypothesis. Live text
///   for free, at whatever cadence the model can sustain.
/// - On `finishAudio()` it runs one last pass over the complete recording and
///   publishes that as finalized. This is the transcript that matters — full
///   context, so the tail of a sentence can still fix the head of it.
///
/// Audio never leaves the device; the model was downloaded once by
/// `WhisperModelStore` and everything here is CoreML on the Neural Engine.
final class WhisperDictationEngine: DictationEngine {
    let updates: AsyncThrowingStream<DictationUpdate, Error>
    let label: String

    /// Whisper does all its decoding inside `finishAudio()`, so the budget has
    /// to cover a full pass over the recording rather than a stream's tail.
    /// Real-time factor on the Neural Engine is well under 1× even for the
    /// large models, so twice the audio duration is generous; the floor keeps
    /// short clips from being cut off by model load or a cold Neural Engine.
    var finalizeTimeout: Duration {
        .seconds(max(20, Int(recordedSeconds * 2)))
    }

    private let store: any WhisperTranscribing
    private let variant: String
    /// Whisper language code, or nil to let the model detect it.
    private let language: String?
    private let updateContinuation: AsyncThrowingStream<DictationUpdate, Error>.Continuation
    private let converter = DictationBufferConverter()

    /// 16 kHz mono float — the only format Whisper's mel front-end accepts.
    ///
    /// Left optional rather than force-unwrapped. These parameters are valid
    /// on every Apple platform, so this is not expected to fail — but a trap
    /// in the middle of a dictation session is a much worse way to find out
    /// otherwise than the message `start()` raises, and it matches how the
    /// converter below already treats an unavailable format.
    private static let targetFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)

    /// Hard stop on a session's length. 16 kHz float is ~64 KB/s, so ten
    /// minutes is ~38 MB resident — beyond that a forgotten open mic is a
    /// memory problem, and no one dictates a ten-minute shell command.
    private static let maximumSamples = 16_000 * 600

    /// Don't preview until there's enough audio to say something meaningful;
    /// below this Whisper reliably invents a stock phrase from silence.
    private static let previewFloorSamples = 16_000 * 2

    /// Guards `samples` — `append` runs on the audio tap's real-time thread
    /// while preview passes read from a Task.
    private let lock = NSLock()
    private var samples: [Float] = []

    private var previewTask: Task<Void, Never>?
    private var cancelled = false

    private var recordedSeconds: Double {
        lock.lock()
        defer { lock.unlock() }
        return Double(samples.count) / 16_000
    }

    init(store: any WhisperTranscribing, variant: String, language: String?, label: String) {
        self.store = store
        self.variant = variant
        self.language = language
        self.label = label
        (updates, updateContinuation) = AsyncThrowingStream.makeStream()
    }

    // MARK: Lifecycle

    /// Loads the model. Never downloads: fetching several hundred megabytes is
    /// an explicit, sized, cancellable choice the user makes in Settings, not
    /// something that happens because they tapped a mic key — possibly on
    /// cellular, mid-session, with a terminal open.
    func prepare(onProgress: @escaping @Sendable (DictationPreparation) -> Void) async throws {
        onProgress(.loading)
        do {
            try await store.warmUp(variant: variant)
        } catch let failure as WhisperModelFailure {
            // A missing model is a fixable setup problem with a specific
            // destination, not a generic engine fault — say so, so the overlay
            // can point at the screen that fixes it.
            if case .notInstalled = failure { throw DictationFailure.noWhisperModel }
            throw DictationFailure.engine(failure.localizedDescription)
        } catch {
            throw DictationFailure.engine(error.localizedDescription)
        }
    }

    func start() async throws {
        guard Self.targetFormat != nil else {
            throw DictationFailure.audioCapture(
                String(localized: "16 kHz mono audio is unavailable on this device."))
        }
        lock.lock()
        samples.removeAll(keepingCapacity: true)
        lock.unlock()
        previewTask = Task { [weak self] in await self?.runPreviewLoop() }
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        // `start()` already refused the session if this were nil.
        guard let format = Self.targetFormat else { return }
        let converted: AVAudioPCMBuffer
        do {
            converted = try converter.convert(buffer, to: format)
        } catch {
            // One malformed buffer is a dropped frame, not a dead session.
            Log.voice.error("whisper buffer conversion failed: \(error.localizedDescription)")
            return
        }
        guard let channel = converted.floatChannelData?[0] else { return }
        let frames = Int(converted.frameLength)
        guard frames > 0 else { return }
        lock.lock()
        if samples.count < Self.maximumSamples {
            samples.append(contentsOf: UnsafeBufferPointer(start: channel, count: frames))
        }
        lock.unlock()
    }

    func finishAudio() async {
        previewTask?.cancel()
        previewTask = nil
        guard !cancelled else {
            updateContinuation.finish()
            return
        }
        lock.lock()
        let final = samples
        lock.unlock()
        // Too short to be speech — a mis-tap, or the mic opened and closed.
        // Return nothing rather than whatever Whisper invents from silence.
        guard final.count >= WhisperModelStore.minimumSamples else {
            updateContinuation.yield(DictationUpdate(finalizedText: "", volatileText: ""))
            updateContinuation.finish()
            return
        }
        do {
            let text = try await store.transcribe(
                samples: final, language: language, variant: variant)
            updateContinuation.yield(DictationUpdate(finalizedText: text, volatileText: ""))
            updateContinuation.finish()
        } catch {
            if cancelled || error is CancellationError {
                updateContinuation.finish()
            } else {
                updateContinuation.finish(throwing: DictationFailure.engine(error.localizedDescription))
            }
        }
    }

    func cancel() {
        cancelled = true
        previewTask?.cancel()
        previewTask = nil
        updateContinuation.finish()
    }

    // MARK: Live preview

    /// Re-transcribe what's been heard so far, over and over, publishing each
    /// result as the volatile hypothesis.
    ///
    /// Self-throttling by construction: passes are strictly sequential and the
    /// gap after one is at least as long as that pass took. On a fast model
    /// the preview updates about once a second; on `large-v3-turbo` with a
    /// long utterance it slows to every few seconds, which is the correct
    /// degradation — the preview is a courtesy, and the pass that counts is
    /// the one in `finishAudio()`.
    private func runPreviewLoop() async {
        let minimumGap = Duration.milliseconds(900)
        while !Task.isCancelled {
            try? await Task.sleep(for: minimumGap)
            if Task.isCancelled { return }
            lock.lock()
            let snapshot = samples
            lock.unlock()
            guard snapshot.count >= Self.previewFloorSamples else { continue }
            let started = ContinuousClock.now
            do {
                let text = try await store.transcribe(
                    samples: snapshot, language: language, variant: variant)
                if Task.isCancelled { return }
                if !text.isEmpty {
                    updateContinuation.yield(
                        DictationUpdate(finalizedText: "", volatileText: text))
                }
            } catch {
                // A failed preview says nothing about the final pass — the
                // model may simply have been mid-load. Keep listening.
                Log.voice.error("whisper preview pass failed: \(error.localizedDescription)")
            }
            // Back off by however long that took, so a slow model yields the
            // CPU instead of queueing passes back to back.
            let elapsed = ContinuousClock.now - started
            if elapsed > minimumGap, !Task.isCancelled {
                try? await Task.sleep(for: elapsed)
            }
        }
    }
}
