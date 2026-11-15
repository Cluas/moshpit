import CryptoKit
import Foundation
import SherpaOnnxC

// MARK: - Catalog entry

/// Qwen3-ASR 0.6B, the local model for Chinese speech that carries English
/// terms (docs/nsh/ASR-2026-11.md has the measurements that picked it).
///
/// Why this one: it hears a sentence that switches language mid-way as one
/// sentence, it keeps English terms in English rather than translating them,
/// and it takes hotwords — the file names, branch and agent on the pane's
/// screen (`DictationHotwords`) go into its prompt, which is what moved the
/// share of terms heard right from 69% to 77% in the eval.
///
/// The weights are the 4-bit build of the sherpa-onnx export (block-32
/// `MatMulNBits`, external `.weights` files that onnxruntime maps from disk
/// instead of copying): 0.6 GB on disk, about half a gigabyte resident once
/// loaded. They run on sherpa-onnx's onnxruntime, on the CPU.
enum Qwen3ASR {
    /// The catalog id, and the folder name under the store's base.
    static let variant = "qwen3-asr-0.6b-int4"

    /// One file of the model, pinned: the download is checked against
    /// `sha256` before it is moved into place, so a file at its final path is
    /// a verified one.
    struct RemoteFile: Equatable, Sendable {
        let path: String
        let bytes: Int64
        let sha256: String
    }

    /// The Hugging Face repo and the commit the files are pinned to. Apache-2.0
    /// (the weights are Qwen's, quantized; the export is sherpa-onnx's).
    static let repo = "goodpixelltd/qwen3-asr-0.6b-int4-sherpa-onnx"
    static let revision = "9012f07ac1ce43c7e9fbd3afd8cad90fe147f88e"

    static let files: [RemoteFile] = [
        RemoteFile(path: "conv_frontend.onnx", bytes: 44_148_281,
                   sha256: "d22dc4423e0940e49884e903d2ea2f7e5567c14fc1aed97e4e26d6b8f208ef9e"),
        RemoteFile(path: "encoder.int4.onnx", bytes: 5_622_024,
                   sha256: "774e997d23a2f06c3f544666625c93a3b824fdd83d04c6acd88ee25f4e3b6e2e"),
        RemoteFile(path: "encoder.int4.weights", bytes: 110_308_864,
                   sha256: "dc66975d9be10fbd31f89c76d33dd0bf4a8059ef14da4abf545bbf9895a5293d"),
        RemoteFile(path: "decoder.int4.onnx", bytes: 891_837,
                   sha256: "c588d2a89460315f51ca8c65dbe85a0619891500fc853e5b673e3c914c25b2d3"),
        RemoteFile(path: "decoder.int4.weights", bytes: 469_962_752,
                   sha256: "126c8e16074cc87250dacbf07602315e3bc5239c93ea414773b6e3a6fd4bf466"),
        RemoteFile(path: "tokenizer/merges.txt", bytes: 1_671_853,
                   sha256: "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5"),
        RemoteFile(path: "tokenizer/tokenizer_config.json", bytes: 12_487,
                   sha256: "4942d005604266809309cabc9f4e9cb89ce855d59b14681fdc0e1cc62ea26c4c"),
        RemoteFile(path: "tokenizer/vocab.json", bytes: 2_776_833,
                   sha256: "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910"),
    ]

    static var totalBytes: Int64 { files.reduce(0) { $0 + $1.bytes } }

    static func url(for file: RemoteFile) -> URL {
        URL(string: "https://huggingface.co/\(repo)/resolve/\(revision)/\(file.path)")!
    }

    /// Every file in place at its size. Sizes, not hashes: a file only gets
    /// to its final path once its hash matched (``ModelFileDownloader``), so
    /// the size is a cheap check that nothing has been truncated since.
    static func isComplete(_ folder: URL) -> Bool {
        files.allSatisfy { file in
            let url = folder.appendingPathComponent(file.path)
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize
            return size.map { Int64($0) == file.bytes } ?? false
        }
    }

    /// Offered on phones with 6 GB of memory or more. A short dictation peaks
    /// well under a gigabyte, but a minute-long one reaches 1.6 GB (the
    /// build's own measurement on an iPhone 14 Pro), which a 4 GB phone
    /// cannot spare beside a terminal.
    static var isSupportedDevice: Bool {
        ProcessInfo.processInfo.physicalMemory >= 5_500_000_000
    }

    /// Hotwords go into the prompt the decoder reads before the audio, so
    /// each one costs prefill time: the pane's best forty.
    static let hotwordLimit = 40
}

// MARK: - Model

/// A loaded Qwen3-ASR recognizer. Not Sendable; the store keeps it and runs
/// one transcription at a time on it.
final class Qwen3ASRModel: WhisperLoadedModel {
    private let recognizer: OpaquePointer

    private init(recognizer: OpaquePointer) {
        self.recognizer = recognizer
    }

    deinit {
        SherpaOnnxDestroyOfflineRecognizer(recognizer)
    }

    /// Longest stretch decoded in one go. The decoder's sequence holds the
    /// prompt, the audio's tokens and the text; past ~35 s of audio it no
    /// longer fits `maxTotalLen`, so a long dictation is cut at its quietest
    /// moment into pieces of at most this.
    static let maximumPieceSamples = 16_000 * 20
    static let maxTotalLen: Int32 = 768

    /// onnxruntime session options for the three graphs, as sherpa-onnx reads
    /// them from a `cpu:<file>` provider string (B679): no CPU memory arena,
    /// no memory pattern.
    ///
    /// With its arena onnxruntime keeps the largest working set any decode
    /// ever needed — the decoder's KV caches for 768 positions, the
    /// encoder's activations — for as long as the model is loaded. Measured
    /// with the app's configuration on a 10 s utterance: 1.33 GB resident
    /// between decodes and 1.50 GB at a decode's peak with it, against
    /// 0.53 GB and 0.98 GB without; a dictation's preview passes decode
    /// every second or so, so the model sat at its peak for the whole
    /// dictation, beside the live terminals. Decoding is a little slower
    /// without (allocations per run), within the noise of the measurement.
    static let sessionOptions = "EnableCpuMemArena=0\nEnableMemPattern=0\n"

    /// The provider string carrying ``sessionOptions``: the options written
    /// to a file under `directory`, or plain "cpu" when that fails —
    /// sherpa-onnx exits the process if a named config file can't be
    /// opened, so the file is written and checked right before use.
    static func provider(writingOptionsTo directory: URL) -> String {
        let file = directory.appendingPathComponent("qwen3-asr-session.cfg")
        do {
            try Data(sessionOptions.utf8).write(to: file, options: .atomic)
        } catch {
            return "cpu"
        }
        guard FileManager.default.isReadableFile(atPath: file.path) else { return "cpu" }
        return "cpu:" + file.path
    }

    /// Builds the recognizer from the downloaded folder. Seconds: onnxruntime
    /// maps the weights and plans the graphs. Throws, rather than trapping,
    /// when sherpa-onnx refuses the files.
    static func load(folder: URL) async throws -> Qwen3ASRModel {
        guard Qwen3ASR.isComplete(folder) else {
            throw WhisperModelFailure.notInstalled(WhisperModelStore.displayName(for: Qwen3ASR.variant))
        }
        let frontendPath = folder.appendingPathComponent("conv_frontend.onnx").path
        let encoderPath = folder.appendingPathComponent("encoder.int4.onnx").path
        let decoderPath = folder.appendingPathComponent("decoder.int4.onnx").path
        let tokenizerPath = folder.appendingPathComponent("tokenizer").path
        let threads = Int32(max(1, min(4, ProcessInfo.processInfo.activeProcessorCount - 2)))
        let providerString = provider(writingOptionsTo: FileManager.default.temporaryDirectory)
        let maxLength = maxTotalLen
        let recognizer: OpaquePointer? = await Task.detached(priority: .userInitiated) {
            // The strings only have to live through the call: sherpa-onnx
            // copies the configuration when it builds the recognizer.
            frontendPath.withCString { frontend in
                encoderPath.withCString { encoder in
                    decoderPath.withCString { decoder in
                        tokenizerPath.withCString { tokenizer in
                            providerString.withCString { provider in
                                "greedy_search".withCString { method -> OpaquePointer? in
                                    var config = SherpaOnnxOfflineRecognizerConfig()
                                    config.feat_config.sample_rate = 16_000
                                    config.feat_config.feature_dim = 128
                                    config.model_config.qwen3_asr.conv_frontend = frontend
                                    config.model_config.qwen3_asr.encoder = encoder
                                    config.model_config.qwen3_asr.decoder = decoder
                                    config.model_config.qwen3_asr.tokenizer = tokenizer
                                    config.model_config.qwen3_asr.max_total_len = maxLength
                                    config.model_config.qwen3_asr.max_new_tokens = 256
                                    config.model_config.num_threads = threads
                                    config.model_config.provider = provider
                                    config.decoding_method = method
                                    return SherpaOnnxCreateOfflineRecognizer(&config)
                                }
                            }
                        }
                    }
                }
            }
        }.value
        guard let recognizer else {
            throw WhisperModelFailure.incomplete(WhisperModelStore.displayName(for: Qwen3ASR.variant))
        }
        return Qwen3ASRModel(recognizer: recognizer)
    }

    /// `language` is Whisper's setting and is not used: Qwen3-ASR decides per
    /// sentence, which is the point of it for speech that mixes two.
    func transcribe(samples: [Float], language _: String?, hotwords: [String]) async throws -> String {
        let words = Array(hotwords.prefix(Qwen3ASR.hotwordLimit))
        let csv = DictationHotwords.csv(words)
        var text = ""
        for piece in Self.pieces(of: samples) {
            try Task.checkCancellation()
            var part = decode(samples[piece], hotwords: csv)
            if !words.isEmpty, Self.isPromptEcho(part, hotwords: words) {
                // The hotwords came back instead of the speech: decode again
                // without them.
                try Task.checkCancellation()
                part = decode(samples[piece], hotwords: "")
            }
            text = DictationTranscript.join(text, part)
        }
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The decoder read its hotwords back instead of the audio. Measured on a
    /// one-second "git status": with the pane's words in the prompt the 4-bit
    /// model answered with the word list itself. Recognized as three or more
    /// hotwords making up four fifths of the text or more — a real sentence
    /// that names three files still has words of its own around them. Pure.
    static func isPromptEcho(_ text: String, hotwords: [String]) -> Bool {
        let lowered = text.lowercased()
        let hits = hotwords.filter { lowered.contains($0.lowercased()) }
        guard hits.count >= 3 else { return false }
        let length = { (text: String) in
            text.unicodeScalars.filter { !CharacterSet.whitespacesAndNewlines.contains($0) }.count
        }
        let covered = hits.reduce(0) { $0 + length($1) }
        return Double(covered) >= 0.8 * Double(length(text))
    }

    private func decode(_ samples: ArraySlice<Float>, hotwords: String) -> String {
        guard let stream = SherpaOnnxCreateOfflineStream(recognizer) else { return "" }
        defer { SherpaOnnxDestroyOfflineStream(stream) }
        if !hotwords.isEmpty {
            SherpaOnnxOfflineStreamSetOption(stream, "hotwords", hotwords)
        }
        samples.withUnsafeBufferPointer { buffer in
            SherpaOnnxAcceptWaveformOffline(stream, 16_000, buffer.baseAddress, Int32(buffer.count))
        }
        SherpaOnnxDecodeOfflineStream(recognizer, stream)
        guard let result = SherpaOnnxGetOfflineStreamResult(stream) else { return "" }
        defer { SherpaOnnxDestroyOfflineRecognizerResult(result) }
        return result.pointee.text.map { String(cString: $0) } ?? ""
    }

    /// Cuts `samples` into pieces of at most ``maximumPieceSamples``, each
    /// cut at the quietest 100 ms of the last four seconds before the limit,
    /// so no word is split if there was any pause to split at. Pure.
    static func pieces(of samples: [Float], limit: Int = maximumPieceSamples) -> [Range<Int>] {
        var pieces: [Range<Int>] = []
        var start = 0
        let window = 1_600  // 100 ms
        let search = 16_000 * 4
        while samples.count - start > limit {
            let end = start + limit
            var cut = end
            var quietest = Float.greatestFiniteMagnitude
            var probe = max(start + window, end - search)
            while probe + window <= end {
                var energy: Float = 0
                for i in probe ..< probe + window { energy += samples[i] * samples[i] }
                if energy < quietest {
                    quietest = energy
                    cut = probe + window / 2
                }
                probe += window
            }
            pieces.append(start ..< cut)
            start = cut
        }
        if start < samples.count { pieces.append(start ..< samples.count) }
        return pieces
    }
}

// MARK: - Download

/// Fetches a pinned list of files into a folder, resumably: each file is
/// written to `<path>.part`, carried on from where it stopped with a `Range`
/// request, hashed, and moved into place only when its SHA-256 matches. A
/// file already at its final path is skipped.
///
/// Its own small thing rather than WhisperKit's Hub downloader, which only
/// knows a repo laid out as one folder per Whisper variant. Same rules as the
/// store's Whisper downloads otherwise: an ordinary session, not a background
/// one (`WhisperModelStore.downloadOnce` says why), and what is on disk is
/// what Resume continues from.
enum ModelFileDownloader {
    enum Failure: Error, LocalizedError, Equatable {
        case status(Int)
        case checksum(String)

        var errorDescription: String? {
            switch self {
            case .status(let code):
                return String(localized: "The server answered \(code).")
            case .checksum(let file):
                return String(localized: "\(file) arrived damaged. Try again.")
            }
        }
    }

    /// Download every file not yet in place. `onProgress` gets 0…1 over the
    /// whole list's bytes, counting what earlier runs left on disk.
    static func fetch(_ files: [Qwen3ASR.RemoteFile], into folder: URL, url: (Qwen3ASR.RemoteFile) -> URL,
                      onProgress: @escaping @Sendable (Double) -> Void) async throws {
        let total = Double(max(1, files.reduce(0) { $0 + $1.bytes }))
        var done: Int64 = 0
        for file in files {
            let destination = folder.appendingPathComponent(file.path)
            if fileSize(destination) == file.bytes {
                done += file.bytes
                continue
            }
            try FileManager.default.createDirectory(
                at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
            let part = destination.appendingPathExtension("part")
            let base = done
            try Task.checkCancellation()
            do {
                try await download(url(file), to: part, expected: file.bytes) { written in
                    onProgress(Double(base + written) / total)
                }
            } catch {
                // A paused download ends as URLSession's "cancelled": say so
                // as a cancellation, not a failure to show.
                if Task.isCancelled { throw CancellationError() }
                throw error
            }
            guard try sha256(of: part) == file.sha256 else {
                try? FileManager.default.removeItem(at: part)
                throw Failure.checksum(file.path)
            }
            try? FileManager.default.removeItem(at: destination)
            try FileManager.default.moveItem(at: part, to: destination)
            done += file.bytes
            onProgress(Double(done) / total)
        }
    }

    static func fileSize(_ url: URL) -> Int64? {
        (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize.map(Int64.init)
    }

    /// Hash a file in 4 MB reads, so a 470 MB weights file never sits in memory.
    static func sha256(of url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while let chunk = try handle.read(upToCount: 4 << 20), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    /// One file into `part`, appending from its current size. `onWritten` gets
    /// the bytes in `part` so far, every megabyte or so.
    private static func download(_ url: URL, to part: URL, expected: Int64,
                                 onWritten: @escaping @Sendable (Int64) -> Void) async throws {
        var offset = fileSize(part) ?? 0
        if offset > expected {
            try? FileManager.default.removeItem(at: part)
            offset = 0
        }
        if offset == expected { return }
        if offset == 0 {
            FileManager.default.createFile(atPath: part.path, contents: nil)
        }
        var request = URLRequest(url: url)
        request.setValue("bytes=\(offset)-", forHTTPHeaderField: "Range")
        let writer = try ChunkWriter(part: part, offset: offset, onWritten: onWritten)
        let queue = OperationQueue()
        queue.maxConcurrentOperationCount = 1
        let session = URLSession(configuration: .default, delegate: writer, delegateQueue: queue)
        defer { session.finishTasksAndInvalidate() }
        let task = session.dataTask(with: request)
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                writer.finished = continuation
                task.resume()
            }
        } onCancel: {
            task.cancel()
        }
    }

    /// Appends a response body to a file as it arrives, chunk by chunk. All
    /// its callbacks come on one serial delegate queue.
    private final class ChunkWriter: NSObject, URLSessionDataDelegate, @unchecked Sendable {
        private let handle: FileHandle
        private var written: Int64
        private var reported: Int64
        private let onWritten: @Sendable (Int64) -> Void
        private var failure: Error?
        var finished: CheckedContinuation<Void, Error>?

        init(part: URL, offset: Int64, onWritten: @escaping @Sendable (Int64) -> Void) throws {
            handle = try FileHandle(forWritingTo: part)
            written = offset
            reported = offset
            self.onWritten = onWritten
        }

        deinit {
            try? handle.close()
        }

        func urlSession(_: URLSession, dataTask _: URLSessionDataTask, didReceive response: URLResponse,
                        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            do {
                switch status {
                case 206:
                    try handle.seekToEnd()
                case 200:
                    // The server ignored the range: start the file over.
                    try handle.truncate(atOffset: 0)
                    written = 0
                default:
                    throw Failure.status(status)
                }
                completionHandler(.allow)
            } catch {
                failure = error
                completionHandler(.cancel)
            }
        }

        func urlSession(_: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
            do {
                try handle.write(contentsOf: data)
                written += Int64(data.count)
                if written - reported >= 1 << 20 {
                    reported = written
                    onWritten(written)
                }
            } catch {
                failure = error
                dataTask.cancel()
            }
        }

        func urlSession(_: URLSession, task _: URLSessionTask, didCompleteWithError error: Error?) {
            try? handle.synchronize()
            onWritten(written)
            if let failure = failure ?? error {
                finished?.resume(throwing: failure)
            } else {
                finished?.resume()
            }
            finished = nil
        }
    }
}
