// Harness-only command line over the app's UNCHANGED install sources
// (Moshpit/Services/Install/{HostScripts,InstallManifest,HostChannel,
// HostInstaller,AgentHookConfig}.swift and Moshpit/Services/
// HostCapabilities.swift, against the MoshpitKit module), built by
// harness/build-swift.sh. examples/install_harness.rs is the same command
// line over the Rust crate (src/harness.rs, which documents the format);
// harness/equivalence.py drives both.
//
//   install-swift run <cases> <out>

import Foundation
import MoshpitKit

func hex(_ bytes: [UInt8]) -> String {
    let digits = Array("0123456789abcdef".utf8)
    var out = [UInt8]()
    for b in bytes {
        out.append(digits[Int(b >> 4)])
        out.append(digits[Int(b & 0xF)])
    }
    return String(decoding: out, as: UTF8.self)
}

func unhexBytes(_ text: Substring) -> [UInt8] {
    var out = [UInt8]()
    var index = text.startIndex
    while index < text.endIndex {
        let next = text.index(index, offsetBy: 2)
        out.append(UInt8(text[index..<next], radix: 16)!)
        index = next
    }
    return out
}

func arg(_ token: Substring) -> String {
    token == "~" ? "" : String(decoding: unhexBytes(token), as: UTF8.self)
}

func optArg(_ token: Substring) -> String? { token == "-" ? nil : arg(token) }

func out(_ text: String) -> String { text.isEmpty ? "~" : hex(Array(text.utf8)) }

func optOut(_ text: String?) -> String { text.map(out) ?? "-" }

func component(_ key: String) -> InstallComponent {
    if key == "stamp" { return .stamp }
    if key == "sender" { return .sender }
    if key.hasPrefix("hooks.") { return .hooks(agent: String(key.dropFirst(6))) }
    return .pairing(conn: String(key.dropFirst(8)))
}

/// `"installedAt" : "<date>"` masked: Swift stamps manifests with Date().
func masked(_ text: String) -> String {
    text.replacingOccurrences(of: #""installedAt" : "[^"]*""#, with: #""installedAt" : "*""#,
                              options: .regularExpression)
}

func manifestOut(_ manifest: InstallManifest) -> String {
    if manifest == .empty { return "empty" }
    let parts = manifest.components.keys.sorted { Array($0.utf8).lexicographicallyPrecedes(Array($1.utf8)) }.map { key in
        let c = manifest.components[key]!
        return [out(key), optOut(c.digest), String(c.installedAt.timeIntervalSince1970.bitPattern, radix: 16),
                optOut(c.configPath), optOut(c.connectionId), optOut(c.relayURL)].joined(separator: "|")
    }
    return "schema=\(manifest.schema) " + (parts.isEmpty ? "-" : parts.joined(separator: ","))
}

func action(_ action: InstallStep.Action) -> String {
    switch action {
    case .installed(let digest): return "installed:" + digest
    case .alreadyCurrent: return "alreadyCurrent"
    case .removed: return "removed"
    case .notApplicable: return "notApplicable"
    case .failed(let why): return "failed:" + out(why)
    }
}

func reportOut(_ report: InstallReport) -> String {
    let steps = report.steps.map { "\($0.component.key)=\(action($0.action))" }.joined(separator: ",")
    var blocked = "-"
    if case .missingTools(let tools) = report.blocked { blocked = tools.joined(separator: ",") }
    let manifest = report.manifest.map {
        out(masked(String(decoding: try! JSONEncoder.moshpitInstall.encode($0), as: UTF8.self)))
    } ?? "-"
    return "steps=\(steps.isEmpty ? "-" : steps) blocked=\(blocked) user=\(optOut(report.userActionRequired)) " +
        "ok=\(report.succeeded ? 1 : 0) manifest=\(manifest)"
}

/// A simulated host: records commands, keeps written files, answers
/// checksums from what it holds (or wrongly, or not at all).
final class FakeHost: HostChannel, @unchecked Sendable {
    var commands: [String] = []
    var files: [String: String] = [:]
    var preflight = ""
    var checksums = "ok"
    var socket = ""
    var stamp = ""

    func run(_ command: String) async throws -> Data {
        var recorded = command
        defer { commands.append(recorded) }
        if command.contains("printf 'home=") { return Data(preflight.utf8) }
        if command.contains("socket_path") { return Data(socket.utf8) }
        if command.contains("display-message -p -t") { return Data(stamp.utf8) }
        if let start = command.range(of: "base64 -d > \""),
           let end = command.range(of: "\"", range: start.upperBound..<command.endIndex),
           let p0 = command.range(of: "printf %s '"),
           let p1 = command.range(of: "' | base64 -d", range: p0.upperBound..<command.endIndex) {
            let path = String(command[start.upperBound..<end.lowerBound])
            let text = String(decoding: Data(base64Encoded: String(command[p0.upperBound..<p1.lowerBound]))!,
                              as: UTF8.self)
            files[path] = text
            if path.hasSuffix("manifest.json") {
                recorded = command.replacingCharacters(
                    in: p0.upperBound..<p1.lowerBound, with: Data(masked(text).utf8).base64EncodedString())
            }
            return Data()
        }
        if command.hasPrefix("cat \"") {
            let path = String(command.dropFirst(5).prefix { $0 != "\"" })
            return Data((files[path] ?? "").utf8)
        }
        if command.contains("awk '{print $1}'") {
            switch checksums {
            case "wrong": return Data("deadbeefdeadbeef".utf8)
            case "empty": return Data()
            default:
                guard let start = command.range(of: "sha256sum \""),
                      let end = command.range(of: "\"", range: start.upperBound..<command.endIndex),
                      let body = files[String(command[start.upperBound..<end.lowerBound])] else { return Data() }
                return Data(ContentDigest.of(body).utf8)
            }
        }
        if command.hasPrefix("rm -f \"") {
            files[String(command.dropFirst(7).prefix { $0 != "\"" })] = nil
        }
        return Data()
    }
}

func flow(_ f: [Substring]) async -> String {
    // flow <kind> <preflight> <manifest|-> <config path|-> <config|-> <checksums> <args…>
    let host = FakeHost()
    host.preflight = arg(f[2])
    if f[3] != "-" { host.files[InstallManifest.path] = arg(f[3]) }
    if f[4] != "-" && f[5] != "-" { host.files[arg(f[4])] = arg(f[5]) }
    host.checksums = String(f[6])
    let installer = HostInstaller(channel: host)
    var result = ""
    do {
        let state = try await installer.inspect()
        switch f[1] {
        case "pairing":
            let pairing = try JSONDecoder().decode(PushPairing.self, from: Data(unhexBytes(f[7])))
            result = reportOut(try await installer.installPairing(pairing, state: state))
        case "hooks":
            result = reportOut(try await installer.installHooks(agent: HookAgent.named(String(f[7]))!, state: state))
        case "unhooks":
            result = reportOut(try await installer.uninstallHooks(agent: HookAgent.named(String(f[7]))!, state: state))
        case "uninstall":
            let components = f[7].split(separator: ",").map { component(arg($0)) }
            let after = try await installer.uninstall(components, state: state)
            result = out(masked(String(decoding: try JSONEncoder.moshpitInstall.encode(after), as: UTF8.self)))
        case "stamp":
            host.socket = arg(f[8])
            host.stamp = arg(f[9])
            switch try await installer.selfTestStamp(pane: arg(f[7])) {
            case .stamped: result = "stamped"
            case .didNotStamp(let read): result = "didNotStamp:" + out(read)
            case .unavailable: result = "unavailable"
            }
        case "push":
            _ = try await installer.selfTestPush(nonce: arg(f[7]))
            result = "pushed"
        case "status":
            let agents = HookAgent.all.map { "\($0.id)=\(statusOut(state.hooksStatus(agent: $0.id)))" }
            let auto = HostAutoCareHarness.installedHookAgents(state.manifest).map(\.id).joined(separator: ",")
            result = "stamp=\(statusOut(state.status(.stamp))) sender=\(statusOut(state.status(.sender))) " +
                agents.joined(separator: " ") + " paired=\(state.pairedButNothingWillPush ? 1 : 0) " +
                "installed=\(auto.isEmpty ? "-" : auto) manifest=\(manifestOut(state.manifest))"
        default:
            fatalError("bad flow \(f[1])")
        }
    } catch {
        result = "threw"
    }
    // Each command as the first 64 bits of its SHA-256: equal digests are
    // equal commands, and the scripts' payloads would make lines of 40 KB.
    return result + " || " + host.commands.map { String(ContentDigest.of($0).prefix(16)) }.joined(separator: ",")
}

func statusOut(_ status: ComponentStatus) -> String {
    switch status {
    case .absent: return "absent"
    case .current: return "current"
    case .stale(let installed): return "stale:" + out(installed)
    }
}

/// `HostAutoCare.installedHookAgents`, which lives on a main-actor class the
/// harness cannot build; the same one line.
enum HostAutoCareHarness {
    static func installedHookAgents(_ manifest: InstallManifest) -> [HookAgent] {
        HookAgent.all.filter { manifest[.hooks(agent: $0.id)] != nil }
    }
}

func run(_ line: Substring) async -> String {
    let f = line.split(separator: " ", omittingEmptySubsequences: false)
    switch f[0] {
    case "scripts":
        return [out(HostScripts.stamp), out(HostScripts.sender), HostScripts.digest(of: .stamp)!,
                HostScripts.digest(of: .sender)!].joined(separator: " ")
    case "conf":
        let pairing = try! JSONDecoder().decode(PushPairing.self, from: Data(unhexBytes(f[1])))
        let conf = HostInstaller.pushConf(pairing)
        return out(conf) + " " + ContentDigest.of(conf)
    case "merge", "remove":
        let agent = HookAgent.named(String(f[1]))!
        do {
            let text = f[0] == "merge" ? try AgentHookConfig.merged(existing: arg(f[2]), agent: agent)
                                       : try AgentHookConfig.removed(existing: arg(f[2]), agent: agent)
            return "ok " + out(text)
        } catch {
            return "err " + out(error.localizedDescription)
        }
    case "agent":
        let a = HookAgent.named(String(f[1]))!
        return [out(a.displayName), out(a.configPath), out(a.configHint), a.format == .claudeJSON ? "json" : "toml",
                optOut(a.trustStep)].joined(separator: " ")
    case "manifest":
        var manifest = InstallManifest.empty
        for op in f.dropFirst() {
            let p = op.split(separator: ":", omittingEmptySubsequences: false)
            manifest[component(arg(p[0]))] = .init(
                digest: optArg(p[1]), installedAt: Date(timeIntervalSince1970: TimeInterval(Int(p[5])!)),
                configPath: optArg(p[2]), connectionId: optArg(p[3]), relayURL: optArg(p[4]))
        }
        return out(String(decoding: try! JSONEncoder.moshpitInstall.encode(manifest), as: UTF8.self))
    case "mread":
        final class Text: HostChannel, @unchecked Sendable {
            let text: String
            init(_ text: String) { self.text = text }
            func run(_ command: String) async throws -> Data { Data(text.utf8) }
        }
        return manifestOut(try! await InstallManifest.read(from: Text(arg(f[1]))))
    case "facts":
        let facts = HostFacts.parse(arg(f[1]))
        let tools = facts.tools.keys.sorted { Array($0.utf8).lexicographicallyPrecedes(Array($1.utf8)) }
            .map { out($0) + "=" + (facts.tools[$0]! ? "1" : "0") }
        return [out(facts.home), out(facts.shell), out(facts.uname), tools.isEmpty ? "-" : tools.joined(separator: ","),
                facts.canPush ? "1" : "0", facts.missingForPush.joined(separator: ",")].joined(separator: " ")
    case "caps":
        let c = HostCapabilities.parse(arg(f[1]))
        return [c.hasTmux, c.hasMoshServer, c.hasHerdr, c.hasPython3].map { $0 ? "1" : "0" }.joined() + " " +
            out(c.os) + " " + (c.packageManager?.rawValue ?? "-")
    case "probe":
        return out(HostCapabilities.probeCommand) + " " + out(HostCapabilities.extraPathDirs)
    case "install":
        let m = Multiplexer(rawValue: String(f[1]))!
        let manager = f[2] == "-" ? nil : PackageManager(rawValue: String(f[2]))!
        return optOut(m.installCommand(using: manager))
    case "cmd":
        switch f[1] {
        case "write": return out(HostCommands.writeFile(path: arg(f[2]), mode: arg(f[3]), base64: arg(f[4])))
        case "read": return out(HostCommands.readFile(path: arg(f[2])))
        case "sha": return out(HostCommands.sha256(path: arg(f[2])))
        case "preflight": return out(HostCommands.preflight)
        case "backup": return out(HostCommands.backupOnce(configPath: arg(f[2])))
        case "socket": return out(HostCommands.tmuxSocketPath)
        case "selftest": return out(HostCommands.selfTest(state: arg(f[2]), pane: arg(f[3]), tmuxSocket: arg(f[4])))
        case "readstamp": return out(HostCommands.readStamp(pane: arg(f[2]), tmuxSocket: arg(f[3])))
        case "clearstamp": return out(HostCommands.clearStamp(pane: arg(f[2]), tmuxSocket: arg(f[3])))
        case "remove": return out(HostCommands.removeFile(path: arg(f[2])))
        case "prune": return out(HostCommands.pruneBackups(configPath: arg(f[2]), keep: Int(f[3])!))
        case "quote": return out(HostCommands.quote(arg(f[2])))
        case "safe": return HostCommands.isSafePath(arg(f[2])) ? "1" : "0"
        case "path": return out(InstallComponent.pairing(conn: arg(f[2])).path!) + " " +
            out(component(arg(f[3])).key) + " " + optOut(component(arg(f[3])).path) + " " + component(arg(f[3])).mode
        default: fatalError("bad cmd \(f[1])")
        }
    case "flow":
        return await flow(f)
    default:
        fatalError("unknown command \(f[0])")
    }
}

@main
struct Main {
    static func main() async {
        let args = CommandLine.arguments
        guard args.count == 4, args[1] == "run" else {
            FileHandle.standardError.write(Data("usage: install-swift run <cases> <out>\n".utf8))
            exit(2)
        }
        let cases = try! String(contentsOfFile: args[2], encoding: .utf8)
        var lines: [String] = []
        for line in cases.split(separator: "\n", omittingEmptySubsequences: true) where !line.hasPrefix("#") {
            lines.append(await run(line))
        }
        try! (lines.joined(separator: "\n") + "\n").write(toFile: args[3], atomically: true, encoding: .utf8)
    }
}
