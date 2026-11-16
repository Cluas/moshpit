import Foundation
import Testing
@testable import Moshpit

/// The pet counts each device's finished tasks under an id the device keeps
/// in its preferences (``CompanionPet/deviceId(_:protectedData:)``). That is a
/// write into preferences sealed until the first unlock, and the
/// sealed-launch rule holds (B866): nothing is written while they can't be
/// trusted — a write then can leave the file holding that one key.
@Suite("The pet's device id is kept once the defaults can be trusted")
@MainActor
struct PetDeviceIdTests {

    @Test("a launch before the first unlock keeps no id; the first trusted one keeps it for good")
    func sealedLaunchKeepsNoId() throws {
        let suite = "test.pet.device.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        let sealed: () -> Bool = { false }
        let launchOnly = CompanionPet.deviceId(defaults, protectedData: sealed)
        #expect(!launchOnly.isEmpty)
        #expect(defaults.object(forKey: "moshpit.pet.device") == nil,
                "written while the preferences could still be sealed")

        let open: () -> Bool = { true }
        let kept = CompanionPet.deviceId(defaults, protectedData: open)
        #expect(defaults.string(forKey: "moshpit.pet.device") == kept)
        #expect(CompanionPet.deviceId(defaults, protectedData: open) == kept, "the same id, every launch")
        #expect(CompanionPet.deviceId(defaults, protectedData: sealed) == kept,
                "a later sealed launch that can read it uses it")
    }
}
