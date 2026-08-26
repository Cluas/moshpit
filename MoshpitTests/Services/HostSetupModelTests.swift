import Foundation
import Testing
@testable import Moshpit

/// The state machine behind the host-setup screen.
///
/// Worth testing precisely because its predecessor's state machine was where the
/// damage lived: it had no representation for "installed but out of date", its
/// verification resolved on evidence it had not asked for, and a failed install
/// rendered identically to a successful one. Each of those is a test here.
@MainActor
@Suite("Host setup model")
struct HostSetupModelTests {

    final class FakePush: PushCoordinating {
        var lastRelayError: String?
        var paired: [UUID] = []
        var unpaired: [UUID] = []
        var pairError: Error?
        var selfTestArrives = true
        var awaited: [String] = []

        func pair(connectionId: UUID, hostLabel: String, relayURL: String) async throws -> PushPairing {
            if let pairError { throw pairError }
            paired.append(connectionId)
            return PushPairing.make(connectionId: connectionId, hostLabel: hostLabel,
                                    relayURL: relayURL)
        }
        func unpair(connectionId: UUID) { unpaired.append(connectionId) }
        func awaitSelfTest(nonce: String, timeout: Duration) async -> Bool {
            awaited.append(nonce)
            return selfTestArrives
        }
        func forgetSelfTest(nonce: String) {}
    }

    static func connection() -> ServerConnection {
        ServerConnection(id: UUID(), name: "m1-pro", host: "10.0.0.2", port: 22,
                         username: "cluas", authMethod: .key, connectionProtocol: .ssh,
                         sshPort: 22, keychainRef: nil)
    }

    static func model(_ channel: RecordingChannel,
                      push: FakePush,
                      connection conn: ServerConnection? = nil) -> (HostSetupModel, FakePush) {
        let model = HostSetupModel(connection: conn ?? connection(), push: push) {
            HostInstaller(channel: channel)
        }
        return (model, push)
    }

    static let allTools = HostInstallerTests.allTools

    // MARK: - Reading

    @Test("inspect reports what the host can do")
    func inspect() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools)]
        let (model, _) = Self.model(channel, push: FakePush())
        await model.inspect()

        #expect(model.phase == .idle)
        #expect(model.state?.facts.canPush == true)
        #expect(model.error == nil)
        #expect(model.status(model.pairingComponent) == .absent)
    }

    @Test("a channel that cannot be opened is an error on screen, not a spinner forever")
    func inspectFailure() async throws {
        let channel = RecordingChannel()
        channel.failOn = "printf"
        let (model, _) = Self.model(channel, push: FakePush())
        await model.inspect()
        #expect(model.phase == .idle)
        #expect(model.error != nil)
    }

    // MARK: - Installing

    @Test("a failed install says why, and does not read as success")
    func failedInstallSurfaces() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools),
                         ("cat \"$HOME/.claude/settings.json\"", "{broken")]
        let (model, _) = Self.model(channel, push: FakePush())
        await model.inspect()
        await model.installHooks()

        #expect(model.error != nil)
        #expect(model.error?.contains("valid JSON") == true)
        #expect(model.status(.hooks(agent: "claude")) == .absent)
    }

    @Test("a host missing tools is refused with the tools named")
    func missingToolsMessage() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", "home=/h\ncurl=no\nopenssl=no\nbase64=yes")]
        let (model, push) = Self.model(channel, push: FakePush())
        await model.inspect()
        await model.pair(relayURL: "https://push.example.org")

        #expect(model.error?.contains("openssl") == true)
        #expect(model.error?.contains("curl") == true)
        // Registered with the relay first, so nothing was left half-done on a
        // host that could never have used it.
        #expect(push.paired.count == 1)
        #expect(channel.sent(containing: "push.conf\"; chmod") == nil)
    }

    @Test("an empty relay address is caught before anything is attempted")
    func emptyRelay() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools)]
        let (model, push) = Self.model(channel, push: FakePush())
        await model.pair(relayURL: "   ")

        #expect(model.error?.contains("push relay") == true)
        #expect(push.paired.isEmpty)
        #expect(channel.commands.isEmpty)
    }

    // MARK: - Proving

    @Test("pairing proves itself with a real push, and says so")
    func pairAndProve() async throws {
        let channel = HostInstallerTests.healthyChannel()
        let (model, push) = Self.model(channel, push: FakePush())
        await model.inspect()
        await model.pair(relayURL: "https://push.example.org")

        #expect(model.error == nil)
        #expect(push.paired.count == 1)
        #expect(push.awaited.count == 1, "pairing must prove itself, not just claim to")
        if case .proven = model.pushProof {} else {
            Issue.record("expected proven, got \(model.pushProof)")
        }
    }

    @Test("a push that never arrives is a failure with an actionable reason")
    func pushNeverArrives() async throws {
        let channel = HostInstallerTests.healthyChannel()
        let push = FakePush()
        push.selfTestArrives = false
        let (model, _) = Self.model(channel, push: push)
        await model.inspect()
        await model.provePush()

        guard case .failed(let why) = model.pushProof else {
            Issue.record("expected failure, got \(model.pushProof)")
            return
        }
        // Not "failed", and not a substring of one phrasing either — what this
        // pins is that the message leads with the LIKELIEST cause. On the first
        // real-device run the notification arrived perfectly on a lock screen and
        // this screen still reported failure, because the proof comes back
        // through willPresent and iOS only calls that in the foreground. A
        // message that opened with "check your permissions" would have sent
        // someone to dismantle a pairing that worked.
        #expect(why.lowercased().contains("foreground"))
        #expect(why.lowercased().contains("notifications"), "the fallbacks still get a mention")
    }

    @Test("every self-test uses a fresh nonce, so a stale push proves nothing")
    func nonceIsFresh() async throws {
        let channel = HostInstallerTests.healthyChannel()
        let (model, push) = Self.model(channel, push: FakePush())
        await model.provePush()
        await model.provePush()
        #expect(Set(push.awaited).count == 2)
    }

    @Test("the hook test needs a pane, and says so instead of failing")
    func hooksNeedAPane() async throws {
        let channel = RecordingChannel()
        let (model, _) = Self.model(channel, push: FakePush())
        await model.proveHooks(pane: nil)

        guard case .unavailable(let why) = model.hooksProof else {
            Issue.record("expected unavailable, got \(model.hooksProof)")
            return
        }
        #expect(why.contains("tmux pane"))
        #expect(channel.commands.isEmpty)
    }

    @Test("a host whose tmux is elsewhere is told the install is still fine")
    func hooksProofUnavailable() async throws {
        let channel = RecordingChannel()   // socket probe answers empty
        let (model, _) = Self.model(channel, push: FakePush())
        await model.proveHooks(pane: "%3")

        guard case .unavailable(let why) = model.hooksProof else {
            Issue.record("expected unavailable, got \(model.hooksProof)")
            return
        }
        #expect(why.contains("hooks themselves are unaffected"))
    }

    @Test("firing the stamp and reading it back is what counts as proof")
    func hooksProven() async throws {
        let channel = RecordingChannel()
        channel.rules = [("socket_path", "/tmp/tmux-501/default"),
                         ("display-message -p -t", "working|moshpit-selftest|")]
        let (model, _) = Self.model(channel, push: FakePush())
        await model.proveHooks(pane: "%3")
        if case .proven = model.hooksProof {} else {
            Issue.record("expected proven, got \(model.hooksProof)")
        }
    }

    @Test("a phone the relay does not know is said so, even when the host is perfect")
    func relayErrorSurfaces() async throws {
        let channel = HostInstallerTests.healthyChannel()
        let push = FakePush()
        push.lastRelayError = "Relay rejected the registration (401)"
        let (model, _) = Self.model(channel, push: push)
        await model.inspect()

        // Everything on the HOST is fine; the failure is on this side, and the
        // pairing row reads the host's manifest so it cannot see it. Before this
        // was wired up, four writers set that value and nothing ever read it.
        #expect(model.relayError == "Relay rejected the registration (401)")
    }

    // MARK: - The state the old design had no name for

    @Test("paired with a stale stamp script is surfaced, not inferred")
    func pairedButNothingWillPush() async throws {
        let conn = Self.connection()
        var manifest = InstallManifest.empty
        manifest[.pairing(conn: conn.id.uuidString)] = .init(
            installedAt: Date(), connectionId: conn.id.uuidString, relayURL: "r")
        manifest[.stamp] = .init(digest: "from-an-older-build", installedAt: Date())
        let json = try JSONEncoder.moshpitInstall.encode(manifest)

        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools),
                         ("manifest.json", String(data: json, encoding: .utf8)!)]
        let (model, _) = Self.model(channel, push: FakePush(), connection: conn)
        await model.inspect()

        #expect(model.status(.stamp) == .stale(installed: "from-an-older-build"))
        #expect(model.pairedButNothingWillPush)
    }

    @Test("unpairing forgets the host on both sides")
    func unpair() async throws {
        let conn = Self.connection()
        var manifest = InstallManifest.empty
        manifest[.pairing(conn: conn.id.uuidString)] = .init(
            installedAt: Date(), connectionId: conn.id.uuidString, relayURL: "r")
        let json = try JSONEncoder.moshpitInstall.encode(manifest)
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools),
                         ("manifest.json", String(data: json, encoding: .utf8)!)]
        let (model, push) = Self.model(channel, push: FakePush(), connection: conn)
        await model.inspect()
        await model.unpair()

        #expect(push.unpaired.count == 1, "the phone must drop its copy of the secret too")
        #expect(channel.sent(containing: "rm -f")?.contains("push.d") == true,
                "unpair must remove THIS device's push.d file")
        #expect(model.pushProof == .untested)
    }
}
