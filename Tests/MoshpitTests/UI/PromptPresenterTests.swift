import Foundation
import Testing
import UIKit
import UserNotifications

@testable import Moshpit

/// B632: what a connect asks the person is the app's to present, over
/// whatever is frontmost — not the page's that started the session. Home
/// presented only the sessions it had started, until their first connect,
/// and the terminal only its own: a reconnect's question with the person on
/// Home and that terminal closed had nobody to present it, and the dial
/// waited on a question nobody showed.
@MainActor
@Suite("The app's prompt presenter")
struct PromptPresenterTests {
    private func hub() -> SessionHub {
        SessionHub(metrics: SessionMetricsRegistry())
    }

    @Test("a reconnect's sign-in question is presented with no page asking for it")
    func reconnectQuestion() {
        let hub = hub()
        // A live session, its terminal closed, Home on screen: no page
        // holds it. Its reconnect asks the OTP (B563).
        let session = hub.prepare(ServerConnection(host: "box", username: "u"))
        let center = PromptCenter(hub: hub)
        #expect(!center.pending)
        session.viewModel.signInQuestion = TerminalViewModel.SignInQuestion(
            host: "box", words: "Verification code:", echo: false) { _ in }
        #expect(center.pending)
        #expect(center.question?.words == "Verification code:")
    }

    @Test("a changed host key with no page open is presented, and its notice says it changed")
    func changedHostKey() {
        let hub = hub()
        let session = hub.prepare(ServerConnection(host: "box", username: "u"))
        let center = PromptCenter(hub: hub)
        session.viewModel.hostKeyPrompt = TerminalViewModel.HostKeyPrompt(
            host: "box", port: 22, fingerprint: "SHA256:new", previousFingerprint: "SHA256:old") { _ in }
        #expect(center.hostKey?.previousFingerprint == "SHA256:old")
        #expect(center.pending)
        session.viewModel.hostKeyPrompt = nil
        #expect(!center.pending)
    }

    @Test("what the background notification says, by kind")
    func noticeWords() {
        let id = UUID()
        #expect(PromptNotice.hostKey(connection: id, host: "box", changed: true).body
                == String(localized: "\("box")'s host key has changed. Open Moshpit to decide."))
        #expect(PromptNotice.hostKey(connection: id, host: "box", changed: false).body
                == String(localized: "Confirm \("box")'s host key to finish connecting."))
        #expect(PromptNotice.passphrase(connection: id, host: "box").body
                == String(localized: "Enter the passphrase of the key for \("box") to finish connecting."))
        #expect(PromptNotice.question(connection: id, host: "box").body
                == String(localized: "\("box") asks a question before you can sign in. Open Moshpit to answer."))
        #expect(PromptNotice.question(connection: id, host: "box").id == PromptNotice.id(id, .question))
    }

    @Test("one question of each kind at a time, in the connections' order")
    func oneAtATime() throws {
        let first = try #require(UUID(uuidString: "00000000-0000-4000-8000-000000000001"))
        let second = try #require(UUID(uuidString: "00000000-0000-4000-8000-000000000002"))
        var a = ServerConnection(host: "a", username: "u")
        a.id = first
        var b = ServerConnection(host: "b", username: "u")
        b.id = second
        let sessions = [SessionHub.ActiveSession(connection: b), SessionHub.ActiveSession(connection: a)]
        let center = PromptCenter(sessions: { sessions })
        for session in sessions {
            session.viewModel.signInQuestion = TerminalViewModel.SignInQuestion(
                host: session.connection.host, words: "Code:", echo: false) { _ in }
        }
        #expect(center.question?.host == "a")
        sessions[1].viewModel.signInQuestion = nil
        #expect(center.question?.host == "b", "the next waits behind the first")
    }

    /// A question arriving under another sheet (Install Assist's reconnect):
    /// the presenter's window is its own, above the app's window and every
    /// sheet presented in it, so what it presents is never covered.
    @Test("the presenter's window is above the app's, and its sheets")
    func aboveSheets() {
        #expect(PromptWindowController.level > UIWindow.Level.normal)
        #expect(PromptWindowController.level < UIWindow.Level.alert)
        let window = PromptWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
        let root = UIViewController()
        window.rootViewController = root
        window.isHidden = false
        // Its empty background takes no touch: the app under it does.
        #expect(window.hitTest(CGPoint(x: 50, y: 50), with: nil) == nil)
        window.isHidden = true
    }

    @Test("notices: posted in the background, once; taken back when answered or in the foreground")
    func notices() {
        var background = false
        var added: [String] = []
        var removed: [String] = []
        let notifier = PromptNotifier(background: { background },
                                      add: { added.append($0.identifier) },
                                      remove: { removed += $0 })
        let otp = PromptNotice.question(connection: UUID(), host: "a")
        let key = PromptNotice.hostKey(connection: UUID(), host: "b", changed: true)

        notifier.asked(otp)
        #expect(added.isEmpty, "the app shows it itself")
        background = true
        notifier.asked(otp)
        notifier.asked(otp)
        #expect(added == [otp.id])
        notifier.answered(otp.id)
        notifier.answered(otp.id)
        #expect(removed == [otp.id], "answered or withdrawn, once")
        notifier.asked(key)
        notifier.foreground()
        #expect(added == [otp.id, key.id])
        #expect(removed == [otp.id, key.id], "the app came forward: it shows the question")
        #expect(notifier.postedIds.isEmpty)
    }
}
