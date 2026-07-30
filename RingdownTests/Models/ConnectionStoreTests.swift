import Foundation
import Testing
@testable import Ringdown

/// Tests for `ConnectionStore` use an isolated `UserDefaults` suite per test
/// so they never read or pollute the device's standard defaults. Each test
/// removes its persistent domain on exit to guarantee no cross-test leakage.
@Suite("ConnectionStore")
struct ConnectionStoreTests {

    /// Storage key used internally by `ConnectionStore` — duplicated here so
    /// the corrupt-data test can write garbage directly to it.
    private static let storageKey = "ringdown.connections"

    /// Helper that allocates a fresh suite-name and matching `UserDefaults`.
    /// Returns the name so the caller can clean it up via `removePersistentDomain`.
    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.connectionstore.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        return (defaults, name)
    }

    /// Convenience factory for a minimally-populated `ServerConnection`.
    private static func sample(
        name: String = "test-host",
        host: String = "example.com",
        username: String = "alice"
    ) -> ServerConnection {
        ServerConnection(
            name: name,
            host: host,
            username: username
        )
    }

    @Test("a freshly initialised store with empty defaults reports zero connections")
    func emptyOnInit() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let store = ConnectionStore(defaults: defaults)
        #expect(store.connections.isEmpty)
    }

    @Test("add then read returns the added connection")
    func addAndLoad() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let store = ConnectionStore(defaults: defaults)
        let connection = Self.sample(name: "alpha")
        store.add(connection)

        #expect(store.connections.count == 1)
        #expect(store.connections.first?.id == connection.id)
        #expect(store.connections.first?.name == "alpha")
    }

    @Test("update mutates a connection identified by id")
    func updateById() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let store = ConnectionStore(defaults: defaults)
        var connection = Self.sample(name: "original")
        store.add(connection)

        connection.name = "renamed"
        connection.host = "new.example.com"
        store.update(connection)

        #expect(store.connections.count == 1)
        #expect(store.connections.first?.name == "renamed")
        #expect(store.connections.first?.host == "new.example.com")
    }

    @Test("update with an unknown id is a no-op")
    func updateUnknownIdIsNoOp() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let store = ConnectionStore(defaults: defaults)
        let connection = Self.sample(name: "kept")
        store.add(connection)

        let ghost = Self.sample(name: "ghost") // different UUID
        store.update(ghost)

        #expect(store.connections.count == 1)
        #expect(store.connections.first?.name == "kept")
    }

    @Test("delete removes the connection with the given id and is a no-op for unknown ids")
    func deleteById() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let store = ConnectionStore(defaults: defaults)
        let a = Self.sample(name: "a")
        let b = Self.sample(name: "b")
        store.add(a)
        store.add(b)

        store.delete(id: a.id)
        #expect(store.connections.count == 1)
        #expect(store.connections.first?.id == b.id)

        // Deleting a non-existent id should not crash and should leave state unchanged.
        store.delete(id: UUID())
        #expect(store.connections.count == 1)
        #expect(store.connections.first?.id == b.id)
    }

    @Test("connections persist across separate ConnectionStore instances")
    func persistenceAcrossInstances() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let writer = ConnectionStore(defaults: defaults)
        let connection = Self.sample(name: "persisted")
        writer.add(connection)

        let reader = ConnectionStore(defaults: defaults)
        #expect(reader.connections.count == 1)
        #expect(reader.connections.first?.id == connection.id)
        #expect(reader.connections.first?.name == "persisted")
    }

    @Test("corrupt JSON in the storage key recovers to an empty array without crashing")
    func corruptDataRecoversToEmpty() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        // Write garbage to the same key ConnectionStore reads from.
        let garbage = Data("not valid json {{{".utf8)
        defaults.set(garbage, forKey: Self.storageKey)

        let store = ConnectionStore(defaults: defaults)
        #expect(store.connections.isEmpty)
    }
}
