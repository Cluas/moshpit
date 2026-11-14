import Foundation
import Testing
@testable import Moshpit

/// The DEBUG smoke seed (`-MOSHPIT_SEED_USER`, `-MOSHPIT_SEED_KEY_B64`, …)
/// writes nothing that outlives the launch (B200). It used to delete every
/// saved connection, put its own in their place, plant its key in the
/// keychain, trust the host for good and turn notifications off: a debug
/// build on a phone in daily use took the person's connections with it.
@Suite("DEBUG smoke seed")
@MainActor
struct SmokeSeedTests {

    static let id = UUID(uuidString: "5EED5EED-0000-4000-8000-000000000200")!

    static func arguments(_ extra: [String] = []) -> [String] {
        ["-MOSHPIT_SEED_USER", "rig",
         "-MOSHPIT_SEED_KEY_B64", Data("a test key, never read\n".utf8).base64EncodedString(),
         "-MOSHPIT_SEED_HOST", "rig.example.com", "-MOSHPIT_SEED_PORT", "2222",
         "-MOSHPIT_SEED_ID", id.uuidString, "-MOSHPIT_SEED_TMUX", "1"] + extra
    }

    /// Whether the app's saved connections name the seed's. Read as text,
    /// not compared with what was there before: suites running alongside
    /// this one save connections of their own under that key (a fresh
    /// simulator starts with none), so its bytes are no measure, and the
    /// seed's id in them is.
    static func savedNameTheSeed() -> Bool {
        guard let data = UserDefaults.standard.data(forKey: "moshpit.connections") else { return false }
        return String(decoding: data, as: UTF8.self).contains(id.uuidString)
    }

    @Test("the seeded connection is this launch's alone: the saved ones and the keychain are left as they are")
    func seedWritesNothing() async throws {
        #expect(!Self.savedNameTheSeed(), "a seed from an earlier run was saved")
        let seed = try #require(await SmokeSeed.prepare(arguments: Self.arguments()))

        #expect(seed.connection.id == Self.id)
        #expect(seed.connection.host == "rig.example.com" && seed.connection.port == 2222)
        #expect(seed.connection.username == "rig" && seed.connection.useTmux)
        #expect(seed.store.connections.map(\.id) == [Self.id], "Home shows the seeded connection alone")
        #expect(seed.opensTerminal)
        #expect(!Self.savedNameTheSeed(), "the seed went into the saved connections")
        let ref = try #require(seed.connection.keychainRef)
        #expect(ref == "smoke.seed.\(Self.id.uuidString)")
        #expect(!SystemKeychainBackend().exists(account: ref), "the seed's key went into the keychain")
        #expect(!SystemKeychainBackend().exists(account: Self.id.uuidString))

        // The launch's store keeps what is done to it in memory only.
        var renamed = seed.connection
        renamed.name = "renamed"
        seed.store.update(renamed)
        #expect(!Self.savedNameTheSeed(), "the launch's store wrote to the saved connections")
    }

    @Test("-MOSHPIT_SEED_HOME 1 stays on Home; no key, no seed")
    func homeAndNothing() async throws {
        let home = try #require(await SmokeSeed.prepare(arguments: Self.arguments(["-MOSHPIT_SEED_HOME", "1"])))
        #expect(!home.opensTerminal)
        #expect(await SmokeSeed.prepare(arguments: ["-MOSHPIT_SEED_USER", "rig"]) == nil)
    }

    /// A seed's key used to sit in the cache every background clears: a
    /// seeded session that reconnected after one had no key left.
    @Test("a seed's key is read before the keychain, and outlives a background")
    func seedKeyOutlivesABackground() async throws {
        let service = SSHService(keychain: .inMemory(), hostKeyValidator: .inMemory())
        let connection = ServerConnection(name: "rig", host: "rig.example.com", username: "rig",
                                          authMethod: .password, keychainRef: "smoke.seed.none")
        await service.seedSecretForThisLaunch(connectionId: connection.id, secret: "launch only")
        await service.clearAllCachedSecrets()   // what a background does
        let credential = try await service.rustCoreCredential(connection) { _, _ in nil }
        #expect(credential == .password("launch only"))
    }
}
