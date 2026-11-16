import Foundation
import MoshpitKit

/// Which path candidates on one terminal the host has confirmed, and where
/// they point. One per terminal (a tmux pane has its own working directory),
/// fed by the session's probe (`PathProbe` over the file-transfer SSH).
///
/// Lookups are synchronous and never block the link pass: an unknown
/// candidate answers nil, joins the queue, and a debounced batch asks the
/// host about everything queued in one exec round trip. When answers land,
/// `onResolved` asks for another pass, which then finds them in the cache.
///
/// Misses are remembered for a while, not forever: agents often name a file
/// a moment before writing it ("I'll save the chart to out/a.png").
///
/// Fix (B817): a probe that got no answer (no connection yet, a session
/// still coming back) was kept as a miss, and neither kind was asked about
/// again unless something new was printed after its window: the link pass
/// runs on output, an answer, or the pane coming on screen, and a quiet
/// screen has none. A path printed while the connection came back stayed
/// plain text for good. No answer is not a miss now — it is asked again a
/// little later, the wait doubling while the host stays out of reach — and
/// at each retry the pass is asked for (`onResolved`) whether or not
/// anything was printed.
@MainActor
final class RemotePathLinks {
    typealias Probe = (_ cwd: String?, _ batch: [TerminalPathScanner.Candidate]) async -> [Int: PathProbe.Hit]?

    /// Called after a batch confirmed something, or after the working
    /// directory moved (every relative answer is stale then).
    var onResolved: (() -> Void)?

    /// A miss is asked again after this long: the agent may name a file
    /// before it writes it. Not sooner — every relative miss costs the host
    /// a `find`.
    static let missRetry: TimeInterval = 30
    /// The first wait after a probe that got no answer; it doubles each time
    /// in a row, up to ``missRetry``.
    static let noAnswerRetry: TimeInterval = 2
    static let workingDirectoryTTL: TimeInterval = 10
    static let batchLimit = 48
    /// A full screen of agent output reaches the link pass in several
    /// bursts; one probe for the lot beats one per burst.
    static let debounce: Duration = .milliseconds(250)
    /// The cache is per terminal and its keys are short strings; past this
    /// many it is simply started over.
    static let entryLimit = 4000

    private enum Entry {
        case hit(PathProbe.Hit)
        case miss(Date)
        /// Asked, and the host gave no answer: asked again from this time.
        case unanswered(Date)
        case pending
    }

    private let workingDirectory: () async -> String?
    private let probe: Probe
    private let now: () -> Date
    private let retryAfterMiss: TimeInterval
    private let retryAfterNoAnswer: TimeInterval
    /// Probes in a row that got no answer: each doubles the wait.
    private var unanswered = 0
    /// The pass asked for at a retry, and when it is due.
    private var repass: Task<Void, Never>?
    private var repassDue: Date?

    private var entries: [String: Entry] = [:]
    private var queued: [TerminalPathScanner.Candidate] = []
    private var drainTask: Task<Void, Never>?
    private var cwd: String?
    private var cwdFetchedAt: Date?

    init(workingDirectory: @escaping () async -> String?,
         probe: @escaping Probe,
         now: @escaping () -> Date = Date.init,
         retryAfterMiss: TimeInterval = RemotePathLinks.missRetry,
         retryAfterNoAnswer: TimeInterval = RemotePathLinks.noAnswerRetry) {
        self.workingDirectory = workingDirectory
        self.probe = probe
        self.now = now
        self.retryAfterMiss = retryAfterMiss
        self.retryAfterNoAnswer = retryAfterNoAnswer
    }

    /// The confirmed host path for `candidate`, or nil. Asking about one the
    /// host hasn't been asked about yet is what queues it.
    func resolvedPath(for candidate: TerminalPathScanner.Candidate) -> String? {
        let key = Self.key(cwd: cwd, candidate)
        switch entries[key] {
        case .hit(let hit)?:
            return hit.path
        case .pending?:
            return nil
        case .miss(let at)?:
            guard now().timeIntervalSince(at) >= retryAfterMiss else { return nil }
        case .unanswered(let retryAt)?:
            guard now() >= retryAt else { return nil }
        case nil:
            break
        }
        if entries.count >= Self.entryLimit { entries.removeAll() }
        entries[key] = .pending
        queued.append(candidate)
        if drainTask == nil {
            drainTask = Task { [weak self] in
                try? await Task.sleep(for: Self.debounce)
                await self?.drain()
            }
        }
        return nil
    }

    /// Forget the working directory, so the next batch asks again — for an
    /// owner that just learned it changed (OSC 7, a pane switch).
    func invalidateWorkingDirectory() {
        cwdFetchedAt = nil
    }

    /// The session is live again (B818): what got no answer while it was
    /// away is asked at once, the backoff starts over, the working directory
    /// is asked again, and a pass is asked for now — not up to
    /// ``missRetry`` after the link came back.
    func connectionCameBack() {
        unanswered = 0
        for (key, entry) in entries {
            if case .unanswered = entry { entries[key] = nil }
        }
        cwdFetchedAt = nil
        repass?.cancel()
        repass = nil
        repassDue = nil
        onResolved?()
    }

    private func drain() async {
        var changed = false
        while !queued.isEmpty {
            let batch = Array(queued.prefix(Self.batchLimit))
            queued.removeFirst(batch.count)

            if cwdFetchedAt.map({ now().timeIntervalSince($0) >= Self.workingDirectoryTTL }) ?? true {
                let fresh = await workingDirectory()
                cwdFetchedAt = now()
                if fresh != cwd {
                    // Every relative answer (and every pending key) belonged
                    // to the old directory. The pass `onResolved` triggers
                    // re-asks for whatever is still on screen.
                    cwd = fresh
                    entries.removeAll()
                    changed = true
                }
            }

            let stamp = now()
            guard let hits = await probe(cwd, batch) else {
                Log.input.notice("path probe: no answer for \(batch.count, privacy: .public) path(s); asked again later")
                // No answer (no SSH right now) is not a miss: asked again a
                // little later, never on every pass in between — that wait
                // is what keeps a dead channel from being hit by every link
                // pass — and the pass asked for then (B817).
                unanswered += 1
                let wait = min(retryAfterNoAnswer * pow(2, Double(min(unanswered - 1, 8))), retryAfterMiss)
                let retryAt = stamp.addingTimeInterval(wait)
                for candidate in batch { entries[Self.key(cwd: cwd, candidate)] = .unanswered(retryAt) }
                askForAPass(after: wait)
                continue
            }
            unanswered = 0
            // What a capture's log shows of a link that didn't appear (B817):
            // how many were confirmed, and whether the probe had a directory
            // to look in.
            let directory = cwd == nil ? "unknown" : "known"
            Log.input.notice("path probe: \(hits.count, privacy: .public) of \(batch.count, privacy: .public) confirmed, working directory \(directory, privacy: .public)")
            for (index, candidate) in batch.enumerated() {
                entries[Self.key(cwd: cwd, candidate)] = hits[index].map(Entry.hit) ?? .miss(stamp)
            }
            if !hits.isEmpty { changed = true }
            // A miss is asked about again once its window has passed, with
            // or without new output (B817).
            if hits.count < batch.count { askForAPass(after: retryAfterMiss) }
        }
        drainTask = nil
        if changed { onResolved?() }
    }

    /// A link pass (`onResolved`) `delay` from now, unless one is due
    /// sooner: a retry's, when the screen may have printed nothing since.
    private func askForAPass(after delay: TimeInterval) {
        let due = now().addingTimeInterval(delay)
        if let repassDue, repassDue <= due, repass != nil { return }
        repass?.cancel()
        repassDue = due
        repass = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled, let self else { return }
            self.repass = nil
            self.repassDue = nil
            self.onResolved?()
        }
    }

    private static func key(cwd: String?, _ candidate: TerminalPathScanner.Candidate) -> String {
        (cwd ?? "") + "\u{0}" + (candidate.directoryOnly ? "D" : "F") + candidate.token
    }
}
