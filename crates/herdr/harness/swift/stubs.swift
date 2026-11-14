// Harness-only stand-ins for the app types the herdr sources name but the
// harness cannot link: Multiplexer (ServerConnection.swift), SSHSession
// (Citadel) and HostCapabilities. Never part of the app. extraPathDirs is
// copied verbatim from Moshpit/Services/HostCapabilities.swift
// (build-swift.sh checks it still matches).

import Foundation

enum Multiplexer: String {
    case none, tmux, herdr
}

actor SSHSession {
    func executeCommand(_ command: String) async throws -> Data {
        fatalError("the harness never runs commands over SSH")
    }
}

enum HostCapabilities {
    static let extraPathDirs =
        "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin"
}
