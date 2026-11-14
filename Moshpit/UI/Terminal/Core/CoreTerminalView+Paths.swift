import MoshpitFFI
import UIKit

/// Paths an agent prints, as links the host confirmed (TERMINAL-VIEW.md
/// V43), on the core: the core picks the candidates on screen
/// (crates/term/src/paths.rs, the rules of the app's `TerminalPathScanner`),
/// the session's ``RemotePathLinks`` asks the host about them, and the ones
/// it confirmed go back to the core, which draws them as links on exactly
/// their own cells and opens them on a tap (`moshpit-preview://`, to
/// ``SwiftTerminalView/Coordinator/openLink(_:)``).
///
/// The pass runs when SwiftTerm's does: 0.15 s after output goes quiet, at
/// once when the host answers, when the pane comes on screen; only for a
/// terminal on screen while the app is active — every new candidate costs
/// the host a probe, and a tmux session's hidden panes keep printing.
extension CoreTerminalView {
    /// The session gave (or took) the terminal's resolver.
    func pathLinksChanged() {
        guard let term else { return }
        let kinds = pathLinks == nil ? "" : PreviewableFile.extensions.sorted().joined(separator: ",")
        kinds.withCString { mp_term_set_path_extensions(term, $0) }
        pathLinks?.onResolved = { [weak self] in self?.schedulePathPass(after: 0) }
        if pathLinks == nil {
            pathPass?.cancel()
            pathPass = nil
            takeFrame()
        } else {
            schedulePathPass(after: 0)
        }
    }

    func schedulePathPass(after delay: TimeInterval = 0.15) {
        guard pathLinks != nil else { return }
        pathPass?.cancel()
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { self?.runPathPass() }
        }
        pathPass = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    /// The candidates on screen to the resolver (an unknown one is queued
    /// for the host), the confirmed ones to the core.
    func runPathPass() {
        pathPass = nil
        guard let term, let links = pathLinks, window != nil,
              UIApplication.shared.applicationState != .background else { return }
        var table = ""
        for candidate in Self.pathCandidates(mp_term_path_candidates(term)) {
            guard let path = links.resolvedPath(for: candidate) else { continue }
            table += (candidate.directoryOnly ? "D" : "F") + "\t" + candidate.token + "\t"
                + RemotePathLink.url(for: path) + "\n"
        }
        guard table.withCString({ mp_term_set_path_links(term, $0) }) else { return }
        pathPassFraming = true
        takeFrame()
        pathPassFraming = false
    }

    /// The core's candidate lines ("F|D<tab>token"), the string freed.
    static func pathCandidates(_ lines: UnsafeMutablePointer<CChar>?) -> [TerminalPathScanner.Candidate] {
        guard let lines else { return [] }
        defer { mp_string_free(lines) }
        return String(cString: lines).split(separator: "\n").compactMap { line in
            let parts = line.split(separator: "\t", maxSplits: 1)
            guard parts.count == 2, parts[0] == "F" || parts[0] == "D" else { return nil }
            let token = String(parts[1])
            return TerminalPathScanner.Candidate(token: token, offset: 0, length: token.count,
                                                 directoryOnly: parts[0] == "D")
        }
    }
}
