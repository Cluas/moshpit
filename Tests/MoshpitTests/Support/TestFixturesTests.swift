import Foundation
import Testing

/// The fixtures must travel with the test bundle: a real device can't read
/// the Mac checkout their `#filePath` names (see ``TestFixtures``).
@Suite("test fixtures")
struct TestFixturesTests {

    /// Every fixture a suite reads, relative to `Fixtures/`.
    static let used = [
        "herdr-snapshot.json",
        "herdr-socket-snapshot.json",
        "herdr-socket-events.jsonl",
        "home-acceptance.json",
        "mosh-switch/meta.json",
        "mosh-switch/reference.bin",
        "mosh-switch/phase0.bin", "mosh-switch/phase1.bin",
        "mosh-switch/phase2.bin", "mosh-switch/phase3.bin",
        "mosh-switch/truth0.txt", "mosh-switch/truth1.txt",
        "mosh-switch/truth2.txt", "mosh-switch/truth3.txt",
    ]

    @Test("The fixtures ship inside the test bundle, folder layout and all")
    func fixturesAreBundled() throws {
        let root = try #require(TestFixtures.bundledRoot)
        for path in Self.used {
            #expect(FileManager.default.fileExists(atPath: root.appendingPathComponent(path).path),
                    "\(path) is not in the test bundle — a device run can't read the checkout")
        }
    }

    @Test("Fixture reads resolve to the bundled copy")
    func readsResolveToTheBundle() throws {
        let root = try #require(TestFixtures.bundledRoot)
        for path in Self.used {
            #expect(TestFixtures.url(path).standardizedFileURL.path
                        .hasPrefix(root.standardizedFileURL.path),
                    "\(path) resolved outside the bundle")
        }
    }
}
