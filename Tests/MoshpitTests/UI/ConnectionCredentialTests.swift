import Foundation
import Testing
@testable import Moshpit

/// Saving the connection editor, on the store and the keychain. What the
/// credential is — a managed key's item never written or owned, a
/// connection's own item never kept under another method (B298) — is the
/// shared model's (`crates/app/src/connection_form/tests.rs`); this checks
/// the host does what the model's plan says, and nothing when it refuses.
@Suite("Connection credential")
struct ConnectionCredentialTests {

    /// It applied the draft's method first, so the credential step could
    /// not tell the method had changed (B298).
    @Test("Saving a pasted-key connection as Password with no password: refused, store and keychain as they were")
    @MainActor
    func savingRefusesTheSwitch() async throws {
        let name = "r40b.credential.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults)
        let ref = "moshpit.secret.r40b-own"
        let keychain = KeychainService.inMemory(seed: [ref: Data("PASTED-KEY".utf8)])
        var saved = ServerConnection()
        saved.name = "box"
        saved.host = "box.example.com"
        saved.username = "me"
        saved.authMethod = .key
        saved.keychainRef = ref
        store.add(saved)

        var draft = ConnectionDraft(saved)
        draft.auth = .password
        var secrets = ConnectionSecrets()
        await #expect(throws: ConnectionSaveRefusal.NewSecretNeeded(auth: .password)) {
            try await draft.save(existing: saved, store: store, keychain: keychain,
                                 settings: .shared, keys: [], secrets: secrets)
        }
        #expect(store.connections.map(\.authMethod) == [.key], "the store was written")
        #expect(store.connections.first?.keychainRef == ref)
        #expect(try await keychain.loadPrivateKey(forRef: ref) == "PASTED-KEY")

        secrets.password = "hunter2"
        try await draft.save(existing: saved, store: store, keychain: keychain, settings: .shared, keys: [],
                             secrets: secrets)
        let switched = try #require(store.connections.first)
        #expect(switched.authMethod == .password && switched.keychainRef == ref)
        #expect(try await keychain.loadPassword(forRef: ref) == "hunter2")
    }

    /// B782: restored onto another iPhone, a connection has its record and
    /// not its password. Its draft keeps nothing and says whose; the password
    /// typed goes to the item the record names, and Home's Connect connects
    /// it again.
    @Test("A password not on this iPhone: the draft keeps nothing, and a new one goes to its own item")
    @MainActor
    func savingAMissingPassword() async throws {
        let name = "b782.credential.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults)
        let ref = "moshpit.secret.b782-own"
        let keychain = KeychainService.inMemory(seed: [:])
        var saved = ServerConnection()
        saved.name = "nas"
        saved.host = "nas.example.com"
        saved.username = "me"
        saved.keychainRef = ref
        store.add(saved)
        store.setSecretMissing([saved.id])
        #expect(store.secretMissing == [saved.id])

        #expect(ConnectionDraft(saved).stored == .password)
        let draft = ConnectionDraft(saved, secretMissing: true)
        #expect(draft.stored == StoredCredential.none)
        #expect(draft.missing?.name == "nas")
        #expect(!draft.canSave(keys: [], secrets: ConnectionSecrets()), "Save waits for the password")

        var secrets = ConnectionSecrets()
        secrets.password = "hunter2"
        try await draft.save(existing: saved, store: store, keychain: keychain, settings: .shared, keys: [],
                             secrets: secrets)
        #expect(store.connections.first?.keychainRef == ref)
        #expect(try await keychain.loadPassword(forRef: ref) == "hunter2")
        #expect(store.secretMissing.isEmpty, "it connects again")
    }

    @Test("A new connection: its password in a fresh item, the connection added; a managed key's item never written")
    @MainActor
    func savingANewConnection() async throws {
        let name = "test.credential.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults)
        let keychain = KeychainService.inMemory(seed: [:])
        var draft = ConnectionDraft(newWith: .shared)
        draft.name = "box"
        draft.host = "box.example.com"
        draft.username = "me"
        try await draft.save(existing: nil, store: store, keychain: keychain, settings: .shared, keys: [],
                             secrets: ConnectionSecrets(password: "hunter2"))
        let added = try #require(store.connections.first)
        let ref = try #require(added.keychainRef)
        #expect(try await keychain.loadPassword(forRef: ref) == "hunter2")

        let key = SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated,
                               keychainRef: "moshpit.secret.key-k")
        var keyed = ConnectionDraft(added)
        keyed.auth = .key
        keyed.selectedKeyId = key.id
        try await keyed.save(existing: added, store: store, keychain: keychain, settings: .shared, keys: [key],
                             secrets: ConnectionSecrets(password: "not-written"))
        let pointing = try #require(store.connections.first)
        #expect(pointing.keychainRef == key.keychainRef && pointing.sshKeyId == key.id)
        #expect((try? await keychain.loadPassword(forRef: "moshpit.secret.key-k")) == nil,
                "a password written into the key's item")
    }

    /// B325: switched to a managed key, the connection's own password item
    /// stayed named by nothing, for good (a pasted key came back as a
    /// "Recovered key"). The save hands it to `release`, which drops it once
    /// nothing else names it.
    @Test("Switched to a managed key, the connection's own item is released and dropped")
    @MainActor
    func switchingToAManagedKeyReleasesTheOwnItem() async throws {
        let name = "b325.credential.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults)
        let keyStore = SSHKeyStore(defaults: defaults)
        let keychain = KeychainService.inMemory(seed: [:])
        var draft = ConnectionDraft(newWith: .shared)
        draft.name = "box"
        draft.host = "box.example.com"
        draft.username = "me"
        try await draft.save(existing: nil, store: store, keychain: keychain, settings: .shared, keys: [],
                             secrets: ConnectionSecrets(password: "hunter2"))
        let added = try #require(store.connections.first)
        let own = try #require(added.keychainRef)

        let key = SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated,
                               keychainRef: "moshpit.secret.key-k")
        keyStore.add(key)
        var keyed = ConnectionDraft(added)
        keyed.auth = .key
        keyed.selectedKeyId = key.id
        var released: [String] = []
        try await keyed.save(existing: added, store: store, keychain: keychain, settings: .shared,
                             keys: [key], secrets: ConnectionSecrets(),
                             release: { released.append($0) })
        #expect(released == [own])
        for ref in released {
            store.releaseSecret(ref, keys: keyStore, keychain: keychain.storeHost)
        }
        #expect((try? await keychain.loadPassword(forRef: own)) == nil, "the old password stayed")
        #expect(store.connections.first?.keychainRef == key.keychainRef)
    }

    /// B328: pointed at another host, the connection kept the push pairing
    /// of the box it pointed at before, which went on pushing under it. A
    /// save that moves it forgets the pairing first; one that can't, saves
    /// nothing; a rename forgets nothing.
    @Test("Moved to another host, the connection's pairing is forgotten first; a rename keeps it")
    @MainActor
    func movingToAnotherHostForgetsThePairing() async throws {
        let name = "b328.credential.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults)
        let keychain = KeychainService.inMemory(seed: ["moshpit.secret.own": Data("hunter2".utf8)])
        var saved = ServerConnection()
        saved.name = "box"
        saved.host = "box.example.com"
        saved.username = "me"
        saved.authMethod = .password
        saved.keychainRef = "moshpit.secret.own"
        store.add(saved)
        var forgotten: [UUID] = []

        var renamed = ConnectionDraft(saved)
        renamed.name = "renamed"
        try await renamed.save(existing: saved, store: store, keychain: keychain, settings: .shared,
                               keys: [], secrets: ConnectionSecrets(),
                               forgetPairing: { forgotten.append($0) })
        #expect(forgotten.isEmpty, "a rename forgot the pairing")

        let now = try #require(store.connections.first)
        var moved = ConnectionDraft(now)
        moved.host = "other.example.com"
        await #expect(throws: PushPairingForgetFailure.self) {
            try await moved.save(existing: now, store: store, keychain: keychain, settings: .shared,
                                 keys: [], secrets: ConnectionSecrets(),
                                 forgetPairing: { _ in throw PushPairingForgetFailure() })
        }
        #expect(store.connections.first?.host == "box.example.com", "saved though the pairing stayed")

        // B534: with a new password too, the forget that fails leaves the
        // record signing in with its own (the new one went to a fresh
        // item, deleted again), and nothing saved.
        var movedWithPassword = ConnectionDraft(now)
        movedWithPassword.host = "third.example.com"
        await #expect(throws: PushPairingForgetFailure.self) {
            try await movedWithPassword.save(existing: now, store: store, keychain: keychain, settings: .shared,
                                             keys: [], secrets: ConnectionSecrets(password: "new-pass"),
                                             forgetPairing: { _ in throw PushPairingForgetFailure() })
        }
        #expect(try await keychain.loadPassword(forRef: "moshpit.secret.own") == "hunter2",
                "the record kept signs in with the new password")
        #expect(store.connections.first?.keychainRef == "moshpit.secret.own")

        try await moved.save(existing: now, store: store, keychain: keychain, settings: .shared,
                             keys: [], secrets: ConnectionSecrets(),
                             forgetPairing: { forgotten.append($0) })
        #expect(forgotten == [saved.id])
        #expect(store.connections.first?.host == "other.example.com")
    }
}

