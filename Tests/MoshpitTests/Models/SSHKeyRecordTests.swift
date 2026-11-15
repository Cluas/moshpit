import Foundation
import Testing
@testable import Moshpit

/// `SSHKeyRecord` as every build up to and including 1.0.8 declares it, field
/// for field, with the four algorithms those builds know.
///
/// Those builds read the stored list the way `decodeAsOlderBuild` does — the
/// whole array in one decode — and took a list that failed it as no keys at
/// all, then saved that empty list over every key on the next add or delete.
/// TestFlight keeps such a build a tap away.
private struct OlderBuildKeyRecord: Codable, Equatable {
    enum Algorithm: String, Codable {
        case ed25519 = "ED25519"
        case ecdsaSK = "ECDSA-sk"
        case rsa4096 = "RSA-4096"
        case seP256 = "SE · P256"
    }

    enum Source: String, Codable {
        case secureEnclave, generated, imported, hardware
    }

    var id = UUID()
    var name: String
    var algorithm: Algorithm
    var comment: String = ""
    var fingerprint: String = ""
    var publicKey: String = ""
    var source: Source
    var boundHosts: [String] = []
    var keychainRef: String?
    var requireBiometry: Bool = true
    var createdAt = Date()
    var lastUsedAt: Date?
}

@Suite("SSH key records across builds")
struct SSHKeyRecordTests {
    private static let storageKey = "moshpit.sshkeys.v1"

    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.sshkeyrecords.\(UUID().uuidString)"
        return (UserDefaults(suiteName: name)!, name)
    }

    private static func decodeAsOlderBuild(_ defaults: UserDefaults) throws -> [OlderBuildKeyRecord] {
        let data = try #require(defaults.data(forKey: storageKey))
        return try JSONDecoder().decode([OlderBuildKeyRecord].self, from: data)
    }

    /// One key of every kind the app makes, plain ECDSA among them.
    private static let everyKind: [SSHKeyRecord] = [
        SSHKeyRecord(name: "laptop", algorithm: .ed25519,
                     publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA== laptop", source: .generated),
        SSHKeyRecord(name: "work", algorithm: .ecdsa,
                     publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY= work", source: .imported),
        SSHKeyRecord(name: "server", algorithm: .rsa4096,
                     publicKey: "ssh-rsa AAAAB3NzaC1yc2E= server", source: .generated),
        SSHKeyRecord(name: "phone", algorithm: .seP256, source: .secureEnclave),
        SSHKeyRecord(name: "yubikey", algorithm: .ecdsaSK,
                     publicKey: "sk-ecdsa-sha2-nistp256@openssh.com AAAA yubikey", source: .hardware),
    ]

    @Test("An older build reads every key this build saved, a plain ECDSA key among them")
    func olderBuildReadsWhatThisBuildSaves() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = SSHKeyStore(defaults: defaults)
        for record in Self.everyKind { store.add(record) }

        // The old code stored "ECDSA" here; the older build's decode of the
        // whole list threw, and it showed (then saved) no keys at all.
        let older = try Self.decodeAsOlderBuild(defaults)
        #expect(older.map(\.id) == Self.everyKind.map(\.id))
        #expect(older.map(\.algorithm) == [.ed25519, .ecdsaSK, .rsa4096, .seP256, .ecdsaSK],
                "older builds labelled every ECDSA key ECDSA-sk")
        #expect(older.map(\.name) == Self.everyKind.map(\.name))
        #expect(older.map(\.publicKey) == Self.everyKind.map(\.publicKey))
    }

    @Test("This build reads its own records back with their real algorithms")
    func thisBuildReadsItsOwn() {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = SSHKeyStore(defaults: defaults)
        for record in Self.everyKind { store.add(record) }

        #expect(SSHKeyStore(defaults: defaults).keys == Self.everyKind)
    }

    @Test("What an older build saves back, this build reads, a plain ECDSA key still plain ECDSA")
    func thisBuildReadsWhatAnOlderBuildSaved() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = SSHKeyStore(defaults: defaults)
        for record in Self.everyKind { store.add(record) }

        // The older build edits the list and saves it the only way it can:
        // without the field it doesn't know.
        var older = try Self.decodeAsOlderBuild(defaults)
        older.removeAll { $0.name == "server" }
        defaults.set(try JSONEncoder().encode(older), forKey: Self.storageKey)

        let reread = SSHKeyStore(defaults: defaults)
        #expect(reread.keys == Self.everyKind.filter { $0.name != "server" })
        #expect(reread.keys.first { $0.name == "work" }?.algorithm == .ecdsa)
        #expect(reread.keys.first { $0.name == "yubikey" }?.algorithm == .ecdsaSK)
    }

    @Test("A key an older build saved reads as it always did")
    func olderBuildsOwnRecordsRead() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let saved = [
            OlderBuildKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated),
            OlderBuildKeyRecord(name: "server", algorithm: .rsa4096, source: .imported),
        ]
        defaults.set(try JSONEncoder().encode(saved), forKey: Self.storageKey)

        let keys = SSHKeyStore(defaults: defaults).keys
        #expect(keys.map(\.id) == saved.map(\.id))
        #expect(keys.map(\.algorithm) == [.ed25519, .rsa4096])
    }

    @Test("A real algorithm from a newer build that this one doesn't know falls back to the shared name")
    func unknownRealAlgorithmFallsBack() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = SSHKeyStore(defaults: defaults)
        store.add(SSHKeyRecord(name: "token", algorithm: .ed25519, source: .imported))
        let data = try #require(defaults.data(forKey: Self.storageKey))
        var entries = try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        entries[0]["keyAlgorithm"] = "ED448"
        defaults.set(try JSONSerialization.data(withJSONObject: entries), forKey: Self.storageKey)

        let keys = SSHKeyStore(defaults: defaults).keys
        #expect(keys.map(\.name) == ["token"])
        #expect(keys.first?.algorithm == .ed25519)
    }

    // MARK: RSA key sizes

    private static var rsa2048: SSHKeyFixtures.Key {
        SSHKeyFixtures.all.first { $0.name == "RSA 2048 · OpenSSH" }!
    }

    @Test("An RSA key's badge says its real size, not RSA-4096 whatever it is")
    func rsaBadgeShowsRealSize() throws {
        let imported = try SSHKeyFactory.importKey(privatePEM: Self.rsa2048.privateKey, publicKeyLine: nil, comment: "")
        #expect(imported.bits == 2048)
        let record = SSHKeyRecord(name: "work", algorithm: imported.algorithm, bits: imported.bits,
                                  publicKey: imported.publicKeyLine, source: .imported)
        #expect(record.badgeText == "RSA-2048")
        #expect(SSHKeyRecord(name: "server", algorithm: .rsa4096, bits: 4096, source: .generated).badgeText
                == "RSA-4096")
        // Size unknown: no size it may not be.
        #expect(SSHKeyRecord(name: "old", algorithm: .rsa4096, source: .imported).badgeText == "RSA")
        #expect(SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated).badgeText == "ED25519")
    }

    @Test("An RSA key an older build saved shows its real size, read off its public key")
    func olderBuildsRSARecordShowsItsSize() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let saved = [OlderBuildKeyRecord(name: "server", algorithm: .rsa4096,
                                         publicKey: Self.rsa2048.publicKey, source: .imported)]
        defaults.set(try JSONEncoder().encode(saved), forKey: Self.storageKey)

        let key = try #require(SSHKeyStore(defaults: defaults).keys.first)
        #expect(key.bits == 2048)
        #expect(key.badgeText == "RSA-2048")
    }

    @Test("The size is kept where an older build doesn't look, beside the RSA-4096 it reads")
    func rsaSizeIsDowngradeSafe() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = SSHKeyStore(defaults: defaults)
        store.add(SSHKeyRecord(name: "server", algorithm: .rsa4096, bits: 3072, source: .imported))

        #expect(try Self.decodeAsOlderBuild(defaults).map(\.algorithm) == [.rsa4096])
        #expect(SSHKeyStore(defaults: defaults).keys.first?.bits == 3072)
    }

    /// B601: SSH Keys tells a key unused for ninety days (the gray avatar,
    /// "unused since") by its last use, which nothing ever wrote. A sign-in
    /// with a managed key writes it now: found by the connection's key id,
    /// else by the keychain item it signs in with; once a day; never for a
    /// password connection.
    @Test("a sign-in with a managed key records the key's last use, once a day")
    func signInRecordsLastUse() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let store = SSHKeyStore(defaults: defaults)
        var laptop = SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated)
        laptop.keychainRef = "moshpit.secret.laptop"
        var pi = SSHKeyRecord(name: "pi", algorithm: .ed25519, source: .imported)
        pi.keychainRef = "moshpit.secret.pi"
        store.add(laptop)
        store.add(pi)
        var byId = ServerConnection(name: "nas", host: "192.0.2.7", port: 22, username: "me",
                                    authMethod: .key, keychainRef: "moshpit.secret.laptop")
        byId.sshKeyId = laptop.id
        #expect(store.signedIn(with: byId))
        let first = store.keys.first { $0.id == laptop.id }?.lastUsedAt
        #expect(first != nil)
        #expect(SSHKeyStore(defaults: defaults).keys.first { $0.id == laptop.id }?.lastUsedAt == first,
                "stored")
        #expect(!store.signedIn(with: byId), "once a day")
        // A connection saved without the key's id: its keychain item names it.
        let byRef = ServerConnection(name: "pi", host: "192.0.2.8", port: 22, username: "me",
                                     authMethod: .key, keychainRef: "moshpit.secret.pi")
        #expect(store.signedIn(with: byRef))
        #expect(store.keys.first { $0.id == pi.id }?.lastUsedAt != nil)
        let password = ServerConnection(name: "box", host: "192.0.2.9", port: 22, username: "me",
                                        authMethod: .password, keychainRef: "moshpit.secret.laptop")
        #expect(!store.signedIn(with: password))
    }

    /// B726: a key 1.0.8 imported pasted without its public half, as it
    /// saved it — an OpenSSH ECDSA key labelled ED25519 (its guess for any
    /// PEM not `BEGIN RSA` or `BEGIN EC`), no public key, no fingerprint,
    /// behind Face ID — shows what it is once its first sign-in has read
    /// it, and only then: nothing else about the record changes. A SEC1 EC
    /// key 1.0.8 labelled ECDSA-sk reads as plain ECDSA from the start,
    /// which the connection form's picker offers.
    @Test("a key 1.0.8 imported without its public half shows what it is after its first sign-in")
    func signInLearnsWhatAKeyIs() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let pasted = OlderBuildKeyRecord(name: "work", algorithm: .ed25519, source: .imported,
                                         keychainRef: "moshpit.secret.work")
        let sec1 = OlderBuildKeyRecord(name: "router", algorithm: .ecdsaSK, source: .imported,
                                       keychainRef: "moshpit.secret.router")
        defaults.set(try JSONEncoder().encode([pasted, sec1]), forKey: Self.storageKey)
        let store = SSHKeyStore(defaults: defaults)
        #expect(store.keys.first { $0.name == "router" }?.algorithm == .ecdsa, "in the picker")
        let key = try String(decoding: TestFixtures.data("keys-written-by-rust/imported-ecdsa-p-256---openssh"),
                             as: UTF8.self)
        let connection = ServerConnection(name: "box", host: "192.0.2.7", port: 22, username: "me",
                                          authMethod: .key, keychainRef: "moshpit.secret.work")
        #expect(store.learnFacts(for: connection, privateKey: key))
        let work = try #require(store.keys.first { $0.name == "work" })
        #expect(work.algorithm == .ecdsa)
        #expect(work.fingerprint == "SHA256:Y82atu0ehMoZKkgZksBoszn80yjGIvGqADbZjBF3FYg")
        #expect(work.publicKey.hasPrefix("ecdsa-sha2-nistp256 AAAA"))
        #expect(work.requireBiometry && work.boundHosts.isEmpty && work.keychainRef == "moshpit.secret.work")
        #expect(SSHKeyStore(defaults: defaults).keys.first { $0.name == "work" }?.fingerprint == work.fingerprint,
                "stored")
        #expect(!store.learnFacts(for: connection, privateKey: key), "once: it has its facts now")
    }

    /// B781: a key whose private half didn't come with a restore is imported
    /// again into its own record — the same id, name, comment and Face ID
    /// choice, its item under the record's own keychain account, no longer
    /// missing. Another key (another fingerprint) is refused and nothing is
    /// written.
    @Test("Import Again fills the key's own record, and refuses another key")
    @MainActor
    func importAgainFillsTheSameRecord() async throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let store = SSHKeyStore(defaults: defaults)
        var work = SSHKeyRecord(name: "work", algorithm: .ecdsa, comment: "dev@example.com",
                                fingerprint: "SHA256:Y82atu0ehMoZKkgZksBoszn80yjGIvGqADbZjBF3FYg",
                                publicKey: "ecdsa-sha2-nistp256 AAAA work", source: .imported)
        work.keychainRef = "moshpit.secret.work"
        store.add(work)
        store.setMissing([work.id])
        #expect(store.missingKeys.map(\.id) == [work.id] && store.importedKeys.isEmpty)
        let keychain = KeychainService.inMemory()
        let request = { (text: String) in
            SSHKeyCreation.Request(importing: true, name: "ignored", algorithm: .ed25519, comment: "",
                                   requireFaceID: false, secureEnclave: false, privateKey: text,
                                   passphrase: "", publicKeyLine: "")
        }
        let other = try String(decoding: TestFixtures.data("keys-written-by-rust/imported-ed25519---openssh"),
                               as: UTF8.self)
        await #expect(throws: SSHKeyCreation.ImportAgainError.self) {
            _ = try await SSHKeyCreation.importAgain(request(other), into: work, keychain: keychain, store: store)
        }
        let wroteForAnother = await keychain.contains(ref: "moshpit.secret.work")
        #expect(!wroteForAnother, "nothing written for another key")
        let key = try String(decoding: TestFixtures.data("keys-written-by-rust/imported-ecdsa-p-256---openssh"),
                             as: UTF8.self)
        let back = try await SSHKeyCreation.importAgain(request(key), into: work, keychain: keychain, store: store)
        #expect(back.id == work.id && back.name == "work" && back.comment == "dev@example.com")
        #expect(back.keychainRef == "moshpit.secret.work" && !back.requireBiometry && !back.missing)
        #expect(back.publicKey.hasPrefix("ecdsa-sha2-nistp256 AAAA") && back.publicKey != work.publicKey)
        let wrote = await keychain.contains(ref: "moshpit.secret.work")
        #expect(wrote)
        #expect(store.keys.count == 1 && store.missingKeys.isEmpty && store.importedKeys.map(\.id) == [work.id])
    }
}
