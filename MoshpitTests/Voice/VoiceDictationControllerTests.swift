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

        private(set) var started = false
        private(set) var appended = 0
        private(set) var finishAudioCalled = false
        private(set) var cancelCalled = false

        init() {
            (updates, continuation) = AsyncThrowingStream.makeStream()
        }

        func prepare(onDownloadProgress: @escaping @Sendable (Double) -> Void) async throws {
            if let prepareError { throw prepareError }
            for value in downloadProgress { onDownloadProgress(value) }
            while holdPrepare {
                try? await Task.sleep(for: .milliseconds(10))
            }
        }

        func start() async throws { started = true }
        func append(_: AVAudioPCMBuffer) { appended += 1 }
        func finishAudio() async {
            finishAudioCalled = true
            continuation.finish()
        }

        func cancel() {
            cancelCalled = true
            continuation.finish()
        }
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
        VoiceDictationController(engineFactory: { _ in [engine] }, audioSource: audio)
    }

    private func makeController(engines: [FakeEngine],
                                audio: FakeAudioSource) -> VoiceDictationController {
        VoiceDictationController(engineFactory: { _ in engines }, audioSource: audio)
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

        await controller.start(localeId: "")

        #expect(controller.phase == .failed(.microphoneDenied))
        #expect(!audio.started)
        #expect(!engine.started)
    }

    @Test("Happy path: volatile shimmers, finals accumulate, finish inserts")
    func happyPath() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(localeId: "")
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

        await controller.start(localeId: "")
        engine.continuation.yield(DictationUpdate(finalizedText: "rm -rf", volatileText: ""))
        _ = await waitUntil(controller.finalizedText == "rm -rf")

        controller.cancel()

        #expect(engine.cancelCalled)
        #expect(audio.stopped)
        #expect(controller.phase == .idle)
    }

    @Test("Model download is visible as its own phase")
    func downloadPhase() async {
        let engine = FakeEngine()
        engine.downloadProgress = [0.4]
        engine.holdPrepare = true
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        let startTask = Task { await controller.start(localeId: "") }
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

        await controller.start(localeId: "")

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

        await controller.start(localeId: "")

        #expect(controller.phase == .failed(.speechRecognitionDenied))
        #expect(!next.started)
    }

    @Test("Unsupported language surfaces as a failure")
    func unsupportedLanguage() async {
        let engine = FakeEngine()
        engine.prepareError = DictationFailure.unsupportedLanguage("Klingon")
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(localeId: "tlh")

        #expect(controller.phase == .failed(.unsupportedLanguage("Klingon")))
    }

    @Test("Finishing with nothing heard returns nil")
    func emptyFinish() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(localeId: "")
        let text = await controller.finish()

        #expect(text == nil)
        #expect(controller.phase == .idle)
    }

    @Test("Mic buffers are forwarded to the engine")
    func bufferForwarding() async throws {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(localeId: "")
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

        await controller.start(localeId: "")
        engine.continuation.yield(DictationUpdate(finalizedText: "tail the logs", volatileText: ""))
        _ = await waitUntil(controller.finalizedText == "tail the logs")

        audio.onInterruption?()
        #expect(await waitUntil(controller.phase == .interrupted))
        #expect(audio.stopped)

        let text = await controller.finish()
        #expect(text == "tail the logs")
    }

    @Test("Engine failure mid-listen surfaces and stops capture")
    func engineFailureSurfaces() async {
        let engine = FakeEngine()
        let audio = FakeAudioSource()
        let controller = makeController(engine: engine, audio: audio)

        await controller.start(localeId: "")
        engine.continuation.finish(throwing: DictationFailure.engine("asset gone"))

        #expect(await waitUntil(controller.phase == .failed(.engine("asset gone"))))
        #expect(audio.stopped)
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

    @Test("Empty sides pass through")
    func empties() {
        #expect(DictationTranscript.join("", "x") == "x")
        #expect(DictationTranscript.join("x", "") == "x")
        #expect(DictationTranscript.join("", "") == "")
    }

    @Test("Mixed ASCII/CJK boundary joins tight")
    func mixed() {
        #expect(DictationTranscript.join("用 git", "提交") == "用 git提交")
        #expect(DictationTranscript.join("提交到", "main") == "提交到main")
    }
}
