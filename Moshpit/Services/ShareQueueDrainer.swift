import Foundation

/// Drains the share extension's queue: for every entry whose session is
/// live, run the shared images through the normal pipeline, upload over
/// that session's channel, and deliver the paths to the queued pane in the
/// queued agent's dialect. Entries whose session isn't live simply wait —
/// the queue is re-drained on every foreground return and session start.
///
/// Deliberately Insert-only (no Return): a queued image arrives while the
/// user isn't looking at the pane, and the keystroke that submits a prompt
/// executes a shell command — the same trust split as everywhere else, with
/// the cautious side chosen because nobody is watching.
@MainActor
enum ShareQueueDrainer {
    private static var draining = false

    /// Entries older than this are dropped rather than delivered — a path
    /// pasted into a prompt days later, into whatever that pane runs NOW,
    /// is a hazard, not a delivery.
    private static let maxAge: TimeInterval = 48 * 60 * 60

    static func drain(hub: SessionHub) async {
        guard !draining else { return }
        draining = true
        defer { draining = false }

        for entry in ShareQueue.load() {
            if Date().timeIntervalSince(entry.createdAt) > maxAge {
                ShareQueue.remove(entry.id)
                continue
            }
            guard let session = hub.sessions[entry.connectionId],
                  session.viewModel.connState == .live else { continue }
            let images = ShareQueue.imageData(for: entry)
            guard !images.isEmpty else {
                ShareQueue.remove(entry.id)
                continue
            }
            do {
                let uploader = try await session.acquireFileTransferSSH()
                var paths: [String] = []
                for raw in images {
                    let processed = try await Task.detached(priority: .utility) {
                        try ImageAttachmentPipeline.process(raw)
                    }.value
                    let path = try await uploader.uploadToUploadsDirectory(
                        processed.data, named: processed.filename, progress: nil)
                    session.imageUploads.record(remotePath: path)
                    paths.append(path)
                }
                let text = ImageAttachmentPipeline.insertText(
                    forRemotePaths: paths,
                    style: .forAgent(entry.agentName))
                if await session.deliverPaste(text, toPane: entry.paneId) {
                    ShareQueue.remove(entry.id)
                }
            } catch {
                // Leave the entry queued — the next drain retries with a
                // fresh channel. maxAge caps how long it can linger.
                continue
            }
        }
    }
}
