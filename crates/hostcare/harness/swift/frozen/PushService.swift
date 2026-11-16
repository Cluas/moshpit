import Foundation
import Observation
import UIKit
import UserNotifications
import MoshpitKit

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
///   * ask iOS for a device token
///   * mint per-connection pairing material: the end-to-end secret locally,
///     the send token at the relay (an HMAC over the routing facts; the relay
///     keeps nothing — see push-relay/token.go)
///   * keep those credentials fresh: a rotated device token or an ageing mint
///     re-mints, and the installer rewrites the host's conf on its next pass
///   * forget a host when the user unpairs — locally, by deleting the secret,
///     which is the only revocation a stateless relay needs the phone for
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
    /// Why this phone has no usable relay credential, if it has none. nil when
    /// the last mint succeeded.
    var lastRelayError: String? { get }
    func pair(connectionId: UUID, hostLabel: String, relayURL: String) async throws -> PushPairing
    /// The stored pairing for a connection, re-minted if its credential is
    /// missing, stale, or was issued for a device token this phone no longer
    /// has. nil when nothing is paired OR the credential cannot be minted right
    /// now (no device token yet) — a pairing that is not ready must never be
    /// written to a host.
    func ensureReady(connectionId: UUID) async -> PushPairing?
    /// Forget a connection's pairing on this phone. Throws when it couldn't:
    /// the secret is still here.
    func unpair(connectionId: UUID) throws
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

    /// Asks a relay for a send token over a device token and a connection:
    /// ``PushRelayClient/mint(apnsToken:connectionId:relayURL:)`` in the app.
    typealias Mint = @MainActor (_ apnsToken: String, _ connectionId: UUID, _ relayURL: String)
        async throws -> (sendToken: String, iat: Date)

    /// The pairings file this service keeps to itself — only ever a test's.
    /// nil: the App Group store (``PushPairingStore``) the notification
    /// service extension reads, which is the app's one and only.
    private let ownFile: PushPairingFile?
    private let mint: Mint
    /// Where the device token is remembered between launches — nil for a
    /// test's service, which must not write over the app's.
    private let tokenDefaults: UserDefaults?

    private init() {
        ownFile = nil
        mint = PushRelayClient.mint
        tokenDefaults = .standard
        deviceToken = UserDefaults.standard.string(forKey: tokenKey)
    }

    /// A service over its own pairings file, asking its own relay. The app
    /// has exactly one service, ``shared``; this is for tests, which can't
    /// reach the App Group container in an unsigned build and must not
    /// touch the real one in a signed build.
    init(pairingFile: PushPairingFile, deviceToken: String?, mint: @escaping Mint) {
        ownFile = pairingFile
        self.mint = mint
        tokenDefaults = nil
        self.deviceToken = deviceToken
    }

    var pairings: [PushPairing] { ownFile?.read() ?? PushPairingStore.read() }

    private func storedPairing(for connectionId: UUID) -> PushPairing? {
        pairings.first { $0.connectionId == connectionId }
    }

    private func save(_ pairing: PushPairing) -> Bool {
        ownFile?.upsert(pairing) ?? PushPairingStore.upsert(pairing)
    }

    private func forget(_ connectionId: UUID) -> Bool {
        ownFile?.remove(connectionId: connectionId) ?? PushPairingStore.remove(connectionId: connectionId)
    }

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
            case .authorized, .provisional, .ephemeral:
                Log.push.info("\(count, privacy: .public) paired host(s), notifications authorized — asking iOS for a device token")
                Task { @MainActor in UIApplication.shared.registerForRemoteNotifications() }
            case .notDetermined:
                Log.push.info("\(count, privacy: .public) paired host(s), permission undecided — prompting")
                UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, _ in
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
        tokenDefaults?.set(hex, forKey: tokenKey)
        lastError = nil
        // Wake anything parked on the token — a first pair blocks here between
        // the permission prompt and its mint.
        for waiter in tokenWaiters.values { waiter.resume() }
        tokenWaiters.removeAll()
        if changed || syncPending {
            Task { await mintNeededPairings() }
        }
    }

    /// Wait briefly for iOS to hand over a device token, or give up.
    ///
    /// The first pair on a fresh install lands here: authorization was granted
    /// a breath ago, `registerForRemoteNotifications()` has been called, and
    /// the token arrives through the app delegate within a second or two. The
    /// same discipline as `awaitSelfTest`: removal from the dictionary is the
    /// single wake-up gate, so the continuation resumes exactly once.
    private func awaitDeviceToken(timeout: Duration = .seconds(10)) async -> String? {
        if let deviceToken { return deviceToken }
        let id = UUID()
        let timeoutTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: timeout)
            guard let self, !Task.isCancelled else { return }
            self.tokenWaiters.removeValue(forKey: id)?.resume()
        }
        await withCheckedContinuation { continuation in
            tokenWaiters[id] = continuation
        }
        timeoutTask.cancel()
        return deviceToken
    }

    @ObservationIgnored private var tokenWaiters: [UUID: CheckedContinuation<Void, Never>] = [:]

    func handleRegistrationFailure(_ error: Error) {
        Log.push.error("iOS refused to issue a device token: \(error.localizedDescription, privacy: .public)")
        // A simulator without a signed-in Apple ID fails here every launch, and
        // so does a build whose entitlements lack aps-environment. Record it
        // rather than logging into the void.
        lastError = error.localizedDescription
        syncPending = true
    }

    /// Re-mint every pairing whose credential is missing, stale, or was issued
    /// for a device token this phone no longer has. Called on token change,
    /// after pairing, and on return to foreground.
    ///
    /// There is nothing to "announce" any more — the relay keeps no registry —
    /// so a healthy pairing costs zero requests here. The installer notices an
    /// updated credential by conf digest and rewrites the host's file on its
    /// next pass.
    func mintNeededPairings() async {
        guard deviceToken != nil else {
            Log.push.info("mint deferred — no device token yet")
            syncPending = true
            return
        }
        // Two overlapping rounds cannot corrupt anything (both are on the main
        // actor) but they can interleave, so the surviving `lastError` comes
        // from whichever finished last rather than whichever ran last. One at a
        // time — but a request made while a round is out is not dropped. It is
        // what a device token changing mid-round looks like: dropped, every
        // pairing that round minted stayed on the old token until the next
        // return to the foreground past the refresh floor, or the next
        // connection. It is remembered instead, and the round in flight runs
        // once more when it is done — once however many came in, with the
        // token current by then.
        guard !syncInFlight else {
            mintRoundRequested = true
            return
        }
        syncInFlight = true
        defer { syncInFlight = false }
        repeat {
            mintRoundRequested = false
            guard let token = deviceToken else { return }
            await mintRound(token: token)
        } while mintRoundRequested
    }

    /// One pass over the pairings, minting each that needs it over `token`.
    private func mintRound(token: String) async {
        var failed = false
        for listed in pairings where listed.needsMint(currentToken: token) {
            // Read again first: the list is from before the first relay
            // round trip, and a pairing forgotten since — its connection
            // deleted while another host's token was out — must not have its
            // relay asked about it again.
            guard let pairing = storedPairing(for: listed.connectionId),
                  pairing.needsMint(currentToken: token) else { continue }
            do {
                _ = try await mintAndStore(pairing, token: token)
            } catch {
                failed = true
                lastError = error.localizedDescription
                Log.push.error("relay \(pairing.relayURL, privacy: .public) refused to mint: \(error.localizedDescription, privacy: .public)")
            }
        }
        syncPending = failed
        if !failed {
            lastError = nil
            lastSyncSucceededAt = Date()
        }
    }

    /// Ask the relay for a send token over the CURRENT device token and store
    /// the updated pairing. The relay keeps nothing; what it returns is an HMAC
    /// it can recompute from the routing facts every push carries.
    ///
    /// nil when the pairing moved on while the relay was answering — it was
    /// forgotten (its connection deleted, its host unpaired) or replaced (re-
    /// paired, or pointed at another relay) — and nothing was stored. Writing
    /// the answer back regardless brought a forgotten pairing back to life:
    /// its secret on the phone again, and its token minted again on every
    /// pass after, for a connection that no longer existed.
    private func mintAndStore(_ pairing: PushPairing, token: String) async throws -> PushPairing? {
        let minted = try await mint(token, pairing.connectionId, pairing.relayURL)
        guard var updated = storedPairing(for: pairing.connectionId),
              updated.secretHex == pairing.secretHex,
              updated.relayURL == pairing.relayURL else {
            Log.push.info("pairing for \(pairing.hostLabel, privacy: .public) was forgotten or replaced while minting — nothing stored")
            return nil
        }
        updated.sendToken = minted.sendToken
        updated.sendTokenIssuedAt = minted.iat
        updated.apnsToken = token
        updated.apnsEnv = Self.apnsEnvironment
        guard save(updated) else { throw PushRelayClient.Failure.couldNotSave }
        Log.push.info("minted a send token for \(pairing.hostLabel, privacy: .public) at \(pairing.relayURL, privacy: .public)")
        return updated
    }

    func ensureReady(connectionId: UUID) async -> PushPairing? {
        guard let pairing = storedPairing(for: connectionId) else { return nil }
        guard pairing.needsMint(currentToken: deviceToken) else { return pairing }
        registerForRemoteNotifications()
        guard let token = await awaitDeviceToken() else {
            Log.push.info("credential for \(pairing.hostLabel, privacy: .public) not ready — no device token")
            return nil
        }
        do {
            return try await mintAndStore(pairing, token: token)
        } catch {
            lastError = error.localizedDescription
            Log.push.error("mint for \(pairing.hostLabel, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
            return nil
        }
    }

    /// Called when the app returns to the foreground: a cheap local check that
    /// re-mints only what needs it (rotated device token, ageing credential).
    func refreshRegistration() {
        guard !pairings.isEmpty, deviceToken != nil else {
            if syncPending { registerForRemoteNotifications() }
            return
        }
        guard !Self.shouldSkipRefresh(syncPending: syncPending,
                                      lastSuccess: lastSyncSucceededAt,
                                      now: Date()) else { return }
        Task { await mintNeededPairings() }
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
    /// A mint was asked for while a round was out: run another when it ends.
    @ObservationIgnored private var mintRoundRequested = false

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

    /// Mint material for a connection and return a READY pairing — end-to-end
    /// secret from this phone's CSPRNG, send token from the relay.
    ///
    /// The relay is asked BEFORE the host is touched, for the same reason
    /// registration used to be: if the mint fails there is nothing to undo on
    /// the host, and the user sees the real error instead of a host configured
    /// with a credential that could never have worked.
    ///
    /// A first-ever pair usually lands here seconds after notification
    /// permission was granted, before iOS has handed over a device token — so
    /// this waits briefly for one rather than failing the commonest path.
    func pair(connectionId: UUID, hostLabel: String, relayURL: String) async throws -> PushPairing {
        // A re-pair mints a fresh SECRET on purpose: the previous conf on that
        // host stops decrypting, which is the whole point of re-pairing after a
        // machine changes hands. (Its send token dies of relay-side TTL — a
        // stateless relay has no row to delete — but a token without the secret
        // can only ever produce the generic fallback line.)
        let pairing = PushPairing.make(connectionId: connectionId,
                                       hostLabel: hostLabel,
                                       relayURL: relayURL)
        // Abort before the host is touched. A pairing the phone failed to save,
        // installed on a host anyway, is the worst arrangement available: the
        // host holds secrets, every push it sends is undecryptable, and there is
        // no local record left to unpair with.
        guard save(pairing) else { throw PushRelayClient.Failure.couldNotSave }
        registerForRemoteNotifications()
        guard let token = await awaitDeviceToken() else {
            // Stored but credential-less: `ensureReady`/`mintNeededPairings`
            // completes it the moment a token exists. Failing loudly here beats
            // silently installing a conf the sender would skip.
            syncPending = true
            throw PushRelayClient.Failure.noDeviceToken
        }
        // Forgotten or replaced while the relay answered: what would be
        // installed is no longer this phone's pairing.
        guard let ready = try await mintAndStore(pairing, token: token) else {
            throw PushRelayClient.Failure.couldNotSave
        }
        return ready
    }

    /// Forget a host. Local-only, and that is now the whole story: deleting the
    /// pairing SECRET is what stops this device reading anything, and the relay
    /// has no registration to forget. A conf left on an unreachable host keeps
    /// a valid send token until its TTL runs out — such a host can ring the
    /// generic fallback line, never content — and the unpair flow's host-side
    /// file removal handles the reachable case.
    ///
    /// Throws when the pairings file couldn't be written (or read, or kept a
    /// copy of): the secret is still on this phone, and the send token is
    /// still re-minted for it. That used to be dropped without a word, and
    /// the screen went on to call the host unpaired — after which the next
    /// connect's auto-care, finding the pairing still here, installed it on
    /// the host again.
    func unpair(connectionId: UUID) throws {
        guard forget(connectionId) else {
            throw PushRelayClient.Failure.couldNotForget
        }
    }

    // MARK: - Pairings whose connection is gone

    /// Set once this launch's sweep has run.
    @ObservationIgnored private var orphansSwept = false

    /// Forget every pairing whose connection is no longer saved — once per
    /// launch, the first time the saved connections can be trusted.
    ///
    /// A delete takes its connection's pairing with it now
    /// (``ConnectionDeletion``); connections deleted before that left theirs
    /// behind — listed under Settings' paired hosts, their secrets on the
    /// phone, their send tokens minted at the relay on every pass, for hosts
    /// nothing on screen reaches any more.
    ///
    /// `saved` is every connection the store holds, decoded or not
    /// (``ConnectionStore/savedIds``): a pairing whose connection this build
    /// merely can't decode stays, its connection is still the user's. nil
    /// says which connections are saved can't be told — the store can't be
    /// trusted yet (a launch before the first unlock reads the sealed list
    /// as empty), or holds something it can't tell by id — and sweeps
    /// nothing; the sweep waits for a later call. Returns what was forgotten.
    @discardableResult
    func forgetOrphanedPairingsOnce(saved: Set<UUID>?) -> [PushPairing] {
        guard !orphansSwept else { return [] }
        guard let saved else {
            Log.push.info("orphaned pairings: which connections are saved can't be told yet — not sweeping")
            return []
        }
        orphansSwept = true
        var forgotten: [PushPairing] = []
        for pairing in pairings where !saved.contains(pairing.connectionId) {
            let label = "\(pairing.hostLabel) (\(pairing.connectionId.uuidString))"
            if forget(pairing.connectionId) {
                forgotten.append(pairing)
                Log.push.notice("forgot the pairing for \(label, privacy: .public): its connection is no longer saved")
            } else {
                Log.push.error("couldn't forget the pairing for \(label, privacy: .public), whose connection is no longer saved")
            }
        }
        return forgotten
    }
}

/// The relay's HTTP surface, as seen from the phone. Exactly one call: "here is
/// my device token and a connection id — mint me a send token". The relay
/// answers with an HMAC and remembers nothing.
enum PushRelayClient {
    enum Failure: LocalizedError {
        case badURL
        case couldNotSave
        case couldNotForget
        case noDeviceToken
        case status(Int, String)

        var errorDescription: String? {
            switch self {
            case .badURL:
                return String(localized: "That relay address isn't a valid URL.")
            case .couldNotSave:
                return String(localized: "Couldn't save the pairing on this phone, so nothing was installed on the host. The diagnostics log has the reason.")
            case .couldNotForget:
                return String(localized: "Couldn't remove the pairing from this phone, so the host was left paired. The diagnostics log has the reason.")
            case .noDeviceToken:
                return String(localized: "iOS hasn't issued a push token to this phone yet — check that notifications are allowed, then try again.")
            case let .status(code, body):
                return String(localized: "Relay refused to mint a send token (\(code)): \(body)")
            }
        }
    }

    /// Ask the relay for a send token over this device token and connection.
    static func mint(apnsToken: String, connectionId: UUID,
                     relayURL: String) async throws -> (sendToken: String, iat: Date) {
        guard let url = URL(string: relayURL + "/v1/mint"),
              url.scheme == "https" || url.host == "localhost" || url.host == "127.0.0.1"
        else { throw Failure.badURL }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.timeoutInterval = 15
        let payload = [
            "apnsToken": apnsToken,
            "conn": connectionId.uuidString,
        ]
        request.httpBody = try JSONEncoder().encode(payload)

        // One retry, on NETWORK failure only. A live re-pair was watched dying
        // at exactly this step on the old register flow — a transient error
        // that one retry would have covered. An HTTP status is the relay's
        // ANSWER; asking again would not change it.
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
        struct Minted: Decodable {
            let sendToken: String
            let iat: Int64
        }
        let minted = try JSONDecoder().decode(Minted.self, from: data)
        return (minted.sendToken, Date(timeIntervalSince1970: TimeInterval(minted.iat)))
    }
}

/// Minimal app delegate, added solely because remote-notification tokens are
/// delivered nowhere else — SwiftUI has no equivalent hook. Keep it an
/// NSObject: as a UIResponder vending `keyCommands` it silently broke every
/// SwiftUI `.keyboardShortcut` in the app.
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
