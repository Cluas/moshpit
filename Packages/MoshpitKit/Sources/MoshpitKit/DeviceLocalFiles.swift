import Foundation

/// The App Group's folder for what holds of this device alone — the
/// standing set, the pane endings beside it (B636), the finish records
/// (crates/seal's files, which the app and the push extension keep through
/// Rust), the widget's snapshot (``AgentWidgetStore``) — kept out of backups
/// (B785). They rode a backup to a new phone: after a Quick Start, minutes
/// old, the old phone's standing entries held the 0→1 edge there for hours
/// (B727's six), and the widget showed the old phone's agents. A folder, not
/// each file: every save replaces its file (an atomic rename), and a file's
/// own exclusion went with the file it replaced.
///
/// What an earlier build kept at the container's root is deleted, not
/// carried in: on a phone restored from that build's backup it is the old
/// phone's, and on any other it is minutes of waiting state.
public enum DeviceLocalFiles {
    static let folderName = "device-only"

    private static let lock = NSLock()
    /// The files made ready this process (`<container>/<filename>`): the
    /// folder is set up and the root cleared once, not at every read.
    nonisolated(unsafe) private static var ready: Set<String> = []

    /// `filename` in the folder of the App Group's container; nil with no
    /// container (an unsigned build).
    public static func url(_ filename: String, appGroup: String = AppGroup.id) -> URL? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
        else { return nil }
        let key = container.path + "/" + filename
        lock.lock()
        defer { lock.unlock() }
        if ready.contains(key) {
            return container.appendingPathComponent(folderName, isDirectory: true)
                .appendingPathComponent(filename)
        }
        guard let url = url(filename, in: container) else { return nil }
        ready.insert(key)
        return url
    }

    /// `filename` in the folder of `container`: the folder made if it isn't
    /// there and kept out of backups, the file of that name at the root
    /// removed. Nil when the folder can't be made.
    public static func url(_ filename: String, in container: URL) -> URL? {
        let manager = FileManager.default
        var folder = container.appendingPathComponent(folderName, isDirectory: true)
        do {
            try manager.createDirectory(at: folder, withIntermediateDirectories: true)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try folder.setResourceValues(values)
        } catch {
            Log.push.error("device-only folder: \(error.localizedDescription, privacy: .public)")
            return nil
        }
        // An earlier build's copy, and what it kept beside it (`.lock`,
        // `.backup`): the old phone's state on a restored one.
        let siblings = (try? manager.contentsOfDirectory(atPath: container.path)) ?? []
        for name in siblings where name == filename || name.hasPrefix(filename + ".") {
            try? manager.removeItem(at: container.appendingPathComponent(name))
        }
        return folder.appendingPathComponent(filename)
    }
}
