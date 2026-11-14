// Harness-only stand-ins for the app types the install sources name that a
// command-line build cannot link: SSHSession (Citadel), HostConnection (the
// connection protocol HostCapabilities probes over, whose file pulls in the
// SFTP types) and Multiplexer (Models/ServerConnection.swift, whose labels
// need the app's string catalog). Never part of the app. build-swift.sh
// checks that the Multiplexer cases are still the app's.

import Foundation

actor SSHSession {
    func executeCommand(_ command: String) async throws -> Data {
        fatalError("the harness never runs commands over SSH")
    }
}

/// The one member of the app's `HostConnection` that the harnessed sources
/// call.
protocol HostConnection: Sendable {
    func executeCommand(_ command: String) async throws -> Data
}

enum Multiplexer: String, Codable, CaseIterable, Sendable {
    case none
    case tmux
    case herdr
}
