// mosh-live: the app's mosh code against a real mosh-server, started through
// a private sshd (scripts/verify/mosh-live/run.sh starts one). The transport,
// the wire, the crypto, the bootstrap's command line and its parser — and,
// from round twenty-two on, the rules for when the app stops the server
// (MoshServerReap.swift) — are the app's files, unchanged (build.sh). Built
// from a tree before those rules existed, the rules are the ones SessionHub
// had then (old-rules.swift). One command per run:
//
//   mosh-live reach <ssh host> <port> <user> <key> <known_hosts> <udp target>
//       bootstrap mosh-server over SSH to <ssh host>, then mosh's UDP to
//       <udp target>: `name` (the host name, what the app used before B171)
//       or `peer` (the address the SSH connection reached, $SSH_CONNECTION's
//       server address). Prints `reached` when the shell echoes, `black` when
//       nothing comes back, and the datagrams received.
//   mosh-live blackhole <ssh host> <port> <user> <key> <known_hosts>
//       a session through a UDP relay that, once the shell has answered,
//       drops everything both ways until the transport says the return path
//       is dead; then what the app's rule does with the server (B165), then
//       the path heals: does the same shell answer?
//   mosh-live disconnect <ssh host> <port> <user> <key> <known_hosts>
//       a session that has talked to its server is closed by the user (B164):
//       what the app's rule does with the server, and whether the server and
//       its shell are still running afterwards.
//
// Every mosh-server started prints `PID <pid>` so run.sh can stop it.

import Foundation
import Network

setvbuf(stdout, nil, _IOLBF, 0)

// MARK: - Output

func say(_ line: String) { print(line) }

func fail(_ line: String) -> Never {
    print("error \(line)")
    exit(1)
}

func alive(_ pid: Int32) -> Bool { kill(pid, 0) == 0 || errno == EPERM }

// MARK: - The session under test

/// A mosh session: the app's bootstrap and transport, and what the terminal
/// shows of it.
final class Session: @unchecked Sendable {
    let ssh: SSHSession
    let credentials: MoshCredentials
    let transport: MoshTransport
    private let lock = NSLock()
    private var screen = ""
    private var returnPathDead: (at: Date, heard: Bool?)?

    init(ssh: SSHSession, credentials: MoshCredentials, transport: MoshTransport) {
        self.ssh = ssh
        self.credentials = credentials
        self.transport = transport
    }

    /// Bootstraps mosh-server over `ssh` with the app's command line and
    /// parser; datagrams go to `udpHost` (`relay` swaps in a relay's port).
    static func start(ssh: SSHSession, udpHost: String,
                      relay: ((UInt16) -> UInt16)? = nil) async throws -> Session {
        let found = try await MoshBootstrap.start(over: ssh, host: udpHost)
        guard let pid = found.serverPid else { fail("mosh-server's pid wasn't in its output") }
        say("PID \(pid)")
        var credentials = found
        if let relay {
            credentials = MoshCredentials(host: udpHost, udpPort: relay(found.udpPort),
                                          key: found.key, serverPid: found.serverPid)
        }
        let transport = try MoshTransport(credentials: credentials)
        let session = Session(ssh: ssh, credentials: found, transport: transport)
        await MainActor.run {
            transport.onReturnPathDead = returnPathCallback { heard in
                session.noteReturnPathDead(heard: heard)
            }
        }
        Task {
            for await frame in transport.hostStream {
                session.feed(frame.bytes)
            }
        }
        await transport.start(cols: 80, rows: 24)
        return session
    }

    var pid: Int32 { Int32(credentials.serverPid ?? 0) }

    func feed(_ bytes: Data) {
        let now = Date()
        lock.withLock {
            screen += String(decoding: bytes, as: UTF8.self)
            if !bytes.isEmpty { paints.append((now, String(decoding: bytes, as: UTF8.self))) }
        }
    }

    /// Every frame with screen bytes, when it came (herdr-retarget).
    private var paints: [(at: Date, text: String)] = []
    var paintCount: Int { lock.withLock { paints.count } }

    /// The frames from `index` on, with when each came.
    func frames(after index: Int) -> [(at: Date, text: String)] {
        lock.withLock { Array(paints.dropFirst(index)) }
    }

    /// The first frame from `index` on that came after `landedAt` (any
    /// frame, without it) and holds `text` (any bytes, without it).
    func firstPaint(containing text: String? = nil, after index: Int, landedAt: Date? = nil) -> Date? {
        lock.withLock {
            paints.dropFirst(index).first { paint in
                (landedAt.map { paint.at >= $0 } ?? true) && (text.map { paint.text.contains($0) } ?? true)
            }?.at
        }
    }

    func waitForPaint(containing text: String?, after index: Int, landedAt: Date? = nil,
                      seconds: Double) async -> Date? {
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            if let at = firstPaint(containing: text, after: index, landedAt: landedAt) { return at }
            try? await Task.sleep(for: .milliseconds(2))
        } while Date() < deadline
        return nil
    }

    var text: String { lock.withLock { screen } }

    func noteReturnPathDead(heard: Bool?) {
        lock.withLock {
            if returnPathDead == nil { returnPathDead = (Date(), heard) }
        }
    }

    var returnPathReport: (at: Date, heard: Bool?)? { lock.withLock { returnPathDead } }

    func type(_ keys: String) async {
        await transport.send(Data(keys.utf8))
    }

    /// Waits up to `seconds` for `pattern` on the screen; the first match.
    func wait(for pattern: String, seconds: Double) async -> String? {
        let regex = try! NSRegularExpression(pattern: pattern)
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            let screen = text
            if let match = regex.firstMatch(in: screen, range: NSRange(screen.startIndex..., in: screen)),
               let range = Range(match.range, in: screen) {
                return String(screen[range])
            }
            try? await Task.sleep(for: .milliseconds(50))
        } while Date() < deadline
        return nil
    }

    /// Types `keys` once the shell has said something (a login shell that is
    /// still starting can eat what is typed into it), and waits up to
    /// `seconds` for `pattern`; types once more if nothing came.
    func ask(_ keys: String, expecting pattern: String, seconds: Double) async -> String? {
        for _ in 0..<100 where text.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        try? await Task.sleep(for: .milliseconds(500))
        for _ in 0..<2 {
            await type(keys)
            if let found = await wait(for: pattern, seconds: seconds / 2) { return found }
        }
        return nil
    }

    /// What the app runs to stop a server it started (SessionHub, both the
    /// return-path reaper and stop()'s): `kill <pid>` over its own connection.
    func stopServer() async {
        _ = try? await ssh.executeCommand("kill \(pid) 2>/dev/null")
    }
}

// MARK: - A UDP relay that can drop everything

/// Between the transport and mosh-server: datagrams both ways, or none while
/// `blackhole` is on (a path that died without a word).
final class UDPRelay: @unchecked Sendable {
    private let front: Int32
    private let back: Int32
    private let lock = NSLock()
    private var dropping = false
    private var client: sockaddr_in?
    let port: UInt16

    init(server port: UInt16) {
        let front = socket(AF_INET, SOCK_DGRAM, 0)
        let back = socket(AF_INET, SOCK_DGRAM, 0)
        var local = sockaddr_in()
        local.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        local.sin_family = sa_family_t(AF_INET)
        local.sin_addr.s_addr = inet_addr("127.0.0.1")
        local.sin_port = 0
        withUnsafePointer(to: &local) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                _ = bind(front, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        var bound = sockaddr_in()
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        withUnsafeMutablePointer(to: &bound) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { _ = getsockname(front, $0, &length) }
        }
        var server = sockaddr_in()
        server.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        server.sin_family = sa_family_t(AF_INET)
        server.sin_addr.s_addr = inet_addr("127.0.0.1")
        server.sin_port = port.bigEndian
        withUnsafePointer(to: &server) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                _ = connect(back, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        self.front = front
        self.back = back
        self.port = UInt16(bigEndian: bound.sin_port)
        Thread.detachNewThread { [self] in pump() }
    }

    var blackhole: Bool {
        get { lock.withLock { dropping } }
        set { lock.withLock { dropping = newValue } }
    }

    private func pump() {
        var buffer = [UInt8](repeating: 0, count: 65_536)
        while true {
            var fds = [pollfd(fd: front, events: Int16(POLLIN), revents: 0),
                       pollfd(fd: back, events: Int16(POLLIN), revents: 0)]
            guard poll(&fds, 2, 1_000) >= 0 else { continue }
            if fds[0].revents & Int16(POLLIN) != 0 {
                var from = sockaddr_in()
                var length = socklen_t(MemoryLayout<sockaddr_in>.size)
                let count = withUnsafeMutablePointer(to: &from) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                        recvfrom(front, &buffer, buffer.count, 0, $0, &length)
                    }
                }
                if count > 0 {
                    lock.withLock { client = from }
                    if !blackhole { _ = send(back, buffer, count, 0) }
                }
            }
            if fds[1].revents & Int16(POLLIN) != 0 {
                let count = recv(back, &buffer, buffer.count, 0)
                if count > 0, !blackhole, var to = lock.withLock({ client }) {
                    withUnsafePointer(to: &to) {
                        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                            _ = sendto(front, buffer, count, 0, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                        }
                    }
                }
            }
        }
    }
}

// MARK: - Commands

let arguments = Array(CommandLine.arguments.dropFirst())
guard arguments.count >= 6 else {
    fail("usage: mosh-live reach|blackhole|disconnect <ssh host> <port> <user> <key> <known_hosts> [udp target]")
}
let command = arguments[0]
let ssh = SSHSession(host: arguments[1], port: arguments[2], user: arguments[3],
                     key: arguments[4], knownHosts: arguments[5])
say("rules: \(appRules)")

/// The address the SSH connection reached: `$SSH_CONNECTION`'s third field.
func peerAddress() async throws -> String {
    let output = String(decoding: try await ssh.executeCommand("echo \"$SSH_CONNECTION\""), as: UTF8.self)
    let fields = output.split(whereSeparator: \.isWhitespace)
    guard fields.count >= 3 else { fail("no SSH_CONNECTION: \(output)") }
    return String(fields[2])
}

switch command {
case "reach":
    guard arguments.count >= 7 else { fail("reach needs a udp target: name or peer") }
    let peer = try await peerAddress()
    let target = arguments[6] == "peer" ? peer : arguments[1]
    say("ssh to \(arguments[1]) reached \(peer); mosh's UDP goes to \(target)")
    let session = try await Session.start(ssh: ssh, udpHost: target)
    let echoed = await session.ask("echo reach-$((6*7))\r", expecting: "reach-42", seconds: 8)
    let received = await session.transport.currentDiagnostics.datagramsReceived
    say(echoed != nil ? "reached (datagrams received: \(received))" : "black (datagrams received: \(received))")
    await session.transport.close()
    await session.stopServer()

case "blackhole":
    var relay: UDPRelay?
    let session = try await Session.start(ssh: ssh, udpHost: "127.0.0.1") { port in
        let made = UDPRelay(server: port)
        relay = made
        return made.port
    }
    guard let relay else { fail("no relay") }
    guard let first = await session.ask("echo shell=$$\r", expecting: "shell=[0-9]+", seconds: 10) else {
        fail("the shell never answered: \(session.text.suffix(200))")
    }
    let shell = Int32(first.dropFirst("shell=".count))!
    say("contact: the shell \(shell) under mosh-server \(session.pid) answered")
    relay.blackhole = true
    let cut = Date()
    say("the path drops everything both ways")
    var waited = 0
    while session.returnPathReport == nil, waited < 600 {
        try? await Task.sleep(for: .milliseconds(100))
        waited += 1
    }
    guard let report = session.returnPathReport else { fail("the return path was never reported dead") }
    let heard = report.heard.map { "\($0)" } ?? "not said"
    say(String(format: "return path reported dead %.1f s into the silence (heard from the server: %@)",
               report.at.timeIntervalSince(cut), heard))
    let reap = shouldStopOnReturnPathDead(heardFromServer: report.heard)
    say(reap ? "the app's rule: stop the server" : "the app's rule: leave the server alone, offer SSH")
    if reap { await session.stopServer() }
    try? await Task.sleep(for: .seconds(2))
    relay.blackhole = false
    say("the path heals")
    let mark = session.text.count
    await session.type("echo back=$$\r")
    var answered: String?
    for _ in 0..<150 {
        let fresh = String(session.text.dropFirst(mark))
        if let range = fresh.range(of: "back=[0-9]+", options: .regularExpression) {
            answered = String(fresh[range])
            break
        }
        try? await Task.sleep(for: .milliseconds(100))
    }
    let serverAlive = alive(session.pid)
    let shellAlive = alive(shell)
    say("mosh-server \(session.pid) \(serverAlive ? "running" : "gone"), shell \(shell) \(shellAlive ? "running" : "gone")")
    if let answered, answered == "back=\(shell)" {
        say("survived: the same shell answers after the path healed")
    } else {
        say("lost: no answer after the path healed\(answered.map { " (\($0))" } ?? "")")
    }
    await session.transport.close()
    await session.stopServer()

case "disconnect":
    let session = try await Session.start(ssh: ssh, udpHost: "127.0.0.1")
    guard let first = await session.ask("echo shell=$$\r", expecting: "shell=[0-9]+", seconds: 10) else {
        fail("the shell never answered: \(session.text.suffix(200))")
    }
    let shell = Int32(first.dropFirst("shell=".count))!
    let contacted = await session.transport.currentDiagnostics.datagramsReceived > 0
    say("contact: the shell \(shell) under mosh-server \(session.pid) answered (contacted: \(contacted))")
    // The user disconnects: the transport closes, then the rule decides.
    await session.transport.close()
    let stop = shouldStopOnUserClose(contacted: contacted)
    say(stop ? "the app's rule: stop the server" : "the app's rule: leave the server running")
    let began = Date()
    if stop { await session.stopServer() }
    var gone: Double?
    for _ in 0..<150 {
        if !alive(session.pid), !alive(shell) {
            gone = Date().timeIntervalSince(began)
            break
        }
        try? await Task.sleep(for: .milliseconds(100))
    }
    if let gone {
        say(String(format: "stopped: mosh-server and its shell gone %.1f s after the disconnect", gone))
    } else {
        say("left running: mosh-server \(session.pid) \(alive(session.pid) ? "running" : "gone"), shell \(shell) \(alive(shell) ? "running" : "gone") 15 s after the disconnect")
        await session.stopServer()
    }

case "herdr-retarget":
    await herdrRetarget(ssh: ssh, Array(arguments.dropFirst(6)))

case "herdr-life":
    await herdrLife(ssh: ssh, Array(arguments.dropFirst(6)))

default:
    fail("unknown command \(command)")
}
exit(0)
