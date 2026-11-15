import Foundation

/// The share extension's hand-off to the app: image bytes plus their target,
/// dropped into the App Group container. The extension only WRITES — it has
/// no SSH, no sessions, and a ~120MB ceiling — and the app DRAINS the queue
/// whenever a matching session is (or becomes) live: process through the
/// normal pipeline, upload, deliver the path to the queued pane.
///
/// Lives in MoshpitKit because the app and the MoshpitShare extension both
/// touch it — the extension writes, the app drains.
///
/// One file per entry (`entries/<id>.json`, written by an atomic rename), so
/// no writer ever reads and rewrites another's (B269). The queue was one
/// index, `queue.json`, that the extension (and the Shortcuts action)
/// appended to and the app's drain rewrote — read, change, write, with no
/// coordination between the processes: a share made while a drain removed
/// or updated an entry lost one of the two writes. The new entry vanished,
/// and its images stayed in the App Group for good (or a delivered entry
/// came back with its images already gone). Now an append writes only its
/// own new file, and the drain changes or deletes only the entry it holds.
/// What earlier builds left in `queue.json` is still read, and only the app
/// rewrites it, entry by entry, as it drains them; nothing writes it any
/// more otherwise.
/// One shared image as the share extension gets it, to be written into the
/// queue as it is ([`ShareQueueStore/write(_:)`]): nothing is decoded.
public enum SharedImage: Sendable {
    /// A file of the sharing app's (Photos, Files): copied.
    case file(URL)
    /// A file of our own made for this share (a copy taken while the
    /// provider's URL was valid): moved.
    case temporaryFile(URL)
    /// Bytes the sharing app handed over in memory (a screenshot, a web
    /// image): written.
    case data(Data)
}

public enum ShareQueue {
    public enum Failure: Error {
        /// Asked to queue a batch with no image in it.
        case noImages
        /// Asked to queue images another entry already has (B341).
        case alreadyQueued
    }

    public struct Entry: Codable, Identifiable, Equatable {
        public var id = UUID()
        public var connectionId: UUID
        public var paneId: String
        /// The agent's name at queue time ("claude", "codex") — picks the
        /// insert dialect on drain.
        public var agentName: String
        /// Human label for the app's pending-uploads UI ("claude · work").
        public var label: String
        /// Filenames inside ``imagesDirectory`` — bytes exactly as shared;
        /// the app runs the normal scale/strip pipeline on drain.
        public var files: [String]
        public var createdAt: Date
        /// Where each of ``files`` already went on the host, by file name:
        /// a delivery can fail after the upload (the pane is gone, the paste
        /// is refused), the entry stays queued, and the next drain delivers
        /// these paths again instead of uploading the images again — every
        /// retry left another copy on the host (B196). nil (and missing in
        /// what earlier builds wrote) until something is uploaded.
        public var uploaded: [String: String]?
        /// Why the drain held this back instead of delivering it (B622):
        /// its pane no longer runs the agent it was queued for. A held
        /// entry is not tried again; it waits for the person (another
        /// pane, or discard) and still ages out. nil (and missing in what
        /// earlier builds wrote) while it may still go.
        public var heldBack: HeldBack?

        public init(id: UUID = UUID(), connectionId: UUID, paneId: String, agentName: String,
                    label: String, files: [String], createdAt: Date,
                    uploaded: [String: String]? = nil, heldBack: HeldBack? = nil) {
            self.id = id; self.connectionId = connectionId; self.paneId = paneId
            self.agentName = agentName; self.label = label; self.files = files; self.createdAt = createdAt
            self.uploaded = uploaded; self.heldBack = heldBack
        }
    }

    /// A share the drain would not type into its pane (B622).
    public struct HeldBack: Codable, Equatable, Sendable {
        public enum Reason: String, Codable, Sendable {
            /// The pane is gone.
            case paneGone
            /// The agent exited: its pane runs a shell now.
            case agentExited
            /// The pane runs another program now.
            case anotherProgram
        }

        public var reason: Reason
        /// What runs in the pane now (the shell, the other program); nil
        /// when the pane is gone.
        public var now: String?
        public var at: Date

        public init(reason: Reason, now: String?, at: Date) {
            self.reason = reason; self.now = now; self.at = at
        }
    }

    public static var containerURL: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: AgentWidgetStore.appGroup)
    }

    public static var imagesDirectory: URL? {
        containerURL?.appendingPathComponent("ShareQueue", isDirectory: true)
    }

    /// The queue in the App Group.
    private static var store: ShareQueueStore? {
        imagesDirectory.map { ShareQueueStore(directory: $0) }
    }

    public static func load() -> [Entry] {
        store?.load() ?? []
    }

    /// Persist one shared batch: bytes to disk, then the entry's own file.
    /// Called from the Shortcuts action.
    public static func append(target: (connectionId: UUID, paneId: String),
                              agentName: String, label: String,
                              images: [Data]) throws {
        guard let store else { throw CocoaError(.fileNoSuchFile) }
        try store.append(target: target, agentName: agentName, label: label, images: images)
    }

    /// The share extension's loader: ``ShareQueueStore/stage(count:image:)``
    /// on the App Group's queue (nothing staged without one).
    public static func stage(count: Int, image: (Int) async throws -> SharedImage?) async -> [String] {
        guard let store else { return [] }
        return await store.stage(count: count, image: image)
    }

    /// The entry for images ``stage(count:image:)`` wrote.
    public static func append(target: (connectionId: UUID, paneId: String),
                              agentName: String, label: String,
                              files: [String]) throws {
        guard let store else { throw CocoaError(.fileNoSuchFile) }
        try store.append(target: target, agentName: agentName, label: label, files: files)
    }

    /// Drops staged images no entry will name (the share was cancelled).
    public static func discard(_ files: [String]) {
        store?.discard(files)
    }

    /// Drop a drained (or expired) entry and its bytes.
    public static func remove(_ id: UUID) {
        store?.remove(id)
    }

    /// Drop every entry queued for connection `connectionId`, and their
    /// images: the connection is being deleted (B343).
    @discardableResult
    public static func removeEntries(for connectionId: UUID) -> Int {
        store?.removeEntries(for: connectionId) ?? 0
    }

    /// Remember that `file` of entry `id` is on the host at `remotePath`
    /// (``Entry/uploaded``), as soon as it is: a drain cut short after one
    /// of several uploads keeps the ones that went. False when it couldn't
    /// be written (or the entry is gone).
    @discardableResult
    public static func recordUpload(_ id: UUID, file: String, remotePath: String) -> Bool {
        store?.recordUpload(id, file: file, remotePath: remotePath) ?? false
    }

    /// Hold entry `id` back (``Entry/heldBack``). True when it was not held
    /// before: the one time the person is told.
    @discardableResult
    public static func holdBack(_ id: UUID, _ held: HeldBack) -> Bool {
        store?.holdBack(id, held) ?? false
    }

    /// Images no entry names, older than `age`: what the lost writes of
    /// earlier builds left behind (B269). A share still being written has
    /// its images on disk a moment before its entry, and is far younger.
    public static func sweepOrphanedImages(olderThan age: TimeInterval) {
        store?.sweepOrphanedImages(olderThan: age)
    }

    /// The entry's files with their bytes, in order — each with where it
    /// already went on the host, if it did — read one at a time as they are
    /// iterated (B342).
    public static func images(for entry: Entry) -> ShareQueueImages {
        store?.images(for: entry) ?? ShareQueueImages(files: [], read: { _ in Data() })
    }

    public static func imageData(for entry: Entry) -> [Data] {
        images(for: entry).map(\.data)
    }
}

/// One share sheet's staged images, to the sheet's end (B344). The sheet
/// discarded what it had staged only on Cancel; swiped down, it left its
/// images in the queue's directory, named by no entry, until the orphan
/// sweep took them two days later — full-size photos in the App Group.
/// Now every way the sheet goes ends this, once: what no entry names is
/// discarded (a queued share's images are named, and stay). A sheet that
/// goes while its images are still being written discards them as they
/// arrive.
public final class ShareStaging {
    private let discard: ([String]) -> Void
    private var staged: [String] = []
    /// The sheet has gone.
    public private(set) var ended = false

    /// `discard`: ``ShareQueue/discard(_:)`` by default (tests give a
    /// store of their own).
    public init(discard: @escaping ([String]) -> Void = { ShareQueue.discard($0) }) {
        self.discard = discard
    }

    /// Staging wrote `names`. A sheet already gone discards them now.
    public func didStage(_ names: [String]) {
        staged = names
        if ended { discard(names) }
    }

    /// The sheet went, however it went: Cancel, Done, a swipe. Once.
    public func end() {
        guard !ended else { return }
        ended = true
        discard(staged)
    }
}

/// An entry's images, in order, each read only when the iteration reaches
/// it and let go when it moves on (B342). The drain used to read a whole
/// entry's originals before handling the first: ten full-size photos is
/// hundreds of megabytes, held in the app — maybe in the background, where
/// the system's limit is lower — for the whole drain. An image that can't
/// be read is left out, as before. ``count`` and ``isEmpty`` read nothing:
/// they say which of the files are there.
public struct ShareQueueImages: Sequence {
    public typealias Element = (file: String, data: Data, uploadedTo: String?)

    let files: [(file: String, url: URL, uploadedTo: String?)]
    let read: (URL) throws -> Data

    /// How many of the entry's files are there.
    public var count: Int {
        files.filter { FileManager.default.fileExists(atPath: $0.url.path) }.count
    }

    /// None of the entry's files is there.
    public var isEmpty: Bool { count == 0 }

    public func makeIterator() -> Iterator { Iterator(files: files[...], read: read) }

    public struct Iterator: IteratorProtocol {
        var files: ArraySlice<(file: String, url: URL, uploadedTo: String?)>
        let read: (URL) throws -> Data

        public mutating func next() -> Element? {
            while let next = files.popFirst() {
                if let data = try? read(next.url) { return (next.file, data, next.uploadedTo) }
            }
            return nil
        }
    }
}

/// The queue in one directory: the images, one file per entry under
/// `entries/`, and the single index earlier builds wrote (`queue.json`). See
/// ``ShareQueue`` for why each entry has a file of its own.
public struct ShareQueueStore: Sendable {
    public let directory: URL
    /// How an image's bytes are read (tests count the reads).
    private let read: @Sendable (URL) throws -> Data

    public init(directory: URL, read: @escaping @Sendable (URL) throws -> Data = { try Data(contentsOf: $0) }) {
        self.directory = directory
        self.read = read
    }

    private var entriesDirectory: URL {
        directory.appendingPathComponent("entries", isDirectory: true)
    }

    /// Where earlier builds kept every entry.
    private var legacyIndexURL: URL {
        directory.appendingPathComponent("queue.json", isDirectory: false)
    }

    private func entryURL(_ id: UUID) -> URL {
        entriesDirectory.appendingPathComponent(id.uuidString + ".json", isDirectory: false)
    }

    /// Every entry, oldest first.
    public func load() -> [ShareQueue.Entry] {
        var entries: [UUID: ShareQueue.Entry] = [:]
        for entry in loadLegacy() { entries[entry.id] = entry }
        let files = (try? FileManager.default.contentsOfDirectory(
            at: entriesDirectory, includingPropertiesForKeys: nil)) ?? []
        for file in files where file.pathExtension == "json" {
            // A file being renamed into place is either there whole or not
            // at all. One that can't be read now (a launch before the first
            // unlock) is left for later, never fatal.
            guard let data = try? Data(contentsOf: file) else { continue }
            guard let entry = try? JSONDecoder().decode(ShareQueue.Entry.self, from: data) else {
                // Read, and not an entry: it never will be one. It used to
                // be skipped on every load for good (B345); its images, named
                // by nothing now, go with the orphan sweep.
                Log.island.error("share queue: an entry file that isn't an entry was removed")
                try? FileManager.default.removeItem(at: file)
                continue
            }
            entries[entry.id] = entry
        }
        return entries.values.sorted {
            ($0.createdAt, $0.id.uuidString) < ($1.createdAt, $1.id.uuidString)
        }
    }

    public func append(target: (connectionId: UUID, paneId: String),
                       agentName: String, label: String, images: [Data]) throws {
        // A batch with no image in it is no share. The share sheet offered
        // its targets before the images had loaded, a tap queued an empty
        // entry and said "Queued for claude", and the drain dropped it for
        // having nothing to deliver: the picture was never sent anywhere.
        guard !images.isEmpty else { throw ShareQueue.Failure.noImages }
        let files = try images.map { try write(.data($0)) }
        try append(target: target, agentName: agentName, label: label, files: files)
    }

    /// The share extension's loader (B315): `count` shared images written
    /// into the queue one at a time. Image `index + 1` is asked for only
    /// once image `index` is on disk and let go, so the process holds at
    /// most one image at once whatever the batch: the extension is killed
    /// around 120 MB, and ten full-size originals read in together could
    /// pass it, the share dying without a word. An image that can't be read
    /// or written is left out. The names written, in order, for
    /// ``append(target:agentName:label:files:)`` (or ``discard(_:)``).
    public func stage(count: Int, image: (Int) async throws -> SharedImage?) async -> [String] {
        var names: [String] = []
        for index in 0..<count {
            guard let shared = try? await image(index) else { continue }
            if let name = try? write(shared) { names.append(name) }
        }
        return names
    }

    /// Puts one shared image in the queue's directory as it is (nothing
    /// decoded), named by no entry yet; its name.
    public func write(_ image: SharedImage) throws -> String {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let name = UUID().uuidString + ".img"
        let destination = directory.appendingPathComponent(name)
        switch image {
        case .file(let url):
            try FileManager.default.copyItem(at: url, to: destination)
        case .temporaryFile(let url):
            try FileManager.default.moveItem(at: url, to: destination)
        case .data(let data):
            try data.write(to: destination, options: .atomic)
        }
        // Dated now (B340): a copy or a move keeps the shared file's own
        // time — a photo from last week — and until a target is tapped the
        // image is named by no entry, so a drain's sweep of orphans older
        // than its age took a share still being made: "Queued for claude",
        // and nothing ever came.
        try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: destination.path)
        return name
    }

    /// The entry for images already written into the directory
    /// (``write(_:)``, ``stage(count:image:)``).
    ///
    /// An image belongs to one entry (B341): a target tapped twice before
    /// the sheet changed queued the same images under two entries, and the
    /// first one drained deleted the images the second still named (or both
    /// delivered them). Images another entry has are refused.
    public func append(target: (connectionId: UUID, paneId: String),
                       agentName: String, label: String, files: [String]) throws {
        guard !files.isEmpty else { throw ShareQueue.Failure.noImages }
        let named = Set(load().flatMap(\.files))
        guard named.isDisjoint(with: files) else { throw ShareQueue.Failure.alreadyQueued }
        try save(ShareQueue.Entry(connectionId: target.connectionId, paneId: target.paneId,
                                  agentName: agentName, label: label,
                                  files: files, createdAt: Date()))
    }

    /// Removes written images no entry names: a staged share that was
    /// cancelled. (What a killed extension leaves, the drain's sweep of
    /// orphans takes later.)
    public func discard(_ files: [String]) {
        let named = Set(load().flatMap(\.files))
        for file in files where !named.contains(file) {
            try? FileManager.default.removeItem(at: directory.appendingPathComponent(file))
        }
    }

    public func remove(_ id: UUID) {
        guard let entry = load().first(where: { $0.id == id }) else { return }
        for file in entry.files {
            try? FileManager.default.removeItem(at: directory.appendingPathComponent(file))
        }
        try? FileManager.default.removeItem(at: entryURL(id))
        dropLegacy(id)
    }

    /// Removes every entry queued for connection `connectionId`, with its
    /// images; how many (B343). A deleted connection's shares can never be
    /// delivered — the drain only takes an entry whose session is live — and
    /// they stayed in the App Group, photos included, until they aged out.
    @discardableResult
    public func removeEntries(for connectionId: UUID) -> Int {
        let doomed = load().filter { $0.connectionId == connectionId }
        for entry in doomed { remove(entry.id) }
        return doomed.count
    }

    @discardableResult
    public func recordUpload(_ id: UUID, file: String, remotePath: String) -> Bool {
        guard var entry = load().first(where: { $0.id == id }) else { return false }
        entry.uploaded = (entry.uploaded ?? [:]).merging([file: remotePath]) { _, new in new }
        // An entry of an earlier build's index moves to a file of its own.
        guard (try? save(entry)) != nil else { return false }
        dropLegacy(id)
        return true
    }

    /// Holds entry `id` back; true when it was not held before (B622).
    @discardableResult
    public func holdBack(_ id: UUID, _ held: ShareQueue.HeldBack) -> Bool {
        guard var entry = load().first(where: { $0.id == id }), entry.heldBack == nil else { return false }
        entry.heldBack = held
        guard (try? save(entry)) != nil else { return false }
        dropLegacy(id)
        return true
    }

    /// The entry's images, read as they are iterated (B342).
    public func images(for entry: ShareQueue.Entry) -> ShareQueueImages {
        ShareQueueImages(
            files: entry.files.map { (file: $0, url: directory.appendingPathComponent($0), uploadedTo: entry.uploaded?[$0]) },
            read: read)
    }

    public func sweepOrphanedImages(olderThan age: TimeInterval) {
        let named = Set(load().flatMap(\.files))
        let files = (try? FileManager.default.contentsOfDirectory(
            at: directory, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        let cutoff = Date().addingTimeInterval(-age)
        for file in files where file.pathExtension == "img" && !named.contains(file.lastPathComponent) {
            let modified = (try? file.resourceValues(forKeys: [.contentModificationDateKey]))?
                .contentModificationDate ?? .distantFuture
            if modified < cutoff { try? FileManager.default.removeItem(at: file) }
        }
    }

    // MARK: Files

    /// The entry's own file, replaced whole by a rename: a reader sees the
    /// old entry or the new one, never a torn one, and no other entry is
    /// read or written.
    private func save(_ entry: ShareQueue.Entry) throws {
        try FileManager.default.createDirectory(at: entriesDirectory, withIntermediateDirectories: true)
        try JSONEncoder().encode(entry).write(to: entryURL(entry.id), options: .atomic)
    }

    private func loadLegacy() -> [ShareQueue.Entry] {
        guard let data = try? Data(contentsOf: legacyIndexURL),
              let entries = try? JSONDecoder().decode([ShareQueue.Entry].self, from: data) else { return [] }
        return entries
    }

    /// Takes `id` out of the earlier builds' index: only the app's drain
    /// does this (one entry at a time, on the main actor), and nothing else
    /// writes that file any more. Gone when it is empty.
    private func dropLegacy(_ id: UUID) {
        var entries = loadLegacy()
        guard let index = entries.firstIndex(where: { $0.id == id }) else { return }
        entries.remove(at: index)
        if entries.isEmpty {
            try? FileManager.default.removeItem(at: legacyIndexURL)
        } else if let data = try? JSONEncoder().encode(entries) {
            try? data.write(to: legacyIndexURL, options: .atomic)
        }
    }
}
