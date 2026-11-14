import Foundation
import UIKit
import NIOCore
import NIOPosix
import NIOSSH
import Testing
@testable import Moshpit

@Suite("HostKeyValidator")
struct HostKeyValidatorTests {

    // MARK: - First-trust lifecycle

    @Test("unknown → trust → trusted, then mismatch → re-trust → trusted")
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

        // 4. Accepting the new key replaces the old one: fpB is trusted, fpA
        //    is now the mismatch.
        await validator.trust(host: host, port: port, fingerprint: fpB)
        let reTrusted = await validator.decide(host: host, port: port, fingerprint: fpB)
        #expect(reTrusted == .trusted)
        let old = await validator.decide(host: host, port: port, fingerprint: fpA)
        #expect(old == .changed(expected: fpB))
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

        let store = UserDefaultsKnownHostsStore(defaults: defaults, key: "moshpit.knownHosts.test")
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
        let store = KeychainKnownHostsStore(backend: backend, account: "moshpit.knownHosts.test")
        let first = HostKeyValidator(store: store)
        await first.trust(host: "persist.example", port: 22, fingerprint: "SHA256:STORED")

        // A brand-new validator reading a fresh store over the same backend must
        // recover the committed fingerprint.
        let second = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: "moshpit.knownHosts.test"))
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

/// The known-hosts map is a `WhenUnlockedThisDeviceOnly` keychain item, so
/// it can't be read while the device is locked — and `SSHService.shared`,
/// whose validator used to read it once at init, is created by the first SSH
/// of the process, whenever that happens. A failed read came back as an empty
/// map and stayed cached: every host showed as new, and the first "Trust"
/// wrote a one-entry map over every fingerprint the user had.
@Suite("HostKeyValidator — a locked keychain")
struct HostKeyValidatorLockedKeychainTests {

    private static let account = "moshpit.knownHosts.test"
    private static let trusted = ["a.example:22": "SHA256:AAAA", "b.example:22": "SHA256:BBBB"]
    private static func hostKey() throws -> NIOSSHPublicKey {
        try NIOSSHPublicKey(
            openSSHPublicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDgQhgqywG4eG3cXK2gEELsrEXf5Q4v4JzrxSrrRVcFg")
    }

    private func lockedKeychain(holding table: [String: String] = trusted) throws -> LockableKeychainBackend {
        LockableKeychainBackend(seed: [Self.account: try JSONEncoder().encode(table)], locked: true)
    }

    private func storedTable(_ backend: LockableKeychainBackend) throws -> [String: String] {
        try JSONDecoder().decode([String: String].self,
                                 from: backend.peek(account: Self.account) ?? Data())
    }

    private func waitUntil(_ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("A read made while locked is not an empty map, and the next one after an unlock sees every host")
    func lockedReadIsNeitherEmptyNorCached() async throws {
        let backend = try lockedKeychain()
        let validator = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: Self.account))

        let whileLocked = await validator.decide(host: "a.example", port: 22, fingerprint: "SHA256:AAAA")
        #expect(whileLocked != .unknown, "a host trusted long ago was offered as new")
        #expect(whileLocked != .trusted, "nothing was read, so nothing can pass")

        backend.locked = false
        let afterUnlock = await validator.decide(host: "a.example", port: 22, fingerprint: "SHA256:AAAA")
        #expect(afterUnlock == .trusted, "the empty answer from the locked read stuck for the life of the process")
    }

    @Test("Trusting a new host after a locked first read keeps every other fingerprint")
    func trustAfterLockedReadKeepsTheMap() async throws {
        let backend = try lockedKeychain()
        let validator = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: Self.account))
        // The first handshake of the process, with the phone still locked.
        _ = await validator.decide(host: "c.example", port: 22, fingerprint: "SHA256:CCCC")

        backend.locked = false
        await validator.trust(host: "c.example", port: 22, fingerprint: "SHA256:CCCC")

        var expected = Self.trusted
        expected["c.example:22"] = "SHA256:CCCC"
        #expect(try storedTable(backend) == expected,
                "the trust was written over every fingerprint the user had")
    }

    @Test("A trust made while locked holds for now and is written over the map once it can be read")
    func trustWhileLockedIsWrittenAfterUnlock() async throws {
        let backend = try lockedKeychain()
        let unlocks = NotificationCenter()
        let validator = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: Self.account),
                                         unlocks: unlocks)

        await validator.trust(host: "c.example", port: 22, fingerprint: "SHA256:CCCC")
        #expect(backend.saves == 0, "nothing may be written before the map has been read")
        #expect(await validator.decide(host: "c.example", port: 22, fingerprint: "SHA256:CCCC") == .trusted,
                "the user's answer counts for this process")

        backend.locked = false
        unlocks.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        #expect(await waitUntil { backend.saves == 1 }, "the unlock never brought the trust to the store")

        var expected = Self.trusted
        expected["c.example:22"] = "SHA256:CCCC"
        #expect(try storedTable(backend) == expected)
    }

    @Test("A handshake while the trusted keys are locked away fails without asking")
    func lockedHandshakeFailsWithoutAPrompt() async throws {
        let backend = try lockedKeychain()
        let validator = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: Self.account))
        let delegate = TOFUHostKeyDelegate(validator: validator, host: "a.example", port: 22)
        let key = try Self.hostKey()
        let group = MultiThreadedEventLoopGroup(numberOfThreads: 1)
        let promise: EventLoopPromise<Void> = group.next().makePromise()

        delegate.validateHostKey(hostKey: key, validationCompletePromise: promise)

        do {
            try await promise.futureResult.get()
            Issue.record("a host whose key could not be checked was let through")
        } catch is HostKeyNeedsDecision {
            // What makes SSHService show the new-host card.
            Issue.record("the first-connection prompt would be shown for a host trusted long ago")
        } catch let error as KnownHostsUnreadable {
            #expect(error.locked)
        }
        #expect(backend.saves == 0)
        try await group.shutdownGracefully()
    }

    @Test("The locked failure reads as a sentence about unlocking")
    func lockedFailureIsNamed() {
        let mapped = SSHError.map(KnownHostsUnreadable(reason: "the device is locked", locked: true))
        guard case .hostKeysLocked = mapped else {
            Issue.record("mapped to \(mapped.diagnostic)")
            return
        }
        #expect(!mapped.description.isEmpty)
    }

    @Test("The store says it could not read, rather than reading nothing")
    func storeThrowsWhileLocked() throws {
        let backend = try lockedKeychain()
        let store = KeychainKnownHostsStore(backend: backend, account: Self.account)
        #expect(throws: (any Error).self) { _ = try store.read() }
        backend.locked = false
        #expect(try store.read() == Self.trusted)
        // Nothing stored at all is the one empty answer.
        let empty = KeychainKnownHostsStore(backend: InMemoryKeychainBackend(), account: Self.account)
        #expect(try empty.read() == [:])
    }

    @Test("A map this build can't decode is copied untouched before the first trust is written over it")
    func undecodableMapIsBackedUp() async throws {
        // Host a in a shape a newer build might write; host b as today.
        let original = Data(#"{"a.example:22":["SHA256:AAAA","SHA256:AAA2"],"b.example:22":"SHA256:BBBB"}"#.utf8)
        let backend = LockableKeychainBackend(seed: [Self.account: original], locked: false)
        let validator = HostKeyValidator(store: KeychainKnownHostsStore(backend: backend, account: Self.account))

        #expect(await validator.decide(host: "b.example", port: 22, fingerprint: "SHA256:BBBB") == .trusted,
                "the entries this build can read are still used")
        await validator.trust(host: "c.example", port: 22, fingerprint: "SHA256:CCCC")

        #expect(backend.peek(account: Self.account + ".backup") == original,
                "the only copy of the map was written over")
        #expect(try storedTable(backend)["b.example:22"] == "SHA256:BBBB")
        #expect(try storedTable(backend)["c.example:22"] == "SHA256:CCCC")
    }

    /// B155: the salvage read the map through JSONSerialization, which drops
    /// a leading U+FEFF from every key — the fingerprint came back under
    /// another host, and the next trust wrote it there.
    @Test("A salvaged map keeps a host that starts with U+FEFF as it was stored")
    func salvageKeepsByteOrderMark() throws {
        let original = Data("{\"\u{FEFF}b.example:22\":\"SHA256:BBBB\",\"a.example:22\":[\"SHA256:AAAA\"]}".utf8)
        let backend = LockableKeychainBackend(seed: [Self.account: original], locked: false)
        let store = KeychainKnownHostsStore(backend: backend, account: Self.account)

        let table = try store.read()

        #expect(table == ["\u{FEFF}b.example:22": "SHA256:BBBB"], "\(table.keys.map { Array($0.unicodeScalars) })")
    }
}

/// A keychain that answers the way the real one does for a
/// `WhenUnlockedThisDeviceOnly` item while the device is locked: every call
/// fails with errSecInteractionNotAllowed.
final class LockableKeychainBackend: KeychainBackend, @unchecked Sendable {
    private let lock = NSLock()
    private var store: [String: Data]
    private var _locked: Bool
    private var _saves = 0

    init(seed: [String: Data] = [:], locked: Bool) {
        store = seed
        _locked = locked
    }

    var locked: Bool {
        get { lock.withLock { _locked } }
        set { lock.withLock { _locked = newValue } }
    }

    /// Successful saves so far.
    var saves: Int { lock.withLock { _saves } }

    /// What is stored, locked or not — the test's view, not the app's.
    func peek(account: String) -> Data? { lock.withLock { store[account] } }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        try lock.withLock {
            guard !_locked else { throw KeychainError.interactionNotAllowed }
            store[account] = secret
            _saves += 1
        }
    }

    func load(account: String, prompt: String) throws -> Data {
        try lock.withLock {
            guard !_locked else { throw KeychainError.interactionNotAllowed }
            guard let data = store[account] else { throw KeychainError.notFound }
            return data
        }
    }

    func delete(account: String) throws {
        try lock.withLock {
            guard !_locked else { throw KeychainError.interactionNotAllowed }
            store[account] = nil
        }
    }

    func exists(account: String) -> Bool {
        lock.withLock { store[account] != nil }
    }
}


/// B226: a launch before the first unlock after a reboot can find no
/// known-hosts item at all where there is one — `notFound`, not the lock's
/// refusal. That empty map was believed and cached as the table: every host
/// read as new, and after the unlock the cached empty map stayed, so the
/// first "Trust" wrote one entry over every fingerprint the user had. Here
/// the stubbed keychain answers `notFound` until the unlock, a few
/// milliseconds late on every call, as securityd does.
@Suite("HostKeyValidator — before the first unlock")
struct HostKeyValidatorSealedLaunchTests {

    private static let account = "moshpit.knownHosts.sealed"
    private static let trusted = ["a.example:22": "SHA256:AAAA", "b.example:22": "SHA256:BBBB"]

    /// The device's first unlock since boot: until then the keychain shows
    /// no item at all, and protected data is unavailable.
    final class SealedKeychain: KeychainBackend, @unchecked Sendable {
        private let lock = NSLock()
        private var items: [String: Data]
        private var _sealed = true
        private var _saves = 0

        init(_ items: [String: Data]) { self.items = items }

        var sealed: Bool {
            get { lock.withLock { _sealed } }
            set { lock.withLock { _sealed = newValue } }
        }
        var saves: Int { lock.withLock { _saves } }
        func peek(_ account: String) -> Data? { lock.withLock { items[account] } }

        func save(secret: Data, account: String, requireBiometry: Bool) throws {
            Thread.sleep(forTimeInterval: 0.003)
            try lock.withLock {
                guard !_sealed else { throw KeychainError.interactionNotAllowed }
                items[account] = secret
                _saves += 1
            }
        }

        func load(account: String, prompt: String) throws -> Data {
            Thread.sleep(forTimeInterval: 0.003)
            return try lock.withLock {
                // Sealed: as if nothing were stored.
                guard !_sealed, let data = items[account] else { throw KeychainError.notFound }
                return data
            }
        }

        func delete(account: String) throws {
            try lock.withLock {
                guard !_sealed else { throw KeychainError.interactionNotAllowed }
                items[account] = nil
            }
        }

        func exists(account: String) -> Bool { lock.withLock { !_sealed && items[account] != nil } }
    }

    private func sealed() throws -> SealedKeychain {
        SealedKeychain([Self.account: try JSONEncoder().encode(Self.trusted)])
    }

    private func store(_ keychain: SealedKeychain) -> KeychainKnownHostsStore {
        KeychainKnownHostsStore(backend: keychain, account: Self.account,
                                protectedDataAvailable: { !keychain.sealed })
    }

    private func stored(_ keychain: SealedKeychain) throws -> [String: String] {
        try JSONDecoder().decode([String: String].self, from: keychain.peek(Self.account) ?? Data())
    }

    private func waitUntil(_ done: () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !(await done()) {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("An empty read before the first unlock is the lock's answer, not an empty map")
    func anEmptySealedReadIsLocked() throws {
        let keychain = try sealed()
        let store = store(keychain)
        do {
            let read = try store.read()
            Issue.record("a sealed keychain's \"nothing there\" was taken as the map: \(read)")
        } catch let error as KnownHostsUnreadable {
            #expect(error.locked)
        }
        keychain.sealed = false
        #expect(try store.read() == Self.trusted)
        // Unlocked, nothing stored is still the one empty answer.
        let empty = KeychainKnownHostsStore(backend: InMemoryKeychainBackend(), account: Self.account,
                                            protectedDataAvailable: { true })
        #expect(try empty.read() == [:])
    }

    @Test("A trust after a sealed first read keeps every fingerprint, and known hosts stay known")
    func aTrustAfterASealedReadKeepsTheMap() async throws {
        let keychain = try sealed()
        let unlocks = NotificationCenter()
        let validator = HostKeyValidator(store: store(keychain), unlocks: unlocks)

        // The first handshake of the process: a background launch before the
        // first unlock (a notification reply, say).
        let before = await validator.decide(host: "a.example", port: 22, fingerprint: "SHA256:AAAA")
        #expect(before == .unverifiable, "a host trusted long ago was offered as new: \(before)")

        keychain.sealed = false
        unlocks.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        #expect(await waitUntil {
            await validator.decide(host: "a.example", port: 22, fingerprint: "SHA256:AAAA") == .trusted
        }, "the empty sealed read stuck as the table for the life of the process")

        await validator.trust(host: "c.example", port: 22, fingerprint: "SHA256:CCCC")
        var expected = Self.trusted
        expected["c.example:22"] = "SHA256:CCCC"
        #expect(try stored(keychain) == expected, "the trust was written over every fingerprint the user had")
        #expect(keychain.saves == 1)
    }

    @Test("A trust lands on the map as stored now, not on the copy this process read first")
    func aTrustMergesOntoTheStoredMap() async throws {
        let keychain = try sealed()
        keychain.sealed = false
        // Two writers of the one item in one process: the Swift transport's
        // validator and the Rust core's sessions.
        let swift = HostKeyValidator(store: store(keychain), unlocks: NotificationCenter())
        let rust = HostKeyValidator(store: store(keychain), unlocks: NotificationCenter())
        #expect(await swift.decide(host: "a.example", port: 22, fingerprint: "SHA256:AAAA") == .trusted)

        await rust.trust(host: "r.example", port: 22, fingerprint: "SHA256:RRRR")
        await swift.trust(host: "s.example", port: 22, fingerprint: "SHA256:SSSS")

        var expected = Self.trusted
        expected["r.example:22"] = "SHA256:RRRR"
        expected["s.example:22"] = "SHA256:SSSS"
        #expect(try stored(keychain) == expected, "a trust the other writer made was written away")
    }
}
