import Foundation
import Testing

@testable import Moshpit

/// The troubleshooting page promises "plain language, not stderr" — that the
/// messages say something actionable, whatever error is underneath. These
/// tests hold that promise to account.
///
/// It was not being kept. Pointing a screenshot run at an unroutable address
/// produced an alert reading:
///
///     SSH error: The operation couldn't be completed.
///     (Citadel.ClientHandshakeHandler.(unknown context at $105f91dd4).
///     (unknown context at $105f91de0).Disconnected error 1.)
///
/// — pointer addresses and an internal type name, shown to a user, on the one
/// path a new user is most likely to hit first. The Rust session's failures
/// read their words from the same catalog (``RustCopy``).
@Suite("SSH error words")
struct SSHErrorMappingTests {

    /// Stands in for a library error that doesn't implement LocalizedError.
    private struct OpaqueError: Error, CustomStringConvertible {
        let description: String
    }

    @Test("known connection failures keep their own wording")
    func knownCasesKeepTheirWording() {
        #expect(SSHError.connectionFailed.description.contains("Couldn't reach"))
        #expect(SSHError.authenticationFailed.description.contains("Authentication failed"))
        #expect(SSHError.handshakeFailed.description.contains("closed the connection"))
    }

    /// B151: the user's own Cancel on the new-host card reached the alert as
    /// "The connection failed for a reason Moshpit didn't recognise…".
    @Test("a declined host key is its own words, not an unrecognised failure")
    func declinedHostKey() {
        let declined = SSHError.hostKeyRejected.description
        #expect(declined != SSHError.underlying(OpaqueError(description: "x")).description)
        #expect(declined.contains("trust"))
    }

    @Test("the detail survives for logs even when the alert stays plain")
    func diagnosticKeepsTheDetail() {
        let raw = OpaqueError(description: "ClientHandshakeHandler $105f91dd4")
        let wrapped = SSHError.underlying(raw)
        #expect(!wrapped.description.contains("ClientHandshakeHandler"))
        #expect(wrapped.diagnostic.contains("ClientHandshakeHandler"))
        // NSError bridging gets the sentence, not "SSHError error 1."
        #expect((wrapped as NSError).localizedDescription == wrapped.description)
    }

    /// Anything that reaches the user must be a sentence, not a dump: every
    /// case, whatever it wraps.
    @Test("every message is a sentence that exposes no internals")
    func messagesArePresentable() {
        let opaque = OpaqueError(description:
            "Citadel.ClientHandshakeHandler.(unknown context at $105f91dd4).Disconnected error 1.")
        let all: [SSHError] = [
            .missingKeychainRef, .sessionClosed, .authenticationFailed, .authMethodRejected,
            .channelOpenFailed, .connectionFailed, .handshakeFailed, .credentialUnavailable,
            .credentialCancelled, .credentialLocked, .hostKeysLocked, .hostKeyRejected,
            .proxyUnreachable("127.0.0.1:1080"), .proxyFailed, .keyNeedsPassphrase,
            .keyUnusable(.wrongPassphrase), .underlying(opaque),
        ]
        for error in all {
            let message = error.description
            #expect(!message.isEmpty, "\(error.diagnostic): no words")
            #expect(!message.contains("$"), "leaks a pointer: \(message)")
            #expect(!message.lowercased().contains("unknown context"), "leaks internals: \(message)")
        }
    }
}
