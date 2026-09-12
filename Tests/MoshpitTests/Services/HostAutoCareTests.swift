import Foundation
import Testing
@testable import Moshpit

/// The judgment calls behind silent host care. The full tending pass needs a
/// live session and is exercised by the simulator end-to-end; what lives here
/// is the part that decides WHAT to do, which must never be wrong silently.
@Suite("Host auto-care decisions")
struct HostAutoCareTests {

    @Test("only agents the host actually carries are ever refreshed")
    func consentScope() {
        var manifest = InstallManifest.empty
        manifest[.hooks(agent: "claude")] = .init(digest: "d", installedAt: Date(),
                                                  configPath: "$HOME/.claude/settings.json")
        let agents = HostAutoCare.installedHookAgents(manifest)
        #expect(agents.map(\.id) == ["claude"],
                "refreshing an agent the user never enabled would BE the consent violation")
        #expect(HostAutoCare.installedHookAgents(.empty).isEmpty)
    }

    @Test("a device recognises its own pairing wherever an install of any age put it")
    func pairingLookupSpansFormats() {
        let conn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"
        // Modern per-device key.
        var modern = InstallManifest.empty
        modern[.pairing(conn: conn)] = .init(digest: "x", installedAt: Date(),
                                             connectionId: conn, relayURL: "r")
        #expect(modern.pairingEntry(conn: conn)?.digest == "x")

        // Legacy shared key, recorded for THIS device.
        var legacy = InstallManifest.empty
        legacy.components["pairing"] = .init(digest: "y", installedAt: Date(),
                                             connectionId: conn, relayURL: "r")
        #expect(legacy.pairingEntry(conn: conn)?.digest == "y")

        // Legacy key recorded for ANOTHER device is not ours to see — auto-care
        // reading it as its own would overwrite a different phone's pairing,
        // which is the exact single-device bug multi-device exists to end.
        #expect(legacy.pairingEntry(conn: "another-device") == nil)
    }

    @Test("removing one device's pairing leaves every other device's standing")
    func removalIsScoped() {
        var manifest = InstallManifest.empty
        manifest[.pairing(conn: "a")] = .init(installedAt: Date(), connectionId: "a", relayURL: "r")
        manifest[.pairing(conn: "b")] = .init(installedAt: Date(), connectionId: "b", relayURL: "r")
        manifest.removePairing(conn: "a")
        #expect(manifest.pairingEntry(conn: "a") == nil)
        #expect(manifest.pairingEntry(conn: "b") != nil)
        #expect(manifest.pairingEntries.count == 1)
    }
}
