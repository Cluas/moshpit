import AVFoundation
import Foundation
import Testing
@testable import Moshpit

/// The dictation state machine, held still with a scripted engine and a fake
/// mic — permission denial, model download, live volatile/final assembly,
/// insert vs. cancel, interruption, and engine failure, none of which need
/// audio hardware to be true.
@Suite("Voice dictation controller")
@MainActor
struct VoiceDictationControllerTests {

    // MARK: Fakes

    final class FakeEngine: DictationEngine {
        let updates: AsyncThrowingStream<DictationUpdate, Error>
        let continuation: AsyncThrowingStream<DictationUpdate, Error>.Continuation

        var prepareError: Error?
        /// Progress values `prepare` reports before returning.
        var downloadProgress: [Double] = []
        /// While true, `prepare` idles after reporting progress — lets a test
        /// observe the `.downloadingModel` phase instead of racing past it.
        var holdPrepare = false
        /// Report a `.loading` step after any download progress.
        var reportsLoading = false
        var label = "Fake · Test"
        var finalizeTimeout: Duration = .seconds(5)
        /// `finishAudio()` returns but `updates` never ends: a recognizer that
        /// never delivers its final result once the audio has ended.
        var finalNeverArrives = false
        /// `finishAudio()` itself doesn't return until `releaseFinish()`: a
        /// final pass stuck inside the engine.
        var finishAudioHangs = false

        private(set) var started = false
        private(set) var appended = 0
        private(set) var finishAudioCalled = false
        /// Set as a hanging `finishAudio()` returns: whether the work it was
        /// doing had been cancelled by then.
        private(set) var finishSawCancellation: Bool?
        private(set) var cancelCalled = false

        private let finishGate = NSLock()
        private var finishWaiter: CheckedContinuation<Void, Never>?
        private var finishReleased = false

        init() {
            (updates, continuation) = AsyncThrowingStream.makeStream()
        }

        func prepare(onProgress: @escaping @Sendable (DictationPreparation) -> Void) async throws {
            if let prepareError { throw prepareError }
            for value in downloadProgress { onProgress(.downloading(progress: value)) }
            if reportsLoading { onProgress(.loading) }
            while holdPrepare {
                try? await Task.sleep(for: .milliseconds(10))
            }
        }

        func start() async throws { started = true }
        func append(_: AVAudioPCMBuffer) { appended += 1 }
        func finishAudio() async {
            finishAudioCalled = true
            if finishAudioHangs {
                // Not cancellable, like the awaits a real engine sits on.
                await withCheckedContinuation { (waiter: CheckedContinuation<Void, Never>) in
                    let released = finishGate.withLock { () -> Bool in
                        if !finishReleased { finishWaiter = waiter }
                        return finishReleased
                    }
                    if released { waiter.resume() }
                }
                finishSawCancellation = Task.isCancelled
            }
            if !finalNeverArrives { continuation.finish() }
        }

        /// Lets a hanging `finishAudio()` return.
        func releaseFinish() {
            let waiter = finishGate.withLock { () -> CheckedContinuation<Void, Never>? in
                finishReleased = true
                defer { finishWaiter = nil }
                return finishWaiter
            }
            waiter?.resume()
        }

        func cancel() {
            cancelCalled = true
            continuation.finish()
        }
    }

    /// Where a stuck engine is stuck once the mic stops.
    enum Stall: CaseIterable, Sendable {
        /// The final result never arrives after `finishAudio()` returned.
        case finalResult
        /// `finishAudio()` never returns.
        case finishAudio
    }

    final class FakeAudioSource: DictationAudioSource {
        var permission = true
        var startError: Error?
        private(set) var started = false
        private(set) var stopped = false
        var onBuffer: (@Sendable (AVAudioPCMBuffer) -> Void)?
        var onLevel: (@Sendable (Float) -> Void)?
        var onInterruption: (@Sendable () -> Void)?

        func requestPermission() async -> Bool { permission }

        func start(onBuffer: @escaping @Sendable (AVAudioPCMBuffer) -> Void,
                   onLevel: @escaping @Sendable (Float) -> Void,
                   onInterruption: @escaping @Sendable () -> Void) throws {
            if let startError { throw startError }
            self.onBuffer = onBuffer
            self.onLevel = onLevel
            self.onInterruption = onInterruption
            started = true
        }

        func stop() { stopped = true }
    }

    private func makeController(engine: FakeEngine,
                                audio: FakeAudioSource) -> VoiceDictationController {
        VoiceDictationController(engineFactory: { _, _ in [engine] }, audioSource: audio)
    }

    private func makeController(engines: [FakeEngine],
                                audio: FakeAudioSource) -> VoiceDictationController {
        VoiceDictationController(engineFactory: { _, _ in engines }, audioSource: audio)
    }

    /// Main-actor-friendly poll: updates arrive via tasks the controller
    /// spawns, so tests wait for the state to settle instead of assuming
    /// scheduling order.
    private func waitUntil(_ condition: @autoclosure () -> Bool,
                           timeout: Duration = .seconds(2)) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return condition()
    }

    // MARK: Tests

    @Test("Mic permission denied fails without touching capture")
    func micDenied() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        audio.permission = false
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())

        #expect(controller.phase == .failed(.microphoneDenied))
        #expect(!audio.started)
        #expect(!engine.started)
    }

    @Test("Happy path: volatile shimmers, finals accumulate, finish inserts")
    func happyPath() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        #expect(controller.phase == .listening)
        #expect(audio.started)

        engine.continuation.yield(DictationUpdate(finalizedText: "", volatileText: "deploy the"))
        #expect(await waitUntil(controller.volatileText == "deploy the"))
        #expect(controller.transcript == "deploy the")

        engine.continuation.yield(DictationUpdate(finalizedText: "deploy the staging branch", volatileText: ""))
        #expect(await waitUntil(controller.finalizedText == "deploy the staging branch"))

        let text = await controller.finish()
        #expect(text == "deploy the staging branch")
        #expect(engine.finishAudioCalled)
        #expect(audio.stopped)
        #expect(controller.phase == .idle)
    }

    @Test("Cancel discards the transcript and tears down")
    func cancelDiscards() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "rm -rf", volatileText: ""))
        _ = await waitUntil(controller.finalizedText == "rm -rf")

        controller.cancel()

        #expect(engine.cancelCalled)
        #expect(audio.stopped)
        #expect(controller.phase == .idle)
    }

    @Test("Cancelled while the model loads: the mic never opens and the engine is let go")
    func cancelWhilePreparing() async {
        let engine = FakeEngine()
        engine.reportsLoading = true
        engine.holdPrepare = true
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        let startTask = Task { await controller.start(DictationRequest()) }
        #expect(await waitUntil(controller.phase == .loadingModel))

        // The mic key (or Cancel) while "Loading model…" is up. The overlay
        // goes away with it, so a session that carried on would be listening
        // where nobody can see or stop it — and a Whisper engine left running
        // holds its model for good.
        controller.cancel()
        engine.holdPrepare = false
        await startTask.value

        #expect(!audio.started)
        #expect(!engine.started)
        #expect(engine.cancelCalled)
        #expect(controller.phase == .idle)
    }

    @Test("Model download is visible as its own phase")
    func downloadPhase() async {
        let engine = FakeEngine()
        engine.downloadProgress = [0.4]
        engine.holdPrepare = true
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        let startTask = Task { await controller.start(DictationRequest()) }
        #expect(await waitUntil(controller.phase == .downloadingModel(progress: 0.4)))

        engine.holdPrepare = false
        await startTask.value
        #expect(controller.phase == .listening)
    }

    @Test("A dead first engine degrades to the next candidate")
    func engineFallback() async {
        let broken = FakeEngine()
        broken.prepareError = DictationFailure.engine("not subscribed to transcription.en")
        let working = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engines: [broken, working], audio: audio)

        await controller.start(DictationRequest())

        #expect(controller.phase == .listening)
        #expect(broken.cancelCalled)
        #expect(working.started)

        working.continuation.yield(DictationUpdate(finalizedText: "echo ok", volatileText: ""))
        let text = await controller.finish()
        #expect(text == "echo ok")
    }

    @Test("Speech-recognition denial stops the chain instead of re-asking")
    func denialStopsChain() async {
        let denied = FakeEngine()
        denied.prepareError = DictationFailure.speechRecognitionDenied
        let next = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engines: [denied, next], audio: audio)

        await controller.start(DictationRequest())

        #expect(controller.phase == .failed(.speechRecognitionDenied))
        #expect(!next.started)
    }

    @Test("Unsupported language surfaces as a failure")
    func unsupportedLanguage() async {
        let engine = FakeEngine()
        engine.prepareError = DictationFailure.unsupportedLanguage("Klingon")
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest(appleLocaleId: "tlh"))

        #expect(controller.phase == .failed(.unsupportedLanguage("Klingon")))
    }

    @Test("Finishing with nothing heard returns nil")
    func emptyFinish() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        let text = await controller.finish()

        #expect(text == nil)
        #expect(controller.phase == .idle)
    }

    @Test("Mic buffers are forwarded to the engine")
    func bufferForwarding() async throws {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        let format = try #require(AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 160))
        audio.onBuffer?(buffer)

        #expect(await waitUntil(engine.appended == 1))
        _ = controller // keep alive through the poll
    }

    @Test("Interruption keeps the transcript and parks in .interrupted")
    func interruptionKeepsTranscript() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "tail the logs", volatileText: ""))
        _ = await waitUntil(controller.finalizedText == "tail the logs")

        audio.onInterruption?()
        #expect(await waitUntil(controller.phase == .interrupted))
        #expect(audio.stopped)

        let text = await controller.finish()
        #expect(text == "tail the logs")
    }

    @Test("An engine that never finishes can't hold the overlay on finishing: the budget ends it",
          arguments: Stall.allCases)
    func finishingIsBounded(stall: Stall) async {
        let engine = FakeEngine()
        engine.finalizeTimeout = .milliseconds(100)
        engine.finalNeverArrives = true
        engine.finishAudioHangs = stall == .finishAudio
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "git status", volatileText: ""))
        #expect(await waitUntil(controller.finalizedText == "git status"))

        let finishing = Task { await controller.finish() }
        // Thirty times the budget. The task group this replaced never got
        // there: it waited for the finishing side however long that took.
        let ended = await waitUntil(controller.phase == .idle, timeout: .seconds(3))
        #expect(ended, "still on \(controller.phase) long after the 100 ms budget")
        #expect(engine.cancelCalled, "the engine was never let go")

        // Unstick the fake so a failing run still ends instead of hanging.
        engine.releaseFinish()
        engine.continuation.finish()
        #expect(await finishing.value == "git status", "what was heard is kept")
        if stall == .finishAudio {
            // The abandoned final pass was told to stop, not left decoding.
            #expect(await waitUntil(engine.finishSawCancellation != nil))
            #expect(engine.finishSawCancellation == true)
        }
    }

    /// Cancel stays on screen while the engine finalizes — seconds, with
    /// Whisper decoding the whole utterance inside `finishAudio()`. The
    /// transcript came back anyway once the engine was done: the overlay had
    /// gone, and the words just cancelled were typed into the terminal — with
    /// a Return after them when Send had started the finish.
    @Test("Cancel while the engine finalizes: nothing is handed out")
    func cancelWhileFinishing() async {
        let engine = FakeEngine()
        engine.finishAudioHangs = true
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "rm -rf build", volatileText: ""))
        #expect(await waitUntil(controller.finalizedText == "rm -rf build"))

        // Send.
        let send = Task { await controller.finish() }
        #expect(await waitUntil(controller.phase == .finishing))
        // Cancel, while the final pass is still running.
        controller.cancel()
        engine.releaseFinish()

        let typed = await send.value
        #expect(typed == nil, "a cancelled dictation was typed into the terminal: \(typed ?? "")")
        #expect(controller.transcript.isEmpty)
    }

    /// Insert stays tappable while the engine finalizes; the second tap got
    /// the transcript too (half of it, straight away) and the words went into
    /// the terminal twice.
    @Test("Insert tapped twice while the engine finalizes inserts once")
    func finishTwiceInsertsOnce() async {
        let engine = FakeEngine()
        engine.finishAudioHangs = true
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "git push", volatileText: ""))
        #expect(await waitUntil(controller.finalizedText == "git push"))

        let first = Task { await controller.finish() }
        #expect(await waitUntil(controller.phase == .finishing))
        let second = Task { await controller.finish() }
        try? await Task.sleep(for: .milliseconds(50))
        engine.releaseFinish()

        let results = [await first.value, await second.value].compactMap { $0 }
        #expect(results == ["git push"], "handed out \(results.count) times: \(results)")
        #expect(controller.phase == .idle)
    }

    /// Insert while an interruption is still finalizing waits for it and
    /// gets the whole transcript, and the session ends there.
    @Test("Insert during an interruption's finalize gets the full transcript")
    func finishJoinsTheInterruptionsStop() async {
        let engine = FakeEngine()
        engine.finishAudioHangs = true
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "tail", volatileText: ""))
        #expect(await waitUntil(controller.finalizedText == "tail"))

        audio.onInterruption?()
        #expect(await waitUntil(controller.phase == .finishing))
        let insert = Task { await controller.finish() }
        try? await Task.sleep(for: .milliseconds(50))
        // The engine's last words land while both are waiting.
        engine.continuation.yield(DictationUpdate(finalizedText: "tail the logs", volatileText: ""))
        try? await Task.sleep(for: .milliseconds(50))
        engine.releaseFinish()

        #expect(await insert.value == "tail the logs")
        try? await Task.sleep(for: .milliseconds(50))
        #expect(controller.phase == .idle, "the finished session went back to \(controller.phase)")
    }

    @Test("Engine failure mid-listen surfaces and stops capture")
    func engineFailureSurfaces() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())
        engine.continuation.finish(throwing: DictationFailure.engine("asset gone"))

        #expect(await waitUntil(controller.phase == .failed(.engine("asset gone"))))
        #expect(audio.stopped)
    }

    @Test("A mic that fails to start is stopped again: the audio session isn't left on (B860)")
    func failedMicStartStopsCapture() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let failure = DictationFailure.audioCapture("input busy")
        audio.startError = failure
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(DictationRequest())

        #expect(controller.phase == .failed(.audioCapture(failure.localizedDescription)))
        #expect(engine.cancelCalled)
        #expect(audio.stopped, "the half-started capture was never stopped")
    }

    @Test("A dictation let go while listening stops the mic and the engine (B861)")
    func droppedWhileListening() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        weak var gone: VoiceDictationController?
        do {
            let controller = makeController(engine: engine, audio: audio)
            gone = controller
            await controller.start(DictationRequest())
            #expect(controller.phase == .listening)
        }
        _ = await waitUntil(gone == nil)
        #expect(gone == nil, "nothing should hold a dictation its screen let go")
        #expect(audio.stopped, "the mic is still on, with nobody to show it")
        #expect(engine.cancelCalled, "the engine is still listening, for nobody")
    }

    @Test("A dictation let go while its model loads doesn't leave the mic on (B861)")
    func droppedWhilePreparing() async {
        let engine = FakeEngine()
        engine.holdPrepare = true
        let audio = FakeAudioSource()
        weak var gone: VoiceDictationController?
        do {
            let controller = makeController(engine: engine, audio: audio)
            gone = controller
            Task { await controller.start(DictationRequest()) }
            _ = await waitUntil(controller.phase == .starting)
        }
        try? await Task.sleep(for: .milliseconds(30))
        engine.holdPrepare = false
        _ = await waitUntil(gone == nil)
        #expect(gone == nil)
        #expect(!audio.started || audio.stopped, "the mic opened after its screen was gone, and stayed on")
    }
}

/// The CJK-aware transcript joiner — the reason "你好"+"世界" doesn't grow a
/// space and "hello"+"world" does.
@Suite("Dictation transcript joining")
struct DictationTranscriptTests {
    @Test("ASCII words get a separating space")
    func asciiWords() {
        #expect(DictationTranscript.join("hello", "world") == "hello world")
    }

    @Test("CJK segments join without spaces")
    func cjk() {
        #expect(DictationTranscript.join("你好", "世界") == "你好世界")
    }

    @Test("Existing whitespace is respected")
    func existingSpace() {
        #expect(DictationTranscript.join("hello ", "world") == "hello world")
    }

    @Test("ASCII clause punctuation takes a space; CJK punctuation doesn't")
    func punctuation() {
        #expect(DictationTranscript.join("wait,", "go") == "wait, go")
        #expect(DictationTranscript.join("Hello.", "How are you") == "Hello. How are you")
        #expect(DictationTranscript.join("hello", "，世界") == "hello，世界")
        #expect(DictationTranscript.join("你好。", "再说一次") == "你好。再说一次")
    }

    @Test("Chinese characters change script; English and punctuation don't (B670)")
    func scriptConversion() {
        let traditional = "因為我也很少stay in home，所以是我的parents。"
        let simplified = "因为我也很少stay in home，所以是我的parents。"
        #expect(DictationTranscript.converting(traditional, to: .simplified) == simplified)
        #expect(DictationTranscript.converting(simplified, to: .traditional) == traditional)
        #expect(DictationTranscript.converting(traditional, to: nil) == traditional)
        #expect(DictationTranscript.converting("git status", to: .simplified) == "git status")
    }

    @Test("A decoder loop is kept once (B670)")
    func repetitionLoop() {
        let loop = "那如果有" + String(repeating: "SK-", count: 40)
        #expect(DictationTranscript.collapsingRepeats(loop) == "那如果有SK-")
        let phrase = "我的parents，" + String(repeating: "my parents，", count: 9)
        #expect(DictationTranscript.collapsingRepeats(phrase) == "我的parents，my parents，")
    }

    @Test("Speech that repeats itself a few times is left alone")
    func ordinaryRepeats() {
        for text in ["the name is blah blah blah blah blah", "git checkout feature/login 然后跑一下 npm test",
                     "好好好", "no no no no", "----", ""] {
            #expect(DictationTranscript.collapsingRepeats(text) == text)
        }
    }

    @Test("Empty sides pass through")
    func empties() {
        #expect(DictationTranscript.join("", "x") == "x")
        #expect(DictationTranscript.join("x", "") == "x")
        #expect(DictationTranscript.join("", "").isEmpty)
    }

    @Test("Mixed ASCII/CJK boundary joins tight")
    func mixed() {
        #expect(DictationTranscript.join("用 git", "提交") == "用 git提交")
        #expect(DictationTranscript.join("提交到", "main") == "提交到main")
    }
}
