import Foundation
import MoshpitKit
import SherpaOnnxC

// MARK: - Model

/// A loaded X-ASR recognizer and the one stream a dictation runs on it.
///
/// Every sherpa-onnx call happens on one serial queue, in the order the
/// dictation made them: audio is never decoded out of order, and the queue
/// is not the cooperative pool, which decoding would otherwise hold up.
final class XASRStream: StreamingRecognizing, @unchecked Sendable {
    private let queue = DispatchQueue(label: "voice.x-asr", qos: .userInitiated)
    private var recognizer: OpaquePointer?
    private var stream: OpaquePointer?

    private init(recognizer: OpaquePointer, stream: OpaquePointer) {
        self.recognizer = recognizer
        self.stream = stream
    }

    deinit {
        if let stream { SherpaOnnxDestroyOnlineStream(stream) }
        if let recognizer { SherpaOnnxDestroyOnlineRecognizer(recognizer) }
    }

    /// Load the model from its folder and open a stream. With `hotwords`,
    /// decoding is a beam search biased toward them (when the vocabulary
    /// could be made); without, it is greedy, which is all a preview needs.
    static func open(folder: URL, hotwords: [String]) async throws -> XASRStream {
        guard XASR.isComplete(folder) else {
            throw WhisperModelFailure.notInstalled(WhisperModelStore.displayName(for: XASR.variant))
        }
        let words = Array(hotwords.prefix(XASR.hotwordLimit))
        let vocabulary = words.isEmpty ? nil : SentencePieceVocabulary.ensure(in: folder)
        let biased = vocabulary != nil
        let path = { (name: String) in folder.appendingPathComponent(name).path }
        let provider = Qwen3ASRModel.provider(
            physicalMemory: ProcessInfo.processInfo.physicalMemory,
            writingOptionsTo: FileManager.default.temporaryDirectory)
        let hotwordText = biased ? XASR.hotwordLines(words) : ""
        let strings = [path("encoder.int8.onnx"), path("decoder.onnx"), path("joiner.int8.onnx"),
                       path("tokens.txt"), provider, biased ? "modified_beam_search" : "greedy_search",
                       biased ? "cjkchar+bpe" : "", vocabulary?.path ?? "", hotwordText]
        let opened: (OpaquePointer, OpaquePointer)? = await Task.detached(priority: .userInitiated) {
            withCStrings(strings) { c -> (OpaquePointer, OpaquePointer)? in
                var config = SherpaOnnxOnlineRecognizerConfig()
                config.feat_config.sample_rate = 16_000
                config.feat_config.feature_dim = 80
                config.model_config.transducer.encoder = c[0]
                config.model_config.transducer.decoder = c[1]
                config.model_config.transducer.joiner = c[2]
                config.model_config.tokens = c[3]
                config.model_config.num_threads = XASR.threads
                config.model_config.provider = c[4]
                config.decoding_method = c[5]
                config.max_active_paths = 4
                config.model_config.modeling_unit = c[6]
                config.model_config.bpe_vocab = c[7]
                config.hotwords_score = 1.5
                if !hotwordText.isEmpty {
                    config.hotwords_buf = c[8]
                    config.hotwords_buf_size = Int32(hotwordText.utf8.count)
                }
                config.enable_endpoint = 0
                guard let recognizer = SherpaOnnxCreateOnlineRecognizer(&config) else { return nil }
                guard let stream = SherpaOnnxCreateOnlineStream(recognizer) else {
                    SherpaOnnxDestroyOnlineRecognizer(recognizer)
                    return nil
                }
                return (recognizer, stream)
            }
        }.value
        guard let (recognizer, stream) = opened else {
            throw WhisperModelFailure.incomplete(WhisperModelStore.displayName(for: XASR.variant))
        }
        Log.voice.info(
            "voice stream: model=\(XASR.variant, privacy: .public) hotwords=\(biased ? words.count : 0, privacy: .public)\(ProcessMemory.logSuffix, privacy: .public)")
        return XASRStream(recognizer: recognizer, stream: stream)
    }

    func feed(_ samples: [Float]) {
        queue.async { [self] in
            guard let recognizer, let stream, !samples.isEmpty else { return }
            samples.withUnsafeBufferPointer {
                SherpaOnnxOnlineStreamAcceptWaveform(stream, 16_000, $0.baseAddress, Int32($0.count))
            }
            while SherpaOnnxIsOnlineStreamReady(recognizer, stream) == 1 {
                SherpaOnnxDecodeOnlineStream(recognizer, stream)
            }
        }
    }

    func partialText() async -> String {
        await withCheckedContinuation { continuation in
            queue.async { [self] in continuation.resume(returning: currentText()) }
        }
    }

    func finish() async -> String {
        await withCheckedContinuation { continuation in
            queue.async { [self] in
                guard let recognizer, let stream else { return continuation.resume(returning: "") }
                let tail = [Float](repeating: 0, count: XASR.tailSamples)
                tail.withUnsafeBufferPointer {
                    SherpaOnnxOnlineStreamAcceptWaveform(stream, 16_000, $0.baseAddress, Int32($0.count))
                }
                SherpaOnnxOnlineStreamInputFinished(stream)
                while SherpaOnnxIsOnlineStreamReady(recognizer, stream) == 1 {
                    SherpaOnnxDecodeOnlineStream(recognizer, stream)
                }
                continuation.resume(returning: currentText())
            }
        }
    }

    func close() {
        queue.async { [self] in
            if let stream { SherpaOnnxDestroyOnlineStream(stream) }
            if let recognizer { SherpaOnnxDestroyOnlineRecognizer(recognizer) }
            stream = nil
            recognizer = nil
            Log.voice.info("voice stream closed: model=\(XASR.variant, privacy: .public)\(ProcessMemory.logSuffix, privacy: .public)")
        }
    }

    /// On the queue only.
    private func currentText() -> String {
        guard let recognizer, let stream,
              let result = SherpaOnnxGetOnlineStreamResult(recognizer, stream) else { return "" }
        defer { SherpaOnnxDestroyOnlineRecognizerResult(result) }
        guard let text = result.pointee.text else { return "" }
        return XASR.tidy(String(cString: text))
    }

}

/// Call `body` with C copies of `strings`, alive for the call.
private func withCStrings<R>(_ strings: [String], _ body: ([UnsafePointer<CChar>]) -> R) -> R {
    let copies = strings.map { strdup($0)! }
    defer { copies.forEach { free($0) } }
    return body(copies.map { UnsafePointer($0) })
}
