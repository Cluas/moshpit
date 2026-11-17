import AVFoundation
import Foundation
import MoshpitKit

/// Dictation on X-ASR alone (B763): the model streams the words while the
/// person speaks and writes the text that is inserted, the pane's hotwords
/// in its beam search. The local engine on phones without the memory for
/// Qwen3-ASR, or wherever the person picked X-ASR.
///
/// The model is opened when listening starts, without holding up the
/// microphone, and let go when the dictation ends (``StreamingDictationCore``).
final class XASRDictationEngine: DictationEngine {
    let updates: AsyncThrowingStream<DictationUpdate, Error>
    let label: String

    /// The streaming text is nearly all there when the audio ends; the room
    /// is for a model still loading, which then decodes the whole recording
    /// at about a seventh of real time.
    var finalizeTimeout: Duration {
        .seconds(max(15, Int(recordedSeconds)))
    }

    private let load: StreamingDictationCore.Load
    private let script: DictationTranscript.ChineseScript?
    private let isInstalled: @Sendable () -> Bool
    private let updateContinuation: AsyncThrowingStream<DictationUpdate, Error>.Continuation
    private let converter = DictationBufferConverter()
    private static let targetFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)
    /// Ten minutes, as for the other local engines.
    private static let maximumSamples = 16_000 * 600

    private let lock = NSLock()
    private var core: StreamingDictationCore?
    private var heard = 0
    private var cancelled = false

    private var recordedSeconds: Double {
        lock.withLock { Double(heard) / 16_000 }
    }

    /// `load` opens the model for this dictation; the app's is
    /// ``XASRDictationEngine/opener(hotwords:)``.
    init(load: @escaping StreamingDictationCore.Load, label: String,
         script: DictationTranscript.ChineseScript? = nil,
         isInstalled: @escaping @Sendable () -> Bool = { WhisperModelStore.isInstalled(XASR.variant) }) {
        self.load = load
        self.label = label
        self.script = script
        self.isInstalled = isInstalled
        (updates, updateContinuation) = AsyncThrowingStream.makeStream()
    }

    /// Opens X-ASR from its folder with these hotwords.
    static func opener(hotwords: [String]) -> StreamingDictationCore.Load {
        let folder = WhisperModelStore.folder(for: XASR.variant)
        let words = Array(hotwords.prefix(XASR.hotwordLimit))
        return { try await XASRStream.open(folder: folder, hotwords: words) }
    }

    deinit {
        lock.withLock { core }?.cancel()
    }

    /// Nothing to wait for: the model loads once listening starts. Only a
    /// model that isn't on the device stops the dictation here, with the
    /// fix it needs.
    func prepare(onProgress: @escaping @Sendable (DictationPreparation) -> Void) async throws {
        onProgress(.loading)
        Log.voice.info("voice prepare: model=\(XASR.variant, privacy: .public)\(ProcessMemory.logSuffix, privacy: .public)")
        guard isInstalled() else { throw DictationFailure.noWhisperModel }
    }

    func start() async throws {
        guard Self.targetFormat != nil else {
            throw DictationFailure.audioCapture(
                String(localized: "16 kHz mono audio is unavailable on this device."))
        }
        let script = self.script
        let continuation = updateContinuation
        let core = StreamingDictationCore(load: load, finalPass: nil, onLoadFailure: { reason in
            // The panel shows it and stops the mic; Insert has nothing to
            // type (B862).
            Log.voice.error("x-asr load failed: \(reason, privacy: .public)")
            continuation.finish(throwing: DictationFailure.engine(reason))
        }) { text in
            continuation.yield(DictationUpdate(
                finalizedText: "", volatileText: DictationTranscript.converting(text, to: script)))
        }
        let begun = lock.withLock { () -> Bool in
            guard !cancelled else { return false }
            self.core = core
            return true
        }
        guard begun else { throw CancellationError() }
        core.begin()
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        guard let format = Self.targetFormat else { return }
        let converted: AVAudioPCMBuffer
        do {
            converted = try converter.convert(buffer, to: format)
        } catch {
            Log.voice.error("x-asr buffer conversion failed: \(error.localizedDescription)")
            return
        }
        guard let channel = converted.floatChannelData?[0] else { return }
        let frames = Int(converted.frameLength)
        guard frames > 0 else { return }
        let core = lock.withLock { () -> StreamingDictationCore? in
            guard heard < Self.maximumSamples else { return nil }
            heard += frames
            return self.core
        }
        core?.append(Array(UnsafeBufferPointer(start: channel, count: frames)))
    }

    func finishAudio() async {
        guard let core = lock.withLock({ cancelled ? nil : core }) else {
            updateContinuation.finish()
            return
        }
        let started = ContinuousClock.now
        let audio = String(format: "%.2f", recordedSeconds)
        switch await core.finish() {
        case .text(let text):
            let shown = DictationTranscript.converting(text, to: script)
            Log.voice.info(
                "voice final: model=\(XASR.variant, privacy: .public) audio=\(audio, privacy: .public)s wall=\(WhisperModelStore.seconds(ContinuousClock.now - started), privacy: .public)s chars=\(shown.count, privacy: .public)\(ProcessMemory.logSuffix, privacy: .public)")
            updateContinuation.yield(DictationUpdate(finalizedText: shown, volatileText: ""))
            updateContinuation.finish()
        case .cancelled:
            updateContinuation.finish()
        case .failed(let reason):
            updateContinuation.finish(throwing: DictationFailure.engine(reason))
        }
    }

    func cancel() {
        let core = lock.withLock { () -> StreamingDictationCore? in
            cancelled = true
            return self.core
        }
        core?.cancel()
        updateContinuation.finish()
    }
}
