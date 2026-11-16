import Foundation
import MoshpitFFI
import MoshpitKit

/// What the NSH screens leave for the diagnostics log (B366).
///
/// Canary 116 showed a page with its bar and nothing under it, and a restart
/// fixed it; nothing could say why. Every reason the core knew went to
/// stderr, which a phone keeps nowhere, and the Recent Log reads only this
/// process's log, which the restart took with it. So:
///
/// - the core's warnings and errors (a post the screen refused, a body that
///   panicked, an engine that would not start) come here through
///   `mp_ui_set_log_handler` (``forwardRustLog()``) and go to `Log.ui`;
/// - the app's own NSH failures go through ``error(_:)`` too;
/// - and every such line is also kept in a small ring in the App Group
///   (``NSHTrail``), which the Recent Log merges for the launches before
///   this one (``DiagnosticsLog/recent()``).
enum NSHDiagnostics {
    /// What the Recent Log labels a line from an earlier launch with.
    static let source = "ui·earlier run"

    /// How far back the Recent Log reaches into earlier launches: a page
    /// that went blank, the app restarted, the log read later that day.
    static let earlierWindow: TimeInterval = 24 * 60 * 60

    /// The ring in the App Group (none without the group: a test host).
    static let trail = NSHTrail(url: FileManager.default
        .containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
        .appendingPathComponent("nsh-diagnostics.json"))

    /// An NSH failure: to `Log.ui` as an error, and to the trail. Any thread.
    static func error(_ text: String) {
        Log.ui.error("\(text, privacy: .public)")
        trail.record(text)
    }

    /// The line a forwarded record of the core makes: its level (1 an
    /// error, 2 a warning), target and message.
    static func rustLine(level: UInt8, target: String, message: String) -> String {
        let word = level <= 1 ? "error" : "warning"
        return "core [\(target)] \(word): \(message)"
    }

    /// Has the core hand its warnings and errors to ``error(_:)`` from now
    /// on. Once per process (later calls do nothing); before the first
    /// surface, which is when the core's logger starts.
    static func forwardRustLog() {
        _ = forwarding
    }

    /// Makes the trail's file when it isn't there yet, once a process, and
    /// says in the log when it can't (B445): a phone without the file read
    /// as a broken trail, when nothing had failed yet to write one.
    static func prepareTrail() {
        _ = preparing
    }

    private static let preparing: Void = {
        switch trail.prepare() {
        case .ready:
            break
        case .noContainer:
            Log.ui.notice("nsh trail: no App Group container, so earlier runs' failures are not kept")
        case .failed(let why):
            Log.ui.notice("nsh trail: can't make the file: \(why, privacy: .public)")
        }
    }()

    private static let forwarding: Void = {
        mp_ui_set_log_handler({ _, level, target, message in
            NSHDiagnostics.error(NSHDiagnostics.rustLine(
                level: level, target: String(cString: target), message: String(cString: message)))
        }, nil)
    }()
}

/// A bounded ring of NSH failure lines in a file, each stamped with the
/// launch that wrote it, so a later launch can show the ones the restart
/// took out of the system's log. The way the push extension's diagnostics ring keeps the
/// extension's lines: plain JSON, read-modify-write, silent on failure (a
/// diagnostic must never be what breaks a page). Thread-safe.
final class NSHTrail: @unchecked Sendable {
    struct Line: Codable, Equatable {
        let at: Date
        /// The launch that wrote it (``NSHTrail/launch``).
        let launch: String
        let text: String
    }

    /// This process's launch: a line it wrote is in the system's log too.
    static let thisLaunch = UUID().uuidString

    let url: URL?
    let capacity: Int
    let launch: String
    private let lock = NSLock()

    init(url: URL?, capacity: Int = 120, launch: String = NSHTrail.thisLaunch) {
        self.url = url
        self.capacity = capacity
        self.launch = launch
    }

    private static let encoder: JSONEncoder = {
        let e = JSONEncoder(); e.dateEncodingStrategy = .iso8601
        return e
    }()

    private static let decoder: JSONDecoder = {
        let d = JSONDecoder(); d.dateDecodingStrategy = .iso8601
        return d
    }()

    /// Appends `text`, the oldest line dropped past ``capacity``.
    func record(_ text: String, now: Date = Date()) {
        guard let url else { return }
        lock.lock()
        defer { lock.unlock() }
        var lines = unlockedRead()
        lines.append(Line(at: now, launch: launch, text: text))
        if lines.count > capacity { lines.removeFirst(lines.count - capacity) }
        guard let data = try? Self.encoder.encode(lines) else { return }
        // Readable after the first unlock, as the push trail is: a launch
        // before it writes nothing (the line is in `Log.ui` regardless).
        do {
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            guard !writeFailureSaid else { return }
            writeFailureSaid = true
            Log.ui.notice("nsh trail: can't write: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// A failed write was said (once a process: the trail is never what
    /// floods the log).
    private var writeFailureSaid = false

    /// What ``prepare()`` found.
    enum Readiness: Equatable {
        /// No App Group (a test host): nothing is kept.
        case noContainer
        /// The file is there (or made now, empty).
        case ready
        /// It could not be made: why.
        case failed(String)
    }

    /// Makes the file, an empty ring, when it isn't there yet: then a phone
    /// without it is one that could not write it, not one where nothing
    /// failed (B445).
    @discardableResult
    func prepare() -> Readiness {
        guard let url else { return .noContainer }
        lock.lock()
        defer { lock.unlock() }
        if FileManager.default.fileExists(atPath: url.path) { return .ready }
        do {
            try Data("[]".utf8).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            return .ready
        } catch {
            return .failed(error.localizedDescription)
        }
    }

    /// Every line kept, oldest first.
    func read() -> [Line] {
        lock.lock()
        defer { lock.unlock() }
        return unlockedRead()
    }

    /// The lines earlier launches wrote since `since`: what the system's
    /// log of this process cannot show.
    func earlier(since: Date) -> [Line] {
        read().filter { $0.launch != launch && $0.at >= since }
    }

    private func unlockedRead() -> [Line] {
        guard let url, let data = try? Data(contentsOf: url),
              let lines = try? Self.decoder.decode([Line].self, from: data)
        else { return [] }
        return lines
    }
}

/// Which refused posts a surface has logged: the first of each op only, so
/// a page posting the same refused state on every change says it once.
struct NSHPostLog {
    private var logged: Set<String> = []

    /// Whether this is the first refusal of `op` (and from now on not).
    mutating func first(_ op: String) -> Bool {
        logged.insert(op).inserted
    }

    /// A message's op, `?` without one.
    static func op(of message: [String: Any]) -> String {
        message["op"] as? String ?? "?"
    }

    /// The op of a message written as JSON, `?` when it has none.
    static func op(ofJSON json: String) -> String {
        guard let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        else { return "?" }
        return op(of: object)
    }

    /// The keys of `message` whose values JSON can't write (a NaN, an
    /// infinity, a Date, a Swift value Foundation doesn't bridge…), sorted:
    /// what a not-JSON line names. A nil Optional is not one of them: boxed
    /// in `Any` it bridges to `NSNull`, which JSON writes as null.
    static func unwritable(_ message: [String: Any]) -> [String] {
        message.keys.sorted().filter { key in
            guard let value = message[key] else { return false }
            return !JSONSerialization.isValidJSONObject([value])
        }
    }
}

/// What a screen's error boundary tells the host (`ui/src/boundary.rs`).
enum NSHBoundaryEvent: Equatable {
    /// The body failed: which component, and its panic's message.
    case failed(component: String, message: String)
    /// Retry was pressed under it.
    case retry

    /// The event of `message`, nil for any other.
    init?(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "failed":
            self = .failed(component: message["component"] as? String ?? "?",
                           message: message["message"] as? String ?? "")
        case "retry":
            self = .retry
        default:
            return nil
        }
    }
}

/// A look at a screen a while after it ran (B366): whether it listens,
/// whether the page posted it anything, whether it applied what came. Each
/// way a page ends up as its bar alone reads differently here.
enum NSHWatch {
    /// How long after a screen runs it is looked at.
    static let delay: TimeInterval = 3

    /// The channel's counts (`mp_ui_surface_counts`).
    struct Counts: Equatable {
        var heard: Bool
        var posted: UInt64
        var applied: UInt64
        var lost: UInt64
    }

    enum Verdict: Equatable {
        /// Listening, told something, applied all of it.
        case healthy
        /// Its view never took its inbox: the engine never started, or the
        /// surface was never laid out.
        case neverListened
        /// It listens, but the page posted it nothing.
        case nothingPosted
        /// Posts found its view gone: its task ended (a panic ends it).
        case stoppedListening
        /// Posted more than it applied.
        case behind
    }

    static func verdict(_ counts: Counts) -> Verdict {
        if !counts.heard { return .neverListened }
        if counts.lost > 0 { return .stoppedListening }
        if counts.posted == 0 { return .nothingPosted }
        if counts.applied < counts.posted { return .behind }
        return .healthy
    }

    /// The log line for `screen` at `counts`.
    static func line(screen: String, counts: Counts) -> String {
        let what: String = switch verdict(counts) {
        case .healthy: "fine"
        case .neverListened: "never started listening"
        case .nothingPosted: "the page posted it nothing"
        case .stoppedListening: "stopped listening"
        case .behind: "has not applied what it was posted"
        }
        return "NSH \(screen): \(what) \(Int(delay)) s after it ran"
            + " (heard \(counts.heard), posted \(counts.posted), applied \(counts.applied), lost \(counts.lost))"
    }

    /// Looks at `surface`'s screen ``delay`` after it ran: a screen that
    /// isn't fine is logged as an error.
    @MainActor
    static func watch(_ surface: NSHSurface) {
        guard let screen = surface.screenName else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak surface] in
            MainActor.assumeIsolated {
                guard let surface, surface.screenName == screen, let counts = surface.counts else { return }
                let line = Self.line(screen: screen, counts: counts)
                if verdict(counts) == .healthy {
                    Log.ui.info("\(line, privacy: .public)")
                } else {
                    NSHDiagnostics.error(line)
                }
            }
        }
    }
}
