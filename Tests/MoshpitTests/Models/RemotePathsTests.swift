import Foundation
import Testing
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif
@testable import Moshpit

/// Path links: the probe that asks the host which candidates exist and the
/// cache that batches those asks (the scanner that finds them in agent prose,
/// and the pass that tags confirmed paths, are the core's: crates/term and
/// CorePathLinkTests). The lines come from a
/// real transcript (2026-10-18): Claude Code reporting a comparison image as
/// a relative path after a full-width colon, and a codex prompt naming bare
/// file names it was told to write into the current directory.
@Suite("Remote path links")
struct RemotePathsTests {

    /// A probe batch, as a terminal's path pass hands it over: the token,
    /// and whether it can only be worth a link as a folder (the scanner that
    /// finds them is the core's, crates/term/src/paths.rs).
    static func candidates(_ tokens: String...) -> [TerminalPathScanner.Candidate] {
        tokens.map { token in
            let name = token.split(separator: "/").last ?? ""
            let folder = token.hasSuffix("/") || !name.contains(".")
            return TerminalPathScanner.Candidate(token: token, offset: 0, length: token.count,
                                                 directoryOnly: folder)
        }
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
        let batch = Self.candidates("out/a.png", "out/plots")
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
        let batch = Self.candidates(#"out\"#, "out/a.png")
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
        let batch = Self.candidates(
            "qchar/out/GripLab.png", "01_sheet.png", "plots/", "src/", "missing.png", "y.png", "~/repo/plots/",
            "\(cwd)/qchar/out/GripLab.png", "build/concept/v2/", "02_poses.png", "b.png")
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
        // The core offers a CJK-headed path from every start, longest
        // first (crates/term/src/paths.rs: wide heads).
        let batch = Self.candidates("输出到成品/西游-粗剪.mp4", "到成品/西游-粗剪.mp4", "成品/西游-粗剪.mp4",
                                    "品/西游-粗剪.mp4", "已经成品/粗剪.mp4", "经成品/粗剪.mp4", "成品/粗剪.mp4",
                                    "品/粗剪.mp4")
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
        let bytes = Data(base64Encoded: try sh(RemoteFileCommands.cat(root + "/a.png", size: 5)),
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
        RemotePathsTests.candidates(token)[0]
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

    /// Whether `links` asks for another link pass (`onResolved`) within
    /// `limit`: what the terminal runs a pass on.
    private func asksForAPass(_ links: RemotePathLinks, within limit: Duration) async -> Bool {
        var asked = false
        links.onResolved = { asked = true }
        let deadline = ContinuousClock.now + limit
        while !asked, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
        return asked
    }

    /// B817: a probe that got no answer — no connection yet, a session
    /// still coming back — was kept as a miss for 30 s, and nothing asked
    /// again after that: the link pass runs on new output, an answer, or the
    /// pane coming on screen, and a quiet screen has none of them. A path
    /// printed while the connection came back stayed plain text for good
    /// (the screenshots' "Wrote bench/latency.png" never opened from its
    /// tap; the Files sheet did). No answer is not a miss: the pass is asked
    /// for again shortly, without new output, and the path becomes a link.
    @Test("a probe that got no answer is asked again shortly, without new output")
    func noAnswerIsAskedAgain() async throws {
        let fake = FakeProbe()
        fake.table = ["bench/latency.png": .init(path: "/r/bench/latency.png", isDirectory: false)]
        var reachable = false
        let links = RemotePathLinks(workingDirectory: { "/r" },
                                    probe: { cwd, batch in reachable ? fake.run(cwd, batch) : nil },
                                    retryAfterNoAnswer: 0.5)
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == nil)
        // After the batch went out (its debounce) and before its retry.
        try await Task.sleep(for: .milliseconds(400))
        reachable = true
        #expect(await asksForAPass(links, within: .seconds(5)),
                "nothing asked for another pass after a probe that got no answer")
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == nil, "asked again")
        await settle(links)
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == "/r/bench/latency.png")
    }

    /// And when the session is live again, what got no answer while it was
    /// away is asked at once, not at its retry (up to 30 s later).
    @Test("a reconnect asks for a pass at once, and what got no answer is asked again")
    func reconnectAsksAtOnce() async throws {
        let fake = FakeProbe()
        fake.table = ["bench/latency.png": .init(path: "/r/bench/latency.png", isDirectory: false)]
        var reachable = false
        let links = RemotePathLinks(workingDirectory: { "/r" },
                                    probe: { cwd, batch in reachable ? fake.run(cwd, batch) : nil },
                                    retryAfterNoAnswer: 30)
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == nil)
        try await Task.sleep(for: .milliseconds(400))
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == nil, "waits for its retry")
        reachable = true
        var asked = false
        links.onResolved = { asked = true }
        links.connectionCameBack()
        #expect(asked, "the reconnect asked for no pass")
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == nil, "asked again")
        await settle(links)
        #expect(links.resolvedPath(for: candidate("bench/latency.png")) == "/r/bench/latency.png")
    }

    /// Nor is a dead connection asked on every pass: until the retry, the
    /// unanswered path waits.
    @Test("an unanswered path isn't asked again on every pass before its retry")
    func noAnswerWaitsForItsRetry() async throws {
        var calls = 0
        let links = RemotePathLinks(workingDirectory: { "/r" },
                                    probe: { _, _ in calls += 1; return nil },
                                    retryAfterNoAnswer: 30)
        _ = links.resolvedPath(for: candidate("bench/latency.png"))
        try await Task.sleep(for: .milliseconds(600))
        #expect(calls == 1)
        for _ in 0..<5 { _ = links.resolvedPath(for: candidate("bench/latency.png")) }
        try await Task.sleep(for: .milliseconds(600))
        #expect(calls == 1, "a pass asked a host that hadn't answered again at once")
    }

    /// The same for a miss: an agent names a file a moment before it writes
    /// it, and the miss was only asked about again if something new was
    /// printed after its window. Once the window passes, the pass is asked
    /// for, and the file that has appeared since becomes a link.
    @Test("a miss comes back to the pass once its window passes, without new output")
    func missComesBackWithoutOutput() async throws {
        let fake = FakeProbe()
        let links = RemotePathLinks(workingDirectory: { "/r" }, probe: { fake.run($0, $1) },
                                    retryAfterMiss: 0.6)
        #expect(links.resolvedPath(for: candidate("out/a.png")) == nil)
        // After the batch went out (its debounce) and before its retry.
        try await Task.sleep(for: .milliseconds(400))
        #expect(fake.calls.count == 1)
        fake.table = ["out/a.png": .init(path: "/r/out/a.png", isDirectory: false)]
        #expect(await asksForAPass(links, within: .seconds(5)), "the miss never came back to the pass")
        #expect(links.resolvedPath(for: candidate("out/a.png")) == nil, "asked again")
        await settle(links)
        #expect(links.resolvedPath(for: candidate("out/a.png")) == "/r/out/a.png")
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
}
