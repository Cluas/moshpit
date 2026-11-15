import Foundation

/// One borrow of a Rust session's own SSH connection (crates/ffi
/// `BorrowedSsh`) as a ``HostConnection``: what a side feature's call runs
/// on when its session is on the Rust core (W14b). It holds the connection,
/// not the session — when the session reconnects or ends it closes the
/// connection, and what is in flight here fails at once
/// (``HostConnectionError/notAnswering``); ``RustSideChannel`` borrows again
/// for the next call.
///
/// SFTP first. A host with the subsystem turned off gets the exec forms
/// (`RemoteFileCommands`, `RemoteFileEdits`), on exec channels of the same
/// connection — and only that host: SFTP saying a file isn't there, or isn't
/// this account's to read, is said so, not tried again over exec and
/// reported as a generic "couldn't reach the host".
///
/// A command's answer is its stdout, what the parsers read: stderr merged
/// in is something a parser can take for data. It succeeds only with exit
/// status 0 on a channel the host closed in order (B136). The exec forms of
/// an edit read their marker whatever the status: `RemoteFileEdits` prints
/// "taken" and exits 1, which a status check alone turns into a generic
/// failure.
///
/// A Swift task's cancellation doesn't reach a UniFFI future, so each call
/// goes through the connection bound to a token of its own
/// (`BorrowedSsh.cancellable`, a ``BorrowCancel``), which the task's
/// cancellation trips: the task gets `CancellationError` at once
/// (``awaitingCore(stop:_:)``) and the core gives the call up too (B243) —
/// a read in flight is no longer waited for, a transfer stops where it is
/// and lets go of what it holds on the host. Before, a cancelled stat or
/// listing ran to its end on the core's runtime, and a transfer to its next
/// progress report.
final class BorrowedHost: HostConnection {
    let ssh: any BorrowedSshProtocol

    init(_ ssh: any BorrowedSshProtocol) {
        self.ssh = ssh
    }

    // MARK: Commands

    func executeCommand(_ command: String) async throws -> Data {
        let output = try await core { try await $0.run(command: command) }
        return try Self.succeeded(output)
    }

    /// stdout of a command that exited 0 on a channel closed in order.
    static func succeeded(_ output: ExecOutput) throws -> Data {
        // Not closed in order: the connection went away mid-command.
        guard output.finished else { throw HostConnectionError.notAnswering }
        guard let status = output.status else { throw HostConnectionError.endedWithoutStatus }
        guard status == 0 else { throw HostConnectionError.exited(status: Int(status)) }
        return output.stdout
    }

    // MARK: Reading

    func remoteStat(_ path: String) async throws -> RemoteFileEntry {
        if let entry = try await sftp({ try await $0.sftpStat(path: path) }) {
            return Self.entry(entry)
        }
        let output = try await executeCommand(RemoteFileCommands.stat(path))
        guard let entry = RemoteFileCommands.parseStat(String(decoding: output, as: UTF8.self), path: path)
        else { throw HostConnectionError.notFound }
        return entry
    }

    func openRemoteFile(_ path: String) async throws -> RemoteFileHandle {
        guard let file = try await sftp({ try await $0.sftpOpen(path: path) }) else {
            // Ranges need SFTP: the caller downloads instead.
            throw HostConnectionError.failed("no SFTP on this host to stream \(path) through")
        }
        return RemoteFileHandle(size: file.size(), read: { offset, length in
            [UInt8](try await Self.core { try await file.read(offset: offset, length: length) })
        }, close: {
            await file.close()
        })
    }

    func remoteHomeDirectory() async throws -> String {
        do {
            if let home = try await sftp({ try await $0.sftpHome() }), Self.isAbsolute(home) {
                return home
            }
        } catch HostConnectionError.notFound {
            // SFTP gave no absolute path for `.`: the shell is asked.
        }
        var home = String(decoding: try await executeCommand("pwd"), as: UTF8.self)
        // `pwd` ends the path with a newline, and only that is cut: a home
        // ending in a space is that directory (B57).
        if home.hasSuffix("\n") { home.removeLast() }
        guard Self.isAbsolute(home) else { throw HostConnectionError.notFound }
        return home
    }

    func listRemoteDirectory(_ path: String) async throws -> [RemoteFileEntry] {
        if let entries = try await sftp({ try await $0.sftpList(path: path) }) {
            return entries.map(Self.entry)
        }
        let output = try await executeCommand(RemoteFileCommands.list(path))
        return RemoteFileCommands.parseList(String(decoding: output, as: UTF8.self), in: path)
    }

    func downloadRemoteFile(_ path: String, to destination: URL,
                            progress: (@Sendable (Double) -> Void)?) async throws {
        let relay = ProgressRelay(progress)
        let copied = try await sftp(stop: { relay.stop() }) {
            try await $0.sftpDownload(path: path, destination: destination.path, progress: relay)
        }
        if copied != nil { return }
        // No SFTP: the whole file as one base64 reply, up to the limit.
        let entry = try await remoteStat(path)
        guard !entry.isDirectory else { throw HostConnectionError.notFound }
        guard (entry.size ?? 0) <= RemoteFileCommands.execDownloadLimit else {
            throw HostConnectionError.tooLargeWithoutSFTP
        }
        let output = try await executeCommand(RemoteFileCommands.cat(path))
        guard let data = Data(base64Encoded: output, options: .ignoreUnknownCharacters) else {
            throw HostConnectionError.notFound
        }
        try data.write(to: destination)
        progress?(1)
    }

    // MARK: Changing the folder

    func createRemoteDirectory(_ path: String) async throws {
        if try await sftp(writing: true, { try await $0.sftpCreateDir(path: path) }) != nil { return }
        try await edit(RemoteFileEdits.makeDirectory(path))
    }

    func renameRemoteItem(_ path: String, to newPath: String) async throws {
        if try await sftp(writing: true, { try await $0.sftpRename(from: path, to: newPath) }) != nil {
            return
        }
        try await edit(RemoteFileEdits.rename(path, to: newPath))
    }

    /// A file over SFTP; a folder, with everything in it, over `rm -rf` (SFTP
    /// removes only empty ones).
    func deleteRemoteItem(_ entry: RemoteFileEntry) async throws {
        guard let command = RemoteFileEdits.remove(entry.path, isDirectory: entry.isDirectory) else {
            throw HostConnectionError.writeDenied
        }
        if !entry.isDirectory,
           try await sftp(writing: true, { try await $0.sftpRemoveFile(path: entry.path) }) != nil {
            return
        }
        try await edit(command)
    }

    func uploadRemoteFile(from local: URL, to remotePath: String,
                          progress: (@Sendable (Double) -> Void)?) async throws {
        let relay = ProgressRelay(progress)
        let sent: Void? = try await sftp(writing: true, stop: { relay.stop() }) {
            try await $0.sftpUploadNew(local: local.path, remote: remotePath, progress: relay)
        }
        if sent != nil { return }
        let size = (try? FileManager.default.attributesOfItem(atPath: local.path)[.size] as? UInt64) ?? 0
        guard size <= RemoteFileCommands.execDownloadLimit else {
            throw HostConnectionError.tooLargeWithoutSFTP
        }
        let data = try Data(contentsOf: local)
        let plan = RemoteFileEdits.upload(to: remotePath)
        try await edit(plan.begin)
        do {
            var offset = 0
            while offset < data.count {
                try Task.checkCancellation()
                let end = min(offset + ExecUploadCommands.chunkBytes, data.count)
                _ = try await executeCommand(plan.append(data[offset..<end].base64EncodedString()))
                offset = end
                progress?(0.95 * Double(offset) / Double(max(data.count, 1)))
            }
            try await edit(plan.finish)
        } catch {
            // B331: given up or failed, it used to leave its half-file in the
            // person's folder for good. A stop answers at once and the
            // clearing follows; a failure waits for it.
            let clear = { [self] in _ = try? await self.executeCommand(plan.abandon) }
            if error is CancellationError {
                Task { await clear() }
            } else {
                await clear()
            }
            throw error
        }
        progress?(1)
    }

    // MARK: Uploads

    /// Into `~/.moshpit/uploads/` (directory 700, file 600): the core goes
    /// over SFTP, or over exec channels on a host without it, and remembers
    /// for this connection which one worked.
    func uploadToUploadsDirectory(_ data: Data, named filename: String,
                                  progress: (@Sendable (Double) -> Void)?) async throws -> String {
        let relay = ProgressRelay(progress)
        return try await core(writing: true, stop: { relay.stop() }) {
            try await $0.uploadToUploadsDirectory(data: data, filename: filename, progress: relay)
        }
    }

    // MARK: Calling the core

    /// `work`'s answer, on the connection bound to a token of its own that
    /// a cancellation (or a stop) trips; its failure in the app's words.
    private func core<T: Sendable>(writing: Bool = false, stop: (@Sendable () -> Void)? = nil,
                                   _ work: @escaping @Sendable (BorrowedSsh) async throws -> T) async throws -> T {
        do {
            return try await cancellable(stop: stop, work)
        } catch {
            throw Self.mapped(error, writing: writing)
        }
    }

    /// `work` on the connection bound to a fresh ``BorrowCancel``, tripped
    /// when the task is cancelled (or the call given up), with `stop`.
    private func cancellable<T: Sendable>(stop: (@Sendable () -> Void)?,
                                          _ work: @escaping @Sendable (BorrowedSsh) async throws -> T) async throws -> T {
        let cancel = BorrowCancel()
        let lent = ssh.cancellable(cancel: cancel)
        return try await awaitingCore(stop: {
            cancel.cancel()
            stop?()
        }) {
            try await work(lent)
        }
    }

    private static func core<T: Sendable>(_ work: @escaping @Sendable () async throws -> T) async throws -> T {
        do {
            return try await awaitingCore(stop: nil, work)
        } catch {
            throw mapped(error, writing: false)
        }
    }

    /// An SFTP call's answer, or nil when the host has no SFTP — the cue for
    /// the exec form.
    private func sftp<T: Sendable>(writing: Bool = false, stop: (@Sendable () -> Void)? = nil,
                                   _ work: @escaping @Sendable (BorrowedSsh) async throws -> T) async throws -> T? {
        do {
            return try await cancellable(stop: stop, work)
        } catch BorrowFailure.Sftp(why: .unavailable, path: _, log: _) {
            return nil
        } catch {
            throw Self.mapped(error, writing: writing)
        }
    }

    /// The core's failure as the app says it (anything else as it is).
    static func mapped(_ error: Error, writing: Bool) -> Error {
        guard let failure = error as? BorrowFailure else { return error }
        switch failure {
        case .NotConnected:
            return HostConnectionError.notConnected
        case .Cancelled:
            return CancellationError()
        case .Ssh(let cause, let log):
            switch cause {
            // The connection, not the request: closed under the call, a
            // channel that never answered, a side connection that couldn't
            // be made.
            case .plain, .channel(timedOut: true), .unreachable, .handshake, .proxy:
                return HostConnectionError.notAnswering
            default:
                return HostConnectionError.failed(log)
            }
        case .Sftp(let why, _, let log):
            switch why {
            case .notFound: return HostConnectionError.notFound
            case .permissionDenied: return writing ? HostConnectionError.writeDenied : HostConnectionError.readDenied
            case .nameTaken: return HostConnectionError.nameTaken
            case .refused: return writing ? HostConnectionError.writeDenied : HostConnectionError.failed(log)
            case .transport: return HostConnectionError.notAnswering
            case .unavailable, .local: return HostConnectionError.failed(log)
            }
        }
    }

    // MARK: Pieces

    /// An exec form of an edit (`RemoteFileEdits`): its marker decides,
    /// read before the exit status — "taken" comes with status 1.
    private func edit(_ command: String) async throws {
        let output = try await core(writing: true) { try await $0.run(command: command) }
        let said = String(decoding: output.stdout, as: UTF8.self)
        if said.contains(RemoteFileEdits.takenMarker) { throw HostConnectionError.nameTaken }
        guard output.finished else { throw HostConnectionError.notAnswering }
        guard output.status == 0, said.contains(RemoteFileEdits.okMarker) else {
            throw HostConnectionError.writeDenied
        }
    }

    static func entry(_ value: RemoteEntryValue) -> RemoteFileEntry {
        RemoteFileEntry(path: value.path, isDirectory: value.isDirectory, size: value.size,
                        modified: value.modified.map { Date(timeIntervalSince1970: TimeInterval($0)) })
    }

    /// The first byte is `/` (not the first Character: `/` and a combining
    /// mark after it are one, B39).
    private static func isAbsolute(_ path: String) -> Bool {
        path.utf8.first == UInt8(ascii: "/")
    }
}

// MARK: - Progress

/// A transfer's progress for the core (``TransferProgress``), called on the
/// core's threads: passed on to `report` until ``stop()``, after which the
/// core is told to stop at its next report.
final class ProgressRelay: TransferProgress, @unchecked Sendable {
    private let report: (@Sendable (Double) -> Void)?
    private let lock = NSLock()
    private var stopped = false

    init(_ report: (@Sendable (Double) -> Void)?) {
        self.report = report
    }

    func progress(fraction: Double) -> Bool {
        lock.lock()
        let going = !stopped
        lock.unlock()
        if going { report?(fraction) }
        return going
    }

    func stop() {
        lock.lock()
        stopped = true
        lock.unlock()
    }
}

// MARK: - Waiting on the core

/// Awaits `work` — a call into the core, which a Swift task can't cancel —
/// unless the task is cancelled first: then `stop` is asked (a transfer's
/// relay, so the core gives up at its next report) and `CancellationError`
/// thrown at once, while `work` runs on to its end unwatched. A cancelled
/// preview or upload never waits on the wire.
///
/// With `timeout`, `work` that hasn't answered within `seconds` is given up
/// on the same way, and `error` thrown.
func awaitingCore<T: Sendable>(stop: (@Sendable () -> Void)? = nil,
                               timeout: (seconds: Double, error: Error)? = nil,
                               _ work: @escaping @Sendable () async throws -> T) async throws -> T {
    try Task.checkCancellation()
    let gate = CoreGate<T>()
    return try await withTaskCancellationHandler {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<T, Error>) in
            gate.wait(continuation)
            Task {
                do {
                    gate.finish(.success(try await work()))
                } catch {
                    gate.finish(.failure(error))
                }
            }
            if let (seconds, error) = timeout {
                let failure = UncheckedError(error)
                Task {
                    try? await Task.sleep(nanoseconds: UInt64(max(0, seconds) * 1_000_000_000))
                    if gate.finish(.failure(failure.error)) { stop?() }
                }
            }
        }
    } onCancel: {
        stop?()
        gate.finish(.failure(CancellationError()))
    }
}

/// An error handed to the timeout's task (errors aren't all `Sendable`).
private struct UncheckedError: @unchecked Sendable {
    let error: Error
    init(_ error: Error) { self.error = error }
}

/// The first answer wins — the call's, or the cancellation's — whichever
/// side gets there before the continuation is in place.
private final class CoreGate<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<T, Error>?
    private var early: Result<T, Error>?
    private var answered = false

    func wait(_ continuation: CheckedContinuation<T, Error>) {
        lock.lock()
        if let early, !answered {
            answered = true
            lock.unlock()
            continuation.resume(with: early)
            return
        }
        self.continuation = continuation
        lock.unlock()
    }

    /// Whether this was the answer (the first).
    @discardableResult
    func finish(_ result: Result<T, Error>) -> Bool {
        lock.lock()
        guard !answered else {
            lock.unlock()
            return false
        }
        guard let continuation else {
            let first = early == nil
            if first { early = result }
            lock.unlock()
            return first
        }
        self.continuation = nil
        answered = true
        lock.unlock()
        continuation.resume(with: result)
        return true
    }
}
