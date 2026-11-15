import Darwin
import Foundation
import MoshpitKit
import Testing

/// B315: the share extension read every shared image (up to ten full-size
/// originals) into memory before writing any, and a share extension is
/// killed around 120 MB: a big multi-photo share could die without a word.
/// Its loader now writes them into the queue one at a time
/// (`ShareQueueStore.stage(count:image:)`), so it holds at most one.
@Suite("share extension: a batch is written one image at a time (B315)", .serialized)
struct ShareQueueStagingTests {
    private func scratch() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-staging-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// The images written into `directory`.
    private func written(in directory: URL) -> Int {
        ((try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? [])
            .filter { $0.hasSuffix(".img") }.count
    }

    /// The process's memory as the system counts it against its limit (the
    /// footprint jetsam reads).
    private func footprint() -> Int {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? Int(info.phys_footprint) : 0
    }

    @Test("each image is on disk before the next is asked for, in order")
    func oneAtATime() async throws {
        let directory = try scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        var writtenWhenAsked: [Int] = []
        let names = await store.stage(count: 10) { index in
            writtenWhenAsked.append(written(in: directory))
            return .data(Data("image \(index)".utf8))
        }
        #expect(writtenWhenAsked == Array(0..<10))
        #expect(names.count == 10)
        let contents = names.compactMap { try? String(contentsOf: directory.appendingPathComponent($0), encoding: .utf8) }
        #expect(contents == (0..<10).map { "image \($0)" })
    }

    @Test("ten large images: the loader holds about one at a time, never the batch")
    func boundedByOneImage() async throws {
        let directory = try scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        let size = 40 << 20
        let base = footprint()
        var grown: [Int] = []
        let names = await store.stage(count: 10) { index in
            // What the loader still holds of the images before this one.
            grown.append(footprint() - base)
            // A full-size original's worth, every page touched.
            return .data(Data(repeating: UInt8(index), count: size))
        }
        #expect(names.count == 10)
        let peak = grown.max() ?? 0
        let held = grown.map { $0 >> 20 }
        // Ten held together would be 360 MB by the last. One at a time stays
        // about one image (the allocator keeps a freed one counted until it
        // reuses it: 40-50 MB on the Mac) and does not grow with the batch.
        #expect(peak < 2 * size, "the loader held \(peak >> 20) MB when an image was asked for: \(held)")
        #expect(grown[9] - grown[1] < size / 2, "it grew with the batch: \(held) MB")
    }

    @Test("a shared file is copied as it is, a temporary one moved; nothing decoded")
    func files() async throws {
        let directory = try scratch()
        let source = try scratch()
        defer {
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: source)
        }
        let theirs = source.appendingPathComponent("IMG_0001.HEIC")
        let ours = source.appendingPathComponent("copy")
        try Data("not even an image".utf8).write(to: theirs)
        try Data("our copy".utf8).write(to: ours)
        let store = ShareQueueStore(directory: directory)
        let names = await store.stage(count: 2) { [.file(theirs), .temporaryFile(ours)][$0] }
        #expect(names.count == 2)
        #expect(FileManager.default.fileExists(atPath: theirs.path), "the sharing app's file stays")
        #expect(!FileManager.default.fileExists(atPath: ours.path), "our copy moved")
        let contents = names.compactMap { try? String(contentsOf: directory.appendingPathComponent($0), encoding: .utf8) }
        #expect(contents == ["not even an image", "our copy"])
    }

    @Test("an image that can't be read is left out; a tap gives the rest an entry, Cancel drops them")
    func entryAndDiscard() async throws {
        let directory = try scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        struct Unreadable: Error {}
        let names = await store.stage(count: 3) { index -> SharedImage? in
            if index == 1 { throw Unreadable() }
            return .data(Data("image \(index)".utf8))
        }
        #expect(names.count == 2)
        let connection = UUID()
        try store.append(target: (connection, "%1"), agentName: "claude", label: "claude · work", files: names)
        let entry = try #require(store.load().first { $0.connectionId == connection })
        #expect(entry.files == names)
        #expect(store.images(for: entry).map { String(decoding: $0.data, as: UTF8.self) } == ["image 0", "image 2"])

        let cancelled = await store.stage(count: 2) { .data(Data("cancelled \($0)".utf8)) }
        #expect(written(in: directory) == 4)
        store.discard(cancelled + names)
        #expect(written(in: directory) == 2, "the cancelled share's images go, the queued entry's stay")
        #expect(throws: ShareQueue.Failure.self) {
            try store.append(target: (connection, "%1"), agentName: "claude", label: "x", files: [])
        }
    }
}

/// B340: a photo shared from Photos or Files is copied into the queue with
/// its own modification time — last week's — and until a target is tapped
/// it is named by no entry. A drain meanwhile (its sweep of orphans older
/// than 48 h) took it: "Queued for claude", and nothing ever came. A staged
/// image is dated when it is staged.
@Suite("share extension: a share still being made survives the orphan sweep (B340)")
struct ShareQueueStagingAgeTests {
    @Test("an old photo staged now is not an old orphan")
    func stagedNow() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-age-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = FileManager.default.temporaryDirectory
            .appendingPathComponent("photo-\(UUID().uuidString).jpg")
        defer { try? FileManager.default.removeItem(at: source) }
        try Data([1, 2, 3]).write(to: source)
        try FileManager.default.setAttributes(
            [.modificationDate: Date().addingTimeInterval(-7 * 24 * 3600)], ofItemAtPath: source.path)
        let store = ShareQueueStore(directory: directory)
        let staged = try store.write(.file(source))
        store.sweepOrphanedImages(olderThan: 48 * 3600)
        #expect(FileManager.default.fileExists(atPath: directory.appendingPathComponent(staged).path),
                "the sweep took a share still being made")
        try store.append(target: (UUID(), "%1"), agentName: "claude", label: "claude · box", files: [staged])
        #expect(store.load().first?.files == [staged])
    }
}

/// B341: a target tapped twice before the share sheet changed queued the
/// same staged images under two entries; the first drained deleted the
/// images the second still named (or both delivered them). An image
/// belongs to one entry.
@Suite("share extension: an image is queued once (B341)")
struct ShareQueueOneEntryTests {
    @Test("the same staged images are refused for a second entry")
    func queuedOnce() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-once-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        let staged = [try store.write(.data(Data([1, 2, 3])))]
        let target = (connectionId: UUID(), paneId: "%1")
        try store.append(target: target, agentName: "claude", label: "claude · box", files: staged)
        #expect(throws: ShareQueue.Failure.alreadyQueued) {
            try store.append(target: target, agentName: "claude", label: "claude · box", files: staged)
        }
        #expect(store.load().count == 1)
        // Other images still queue.
        let more = [try store.write(.data(Data([4])))]
        try store.append(target: target, agentName: "claude", label: "claude · box", files: more)
        #expect(store.load().count == 2)
    }
}

/// B342: the drain read a whole entry's originals before handling the
/// first — ten full-size photos is hundreds of MB in the app, maybe in the
/// background. An entry's images are read one at a time as the drain's
/// loop reaches each, and let go as it moves on.
@Suite("share queue: the drain reads an entry's images one at a time (B342)", .serialized)
struct ShareQueueDrainReadTests {
    /// Counts the store's reads.
    final class Reads: @unchecked Sendable {
        private let lock = NSLock()
        private var n = 0
        var count: Int { lock.withLock { n } }
        func read(_ url: URL) throws -> Data {
            lock.withLock { n += 1 }
            return try Data(contentsOf: url)
        }
    }

    private func scratch() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("share-drain-\(UUID().uuidString)", isDirectory: true)
    }

    private func footprint() -> Int {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? Int(info.phys_footprint) : 0
    }

    @Test("image k is read only once the drain has handled the ones before it")
    func readAsReached() throws {
        let directory = scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let reads = Reads()
        let store = ShareQueueStore(directory: directory, read: { try reads.read($0) })
        let files = try (0..<5).map { try store.write(.data(Data("image \($0)".utf8))) }
        try store.append(target: (UUID(), "%1"), agentName: "claude", label: "claude · box", files: files)
        let entry = try #require(store.load().first)
        let images = store.images(for: entry)
        #expect(images.count == 5 && !images.isEmpty)
        #expect(reads.count == 0, "count and isEmpty read nothing")
        var readWhenHandled: [Int] = []
        var contents: [String] = []
        for image in images {
            readWhenHandled.append(reads.count)
            contents.append(String(decoding: image.data, as: UTF8.self))
        }
        #expect(readWhenHandled == [1, 2, 3, 4, 5])
        #expect(contents == (0..<5).map { "image \($0)" })
    }

    @Test("eight large images: the drain's loop holds about one at a time")
    func boundedByOneImage() throws {
        let directory = scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        let size = 24 << 20
        var files: [String] = []
        for index in 0..<8 {
            var data = Data(count: size)
            data[0] = UInt8(index)
            files.append(try store.write(.data(data)))
        }
        try store.append(target: (UUID(), "%1"), agentName: "claude", label: "claude · box", files: files)
        let entry = try #require(store.load().first)
        let base = footprint()
        var peak = 0
        for image in store.images(for: entry) {
            _ = image.data.reduce(0) { $0 &+ Int($1) }
            peak = max(peak, footprint() - base)
        }
        #expect(peak < 3 * size, "the drain held \(peak >> 20) MB for \(size >> 20) MB images")
    }
}

/// B344: the share sheet discarded what it had staged only on Cancel.
/// Swiped down, it left full-size photos in the App Group, named by no
/// entry, until the orphan sweep two days later. Every way the sheet goes
/// ends its staging, once.
@Suite("share extension: a sheet however it goes leaves nothing staged (B344)")
struct ShareStagingEndTests {
    private func store() -> (ShareQueueStore, URL) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-end-\(UUID().uuidString)", isDirectory: true)
        return (ShareQueueStore(directory: directory), directory)
    }

    private func exists(_ name: String, in directory: URL) -> Bool {
        FileManager.default.fileExists(atPath: directory.appendingPathComponent(name).path)
    }

    @Test("a sheet swiped away after staging discards its images")
    func endedAfterStaging() throws {
        let (store, directory) = store()
        defer { try? FileManager.default.removeItem(at: directory) }
        let staging = ShareStaging(discard: { store.discard($0) })
        let names = [try store.write(.data(Data([1]))), try store.write(.data(Data([2])))]
        staging.didStage(names)
        staging.end()
        #expect(names.allSatisfy { !exists($0, in: directory) })
    }

    @Test("a queued share's images stay when its sheet goes")
    func queuedImagesStay() throws {
        let (store, directory) = store()
        defer { try? FileManager.default.removeItem(at: directory) }
        let staging = ShareStaging(discard: { store.discard($0) })
        let names = [try store.write(.data(Data([1])))]
        staging.didStage(names)
        try store.append(target: (UUID(), "%1"), agentName: "claude", label: "claude · box", files: names)
        staging.end()
        #expect(exists(names[0], in: directory))
    }

    @Test("a sheet gone while still staging discards the images as they arrive")
    func endedWhileStaging() throws {
        let (store, directory) = store()
        defer { try? FileManager.default.removeItem(at: directory) }
        let staging = ShareStaging(discard: { store.discard($0) })
        staging.end()
        let names = [try store.write(.data(Data([1])))]
        staging.didStage(names)
        #expect(!exists(names[0], in: directory))
    }

    @Test("ending twice discards once")
    func endsOnce() {
        var calls = 0
        let staging = ShareStaging(discard: { _ in calls += 1 })
        staging.didStage(["a.img"])
        staging.end()
        staging.end()
        #expect(calls == 1 && staging.ended)
    }
}

/// B345: an entry file that was read but isn't an entry was skipped on
/// every load, for good. It is removed, with a log line; one that can't be
/// read now (a launch before the first unlock) is left for later.
@Suite("share queue: an entry file that isn't one is removed (B345)")
struct ShareQueueBadEntryTests {
    @Test("a torn entry goes, an unreadable one stays, a good one loads")
    func badEntries() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-bad-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        try store.append(target: (UUID(), "%1"), agentName: "claude", label: "good", images: [Data([1])])
        let entries = directory.appendingPathComponent("entries", isDirectory: true)
        let torn = entries.appendingPathComponent(UUID().uuidString + ".json")
        try Data("{\"id\": \"not a whole entry".utf8).write(to: torn)
        let unreadable = entries.appendingPathComponent(UUID().uuidString + ".json")
        try Data("{}".utf8).write(to: unreadable)
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: unreadable.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: unreadable.path) }

        #expect(store.load().map(\.label) == ["good"])
        #expect(!FileManager.default.fileExists(atPath: torn.path), "an entry file that isn't one stayed")
        #expect(FileManager.default.fileExists(atPath: unreadable.path), "a file that can't be read now went")
    }
}

@Suite("share queue: a share whose pane moved on is held back, and said once (B622)")
struct ShareQueueHeldBackTests {
    @Test("held once: the second hold says nothing new; images and upload kept")
    func heldOnce() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-held-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        try store.append(target: (UUID(), "%3"), agentName: "claude", label: "claude · work",
                         images: [Data([1])])
        let entry = try #require(store.load().first)
        #expect(store.recordUpload(entry.id, file: entry.files[0], remotePath: "/tmp/up.png"))
        let held = ShareQueue.HeldBack(reason: .anotherProgram, now: "vim", at: Date())

        #expect(store.holdBack(entry.id, held), "the first hold is the one to tell")
        #expect(!store.holdBack(entry.id, ShareQueue.HeldBack(reason: .paneGone, now: nil, at: Date())),
                "a held share was told again")
        let after = try #require(store.load().first)
        #expect(after.heldBack == held)
        #expect(after.uploaded == [entry.files[0]: "/tmp/up.png"])
        #expect(store.images(for: after).count == 1)
        #expect(!store.holdBack(UUID(), held), "an entry that isn't there was held")
    }

    @Test("an entry an earlier build wrote reads as not held")
    func earlierBuild() throws {
        let json = """
        {"id":"\(UUID().uuidString)","connectionId":"\(UUID().uuidString)","paneId":"%3",
         "agentName":"claude","label":"claude","files":["a.img"],"createdAt":0}
        """
        let entry = try JSONDecoder().decode(ShareQueue.Entry.self, from: Data(json.utf8))
        #expect(entry.heldBack == nil)
    }
}
