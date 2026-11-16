import Foundation
import MoshpitKit

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
///
/// It only ever cares for a connection that is still saved and not on its way
/// out. A pass reads the host first, which takes a while, and a connection
/// deleted in that while — Home's delete waits seconds for the session to end
/// — used to be paired anyway: the delete had just forgotten its pairing (or
/// it never had one), so the pass found none and made one, minted at the relay
/// and written to the phone, for a connection that was gone a moment later. A
/// delete now tells care first (``connectionDeletionBegan(_:)``), which
/// cancels the pass in flight; and the moment a pass would pair, it checks
/// again that the connection is saved and not being deleted.
@MainActor
@Observable
final class HostAutoCare {
    static let shared = HostAutoCare()

    /// A connection whose host has no hooks yet — the UI's cue to ask the one
    /// consent question. Cleared when consent is given, declined, or the
    /// session goes away.
    var needsConsent: ServerConnection?

    /// The saved connections — the app hands its store over at launch. Care
    /// acts only for a connection this holds: one deleted while a pass was
    /// out, or never saved at all, gets nothing done for it, and no pairing
    /// above all. With no store handed over, care acts for none.
    @ObservationIgnored var connectionStore: ConnectionStore?

    /// Connections already tended this app run. Once is enough: drift comes
    /// from app updates and re-installs, not from minute to minute.
    @ObservationIgnored private var tended: Set<UUID> = []
    /// The pass running for each connection — the one a delete of that
    /// connection cancels and waits out — and a token telling it from the
    /// next one.
    @ObservationIgnored private var inFlight: [UUID: (token: UUID, task: Task<Void, Never>)] = [:]
    /// Connections whose delete has begun and not yet ended.
    @ObservationIgnored private var deleting: Set<UUID> = []
    /// Connections whose unpair (Host Setup) has begun and not yet ended.
    @ObservationIgnored private var unpairing: Set<UUID> = []
    /// The last pairing operation queued for each connection's host — care's
    /// or Host Setup's "Pair" — which the next one waits out (B123).
    @ObservationIgnored private var pairingLanes: [UUID: Task<Void, Never>] = [:]

    private let settings: AppSettings
    private let push: any PushCoordinating

    init(settings: AppSettings = .shared, push: (any PushCoordinating)? = nil,
         connectionStore: ConnectionStore? = nil) {
        self.settings = settings
        self.push = push ?? PushService.shared
        self.connectionStore = connectionStore
    }

    /// Forget the per-run memory for a connection so an explicit user action in
    /// the sheet (install, unpair) is followed by a fresh look next session.
    func forget(connectionId: UUID) {
        tended.remove(connectionId)
    }

    /// Whether a pass is running for `connectionId` right now.
    func isTending(_ connectionId: UUID) -> Bool {
        inFlight[connectionId] != nil
    }

    func tend(connection: ServerConnection, session: SessionHub.ActiveSession) {
        tend(connection: connection) { try await session.hostInstaller() }
    }

    /// ``tend(connection:session:)`` over any way of reaching the host — the
    /// session's exec channel in the app, a fake host in tests.
    func tend(connection: ServerConnection,
              installer: @escaping @MainActor () async throws -> HostInstaller) {
        // Harness escape hatch. Every seeded test connection points at a REAL
        // host (a scratch tmux, but the login's real $HOME) — without this, a
        // perf probe or reconnect harness would silently pair its throwaway
        // connection onto the developer's actual ~/.moshpit and register a
        // one-run device with the production relay, every single run.
        guard !ProcessInfo.processInfo.arguments.contains("-MOSHPIT_AUTOCARE_OFF") else { return }
        guard settings.notificationsEnabled || settings.liveActivityEnabled else { return }
        let id = connection.id
        guard !tended.contains(id), inFlight[id] == nil, isKept(id) else { return }
        track(id) { [weak self] in
            await self?.tendNow(connection: connection, installer: installer)
        }
    }

    /// Run `pass` as the one in flight for `id`. It stops being in flight as
    /// it ends, before anyone waiting on it hears that it has.
    private func track(_ id: UUID, _ pass: @escaping @MainActor () async -> Void) {
        let token = UUID()
        let task = Task { @MainActor [weak self] in
            await pass()
            if self?.inFlight[id]?.token == token { self?.inFlight[id] = nil }
        }
        inFlight[id] = (token, task)
    }

    // MARK: One pairing at a time

    /// Run `operation` as the one pairing operation for `connectionId`'s host:
    /// after the one queued before it, never beside it (B123). Care's pass
    /// pairs through here, and so does Host Setup's "Pair" — they used to run
    /// side by side: both minted a pairing, each saved its own on the phone
    /// (the later save wins) and installed it on the host (the later install
    /// wins), and a phone left holding one secret with the host holding the
    /// other gets pushes it can't open. `waited` is whether another operation
    /// ran first: what the host held, read before, may be stale.
    ///
    /// The operation is cancelled with its caller (a delete or an unpair
    /// cancels care's pass, waiting in line or not).
    func serializePairing<T: Sendable>(_ connectionId: UUID,
                             _ operation: @escaping @MainActor (_ waited: Bool) async throws -> T) async throws -> T {
        let previous = pairingLanes[connectionId]
        let run = Task { @MainActor () async throws -> T in
            if let previous { await previous.value }
            try Task.checkCancellation()
            return try await operation(previous != nil)
        }
        let lane = Task { @MainActor in _ = try? await run.value }
        pairingLanes[connectionId] = lane
        defer { if pairingLanes[connectionId] == lane { pairingLanes[connectionId] = nil } }
        return try await withTaskCancellationHandler {
            try await run.value
        } onCancel: {
            run.cancel()
        }
    }

    // MARK: Deleting a connection

    /// A delete of `connectionId` has begun. From here on nothing in care
    /// acts for it — no consent question, no pairing made, minted or
    /// written to its host — and the pass in flight for it, if any, is
    /// cancelled. Returns that pass, for the delete to wait out before the
    /// connection goes (``ConnectionDeletion``).
    func connectionDeletionBegan(_ connectionId: UUID) -> Task<Void, Never>? {
        deleting.insert(connectionId)
        if needsConsent?.id == connectionId { needsConsent = nil }
        let pass = inFlight[connectionId]?.task
        pass?.cancel()
        return pass
    }

    /// The delete of `connectionId` is over: the connection is gone, or —
    /// its pairing couldn't be forgotten — kept, and then a saved connection
    /// like any other, tended afresh on its next session (a pass the delete
    /// cancelled may not have finished).
    func connectionDeletionEnded(_ connectionId: UUID) {
        deleting.remove(connectionId)
        tended.remove(connectionId)
    }

    // MARK: Unpairing a host

    /// The user asked Host Setup to unpair `connectionId`'s host — the
    /// delete's handshake, for the pairing alone. From here until
    /// ``connectionUnpairEnded(_:)`` nothing in care pairs it, and the pass
    /// in flight for it, if any, is cancelled and returned for the unpair to
    /// wait out.
    ///
    /// A pass starts with every session and reads the host first, so an
    /// unpair made soon after connecting used to land while it was out: the
    /// pass came back, found no pairing — the unpair had just forgotten it —
    /// and made a new one, on the phone and on the host, a few seconds after
    /// the screen had said "unpaired".
    func connectionUnpairBegan(_ connectionId: UUID) -> Task<Void, Never>? {
        unpairing.insert(connectionId)
        let pass = inFlight[connectionId]?.task
        pass?.cancel()
        return pass
    }

    /// The unpair of `connectionId` is over, done or not. Its next session
    /// gets a fresh look, as after any explicit change in the sheet.
    func connectionUnpairEnded(_ connectionId: UUID) {
        unpairing.remove(connectionId)
        tended.remove(connectionId)
    }

    /// Saved, and neither being deleted nor unpaired.
    private func isKept(_ id: UUID) -> Bool {
        guard !deleting.contains(id), !unpairing.contains(id), let connectionStore else { return false }
        return connectionStore.connections.contains { $0.id == id }
    }

    /// Whether the pass running now may still act for `id`: it wasn't
    /// cancelled, and the connection is saved and not being deleted. Asked
    /// after every wait on the host or the relay that comes before acting.
    private func mayAct(for id: UUID) -> Bool {
        !Task.isCancelled && isKept(id)
    }

    private func tendNow(connection: ServerConnection,
                         installer makeInstaller: @MainActor () async throws -> HostInstaller) async {
        do {
            let installer = try await makeInstaller()
            let state = try await installer.inspect()
            // Reading the host takes a while; the connection may be on its
            // way out by now.
            guard mayAct(for: connection.id) else {
                Log.island.info("autocare: \(connection.displayName, privacy: .public) is being deleted or no longer saved — nothing done")
                return
            }
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
                guard mayAct(for: connection.id) else { return }
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

    /// Care's pairing step, in line with any other pairing operation for the
    /// host (``serializePairing(_:_:)``): one waited out first — Host Setup's
    /// "Pair" — has changed what the phone and the host hold, so the host is
    /// read again and the phone's pairing taken as it is now.
    private func ensurePairing(connection: ServerConnection,
                               installer: HostInstaller,
                               state: InstallState) async throws {
        guard settings.notificationsEnabled else { return }
        try await serializePairing(connection.id) { [weak self] waited in
            guard let self else { return }
            let current = waited ? try await installer.inspect() : state
            try await self.ensurePairingNow(connection: connection, installer: installer, state: current)
        }
    }

    private func ensurePairingNow(connection: ServerConnection,
                                  installer: HostInstaller,
                                  state: InstallState) async throws {
        let conn = connection.id.uuidString
        // The moment care pairs — or touches a pairing at all — the
        // connection must still be saved and not being deleted. Nothing
        // between here and the pairing being saved waits, so a delete either
        // came first and is seen here, or comes after and forgets what was
        // saved (the relay's late answer for it is dropped by `PushService`).
        guard mayAct(for: connection.id) else {
            Log.island.info("autocare: not pairing \(connection.displayName, privacy: .public) — it is being deleted or no longer saved")
            return
        }

        if var local = PushPairingStore.pairing(for: connection.id) {
            // Heal a pairing minted while the relay setting was broken (the
            // ".org"-less typo era): the secret is fine — it is random, not
            // derived from the address — so only the address needs correcting,
            // locally and (via the digest mismatch below) on the host. The
            // send token is re-minted by `ensureReady` right after, because it
            // belongs to the relay that issued it.
            if local.relayURL != settings.pushRelayURL {
                Log.island.info("autocare: correcting relay address on \(connection.displayName, privacy: .public)")
                local.relayURL = settings.pushRelayURL
                local.sendToken = ""
                _ = PushPairingStore.upsert(local)
            }
            // Re-mint if the credential is missing, ageing toward its relay-side
            // TTL, or was issued for a device token this phone no longer has. A
            // pairing that cannot be made ready (no device token yet) must not
            // be written: the conf would carry empty routing fields the sender
            // skips — the next pass completes it instead.
            guard let ready = await push.ensureReady(connectionId: connection.id) else {
                Log.island.info("autocare: pairing for \(connection.displayName, privacy: .public) not ready — will retry next pass")
                return
            }
            // The relay took a while: a connection deleted meanwhile has
            // nothing more written to its host.
            guard mayAct(for: connection.id) else { return }
            // The host's copy must match ours byte for byte.
            let expected = ContentDigest.of(HostInstaller.pushConf(ready))
            if state.manifest.pairingEntry(conn: conn)?.digest != expected {
                Log.island.info("autocare: rewriting pairing on \(connection.displayName, privacy: .public)")
                _ = try await installer.installPairing(ready, state: state)
            }
            return
        }

        // Never paired from this device: mint, register, install — the whole
        // flow the Pair button used to be.
        Log.island.info("autocare: pairing \(connection.displayName, privacy: .public)")
        let pairing = try await push.pair(connectionId: connection.id,
                                          hostLabel: connection.displayName,
                                          relayURL: settings.pushRelayURL)
        guard mayAct(for: connection.id) else { return }
        _ = try await installer.installPairing(pairing, state: state)
    }

    /// The user said yes: perform the one consented write (hook registration
    /// for the primary agent) and then let the ordinary tending pass finish the
    /// job — scripts, pairing, registration. Claude Code is the default the
    /// prompt names; other agents remain a Host Setup choice.
    func grantConsent(connection: ServerConnection, session: SessionHub.ActiveSession) {
        needsConsent = nil
        guard let claude = HookAgent.all.first(where: { $0.id == "claude" }),
              isKept(connection.id) else { return }
        // In flight like any pass, so a delete cancels and waits out this one
        // too; after whatever pass is already out for the connection.
        let previous = inFlight[connection.id]?.task
        track(connection.id) { [weak self] in
            await previous?.value
            guard let self, mayAct(for: connection.id) else { return }
            do {
                let installer = try await session.hostInstaller()
                let state = try await installer.inspect()
                guard mayAct(for: connection.id) else { return }
                _ = try await installer.installHooks(agent: claude, state: state)
                let after = try await installer.inspect()
                try await ensurePairing(connection: connection,
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
