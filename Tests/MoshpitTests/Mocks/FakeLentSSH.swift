import Foundation
@testable import Moshpit

/// A Rust session's lent connection (crates/ffi `BorrowedSsh`), faked for
/// W14b's side features: a little file system over "SFTP", a table of
/// commands over "exec", and a link's delay before every answer — never an
/// answer inside the call that asked, which no real link gives.
final class FakeLentSSH: BorrowedSsh, @unchecked Sendable {
    struct Answer {
        var stdout = Data()
        var stderr = Data()
        var status: UInt32? = 0
        var finished = true
    }

    private let lock = NSLock()
    private var state = State()

    private struct State {
        var delay: Duration = .milliseconds(12)
        var sftpAvailable = true
        var files: [String: Data] = [:]
        /// A size SFTP's stat reports in place of the file's own (a file
        /// past 50 MB without holding 50 MB).
        var sizes: [String: UInt64] = [:]
        var folders: Set<String> = []
        var denied: Set<String> = []
        var home = "/home/dev"
        var commands: @Sendable (String) -> Answer = { _ in Answer() }
        var failure: BorrowFailure?
        var hang: Set<String> = []
        var ran: [String] = []
        var sftpCalls: [String] = []
        var uploads: [String: Data] = [:]
        var progressAnswers: [Bool] = []
        var opened: [FakeLentFile] = []
        var cancels: [BorrowCancel] = []
        var downloadHolds: [Duration] = []
    }

    init() {
        super.init(noHandle: .init())
    }

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("a fake has no handle")
    }

    private func with<T>(_ body: (inout State) -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body(&state)
    }

    // MARK: Setting it up

    /// How long each answer takes (a loopback round trip by default).
    var delay: Duration {
        get { with { $0.delay } }
        set { with { $0.delay = newValue } }
    }

    /// false: the host has SFTP turned off — every SFTP call says so.
    var sftpAvailable: Bool {
        get { with { $0.sftpAvailable } }
        set { with { $0.sftpAvailable = newValue } }
    }

    var home: String {
        get { with { $0.home } }
        set { with { $0.home = newValue } }
    }

    /// Every call throws this (the connection gone under it).
    var failure: BorrowFailure? {
        get { with { $0.failure } }
        set { with { $0.failure = newValue } }
    }

    /// The exec channel's answers.
    var commands: @Sendable (String) -> Answer {
        get { with { $0.commands } }
        set { with { $0.commands = newValue } }
    }

    func file(_ path: String, _ data: Data) { with { $0.files[path] = data } }
    /// `path`'s stat says it is `size` bytes, whatever it holds.
    func reportSize(_ path: String, _ size: UInt64) { with { $0.sizes[path] = size } }
    func folder(_ path: String) { with { _ = $0.folders.insert(path) } }
    func deny(_ path: String) { with { _ = $0.denied.insert(path) } }
    /// Calls on `path` (or the command `path`) never answer.
    func hang(_ what: String) { with { _ = $0.hang.insert(what) } }
    /// Extra time the next downloads take once their destination is
    /// created, one per download in the order they're asked for.
    func holdDownloads(_ holds: Duration...) { with { $0.downloadHolds += holds } }

    // MARK: What happened

    var ran: [String] { with { $0.ran } }
    var sftpCalls: [String] { with { $0.sftpCalls } }
    var uploads: [String: Data] { with { $0.uploads } }
    /// What the progress callbacks answered, in order.
    var progressAnswers: [Bool] { with { $0.progressAnswers } }
    var opened: [FakeLentFile] { with { $0.opened } }
    /// The token each call was bound to (`cancellable`), in order: real
    /// ones, the core's own.
    var cancels: [BorrowCancel] { with { $0.cancels } }
    func contents(_ path: String) -> Data? { with { $0.files[path] } }

    // MARK: The link

    private func answer<T>(_ what: String, sftp: Bool, _ body: () throws -> T) async throws -> T {
        let (delay, hangs) = with { ($0.delay, $0.hang.contains(what)) }
        if hangs { try await Task.sleep(for: .seconds(3600)) }
        try await Task.sleep(for: delay)
        if let failure { throw failure }
        if sftp {
            let available = with { state -> Bool in
                state.sftpCalls.append(what)
                return state.sftpAvailable
            }
            guard available else {
                throw BorrowFailure.Sftp(why: .unavailable, path: what, log: "the host refused the sftp subsystem")
            }
        }
        return try body()
    }

    private func notFound(_ path: String) -> BorrowFailure {
        BorrowFailure.Sftp(why: .notFound, path: path, log: "no such file")
    }

    private func checked(_ path: String) throws {
        if with({ $0.denied.contains(path) }) {
            throw BorrowFailure.Sftp(why: .permissionDenied, path: path, log: "permission denied")
        }
    }

    private func report(_ progress: TransferProgress?, chunks: Int) async throws {
        for chunk in 1...chunks {
            try await Task.sleep(for: .milliseconds(5))
            let going = progress?.progress(fraction: Double(chunk) / Double(chunks)) ?? true
            with { $0.progressAnswers.append(going) }
            if !going { throw BorrowFailure.Cancelled }
        }
    }

    // MARK: BorrowedSsh

    override func isClosed() -> Bool { failure != nil }

    /// The fake is its own bound connection; it keeps the token to show
    /// whether the call's cancellation reached the core.
    override func cancellable(cancel: BorrowCancel) -> BorrowedSsh {
        with { $0.cancels.append(cancel) }
        return self
    }

    override func run(command: String) async throws -> ExecOutput {
        with { $0.ran.append(command) }
        let answer = try await self.answer(command, sftp: false) { commands(command) }
        return ExecOutput(stdout: answer.stdout, stderr: answer.stderr, status: answer.status,
                          finished: answer.finished)
    }

    override func sftpHome() async throws -> String {
        try await answer("~", sftp: true) { home }
    }

    override func sftpStat(path: String) async throws -> RemoteEntryValue {
        try await answer(path, sftp: true) {
            try checked(path)
            if with({ $0.folders.contains(path) }) {
                return RemoteEntryValue(path: path, isDirectory: true, size: 64, modified: 1_700_000_000)
            }
            guard let data = with({ $0.files[path] }) else { throw notFound(path) }
            return RemoteEntryValue(path: path, isDirectory: false,
                                    size: with({ $0.sizes[path] }) ?? UInt64(data.count),
                                    modified: 1_700_000_000)
        }
    }

    override func sftpList(path: String) async throws -> [RemoteEntryValue] {
        try await answer(path, sftp: true) {
            try checked(path)
            guard with({ $0.folders.contains(path) }) else { throw notFound(path) }
            let prefix = path.hasSuffix("/") ? path : path + "/"
            return with { state in
                let files = state.files.filter { $0.key.hasPrefix(prefix) && !$0.key.dropFirst(prefix.count).contains("/") }
                    .map { RemoteEntryValue(path: $0.key, isDirectory: false, size: UInt64($0.value.count),
                                            modified: 1_700_000_000) }
                let folders = state.folders.filter { $0.hasPrefix(prefix) && !$0.dropFirst(prefix.count).contains("/") }
                    .map { RemoteEntryValue(path: $0, isDirectory: true, size: 64, modified: 1_700_000_000) }
                return (files + folders).sorted { $0.path < $1.path }
            }
        }
    }

    /// As the core does it: the destination is created (or cut to nothing)
    /// once the file is open, and the bytes land in it as the reads answer.
    override func sftpDownload(path: String, destination: String, progress: TransferProgress?) async throws -> UInt64 {
        let hold = with { $0.downloadHolds.isEmpty ? Duration.zero : $0.downloadHolds.removeFirst() }
        let data = try await answer(path, sftp: true) { () -> Data in
            try checked(path)
            guard let data = with({ $0.files[path] }) else { throw notFound(path) }
            return data
        }
        let descriptor = Darwin.open(destination, O_WRONLY | O_CREAT | O_TRUNC, 0o644)
        guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        let out = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        try await Task.sleep(for: hold)
        try await report(progress, chunks: 8)
        try out.write(contentsOf: data)
        try out.close()
        return UInt64(data.count)
    }

    override func sftpOpen(path: String) async throws -> BorrowedFile {
        try await answer(path, sftp: true) {
            try checked(path)
            guard let data = with({ $0.files[path] }), !data.isEmpty else { throw notFound(path) }
            let file = FakeLentFile(data, delay: delay)
            with { $0.opened.append(file) }
            return file
        }
    }

    override func sftpRead(path: String, offset: UInt64, length: UInt32) async throws -> FileChunk {
        try await answer(path, sftp: true) {
            guard let data = with({ $0.files[path] }) else { throw notFound(path) }
            let start = Int(min(offset, UInt64(data.count)))
            let end = min(data.count, start + Int(length))
            return FileChunk(bytes: data.subdata(in: start..<end), size: UInt64(data.count))
        }
    }

    override func sftpCreateDir(path: String) async throws {
        try await answer(path, sftp: true) {
            try checked((path as NSString).deletingLastPathComponent)
            let taken = with { $0.folders.contains(path) || $0.files[path] != nil }
            if taken { throw BorrowFailure.Sftp(why: .refused, path: path, log: "failure") }
            with { _ = $0.folders.insert(path) }
        }
    }

    override func sftpRename(from: String, to: String) async throws {
        try await answer(from, sftp: true) {
            let taken = with { $0.folders.contains(to) || $0.files[to] != nil }
            if taken { throw BorrowFailure.Sftp(why: .nameTaken, path: to, log: "taken") }
            with { state in
                if let data = state.files.removeValue(forKey: from) { state.files[to] = data }
                if state.folders.remove(from) != nil { state.folders.insert(to) }
            }
        }
    }

    override func sftpRemoveFile(path: String) async throws {
        try await answer(path, sftp: true) {
            try checked(path)
            guard with({ $0.files.removeValue(forKey: path) }) != nil else { throw notFound(path) }
        }
    }

    override func sftpUploadNew(local: String, remote: String, progress: TransferProgress?) async throws {
        let data = try Data(contentsOf: URL(fileURLWithPath: local))
        try await answer(remote, sftp: true) {
            try checked((remote as NSString).deletingLastPathComponent)
            if with({ $0.files[remote] != nil }) {
                throw BorrowFailure.Sftp(why: .nameTaken, path: remote, log: "taken")
            }
        }
        try await report(progress, chunks: 4)
        with { $0.files[remote] = data }
    }

    override func sftpWrite(remote: String, data: Data, permissions: UInt32) async throws {
        try await answer(remote, sftp: true) { with { $0.files[remote] = data } }
    }

    override func uploadToUploadsDirectory(data: Data, filename: String, progress: TransferProgress?) async throws -> String {
        let path = "\(home)/.moshpit/uploads/\(filename)"
        try await answer(filename, sftp: false) {}
        try await report(progress, chunks: 8)
        with { $0.uploads[path] = data }
        return path
    }
}

/// A file the fake lent open (``FakeLentSSH/sftpOpen(path:)``).
final class FakeLentFile: BorrowedFile, @unchecked Sendable {
    let data: Data
    private let delay: Duration
    private let lock = NSLock()
    private var isClosed = false

    init(_ data: Data, delay: Duration) {
        self.data = data
        self.delay = delay
        super.init(noHandle: .init())
    }

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("a fake has no handle")
    }

    var closed: Bool {
        lock.lock()
        defer { lock.unlock() }
        return isClosed
    }

    override func size() -> UInt64 { UInt64(data.count) }

    override func read(offset: UInt64, length: UInt32) async throws -> Data {
        try await Task.sleep(for: delay)
        if closed { throw BorrowFailure.Sftp(why: .transport, path: "file", log: "closed") }
        let start = Int(min(offset, UInt64(data.count)))
        let end = min(data.count, start + Int(length))
        return data.subdata(in: start..<end)
    }

    override func close() async {
        lock.withLock { isClosed = true }
    }
}
