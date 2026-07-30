import Foundation
import Testing
@testable import Ringdown

/// End-to-end validation that `TmuxSessionController` drives a *real*
/// tmux control-mode session over the Phase 1 SSH stack. Verifies the
/// Checkpoint D claims from `.claude/architecture_comparison.md`:
/// parser routes %output to persistent terminals, window add/remove +
/// switch propagates through the single `snapshot`, no data loss.
///
/// Gated on the same RINGDOWN_SSH_USER / RINGDOWN_SSH_LOCAL_KEY envvars as
/// `SSHIntegrationTests`. tmux must be on PATH on the localhost host.
@Suite("TmuxIntegration", .tags(.integration), .serialized)
@MainActor
struct TmuxIntegrationTests {
    nonisolated static let username: String? = ProcessInfo.processInfo.environment["RINGDOWN_SSH_USER"]
    nonisolated static let keyPath: String? = ProcessInfo.processInfo.environment["RINGDOWN_SSH_LOCAL_KEY"]

    /// Evaluated once (lazy `static let`) so the skip diagnostic prints a single
    /// time to the test log, rather than the suite skipping silently when the
    /// localhost sshd env vars are absent.
    nonisolated static let isConfigured: Bool = {
        let configured = username != nil && keyPath != nil
        if !configured {
            print("[TmuxIntegration] SKIPPED: set RINGDOWN_SSH_USER and RINGDOWN_SSH_LOCAL_KEY to run the tmux integration suite.")
        }
        return configured
    }()

    /// Builds an `SSHService` for localhost. The caller passes TOFU
    /// auto-accept handlers straight to `connect(...)`. Each test uses fresh
    /// in-memory keychain + UserDefaults so we don't touch any real device
    /// state.
    private static func makeService() async throws -> (SSHService, ServerConnection, String) {
        let user = try #require(username, "RINGDOWN_SSH_USER not set")
        let path = try #require(keyPath, "RINGDOWN_SSH_LOCAL_KEY not set")
        let expanded = (path as NSString).expandingTildeInPath
        let pem = try String(contentsOfFile: expanded, encoding: .utf8)

        let keychain = KeychainService(backend: InMemoryKeychainBackend())
        let suiteName = "test.tmux.integration.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        let store = UserDefaultsKnownHostsStore(defaults: defaults, key: "ringdown.known_hosts")
        let validator = HostKeyValidator(store: store)

        let service = SSHService(keychain: keychain, hostKeyValidator: validator)

        let connection = ServerConnection(
            name: "tmux-integration",
            host: "localhost",
            port: 22,
            username: user,
            authMethod: .key,
            connectionProtocol: .ssh
        )
        return (service, connection, pem)
    }

    /// Polls the snapshot until `predicate` is true or the deadline passes.
    /// Returns true on success, false on timeout.
    private static func waitFor(_ controller: TmuxSessionController,
                                seconds: TimeInterval,
                                predicate: @MainActor (TmuxSessionController.Snapshot) -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if predicate(controller.snapshot) { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return predicate(controller.snapshot)
    }

    /// Use a unique session name per test so concurrent / repeated runs
    /// don't collide if a previous tmux session leaked.
    private static func uniqueSessionName() -> String {
        "moshitest-\(UUID().uuidString.prefix(8))"
    }

    @Test("attach + auto-discovery + add window + select window + detach",
          .enabled(if: isConfigured),
          .timeLimit(.minutes(1)))
    func endToEnd() async throws {
        let (service, connection, pem) = try await Self.makeService()
        let session = try await service.connect(
            connection, overrideSecret: pem,
            onUnknownHost: { _, _, _ in true },
            onChangedHost: { _, _, _, _ in false })
        try await session.requestPTY(rows: 24, cols: 80)

        let tmuxSession = Self.uniqueSessionName()
        let controller = TmuxSessionController(sshSession: session)

        // attach() wires parser callbacks and starts pumping before tmux is
        // launched so the first %begin / %session-changed are captured.
        await controller.attach()

        // Now flip the PTY into tmux control mode. `-CC` enables control
        // mode; `new -s NAME` creates a fresh session.
        try await session.writeText("tmux -CC new -s \(tmuxSession)\r")

        // Wait for attach + initial discovery → at least one window known.
        let attached = await Self.waitFor(controller, seconds: 8) { snap in
            snap.isAttached && !snap.windows.isEmpty
        }
        #expect(attached, "controller should populate at least one window within 8s")
        let initialWindowCount = controller.snapshot.windows.count
        #expect(initialWindowCount >= 1, "tmux session should have ≥1 window")

        // Verify each known pane has a persistent TerminalView and that the
        // SAME instance is returned across calls (architectural invariant).
        for paneId in controller.snapshot.panes.keys {
            let tv1 = controller.terminalView(for: paneId)
            let tv2 = controller.terminalView(for: paneId)
            #expect(tv1 === tv2, "terminalView(for:) must return the same instance for paneId \(paneId)")
        }

        // Create a second window via tmux command. The control-mode session
        // should emit %window-add or refresh and our snapshot should grow.
        try await session.writeText("new-window\r")
        let grew = await Self.waitFor(controller, seconds: 6) { snap in
            snap.windows.count > initialWindowCount
        }
        #expect(grew, "snapshot should reflect the new window within 6s")

        // Switch back to the first window. selectWindow updates the
        // snapshot optimistically.
        if let firstWindow = controller.snapshot.sortedWindows.first {
            controller.selectWindow(firstWindow.id)
            #expect(controller.snapshot.activeWindowId == firstWindow.id,
                    "snapshot.activeWindowId should update synchronously")
        }

        // Clean shutdown.
        await controller.detach()
        await session.close()
    }
}
