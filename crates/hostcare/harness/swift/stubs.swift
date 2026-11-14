// Harness-only stand-ins for the app types the app's Swift host care
// (`HostAutoCare`, `HostSetupModel`, `PushService` at 81c121a1) names that a
// command-line build cannot link. Never part of the app.

import Foundation
import MoshpitKit

struct ServerConnection: Identifiable, Sendable {
    let id: UUID
    let name: String
    var displayName: String { name }
}

@MainActor
final class ConnectionStore {
    var connections: [ServerConnection] = []
}

@MainActor
@Observable
final class AppSettings {
    static let shared = AppSettings()
    static let defaultPushRelay = "https://push.example.org"
    var notificationsEnabled = true
    var liveActivityEnabled = true
    var pushRelayURL = AppSettings.defaultPushRelay
    var hostSetupDeclined: Set<String> = []
}

/// The one thing care and the screen ask of a session: an installer on its
/// host.
@MainActor
final class SessionHub {
    @MainActor
    final class ActiveSession {
        let connection: ServerConnection
        let channel: any HostChannel
        init(connection: ServerConnection, channel: any HostChannel) {
            self.connection = connection
            self.channel = channel
        }
        func hostInstaller() async throws -> HostInstaller {
            HostInstaller(channel: channel)
        }
    }
}

@MainActor
enum ConnectionDeletion {
    static let careWindDown: Duration = .seconds(5)
}

actor SSHSession {
    func executeCommand(_ command: String) async throws -> Data {
        fatalError("the harness never runs commands over SSH")
    }
}

protocol HostConnection: Sendable {
    func executeCommand(_ command: String) async throws -> Data
}

enum Multiplexer: String, Codable, CaseIterable, Sendable {
    case none
    case tmux
    case herdr
}
