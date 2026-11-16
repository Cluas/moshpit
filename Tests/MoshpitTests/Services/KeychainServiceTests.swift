import Foundation
import Testing
@testable import Moshpit

/// Tests exercise `KeychainService` against an `InMemoryKeychainBackend` so
/// they never touch the device's real keychain and never prompt for biometrics.
@Suite("KeychainService")
struct KeychainServiceTests {

    // MARK: - Password round-trip

    @Test("save → load returns the same password")
    func passwordRoundTrip() async throws {
        let service = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await service.savePassword("hunter2", forRef: ref, requireBiometry: false)
        let loaded = try await service.loadPassword(forRef: ref, reason: "test")
        #expect(loaded == "hunter2")
    }

    @Test("savePassword overwrites an existing value")
    func passwordOverwrite() async throws {
        let service = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await service.savePassword("first", forRef: ref, requireBiometry: false)
        try await service.savePassword("second", forRef: ref, requireBiometry: false)
        let loaded = try await service.loadPassword(forRef: ref, reason: "test")
        #expect(loaded == "second")
    }

    // MARK: - Private key round-trip

    @Test("private key PEM round-trips through the keychain")
    func privateKeyRoundTrip() async throws {
        let service = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        let pem = """
        -----BEGIN OPENSSH PRIVATE KEY-----
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAA
        -----END OPENSSH PRIVATE KEY-----
        """
        try await service.savePrivateKey(pem, forRef: ref, requireBiometry: false)
        let loaded = try await service.loadPrivateKey(forRef: ref, reason: "test")
        #expect(loaded == pem)
    }

    // MARK: - Raw secret

    @Test("raw secret round-trip preserves bytes exactly")
    func rawSecretRoundTrip() async throws {
        let service = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        let bytes = Data([0x00, 0x01, 0xFE, 0xFF, 0x42])
        try await service.saveSecret(bytes, forRef: ref, requireBiometry: false)
        let loaded = try await service.loadSecret(forRef: ref, reason: "test")
        #expect(loaded == bytes)
    }

    // MARK: - Delete

    @Test("delete removes a stored secret")
    func deleteRemovesSecret() async throws {
        let service = KeychainService.inMemory()
        let ref = storeNewKeychainRef()
        try await service.savePassword("toBeDeleted", forRef: ref, requireBiometry: false)
        try await service.delete(forRef: ref)
        await #expect(throws: KeychainError.notFound) {
            _ = try await service.loadPassword(forRef: ref, reason: "test")
        }
    }

    @Test("delete is idempotent for missing refs")
    func deleteIdempotent() async throws {
        let service = KeychainService.inMemory()
        // Should not throw — delete on unknown ref is a no-op.
        try await service.delete(forRef: "moshpit.secret.does-not-exist")
    }

    // MARK: - notFound

    @Test("loadPassword throws notFound for an unknown ref")
    func loadMissingThrowsNotFound() async throws {
        let service = KeychainService.inMemory()
        await #expect(throws: KeychainError.notFound) {
            _ = try await service.loadPassword(forRef: "missing", reason: "test")
        }
    }

    // MARK: - Distinct keys are isolated

    @Test("two refs hold independent secrets")
    func distinctRefsAreIndependent() async throws {
        let service = KeychainService.inMemory()
        let ref1 = storeNewKeychainRef()
        let ref2 = storeNewKeychainRef()
        try await service.savePassword("alpha", forRef: ref1, requireBiometry: false)
        try await service.savePassword("beta", forRef: ref2, requireBiometry: false)
        let a = try await service.loadPassword(forRef: ref1, reason: "test")
        let b = try await service.loadPassword(forRef: ref2, reason: "test")
        #expect(a == "alpha")
        #expect(b == "beta")
    }

    // MARK: - Ref uniqueness

    /// Refs come from the Rust store (B810: one set of the refs made this
    /// launch, the one orphan recovery asks).
    @Test("refs are unique opaque IDs")
    func refsAreUnique() async throws {
        var seen = Set<String>()
        for _ in 0..<32 {
            let ref = storeNewKeychainRef()
            #expect(ref.hasPrefix("moshpit.secret."))
            #expect(seen.contains(ref) == false)
            seen.insert(ref)
        }
    }
}

/// The simulator's stand-in for the keychain: one JSON file per backend. A
/// file it couldn't decode read as an empty vault, and the next save — any
/// secret, for any connection — wrote a one-entry vault over every key.
@Suite("FileVaultKeychainBackend")
struct FileVaultKeychainBackendTests {

    /// Where `FileVaultKeychainBackend(filename:)` keeps `filename`.
    private func vaultURL(_ filename: String) throws -> URL {
        try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                    appropriateFor: nil, create: true)
            .appendingPathComponent(filename)
    }

    private func removeVault(_ url: URL) {
        let dir = url.deletingLastPathComponent()
        for name in (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        where name.hasPrefix(url.lastPathComponent) {
            try? FileManager.default.removeItem(at: dir.appendingPathComponent(name))
        }
    }

    @Test("A vault that won't decode is never written over, and a copy is kept")
    func undecodableVaultIsNotOverwritten() throws {
        let filename = "test-vault-\(UUID().uuidString).json"
        let url = try vaultURL(filename)
        defer { removeVault(url) }
        // Every stored key, in a shape this build can't decode.
        let original = Data(#"{"version":2,"items":{"moshpit.secret.a":"c2VjcmV0"}}"#.utf8)
        try original.write(to: url)
        let backend = FileVaultKeychainBackend(filename: filename)

        #expect(throws: (any Error).self, "a save went ahead over a vault it couldn't read") {
            try backend.save(secret: Data("new".utf8), account: "moshpit.secret.b", requireBiometry: false)
        }
        #expect(throws: (any Error).self) { try backend.delete(account: "moshpit.secret.a") }
        #expect(try Data(contentsOf: url) == original, "the vault was written over")
        #expect(try Data(contentsOf: url.appendingPathExtension("backup")) == original,
                "no untouched copy of the vault was kept")

        // And a load says "unreadable", never "nothing stored here".
        do {
            _ = try backend.load(account: "moshpit.secret.a", prompt: "")
            Issue.record("loaded a secret out of a vault it couldn't decode")
        } catch {
            #expect((error as? KeychainError) != .notFound)
        }
    }

    @Test("No vault yet is an empty one: saves, loads and deletes work")
    func missingVaultIsEmpty() throws {
        let filename = "test-vault-\(UUID().uuidString).json"
        let url = try vaultURL(filename)
        defer { removeVault(url) }
        let backend = FileVaultKeychainBackend(filename: filename)

        #expect(throws: KeychainError.notFound) { _ = try backend.load(account: "a", prompt: "") }
        try backend.save(secret: Data("one".utf8), account: "a", requireBiometry: false)
        try backend.save(secret: Data("two".utf8), account: "b", requireBiometry: false)
        #expect(try backend.load(account: "a", prompt: "") == Data("one".utf8))
        try backend.delete(account: "a")
        #expect(!backend.exists(account: "a"))
        #expect(try backend.load(account: "b", prompt: "") == Data("two".utf8))
    }
}

/// Whether the test host can reach the real keychain: a signed build on a
/// device. The simulator's unsigned host gets `errSecMissingEntitlement`.
enum KeychainTestHost {
    static let hasKeychain: Bool = {
        #if targetEnvironment(simulator)
        return false
        #else
        return true
        #endif
    }()
}

/// `SystemKeychainBackend` saving over an item that is already there.
///
/// The overwrite used to be delete-then-add, and between the two calls the
/// account held nothing: an add the keychain refused there — the device
/// locking mid-save, any keychain error — left the item gone for good. The
/// known-hosts table is ONE item, so that was every fingerprint ever trusted.
@Suite("System keychain backend — saving over an item")
struct SystemKeychainBackendSaveTests {

    /// Generic passwords by account, answering add / update / delete the way
    /// Keychain Services does. After `callsBeforeLock` calls the device locks
    /// and every call after answers `errSecInteractionNotAllowed` — what a
    /// `WhenUnlocked` item gets while the device is locked.
    final class FakeKeychain: @unchecked Sendable {
        private let lock = NSLock()
        private var items: [String: Data]
        /// Each item's `kSecAttrAccessible`, as last added or updated.
        private var classes: [String: String] = [:]
        private var callsLeft: Int?
        private var _calls: [String] = []

        init(items: [String: Data] = [:], callsBeforeLock: Int? = nil,
             classes: [String: String] = [:]) {
            self.items = items
            self.callsLeft = callsBeforeLock
            self.classes = classes
        }

        var calls: [String] { lock.withLock { _calls } }
        func item(_ account: String) -> Data? { lock.withLock { items[account] } }
        func protectionClass(_ account: String) -> String? { lock.withLock { classes[account] } }

        var writes: KeychainWrites {
            KeychainWrites(
                add: { [self] in self.add($0) },
                update: { [self] in self.update($0, $1) },
                delete: { [self] in self.delete($0) })
        }

        private static func fields(_ dictionary: CFDictionary) -> [String: Any] {
            (dictionary as NSDictionary) as? [String: Any] ?? [:]
        }

        /// Counts the call; false once the device has locked.
        private func unlocked(_ call: String) -> Bool {
            _calls.append(call)
            guard let left = callsLeft else { return true }
            callsLeft = left - 1
            return left > 0
        }

        private func add(_ attributes: CFDictionary) -> OSStatus {
            let fields = Self.fields(attributes)
            return lock.withLock {
                guard unlocked("add") else { return errSecInteractionNotAllowed }
                guard let account = fields[kSecAttrAccount as String] as? String,
                      let data = fields[kSecValueData as String] as? Data else { return errSecParam }
                guard items[account] == nil else { return errSecDuplicateItem }
                items[account] = data
                classes[account] = fields[kSecAttrAccessible as String] as? String
                return errSecSuccess
            }
        }

        private func update(_ query: CFDictionary, _ changes: CFDictionary) -> OSStatus {
            let account = Self.fields(query)[kSecAttrAccount as String] as? String
            let data = Self.fields(changes)[kSecValueData as String] as? Data
            let access = Self.fields(changes)[kSecAttrAccessible as String] as? String
            return lock.withLock {
                guard unlocked("update") else { return errSecInteractionNotAllowed }
                guard let account, items[account] != nil else { return errSecItemNotFound }
                if let data { items[account] = data }
                if let access { classes[account] = access }
                return errSecSuccess
            }
        }

        private func delete(_ query: CFDictionary) -> OSStatus {
            let account = Self.fields(query)[kSecAttrAccount as String] as? String
            return lock.withLock {
                guard unlocked("delete") else { return errSecInteractionNotAllowed }
                guard let account, items.removeValue(forKey: account) != nil else { return errSecItemNotFound }
                return errSecSuccess
            }
        }
    }

    private let account = "moshpit.knownHosts"
    private let trusted = Data(#"{"m1:22":"SHA256:aaaa"}"#.utf8)
    private let updated = Data(#"{"m1:22":"SHA256:aaaa","m2:22":"SHA256:bbbb"}"#.utf8)

    @Test("A save the keychain refuses partway leaves the item that was there")
    func refusedSaveKeepsTheItem() {
        // One call goes through, then the device locks.
        let keychain = FakeKeychain(items: [account: trusted], callsBeforeLock: 1)
        let backend = SystemKeychainBackend(service: "test", writes: keychain.writes)

        #expect(throws: KeychainError.interactionNotAllowed) {
            try backend.save(secret: updated, account: account, requireBiometry: false)
        }

        #expect(keychain.item(account) == trusted,
                "the item was deleted and the add that should have replaced it was refused")
    }

    @Test("Saving over an item rewrites it where it stands — it is never deleted")
    func overwriteIsInPlace() throws {
        let keychain = FakeKeychain(items: [account: trusted])
        let backend = SystemKeychainBackend(service: "test", writes: keychain.writes)

        try backend.save(secret: updated, account: account, requireBiometry: false)

        #expect(keychain.item(account) == updated)
        #expect(!keychain.calls.contains("delete"), "the overwrite went through a moment with no item: \(keychain.calls)")
    }

    /// B783: the trusted host keys are public, the trust anchors a phone
    /// restored from a backup should keep: they are saved `AfterFirstUnlock`,
    /// which moves with an encrypted backup and Quick Start — every other
    /// item stays `ThisDeviceOnly`. A `ThisDeviceOnly` map an earlier build
    /// saved is moved where it stands: one update of its class, every byte
    /// kept; refused (the device locked), it stays exactly as it was.
    @Test("The trusted host keys move with a backup; nothing else does; an earlier map is moved in place")
    func trustedHostKeysMoveWithBackups() throws {
        let thisDevice = kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String
        let moves = kSecAttrAccessibleAfterFirstUnlock as String
        let keychain = FakeKeychain()
        let backend = SystemKeychainBackend(service: "test", writes: keychain.writes)
        try backend.save(secret: trusted, account: account, requireBiometry: false)
        try backend.save(secret: trusted, account: "moshpit.knownHosts.backup", requireBiometry: false)
        try backend.save(secret: Data("hunter2".utf8), account: "moshpit.secret.nas", requireBiometry: false)
        #expect(keychain.protectionClass(account) == moves)
        #expect(keychain.protectionClass("moshpit.knownHosts.backup") == moves)
        #expect(keychain.protectionClass("moshpit.secret.nas") == thisDevice)

        // A map 1.0.8 (or an earlier 1.1.0 Canary or TestFlight build) saved.
        let earlier = FakeKeychain(items: [account: trusted], classes: [account: thisDevice])
        let upgraded = SystemKeychainBackend(service: "test", writes: earlier.writes)
        try upgraded.moveWithBackups(account: account)
        #expect(earlier.protectionClass(account) == moves)
        #expect(earlier.item(account) == trusted, "every byte kept")
        #expect(earlier.calls == ["update"], "never deleted and added again: \(earlier.calls)")
        try upgraded.moveWithBackups(account: "moshpit.knownHosts.backup")   // none: nothing to move

        let locked = FakeKeychain(items: [account: trusted], callsBeforeLock: 0, classes: [account: thisDevice])
        #expect(throws: KeychainError.interactionNotAllowed) {
            try SystemKeychainBackend(service: "test", writes: locked.writes).moveWithBackups(account: account)
        }
        #expect(locked.item(account) == trusted && locked.protectionClass(account) == thisDevice)
    }

    @Test("With no item yet, a save adds one")
    func firstSaveAdds() throws {
        let keychain = FakeKeychain()
        let backend = SystemKeychainBackend(service: "test", writes: keychain.writes)

        try backend.save(secret: trusted, account: account, requireBiometry: false)

        #expect(keychain.item(account) == trusted)
        #expect(keychain.calls == ["add"])
    }

    /// The same overwrite against the real keychain: the attributes the
    /// in-place rewrite hands `SecItemUpdate` have to be ones it accepts. A
    /// throwaway service, no biometry, removed afterwards — nothing the app
    /// stores is touched, and nothing prompts.
    @Test("On a device, a save over a real keychain item replaces it",
          .enabled(if: KeychainTestHost.hasKeychain, "the simulator's unsigned test host has no keychain"))
    func realKeychainOverwrite() throws {
        let backend = SystemKeychainBackend(service: "com.cluas.moshpit.tests.\(UUID().uuidString)")
        let account = "overwrite"
        defer { try? backend.delete(account: account) }

        try backend.save(secret: trusted, account: account, requireBiometry: false)
        try backend.save(secret: updated, account: account, requireBiometry: false)

        #expect(try backend.load(account: account, prompt: "") == updated)
    }
}
