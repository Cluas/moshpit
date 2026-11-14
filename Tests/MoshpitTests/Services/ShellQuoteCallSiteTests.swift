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

    /// `hostile(mark)` as one quoted word.
    static func quoted(_ mark: String) -> String { "'it'\\''\(mark); touch INJECTED; '\\'''" }

    private func injected(in directory: URL) -> Bool {
        FileManager.default.fileExists(atPath: directory.appendingPathComponent("INJECTED").path)
    }

    // MARK: - The helpers that were copies

    @Test("Each former copy quotes the apostrophe whatever is attached to it", arguments: marks)
    func formerCopies(_ mark: String) {
        let value = Self.hostile(mark), quoted = Self.quoted(mark)
        #expect(ExecUploadCommands.quote(value) == quoted)
        #expect(SessionHub.ActiveSession.shellQuote(value) == quoted)
        #expect(PosixShell.unquote(quoted) == value)
    }

    @Test("A prepended mark before the apostrophe doesn't hide it either")
    func prependedMark() {
        let value = "x\u{600}'; touch INJECTED; '"
        #expect(PosixShell.unquote(ExecUploadCommands.quote(value)) == value)
        #expect(PosixShell.unquote(SessionHub.ActiveSession.shellQuote(value)) == value)
    }

    // MARK: - tmux: session names typed into the login shell

    @Test("The sidecar's chain quotes a session name", arguments: marks)
    func tmuxAttachLines(_ mark: String) throws {
        let name = "dev'\(mark); touch INJECTED; '"
        let target = "'=dev'\\''\(mark); touch INJECTED; '\\'''"
        let chain = try #require(PosixShell.shPayload(SessionHub.ActiveSession.tmuxSidecarAttachChain(
            tmux: "tmux", preferred: .init(id: "$7", name: name))))
        #expect(chain.hasPrefix("tmux -CC attach -t '$7' 2>/dev/null || "))
        #expect(chain.contains(" || tmux -CC attach -t \(target) 2>/dev/null || "))
    }

    // MARK: - tmux: names on the control channel

    private func attachedController() async -> (TmuxSessionController, MockTmuxTransport) {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        let deadline = ContinuousClock.now + .seconds(1)
        while await transport.recordedCommands().count < 3, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
        return (controller, transport)
    }

    @Test("Session and window names reach tmux as one quoted argument", arguments: marks)
    func tmuxNames(_ mark: String) async throws {
        let (controller, transport) = await attachedController()
        let baseline = await transport.recordedCommands().count
        // A tmux command-string literal, not a shell word (B139): inside its
        // double quotes an apostrophe is nothing special, marked or not.
        let name = Self.hostile(mark), quoted = "\"\(name)\""

        controller.renameSession("$1", to: name)
        controller.renameWindow("@1", to: name)
        controller.newSession(named: name)
        controller.newWindow(named: name)

        let deadline = ContinuousClock.now + .seconds(2)
        while await transport.recordedCommands().count < baseline + 4, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
        let sent = await transport.recordedCommands().dropFirst(baseline)
            .map { $0.trimmingCharacters(in: .newlines) }
        #expect(sent.contains("rename-session -t $1 -- \(quoted)"))
        #expect(sent.contains("rename-window -t @1 -- \(quoted)"))
        #expect(sent.contains("new-session -d -P -F '#{session_id}' -s \(quoted)"))
        #expect(sent.contains("new-window -P -F '#{window_id}' -n \(quoted)"))
    }

    // MARK: - Image paths pasted at a prompt

    @Test("An uploaded path with a marked apostrophe is quoted in every insert style", arguments: marks)
    func imagePaths(_ mark: String) {
        let path = "/home/o'\(mark)b/.moshpit/uploads/IMG-1.jpg"
        let quoted = "'/home/o'\\''\(mark)b/.moshpit/uploads/IMG-1.jpg'"
        #expect(ImageAttachmentPipeline.insertText(forRemotePaths: [path]) == " \(quoted) ")
        #expect(ImageAttachmentPipeline.insertText(forRemotePaths: [path], style: .atMention) == " \(quoted) ")
        #expect(ImageAttachmentPipeline.insertText(forRemotePaths: [path], style: .aiderAdd) == "/add \(quoted)")
        // A space with a mark on it still needs the quotes.
        let spaced = "/home/a \u{301}b/IMG-1.jpg"
        #expect(ImageAttachmentPipeline.insertText(forRemotePaths: [spaced]) == " '\(spaced)' ")
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
        let plan = ExecUploadCommands.plan(home: path, filename: "IMG-1.jpg")
        #expect(plan.begin.contains("'/srv/it'\\''\(mark); touch INJECTED; '\\''/.moshpit/uploads'"))
    }

    @Test("Through a real shell, file commands make the very names they were given, and nothing runs",
          .enabled(if: PosixShell.available), arguments: marks)
    func fileCommandsRun(_ mark: String) throws {
        let directory = try PosixShell.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let home = directory.appendingPathComponent(Self.hostile(mark)).path
        let plan = ExecUploadCommands.plan(home: home, filename: "IMG-1.jpg")
        let made = try #require(PosixShell.run("cd \(ExecUploadCommands.quote(directory.path)) && mkdir -p \(ExecUploadCommands.quote(home)) && \(plan.begin)"))
        #expect(made.exitCode == 0, "\(made.text)")
        #expect(FileManager.default.fileExists(atPath: home + "/.moshpit/uploads"))
        let folder = home + "/\(Self.hostile(mark)) 2"
        let mkdir = try #require(PosixShell.run("cd \(ExecUploadCommands.quote(directory.path)) && \(RemoteFileEdits.makeDirectory(folder))"))
        #expect(mkdir.text.contains(RemoteFileEdits.okMarker))
        #expect(FileManager.default.fileExists(atPath: folder))
        let probe = PathProbe.command(cwd: home, candidates: [
            TerminalPathScanner.Candidate(token: "out/a.png", offset: 0, length: 9, directoryOnly: false)])
        _ = try #require(PosixShell.run("cd \(ExecUploadCommands.quote(directory.path)) && \(probe)"))
        #expect(!injected(in: directory))
    }

    // The installer's self-test commands moved to the core with the
    // installer: crates/install/tests/stamp.rs quotes the host's tmux socket
    // path with every mark here.
}
