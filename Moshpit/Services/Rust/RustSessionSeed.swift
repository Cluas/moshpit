#if DEBUG
import Foundation
import MoshpitKit
import QuartzCore

/// DEBUG launch arguments for driving a session on a device without a
/// finger on it (W14a's rig: a private sshd on the Mac, the iPad over the
/// LAN or Tailscale). Nothing here writes anything that outlives the launch:
/// the connection isn't stored, the key never enters the keychain, the host
/// key is trusted for this launch only (``LaunchTrust``).
///
/// - `-MOSHPIT_RUST_SEED_HOST h -MOSHPIT_RUST_SEED_PORT p -MOSHPIT_RUST_SEED_USER u
///   -MOSHPIT_RUST_SEED_KEY_B64 <openssh key, base64> -MOSHPIT_RUST_SEED_TRUST_FP SHA256:…`
///   opens a terminal on that connection at launch; `-MOSHPIT_RUST_SEED_MUX
///   tmux`, `-MOSHPIT_RUST_SEED_MOSH 1` (predictive echo off, so it can run
///   on either path), `-MOSHPIT_RUST_SEED_TMUX_BIN`, `-MOSHPIT_RUST_SEED_MOSH_BIN`.
/// - `-MOSHPIT_RUST_FAULTS "cut@15,stall@45,blackhole@20-35,echo@40"`: the
///   core's fault hooks, seconds after the session first connects; what
///   follows each is logged with its timing (`echo@t` times ten keystrokes
///   then, to see how typing fares after a fault).
/// - `-MOSHPIT_ECHO_PROBE 100`: once the terminal on screen is live, types
///   that many keys one at a time and times each from the keystroke to its
///   echo being fed to the terminal, on whichever path the session runs.
///
/// Results go to stderr (`devicectl device process launch --console`) as
/// `[w14] …` lines, and to the log.
@MainActor
enum RustSessionSeed {
    static func say(_ line: String) {
        FileHandle.standardError.write(Data("[w14] \(line)\n".utf8))
        Log.ssh.notice("w14: \(line, privacy: .public)")
    }

    private static func arg(_ flag: String) -> String? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
        return args[i + 1]
    }

    // MARK: The seeded connection

    nonisolated private static let lock = NSLock()
    nonisolated(unsafe) private static var keys: [UUID: String] = [:]

    /// The seeded key of connection `id`, if it is the seeded one.
    nonisolated static func credential(for id: UUID) -> String? {
        lock.lock(); defer { lock.unlock() }
        return keys[id]
    }

    /// What a session remembers about a connection, in the app's
    /// preferences: the seed's are dropped before and after its runs (and
    /// `-MOSHPIT_RUST_SEED_FORGET id,id` drops those of earlier seeds), so a
    /// launch leaves nothing behind.
    static func forget(_ id: UUID) {
        let defaults = UserDefaults.standard
        for key in ["hostcaps.\(id.uuidString)", "moshpit.tmux.lastSelection.\(id.uuidString)"]
            where defaults.object(forKey: key) != nil {
            defaults.removeObject(forKey: key)
            say("forgot \(key)")
        }
    }

    /// The connection the launch asks for, ready for both paths, or nil.
    static func connectionFromLaunchArguments() async -> ServerConnection? {
        for id in (arg("-MOSHPIT_RUST_SEED_FORGET") ?? "").split(separator: ",") {
            if let id = UUID(uuidString: String(id)) { forget(id) }
        }
        guard let host = arg("-MOSHPIT_RUST_SEED_HOST"),
              let user = arg("-MOSHPIT_RUST_SEED_USER"),
              let b64 = arg("-MOSHPIT_RUST_SEED_KEY_B64"),
              let data = Data(base64Encoded: b64),
              let key = String(data: data, encoding: .utf8)
        else { return nil }
        let port = Int(arg("-MOSHPIT_RUST_SEED_PORT") ?? "22") ?? 22
        let id = arg("-MOSHPIT_RUST_SEED_ID").flatMap(UUID.init(uuidString:)) ?? UUID()
        lock.withLock { keys[id] = key }
        forget(id)
        // The Swift path reads its cache before the keychain.
        await SSHService.shared.seedSecretForThisLaunch(connectionId: id, secret: key)
        if let fingerprint = arg("-MOSHPIT_RUST_SEED_TRUST_FP") {
            LaunchTrust.trust(host: host, port: port, fingerprint: fingerprint)
        }
        let mosh = arg("-MOSHPIT_RUST_SEED_MOSH") == "1"
        var connection = ServerConnection(
            id: id, name: "w14-rig", host: host, port: port, username: user,
            authMethod: .key, connectionProtocol: mosh ? .mosh : .ssh, sshPort: port,
            moshServerPath: arg("-MOSHPIT_RUST_SEED_MOSH_BIN"),
            keychainRef: "w14.seed.\(id.uuidString)",
            predictModeRaw: PredictMode.never.rawValue,
            tmuxPath: arg("-MOSHPIT_RUST_SEED_TMUX_BIN"))
        connection.multiplexer = arg("-MOSHPIT_RUST_SEED_MUX").flatMap(Multiplexer.init(rawValue:)) ?? .none
        say("seeded \(user)@\(host):\(port) \(mosh ? "mosh" : "ssh")+\(connection.multiplexer.rawValue)")
        return connection
    }

    // MARK: Faults

    private struct Fault {
        let at: Double
        let kind: String
        let until: Double?
    }

    private static var faults: [Fault] = {
        guard let spec = arg("-MOSHPIT_RUST_FAULTS") else { return [] }
        return spec.split(separator: ",").compactMap { item in
            let parts = item.split(separator: "@")
            guard parts.count == 2 else { return nil }
            let window = parts[1].split(separator: "-").compactMap { Double($0) }
            guard let at = window.first else { return nil }
            return Fault(at: at, kind: String(parts[0]), until: window.count > 1 ? window[1] : nil)
        }
    }()

    private static var scheduled = false
    private static var faultAt: CFTimeInterval?
    private static var faultName = ""

    /// What happened to a seeded session, for the fault timings.
    static func observe(_ event: SessionEventKind, driver: RustSessionDriver) {
        let now = CACurrentMediaTime()
        let since = faultAt.map { Int((now - $0) * 1000) }
        switch event {
        case .state(let state):
            if let since { say("\(faultName): \(String(describing: state)) after \(since) ms") }
            if state == .connected, !scheduled, !faults.isEmpty {
                scheduled = true
                schedule(on: driver)
            }
        case .firstFrame(let attach):
            if let since { say("\(faultName): first frame of attach \(attach) after \(since) ms") }
        case .returnPathDead:
            if let since { say("\(faultName): mosh return path dead after \(since) ms") }
        default:
            break
        }
    }

    private static func schedule(on driver: RustSessionDriver) {
        for fault in faults {
            Task { @MainActor [weak driver] in
                try? await Task.sleep(for: .seconds(fault.at))
                guard let driver, let id = driver.id else { return }
                faultAt = CACurrentMediaTime()
                faultName = fault.kind
                say("injecting \(fault.kind) into session \(id)")
                switch fault.kind {
                case "cut": RustCore.shared.sessions.injectFault(session: id, fault: .cutTcp)
                case "stall": RustCore.shared.sessions.injectFault(session: id, fault: .stallTcp)
                case "blackhole":
                    RustCore.shared.sessions.injectFault(session: id, fault: .moshBlackhole(on: true))
                    if let until = fault.until {
                        try? await Task.sleep(for: .seconds(until - fault.at))
                        say("blackhole off after \(Int((CACurrentMediaTime() - (faultAt ?? 0)) * 1000)) ms")
                        RustCore.shared.sessions.injectFault(session: id, fault: .moshBlackhole(on: false))
                    }
                case "echo":
                    // How typing fares at this point (after a fault ended).
                    if let session = RustCore.visibleSession { await probe(session, count: 10) }
                default:
                    say("unknown fault \(fault.kind)")
                }
            }
        }
    }

    // MARK: The echo probe

    private static var probing = false

    /// The terminal on screen changed: probe it once it is live, if asked.
    static func sessionBecameVisible(_ session: SessionHub.ActiveSession?) {
        guard let session, !probing, let count = arg("-MOSHPIT_ECHO_PROBE").flatMap(Int.init), count > 0
        else { return }
        probing = true
        Task { @MainActor [weak session] in
            // Live, attached and painted, then a beat for the prompt.
            for _ in 0..<120 {
                guard let session else { return }
                let tmuxReady = session.tmuxController.map { $0.snapshot.isAttached } ?? true
                if session.viewModel.connState == .live, tmuxReady, !session.awaitingFirstFrame { break }
                try? await Task.sleep(for: .milliseconds(500))
            }
            try? await Task.sleep(for: .seconds(3))
            guard let session else { return }
            await probe(session, count: count)
        }
    }

    private static func probe(_ session: SessionHub.ActiveSession, count: Int) async {
        let path = (session.rust == nil ? "swift" : "rust")
            + ((session.tmuxController?.activeCoordinator ?? session.coordinator).surface is CoreTerminalView
               ? "+core" : "")
        let transport = "\(session.connection.connectionProtocol.rawValue)+\(session.connection.multiplexer.rawValue)"
        /// From the keystroke to its echo fed to the terminal (a terminal
        /// fed by the coordinator; the core reading its channel itself is
        /// never fed).
        var fed: [Double] = []
        /// A core terminal: from the keystroke to the first frame taken whose
        /// cursor row shows the echo (whoever fed it).
        var shown: [Double] = []
        var lost = 0
        let letters = Array("abcdefghijklmnopqrstuvwxyz".utf8)
        var typed = ""
        for index in 0..<count {
            guard let coordinator = session.tmuxController?.activeCoordinator ?? Optional(session.coordinator),
                  let surface = coordinator.surface else {
                say("echo probe: no terminal"); break
            }
            if index > 0, index % 20 == 0 {
                coordinator.sendInput([0x15][...])   // ^U: clear the line
                typed = ""
                try? await Task.sleep(for: .milliseconds(300))
            }
            let byte = letters[index % letters.count]
            typed.append(Character(UnicodeScalar(byte)))
            let expected = typed
            let core = surface as? CoreTerminalView
            let echoed = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
                MainActor.assumeIsolated {
                    var finished = false
                    var sawFeed = false, sawFrame = core == nil
                    let start = CACurrentMediaTime()
                    @MainActor func finish(_ ok: Bool) {
                        guard !finished else { return }
                        finished = true
                        SwiftTerminalView.Coordinator.debugFeedTap = nil
                        core?.onFrame = nil
                        continuation.resume(returning: ok)
                    }
                    SwiftTerminalView.Coordinator.debugFeedTap = { into, data in
                        MainActor.assumeIsolated {
                            guard !sawFeed, into === coordinator, data.contains(byte) else { return }
                            sawFeed = true
                            fed.append((CACurrentMediaTime() - start) * 1000)
                            if core == nil { finish(true) }
                        }
                    }
                    core?.onFrame = { [weak core] in
                        guard let core, !sawFrame else { return }
                        let row = Int(core.rows.info.cursor.row)
                        guard core.rowText(row).hasSuffix(expected) else { return }
                        sawFrame = true
                        shown.append((CACurrentMediaTime() - start) * 1000)
                        finish(true)
                    }
                    coordinator.sendInput([byte][...])
                    Task { @MainActor in
                        try? await Task.sleep(for: .seconds(2))
                        finish(false)
                    }
                }
            }
            if !echoed { lost += 1 }
            try? await Task.sleep(for: .milliseconds(40))
        }
        (session.tmuxController?.activeCoordinator ?? session.coordinator).sendInput([0x15][...])
        func line(_ name: String, _ values: [Double]) {
            let v = values.sorted()
            func at(_ q: Double) -> String {
                guard !v.isEmpty else { return "-" }
                return String(format: "%.2f", v[min(v.count - 1, Int((Double(v.count - 1) * q).rounded()))])
            }
            say("echo \(path) \(transport)\(name): n=\(v.count) lost=\(lost) p50=\(at(0.5)) p95=\(at(0.95)) p99=\(at(0.99)) max=\(at(1)) ms")
        }
        if !fed.isEmpty { line("", fed) }
        if !shown.isEmpty { line(" drawn", shown) }
    }
}
#endif
