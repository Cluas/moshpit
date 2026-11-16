import Foundation
import UIKit
import UserNotifications
import MoshpitKit

/// The iOS half of remote push: the device token and the permission that
/// gets one. Everything else about pairing — minting at the relay, keeping
/// credentials fresh, pairing and unpairing a host, the self-test pushes,
/// the pairings a deleted connection left — is the core's
/// (``HostCare``, `crates/hostcare`), which asks for a token through
/// ``register()`` and hears of one through ``handle(deviceToken:)``.
///
/// A push is the only mechanism iOS gives for waking the app when an agent
/// asks for something, and Apple only delivers one signed with this app's
/// team key — which is why a relay exists at all. The incoming notification
/// itself is not handled here: it carries the same category and `userInfo`
/// a local one does, so `AgentNotificationHandler` routes it.
@MainActor
enum PushRegistration {
    private static let tokenKey = "moshpit.push.deviceToken"

    /// The device token iOS handed over on an earlier launch, hex.
    static var storedToken: String? { UserDefaults.standard.string(forKey: tokenKey) }

    /// Which APNs environment this build's tokens belong to. A
    /// development-signed build (anything out of Xcode) gets a sandbox token;
    /// App Store and TestFlight builds get production ones. The one case this
    /// guess gets wrong — a TestFlight build installed over a development
    /// one — self-corrects at the relay, which retries the other host on
    /// BadDeviceToken and remembers the answer.
    nonisolated static var apnsEnvironment: String {
        #if DEBUG
        "sandbox"
        #else
        "production"
        #endif
    }

    /// Ask iOS for a device token. The core asks only with something
    /// paired: an install with nothing paired never prompts.
    ///
    /// A paired install with permission still UNDECIDED does prompt. It has
    /// to: authorization is otherwise only requested when a session is
    /// tracked, so someone who paired a host and had not yet connected would
    /// sit with a working relay, a working hook, and no device token. Pairing
    /// a host IS the moment the user asked for notifications from it.
    static func register() {
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral:
                Log.push.info("paired host(s), notifications authorized — asking iOS for a device token")
                Task { @MainActor in UIApplication.shared.registerForRemoteNotifications() }
            case .notDetermined:
                Log.push.info("paired host(s), permission undecided — prompting")
                UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, _ in
                    Log.push.info("notification permission granted=\(granted, privacy: .public)")
                    guard granted else { return }
                    Task { @MainActor in UIApplication.shared.registerForRemoteNotifications() }
                }
            case .denied:
                // Settings is the only place this can be reversed. Worth a log
                // line: from the outside it looks like every other silence.
                Log.push.info("notifications denied — pushes cannot be shown until Settings is changed")
            @unknown default:
                break
            }
        }
    }

    static func handle(deviceToken data: Data) {
        let hex = data.map { String(format: "%02x", $0) }.joined()
        // A prefix only: enough to line up with the relay's own fingerprints
        // in a log, not enough to be usable as an address.
        Log.push.info("device token \(hex.prefix(8), privacy: .public)… changed=\(hex != storedToken, privacy: .public)")
        UserDefaults.standard.set(hex, forKey: tokenKey)
        HostCare.shared.core.deviceToken(token: hex)
    }

    static func handleRegistrationFailure(_ error: Error) {
        // A build whose entitlements lack aps-environment fails here every
        // launch. Recorded, so the Host Setup screen can say so.
        Log.push.error("iOS refused to issue a device token: \(error.localizedDescription, privacy: .public)")
        HostCare.shared.core.registrationFailed(why: error.localizedDescription)
    }
}

/// Minimal app delegate, added solely because remote-notification tokens are
/// delivered nowhere else — SwiftUI has no equivalent hook. Keep it an
/// NSObject: as a UIResponder vending `keyCommands` it silently broke every
/// SwiftUI `.keyboardShortcut` in the app.
final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { @MainActor in PushRegistration.handle(deviceToken: deviceToken) }
    }

    func application(_ application: UIApplication,
                     didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Task { @MainActor in PushRegistration.handleRegistrationFailure(error) }
    }
}
