import AVFoundation
import Foundation
import Testing
@testable import Moshpit

/// A spoken "new line" never reaches a pane as Return (B672).
@Suite("Dictation line breaks")
@MainActor
struct DictationLineBreakTests {
    @Test("Line breaks, and the spaces around them, become one space")
    func folding() {
        #expect(DictationTranscript.singleLine("First line.\nSecond line") == "First line. Second line")
        #expect(DictationTranscript.singleLine("a \r\n\n b") == "a b")
        #expect(DictationTranscript.singleLine("第一行\u{2028}第二行") == "第一行 第二行")
        #expect(DictationTranscript.singleLine("git status") == "git status")
        #expect(DictationTranscript.singleLine("") == "")
    }

    @Test("Apple's dictation writes \"new line\" as a line break: the overlay shows, and Insert types, one line")
    func controllerTranscriptIsOneLine() async {
        typealias Fakes = VoiceDictationControllerTests
        let engine = Fakes.FakeEngine()
        let audio = Fakes.FakeAudioSource()
        let controller = VoiceDictationController(engineFactory: { _, _ in [engine] }, audioSource: audio)
        await controller.start(DictationRequest())
        // What DictationTranscriber (en-US) returned for "first line new line
        // second line", measured.
        engine.continuation.yield(DictationUpdate(finalizedText: "First line.\nSecond line",
                                                  volatileText: "and\nmore"))
        let deadline = ContinuousClock.now + .seconds(2)
        while controller.finalizedText.isEmpty, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
        #expect(controller.finalizedText == "First line. Second line")
        #expect(controller.volatileText == "and more")
        let text = await controller.finish()
        #expect(text == "First line. Second line and more")
        #expect(text?.contains("\n") == false)
    }
}
