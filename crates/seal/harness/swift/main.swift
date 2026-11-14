// Harness-only command line over the app's UNCHANGED push sources
// (Packages/MoshpitKit/Sources/MoshpitKit/{Log,Push/*,Island/
// AgentNotifications,Island/AgentControlBridge}.swift), built by
// harness/build-swift.sh. examples/seal_harness.rs is the same command line
// over the Rust crate (src/harness.rs); harness/equivalence.py feeds both the
// same cases and compares.
//
//   seal-swift run <cases> <out> <scratch-dir>   one result line per case
//   seal-swift tables                            numericType ranges
//
// The case format is documented in src/harness.rs.

import Foundation
import UserNotifications

// MARK: - Encoding helpers

func hex(_ bytes: [UInt8]) -> String {
    let digits = Array("0123456789abcdef".utf8)
    var out = [UInt8]()
    out.reserveCapacity(bytes.count * 2)
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

/// A string argument: `~` is empty, otherwise hex of its UTF-8.
func arg(_ token: Substring) -> String {
    token == "~" ? "" : String(decoding: unhexBytes(token), as: UTF8.self)
}

/// An optional string argument: `-` is nil.
func optArg(_ token: Substring) -> String? {
    token == "-" ? nil : arg(token)
}

/// A string result: `~` when empty, otherwise hex of its UTF-8.
func out(_ text: String) -> String {
    text.isEmpty ? "~" : hex(Array(text.utf8))
}

func optOut(_ text: String?) -> String {
    text.map(out) ?? "-"
}

func dateOut(_ date: Date?) -> String {
    guard let date else { return "-" }
    return String(date.timeIntervalSinceReferenceDate.bitPattern, radix: 16)
}

func failure(_ error: Error) -> String {
    guard let failure = error as? PushSealedBox.Failure else { return "other" }
    switch failure {
    case .badSecret: return "badSecret"
    case .unsupportedVersion(let v): return "unsupportedVersion(\(v))"
    case .malformed: return "malformed"
    case .authenticationFailed: return "authenticationFailed"
    case .decryptFailed: return "decryptFailed"
    }
}

func statusOut(_ s: PushSealedBox.Status) -> String {
    [out(s.conn), out(s.host), optOut(s.sess), out(s.pane), optOut(s.agent), out(s.state),
     optOut(s.title), String(s.ts), s.dur.map(String.init) ?? "-"].joined(separator: " ")
}

func pairingOut(_ p: PushPairing) -> String {
    [p.connectionId.uuidString, out(p.hostLabel), out(p.secretHex), out(p.sendToken), out(p.relayURL),
     dateOut(p.createdAt), optOut(p.apnsToken), optOut(p.apnsEnv), dateOut(p.sendTokenIssuedAt)]
        .joined(separator: " ")
}

func entryOut(_ e: PushStanding.Entry) -> String {
    [out(e.pane), optOut(e.agent), optOut(e.title), optOut(e.location), String(e.since), dateOut(e.recordedAt)]
        .joined(separator: " ")
}

/// A file's bytes as hex, `-` when absent.
func fileOut(_ url: URL) -> String {
    guard let data = try? Data(contentsOf: url) else { return "-" }
    return data.isEmpty ? "~" : hex([UInt8](data))
}

/// `<file>`, `<file>.backup`, and every `<file>.backup-*` (contents sorted).
func filesOut(_ url: URL) -> String {
    let dir = url.deletingLastPathComponent()
    let name = url.lastPathComponent
    let stamped = ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? [])
        .filter { $0.hasPrefix(name + ".backup-") }
        .map { fileOut(dir.appendingPathComponent($0)) }
        .sorted()
    return "file=\(fileOut(url)) backup=\(fileOut(url.appendingPathExtension("backup"))) stamped=\(stamped.isEmpty ? "-" : stamped.joined(separator: ","))"
}

func level(_ l: UNNotificationInterruptionLevel) -> String {
    switch l {
    case .passive: return "passive"
    case .active: return "active"
    case .timeSensitive: return "timeSensitive"
    case .critical: return "critical"
    @unknown default: return "unknown"
    }
}

// MARK: - Commands

func status(_ f: [Substring], _ at: Int) -> PushSealedBox.Status {
    PushSealedBox.Status(conn: arg(f[at]), host: arg(f[at + 1]), sess: optArg(f[at + 2]), pane: arg(f[at + 3]),
                         agent: optArg(f[at + 4]), state: arg(f[at + 5]), title: optArg(f[at + 6]),
                         ts: Int(f[at + 7])!, dur: f[at + 8] == "-" ? nil : Int(f[at + 8])!)
}

/// Prepare a store file: `-` none, `!` there but unreadable, else its bytes.
func prepare(_ url: URL, _ token: Substring) {
    try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    try! FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    switch token {
    case "-": break
    case "!":
        try! Data("[]".utf8).write(to: url)
        try! FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: url.path)
    default:
        try! Data(unhexBytes(token)).write(to: url)
    }
}

func release(_ url: URL) {
    try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: url.path)
}

func run(_ line: Substring, scratch: URL) -> String {
    let f = line.split(separator: " ", omittingEmptySubsequences: false)
    switch f[0] {
    case "derive":
        do {
            let keys = try PushSealedBox.deriveKeys(secretHex: arg(f[1]))
            return "ok \(keys.encHex) \(keys.macHex)"
        } catch { return "err \(failure(error))" }

    case "open":
        let envelope = PushSealedBox.Envelope(v: Int(f[1])!, iv: arg(f[2]), ct: arg(f[3]), mac: arg(f[4]))
        let secret = arg(f[5])
        let raw: String
        do { raw = "raw=" + hex([UInt8](try PushSealedBox.openRaw(envelope, secretHex: secret))) }
        catch { raw = "raw!" + failure(error) }
        let opened: String
        do { opened = "status=" + statusOut(try PushSealedBox.open(envelope, secretHex: secret)) }
        catch { opened = "status!" + failure(error) }
        return raw + " " + opened

    case "seal":
        do {
            let e = try PushSealedBox.seal(Data(unhexBytes(f[3])), secretHex: arg(f[1]), ivHex: arg(f[2]))
            return "ok \(e.v) \(out(e.iv)) \(out(e.ct)) \(out(e.mac))"
        } catch { return "err \(failure(error))" }

    case "status":
        do { return statusOut(try JSONDecoder().decode(PushSealedBox.Status.self, from: Data(unhexBytes(f[1])))) }
        catch { return "err" }

    case "detail":
        return optOut(AgentNotificationCopy.detail(optArg(f[1])))

    case "place":
        return out(AgentNotificationCopy.place(label: arg(f[1]), session: optArg(f[2])))

    case "duration":
        return optOut(AgentNotificationCopy.duration(Int(f[1])!, locale: Locale(identifier: String(f[2]))))

    case "render":
        // render <status 9 fields> <edge> <count> <showDetail> <sound> <label>
        let s = status(f, 1)
        let content = UNMutableNotificationContent()
        content.categoryIdentifier = "left.over"
        PushRemoteNotification.apply(s, to: content, attentionEdge: f[10] == "1", standingCount: Int(f[11])!,
                                     prefs: .init(showDetail: f[12] == "1", sound: f[13] == "1"),
                                     label: optArg(f[14]))
        let info = content.userInfo
        let nonce = AgentNotificationHandler.selfTestNonce(in: info)
        return [out(content.title), out(content.body), content.sound == nil ? "0" : "1",
                level(content.interruptionLevel), out(info[AgentNotifications.connectionKey] as? String ?? "?"),
                out(info[AgentNotifications.paneKey] as? String ?? "?"),
                out(info[PushRemoteNotification.agentKey] as? String ?? "?"),
                out(info[PushRemoteNotification.stateKey] as? String ?? "?"),
                out(info[PushRemoteNotification.detailKey] as? String ?? "?"),
                out(content.categoryIdentifier), optOut(nonce)].joined(separator: " ")

    case "location":
        return out(PushRemoteNotification.location(status(f, 1), label: optArg(f[10])))

    case "payload":
        guard let object = try? JSONSerialization.jsonObject(with: Data(unhexBytes(f[1])), options: [.fragmentsAllowed])
        else { return "badjson" }
        guard let info = object as? [AnyHashable: Any],
              let e = PushRemoteNotification.envelope(in: info) else { return "-" }
        return "\(e.v) \(out(e.iv)) \(out(e.ct)) \(out(e.mac))"

    case "trysecrets":
        // trysecrets <v> <iv> <ct> <mac> <secret>…
        let envelope = PushSealedBox.Envelope(v: Int(f[1])!, iv: arg(f[2]), ct: arg(f[3]), mac: arg(f[4]))
        let secrets = f.dropFirst(5).map(arg)
        return PushRemoteNotification.open(envelope, secrets: Array(secrets)).map(statusOut) ?? "-"

    case "pfile":
        // pfile <initial> <op>… : u:<pairing json hex> r:<uuid> w:<list json hex> R
        let url = scratch.appendingPathComponent("pfile/push-pairings.json")
        prepare(url, f[1])
        let file = PushPairingFile(url: url)
        var results: [String] = []
        for op in f.dropFirst(2) {
            if op.hasPrefix("u:") {
                let pairing = try! JSONDecoder().decode(PushPairing.self, from: Data(unhexBytes(op.dropFirst(2))))
                results.append(file.upsert(pairing) ? "1" : "0")
            } else if op.hasPrefix("r:") {
                results.append(file.remove(connectionId: UUID(uuidString: String(op.dropFirst(2)))!) ? "1" : "0")
            } else if op.hasPrefix("w:") {
                let list = try! JSONDecoder().decode([PushPairing].self, from: Data(unhexBytes(op.dropFirst(2))))
                results.append(file.write(list) ? "1" : "0")
            } else if op == "R" {
                let read = file.read()
                results.append(read.isEmpty ? "[]" : "[" + read.map(pairingOut).joined(separator: ";") + "]")
            } else if op == "C" {
                let c = file.contents()
                results.append("carried=\(c.carried.count) list=\(c.isList ? 1 : 0) unreadable=\(c.unreadable ? 1 : 0)")
            } else if op == "S" {
                results.append(PushPairingStoreHarness.secretsNewestFirst(file).map(out).joined(separator: ","))
            } else if op == "!" {
                release(url)
            }
        }
        release(url)
        return results.joined(separator: " | ") + " || " + filesOut(url)

    case "sfile":
        // sfile <initial> <now unix> <op>… : n:<conn hex>:<pane hex>:<agent|->:<title|->:<since>:<recordedAt ref bits>
        //   c:<conn hex>:<pane hex>  a:<conn hex>  R:<conn hex>  M (read all)  ! (make readable)
        let url = scratch.appendingPathComponent("sfile/standing-attention.json")
        prepare(url, f[1])
        let now = Date(timeIntervalSince1970: TimeInterval(f[2])!)
        let file = PushStandingFile(url: url)
        var results: [String] = []
        for op in f.dropFirst(3) {
            let p = op.split(separator: ":", omittingEmptySubsequences: false)
            switch p[0] {
            case "n":
                let entry = PushStanding.Entry(pane: arg(p[2]), agent: optArg(p[3]), title: optArg(p[4]),
                                               since: Int(p[5])!,
                                               recordedAt: Date(timeIntervalSinceReferenceDate:
                                                                    Double(bitPattern: UInt64(p[6], radix: 16)!)))
                results.append(file.noteStanding(conn: arg(p[1]), entry: entry, now: now) ? "1" : "0")
            case "c":
                results.append("[" + file.clear(conn: arg(p[1]), pane: arg(p[2]), now: now).map(entryOut)
                                   .joined(separator: ";") + "]")
            case "a":
                file.clearAll(conn: arg(p[1]), now: now)
                results.append("ok")
            case "R":
                let entries = (file.read(now: now)[arg(p[1])] ?? []).sorted { $0.recordedAt > $1.recordedAt }
                results.append("[" + entries.map(entryOut).joined(separator: ";") + "]")
            case "M":
                let all = file.read(now: now)
                results.append(all.keys.sorted().map { out($0) + "=" + String(all[$0]!.count) }.joined(separator: ","))
            case "!":
                release(url)
            default:
                fatalError("bad sfile op \(op)")
            }
        }
        release(url)
        return results.joined(separator: " | ") + " || " + filesOut(url)

    case "jsonser":
        do {
            let object = try JSONSerialization.jsonObject(with: Data(unhexBytes(f[1])), options: [.fragmentsAllowed])
            guard JSONSerialization.isValidJSONObject([object]) else { return "invalid" }
            let pretty = try JSONSerialization.data(withJSONObject: object,
                                                    options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes,
                                                              .fragmentsAllowed])
            let compact = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .fragmentsAllowed])
            return hex([UInt8](pretty)) + " " + hex([UInt8](compact))
        } catch { return "err" }

    case "b64":
        return Data(base64Encoded: arg(f[1])).map { $0.isEmpty ? "~" : hex([UInt8]($0)) } ?? "-"

    case "uuid":
        return UUID(uuidString: arg(f[1]))?.uuidString ?? "-"

    case "iso":
        struct D: Decodable { let at: Date }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let json = "{\"at\":\"" + arg(f[1]) + "\"}"
        return (try? decoder.decode(D.self, from: Data(json.utf8))).map { dateOut($0.at) } ?? "-"

    case "isoenc":
        struct E: Encodable { let at: Date }
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let date = Date(timeIntervalSinceReferenceDate: Double(bitPattern: UInt64(f[1], radix: 16)!))
        return String(decoding: try! encoder.encode(E(at: date)), as: UTF8.self)

    case "dbl":
        let x = Double(bitPattern: UInt64(f[1], radix: 16)!)
        let text = String(decoding: try! JSONEncoder().encode([x]), as: UTF8.self)
        return String(text.dropFirst().dropLast())

    case "nsjson":
        // The number a JSONSerialization-read field holds, as the stores read it.
        guard let object = try? JSONSerialization.jsonObject(with: Data(unhexBytes(f[1])), options: [.fragmentsAllowed]),
              let fields = object as? [String: Any] else { return "badjson" }
        let v = fields["v"]
        let int = (v as? Int).map(String.init) ?? "-"
        let double = (v as? Double).map { String($0.bitPattern, radix: 16) } ?? "-"
        return int + " " + double

    default:
        fatalError("unknown command \(f[0])")
    }
}

/// `PushPairingStore.secretsNewestFirst` over a file of the harness's own.
enum PushPairingStoreHarness {
    static func secretsNewestFirst(_ file: PushPairingFile) -> [String] {
        file.read().sorted { $0.createdAt > $1.createdAt }.map(\.secretHex)
    }
}

// MARK: - Main

@main
struct Main {
    static func main() {
        let args = CommandLine.arguments
        switch args.count > 1 ? args[1] : "" {
        case "run" where args.count == 5:
            let cases = try! String(contentsOfFile: args[2], encoding: .utf8)
            let scratch = URL(fileURLWithPath: args[4])
            var lines: [String] = []
            for line in cases.split(separator: "\n", omittingEmptySubsequences: true) where !line.hasPrefix("#") {
                lines.append(run(line, scratch: scratch))
            }
            try! (lines.joined(separator: "\n") + "\n").write(toFile: args[3], atomically: true, encoding: .utf8)
        case "tables":
            var ranges: [(UInt32, UInt32)] = []
            var start: UInt32?
            var previous: UInt32 = 0
            for value in UInt32(0)...0x10FFFF {
                guard let scalar = Unicode.Scalar(value), scalar.properties.numericType != nil else { continue }
                if start != nil, previous == value - 1 {
                    previous = value
                } else {
                    if let s = start { ranges.append((s, previous)) }
                    start = value
                    previous = value
                }
            }
            if let s = start { ranges.append((s, previous)) }
            print(ranges.map { String(format: "%X-%X", $0.0, $0.1) }.joined(separator: " "))
        default:
            FileHandle.standardError.write(Data("usage: seal-swift run <cases> <out> <scratch> | tables\n".utf8))
            exit(2)
        }
    }
}
