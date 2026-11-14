#if DEBUG
import Foundation
import MoshpitKit
import QuartzCore

/// DEBUG: W14b's device rig. `-MOSHPIT_SIDE_PROBE <dir>` (with a seeded
/// connection, ``RustSessionSeed``): once the terminal on screen is live,
/// runs what the features beside it do, through the session's own entry
/// points — the path probe, a preview's stat and download, a stream's first
/// MiB, a folder listing, an image upload, the maintenance pass's read of
/// the host — `-MOSHPIT_SIDE_ROUNDS n` times (5), and logs each step's time
/// as `[w14b] …` lines on stderr (`devicectl device process launch
/// --console`) and in the log. `<dir>` holds `chart.png` (1 MiB) and
/// `clip.mp4` (4 MiB) on the host.
///
/// With `-MOSHPIT_SIDE_BORROW 0` the same calls take the Swift dial W14b
/// replaced (``SessionHub/ActiveSession/lendingRust``): sign-ins counted in
/// the private sshd's log, before against after, on one build. Nothing is
/// written on the host but the uploads (into `~/.moshpit/uploads`), nothing
/// on the phone outlives the launch but the preview's temporary copies.
@MainActor
enum RustSideProbe {
    private static func say(_ line: String) {
        FileHandle.standardError.write(Data("[w14b] \(line)\n".utf8))
        Log.ssh.notice("w14b: \(line, privacy: .public)")
    }

    private static func arg(_ flag: String) -> String? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
        return args[i + 1]
    }

    private static var probing = false

    /// The terminal on screen changed: probe its side features once it is
    /// live, if asked.
    static func sessionBecameVisible(_ session: SessionHub.ActiveSession?) {
        guard let session, !probing, let dir = arg("-MOSHPIT_SIDE_PROBE") else { return }
        probing = true
        let rounds = arg("-MOSHPIT_SIDE_ROUNDS").flatMap(Int.init) ?? 5
        Task { @MainActor [weak session] in
            for _ in 0..<120 {
                guard let session else { return }
                let tmuxReady = session.tmuxController.map { $0.snapshot.isAttached } ?? true
                if session.viewModel.connState == .live, tmuxReady, !session.awaitingFirstFrame { break }
                try? await Task.sleep(for: .milliseconds(500))
            }
            // The connect's own first calls (maintenance, the sweep) go first.
            try? await Task.sleep(for: .seconds(3))
            guard let session else { return }
            await probe(session, dir: dir, rounds: rounds)
        }
    }

    private static func probe(_ session: SessionHub.ActiveSession, dir: String, rounds: Int) async {
        let path = session.rust == nil ? "swift" : (session.lendingRust == nil ? "rust+dial" : "rust+borrow")
        let transport = "\(session.connection.connectionProtocol.rawValue)+\(session.connection.multiplexer.rawValue)"
        say("side probe \(path) \(transport): \(rounds) rounds on \(dir)")
        var times: [String: [Double]] = [:]
        var failures = 0
        let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("w14b-side", isDirectory: true)
        try? FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
        let image = Data((0..<(256 * 1024)).map { UInt8(truncatingIfNeeded: ($0 &* 2654435761) >> 13) })
        for round in 0..<rounds {
            func step(_ name: String, _ work: () async throws -> Void) async {
                let started = CACurrentMediaTime()
                do {
                    try await work()
                    times[name, default: []].append((CACurrentMediaTime() - started) * 1000)
                } catch {
                    failures += 1
                    say("round \(round) \(name): failed after \(Int((CACurrentMediaTime() - started) * 1000)) ms: \(String(describing: error))")
                }
            }
            await step("probe") {
                let batch = [
                    TerminalPathScanner.Candidate(token: "chart.png", offset: 0, length: 9, directoryOnly: false),
                    TerminalPathScanner.Candidate(token: "clip.mp4", offset: 10, length: 8, directoryOnly: false),
                ]
                let hits = await session.probeRemotePaths(cwd: dir, batch: batch)
                guard hits?.count == 2 else { throw HostConnectionError.failed("probe: \(String(describing: hits))") }
            }
            await step("preview") {
                let host = try await session.acquireFileTransferSSH()
                let entry = try await host.remoteStat(dir + "/chart.png")
                let local = scratch.appendingPathComponent("chart-\(round).png")
                try await host.downloadRemoteFile(entry.path, to: local, progress: nil)
                let size = (try FileManager.default.attributesOfItem(atPath: local.path)[.size] as? UInt64) ?? 0
                try? FileManager.default.removeItem(at: local)
                guard size == entry.size else { throw HostConnectionError.failed("preview: \(size) of \(entry.size ?? 0)") }
            }
            await step("stream 1 MiB") {
                let host = try await session.acquireFileTransferSSH()
                let file = try await host.openRemoteFile(dir + "/clip.mp4")
                let got = try await WindowedRead.pull(
                    size: min(file.size, 1 << 20), chunk: 64 * 1024, window: 16,
                    read: { offset, length in try await file.read(from: offset, length: length) },
                    write: { _, _ in })
                await file.close()
                guard got == min(file.size, 1 << 20) else { throw HostConnectionError.failed("stream: \(got)") }
            }
            await step("folder") {
                let host = try await session.acquireFileTransferSSH()
                let listed = try await host.listRemoteDirectory(dir)
                guard !listed.isEmpty else { throw HostConnectionError.failed("folder: empty") }
            }
            await step("upload 256 KiB") {
                let host = try await session.acquireFileTransferSSH()
                _ = try await host.uploadToUploadsDirectory(image, named: "IMG-w14b-\(round).jpg", progress: nil)
            }
            await step("maintenance read") {
                let host = try await session.acquireFileTransferSSH()
                _ = try await host.executeCommand(hostPreflightCommand())
            }
        }
        for name in ["probe", "preview", "stream 1 MiB", "folder", "upload 256 KiB", "maintenance read"] {
            var samples = times[name] ?? []
            samples.sort()
            guard !samples.isEmpty else { continue }
            let p50 = samples[samples.count / 2]
            say(String(format: "side %@ %@: %@ n=%d p50=%.1f min=%.1f max=%.1f ms", path, transport, name,
                       samples.count, p50, samples.first ?? 0, samples.last ?? 0))
        }
        say("side probe done: \(failures) failure(s)")
    }
}
#endif
