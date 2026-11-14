#if DEBUG
import Foundation
import Observation
import SwiftUI
import MoshpitKit

/// A DEBUG-only way to put Set up this host on screen in each of its states,
/// so they can be screenshotted and reviewed, and its NSH page tested.
///
/// The real screen needs a live SSH session; this runs the core's machine
/// (``HostCareCore``) of its own over a canned host whose answers are the
/// core's too (`host_setup_demo_reply`), so every state — an out-of-date
/// install, a relay that does not know this phone — is one launch argument
/// away. Compiled out of Release entirely.
enum HostSetupDemo {

    /// The one connection id every demo scenario shares: the manifest's
    /// pairing names it.
    static let demoConn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"

    static let flag = "-MOSHPIT_HOSTSETUP_DEMO"

    /// Which state to render. Passed as the value after the flag.
    enum Scenario: String, CaseIterable {
        /// Nothing installed on a healthy host.
        case fresh
        /// Hooks and pairing both current.
        case installed
        /// Installed, but from an older build.
        case stale
        /// Paired, with a stamp script too old to fire.
        case nothingWillPush
        /// The host is fine; this PHONE is not registered with the relay.
        case relayUnreachable
        /// A host missing the tools the sender needs.
        case missingTools
    }

    static func scenario(from arguments: [String]) -> Scenario? {
        guard let i = arguments.firstIndex(of: flag) else { return nil }
        guard i + 1 < arguments.count, let s = Scenario(rawValue: arguments[i + 1]) else {
            return .fresh
        }
        return s
    }

    /// A capture run's scenario (`-MOSHPIT_SEED_HOSTSETUP <scenario>` with
    /// `-MOSHPIT_SEED_SCREEN host-setup[-nsh]`): fresh unless it names one.
    static var seededScenario: Scenario {
        let arguments = ProcessInfo.processInfo.arguments
        guard let i = arguments.firstIndex(of: "-MOSHPIT_SEED_HOSTSETUP"), i + 1 < arguments.count,
              let scenario = Scenario(rawValue: arguments[i + 1])
        else { return .fresh }
        return scenario
    }

    static let connection = ServerConnection(
        id: UUID(uuidString: demoConn)!,
        name: "m1-pro", host: "10.0.0.2", port: 22, username: "dev",
        authMethod: .key, connectionProtocol: .ssh, sshPort: 22, keychainRef: nil)

    @MainActor
    static func view(_ scenario: Scenario) -> some View {
        HostSetupPage(connection: connection, testPane: "%3", source: DemoHostCare(scenario))
    }
}

/// The machine of one demo scenario, over its canned host.
@MainActor
@Observable
final class DemoHostCare: HostSetupSource {
    let scenario: HostSetupDemo.Scenario
    private(set) var revisions: [UInt64: Int] = [:]
    @ObservationIgnored private(set) var core: HostCareCore!

    init(_ scenario: HostSetupDemo.Scenario) {
        self.scenario = scenario
        let platform = DemoPlatform(scenario: scenario)
        // A paired scenario's phone holds the pairing its host was given.
        let paired = scenario == .installed || scenario == .nothingWillPush
        let pairings = DemoPairings(paired ? [hostSetupDemoPairing(relay: AppSettings.defaultPushRelay)] : [])
        core = HostCareCore(platform: platform, pairings: pairings, environment: "sandbox",
                            autocareOff: true)
        platform.owner = self
        core.setSettings(notifications: true, liveActivity: true,
                         relayUrl: AppSettings.defaultPushRelay, declined: [], unpaired: [])
        if scenario == .relayUnreachable {
            core.registrationFailed(why: "Relay rejected the registration (401): unknown or expired send token")
        }
    }

    func openSetup(_ connection: ServerConnection, testPane: String?) -> UInt64 {
        // The stale states look like a hook test that couldn't run.
        let stale = scenario == .stale || scenario == .nothingWillPush
        let page = core.openSetup(connection: HostCare.conn(connection), testPane: stale ? nil : testPane)
        if stale { core.setupEvent(page: page, ev: "prove-hooks", agent: nil) }
        return page
    }

    func closeSetup(_ page: UInt64) { core.closeSetup(page: page) }

    func setupMessage(_ page: UInt64) -> String? {
        _ = revisions[page]
        return core.setupMessage(page: page, locale: HostCare.locale)
    }

    func setupEvent(_ page: UInt64, _ ev: String, agent: String?) {
        core.setupEvent(page: page, ev: ev, agent: agent)
    }

    fileprivate func changed(_ change: HostCareChange) {
        if case .page(let page) = change { revisions[page, default: 0] += 1 }
    }

    /// Whether `page` has read its host (for a test to wait on).
    func isIdle(_ page: UInt64) -> Bool {
        core.setupMessage(page: page, locale: "en")?.contains("\"phase\":\"idle\"") == true
    }
}

private final class DemoPlatform: HostCarePlatform, @unchecked Sendable {
    let scenario: HostSetupDemo.Scenario
    weak var owner: DemoHostCare?

    init(scenario: HostSetupDemo.Scenario) { self.scenario = scenario }

    func exec(connection: String, command: String) async throws -> Data {
        hostSetupDemoReply(scenario: scenario.rawValue, command: command, relay: AppSettings.defaultPushRelay)
    }

    func post(url: String, body: String, timeoutSeconds: UInt32) async throws -> HttpReplyValue {
        throw HostCareFailure.Failed(why: "the demo has no relay")
    }

    func registerForPush() {}
    func rememberDeclined(connection: String) {}
    func rememberUnpaired(connection: String, unpaired: Bool) {}

    func changed(change: HostCareChange) {
        Task { @MainActor [weak owner] in owner?.changed(change) }
    }
}

private final class DemoPairings: PairingsFile, @unchecked Sendable {
    private let lock = NSLock()
    private var pairings: [PairingRecord]

    init(_ pairings: [PairingRecord]) { self.pairings = pairings }

    func read() -> [PairingRecord] { lock.withLock { pairings } }

    func upsert(pairing: PairingRecord) -> Bool {
        lock.withLock {
            pairings.removeAll { $0.connectionId == pairing.connectionId }
            pairings.append(pairing)
        }
        return true
    }

    func remove(connectionId: String) -> Bool {
        lock.withLock { pairings.removeAll { $0.connectionId == connectionId } }
        return true
    }
}
#endif
