import Foundation
import MoshpitKit

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
    /// The drain running now, if one is.
    private static var current: Task<Void, Never>?
    /// A drain was asked for while ``current`` ran: it goes round once more.
    private static var again = false

    /// Entries older than this are dropped rather than delivered — a path
    /// pasted into a prompt days later, into whatever that pane runs NOW,
    /// is a hazard, not a delivery.
    private static let maxAge: TimeInterval = 48 * 60 * 60

    /// Deliver what can be delivered now. Returns once it has been tried —
    /// including when a drain was already running: that one goes round again
    /// and this waits for it.
    ///
    /// A drain asked for while another ran used to be dropped, and the one
    /// running works from the queue as it read it when it began. A share
    /// queued meanwhile, or a session that came live meanwhile — the
    /// foreground drain still uploading over a connection the background
    /// had killed while the reconnect lands — waited for the next return to
    /// the foreground, and the Shortcuts action said "delivers when the
    /// session is next live" about a session that was live.
    static func drain(hub: SessionHub) async {
        if let current {
            again = true
            await current.value
            return
        }
        let drain = Task { @MainActor in
            repeat {
                again = false
                await drainOnce(hub: hub)
            } while again
            // In the same turn as the last look at `again`: a request after
            // this starts a drain of its own.
            current = nil
        }
        current = drain
        await drain.value
    }

    private static func drainOnce(hub: SessionHub) async {
        // What earlier builds' lost writes left in the App Group (B269).
        ShareQueue.sweepOrphanedImages(olderThan: maxAge)

        for entry in ShareQueue.load() {
            if Date().timeIntervalSince(entry.createdAt) > maxAge {
                ShareQueue.remove(entry.id)
                continue
            }
            guard let session = hub.sessions[entry.connectionId],
                  session.viewModel.connState == .live else { continue }
            // Read one at a time as the loop reaches each (B342): never the
            // whole batch of originals at once.
            let images = ShareQueue.images(for: entry)
            guard !images.isEmpty else {
                ShareQueue.remove(entry.id)
                continue
            }
            do {
                // Each image is uploaded once (B196): one that already went
                // in an earlier drain — whose delivery then failed, so the
                // entry stayed — is delivered by the path it went to.
                var uploader: (any HostConnection)?
                var paths: [String] = []
                for image in images {
                    if let path = image.uploadedTo {
                        paths.append(path)
                        continue
                    }
                    let raw = image.data
                    let processed: ImageAttachmentPipeline.Processed
                    do {
                        processed = try await Task.detached(priority: .utility) {
                            try ImageAttachmentPipeline.process(raw)
                        }.value
                    } catch ImageAttachmentPipeline.PipelineError.undecodable {
                        // Never going to decode — an SVG passes for an image
                        // to the share sheet and Shortcuts, and ImageIO has
                        // nothing for it. It used to fail the whole batch on
                        // every drain until the entry aged out, the good
                        // images in it with it: "Queued for claude", and
                        // nothing ever came. Leave it out; deliver the rest.
                        Log.island.error("share: an image in the queue can't be decoded — left out")
                        continue
                    }
                    let host: any HostConnection
                    if let uploader {
                        host = uploader
                    } else {
                        host = try await session.acquireFileTransferSSH()
                        uploader = host
                    }
                    let path = try await host.uploadToUploadsDirectory(
                        processed.data, named: processed.filename, progress: nil)
                    ShareQueue.recordUpload(entry.id, file: image.file, remotePath: path)
                    session.imageUploads.record(remotePath: path)
                    paths.append(path)
                }
                guard !paths.isEmpty else {
                    // Not one of its images decodes: nothing will ever go.
                    ShareQueue.remove(entry.id)
                    continue
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
