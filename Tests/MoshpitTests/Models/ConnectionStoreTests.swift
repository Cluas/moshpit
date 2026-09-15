import Foundation
import Testing
@testable import Moshpit

/// Tests for `ConnectionStore` use an isolated `UserDefaults` suite per test
/// so they never read or pollute the device's standard defaults. Each test
/// removes its persistent domain on exit to guarantee no cross-test leakage.
@Suite("ConnectionStore")
struct ConnectionStoreTests {

    /// Storage key used internally by `ConnectionStore` — duplicated here so
    /// the corrupt-data test can write garbage directly to it.
    private static let storageKey = "moshpit.connections"

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

    // MARK: - A launch before the first unlock

    // The report: "opening the app, sometimes every connection is gone" — the
    // first-run card over a phone full of hosts. iOS had launched the app
    // unattended before the first unlock after a reboot (prewarming, a push,
    // a Live Activity intent); the preferences file was still sealed and read
    // as nothing; the store believed it. And the first "+" from that card
    // would have written the empty list over everything.

    /// A phone that starts locked (before first unlock) and can be unlocked
    /// mid-test. What its sealed file reads as is simply an empty suite.
    private final class Phone { var unlocked = false }

    private static func readiness(_ phone: Phone) -> DefaultsReadiness {
        DefaultsReadiness { phone.unlocked }
    }

    /// Put `connections` on disk the way a previous, trusted run would have —
    /// without the sentinel, so the suite still looks like a file that was
    /// sealed at launch and became readable later.
    private static func writePayload(_ connections: [ServerConnection], into defaults: UserDefaults) {
        let (scratch, scratchName) = makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchName) }
        let writer = ConnectionStore(defaults: scratch)
        connections.forEach { writer.add($0) }
        defaults.set(scratch.data(forKey: storageKey), forKey: storageKey)
    }

    @Test("a launch before first unlock shows nothing and trusts nothing")
    func sealedLaunchTrustsNothing() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let phone = Phone()

        let store = ConnectionStore(defaults: defaults, readiness: Self.readiness(phone))
        #expect(store.connections.isEmpty)
        #expect(!store.isAuthoritative)
        #expect(defaults.object(forKey: DefaultsReadiness.sentinelKey) == nil,
                "an untrusted read must not claim the file was there")

        // The user unlocks; the file is readable; the app comes forward.
        Self.writePayload([Self.sample(name: "alpha"), Self.sample(name: "bravo")], into: defaults)
        phone.unlocked = true
        store.reloadIfNeeded()
        #expect(store.connections.map(\.name) == ["alpha", "bravo"])
        #expect(store.isAuthoritative)
        #expect(defaults.bool(forKey: DefaultsReadiness.sentinelKey))
    }

    @Test("adding from a first-run card that should not be there keeps what was on disk")
    func addOnUntrustedReadDoesNotClobber() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let phone = Phone()
        let store = ConnectionStore(defaults: defaults, readiness: Self.readiness(phone))

        // Unlocked, but no foreground hop has reloaded the store yet: the tap
        // on "+" is the first thing that happens.
        Self.writePayload([Self.sample(name: "alpha"), Self.sample(name: "bravo")], into: defaults)
        phone.unlocked = true
        store.add(Self.sample(name: "charlie"))

        #expect(store.connections.map(\.name) == ["alpha", "bravo", "charlie"])
        #expect(ConnectionStore(defaults: defaults).connections.count == 3,
                "the old bug wrote [charlie] over alpha and bravo")
    }

    @Test("nothing is written while the file is still sealed")
    func noWriteWhileSealed() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults, readiness: Self.readiness(Phone()))

        store.add(Self.sample(name: "charlie"))
        #expect(store.connections.count == 1)
        #expect(defaults.data(forKey: Self.storageKey) == nil,
                "a sealed file is not an empty list to overwrite")
    }

    @Test("the sentinel makes a locked read trusted: the file was there")
    func sentinelTrustsALockedRead() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        Self.writePayload([Self.sample(name: "alpha")], into: defaults)
        defaults.set(true, forKey: DefaultsReadiness.sentinelKey)

        // Locked (after first unlock, say — a push arrived): the file reads.
        let store = ConnectionStore(defaults: defaults, readiness: Self.readiness(Phone()))
        #expect(store.connections.map(\.name) == ["alpha"])
        #expect(store.isAuthoritative)
    }
}
