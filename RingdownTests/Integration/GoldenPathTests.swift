import Foundation
import Testing
@testable import Ringdown

/// End-to-end "Add → Connect" golden path: simulates what `NewConnectionView.save()`
/// does (write secret to keychain, add `ServerConnection` to store), then opens
/// an SSH session via `SSHService` exactly the way `TerminalViewModel.start()`
/// would. Validates that the form's persistence shape matches what the SSH
/// service expects to read back out.
///
/// Gated on the same `RINGDOWN_SSH_USER` / `RINGDOWN_SSH_LOCAL_KEY` env vars as
/// `SSHIntegrationTests`. When unset the suite is skipped — and the diagnostic
/// below makes that skip visible in the test log rather than silent.
@Suite("GoldenPath", .tags(.integration), .serialized)
struct GoldenPathTests {
    static var username: String? { ProcessInfo.processInfo.environment["RINGDOWN_SSH_USER"] }
    static var keyPath: String? { ProcessInfo.processInfo.environment["RINGDOWN_SSH_LOCAL_KEY"] }

    /// Evaluated once (lazy `static let`) so the skip diagnostic prints a single
    /// time to the test log rather than the suite skipping silently.
    static let isConfigured: Bool = {
        let configured = username != nil && keyPath != nil
        if !configured {
            print("[GoldenPath] SKIPPED: set RINGDOWN_SSH_USER and RINGDOWN_SSH_LOCAL_KEY to run the golden-path integration suite.")
        }
        return configured
    }()

    @Test("Add-via-form persistence shape feeds a successful SSH connect",
          .enabled(if: isConfigured),
          .timeLimit(.minutes(1)))
    func formSaveThenSSHConnect() async throws {
        let user = try #require(Self.username)
        let path = try #require(Self.keyPath)
        let expanded = (path as NSString).expandingTildeInPath
        let pem = try String(contentsOfFile: expanded, encoding: .utf8)

        // === Step 1: Replicate exactly what NewConnectionView.save() does. ===
        let keychain = KeychainService(backend: InMemoryKeychainBackend())
        let suiteName = "test.golden.\(UUID().uuidString)"
        let storeDefaults = UserDefaults(suiteName: suiteName)!
        let store = ConnectionStore(defaults: storeDefaults)

        let connectionID = UUID()
        let keychainRef = connectionID.uuidString
        try await keychain.savePrivateKey(pem, forRef: keychainRef, requireBiometry: false)

        let connection = ServerConnection(
            id: connectionID,
            name: "golden",
            host: "localhost",
            port: 22,
            username: user,
            authMethod: .key,
            connectionProtocol: .ssh,
            sshPort: 22,
            keychainRef: keychainRef
        )
        store.add(connection)

        // === Step 2: Verify the persistence layer round-trip. ===
        let reloadedDefaults = UserDefaults(suiteName: suiteName)!
        let reloaded = ConnectionStore(defaults: reloadedDefaults)
        let found = try #require(
            reloaded.connections.first { $0.id == connectionID },
            "saved connection must survive UserDefaults round-trip"
        )
        #expect(found.host == "localhost")
        #expect(found.username == user)
        #expect(found.keychainRef == keychainRef)

        // === Step 3: Pull the secret back the same way SSHService would. ===
        let storedPEM = try await keychain.loadPrivateKey(forRef: keychainRef)
        #expect(storedPEM == pem, "round-tripped PEM must match what was saved")

        // === Step 4: Use exactly that ServerConnection + secret to SSH. ===
        let knownHostsDefaults = UserDefaults(suiteName: "test.golden.known.\(UUID().uuidString)")!
        let validator = HostKeyValidator(
            store: UserDefaultsKnownHostsStore(defaults: knownHostsDefaults, key: "known")
        )
        let service = SSHService(keychain: keychain, hostKeyValidator: validator)

        // overrideSecret mirrors the path TerminalViewModel would take when
        // pulling the key from the keychain *just before* connecting — same
        // shape, different sourcing. TOFU handlers are passed straight to
        // `connect(...)` now (per-call parameters, not mutable service state).
        let session = try await service.connect(
            found, overrideSecret: storedPEM,
            onUnknownHost: { _, _, _ in true },
            onChangedHost: { _, _, _, _ in false })
        try await session.requestPTY(rows: 24, cols: 80)

        // Confirm bytes flow.
        let marker = "moshi-golden-\(UUID().uuidString.prefix(8))"
        try await session.writeText("printf '%s\\n' \(marker)\n")
        let saw = await waitFor(String(marker), in: session, seconds: 10)
        #expect(saw, "shell echo round-trip via the saved keychain credential")

        await session.close()
    }

    @Test("Password-method save then load returns the same secret",
          .enabled(if: isConfigured))
    func passwordSecretRoundTrip() async throws {
        let keychain = KeychainService(backend: InMemoryKeychainBackend())
        let id = UUID()
        try await keychain.savePassword("hunter2 ⚡️", forRef: id.uuidString, requireBiometry: false)

        let loaded = try await keychain.loadPassword(forRef: id.uuidString)
        #expect(loaded == "hunter2 ⚡️")
    }

    @Test("Deleting a connection erases the paired keychain secret",
          .enabled(if: isConfigured))
    func deleteRemovesKeychainSecret() async throws {
        let keychain = KeychainService(backend: InMemoryKeychainBackend())
        let id = UUID()
        try await keychain.savePassword("temporary", forRef: id.uuidString, requireBiometry: false)
        #expect(await keychain.contains(ref: id.uuidString))

        // Mirrors HomeView.performDelete: keychain.delete(forRef:) gets called
        // alongside store.delete(id:).
        try await keychain.delete(forRef: id.uuidString)
        #expect(await !keychain.contains(ref: id.uuidString))
    }

    // MARK: - Helpers

    private func waitFor(_ needle: String, in session: SSHSession, seconds: TimeInterval) async -> Bool {
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
}

