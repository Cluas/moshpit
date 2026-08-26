import Foundation

/// The machinery that used to be buttons.
///
/// Pairing is a key exchange between this device and a host — nothing in it
/// ever needed a human's judgment, and the Pair / Re-pair / "Update" ceremony
/// existed only because the machinery had no other trigger. Now it has one:
/// every time a session's control plane comes up, this walks the host once and
/// silently puts right whatever has drifted —
///
///   - scripts out of date (an app update shipped new ones) → reinstalled
///   - this device's pairing missing or stale on the host → written
///   - no local pairing at all → minted, registered, installed
///
/// The ONE thing it never does is install hooks on a host that has none:
/// hooks registration edits the user's agent config (`~/.claude/settings.json`),
/// and that first write is the single genuine consent moment the whole feature
/// has. A host with no hooks is reported via ``needsConsent`` for the UI to ask
/// — once — and is otherwise left exactly as it is.
///
/// Everything here is best-effort and silent: a host that is offline, slow, or
/// missing tools just stays as it was until the next session. Failures are
/// logged, never surfaced — the sheet remains the place to see and fix state by
/// hand.
@MainActor
@Observable
final class HostAutoCare {
    static let shared = HostAutoCare()

    /// A connection whose host has no hooks yet — the UI's cue to ask the one
    /// consent question. Cleared when consent is given, declined, or the
    /// session goes away.
    var needsConsent: ServerConnection?

    /// Connections already tended this app run. Once is enough: drift comes
    /// from app updates and re-installs, not from minute to minute.
    @ObservationIgnored private var tended: Set<UUID> = []
    @ObservationIgnored private var inFlight: Set<UUID> = []

    private let settings: AppSettings
    private let push: any PushCoordinating

    init(settings: AppSettings = .shared, push: any PushCoordinating = PushService.shared) {
        self.settings = settings
        self.push = push
    }

    /// Forget the per-run memory for a connection so an explicit user action in
    /// the sheet (install, unpair) is followed by a fresh look next session.
    func forget(connectionId: UUID) {
        tended.remove(connectionId)
    }

    func tend(connection: ServerConnection, session: SessionHub.ActiveSession) {
        // Harness escape hatch. Every seeded test connection points at a REAL
        // host (a scratch tmux, but the login's real $HOME) — without this, a
        // perf probe or reconnect harness would silently pair its throwaway
        // connection onto the developer's actual ~/.moshpit and register a
        // one-run device with the production relay, every single run.
        guard !ProcessInfo.processInfo.arguments.contains("-MOSHPIT_AUTOCARE_OFF") else { return }
        guard settings.notificationsEnabled || settings.liveActivityEnabled else { return }
        guard !tended.contains(connection.id), !inFlight.contains(connection.id) else { return }
        inFlight.insert(connection.id)
        Task { [weak self] in
            defer { self?.inFlight.remove(connection.id) }
            await self?.tendNow(connection: connection, session: session)
        }
    }

    private func tendNow(connection: ServerConnection, session: SessionHub.ActiveSession) async {
        do {
            let installer = try await session.hostInstaller()
            let state = try await installer.inspect()
            tended.insert(connection.id)

            let agents = Self.installedHookAgents(state.manifest)
            guard !agents.isEmpty else {
                // No hooks: nothing was consented to on this host. Ask (the UI
                // decides how), unless the user already said no for this
                // connection.
                if !settings.hostSetupDeclined.contains(connection.id.uuidString) {
                    needsConsent = connection
                }
                return
            }

            // Scripts drift when the app updates; bring every consented agent's
            // registration current. `installHooks` re-lands stamp + sender and
            // re-merges the config — all idempotent.
            // The sender is checked EXPLICITLY: `hooksStatus` folds in the stamp
            // but not the sender, and a stale sender under a fresh pairing fails
            // silently — the exact hole a live probe fell into (an old
            // single-pairing sender ignored a freshly written push.d file, and a
            // targeted self-test went to the wrong device).
            var current = state
            let senderStale = current.status(.sender) != .current
            for agent in agents where senderStale || current.hooksStatus(agent: agent.id) != .current {
                Log.island.info("autocare: refreshing \(agent.id, privacy: .public) hooks on \(connection.displayName, privacy: .public)")
                _ = try await installer.installHooks(agent: agent, state: current)
                current = try await installer.inspect()
            }

            try await ensurePairing(connection: connection, installer: installer, state: current)
        } catch {
            // Silent by design; the sheet is the diagnostic surface.
            Log.island.info("autocare: \(connection.displayName, privacy: .public) skipped: \(String(describing: error), privacy: .public)")
        }
    }

    private func ensurePairing(connection: ServerConnection,
                               installer: HostInstaller,
                               state: InstallState) async throws {
        guard settings.notificationsEnabled else { return }
        let conn = connection.id.uuidString

        if var local = PushPairingStore.pairing(for: connection.id) {
            // Heal a pairing minted while the relay setting was broken (the
            // ".org"-less typo era): the secrets and tokens are fine — they are
            // random, not derived from the address — so only the address needs
            // correcting, locally and (via the digest mismatch below) on the
            // host. Re-minting would orphan the relay registration instead.
            if local.relayURL != settings.pushRelayURL {
                Log.island.info("autocare: correcting relay address on \(connection.displayName, privacy: .public)")
                local.relayURL = settings.pushRelayURL
                _ = PushPairingStore.upsert(local)
            }
            // The host's copy must match ours byte for byte; `pairingEntry`
            // also finds a pre-multi-device install, whose digest matches when
            // the content does — reinstalling then migrates it to push.d.
            let expected = ContentDigest.of(HostInstaller.pushConf(local))
            if state.manifest.pairingEntry(conn: conn)?.digest != expected {
                Log.island.info("autocare: rewriting pairing on \(connection.displayName, privacy: .public)")
                _ = try await installer.installPairing(local, state: state)
            }
            return
        }

        // Never paired from this device: mint, register, install — the whole
        // flow the Pair button used to be.
        Log.island.info("autocare: pairing \(connection.displayName, privacy: .public)")
        let pairing = try await push.pair(connectionId: connection.id,
                                          hostLabel: connection.displayName,
                                          relayURL: settings.pushRelayURL)
        _ = try await installer.installPairing(pairing, state: state)
    }

    /// The user said yes: perform the one consented write (hook registration
    /// for the primary agent) and then let the ordinary tending pass finish the
    /// job — scripts, pairing, registration. Claude Code is the default the
    /// prompt names; other agents remain a Host Setup choice.
    func grantConsent(connection: ServerConnection, session: SessionHub.ActiveSession) {
        needsConsent = nil
        guard let claude = HookAgent.all.first(where: { $0.id == "claude" }) else { return }
        Task { [weak self] in
            do {
                let installer = try await session.hostInstaller()
                let state = try await installer.inspect()
                _ = try await installer.installHooks(agent: claude, state: state)
                let after = try await installer.inspect()
                try await self?.ensurePairing(connection: connection,
                                              installer: installer, state: after)
            } catch {
                Log.island.info("autocare: enable on \(connection.displayName, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// The user said no. Never ask for this connection again; the sheet stays
    /// available for a change of mind.
    func decline(connection: ServerConnection) {
        needsConsent = nil
        settings.hostSetupDeclined.insert(connection.id.uuidString)
    }

    /// The user said "not now": ask again next app run, not next session.
    func dismissConsent() {
        needsConsent = nil
    }

    /// The agents whose hooks THIS host already carries — i.e. what was
    /// consented to at some point. Static and pure for the tests' sake.
    nonisolated static func installedHookAgents(_ manifest: InstallManifest) -> [HookAgent] {
        HookAgent.all.filter { manifest[.hooks(agent: $0.id)] != nil }
    }
}
