// Harness-only stand-ins for the two app types MoshBootstrap.swift names
// that a command-line build cannot link: SSHSession (Citadel) and
// HostCapabilities. Never part of the app. extraPathDirs is copied verbatim
// from Moshpit/Services/HostCapabilities.swift (build-swift.sh checks that
// it still matches).

import Foundation

actor SSHSession {
    func executeCommand(_ command: String) async throws -> Data {
        fatalError("the harness never runs commands over SSH")
    }
}

enum HostCapabilities {
    static let extraPathDirs =
        "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin"
}
