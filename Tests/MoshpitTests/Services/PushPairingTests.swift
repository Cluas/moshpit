import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// Pairing material, the one-liner the user runs on the host, and the drift
/// guard between the shell sender and its Swift copy.
@Suite("Push pairing")
struct PushPairingTests {

    // MARK: - Material

    @Test("make mints the secret locally and leaves the credential to the relay")
    func material() {
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                 relayURL: "https://push.example.org")
        let b = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                 relayURL: "https://push.example.org")
        for pairing in [a, b] {
            #expect(pairing.secretHex.count == 64)
            #expect(Data(moshpitHex: pairing.secretHex) != nil)
            // The send token is the RELAY's to mint — an HMAC over the routing
            // facts, which do not exist until a device token does. A fresh
            // pairing must therefore be visibly not-ready, never installable.
            #expect(pairing.sendToken.isEmpty)
            #expect(!pairing.isReadyToInstall)
            #expect(pairing.needsMint(currentToken: nil))
        }
        #expect(a.secretHex != b.secretHex)
    }

    @Test("a credential re-mints when the device token rotates or age nears the relay TTL")
    func mintLifecycle() {
        var pairing = PushPairing.make(connectionId: UUID(), hostLabel: "h",
                                       relayURL: "https://r")
        let now = Date()
        pairing.sendToken = String(repeating: "ab", count: 32)
        pairing.apnsToken = String(repeating: "CD", count: 32)
        pairing.apnsEnv = "production"
        pairing.sendTokenIssuedAt = now

        #expect(pairing.isReadyToInstall)
        // Same token, fresh mint: nothing to do. Hex casing must not split it.
        #expect(!pairing.needsMint(currentToken: String(repeating: "cd", count: 32), now: now))
        // The phone's token rotated (reinstall, restore): the HMAC no longer
        // matches what the conf will claim, so it must re-mint.
        #expect(pairing.needsMint(currentToken: String(repeating: "ee", count: 32), now: now))
        // Ageing towards the relay-side TTL re-mints long before expiry.
        #expect(pairing.needsMint(currentToken: String(repeating: "cd", count: 32),
                                  now: now.addingTimeInterval(PushPairing.refreshAfter + 1)))
        // A pairing stored by an older build (no routing fields) must read as
        // needing a mint, not as ready.
        pairing.apnsToken = nil
        #expect(pairing.needsMint(currentToken: String(repeating: "cd", count: 32), now: now))
        #expect(!pairing.isReadyToInstall)
    }

    @Test("a trailing slash on the relay URL does not become a double slash")
    func urlNormalisation() {
        let pairing = PushPairing.make(connectionId: UUID(), hostLabel: "h",
                                       relayURL: "  https://push.example.org/  ")
        #expect(pairing.relayURL == "https://push.example.org")
    }

    // The push conf the host receives is built as content, not a command:
    // crates/install's `push_conf` and its tests.
}

@Suite("Pairing store persistence", .serialized, .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
struct PushPairingStorePersistenceTests {

    /// The store is real shared state with no injection seam — on device the
    /// extension has nobody to inject it — so these save and restore.
    private func withCleanStore(_ body: () -> Void) {
        let saved = PushPairingStore.read()
        defer { PushPairingStore.write(saved) }
        PushPairingStore.write([])
        body()
    }

    @Test("a save reports success only when it can be read back")
    func writeVerifies() {
        withCleanStore {
            let pairing = PushPairing.make(connectionId: UUID(), hostLabel: "m1-pro",
                                          relayURL: "https://push.example.org")
            // The return value is the point: it used to be three silent `try?`s,
            // so a failed save left the app behaving exactly as if nothing were
            // paired — no prompt, no token, no push, no explanation.
            #expect(PushPairingStore.upsert(pairing))
            #expect(PushPairingStore.read().count == 1)
            #expect(PushPairingStore.pairing(for: pairing.connectionId)?.secretHex
                    == pairing.secretHex)
        }
    }

    @Test("re-pairing one host replaces it and leaves the others alone")
    func upsertReplaces() {
        withCleanStore {
            let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
            let b = PushPairing.make(connectionId: UUID(), hostLabel: "b", relayURL: "https://b")
            #expect(PushPairingStore.upsert(a))
            #expect(PushPairingStore.upsert(b))

            let again = PushPairing.make(connectionId: a.connectionId, hostLabel: "a",
                                         relayURL: "https://a2")
            #expect(PushPairingStore.upsert(again))
            #expect(PushPairingStore.read().count == 2)
            #expect(PushPairingStore.pairing(for: a.connectionId)?.relayURL == "https://a2")
            #expect(PushPairingStore.pairing(for: b.connectionId)?.relayURL == "https://b")
            // Newest first is what the extension relies on when it tries keys.
            #expect(PushPairingStore.secretsNewestFirst().first == again.secretHex)
        }
    }
}

// MARK: - What one build can't read, it keeps

/// A pairing entry in a shape this build can't decode — what a newer build
/// might write (here: its secret moved under a versioned key), and what this
/// one meets after a rollback.
private func foreignPairing(_ id: UUID, host: String) -> [String: Any] {
    ["connectionId": id.uuidString, "hostLabel": host,
     "secret": ["v": 2, "hex": String(repeating: "ab", count: 32)],
     "relayURL": "https://push.example.org", "createdAt": 784_000_000.0]
}

/// One pairing as the store writes it, as a JSON value.
private func storedForm(_ pairing: PushPairing) throws -> Any {
    let data = try JSONEncoder().encode([pairing])
    return try #require((try JSONSerialization.jsonObject(with: data) as? [Any])?.first)
}

/// The connection ids in a pairings file, in stored order — read as plain
/// JSON, so entries no build decodes are counted too.
private func storedIds(at url: URL) throws -> [String] {
    let entries = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [[String: Any]] ?? []
    return entries.compactMap { $0["connectionId"] as? String }
}

/// The pairings file read an entry at a time. It used to decode in one
/// piece, and anything that failed — one entry from a newer build after a
/// rollback, a damaged byte, a file that couldn't be opened — read as "no
/// pairings", which the next pair or unpair wrote over every other host.
@Suite("Pairing file — what this build can't read, it keeps")
struct PushPairingFileTests {

    private func makeFile() throws -> PushPairingFile {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("pairings-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return PushPairingFile(url: dir.appendingPathComponent("push-pairings.json"))
    }

    private func cleanUp(_ file: PushPairingFile) {
        try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent())
    }

    @Test("Pairing a host keeps an entry this build can't decode, and the ones it can")
    func upsertKeepsWhatItCantRead() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
        let foreignId = UUID()
        try JSONSerialization.data(withJSONObject: [try storedForm(a), foreignPairing(foreignId, host: "b")])
            .write(to: file.url)
        #expect(file.read().map(\.connectionId) == [a.connectionId], "one bad entry hid every pairing")

        let c = PushPairing.make(connectionId: UUID(), hostLabel: "c", relayURL: "https://c")
        #expect(file.upsert(c))

        #expect(try storedIds(at: file.url).sorted()
                == [a.connectionId, c.connectionId, foreignId].map(\.uuidString).sorted(),
                "pairing one host deleted the others")
        #expect(file.read().map(\.connectionId) == [a.connectionId, c.connectionId])
    }

    @Test("Unpairing one host keeps every other entry, readable or not")
    func removeKeepsWhatItCantRead() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
        let c = PushPairing.make(connectionId: UUID(), hostLabel: "c", relayURL: "https://c")
        let foreignId = UUID()
        try JSONSerialization.data(withJSONObject: [try storedForm(a), foreignPairing(foreignId, host: "b"),
                                                    try storedForm(c)])
            .write(to: file.url)

        #expect(file.remove(connectionId: a.connectionId))
        #expect(try storedIds(at: file.url).sorted() == [c.connectionId, foreignId].map(\.uuidString).sorted())

        // Unpairing the host whose entry this build can't read removes it.
        #expect(file.remove(connectionId: foreignId))
        #expect(try storedIds(at: file.url) == [c.connectionId.uuidString])
    }

    /// Deleting a connection asks the file to forget its pairing whether or
    /// not it ever had one. A connection the file doesn't name is nothing to
    /// forget, and a write would only give the forgetting a way to fail —
    /// which stops the delete.
    @Test("Forgetting a connection the file doesn't name leaves the file as it is")
    func removeOfAStrangerWritesNothing() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
        #expect(file.write([a]))
        let identity = { try FileManager.default.attributesOfItem(atPath: file.url.path)[.systemFileNumber] as? Int }
        let before = try identity()

        #expect(file.remove(connectionId: UUID()))

        let after = try identity()
        #expect(before != nil && after == before, "the file was written over to forget nothing")
        #expect(file.read().map(\.connectionId) == [a.connectionId])
    }

    @Test("A file that isn't a list at all is copied untouched before the first pairing is written")
    func garbageIsBackedUpFirst() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let garbage = Data(#"{"pairings":"in a shape nobody here knows"}"#.utf8)
        try garbage.write(to: file.url)
        #expect(file.read().isEmpty)

        let c = PushPairing.make(connectionId: UUID(), hostLabel: "c", relayURL: "https://c")
        #expect(file.upsert(c))
        #expect(try Data(contentsOf: file.backupURL) == garbage, "the only copy of the old store was written over")
        #expect(file.read().map(\.connectionId) == [c.connectionId])
    }

    @Test("A file that is there but can't be read is never written over")
    func unreadableFileIsNotWritten() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
        #expect(file.write([a]))
        let before = try Data(contentsOf: file.url)
        // Stands in for a read before the first unlock, or an I/O error.
        try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: file.url.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.url.path) }
        try #require((try? Data(contentsOf: file.url)) == nil, "this filesystem still lets the owner read a 000 file")

        let c = PushPairing.make(connectionId: UUID(), hostLabel: "c", relayURL: "https://c")
        #expect(!file.upsert(c), "a save went ahead over a store it couldn't read")
        #expect(!file.remove(connectionId: a.connectionId))

        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.url.path)
        #expect(try Data(contentsOf: file.url) == before)
    }

    @Test("What the store writes is still the array every build and the extension decode")
    func formatIsUnchanged() throws {
        let file = try makeFile()
        defer { cleanUp(file) }
        let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
        let b = PushPairing.make(connectionId: UUID(), hostLabel: "b", relayURL: "https://b")
        #expect(file.upsert(a))
        #expect(file.upsert(b))
        let decoded = try JSONDecoder().decode([PushPairing].self, from: Data(contentsOf: file.url))
        #expect(decoded.map(\.connectionId) == [a.connectionId, b.connectionId])
        #expect(decoded.map(\.secretHex) == [a.secretHex, b.secretHex])
    }
}

/// The same, through the App Group store the app and the extension share —
/// the real file, restored byte for byte afterwards.
@Suite("Pairing store — entries it can't decode", .serialized,
       .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
struct PushPairingStoreToleranceTests {

    private func withRawStore(_ body: (URL) throws -> Void) throws {
        let container = try #require(FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: PushPairingStore.appGroup))
        let url = container.appendingPathComponent("push-pairings.json")
        let manager = FileManager.default
        let siblings = Set((try? manager.contentsOfDirectory(atPath: container.path)) ?? [])
        let saved = try? Data(contentsOf: url)
        defer {
            if let saved {
                try? saved.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                var values = URLResourceValues()
                values.isExcludedFromBackup = true
                var mutable = url
                try? mutable.setResourceValues(values)
            } else {
                try? manager.removeItem(at: url)
            }
            // Copies this test made, and only those.
            for name in (try? manager.contentsOfDirectory(atPath: container.path)) ?? []
            where !siblings.contains(name) && name.hasPrefix("push-pairings.json.") {
                try? manager.removeItem(at: container.appendingPathComponent(name))
            }
        }
        try body(url)
    }

    @Test("Pairing one host keeps a host whose entry this build can't decode")
    func upsertKeepsForeignEntry() throws {
        try withRawStore { url in
            let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
            let foreignId = UUID()
            try JSONSerialization.data(withJSONObject: [try storedForm(a), foreignPairing(foreignId, host: "b")])
                .write(to: url)

            let c = PushPairing.make(connectionId: UUID(), hostLabel: "c", relayURL: "https://c")
            #expect(PushPairingStore.upsert(c))

            let ids = try storedIds(at: url)
            #expect(ids.contains(a.connectionId.uuidString), "pairing c deleted a")
            #expect(ids.contains(foreignId.uuidString), "pairing c deleted the entry this build can't read")
            #expect(PushPairingStore.pairing(for: a.connectionId) != nil)
        }
    }

    @Test("Unpairing one host keeps a host whose entry this build can't decode")
    func removeKeepsForeignEntry() throws {
        try withRawStore { url in
            let a = PushPairing.make(connectionId: UUID(), hostLabel: "a", relayURL: "https://a")
            let c = PushPairing.make(connectionId: UUID(), hostLabel: "c", relayURL: "https://c")
            let foreignId = UUID()
            try JSONSerialization.data(withJSONObject: [try storedForm(a), foreignPairing(foreignId, host: "b"),
                                                        try storedForm(c)])
                .write(to: url)

            PushPairingStore.remove(connectionId: a.connectionId)

            let ids = try storedIds(at: url)
            #expect(ids.contains(c.connectionId.uuidString), "unpairing a deleted c")
            #expect(ids.contains(foreignId.uuidString), "unpairing a deleted the entry this build can't read")
        }
    }
}
