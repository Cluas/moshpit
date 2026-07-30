import Foundation
import Testing
@testable import Ringdown

@Suite("HostKeyValidator")
struct HostKeyValidatorTests {

    // MARK: - First-trust lifecycle

    @Test("unknown → trust → trusted, then mismatch → forget → unknown")
    func fullLifecycle() async throws {
        let validator = HostKeyValidator.inMemory()
        let host = "example.com"
        let port = 22
        let fpA = "SHA256:AAAA"
        let fpB = "SHA256:BBBB"

        // 1. First contact: unknown.
        let first = await validator.decide(host: host, port: port, fingerprint: fpA)
        #expect(first == .unknown)

        // 2. Commit trust, then same fingerprint matches.
        await validator.trust(host: host, port: port, fingerprint: fpA)
        let match = await validator.decide(host: host, port: port, fingerprint: fpA)
        #expect(match == .trusted)

        // 3. Server presents a different key → .changed(expected: fpA)
        let mismatch = await validator.decide(host: host, port: port, fingerprint: fpB)
        #expect(mismatch == .changed(expected: fpA))

        // 4. forget clears the stored entry.
        await validator.forget(host: host, port: port)
        let afterForget = await validator.decide(host: host, port: port, fingerprint: fpA)
        #expect(afterForget == .unknown)

        // 5. Re-trust with fpB succeeds.
        await validator.trust(host: host, port: port, fingerprint: fpB)
        let reTrusted = await validator.decide(host: host, port: port, fingerprint: fpB)
        #expect(reTrusted == .trusted)
    }

    // MARK: - Different host:port pairs are isolated

    @Test("entries are scoped by host AND port")
    func hostPortIsolation() async throws {
        let validator = HostKeyValidator.inMemory()
        await validator.trust(host: "h", port: 22, fingerprint: "SHA256:X")
        let samePort = await validator.decide(host: "h", port: 22, fingerprint: "SHA256:X")
        let diffPort = await validator.decide(host: "h", port: 2222, fingerprint: "SHA256:X")
        let diffHost = await validator.decide(host: "other", port: 22, fingerprint: "SHA256:X")
        #expect(samePort == .trusted)
        #expect(diffPort == .unknown)
        #expect(diffHost == .unknown)
    }

    // MARK: - Persistence via store

    @Test("UserDefaults-backed store survives a fresh validator")
    func persistsAcrossValidatorInstances() async throws {
        let suiteName = "test.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = UserDefaultsKnownHostsStore(defaults: defaults, key: "ringdown.knownHosts.test")
        let first = HostKeyValidator(store: store)
        await first.trust(host: "persist.example", port: 22, fingerprint: "SHA256:STORED")

        // New validator reads the same store — entry should be there.
        let second = HostKeyValidator(store: store)
        let decision = await second.decide(host: "persist.example", port: 22, fingerprint: "SHA256:STORED")
        #expect(decision == .trusted)
    }

    @Test("Keychain-backed store survives a fresh validator")
    func keychainStorePersistsAcrossValidatorInstances() async throws {
        // Shared backend stands in for the on-device keychain so the round-trip
        // exercises the production `KeychainKnownHostsStore` code path without
        // touching the real keychain.
        let backend = InMemoryKeychainBackend()
        let store = KeychainKnownHostsStore(backend: backend, account: "ringdown.knownHosts.test")
        let first = HostKeyValidator(store: store)
        await first.trust(host: "persist.example", port: 22, fingerprint: "SHA256:STORED")

        // A brand-new validator reading a fresh store over the same backend must
        // recover the committed fingerprint.
        let second = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: "ringdown.knownHosts.test"))
        let decision = await second.decide(host: "persist.example", port: 22, fingerprint: "SHA256:STORED")
        #expect(decision == .trusted)

        // A mismatching key against the persisted entry is flagged as changed.
        let changed = await second.decide(host: "persist.example", port: 22, fingerprint: "SHA256:OTHER")
        #expect(changed == .changed(expected: "SHA256:STORED"))
    }

    // MARK: - snapshot()

    @Test("snapshot reflects current trust map")
    func snapshotReflectsState() async throws {
        let validator = HostKeyValidator.inMemory()
        await validator.trust(host: "a", port: 22, fingerprint: "SHA256:1")
        await validator.trust(host: "b", port: 2222, fingerprint: "SHA256:2")

        let snap = await validator.snapshot()
        #expect(snap["a:22"] == "SHA256:1")
        #expect(snap["b:2222"] == "SHA256:2")
        #expect(snap.count == 2)
    }

    // MARK: - Trust overwrites existing

    @Test("trust overwrites the previously stored fingerprint")
    func trustOverwrites() async throws {
        let validator = HostKeyValidator.inMemory()
        await validator.trust(host: "h", port: 22, fingerprint: "SHA256:OLD")
        await validator.trust(host: "h", port: 22, fingerprint: "SHA256:NEW")
        let decision = await validator.decide(host: "h", port: 22, fingerprint: "SHA256:NEW")
        #expect(decision == .trusted)
        let oldDecision = await validator.decide(host: "h", port: 22, fingerprint: "SHA256:OLD")
        #expect(oldDecision == .changed(expected: "SHA256:NEW"))
    }
}
