import CryptoKit
import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// B232: deleting an SSH key took its record and left its private key in the
/// keychain for good — no record named the item any more, so nothing in the
/// app could ever reach it again — and deleting a connection that signed in
/// with such a key left it too. Now an item goes with the last record that
/// names it (counted over every saved connection and key record, the carried
/// ones included), a key a saved connection still uses isn't deleted at all,
/// and the private keys earlier builds orphaned are listed in SSH Keys again
/// once a launch — never deleted: an orphan may be the only copy of a key
/// whose record a lost key list took with it. Only `moshpit.secret.` items
/// nothing names are looked at, never anything else.
///
/// B810: every rule here is the core's (`crates/store/src/secrets.rs`), the
/// app's Swift copy of it gone — that copy kept its own set of the refs made
/// this launch, and the two disagreed depending on who made an item. These
/// run the app's calls into the core.
///
/// Over a keychain that answers the way the real one does, a little late
/// (every call waits a few milliseconds), and stores on scratch defaults.
@MainActor
@Suite("Keychain items go with the last record naming them", .serialized)
struct KeychainItemsTests {

    /// The keychain, in memory, answering each call after `delay` — the
    /// real one is a round trip to securityd.
    final class SlowKeychain: KeychainBackend, @unchecked Sendable {
        private let lock = NSLock()
        private var items: [String: Data]
        private let delay: TimeInterval
        /// Accounts whose delete the keychain refuses.
        var refusesDelete: Set<String> = []
        /// What happens between the listing being taken and it being
        /// returned: the app going on while securityd answers.
        var whileListing: (@Sendable () -> Void)?

        init(_ items: [String: Data] = [:], delay: TimeInterval = 0.004) {
            self.items = items
            self.delay = delay
        }

        var accountsNow: Set<String> { lock.withLock { Set(items.keys) } }

        func save(secret: Data, account: String, requireBiometry: Bool) throws {
            Thread.sleep(forTimeInterval: delay)
            lock.withLock { items[account] = secret }
        }

        func load(account: String, prompt: String) throws -> Data {
            Thread.sleep(forTimeInterval: delay)
            guard let data = lock.withLock({ items[account] }) else { throw KeychainError.notFound }
            return data
        }

        func delete(account: String) throws {
            Thread.sleep(forTimeInterval: delay)
            if lock.withLock({ refusesDelete.contains(account) }) {
                throw KeychainError.interactionNotAllowed
            }
            _ = lock.withLock { items.removeValue(forKey: account) }
        }

        func exists(account: String) -> Bool {
            lock.withLock { items[account] != nil }
        }

        func accounts() throws -> [String] {
            let listed = lock.withLock { Array(items.keys) }
            Thread.sleep(forTimeInterval: delay)
            whileListing?()
            return listed
        }

        /// Accounts behind Face ID: read only by asking.
        var gated: Set<String> = []

        func loadSilently(account: String) throws -> Data {
            Thread.sleep(forTimeInterval: delay)
            if lock.withLock({ gated.contains(account) }) { throw KeychainError.interactionNotAllowed }
            guard let data = lock.withLock({ items[account] }) else { throw KeychainError.notFound }
            return data
        }
    }

    /// A phone with nothing paired: a delete always lets go.
    @MainActor final class NoPairing: ConnectionCareHandshake {
        func deletionBegan(_ id: UUID) throws {}
        func forgetWindDown(_ id: UUID) async {}
        func careWindDown(_ id: UUID) async {}
        func deletionEnded(_ id: UUID) {}
    }

    private struct Stores {
        let connections: ConnectionStore
        let keys: SSHKeyStore
        let defaults: UserDefaults
        let suite: String
    }

    private static func stores(readable: Bool = true) -> Stores {
        let suite = "test.keychainrefs.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        let readiness = { readable }
        return Stores(connections: ConnectionStore(defaults: defaults, protectedData: readiness),
                      keys: SSHKeyStore(defaults: defaults, protectedData: readiness),
                      defaults: defaults, suite: suite)
    }

    private static func ref() -> String {
        // Not made through `storeNewKeychainRef()`: an item an earlier
        // launch made — before B810, the Swift path's `generateRef()`, which
        // wrote this same form.
        "moshpit.secret." + UUID().uuidString.lowercased()
    }

    private static func key(_ name: String, ref: String?) -> SSHKeyRecord {
        SSHKeyRecord(name: name, algorithm: .ed25519, publicKey: "ssh-ed25519 AAAA \(name)",
                     source: .generated, keychainRef: ref, requireBiometry: false)
    }

    private static func connection(_ name: String, key: SSHKeyRecord? = nil,
                                   ownRef: String? = nil) -> ServerConnection {
        var connection = ServerConnection(name: name, host: "\(name).example.com", username: "dev",
                                          authMethod: key == nil ? .password : .key,
                                          keychainRef: key?.keychainRef ?? ownRef)
        connection.sshKeyId = key?.id
        connection.sshKeyAlgorithmRaw = key?.algorithm.rawValue
        return connection
    }

    // MARK: - Deleting a key

    @Test("Deleting a key nothing uses takes its private key out of the keychain")
    func deletingAKeyDeletesItsPrivateKey() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let ref = Self.ref()
        let other = Self.ref()
        let keychain = SlowKeychain([ref: Data("PRIVATE".utf8), other: Data("OTHER".utf8)])
        let service = KeychainService(backend: keychain)
        let laptop = Self.key("laptop", ref: ref)
        let kept = Self.key("kept", ref: other)
        s.keys.add(laptop)
        s.keys.add(kept)

        #expect(SSHKeyDeletion.check(laptop, connections: s.connections, keys: s.keys) == .free)
        try SSHKeyDeletion.delete(laptop, from: s.keys, connections: s.connections, keychain: service.storeHost)

        #expect(s.keys.keys.map(\.id) == [kept.id])
        #expect(!keychain.accountsNow.contains(ref), "the deleted key's private key is still in the keychain")
        #expect(keychain.accountsNow.contains(other))
    }

    @Test("A key a saved connection signs in with isn't deleted, and the check names the connection")
    func aKeyInUseStays() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let ref = Self.ref()
        let keychain = SlowKeychain([ref: Data("PRIVATE".utf8)])
        let service = KeychainService(backend: keychain)
        let laptop = Self.key("laptop", ref: ref)
        s.keys.add(laptop)
        s.connections.add(Self.connection("pi", key: laptop))

        #expect(SSHKeyDeletion.check(laptop, connections: s.connections, keys: s.keys) == .inUse(["pi"]))
        #expect(throws: SSHKeyDeletion.Failure.inUse(["pi"])) {
            try SSHKeyDeletion.delete(laptop, from: s.keys, connections: s.connections, keychain: service.storeHost)
        }
        #expect(s.keys.keys.map(\.id) == [laptop.id])
        #expect(keychain.accountsNow.contains(ref))
    }

    @Test("A key the keychain won't let go of keeps its record — the one way back to it")
    func aRefusedDeleteKeepsTheRecord() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let ref = Self.ref()
        let keychain = SlowKeychain([ref: Data("PRIVATE".utf8)])
        keychain.refusesDelete = [ref]
        let service = KeychainService(backend: keychain)
        let laptop = Self.key("laptop", ref: ref)
        s.keys.add(laptop)

        #expect(throws: SSHKeyDeletion.Failure.self) {
            try SSHKeyDeletion.delete(laptop, from: s.keys, connections: s.connections, keychain: service.storeHost)
        }
        #expect(s.keys.keys.map(\.id) == [laptop.id])
        #expect(keychain.accountsNow.contains(ref))
    }

    @Test("Stores that can't be trusted yet delete nothing")
    func untrustedStoresDeleteNothing() async throws {
        let s = Self.stores(readable: false)
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let ref = Self.ref()
        let laptop = Self.key("laptop", ref: ref)
        #expect(SSHKeyDeletion.check(laptop, connections: s.connections, keys: s.keys) == .unknown)
        #expect(s.connections.keychainRefs() == nil)
    }

    @Test("The NSH page's Delete takes the private key too, and its notice names who still uses a key")
    func theNSHModelDeletesAndExplains() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let free = Self.ref()
        let used = Self.ref()
        let keychain = SlowKeychain([free: Data("A".utf8), used: Data("B".utf8)])
        let service = KeychainService(backend: keychain)
        let laptop = Self.key("laptop", ref: free)
        let work = Self.key("work", ref: used)
        s.keys.add(laptop)
        s.keys.add(work)
        s.connections.add(Self.connection("nas", key: work))
        let model = SSHKeysModel()
        model.update(SSHKeysSnapshot(store: s.keys, connections: s.connections))

        model.requestDelete(work, SSHKeyDeletion.check(work, connections: s.connections, keys: s.keys))
        #expect(model.deleting == nil, "no delete card for a key a connection uses")
        #expect(model.notice == .inUse(["nas"]))
        let state = SSHKeysModel.state(SSHKeysSnapshot(store: s.keys, connections: s.connections), deleting: nil, notice: model.notice)
        let notice = try #require(state.notice)
        #expect(notice.message.contains("nas"))
        model.dismissNotice()
        #expect(model.notice == nil)

        model.requestDelete(laptop, SSHKeyDeletion.check(laptop, connections: s.connections, keys: s.keys))
        #expect(model.deleting == laptop.id)
        model.delete(laptop, from: s.keys, connections: s.connections, keychain: service.storeHost)
        let deadline = ContinuousClock.now + .seconds(5)
        while s.keys.keys.contains(where: { $0.id == laptop.id }), ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(s.keys.keys.map(\.id) == [work.id])
        #expect(!keychain.accountsNow.contains(free))
        #expect(keychain.accountsNow.contains(used))
    }

    // MARK: - Deleting a connection

    @Test("Deleting the last connection on a deleted key's item takes the item with it")
    func theLastUserOfAnOrphanedKeyTakesItsItem() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let ref = Self.ref()
        // A key an earlier build deleted while this connection used it: the
        // record is gone, the connection still names its item.
        let gone = Self.key("gone", ref: ref)
        let pi = Self.connection("pi", key: gone)
        s.connections.add(pi)
        let keychain = SlowKeychain([ref: Data("PRIVATE".utf8)])
        let service = KeychainService(backend: keychain)
        try await ConnectionDeletion.delete(pi, from: s.connections, keys: s.keys, keychain: service.storeHost,
                                            care: NoPairing(), tearDown: {}, dropShares: { _ in })

        #expect(!keychain.accountsNow.contains(ref), "the private key no record names stays in the keychain")
        #expect(s.connections.connections.isEmpty)
    }

    @Test("Deleting a connection keeps a listed key's item, and one another connection names")
    func namedItemsStay() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let keyRef = Self.ref()
        let shared = Self.ref()
        let laptop = Self.key("laptop", ref: keyRef)
        s.keys.add(laptop)
        let pi = Self.connection("pi", key: laptop)
        let nas = Self.connection("nas", ownRef: shared)
        let nas2 = Self.connection("nas2", ownRef: shared)
        s.connections.add(pi)
        s.connections.add(nas)
        s.connections.add(nas2)
        let keychain = SlowKeychain([keyRef: Data("KEY".utf8), shared: Data("pw".utf8)])
        let service = KeychainService(backend: keychain)
        let care = NoPairing()

        try await ConnectionDeletion.delete(pi, from: s.connections, keys: s.keys, keychain: service.storeHost,
                                            care: care, tearDown: {}, dropShares: { _ in })
        try await ConnectionDeletion.delete(nas, from: s.connections, keys: s.keys, keychain: service.storeHost,
                                            care: care, tearDown: {}, dropShares: { _ in })
        #expect(keychain.accountsNow == [keyRef, shared])
        try await ConnectionDeletion.delete(nas2, from: s.connections, keys: s.keys, keychain: service.storeHost,
                                            care: care, tearDown: {}, dropShares: { _ in })
        #expect(keychain.accountsNow == [keyRef])
    }

    // MARK: - Orphans: recovered, never deleted

    /// A private key the app makes, as the keychain holds it.
    private static func privateKey(_ comment: String) throws -> SSHKeyFactory.Generated {
        try SSHKeyFactory.generate(algorithm: .ed25519, comment: comment)
    }

    @Test("an orphaned private key becomes a key record again — nothing is deleted, nothing else is touched")
    func anOrphanedKeyIsRecovered() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let lost = try Self.privateKey("lost")
        let orphan = Self.ref()
        let password = Self.ref()
        let gated = Self.ref()
        let byConnection = Self.ref()
        let byKey = Self.ref()
        let byCarried = Self.ref()
        let items: [String: Data] = [
            orphan: lost.privateBlob,
            password: Data("hunter2".utf8),
            gated: try Self.privateKey("behind Face ID").privateBlob,
            byConnection: Data("pw".utf8),
            byKey: try Self.privateKey("listed").privateBlob,
            byCarried: try Self.privateKey("newer build").privateBlob,
            "moshpit.knownHosts": Data("{}".utf8),
            "someone.else": try Self.privateKey("not ours").privateBlob,
            "home.seed.1234": try Self.privateKey("seed").privateBlob,
        ]
        let keychain = SlowKeychain(items)
        keychain.gated = [gated]
        let service = KeychainService(backend: keychain)
        s.keys.add(Self.key("laptop", ref: byKey))
        s.connections.add(Self.connection("pi", ownRef: byConnection))
        // A connection a newer build saved, which this build can't decode:
        // carried as stored, and its item is still its.
        let stored = try #require(s.defaults.data(forKey: "moshpit.connections"))
        var list = try #require(try JSONSerialization.jsonObject(with: stored) as? [Any])
        list.append(["id": UUID().uuidString, "name": "future", "authMethod": "passkey",
                     "keychainRef": byCarried])
        s.defaults.set(try JSONSerialization.data(withJSONObject: list), forKey: "moshpit.connections")
        s.connections.load()

        let recovered = s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost)

        #expect(recovered.map(\.keychainRef) == [orphan], "\(recovered.map(\.name))")
        #expect(keychain.accountsNow == Set(items.keys), "recovery deleted something")
        let record = try #require(s.keys.keys.first { $0.keychainRef == orphan })
        #expect(record.fingerprint == lost.fingerprint)
        #expect(record.publicKey == lost.publicKeyLine)
        #expect(record.algorithm == .ed25519)
        #expect(record.source == .imported)
        #expect(!record.requireBiometry)
        let short = String(lost.fingerprint.dropFirst("SHA256:".count).prefix(8))
        #expect(record.name == String(localized: "Recovered key \(short)"))
        #expect(record.name.contains(short))
        // Stored: SSH Keys lists it after a relaunch, and the keys page
        // shows it with the rest.
        #expect(SSHKeyStore(defaults: s.defaults).keys.contains { $0.keychainRef == orphan })
        #expect(SSHKeysSnapshot(store: s.keys, connections: s.connections).imported.contains { $0.id == record.id })
    }

    /// B810: the core reads every stored private key but a Secure Enclave
    /// key's handle, which only CryptoKit opens — and for that key the item
    /// is the only copy there is. Recovery asks the app about what the core
    /// can't read (``EnclaveKeyReader``) and lists it again as a Secure
    /// Enclave key. (A device has one; the simulator doesn't.)
    @Test("an orphaned Secure Enclave key is listed again as one", .enabled(if: SecureEnclave.isAvailable))
    func anOrphanedEnclaveKeyIsRecovered() throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let handle = try SecureEnclave.P256.Signing.PrivateKey().dataRepresentation
        let described = try #require(SSHKeyFactory.describeEnclaveHandle(handle))
        let orphan = Self.ref()
        let keychain = SlowKeychain([orphan: handle])
        let service = KeychainService(backend: keychain)

        let recovered = s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost)

        let record = try #require(recovered.first)
        #expect(recovered.count == 1)
        #expect(record.keychainRef == orphan)
        #expect(record.algorithm == .seP256)
        #expect(record.source == .secureEnclave)
        #expect(record.fingerprint == described.fingerprint)
        #expect(record.publicKey == described.publicKeyLine)
        #expect(!record.requireBiometry)
        #expect(keychain.accountsNow == [orphan], "recovery changed the keychain")
    }

    @Test("a second launch adds nothing: a recovered key is named from then on")
    func recoveryIsOncePerOrphan() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let orphan = Self.ref()
        let keychain = SlowKeychain([orphan: try Self.privateKey("lost").privateBlob,
                                     Self.ref(): Data("a password".utf8)])
        let service = KeychainService(backend: keychain)
        #expect(s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost).count == 1)
        // The same launch looks once.
        #expect(s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost).isEmpty)
        // The next launch: fresh stores over the same defaults and keychain.
        let connections = ConnectionStore(defaults: s.defaults)
        let keys = SSHKeyStore(defaults: s.defaults)
        #expect(keys.recoverOrphansOnce(connections: connections, keychain: service.storeHost).isEmpty)
        #expect(keys.keys.filter { $0.keychainRef == orphan }.count == 1)
        #expect(keychain.accountsNow.count == 2)
    }

    @Test("a recovered key deleted in SSH Keys takes its secret with it, and stays gone")
    func aRecoveredKeyDeletesLikeAnyKey() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let orphan = Self.ref()
        let keychain = SlowKeychain([orphan: try Self.privateKey("lost").privateBlob])
        let service = KeychainService(backend: keychain)
        let record = try #require(s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost).first)

        try SSHKeyDeletion.delete(record, from: s.keys, connections: s.connections, keychain: service.storeHost)
        #expect(s.keys.keys.isEmpty)
        #expect(!keychain.accountsNow.contains(orphan))
        // A later launch: fresh stores, and the key stays gone.
        let keys = SSHKeyStore(defaults: s.defaults)
        #expect(keys.recoverOrphansOnce(connections: ConnectionStore(defaults: s.defaults),
                                        keychain: service.storeHost).isEmpty)
    }

    /// B810: the app made its refs in Swift and kept its own set of them,
    /// which the core's recovery never saw — an item made this launch was
    /// an orphan to it. Refs come from the core now: one set, the one
    /// recovery asks.
    @Test("an item made this launch is no orphan, though its record isn't saved yet")
    func anItemOnItsWayIsNotAnOrphan() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let keychain = SlowKeychain()
        let service = KeychainService(backend: keychain)
        // Add Key's order: the private key under a fresh ref, then the
        // record — recovery lists the keychain in between.
        let ref = storeNewKeychainRef()
        try await service.saveSecret(try Self.privateKey("new").privateBlob, forRef: ref, requireBiometry: false)
        let keys = s.keys
        keychain.whileListing = {
            DispatchQueue.main.async { MainActor.assumeIsolated { keys.add(Self.key("new", ref: ref)) } }
        }

        let recovered = s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost)

        #expect(recovered.isEmpty)
        try await Task.sleep(for: .milliseconds(20))
        #expect(s.keys.keys.filter { $0.keychainRef == ref }.count == 1, "the key being added was listed twice")
        #expect(keychain.accountsNow.contains(ref))
    }

    @Test("a launch before the first unlock recovers nothing, and recovers once the stores can be trusted")
    func nothingBeforeTheStoresCanBeTrusted() async throws {
        let sealed = Self.stores(readable: false)
        defer { sealed.defaults.removePersistentDomain(forName: sealed.suite) }
        let named = Self.ref()
        let keychain = SlowKeychain([named: try Self.privateKey("pi's key").privateBlob])
        let service = KeychainService(backend: keychain)
        // What a sealed launch reads: nothing at all.
        #expect(sealed.keys.recoverOrphansOnce(connections: sealed.connections, keychain: service.storeHost).isEmpty)
        // Unlocked: the stores read what they hold — a connection names the
        // item — and the look runs.
        let open = Self.stores()
        defer { open.defaults.removePersistentDomain(forName: open.suite) }
        open.connections.add(Self.connection("pi", ownRef: named))
        #expect(open.keys.recoverOrphansOnce(connections: open.connections, keychain: service.storeHost).isEmpty)
        #expect(open.keys.keys.isEmpty, "a connection's own key was listed as an orphan")
        #expect(keychain.accountsNow == [named])
    }

    // MARK: - Items connections let go of (B428)

    private static let toRelease = "moshpit.secrets.toRelease"

    @Test("B428: a pasted key a save let go of while its session ran is dropped at the next launch, never offered back")
    func aListedItemIsDroppedAtTheNextLaunch() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let pasted = Self.ref()
        let managed = Self.ref()
        let keychain = SlowKeychain([pasted: try Self.privateKey("pasted").privateBlob,
                                     managed: try Self.privateKey("managed").privateBlob])
        let service = KeychainService(backend: keychain)
        let laptop = Self.key("laptop", ref: managed)
        s.keys.add(laptop)
        s.connections.add(Self.connection("pi", key: laptop))
        // An earlier launch let go of it while its session still signed in
        // with it, and ended before the session did.
        s.defaults.set([pasted], forKey: Self.toRelease)

        let recovered = s.keys.recoverOrphansOnce(connections: s.connections, keychain: service.storeHost)
        #expect(recovered.isEmpty, "offered back as a key: \(recovered.map(\.name))")
        let settled = s.connections.settleReleasesOnce(keys: s.keys, keychain: service.storeHost)
        let accounts = settled.map(\.account)
        #expect(accounts == [pasted])
        #expect(keychain.accountsNow == [managed])
        let left = s.defaults.stringArray(forKey: Self.toRelease)
        #expect(left == nil)
    }

    @Test("B428: an item listed this launch waits for its session; one a record names again stays")
    func aHeldItemWaitsAndANamedOneStays() async throws {
        let s = Self.stores()
        defer { s.defaults.removePersistentDomain(forName: s.suite) }
        let held = Self.ref()
        let named = Self.ref()
        let keychain = SlowKeychain([held: Data("pw".utf8), named: Data("pw2".utf8)])
        let service = KeychainService(backend: keychain)
        // This launch's session still reads `held`.
        s.connections.holdSecret(held)
        // An earlier launch listed `named`, which a connection picked up since.
        s.connections.add(Self.connection("pi", ownRef: named))
        s.defaults.set([held, named], forKey: Self.toRelease)

        let settled = s.connections.settleReleasesOnce(keys: s.keys, keychain: service.storeHost)
        let accounts = settled.map(\.account)
        #expect(accounts == [named])
        #expect(keychain.accountsNow == [held, named])
        // The session reconnected as saved, or ended.
        let fate = s.connections.releaseSecret(held, keys: s.keys, keychain: service.storeHost)
        #expect(fate == .deleted)
        #expect(keychain.accountsNow == [named])
        let left = s.defaults.stringArray(forKey: Self.toRelease)
        #expect(left == nil)
    }
}
