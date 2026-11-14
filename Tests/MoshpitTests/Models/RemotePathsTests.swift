import Foundation
import SwiftTerm
import Testing
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif
@testable import Moshpit

/// Path links: the scanner that finds candidates in agent prose, the probe
/// that asks the host which exist, the cache that batches those asks, and
/// the pass that tags confirmed paths on the terminal. The lines come from a
/// real transcript (2026-10-18): Claude Code reporting a comparison image as
/// a relative path after a full-width colon, and a codex prompt naming bare
/// file names it was told to write into the current directory.
@Suite("Remote path links")
struct RemotePathsTests {

    private func tokens(_ line: String) -> [String] {
        TerminalPathScanner.scan(line).map(\.token)
    }

    // MARK: Scanner

    @Test("the transcript's own line yields its image, its README and its CLAUDE.md")
    func transcriptLine() {
        let line = "  3. 对比图：qchar/out/GripLab.png（准备、放网、平抽）。规则见 CLAUDE.md 第 3 步和 qchar/README.md「体型怎么挑」。"
        let found = TerminalPathScanner.scan(line)
        #expect(found.map(\.token) == ["qchar/out/GripLab.png", "CLAUDE.md", "qchar/README.md"])
        #expect(found.allSatisfy { !$0.directoryOnly })
        let image = found[0]
        let start = line.index(line.startIndex, offsetBy: image.offset)
        #expect(String(line[start...].prefix(image.length)) == "qchar/out/GripLab.png")
    }

    @Test("sentence punctuation, emphasis and brackets end the path; they're not part of it")
    func punctuation() {
        #expect(tokens("saved to out/a.png.") == ["out/a.png"])
        #expect(tokens("**out/plot.png** and (charts/b.jpg), `c.gif`") == ["out/plot.png", "charts/b.jpg", "c.gif"])
        #expect(tokens("保存到out/a.png了") == ["out/a.png"])
        #expect(tokens("see src/main.swift:12 and out/a.png:3") == ["out/a.png"])
    }

    @Test("rooted, home and dot-relative paths are candidates; a bare word needs a previewable extension")
    func shapes() {
        #expect(tokens("/Users/me/Desktop/shot.PNG ~/Downloads/clip.mov ./a.pdf ../b.md")
                == ["/Users/me/Desktop/shot.PNG", "~/Downloads/clip.mov", "./a.pdf", "../b.md"])
        #expect(tokens("README Makefile notes.txt") == ["notes.txt"])
    }

    @Test("folders: no extension or a trailing slash, never a date, a fraction or a slash command")
    func folders() {
        let found = TerminalPathScanner.scan("outputs in out/plots and ~/Desktop/charts/ v1.2/ src/main.swift 10/18 1/2 /config")
        #expect(found.map(\.token) == ["out/plots", "~/Desktop/charts/", "v1.2/"])
        #expect(found.allSatisfy { $0.directoryOnly })
    }

    @Test("a CJK name inside a path is part of it; CJK prose after a path is not")
    func wideNames() {
        #expect(tokens("改稿：episodes/nolift/改稿建议.md。") == ["episodes/nolift/改稿建议.md"])
        #expect(tokens("见 out/羽毛球/a.png 和 out/图/，") == ["out/羽毛球/a.png", "out/图/"])
        #expect(tokens("在 out/里面有图") == ["out/"])
        #expect(tokens("保存到out/a.png了") == ["out/a.png"])
        // A row as the terminal hands it over: each wide character's second
        // column is a "\0" cell.
        let found = TerminalPathScanner.scan("看x/图\0表\0.png。\0")
        #expect(found.map(\.token) == ["x/图表.png"])
        #expect(found.first?.offset == 1)
        #expect(found.first?.length == 10)
    }

    @Test("a path that starts with a CJK name is offered from every possible start, longest first")
    func wideHeads() {
        // The 1.0.8 (1.8) miss: the run was only "/", so the path came out
        // rooted — "/西游…mp4" — and the host never confirmed it.
        let row = "for you: 成\0品\0/西\0游\0-写\0实\0版\0-粗\0剪\0.mp4. It's"
        let found = TerminalPathScanner.scan(row)
        #expect(found.map(\.token) == ["成品/西游-写实版-粗剪.mp4", "品/西游-写实版-粗剪.mp4", "/西游-写实版-粗剪.mp4"])
        #expect(found.first?.offset == 9)
        #expect(found.first?.length == 2 * 2 + 1 + 2 * 2 + 1 + 3 * 2 + 1 + 2 * 2 + 4, "every wide column, not the period")
        // A bare CJK file name, hyphens and all, back to the space.
        #expect(tokens("见 写实版-粗剪.mp4。") == ["写实版-粗剪.mp4", "实版-粗剪.mp4", "版-粗剪.mp4",
                                               "粗剪.mp4", "剪.mp4"], "a bare \".mp4\" is no file")
        // Prose run into the name: the host decides where the path starts.
        #expect(tokens("输出到成品/a.png").prefix(2) == ["输出到成品/a.png", "出到成品/a.png"])
        // A run opening on a letter is prose meeting a path, not a CJK name.
        #expect(tokens("保存到out/a.png了") == ["out/a.png"])
        #expect(tokens("规则见CLAUDE.md") == ["CLAUDE.md"])
    }

    @Test("URLs and other non-paths are left alone")
    func nonPaths() {
        #expect(tokens("https://example.com/a.png and file:///tmp/x.png") == [])
        #expect(tokens("a~b/c.png me@example.com") == [])
    }

    // MARK: Probe

    @Test("the probe's output maps back onto its batch; junk is ignored")
    func parse() {
        let out = "0\tf\t/repo/./out/a.png\n2\td\t/repo/plots/\n7\tf\t/nope\nbad line\n1\tx\t/repo/b\n3\tf\trelative\n"
        let hits = PathProbe.parse(out, count: 4)
        #expect(hits == [0: .init(path: "/repo/out/a.png", isDirectory: false),
                         2: .init(path: "/repo/plots", isDirectory: true)])
    }

    @Test("every candidate is passed as one quoted argument, flagged file or folder")
    func command() {
        let batch = TerminalPathScanner.scan("out/a.png out/plots")
        let cmd = PathProbe.command(cwd: "/Users/me/it's here", candidates: batch)
        #expect(cmd.hasPrefix("sh -c '"))
        #expect(cmd.hasSuffix(" 'Fout/a.png' 'Dout/plots'"))
        #expect(cmd.contains("'/Users/me/it'\\''s here'"))
    }

    /// B138: fish takes `\'` and `\\` as escapes inside single quotes, so a
    /// value ending in `\` — a working directory, a path an agent printed —
    /// left its quote open and the next value ran on unquoted. Every word of
    /// the probe now reads the same in fish as in sh.
    @Test("a value ending in a backslash stays one word in fish too")
    func probeWordsReadAlikeInFish() {
        let batch = TerminalPathScanner.scan(#"out\ out/a.png"#)
        let cmd = PathProbe.command(cwd: #"/Users/me/odd\"#, candidates: batch)
        let words = PosixShell.rawWords(cmd)
        #expect(words.count >= 6, "\(words)")
        for word in words {
            let plain = word.unicodeScalars.allSatisfy { $0 != "'" && $0 != "\\" } ? word : nil
            let posix = PosixShell.unquote(word) ?? plain
            #expect(posix != nil, "\(word)")
            #expect(PosixShell.unquoteAsFish(word) ?? plain == posix, "\(word.prefix(80))")
        }
        #expect(words.contains(Shell.quote(#"/Users/me/odd\"#)))
    }

    #if os(macOS)
    /// Runs the probe for real against a scratch tree, through the login
    /// shell a Mac host actually has (zsh) and through plain sh.
    @Test("the probe script resolves, searches below the working directory, and filters folders",
          arguments: ["/bin/zsh", "/bin/sh"])
    func probeRuns(shell: String) throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("probe-\(UUID().uuidString)").path
        defer { try? fm.removeItem(atPath: root) }
        let files = ["repo/qchar/out/GripLab.png", "repo/v2/01_sheet.png", "repo/plots/a.png",
                     "repo/src/main.swift", "repo/node_modules/x/y.png", "repo/out dir/c.png",
                     "repo/episodes/ep/build/concept/v2/02_poses.png", "repo/old/b.png", "repo/new/b.png"]
        for file in files {
            let path = root + "/" + file
            try fm.createDirectory(atPath: (path as NSString).deletingLastPathComponent,
                                   withIntermediateDirectories: true)
            fm.createFile(atPath: path, contents: Data([0x89]))
        }
        let past = Date(timeIntervalSinceNow: -3600)
        try fm.setAttributes([.modificationDate: past], ofItemAtPath: root + "/repo/old/b.png")
        let cwd = root + "/repo"
        let batch = TerminalPathScanner.scan(
            "qchar/out/GripLab.png 01_sheet.png plots/ src/ missing.png y.png ~/repo/plots/ \(cwd)/qchar/out/GripLab.png "
            + "build/concept/v2/ 02_poses.png b.png")
        #expect(batch.count == 11)

        func probe(cwd: String) throws -> [Int: PathProbe.Hit] {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: shell)
            process.arguments = ["-c", PathProbe.command(cwd: cwd, candidates: batch)]
            process.environment = ["HOME": root, "PATH": "/usr/bin:/bin"]
            let pipe = Pipe()
            process.standardOutput = pipe
            try process.run()
            process.waitUntilExit()
            let output = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            return PathProbe.parse(output, count: batch.count)
        }
        let hits = try probe(cwd: cwd)

        #expect(hits[0] == .init(path: cwd + "/qchar/out/GripLab.png", isDirectory: false))
        #expect(hits[1] == .init(path: cwd + "/v2/01_sheet.png", isDirectory: false),
                "a bare name the agent wrote after `cd v2` is found below the working directory")
        #expect(hits[2] == .init(path: cwd + "/plots", isDirectory: true))
        #expect(hits[3] == nil, "a folder with nothing previewable in it is not a link")
        #expect(hits[4] == nil)
        #expect(hits[5] == nil, "node_modules is never searched")
        #expect(hits[6] == .init(path: root + "/repo/plots", isDirectory: true))
        #expect(hits[7] == .init(path: cwd + "/qchar/out/GripLab.png", isDirectory: false))
        #expect(hits[8] == .init(path: cwd + "/episodes/ep/build/concept/v2", isDirectory: true),
                "a folder is searched for too")
        #expect(hits[9] == .init(path: cwd + "/episodes/ep/build/concept/v2/02_poses.png", isDirectory: false),
                "six levels down")
        #expect(hits[10] == .init(path: cwd + "/new/b.png", isDirectory: false), "the newest match wins")

        let fromHome = try probe(cwd: root)
        #expect(fromHome[1] == nil, "a pane in $HOME is never searched")
        #expect(fromHome[6] == .init(path: root + "/repo/plots", isDirectory: true))
    }

    @Test("a CJK-named path's starts are settled by the host, on folder boundaries",
          arguments: ["/bin/zsh", "/bin/sh"])
    func probeWideHeads(shell: String) throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("probe-\(UUID().uuidString)").path
        defer { try? fm.removeItem(atPath: root) }
        for file in ["repo/成品/西游-粗剪.mp4", "repo/episodes/ep/成品/粗剪.mp4"] {
            let path = root + "/" + file
            try fm.createDirectory(atPath: (path as NSString).deletingLastPathComponent,
                                   withIntermediateDirectories: true)
            fm.createFile(atPath: path, contents: Data([0x00]))
        }
        let cwd = root + "/repo"
        let batch = TerminalPathScanner.scan("输出到成品/西游-粗剪.mp4，已经成品/粗剪.mp4")
        let process = Process()
        process.executableURL = URL(fileURLWithPath: shell)
        process.arguments = ["-c", PathProbe.command(cwd: cwd, candidates: batch)]
        process.environment = ["HOME": root, "PATH": "/usr/bin:/bin"]
        let pipe = Pipe()
        process.standardOutput = pipe
        try process.run()
        process.waitUntilExit()
        let hits = PathProbe.parse(String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self),
                                   count: batch.count)
        let confirmed = batch.indices.compactMap { i in hits[i].map { (batch[i].token, $0.path) } }
        #expect(confirmed.map(\.0) == ["成品/西游-粗剪.mp4", "成品/粗剪.mp4"],
                "\"品/…\" never matches inside \"成品/…\"")
        #expect(confirmed.map(\.1) == [cwd + "/成品/西游-粗剪.mp4", cwd + "/episodes/ep/成品/粗剪.mp4"])
    }
    #endif

    // MARK: Folder entries and the exec fallbacks

    @Test("a folder lists subfolders and previewable files, newest first, dot files hidden")
    func browsable() {
        func entry(_ name: String, dir: Bool = false, at seconds: TimeInterval?) -> RemoteFileEntry {
            RemoteFileEntry(path: "/r/" + name, isDirectory: dir, size: 1,
                            modified: seconds.map { Date(timeIntervalSince1970: $0) })
        }
        let shown = RemoteFileEntry.browsable([
            entry("old.png", at: 10), entry("main.swift", at: 50), entry(".hidden.png", at: 60),
            entry("new.mp4", at: 40), entry("plots", dir: true, at: 20), entry("undated.pdf", at: nil),
        ])
        #expect(shown.map(\.name) == ["new.mp4", "plots", "old.png", "undated.pdf"])
        #expect(shown.filter(\.isMedia).map(\.name) == ["new.mp4", "old.png"], "the grid: images and video")
    }

    @Test("the title menu climbs from a folder to the root, nearest first")
    func enclosingFolders() {
        #expect(RemoteFileEntry.enclosingFolders(of: "/Users/c/code/x")
                == ["/Users/c/code", "/Users/c", "/Users", "/"])
        #expect(RemoteFileEntry.enclosingFolders(of: "/tmp/out/") == ["/tmp", "/"])
        #expect(RemoteFileEntry.enclosingFolders(of: "/").isEmpty)
        #expect(RemoteFileEntry.enclosingFolders(of: "build/out").isEmpty, "only absolute paths")
    }

    @Test("the fallback stat and listing parse what the shell prints")
    func fallbackParsing() {
        #expect(RemoteFileCommands.parseStat("d\n", path: "/r/plots")?.isDirectory == true)
        #expect(RemoteFileCommands.parseStat("f   1234\n", path: "/r/a.png")?.size == 1234)
        #expect(RemoteFileCommands.parseStat("", path: "/r/nope") == nil)
        // B333: NUL-separated `<d|f> <seconds|-> <name>` records, newest first.
        let listed = RemoteFileCommands.parseList(
            "f 1700000001 a.png\u{0}d 1700000002 sub\u{0}f 1700000003 b.png\u{0}", in: "/r/")
        let paths = listed.map(\.path)
        let kinds = listed.map(\.isDirectory)
        #expect(paths == ["/r/b.png", "/r/sub", "/r/a.png"])
        #expect(kinds == [false, true, false])
    }

    #if os(macOS)
    @Test("the fallback commands run against a real folder")
    func fallbackRuns() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("fallback-\(UUID().uuidString)/it's here").path
        defer { try? fm.removeItem(atPath: (root as NSString).deletingLastPathComponent) }
        try fm.createDirectory(atPath: root + "/sub", withIntermediateDirectories: true)
        fm.createFile(atPath: root + "/a.png", contents: Data([1, 2, 3, 4, 5]))

        func sh(_ command: String) throws -> String {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/bin/zsh")
            process.arguments = ["-c", command]
            let pipe = Pipe()
            process.standardOutput = pipe
            try process.run()
            process.waitUntilExit()
            return String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        }
        #expect(RemoteFileCommands.parseStat(try sh(RemoteFileCommands.stat(root + "/a.png")),
                                             path: root + "/a.png")?.size == 5)
        #expect(RemoteFileCommands.parseStat(try sh(RemoteFileCommands.stat(root)), path: root)?.isDirectory == true)
        #expect(Set(RemoteFileCommands.parseList(try sh(RemoteFileCommands.list(root)), in: root).map(\.name))
                == ["a.png", "sub"])
        let bytes = Data(base64Encoded: try sh(RemoteFileCommands.cat(root + "/a.png")),
                         options: .ignoreUnknownCharacters)
        #expect(bytes == Data([1, 2, 3, 4, 5]))
    }
    #endif

    // MARK: Link payload

    // MARK: Pulling a file down

    /// Runs `WindowedRead` against an in-memory "remote" file whose reads
    /// answer at most `cap(offset, length)` bytes, and returns what landed.
    private func pull(_ remote: [UInt8], claimedSize: UInt64, chunk: UInt32, window: Int,
                      cap: @escaping @Sendable (UInt64, UInt32) -> Int) async throws -> (count: UInt64, data: [UInt8]) {
        var sink: [UInt8] = []
        let count = try await WindowedRead.pull(
            size: claimedSize, chunk: chunk, window: window,
            read: { offset, length in
                guard offset < UInt64(remote.count) else { return [] }
                let start = Int(offset)
                let take = min(Int(length), cap(offset, length), remote.count - start)
                return Array(remote[start..<(start + take)])
            },
            write: { offset, bytes in
                let end = Int(offset) + bytes.count
                if sink.count < end { sink += [UInt8](repeating: 0, count: end - sink.count) }
                sink.replaceSubrange(Int(offset)..<end, with: bytes)
            })
        return (count, sink)
    }

    @Test("a read the server answers short is asked for again, not taken as the end of the file")
    func shortReads() async throws {
        let remote = (0..<1_349_374).map { UInt8(truncatingIfNeeded: $0 &* 31 &+ $0 >> 8) }
        // OpenSSH's sftp-server answers at most 255 KiB per read: the 1.0.8
        // (1.1) build asked for 256 KiB and kept only the first strip.
        let openSSH = try await pull(remote, claimedSize: UInt64(remote.count), chunk: 256 * 1024, window: 8,
                                     cap: { _, _ in 255 * 1024 })
        #expect(openSSH.count == UInt64(remote.count))
        #expect(openSSH.data == remote)
        // Irregular short answers, many in flight.
        let ragged = try await pull(remote, claimedSize: UInt64(remote.count), chunk: 64 * 1024, window: 16,
                                    cap: { offset, length in offset % 3 == 0 ? Int(length) : max(1, Int(length) / 3) })
        #expect(ragged.data == remote)
    }

    @Test("a file that shrank, or one of unknown size, ends at its real end")
    func endOfFile() async throws {
        let remote = (0..<150_000).map { UInt8(truncatingIfNeeded: $0 &* 7) }
        let shrank = try await pull(remote, claimedSize: 400_000, chunk: 64 * 1024, window: 8,
                                    cap: { _, length in Int(length) })
        #expect(shrank.count == 150_000)
        #expect(shrank.data == remote)
        let unknown = try await pull(remote, claimedSize: 0, chunk: 64 * 1024, window: 8,
                                     cap: { _, length in max(1, Int(length) - 1000) })
        #expect(unknown.count == 150_000)
        #expect(unknown.data == remote)
    }

    @Test("pieces that land out of order reach a player in order")
    func inOrder() {
        var order = InOrderBytes()
        #expect(order.take(4, [4, 5]).isEmpty, "held until what comes before it is in")
        #expect(order.take(0, [0, 1]) == [0, 1], "the next bytes go straight through")
        #expect(order.take(2, [2, 3]) == [2, 3, 4, 5], "a gap filling releases everything behind it")
        #expect(order.next == 6)
        #expect(order.take(0, [0]).isEmpty, "a piece already passed is ignored")
    }

    @Test("a streamed byte range arrives whole and in order through short, reordered reads")
    func streamedRange() async throws {
        let remote = (0..<1_200_000).map { UInt8(truncatingIfNeeded: $0 &* 13 &+ $0 >> 9) }
        let start: UInt64 = 300_001
        let end: UInt64 = 1_100_000
        var order = InOrderBytes()
        var played: [UInt8] = []
        _ = try await WindowedRead.pull(
            size: end - start, chunk: 64 * 1024, window: 16,
            read: { offset, length in
                // Later ranges answer first, and every third one short.
                try await Task.sleep(nanoseconds: UInt64(2_000_000 - Int(offset % 1_000_000) * 2))
                let from = Int(start + offset)
                let take = offset % 3 == 0 ? Int(length) / 2 : Int(length)
                return Array(remote[from..<min(from + take, remote.count)])
            },
            write: { offset, bytes in played += order.take(offset, bytes) })
        #expect(played.count == Int(end - start))
        #expect(played == Array(remote[Int(start)..<Int(end)]))
    }

    // MARK: Changing files

    @Test("names: typed names are trimmed and checked; uploads never overwrite; deletes need depth")
    func editNames() {
        #expect(RemoteFileEdits.validName("  plots ") == "plots")
        #expect(RemoteFileEdits.validName("") == nil)
        #expect(RemoteFileEdits.validName("..") == nil)
        #expect(RemoteFileEdits.validName("a/b") == nil)
        // B141: a `/` or NUL hidden in one character with a combining mark
        // after it still splits the name on the host.
        #expect(RemoteFileEdits.validName("a/\u{301}b") == nil)
        #expect(RemoteFileEdits.validName("x/\u{20DD}") == nil)
        #expect(RemoteFileEdits.validName("a\0\u{301}b") == nil)
        #expect(RemoteFileEdits.validName("e\u{301}t\u{E9}.md") == "e\u{301}t\u{E9}.md")
        let taken: Set = ["shot.png", "shot 2.png", "Makefile", "archive.tar.gz"]
        #expect(RemoteFileEdits.uniqueName("new.png", taken: taken) == "new.png")
        #expect(RemoteFileEdits.uniqueName("shot.png", taken: taken) == "shot 3.png")
        #expect(RemoteFileEdits.uniqueName("Makefile", taken: taken) == "Makefile 2")
        #expect(RemoteFileEdits.uniqueName("archive.tar.gz", taken: taken) == "archive.tar 2.gz")
        #expect(RemoteFileEdits.child("a.png", of: "/r/out") == "/r/out/a.png")
        #expect(RemoteFileEdits.child("a.png", of: "/r/out/") == "/r/out/a.png")
        #expect(RemoteFileEdits.deletable("/Users/me/out/a.png"))
        #expect(!RemoteFileEdits.deletable("/"))
        #expect(!RemoteFileEdits.deletable("/usr"))
        #expect(!RemoteFileEdits.deletable("relative/a.png"))
        #expect(!RemoteFileEdits.deletable("/Users/me/../.."))
        #expect(RemoteFileEdits.remove("/", isDirectory: true) == nil)
    }

    #if os(macOS)
    /// The exec fallbacks, run for real on a scratch tree through zsh and sh,
    /// with a quote and a space in the names.
    @Test("the exec edits create, rename, refuse a taken name, upload and delete",
          arguments: ["/bin/zsh", "/bin/sh"])
    func execEditsRun(shell: String) throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("edits-\(UUID().uuidString)").path
        try fm.createDirectory(atPath: root, withIntermediateDirectories: true)
        defer { try? fm.removeItem(atPath: root) }
        func run(_ command: String) throws -> String {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: shell)
            process.arguments = ["-c", command]
            process.environment = ["PATH": "/usr/bin:/bin"]
            let pipe = Pipe()
            process.standardOutput = pipe
            try process.run()
            process.waitUntilExit()
            return String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        }
        let folder = root + "/it's plots"
        #expect(try run(RemoteFileEdits.makeDirectory(folder)).contains(RemoteFileEdits.okMarker))
        #expect(fm.fileExists(atPath: folder))

        let payload = Data((0..<200_000).map { UInt8(truncatingIfNeeded: $0 &* 13) })
        let file = folder + "/a b.png"
        let plan = RemoteFileEdits.upload(to: file)
        #expect(try run(plan.begin).contains(RemoteFileEdits.okMarker))
        var offset = 0
        while offset < payload.count {
            let end = min(offset + 48 * 1024, payload.count)
            _ = try run(plan.append(payload[offset..<end].base64EncodedString()))
            offset = end
        }
        #expect(try run(plan.finish).contains(RemoteFileEdits.okMarker))
        #expect(fm.contents(atPath: file) == payload)
        #expect(!fm.fileExists(atPath: file + ".b64part"))
        #expect(try run(RemoteFileEdits.upload(to: file).begin).contains(RemoteFileEdits.takenMarker),
                "an upload never starts over an existing file")

        fm.createFile(atPath: folder + "/taken.png", contents: Data([1]))
        #expect(try run(RemoteFileEdits.rename(file, to: folder + "/taken.png")).contains(RemoteFileEdits.takenMarker))
        #expect(fm.contents(atPath: folder + "/taken.png") == Data([1]), "a rename never overwrites")
        #expect(try run(RemoteFileEdits.rename(file, to: folder + "/c.png")).contains(RemoteFileEdits.okMarker))
        #expect(fm.contents(atPath: folder + "/c.png") == payload)

        #expect(try run(RemoteFileEdits.remove(folder + "/c.png", isDirectory: false)!).contains(RemoteFileEdits.okMarker))
        #expect(!fm.fileExists(atPath: folder + "/c.png"))
        #expect(try run(RemoteFileEdits.remove(folder, isDirectory: true)!).contains(RemoteFileEdits.okMarker))
        #expect(!fm.fileExists(atPath: folder))
    }
    #endif

    @Test("a host path survives the round trip through the link payload")
    func linkRoundTrip() {
        for path in ["/Users/me/out/a.png", "/tmp/with space/图.png", "/a?b#c.png"] {
            #expect(RemotePathLink.path(from: RemotePathLink.url(for: path)) == path)
        }
        #expect(RemotePathLink.path(from: "file:///Users/me/out/a.png") == "/Users/me/out/a.png")
        #expect(RemotePathLink.path(from: "file://mac.local/Users/me/a.png") == "/Users/me/a.png")
        #expect(RemotePathLink.path(from: "https://example.com/a.png") == nil)
    }
}

// MARK: - Cache and link pass

@MainActor
@Suite("Remote path links — cache and tagging")
struct RemotePathLinksTests {

    /// A probe that answers from a fixed table and counts its calls.
    final class FakeProbe {
        var table: [String: PathProbe.Hit] = [:]
        var calls: [[String]] = []
        var cwds: [String?] = []
        func run(_ cwd: String?, _ batch: [TerminalPathScanner.Candidate]) -> [Int: PathProbe.Hit] {
            calls.append(batch.map(\.token))
            cwds.append(cwd)
            var hits: [Int: PathProbe.Hit] = [:]
            for (i, c) in batch.enumerated() { hits[i] = table[c.token] }
            return hits
        }
    }

    private func candidate(_ token: String) -> TerminalPathScanner.Candidate {
        TerminalPathScanner.scan(token)[0]
    }

    private func settle(_ links: RemotePathLinks) async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            links.onResolved = { c.resume() }
        }
    }

    @Test("unknown candidates queue up and go to the host in one batch")
    func batches() async {
        let fake = FakeProbe()
        fake.table = ["out/a.png": .init(path: "/r/out/a.png", isDirectory: false)]
        let links = RemotePathLinks(workingDirectory: { "/r" }, probe: { fake.run($0, $1) })
        #expect(links.resolvedPath(for: candidate("out/a.png")) == nil)
        #expect(links.resolvedPath(for: candidate("out/b.png")) == nil)
        await settle(links)
        #expect(fake.calls == [["out/a.png", "out/b.png"]])
        #expect(fake.cwds == ["/r"])
        #expect(links.resolvedPath(for: candidate("out/a.png")) == "/r/out/a.png")
        #expect(links.resolvedPath(for: candidate("out/b.png")) == nil)
        #expect(fake.calls.count == 1, "a fresh miss is not asked about again right away")
    }

    @Test("a miss is asked about again once the retry window has passed")
    func missRetries() async throws {
        var clock = Date(timeIntervalSince1970: 1000)
        let fake = FakeProbe()
        let links = RemotePathLinks(workingDirectory: { "/r" }, probe: { fake.run($0, $1) },
                                    now: { clock })
        _ = links.resolvedPath(for: candidate("out/a.png"))
        try await Task.sleep(for: .milliseconds(600))
        #expect(fake.calls.count == 1)
        fake.table = ["out/a.png": .init(path: "/r/out/a.png", isDirectory: false)]
        clock += RemotePathLinks.missRetry + 1
        #expect(links.resolvedPath(for: candidate("out/a.png")) == nil)
        await settle(links)
        #expect(links.resolvedPath(for: candidate("out/a.png")) == "/r/out/a.png")
    }

    private func makeTerminal() -> Terminal {
        TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 700)).getTerminal()
    }

    private func link(_ t: Terminal, row: Int, col: Int) -> String? {
        t.link(at: .screen(Position(col: col, row: row)), mode: .explicitOnly)
    }

    @Test("a confirmed path behind CJK prose is tagged on exactly its own columns")
    func tagsTerminal() async {
        let t = makeTerminal()
        let prefix = "  3. 对比图："
        let probeTerminal = makeTerminal()
        probeTerminal.feed(text: prefix)
        let prefixCols = probeTerminal.buffer.x
        t.feed(text: prefix + "qchar/out/GripLab.png（准备）\r\nsee https://example.com/x.png\r\n")

        let fake = FakeProbe()
        fake.table = ["qchar/out/GripLab.png": .init(path: "/r/qchar/out/GripLab.png", isDirectory: false)]
        let links = RemotePathLinks(workingDirectory: { "/r" }, probe: { fake.run($0, $1) })
        PlainLinkDetector.linkify(terminal: t)
        PathLinkDetector.linkify(terminal: t, resolver: links)
        #expect(link(t, row: 0, col: prefixCols) == nil, "nothing is tagged before the host answers")
        await settle(links)
        PathLinkDetector.linkify(terminal: t, resolver: links)

        let expected = RemotePathLink.url(for: "/r/qchar/out/GripLab.png")
        #expect(link(t, row: 0, col: prefixCols) == expected)
        #expect(link(t, row: 0, col: prefixCols + "qchar/out/GripLab.png".count - 1) == expected)
        #expect(link(t, row: 0, col: prefixCols - 1) == nil)
        #expect(link(t, row: 0, col: prefixCols + "qchar/out/GripLab.png".count) == nil)
        #expect(link(t, row: 1, col: 4) == "https://example.com/x.png", "the URL keeps its own link")
        #expect(fake.calls.flatMap { $0 }.contains("example.com/x.png") == false)
    }

    @Test("a path with a CJK name is tagged across its wide columns")
    func tagsWideName() async {
        let t = makeTerminal()
        t.feed(text: "改稿：episodes/nolift/改稿建议.md。\r\n")
        let fake = FakeProbe()
        fake.table = ["episodes/nolift/改稿建议.md": .init(path: "/r/episodes/nolift/改稿建议.md", isDirectory: false)]
        let links = RemotePathLinks(workingDirectory: { "/r" }, probe: { fake.run($0, $1) })
        PathLinkDetector.linkify(terminal: t, resolver: links)
        await settle(links)
        PathLinkDetector.linkify(terminal: t, resolver: links)

        let expected = RemotePathLink.url(for: "/r/episodes/nolift/改稿建议.md")
        let first = 6, last = 6 + "episodes/nolift/".count + 4 * 2 + ".md".count - 1
        #expect(link(t, row: 0, col: first - 1) == nil)
        #expect(link(t, row: 0, col: first) == expected)
        #expect(link(t, row: 0, col: last) == expected)
        #expect(link(t, row: 0, col: last + 1) == nil, "the full-width period is not part of it")
    }

    @Test("a CJK-named path is tagged from the start the host confirmed")
    func tagsWideHead() async {
        let t = makeTerminal()
        t.feed(text: "已经输出到成品/粗剪.mp4。\r\n")
        let fake = FakeProbe()
        fake.table = ["成品/粗剪.mp4": .init(path: "/r/成品/粗剪.mp4", isDirectory: false)]
        let links = RemotePathLinks(workingDirectory: { "/r" }, probe: { fake.run($0, $1) })
        PathLinkDetector.linkify(terminal: t, resolver: links)
        await settle(links)
        PathLinkDetector.linkify(terminal: t, resolver: links)

        let expected = RemotePathLink.url(for: "/r/成品/粗剪.mp4")
        let first = 5 * 2, last = first + 2 * 2 + 1 + 2 * 2 + ".mp4".count - 1
        #expect(link(t, row: 0, col: first - 1) == nil, "the prose before it stays prose")
        #expect(link(t, row: 0, col: first) == expected)
        #expect(link(t, row: 0, col: last) == expected)
        #expect(link(t, row: 0, col: last + 1) == nil)
    }
}
