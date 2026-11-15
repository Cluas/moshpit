import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// The pairings file as the notification service reads it
/// (``PushPairingFile``), over what the app's store writes (the Rust store,
/// crates/seal's pairing file, whose own tests hold the writes: an entry
/// this build can't decode carried, a file that isn't a list copied aside,
/// one that can't be read never written over, the format every build
/// decodes, the pairing material and its minting).
@Suite("Push pairing file, as the notification service reads it")
struct PushPairingTests {

    /// A pairing entry in a shape this build can't decode — what a newer
    /// build might write (here: its secret moved under a versioned key).
    private func foreignPairing(_ id: UUID, host: String) -> [String: Any] {
        ["connectionId": id.uuidString, "hostLabel": host,
         "secret": ["v": 2, "hex": String(repeating: "ab", count: 32)],
         "relayURL": "https://push.example.org", "createdAt": 784_000_000.0]
    }

    private func pairing(_ host: String, at seconds: TimeInterval) -> PushPairing {
        PushPairing(connectionId: UUID(), hostLabel: host, secretHex: String(repeating: "cd", count: 32),
                    sendToken: "t", relayURL: "https://push.example.org",
                    createdAt: Date(timeIntervalSinceReferenceDate: seconds),
                    apnsToken: "ABC", apnsEnv: "sandbox", sendTokenIssuedAt: Date(timeIntervalSinceReferenceDate: seconds))
    }

    @Test("what the app's store writes reads back pairing by pairing; an entry it can't decode is skipped")
    func readsWhatTheStoreWrites() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("pairings-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("push-pairings.json")
        #expect(PushPairingFile(url: url).read().isEmpty, "no file: nothing paired yet")

        let store = RustPushPairings(url: url)
        let a = pairing("a · 研", at: 1), b = pairing("b", at: 2)
        #expect(store.upsert(a) && store.upsert(b))
        #expect(PushPairingFile(url: url).read() == [a, b])

        // A newer build's entry beside them: skipped here, kept on disk.
        var entries = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [Any])
        entries.insert(foreignPairing(UUID(), host: "newer"), at: 1)
        try JSONSerialization.data(withJSONObject: entries).write(to: url)
        #expect(PushPairingFile(url: url).read() == [a, b])
        #expect(store.read() == [a, b], "the store reads it alike")

        // Not a list at all: nothing read, nothing guessed.
        try Data("not json {{{".utf8).write(to: url)
        #expect(PushPairingFile(url: url).read().isEmpty)
    }
}
