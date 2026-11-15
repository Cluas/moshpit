// Helpers of the store harness's Swift front end (main.swift, over the
// app's sources as 1.0.8 shipped them): the tokens, the minted-id masking,
// snapshots in and out, a lockable keychain. The case format is in
// src/harness.rs.

import Foundation

// MARK: - Tokens

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

func unhex(_ text: Substring) -> [UInt8]? {
    let chars = Array(text.utf8)
    guard chars.count % 2 == 0 else { return nil }
    var out = [UInt8]()
    var i = 0
    while i < chars.count {
        guard let hi = hexValue(chars[i]), let lo = hexValue(chars[i + 1]) else { return nil }
        out.append(hi << 4 | lo)
        i += 2
    }
    return out
}

func hexValue(_ c: UInt8) -> UInt8? {
    switch c {
    case 48...57: return c - 48
    case 97...102: return c - 87
    case 65...70: return c - 55
    default: return nil
    }
}

func arg(_ token: Substring) -> String {
    token == "~" ? "" : String(decoding: unhex(token)!, as: UTF8.self)
}

func out(_ text: String) -> String {
    text.isEmpty ? "~" : hex(Array(text.utf8))
}

func bits(_ value: Double) -> String {
    let s = String(value.bitPattern, radix: 16)
    return String(repeating: "0", count: 16 - s.count) + s
}

func flag(_ value: Bool) -> String { value ? "1" : "0" }

func optFlag(_ value: Bool?) -> String { value.map(flag) ?? "-" }

func dateOut(_ date: Date) -> String { bits(date.timeIntervalSinceReferenceDate) }

func utf8Less(_ a: String, _ b: String) -> Bool {
    Array(a.utf8).lexicographicallyPrecedes(Array(b.utf8))
}

// MARK: - Minted ids

/// Every id-shaped run: a UUID (8-4-4-4-12 hex digits, either case), or a
/// minted `custom-xxxxxxxx` / `app-xxxxxxxx`.
func idsIn(_ text: String) -> [String] {
    let bytes = Array(text.utf8)
    var found: [String] = []
    var at = 0
    func isHex(_ b: UInt8) -> Bool { hexValue(b) != nil }
    func lowerHex(_ b: UInt8) -> Bool { (48...57).contains(b) || (97...102).contains(b) }
    while at < bytes.count {
        if at + 36 <= bytes.count, (0..<36).allSatisfy({ i in
            [8, 13, 18, 23].contains(i) ? bytes[at + i] == 45 : isHex(bytes[at + i])
        }) {
            found.append(String(decoding: bytes[at..<at + 36], as: UTF8.self))
            at += 36
            continue
        }
        var matched = false
        for prefix in ["custom-", "app-"] {
            let p = Array(prefix.utf8)
            if at + p.count + 8 <= bytes.count, Array(bytes[at..<at + p.count]) == p,
               bytes[at + p.count..<at + p.count + 8].allSatisfy(lowerHex) {
                found.append(String(decoding: bytes[at..<at + p.count + 8], as: UTF8.self))
                at += p.count + 8
                matched = true
                break
            }
        }
        if !matched { at += 1 }
    }
    return found
}

final class Minted {
    var known: Set<String>
    var seen: [String] = []

    init(_ inputs: [String]) {
        known = Set(inputs.flatMap(idsIn).map { $0.uppercased() })
    }

    func mask(_ text: String) -> String {
        var result = text
        for id in idsIn(text) where !known.contains(id.uppercased()) {
            let index: Int
            if let i = seen.firstIndex(of: id) { index = i } else { seen.append(id); index = seen.count - 1 }
            result = result.replacingOccurrences(of: id, with: "#\(index + 1)", options: .literal)
        }
        return result
    }

    func out(_ text: String) -> String { Harness_out(mask(text)) }
    func opt(_ text: String?) -> String { text.map { self.out($0) } ?? "-" }
}

func Harness_out(_ text: String) -> String { out(text) }

// MARK: - Snapshots

func plistFromJSON(_ value: Any) -> Any {
    let pair = value as! [Any]
    let tag = pair[0] as! String
    switch tag {
    case "b": return (pair[1] as! NSNumber).boolValue
    case "i": return NSNumber(value: Int64(pair[1] as! String)!)
    case "f": return NSNumber(value: Double(bitPattern: UInt64(pair[1] as! String, radix: 16)!))
    case "t": return Date(timeIntervalSinceReferenceDate: Double(bitPattern: UInt64(pair[1] as! String, radix: 16)!))
    case "s": return arg(Substring(pair[1] as! String))
    case "x":
        let text = pair[1] as! String
        return text == "~" ? Data() : Data(unhex(Substring(text))!)
    case "a": return (pair[1] as! [Any]).map(plistFromJSON)
    case "o":
        let dict = NSMutableDictionary()
        for member in pair[1] as! [[Any]] {
            dict[arg(Substring(member[0] as! String))] = plistFromJSON(member[1])
        }
        return dict
    default: fatalError("no snapshot value \(tag)")
    }
}

func plistToJSON(_ value: Any) -> Any {
    if let data = value as? Data { return ["x", data.isEmpty ? "~" : hex([UInt8](data))] }
    if let string = value as? String { return ["s", out(string)] }
    if let date = value as? Date { return ["t", bits(date.timeIntervalSinceReferenceDate)] }
    if let array = value as? [Any] { return ["a", array.map(plistToJSON)] }
    if let dict = value as? NSDictionary {
        return ["o", dict.allKeys.map { key in [out(key as! String), plistToJSON(dict[key]!)] }]
    }
    if let number = value as? NSNumber {
        if CFGetTypeID(number) == CFBooleanGetTypeID() { return ["b", number.boolValue] }
        if CFNumberIsFloatType(number) { return ["f", bits(number.doubleValue)] }
        return ["i", String(number.int64Value)]
    }
    fatalError("no plist value \(type(of: value))")
}

struct Snapshot {
    var defaults: [(String, Any)] = []
    var keychain: [String: Data] = [:]
}

func parseSnapshot(_ token: Substring) -> Snapshot {
    var snapshot = Snapshot()
    guard token != "~" else { return snapshot }
    let object = try! JSONSerialization.jsonObject(with: Data(unhex(token)!)) as! [String: Any]
    for pair in object["d"] as? [[Any]] ?? [] {
        snapshot.defaults.append((pair[0] as! String, plistFromJSON(pair[1])))
    }
    for pair in object["k"] as? [[Any]] ?? [] {
        let text = pair[1] as! String
        snapshot.keychain[pair[0] as! String] = text == "~" ? Data() : Data(unhex(Substring(text))!)
    }
    return snapshot
}

func snapshotTexts(_ snapshot: Snapshot) -> [String] {
    var texts: [String] = []
    func walk(_ value: Any) {
        if let data = value as? Data { texts.append(String(decoding: data, as: UTF8.self)); return }
        if let string = value as? String { texts.append(string); return }
        if let array = value as? [Any] { array.forEach(walk); return }
        if let dict = value as? NSDictionary {
            for (k, v) in dict { texts.append(k as! String); walk(v) }
        }
    }
    snapshot.defaults.forEach { walk($0.1) }
    snapshot.keychain.values.forEach { texts.append(String(decoding: $0, as: UTF8.self)) }
    return texts
}

func dataOut(_ data: Data, _ m: Minted) -> String {
    if data.isEmpty { return "~" }
    // `String(data:encoding:)` only checks the bytes: it would drop a leading
    // byte order mark, so the text itself is decoded as is.
    if String(data: data, encoding: .utf8) != nil {
        return hex(Array(m.mask(String(decoding: data, as: UTF8.self)).utf8))
    }
    return hex([UInt8](data))
}

/// A number as a snapshot prints it: by value (`UserDefaults` may keep an
/// equal value of another kind — a stored 0 over a set `false`), a double
/// with a fraction by its bits.
func numberOut(_ number: NSNumber) -> String {
    if CFGetTypeID(number) == CFBooleanGetTypeID() { return number.boolValue ? "n1" : "n0" }
    if CFNumberIsFloatType(number) {
        let d = number.doubleValue
        if d.rounded() == d && abs(d) < 9_223_372_036_854_775_808.0 { return "n\(Int64(d))" }
        return "f" + bits(d)
    }
    return "n\(number.int64Value)"
}

func plistOut(_ value: Any, _ m: Minted) -> String {
    if let data = value as? Data { return "x" + dataOut(data, m) }
    if let string = value as? String { return "s" + m.out(string) }
    if let date = value as? Date { return "t" + bits(date.timeIntervalSinceReferenceDate) }
    if let array = value as? [Any] { return "a[" + array.map { plistOut($0, m) }.joined(separator: ",") + "]" }
    if let dict = value as? NSDictionary {
        let keys = (dict.allKeys as! [String]).sorted(by: utf8Less)
        return "o{" + keys.map { out($0) + "=" + plistOut(dict[$0]!, m) }.joined(separator: ",") + "}"
    }
    if let number = value as? NSNumber { return numberOut(number) }
    fatalError("no plist value \(type(of: value))")
}

final class Suite {
    let name: String
    let defaults: UserDefaults

    init(_ n: Int) {
        name = "w5.store.\(ProcessInfo.processInfo.processIdentifier).\(n)"
        defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
    }

    var contents: [String: Any] { defaults.persistentDomain(forName: name) ?? [:] }

    func snap(_ m: Minted) -> String {
        let contents = self.contents
        let keys = contents.keys.sorted(by: utf8Less)
        return "snap{" + keys.map { out($0) + "=" + plistOut(contents[$0]!, m) }.joined(separator: ",") + "}"
    }

    func token(_ keychain: [String: Data]) -> String {
        let contents = self.contents
        if contents.isEmpty && keychain.isEmpty { return "~" }
        let d = contents.keys.sorted(by: utf8Less).map { [$0, plistToJSON(contents[$0]!)] as [Any] }
        let k = keychain.keys.sorted(by: utf8Less).map { [$0, keychain[$0]!.isEmpty ? "~" : hex([UInt8](keychain[$0]!))] as [Any] }
        let data = try! JSONSerialization.data(withJSONObject: ["d": d, "k": k])
        return hex([UInt8](data))
    }

    /// Remove the suite, and the empty plist cfprefsd leaves behind.
    func remove() {
        defaults.removePersistentDomain(forName: name)
        let file = URL(fileURLWithPath: NSHomeDirectory())
            .appendingPathComponent("Library/Preferences/\(name).plist")
        try? FileManager.default.removeItem(at: file)
    }
}

// MARK: - The keychain

final class TestKeychain: KeychainBackend, @unchecked Sendable {
    var items: [String: Data]
    var locked = false
    init(_ items: [String: Data]) { self.items = items }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        if locked { throw KeychainError.interactionNotAllowed }
        items[account] = secret
    }
    func load(account: String, prompt: String) throws -> Data {
        if locked { throw KeychainError.interactionNotAllowed }
        guard let data = items[account] else { throw KeychainError.notFound }
        return data
    }
    func delete(account: String) throws { items.removeValue(forKey: account) }
    func exists(account: String) -> Bool { items[account] != nil }
}

func kcOut(_ keychain: TestKeychain, _ m: Minted) -> String {
    "kc{" + keychain.items.keys.sorted(by: utf8Less).map { out($0) + "=x" + dataOut(keychain.items[$0]!, m) }
        .joined(separator: ",") + "}"
}
