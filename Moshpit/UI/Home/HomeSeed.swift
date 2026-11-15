#if DEBUG
import Foundation

/// A DEBUG launch argument that fills Home with a set of connections for
/// this launch only, on a device (`-MOSHPIT_SEED_HOME_FIXTURE <base64
/// JSON>`). Nothing here
/// outlives the launch: the connections live in a store that can never
/// write (it never trusts what it read: ``DefaultsReadiness``), the key in
/// the connect path's in-memory cache, the hosts' keys in ``LaunchTrust``.
/// The app's own store, keychain and settings are not touched; the seeded
/// Home shows instead of the saved one.
///
/// The JSON:
///
/// ```json
/// {"key": "<an openssh private key, base64>",
///  "trust": [{"host": "192.0.2.22", "port": 52219, "fingerprint": "SHA256:…"}],
///  "connections": [{"id": "…", "name": "studio", "host": "192.0.2.22", "port": 52219,
///                   "user": "cluas", "mosh": false, "mux": "tmux", "tmuxPath": "/tmp/…",
///                   "connect": true}]}
/// ```
///
/// Connections with `"connect": true` connect as Home appears, as a tap on
/// their heads would. Pass `-MOSHPIT_AUTOCARE_OFF` too, so no host is
/// tended, and `-moshpit.settings.notifications '<false/>'` (and
/// `liveActivity`) for quiet captures: argument-domain values, never
/// written.
@MainActor
enum HomeSeed {
    struct Fixture: Decodable {
        struct Trust: Decodable {
            let host: String
            let port: Int
            let fingerprint: String
        }

        struct Connection: Decodable {
            let id: UUID
            let name: String
            let host: String
            var port: Int?
            let user: String
            var mosh: Bool?
            var mux: String?
            var tmuxPath: String?
            var herdrPath: String?
            var connect: Bool?
        }

        var key: String?
        var trust: [Trust]?
        let connections: [Connection]
    }

    /// The launch's fixture, if it names one.
    static let fixture: Fixture? = {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_SEED_HOME_FIXTURE"), i + 1 < args.count,
              let data = Data(base64Encoded: args[i + 1])
        else { return nil }
        do {
            return try JSONDecoder().decode(Fixture.self, from: data)
        } catch {
            print("[home seed] unreadable fixture: \(error)")
            return nil
        }
    }()

    /// The seeded connections, as the app's model has them.
    static let connections: [ServerConnection] = (fixture?.connections ?? []).map { seed in
        var connection = ServerConnection(
            id: seed.id, name: seed.name, host: seed.host, port: seed.port ?? 22,
            username: seed.user, authMethod: .key,
            connectionProtocol: seed.mosh == true ? .mosh : .ssh,
            sshPort: seed.port ?? 22,
            keychainRef: "home.seed.\(seed.id.uuidString)",
            predictModeRaw: PredictMode.never.rawValue,
            tmuxPath: seed.tmuxPath, herdrPath: seed.herdrPath)
        connection.multiplexer = seed.mux.flatMap(Multiplexer.init(rawValue:)) ?? .none
        return connection
    }

    /// The store Home shows in place of the saved one: the seeded
    /// connections, for this launch only (``launchOnlyStore(_:suite:)``).
    ///
    /// Made before Home is, so it is also where the fixture's hosts are
    /// trusted: Home's record asks the core for each host's trusted key (its
    /// face), and the core keeps the trusted map from its first read on, so
    /// a trust added after Home appeared (``prepare()``) came too late and
    /// the New Host card asked anyway (the first screenshot run).
    static let store: ConnectionStore? = {
        guard let fixture else { return nil }
        for trust in fixture.trust ?? [] {
            LaunchTrust.trust(host: trust.host, port: trust.port, fingerprint: trust.fingerprint)
        }
        return launchOnlyStore(connections, suite: "moshpit.home-seed")
    }()

    /// A store holding `connections` for this launch only: the Rust store
    /// over preferences kept in memory (``LaunchOnlyPreferences``), whose
    /// read is never trusted, so it never writes, not even there. What a
    /// DEBUG seed shows in place of the saved connections, leaving those
    /// alone. `suite` (with the process id) scopes the session-memory sweep
    /// to a domain of its own.
    ///
    /// It used to register the connections in a suite of its own, but a
    /// suite's `register(defaults:)` lands in the process's registration
    /// domain, which every `UserDefaults` reads, `.standard` too: on a
    /// phone with no connections saved, the app's own store read the
    /// seed's back as saved (B200, caught by the first cloud run of the
    /// app's tests on a fresh simulator).
    static func launchOnlyStore(_ connections: [ServerConnection], suite name: String) -> ConnectionStore {
        let preferences = LaunchOnlyPreferences()
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        if let data = try? encoder.encode(connections), let value = PlistNodes.value(data) {
            preferences.set(key: "moshpit.connections", value: value)
        }
        let suite = UserDefaults(suiteName: "\(name).\(ProcessInfo.processInfo.processIdentifier)")!
        return ConnectionStore(backing: RustConnectionStore(preferences: preferences, mode: .system),
                               defaults: suite)
    }

    /// What connecting the seeded connections needs, for this launch only:
    /// the key in the connect path's cache, the hosts' keys trusted. Also
    /// forgets what earlier seeded sessions remembered about them.
    static func prepare() async {
        guard let fixture else { return }
        for trust in fixture.trust ?? [] {
            LaunchTrust.trust(host: trust.host, port: trust.port, fingerprint: trust.fingerprint)
        }
        if let b64 = fixture.key, let data = Data(base64Encoded: b64),
           let key = String(data: data, encoding: .utf8) {
            for connection in connections {
                await SSHService.shared.seedSecretForThisLaunch(connectionId: connection.id, secret: key)
            }
        }
        for connection in connections {
            RustSessionSeed.forget(connection.id)
        }
    }

    /// The seeded connections to connect as Home appears.
    static var autoConnect: [ServerConnection] {
        let ids = Set((fixture?.connections ?? []).filter { $0.connect == true }.map(\.id))
        return connections.filter { ids.contains($0.id) }
    }
}
/// Preferences in memory, for a DEBUG seed's launch-only store: nothing
/// in them reaches the app's preferences, and protected data reads as
/// unavailable, so a store over them shows what they hold and never
/// writes.
final class LaunchOnlyPreferences: PreferencesHost, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: PlistValue] = [:]

    func get(key: String) -> PlistValue? {
        lock.lock()
        defer { lock.unlock() }
        return values[key]
    }

    func set(key: String, value: PlistValue?) {
        lock.lock()
        defer { lock.unlock() }
        values[key] = value
    }

    func protectedDataAvailable() -> Bool { false }
}
#endif
