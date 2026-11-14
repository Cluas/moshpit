import Foundation
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
             keychainRef, requireBiometry, createdAt, lastUsedAt
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
        // Unreadable here means unreadable everywhere: the record is carried
        // as stored (``StoredList``), not guessed at.
        let olderBuildsName = try container.decode(SSHKeyAlgorithm.self, forKey: .algorithm)
        // A real algorithm from a build newer than this one may be one this
        // build has no name for; then it reads what older builds read.
        let real = (try? container.decodeIfPresent(String.self, forKey: .keyAlgorithm))
            .flatMap { $0 }
            .flatMap(SSHKeyAlgorithm.init(rawValue:))
        let algorithm = real ?? Self.recoveredAlgorithm(olderBuildsName: olderBuildsName, publicKey: publicKey)
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
    }

    /// The algorithm of a record with no (readable) `keyAlgorithm`: one an
    /// older build saved, dropping the field it doesn't know. Its ECDSA-sk
    /// covers every ECDSA key; the public key line says which ones are plain
    /// ECDSA, and those are named that again.
    private static func recoveredAlgorithm(olderBuildsName: SSHKeyAlgorithm, publicKey: String) -> SSHKeyAlgorithm {
        guard olderBuildsName == .ecdsaSK, publicKey.hasPrefix("ecdsa-sha2-") else { return olderBuildsName }
        return .ecdsa
    }
}

// MARK: - Store

@Observable
final class SSHKeyStore {
    private static let storageKey = "moshpit.sshkeys.v1"
    /// Read one record at a time — see ``StoredList``. A record this build
    /// can't decode (a newer build's algorithm, say) is carried, not dropped:
    /// dropping it orphaned the key in the Keychain, the one place the private
    /// half lives.
    private static let list = StoredList<SSHKeyRecord>(key: storageKey)

    private(set) var keys: [SSHKeyRecord] = []

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let readiness: DefaultsReadiness
    /// True once `keys` came from defaults known to be readable
    /// (``DefaultsReadiness``) and anything the read skipped is safe; until
    /// then nothing here is written back.
    @ObservationIgnored private(set) var isAuthoritative = false
    /// Stored records this build could not decode, written back as stored.
    @ObservationIgnored private var carried: [Any] = []
    /// False when the last read found something other than a list under the
    /// key (see ``StoredList``): whatever keys it held can't be told apart.
    @ObservationIgnored private var readAList = true

    /// What reads and writes the records when it isn't the code below:
    /// the Rust store, behind the `MOSHPIT_RUST_STORE` switch
    /// (``SSHKeyStoreBacking``). nil: the code below, as ever.
    @ObservationIgnored private let backing: SSHKeyStoreBacking?

    init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        self.defaults = defaults
        self.readiness = readiness
        backing = nil
        load()
    }

    /// A store whose every read and write is `backing`'s; this object keeps
    /// what the views observe. `defaults` and `readiness` go unused.
    init(backing: SSHKeyStoreBacking) {
        defaults = .standard
        readiness = .always
        self.backing = backing
        adopt(backing)
    }

    /// Take what `backing` holds now, changing only what changed.
    private func adopt(_ backing: SSHKeyStoreBacking) {
        let held = backing.keys
        if held != keys { keys = held }
        isAuthoritative = backing.isAuthoritative
    }

    var deviceKeys: [SSHKeyRecord] { keys.filter { $0.source == .secureEnclave } }
    var importedKeys: [SSHKeyRecord] { keys.filter { $0.source != .secureEnclave } }

    /// Every keychain ref a stored key record holds, the carried ones' too —
    /// each the only handle on a private key. nil when that can't be known:
    /// a read that can't be trusted (``isAuthoritative``), a stored value
    /// that isn't a list, a carried entry whose `keychainRef` can't be read.
    /// `excluding` leaves one record out (the key being deleted).
    func keychainRefs(excluding excluded: UUID? = nil) -> Set<String>? {
        if let backing { return backing.keychainRefs(excluding: excluded) }
        guard isAuthoritative, readAList else { return nil }
        var refs = Set(keys.filter { $0.id != excluded }.compactMap(\.keychainRef))
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

    func add(_ record: SSHKeyRecord) {
        if let backing {
            backing.add(record)
            return adopt(backing)
        }
        reloadIfNeeded()
        keys.append(record)
        persist()
    }

    /// The record alone. Deleting a key is ``SSHKeyDeletion``'s: the record
    /// leaving here never took its private key with it (B232).
    func remove(id: UUID) {
        if let backing {
            backing.remove(id: id)
            return adopt(backing)
        }
        reloadIfNeeded()
        keys.removeAll { $0.id == id }
        persist()
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    func reloadIfNeeded() {
        if let backing {
            backing.reloadIfNeeded()
            return adopt(backing)
        }
        if !isAuthoritative { load() }
    }

    private func load() {
        let readable = readiness.isReadable(defaults)
        let contents = Self.list.read(from: defaults)
        keys = contents.elements
        carried = contents.carried
        readAList = contents.isList
        isAuthoritative = readable && Self.list.secure(contents, in: defaults)
        if readable { readiness.markReadable(defaults) }
    }

    private func persist() {
        // A sealed file is not an empty list; never write over one.
        guard isAuthoritative else { return }
        if let data = Self.list.encode(keys, carrying: carried) {
            defaults.set(data, forKey: Self.storageKey)
        }
    }
}

/// What an ``SSHKeyStore`` reads and writes through when it isn't its own
/// Swift code: the Rust store (crates/store, through crates/ffi), behind the
/// `MOSHPIT_RUST_STORE` switch (`RustStoreSwitch`). Each member keeps the
/// contract of the store's member of the same name. Key material is never
/// here: the records name their keychain items, as ever.
protocol SSHKeyStoreBacking: AnyObject {
    var keys: [SSHKeyRecord] { get }
    var isAuthoritative: Bool { get }
    func add(_ record: SSHKeyRecord)
    func remove(id: UUID)
    func reloadIfNeeded()
    func keychainRefs(excluding: UUID?) -> Set<String>?
}

#if DEBUG
extension SSHKeyStore {
    /// A capture run's store (``KeysCapture``): `records`, in memory. It reads
    /// from a suite of its own (never the app's defaults) as defaults it
    /// can't trust, so it writes nothing: not the records, not the readiness
    /// sentinel.
    @MainActor
    static func capture(_ records: [SSHKeyRecord]) -> SSHKeyStore {
        // A suite name UserDefaults takes; never a fallback to the app's.
        let store = SSHKeyStore(defaults: UserDefaults(suiteName: "moshpit.capture.keys")!,
                                readiness: DefaultsReadiness { false })
        store.keys = records
        return store
    }
}
#endif
