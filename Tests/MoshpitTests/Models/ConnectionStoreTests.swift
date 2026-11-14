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

    // MARK: - An entry this build can't read

    // The old load decoded the list in one piece: one entry it could not
    // decode — a field a newer build added as required, an enum case an
    // older build never heard of — made the WHOLE list read as empty, and
    // the store trusted that. The next add, edit or delete then saved the
    // empty list over every host.

    /// Two hosts this build reads, and between them one it can't: an
    /// `authMethod` no build has shipped.
    private static let listWithAnUnreadableHost = """
    [{"id":"0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1","name":"alpha","host":"alpha.example.com",
      "port":22,"username":"dev","authMethod":"password","connectionProtocol":"ssh","sshPort":22,
      "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-01T09:00:00Z",
      "useTmux":true},
     {"id":"1A2B3C4D-5E6F-4071-8293-A4B5C6D7E8F9","name":"from-a-newer-build","host":"new.example.com",
      "port":22,"username":"dev","authMethod":"certificate","connectionProtocol":"ssh","sshPort":22,
      "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-02T09:00:00Z",
      "useTmux":true,"certificateRef":"cert-1"},
     {"id":"2B3C4D5E-6F70-4182-93A4-B5C6D7E8F901","name":"bravo","host":"bravo.example.com",
      "port":2222,"username":"ops","authMethod":"key","connectionProtocol":"mosh","sshPort":2222,
      "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-03T09:00:00Z",
      "useTmux":false}]
    """

    /// The raw stored entries, as objects — what another build would read.
    private static func storedEntries(_ defaults: UserDefaults) throws -> [[String: Any]] {
        let data = try #require(defaults.data(forKey: storageKey))
        return try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
    }

    @Test("a host this build can't decode is skipped, not the whole list")
    func unreadableHostSkipsOnlyItself() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(Data(Self.listWithAnUnreadableHost.utf8), forKey: Self.storageKey)

        let store = ConnectionStore(defaults: defaults)

        #expect(store.connections.map(\.name) == ["alpha", "bravo"],
                "the old load read this whole list as empty")
        #expect(store.isAuthoritative)
    }

    @Test("saving after a skip keeps the skipped host exactly as stored")
    func saveKeepsTheUnreadableHost() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(Data(Self.listWithAnUnreadableHost.utf8), forKey: Self.storageKey)
        let store = ConnectionStore(defaults: defaults)

        store.add(Self.sample(name: "charlie"))
        var bravo = try #require(store.connections.first { $0.name == "bravo" })
        bravo.host = "bravo2.example.com"
        store.update(bravo)

        // Everything this build reads, and the host it can't, still stored.
        let entries = try Self.storedEntries(defaults)
        #expect(entries.compactMap { $0["name"] as? String }.sorted()
                == ["alpha", "bravo", "charlie", "from-a-newer-build"],
                "the old store saved [charlie] over all three")
        let kept = try #require(entries.first { $0["name"] as? String == "from-a-newer-build" })
        #expect(kept["authMethod"] as? String == "certificate")
        #expect(kept["certificateRef"] as? String == "cert-1", "its fields this build doesn't know survive too")
        // And the next launch reads the same list again.
        #expect(ConnectionStore(defaults: defaults).connections.map(\.name).sorted()
                == ["alpha", "bravo", "charlie"])
        #expect(ConnectionStore(defaults: defaults).connections.first { $0.name == "bravo" }?.host
                == "bravo2.example.com")
    }

    @Test("deleting every host this build can see still keeps the one it can't")
    func deleteAllVisibleKeepsTheUnreadableHost() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(Data(Self.listWithAnUnreadableHost.utf8), forKey: Self.storageKey)
        let store = ConnectionStore(defaults: defaults)

        for connection in store.connections { store.delete(id: connection.id) }

        #expect(store.connections.isEmpty)
        #expect(try Self.storedEntries(defaults).compactMap { $0["name"] as? String }
                == ["from-a-newer-build"])
    }

    @Test("the first list with a skipped host is backed up untouched before anything is saved")
    func skippedHostListIsBackedUp() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let original = Data(Self.listWithAnUnreadableHost.utf8)
        defaults.set(original, forKey: Self.storageKey)

        let store = ConnectionStore(defaults: defaults)
        store.add(Self.sample(name: "charlie"))

        let backups = try #require(defaults.array(forKey: Self.storageKey + ".backup"))
        #expect(backups.count == 1)
        #expect(backups.first as? Data == original, "byte for byte what was stored")
    }

    @Test("a value that isn't a list at all is backed up before the first save replaces it")
    func garbageIsBackedUpBeforeItIsReplaced() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let garbage = Data("not valid json {{{".utf8)
        defaults.set(garbage, forKey: Self.storageKey)

        let store = ConnectionStore(defaults: defaults)
        #expect(store.connections.isEmpty)
        store.add(Self.sample(name: "charlie"))

        #expect(ConnectionStore(defaults: defaults).connections.map(\.name) == ["charlie"])
        let backups = try #require(defaults.array(forKey: Self.storageKey + ".backup"),
                                   "the old store wrote over it and kept nothing")
        #expect(backups.first as? Data == garbage)

        // Reading the same garbage again doesn't stack a second copy.
        defaults.set(garbage, forKey: Self.storageKey)
        _ = ConnectionStore(defaults: defaults)
        #expect(defaults.array(forKey: Self.storageKey + ".backup")?.count == 1)
    }

    @Test("a skip on a launch that can't trust the file writes nothing, not even the backup")
    func skipWhileSealedWritesNothing() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let original = Data(Self.listWithAnUnreadableHost.utf8)
        defaults.set(original, forKey: Self.storageKey)
        let phone = Phone()

        let store = ConnectionStore(defaults: defaults, readiness: Self.readiness(phone))
        #expect(store.connections.map(\.name) == ["alpha", "bravo"])
        #expect(!store.isAuthoritative)
        store.add(Self.sample(name: "charlie"))
        #expect(defaults.data(forKey: Self.storageKey) == original)
        #expect(defaults.object(forKey: Self.storageKey + ".backup") == nil)

        // Unlocked: the next change reads again, keeps the copy, then saves.
        phone.unlocked = true
        store.add(Self.sample(name: "delta"))
        #expect(store.isAuthoritative)
        #expect(try Self.storedEntries(defaults).compactMap { $0["name"] as? String }.sorted()
                == ["alpha", "bravo", "delta", "from-a-newer-build"])
        #expect(defaults.array(forKey: Self.storageKey + ".backup")?.first as? Data == original)
    }

    // MARK: - Rollback

    /// What an earlier build's synthesized decoder demands of every stored
    /// connection: each field that was non-optional there. One this build
    /// stops writing makes the whole list undecodable to it — and its
    /// `load()` reads an undecodable list as empty, then saves over it.
    private struct EarlierBuildConnection: Decodable {
        let id: UUID
        let name: String
        let host: String
        let port: Int
        let username: String
        let authMethod: String
        let connectionProtocol: String
        let sshPort: Int
        let moshPortRangeStart: Int
        let moshPortRangeEnd: Int
        let createdAt: Date
        let forwardAgent: Bool
        let compression: Bool
        let keepAliveInterval: Int
        let useTailscale: Bool
        let tmuxAutoAttach: Bool
        let tmuxCreateNew: Bool
        let useTmux: Bool
    }

    @Test("what this build saves still decodes in an earlier build, fields it dropped included")
    func savedListSurvivesARollback() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let store = ConnectionStore(defaults: defaults)
        store.add(Self.sample(name: "alpha"))
        store.add(Self.sample(name: "bravo"))

        let data = try #require(defaults.data(forKey: Self.storageKey))
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let earlier = try decoder.decode([EarlierBuildConnection].self, from: data)
        #expect(earlier.map(\.name) == ["alpha", "bravo"])
        // …and this build still reads its own list back.
        #expect(ConnectionStore(defaults: defaults).connections.map(\.name) == ["alpha", "bravo"])
    }

    /// `moshpit.connections` as an earlier build wrote it: every field the
    /// model has since dropped is there, set away from its default, next to
    /// the ones that are still live. No `multiplexerRaw`: it predates the
    /// picker, so `useTmux` is what decides.
    private static let earlierBuildList = """
    [{
      "id": "5D1A7E0C-3F42-4B8E-9C61-0A2B3C4D5E6F",
      "name": "studio",
      "host": "studio.example.com",
      "port": 2222,
      "username": "dev",
      "authMethod": "key",
      "connectionProtocol": "mosh",
      "sshPort": 2222,
      "moshPortRangeStart": 60001,
      "moshPortRangeEnd": 60999,
      "moshServerPath": "/opt/homebrew/bin/mosh-server",
      "keychainRef": "ref-studio",
      "createdAt": "2026-08-27T09:00:00Z",
      "lastConnectedAt": "2026-11-01T12:30:00Z",
      "startupCommand": "cd ~/code",
      "forwardAgent": true,
      "compression": true,
      "keepAliveInterval": 30,
      "jumpHost": "bastion.example.com",
      "useTailscale": true,
      "tmuxAutoAttach": false,
      "tmuxCreateNew": false,
      "useTmux": true,
      "predictModeRaw": "always",
      "roamOnCellular": false
    }]
    """

    @Test("a list an earlier build stored, with every field since dropped, still loads")
    func earlierBuildListLoads() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(Data(Self.earlierBuildList.utf8), forKey: Self.storageKey)

        let store = ConnectionStore(defaults: defaults)

        #expect(store.connections.count == 1, "an undecodable list loads as empty")
        let loaded = try #require(store.connections.first)
        #expect(loaded.id == UUID(uuidString: "5D1A7E0C-3F42-4B8E-9C61-0A2B3C4D5E6F"))
        #expect(loaded.name == "studio")
        #expect(loaded.host == "studio.example.com")
        #expect(loaded.port == 2222)
        #expect(loaded.username == "dev")
        #expect(loaded.authMethod == .key)
        #expect(loaded.connectionProtocol == .mosh)
        #expect(loaded.moshServerPath == "/opt/homebrew/bin/mosh-server")
        #expect(loaded.keychainRef == "ref-studio")
        #expect(loaded.lastConnectedAt != nil)
        #expect(loaded.predictMode == .always)
        #expect(loaded.roamOnCellular == false)
        #expect(loaded.multiplexer == .tmux)

        // Saving it again keeps it readable by that earlier build.
        store.update(loaded)
        let data = try #require(defaults.data(forKey: Self.storageKey))
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let earlier = try decoder.decode([EarlierBuildConnection].self, from: data)
        #expect(earlier.map(\.name) == ["studio"])
        #expect(earlier.first?.useTmux == true)
    }

    /// B429: the terminal kept its own copy of the connection from when it
    /// opened and wrote it back whole on a SSH ⇄ mosh switch — on the iPad
    /// an edit saved from the sidebar beside it (another host, user, key)
    /// was undone, and the live session pointed back at the old box. The
    /// switch now changes the protocol alone, on the saved record.
    @Test("switching the protocol keeps an edit saved since the terminal opened")
    func switchingTheProtocolKeepsAnEditSavedSince() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ConnectionStore(defaults: defaults)
        let opened = Self.sample(host: "old.example.com")
        store.add(opened)
        // Edited and saved from the sidebar while the terminal is open.
        var edited = opened
        edited.host = "new.example.com"
        edited.username = "bob"
        store.update(edited)

        let switched = try #require(store.switchProtocol(of: opened.id))

        #expect(switched.connectionProtocol == .mosh)
        #expect(switched.host == "new.example.com")
        #expect(switched.username == "bob")
        let reread = ConnectionStore(defaults: defaults)
        let stored = try #require(reread.saved(opened.id))
        #expect(stored.host == "new.example.com")
        #expect(stored.connectionProtocol == .mosh)
        // What the old switch wrote: the copy from when it opened.
        #expect(opened.host == "old.example.com")
        let unknown = store.switchProtocol(of: UUID())
        #expect(unknown == nil)
    }
}
