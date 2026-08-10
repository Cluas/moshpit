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
        let ref = service.generateRef()
        try await service.savePassword("hunter2", forRef: ref, requireBiometry: false)
        let loaded = try await service.loadPassword(forRef: ref, reason: "test")
        #expect(loaded == "hunter2")
    }

    @Test("savePassword overwrites an existing value")
    func passwordOverwrite() async throws {
        let service = KeychainService.inMemory()
        let ref = service.generateRef()
        try await service.savePassword("first", forRef: ref, requireBiometry: false)
        try await service.savePassword("second", forRef: ref, requireBiometry: false)
        let loaded = try await service.loadPassword(forRef: ref, reason: "test")
        #expect(loaded == "second")
    }

    // MARK: - Private key round-trip

    @Test("private key PEM round-trips through the keychain")
    func privateKeyRoundTrip() async throws {
        let service = KeychainService.inMemory()
        let ref = service.generateRef()
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
        let ref = service.generateRef()
        let bytes = Data([0x00, 0x01, 0xFE, 0xFF, 0x42])
        try await service.saveSecret(bytes, forRef: ref, requireBiometry: false)
        let loaded = try await service.loadSecret(forRef: ref, reason: "test")
        #expect(loaded == bytes)
    }

    // MARK: - Delete

    @Test("delete removes a stored secret")
    func deleteRemovesSecret() async throws {
        let service = KeychainService.inMemory()
        let ref = service.generateRef()
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
        let ref1 = service.generateRef()
        let ref2 = service.generateRef()
        try await service.savePassword("alpha", forRef: ref1, requireBiometry: false)
        try await service.savePassword("beta", forRef: ref2, requireBiometry: false)
        let a = try await service.loadPassword(forRef: ref1, reason: "test")
        let b = try await service.loadPassword(forRef: ref2, reason: "test")
        #expect(a == "alpha")
        #expect(b == "beta")
    }

    // MARK: - contains()

    @Test("contains reflects save/delete state")
    func containsReflectsState() async throws {
        let service = KeychainService.inMemory()
        let ref = service.generateRef()
        let beforeSave = await service.contains(ref: ref)
        #expect(beforeSave == false)
        try await service.savePassword("x", forRef: ref, requireBiometry: false)
        let afterSave = await service.contains(ref: ref)
        #expect(afterSave == true)
        try await service.delete(forRef: ref)
        let afterDelete = await service.contains(ref: ref)
        #expect(afterDelete == false)
    }

    // MARK: - generateRef uniqueness

    @Test("generateRef returns unique opaque IDs")
    func generateRefUnique() async throws {
        let service = KeychainService.inMemory()
        var seen = Set<String>()
        for _ in 0..<32 {
            let ref = service.generateRef()
            #expect(ref.hasPrefix("moshpit.secret."))
            #expect(seen.contains(ref) == false)
            seen.insert(ref)
        }
    }
}
