import Foundation
import Testing

@testable import Moshpit

/// The troubleshooting page promises "plain language, not stderr" — that the
/// messages are rewritten from the raw library errors so they say something
/// actionable. These tests hold that promise to account.
///
/// It was not being kept. Pointing a screenshot run at an unroutable address
/// produced an alert reading:
///
///     SSH error: The operation couldn't be completed.
///     (Citadel.ClientHandshakeHandler.(unknown context at $105f91dd4).
///     (unknown context at $105f91de0).Disconnected error 1.)
///
/// — pointer addresses and an internal type name, shown to a user, on the one
/// path a new user is most likely to hit first.
@Suite("SSH error mapping")
struct SSHErrorMappingTests {

    /// Stands in for a Citadel/NIO error, which cannot be constructed here.
    /// The classifier reads `String(describing:)`, so reproducing the text is
    /// what matters.
    private struct OpaqueError: Error, CustomStringConvertible {
        let description: String
    }

    @Test("a handshake that drops is explained, not dumped")
    func handshakeDisconnect() {
        let raw = OpaqueError(description:
            "Citadel.ClientHandshakeHandler.(unknown context at $105f91dd4)."
            + "(unknown context at $105f91de0).Disconnected error 1.")
        let mapped = SSHError.map(raw)

        #expect(mapped.isHandshakeFailed)
        let message = mapped.description
        #expect(!message.contains("Citadel"))
        #expect(!message.contains("$"))
        #expect(!message.contains("unknown context"))
    }

    @Test("known connection failures keep their own wording")
    func knownCasesKeepTheirWording() {
        #expect(SSHError.connectionFailed.description.contains("Couldn't reach"))
        #expect(SSHError.authenticationFailed.description.contains("Authentication failed"))
        #expect(SSHError.handshakeFailed.description.contains("closed the connection"))
    }

    @Test("the detail survives for logs even when the alert stays plain")
    func diagnosticKeepsTheDetail() {
        let raw = OpaqueError(description: "Citadel.ClientHandshakeHandler $105f91dd4")
        let mapped = SSHError.underlying(raw)
        #expect(!mapped.description.contains("Citadel"))
        #expect(mapped.diagnostic.contains("Citadel"))
    }

    /// Anything that reaches the user must be a sentence, not a dump. This is
    /// the guard that would have caught the original defect: it does not care
    /// which case an error maps to, only that what is shown is presentable.
    @Test("no mapped message exposes internals")
    func messagesArePresentable() {
        let samples = [
            "Citadel.ClientHandshakeHandler.(unknown context at $105f91dd4).Disconnected error 1.",
            "Connection refused",
            "NIOSSH.SSHClientError.allAuthenticationOptionsFailed",
            "channelCreationFailed",
        ]
        for text in samples {
            let message = SSHError.map(OpaqueError(description: text)).description
            #expect(!message.contains("$"), "leaks a pointer: \(message)")
            #expect(!message.lowercased().contains("unknown context"),
                    "leaks internals: \(message)")
        }
    }
}

private extension SSHError {
    var isHandshakeFailed: Bool {
        if case .handshakeFailed = self { return true }
        return false
    }
    var isConnectionFailed: Bool {
        if case .connectionFailed = self { return true }
        return false
    }
}
