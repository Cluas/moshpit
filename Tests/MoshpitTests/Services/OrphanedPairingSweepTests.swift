import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// Pairings left behind by connections deleted before a delete took its
/// pairing along are forgotten — once per launch, and only when the saved
/// connections can be trusted.
///
/// They stayed: listed under Settings' paired hosts, their secrets on the
/// phone, their send tokens minted at the relay on every pass. The sweep is
/// the core's (`crates/hostcare`); these run it against a real
/// `ConnectionStore` over a scratch suite — whose `savedIds` decide what may
/// go — and a real core over a pairings file of its own.
@MainActor
@Suite("Orphaned push pairings")
struct OrphanedPairingSweepTests {

    private func core(_ file: PushPairingFile) -> HostCareCore {
        HostCareCore(platform: ConnectionDeletionTests.NoHost(),
                     pairings: ConnectionDeletionTests.FilePairings(file),
                     environment: "sandbox", autocareOff: true)
    }

    /// The launch's sweep, as ``HostCare`` runs it: what the store holds,
    /// told; then the sweep. How many it forgot.
    private func sweep(_ core: HostCareCore, _ store: ConnectionStore) -> UInt32 {
        HostCare.tell(core, saved: store)
        return core.forgetOrphanedPairings()
    }

    /// A phone that starts before its first unlock — its preferences sealed —
    /// and can be unlocked mid-test.
    private final class Phone { var unlocked = false }

    private static let storageKey = "moshpit.connections"
    private static let relayURL = "https://relay.example.com"

    private static let alpha = UUID(uuidString: "0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1")!
    private static let newer = UUID(uuidString: "1A2B3C4D-5E6F-4071-8293-A4B5C6D7E8F9")!
    private static let bravo = UUID(uuidString: "2B3C4D5E-6F70-4182-93A4-B5C6D7E8F901")!

    /// Two connections this build reads, and between them one it can't: an
    /// `authMethod` no build has shipped — saved by a newer build, say.
    private static let listWithAnUndecodableConnection = """
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

    private func makeDefaults() -> (UserDefaults, String) {
        let suite = "test.orphanedpairings.\(UUID().uuidString)"
        return (UserDefaults(suiteName: suite)!, suite)
    }

    private func makePairingsFile() throws -> PushPairingFile {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("orphans-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return PushPairingFile(url: dir.appendingPathComponent("push-pairings.json"))
    }

    private func cleanUp(_ file: PushPairingFile) {
        try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent())
    }

    /// One pairing per id, none minted yet — each due a send token.
    private func pair(_ ids: [UUID], in file: PushPairingFile) -> Bool {
        file.write(ids.map {
            PushPairing.make(connectionId: $0, hostLabel: "host-\($0.uuidString.prefix(4))",
                             relayURL: Self.relayURL)
        })
    }

    /// (That its token isn't minted again is the core's test:
    /// `crates/hostcare/tests/push.rs`.)
    @Test("A pairing whose connection is gone is forgotten")
    func orphanIsForgotten() throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (defaults, suite) = makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = ConnectionStore(defaults: defaults)
        let kept = ServerConnection(name: "kept", host: "a.example.com", username: "dev")
        store.add(kept)
        let gone = UUID()   // deleted by a build that left its pairing behind
        try #require(pair([kept.id, gone], in: file))
        let core = core(file)

        #expect(sweep(core, store) == 1)
        #expect(file.read().map(\.connectionId) == [kept.id],
                "a deleted connection's pairing is still on the phone")
    }

    /// A connection this build can't decode is still stored — kept as it was,
    /// for the build that wrote it — so it isn't gone, and neither is its
    /// pairing.
    @Test("A connection this build can't decode keeps its pairing")
    func undecodableConnectionKeepsItsPairing() throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (defaults, suite) = makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(Data(Self.listWithAnUndecodableConnection.utf8), forKey: Self.storageKey)
        let store = ConnectionStore(defaults: defaults)
        try #require(store.connections.map(\.id) == [Self.alpha, Self.bravo])
        let gone = UUID()
        try #require(pair([Self.alpha, Self.newer, Self.bravo, gone], in: file))
        let core = core(file)

        #expect(sweep(core, store) == 1)
        #expect(file.read().map(\.connectionId) == [Self.alpha, Self.newer, Self.bravo],
                "the pairing of a connection this build merely can't decode was forgotten")
    }

    /// An entry with no id can't be matched to any pairing — any of them may
    /// be its.
    @Test("A stored connection with no readable id stops the sweep")
    func unidentifiableConnectionStopsTheSweep() throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (defaults, suite) = makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = """
        [{"id":"0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1","name":"alpha","host":"alpha.example.com",
          "port":22,"username":"dev","authMethod":"password","connectionProtocol":"ssh","sshPort":22,
          "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-01T09:00:00Z",
          "useTmux":true},
         {"name":"mystery","host":"mystery.example.com"}]
        """
        defaults.set(Data(list.utf8), forKey: Self.storageKey)
        let store = ConnectionStore(defaults: defaults)
        try #require(store.isAuthoritative)
        try #require(pair([Self.alpha, UUID()], in: file))
        let core = core(file)

        #expect(store.savedIds == nil)
        #expect(sweep(core, store) == 0)
        #expect(file.read().count == 2)
    }

    @Test("A stored value that isn't a list at all stops the sweep")
    func nonListStopsTheSweep() throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (defaults, suite) = makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(Data(#"{"not":"a list"}"#.utf8), forKey: Self.storageKey)
        let store = ConnectionStore(defaults: defaults)
        try #require(store.connections.isEmpty)
        try #require(pair([UUID(), UUID()], in: file))
        let core = core(file)

        #expect(store.savedIds == nil, "an unreadable value read as no connections at all")
        #expect(sweep(core, store) == 0)
        #expect(file.read().count == 2)
    }

    /// Before the first unlock after a reboot the preferences are sealed, and
    /// the list reads as empty: every pairing would look orphaned.
    @Test("A launch that can't trust the saved connections yet sweeps nothing, then sweeps once it can")
    func lockedLaunchSweepsNothing() throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (defaults, suite) = makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let phone = Phone()
        let store = ConnectionStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
        let kept = ServerConnection(name: "kept", host: "a.example.com", username: "dev")
        let gone = UUID()
        try #require(pair([kept.id, gone], in: file))
        let core = core(file)

        #expect(sweep(core, store) == 0)
        #expect(file.read().map(\.connectionId) == [kept.id, gone],
                "a sealed list swept every pairing on the phone")

        // Unlocked: the list on disk is readable, and the app comes forward.
        let (scratch, scratchSuite) = makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchSuite) }
        ConnectionStore(defaults: scratch).add(kept)
        defaults.set(scratch.data(forKey: Self.storageKey), forKey: Self.storageKey)
        phone.unlocked = true
        store.reloadIfNeeded()

        #expect(sweep(core, store) == 1)
        #expect(file.read().map(\.connectionId) == [kept.id])
    }

    @Test("The sweep runs once a launch")
    func sweepsOnce() throws {
        let file = try makePairingsFile()
        defer { cleanUp(file) }
        let (defaults, suite) = makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = ConnectionStore(defaults: defaults)
        let kept = ServerConnection(name: "kept", host: "a.example.com", username: "dev")
        store.add(kept)
        let first = UUID()
        try #require(pair([kept.id, first], in: file))
        let core = core(file)
        #expect(sweep(core, store) == 1)
        #expect(file.read().map(\.connectionId) == [kept.id])

        let later = UUID()
        try #require(file.upsert(PushPairing.make(connectionId: later, hostLabel: "later",
                                                  relayURL: Self.relayURL)))
        #expect(sweep(core, store) == 0)
        #expect(file.read().map(\.connectionId) == [kept.id, later])
    }
}
