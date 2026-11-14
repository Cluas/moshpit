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
}
