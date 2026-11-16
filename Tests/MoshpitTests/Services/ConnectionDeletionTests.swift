import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// Deleting a connection takes its push pairing with it — first, since it is
/// the one step that can fail and every step after it can't be taken back.
///
/// The pairing and host care are the core's (`crates/hostcare`, whose
/// `tests/push.rs` and `tests/care.rs` hold the rest: a token never minted
/// again, a relay's late answer dropped, a care pass stood down). What is
/// left here is the delete's own order, over a handshake that records it,
/// and the handshake over a real core and a pairings file of the test's own
/// (the App Group container is out of reach in an unsigned build, and must
/// not be touched in a signed one).
@MainActor
@Suite("Deleting a connection")
struct ConnectionDeletionTests {

    /// The keychain as the core reaches it (B810: the delete's item is the
    /// core's to drop), in memory, keeping what it was asked to delete.
    final class Keychain: KeychainHost, @unchecked Sendable {
        private let lock = NSLock()
        private var items: [String: Data]
        private(set) var deleted: [String] = []

        init(_ accounts: [String] = []) {
            items = Dictionary(uniqueKeysWithValues: accounts.map { ($0, Data("secret".utf8)) })
        }

        func save(account: String, secret: Data, requireBiometry: Bool) throws {
            lock.withLock { items[account] = secret }
        }

        func load(account: String, prompt: String) throws -> Data {
            guard let data = lock.withLock({ items[account] }) else { throw KeychainFailure.NotFound }
            return data
        }

        func delete(account: String) throws {
            lock.withLock {
                deleted.append(account)
                items[account] = nil
            }
        }

        func exists(account: String) -> Bool { lock.withLock { items[account] != nil } }
        func accounts() throws -> [String] { lock.withLock { Array(items.keys) } }
        func loadSilently(account: String) throws -> Data { try load(account: account, prompt: "") }
    }

    /// Records what the delete told care, and when.
    @MainActor final class Recorder: ConnectionCareHandshake {
        var steps: [String] = []
        var cannotForget = false

        func deletionBegan(_ id: UUID) throws {
            steps.append("began")
            if cannotForget { throw PushPairingForgetFailure() }
        }

        func forgetWindDown(_ id: UUID) async { steps.append("forget") }
        func careWindDown(_ id: UUID) async { steps.append("wind down") }
        func deletionEnded(_ id: UUID) { steps.append("ended") }
    }

    /// No host to reach and no relay: the core's platform for a delete.
    final class NoHost: HostCarePlatform, @unchecked Sendable {
        func exec(connection: String, command: String) async throws -> Data {
            throw HostCareFailure.Failed(why: "no host")
        }
        func post(url: String, body: String, timeoutSeconds: UInt32) async throws -> HttpReplyValue {
            throw HostCareFailure.Failed(why: "no relay")
        }
        func registerForPush() {}
        func rememberDeclined(connection: String) {}
        func rememberUnpaired(connection: String, unpaired: Bool) {}
        func changed(change: HostCareChange) {}
    }

    /// A pairings file of the test's own, through the Rust store the app
    /// and the notification service both read it with.
    struct TestPairingsFile {
        let url: URL
        private var store: RustPushPairings { RustPushPairings(url: url) }
        func read() -> [PairingRecord] { store.read() }
        @discardableResult func write(_ pairings: [PairingRecord]) -> Bool { store.write(pairings) }
        @discardableResult func upsert(_ pairing: PairingRecord) -> Bool { store.upsert(pairing) }
        func remove(connectionId: String) -> Bool { store.remove(connectionId: connectionId) }
    }

    /// A pairing for connection `id`, its send token not minted yet.
    static func pairing(_ id: UUID, host: String, relayURL: String = relayURL) -> PairingRecord {
        PairingRecord(connectionId: id.uuidString, hostLabel: host, secretHex: String(repeating: "ab", count: 32),
                      sendToken: "", relayUrl: relayURL, createdAt: Date().timeIntervalSinceReferenceDate,
                      apnsToken: nil, apnsEnv: nil, sendTokenIssuedAt: nil)
    }

    /// A pairings file of the test's own, as the core reaches one.
    final class FilePairings: PairingsFile, @unchecked Sendable {
        let file: TestPairingsFile
        init(_ file: TestPairingsFile) { self.file = file }
        func read() -> [PairingRecord] { file.read() }
        func upsert(pairing: PairingRecord) -> Bool { file.upsert(pairing) }
        func remove(connectionId: String) -> Bool { file.remove(connectionId: connectionId) }
    }

    static let relayURL = "https://relay.example.com"

    private func makePairingsFile() throws -> TestPairingsFile {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("deletion-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return TestPairingsFile(url: dir.appendingPathComponent("push-pairings.json"))
    }

    private func cleanUp(_ file: TestPairingsFile) {
        try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent())
    }

    private func makeStore() -> (store: ConnectionStore, defaults: UserDefaults, suite: String) {
        let suite = "test.connectiondeletion.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        return (ConnectionStore(defaults: defaults), defaults, suite)
    }

    private func core(_ file: TestPairingsFile) -> HostCareCore {
        HostCareCore(platform: NoHost(), pairings: FilePairings(file), environment: "sandbox", autocareOff: true)
    }

    @Test("Care hears of the delete first, the session ends, the pass is waited out, then the connection goes")
    func theDeletesOrder() async throws {
        let (store, defaults, suite) = makeStore()
        defer { defaults.removePersistentDomain(forName: suite) }
        let connection = ServerConnection(name: "doomed", host: "a.example.com", username: "dev",
                                          keychainRef: "connection-secret")
        store.add(connection)
        let care = Recorder()
        let keychain = Keychain(["connection-secret"])

        try await ConnectionDeletion.delete(
            connection, from: store, keys: SSHKeyStore(defaults: defaults), keychain: keychain, care: care,
            tearDown: { care.steps.append("tear down") },
            dropShares: { _ in
                care.steps.append(store.connections.isEmpty ? "shares after" : "shares")
                care.steps.append(keychain.deleted.isEmpty ? "secret still there" : "secret gone")
            })

        #expect(care.steps == ["began", "forget", "tear down", "wind down", "shares", "secret still there", "ended"])
        // The record and its item go together, the core's one call (B810).
        #expect(keychain.deleted == ["connection-secret"])
        #expect(store.connections.isEmpty)
    }

    /// B327: from the moment care hears of the delete until the record goes,
    /// the delete is under way, and nothing starts a session for it (Home's
    /// card stays tappable for those seconds); after it, nothing is held.
    @Test("A delete is under way from care's hearing of it until the record goes")
    func aDeleteIsUnderWayUntilTheRecordGoes() async throws {
        let (store, defaults, suite) = makeStore()
        defer { defaults.removePersistentDomain(forName: suite) }
        let connection = ServerConnection(name: "doomed", host: "a.example.com", username: "dev",
                                          keychainRef: "connection-secret")
        store.add(connection)
        let care = Recorder()
        #expect(!ConnectionDeletion.isUnderWay(connection.id))
        var during: [Bool] = []
        try await ConnectionDeletion.delete(
            connection, from: store, keys: SSHKeyStore(defaults: defaults),
            keychain: Keychain(["connection-secret"]), care: care,
            tearDown: { during.append(ConnectionDeletion.isUnderWay(connection.id)) },
            dropShares: { _ in during.append(ConnectionDeletion.isUnderWay(connection.id)) })
        #expect(during == [true, true])
        #expect(!ConnectionDeletion.isUnderWay(connection.id), "held after the delete")
    }

    /// B343: a deleted connection's queued shares could never be delivered
    /// (the drain takes an entry only for a live session), and they stayed
    /// in the App Group, photos included, until they aged out. The delete
    /// takes them, and only its own.
    @Test("Deleting a connection drops what the share extension queued for it, and only that")
    func deletingDropsItsQueuedShares() async throws {
        let (store, defaults, suite) = makeStore()
        defer { defaults.removePersistentDomain(forName: suite) }
        let queue = FileManager.default.temporaryDirectory
            .appendingPathComponent("deletion-shares-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: queue) }
        let shares = ShareQueueStore(directory: queue)
        let doomed = ServerConnection(name: "doomed", host: "a.example.com", username: "dev")
        let kept = ServerConnection(name: "kept", host: "b.example.com", username: "dev")
        store.add(doomed)
        store.add(kept)
        try shares.append(target: (doomed.id, "%1"), agentName: "claude", label: "doomed", images: [Data("a".utf8)])
        try shares.append(target: (doomed.id, "%2"), agentName: "codex", label: "doomed", images: [Data("b".utf8)])
        try shares.append(target: (kept.id, "%1"), agentName: "claude", label: "kept", images: [Data("c".utf8)])
        let doomedFiles = shares.load().filter { $0.connectionId == doomed.id }.flatMap(\.files)

        try await ConnectionDeletion.delete(doomed, from: store, keys: SSHKeyStore(defaults: defaults),
                                            keychain: Keychain(), care: Recorder(), tearDown: {},
                                            dropShares: { shares.removeEntries(for: $0) })

        #expect(shares.load().map(\.label) == ["kept"])
        for file in doomedFiles {
            #expect(!FileManager.default.fileExists(atPath: queue.appendingPathComponent(file).path),
                    "a deleted connection's photo stayed")
        }
        #expect(shares.images(for: try #require(shares.load().first)).count == 1, "the kept share lost its photo")
    }

    /// Forgetting is the step that can fail, and everything after it can't be
    /// taken back — so it goes first, and stops the rest.
    @Test("A phone that can't forget the pairing keeps the connection, and the delete says so")
    func unforgettablePairingKeepsTheConnection() async throws {
        let (store, defaults, suite) = makeStore()
        defer { defaults.removePersistentDomain(forName: suite) }
        let connection = ServerConnection(name: "stuck", host: "c.example.com", username: "dev",
                                          keychainRef: "connection-secret")
        store.add(connection)
        let care = Recorder()
        care.cannotForget = true
        var tornDown = false
        let keychain = Keychain(["connection-secret"])
        var sharesDropped = false

        await #expect(throws: PushPairingForgetFailure.self) {
            try await ConnectionDeletion.delete(connection, from: store, keys: SSHKeyStore(defaults: defaults),
                                                keychain: keychain, care: care, tearDown: { tornDown = true },
                                                dropShares: { _ in sharesDropped = true })
        }
        #expect(!sharesDropped, "the queued shares went for a delete that didn't happen")
        #expect(store.connections.map(\.id) == [connection.id],
                "the connection went, leaving a pairing nothing on screen owns")
        #expect(!tornDown, "the session was ended for a delete that didn't happen")
        #expect(keychain.deleted.isEmpty, "the keychain secret went for a delete that didn't happen")
        #expect(care.steps == ["began"])
    }

    @Test("Through the core, deleting a connection forgets its push pairing and only its")
    func deletingForgetsThePairing() async throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (store, defaults, suite) = makeStore()
        defer { defaults.removePersistentDomain(forName: suite) }
        let doomed = ServerConnection(name: "doomed", host: "a.example.com", username: "dev")
        let kept = ServerConnection(name: "kept", host: "b.example.com", username: "dev")
        store.add(doomed)
        store.add(kept)
        try #require(file.write([
            Self.pairing(doomed.id, host: doomed.name),
            Self.pairing(kept.id, host: kept.name),
        ]))

        try await ConnectionDeletion.delete(doomed, from: store, keys: SSHKeyStore(defaults: defaults),
                                            keychain: Keychain(), care: CoreCareHandshake(core: core(file)),
                                            tearDown: {}, dropShares: { _ in })

        #expect(store.connections.map(\.id) == [kept.id])
        #expect(file.read().map(\.connectionId) == [kept.id.uuidString],
                "the deleted connection's push secret is still on the phone")
    }

    @Test("Through the core, a connection that was never paired deletes as before, secret and all")
    func unpairedConnectionDeletes() async throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (store, defaults, suite) = makeStore()
        defer { defaults.removePersistentDomain(forName: suite) }
        let connection = ServerConnection(name: "plain", host: "d.example.com", username: "dev",
                                          keychainRef: "connection-secret")
        store.add(connection)
        let keychain = Keychain(["connection-secret"])

        try await ConnectionDeletion.delete(connection, from: store, keys: SSHKeyStore(defaults: defaults),
                                            keychain: keychain, care: CoreCareHandshake(core: core(file)),
                                            tearDown: {}, dropShares: { _ in })

        #expect(store.connections.isEmpty)
        #expect(keychain.deleted == ["connection-secret"])
        #expect(!FileManager.default.fileExists(atPath: file.url.path),
                "forgetting a pairing that never existed wrote a pairings file")
    }
}
