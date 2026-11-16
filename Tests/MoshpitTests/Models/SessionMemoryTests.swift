import Foundation
import Testing
@testable import Moshpit

/// B199: what a session remembers about a connection — its last tmux
/// selection (`moshpit.tmux.lastSelection.<id>`) and its host's capabilities
/// (`hostcaps.<id>`) — stayed in UserDefaults after the connection was
/// deleted, for good (an iPad had about 250 such keys). A delete takes them
/// now (the Rust store, crates/store), and what earlier deletes left is
/// swept once per launch — only when the saved connections can be told.
@Suite("Session memory goes with its connection (B199)")
@MainActor
struct SessionMemoryTests {

    /// A suite of the test's own, and a phone that may still be locked.
    final class Bench {
        let name = "r32.session-memory.\(UUID().uuidString)"
        lazy var defaults = UserDefaults(suiteName: name)!
        var unlocked = true
        deinit { UserDefaults.standard.removePersistentDomain(forName: name) }

        func store(rust: Bool) -> ConnectionStore {
            if rust {
                return ConnectionStore(backing: RustConnectionStore(preferences: Prefs(self), mode: .system),
                                       defaults: defaults)
            }
            return ConnectionStore(defaults: defaults, readiness: DefaultsReadiness { [unowned self] in unlocked })
        }

        /// The key the Rust core keeps `id`'s tmux selection under.
        static func selectionKey(_ id: UUID) -> String { "moshpit.tmux.lastSelection.\(id.uuidString)" }

        /// What the two caches write for `id`: the tmux selection as the
        /// core writes it (`crates/store/src/session_memory.rs`), the
        /// capabilities through their own code.
        @MainActor func remember(_ id: UUID) {
            defaults.set(Data(#"{"session":"$1","window":"@2","pane":"%3"}"#.utf8), forKey: Self.selectionKey(id))
            HostCapabilityCache(defaults: defaults)
                .store(HostCapabilities(hasTmux: true, hasMoshServer: true, os: "Linux", packageManager: nil),
                       for: id)
        }

        /// Whether each of the two keys is there for `id`.
        @MainActor func remembers(_ id: UUID) -> [Bool] {
            [defaults.data(forKey: Self.selectionKey(id)) != nil,
             HostCapabilityCache(defaults: defaults).capabilities(for: id) != nil]
        }

        /// Puts `connections` in the suite as a previous trusted run of the
        /// Swift store would have, without the sentinel: on a locked launch
        /// the suite reads as a sealed file.
        func save(_ connections: [ServerConnection]) {
            let scratch = Bench()
            let writer = ConnectionStore(defaults: scratch.defaults)
            connections.forEach { writer.add($0) }
            defaults.set(scratch.defaults.data(forKey: "moshpit.connections"), forKey: "moshpit.connections")
        }
    }

    /// The app's preferences host over the bench's suite, locked or not as
    /// the bench says.
    final class Prefs: PreferencesHost, @unchecked Sendable {
        let bench: Bench
        let inner: RustPreferencesHost
        init(_ bench: Bench) {
            self.bench = bench
            inner = RustPreferencesHost(defaults: bench.defaults, protectedData: ProtectedDataFlag(fixed: true))
        }
        func get(key: String) -> PlistValue? { bench.unlocked ? inner.get(key: key) : nil }
        func set(key: String, value: PlistValue?) { inner.set(key: key, value: value) }
        func protectedDataAvailable() -> Bool { bench.unlocked }
    }

    private static func connection(_ name: String) -> ServerConnection {
        ServerConnection(name: name, host: "\(name).example.com", username: "dev")
    }

    @Test("the two keys are the ones the caches write")
    func keysAreTheCaches() {
        let bench = Bench()
        let id = UUID()
        bench.remember(id)
        let written = Set(bench.defaults.dictionaryRepresentation().keys.filter { $0.hasSuffix(id.uuidString) })
        #expect(written == Set(SessionMemory.keys(id)))
        #expect(SessionMemory.keys(id).allSatisfy { SessionMemory.connection(of: $0) == id })
    }

    @Test("deleting a connection takes what sessions remember about it, and only that", arguments: [false, true])
    func deleteTakesItsMemory(rust: Bool) {
        let bench = Bench()
        let store = bench.store(rust: rust)
        let (gone, kept) = (Self.connection("gone"), Self.connection("kept"))
        store.add(gone)
        store.add(kept)
        bench.remember(gone.id)
        bench.remember(kept.id)

        store.delete(id: gone.id)

        #expect(store.connections.map(\.id) == [kept.id])
        #expect(bench.remembers(gone.id) == [false, false])
        #expect(bench.remembers(kept.id) == [true, true])
    }

    @Test("a delete on a launch before the first unlock takes nothing", arguments: [false, true])
    func sealedDeleteTakesNothing(rust: Bool) {
        let bench = Bench()
        let gone = Self.connection("gone")
        bench.save([gone])
        bench.remember(gone.id)
        bench.unlocked = false
        let store = bench.store(rust: rust)

        store.delete(id: gone.id)

        #expect(bench.remembers(gone.id) == [true, true])
    }

    @Test("the launch sweep takes only the two families of connections no longer saved, once",
          arguments: [false, true])
    func sweepTakesOnlyOrphans(rust: Bool) {
        let bench = Bench()
        let kept = Self.connection("kept")
        bench.save([kept])
        let (gone, alsoGone) = (UUID(), UUID())
        for id in [kept.id, gone, alsoGone] { bench.remember(id) }
        // Not a UUID; another family with a deleted id: neither is judged.
        bench.defaults.set(Data("{}".utf8), forKey: "hostcaps.legacy")
        bench.defaults.set(Data("{}".utf8), forKey: "moshpit.tmux.other.\(gone.uuidString)")

        // Before the first unlock the list reads as empty: every connection
        // would look deleted, so nothing is swept — and the sweep waits.
        bench.unlocked = false
        let store = bench.store(rust: rust)
        #expect(store.sweepSessionMemoryOnce().isEmpty)
        #expect(bench.remembers(gone) == [true, true])

        bench.unlocked = true
        store.reloadIfNeeded()
        let removed = store.sweepSessionMemoryOnce()

        // The suite's keys; a suite's view of the defaults may also hold
        // the test host's own.
        let ours = Set(removed).filter {
            SessionMemory.connection(of: $0).map([kept.id, gone, alsoGone].contains) ?? false
        }
        #expect(ours == Set(SessionMemory.keys(gone) + SessionMemory.keys(alsoGone)))
        #expect(bench.remembers(gone) == [false, false])
        #expect(bench.remembers(alsoGone) == [false, false])
        #expect(bench.remembers(kept.id) == [true, true])
        #expect(bench.defaults.data(forKey: "hostcaps.legacy") != nil)
        #expect(bench.defaults.data(forKey: "moshpit.tmux.other.\(gone.uuidString)") != nil)

        // Once per launch.
        bench.remember(gone)
        #expect(store.sweepSessionMemoryOnce().isEmpty)
        #expect(bench.remembers(gone) == [true, true])
    }
}
