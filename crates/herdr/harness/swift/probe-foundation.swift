// Prints the Foundation behaviour the herdr port reproduces (or documents as
// a divergence), measured on whatever macOS runs it. The rules in
// src/json.rs, src/base64.rs and src/text.rs, and the tests in tests/json.rs,
// were written from this output (macOS 27.0.1).
//
//   swiftc -O -o /tmp/probe-foundation harness/swift/probe-foundation.swift && /tmp/probe-foundation

import Foundation

func show(_ text: String) -> String {
    text.unicodeScalars.map { $0.value < 0x20 || $0.value >= 0x7F ? String(format: "\\u{%X}", $0.value) : String($0) }
        .joined()
}

func number(_ value: Any?) -> String {
    if value is [String: Any] { return "{…}" }
    if value is [Any] { return "[…]" }
    guard let number = value as? NSNumber else { return "\(String(describing: value))" }
    let kind = CFGetTypeID(number) == CFBooleanGetTypeID() ? "bool"
        : number is NSDecimalNumber ? "dec" : String(cString: number.objCType)
    return "\(kind) int=\((number as? Int).map(String.init) ?? "-") u64=\((number as? UInt64).map(String.init) ?? "-") "
        + "bool=\((number as? Bool).map(String.init) ?? "-")"
}

func json(_ bytes: [UInt8]) -> String {
    guard let object = try? JSONSerialization.jsonObject(with: Data(bytes)) else { return "refused" }
    if let dict = object as? [String: Any] {
        return "{" + dict.keys.sorted().map { "\(show($0)):\(dict[$0] is String ? show(dict[$0] as! String) : number(dict[$0]))" }
            .joined(separator: ",") + "}"
    }
    return "array(\((object as? [Any])?.count ?? -1))"
}

print("== JSONSerialization: acceptance")
for text in ["\"x\"", "1", " {} ", "{} x", "\u{C}{}", "{\"a\":1,}", "[1,]", "[1,,]", "{,}", "{\"a\":1,\"a\":2}",
             "{\"a\":\"\u{1}\"}", "{\"a\":\"\\ud800\"}", "{\"a\":\"\\ud800\\u0041\"}", "{\"a\":\"\\U0041\"}",
             "{\"a\":\"\u{FEFF}x\"}", "{\"\\ufeffa\":1}", "{\"a\":01}", "{\"a\":1.}", "{\"a\":+1}"] {
    print("\(show(text)) => \(json(Array(text.utf8)))")
}
print("== JSONSerialization: depth (a value inside N containers)")
for depth in [512, 513] {
    print("arrays \(depth + 1) (innermost empty) => \(json(Array((String(repeating: "[", count: depth + 1) + String(repeating: "]", count: depth + 1)).utf8)))")
    print("objects \(depth) holding 1 => \(json(Array((String(repeating: "{\"a\":", count: depth) + "1" + String(repeating: "}", count: depth)).utf8)))")
}
print("== JSONSerialization: encoding sniff")
print("UTF-16LE {\"a\":1} => \(json(Array("{\"a\":1}".utf16).flatMap { [UInt8($0 & 0xFF), UInt8($0 >> 8)] }))")
print("== Numbers and casts")
for literal in ["0", "-0", "-0.0", "1", "2", "1.0", "1.5", "2e0", "1e19", "1e20", "9223372036854775808",
                "18446744073709551615", "18446744073709551616", "0.12345678901234567", "0.123456789012345678",
                "100000000000000000.5", "0.99999999999999999999999999999999999999999", "1e400", "-1e400",
                "1e0001", "1e+0001", "1e-400", "true"] {
    let object = try? JSONSerialization.jsonObject(with: Data("{\"v\":\(literal)}".utf8)) as? [String: Any]
    print("\(literal) => \(object.map { number($0["v"]) } ?? "refused")")
}
print("== Data(base64Encoded:)")
for text in ["aGk=", "aGk", "aGk==", "aGl=", "YQ=", "====", "AAAAAA=Q", "aG k=", ""] {
    print("\(text) => \(Data(base64Encoded: text).map { $0.map { String(format: "%02x", $0) }.joined() } ?? "nil")")
}
print("== Character sets")
for (name, set) in [("whitespaces", CharacterSet.whitespaces), ("newlines", .newlines)] {
    let members = [0x9, 0xA, 0xB, 0xC, 0xD, 0x20, 0x85, 0xA0, 0x1680, 0x180E, 0x2000, 0x200A, 0x200B, 0x2028,
                   0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF].filter { set.contains(Unicode.Scalar($0)!) }
    print("\(name): \(members.map { String(format: "%X", $0) }.joined(separator: " "))")
}
print("== Strings")
print("String(bytes: BOM hi) => \(String(bytes: [0xEF, 0xBB, 0xBF, 0x68, 0x69], encoding: .utf8).map(show) ?? "nil")")
print("String(decoding: ED A0 80 68) => \(show(String(decoding: [0xED, 0xA0, 0x80, 0x68], as: UTF8.self)))")
print("\"herdr: \\u{301}x\".hasPrefix(\"herdr: \") => \("herdr: \u{301}x".hasPrefix("herdr: "))")
print("\"a\\r\\nb\".components(separatedBy: \"\\n\") => \("a\r\nb".components(separatedBy: "\n").map(show))")
print("\"a;\\u{301}b\".components(separatedBy: \";\") => \("a;\u{301}b".components(separatedBy: ";").map(show))")
print("\"a\\u{37E}b\".components(separatedBy: \";\") => \("a\u{37E}b".components(separatedBy: ";").map(show))")
print("\"x:p\\u{301}:p3\".range(of: \":p\") after => \(("x:p\u{301}:p3".range(of: ":p")).map { show(String("x:p\u{301}:p3"[$0.upperBound...])) } ?? "nil")")
print("Int(\"1\\u{663}\") => \(String(describing: Int("1\u{663}")))  \"1\\u{663}\".prefix(isNumber) => \(show(String("1\u{663}".prefix { $0.isNumber })))")
print("== Quoting (HerdrLaunch.quote's shape) through /bin/sh")
for value in ["it's", "x'\u{301}; echo INJECTED; '"] {
    let quoted = "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sh")
    process.arguments = ["-c", "printf '[%s]' " + quoted]
    let pipe = Pipe()
    process.standardOutput = pipe
    try! process.run()
    process.waitUntilExit()
    print("\(show(value)) => \(show(String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)))")
}
