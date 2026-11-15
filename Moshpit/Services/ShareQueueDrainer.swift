import Foundation
import MoshpitKit
import UserNotifications

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
///
/// Never into a pane that has moved on (B622): a share waits up to two
/// days, and by then its pane may be gone, back at its shell, or — tmux
/// numbering panes again from %0 after its server restarts — another
/// program's under the same id. Before the upload and again before the
/// paste, the core decides from the session's panes and stamps
/// (``shareTarget(pane:agent:panes:)``); a share it won't deliver is held
/// back, and the person told once, never dropped without a word.
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

    /// How long a drain waits for a session just come live to show its
    /// tree and read its stamps before deciding where a share goes.
    private static let paneWait: Duration = .seconds(15)

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
        // Home's notices follow what this drain held, delivered or let go.
        defer { HeldShares.shared.refresh() }
        // What earlier builds' lost writes left in the App Group (B269).
        ShareQueue.sweepOrphanedImages(olderThan: maxAge)

        for entry in ShareQueue.load() {
            if Date().timeIntervalSince(entry.createdAt) > maxAge {
                ShareQueue.remove(entry.id)
                continue
            }
            // Held back: it waits for the person, not for the pane.
            guard entry.heldBack == nil else { continue }
            guard let session = hub.sessions[entry.connectionId],
                  session.viewModel.connState == .live else { continue }
            // Where it may go, before anything is uploaded for it.
            guard let before = await target(of: entry, in: session) else { continue }
            guard case .deliver = before else {
                holdBack(entry, before, hub: hub)
                continue
            }
            do {
                guard let paths = try await upload(entry, over: session) else {
                    // Not one of its images is there or decodes: nothing
                    // will ever go.
                    ShareQueue.remove(entry.id)
                    continue
                }
                let text = ImageAttachmentPipeline.insertText(
                    forRemotePaths: paths,
                    style: .forAgent(entry.agentName))
                // Again: an upload takes long enough for the pane to change.
                guard let now = await target(of: entry, in: session) else { continue }
                guard case .deliver(let pane) = now else {
                    holdBack(entry, now, hub: hub)
                    continue
                }
                if await session.deliverPaste(text, toPane: pane) {
                    ShareQueue.remove(entry.id)
                }
            } catch {
                // Leave the entry queued — the next drain retries with a
                // fresh channel. maxAge caps how long it can linger.
                continue
            }
        }
    }

    /// `entry`'s images on the host, by the paths they went to, in order:
    /// uploaded over `session` now unless an earlier try already did (B196),
    /// read one at a time (B342), an image that won't decode left out. Nil
    /// when not one of them is there or decodes.
    private static func upload(_ entry: ShareQueue.Entry,
                               over session: SessionHub.ActiveSession) async throws -> [String]? {
        let images = ShareQueue.images(for: entry)
        guard !images.isEmpty else { return nil }
        // Each image is uploaded once (B196): one that already went in an
        // earlier drain — whose delivery then failed, so the entry stayed —
        // is delivered by the path it went to.
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
                // Never going to decode — an SVG passes for an image to the
                // share sheet and Shortcuts, and ImageIO has nothing for it.
                // It used to fail the whole batch on every drain until the
                // entry aged out, the good images in it with it: "Queued for
                // claude", and nothing ever came. Leave it out; deliver the
                // rest.
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
        return paths.isEmpty ? nil : paths
    }

    // MARK: The person's pick (B622)

    /// Sends held share `id` into `pane` of its session, which the person
    /// picked: no agent check, the pane's own agent (its stamp, else its
    /// program) picks the insert's dialect. True once it is delivered and
    /// gone from the queue; false leaves it held, to pick again.
    static func send(_ id: UUID, toPane pane: String, hub: SessionHub) async -> Bool {
        defer { HeldShares.shared.refresh() }
        guard let entry = ShareQueue.load().first(where: { $0.id == id }),
              let session = hub.sessions[entry.connectionId],
              session.viewModel.connState == .live else { return false }
        do {
            guard let paths = try await upload(entry, over: session) else {
                ShareQueue.remove(entry.id)
                return false
            }
            let agent: String?
            if let control = session.tmuxControl {
                agent = control.agentHooks[pane]?.agent ?? control.snapshot.panes[pane]?.command
            } else {
                agent = session.herdrControl?.agentHooks[pane]?.agent
            }
            let text = ImageAttachmentPipeline.insertText(forRemotePaths: paths, style: .forAgent(agent))
            guard await session.deliverPaste(text, toPane: pane) else { return false }
            ShareQueue.remove(entry.id)
            return true
        } catch {
            Log.island.error("share: a held share's upload failed: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }

    // MARK: Where a share may go (B622)

    /// Where `entry` may go now, once the session shows its panes; nil
    /// while it doesn't yet (the entry waits for the next drain).
    private static func target(of entry: ShareQueue.Entry,
                               in session: SessionHub.ActiveSession) async -> ShareTarget? {
        let deadline = ContinuousClock.now + paneWait
        while true {
            if let panes = panes(of: session) {
                return shareTarget(pane: entry.paneId, agent: entry.agentName, panes: panes)
            }
            guard ContinuousClock.now < deadline, session.viewModel.connState == .live else { return nil }
            try? await Task.sleep(for: .milliseconds(250))
        }
    }

    /// The session's panes, what runs in each and its stamped agent, once
    /// its multiplexer has shown its tree and read its stamps; nil until
    /// then.
    private static func panes(of session: SessionHub.ActiveSession) -> [SharePaneRecord]? {
        if let control = session.tmuxControl {
            guard control.snapshot.isAttached, control.agentHooksLoaded else { return nil }
            let hooks = control.agentHooks
            return control.snapshot.panes.values.map {
                SharePaneRecord(id: $0.id, agent: hooks[$0.id]?.agent, command: $0.command)
            }
        }
        if let control = session.herdrControl {
            guard control.snapshot.isAttached, control.agentHooksLoaded else { return nil }
            // herdr names a pane by the agent it detects there, else by its
            // title — not by a process. Only that detection says an agent
            // runs; a pane without one is taken as back at its shell.
            let hooks = control.agentHooks
            return control.snapshot.panes.values.map {
                let agent = hooks[$0.id]?.agent
                return SharePaneRecord(id: $0.id, agent: agent, command: agent ?? "")
            }
        }
        // A plain shell has no panes to find a share's in.
        return session.connection.multiplexer == .none ? [] : nil
    }

    /// Hold `entry` back for `target`, and tell the person the first time:
    /// Home says it while the app is in front, a notification while it is
    /// not.
    private static func holdBack(_ entry: ShareQueue.Entry, _ target: ShareTarget, hub: SessionHub) {
        let held: ShareQueue.HeldBack
        switch target {
        case .deliver: return
        case .paneGone: held = .init(reason: .paneGone, now: nil, at: Date())
        case .agentExited(let shell): held = .init(reason: .agentExited, now: shell, at: Date())
        case .anotherProgram(let now): held = .init(reason: .anotherProgram, now: now, at: Date())
        }
        Log.island.info("share: held back, its pane moved on (\(held.reason.rawValue, privacy: .public))")
        guard ShareQueue.holdBack(entry.id, held), !hub.isForeground else { return }
        tell(entry, held)
    }

    /// The one word the person gets about a share held while the app is
    /// away: a notification. (In front, Home's notice says it.)
    private static func tell(_ entry: ShareQueue.Entry, _ held: ShareQueue.HeldBack) {
        let content = UNMutableNotificationContent()
        content.title = String(localized: "A share wasn't delivered")
        content.body = held.reason == .paneGone
            ? String(localized: "That pane is closed.")
            : String(localized: "That pane isn't running \(entry.agentName) any more.")
        let request = UNNotificationRequest(identifier: "moshpit.share.held.\(entry.id.uuidString)",
                                            content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }
}

/// The shares the drain held back (B622), as Home's notices show them:
/// read from the queue again whenever a drain ends or Home asks.
@Observable
@MainActor
final class HeldShares {
    static let shared = HeldShares()

    /// Held entries, oldest first.
    private(set) var entries: [ShareQueue.Entry] = []

    /// The queue's held entries now.
    func refresh() {
        let held = ShareQueue.load().filter { $0.heldBack != nil }
        if held != entries { entries = held }
    }

    /// `{"op": "held", "shares": […]}`: Home's message for them.
    var message: String { Self.message(entries) }

    /// Home's message for held `entries`.
    nonisolated static func message(_ entries: [ShareQueue.Entry]) -> String {
        let shares: [[String: String]] = entries.compactMap { entry in
            guard let held = entry.heldBack else { return nil }
            return ["id": entry.id.uuidString, "connection": entry.connectionId.uuidString,
                    "agent": entry.agentName, "reason": held.reason.rawValue]
        }
        let data = (try? JSONSerialization.data(
            withJSONObject: ["op": "held", "shares": shares], options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    /// Lets the held share `id` go.
    func discard(_ id: UUID) {
        ShareQueue.remove(id)
        refresh()
    }
}
