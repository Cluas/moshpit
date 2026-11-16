import Foundation

// Foundation only, on purpose: the hand-off logic here is what the tests pin
// down, and it must build without AVFoundation or the speech runtime.

/// A recognizer that streams: audio in as it arrives, the words so far out at
/// any time, and the whole text once the audio has ended (X-ASR, ``XASR``).
protocol StreamingRecognizing: AnyObject, Sendable {
    /// More audio, 16 kHz mono. Returns quickly; decoding happens behind it.
    func feed(_ samples: [Float])
    /// The words heard so far, after everything fed before this call.
    func partialText() async -> String
    /// No more audio: decode the tail and return the whole text.
    func finish() async -> String
    /// Let go of the model. Nothing comes back from it afterwards.
    func close()
}

/// One dictation on a streaming recognizer, from the first sample to the
/// text that is inserted (B763).
///
/// The recognizer is loaded in the background when the dictation begins, and
/// the microphone does not wait for it: audio heard meanwhile is kept and fed
/// in one go once the model is there, which a streaming model catches up on
/// at a fraction of real time. While it listens, every new hypothesis goes
/// out as volatile text.
///
/// When the audio ends there are two ways to the inserted text:
/// - **The streaming model's own** (X-ASR alone): its final text, the pane's
///   hotwords already in its decoding.
/// - **A final pass** (Qwen3-ASR where memory allows): the streaming model's
///   final text is shown first, the streaming model is let go so the two are
///   never at their peaks together, and the final pass over the whole
///   recording replaces it. If the final pass fails, the streaming text is
///   what the person said, so it is the one inserted.
///
/// Cancelling at any point lets the model go and publishes nothing more.
final class StreamingDictationCore: @unchecked Sendable {
    /// What a finished dictation came to.
    enum Outcome: Equatable, Sendable {
        /// Insert this.
        case text(String)
        /// Nothing to insert: cancelled, or the recognizer never came up.
        case cancelled
        /// The streaming model could not be loaded and there is no final
        /// pass to fall back on.
        case failed(String)
    }

    typealias Load = @Sendable () async throws -> any StreamingRecognizing
    typealias FinalPass = @Sendable ([Float]) async throws -> String

    /// Below this a recording is a mis-tap, not speech (0.6 s, as the
    /// batch engines treat it).
    static let minimumSamples = 9_600

    private let load: Load
    private let finalPass: FinalPass?
    private let onVolatile: @Sendable (String) -> Void
    /// How often the pump hands audio to the recognizer and asks it for
    /// words.
    private let tick: Duration

    private let lock = NSLock()
    private var samples: [Float] = []
    private var fed = 0
    private var recognizer: (any StreamingRecognizing)?
    private var loadFailure: String?
    private var shown = ""
    private var cancelled = false
    private var finished = false
    /// The recognizer was let go: a load that lands afterwards closes itself.
    private var closed = false
    private var loadTask: Task<Void, Never>?
    private var pumpTask: Task<Void, Never>?

    init(load: @escaping Load, finalPass: FinalPass?, tick: Duration = .milliseconds(150),
         onVolatile: @escaping @Sendable (String) -> Void) {
        self.load = load
        self.finalPass = finalPass
        self.tick = tick
        self.onVolatile = onVolatile
    }

    deinit {
        loadTask?.cancel()
        pumpTask?.cancel()
        recognizer?.close()
    }

    /// Start loading the model and listening. Returns at once.
    func begin() {
        let load = self.load
        loadTask = Task { [weak self] in
            do {
                let loaded = try await load()
                guard let self else { loaded.close(); return }
                let keep = self.lock.withLock { () -> Bool in
                    guard !self.cancelled, !self.closed else { return false }
                    self.recognizer = loaded
                    return true
                }
                if !keep { loaded.close() }
            } catch {
                self?.lock.withLock { self?.loadFailure = String(describing: error) }
            }
        }
        pumpTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.pump()
                try? await Task.sleep(for: self.tick)
            }
        }
    }

    /// Audio from the microphone, already 16 kHz mono. Called on the audio
    /// thread: it only appends under a lock.
    func append(_ chunk: [Float]) {
        lock.withLock {
            guard !cancelled, !finished else { return }
            samples.append(contentsOf: chunk)
        }
    }

    /// Everything heard so far.
    var recording: [Float] {
        lock.withLock { samples }
    }

    /// Hand the recognizer what it hasn't had, and show what it heard if
    /// that changed.
    private func pump() async {
        guard let recognizer = feedPending(unlessFinished: true) else { return }
        let text = await recognizer.partialText()
        let show = lock.withLock { () -> Bool in
            guard !cancelled, !finished, text != shown, !text.isEmpty else { return false }
            shown = text
            return true
        }
        if show { onVolatile(text) }
    }

    /// Feed the recognizer the audio it hasn't had. Under the lock, so the
    /// pump and the finish can never hand it two stretches out of order
    /// (`feed` only queues them).
    private func feedPending(unlessFinished: Bool) -> (any StreamingRecognizing)? {
        lock.withLock {
            guard !cancelled, !(unlessFinished && finished), let recognizer else { return nil }
            if fed < samples.count {
                recognizer.feed(Array(samples[fed...]))
                fed = samples.count
            }
            return recognizer
        }
    }

    /// The audio has ended: the text to insert.
    func finish() async -> Outcome {
        let audio = lock.withLock { () -> [Float]? in
            guard !cancelled, !finished else { return nil }
            finished = true
            return samples
        }
        pumpTask?.cancel()
        guard let audio else { return .cancelled }
        // Too short to be speech: nothing, rather than what a model makes of
        // a click.
        guard audio.count >= Self.minimumSamples else {
            closeRecognizer()
            loadTask?.cancel()
            return .text("")
        }
        // The model may still be loading; the streaming text is worth waiting
        // for unless a final pass will produce the text anyway.
        if finalPass == nil { await loadTask?.value }
        var streamed = ""
        if let recognizer = feedPending(unlessFinished: false) {
            streamed = await recognizer.finish()
            if isCancelled { closeRecognizer(); return .cancelled }
            if finalPass != nil, !streamed.isEmpty, streamed != lock.withLock({ shown }) {
                lock.withLock { shown = streamed }
                onVolatile(streamed)
            }
        }
        // Never both models at their peaks: the streaming one goes before
        // the final pass starts.
        closeRecognizer()
        loadTask?.cancel()
        guard let finalPass else {
            if streamed.isEmpty, let failure = lock.withLock({ loadFailure }) {
                return .failed(failure)
            }
            return .text(streamed)
        }
        do {
            let text = try await finalPass(audio)
            return isCancelled ? .cancelled : .text(text)
        } catch {
            if isCancelled || error is CancellationError { return .cancelled }
            // The final pass broke; what the streaming model heard is still
            // what the person said.
            return .text(streamed)
        }
    }

    /// Stop now: no more updates, the model let go.
    func cancel() {
        lock.withLock { cancelled = true }
        pumpTask?.cancel()
        loadTask?.cancel()
        closeRecognizer()
    }

    private var isCancelled: Bool {
        lock.withLock { cancelled }
    }

    private func closeRecognizer() {
        let held = lock.withLock { () -> (any StreamingRecognizing)? in
            closed = true
            defer { recognizer = nil }
            return recognizer
        }
        held?.close()
    }
}
