import AVFoundation
import Foundation
import Testing
@testable import Moshpit

/// X-ASR's words are a preview on the panel, never keystrokes: one dictation
/// writes to the session once, the final text, and a cancelled one writes
/// nothing (B763).
///
/// The terminal's only write is `finishDictation`: it types what
/// `VoiceDictationController.finish(submit:)` returns. `session` below does
/// exactly that, so counting its writes counts the session's.
@Suite("Dictation writes once")
@MainActor
struct DictationSingleWriteTests {
    typealias Fakes = VoiceDictationControllerTests
    typealias Engines = WhisperDictationEngineTests

    /// What the terminal typed.
    @MainActor
    final class Session {
        private(set) var writes: [String] = []
        /// `finishDictation`'s body: type the commit, if there is one.
        func finish(_ controller: VoiceDictationController, submit: Bool = false) async {
            if let commit = await controller.finish(submit: submit) { writes.append(commit.text) }
        }
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(3)
        while ContinuousClock.now < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return condition()
    }

    private func buffer() throws -> AVAudioPCMBuffer {
        let format = try #require(AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 16_000))
        buffer.frameLength = 16_000
        return buffer
    }

    /// A Qwen3-ASR dictation previewed by X-ASR, listening, with the preview
    /// on the panel.
    private func listening(final: String) async throws
        -> (VoiceDictationController, Engines.CountingStream, Fakes.FakeAudioSource) {
        let stream = Engines.CountingStream()
        let store = Engines.ScriptedTranscriber(answers: [final])
        let audio = Fakes.FakeAudioSource()
        let controller = VoiceDictationController(engineFactory: { _, _ in
            [WhisperDictationEngine(store: store, variant: Qwen3ASR.variant, language: nil,
                                    label: "Qwen3-ASR", streaming: { stream })]
        }, audioSource: audio)
        await controller.start(DictationRequest())
        for _ in 0 ..< 2 { audio.onBuffer?(try buffer()) }
        #expect(await waitUntil(controller.volatileText.hasPrefix("听到")))
        return (controller, stream, audio)
    }

    @Test("One dictation types once: the final pass's text, not the preview's, however often Insert is tapped")
    func oneWrite() async throws {
        let (controller, stream, _) = try await listening(final: "部署 staging 分支。")
        let session = Session()
        // Insert, and Send while the final pass still runs.
        async let first: Void = session.finish(controller)
        async let second: Void = session.finish(controller, submit: true)
        _ = await (first, second)
        #expect(session.writes == ["部署 staging 分支。"])
        #expect(stream.isClosed)
    }

    @Test("A dictation cancelled mid-utterance types nothing, then or later")
    func cancelWritesNothing() async throws {
        let (controller, stream, _) = try await listening(final: "never typed")
        let session = Session()
        controller.cancel()
        await session.finish(controller)
        #expect(session.writes.isEmpty)
        #expect(await waitUntil(stream.isClosed))
    }

    @Test("X-ASR that won't load fails on the panel, the mic off, and types nothing (B862)")
    func loadFailureTypesNothing() async throws {
        struct Corrupt: Error {}
        let audio = Fakes.FakeAudioSource()
        let controller = VoiceDictationController(engineFactory: { _, _ in
            [XASRDictationEngine(load: { throw Corrupt() }, label: "X-ASR", isInstalled: { true })]
        }, audioSource: audio)
        await controller.start(DictationRequest())
        for _ in 0 ..< 3 { audio.onBuffer?(try buffer()) }
        #expect(await waitUntil(controller.phase != .listening), "listening for good, words never coming")
        if case .failed = controller.phase {} else { Issue.record("phase \(controller.phase)") }
        #expect(audio.stopped)
        let session = Session()
        await session.finish(controller)
        #expect(session.writes.isEmpty)
    }
}
