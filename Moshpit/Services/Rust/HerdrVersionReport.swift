import Foundation

/// The two ends of a herdr version skew, as the Rust core reports them
/// (crates/herdr's `VersionReport`, read from `herdr status`): the `herdr`
/// the core's commands resolve to (the "client") and the server actually
/// running. Worded here for the terminal banner and the Home row.
struct HerdrVersionReport: Equatable {
    var client: String
    var server: String

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
