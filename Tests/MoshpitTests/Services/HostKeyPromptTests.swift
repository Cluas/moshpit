import Foundation
import Testing
@testable import Moshpit

/// B132: the new-host and changed-key cards are answered with no connection
/// open and no clock running, and a connect that fails closes what it opened.
/// The card used to be answered inside Citadel's sign-in, which has 10 s for
/// everything: a user who read the fingerprint for longer got "can't reach the
/// server" while the Trust they then tapped was saved anyway, finishing a
/// sign-in on a connection nobody held. These run the app's real connect path
/// against an SSH server in the test process (`LoopbackSSHServer`).
@Suite("Host key questions outside the sign-in", .serialized)
struct HostKeyPromptTests {

    private static func connection(to server: LoopbackSSHServer) -> ServerConnection {
        ServerConnection(name: "t", host: "127.0.0.1", port: server.port, username: "me",
                         authMethod: .password, keychainRef: nil)
    }

    private static func service(_ validator: HostKeyValidator) -> SSHService {
        SSHService(keychain: .inMemory(), hostKeyValidator: validator)
    }

    @Test("Longer than the sign-in's 10 s over the new-host card, and the connect still goes through")
    func thinkingTimeDoesNotCount() async throws {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory()
        let notes = Notes()
        let session = try await Self.service(validator).connect(
            Self.connection(to: server), overrideSecret: "right",
            onUnknownHost: { _, _, fingerprint in
                notes.ask(fingerprint)
                // Nothing is held open while the user reads the card.
                notes.nothingOpen = await server.waitUntil(3) { $0.stillOpen == 0 }
                try? await Task.sleep(for: .seconds(11))
                return true
            })
        #expect(notes.asked == [server.fingerprint], "asked \(notes.asked)")
        #expect(notes.nothingOpen, "a connection was held open while the card was up")
        #expect(await validator.snapshot()["127.0.0.1:\(server.port)"] == server.fingerprint)
        #expect(await session.isConnected)
        await session.close()
        await server.stop()
    }

    @Test("A connect that fails closes the connection it opened")
    func failedConnectCloses() async throws {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory(seed: ["127.0.0.1:\(server.port)": server.fingerprint])
        do {
            _ = try await Self.service(validator).connect(Self.connection(to: server), overrideSecret: "wrong")
            Issue.record("signed in with the wrong password")
        } catch SSHError.authenticationFailed {
        }
        #expect(server.accepted == 1)
        #expect(await server.waitUntil(3) { $0.stillOpen == 0 }, "the refused connection was left open")
        await server.stop()
    }

    @Test("A declined host key is not trusted, and nothing is left open")
    func declinedHostKey() async throws {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory()
        do {
            _ = try await Self.service(validator).connect(Self.connection(to: server), overrideSecret: "right",
                                                         onUnknownHost: { _, _, _ in false })
            Issue.record("connected to a host the user didn't trust")
        } catch SSHError.hostKeyRejected {
        } catch {
            Issue.record("declining failed with \(error), not .hostKeyRejected")
        }
        #expect(await validator.snapshot().isEmpty)
        #expect(server.accepted == 1, "dialled again after a no")
        #expect(await server.waitUntil(3) { $0.stillOpen == 0 })
        await server.stop()
    }

    @Test("A changed key is asked about beside the one it replaces, then trusted")
    func changedKey() async throws {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory(seed: ["127.0.0.1:\(server.port)": "SHA256:the-old-key"])
        let notes = Notes()
        let session = try await Self.service(validator).connect(
            Self.connection(to: server), overrideSecret: "right",
            onUnknownHost: { _, _, _ in notes.ask("unknown?"); return false },
            onChangedHost: { _, _, new, old in notes.ask("\(old) → \(new)"); return true })
        #expect(notes.asked == ["SHA256:the-old-key → \(server.fingerprint)"])
        #expect(await validator.snapshot()["127.0.0.1:\(server.port)"] == server.fingerprint)
        await session.close()
        await server.stop()
    }

    @Test("A trusted host is one connection, and nobody is asked")
    func trustedHost() async throws {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory(seed: ["127.0.0.1:\(server.port)": server.fingerprint])
        let notes = Notes()
        let session = try await Self.service(validator).connect(
            Self.connection(to: server), overrideSecret: "right",
            onUnknownHost: { _, _, _ in notes.ask("unknown"); return true },
            onChangedHost: { _, _, _, _ in notes.ask("changed"); return true })
        #expect(notes.asked.isEmpty)
        #expect(server.accepted == 1)
        await session.close()
        await server.stop()
    }
}

private final class Notes: @unchecked Sendable {
    private let lock = NSLock()
    private var questions: [String] = []
    private var open = false
    var asked: [String] { lock.withLock { questions } }
    func ask(_ question: String) { lock.withLock { questions.append(question) } }
    var nothingOpen: Bool {
        get { lock.withLock { open } }
        set { lock.withLock { open = newValue } }
    }
}
