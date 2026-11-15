import AVFoundation
import Foundation
import Testing
@testable import Moshpit

/// Send presses Return only on the words that were on screen (B676).
@Suite("Dictation send")
@MainActor
struct DictationSendTests {
    typealias Fakes = VoiceDictationControllerTests

    private func waitUntil(_ condition: @autoclosure () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(2)
        while ContinuousClock.now < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return condition()
    }

    /// Shows `shown`, taps Send (or Insert), and has the engine's final pass
    /// answer `final` before it finishes.
    private func send(shown: String, final: String, submit: Bool = true) async -> VoiceDictationController.Commit? {
        let engine = Fakes.FakeEngine()
        engine.finishAudioHangs = true
        let controller = VoiceDictationController(engineFactory: { _, _ in [engine] },
                                                  audioSource: Fakes.FakeAudioSource())
        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "", volatileText: shown))
        _ = await waitUntil(controller.transcript == shown)
        let finishing = Task { await controller.finish(submit: submit) }
        _ = await waitUntil(engine.finishAudioCalled)
        engine.continuation.yield(DictationUpdate(finalizedText: final, volatileText: ""))
        _ = await waitUntil(controller.finalizedText == final)
        engine.releaseFinish()
        return await finishing.value
    }

    @Test("Words the final pass added after the tap: typed, not sent (B676)")
    func wordsChanged() async {
        // Whisper's preview a second behind: the tap saw half the sentence.
        let commit = await send(shown: "rm -rf build", final: "rm -rf build and the cache")
        #expect(commit == .init(text: "rm -rf build and the cache", submit: false))
    }

    @Test("A different file named by the final pass: typed, not sent")
    func fileChanged() async {
        let commit = await send(shown: "rm -rf /tmp/build", final: "rm -rf /")
        #expect(commit?.submit == false)
    }

    @Test("Only case and a full stop changed: sent")
    func punctuationOnly() async {
        let commit = await send(shown: "git status", final: "Git status.")
        #expect(commit == .init(text: "Git status.", submit: true))
    }

    @Test("Insert never sends")
    func insertNeverSends() async {
        let commit = await send(shown: "git status", final: "git status", submit: false)
        #expect(commit == .init(text: "git status", submit: false))
    }

    @Test("What counts as the same words")
    func sameWords() {
        #expect(DictationTranscript.sameWords("git status", "Git status."))
        #expect(DictationTranscript.sameWords("帮我把这个 PR merge 一下", "帮我把这个PR merge一下。"))
        #expect(!DictationTranscript.sameWords("git checkout -b login", "git checkout b login"))
        #expect(!DictationTranscript.sameWords("rm -rf /tmp", "rm -rf /"))
        #expect(!DictationTranscript.sameWords("deploy the", "deploy the staging branch"))
    }
}
