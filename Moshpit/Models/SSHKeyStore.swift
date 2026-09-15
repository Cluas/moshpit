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
    case rsa4096 = "RSA-4096"
    case seP256 = "SE · P256"
}

/// Public metadata for one key. Private material lives in the Keychain
/// (or inside the Secure Enclave), referenced by `keychainRef`.
struct SSHKeyRecord: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String
    var algorithm: SSHKeyAlgorithm
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
        source == .secureEnclave ? "SE · \(algorithm == .seP256 ? "P256" : algorithm.rawValue)" : algorithm.rawValue
    }
}

// MARK: - Store

@Observable
final class SSHKeyStore {
    private static let storageKey = "moshpit.sshkeys.v1"

    private(set) var keys: [SSHKeyRecord] = []

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let readiness: DefaultsReadiness
    /// True once `keys` came from defaults known to be readable
    /// (``DefaultsReadiness``); until then nothing here is written back.
    @ObservationIgnored private(set) var isAuthoritative = false

    init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        self.defaults = defaults
        self.readiness = readiness
        load()
    }

    var deviceKeys: [SSHKeyRecord] { keys.filter { $0.source == .secureEnclave } }
    var importedKeys: [SSHKeyRecord] { keys.filter { $0.source != .secureEnclave } }

    func add(_ record: SSHKeyRecord) {
        reloadIfNeeded()
        keys.append(record)
        persist()
    }

    func update(_ record: SSHKeyRecord) {
        reloadIfNeeded()
        guard let i = keys.firstIndex(where: { $0.id == record.id }) else { return }
        keys[i] = record
        persist()
    }

    func remove(id: UUID) {
        reloadIfNeeded()
        keys.removeAll { $0.id == id }
        persist()
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    func reloadIfNeeded() {
        if !isAuthoritative { load() }
    }

    private func load() {
        let readable = readiness.isReadable(defaults)
        if let data = defaults.data(forKey: Self.storageKey),
           let decoded = try? JSONDecoder().decode([SSHKeyRecord].self, from: data) {
            keys = decoded
        } else {
            keys = []
        }
        isAuthoritative = readable
        if readable { readiness.markReadable(defaults) }
    }

    private func persist() {
        // A sealed file is not an empty list; never write over one.
        guard isAuthoritative else { return }
        if let data = try? JSONEncoder().encode(keys) {
            defaults.set(data, forKey: Self.storageKey)
        }
    }
}
