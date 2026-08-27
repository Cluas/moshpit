import Foundation

/// Puts Moshpit's host-side pieces on a host, and can say honestly whether it
/// worked.
///
/// One engine for every install this app does — the agent hooks and a push
/// pairing — because they were failing the same three ways:
///
///   1. **Mechanism.** Both typed shell into whatever pane was active. That
///      cannot run while an agent holds the pane (there was a guard refusing
///      exactly that), leaves secrets in the user's shell history, and has no
///      return channel, so a failure looked identical to a success. This runs
///      over an exec channel instead — no pane, no history, real output.
///   2. **Verification.** The hooks flow resolved if ANY pane on the connection
///      carried ANY agent stamp, which both false-passes (a stamp left by an
///      earlier install) and false-fails (a correct install that has not fired
///      yet, where the UI's advice was "run an agent turn, then re-check"). Here
///      a write is confirmed by digest, and the runtime path is proven by firing
///      it — see ``selfTestPush``.
///   3. **Lifecycle.** There was no record of what had been installed, no way to
///      remove it, and no way to notice a stale copy. The day the stamp script
///      gained a push hand-off, every installed copy silently stopped being
///      current and nothing could tell. ``InstallManifest`` is content-addressed
///      precisely so that cannot recur.
///
/// It also needs no `jq` and no `python3` on the host. Reading and merging an
/// agent's config happens HERE, in Swift, over a channel that only has to
/// manage `cat` and `base64 -d`; the old installer shipped a jq program plus a
/// python fallback plus a heredoc to hold them, which was most of its 6.5 KB.
struct HostInstaller {
    let channel: HostChannel

    // MARK: - Reading

    /// What this host can do. Gathered before anything is written so a host
    /// missing `openssl` is told so, rather than installing a sender that will
    /// fail silently at 3am.
    func preflight() async throws -> HostFacts {
        HostFacts.parse(try await channel.runText(HostCommands.preflight))
    }

    /// Facts plus what is already installed.
    func inspect() async throws -> InstallState {
        let facts = try await preflight()
        let manifest = try await InstallManifest.read(from: channel)
        return InstallState(facts: facts, manifest: manifest)
    }

    // MARK: - Writing

    /// Write one of the fixed script components, unless the host already has
    /// exactly this content.
    ///
    /// Returns what it did, so a caller can tell "already current" from "just
    /// installed" — the difference between a re-pair that changed nothing and
    /// one that repaired a stale script.
    @discardableResult
    func installScript(_ component: InstallComponent,
                       state: InstallState) async throws -> InstallStep {
        guard let body = HostScripts.body(of: component),
              let path = component.path,
              let want = HostScripts.digest(of: component)
        else { return InstallStep(component: component, action: .notApplicable) }

        if case .current = state.status(component) {
            return InstallStep(component: component, action: .alreadyCurrent)
        }
        _ = try await channel.run(HostCommands.writeFile(
            path: path, mode: component.mode,
            base64: Data(body.utf8).base64EncodedString()))

        let got = try await channel.runText(HostCommands.sha256(path: path))
        guard got == want else {
            return InstallStep(component: component,
                               action: .failed(verifyFailure(path: path, want: want, got: got)))
        }
        return InstallStep(component: component, action: .installed(digest: want))
    }

    /// Install (or repair) everything a push pairing needs, and record it.
    ///
    /// Order matters: the sender lands before `push.conf`, so there is never a
    /// moment where a host holds secrets and no program that could use them.
    ///
    /// The stamp script is included because pairing without it is a trap the old
    /// design shipped: `push.conf` present, sender present, and nothing that
    /// ever calls the sender — a host that looks paired and pushes nothing.
    func installPairing(_ pairing: PushPairing,
                        state: InstallState) async throws -> InstallReport {
        var report = InstallReport(facts: state.facts)

        guard state.facts.canPush else {
            report.blocked = .missingTools(state.facts.missingForPush)
            return report
        }

        report.steps.append(try await installScript(.sender, state: state))
        report.steps.append(try await installScript(.stamp, state: state))
        // If the sender did not land, do NOT write the secrets. A host holding
        // a pairing secret with no program that reads it is the worst of both:
        // nothing works, and there is a credential on disk to show for it.
        guard !report.steps.contains(where: \.isFailure) else { return report }

        let component = InstallComponent.pairing(conn: pairing.connectionId.uuidString)
        let conf = Self.pushConf(pairing)
        let path = component.path!
        _ = try await channel.run(HostCommands.writeFile(
            path: path, mode: component.mode,
            base64: Data(conf.utf8).base64EncodedString()))

        let want = ContentDigest.of(conf)
        let got = try await channel.runText(HostCommands.sha256(path: path))
        if got == want {
            report.steps.append(InstallStep(component: component, action: .installed(digest: want)))
        } else {
            report.steps.append(InstallStep(
                component: component,
                action: .failed(verifyFailure(path: path, want: want, got: got))))
            return report
        }
        // A pre-multi-device install left ONE shared push.conf. If it recorded
        // THIS device, it is superseded by the per-device file just written —
        // remove both the file and its manifest entry so the sender does not
        // push twice to one phone. Another device's legacy pairing is left
        // strictly alone; it still works through the sender's legacy read.
        var manifest = state.manifest
        if manifest.components["pairing"]?.connectionId == pairing.connectionId.uuidString {
            _ = try await channel.run(HostCommands.removeFile(path: InstallComponent.legacyPairingPath))
        }
        manifest.removePairing(conn: pairing.connectionId.uuidString)

        let now = Date()
        for step in report.steps {
            guard case .installed(let digest) = step.action else { continue }
            var isPairing = false
            if case .pairing = step.component { isPairing = true }
            manifest[step.component] = InstallManifest.Component(
                digest: digest, installedAt: now,
                connectionId: isPairing ? pairing.connectionId.uuidString : nil,
                relayURL: isPairing ? pairing.relayURL : nil)
        }
        try await manifest.write(to: channel)
        report.manifest = manifest
        return report
    }

    /// `push.conf`, exactly as the sender parses it.
    ///
    /// A pure function so the exact bytes are pinned by a test: this file is the
    /// only copy of a pairing secret that leaves the phone, and a stray quote or
    /// a missing newline here is a pairing that fails with a MAC error nobody
    /// can read.
    nonisolated static func pushConf(_ pairing: PushPairing) -> String {
        // APNS_TOKEN / APNS_ENV / SEND_IAT are the routing facts a stateless
        // relay verifies the send token against — the sender puts them in every
        // request, and the relay recomputes the HMAC instead of looking anything
        // up. A pairing missing them (isReadyToInstall == false) must never get
        // here; the empty fields it would produce make the sender skip the conf.
        """
        RELAY_URL=\(pairing.relayURL)
        SEND_TOKEN=\(pairing.sendToken)
        SECRET=\(pairing.secretHex)
        CONN=\(pairing.connectionId.uuidString)
        APNS_TOKEN=\(pairing.apnsToken ?? "")
        APNS_ENV=\(pairing.apnsEnv ?? "production")
        SEND_IAT=\(Int((pairing.sendTokenIssuedAt ?? Date(timeIntervalSince1970: 0)).timeIntervalSince1970))

        """
    }

    /// Register Moshpit's hooks in one agent's own config.
    ///
    /// Read, merge in Swift, write back — the whole reason the host needs no `jq`
    /// and no `python3`. A config that will not parse is REPORTED, never
    /// overwritten: the old installer's jq simply failed on it and the sheet went
    /// on saying the same thing it always said.
    ///
    /// The stamp script and the sender are brought current in the same pass. The
    /// sender is installed even on a host with no pairing yet — it exits 0
    /// without a `push.conf`, and having it there means pairing later needs no
    /// second visit to this screen.
    func installHooks(agent: HookAgent, state: InstallState) async throws -> InstallReport {
        var report = InstallReport(facts: state.facts)

        report.steps.append(try await installScript(.stamp, state: state))
        report.steps.append(try await installScript(.sender, state: state))
        guard !report.steps.contains(where: \.isFailure) else { return report }

        let component = InstallComponent.hooks(agent: agent.id)
        let existing = try await channel.runText(HostCommands.readFile(path: agent.configPath))
        let merged: String
        do {
            merged = try AgentHookConfig.merged(existing: existing, agent: agent)
        } catch {
            report.steps.append(InstallStep(
                component: component,
                action: .failed(error.localizedDescription)))
            return report
        }

        _ = try await channel.run(HostCommands.backupOnce(configPath: agent.configPath))
        _ = try await channel.run(HostCommands.writeFile(
            path: agent.configPath, mode: "644",
            base64: Data(merged.utf8).base64EncodedString()))

        let want = ContentDigest.of(merged)
        let got = try await channel.runText(HostCommands.sha256(path: agent.configPath))
        guard got == want else {
            report.steps.append(InstallStep(
                component: component,
                action: .failed(verifyFailure(path: agent.configPath, want: want, got: got))))
            return report
        }
        report.steps.append(InstallStep(component: component, action: .installed(digest: want)))
        // Carried on the report rather than left to the caller to remember: an
        // agent that will not run what we just installed until the user acts is
        // not an installed state worth reporting as success on its own.
        report.userActionRequired = agent.trustStep
        _ = try await channel.run(HostCommands.pruneBackups(configPath: agent.configPath, keep: 1))

        var manifest = state.manifest
        let now = Date()
        for step in report.steps {
            guard case .installed(let digest) = step.action else { continue }
            manifest[step.component] = InstallManifest.Component(
                digest: digest, installedAt: now,
                configPath: step.component == component ? agent.configPath : nil)
        }
        try await manifest.write(to: channel)
        report.manifest = manifest
        return report
    }

    /// Take Moshpit's hooks back out of an agent's config.
    ///
    /// Surgical, not "restore the backup": a config the user has edited since
    /// installing must not lose those edits to an uninstall. The path comes from
    /// the manifest, so it edits the file the install actually touched even if
    /// the agent's default location has moved since.
    func uninstallHooks(agent: HookAgent, state: InstallState) async throws -> InstallReport {
        var report = InstallReport(facts: state.facts)
        let component = InstallComponent.hooks(agent: agent.id)
        let path = state.manifest[component]?.configPath ?? agent.configPath
        guard HostCommands.isSafePath(path) else {
            report.steps.append(InstallStep(component: component, action: .failed(
                String(localized: "The manifest on this host names a config path Moshpit won't run a command against: \(path)"))))
            return report
        }

        let existing = try await channel.runText(HostCommands.readFile(path: path))
        let stripped: String
        do {
            stripped = try AgentHookConfig.removed(existing: existing, agent: agent)
        } catch {
            report.steps.append(InstallStep(component: component,
                                            action: .failed(error.localizedDescription)))
            return report
        }
        _ = try await channel.run(HostCommands.writeFile(
            path: path, mode: "644",
            base64: Data(stripped.utf8).base64EncodedString()))

        var manifest = state.manifest
        manifest[component] = nil
        try await manifest.write(to: channel)
        report.manifest = manifest
        report.steps.append(InstallStep(component: component, action: .removed))
        return report
    }

    // MARK: - Proving

    /// Ask the host to send one real push, and hand back the nonce to wait for.
    ///
    /// This is the only check in the flow that cannot lie. Every local check —
    /// the file exists, its digest matches, `openssl` is installed — confirms
    /// something about the host; none of them confirm that a notification
    /// reaches this phone, which is the entire feature. A `--test` push travels
    /// host → relay → Apple → phone, and either arrives carrying this nonce or
    /// does not.
    ///
    /// The caller starts waiting BEFORE calling this: the round trip through
    /// APNs is occasionally faster than the exec channel's own reply.
    @discardableResult
    func selfTestPush(nonce: String) async throws -> String {
        let path = InstallComponent.sender.path!
        return try await channel.runText(
            "sh \"\(path)\" --test \(HostCommands.quote(nonce)) 2>&1")
    }

    /// A nonce the phone can match a push against. Short enough to read in a log,
    /// random enough that a stale test push from an earlier attempt cannot be
    /// mistaken for this one.
    static func makeNonce() -> String {
        "selftest-" + PushPairing.randomHex(bytes: 6)
    }

    /// Prove the hook path works by firing the stamp script and reading the pane
    /// back, without waiting for the user to run an agent turn.
    ///
    /// This is the check the old flow could not make. Its "Re-check" polled for
    /// any pane carrying any stamp, which passed on a leftover from an earlier
    /// install and failed on a correct install that simply had not fired yet —
    /// so its advice was "run an agent turn in any pane, then re-check". Firing
    /// it directly proves the real thing: the script is there, it is executable,
    /// and it can reach tmux.
    ///
    /// `.unavailable` when the host's tmux server is not on the default socket
    /// (or absent). That is reported, not guessed around: the install itself is
    /// unaffected, only this proof is.
    func selfTestStamp(pane: String) async throws -> StampProof {
        let socket = try await channel.runText(HostCommands.tmuxSocketPath)
        guard !socket.isEmpty else { return .unavailable }
        _ = try await channel.run(HostCommands.selfTest(
            state: "working", pane: pane, tmuxSocket: socket))
        let read = try await channel.runText(HostCommands.readStamp(
            pane: pane, tmuxSocket: socket))
        _ = try await channel.run(HostCommands.clearStamp(pane: pane, tmuxSocket: socket))

        let fields = read.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
        guard fields.count >= 2, fields[0] == "working",
              fields[1] == PushRemoteNotification.selfTestAgent
        else { return .didNotStamp(read) }
        return .stamped
    }

    enum StampProof: Equatable {
        case stamped
        case didNotStamp(String)
        case unavailable
    }

    // MARK: - Removing

    /// Remove components and forget them.
    ///
    /// Manifest-driven rather than guessing at paths, so it can only delete what
    /// this app actually put there.
    func uninstall(_ components: [InstallComponent],
                   state: InstallState) async throws -> InstallManifest {
        var manifest = state.manifest
        for component in components {
            if let path = component.path {
                _ = try await channel.run(HostCommands.removeFile(path: path))
            }
            manifest[component] = nil
        }
        try await manifest.write(to: channel)
        return manifest
    }

    private func verifyFailure(path: String, want: String, got: String) -> String {
        if got.isEmpty {
            return "wrote \(path) but could not read it back — check the home directory is writable"
        }
        return "\(path) landed with a different checksum (wanted \(want.prefix(8))…, got \(got.prefix(8))…)"
    }
}

/// Facts plus manifest, and the one question the UI keeps asking.
struct InstallState: Equatable {
    var facts: HostFacts
    var manifest: InstallManifest

    func status(_ component: InstallComponent) -> ComponentStatus {
        guard let recorded = manifest[component] else { return .absent }
        guard let want = HostScripts.digest(of: component) else {
            // Not a fixed-content component (a hook registration, a pairing):
            // its presence in the manifest is all there is to know.
            return .current
        }
        guard let have = recorded.digest else { return .stale(installed: "unknown") }
        return have == want ? .current : .stale(installed: have)
    }

    /// What "hooks for this agent" is really worth: the WORSE of the config
    /// registration and the stamp script it calls.
    ///
    /// Reporting the registration alone put "Current" on screen directly above a
    /// warning card saying the script was out of date — two contradictory claims
    /// in one view, caught by looking at the screenshot. The registration is
    /// worthless without the script: what the agent invokes is the file, so if
    /// that is missing or stale then nothing fires, whatever the config says.
    ///
    /// A missing script reports `.absent` rather than `.stale` so the button
    /// offers "Install"; a stale one reports `.stale` so it offers "Update".
    func hooksStatus(agent: String) -> ComponentStatus {
        let registration = status(.hooks(agent: agent))
        if registration == .absent { return .absent }
        switch status(.stamp) {
        case .current:               return registration
        case .absent:                return .absent
        case .stale(let installed):  return .stale(installed: installed)
        }
    }

    /// A paired host whose stamp script is missing or out of date pushes
    /// nothing, and the old design had no way to notice. Named so the UI has to
    /// deal with it.
    var pairedButNothingWillPush: Bool {
        guard !manifest.pairingEntries.isEmpty else { return false }
        if case .current = status(.stamp) { return false }
        return true
    }

}

enum ComponentStatus: Equatable {
    case absent
    case current
    /// Installed, but not the content this build ships.
    case stale(installed: String)
}

struct InstallStep: Equatable {
    let component: InstallComponent
    let action: Action

    enum Action: Equatable {
        case installed(digest: String)
        case alreadyCurrent
        case removed
        case notApplicable
        case failed(String)
    }

    var isFailure: Bool { if case .failed = action { return true }; return false }
}

/// Everything that happened, in the order it happened.
struct InstallReport: Equatable {
    var facts: HostFacts
    var steps: [InstallStep] = []
    var manifest: InstallManifest?
    /// Set when nothing was attempted, because the host cannot support it.
    var blocked: Blocked?
    /// Set when the install landed but will not DO anything until the user takes
    /// a step on the host — Codex's hook trust being the case that exists.
    var userActionRequired: String?

    enum Blocked: Equatable {
        case missingTools([String])
    }

    var succeeded: Bool { blocked == nil && !steps.contains(where: \.isFailure) }

    var failures: [String] {
        steps.compactMap { if case .failed(let why) = $0.action { return why } else { return nil } }
    }
}
