import Foundation

/// The share extension's hand-off to the app: image bytes plus their target,
/// dropped into the App Group container. The extension only WRITES — it has
/// no SSH, no sessions, and a ~120MB ceiling — and the app DRAINS the queue
/// whenever a matching session is (or becomes) live: process through the
/// normal pipeline, upload, deliver the path to the queued pane.
///
/// Compiled into both the app and the MoshpitShare extension (this file
/// lives in Island/ for the same reason the widget bridge does).
enum ShareQueue {
    struct Entry: Codable, Identifiable, Equatable {
        var id = UUID()
        var connectionId: UUID
        var paneId: String
        /// The agent's name at queue time ("claude", "codex") — picks the
        /// insert dialect on drain.
        var agentName: String
        /// Human label for the app's pending-uploads UI ("claude · work").
        var label: String
        /// Filenames inside ``imagesDirectory`` — bytes exactly as shared;
        /// the app runs the normal scale/strip pipeline on drain.
        var files: [String]
        var createdAt: Date
    }

    static var containerURL: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: AgentWidgetStore.appGroup)
    }

    static var imagesDirectory: URL? {
        containerURL?.appendingPathComponent("ShareQueue", isDirectory: true)
    }

    private static var indexURL: URL? {
        imagesDirectory?.appendingPathComponent("queue.json", isDirectory: false)
    }

    static func load() -> [Entry] {
        guard let indexURL, let data = try? Data(contentsOf: indexURL),
              let entries = try? JSONDecoder().decode([Entry].self, from: data)
        else { return [] }
        return entries
    }

    /// Persist one shared batch: bytes to disk, entry to the index. Called
    /// from the extension. Atomic enough for the contention that actually
    /// exists (one human, one share sheet at a time).
    static func append(target: (connectionId: UUID, paneId: String),
                       agentName: String, label: String,
                       images: [Data]) throws {
        guard let imagesDirectory else { throw CocoaError(.fileNoSuchFile) }
        try FileManager.default.createDirectory(at: imagesDirectory,
                                                withIntermediateDirectories: true)
        var files: [String] = []
        for data in images {
            let name = UUID().uuidString + ".img"
            try data.write(to: imagesDirectory.appendingPathComponent(name),
                           options: .atomic)
            files.append(name)
        }
        var entries = load()
        entries.append(Entry(connectionId: target.connectionId, paneId: target.paneId,
                             agentName: agentName, label: label,
                             files: files, createdAt: Date()))
        try save(entries)
    }

    /// Drop a drained (or expired) entry and its bytes.
    static func remove(_ id: UUID) {
        var entries = load()
        guard let index = entries.firstIndex(where: { $0.id == id }) else { return }
        if let imagesDirectory {
            for file in entries[index].files {
                try? FileManager.default.removeItem(
                    at: imagesDirectory.appendingPathComponent(file))
            }
        }
        entries.remove(at: index)
        try? save(entries)
    }

    static func imageData(for entry: Entry) -> [Data] {
        guard let imagesDirectory else { return [] }
        return entry.files.compactMap {
            try? Data(contentsOf: imagesDirectory.appendingPathComponent($0))
        }
    }

    private static func save(_ entries: [Entry]) throws {
        guard let indexURL else { throw CocoaError(.fileNoSuchFile) }
        let data = try JSONEncoder().encode(entries)
        try data.write(to: indexURL, options: .atomic)
    }
}
