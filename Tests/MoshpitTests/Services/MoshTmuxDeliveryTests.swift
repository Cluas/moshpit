import Foundation
import Network
import Testing
@testable import Moshpit

/// mosh+tmux: one pane's input on purpose — a notification's reply, a share —
/// before the `-CC` sidecar is up (the first second or two of every connect,
/// and while it is rebuilt).
///
/// The sidecar places a tmux pane's input by the pane's id. Without it the
/// reply went down the mosh shell, which is tmux's own client (or the login
/// shell before it attaches): it landed in tmux's ACTIVE pane — whichever the
/// desktop left focused — or ran as a command line, and was said sent. It
/// waits for the sidecar now, as crates/session does (`delivery_route`).
@Suite("mosh+tmux: a reply before the sidecar is up waits for it")
@MainActor
struct MoshTmuxDeliveryTests {

    /// The UDP side of the mosh transport: every datagram it sends.
    final class Channel: DatagramChannel, @unchecked Sendable {
        var state: NWConnection.State = .ready
        var onStateChange: (@Sendable (NWConnection.State) -> Void)?
        var onBetterPath: (@Sendable (Bool) -> Void)?
        private let lock = NSLock()
        private var _sent = 0
        var sent: Int { lock.withLock { _sent } }
        func start() {}
        func restart() {}
        func cancel() {}
        func send(_ datagram: Data) { lock.withLock { _sent += 1 } }
        func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {}
    }

    private func connection() -> ServerConnection {
        var connection = ServerConnection(
            name: "deliver",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .mosh
        connection.multiplexer = .tmux
        return connection
    }

    /// A live mosh+tmux session whose sidecar isn't up yet.
    private func session(_ channel: Channel) throws -> SessionHub.ActiveSession {
        let session = SessionHub.ActiveSession(connection: connection())
        let key = Data((0..<16).map { UInt8($0) })
        let mosh = try MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: channel)
        session.installForTesting(moshTransport: mosh)
        return session
    }

    /// The sidecar as it comes up: attached, its tree read a round trip
    /// later — one window, panes `%0` (active, the desktop's) and `%1`.
    private func sidecar() async -> (TmuxSessionController, MockTmuxTransport) {
        let transport = MockTmuxTransport()
        let control = TmuxSessionController(sshSession: transport, rendersOutput: false)
        await control.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        return (control, transport)
    }

    private func listTree(_ transport: MockTmuxTransport) {
        transport.pushText("""
        %begin 1 1 1
        $0 1 work
        %end 1 1 1
        %begin 2 2 1
        $0 @0 0 81x24,0,0,0 1 2 work
        %end 2 2 1
        %begin 3 3 1
        %0 @0 0 80 12 1 0 0 sh
        %1 @0 1 80 11 0 0 0 sh
        %end 3 3 1

        """)
    }

    private func waitUntil(_ done: () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !(await done()) {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("A reply sent before the sidecar is up goes to its pane by id once the sidecar lists it — never down mosh")
    func replyWaitsForTheSidecar() async throws {
        let channel = Channel()
        let session = try session(channel)
        let reply = Task { await session.deliverInput(Data("y\r".utf8), toPane: "%1") }
        // The sidecar comes up a moment later; its tree a round trip after.
        try await Task.sleep(for: .milliseconds(150))
        let (control, transport) = await sidecar()
        session.installForTesting(moshTransport: session.moshTransport, moshControl: control)
        #expect(await waitUntil { await transport.recordedCommands().count >= 3 })
        try await Task.sleep(for: .milliseconds(150))
        listTree(transport)
        #expect(await reply.value, "said not sent")
        #expect(channel.sent == 0, "the reply went down mosh, into tmux's active pane")
        let commands = await transport.recordedCommands()
        #expect(commands.contains { $0.hasPrefix("send-keys -t %1 -H 79 0d") }, "\(commands)")
        #expect(!commands.contains { $0.hasPrefix("send-keys -t %0") })
        await session.stop()
    }

    @Test("A reply whose sidecar never comes up is said not sent, and nothing goes down mosh")
    func noSidecarSendsNothing() async throws {
        let channel = Channel()
        let session = try session(channel)
        session.moshSidecarDeliveryWait = 0.4
        let began = ContinuousClock.now
        let sent = await session.deliverInput(Data("y\r".utf8), toPane: "%1")
        #expect(!sent, "reported sent with no sidecar to place it")
        #expect(ContinuousClock.now - began < .seconds(3))
        #expect(channel.sent == 0, "the reply went down mosh, into tmux's active pane")
        await session.stop()
    }

    @Test("A reply for a tmux pane on a host with no tmux is refused at once")
    func noTmuxRefusesAtOnce() async throws {
        let channel = Channel()
        let session = try session(channel)
        session.degrade = DegradeNotice.forMissing(.tmux)
        let began = ContinuousClock.now
        let sent = await session.deliverInput(Data("y\r".utf8), toPane: "%1")
        #expect(!sent)
        #expect(ContinuousClock.now - began < .seconds(1))
        #expect(channel.sent == 0, "typed into the plain shell")
        await session.stop()
    }

    @Test("A reply for a pane the sidecar doesn't list is said not sent, at once")
    func unknownPaneIsRefused() async throws {
        let channel = Channel()
        let session = try session(channel)
        let (control, transport) = await sidecar()
        session.installForTesting(moshTransport: session.moshTransport, moshControl: control)
        #expect(await waitUntil { await transport.recordedCommands().count >= 3 })
        listTree(transport)
        #expect(await waitUntil { control.snapshot.panes["%0"] != nil })
        let began = ContinuousClock.now
        let sent = await session.deliverInput(Data("y\r".utf8), toPane: "%9")
        #expect(!sent)
        #expect(ContinuousClock.now - began < .seconds(1))
        #expect(channel.sent == 0)
        await session.stop()
    }
}
