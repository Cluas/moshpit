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

    /// A pairings file of the test's own, as the core reaches one.
    final class FilePairings: PairingsFile, @unchecked Sendable {
        let file: PushPairingFile
        init(_ file: PushPairingFile) { self.file = file }
        func read() -> [PairingRecord] { file.read().map(PairingRecord.init) }
        func upsert(pairing: PairingRecord) -> Bool { PushPairing(pairing).map(file.upsert) ?? false }
        func remove(connectionId: String) -> Bool {
            UUID(uuidString: connectionId).map(file.remove(connectionId:)) ?? true
        }
    }

    private static let relayURL = "https://relay.example.com"

    private func makePairingsFile() throws -> PushPairingFile {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("deletion-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return PushPairingFile(url: dir.appendingPathComponent("push-pairings.json"))
    }

    private func cleanUp(_ file: PushPairingFile) {
        try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent())
    }

    private func makeStore() -> (store: ConnectionStore, defaults: UserDefaults, suite: String) {
        let suite = "test.connectiondeletion.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        return (ConnectionStore(defaults: defaults), defaults, suite)
    }

    private func core(_ file: PushPairingFile) -> HostCareCore {
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

        try await ConnectionDeletion.delete(
            connection, from: store, keys: SSHKeyStore(defaults: defaults), care: care,
            tearDown: { care.steps.append("tear down") },
            dropSecret: { _ in care.steps.append(store.connections.isEmpty ? "secret after" : "secret") },
            dropShares: { _ in care.steps.append(store.connections.isEmpty ? "shares after" : "shares") })

        #expect(care.steps == ["began", "forget", "tear down", "wind down", "secret", "shares", "ended"])
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
            connection, from: store, keys: SSHKeyStore(defaults: defaults), care: care,
            tearDown: { during.append(ConnectionDeletion.isUnderWay(connection.id)) },
            dropSecret: { _ in during.append(ConnectionDeletion.isUnderWay(connection.id)) },
            dropShares: { _ in during.append(ConnectionDeletion.isUnderWay(connection.id)) })
        #expect(during == [true, true, true])
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
                                            care: Recorder(), tearDown: {}, dropSecret: { _ in },
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
        var dropped: [String] = []
        var sharesDropped = false

        await #expect(throws: PushPairingForgetFailure.self) {
            try await ConnectionDeletion.delete(connection, from: store, keys: SSHKeyStore(defaults: defaults),
                                                care: care, tearDown: { tornDown = true },
                                                dropSecret: { dropped.append($0) },
                                                dropShares: { _ in sharesDropped = true })
        }
        #expect(!sharesDropped, "the queued shares went for a delete that didn't happen")
        #expect(store.connections.map(\.id) == [connection.id],
                "the connection went, leaving a pairing nothing on screen owns")
        #expect(!tornDown, "the session was ended for a delete that didn't happen")
        #expect(dropped.isEmpty, "the keychain secret went for a delete that didn't happen")
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
            PushPairing.make(connectionId: doomed.id, hostLabel: doomed.name, relayURL: Self.relayURL),
            PushPairing.make(connectionId: kept.id, hostLabel: kept.name, relayURL: Self.relayURL),
        ]))

        try await ConnectionDeletion.delete(doomed, from: store, keys: SSHKeyStore(defaults: defaults),
                                            care: CoreCareHandshake(core: core(file)),
                                            tearDown: {}, dropSecret: { _ in }, dropShares: { _ in })

        #expect(store.connections.map(\.id) == [kept.id])
        #expect(file.read().map(\.connectionId) == [kept.id],
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
        var dropped: [String] = []

        try await ConnectionDeletion.delete(connection, from: store, keys: SSHKeyStore(defaults: defaults),
                                            care: CoreCareHandshake(core: core(file)),
                                            tearDown: {}, dropSecret: { dropped.append($0) },
                                            dropShares: { _ in })

        #expect(store.connections.isEmpty)
        #expect(dropped == ["connection-secret"])
        #expect(!FileManager.default.fileExists(atPath: file.url.path),
                "forgetting a pairing that never existed wrote a pairings file")
    }
}
