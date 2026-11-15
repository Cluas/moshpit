import Foundation
import Observation

/// The saved connections, as the views watch them: the Rust store
/// (`crates/store/src/connections.rs`, through ``ConnectionStoreBacking``)
/// does every read and write — the stored form 1.0.8 reads, entries this
/// build can't decode carried as stored, the fields earlier builds require
/// written for a rollback, the sealed-launch rule — and this object keeps
/// what the views observe.
///
/// Secrets (passwords, private keys) are not stored here: they live in the
/// keychain, under each connection's `keychainRef`.
@Observable
final class ConnectionStore {
    private(set) var connections: [ServerConnection] = []

    /// True once `connections` came from preferences known to be readable
    /// (the sealed-launch rule): until then the list is shown, never
    /// written over, read again when the app comes forward
    /// (``reloadIfNeeded()``).
    @ObservationIgnored private(set) var isAuthoritative = false

    /// Set by the first ``sweepSessionMemoryOnce()`` that could tell the
    /// saved connections.
    @ObservationIgnored private var sessionMemorySwept = false

    /// The preferences the backing keeps its data in: only to list the
    /// keys ``sweepSessionMemoryOnce()`` hands it.
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let backing: ConnectionStoreBacking

    /// The Rust store over `defaults`, readable as `readiness` says (tests
    /// and scratch suites; the app's is ``AppStores/connections()``).
    convenience init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        let host = RustPreferencesHost(defaults: defaults, available: readiness.protectedDataAvailable)
        self.init(backing: RustConnectionStore(preferences: host, mode: .system), defaults: defaults)
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe.
    init(backing: ConnectionStoreBacking, defaults: UserDefaults = .standard) {
        self.defaults = defaults
        self.backing = backing
        adopt()
    }

    /// The Rust store under this one, for a call that reads more than one
    /// store (``KeychainReferences/findMissing(connections:keys:keychain:)``).
    var rustHandle: ConnectionStoreHandle? { (backing as? RustConnectionStore)?.handle }

    /// Take what the backing holds now, changing only what changed (a
    /// foreground's no-op reload must not redraw every view).
    private func adopt() {
        let held = backing.connections
        if held != connections { connections = held }
        isAuthoritative = backing.isAuthoritative
    }

    // MARK: - CRUD

    // Every mutation re-reads first if the last read could not be trusted
    // (the backing's): the user who taps "+" on a first-run card that should
    // not be there has unlocked the phone to do it, so the real list is
    // readable by now — and the new connection joins it instead of
    // replacing it.

    func add(_ connection: ServerConnection) {
        backing.add(connection)
        adopt()
    }

    /// Pointed at another box, it forgets what sessions remembered about
    /// the old one (B431).
    func update(_ connection: ServerConnection) {
        backing.update(connection)
        adopt()
    }

    /// Connection `id`'s saved record now: what a connect dials and what a
    /// change to one field writes onto, never a view's copy from when it
    /// opened (B429).
    func saved(_ id: UUID) -> ServerConnection? {
        connections.first { $0.id == id }
    }

    /// Switch connection `id` between SSH and mosh on its saved record, and
    /// change nothing else; the record as saved, nil when there is none
    /// (B429: the terminal wrote its own copy back, undoing an edit saved
    /// meanwhile — on the iPad, from the sidebar beside it).
    @discardableResult
    func switchProtocol(of id: UUID) -> ServerConnection? {
        guard var current = saved(id) else { return nil }
        current.connectionProtocol = current.connectionProtocol == .mosh ? .ssh : .mosh
        update(current)
        return saved(id)
    }

    /// Deletes the connection, and what sessions remember about it
    /// (``SessionMemory``: its last tmux selection, its host's capabilities)
    /// once the list is trusted (B199).
    func delete(id: UUID) {
        backing.delete(id: id)
        adopt()
    }

    /// Removes what sessions remember about connections no longer saved:
    /// what deletes left behind before they took it along (B199; an iPad had
    /// about 250 such keys). Once per launch, the first time it can be done
    /// — only when every saved connection can be told (``savedIds``): a
    /// launch before the first unlock reads the list as empty, and would
    /// take every connection's for an orphan. Returns the keys removed.
    @discardableResult
    func sweepSessionMemoryOnce() -> [String] {
        guard !sessionMemorySwept, savedIds != nil else { return [] }
        sessionMemorySwept = true
        let keys = Array(defaults.dictionaryRepresentation().keys.filter { SessionMemory.connection(of: $0) != nil })
        return backing.sweepSessionMemory(keys: keys)
    }

    // MARK: - Reading

    func load() {
        backing.load()
        adopt()
    }

    /// Every connection saved on this phone, by id: the ones shown, and the
    /// ones kept as stored because this build can't decode them. nil when
    /// that can't be known (the read isn't trusted, or something stored
    /// can't be told by id): whoever would let go of something because its
    /// connection is gone must take nil as "any connection may still be
    /// here".
    var savedIds: Set<UUID>? { backing.savedIds }

    /// `connection` signed in (B783): its `lastConnectedAt`, at most once a
    /// day. No build wrote it before, and it is what tells a host connected
    /// to before when a restore took its trusted key. Whether it was
    /// written.
    @discardableResult
    func signedIn(_ connection: ServerConnection) -> Bool {
        guard backing.markConnected(id: connection.id) else { return false }
        adopt()
        return true
    }

    /// Every keychain ref a saved connection holds, carried ones included;
    /// nil when that can't be known, by ``savedIds``' rules. `excluding`
    /// leaves one connection out (the one being deleted).
    func keychainRefs(excluding excluded: UUID? = nil) -> Set<String>? {
        backing.keychainRefs(excluding: excluded)
    }

    /// The names of the saved connections that sign in with the SSH key
    /// `keyId` (or point at its keychain item `ref`), carried ones included.
    /// nil when that can't be known (``keychainRefs``' rules).
    func names(usingKey keyId: UUID, ref: String?) -> [String]? {
        backing.names(usingKey: keyId, ref: ref)
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    /// The app calls this when it comes forward and when protected data
    /// becomes available.
    func reloadIfNeeded() {
        backing.reloadIfNeeded()
        adopt()
    }
}

/// What a ``ConnectionStore`` reads and writes through: the Rust store
/// (crates/store, through crates/ffi; ``RustConnectionStore``), or a test's
/// stand-in. Each member keeps the contract of the store's member of the
/// same name.
protocol ConnectionStoreBacking: AnyObject {
    var connections: [ServerConnection] { get }
    var isAuthoritative: Bool { get }
    func add(_ connection: ServerConnection)
    func update(_ connection: ServerConnection)
    func delete(id: UUID)
    func load()
    func reloadIfNeeded()
    var savedIds: Set<UUID>? { get }
    func keychainRefs(excluding: UUID?) -> Set<String>?
    func names(usingKey keyId: UUID, ref: String?) -> [String]?
    /// Of `keys`, removes the session-memory keys of connections no longer
    /// saved, when that can be told; the keys removed.
    func sweepSessionMemory(keys: [String]) -> [String]
    /// Connection `id` signed in now: its last connect, at most once a day;
    /// whether it was written (B783).
    func markConnected(id: UUID) -> Bool
}

/// What a session remembers about a connection between attempts and app
/// launches, in `UserDefaults` under the connection's id: where the person
/// was in tmux (``TmuxSelectionStore``) and what the host could run
/// (``HostCapabilityCache``). Both go with their connection (B199).
enum SessionMemory {
    /// ``TmuxSelectionStore/key(_:)``'s and ``HostCapabilityCache``'s.
    static let prefixes = ["moshpit.tmux.lastSelection.", "hostcaps."]

    /// Every key remembering something about connection `id`.
    static func keys(_ id: UUID) -> [String] {
        prefixes.map { $0 + id.uuidString }
    }

    /// The connection a key remembers something about: its suffix, when the
    /// key is in one of the two families and the suffix is a UUID. Anything
    /// else is not ours to judge.
    static func connection(of key: String) -> UUID? {
        for prefix in prefixes where key.hasPrefix(prefix) {
            return UUID(uuidString: String(key.dropFirst(prefix.count)))
        }
        return nil
    }

}
