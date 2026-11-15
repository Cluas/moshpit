import Foundation
import Testing
@testable import Moshpit

/// W14b: a side feature's call on a Rust session's lent connection
/// (``BorrowedHost`` over crates/ffi `BorrowedSsh`), against a fake that
/// answers after a link's delay.
@Suite("Side features on the session's lent connection")
struct BorrowedHostTests {

    private func scratch() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("w14b-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    // MARK: Commands

    @Test("a command's answer is its stdout, and only exit status 0 is success (B136)")
    func commands() async throws {
        let ssh = FakeLentSSH()
        ssh.commands = { command in
            switch command {
            case "ok": return .init(stdout: Data("out\n".utf8), stderr: Data("noise\n".utf8))
            case "three": return .init(stdout: Data("partial".utf8), status: 3)
            case "killed": return .init(stdout: Data("partial".utf8), status: nil)
            default: return .init(stdout: Data("cut".utf8), status: nil, finished: false)
            }
        }
        let host = BorrowedHost(ssh)
        #expect(try await host.executeCommand("ok") == Data("out\n".utf8))
        await #expect(throws: HostConnectionError.exited(status: 3)) { try await host.executeCommand("three") }
        await #expect(throws: HostConnectionError.endedWithoutStatus) { try await host.executeCommand("killed") }
        // The connection went away mid-command: the connection, not the command.
        await #expect(throws: HostConnectionError.notAnswering) { try await host.executeCommand("dropped") }
        #expect(ssh.ran == ["ok", "three", "killed", "dropped"])
    }

    @Test("the core's failures in the app's words")
    func failures() async throws {
        let ssh = FakeLentSSH()
        let host = BorrowedHost(ssh)
        ssh.failure = .NotConnected
        await #expect(throws: HostConnectionError.notConnected) { try await host.remoteStat("/x") }
        ssh.failure = .Ssh(cause: .plain, log: "closed")
        await #expect(throws: HostConnectionError.notAnswering) { try await host.executeCommand("true") }
        ssh.failure = .Ssh(cause: .channel(timedOut: true), log: "no answer")
        await #expect(throws: HostConnectionError.notAnswering) { try await host.listRemoteDirectory("/x") }
        ssh.failure = .Ssh(cause: .channel(timedOut: false), log: "refused")
        await #expect(throws: HostConnectionError.failed("refused")) { try await host.executeCommand("true") }
        ssh.failure = .Sftp(why: .transport, path: "/x", log: "gone")
        await #expect(throws: HostConnectionError.notAnswering) { try await host.remoteStat("/x") }
        ssh.failure = nil

        ssh.file("/secret", Data("x".utf8))
        ssh.deny("/secret")
        await #expect(throws: HostConnectionError.readDenied) { try await host.remoteStat("/secret") }
        ssh.deny("/locked")
        await #expect(throws: HostConnectionError.writeDenied) {
            try await host.createRemoteDirectory("/locked/new")
        }
        await #expect(throws: HostConnectionError.notFound) { try await host.remoteStat("/nope") }
        // Each in words the person can act on.
        #expect(RemotePreviewCache.message(for: HostConnectionError.reconnecting)
                == HostConnectionError.reconnecting.errorDescription)
        #expect(RemotePreviewCache.message(for: HostConnectionError.notConnected)
                != RemotePreviewCache.message(for: HostConnectionError.notAnswering))
        #expect(HostConnectionError.readDenied.errorDescription != HostConnectionError.writeDenied.errorDescription)
    }

    // MARK: SFTP

    @Test("a preview over SFTP: stat, download with progress, a stream's ranges")
    func preview() async throws {
        let ssh = FakeLentSSH()
        let chart = Data((0..<300_000).map { UInt8($0 % 251) })
        ssh.file("/work/out/chart.png", chart)
        ssh.folder("/work/out")
        let host = BorrowedHost(ssh)

        let entry = try await host.remoteStat("/work/out/chart.png")
        #expect(entry.path == "/work/out/chart.png")
        #expect(!entry.isDirectory)
        #expect(entry.size == 300_000)
        #expect(entry.modified == Date(timeIntervalSince1970: 1_700_000_000))

        let dir = try scratch()
        let local = dir.appendingPathComponent("chart.png")
        let seen = Fractions()
        try await host.downloadRemoteFile("/work/out/chart.png", to: local) { seen.add($0) }
        #expect(try Data(contentsOf: local) == chart)
        #expect(seen.values.last == 1)

        let file = try await host.openRemoteFile("/work/out/chart.png")
        #expect(file.size == 300_000)
        let range = try await file.read(from: 1000, length: 64)
        #expect(Data(range) == chart.subdata(in: 1000..<1064))
        await file.close()
        #expect(ssh.opened.first?.closed == true)

        let listed = try await host.listRemoteDirectory("/work/out")
        #expect(listed.map(\.name) == ["chart.png"])
        #expect(try await host.remoteHomeDirectory() == "/home/dev")
        // Nothing went over exec: the host has SFTP.
        #expect(ssh.ran.isEmpty)
    }

    /// B461: every fetch of a file shared one `.partial`. A tap on a folder
    /// tile whose thumbnail was still loading started a second download
    /// into it, which cut the first's bytes off; the last to finish deleted
    /// the copy the other had handed out and failed its own move.
    @Test("two fetches of one file at once both end with the whole copy, and nothing else is left")
    func overlappingFetches() async throws {
        let ssh = FakeLentSSH()
        let chart = Data((0..<300_000).map { UInt8($0 % 251) })
        ssh.file("/work/out/chart.png", chart)
        // The thumbnail's download is still landing when the tap's starts
        // and finishes.
        ssh.holdDownloads(.milliseconds(400))
        let host = BorrowedHost(ssh)
        let entry = try await host.remoteStat("/work/out/chart.png")
        let files = RemoteFileAccess(scope: "b461-\(UUID().uuidString)", channel: { host })
        let thumbnail = Task { try await RemotePreviewCache.fetch(entry, files: files, progress: { _ in }) }
        try await Task.sleep(for: .milliseconds(100))
        let tapped = try await RemotePreviewCache.fetch(entry, files: files, progress: { _ in })
        let first = try await thumbnail.value
        #expect(first == tapped)
        #expect(try Data(contentsOf: tapped) == chart)
        let folder = tapped.deletingLastPathComponent()
        #expect(try FileManager.default.contentsOfDirectory(atPath: folder.path) == ["chart.png"])
        try? FileManager.default.removeItem(at: folder)
    }

    @Test("a fetch given up leaves nothing behind in the cache (B461)")
    func abandonedFetch() async throws {
        let ssh = FakeLentSSH()
        ssh.file("/work/out/clip.mov", Data(count: 300_000))
        ssh.holdDownloads(.milliseconds(400))
        let host = BorrowedHost(ssh)
        let entry = try await host.remoteStat("/work/out/clip.mov")
        let files = RemoteFileAccess(scope: "b461-\(UUID().uuidString)", channel: { host })
        let fetch = Task { try await RemotePreviewCache.fetch(entry, files: files, progress: { _ in }) }
        try await Task.sleep(for: .milliseconds(100))
        fetch.cancel()
        await #expect(throws: CancellationError.self) { try await fetch.value }
        // The core gives up at its next report, after the hold.
        try await Task.sleep(for: .milliseconds(600))
        let folder = RemotePreviewCache.localURL(for: entry, scope: files.scope).deletingLastPathComponent()
        #expect(try FileManager.default.contentsOfDirectory(atPath: folder.path).isEmpty)
        try? FileManager.default.removeItem(at: folder)
    }

    @Test("a host with SFTP turned off: the Swift path's exec forms, through the one quoter")
    func noSFTP() async throws {
        let ssh = FakeLentSSH()
        ssh.sftpAvailable = false
        let path = "/work/it's \u{301}' here.png"
        let image = Data("PNG!".utf8)
        ssh.commands = { command in
            switch command {
            case RemoteFileCommands.stat(path): return .init(stdout: Data("f        4\n".utf8))
            case RemoteFileCommands.cat(path): return .init(stdout: Data((image.base64EncodedString() + "\n").utf8))
            case RemoteFileCommands.list("/work"): return .init(stdout: Data("d 1700000002 sub\u{0}f 1700000001 it's \u{301}' here.png\u{0}".utf8))
            case "pwd": return .init(stdout: Data("/home/dev \n".utf8))
            default: return .init(status: 1)
            }
        }
        let host = BorrowedHost(ssh)
        let entry = try await host.remoteStat(path)
        #expect(entry.size == 4 && !entry.isDirectory && entry.modified == nil)
        let local = try scratch().appendingPathComponent("x.png")
        try await host.downloadRemoteFile(path, to: local, progress: nil)
        #expect(try Data(contentsOf: local) == image)
        #expect(try await host.listRemoteDirectory("/work").map(\.name) == ["sub", "it's \u{301}' here.png"])
        // Only the newline `pwd` ends with is cut: a home ending in a space is that home.
        #expect(try await host.remoteHomeDirectory() == "/home/dev ")
        // A stream needs SFTP; the caller downloads instead.
        await #expect(throws: HostConnectionError.self) { try await host.openRemoteFile(path) }
        // Every word went through Shell.quote, apostrophe and combining mark alike (B113).
        #expect(ssh.ran.contains(RemoteFileCommands.stat(path)))
        #expect(RemoteFileCommands.stat(path).contains(#"'\''"#))
    }

    @Test("past the exec limit a host without SFTP says the file is too large")
    func tooLargeWithoutSFTP() async throws {
        let ssh = FakeLentSSH()
        ssh.sftpAvailable = false
        let big = RemoteFileCommands.execDownloadLimit + 1
        ssh.commands = { command in
            command.contains("if [ -d") ? .init(stdout: Data("f \(big)\n".utf8)) : .init(status: 1)
        }
        let local = try scratch().appendingPathComponent("big")
        await #expect(throws: HostConnectionError.tooLargeWithoutSFTP) {
            try await BorrowedHost(ssh).downloadRemoteFile("/big", to: local, progress: nil)
        }
    }

    // MARK: A tapped path (1.0.8's files, B743)

    /// A tapped path's preview over `ssh`, started.
    @MainActor
    private func preview(_ path: String, on ssh: FakeLentSSH) -> RemotePreviewModel {
        let host = BorrowedHost(ssh)
        let model = RemotePreviewModel(path: path,
                                       files: RemoteFileAccess(scope: "b743-\(UUID().uuidString)") { host })
        model.start()
        return model
    }

    /// The phase the preview settles on once it stops looking (or
    /// downloading): what the card, Quick Look, the player or the folder
    /// sheet takes from there.
    @MainActor
    private func settled(_ model: RemotePreviewModel) async throws -> RemotePreviewModel.Phase {
        for _ in 0..<600 {
            switch model.phase {
            case .looking, .downloading: try await Task.sleep(for: .milliseconds(5))
            default: return model.phase
            }
        }
        return model.phase
    }

    /// 1.0.8 (1)'s note: a tapped file downloads over the file-transfer
    /// connection and opens in Quick Look; one over 50 MB asks first.
    @Test("a tapped file opens at once up to 50 MB; past that it asks first, and a yes fetches it")
    @MainActor
    func fiftyMegabytesAsk() async throws {
        #expect(RemotePreviewModel.confirmAbove == 50 * 1024 * 1024)
        let ssh = FakeLentSSH()
        let bytes = Data("PNG!".utf8)
        for name in ["small.png", "edge.png", "big.png"] { ssh.file("/work/\(name)", bytes) }
        ssh.reportSize("/work/edge.png", RemotePreviewModel.confirmAbove)
        ssh.reportSize("/work/big.png", RemotePreviewModel.confirmAbove + 1)

        for name in ["small.png", "edge.png"] {
            let model = preview("/work/\(name)", on: ssh)
            guard case .ready(let url) = try await settled(model) else {
                Issue.record("\(name) didn't open at once: \(model.phase)")
                continue
            }
            #expect(try Data(contentsOf: url) == bytes)
            try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
        }

        let big = preview("/work/big.png", on: ssh)
        guard case .confirm(let entry) = try await settled(big) else {
            Issue.record("past 50 MB it didn't ask: \(big.phase)")
            return
        }
        #expect(entry.size == RemotePreviewModel.confirmAbove + 1)
        #expect(big.showsHUD, "the card asks")
        big.confirm()
        guard case .ready(let url) = try await settled(big) else {
            Issue.record("a yes didn't fetch it: \(big.phase)")
            return
        }
        #expect(try Data(contentsOf: url) == bytes)
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    }

    /// 1.0.8 (1): a tapped .mp4/.mov/.m4v starts playing at once, reading
    /// only the bytes the player needs; a host without SFTP falls back to
    /// downloading (asking first past 50 MB, as any file).
    @Test("a video streams at once whatever its size; without SFTP it downloads instead")
    @MainActor
    func videoStreamsOrDownloads() async throws {
        let ssh = FakeLentSSH()
        let clip = Data((0..<4096).map { UInt8($0 % 251) })
        for name in ["a.mp4", "b.MOV", "c.m4v"] { ssh.file("/work/\(name)", clip) }
        ssh.reportSize("/work/a.mp4", RemotePreviewModel.confirmAbove * 4)
        for name in ["a.mp4", "b.MOV", "c.m4v"] {
            let model = preview("/work/\(name)", on: ssh)
            guard case .stream(let entry, let file) = try await settled(model) else {
                Issue.record("\(name) didn't stream: \(model.phase)")
                continue
            }
            #expect(entry.name == name)
            #expect(!model.showsHUD, "nothing to wait for or confirm: the player takes it")
            let range = try await file.read(from: 1000, length: 16)
            #expect(Data(range) == clip.subdata(in: 1000..<1016), "a range, read where the player asks")
            await file.close()
        }

        let plain = FakeLentSSH()
        plain.sftpAvailable = false
        let big = RemotePreviewModel.confirmAbove + 1
        plain.commands = { command in
            switch command {
            case RemoteFileCommands.stat("/work/small.mp4"): return .init(stdout: Data("f 4\n".utf8))
            case RemoteFileCommands.cat("/work/small.mp4"): return .init(stdout: Data("TINY".utf8).base64EncodedData() + Data("\n".utf8))
            case RemoteFileCommands.stat("/work/big.mp4"): return .init(stdout: Data("f \(big)\n".utf8))
            default: return .init(status: 1)
            }
        }
        let small = preview("/work/small.mp4", on: plain)
        guard case .ready(let url) = try await settled(small) else {
            Issue.record("without SFTP a video didn't download: \(small.phase)")
            return
        }
        #expect(try Data(contentsOf: url) == Data("TINY".utf8))
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
        let large = preview("/work/big.mp4", on: plain)
        guard case .confirm = try await settled(large) else {
            Issue.record("without SFTP a big video didn't ask first: \(large.phase)")
            return
        }
    }

    /// 1.0.8 (1): a tapped folder opens the folder sheet.
    @Test("a tapped folder hands over to the folder sheet")
    @MainActor
    func folderOpensTheSheet() async throws {
        let ssh = FakeLentSSH()
        ssh.folder("/work/out")
        ssh.file("/work/out/a.png", Data("PNG!".utf8))
        let model = preview("/work/out", on: ssh)
        guard case .folder(let entry) = try await settled(model) else {
            Issue.record("a folder didn't open its sheet: \(model.phase)")
            return
        }
        #expect(entry.isDirectory && entry.path == "/work/out")
        #expect(!model.showsHUD)
    }

    // MARK: Changing the folder

    @Test("the folder sheet's changes over SFTP, and their exec forms read by marker")
    func edits() async throws {
        let ssh = FakeLentSSH()
        ssh.folder("/work")
        ssh.file("/work/a.txt", Data("a".utf8))
        let host = BorrowedHost(ssh)
        try await host.createRemoteDirectory("/work/new")
        try await host.renameRemoteItem("/work/a.txt", to: "/work/b.txt")
        #expect(ssh.contents("/work/b.txt") == Data("a".utf8))
        await #expect(throws: HostConnectionError.nameTaken) {
            try await host.renameRemoteItem("/work/b.txt", to: "/work/new")
        }
        try await host.deleteRemoteItem(RemoteFileEntry(path: "/work/b.txt", isDirectory: false, size: 1, modified: nil))
        #expect(ssh.contents("/work/b.txt") == nil)
        // A folder goes with `rm -rf`, whatever SFTP can do.
        ssh.commands = { _ in .init(stdout: Data((RemoteFileEdits.okMarker + "\n").utf8)) }
        try await host.deleteRemoteItem(RemoteFileEntry(path: "/work/new", isDirectory: true, size: nil, modified: nil))
        #expect(ssh.ran.last == RemoteFileEdits.remove("/work/new", isDirectory: true))
        // Never `/` or a top-level folder.
        await #expect(throws: HostConnectionError.writeDenied) {
            try await host.deleteRemoteItem(RemoteFileEntry(path: "/usr", isDirectory: true, size: nil, modified: nil))
        }

        // No SFTP: the exec form says "taken" with exit status 1 — read
        // before the status, so the person hears the name is taken (the Swift
        // path's status check turned it into "couldn't reach the host").
        ssh.sftpAvailable = false
        ssh.commands = { _ in .init(stdout: Data((RemoteFileEdits.takenMarker + "\n").utf8), status: 1) }
        await #expect(throws: HostConnectionError.nameTaken) {
            try await host.renameRemoteItem("/work/x", to: "/work/y")
        }
        ssh.commands = { _ in .init(stdout: Data((RemoteFileEdits.okMarker + "\n").utf8)) }
        try await host.createRemoteDirectory("/work/made")
        #expect(ssh.ran.last == RemoteFileEdits.makeDirectory("/work/made"))
        ssh.commands = { _ in .init(stdout: Data(), status: 1) }
        await #expect(throws: HostConnectionError.writeDenied) {
            try await host.createRemoteDirectory("/work/nope")
        }
    }

    @Test("a file picked in Files lands under its own name; without SFTP, in base64 chunks")
    func folderUpload() async throws {
        let ssh = FakeLentSSH()
        ssh.folder("/work")
        let local = try scratch().appendingPathComponent("notes.txt")
        let body = Data(repeating: 0x61, count: ExecUploadCommands.chunkBytes * 2 + 10)
        try body.write(to: local)
        let host = BorrowedHost(ssh)
        try await host.uploadRemoteFile(from: local, to: "/work/notes.txt", progress: nil)
        #expect(ssh.contents("/work/notes.txt") == body)
        await #expect(throws: HostConnectionError.nameTaken) {
            try await host.uploadRemoteFile(from: local, to: "/work/notes.txt", progress: nil)
        }

        ssh.sftpAvailable = false
        let plan = RemoteFileEdits.upload(to: "/work/again.txt")
        ssh.commands = { command in
            command == plan.begin || command == plan.finish
                ? .init(stdout: Data((RemoteFileEdits.okMarker + "\n").utf8)) : .init()
        }
        let seen = Fractions()
        try await host.uploadRemoteFile(from: local, to: "/work/again.txt") { seen.add($0) }
        let sent = ssh.ran.drop { $0 != plan.begin }
        #expect(sent.first == plan.begin)
        #expect(sent.last == plan.finish)
        #expect(sent.count == 2 + 3, "three chunks of 48 KiB between begin and finish")
        #expect(seen.values.last == 1)
    }

    // MARK: Uploads

    @Test("an image upload goes into the uploads directory, with progress")
    func imageUpload() async throws {
        let ssh = FakeLentSSH()
        let host = BorrowedHost(ssh)
        let seen = Fractions()
        let path = try await host.uploadToUploadsDirectory(Data("img".utf8), named: "IMG-1.jpg") { seen.add($0) }
        #expect(path == "/home/dev/.moshpit/uploads/IMG-1.jpg")
        #expect(ssh.uploads[path] == Data("img".utf8))
        #expect(seen.values.count == 8 && seen.values.last == 1)
    }

    // MARK: Never waiting on the wire

    @Test("a cancelled transfer returns at once, and the core is told to stop")
    func cancelledTransfer() async throws {
        let ssh = FakeLentSSH()
        ssh.delay = .milliseconds(400)
        let host = BorrowedHost(ssh)
        let started = ContinuousClock.now
        let upload = Task { try await host.uploadToUploadsDirectory(Data("big".utf8), named: "IMG-2.jpg", progress: nil) }
        try await Task.sleep(for: .milliseconds(50))
        upload.cancel()
        await #expect(throws: CancellationError.self) { try await upload.value }
        #expect(ContinuousClock.now - started < .milliseconds(300), "the task didn't wait for the core")
        // The core's next progress report is answered "stop".
        try await Task.sleep(for: .milliseconds(500))
        #expect(ssh.progressAnswers.first == false)
        #expect(ssh.uploads.isEmpty)
    }

    /// B243: the core never heard a cancellation — a cancelled stat or
    /// listing ran to its end there, a transfer to its next progress
    /// report. Each call is bound to a token of its own, which the task's
    /// cancellation trips; one that finished leaves its token alone.
    @Test("a cancelled call trips its own token, which the core gives the call up on")
    func cancelledCallTripsItsToken() async throws {
        let ssh = FakeLentSSH()
        ssh.folder("/srv")
        ssh.hang("/srv/slow")
        let host = BorrowedHost(ssh)
        _ = try await host.remoteStat("/srv")
        #expect(ssh.cancels.count == 1)
        #expect(ssh.cancels.first?.isCancelled() == false, "a finished call's token was tripped")

        let started = ContinuousClock.now
        let stat = Task { try await host.remoteStat("/srv/slow") }
        let list = Task { try await host.listRemoteDirectory("/srv/slow") }
        try await Task.sleep(for: .milliseconds(100))
        stat.cancel()
        list.cancel()
        await #expect(throws: CancellationError.self) { try await stat.value }
        await #expect(throws: CancellationError.self) { try await list.value }
        #expect(ContinuousClock.now - started < .milliseconds(600))
        #expect(ssh.cancels.count == 3)
        #expect(ssh.cancels.dropFirst().allSatisfy { $0.isCancelled() }, "the core wasn't told")
    }

    @Test("a call that doesn't answer in time is given up on, with the error asked for")
    func timeout() async throws {
        let started = ContinuousClock.now
        await #expect(throws: HostConnectionError.notAnswering) {
            try await awaitingCore(timeout: (0.1, HostConnectionError.notAnswering)) {
                try await Task.sleep(for: .seconds(5))
                return 1
            }
        }
        #expect(ContinuousClock.now - started < .seconds(1))
        #expect(try await awaitingCore(timeout: (2, HostConnectionError.notAnswering)) { 7 } == 7)
    }
}

/// Progress values, from any thread.
final class Fractions: @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [Double] = []
    func add(_ value: Double) { lock.lock(); seen.append(value); lock.unlock() }
    var values: [Double] { lock.lock(); defer { lock.unlock() }; return seen }
}
