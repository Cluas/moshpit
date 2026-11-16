import Foundation
import MoshpitKit
import Observation

// MARK: - SSH key records (prototype screens 8/9)

enum SSHKeySource: String, Codable {
    case secureEnclave
    case generated
    case imported
    case hardware

    var sectionTitle: String {
        switch self {
        case .secureEnclave: return "THIS DEVICE · SECURE ENCLAVE"
        default: return "IMPORTED"
        }
    }
}

enum SSHKeyAlgorithm: String, Codable, CaseIterable {
    case ed25519 = "ED25519"
    case ecdsaSK = "ECDSA-sk"
    /// Any RSA key, whatever its size: "RSA-4096" is what every build has
    /// stored for RSA, so it stays. The real size is ``SSHKeyRecord/bits``.
    case rsa4096 = "RSA-4096"
    case seP256 = "SE · P256"
    /// An imported software ECDSA key (`ecdsa-sha2-nistp*`) — neither a
    /// security key (`ecdsaSK`) nor the Secure Enclave (`seP256`).
    ///
    /// Newer than every released build, which can't decode it: a stored key
    /// record names it only in `keyAlgorithm` (see ``SSHKeyRecord``).
    case ecdsa = "ECDSA"

    /// What a stored record says in `algorithm`, the field every build reads:
    /// a value every build decodes. Older builds labelled every ECDSA key
    /// ECDSA-sk, so that is what they get.
    var olderBuildsName: SSHKeyAlgorithm {
        self == .ecdsa ? .ecdsaSK : self
    }
}

/// Public metadata for one key. Private material lives in the Keychain
/// (or inside the Secure Enclave), referenced by `keychainRef`.
///
/// Stored with two algorithm fields (``CodingKeys``): `algorithm`, which every
/// build reads and which is always one older builds decode
/// (``SSHKeyAlgorithm/olderBuildsName``), and `keyAlgorithm`, the real one,
/// which this build prefers. Builds before this one read the key list in one
/// piece and took a record they couldn't decode as NO keys at all, then saved
/// that empty list over every key on the next change; an "ECDSA" in
/// `algorithm` was enough.
struct SSHKeyRecord: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String
    /// The real algorithm, whatever `algorithm` says in storage.
    var algorithm: SSHKeyAlgorithm
    /// An RSA key's modulus size, which is what its badge shows. Stored
    /// beside `algorithm`, whose "RSA-4096" every build reads; nil when not
    /// known (nor RSA).
    var bits: Int?
    var comment: String = ""
    /// "SHA256:…" fingerprint of the public key blob.
    var fingerprint: String = ""
    /// Full `authorized_keys` line.
    var publicKey: String = ""
    var source: SSHKeySource
    var boundHosts: [String] = []
    var keychainRef: String?
    var requireBiometry: Bool = true
    var createdAt = Date()
    var lastUsedAt: Date?
    /// The kind of key it is when no connect path signs in with it
    /// ("ECDSA brainpoolP256r1", "DSA"), read off its private key (B780):
    /// one 1.0.8 imported without its public half. nil: it signs in, or it
    /// hasn't been read.
    var refused: String?

    /// Its private half isn't in this device's keychain (B781): a phone
    /// restored from a backup has the record and not the key — every item
    /// the app writes stays on its device, and a Secure Enclave key never
    /// leaves its chip. Told by the store (``SSHKeyStore/setMissing(_:)``),
    /// never stored.
    var missing = false

    /// The kind of key a connect path can't sign in with, when this is one:
    /// what its private key said (``refused``), else what its public key
    /// line says (``StoredKey/refusedType(ofPublicKeyLine:)``).
    var refusedType: String? {
        refused ?? StoredKey.refusedType(ofPublicKeyLine: publicKey)
    }

    var badgeText: String {
        if source == .secureEnclave { return "SE · \(algorithm == .seP256 ? "P256" : algorithm.rawValue)" }
        // Every RSA key used to read RSA-4096, the one RSA name the enum has.
        if algorithm == .rsa4096 { return bits.map { "RSA-\($0)" } ?? "RSA" }
        return algorithm.rawValue
    }
}

extension SSHKeyRecord {
    private enum CodingKeys: String, CodingKey {
        case id, name, algorithm, comment, fingerprint, publicKey, source, boundHosts,
             keychainRef, requireBiometry, createdAt, lastUsedAt, refused
        /// The real algorithm, where `algorithm` holds what older builds read.
        case keyAlgorithm
        case bits
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        comment = try container.decode(String.self, forKey: .comment)
        fingerprint = try container.decode(String.self, forKey: .fingerprint)
        let publicKey = try container.decode(String.self, forKey: .publicKey)
        self.publicKey = publicKey
        source = try container.decode(SSHKeySource.self, forKey: .source)
        boundHosts = try container.decode([String].self, forKey: .boundHosts)
        keychainRef = try container.decodeIfPresent(String.self, forKey: .keychainRef)
        requireBiometry = try container.decode(Bool.self, forKey: .requireBiometry)
        createdAt = try container.decode(Date.self, forKey: .createdAt)
        lastUsedAt = try container.decodeIfPresent(Date.self, forKey: .lastUsedAt)
        refused = (try? container.decodeIfPresent(String.self, forKey: .refused)).flatMap { $0 }
        // Unreadable here means unreadable everywhere: the store carries the
        // record as stored, not guessed at.
        let olderBuildsName = try container.decode(SSHKeyAlgorithm.self, forKey: .algorithm)
        // A real algorithm from a build newer than this one may be one this
        // build has no name for; then it reads what older builds read.
        let real = (try? container.decodeIfPresent(String.self, forKey: .keyAlgorithm))
            .flatMap { $0 }
            .flatMap(SSHKeyAlgorithm.init(rawValue:))
        let algorithm = real ?? Self.recoveredAlgorithm(olderBuildsName: olderBuildsName, publicKey: publicKey,
                                                        source: source)
        self.algorithm = algorithm
        // A record from before the size was kept (or one an older build saved
        // back without it): the public key line still says.
        bits = (try? container.decodeIfPresent(Int.self, forKey: .bits)).flatMap { $0 }
            ?? (algorithm == .rsa4096 ? SSHKeyFactory.rsaBits(ofPublicKeyLine: publicKey) : nil)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(name, forKey: .name)
        try container.encode(algorithm.olderBuildsName, forKey: .algorithm)
        try container.encode(algorithm, forKey: .keyAlgorithm)
        try container.encodeIfPresent(bits, forKey: .bits)
        try container.encode(comment, forKey: .comment)
        try container.encode(fingerprint, forKey: .fingerprint)
        try container.encode(publicKey, forKey: .publicKey)
        try container.encode(source, forKey: .source)
        try container.encode(boundHosts, forKey: .boundHosts)
        try container.encodeIfPresent(keychainRef, forKey: .keychainRef)
        try container.encode(requireBiometry, forKey: .requireBiometry)
        try container.encode(createdAt, forKey: .createdAt)
        try container.encodeIfPresent(lastUsedAt, forKey: .lastUsedAt)
        try container.encodeIfPresent(refused, forKey: .refused)
    }

    /// The algorithm of a record with no (readable) `keyAlgorithm`: one an
    /// older build saved, dropping the field it doesn't know. Its ECDSA-sk
    /// covers every ECDSA key; the public key line says which ones are plain
    /// ECDSA, and so is one with no line at all (B726): a SEC1 EC key 1.0.8
    /// imported without its public half — 1.0.8 never labelled a security
    /// key ECDSA-sk. A hardware key saved back by an older build keeps its
    /// label. The Rust store's `recovered_algorithm`.
    private static func recoveredAlgorithm(olderBuildsName: SSHKeyAlgorithm, publicKey: String,
                                           source: SSHKeySource) -> SSHKeyAlgorithm {
        guard olderBuildsName == .ecdsaSK, source != .hardware,
              publicKey.isEmpty || publicKey.hasPrefix("ecdsa-sha2-") else { return olderBuildsName }
        return .ecdsa
    }
}

// MARK: - Store

/// The SSH key records, as the views watch them: the Rust store
/// (`crates/store/src/ssh_keys.rs`, through ``SSHKeyStoreBacking``) does
/// every read and write — the stored form every build reads, records this
/// build can't decode carried as stored (dropping one orphaned its private
/// key in the keychain), the sealed-launch rule — and this object keeps what
/// the views observe.
@Observable
final class SSHKeyStore {
    private(set) var keys: [SSHKeyRecord] = []

    /// True once `keys` came from preferences known to be readable and
    /// anything the read skipped is safe; until then nothing here is
    /// written back.
    @ObservationIgnored private(set) var isAuthoritative = false

    /// Internal, not private: the calls over both stores take it
    /// (``ConnectionStore``'s delete and release).
    @ObservationIgnored let backing: SSHKeyStoreBacking

    /// Set once a launch's recovery has looked
    /// (``recoverOrphansOnce(connections:keychain:reader:)``).
    @ObservationIgnored private var orphansLooked = false

    /// Set once a launch has put right what keys show
    /// (``learnKeyFactsOnce(keychain:)``).
    @ObservationIgnored private var factsLearned = false

    /// The Rust store over `defaults`, readable as `protectedData` says (the core's readiness rule) (tests
    /// and scratch suites; the app's is ``AppStores/sshKeys()``).
    convenience init(defaults: UserDefaults = .standard, protectedData: @escaping () -> Bool = { true }) {
        let host = RustPreferencesHost(defaults: defaults, available: protectedData)
        self.init(backing: RustSSHKeyStore(preferences: host, mode: .system))
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe.
    init(backing: SSHKeyStoreBacking) {
        self.backing = backing
        adopt()
    }

    /// Take what the backing holds now, changing only what changed.
    private func adopt() {
        var held = backing.keys
        for index in held.indices where missingIds.contains(held[index].id) {
            held[index].missing = true
        }
        if held != keys { keys = held }
        isAuthoritative = backing.isAuthoritative
    }

    /// The keys whose private half isn't in this device's keychain (B781),
    /// as ``KeychainReferences/findMissing(connections:keys:keychain:)`` last
    /// told; never stored.
    @ObservationIgnored private var missingIds: Set<UUID> = []

    /// These keys' private halves aren't here (B781): each record says so
    /// (``SSHKeyRecord/missing``) and SSH Keys groups them apart.
    func setMissing(_ ids: Set<UUID>) {
        guard ids != missingIds else { return }
        missingIds = ids
        adopt()
    }

    /// The Rust store under this one, for a call that reads more than one
    /// store (``KeychainReferences/findMissing(connections:keys:keychain:)``).
    var rustHandle: KeyStoreHandle? { (backing as? RustSSHKeyStore)?.handle }

    var deviceKeys: [SSHKeyRecord] { keys.filter { $0.source == .secureEnclave && !$0.missing } }
    var importedKeys: [SSHKeyRecord] { keys.filter { $0.source != .secureEnclave && !$0.missing } }
    /// The keys whose private half isn't on this device (B781).
    var missingKeys: [SSHKeyRecord] { keys.filter(\.missing) }

    /// Every keychain ref a stored key record holds, the carried ones' too —
    /// each the only handle on a private key. nil when that can't be known
    /// (a read that can't be trusted, a stored value that isn't a list, a
    /// carried entry whose `keychainRef` can't be read). `excluding` leaves
    /// one record out (the key being deleted).
    func keychainRefs(excluding excluded: UUID? = nil) -> Set<String>? {
        backing.keychainRefs(excluding: excluded)
    }

    func add(_ record: SSHKeyRecord) {
        backing.add(record)
        adopt()
    }

    /// The record alone. Deleting a key is ``delete(id:connections:keychain:)``:
    /// the record leaving here never took its private key with it (B232).
    func remove(id: UUID) {
        backing.remove(id: id)
        adopt()
    }

    /// What deleting key `id` would do now (`secrets::check_key`): whether a
    /// saved connection still signs in with it. nil when the stores
    /// couldn't be asked.
    func checkDelete(id: UUID, connections: ConnectionStore) -> StoreKeyCheck? {
        connections.reloadIfNeeded()
        let check = backing.checkDelete(id: id, connections: connections.backing)
        adopt()
        return check
    }

    /// Delete key `id`: its private key out of `keychain` first, then its
    /// record — unless a saved connection still signs in with it, its users
    /// can't be told, or the keychain keeps the item; the item stays, the
    /// record going, only while another record names it
    /// (`secrets::delete_key`, B232). nil when the stores couldn't be asked.
    func delete(id: UUID, connections: ConnectionStore, keychain: KeychainHost) -> StoreKeyDeletion? {
        let deletion = backing.deleteKey(id: id, connections: connections.backing, keychain: keychain)
        adopt()
        connections.reloadIfNeeded()
        return deletion
    }

    /// Give every private key earlier builds orphaned a key record again
    /// (B232): once a launch, the first time both stores can be trusted.
    /// The core's recovery (`secrets::recover_orphans`, B810): only a
    /// `moshpit.secret.` item nothing names, not made this launch and not
    /// listed to drop (B428), read without asking, that reads as a private
    /// key — the core reads an OpenSSH or PEM key itself and asks `reader`
    /// about the rest (a Secure Enclave key's handle). "Recovered key" and
    /// the start of its fingerprint, in the app's language. Nothing is ever
    /// deleted or rewritten. Returns the records added.
    @discardableResult
    func recoverOrphansOnce(connections: ConnectionStore, keychain: KeychainHost,
                            reader: StoredKeyReader? = EnclaveKeyReader()) -> [SSHKeyRecord] {
        guard !orphansLooked else { return [] }
        // The app's own words, with the core's slot where the fingerprint
        // goes: interpolated, it is the catalog's "Recovered key %@".
        let slot = "\u{FFFC}"
        let template = String(localized: "Recovered key \(slot)").replacingOccurrences(of: slot, with: "%@")
        switch backing.recoverOrphans(connections: connections.backing, keychain: keychain, reader: reader,
                                      nameTemplate: template, now: Date()) {
        case .recovered(let added)?:
            orphansLooked = true
            adopt()
            if !added.isEmpty {
                Log.store.notice("keychain recovery: \(added.count, privacy: .public) orphaned private key(s) listed in SSH Keys again")
            }
            return added.compactMap(SSHKeyRecord.init)
        case .unknown?:
            Log.store.info("keychain recovery: which items are named can't be told yet — not looking")
            return []
        case .keychainFailed(let message)?:
            // Locked again, or a keychain that can't list: the next time.
            Log.store.info("keychain recovery: couldn't list the items (\(message, privacy: .public))")
            return []
        case nil:
            return []
        }
    }

    /// What each key 1.0.8 imported without its public half is (B726):
    /// ``learnKeyFacts(keychain:)`` once a launch, once the key list can be
    /// trusted.
    @discardableResult
    func learnKeyFactsOnce(keychain: KeychainHost) -> [SSHKeyRecord] {
        guard !factsLearned, isAuthoritative else { return [] }
        factsLearned = true
        let learned = learnKeyFacts(keychain: keychain)
        if !learned.isEmpty {
            Log.store.notice("key facts: \(learned.count, privacy: .public) key(s) an earlier build imported now show what they are")
        }
        return learned
    }

    /// Put right what each key 1.0.8 imported without its public half shows
    /// (B726): its type, size, public key and fingerprint, off its own
    /// private key, read from `keychain` without asking. One behind Face ID
    /// waits for its first sign-in (``learnFacts(for:privateKey:)``).
    /// Returns the records changed.
    @discardableResult
    func learnKeyFacts(keychain: KeychainHost) -> [SSHKeyRecord] {
        let learned = backing.learnKeyFacts(keychain: keychain)
        if !learned.isEmpty { adopt() }
        return learned
    }

    /// `connection` read its key for a sign-in — `privateKey`, as read: a
    /// key 1.0.8 imported without its public half takes its type, size,
    /// public key and fingerprint from it (B726). The key is its record by
    /// id, else the one holding its keychain item, as ``signedIn(with:)``
    /// finds it. Whether they were written.
    @discardableResult
    func learnFacts(for connection: ServerConnection, privateKey: String) -> Bool {
        guard connection.authMethod == .key else { return false }
        let record = keys.first { $0.id == connection.sshKeyId }
            ?? connection.keychainRef.flatMap { ref in keys.first { $0.keychainRef == ref } }
        guard let record, record.publicKey.isEmpty, record.refused == nil,
              backing.learnFacts(id: record.id, privateKey: privateKey) else { return false }
        adopt()
        return true
    }

    /// Key `id`'s private key, imported again into its own record (B781):
    /// `key` read off it, its item already saved under the record's
    /// `keychainRef`. The record keeps its name and every connection that
    /// signs in with it; a key with another fingerprint is refused. Whether
    /// it was written; the key is no longer missing once it was.
    func refill(_ id: UUID, with key: SSHKeyFactory.Generated, requireBiometry: Bool) -> StoreRefill {
        let refill = backing.refill(id: id, key: key, requireBiometry: requireBiometry)
        if refill == .refilled { missingIds.remove(id) }
        adopt()
        return refill
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    func reloadIfNeeded() {
        backing.reloadIfNeeded()
        adopt()
    }

    /// `connection` signed in (B601): the managed key it signs in with — its
    /// record by id, else the one holding its keychain item — gets its last
    /// use, at most once a day. Nothing ever wrote it, so SSH Keys' unused
    /// key (the gray avatar, "unused since") never showed. Whether it was
    /// written.
    @discardableResult
    func signedIn(with connection: ServerConnection) -> Bool {
        guard connection.authMethod == .key else { return false }
        let record = keys.first { $0.id == connection.sshKeyId }
            ?? connection.keychainRef.flatMap { ref in keys.first { $0.keychainRef == ref } }
        guard let record, backing.markUsed(id: record.id) else { return false }
        adopt()
        return true
    }
}

/// What an ``SSHKeyStore`` reads and writes through: the Rust store
/// (crates/store, through crates/ffi; ``RustSSHKeyStore``), or a test's
/// stand-in. Each member keeps the contract of the store's member of the
/// same name. Key material is never here: the records name their keychain
/// items.
protocol SSHKeyStoreBacking: AnyObject {
    var keys: [SSHKeyRecord] { get }
    var isAuthoritative: Bool { get }
    func add(_ record: SSHKeyRecord)
    func remove(id: UUID)
    func reloadIfNeeded()
    func keychainRefs(excluding: UUID?) -> Set<String>?
    /// Key `id` signed in now: its last use, at most once a day; whether it
    /// was written.
    func markUsed(id: UUID) -> Bool
    /// Every key 1.0.8 imported without its public half, read from
    /// `keychain` without asking, its facts kept (B726); the records
    /// changed.
    func learnKeyFacts(keychain: KeychainHost) -> [SSHKeyRecord]
    /// Key `id`'s facts off `privateKey`, if its record lacks them (B726);
    /// whether they were written.
    func learnFacts(id: UUID, privateKey: String) -> Bool
    /// Key `id`'s private key imported again (B781): `key`'s facts kept on
    /// the same record, unless it is another key.
    func refill(id: UUID, key: SSHKeyFactory.Generated, requireBiometry: Bool) -> StoreRefill
    /// The orphaned private keys listed again (`secrets::recover_orphans`);
    /// nil when the stores couldn't be asked.
    func recoverOrphans(connections: ConnectionStoreBacking, keychain: KeychainHost,
                        reader: StoredKeyReader?, nameTemplate: String, now: Date) -> StoreRecovery?
    /// What deleting key `id` would do now (`secrets::check_key`).
    func checkDelete(id: UUID, connections: ConnectionStoreBacking) -> StoreKeyCheck?
    /// Key `id` deleted, its private key first (`secrets::delete_key`).
    func deleteKey(id: UUID, connections: ConnectionStoreBacking, keychain: KeychainHost) -> StoreKeyDeletion?
}

#if DEBUG
extension SSHKeyStore {
    /// A capture run's store (``KeysCapture``): `records`, for this launch
    /// only — the Rust store over preferences kept in memory
    /// (``LaunchOnlyPreferences``), whose read is never trusted, so it
    /// writes nothing, the app's own keys left alone.
    @MainActor
    static func capture(_ records: [SSHKeyRecord]) -> SSHKeyStore {
        let preferences = LaunchOnlyPreferences()
        if let data = try? JSONEncoder().encode(records), let value = PlistNodes.value(data) {
            preferences.set(key: "moshpit.sshkeys.v1", value: value)
        }
        return SSHKeyStore(backing: RustSSHKeyStore(preferences: preferences, mode: .system))
    }
}
#endif
