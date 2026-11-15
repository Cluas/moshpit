import Foundation
import Testing
@testable import Moshpit

/// Every command line that carries a value someone else chose — a session
/// or window name, a file name on the host, the host's home or tmux socket —
/// quotes it whole, whatever the value holds. (herdr's — a pane's working
/// directory, a worktree path, an agent name — are the Rust core's:
/// crates/herdr/tests/launch.rs.)
///
/// Each copy of the quoting helper escaped `'` by `Character`, so an
/// apostrophe that a combining mark, a variation selector or an emoji
/// modifier was attached to stayed bare, closed the quote, and the rest ran
/// on the host. Driven through each path's own entry point; where a test can
/// spawn a shell, the command is run for real, from a scratch directory
/// where a successful injection would leave `INJECTED`.
@Suite("shell quoting at every call site", .serialized)
@MainActor
struct ShellQuoteCallSiteTests {

    /// Marks that join an apostrophe before them into one `Character`.
    static let marks = ["\u{301}", "\u{FE0F}", "\u{1F3FB}", "\u{200D}", "\u{903}"]

    /// A value someone else chose, with `mark` after its apostrophe.
    static func hostile(_ mark: String) -> String { "it'\(mark); touch INJECTED; '" }

    private func injected(in directory: URL) -> Bool {
        FileManager.default.fileExists(atPath: directory.appendingPathComponent("INJECTED").path)
    }

    // MARK: - Files on the host

    @Test("File commands and the path probe quote a host's file names", arguments: marks)
    func fileCommands(_ mark: String) {
        let path = "/srv/\(Self.hostile(mark))"
        let quoted = "'/srv/it'\\''\(mark); touch INJECTED; '\\'''"
        #expect(RemoteFileCommands.stat(path).contains("[ -d \(quoted) ]"))
        // B333: the listing script takes the folder as "$1", quoted once at the end.
        let list = RemoteFileCommands.list(path)
        #expect(list.hasPrefix("sh -c "))
        #expect(list.hasSuffix(" moshpit-edit \(quoted)"))
        #expect(RemoteFileCommands.cat(path) == "base64 < \(quoted)")
        #expect(RemoteFileEdits.makeDirectory(path).hasSuffix("moshpit-edit \(quoted)"))
        #expect(RemoteFileEdits.remove(path, isDirectory: false)?.hasSuffix("moshpit-edit \(quoted)") == true)
        let probe = PathProbe.command(cwd: path, candidates: [
            TerminalPathScanner.Candidate(token: "out/a.png", offset: 0, length: 9, directoryOnly: false)])
        #expect(probe.contains("moshpit-probe \(quoted) "))
    }

    @Test("Through a real shell, file commands make the very names they were given, and nothing runs",
          .enabled(if: PosixShell.available), arguments: marks)
    func fileCommandsRun(_ mark: String) throws {
        let directory = try PosixShell.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let home = directory.appendingPathComponent(Self.hostile(mark)).path
        let made = try #require(PosixShell.run("cd \(Shell.quote(directory.path)) && mkdir -p \(Shell.quote(home))"))
        #expect(made.exitCode == 0, "\(made.text)")
        #expect(FileManager.default.fileExists(atPath: home))
        let folder = home + "/\(Self.hostile(mark)) 2"
        let mkdir = try #require(PosixShell.run("cd \(Shell.quote(directory.path)) && \(RemoteFileEdits.makeDirectory(folder))"))
        #expect(mkdir.text.contains(RemoteFileEdits.okMarker))
        #expect(FileManager.default.fileExists(atPath: folder))
        let probe = PathProbe.command(cwd: home, candidates: [
            TerminalPathScanner.Candidate(token: "out/a.png", offset: 0, length: 9, directoryOnly: false)])
        _ = try #require(PosixShell.run("cd \(Shell.quote(directory.path)) && \(probe)"))
        #expect(!injected(in: directory))
    }

    // The installer's self-test commands moved to the core with the
    // installer: crates/install/tests/stamp.rs quotes the host's tmux socket
    // path with every mark here.
}
