import Foundation

/// An exclusive `flock` on `<file>.lock` around one read-modify-write of an
/// App Group file: one at a time, across processes (the app and the
/// notification service) and across opens within one. The finishes and the
/// standing set take it, as their Rust copies do on the same lock file
/// (crates/seal `store::Lock`, B347). No lock (the lock file can't be
/// opened) runs `body` anyway, as before the lock existed: the worst it
/// costs is the race it is there to close.
enum AppGroupFileLock {
    static func around<T>(_ url: URL, _ body: () -> T) -> T {
        let path = url.appendingPathExtension("lock").path
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(),
                                                 withIntermediateDirectories: true)
        let fd = open(path, O_CREAT | O_RDWR, 0o600)
        guard fd >= 0 else { return body() }
        defer { close(fd) }
        guard flock(fd, LOCK_EX) == 0 else { return body() }
        defer { flock(fd, LOCK_UN) }
        return body()
    }

    /// ``around(_:_:)`` that waits `budget` at most (B636): nil when another
    /// holder kept the lock that long, and `body` didn't run. For what only
    /// refines a notification: the service has seconds in all, and goes on
    /// without it rather than late.
    static func attempt<T>(_ url: URL, budget: TimeInterval, _ body: () -> T) -> T? {
        let path = url.appendingPathExtension("lock").path
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(),
                                                 withIntermediateDirectories: true)
        let fd = open(path, O_CREAT | O_RDWR, 0o600)
        guard fd >= 0 else { return body() }
        defer { close(fd) }
        let deadline = Date().addingTimeInterval(budget)
        while flock(fd, LOCK_EX | LOCK_NB) != 0 {
            guard Date() < deadline else { return nil }
            usleep(10_000)
        }
        defer { flock(fd, LOCK_UN) }
        return body()
    }
}
