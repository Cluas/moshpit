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
@Observable
final class ConnectionStore {
    private static let storageKey = "moshpit.connections"

    private(set) var connections: [ServerConnection] = []

    /// True once `connections` came from defaults known to be readable. Until
    /// then the list is a guess: shown, never written over, read again when
    /// the app comes forward (``reloadIfNeeded()``).
    @ObservationIgnored private(set) var isAuthoritative = false

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let readiness: DefaultsReadiness

    init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        self.defaults = defaults
        self.readiness = readiness
        load()
    }

    // MARK: - CRUD

    // Every mutation re-reads first if the last read could not be trusted:
    // the user who taps "+" on a first-run card that should not be there has
    // unlocked the phone to do it, so the real list is readable by now — and
    // the new connection joins it instead of replacing it.

    func add(_ connection: ServerConnection) {
        reloadIfNeeded()
        connections.append(connection)
        persist()
    }

    func update(_ connection: ServerConnection) {
        reloadIfNeeded()
        guard let index = connections.firstIndex(where: { $0.id == connection.id }) else {
            return
        }
        connections[index] = connection
        persist()
    }

    func delete(id: UUID) {
        reloadIfNeeded()
        connections.removeAll(where: { $0.id == id })
        persist()
    }

    // MARK: - Persistence

    func load() {
        let readable = readiness.isReadable(defaults)
        if let data = defaults.data(forKey: Self.storageKey) {
            do {
                let decoder = JSONDecoder()
                decoder.dateDecodingStrategy = .iso8601
                connections = try decoder.decode([ServerConnection].self, from: data)
            } catch {
                // Corrupt data — drop it rather than crash.
                connections = []
            }
        } else {
            connections = []
        }
        isAuthoritative = readable
        if readable { readiness.markReadable(defaults) }
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    /// The app calls this when it comes forward and when protected data
    /// becomes available.
    func reloadIfNeeded() {
        if !isAuthoritative { load() }
    }

    private func persist() {
        // Never write over a file that could not be read: a sealed file is not
        // an empty list, and the list on disk is the user's.
        guard isAuthoritative else { return }
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
