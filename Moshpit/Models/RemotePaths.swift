import Foundation

// The remote-path model (W6) is the shared Rust one
// (`crates/app/src/remote_paths.rs`, through crates/ffi/src/app.rs), the
// same Android runs: what a path is worth, the probe that asks the host
// about the paths a terminal shows, the link a confirmed path carries, a
// folder's listing, and the exec forms of reading and changing files. The
// enums below are this app's names for it, so the services and views that
// call them read as before. What stays Swift: the candidate a terminal's
// path pass hands the probe (``TerminalPathScanner/Candidate``; the
// scanner is the terminal core's, `crates/term/src/paths.rs`), and the
// windowed SFTP reads (``WindowedRead``, ``InOrderBytes``), which are this
// app's SFTP client's async loops.

// MARK: - What a tap can open

/// Files a tap on a terminal path opens in place — what QuickLook shows on
/// iOS without help (the shared model's list). Source code is deliberately
/// not there: Claude Code mentions dozens of source paths per turn.
enum PreviewableFile {
    typealias Kind = RemoteFileKind

    /// Every extension worth a link, without its dot.
    static let extensions: [String] = remoteFileExtensions()

    /// The kind a path's extension names, case-insensitively. nil for
    /// anything QuickLook wouldn't show (or that isn't worth a link).
    static func kind(of path: String) -> Kind? {
        remoteFileKind(path: path)
    }
}

// MARK: - Finding paths in terminal text

/// The runs of a terminal line that could name a file or folder on the
/// host — candidates only, found by the terminal core
/// (`crates/term/src/paths.rs`; the Swift scanner went with SwiftTerm,
/// batch B). Nothing here knows whether a path EXISTS; `PathProbe` asks the
/// host, and only what it confirms ever becomes a link, which is what keeps
/// prose like "s45/s55" or "and/or" from turning blue.
enum TerminalPathScanner {
    struct Candidate: Equatable, Sendable {
        /// The path as printed (`~/…`, `/…`, `./…`, relative, or a bare
        /// file name).
        let token: String
        /// Column offset of the first character within the scanned line.
        let offset: Int
        let length: Int
        /// True when the token can only be worth a link as a FOLDER: its
        /// last component names no previewable file. The probe then accepts
        /// it only if it is a directory holding something previewable.
        let directoryOnly: Bool
    }
}

// MARK: - Asking the host

/// Asks the host which candidates are files (or folders holding something
/// previewable), below the working directory too (the shared model's
/// probe script).
enum PathProbe {
    struct Hit: Equatable, Sendable {
        let path: String
        let isDirectory: Bool
    }

    /// The full command line for one batch. `cwd` nil = unknown: relative
    /// paths then resolve against `$HOME` and nothing is searched for.
    static func command(cwd: String?, candidates: [TerminalPathScanner.Candidate]) -> String {
        remotePathProbeCommand(cwd: cwd, candidates: candidates.map {
            PathCandidateRecord(token: $0.token, directoryOnly: $0.directoryOnly)
        })
    }

    /// The probe's `index<TAB>f|d<TAB>path` lines, back onto the batch.
    static func parse(_ output: String, count: Int) -> [Int: Hit] {
        var hits: [Int: Hit] = [:]
        for (index, hit) in remotePathParseProbe(output: output, count: UInt32(clamping: max(0, count))) {
            hits[Int(index)] = Hit(path: hit.path, isDirectory: hit.isDirectory)
        }
        return hits
    }

    /// Lexical cleanup of what the shell built (`/repo/./out/`, `//`).
    static func tidy(_ path: String) -> String {
        remotePathTidy(path: path)
    }
}

// MARK: - The link a tap carries

/// What a confirmed path is tagged with on the terminal cells, and what a
/// tap hands back: a private scheme only this app reads.
enum RemotePathLink {
    static let scheme = "moshpit-preview"

    static func url(for path: String) -> String {
        remotePathLink(path: path)
    }

    /// The host path a tapped link names, or nil if it isn't one of ours or
    /// a `file://` link.
    static func path(from link: String) -> String? {
        remotePathFromLink(link: link)
    }
}

// MARK: - Reading what a link points at

/// One file or folder on the host, as the preview needs it.
struct RemoteFileEntry: Sendable, Hashable, Identifiable {
    let path: String
    let isDirectory: Bool
    /// nil when the host didn't say (the exec fallback lists names only).
    let size: UInt64?
    let modified: Date?

    var id: String { path }
    var name: String { (path as NSString).lastPathComponent }
    var kind: PreviewableFile.Kind? { isDirectory ? nil : PreviewableFile.kind(of: name) }
    /// Images and video — what a folder shows as thumbnails.
    var isMedia: Bool { kind == .image || kind == .video }

    /// The folders enclosing `path`, nearest first, down to `/`.
    static func enclosingFolders(of path: String) -> [String] {
        remotePathEnclosingFolders(path: path)
    }

    /// What a folder view shows: subfolders and previewable files, no dot
    /// files, newest first; undated entries keep their order, last.
    static func browsable(_ entries: [RemoteFileEntry]) -> [RemoteFileEntry] {
        remoteFilesBrowsable(entries: entries.map(RemoteFileRecord.init)).map(RemoteFileEntry.init)
    }
}

extension RemoteFileRecord {
    init(_ e: RemoteFileEntry) {
        self.init(path: e.path, isDirectory: e.isDirectory, size: e.size,
                  modified: e.modified?.timeIntervalSince1970)
    }
}

extension RemoteFileEntry {
    init(_ r: RemoteFileRecord) {
        self.init(path: r.path, isDirectory: r.isDirectory, size: r.size,
                  modified: r.modified.map(Date.init(timeIntervalSince1970:)))
    }
}

/// The exec-channel fallbacks for a host whose sshd has the SFTP subsystem
/// turned off (the shared model's commands and parsers).
enum RemoteFileCommands {
    /// The fallback download moves the file as base64 in ONE exec reply.
    static let execDownloadLimit: UInt64 = remoteFileExecDownloadLimit()

    static func stat(_ path: String) -> String { remoteFileStatCommand(path: path) }

    static func parseStat(_ output: String, path: String) -> RemoteFileEntry? {
        remoteFileParseStat(output: output, path: path).map(RemoteFileEntry.init)
    }

    static func list(_ path: String) -> String { remoteFileListCommand(path: path) }

    static func parseList(_ output: String, in folder: String) -> [RemoteFileEntry] {
        remoteFileParseList(output: output, folder: folder).map(RemoteFileEntry.init)
    }

    static func cat(_ path: String) -> String { remoteFileCatCommand(path: path) }
}

// MARK: - Pulling a file down

/// Reads a whole remote file through positioned reads, several in flight at
/// once. An SFTP server may answer a read with FEWER bytes than asked —
/// OpenSSH's caps every read at 255 KiB — and that is not the end of the
/// file: the rest of that range is asked for again. Only an empty answer
/// (EOF) ends the file early, e.g. one that shrank since it was sized.
/// Treating a short read as EOF is what cut a 1.3 MB PNG off after its
/// first strip.
enum WindowedRead {
    /// Pulls `size` bytes. `read(offset, length)` answers at most `length`
    /// bytes starting at `offset` (empty = EOF); `write(offset, bytes)` gets
    /// every piece at its place in the file, in whatever order they land.
    /// Returns how many bytes the file turned out to have. A `size` of 0
    /// (unknown) reads one range at a time until EOF.
    static func pull(size: UInt64, chunk: UInt32, window: Int,
                     read: @escaping @Sendable (UInt64, UInt32) async throws -> [UInt8],
                     write: (UInt64, [UInt8]) throws -> Void,
                     progress: ((UInt64) -> Void)? = nil) async throws -> UInt64 {
        precondition(chunk > 0 && window > 0)
        return try await withThrowingTaskGroup(of: (UInt64, UInt32, [UInt8]).self) { group in
            var next: UInt64 = 0
            var gaps: [(UInt64, UInt32)] = []
            var end: UInt64? = size > 0 ? size : nil
            var eof = false
            var received: UInt64 = 0
            var inFlight = 0

            func nextRange() -> (UInt64, UInt32)? {
                if let gap = gaps.popLast() { return gap }
                if let end {
                    guard next < end else { return nil }
                    let length = UInt32(min(UInt64(chunk), end - next))
                    defer { next += UInt64(length) }
                    return (next, length)
                }
                // Unknown size: one range at a time, so EOF is never raced.
                guard !eof, inFlight == 0 else { return nil }
                defer { next += UInt64(chunk) }
                return (next, chunk)
            }
            func fill() {
                while inFlight < window, let (offset, length) = nextRange() {
                    group.addTask { (offset, length, try await read(offset, length)) }
                    inFlight += 1
                }
            }

            fill()
            while let (offset, length, answer) = try await group.next() {
                inFlight -= 1
                try Task.checkCancellation()
                let bytes = answer.count > Int(length) ? Array(answer.prefix(Int(length))) : answer
                if bytes.isEmpty {
                    // EOF here: nothing at or past this offset exists.
                    eof = true
                    end = min(end ?? offset, offset)
                    gaps.removeAll { $0.0 >= offset }
                } else {
                    try write(offset, bytes)
                    received += UInt64(bytes.count)
                    progress?(received)
                    if bytes.count < Int(length) {
                        gaps.append((offset + UInt64(bytes.count), length - UInt32(bytes.count)))
                    }
                }
                fill()
            }
            return min(received, end ?? received)
        }
    }
}

/// Puts pieces that land out of order back in order. A player fed from
/// `WindowedRead` takes bytes only at the end of what it already has, while
/// the window's answers (and the gaps a short read leaves) arrive in any
/// order; this holds each piece until everything before it is in.
struct InOrderBytes {
    /// The offset of the next byte owed to the reader.
    private(set) var next: UInt64
    private var held: [UInt64: [UInt8]] = [:]

    init(from start: UInt64 = 0) { next = start }

    /// Takes the piece at `offset`; returns whatever is now contiguous from
    /// `next` (often nothing, sometimes several pieces at once).
    mutating func take(_ offset: UInt64, _ bytes: [UInt8]) -> [UInt8] {
        guard !bytes.isEmpty, offset >= next else { return [] }
        held[offset] = bytes
        var ready: [UInt8] = []
        while let piece = held.removeValue(forKey: next) {
            ready += piece
            next += UInt64(piece.count)
        }
        return ready
    }
}

// MARK: - Changing files on the host

/// The folder sheet's create / rename / delete / upload, as far as they are
/// names and command lines (the shared model's). SFTP does the work when
/// the host has it; these exec forms are the fallback.
enum RemoteFileEdits {
    /// Printed by an exec form that did what it was asked.
    static let okMarker = remoteFileEditMarkers()[0]
    /// Printed when the target name is already taken.
    static let takenMarker = remoteFileEditMarkers()[1]
    /// Printed when a delete would take the home folder (B816).
    static let homeMarker = remoteFileEditMarkers()[2]

    /// A name the user typed, trimmed; nil when it can't name one item in a
    /// folder (B141: a `/` or NUL among the scalars).
    static func validName(_ raw: String) -> String? { remoteFileValidName(raw: raw) }

    /// `name`, or the first Finder-style variant not in `taken`.
    static func uniqueName(_ name: String, taken: Set<String>) -> String {
        remoteFileUniqueName(name: name, taken: Array(taken))
    }

    static func child(_ name: String, of directory: String) -> String {
        remoteFileChild(name: name, directory: directory)
    }

    /// Absolute and at least two levels deep — never `/`, never `/usr`.
    static func deletable(_ path: String) -> Bool { remoteFileDeletable(path: path) }

    static func makeDirectory(_ path: String) -> String { remoteFileMakeDirectoryCommand(path: path) }

    static func rename(_ path: String, to newPath: String) -> String {
        remoteFileRenameCommand(path: path, newPath: newPath)
    }

    /// nil for a path `deletable` refuses.
    static func remove(_ path: String, isDirectory: Bool) -> String? {
        remoteFileRemoveCommand(path: path, isDirectory: isDirectory)
    }

    /// An upload over exec: base64 appended to a side file, decoded at the
    /// end. Refuses to start over an existing name.
    struct UploadPlan {
        let begin: String
        let append: (String) -> String
        let finish: String
        /// What an upload given up — stopped, or failed at any step — runs:
        /// its side files removed from the person's folder (B331).
        let abandon: String
    }

    static func upload(to path: String) -> UploadPlan {
        let plan = remoteFileUploadPlan(path: path)
        return UploadPlan(begin: plan.begin, append: { remoteFileUploadAppend(path: path, chunk: $0) },
                          finish: plan.finish, abandon: plan.abandon)
    }
}
