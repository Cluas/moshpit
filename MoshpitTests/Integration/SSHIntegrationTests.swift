import Foundation
import Testing
@testable import Moshpit

/// End-to-end SSH validation against `localhost`. Gated on the environment
/// variable `MOSHPIT_SSH_LOCAL_KEY` pointing at an OpenSSH private key whose
/// public counterpart sits in `~/.ssh/authorized_keys`, and `MOSHPIT_SSH_USER`
/// supplying the username. If either is unset, the suite is skipped — so
/// CI / other developers don't fail to run unit tests just because they
/// lack a sshd setup.
///
/// Run prerequisites (one-time on the host):
///   - `sudo systemsetup -setremotelogin on` (or System Settings → Sharing → Remote Login)
///   - `ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519 -N ""` (if no key)
///   - `cat ~/.ssh/id_ed25519.pub >> ~/.ssh/authorized_keys`
///   - `ssh -i ~/.ssh/id_ed25519 <user>@localhost echo ok` must work
///
/// Then:
///   - `export MOSHPIT_SSH_USER=$(whoami)`
///   - `export MOSHPIT_SSH_LOCAL_KEY=$HOME/.ssh/id_ed25519`
///
/// before invoking `xcodebuild test ... -only-testing:MoshpitTests/SSHIntegration`.
@Suite("SSHIntegration", .tags(.integration), .serialized)
struct SSHIntegrationTests {
    static let username: String? = ProcessInfo.processInfo.environment["MOSHPIT_SSH_USER"]
    static let keyPath: String? = ProcessInfo.processInfo.environment["MOSHPIT_SSH_LOCAL_KEY"]

    /// Evaluated once (lazy `static let`) so the skip diagnostic prints a single
    /// time to the test log — otherwise a missing sshd setup skips these tests
    /// completely silently, which the OSS-readiness audit flagged as a hidden
    /// coverage gap with zero CI visibility.
    static let isConfigured: Bool = {
        let configured = username != nil && keyPath != nil
        if !configured {
            print("[SSHIntegration] SKIPPED: set MOSHPIT_SSH_USER and MOSHPIT_SSH_LOCAL_KEY to run the SSH integration suite (see file header for sshd setup).")
        }
        return configured
    }()

    /// Builds a `SSHService` for the test host. Loads the private key into a
    /// string (Citadel expects OpenSSH PEM text). Returns the service plus the
    /// matching `ServerConnection`; callers pass TOFU auto-accept handlers
    /// directly to `connect(...)` (they're per-call parameters now, not
    /// mutable state installed on the service — see `SSHService.connect`).
    private static func makeService() async throws -> (SSHService, ServerConnection, String) {
        let user = try #require(username, "MOSHPIT_SSH_USER not set")
        let path = try #require(keyPath, "MOSHPIT_SSH_LOCAL_KEY not set")
        let expanded = (path as NSString).expandingTildeInPath
        let pem = try String(contentsOfFile: expanded, encoding: .utf8)

        // Per-suite Keychain + HostKey state — but the integration test uses
        // overrideSecret + auto-trust handlers, so backing storage stays untouched.
        let keychain = KeychainService(backend: InMemoryKeychainBackend())
        let suiteName = "test.ssh.integration.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        let store = UserDefaultsKnownHostsStore(defaults: defaults, key: "moshpit.known_hosts")
        let validator = HostKeyValidator(store: store)

        let service = SSHService(
            keychain: keychain,
            hostKeyValidator: validator
        )

        let connection = ServerConnection(
            name: "integration-localhost",
            host: "localhost",
            port: 22,
            username: user,
            authMethod: .key,
            connectionProtocol: .ssh
        )

        return (service, connection, pem)
    }

    /// Waits up to `seconds` for the session to deliver a chunk whose string
    /// content contains `needle`. Returns true if found, false on timeout.
    private static func waitFor(_ needle: String,
                                in session: SSHSession,
                                seconds: TimeInterval) async -> Bool {
        let timeoutTask = Task { try? await Task.sleep(for: .seconds(seconds)) }
        var seen = ""
        for await chunk in session.dataStream {
            seen += String(decoding: chunk, as: UTF8.self)
            if seen.contains(needle) {
                timeoutTask.cancel()
                return true
            }
            if timeoutTask.isCancelled { return false }
        }
        return false
    }

    @Test("connect + PTY + echo round-trip + close",
          .enabled(if: isConfigured),
          .timeLimit(.minutes(1)))
    func endToEnd() async throws {
        let (service, connection, pem) = try await Self.makeService()
        let session = try await service.connect(
            connection, overrideSecret: pem,
            onUnknownHost: { _, _, _ in true },        // auto-trust localhost
            onChangedHost: { _, _, _, _ in false })    // never accept a changed key
        try await session.requestPTY(rows: 24, cols: 80)

        // Marker is unique so we don't match the prompt echo or motd.
        let marker = "moshi-int-\(UUID().uuidString.prefix(8))"
        try await session.writeText("printf '%s\\n' \(marker)\n")

        let saw = await Self.waitFor(String(marker), in: session, seconds: 10)
        #expect(saw, "session.dataStream should deliver the marker within 10s")

        try await session.resize(rows: 30, cols: 120)

        await session.close()
        let stillConnected = await session.isConnected
        #expect(!stillConnected, "session should report disconnected after close()")
    }
}

extension Tag {
    @Tag static var integration: Self
}
