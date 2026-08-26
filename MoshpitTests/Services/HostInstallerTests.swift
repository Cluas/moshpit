import Foundation
import Testing
@testable import Moshpit

/// A simulated host: it records every command, and remembers the files it was
/// told to write.
///
/// Deliberately more than a stub. An earlier version answered checksums from a
/// fixed table, which meant a test could only ever verify content the test
/// already knew — and pairing content is freshly random every time, so the
/// interesting path was the one it could not cover. Storing what gets written
/// and hashing THAT makes the fake behave like a host: a write followed by a
/// verify agrees, a write followed by a tampered verify does not.
///
/// `rules` still win when set, so a test can force a specific answer (a wrong
/// checksum, an unreadable config) without pretending a real host produced it.
final class RecordingChannel: HostChannel, @unchecked Sendable {
    private let lock = NSLock()
    private var _commands: [String] = []
    private var files: [String: String] = [:]

    /// First match wins; consulted before the simulated filesystem.
    var rules: [(substring: String, reply: String)] = []
    var failOn: String?
    /// Answer every checksum with nothing, as a host with an unwritable home
    /// would.
    var digestsAnswerEmpty = false

    var commands: [String] {
        lock.lock(); defer { lock.unlock() }
        return _commands
    }

    /// What the host would now hold at `path`.
    func file(at path: String) -> String? {
        lock.lock(); defer { lock.unlock() }
        return files[path]
    }

    func run(_ command: String) async throws -> Data {
        lock.lock(); _commands.append(command); lock.unlock()
        if let failOn, command.contains(failOn) {
            throw NSError(domain: "test", code: 1)
        }
        for rule in rules where command.contains(rule.substring) {
            return Data(rule.reply.utf8)
        }
        return Data((simulate(command) ?? "").utf8)
    }

    /// The four command shapes the installer uses, applied to an in-memory
    /// filesystem.
    private func simulate(_ command: String) -> String? {
        lock.lock(); defer { lock.unlock() }

        if let path = Self.writeTarget(command), let payload = Self.base64Payload(command),
           let data = Data(base64Encoded: payload),
           let text = String(data: data, encoding: .utf8) {
            files[path] = text
            return nil
        }
        if command.hasPrefix("cat \"") {
            return files[Self.quotedPath(command, after: "cat \"")] ?? ""
        }
        if command.contains("awk '{print $1}'") {
            if digestsAnswerEmpty { return "" }
            guard let path = Self.checksumTarget(command), let body = files[path] else { return "" }
            return ContentDigest.of(body)
        }
        if command.hasPrefix("rm -f \"") {
            files[Self.quotedPath(command, after: "rm -f \"")] = nil
            return nil
        }
        return nil
    }

    private static func writeTarget(_ command: String) -> String? {
        guard command.contains("base64 -d > \""),
              let start = command.range(of: "base64 -d > \""),
              let end = command.range(of: "\"", range: start.upperBound..<command.endIndex)
        else { return nil }
        return String(command[start.upperBound..<end.lowerBound])
    }

    private static func checksumTarget(_ command: String) -> String? {
        guard let start = command.range(of: "sha256sum \""),
              let end = command.range(of: "\"", range: start.upperBound..<command.endIndex)
        else { return nil }
        return String(command[start.upperBound..<end.lowerBound])
    }

    private static func quotedPath(_ command: String, after prefix: String) -> String {
        let rest = command.dropFirst(prefix.count)
        return String(rest.prefix(while: { $0 != "\"" }))
    }

    static func base64Payload(_ command: String) -> String? {
        guard let start = command.range(of: "printf %s '"),
              let end = command.range(of: "' | base64 -d", range: start.upperBound..<command.endIndex)
        else { return nil }
        return String(command[start.upperBound..<end.lowerBound])
    }

    func sent(containing needle: String) -> String? {
        commands.first { $0.contains(needle) }
    }
}

@Suite("Host installer")
struct HostInstallerTests {

    nonisolated static let pairing = PushPairing(
        connectionId: UUID(uuidString: "3F2504E0-4F89-11D3-9A0C-0305E82C3301")!,
        hostLabel: "m1-pro",
        secretHex: String(repeating: "ab", count: 32),
        sendToken: String(repeating: "cd", count: 32),
        relayURL: "https://push.example.org",
        createdAt: Date(timeIntervalSince1970: 1_755_900_000))

    nonisolated static let allTools = """
        home=/Users/cluas
        shell=/bin/zsh
        uname=Darwin
        openssl=yes
        curl=yes
        jq=yes
        tmux=yes
        python3=yes
        base64=yes
        """

    /// A host with every tool present. Checksums come from the simulated
    /// filesystem, so they agree with whatever was actually written — including
    /// pairing material that is random per run.
    nonisolated static func healthyChannel(extraRules: [(String, String)] = []) -> RecordingChannel {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", allTools)]
        channel.rules += extraRules.map { ($0.0, $0.1) }
        return channel
    }

    // MARK: - Preflight

    @Test("preflight is one round trip and parses every fact")
    func preflight() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools)]
        let facts = try await HostInstaller(channel: channel).preflight()

        #expect(channel.commands.count == 1, "preflight must not cost one exec per tool")
        #expect(facts.home == "/Users/cluas")
        #expect(facts.uname == "Darwin")
        #expect(facts.canPush)
        #expect(facts.hasJq)
        #expect(facts.missingForPush.isEmpty)
    }

    @Test("a host missing openssl is refused before anything is written")
    func missingTools() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", """
            home=/home/x
            curl=yes
            base64=yes
            openssl=no
            """)]
        let installer = HostInstaller(channel: channel)
        let state = try await installer.inspect()
        #expect(!state.facts.canPush)
        #expect(state.facts.missingForPush == ["openssl"])

        let report = try await installer.installPairing(Self.pairing, state: state)
        #expect(report.blocked == .missingTools(["openssl"]))
        #expect(report.steps.isEmpty)
        // The point of preflight: nothing was written to a host that cannot use it.
        #expect(channel.sent(containing: "push.conf") == nil)
    }

    // MARK: - Writing

    @Test("pairing lands the sender, the stamp script and push.conf")
    func installPairing() async throws {
        let channel = Self.healthyChannel()
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installPairing(
            Self.pairing, state: try await installer.inspect())

        #expect(report.succeeded, "\(report.failures)")
        let installed = report.steps.compactMap { step -> InstallComponent? in
            if case .installed = step.action { return step.component }
            return nil
        }
        #expect(installed.contains(.sender))
        #expect(installed.contains(.pairing(conn: Self.pairing.connectionId.uuidString)))
        // Pairing without the stamp script is a host that looks paired and
        // pushes nothing — the trap the old design shipped.
        #expect(installed.contains(.stamp))
    }

    @Test("the secret rides as base64, so no quote can escape into the shell")
    func secretIsBase64() async throws {
        let channel = Self.healthyChannel()
        let installer = HostInstaller(channel: channel)
        _ = try await installer.installPairing(Self.pairing, state: try await installer.inspect())

        let write = try #require(channel.sent(containing: ".conf\"; chmod"))
        // The material itself must never appear literally on a command line.
        #expect(!write.contains(Self.pairing.secretHex))
        #expect(!write.contains(Self.pairing.sendToken))
        // …and what does appear is base64, whose alphabet cannot hold a quote,
        // a dollar, a backtick or a newline.
        let conf = HostInstaller.pushConf(Self.pairing)
        #expect(write.contains(Data(conf.utf8).base64EncodedString()))
        // umask before the write, not chmod after: a chmod leaves a window where
        // a file holding two secrets is world-readable.
        let umask = try #require(write.range(of: "umask 077"))
        let redirect = try #require(write.range(of: "base64 -d >"))
        #expect(umask.lowerBound < redirect.lowerBound)
        #expect(write.contains("chmod 600"))
    }

    @Test("the sender lands before the secrets it will read")
    func orderIsSenderThenSecrets() async throws {
        let channel = Self.healthyChannel()
        let installer = HostInstaller(channel: channel)
        _ = try await installer.installPairing(Self.pairing, state: try await installer.inspect())

        let senderWrite = try #require(channel.commands.firstIndex { $0.contains("moshpit-push.sh\"; chmod") })
        let confWrite = try #require(channel.commands.firstIndex { $0.contains(".conf\"; chmod") })
        #expect(senderWrite < confWrite,
                "a host must never hold pairing secrets with no program that could use them")
    }

    @Test("push.conf is exactly the four lines the sender parses")
    func pushConfBytes() {
        #expect(HostInstaller.pushConf(Self.pairing) == """
            RELAY_URL=https://push.example.org
            SEND_TOKEN=\(String(repeating: "cd", count: 32))
            SECRET=\(String(repeating: "ab", count: 32))
            CONN=3F2504E0-4F89-11D3-9A0C-0305E82C3301

            """)
    }

    @Test("a failed script write stops before the secrets are written")
    func abortsBeforeSecrets() async throws {
        let channel = RecordingChannel()
        // Every checksum comes back wrong, so the sender write "fails".
        channel.rules = [("printf 'home=", Self.allTools), ("awk '{print $1}'", "wrong")]
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installPairing(
            Self.pairing, state: try await installer.inspect())

        #expect(!report.succeeded)
        #expect(channel.sent(containing: ".conf\"; chmod") == nil,
                "a credential must not be left on a host whose sender did not land")
        #expect(channel.sent(containing: "manifest.json") == nil ||
                channel.commands.filter { $0.contains("manifest.json\"; chmod") }.isEmpty,
                "and nothing should be recorded as installed")
    }

    @Test("a checksum that does not match is a reported failure, not a success")
    func verifyMismatch() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools), ("awk '{print $1}'", "deadbeef")]
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installPairing(
            Self.pairing, state: try await installer.inspect())

        #expect(!report.succeeded)
        #expect(report.failures.contains { $0.contains("different checksum") })
    }

    @Test("a write that cannot be read back names the likely cause")
    func verifyEmpty() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools)]
        channel.digestsAnswerEmpty = true
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installPairing(
            Self.pairing, state: try await installer.inspect())
        #expect(report.failures.contains { $0.contains("home directory is writable") })
    }

    // MARK: - Staleness, the thing that had no mechanism at all

    @Test("a script the host already has current is not rewritten")
    func skipsCurrent() async throws {
        var manifest = InstallManifest.empty
        manifest[.sender] = .init(digest: ContentDigest.of(HostScripts.sender), installedAt: Date())
        let state = InstallState(facts: HostFacts.parse(Self.allTools), manifest: manifest)
        #expect(state.status(.sender) == .current)

        let channel = Self.healthyChannel()
        let step = try await HostInstaller(channel: channel).installScript(.sender, state: state)
        #expect(step.action == .alreadyCurrent)
        #expect(channel.commands.isEmpty)
    }

    @Test("a stale script is detected by content, not by a version someone must remember to bump")
    func detectsStale() {
        var manifest = InstallManifest.empty
        manifest[.stamp] = .init(digest: "an-older-build", installedAt: Date())
        let state = InstallState(facts: HostFacts.parse(Self.allTools), manifest: manifest)
        #expect(state.status(.stamp) == .stale(installed: "an-older-build"))
        // This is the case that shipped broken: the stamp script gained a push
        // hand-off and every installed copy silently stopped being current.
        #expect(state.pairedButNothingWillPush == false, "no pairing yet, so nothing to warn about")

        manifest[.pairing(conn: "x")] = .init(installedAt: Date(), connectionId: "x", relayURL: "y")
        let paired = InstallState(facts: state.facts, manifest: manifest)
        #expect(paired.pairedButNothingWillPush,
                "paired with a stale stamp script means pushes that never fire")
    }

    @Test("hooks report the worse of the registration and the script they call")
    func hooksStatusCombines() {
        func state(stamp: String?, registered: Bool) -> InstallState {
            var manifest = InstallManifest.empty
            if let stamp {
                manifest[.stamp] = .init(digest: stamp, installedAt: Date())
            }
            if registered {
                manifest[.hooks(agent: "claude")] = .init(
                    digest: "cfg", installedAt: Date(),
                    configPath: "$HOME/.claude/settings.json")
            }
            return InstallState(facts: HostFacts.parse(Self.allTools), manifest: manifest)
        }
        let current = ContentDigest.of(HostScripts.stamp)

        #expect(state(stamp: current, registered: true).hooksStatus(agent: "claude") == .current)
        // The case a screenshot caught: registered, but the script the agent
        // actually invokes is from an older build, so nothing fires — and the row
        // used to say "Current" right above a card saying otherwise.
        #expect(state(stamp: "older", registered: true).hooksStatus(agent: "claude")
                == .stale(installed: "older"))
        // Registered with no script at all is not installed, whatever the config
        // says — and the button should offer Install.
        #expect(state(stamp: nil, registered: true).hooksStatus(agent: "claude") == .absent)
        #expect(state(stamp: current, registered: false).hooksStatus(agent: "claude") == .absent)
    }

    @Test("an absent manifest reads as nothing installed rather than as an error")
    func absentManifest() async throws {
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools)]
        let state = try await HostInstaller(channel: channel).inspect()
        #expect(state.manifest == .empty)
        #expect(state.status(.stamp) == .absent)
    }

    @Test("a corrupt or future-schema manifest is not guessed at")
    func unreadableManifest() async throws {
        for body in ["{not json", #"{"schema":99,"components":{"stamp":{}}}"#] {
            let channel = RecordingChannel()
            channel.rules = [("printf 'home=", Self.allTools), ("manifest.json", body)]
            let state = try await HostInstaller(channel: channel).inspect()
            #expect(state.manifest == .empty, "would risk uninstalling files it cannot name")
        }
    }

    @Test("the manifest records what landed, and never the secret")
    func manifestContents() async throws {
        let channel = Self.healthyChannel()
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installPairing(
            Self.pairing, state: try await installer.inspect())

        let manifest = try #require(report.manifest)
        let mine = InstallComponent.pairing(conn: Self.pairing.connectionId.uuidString)
        #expect(manifest[mine]?.connectionId == Self.pairing.connectionId.uuidString)
        #expect(manifest[mine]?.relayURL == "https://push.example.org")
        #expect(manifest[.sender]?.digest == ContentDigest.of(HostScripts.sender))

        let written = try #require(channel.sent(containing: "manifest.json\"; chmod"))
        let decoded = String(data: Data(base64Encoded: RecordingChannel.base64Payload(written) ?? "") ?? Data(),
                            encoding: .utf8) ?? ""
        #expect(!decoded.contains(Self.pairing.secretHex))
        #expect(!decoded.contains(Self.pairing.sendToken))
        #expect(decoded.contains("\"schema\" : 1"))
        // World-readable on purpose: it holds no secret, and a user should be
        // able to read what we did to their machine.
        #expect(written.contains("chmod 644"))
    }

    // MARK: - Proving and removing

    @Test("the self-test fires the real sender with a matchable nonce")
    func selfTest() async throws {
        let channel = Self.healthyChannel()
        let nonce = HostInstaller.makeNonce()
        _ = try await HostInstaller(channel: channel).selfTestPush(nonce: nonce)

        let sent = try #require(channel.sent(containing: "--test"))
        #expect(sent.contains("moshpit-push.sh"))
        #expect(sent.contains("'\(nonce)'"))
        #expect(nonce.hasPrefix("selftest-"))
        #expect(HostInstaller.makeNonce() != nonce, "a stale test push must not satisfy a new attempt")
    }

    @Test("uninstall removes only what the manifest says we put there")
    func uninstall() async throws {
        var manifest = InstallManifest.empty
        let mine = InstallComponent.pairing(conn: "c")
        let theirs = InstallComponent.pairing(conn: "other-device")
        manifest[mine] = .init(installedAt: Date(), connectionId: "c", relayURL: "r")
        manifest[theirs] = .init(installedAt: Date(), connectionId: "other-device", relayURL: "r")
        manifest[.sender] = .init(digest: "d", installedAt: Date())
        let state = InstallState(facts: HostFacts.parse(Self.allTools), manifest: manifest)

        let channel = Self.healthyChannel()
        let after = try await HostInstaller(channel: channel).uninstall([mine], state: state)

        #expect(after[mine] == nil)
        #expect(after[theirs] != nil,
                "unpairing one device must never take another device's pairing with it")
        #expect(after[.sender] != nil, "unpairing must not remove a script hooks still need")
        #expect(channel.sent(containing: "rm -f") != nil)
        #expect(channel.sent(containing: "rm -f")?.contains("push.d/c.conf") == true)
    }

    // MARK: - Hooks

    @Test("installing hooks reads, merges in Swift, writes back and verifies")
    func installHooks() async throws {
        let agent = try #require(HookAgent.named("claude"))
        let existing = #"{"model":"opus"}"#
        let merged = try AgentHookConfig.mergedJSON(existing: existing, agent: "claude")

        let channel = RecordingChannel()
        channel.rules = [
            ("printf 'home=", Self.allTools),
            ("cat \"$HOME/.claude/settings.json\"", existing),
        ]
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installHooks(
            agent: agent, state: try await installer.inspect())

        #expect(report.succeeded, "\(report.failures)")
        // The merged config went up as base64 — no jq, no python3, no heredoc.
        let write = try #require(channel.sent(containing: "settings.json\"; chmod"))
        #expect(write.contains(Data(merged.utf8).base64EncodedString()))
        // Neither tool is INVOKED. Preflight legitimately probes for them — the
        // stamp script uses jq for titles when it is there — so the assertion is
        // about the shapes the old installer used to do the merge itself.
        #expect(!channel.commands.contains { $0.contains("jq --arg") || $0.contains("python3 - ") })
        // Backed up once, and the legacy pile pruned.
        #expect(channel.sent(containing: ".moshpit.orig") != nil)
        #expect(channel.sent(containing: "moshpit.bak.") != nil)
        // The manifest remembers WHICH file was edited, so uninstall goes back
        // to the same one even if the agent's default path moves.
        #expect(report.manifest?[.hooks(agent: "claude")]?.configPath == "$HOME/.claude/settings.json")
    }

    @Test("installing for Codex reports the step the user still has to take")
    func codexNeedsTrust() async throws {
        let agent = try #require(HookAgent.named("codex"))
        let channel = RecordingChannel()
        channel.rules = [("printf 'home=", Self.allTools),
                         ("cat \"$HOME/.codex/config.toml\"", "model = \"gpt-5\"\n")]
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installHooks(
            agent: agent, state: try await installer.inspect())

        #expect(report.succeeded)
        // Succeeded, and still not going to do anything yet. Reporting only the
        // first half is how this feature kept lying to people.
        #expect(report.userActionRequired?.contains("/hooks") == true)

        let claude = try #require(HookAgent.named("claude"))
        let plain = RecordingChannel()
        plain.rules = [("printf 'home=", Self.allTools),
                       ("cat \"$HOME/.claude/settings.json\"", "{}")]
        let other = HostInstaller(channel: plain)
        let claudeReport = try await other.installHooks(
            agent: claude, state: try await other.inspect())
        #expect(claudeReport.userActionRequired == nil)
    }

    @Test("a config that will not parse is reported and nothing is written")
    func hooksRefuseBrokenConfig() async throws {
        let agent = try #require(HookAgent.named("claude"))
        let channel = RecordingChannel()
        channel.rules = [
            ("printf 'home=", Self.allTools),
            ("cat \"$HOME/.claude/settings.json\"", "{ this is not json"),
        ]
        let installer = HostInstaller(channel: channel)
        let report = try await installer.installHooks(
            agent: agent, state: try await installer.inspect())

        #expect(!report.succeeded)
        #expect(report.failures.contains { $0.contains("valid JSON") })
        #expect(channel.sent(containing: "settings.json\"; chmod") == nil,
                "a file we cannot read must never be overwritten")
        #expect(channel.sent(containing: ".moshpit.orig") == nil)
    }

    @Test("uninstalling hooks edits the file the install actually touched")
    func uninstallHooks() async throws {
        let agent = try #require(HookAgent.named("claude"))
        var manifest = InstallManifest.empty
        manifest[.hooks(agent: "claude")] = .init(
            digest: "d", installedAt: Date(), configPath: "$HOME/dotfiles/claude.json")
        let state = InstallState(facts: HostFacts.parse(Self.allTools), manifest: manifest)

        let installed = try AgentHookConfig.mergedJSON(existing: #"{"model":"opus"}"#, agent: "claude")
        let channel = RecordingChannel()
        channel.rules = [("cat \"$HOME/dotfiles/claude.json\"", installed)]

        let report = try await HostInstaller(channel: channel)
            .uninstallHooks(agent: agent, state: state)

        #expect(report.manifest?[.hooks(agent: "claude")] == nil)
        let write = try #require(channel.sent(containing: "claude.json\"; chmod"))
        // What went back must be the config with our hooks gone and the user's
        // own content intact.
        let payload = try #require(RecordingChannel.base64Payload(write))
        let restored = String(data: try #require(Data(base64Encoded: payload)), encoding: .utf8) ?? ""
        #expect(!restored.contains("moshpit-stamp.sh"))
        #expect(restored.contains("opus"))
    }

    // MARK: - Proving the hook path

    @Test("the stamp self-test fires the real script and reads the pane back")
    func stampSelfTest() async throws {
        let channel = RecordingChannel()
        channel.rules = [
            ("socket_path", "/private/tmp/tmux-501/default"),
            ("display-message -p -t", "working|moshpit-selftest|"),
        ]
        let proof = try await HostInstaller(channel: channel).selfTestStamp(pane: "%3")
        #expect(proof == .stamped)

        let fired = try #require(channel.sent(containing: "moshpit-stamp.sh"))
        #expect(fired.contains("TMUX='/private/tmp/tmux-501/default,0,0'"))
        #expect(fired.contains("TMUX_PANE='%3'"))
        #expect(fired.contains("'moshpit-selftest'"))
        // It must clean up after itself, or the island shows a phantom agent.
        #expect(channel.sent(containing: "set -pu") != nil)
    }

    @Test("a host whose tmux is elsewhere reports unavailable rather than guessing")
    func stampSelfTestUnavailable() async throws {
        let channel = RecordingChannel()   // socket probe answers empty
        let proof = try await HostInstaller(channel: channel).selfTestStamp(pane: "%1")
        #expect(proof == .unavailable)
        #expect(channel.sent(containing: "moshpit-stamp.sh") == nil)
    }

    @Test("a stamp that did not land is a distinct answer from one that did")
    func stampSelfTestFailed() async throws {
        let channel = RecordingChannel()
        channel.rules = [("socket_path", "/tmp/sock"), ("display-message -p -t", "||")]
        let proof = try await HostInstaller(channel: channel).selfTestStamp(pane: "%1")
        #expect(proof == .didNotStamp("||"))
    }

    // MARK: - Quoting

    @Test("remote values are single-quoted the only way that is correct")
    func quoting() {
        #expect(HostCommands.quote("plain") == "'plain'")
        #expect(HostCommands.quote("it's") == #"'it'\''s'"#)
        #expect(HostCommands.quote("$HOME `whoami`") == "'$HOME `whoami`'")
        #expect(HostCommands.quote("") == "''")
    }

    @Test("the checksum command copes with all three tools a host might have")
    func checksumPortability() {
        let cmd = HostCommands.sha256(path: "$HOME/.moshpit/x")
        for tool in ["sha256sum", "shasum -a 256", "openssl dgst -sha256"] {
            #expect(cmd.contains(tool))
        }
        #expect(cmd.contains("awk '{print $1}'"), "three tools, three output shapes, one digest")
    }

}
