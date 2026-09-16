import Foundation
import Testing
@testable import Moshpit

/// `TerminalViewModel.dialGeneration`: a dial disowned by `disconnect()` (the
/// redial path) must not land on top of its replacement, whichever way it
/// ends. Before this, a dial parked on an unanswered SYN for the whole
/// connect timeout came back as a stale `.failed` over a live session — or
/// as a second session adopted over the first.
@Suite("terminal view model — dial generations")
@MainActor
struct TerminalViewModelDialTests {

    private func connection() -> ServerConnection {
        ServerConnection(
            name: "dial",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
    }

    /// A view model with one dial disowned and a second one adopted.
    private struct Redialed {
        let viewModel: TerminalViewModel
        /// The disowned dial's `start()`, still waiting on its gate.
        let stale: Task<Void, Never>
        let adopted: SSHSession?
    }

    /// Starts a dial that hangs on `gate`, then disowns it and dials again
    /// over `fresh`.
    private func dialTwice(gate: DialGate, fresh: QuietTransport) async throws -> Redialed {
        let script = DialScript { _ in try await gate.wait().get() }
        let viewModel = TerminalViewModel(connection: connection(), dial: script.dialer)
        let stale = Task { await viewModel.start() }
        try await Task.sleep(for: .milliseconds(50))
        #expect(viewModel.status == .connecting)
        await viewModel.disconnect()
        viewModel.resetForReconnect()
        script.behavior = { connection in SSHSession(connection: connection, transport: fresh) }
        await viewModel.start()
        #expect(viewModel.status == .connected)
        #expect(script.dials == 2)
        return Redialed(viewModel: viewModel, stale: stale, adopted: viewModel.session)
    }

    @Test("a disowned dial that later succeeds is closed, not adopted")
    func staleSuccessIsClosed() async throws {
        let gate = DialGate()
        let fresh = QuietTransport()
        let late = QuietTransport()
        let redialed = try await dialTwice(gate: gate, fresh: fresh)

        gate.finish(.success(SSHSession(connection: connection(), transport: late)))
        await redialed.stale.value

        #expect(redialed.viewModel.status == .connected)
        #expect(redialed.viewModel.session === redialed.adopted, "the replacement's session stays")
        #expect(late.shutdowns == 1, "the late arrival is closed, not leaked")
        #expect(fresh.shutdowns == 0)
    }

    @Test("a disowned dial that later fails leaves the replacement alone")
    func staleFailureIsDropped() async throws {
        let gate = DialGate()
        let fresh = QuietTransport()
        let redialed = try await dialTwice(gate: gate, fresh: fresh)

        gate.finish(.failure(DialFailed()))
        await redialed.stale.value

        #expect(redialed.viewModel.status == .connected)
        #expect(redialed.viewModel.session === redialed.adopted)
        #expect(redialed.viewModel.errorMessage == nil, "a superseded dial's failure is not this attempt's")
    }
}
