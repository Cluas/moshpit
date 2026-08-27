#if DEBUG
import Foundation
import SwiftUI

/// A DEBUG-only way to put `HostSetupView` on screen in each of its states, so
/// they can be screenshotted and reviewed.
///
/// It exists because the real screen is gated on a live SSH session — Settings
/// disables its row when there is none, and the sheet's content is empty — so a
/// UI test in a simulator can never reach it. That is not an idb quirk; it is the
/// screen's own precondition.
///
/// Driving it with a canned channel is also strictly BETTER than reaching a real
/// host would be. A real host is in exactly one state at a time, and the two
/// states most in need of a look — an out-of-date install, and a relay that does
/// not know this phone — are the two hardest to arrange on purpose.
///
/// Compiled out of Release entirely: everything here, including the launch-flag
/// read, sits inside `#if DEBUG`, the same as `-MOSHPIT_RESET`.
enum HostSetupDemo {

    /// The one connection id every demo scenario shares — the manifest's pairing
    /// key must name the same id or "paired" scenarios render as unpaired.
    static let demoConn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"


    static let flag = "-MOSHPIT_HOSTSETUP_DEMO"

    /// Which state to render. Passed as the value after the flag.
    enum Scenario: String, CaseIterable {
        /// Nothing installed on a healthy host.
        case fresh
        /// Hooks and pairing both current.
        case installed
        /// Installed, but from an older build — the case with no mechanism at all
        /// before the manifest existed.
        case stale
        /// Paired, with a stamp script too old to fire: secrets on the host and
        /// nothing that will ever send.
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

    @MainActor
    static func view(_ scenario: Scenario) -> some View {
        HostSetupView(model: model(scenario), testPane: "%3")
    }

    @MainActor
    static func model(_ scenario: Scenario) -> HostSetupModel {
        let push = DemoPush()
        if scenario == .relayUnreachable {
            push.lastRelayError = "Relay rejected the registration (401): unknown or expired send token"
        }
        let connection = ServerConnection(
            id: UUID(uuidString: HostSetupDemo.demoConn)!,
            name: "m1-pro", host: "10.0.0.2", port: 22, username: "cluas",
            authMethod: .key, connectionProtocol: .ssh, sshPort: 22, keychainRef: nil)
        let model = HostSetupModel(connection: connection, push: push) {
            HostInstaller(channel: DemoChannel(scenario: scenario))
        }
        if scenario == .stale || scenario == .nothingWillPush {
            // A failed proof is part of what these states look like, and there is
            // no host here to fail against.
            Task { @MainActor in await model.proveHooks(pane: nil) }
        }
        return model
    }

    /// Answers the installer's commands from a fixture.
    private struct DemoChannel: HostChannel {
        let scenario: Scenario

        func run(_ command: String) async throws -> Data {
            Data(reply(to: command).utf8)
        }

        private func reply(to command: String) -> String {
            if command.contains("printf 'home=") {
                let tools = scenario == .missingTools
                    ? "openssl=no\ncurl=no\njq=no\ntmux=yes\nbase64=yes"
                    : "openssl=yes\ncurl=yes\njq=yes\ntmux=yes\nbase64=yes"
                return "home=/Users/cluas\nshell=/bin/zsh\nuname=Darwin\n" + tools
            }
            if command.contains("manifest.json") { return manifest }
            return ""
        }

        private var manifest: String {
            switch scenario {
            case .fresh, .missingTools, .relayUnreachable:
                return ""
            case .installed:
                return json(stampDigest: ContentDigest.of(HostScripts.stamp), paired: true)
            case .stale:
                return json(stampDigest: "an-older-build", paired: false)
            case .nothingWillPush:
                return json(stampDigest: "an-older-build", paired: true)
            }
        }

        private func json(stampDigest: String, paired: Bool) -> String {
            var components: [String: InstallManifest.Component] = [
                InstallComponent.stamp.key: .init(digest: stampDigest, installedAt: Date()),
                InstallComponent.sender.key: .init(
                    digest: ContentDigest.of(HostScripts.sender), installedAt: Date()),
                InstallComponent.hooks(agent: "claude").key: .init(
                    digest: "cfg", installedAt: Date(),
                    configPath: "$HOME/.claude/settings.json"),
            ]
            if paired {
                components[InstallComponent.pairing(conn: HostSetupDemo.demoConn).key] = .init(
                    installedAt: Date(),
                    connectionId: HostSetupDemo.demoConn,
                    relayURL: "https://push.offhook.cluas.eu.org")
            }
            let manifest = InstallManifest(schema: InstallManifest.currentSchema,
                                           components: components)
            guard let data = try? JSONEncoder.moshpitInstall.encode(manifest) else { return "" }
            return String(decoding: data, as: UTF8.self)
        }
    }

    @MainActor
    private final class DemoPush: PushCoordinating {
        var lastRelayError: String?
        func pair(connectionId: UUID, hostLabel: String, relayURL: String) async throws -> PushPairing {
            PushPairing.make(connectionId: connectionId, hostLabel: hostLabel, relayURL: relayURL)
        }
        func ensureReady(connectionId: UUID) async -> PushPairing? { nil }
        func unpair(connectionId: UUID) {}
        func awaitSelfTest(nonce: String, timeout: Duration) async -> Bool { false }
        func forgetSelfTest(nonce: String) {}
    }
}
#endif
