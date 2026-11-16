import Foundation

enum HerdrSnapshot {
    static func clientIsOlder(_ message: String) -> Bool {
        message.contains("older than server")
    }
}

struct HerdrVersionReport: Equatable {
    var client: String
    var server: String

    /// Parse `herdr status`. The shape both 0.8.0 and 0.9.5 print:
    ///
    ///     client:
    ///       version: 0.8.0
    ///       …
    ///     server:
    ///       status: running
    ///       version: 0.9.5
    ///
    /// nil unless BOTH versions are there — a stopped server has none, and an
    /// older herdr without `status` prints something else entirely.
    static func parse(_ output: String) -> HerdrVersionReport? {
        var section = ""
        var versions: [String: String] = [:]
        for raw in output.components(separatedBy: .newlines) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty else { continue }
            // Section headers sit at column 0; their fields are indented.
            if raw.first?.isWhitespace == false {
                section = line.hasSuffix(":") ? String(line.dropLast()) : ""
                continue
            }
            if line.hasPrefix("version:"), versions[section] == nil {
                let value = line.dropFirst("version:".count).trimmingCharacters(in: .whitespaces)
                if !value.isEmpty { versions[section] = value }
            }
        }
        guard let client = versions["client"], let server = versions["server"] else { return nil }
        return HerdrVersionReport(client: client, server: server)
    }

    var mismatched: Bool { Self.compare(client, server) != .orderedSame }

    /// Restarting the server with the client would DOWNGRADE it.
    var clientIsOlder: Bool { Self.compare(client, server) == .orderedAscending }

    /// The skew in one banner-sized line. Which side is older decides the
    /// remedy it names: a stale copy on PATH needs updating on the host; a
    /// stale server is what the restart button (offered alongside) fixes.
    var notice: String {
        clientIsOlder
            ? String(localized: "herdr on the host's PATH is \(client), its server \(server) — update that copy")
            : String(localized: "The host's herdr server is \(server), older than its herdr \(client)")
    }

    /// Dotted-number order: "0.10.0" > "0.9.5", and a pre-release suffix
    /// ("0.9.5-preview") compares by its numbers alone.
    static func compare(_ a: String, _ b: String) -> ComparisonResult {
        func parts(_ v: String) -> [Int] {
            v.split(separator: ".").map { Int($0.prefix { $0.isNumber }) ?? 0 }
        }
        let (x, y) = (parts(a), parts(b))
        for i in 0..<max(x.count, y.count) {
            let (l, r) = (i < x.count ? x[i] : 0, i < y.count ? y[i] : 0)
            if l != r { return l < r ? .orderedAscending : .orderedDescending }
        }
        return .orderedSame
    }
}
