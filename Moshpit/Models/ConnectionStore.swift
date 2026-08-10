import Foundation
import Observation

/// Persists `ServerConnection` metadata to UserDefaults as a JSON-encoded array.
///
/// Secrets (passwords / private keys) are NOT stored here — they live in Keychain,
/// referenced by `ServerConnection.keychainRef`.
@Observable
final class ConnectionStore {
    private static let storageKey = "moshpit.connections"

    private(set) var connections: [ServerConnection] = []

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        load()
    }

    // MARK: - CRUD

    func add(_ connection: ServerConnection) {
        connections.append(connection)
        persist()
    }

    func update(_ connection: ServerConnection) {
        guard let index = connections.firstIndex(where: { $0.id == connection.id }) else {
            return
        }
        connections[index] = connection
        persist()
    }

    func delete(id: UUID) {
        connections.removeAll(where: { $0.id == id })
        persist()
    }

    // MARK: - Persistence

    func load() {
        guard let data = defaults.data(forKey: Self.storageKey) else {
            connections = []
            return
        }
        do {
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            connections = try decoder.decode([ServerConnection].self, from: data)
        } catch {
            // Corrupt data — drop it rather than crash.
            connections = []
        }
    }

    private func persist() {
        do {
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            let data = try encoder.encode(connections)
            defaults.set(data, forKey: Self.storageKey)
        } catch {
            // Encoding a `[ServerConnection]` should never fail, but we
            // intentionally do not crash the app if it does.
        }
    }
}
