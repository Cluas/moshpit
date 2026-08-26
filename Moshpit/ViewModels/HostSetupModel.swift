import Foundation
import Observation

/// Drives the one screen that sets a host up: agent hooks, push pairing, and the
/// proof that either actually works.
///
/// Separate from the view because everything interesting here is a state machine
/// over a remote host, and the screen it replaces got that state machine wrong in
/// ways no test could have caught — it had no notion of "installed but stale",
/// its verification resolved on evidence it had not asked for, and a failure
/// looked exactly like a success. All of that is testable now, and is tested
/// (`HostSetupModelTests`) against a recorded channel.
@MainActor
@Observable
final class HostSetupModel {

    /// What the screen is doing right now. One value, so the view cannot render
    /// two contradictory states at once — the old sheet could show a stale
    /// "Hooks are live" next to a fresh failure.
    enum Phase: Equatable {
        case idle
        case inspecting
        /// A step is running; the string is what to say while it does.
        case working(String)
    }

    /// Whether the runtime path has been PROVEN, as opposed to merely installed.
    enum Proof: Equatable {
        case untested
        case proving
        case proven(Date)
        case failed(String)
        /// The host cannot be asked (no tmux for a stamp test, nothing paired
        /// for a push test). Reported rather than hidden.
        case unavailable(String)
    }

    private let installerProvider: () async throws -> HostInstaller
    private let connection: ServerConnection
    private let push: any PushCoordinating

    private(set) var phase: Phase = .idle
    private(set) var state: InstallState?
    private(set) var lastReport: InstallReport?
    private(set) var error: String?
    private(set) var hooksProof: Proof = .untested
    /// A step the user must take on the host before what was just installed will
    /// run at all. Cleared when a proof succeeds, because that IS the evidence
    /// the step is no longer outstanding.
    private(set) var userActionRequired: String?
    private(set) var pushProof: Proof = .untested

    /// Which agent the install/remove buttons act on.
    var selectedAgent: HookAgent = HookAgent.all[0]

    init(connection: ServerConnection,
         push: any PushCoordinating = PushService.shared,
         installerProvider: @escaping () async throws -> HostInstaller) {
        self.connection = connection
        self.push = push
        self.installerProvider = installerProvider
    }

    /// Convenience for the app: bind to a live session.
    convenience init(session: SessionHub.ActiveSession,
                     push: any PushCoordinating = PushService.shared) {
        self.init(connection: session.connection, push: push) {
            try await session.hostInstaller()
        }
    }

    // MARK: - Reading

    func inspect() async {
        phase = .inspecting
        error = nil
        do {
            state = try await installerProvider().inspect()
        } catch {
            self.error = Self.describe(error)
        }
        phase = .idle
    }

    /// Why this PHONE is not registered with the relay, if it is not.
    ///
    /// Distinct from everything else on this screen, which reports the state of
    /// the HOST: the pairing row reads the host's manifest, so a phone whose
    /// relay registration is missing still shows "Current" there and every push
    /// the host sends is refused. Without this, nothing anywhere would say so.
    var relayError: String? { push.lastRelayError }

    /// True when the host is paired but nothing on it will ever fire a push —
    /// the exact hole the previous design shipped, and the reason this is
    /// surfaced rather than inferred.
    var pairedButNothingWillPush: Bool { state?.pairedButNothingWillPush ?? false }

    func status(_ component: InstallComponent) -> ComponentStatus {
        state?.status(component) ?? .absent
    }

    /// What the hooks row should show: the registration AND the script it calls.
    var hooksStatus: ComponentStatus {
        state?.hooksStatus(agent: selectedAgent.id) ?? .absent
    }

    /// The relay this host is already paired with, if any.
    ///
    /// The sheet seeds its text field from this when the local setting is empty,
    /// because otherwise it displayed the paired address in the row above and an
    /// empty field below it — and "Re-pair" then failed with "enter the address"
    /// while the address was on screen.
    var pairedRelayURL: String? {
        state?.manifest.pairingEntry(conn: connection.id.uuidString)?.relayURL
    }

    /// This device's pairing component, keyed by its connection id — other
    /// devices' pairings on the same host are their own business.
    var pairingComponent: InstallComponent { .pairing(conn: connection.id.uuidString) }

    // MARK: - Installing

    func installHooks() async {
        HostAutoCare.shared.forget(connectionId: connection.id)
        await run("Installing hooks for \(selectedAgent.displayName)…") { installer, state in
            try await installer.installHooks(agent: self.selectedAgent, state: state)
        }
    }

    func removeHooks() async {
        await run("Removing hooks…") { installer, state in
            try await installer.uninstallHooks(agent: self.selectedAgent, state: state)
        }
    }

    /// Pair this host with the relay, then prove it by making the host send one
    /// real push.
    ///
    /// The pairing material is minted and registered with the relay BEFORE the
    /// host is touched: a registration failure then leaves nothing to undo, and
    /// the user sees the real error instead of a host configured against a relay
    /// that never heard of it.
    func pair(relayURL: String) async {
        let trimmed = relayURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            error = String(localized: "Enter the address of your push relay first.")
            return
        }
        phase = .working(String(localized: "Registering this phone with the relay…"))
        error = nil
        pushProof = .untested
        do {
            let pairing = try await push.pair(connectionId: connection.id,
                                              hostLabel: connection.displayName,
                                              relayURL: trimmed)
            phase = .working(String(localized: "Installing on \(connection.displayName)…"))
            let installer = try await installerProvider()
            // `??` cannot take a throwing async right-hand side.
            let current: InstallState
            if let known = state { current = known } else { current = try await installer.inspect() }
            let report = try await installer.installPairing(pairing, state: current)
            lastReport = report
            state = try await installer.inspect()
            phase = .idle
            if report.succeeded {
                await provePush()
            } else {
                error = report.failures.first ?? Self.blockedMessage(report.blocked)
            }
        } catch {
            phase = .idle
            self.error = Self.describe(error)
        }
    }

    func unpair() async {
        HostAutoCare.shared.forget(connectionId: connection.id)
        await run("Unpairing…") { installer, state in
            _ = try await installer.uninstall([self.pairingComponent], state: state)
            self.push.unpair(connectionId: self.connection.id)
            return InstallReport(facts: state.facts,
                                 steps: [InstallStep(component: self.pairingComponent, action: .removed)])
        }
        pushProof = .untested
    }

    // MARK: - Proving

    /// Ask the host to send one real push and wait for it to arrive here.
    ///
    /// The only check in this screen that cannot be satisfied by a local lie. It
    /// travels host → relay → Apple → this phone, and the nonce means a stale
    /// test from an earlier attempt cannot stand in for this one.
    func provePush() async {
        pushProof = .proving
        let nonce = HostInstaller.makeNonce()
        do {
            let installer = try await installerProvider()
            // Start waiting BEFORE firing: on a fast link the notification can
            // beat the exec channel's own reply.
            async let arrival = push.awaitSelfTest(nonce: nonce, timeout: .seconds(20))
            let output = try await installer.selfTestPush(nonce: nonce)
            if await arrival {
                pushProof = .proven(Date())
            } else {
                pushProof = .failed(Self.pushTimeoutReason(senderOutput: output))
            }
            push.forgetSelfTest(nonce: nonce)
        } catch {
            pushProof = .failed(Self.describe(error))
        }
    }

    /// Fire the stamp script by hand and read the pane back.
    func proveHooks(pane: String?) async {
        guard let pane, !pane.isEmpty else {
            hooksProof = .unavailable(String(localized: "Open a tmux pane on this host to test the hooks."))
            return
        }
        hooksProof = .proving
        do {
            switch try await installerProvider().selfTestStamp(pane: pane) {
            case .stamped:
                hooksProof = .proven(Date())
                // A stamp that landed is proof the trust step is done.
                userActionRequired = nil
            case .unavailable:
                hooksProof = .unavailable(String(localized: "This host's tmux isn't on the default socket, so Moshpit can't run the test. The hooks themselves are unaffected."))
            case .didNotStamp(let got):
                // Naming the likeliest cause rather than only the symptom: on
                // Codex an untrusted hook is silent, and "the pane came back
                // empty" is exactly what that looks like.
                let hint = selectedAgent.trustStep.map { " " + $0 } ?? ""
                hooksProof = .failed(got.isEmpty
                    ? String(localized: "The stamp script ran but the pane came back empty.\(hint)")
                    : String(localized: "The pane came back as \"\(got)\" instead of the test stamp."))
            }
        } catch {
            hooksProof = .failed(Self.describe(error))
        }
    }

    // MARK: - Plumbing

    private func run(_ label: String,
                     _ body: @escaping (HostInstaller, InstallState) async throws -> InstallReport) async {
        phase = .working(label)
        error = nil
        do {
            let installer = try await installerProvider()
            // `??` cannot take a throwing async right-hand side.
            let current: InstallState
            if let known = state { current = known } else { current = try await installer.inspect() }
            let report = try await body(installer, current)
            lastReport = report
            userActionRequired = report.userActionRequired
            state = try await installer.inspect()
            if !report.succeeded {
                error = report.failures.first ?? Self.blockedMessage(report.blocked)
            }
        } catch {
            self.error = Self.describe(error)
        }
        phase = .idle
    }

    private static func blockedMessage(_ blocked: InstallReport.Blocked?) -> String? {
        switch blocked {
        case .missingTools(let tools):
            return String(localized: "This host is missing \(tools.joined(separator: ", ")) — install those and try again.")
        case nil:
            return nil
        }
    }

    /// A timeout means the push was accepted somewhere and this screen never saw
    /// it. Name the likeliest cause first, and it is not a broken pairing.
    ///
    /// The proof has to come back through `willPresent`, which iOS only calls
    /// while Moshpit is FOREGROUND. Lock the phone or switch away while the test
    /// is in flight and the notification arrives perfectly, on the lock screen,
    /// with nothing to hand the nonce to — exactly what happened on the first
    /// real-device run, where the notification was visibly correct and this
    /// screen still said it had failed. Leading with "check your permissions"
    /// there sends someone to dismantle a pairing that works.
    private static func pushTimeoutReason(senderOutput: String) -> String {
        if !senderOutput.isEmpty {
            return String(localized: "The host reported: \(senderOutput)")
        }
        return String(localized: "The host sent one, but this screen never saw it arrive. Most likely Moshpit was not in the foreground — the test can only be confirmed while you are looking at this screen, so try again without leaving it. If it still fails, check that notifications are allowed and that the relay can reach Apple.")
    }

    private static func describe(_ error: Error) -> String {
        (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
    }
}
