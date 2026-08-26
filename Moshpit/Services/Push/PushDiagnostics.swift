import Foundation

/// A small shared ring the notification service extension writes into, so the
/// in-app diagnostics screen can show what the extension did.
///
/// It exists because that screen reads `OSLogStore(scope: .currentProcessIdentifier)`,
/// which is structurally blind to the extension — a separate process. The two
/// lines that matter most in this whole feature live there and nowhere else:
/// "opened a <state> push from <host>", and "no key opened this envelope (N
/// available)", whose N is the one thing that separates "the extension could not
/// reach the pairing store" from "the sending host is paired to a different
/// install". iOS gives an app no way to read another process's log, so the
/// extension has to leave its own trail.
///
/// The cost of not having this was paid immediately: a reviewer looking for those
/// lines in the app's log was sent hunting for something that could never appear
/// there, and drew a wrong conclusion from their absence. A self-diagnosis tool
/// that cannot see the riskiest component is worse than none, for the same reason
/// `lastError` was worse than nothing while it had no reader.
///
/// It is deliberately NOT a general logging backend. Four call sites, a bounded
/// ring, plain text. Everything else still goes to `Log.push`, which the app's
/// own process surfaces already.
enum PushDiagnostics {

    /// Enough to cover a few pushes and their retries; small enough that the
    /// read-modify-write costs nothing inside an extension's time budget.
    static let capacity = 60

    /// What the diagnostics screen labels these with.
    static let source = "push·extension"

    struct Line: Codable, Equatable {
        let at: Date
        let text: String
    }

    /// Its own coder, because the install engine's shared one lives in a file
    /// the extension target does not compile — and this type has to work in both
    /// processes or it is pointless.
    private static let coder: (encoder: JSONEncoder, decoder: JSONDecoder) = {
        let e = JSONEncoder(); e.dateEncodingStrategy = .iso8601
        let d = JSONDecoder(); d.dateDecodingStrategy = .iso8601
        return (e, d)
    }()

    private static var url: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: PushPairingStore.appGroup)?
            .appendingPathComponent("push-diagnostics.json")
    }

    /// Append one line, oldest dropped past `capacity`.
    ///
    /// Silent on failure by design — this is the diagnostic channel, and a
    /// diagnostic that can itself derail a notification is worse than a missing
    /// line. `Log.push` still gets everything; this is only the copy that can
    /// cross a process boundary.
    ///
    /// A concurrent write from the app and the extension can lose a line, since
    /// this is a read-modify-write with no lock. Accepted: the alternative is
    /// coordinated file access inside a path that must never block a push, and
    /// losing one line of a ring buffer costs a lot less than that.
    static func record(_ text: String, now: Date = Date()) {
        guard let url else { return }
        var lines = read()
        lines.append(Line(at: now, text: text))
        if lines.count > capacity { lines.removeFirst(lines.count - capacity) }
        guard let data = try? Self.coder.encoder.encode(lines) else { return }
        try? data.write(to: url, options: [.atomic,
                                           .completeFileProtectionUntilFirstUserAuthentication])
    }

    /// Everything recorded, oldest first.
    static func read() -> [Line] {
        guard let url, let data = try? Data(contentsOf: url),
              let lines = try? Self.coder.decoder.decode([Line].self, from: data)
        else { return [] }
        return lines
    }

    /// Lines newer than `since`, for merging into the diagnostics screen's own
    /// window.
    static func recent(since: Date) -> [Line] {
        read().filter { $0.at >= since }
    }

    static func clear() {
        guard let url else { return }
        try? FileManager.default.removeItem(at: url)
    }
}
