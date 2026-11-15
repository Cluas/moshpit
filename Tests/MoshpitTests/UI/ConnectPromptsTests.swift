import Foundation
import SwiftUI
import Testing
@testable import Moshpit

/// E6c: what a connect asks on its way in, off the SwiftUI cards. The host
/// key goes to its NSH sheet whole (`ui/tests/host_key.rs` holds what the
/// sheet says and that no key trusts), its answer comes back once, and a
/// swipe down declines only the prompt still asked; the passphrase and the
/// sign-in question keep the cards' words.
@Suite("Connect prompts")
@MainActor
struct ConnectPromptsTests {
    /// A host-key prompt whose answers land in `answers`.
    private func prompt(previous: String? = nil, older: String? = nil,
                        answers: Answers) -> TerminalViewModel.HostKeyPrompt {
        TerminalViewModel.HostKeyPrompt(
            host: "build-box", port: 2222, fingerprint: "SHA256:k3VJ", previousFingerprint: previous,
            olderAlgorithm: older) { answers.list.append($0) }
    }

    final class Answers { var list: [Bool] = [] }

    private func fields(_ json: String) throws -> [String: Any] {
        try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    @Test("the sheet is told the prompt whole: a new host, a changed key, the older algorithm")
    func message() throws {
        let answers = Answers()
        let new = try fields(HostKeyNSHPage.message(prompt(answers: answers)))
        #expect(new["op"] as? String == "hostKey")
        #expect(new["host"] as? String == "build-box")
        #expect(new["port"] as? Int == 2222)
        #expect(new["fingerprint"] as? String == "SHA256:k3VJ")
        #expect(new["previous"] == nil)
        #expect(new["olderAlgorithm"] == nil)

        let changed = try fields(HostKeyNSHPage.message(prompt(previous: "SHA256:old", answers: answers)))
        #expect(changed["previous"] as? String == "SHA256:old")

        let older = try fields(HostKeyNSHPage.message(prompt(older: "ssh-rsa", answers: answers)))
        #expect(older["olderAlgorithm"] as? String == "ssh-rsa")
    }

    @Test("trust and decline are the only answers the sheet gives")
    func decisions() {
        #expect(HostKeyNSHPage.decision(["ev": "trust"]) == true)
        #expect(HostKeyNSHPage.decision(["ev": "decline"]) == false)
        #expect(HostKeyNSHPage.decision(["ev": "failed", "component": "x", "message": "y"]) == nil)
        #expect(HostKeyNSHPage.decision(["ev": "retry"]) == nil)
        #expect(HostKeyNSHPage.decision([:]) == nil)
    }

    @Test("a swipe down declines the prompt the sheet shows, and nothing else")
    func swipeDeclines() {
        let answers = Answers()
        let asked = prompt(answers: answers)
        var current: TerminalViewModel.HostKeyPrompt? = asked

        // Swiped away while asked: declined, once.
        HostKeyNSHPage.binding { current }.wrappedValue = nil
        #expect(answers.list == [false])

        // Answered first (its decide clears it): the sheet leaving says
        // nothing more.
        answers.list = []
        let binding = HostKeyNSHPage.binding { current }
        current = nil
        binding.wrappedValue = nil
        #expect(answers.list.isEmpty)

        // Another prompt came while the sheet left: it isn't the one shown.
        current = asked
        let shown = HostKeyNSHPage.binding { current }
        let other = prompt(answers: answers)
        current = other
        shown.wrappedValue = nil
        #expect(answers.list.isEmpty)
    }

    @Test("the passphrase and the server's question keep the cards' words")
    func words() {
        let first = TerminalViewModel.KeyPassphrasePrompt(host: "build-box", wrong: false) { _ in }
        #expect(first.message == String(localized: "The key for build-box is locked with a passphrase. Enter it once: from then on the Keychain on this device protects the key, as it does every key you import."))
        let wrong = TerminalViewModel.KeyPassphrasePrompt(host: "build-box", wrong: true) { _ in }
        #expect(wrong.message == String(localized: "That passphrase doesn't unlock this key."))
        let question = TerminalViewModel.SignInQuestion(host: "build-box", words: "Verification code:",
                                                        echo: false) { _ in }
        #expect(question.title == String(localized: "Sign In to build-box"))
    }

    @Test("the host-key screen is NSH's")
    func screen() {
        #expect(NSHScreen.hostKey.name == "host-key")
    }
}
