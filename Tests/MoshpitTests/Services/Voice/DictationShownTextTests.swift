import Foundation
import Testing
@testable import Moshpit

/// The overlay's pieces, side by side, are what Insert types (B673).
@Suite("Dictation shown text")
struct DictationShownTextTests {
    @Test("The guess carries the join's space, and only it")
    func pieces() {
        for (finalized, volatile) in [("帮我把这个", "PR merge 一下"), ("deploy the", "staging branch"),
                                      ("你好", "世界"), ("wait,", "go"), ("hello ", "world"),
                                      ("", "only a guess"), ("only settled", "")] {
            let shown = DictationTranscript.shownPieces(finalized, volatile)
            #expect(shown.finalized == finalized)
            #expect(shown.finalized + shown.volatile == DictationTranscript.join(finalized, volatile),
                    "\(finalized) | \(volatile)")
        }
        #expect(DictationTranscript.shownPieces("帮我把这个", "PR").volatile == "PR")
        #expect(DictationTranscript.shownPieces("deploy the", "branch").volatile == " branch")
    }

    @Test("The panel's message is the shown pieces")
    @MainActor
    func panelMessage() async {
        typealias Fakes = VoiceDictationControllerTests
        let engine = Fakes.FakeEngine()
        let controller = VoiceDictationController(engineFactory: { _, _ in [engine] },
                                                  audioSource: Fakes.FakeAudioSource())
        await controller.start(DictationRequest())
        engine.continuation.yield(DictationUpdate(finalizedText: "deploy the", volatileText: "staging"))
        let deadline = ContinuousClock.now + .seconds(2)
        while controller.volatileText.isEmpty, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
        let panel = TerminalNSHPanel.dictation(controller).object
        let finalized = panel["finalized"] as? String ?? ""
        let volatile = panel["volatile"] as? String ?? ""
        #expect(volatile == " staging")
        #expect(finalized + volatile == controller.transcript)
        controller.cancel()
    }
}
