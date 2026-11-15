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

    @Test("A new connection: its password in a fresh item, the connection added; a managed key's item never written")
    @MainActor
    func savingANewConnection() async throws {
        let name = "w15d.credential.\(UUID().uuidString)"
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
            await KeychainReferences.release(ref, connections: store, keys: keyStore, keychain: keychain)
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

/// B133: a key pasted into the connection form with Windows line endings,
/// indented lines or its line breaks lost — or as PEM — was stored as pasted,
/// and the connect path, which reads only an openssh-key-v1 file with "\n"
/// line breaks, failed it at every connect. The SSH Keys import has stored
/// the converted key since B29; the connection form now does too.
@Suite("A key pasted into the connection form")
struct PastedKeyStorageTests {
    private static let ed25519 = SSHKeyFixtures.all.first { $0.name == "Ed25519 · OpenSSH" }!

    private static func publicBlob(_ text: String) throws -> Data {
        try SSHKeyFactory.publicKeyBlob(of: text)
    }

    private static func indented(_ text: String) -> String {
        text.split(separator: "\n", omittingEmptySubsequences: false).map { "\t  " + $0 }.joined(separator: "\n")
    }

    @Test("Mangled in the paste, it is stored as the key the connect path reads")
    func mangledPastesAreStoredReadable() throws {
        let key = Self.ed25519.privateKey
        let mangled = [
            key.replacingOccurrences(of: "\n", with: "\r\n"),
            Self.indented(key),
            key.replacingOccurrences(of: "\n", with: " "),
            "\n\n" + Self.indented(key.replacingOccurrences(of: "\n", with: "\r\n")) + "\r\n\r\n",
        ]
        for pasted in mangled {
            #expect(!StoredKey.connectPathReads(pasted), "the connect path reads it as pasted after all")
            let stored = ConnectionSecrets.storable(pasted)
            #expect(StoredKey.connectPathReads(stored))
            #expect(try Self.publicBlob(stored) == Self.publicBlob(key))
        }
    }

    @Test("A PEM paste is stored converted; a clean paste exactly as pasted")
    func pemConvertsCleanStays() throws {
        let pem = SSHKeyFixtures.all.first { $0.name == "RSA 2048 · PEM (ssh-keygen -m PEM)" }!
        let stored = ConnectionSecrets.storable(pem.privateKey)
        #expect(stored.hasPrefix("-----BEGIN OPENSSH PRIVATE KEY-----"))
        #expect(try Self.publicBlob(stored) == Self.publicBlob(pem.privateKey))
        #expect(ConnectionSecrets.storable(Self.ed25519.privateKey) == Self.ed25519.privateKey)
    }

    @Test("Behind a passphrase, or not a key at all: kept as pasted, for the connect to deal with")
    func unreadableKeptAsPasted() {
        let encrypted = SSHKeyFixtures.all.first { $0.name == "Ed25519 · OpenSSH · passphrase" }!.privateKey
        #expect(ConnectionSecrets.storable(encrypted) == encrypted)
        #expect(ConnectionSecrets.storable("not a key") == "not a key")
        #expect(ConnectionSecrets.storable(SSHKeyFixtures.ecdsaP384) == SSHKeyFixtures.ecdsaP384)
    }

    @Test("Imported in SSH Keys with every line indented: stored as the key the connect path reads")
    func importIndented() throws {
        let imported = try SSHKeyFactory.importKey(privatePEM: Self.indented(Self.ed25519.privateKey),
                                                   publicKeyLine: nil, comment: "")
        #expect(imported.fingerprint == Self.ed25519.fingerprint)
        #expect(StoredKey.connectPathReads(String(decoding: imported.privateBlob, as: UTF8.self)))
    }
}
