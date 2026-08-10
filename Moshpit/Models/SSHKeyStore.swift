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
    var id: UUID = UUID()
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
    var createdAt: Date = Date()
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

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        load()
    }

    var deviceKeys: [SSHKeyRecord] { keys.filter { $0.source == .secureEnclave } }
    var importedKeys: [SSHKeyRecord] { keys.filter { $0.source != .secureEnclave } }

    func add(_ record: SSHKeyRecord) {
        keys.append(record)
        persist()
    }

    func update(_ record: SSHKeyRecord) {
        guard let i = keys.firstIndex(where: { $0.id == record.id }) else { return }
        keys[i] = record
        persist()
    }

    func remove(id: UUID) {
        keys.removeAll { $0.id == id }
        persist()
    }

    private func load() {
        guard let data = defaults.data(forKey: Self.storageKey),
              let decoded = try? JSONDecoder().decode([SSHKeyRecord].self, from: data) else {
            keys = []
            return
        }
        keys = decoded
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(keys) {
            defaults.set(data, forKey: Self.storageKey)
        }
    }
}
