import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// The push suites persist through the App Group container. A build with
/// CODE_SIGNING_ALLOWED=NO (every public CI run, every checkout without a
/// Team) carries no App Group entitlement, so the container is nil and each
/// store reads back empty: a red that says "unsigned build", not "bug". The
/// suites that need the container skip instead of failing.
enum AppGroupContainer {
    static let isAvailable = FileManager.default
        .containerURL(forSecurityApplicationGroupIdentifier: PushPairingStore.appGroup) != nil
    static let skipReason: Comment = "App Group container unavailable (unsigned build, CODE_SIGNING_ALLOWED=NO)"
}
