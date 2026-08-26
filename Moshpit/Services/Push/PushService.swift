import Foundation
import Observation
import UIKit
import UserNotifications

/// Remote-push half of the Vibe Island control surface.
///
/// The local path — `AgentActivityMonitor` watching a live tmux channel — only
/// reaches a phone that is running Moshpit with a session attached. iOS suspends
/// the app within seconds of it leaving the screen and kills the socket shortly
/// after, and no amount of local cleverness gets it back: nothing in the app is
/// executing to notice the agent asking. A push is the only mechanism iOS gives
/// for waking it, and Apple will only deliver one signed with this app's team
/// key — which is why a relay exists at all.
///
/// This service owns the phone's side of that arrangement:
///   * ask iOS for a device token, and keep the relay's copy of it current
///   * mint per-connection pairing material and hand it to the host as a
///     one-liner
///   * forget a host cleanly when the user unpairs
///
/// It deliberately does NOT handle the incoming notification: a pushed
/// notification carries the same category and `userInfo` a local one does, so
/// `AgentNotificationHandler` routes it through `AgentControlBridge` with no new
/// code. That equivalence is the reason T1 is small.
/// What the host-setup screen needs from the push side.
///
/// A protocol only so that screen's state machine is testable: pairing and
/// proving are the two paths where a wrong state costs the user a phone that
/// stays silent, and they cannot be exercised against real APNs in a unit test.
@MainActor
protocol PushCoordinating: AnyObject {
    /// Why this phone is not registered with its relay, if it is not. nil when
    /// the last attempt succeeded.
    var lastRelayError: String? { get }
    func pair(connectionId: UUID, hostLabel: String, relayURL: String) async throws -> PushPairing
    func unpair(connectionId: UUID)
    func awaitSelfTest(nonce: String, timeout: Duration) async -> Bool
    func forgetSelfTest(nonce: String)
}

@MainActor
@Observable
final class PushService: PushCoordinating {
    static let shared = PushService()

    /// The current APNs device token, hex. nil until iOS hands one over — which
    /// requires notification authorization AND network.
    private(set) var deviceToken: String?
    /// Last registration error.
    ///
    /// Read by the host-setup sheet's push section (`HostSetupModel.relayError`).
    /// It used to say "surfaced in Settings", which was the one thing it was not
    /// doing: four writers, zero readers, so the only value that could tell a
    /// user "your relay does not know this phone" was computed and dropped. That
    /// got heavier when foreground re-announcement became unconditional — every
    /// return to the app recomputes it. A property that claims to tell the user
    /// something and does not is worse than no property, which is the standard
    /// `postDeliveryFailure()` already set here.
    private(set) var lastError: String?

    private let tokenKey = "moshpit.push.deviceToken"
    private var syncPending = false

    private init() {
        deviceToken = UserDefaults.standard.string(forKey: tokenKey)
    }

    var pairings: [PushPairing] { PushPairingStore.read() }

    /// Which APNs environment this build's tokens belong to.
    ///
    /// A development-signed build (anything out of Xcode) gets a sandbox token;
    /// App Store and TestFlight builds get production ones. The one case this
    /// guess gets wrong — a TestFlight build installed over a development one —
    /// self-corrects at the relay, which retries the other host on
    /// BadDeviceToken and remembers the answer.
    var lastRelayError: String? { lastError }

    nonisolated static var apnsEnvironment: String {
        #if DEBUG
        "sandbox"
        #else
        "production"
        #endif
    }

    // MARK: - Token lifecycle

    /// Ask iOS for a device token. Safe to call repeatedly; iOS answers from
    /// cache when it can, which is why this runs on every launch — tokens rotate
    /// on restore, reinstall and occasionally on OS update, and a stale one at
    /// the relay is a notification that silently goes nowhere.
    ///
    /// An install with nothing paired never prompts and never registers: there
    /// is no host that could push to it, so asking would be asking for nothing.
    ///
    /// A paired install with permission still UNDECIDED does prompt. It has to:
    /// authorization is otherwise only ever requested when a session is tracked
    /// (`AgentActivityMonitor.track`), so someone who paired a host and had not
    /// yet connected would sit with a working relay, a working hook, and no
    /// device token — pushes accepted by APNs and dropped on the floor, with
    /// nothing anywhere saying why. Pairing a host IS the moment the user asked
    /// for notifications from it.
    func registerForRemoteNotifications() {
        let count = pairings.count
        guard count > 0 else {
            Log.push.info("no paired hosts — not registering for remote notifications")
            return
        }
        let center = UNUserNotificationCenter.current()
        center.getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .authorized, .provisional:
                Log.push.info("\(count, privacy: .public) paired host(s), notifications authorized — asking iOS for a device token")
                Task { @MainActor in UIApplication.shared.registerForRemoteNotifications() }
            case .notDetermined:
                Log.push.info("\(count, privacy: .public) paired host(s), permission undecided — prompting")
                center.requestAuthorization(options: [.alert, .sound, .badge]) { granted, _ in
                    Log.push.info("notification permission granted=\(granted, privacy: .public)")
                    guard granted else { return }
                    Task { @MainActor in UIApplication.shared.registerForRemoteNotifications() }
                }
            case .denied:
                // Nothing to do and nothing to say to the OS here — Settings is
                // the only place this can be reversed, and the pairing screen is
                // where that belongs, not a prompt iOS will refuse to show
                // again. Worth a log line, because from the outside it is
                // indistinguishable from every other reason for silence.
                Log.push.info("notifications denied — pushes cannot be shown until Settings is changed")
            @unknown default:
                break
            }
        }
    }

    func handle(deviceToken data: Data) {
        let hex = data.moshpitHexString
        let changed = hex != deviceToken
        // A prefix only: enough to line up with the relay's own fingerprints in
        // a log, not enough to be usable as an address.
        Log.push.info("device token \(hex.prefix(8), privacy: .public)… changed=\(changed, privacy: .public)")
        deviceToken = hex
        UserDefaults.standard.set(hex, forKey: tokenKey)
        lastError = nil
        if changed || syncPending {
            Task { await syncAllPairings() }
        }
    }

    func handleRegistrationFailure(_ error: Error) {
        Log.push.error("iOS refused to issue a device token: \(error.localizedDescription, privacy: .public)")
        // A simulator without a signed-in Apple ID fails here every launch, and
        // so does a build whose entitlements lack aps-environment. Record it
        // rather than logging into the void.
        lastError = error.localizedDescription
        syncPending = true
    }

    /// Re-announce every pairing to its relay. Called on token change, after
    /// pairing, and on return to foreground when an earlier attempt failed.
    func syncAllPairings() async {
        guard let token = deviceToken else {
            Log.push.info("relay sync deferred — no device token yet")
            syncPending = true
            return
        }
        // Two overlapping runs cannot corrupt anything (both are on the main
        // actor) but they can interleave, so the surviving `lastError` comes from
        // whichever finished last rather than whichever ran last. One at a time.
        guard !syncInFlight else { return }
        syncInFlight = true
        defer { syncInFlight = false }
        var failed = false
        for pairing in pairings {
            do {
                try await PushRelayClient.register(apnsToken: token, pairing: pairing)
                Log.push.info("registered with relay \(pairing.relayURL, privacy: .public)")
            } catch {
                failed = true
                lastError = error.localizedDescription
                Log.push.error("relay \(pairing.relayURL, privacy: .public) rejected registration: \(error.localizedDescription, privacy: .public)")
            }
        }
        syncPending = failed
        if !failed {
            lastError = nil
            lastSyncSucceededAt = Date()
        }
    }

    /// Called when the app returns to the foreground.
    ///
    /// Re-announces unconditionally, not just when a previous attempt failed. A
    /// relay that lost its registry — restored from an older volume, redeployed
    /// without one — otherwise never hears from this phone again: the token has
    /// not changed, so nothing is pending, and every push it is asked to send
    /// 401s forever with no way for the user to find out. Observed exactly that
    /// while testing against a freshly started relay. One small POST per
    /// foreground buys a system that heals itself.
    func refreshRegistration() {
        guard !pairings.isEmpty, deviceToken != nil else {
            if syncPending { registerForRemoteNotifications() }
            return
        }
        guard !Self.shouldSkipRefresh(syncPending: syncPending,
                                      lastSuccess: lastSyncSucceededAt,
                                      now: Date()) else { return }
        Task { await syncAllPairings() }
    }

    /// The floor on foreground re-announcement, as a pure decision so it is
    /// pinned by a test rather than only exercised by a phone.
    ///
    /// `.active` fires every time the app switcher is scrubbed past, so without
    /// this N paired hosts cost N posts per glance — and a few hosts plus fast
    /// switching is enough to earn the ingress rate limit, which (until the
    /// error was wired to the screen) the user could not have seen. A pending
    /// failure skips the floor: that is the case worth being eager about.
    static func shouldSkipRefresh(syncPending: Bool, lastSuccess: Date?,
                                  now: Date, floor: TimeInterval = 60) -> Bool {
        guard !syncPending, let lastSuccess else { return false }
        return now.timeIntervalSince(lastSuccess) < floor
    }

    @ObservationIgnored private var lastSyncSucceededAt: Date?
    @ObservationIgnored private var syncInFlight = false

    // MARK: - Proving a pairing

    /// Nonces this device has seen arrive, and whoever is waiting for one.
    ///
    /// A set rather than "the last one", because the push can beat the exec
    /// channel's own reply: the sheet asks the host to fire a test and starts
    /// waiting, but on a fast link the notification is already in when the wait
    /// begins. Recording arrivals means the wait cannot miss one it caused.
    @ObservationIgnored private var arrivedSelfTests: Set<String> = []
    @ObservationIgnored private var selfTestWaiters: [String: CheckedContinuation<Void, Never>] = [:]

    /// Called from the notification delegate when a self-test push lands.
    func noteSelfTestArrived(nonce: String) {
        Log.push.info("self-test push arrived: \(nonce, privacy: .public)")
        arrivedSelfTests.insert(nonce)
        selfTestWaiters.removeValue(forKey: nonce)?.resume()
    }

    /// Wait for a specific self-test push, or give up.
    ///
    /// Returns true only if THIS nonce arrived. A stale test push from an
    /// earlier attempt satisfies nothing, which is the whole reason the nonce
    /// exists — the old install flow's equivalent check resolved on any evidence
    /// it could find and called that success.
    ///
    /// Arrival and timeout share ONE wake-up path — `selfTestWaiters.removeValue`
    /// — so the continuation is resumed exactly once and by whichever comes
    /// first. That is not style: a `withTaskGroup` racing a sleeper against a
    /// `withCheckedContinuation` deadlocks on the timeout, because the group
    /// awaits every child before returning and `cancelAll()` cannot resume a
    /// continuation. It hung forever, which left `HostSetupModel.provePush()`
    /// spinning in `.proving` and made its timeout message unreachable — on the
    /// commonest first-pair path of all, where the relay has no device token yet
    /// and no push was ever going to arrive.
    func awaitSelfTest(nonce: String, timeout: Duration = .seconds(20)) async -> Bool {
        if arrivedSelfTests.contains(nonce) { return true }
        let timeoutTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: timeout)
            guard let self, !Task.isCancelled else { return }
            // removeValue is the gate: whoever takes the continuation out of the
            // dictionary owns resuming it, and there is only ever one taker.
            if let waiter = self.selfTestWaiters.removeValue(forKey: nonce) {
                Log.push.info("self-test \(nonce, privacy: .public) never arrived — giving up")
                waiter.resume()
            }
        }
        // Resumed by noteSelfTestArrived, by the timeout above, or by
        // forgetSelfTest — never by more than one of them.
        await withCheckedContinuation { continuation in
            selfTestWaiters[nonce] = continuation
        }
        timeoutTask.cancel()
        return arrivedSelfTests.contains(nonce)
    }

    /// Forget a finished attempt so a long-lived app does not accumulate nonces.
    ///
    /// Resumes a waiter that is somehow still parked rather than dropping its
    /// continuation on the floor: an abandoned `withCheckedContinuation` is a
    /// task that never finishes, and this whole path already paid for that once.
    func forgetSelfTest(nonce: String) {
        arrivedSelfTests.remove(nonce)
        selfTestWaiters.removeValue(forKey: nonce)?.resume()
    }

    // MARK: - Pairing

    /// Mint material for a connection, tell the relay, and return the pairing so
    /// the caller can show its one-liner.
    ///
    /// The relay is told BEFORE the user runs the one-liner on purpose: if
    /// registration fails there is nothing to undo on the host, and the user
    /// sees the error instead of pasting a command that could never have worked.
    func pair(connectionId: UUID, hostLabel: String, relayURL: String) async throws -> PushPairing {
        let pairing = PushPairing.make(connectionId: connectionId,
                                       hostLabel: hostLabel,
                                       relayURL: relayURL)
        // Retire the previous registration FIRST, while its send token still
        // exists. A re-pair replaces the local pairing and overwrites the host's
        // push.conf, so a moment from now that token has no copy anywhere — and
        // the relay's row for it can never be authenticated away again. It would
        // sit there holding this phone's device token forever: harmless to use,
        // but counted against MOSHPIT_RELAY_MAX_DEVICES, and unreclaimable
        // because a row is only dropped when APNs calls the token Gone, which a
        // live token never is. One zombie per re-pair adds up fast on anyone
        // debugging a pairing.
        if let previous = PushPairingStore.pairing(for: connectionId) {
            await forgetAtRelay(previous)
        }
        // Abort before the host is touched. A pairing the phone failed to save,
        // installed on a host anyway, is the worst arrangement available: the
        // host holds secrets, every push it sends is undecryptable, and there is
        // no local record left to unpair with.
        guard PushPairingStore.upsert(pairing) else { throw PushRelayClient.Failure.couldNotSave }
        registerForRemoteNotifications()
        if let token = deviceToken {
            try await PushRelayClient.register(apnsToken: token, pairing: pairing)
        } else {
            // No token yet (authorization just granted, or offline). The pairing
            // is stored and will be announced by handle(deviceToken:).
            syncPending = true
        }
        return pairing
    }

    /// Forget a host, and tell the relay to forget it too.
    ///
    /// The delete IS authenticated — this phone holds the send token, which is
    /// the credential /v1/notify checks — so an earlier note claiming otherwise
    /// was wrong. It matters when the host is unreachable at unpair time and its
    /// `push.conf` survives: without this, that registration keeps delivering
    /// notifications the app can no longer open.
    ///
    /// Best effort, and deliberately not blocking: the local copy of the secret
    /// goes regardless, because that is the part that stops this device reading
    /// anything. A relay that was down keeps a row until the next attempt.
    func unpair(connectionId: UUID) {
        let pairing = PushPairingStore.pairing(for: connectionId)
        PushPairingStore.remove(connectionId: connectionId)
        guard let pairing else { return }
        Task { await forgetAtRelay(pairing) }
    }

    /// Ask the relay to drop one registration. Shared by unpair and re-pair,
    /// because a re-pair IS an unpair followed by a pair — it just never had the
    /// first half.
    ///
    /// Best effort on purpose: a relay that is down must not block pairing. The
    /// cost of a miss is one stale row, which is exactly what this exists to
    /// avoid, so it is worth a log line either way.
    private func forgetAtRelay(_ pairing: PushPairing) async {
        do {
            try await PushRelayClient.unregister(pairing: pairing)
            Log.push.info("relay dropped the registration for \(pairing.hostLabel, privacy: .public)")
        } catch {
            Log.push.error("relay still holds a registration for \(pairing.hostLabel, privacy: .public): \(error.localizedDescription, privacy: .public)")
        }
    }
}

/// The relay's HTTP surface, as seen from the phone. Exactly one call: "this is
/// my device token, file it under this send-token hash".
enum PushRelayClient {
    enum Failure: LocalizedError {
        case badURL
        case couldNotSave
        case status(Int, String)

        var errorDescription: String? {
            switch self {
            case .badURL:
                return String(localized: "That relay address isn't a valid URL.")
            case .couldNotSave:
                return String(localized: "Couldn't save the pairing on this phone, so nothing was installed on the host. The diagnostics log has the reason.")
            case let .status(code, body):
                return String(localized: "Relay rejected the registration (\(code)): \(body)")
            }
        }
    }

    /// Ask the relay to forget this device. Authenticated with the send token,
    /// the same credential a push carries.
    static func unregister(pairing: PushPairing) async throws {
        guard let url = URL(string: pairing.relayURL + "/v1/register") else {
            throw Failure.badURL
        }
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        request.setValue("Bearer \(pairing.sendToken)", forHTTPHeaderField: "authorization")
        request.timeoutInterval = 10
        let (data, response) = try await URLSession.shared.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else {
            throw Failure.status(code, String(data: data.prefix(200), encoding: .utf8) ?? "")
        }
    }

    static func register(apnsToken: String, pairing: PushPairing) async throws {
        guard let url = URL(string: pairing.relayURL + "/v1/register"),
              url.scheme == "https" || url.host == "localhost" || url.host == "127.0.0.1"
        else { throw Failure.badURL }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.timeoutInterval = 15
        let payload = [
            "apnsToken": apnsToken,
            "sendTokenHash": pairing.sendTokenHash,
            "env": PushService.apnsEnvironment,
        ]
        request.httpBody = try JSONEncoder().encode(payload)

        // One retry, on NETWORK failure only. A live re-pair was watched dying
        // at exactly this step: the old registration had just been retired
        // (forget-first — see pair()), the new register hit a transient error,
        // and the device was left registered NOWHERE until auto-care's next
        // pass — sixteen minutes of silence that one retry would have covered.
        // An HTTP status is the relay's ANSWER; asking again would not change it.
        let (data, response): (Data, URLResponse)
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            try await Task.sleep(for: .seconds(2))
            (data, response) = try await URLSession.shared.data(for: request)
        }
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else {
            throw Failure.status(code, String(data: data.prefix(200), encoding: .utf8) ?? "")
        }
    }
}

/// Minimal app delegate, added solely because remote-notification tokens are
/// delivered nowhere else — SwiftUI has no equivalent hook.
final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { @MainActor in PushService.shared.handle(deviceToken: deviceToken) }
    }

    func application(_ application: UIApplication,
                     didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Task { @MainActor in PushService.shared.handleRegistrationFailure(error) }
    }
}
