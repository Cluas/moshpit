import Foundation
import Testing
@testable import Moshpit

/// Pairing material, the one-liner the user runs on the host, and the drift
/// guard between the shell sender and its Swift copy.
@Suite("Push pairing")
struct PushPairingTests {

    // MARK: - Material

    @Test("make mints the secret locally and leaves the credential to the relay")
    func material() {
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                 relayURL: "https://push.example.org")
        let b = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                 relayURL: "https://push.example.org")
        for pairing in [a, b] {
            #expect(pairing.secretHex.count == 64)
            #expect(Data(moshpitHex: pairing.secretHex) != nil)
            // The send token is the RELAY's to mint — an HMAC over the routing
            // facts, which do not exist until a device token does. A fresh
            // pairing must therefore be visibly not-ready, never installable.
            #expect(pairing.sendToken.isEmpty)
            #expect(!pairing.isReadyToInstall)
            #expect(pairing.needsMint(currentToken: nil))
        }
        #expect(a.secretHex != b.secretHex)
    }

    @Test("a credential re-mints when the device token rotates or age nears the relay TTL")
    func mintLifecycle() {
        var pairing = PushPairing.make(connectionId: UUID(), hostLabel: "h",
                                       relayURL: "https://r")
        let now = Date()
        pairing.sendToken = String(repeating: "ab", count: 32)
        pairing.apnsToken = String(repeating: "CD", count: 32)
        pairing.apnsEnv = "production"
        pairing.sendTokenIssuedAt = now

        #expect(pairing.isReadyToInstall)
        // Same token, fresh mint: nothing to do. Hex casing must not split it.
        #expect(!pairing.needsMint(currentToken: String(repeating: "cd", count: 32), now: now))
        // The phone's token rotated (reinstall, restore): the HMAC no longer
        // matches what the conf will claim, so it must re-mint.
        #expect(pairing.needsMint(currentToken: String(repeating: "ee", count: 32), now: now))
        // Ageing towards the relay-side TTL re-mints long before expiry.
        #expect(pairing.needsMint(currentToken: String(repeating: "cd", count: 32),
                                  now: now.addingTimeInterval(PushPairing.refreshAfter + 1)))
        // A pairing stored by an older build (no routing fields) must read as
        // needing a mint, not as ready.
        pairing.apnsToken = nil
        #expect(pairing.needsMint(currentToken: String(repeating: "cd", count: 32), now: now))
        #expect(!pairing.isReadyToInstall)
    }

    @Test("a trailing slash on the relay URL does not become a double slash")
    func urlNormalisation() {
        let pairing = PushPairing.make(connectionId: UUID(), hostLabel: "h",
                                       relayURL: "  https://push.example.org/  ")
        #expect(pairing.relayURL == "https://push.example.org")
    }

    // MARK: - What replaced the one-liner
    //
    // Pairing no longer types anything into a shell, so the tests that pinned
    // that command's shape are gone with it. The install path they covered is
    // now `HostInstallerTests` (which asserts the secret never reaches a command
    // line as anything but base64) and `HostSetupModelTests` (which asserts a
    // pairing proves itself with a real push instead of claiming success).

    @Test("nothing here can put a secret into a shell command any more")
    func noShellPathRemains() {
        let pairing = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                       relayURL: "https://push.example.org")
        // The file the host receives is built as CONTENT, not as a command.
        let conf = HostInstaller.pushConf(pairing)
        #expect(conf.contains(pairing.secretHex))
        #expect(!conf.contains("sh -c"))
        #expect(!conf.contains("$HOME"))
    }
}

@Suite("Pairing store persistence", .serialized)
struct PushPairingStorePersistenceTests {

    /// The store is real shared state with no injection seam — on device the
    /// extension has nobody to inject it — so these save and restore.
    private func withCleanStore(_ body: () -> Void) {
        let saved = PushPairingStore.read()
        defer { PushPairingStore.write(saved) }
        PushPairingStore.write([])
        body()
    }

    @Test("a save reports success only when it can be read back")
    func writeVerifies() {
        withCleanStore {
            let pairing = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                          relayURL: "https://push.example.org")
            // The return value is the point: it used to be three silent `try?`s,
            // so a failed save left the app behaving exactly as if nothing were
            // paired — no prompt, no token, no push, no explanation.
            #expect(PushPairingStore.upsert(pairing))
            #expect(PushPairingStore.read().count == 1)
            #expect(PushPairingStore.pairing(for: pairing.connectionId)?.secretHex
                    == pairing.secretHex)
        }
    }

    @Test("re-pairing one host replaces it and leaves the others alone")
    func upsertReplaces() {
        withCleanStore {
            let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
            let b = PushPairing.make(connectionId: UUID(), hostLabel: "b", relayURL: "https://b")
            #expect(PushPairingStore.upsert(a))
            #expect(PushPairingStore.upsert(b))

            let again = PushPairing.make(connectionId: a.connectionId, hostLabel: "a",
                                         relayURL: "https://a2")
            #expect(PushPairingStore.upsert(again))
            #expect(PushPairingStore.read().count == 2)
            #expect(PushPairingStore.pairing(for: a.connectionId)?.relayURL == "https://a2")
            #expect(PushPairingStore.pairing(for: b.connectionId)?.relayURL == "https://b")
            // Newest first is what the extension relies on when it tries keys.
            #expect(PushPairingStore.secretsNewestFirst().first == again.secretHex)
        }
    }
}
