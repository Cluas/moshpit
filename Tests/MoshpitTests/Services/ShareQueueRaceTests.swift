import Foundation
import MoshpitKit
import Testing
@testable import Moshpit

/// B269: the share extension (and the Shortcuts action) appended to the App
/// Group's `queue.json` while the app's drain rewrote it — read, change,
/// write, no coordination between the processes. A share made during a
/// drain lost one of the two writes: the new entry vanished and its images
/// stayed in the App Group for good, or a delivered entry came back with its
/// images already gone. Two processes on this Mac, 40 shares during a drain
/// of 40: the old index kept 4–6 of the 40 and left 33–35 images no entry
/// named; one file per entry keeps all 40 and none.
@Suite("share queue: two writers at once (B269)", .serialized)
struct ShareQueueRaceTests {

    private func scratch() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("share-queue-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// The app's queue as the extension and the drain see it: the App Group
    /// itself, as the two processes share it, the entries of one connection
    /// of this test's own.
    @Test("a share made during a drain is kept, and what the drain removed stays removed")
    func sharesDuringADrain() throws {
        let connection = UUID()
        defer {
            for entry in ShareQueue.load() where entry.connectionId == connection { ShareQueue.remove(entry.id) }
        }
        for i in 0..<24 {
            try ShareQueue.append(target: (connection, "%\(i)"), agentName: "claude",
                                  label: "old-\(i)", images: [Data("old \(i)".utf8)])
        }
        let old = ShareQueue.load().filter { $0.connectionId == connection }
        try #require(old.count == 24)
        let directory = try #require(ShareQueue.imagesDirectory)

        // The extension shares while the app drains what was there: each
        // drained entry has its upload recorded, then goes.
        DispatchQueue.concurrentPerform(iterations: 2) { writer in
            if writer == 0 {
                for i in 0..<24 {
                    try? ShareQueue.append(target: (connection, "%n\(i)"), agentName: "claude",
                                           label: "new-\(i)", images: [Data("new \(i)".utf8)])
                }
            } else {
                for entry in old {
                    ShareQueue.recordUpload(entry.id, file: entry.files[0], remotePath: "/tmp/up-\(entry.id)")
                    ShareQueue.remove(entry.id)
                }
            }
        }

        let left = ShareQueue.load().filter { $0.connectionId == connection }
        #expect(left.map(\.label).sorted() == (0..<24).map { "new-\($0)" }.sorted(),
                "shares lost or drained entries back: \(left.map(\.label).sorted())")
        for entry in left {
            #expect(ShareQueue.images(for: entry).count == 1, "\(entry.label) lost its image")
        }
        for entry in old {
            #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent(entry.files[0]).path),
                    "\(entry.label)'s image outlived its entry")
        }
    }

    /// What 1.0.8 left in `queue.json` is read, and drained entry by entry:
    /// an upload moves the entry to a file of its own, a removal takes it out
    /// of the index, and the index goes once it is empty.
    @Test("entries an earlier build queued are read and drained")
    func earlierBuildsIndex() throws {
        let directory = try scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        let connection = UUID()
        let queued = (0..<3).map { i in
            ShareQueue.Entry(connectionId: connection, paneId: "%\(i)", agentName: "claude",
                             label: "1.0.8-\(i)", files: ["\(i).img"], createdAt: Date(timeIntervalSince1970: Double(i)))
        }
        for i in 0..<3 { try Data("bytes \(i)".utf8).write(to: directory.appendingPathComponent("\(i).img")) }
        let index = directory.appendingPathComponent("queue.json")
        try JSONEncoder().encode(queued).write(to: index)
        try store.append(target: (connection, "%9"), agentName: "codex", label: "new", images: [Data("n".utf8)])

        #expect(store.load().map(\.label) == ["1.0.8-0", "1.0.8-1", "1.0.8-2", "new"])
        #expect(store.recordUpload(queued[1].id, file: "1.img", remotePath: "/up/1"))
        #expect(store.load().first { $0.id == queued[1].id }?.uploaded == ["1.img": "/up/1"])
        store.remove(queued[0].id)
        #expect(store.load().map(\.label) == ["1.0.8-1", "1.0.8-2", "new"])
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("0.img").path))
        store.remove(queued[2].id)
        #expect(!FileManager.default.fileExists(atPath: index.path), "the emptied index stays behind")
        #expect(store.load().map(\.label) == ["1.0.8-1", "new"])
    }

    /// The images earlier builds' lost writes left behind are swept once
    /// they are old: a share still being written (its images on disk a
    /// moment before its entry) is young and stays, and so does every image
    /// an entry names.
    @Test("images no entry names are swept once old, never a share being written")
    func orphanedImagesAreSwept() throws {
        let directory = try scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        try store.append(target: (UUID(), "%0"), agentName: "claude", label: "kept", images: [Data("k".utf8)])
        let named = try #require(store.load().first?.files.first)
        let orphan = directory.appendingPathComponent("orphan.img")
        let young = directory.appendingPathComponent("young.img")
        try Data("o".utf8).write(to: orphan)
        try Data("y".utf8).write(to: young)
        let days = Date().addingTimeInterval(-3 * 24 * 3600)
        try FileManager.default.setAttributes([.modificationDate: days], ofItemAtPath: orphan.path)
        try FileManager.default.setAttributes([.modificationDate: days],
                                              ofItemAtPath: directory.appendingPathComponent(named).path)

        store.sweepOrphanedImages(olderThan: 48 * 3600)

        #expect(!FileManager.default.fileExists(atPath: orphan.path))
        #expect(FileManager.default.fileExists(atPath: young.path))
        #expect(FileManager.default.fileExists(atPath: directory.appendingPathComponent(named).path))
    }

    /// The share sheet offered its targets before the images had loaded. A
    /// tap meanwhile queued an entry with no image in it, the sheet said
    /// "Queued for claude", and the drain dropped the entry for having
    /// nothing to deliver.
    @Test("a batch with no image in it is refused, not queued")
    func emptyBatchIsRefused() throws {
        let directory = try scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = ShareQueueStore(directory: directory)
        #expect(throws: ShareQueue.Failure.self) {
            try store.append(target: (UUID(), "%0"), agentName: "claude", label: "empty", images: [])
        }
        #expect(store.load().isEmpty, "an entry with nothing in it was queued")
    }
}
