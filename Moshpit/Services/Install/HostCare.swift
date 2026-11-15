import Foundation
import Observation
import Security
import MoshpitKit

/// Host care, the Host Setup screen, Install Assist's plan and the phone's
/// side of push pairing: the Rust core's machine (`crates/hostcare`,
/// through `crates/ffi`'s ``HostCareCore``) — the one Android runs too.
/// What used to be `HostAutoCare`, `HostSetupModel` and the pairing half of
/// `PushService` decides nothing here any more: this is the machine's iOS
/// platform.
///
/// What stays in Swift is what needs iOS:
///  * a command on a connection's host, on its live session's own
///    connection (``SessionHub/ActiveSession/acquireFileTransferSSH()``:
///    `borrowSsh` on the Rust core, the session's SSH otherwise);
///  * the HTTPS request to the relay (`URLSession`);
///  * the device token and the notification permission
///    (``PushRegistration``);
///  * the App Group's pairings file the notification service reads
///    (``PushPairingStore``, with its protection class).
///
/// The app tells it what it reads — the settings and the saved connections
/// — before every ask, and whenever the saved connections change.
@MainActor
@Observable
final class HostCare {
    static let shared = HostCare()

    /// The connection whose host has no hooks yet: the one question care
    /// asks (``TerminalScreen``'s alert).
    private(set) var needsConsent: CareConnection?
    /// Bumped whenever a Host Setup screen's view changed.
    private(set) var pageRevisions: [UInt64: Int] = [:]
    /// Bumped whenever the phone's push side changed.
    private(set) var pushRevision = 0

    @ObservationIgnored let core: HostCareCore
    /// Where the commands go: the app's sessions, by connection.
    @ObservationIgnored weak var hub: SessionHub?
    /// The saved connections care may act for.
    @ObservationIgnored var connectionStore: ConnectionStore? {
        didSet { syncSaved(); observeSaved() }
    }
    @ObservationIgnored var settings: AppSettings = .shared

    private init() {
        let platform = HostCareOnIOS()
        core = HostCareCore(
            platform: platform,
            pairings: AppGroupPairings(),
            environment: PushRegistration.apnsEnvironment,
            // A harness's seeded connections point at REAL hosts (a scratch
            // tmux, but the login's real $HOME): without this, a probe would
            // pair its throwaway connection onto the developer's ~/.moshpit.
            autocareOff: ProcessInfo.processInfo.arguments.contains("-MOSHPIT_AUTOCARE_OFF"))
        core.restoreDeviceToken(token: PushRegistration.storedToken)
        syncInstallId()
        syncSettings()
    }

    /// This install's id (B443), what its pairing's conf on a host is named
    /// by: read from the keychain, made the first time. Told to the core
    /// once; a launch before the first unlock can't read it and tells it at
    /// the next chance (the app calls this as its stores reload).
    func syncInstallId() {
        guard !installIdTold, let id = InstallIdentity.current() else { return }
        core.installId(id: id)
        installIdTold = true
    }

    @ObservationIgnored private var installIdTold = false

    /// The language the core's texts are said in: the one the app shows.
    static var locale: String { Bundle.main.preferredLocalizations.first ?? "en" }

    // MARK: What the app tells it

    func syncSettings() {
        core.setSettings(notifications: settings.notificationsEnabled,
                         liveActivity: settings.liveActivityEnabled,
                         relayUrl: settings.pushRelayURL,
                         declined: settings.hostSetupDeclined.sorted(),
                         unpaired: settings.hostSetupUnpaired.sorted())
    }

    func syncSaved() {
        Self.tell(core, saved: connectionStore)
    }

    /// The notifications switch turned (B700): each host the phone is on
    /// now is walked again, its pairing rewritten to say whether to push
    /// (the sender there reads it). The others hear at their next session.
    func notificationsChanged() {
        syncSettings()
        guard let hub else { return }
        for session in hub.sessions.values where session.viewModel.connState == .live {
            core.tend(connection: Self.conn(session.connection))
        }
    }

    /// Tell `core` which connections `store` holds: the ones it decoded,
    /// and every one it holds (nil while that can't be told).
    static func tell(_ core: HostCareCore, saved store: ConnectionStore?) {
        guard let store else {
            core.setSaved(listed: [], all: nil)
            return
        }
        core.setSaved(listed: store.connections.map(\.id.uuidString),
                      all: store.savedIds.map { $0.map(\.uuidString) })
        // A rename reaches the pairing's name, and with it pushed cards, at
        // once (B451).
        for connection in store.connections {
            core.nameConnection(connection: connection.id.uuidString, label: connection.displayName)
        }
    }

    /// Follow the store: a connection gone from it gets nothing more done.
    private func observeSaved() {
        guard let connectionStore else { return }
        withObservationTracking {
            _ = connectionStore.connections
        } onChange: {
            Task { @MainActor [weak self] in
                self?.syncSaved()
                self?.observeSaved()
            }
        }
    }

    private func sync() {
        syncSettings()
        syncSaved()
    }

    // MARK: Care

    /// A session's control plane for `connection` came up: walk its host
    /// once this app run, silently putting right what drifted; ask (once)
    /// before ever touching a host that has no hooks.
    func tend(_ connection: ServerConnection) {
        sync()
        core.tend(connection: Self.conn(connection))
    }

    func isTending(_ connectionId: UUID) -> Bool {
        core.isTending(connection: connectionId.uuidString)
    }

    /// The person said yes: Claude Code's hooks, then the pairing.
    func grantConsent(_ connection: ServerConnection) {
        sync()
        core.grantConsent(connection: Self.conn(connection))
    }

    /// The person said no: never ask for this connection again.
    func decline(_ connection: ServerConnection) {
        core.decline(connection: connection.id.uuidString)
    }

    /// "Not now": asked again next app run.
    func dismissConsent() {
        core.dismissConsent()
    }

    static func conn(_ connection: ServerConnection) -> CareConnection {
        CareConnection(id: connection.id.uuidString, label: connection.displayName)
    }

    // MARK: The phone's push side

    /// At launch: ask iOS for a device token when something is paired.
    func registerForPush() {
        core.registerForPush()
    }

    /// The app came to the foreground: re-mint only what needs it.
    func foreground() {
        core.foreground()
    }

    func selfTestArrived(nonce: String) {
        core.selfTestArrived(nonce: nonce)
    }

    /// Forget pairings whose connection is gone, once a launch, the first
    /// time the saved connections can be told.
    func forgetOrphanedPairings() {
        syncSaved()
        let forgotten = core.forgetOrphanedPairings()
        if forgotten > 0 {
            Log.push.notice("forgot \(forgotten, privacy: .public) pairing(s) whose connection is no longer saved")
        }
    }

    /// Why this phone has no usable relay credential, if it has none.
    var relayError: String? {
        _ = pushRevision
        return core.relayError(locale: Self.locale)
    }

    // MARK: Host Setup

    func openSetup(_ connection: ServerConnection, testPane: String?) -> UInt64 {
        sync()
        return core.openSetup(connection: Self.conn(connection), testPane: testPane)
    }

    func closeSetup(_ page: UInt64) {
        core.closeSetup(page: page)
        pageRevisions[page] = nil
    }

    /// The page's message, `{"op": "host-setup", …}`.
    func setupMessage(_ page: UInt64) -> String? {
        _ = pageRevisions[page]
        _ = pushRevision
        return core.setupMessage(page: page, locale: Self.locale)
    }

    /// What the page told the host.
    func setupEvent(_ page: UInt64, _ ev: String, agent: String? = nil) {
        sync()
        core.setupEvent(page: page, ev: ev, agent: agent)
    }

    // MARK: From the core

    fileprivate func apply(_ change: HostCareChange) {
        switch change {
        case .consent:
            needsConsent = core.consent()
        case .page(let page):
            pageRevisions[page, default: 0] += 1
        case .push:
            pushRevision += 1
        }
    }

    fileprivate func rememberDeclined(_ connection: String) {
        settings.hostSetupDeclined.insert(connection)
    }

    fileprivate func rememberUnpaired(_ connection: String, _ unpaired: Bool) {
        if unpaired {
            settings.hostSetupUnpaired.insert(connection)
        } else {
            settings.hostSetupUnpaired.remove(connection)
        }
    }

    /// Run `command` on `connectionId`'s host, on its live session.
    fileprivate func run(_ command: String, on connectionId: UUID) async throws -> Data {
        guard let session = hub?.sessions[connectionId] else { throw SSHError.sessionClosed }
        return try await session.acquireFileTransferSSH().executeCommand(command)
    }
}

// MARK: - Deleting a connection

/// What a delete tells care (``ConnectionDeletion``): a protocol so the
/// delete's order is testable without the app's one machine.
@MainActor
protocol ConnectionCareHandshake {
    /// Care stands down for `id` and its pairing is forgotten on this
    /// phone; throws ``PushPairingForgetFailure`` when the phone couldn't
    /// let go — then nothing else may be removed.
    func deletionBegan(_ id: UUID) throws
    /// The deleted connection's pairing has come off its host, while its
    /// session is still up to ask (at most 3 s; best effort, B377).
    func forgetWindDown(_ id: UUID) async
    /// The care pass the delete stood down has ended (at most 5 s).
    func careWindDown(_ id: UUID) async
    func deletionEnded(_ id: UUID)
}

/// The phone couldn't remove a pairing from its pairings file.
struct PushPairingForgetFailure: LocalizedError {
    var errorDescription: String? {
        String(localized: "Couldn't remove the pairing from this phone, so the host was left paired. The diagnostics log has the reason.")
    }
}

/// The handshake over a core: what ``HostCare`` tells its own, and what a
/// test tells one of its own.
@MainActor
struct CoreCareHandshake: ConnectionCareHandshake {
    let core: HostCareCore
    /// Run before each step that reads the saved connections.
    var beforeEach: () -> Void = {}

    func deletionBegan(_ id: UUID) throws {
        beforeEach()
        guard core.deletionBegan(connection: id.uuidString) else { throw PushPairingForgetFailure() }
    }

    func forgetWindDown(_ id: UUID) async {
        await core.forgetWindDown(connection: id.uuidString)
    }

    func careWindDown(_ id: UUID) async {
        await core.careWindDown(connection: id.uuidString)
    }

    func deletionEnded(_ id: UUID) {
        beforeEach()
        core.deletionEnded(connection: id.uuidString)
    }

    /// Connection `id` was saved pointing at another host or port (B328):
    /// its pairing is the box it pointed at before, and is forgotten on
    /// this phone, as a delete does; care pairs the new host on its next
    /// pass. Throws when the phone couldn't let go.
    func hostChanged(_ id: UUID) throws {
        beforeEach()
        guard core.hostChanged(connection: id.uuidString) else { throw PushPairingForgetFailure() }
    }
}

extension HostCare: ConnectionCareHandshake {
    private var handshake: CoreCareHandshake {
        CoreCareHandshake(core: core, beforeEach: { [weak self] in self?.syncSaved() })
    }

    func deletionBegan(_ id: UUID) throws { try handshake.deletionBegan(id) }
    func forgetWindDown(_ id: UUID) async { await handshake.forgetWindDown(id) }
    func careWindDown(_ id: UUID) async { await handshake.careWindDown(id) }
    func deletionEnded(_ id: UUID) { handshake.deletionEnded(id) }
    func hostChanged(_ id: UUID) throws { try handshake.hostChanged(id) }
}

// MARK: - The platform

/// What the machine asks of iOS, on core threads.
private final class HostCareOnIOS: HostCarePlatform, @unchecked Sendable {
    func exec(connection: String, command: String) async throws -> Data {
        guard let id = UUID(uuidString: connection) else {
            throw HostCareFailure.Failed(why: "no such connection")
        }
        do {
            return try await HostCare.shared.run(command, on: id)
        } catch {
            throw HostCareFailure.Failed(why: Self.describe(error))
        }
    }

    func post(url: String, body: String, timeoutSeconds: UInt32) async throws -> HttpReplyValue {
        guard let url = URL(string: url) else {
            throw HostCareFailure.Failed(why: String(localized: "That relay address isn't a valid URL."))
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.timeoutInterval = TimeInterval(timeoutSeconds)
        request.httpBody = Data(body.utf8)
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            return HttpReplyValue(status: UInt16(clamping: status), body: data)
        } catch {
            throw HostCareFailure.Failed(why: error.localizedDescription)
        }
    }

    func registerForPush() {
        Task { @MainActor in PushRegistration.register() }
    }

    func rememberDeclined(connection: String) {
        Task { @MainActor in HostCare.shared.rememberDeclined(connection) }
    }

    func rememberUnpaired(connection: String, unpaired: Bool) {
        Task { @MainActor in HostCare.shared.rememberUnpaired(connection, unpaired) }
    }

    func changed(change: HostCareChange) {
        Task { @MainActor in HostCare.shared.apply(change) }
    }

    private static func describe(_ error: Error) -> String {
        (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
    }
}

/// The App Group's pairings file, as the core reaches it: through
/// ``PushPairingStore`` (its rules, its protection, the Rust store when its
/// switch is on).
private final class AppGroupPairings: PairingsFile, @unchecked Sendable {
    func read() -> [PairingRecord] {
        PushPairingStore.read().map(PairingRecord.init)
    }

    func upsert(pairing: PairingRecord) -> Bool {
        guard let pairing = PushPairing(pairing) else { return false }
        return PushPairingStore.upsert(pairing)
    }

    func remove(connectionId: String) -> Bool {
        guard let id = UUID(uuidString: connectionId) else { return true }
        return PushPairingStore.remove(connectionId: id)
    }
}


/// This install's id (B443): 16 random lowercase hex digits in the
/// keychain, `AfterFirstUnlockThisDeviceOnly` — never in a backup, so a phone
/// restored from another's backup makes its own, and the two keep apart
/// confs on a host instead of writing over each other's.
enum InstallIdentity {
    private static let service = "com.cluas.moshpit.install"
    private static let account = "install-id"

    /// The id, made if there is none yet; nil while the keychain can't be
    /// read (before the first unlock) — never a new one then, which would
    /// part this install from the conf it already has on its hosts.
    static func current() -> String? {
        var query = base
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        switch SecItemCopyMatching(query as CFDictionary, &item) {
        case errSecSuccess:
            guard let data = item as? Data, let id = String(data: data, encoding: .utf8),
                  isWellFormed(id) else { return nil }
            return id
        case errSecItemNotFound:
            let id = mint()
            var add = base
            add[kSecValueData as String] = Data(id.utf8)
            add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(add as CFDictionary, nil) == errSecSuccess else { return nil }
            return id
        default:
            return nil
        }
    }

    private static var base: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: account,
         kSecAttrSynchronizable as String: false]
    }

    /// 16 lowercase hex digits from the system's CSPRNG.
    static func mint() -> String {
        var bytes = [UInt8](repeating: 0, count: 8)
        _ = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        return bytes.map { String(format: "%02x", $0) }.joined()
    }

    /// 8 to 32 lowercase hex digits: what the core takes.
    static func isWellFormed(_ id: String) -> Bool {
        (8...32).contains(id.count) && id.allSatisfy { ("0"..."9").contains($0) || ("a"..."f").contains($0) }
    }
}
