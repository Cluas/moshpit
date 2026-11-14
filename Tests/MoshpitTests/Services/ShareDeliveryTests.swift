import Foundation
import MoshpitKit
import Testing
import UIKit
@testable import Moshpit

/// B172: delivering to a pane says what really happened.
///
/// `deliverPaste` and `deliverInput` returned true whatever became of the
/// bytes, and the share queue deletes an entry it is told was delivered. A
/// paste into a tmux controller still attaching (it drops every command
/// before the attach), into a pane that is gone, into a session that isn't
/// live or down a write that failed was reported delivered — and the image
/// the user shared was gone for good. These run the hub's real start and
/// delivery paths over a fake host whose tmux answers the controller.
@Suite("share delivery", .serialized)
@MainActor
struct ShareDeliveryTests {

    private func connection(_ multiplexer: Multiplexer) -> ServerConnection {
        var connection = ServerConnection(
            name: "share",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.multiplexer = multiplexer
        // A path the user vouches for: no probe degrades it to a plain shell.
        if multiplexer == .tmux { connection.tmuxPath = "tmux" }
        return connection
    }

    private func session(_ connection: ServerConnection, over host: FakeSSHHost) -> SessionHub.ActiveSession {
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        session.probeTimeout = 0.2
        return session
    }

    private func eventually(_ timeout: Double = 5, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }

    @Test("a paste into a tmux controller still attaching is not reported delivered")
    func pasteBeforeTheAttachIsNotDelivered() async {
        // A server with no session: the attach never lands.
        let host = FakeSSHHost(tmux: .init(hasSession: false))
        let session = session(connection(.tmux), over: host)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        #expect(session.tmuxController != nil)
        #expect(await session.deliverPaste("/home/tester/.moshpit/uploads/a.png", toPane: "%0") == false,
                "the controller dropped it before the attach")
        await session.stop()
    }

    @Test("tmux's own answer is the paste's outcome")
    func tmuxAnswersThePaste() async {
        let host = FakeSSHHost(tmux: .init())
        let session = session(connection(.tmux), over: host)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(await eventually { session.tmuxController?.snapshot.panes["%0"] != nil })

        #expect(await session.deliverPaste("a shared path", toPane: "%0"))
        #expect(host.typed.contains("paste-buffer -p -b moshpit-paste -d -t %0"))

        // The pane went away meanwhile: tmux says so.
        host.tmux?.pasteSucceeds = false
        #expect(await session.deliverPaste("a shared path", toPane: "%0") == false)
        await session.stop()
    }

    @Test("a session that isn't live takes nothing")
    func notLiveIsNotDelivered() async {
        let session = session(connection(.none), over: FakeSSHHost())
        #expect(await session.deliverInput(Data("echo hi\r".utf8), toPane: "%0") == false)
        #expect(await session.deliverPaste("echo hi", toPane: "%0") == false)
    }

    @Test("a plain shell says whether the write went out")
    func plainShellWriteOutcome() async {
        let host = FakeSSHHost()
        let session = session(connection(.none), over: host)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.status == .connected)
        #expect(await session.deliverInput(Data("echo one\r".utf8), toPane: "%0"))
        #expect(host.typed.contains("echo one\r"))

        host.failWrites()   // the channel died under a session still marked live
        #expect(await session.deliverInput(Data("echo two\r".utf8), toPane: "%0") == false)
        await session.stop()
    }

    // MARK: The share queue itself

    /// A 2×2 PNG, real enough for the attachment pipeline.
    private func png() -> Data {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2))
        return renderer.pngData { context in
            UIColor.systemTeal.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
    }

    /// A hub with one live tmux session on a host whose tmux refuses pastes
    /// until the test says otherwise. Uploads go the exec way (no SFTP over
    /// a fake connection).
    private func queueBench() async -> (FakeSSHHost, SessionHub, ServerConnection) {
        let host = FakeSSHHost(tmux: .init(pasteSucceeds: false)) { command in
            if command.contains("printf %s \"$HOME\"") { return .output("/home/tester") }
            if command.contains(ExecUploadCommands.successMarker) {
                return .output(ExecUploadCommands.successMarker + "\n")
            }
            return .output("")
        }
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
            session.probeTimeout = 0.2
            return session
        }
        let connection = connection(.tmux)
        let session = hub.prepare(connection)
        // Not hub.start: that drains on its own, racing the drains below.
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(await eventually { session.tmuxController?.snapshot.panes["%0"] != nil })
        return (host, hub, connection)
    }

    @Test("the share queue keeps an entry until it is really delivered",
          .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
    func queueKeepsUndeliveredEntries() async throws {
        let (host, hub, connection) = await queueBench()

        try ShareQueue.append(target: (connectionId: connection.id, paneId: "%0"),
                              agentName: "claude", label: "round 22", images: [png()])
        let entry = try #require(ShareQueue.load().last { $0.connectionId == connection.id })
        defer { ShareQueue.remove(entry.id) }

        // tmux refuses the paste (the pane is gone): the entry stays.
        await ShareQueueDrainer.drain(hub: hub)
        #expect(host.typed.contains("paste-buffer"), "the drain got as far as the paste")
        #expect(ShareQueue.load().contains { $0.id == entry.id },
                "an entry whose paste tmux refused was deleted as delivered")

        // The next drain gets it through: now it goes.
        host.tmux?.pasteSucceeds = true
        await ShareQueueDrainer.drain(hub: hub)
        #expect(!ShareQueue.load().contains { $0.id == entry.id })
        await hub.disconnect(connection.id)
    }

    @Test("a queued share is uploaded once, however many drains its delivery takes (B196)",
          .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
    func queuedShareIsUploadedOnce() async throws {
        // An entry whose delivery failed stays queued (B172), and every
        // later drain uploaded its images again: one more copy on the host
        // per retry.
        let (host, hub, connection) = await queueBench()
        func uploads() -> Int {
            host.commands.filter { $0.contains(ExecUploadCommands.successMarker) }.count
        }
        try ShareQueue.append(target: (connectionId: connection.id, paneId: "%0"),
                              agentName: "claude", label: "round 32", images: [png(), png()])
        let entry = try #require(ShareQueue.load().last { $0.connectionId == connection.id })
        defer { ShareQueue.remove(entry.id) }

        // tmux refuses the paste: both images went up, and the entry says where.
        await ShareQueueDrainer.drain(hub: hub)
        let uploaded = uploads()
        #expect(uploaded >= 2)
        let kept = try #require(ShareQueue.load().first { $0.id == entry.id })
        let paths = try #require(kept.uploaded)
        #expect(Set(paths.keys) == Set(entry.files))
        #expect(paths.count == 2)

        // Refused again: delivered again, not uploaded again.
        await ShareQueueDrainer.drain(hub: hub)
        #expect(uploads() == uploaded, "the retry uploaded the images again")

        host.tmux?.pasteSucceeds = true
        await ShareQueueDrainer.drain(hub: hub)
        #expect(uploads() == uploaded)
        #expect(!ShareQueue.load().contains { $0.id == entry.id })
        for path in paths.values {
            #expect(host.typed.contains(path), "the delivery names \(path)")
        }
        await hub.disconnect(connection.id)
    }

    /// An SVG passes for an image to the share sheet and to Shortcuts, and
    /// ImageIO has nothing for it. The batch failed on it at every drain
    /// until the entry aged out after two days — the images in it that DO
    /// decode were never delivered either.
    @Test("an image that can't be decoded is left out, the rest of its batch delivered",
          .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
    func undecodableImageIsLeftOut() async throws {
        let (host, hub, connection) = await queueBench()
        host.tmux?.pasteSucceeds = true
        let svg = Data(#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"/>"#.utf8)
        try ShareQueue.append(target: (connectionId: connection.id, paneId: "%0"),
                              agentName: "claude", label: "round 41", images: [svg, png()])
        let entry = try #require(ShareQueue.load().last { $0.connectionId == connection.id })
        defer { ShareQueue.remove(entry.id) }

        await ShareQueueDrainer.drain(hub: hub)

        #expect(!ShareQueue.load().contains { $0.id == entry.id },
                "the batch is still queued: its good image never went")
        #expect(host.commands.filter { $0.contains(ExecUploadCommands.successMarker) }.count == 1)
        #expect(host.typed.contains("paste-buffer"))
        await hub.disconnect(connection.id)
    }

    @Test("a batch none of whose images decode is dropped, not retried for two days",
          .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
    func batchOfUndecodablesIsDropped() async throws {
        let (host, hub, connection) = await queueBench()
        try ShareQueue.append(target: (connectionId: connection.id, paneId: "%0"),
                              agentName: "claude", label: "round 41",
                              images: [Data("not an image".utf8)])
        let entry = try #require(ShareQueue.load().last { $0.connectionId == connection.id })
        defer { ShareQueue.remove(entry.id) }

        await ShareQueueDrainer.drain(hub: hub)

        #expect(!ShareQueue.load().contains { $0.id == entry.id })
        #expect(!host.commands.contains { $0.contains(ExecUploadCommands.successMarker) })
        #expect(!host.typed.contains("paste-buffer"))
        await hub.disconnect(connection.id)
    }

    /// A drain asked for while another ran was dropped, and the running one
    /// works from the queue as it read it. A share queued meanwhile — the
    /// Shortcuts action queues and drains in-process, a foreground return
    /// drains while the session start drains — waited for the next return
    /// to the foreground, with the session live all along.
    @Test("a drain asked for during a drain delivers what was queued meanwhile",
          .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
    func drainDuringADrainIsNotDropped() async throws {
        // The first upload is held, so the first drain is mid-flight.
        let uploads = Counter()
        let host = FakeSSHHost(tmux: .init(pasteSucceeds: true)) { command in
            if command.contains("printf %s \"$HOME\"") { return .output("/home/tester") }
            if command.contains(ExecUploadCommands.successMarker) {
                return uploads.next() == 1
                    ? .held("first-upload")
                    : .output(ExecUploadCommands.successMarker + "\n")
            }
            return .output("")
        }
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
            session.probeTimeout = 0.2
            return session
        }
        let connection = connection(.tmux)
        let session = hub.prepare(connection)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(await eventually { session.tmuxController?.snapshot.panes["%0"] != nil })

        try ShareQueue.append(target: (connectionId: connection.id, paneId: "%0"),
                              agentName: "claude", label: "first", images: [png()])
        let first = try #require(ShareQueue.load().last { $0.connectionId == connection.id })
        defer { ShareQueue.remove(first.id) }
        let firstDrain = Task { await ShareQueueDrainer.drain(hub: hub) }
        #expect(await eventually { host.isHolding("first-upload") })

        try ShareQueue.append(target: (connectionId: connection.id, paneId: "%0"),
                              agentName: "claude", label: "second", images: [png()])
        let second = try #require(ShareQueue.load().last { $0.label == "second" && $0.connectionId == connection.id })
        defer { ShareQueue.remove(second.id) }
        let secondDrain = Task { await ShareQueueDrainer.drain(hub: hub) }
        try? await Task.sleep(for: .milliseconds(100))
        host.release("first-upload", with: ExecUploadCommands.successMarker + "\n")
        await firstDrain.value
        await secondDrain.value

        let left = ShareQueue.load().filter { $0.connectionId == connection.id }.map(\.label)
        #expect(left.isEmpty, "still queued with the session live: \(left)")
        await hub.disconnect(connection.id)
    }

    final class Counter: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        func next() -> Int { lock.withLock { count += 1; return count } }
    }

    @Test("an entry earlier builds queued, with no uploads recorded, still decodes")
    func entryWithoutUploadsDecodes() throws {
        let json = #"[{"id":"0F0E0D0C-0000-4000-8000-000000000001","connectionId":"0F0E0D0C-0000-4000-8000-000000000002","paneId":"%0","agentName":"claude","label":"x","files":["a.img"],"createdAt":0}]"#
        let entries = try JSONDecoder().decode([ShareQueue.Entry].self, from: Data(json.utf8))
        #expect(entries.first?.files == ["a.img"])
        #expect(entries.first?.uploaded == nil)
    }
}
