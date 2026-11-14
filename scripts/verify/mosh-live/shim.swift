// Stands in for the app's SSHSession in mosh-live: the one call
// MoshBootstrap.start makes, `executeCommand`, run by the system's ssh
// against the private sshd run.sh started — with run.sh's own key and
// known_hosts, never ~/.ssh. stdout and stderr come back merged, as the
// app's exec channel merges them (mosh-server prints its pid on stderr).

import Foundation

actor SSHSession {
    let host: String
    let port: String
    let user: String
    let key: String
    let knownHosts: String

    init(host: String, port: String, user: String, key: String, knownHosts: String) {
        self.host = host
        self.port = port
        self.user = user
        self.key = key
        self.knownHosts = knownHosts
    }

    func executeCommand(_ command: String) async throws -> Data {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
        process.arguments = [
            "-F", "/dev/null", "-i", key,
            "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none",
            "-o", "UserKnownHostsFile=\(knownHosts)", "-o", "GlobalKnownHostsFile=/dev/null",
            "-o", "StrictHostKeyChecking=accept-new", "-o", "BatchMode=yes",
            "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5",
            "-p", port, "\(user)@\(host)", "--", command,
        ]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        process.standardInput = FileHandle.nullDevice
        try process.run()
        let output = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return output
    }
}
