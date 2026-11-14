import Foundation

// The remote-path model (W6) is the shared Rust one
// (`crates/app/src/remote_paths.rs`, through crates/ffi/src/app.rs), the
// same Android runs: what a path is worth, the probe that asks the host
// about the paths a terminal shows, the link a confirmed path carries, a
// folder's listing, and the exec forms of reading and changing files. The
// enums below are this app's names for it, so the services and views that
// call them read as before. What stays Swift: the candidates the SwiftTerm
// view's lines yield (``TerminalPathScanner``; the new terminal view's are
// the terminal core's, `crates/term/src/paths.rs`), and the windowed SFTP
// reads (``WindowedRead``, ``InOrderBytes``), which are this app's SFTP
// client's async loops.

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

/// Picks out the runs of a terminal line that could name a file or folder
/// on the host — candidates only. Nothing here knows whether a path EXISTS;
/// `PathProbe` asks the host, and only what it confirms ever becomes a
/// link, which is what keeps prose like "s45/s55" or "and/or" from turning
/// blue.
///
/// Splits on anything that can't be part of a path, rather than matching a
/// path regex: a sentence-ending period, a full-width colon (Claude Code's
/// "对比图："), Markdown emphasis, quotes and parentheses all just end the
/// run. ASCII runs — a path glued to CJK prose ("保存到out/a.png") then
/// starts where the ASCII does. A CJK NAME inside a path
/// ("episodes/nolift/改稿建议.md") is picked back up by `wideNameTail`.
enum TerminalPathScanner {
    struct Candidate: Equatable, Sendable {
        /// The path as printed (`~/…`, `/…`, `./…`, relative, or a bare
        /// file name).
        let token: String
        /// Character offset of the first character within the scanned line.
        /// `Terminal.viewportLineText` yields one Character per column, so
        /// this is also the column offset.
        let offset: Int
        let length: Int
        /// True when the token can only be worth a link as a FOLDER: its
        /// last component names no previewable file. The probe then accepts
        /// it only if it is a directory holding something previewable.
        let directoryOnly: Bool
    }

    static func scan(_ line: String) -> [Candidate] {
        let characters = Array(line)
        var found: [Candidate] = []
        var index = 0
        while index < characters.count {
            guard isPathCharacter(characters[index]) else {
                index += 1
                continue
            }
            let start = index
            while index < characters.count, isPathCharacter(characters[index]) { index += 1 }
            let run = String(characters[start..<index])
            let tail = run.contains("/") ? wideNameTail(characters, from: index) : nil
            let body = run + (tail?.name ?? "")
            let end = index + (tail?.columns ?? 0)
            // Longest first: the link pass tags the first one the host
            // confirms, and the shorter ones then start inside its link.
            var variants: [Candidate] = []
            for headStart in wideHeadStarts(characters, before: start, run: run) {
                let head = String(characters[headStart..<start].filter { $0 != "\0" })
                if let candidate = classify(head + body, at: headStart, columns: end - headStart) {
                    variants.append(candidate)
                }
            }
            if tail != nil, let candidate = classify(body, at: start, columns: end - start) {
                variants.append(candidate)
            }
            if tail != nil, !variants.isEmpty {
                found += variants
                index = end
                continue
            }
            if let candidate = classify(run, at: start) { variants.append(candidate) }
            found += variants
        }
        return found
    }

    /// How far back a CJK name glued to a path is followed.
    static let headLimit = 16

    /// Where a path may start before its ASCII run, when the run opens on a
    /// separator glued to a CJK name: `成品` + `/西游.mp4`, `粗剪` + `.mp4`,
    /// `写实版-粗剪` + `.mp4`. Chinese prose runs into the name with no space
    /// ("已经输出到成品/a.mp4"), so the start can't be read off the text:
    /// every ideograph back to the nearest break is offered, farthest
    /// (longest path) first, and the host settles which exists. A run that
    /// opens on a letter ("保存到out/a.png") gets none — that is prose
    /// meeting a path, not a CJK name.
    private static func wideHeadStarts(_ characters: [Character], before start: Int, run: String) -> [Int] {
        guard let first = run.first, "/.-_".contains(first) else { return [] }
        var starts: [Int] = []
        var index = start
        while index > 0, starts.count < headLimit {
            let c = characters[index - 1]
            if c == "\0" {
                // A wide character's second column.
                guard index >= 2, !characters[index - 2].isASCII else { break }
            } else if isWideNameCharacter(c) {
                starts.append(index - 1)
            } else if !"-_.".contains(c) {
                break
            }
            index -= 1
        }
        return starts.reversed()
    }

    /// The CJK-named rest of a path whose ASCII run stopped at an ideograph:
    /// `episodes/nolift/` + `改稿建议.md`, `out/` + `羽毛球/a.png`. Taken only
    /// when it ends the way a path does — a previewable file or a `/` — so
    /// prose that merely follows a path ("保存到out/a.png了", "在 out/里面")
    /// stays prose. A wide character's second column reads as "\0" in the
    /// row text: dropped from the name, still counted in the columns.
    private static func wideNameTail(_ characters: [Character], from start: Int) -> (name: String, columns: Int)? {
        guard start < characters.count, isWideNameCharacter(characters[start]) else { return nil }
        var end = start
        while end < characters.count {
            let c = characters[end]
            if c == "\0" {
                guard end > start, !characters[end - 1].isASCII else { break }
            } else if !isPathCharacter(c) && !isWideNameCharacter(c) {
                break
            }
            end += 1
        }
        while end > start, characters[end - 1] == "." || characters[end - 1] == "-" { end -= 1 }
        let name = String(characters[start..<end].filter { $0 != "\0" })
        let last = name.split(separator: "/").last.map(String.init) ?? ""
        guard name.hasSuffix("/") || PreviewableFile.kind(of: last) != nil else { return nil }
        return (name, end - start)
    }

    private static func isWideNameCharacter(_ c: Character) -> Bool {
        !c.isASCII && (c.isLetter || c.isNumber)
    }

    private static func isPathCharacter(_ c: Character) -> Bool {
        guard c.isASCII else { return false }
        if c.isLetter || c.isNumber { return true }
        return "._@+-/~".contains(c)
    }

    /// `columns`: how many row columns `raw` spans, when that isn't its
    /// character count (a wide name's "\0" columns are not in `raw`).
    private static func classify(_ raw: String, at start: Int, columns: Int? = nil) -> Candidate? {
        // A sentence or list that ends on the path ("saved to out/a.png.")
        // leaves its punctuation on the run.
        var token = Substring(raw)
        while let last = token.last, last == "." || last == "-" { token = token.dropLast() }
        guard !token.isEmpty, token.count <= 1024 else { return nil }
        // `//` is a URL's remains (`https:` already split off) or a typo;
        // `~` anywhere but a leading `~/` is not a path at all.
        guard !token.contains("//") else { return nil }
        if let tilde = token.firstIndex(of: "~") {
            guard tilde == token.startIndex, token.hasPrefix("~/"),
                  token.dropFirst().firstIndex(of: "~") == nil else { return nil }
        }

        let path = String(token)
        let endsWithSlash = path.hasSuffix("/")
        let lastComponent = path.split(separator: "/").last.map(String.init) ?? ""
        guard !lastComponent.isEmpty, lastComponent != ".", lastComponent != ".." else { return nil }

        let previewable = !endsWithSlash && PreviewableFile.kind(of: lastComponent) != nil
        // An extension with no name in front (".mp4"): what is left of a
        // CJK file name once the ideographs are cut off, never a file.
        if previewable, lastComponent.hasPrefix("."), !lastComponent.dropFirst().contains(".") { return nil }
        if !path.contains("/") {
            // A bare word is only a candidate when it names a previewable
            // file ("01_character_sheet.png").
            guard previewable else { return nil }
        } else if !previewable {
            // Folder candidates: no extension, or a trailing slash. Anything
            // else with an extension (`src/main.swift`) is a file we don't
            // preview — not a folder.
            guard endsWithSlash || !lastComponent.contains(".") else { return nil }
            // Dates and fractions ("10/18", "1/2").
            let segments = path.split(separator: "/")
            guard !segments.allSatisfy({ $0.allSatisfy(\.isNumber) }) else { return nil }
            // A single rooted word is a slash command far more often than a
            // folder of outputs ("/config", "/help").
            if path.hasPrefix("/"), segments.count < 2 { return nil }
        }
        return Candidate(token: path, offset: start, length: (columns ?? raw.count) - (raw.count - token.count),
                         directoryOnly: !previewable)
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
