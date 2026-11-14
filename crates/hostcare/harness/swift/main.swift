// The app's Swift host care at 81c121a1 — HostAutoCare, HostSetupModel and
// PushService, unchanged but for PushService's two calls into UIKit (see
// build-swift.sh) — run through the cross-check's scenarios against a
// simulated host, a pairings file of its own and a relay that answers at
// once. One JSON object per scenario on stdout; crosscheck.py holds them
// to the Rust machine's (examples/crosscheck.rs).

import Foundation
import MoshpitKit

let conn = UUID(uuidString: "3F2504E0-4F89-11D3-9A0C-0305E82C3301")!
let token = String(repeating: "ab", count: 32)
let relay = "https://push.example.org"
let allTools = "home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\nopenssl=yes\ncurl=yes\njq=yes\ntmux=yes\npython3=yes\nbase64=yes"

/// A simulated host: every command recorded, the files it was told to
/// write kept; a self-test push "arrives" as the sender is run.
final class Host: HostChannel, @unchecked Sendable {
    private let lock = NSLock()
    private var _commands: [String] = []
    var files: [String: String] = [:]
    var rules: [(String, String)] = [("printf 'home=", allTools)]
    var failOn: String?
    var arrives: ((String) async -> Void)?

    var commands: [String] { lock.withLock { _commands } }
    private var placed = false

    func run(_ command: String) async throws -> Data {
        lock.withLock {
            _commands.append(command)
            // What the scenario's manifest records is on the host, unless
            // the scenario put something else there (crates/hostcare's
            // crosscheck.rs places the same bytes).
            if !placed {
                placed = true
                for (path, body) in fixtureFiles where files[path] == nil { files[path] = body }
            }
        }
        if let failOn, command.contains(failOn) { throw TestError() }
        if let range = command.range(of: "--test '"),
           let end = command[range.upperBound...].firstIndex(of: "'") {
            await arrives?(String(command[range.upperBound..<end]))
        }
        for (needle, reply) in rules where command.contains(needle) { return Data(reply.utf8) }
        return Data(lock.withLock { simulate(command) }.utf8)
    }

    private func simulate(_ command: String) -> String {
        if let path = between(command, "base64 -d > \"", "\""),
           let payload = between(command, "printf %s '", "' | base64 -d"),
           let data = Data(base64Encoded: payload), let text = String(data: data, encoding: .utf8) {
            files[path] = text
            return ""
        }
        if command.hasPrefix("cat \"") {
            return files[String(command.dropFirst(5).prefix(while: { $0 != "\"" }))] ?? ""
        }
        if command.contains("awk '{print $1}'") {
            guard let path = between(command, "sha256sum \"", "\""), let body = files[path] else { return "" }
            return ContentDigest.of(body)
        }
        if command.hasPrefix("rm -f \"") {
            files[String(command.dropFirst(7).prefix(while: { $0 != "\"" }))] = nil
        }
        return ""
    }

    private func between(_ text: String, _ start: String, _ end: String) -> String? {
        guard let a = text.range(of: start), let b = text.range(of: end, range: a.upperBound..<text.endIndex)
        else { return nil }
        return String(text[a.upperBound..<b.lowerBound])
    }
}

/// What the simulated host's failure says (the Rust side's fake says the
/// same).
struct TestError: LocalizedError {
    var errorDescription: String? { "test" }
}

/// The pairings the app reads through `PushPairingStore`, here a file of the
/// run's own — so care's direct reads and the push service's agree.
final class FileBacking: PushPairingBacking, @unchecked Sendable {
    let file: PushPairingFile
    init(_ file: PushPairingFile) { self.file = file }
    func read() -> [PushPairing] { file.read() }
    func write(_ pairings: [PushPairing]) -> Bool { file.write(pairings) }
    func upsert(_ pairing: PushPairing) -> Bool { file.upsert(pairing) }
    func remove(connectionId: UUID) -> Bool { file.remove(connectionId: connectionId) }
}

/// The files the last manifest() records, for the host to hold.
nonisolated(unsafe) var fixtureFiles: [String: String] = [:]

func manifest(stamp: String?, sender: String?, hooks: [String: String], paired: Bool) -> String {
    if let stamp {
        fixtureFiles["$HOME/.moshpit/moshpit-stamp.sh"] = stamp == current.stamp ? HostScripts.stamp : "# an older stamp\n"
    }
    if let sender {
        fixtureFiles["$HOME/.moshpit/moshpit-push.sh"] = sender == current.sender ? HostScripts.sender : "# an older sender\n"
    }
    for (agent, path) in hooks {
        fixtureFiles[path] = try! AgentHookConfig.merged(existing: "", agent: HookAgent.named(agent)!)
    }
    var m = InstallManifest.empty
    let now = Date(timeIntervalSince1970: 1_762_800_000)
    if let stamp { m[.stamp] = .init(digest: stamp, installedAt: now) }
    if let sender { m[.sender] = .init(digest: sender, installedAt: now) }
    for (agent, path) in hooks { m[.hooks(agent: agent)] = .init(installedAt: now, configPath: path) }
    if paired {
        m[.pairing(conn: conn.uuidString)] = .init(installedAt: now, connectionId: conn.uuidString, relayURL: "r")
    }
    return String(decoding: try! JSONEncoder.moshpitInstall.encode(m), as: UTF8.self)
}

let current = (stamp: HostScripts.digest(of: .stamp)!, sender: HostScripts.digest(of: .sender)!)
let claude = ["claude": "$HOME/.claude/settings.json"]

func readyPairing(relayURL: String = relay) -> PushPairing {
    var p = PushPairing.make(connectionId: conn, hostLabel: "m1-pro", relayURL: relayURL)
    p.sendToken = String(repeating: "cd", count: 32)
    p.apnsToken = token
    p.apnsEnv = "production"
    // Minted just now: fresh, as the Rust side's is on its own clock.
    p.sendTokenIssuedAt = Date()
    return p
}

func status(_ s: ComponentStatus) -> String {
    switch s { case .absent: "absent"; case .current: "current"; case .stale: "stale" }
}

func proof(_ p: HostSetupModel.Proof) -> String {
    switch p {
    case .untested: "untested"
    case .proving: "proving"
    case .proven: "proven"
    case .failed(let why): "failed: \(why)"
    case .unavailable(let why): "unavailable: \(why)"
    }
}

func phase(_ p: HostSetupModel.Phase) -> String {
    switch p { case .idle: "idle"; case .inspecting: "inspecting"; case .working(let l): "working: \(l)" }
}

@MainActor
func run(_ name: String) async -> [String: Any] {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("w15b-x-\(UUID().uuidString)")
    try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer {
        try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dir.path)
        try? FileManager.default.removeItem(at: dir)
    }
    let file = PushPairingFile(url: dir.appendingPathComponent("push-pairings.json"))
    PushPairingStore.backing = FileBacking(file)
    fixtureFiles = [:]
    var mints = 0
    let push = PushService(pairingFile: file, deviceToken: token) { _, _, _ in
        mints += 1
        return (String(repeating: "cd", count: 32), Date(timeIntervalSince1970: 1_762_800_000))
    }
    let host = Host()
    host.arrives = { nonce in await MainActor.run { push.noteSelfTestArrived(nonce: nonce) } }
    let settings = AppSettings()
    let store = ConnectionStore()
    let connection = ServerConnection(id: conn, name: "m1-pro")
    store.connections = [connection]
    let care = HostAutoCare(settings: settings, push: push, connectionStore: store)
    let model = HostSetupModel(connection: connection, push: push, care: care) { HostInstaller(channel: host) }
    var page = true
    let readOnly = { try! FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path) }
    func tend() async {
        care.tend(connection: connection) { HostInstaller(channel: host) }
        while care.isTending(conn) { try? await Task.sleep(for: .milliseconds(2)) }
    }

    // A Host Setup page reads its host as it appears (HostSetupNSHPage's
    // task); the Rust machine's page does the same as it opens.
    let pageScenario = !name.hasPrefix("care-") && !name.hasPrefix("unpair")
    if pageScenario {
        switch name {
        case "inspect-fails": host.failOn = "printf"
        case "install-broken-json": host.rules.append(("cat \"$HOME/.claude/settings.json\"", "{broken"))
        case "pair-missing-tools": host.rules = [("printf 'home=", "home=/h\ncurl=no\nopenssl=no\nbase64=yes")]
        case "prove-hooks-stamped":
            host.rules += [("socket_path", "/tmp/tmux-501/default"), ("display-message -p -t", "working|moshpit-selftest|")]
        case "prove-hooks-codex-empty":
            host.rules += [("socket_path", "/tmp/tmux-501/default"), ("display-message -p -t", "")]
        case "prove-push-sender-output":
            host.arrives = nil
            host.rules.append(("--test", "relay refused: 401"))
        case "prove-push-exec-fails":
            host.arrives = nil
        default: break
        }
        await model.inspect()
    }

    let began = ContinuousClock.now
    switch name {
    case "inspect", "inspect-fails":
        break
    case "install-claude":
        await model.installHooks()
    case "install-codex":
        model.select(HookAgent.named("codex")!); await model.installHooks()
    case "install-broken-json":
        await model.installHooks()
    case "install-then-remove":
        await model.installHooks(); await model.removeHooks()
    case "pair", "pair-missing-tools":
        await model.pair(relayURL: relay)
    case "pair-empty-relay":
        await model.pair(relayURL: "   ")
    case "unpair", "unpair-cannot-forget":
        _ = file.write([readyPairing()])
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: true)
        host.files["$HOME/.moshpit/push.d/\(conn.uuidString).conf"] = HostInstaller.pushConf(readyPairing())
        await model.inspect()
        if name == "unpair-cannot-forget" { readOnly() }
        await model.unpair()
    case "prove-hooks-no-pane":
        await model.proveHooks(pane: nil)
    case "prove-hooks-elsewhere":
        await model.proveHooks(pane: "%3")
    case "prove-hooks-stamped":
        await model.proveHooks(pane: "%3")
    case "prove-hooks-codex-empty":
        model.select(HookAgent.named("codex")!)
        await model.proveHooks(pane: "%3")
    case "prove-push-sender-output":
        await model.provePush()
    case "prove-push-exec-fails":
        host.failOn = "--test"
        await model.provePush()
    case "care-fresh":
        page = false; await tend()
    case "care-hooked":
        page = false
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        await tend()
    case "care-stale-sender", "care-stale-stamp":
        page = false
        let both = ["claude": "$HOME/.claude/settings.json", "codex": "$HOME/.codex/config.toml"]
        host.files[InstallManifest.path] = name == "care-stale-sender"
            ? manifest(stamp: current.stamp, sender: "old", hooks: both, paired: false)
            : manifest(stamp: "old", sender: current.sender, hooks: both, paired: false)
        await tend()
    case "care-ready-pairing":
        page = false
        _ = file.write([readyPairing()])
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        await tend()
    case "care-wrong-relay":
        page = false
        _ = file.write([readyPairing(relayURL: "https://push.example")])
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        await tend()
    case "care-notifications-off":
        page = false
        settings.notificationsEnabled = false
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        await tend()
    case "care-unsaved":
        page = false
        store.connections = []
        await tend()
    case "care-grant":
        page = false
        await tend()
        care.grantConsent(connection: connection,
                          session: SessionHub.ActiveSession(connection: connection, channel: host))
        while care.isTending(conn) { try? await Task.sleep(for: .milliseconds(2)) }
    case "care-declined":
        page = false
        settings.hostSetupDeclined = [conn.uuidString]
        await tend()
    case "care-relay-trailing-slash":
        page = false
        settings.pushRelayURL = relay + "/"
        _ = file.write([readyPairing()])
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        await tend()
    case "care-conf-deleted":
        // Paired, the manifest recording this phone's conf — which was
        // deleted by hand on the host (B315).
        page = false
        let phone = readyPairing()
        _ = file.write([phone])
        _ = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        var m = InstallManifest.empty
        let now = Date(timeIntervalSince1970: 1_762_800_000)
        m[.stamp] = .init(digest: current.stamp, installedAt: now)
        m[.sender] = .init(digest: current.sender, installedAt: now)
        m[.hooks(agent: "claude")] = .init(installedAt: now, configPath: claude["claude"]!)
        m[.pairing(conn: conn.uuidString)] = .init(digest: ContentDigest.of(HostInstaller.pushConf(phone)),
                                                  installedAt: now, connectionId: conn.uuidString, relayURL: relay)
        host.files[InstallManifest.path] = String(decoding: try! JSONEncoder.moshpitInstall.encode(m), as: UTF8.self)
        await tend()
    case "care-hooks-removed":
        // The manifest records Claude Code's hooks; they were taken out of
        // its config by hand.
        page = false
        host.files["$HOME/.claude/settings.json"] = "{\"model\":\"opus\"}"
        host.files[InstallManifest.path] = manifest(stamp: current.stamp, sender: current.sender, hooks: claude, paired: false)
        await tend()
    default:
        fatalError("no scenario \(name)")
    }

    var out: [String: Any] = [
        "scenario": name,
        "commands": host.commands,
        "files": host.files,
        "phone": file.read().map { p -> [String: Any] in
            ["conn": p.connectionId.uuidString, "label": p.hostLabel, "secret": p.secretHex,
             "token": p.sendToken, "relay": p.relayURL, "apns": p.apnsToken ?? "", "env": p.apnsEnv ?? ""]
        },
        "consent": care.needsConsent?.id == conn,
        "declined": Array(settings.hostSetupDeclined).sorted(),
        "mints": mints,
        // Whole seconds the scenario's steps took (its waits).
        "waited": Int((ContinuousClock.now - began).components.seconds),
    ]
    if page {
        out["page"] = [
            "phase": phase(model.phase),
            "error": model.error ?? "",
            "hooks": status(model.hooksStatus),
            "pairing": status(model.status(model.pairingComponent)),
            "hooksProof": proof(model.hooksProof),
            "pushProof": proof(model.pushProof),
            "userAction": model.userActionRequired ?? "",
            "relayError": model.relayError ?? "",
        ]
    }
    return out
}

let names = CommandLine.arguments.dropFirst()
Task { @MainActor in
    for name in names {
        let result = await run(name)
        let data = try! JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }
    exit(0)
}
RunLoop.main.run()
