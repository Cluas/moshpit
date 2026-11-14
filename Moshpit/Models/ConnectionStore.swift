import Foundation
import Observation

/// Persists `ServerConnection` metadata to UserDefaults as a JSON-encoded array.
///
/// Secrets (passwords / private keys) are NOT stored here — they live in Keychain,
/// referenced by `ServerConnection.keychainRef`.
///
/// What it reads is only believed when the defaults were readable at the time
/// (``DefaultsReadiness``). A launch before the first unlock after a reboot
/// reads a sealed file as nothing; taken at face value that showed the
/// first-run card over a phone full of hosts, and the first `add` from that
/// card would have written the empty list over them.
///
/// And what it reads is read one connection at a time (``StoredList``): a
/// connection this build cannot decode is skipped, kept as stored and saved
/// back with the rest, instead of turning the whole list into an empty one
/// that the next add, edit or delete wrote over every host.
@Observable
final class ConnectionStore {
    private static let storageKey = "moshpit.connections"
    private static let list = StoredList<ServerConnection>(key: storageKey)

    private(set) var connections: [ServerConnection] = []

    /// True once `connections` came from defaults known to be readable — and,
    /// when the read skipped anything, once what it skipped is safe (see
    /// ``StoredList/secure(_:in:)``). Until then the list is a guess: shown,
    /// never written over, read again when the app comes forward
    /// (``reloadIfNeeded()``).
    @ObservationIgnored private(set) var isAuthoritative = false

    /// Stored connections this build could not decode, exactly as stored.
    /// Not shown; written back after `connections` on every save, so a newer
    /// build that wrote them still has them.
    @ObservationIgnored private var carried: [Any] = []

    /// False when the last read found something other than a list under the
    /// key (see ``StoredList``): whatever connections it held can't be told
    /// apart.
    @ObservationIgnored private var readAList = true

    /// Set by the first ``sweepSessionMemoryOnce()`` that could tell the
    /// saved connections.
    @ObservationIgnored private var sessionMemorySwept = false

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let readiness: DefaultsReadiness
    /// What reads and writes the list when it isn't the code below:
    /// the Rust store, behind the `MOSHPIT_RUST_STORE` switch
    /// (``ConnectionStoreBacking``). nil: the code below, as ever.
    @ObservationIgnored private let backing: ConnectionStoreBacking?

    init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        self.defaults = defaults
        self.readiness = readiness
        backing = nil
        load()
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe. `defaults` — the preferences `backing` keeps
    /// its data in — only lists the keys ``sweepSessionMemoryOnce()`` hands
    /// it; `readiness` goes unused.
    init(backing: ConnectionStoreBacking, defaults: UserDefaults = .standard) {
        self.defaults = defaults
        readiness = .always
        self.backing = backing
        adopt(backing)
    }

    /// Take what `backing` holds now, changing only what changed (a
    /// foreground's no-op reload must not redraw every view).
    private func adopt(_ backing: ConnectionStoreBacking) {
        let held = backing.connections
        if held != connections { connections = held }
        isAuthoritative = backing.isAuthoritative
    }

    // MARK: - CRUD

    // Every mutation re-reads first if the last read could not be trusted:
    // the user who taps "+" on a first-run card that should not be there has
    // unlocked the phone to do it, so the real list is readable by now — and
    // the new connection joins it instead of replacing it.

    func add(_ connection: ServerConnection) {
        if let backing {
            backing.add(connection)
            return adopt(backing)
        }
        reloadIfNeeded()
        connections.append(connection)
        persist()
    }

    func update(_ connection: ServerConnection) {
        if let backing {
            backing.update(connection)
            return adopt(backing)
        }
        reloadIfNeeded()
        guard let index = connections.firstIndex(where: { $0.id == connection.id }) else {
            return
        }
        connections[index] = connection
        persist()
    }

    /// Deletes the connection, and what sessions remember about it
    /// (``SessionMemory``: its last tmux selection, its host's capabilities)
    /// — once the list is trusted, as the delete itself. Those keys used to
    /// stay for good (B199).
    func delete(id: UUID) {
        if let backing {
            backing.delete(id: id)
            return adopt(backing)
        }
        reloadIfNeeded()
        connections.removeAll(where: { $0.id == id })
        persist()
        if isAuthoritative, readiness.isReadable(defaults) {
            for key in SessionMemory.keys(id) { defaults.removeObject(forKey: key) }
        }
    }

    /// Removes what sessions remember about connections no longer saved:
    /// what deletes left behind before they took it along (B199; an iPad had
    /// about 250 such keys). Once per launch, the first time it can be done
    /// — only when every saved connection can be told (``savedIds``) and the
    /// defaults can be believed: a launch before the first unlock reads the
    /// list as empty, and would take every connection's for an orphan. Only
    /// the two key families, only a suffix that is a UUID no saved
    /// connection has. Returns the keys removed.
    @discardableResult
    func sweepSessionMemoryOnce() -> [String] {
        guard !sessionMemorySwept, let saved = savedIds else { return [] }
        let keys = Array(defaults.dictionaryRepresentation().keys.filter { SessionMemory.connection(of: $0) != nil })
        if let backing {
            sessionMemorySwept = true
            return backing.sweepSessionMemory(keys: keys)
        }
        guard readiness.isReadable(defaults) else { return [] }
        sessionMemorySwept = true
        let orphans = SessionMemory.orphans(keys, saved: saved)
        for key in orphans { defaults.removeObject(forKey: key) }
        return orphans
    }

    // MARK: - Persistence

    func load() {
        if let backing {
            backing.load()
            return adopt(backing)
        }
        let readable = readiness.isReadable(defaults)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let contents = Self.list.read(from: defaults, using: decoder)
        connections = contents.elements
        carried = contents.carried
        readAList = contents.isList
        // A read that skipped something is believed only once what it skipped
        // is safe — and securing it writes, so only when readable at all.
        isAuthoritative = readable && Self.list.secure(contents, in: defaults)
        if readable { readiness.markReadable(defaults) }
    }

    /// Every connection saved on this phone, by id: the ones shown, and the
    /// ones kept as stored because this build can't decode them — still the
    /// user's, and a build that can shows them.
    ///
    /// nil when that can't be known: the read isn't trustworthy
    /// (``isAuthoritative`` — a launch before the first unlock reads the
    /// sealed list as empty), or something stored can't be told by id: an
    /// entry with no readable `id`, a value that isn't a list at all.
    /// Whoever would let go of something because its connection is gone must
    /// take nil as "any connection may still be here".
    var savedIds: Set<UUID>? {
        if let backing { return backing.savedIds }
        guard isAuthoritative, readAList else { return nil }
        var ids = Set(connections.map(\.id))
        for entry in carried {
            guard let fields = entry as? [String: Any],
                  let raw = fields["id"] as? String,
                  let id = UUID(uuidString: raw) else { return nil }
            ids.insert(id)
        }
        return ids
    }

    /// Every keychain ref a saved connection holds — the shown ones' and the
    /// carried ones' (a newer build's connection still signs in with its
    /// item). nil when that can't be known, by ``savedIds``' rules, or when
    /// a carried entry holds a `keychainRef` that isn't a string. Whoever
    /// would delete a keychain item because no connection names it must
    /// take nil as "any connection may". `excluding` leaves one connection
    /// out (the one being deleted).
    func keychainRefs(excluding excluded: UUID? = nil) -> Set<String>? {
        if let backing { return backing.keychainRefs(excluding: excluded) }
        guard isAuthoritative, readAList else { return nil }
        var refs = Set(connections.filter { $0.id != excluded }.compactMap(\.keychainRef))
        for entry in carried {
            guard let fields = entry as? [String: Any] else { return nil }
            switch fields["keychainRef"] {
            case nil, is NSNull: continue
            case let ref as String: refs.insert(ref)
            default: return nil
            }
        }
        return refs
    }

    /// The names of the saved connections that sign in with the SSH key
    /// `keyId` (or point at its keychain item `ref`), carried ones included.
    /// nil when that can't be known (``keychainRefs``' rules).
    func names(usingKey keyId: UUID, ref: String?) -> [String]? {
        if let backing { return backing.names(usingKey: keyId, ref: ref) }
        guard isAuthoritative, readAList else { return nil }
        var names = connections
            .filter { $0.sshKeyId == keyId || (ref != nil && $0.keychainRef == ref) }
            .map(\.displayName)
        for entry in carried {
            guard let fields = entry as? [String: Any] else { return nil }
            let byId = (fields["sshKeyId"] as? String).flatMap(UUID.init(uuidString:)) == keyId
            let byRef = ref != nil && fields["keychainRef"] as? String == ref
            guard byId || byRef else { continue }
            let name = fields["name"] as? String ?? ""
            names.append(name.isEmpty
                         ? "\(fields["username"] as? String ?? "")@\(fields["host"] as? String ?? "")"
                         : name)
        }
        return names
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    /// The app calls this when it comes forward and when protected data
    /// becomes available.
    func reloadIfNeeded() {
        if let backing {
            backing.reloadIfNeeded()
            return adopt(backing)
        }
        if !isAuthoritative { load() }
    }

    private func persist() {
        // Never write over a file that could not be read: a sealed file is not
        // an empty list, and the list on disk is the user's.
        guard isAuthoritative else { return }
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        // Encoding a `[ServerConnection]` should never fail; if it does,
        // nothing is written rather than something partial.
        guard let data = Self.list.encode(connections, carrying: carried, using: encoder) else { return }
        defaults.set(Self.addingRollbackFields(to: data), forKey: Self.storageKey)
    }

    // MARK: - Rollback

    /// Fields `ServerConnection` no longer has but every earlier build
    /// REQUIRES on each stored connection (they were non-optional there),
    /// with the values those builds wrote by default. Nothing reads them;
    /// they are written so an older build installed over this one —
    /// TestFlight keeps those a tap away — can still decode the list. It
    /// cannot decode an entry missing any of them, and its `load()` takes an
    /// undecodable list as an empty one, which the next add, edit or delete
    /// then saves over every host the user had. (This build reads entry by
    /// entry and keeps what it can't read; those builds don't.)
    static var rollbackFields: [String: Any] {
        [
            "compression": false,
            "forwardAgent": false,
            "keepAliveInterval": 60,
            "useTailscale": false,
            "tmuxAutoAttach": true,
            "tmuxCreateNew": true,
        ]
    }

    /// `data`, an encoded `[ServerConnection]`, with ``rollbackFields``
    /// filled in wherever an entry lacks them. Entries that aren't objects —
    /// carried as stored, see ``StoredList`` — pass through untouched.
    /// Returned unchanged if it doesn't read as a list.
    ///
    /// Read through ``StoredJSON``, not JSONSerialization alone: every
    /// connection save comes through here, and JSONSerialization dropped a
    /// leading U+FEFF from every name, host and user on the way (B155).
    static func addingRollbackFields(to data: Data) -> Data {
        guard var list = (try? StoredJSON.object(with: data)) as? [Any]
        else { return data }
        for index in list.indices {
            guard var entry = list[index] as? [String: Any] else { continue }
            entry.merge(rollbackFields) { stored, _ in stored }
            list[index] = entry
        }
        return (try? JSONSerialization.data(withJSONObject: list)) ?? data
    }
}

/// What a ``ConnectionStore`` reads and writes through when it isn't its own
/// Swift code: the Rust store (crates/store, through crates/ffi), behind the
/// `MOSHPIT_RUST_STORE` switch (`RustStoreSwitch`). Each member keeps the
/// contract of the store's member of the same name — the same key and
/// format, the sealed-launch rule, carried entries — so the views and the
/// services see one API whichever runs.
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

    /// Of `keys`, those remembering something about a connection not in
    /// `saved` — which must be every connection saved (``ConnectionStore/savedIds``).
    static func orphans(_ keys: [String], saved: Set<UUID>) -> [String] {
        keys.filter { key in connection(of: key).map { !saved.contains($0) } ?? false }
    }
}
