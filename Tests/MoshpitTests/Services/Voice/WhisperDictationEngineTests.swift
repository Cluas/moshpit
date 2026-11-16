import AVFoundation
import Foundation
import Testing
@testable import Moshpit

/// The Whisper engine's own contract, against a scripted transcriber — no
/// model, no Neural Engine, no network. What's pinned here is the behaviour
/// that differs from the streaming engines: audio is buffered and re-read,
/// previews are advisory, and the pass that produces the final transcript is
/// the one in `finishAudio()`.
@Suite("Whisper dictation engine")
struct WhisperDictationEngineTests {

    /// Records what it was asked to transcribe and replays scripted answers.
    actor ScriptedTranscriber: WhisperTranscribing {
        /// Answers handed out in order; the last one repeats once exhausted.
        private var answers: [String]
        private var index = 0
        private(set) var warmUps: [String] = []
        /// Sample counts each `transcribe` call saw, in order.
        private(set) var sampleCounts: [Int] = []
        private(set) var languages: [String?] = []
        /// The hotwords each `transcribe` call was given, in order.
        private(set) var hotwordsHeard: [[String]] = []
        private var warmUpError: Error?
        /// Delay before each answer, to keep a preview in flight.
        private var latency: Duration = .zero
        /// Sessions holding the model right now, and every release made.
        private(set) var openSessions: Set<UUID> = []
        private(set) var ended: [UUID] = []
        /// begin / warmUp / transcribe / end, in the order they happened.
        private(set) var events: [String] = []

        init(answers: [String], warmUpError: Error? = nil) {
            self.answers = answers
            self.warmUpError = warmUpError
        }

        func setLatency(_ value: Duration) { latency = value }

        /// Every `transcribe` throws this from now on.
        private var transcribeError: Error?
        func failTranscribing(with error: Error) { transcribeError = error }

        func warmUp(variant: String) async throws {
            warmUps.append(variant)
            events.append("warmUp")
            if let warmUpError { throw warmUpError }
        }

        func beginSession() -> UUID {
            let id = UUID()
            openSessions.insert(id)
            events.append("begin")
            return id
        }

        func endSession(_ id: UUID) {
            openSessions.remove(id)
            ended.append(id)
            events.append("end")
        }

        func transcribe(samples: [Float], language: String?, hotwords: [String],
                        variant _: String) async throws -> String {
            events.append("transcribe")
            hotwordsHeard.append(hotwords)
            sampleCounts.append(samples.count)
            languages.append(language)
            if latency != .zero { try? await Task.sleep(for: latency) }
            if let transcribeError { throw transcribeError }
            guard !answers.isEmpty else { return "" }
            let answer = answers[min(index, answers.count - 1)]
            index += 1
            return answer
        }
    }

    /// A buffer of `frames` at 16 kHz mono, filled with quiet noise so it
    /// isn't optimized away as empty.
    private func buffer(frames: AVAudioFrameCount) throws -> AVAudioPCMBuffer {
        let format = try #require(AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames))
        buffer.frameLength = frames
        if let channel = buffer.floatChannelData?[0] {
            for i in 0 ..< Int(frames) { channel[i] = Float(i % 32) / 1000 }
        }
        return buffer
    }

    private func makeEngine(_ store: ScriptedTranscriber,
                            language: String? = "zh") -> WhisperDictationEngine {
        WhisperDictationEngine(store: store, variant: "openai_whisper-small",
                               language: language, label: "Small · Chinese")
    }

    /// Polls an actor-held condition: `cancel()` and `deinit` release the
    /// model from a task of their own.
    private func eventually(_ condition: @escaping () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(2)
        while ContinuousClock.now < deadline {
            if await condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return await condition()
    }

    @Test("Final pass reads every sample appended, and lands as finalized text")
    func finalPassSeesEverything() async throws {
        let store = ScriptedTranscriber(answers: ["把这个 commit rebase 到 main"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()

        // Three seconds of audio in three chunks.
        for _ in 0 ..< 3 { engine.append(try buffer(frames: 16_000)) }
        await engine.finishAudio()

        var updates: [DictationUpdate] = []
        for try await update in engine.updates { updates.append(update) }

        let final = try #require(updates.last)
        #expect(final.finalizedText == "把这个 commit rebase 到 main")
        #expect(final.volatileText.isEmpty)
        // The pass that produced it saw all 48 000 samples, not just the last
        // chunk — that whole-utterance context is the entire point.
        let counts = await store.sampleCounts
        #expect(counts.last == 48_000)
    }

    @Test("The pane's hotwords reach the model on every pass")
    func hotwordsPassedThrough() async throws {
        let store = ScriptedTranscriber(answers: ["打开 AppSettings.swift"])
        let engine = WhisperDictationEngine(
            store: store, variant: Qwen3ASR.variant, language: nil, label: "Qwen3-ASR 0.6B",
            hotwords: ["Claude Code", "AppSettings.swift"])
        try await engine.prepare { _ in }
        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        for try await _ in engine.updates {}

        let heard = await store.hotwordsHeard
        #expect(!heard.isEmpty)
        #expect(heard.allSatisfy { $0 == ["Claude Code", "AppSettings.swift"] })
    }

    @Test("What the model writes comes out in the user's Chinese script (B670)")
    func scriptApplied() async throws {
        let store = ScriptedTranscriber(answers: ["那個economic status就是買不買得起"])
        let engine = WhisperDictationEngine(store: store, variant: "openai_whisper-small",
                                            language: nil, label: "Small · Auto", script: .simplified)
        try await engine.prepare { _ in }
        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        var updates: [DictationUpdate] = []
        for try await update in engine.updates { updates.append(update) }
        #expect(updates.last?.finalizedText == "那个economic status就是买不买得起")
    }

    @Test("The configured language reaches the model")
    func languagePassedThrough() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store, language: "zh")
        try await engine.prepare { _ in }
        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        for try await _ in engine.updates {}

        let languages = await store.languages
        #expect(languages.allSatisfy { $0 == "zh" })
    }

    @Test("Auto-detect sends nil rather than a guessed language")
    func autoDetectSendsNil() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store, language: nil)
        try await engine.prepare { _ in }
        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        for try await _ in engine.updates {}

        let languages = await store.languages
        #expect(languages.allSatisfy { $0 == nil })
    }

    @Test("Buffers arriving in a foreign format are resampled, not dropped")
    func resamplesForeignFormat() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()

        // 48 kHz stereo — what an actual iPhone mic tap hands over.
        let format = try #require(AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: false))
        let input = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 48_000))
        input.frameLength = 48_000
        for channel in 0 ..< 2 {
            if let data = input.floatChannelData?[channel] {
                for i in 0 ..< 48_000 { data[i] = Float(i % 64) / 1000 }
            }
        }
        engine.append(input)
        await engine.finishAudio()
        for try await _ in engine.updates {}

        // One second in at 48 kHz stereo must arrive as ~one second of 16 kHz
        // mono — a third of the frames, not all of them and not none.
        let counts = await store.sampleCounts
        let seen = try #require(counts.last)
        #expect(seen > 15_000 && seen < 17_000)
    }

    @Test("Too little audio is never sent to the model")
    func silenceFloor() async throws {
        let store = ScriptedTranscriber(answers: ["hallucinated phrase"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()
        // 0.2 s — below the floor that keeps Whisper from inventing text.
        engine.append(try buffer(frames: 3_200))
        await engine.finishAudio()

        var updates: [DictationUpdate] = []
        for try await update in engine.updates { updates.append(update) }

        #expect(updates.last?.finalizedText.isEmpty == true)
    }

    @Test("A missing model reports as the setup problem it is")
    func missingModelFailure() async throws {
        let store = ScriptedTranscriber(
            answers: [], warmUpError: WhisperModelFailure.notInstalled("Small"))
        let engine = makeEngine(store)

        await #expect(throws: DictationFailure.noWhisperModel) {
            try await engine.prepare { _ in }
        }
    }

    @Test("prepare reports loading, never downloading")
    func prepareReportsLoading() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store)
        // The mic key must never kick off a several-hundred-megabyte
        // transfer; downloads belong to Settings alone.
        nonisolated(unsafe) var steps: [DictationPreparation] = []
        try await engine.prepare { steps.append($0) }

        #expect(steps == [.loading])
        let warmUps = await store.warmUps
        #expect(warmUps == ["openai_whisper-small"])
    }

    @Test("Finalize budget scales with how long the recording is")
    func finalizeBudgetScales() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()

        // Empty: the floor, which has to cover model load on a cold start.
        #expect(engine.finalizeTimeout >= .seconds(20))

        // 60 s of audio needs more than the 5 s a streaming engine gets, or
        // the controller would time out and discard the whole transcript.
        for _ in 0 ..< 60 { engine.append(try buffer(frames: 16_000)) }
        #expect(engine.finalizeTimeout >= .seconds(120))

        engine.cancel()
    }

    @Test("Cancel stops the session without emitting a transcript")
    func cancelIsSilent() async throws {
        let store = ScriptedTranscriber(answers: ["should not appear"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        engine.cancel()

        var updates: [DictationUpdate] = []
        for try await update in engine.updates { updates.append(update) }
        #expect(updates.isEmpty)
    }

    @Test("Live preview publishes a hypothesis while listening")
    func previewPublishesVolatileText() async throws {
        let store = ScriptedTranscriber(answers: ["部署 staging", "部署 staging 分支"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()
        // Over the 2 s preview floor so the loop has something to chew on.
        for _ in 0 ..< 3 { engine.append(try buffer(frames: 16_000)) }

        // Preview arrives as volatile — replaceable, and never mistaken for
        // committed text the user could insert as final.
        let firstPreview = await withTaskGroup(of: DictationUpdate?.self) { group in
            group.addTask {
                do {
                    for try await update in engine.updates where !update.volatileText.isEmpty {
                        return update
                    }
                } catch {}
                return nil
            }
            group.addTask {
                try? await Task.sleep(for: .seconds(5))
                return nil
            }
            // `next()` wraps the child's optional: flatten so the timer's nil
            // and an empty group both read as "no preview".
            let first = await group.next()
            group.cancelAll()
            return first?.flatMap { $0 }
        }

        let preview = try #require(firstPreview)
        #expect(preview.finalizedText.isEmpty)
        #expect(preview.volatileText == "部署 staging")
        engine.cancel()
    }

    // MARK: Holding the model

    @Test("The model is held from prepare until after the final pass")
    func holdsModelThroughFinalPass() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        // Held before the load, so an unload can't land between the two.
        #expect(await store.openSessions.count == 1)
        #expect(await store.events.prefix(2) == ["begin", "warmUp"])

        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        for try await _ in engine.updates {}

        #expect(await store.openSessions.isEmpty)
        // Let go after the pass that counts, never before it.
        #expect(await store.events.suffix(2) == ["transcribe", "end"])
    }

    @Test("Cancel lets the model go")
    func cancelReleasesModel() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        #expect(await store.openSessions.count == 1)
        try await engine.start()
        engine.cancel()

        #expect(await eventually { await store.openSessions.isEmpty })
    }

    @Test("A model that fails to load is let go at once")
    func failedPrepareReleasesModel() async throws {
        let store = ScriptedTranscriber(
            answers: [], warmUpError: WhisperModelFailure.notInstalled("Small"))
        let engine = makeEngine(store)
        await #expect(throws: DictationFailure.noWhisperModel) {
            try await engine.prepare { _ in }
        }
        #expect(await store.openSessions.isEmpty)
        #expect(await store.ended.count == 1)
    }

    @Test("Finish, then a late cancel: the hold is given back once")
    func releasesOnce() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        let engine = makeEngine(store)
        try await engine.prepare { _ in }
        try await engine.start()
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        // What the controller does when finishing overran its budget.
        engine.cancel()
        try await Task.sleep(for: .milliseconds(50))

        #expect(await store.ended.count == 1)
    }

    @Test("An engine dropped without finish or cancel still lets the model go")
    func droppedEngineReleasesModel() async throws {
        let store = ScriptedTranscriber(answers: ["ok"])
        var engine: WhisperDictationEngine? = makeEngine(store)
        try await engine?.prepare { _ in }
        #expect(await store.openSessions.count == 1)

        engine = nil
        #expect(await eventually { await store.openSessions.isEmpty })
    }

    // MARK: X-ASR preview (B763)

    /// A streaming model that says how much it heard, and a fixed final text.
    final class CountingStream: StreamingRecognizing, @unchecked Sendable {
        private let lock = NSLock()
        private var heard = 0
        private(set) var closed = false
        func feed(_ samples: [Float]) { lock.withLock { heard += samples.count } }
        func partialText() async -> String { lock.withLock { heard == 0 ? "" : "听到 \(heard)" } }
        func finish() async -> String { "部署 staging" }
        func close() { lock.withLock { closed = true } }
        var isClosed: Bool { lock.withLock { closed } }
    }

    /// Everything the engine published until it finished.
    private func collect(_ engine: DictationEngine) async -> [DictationUpdate] {
        var updates: [DictationUpdate] = []
        do { for try await update in engine.updates { updates.append(update) } } catch {}
        return updates
    }

    @Test("With X-ASR on the device, its words are the preview and the model's final pass is the text")
    func streamingPreviewHandsOffToTheFinalPass() async throws {
        let store = ScriptedTranscriber(answers: ["部署 staging 分支。"])
        let stream = CountingStream()
        let engine = WhisperDictationEngine(store: store, variant: Qwen3ASR.variant, language: nil,
                                            label: "Qwen3-ASR", streaming: { stream })
        try await engine.prepare { _ in }
        try await engine.start()
        let updates = Task { await collect(engine) }
        for _ in 0 ..< 2 { engine.append(try buffer(frames: 16_000)) }
        try await Task.sleep(for: .milliseconds(400))
        await engine.finishAudio()
        let published = await updates.value

        #expect(published.contains { $0.volatileText.hasPrefix("听到") && $0.finalizedText.isEmpty })
        #expect(published.contains { $0.volatileText == "部署 staging" })
        #expect(published.last == DictationUpdate(finalizedText: "部署 staging 分支。", volatileText: ""))
        // Only the final pass reached the batch model: no re-decoding preview.
        #expect(await store.sampleCounts == [32_000])
        #expect(stream.isClosed)
    }

    @Test("When the final pass fails, what X-ASR heard is inserted")
    func streamingPreviewCoversAFailedFinalPass() async throws {
        struct Broke: Error {}
        let store = ScriptedTranscriber(answers: ["never"])
        await store.failTranscribing(with: Broke())
        let stream = CountingStream()
        let engine = WhisperDictationEngine(store: store, variant: Qwen3ASR.variant, language: nil,
                                            label: "Qwen3-ASR", streaming: { stream })
        try await engine.prepare { _ in }
        try await engine.start()
        let updates = Task { await collect(engine) }
        engine.append(try buffer(frames: 16_000))
        await engine.finishAudio()
        #expect(await updates.value.last == DictationUpdate(finalizedText: "部署 staging", volatileText: ""))
        #expect(stream.isClosed)
    }

    @Test("X-ASR alone: its words while listening, its text at the end, the model let go")
    func streamingEngineAlone() async throws {
        let stream = CountingStream()
        let engine = XASRDictationEngine(load: { stream }, label: "X-ASR")
        try await engine.start()
        let updates = Task { await collect(engine) }
        for _ in 0 ..< 2 { engine.append(try buffer(frames: 16_000)) }
        try await Task.sleep(for: .milliseconds(400))
        await engine.finishAudio()
        let published = await updates.value
        #expect(published.contains { $0.volatileText.hasPrefix("听到") })
        #expect(published.last == DictationUpdate(finalizedText: "部署 staging", volatileText: ""))
        #expect(stream.isClosed)
    }

    @Test("X-ASR alone, cancelled mid-utterance: nothing is inserted and the model is let go")
    func streamingEngineCancelled() async throws {
        let stream = CountingStream()
        let engine = XASRDictationEngine(load: { stream }, label: "X-ASR")
        try await engine.start()
        let updates = Task { await collect(engine) }
        engine.append(try buffer(frames: 16_000))
        try await Task.sleep(for: .milliseconds(200))
        engine.cancel()
        let published = await updates.value
        #expect(published.allSatisfy { $0.finalizedText.isEmpty })
        #expect(stream.isClosed)
    }
}
