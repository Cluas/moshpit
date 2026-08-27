import SwiftUI

/// Set up a host: agent hooks, push pairing, and proof that either works.
///
/// Replaces the paste-a-command sheet. What changed is not the styling — it is
/// what the screen is able to SAY. The old one offered a 6.5 KB one-liner, a
/// "Run in terminal" button that refused to work while an agent held the pane,
/// and a "Re-check" whose success message appeared whenever any pane on the
/// connection carried any stamp. It could not tell installed from stale, could
/// not remove anything, and reported a failed install exactly as it reported a
/// successful one.
///
/// Every row here is driven by what the host actually reports: absent, current,
/// or out of date, and separately whether the runtime path has been PROVEN by
/// firing it. Failures render where the action was taken, in the words the host
/// used.
struct HostSetupView: View {
    @State var model: HostSetupModel
    /// The tmux pane to fire the hook self-test into, when there is one.
    let testPane: String?

    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @State private var relayURL: String = ""

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    hostCard
                    if let error = model.error { errorCard(error) }
                    if model.pairedButNothingWillPush { nothingWillPushCard }
                    hooksSection
                    pushSection
                }
                .padding(.horizontal, Metrics.pageHPad)
                .padding(.top, 16)
                .padding(.bottom, 32)
            }
        }
        .background { MoshpitBackground() }
        .preferredColorScheme(.dark)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .task {
            relayURL = settings.pushRelayURL
            await model.inspect()
            // Seed from what the host is already paired with, so the field is
            // never empty while the row above displays an address.
            if relayURL.isEmpty, let paired = model.pairedRelayURL {
                relayURL = paired
            }
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack {
            Text("Set up this host")
                .font(Face.text(17, .semibold))
                .foregroundStyle(Ink.primary)
            Spacer()
            if model.phase == .idle {
                Button { dismiss() } label: {
                    Text("Done").font(Face.text(15, .semibold)).foregroundStyle(Ink.accent)
                }
                .buttonStyle(.plain)
            } else {
                ProgressView().tint(Ink.accent).scaleEffect(0.8)
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
        .background(Ink.navGlass)
        .overlay(alignment: .bottom) { Rectangle().fill(Ink.hairline).frame(height: 1) }
    }

    // MARK: - Host

    /// What the host can do, gathered before anything is written. The old sheet
    /// checked none of this and installed anyway.
    private var hostCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("HOST")
            if let facts = model.state?.facts {
                VStack(alignment: .leading, spacing: 8) {
                    Text(facts.uname.isEmpty ? "—" : facts.uname)
                        .font(Face.text(14, .semibold))
                        .foregroundStyle(Ink.primary)
                    HStack(spacing: 8) {
                        ForEach(["openssl", "curl", "jq", "tmux"], id: \.self) { tool in
                            toolChip(tool, present: facts.tools[tool] == true)
                        }
                    }
                    if !facts.canPush {
                        Text("Pushes need \(facts.missingForPush.joined(separator: ", ")) on this host.")
                            .font(Face.text(11))
                            .foregroundStyle(Ink.warn)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(14)
                .background(Ink.groupRaised, in: card)
                .overlay(card.strokeBorder(Ink.cardBorder, lineWidth: 1))
            } else {
                Text(phaseLabel ?? String(localized: "Looking at the host…"))
                    .font(Face.text(12))
                    .foregroundStyle(Ink.secondary)
            }
        }
    }

    /// `jq` is deliberately shown as informational rather than missing: without
    /// it the hooks still report state, just without a title.
    private func toolChip(_ name: String, present: Bool) -> some View {
        HStack(spacing: 4) {
            Image(systemName: present ? "checkmark" : "xmark")
                .font(.system(size: 9, weight: .bold))
            Text(name).font(Face.mono(10.5))
        }
        .foregroundStyle(present ? Ink.success : (name == "jq" ? Ink.meta : Ink.warn))
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background((present ? Ink.success : Ink.meta).opacity(0.12), in: Capsule())
    }

    // MARK: - Hooks

    private var hooksSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("AGENT STATUS")
            VStack(alignment: .leading, spacing: 14) {
                statusRow(
                    title: String(localized: "Hooks for \(model.selectedAgent.displayName)"),
                    detail: model.selectedAgent.configHint,
                    status: model.hooksStatus)

                agentPicker

                HStack(spacing: 10) {
                    primaryButton(hooksActionTitle, busy: isWorking) {
                        Task { await model.installHooks() }
                    }
                    if model.hooksStatus != .absent {
                        secondaryButton(String(localized: "Remove")) {
                            Task { await model.removeHooks() }
                        }
                    }
                }

                if let action = model.userActionRequired {
                    Label {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("One more step, on the host")
                                .font(Face.text(13, .semibold)).foregroundStyle(Ink.primary)
                            Text(action)
                                .font(Face.text(12)).foregroundStyle(Ink.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    } icon: {
                        Image(systemName: "hand.raised.fill").foregroundStyle(Ink.warn)
                    }
                    .padding(12)
                    .background(Ink.warn.opacity(0.10), in: control)
                    .overlay(control.strokeBorder(Ink.warn.opacity(0.3), lineWidth: 1))
                    .accessibilityIdentifier("hostsetup-user-action")
                }

                proofRow(model.hooksProof,
                         action: String(localized: "Test the hooks"),
                         explain: String(localized: "Fires the stamp script and reads the pane back — no need to run an agent turn."),
                         enabled: !isWorking) {
                    Task { await model.proveHooks(pane: testPane) }
                }
            }
            .padding(14)
            .background(Ink.groupRaised, in: card)
            .overlay(card.strokeBorder(Ink.cardBorder, lineWidth: 1))
        }
    }

    private var hooksActionTitle: String {
        switch model.hooksStatus {
        case .absent:  return String(localized: "Install")
        case .current: return String(localized: "Reinstall")
        case .stale:   return String(localized: "Update")
        }
    }

    private var agentPicker: some View {
        Menu {
            ForEach(HookAgent.all) { agent in
                Button {
                    model.selectedAgent = agent
                } label: {
                    if agent.id == model.selectedAgent.id {
                        Label(agent.displayName, systemImage: "checkmark")
                    } else {
                        Text(agent.displayName)
                    }
                }
            }
        } label: {
            HStack {
                Text(model.selectedAgent.displayName)
                    .font(Face.text(13, .semibold)).foregroundStyle(Ink.primary)
                Spacer()
                Image(systemName: "chevron.up.chevron.down")
                    .font(.system(size: 11, weight: .semibold)).foregroundStyle(Ink.accent)
            }
            .padding(.horizontal, 12).padding(.vertical, 9)
            .background(Ink.terminalBG, in: control)
            .overlay(control.strokeBorder(Ink.cardBorder, lineWidth: 1))
        }
        .accessibilityIdentifier("hostsetup-agent")
    }

    // MARK: - Push

    private var pushSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("PUSH NOTIFICATIONS")
            VStack(alignment: .leading, spacing: 14) {
                Text("Reaches you when Moshpit isn't running. Your relay signs the push; it never sees what the agent said.")
                    .font(Face.text(12)).foregroundStyle(Ink.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                if let relayError = model.relayError {
                    // The host can be perfectly set up and still never reach
                    // you, because the failure is on THIS side. The pairing row
                    // below reads the host's manifest and cannot see it.
                    Label {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("This phone has no relay credential yet")
                                .font(Face.text(13, .semibold)).foregroundStyle(Ink.primary)
                            Text(relayError)
                                .font(Face.text(12)).foregroundStyle(Ink.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    } icon: {
                        Image(systemName: "antenna.radiowaves.left.and.right.slash")
                            .foregroundStyle(Ink.warn)
                    }
                    .padding(12)
                    .background(Ink.warn.opacity(0.10), in: control)
                    .overlay(control.strokeBorder(Ink.warn.opacity(0.3), lineWidth: 1))
                    .accessibilityIdentifier("hostsetup-relay-error")
                }

                // The detail line carries a FACT — the relay this host is
                // actually paired with — and nothing else. It used to fall back
                // to whatever was typed in the field below, so a "Not installed"
                // chip sat directly above an address rendered exactly like a
                // fact, which was really just the field repeated back. When
                // there is no pairing there is no fact, and `statusRow` hides an
                // empty detail. (Spotted by a peer in the screenshots, 2026-08-24.)
                statusRow(title: String(localized: "Pairing"),
                          detail: model.pairedRelayURL ?? "",
                          status: model.status(model.pairingComponent))

                // No relay-address field, in ANY build. Every audience it could
                // serve has a better door: ordinary installs can only use OUR
                // relay (its power is the APNs key for this bundle id — another
                // address just breaks push, silently); source builders re-sign
                // under their own team anyway, so their door is the default in
                // AppSettings, one line; the simulator harness writes the
                // stored setting directly (`simctl spawn … defaults write`).
                // A DEBUG-only field survived one round on the theory that
                // tests typed into it — none did, ever. What remains is the
                // fact, stated but not editable.
                Text(verbatim: relayURL)
                    .font(Face.mono(11))
                    .foregroundStyle(Ink.meta)

                HStack(spacing: 10) {
                    primaryButton(model.status(model.pairingComponent) == .absent
                                  ? String(localized: "Pair")
                                  : String(localized: "Re-pair"),
                                  busy: isWorking) {
                        Task { await model.pair(relayURL: relayURL) }
                    }
                    if model.status(model.pairingComponent) != .absent {
                        secondaryButton(String(localized: "Unpair")) {
                            Task { await model.unpair() }
                        }
                    }
                }

                proofRow(model.pushProof,
                         action: String(localized: "Send a test notification"),
                         explain: String(localized: "The host sends one for real. If it arrives here, the whole chain works."),
                         enabled: !isWorking && model.status(model.pairingComponent) != .absent) {
                    Task { await model.provePush() }
                }
            }
            .padding(14)
            .background(Ink.groupRaised, in: card)
            .overlay(card.strokeBorder(Ink.cardBorder, lineWidth: 1))
        }
    }

    // MARK: - Pieces

    private func statusRow(title: String, detail: String, status: ComponentStatus) -> some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(Face.text(13, .semibold)).foregroundStyle(Ink.primary)
                if !detail.isEmpty {
                    Text(detail).font(Face.mono(10.5)).foregroundStyle(Ink.meta)
                        .lineLimit(1).truncationMode(.middle)
                }
            }
            Spacer(minLength: 8)
            statusChip(status)
        }
    }

    private func statusChip(_ status: ComponentStatus) -> some View {
        let text: String
        let tint: Color
        switch status {
        case .absent:
            text = String(localized: "Not installed"); tint = Ink.meta
        case .current:
            text = String(localized: "Current"); tint = Ink.success
        case .stale:
            // Named for what it costs, not for what it is: an out-of-date stamp
            // script is a phone that stays silent.
            text = String(localized: "Out of date"); tint = Ink.warn
        }
        return Text(text)
            .font(Face.mono(10, .semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background(tint.opacity(0.14), in: Capsule())
    }

    @ViewBuilder
    private func proofRow(_ proof: HostSetupModel.Proof, action: String, explain: String,
                          enabled: Bool, run: @escaping () -> Void) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider().overlay(Ink.hairline)
            Text(explain).font(Face.text(11)).foregroundStyle(Ink.meta)
                .fixedSize(horizontal: false, vertical: true)
            switch proof {
            case .untested:
                secondaryButton(action, enabled: enabled, wide: true, action: run)
            case .proving:
                HStack(spacing: 6) {
                    ProgressView().tint(Ink.accent).scaleEffect(0.7)
                    Text("Waiting for it to arrive…").font(Face.text(12)).foregroundStyle(Ink.secondary)
                }
            case .proven:
                Label {
                    Text("Proven — it arrived on this phone.")
                        .font(Face.text(12, .semibold))
                } icon: {
                    Image(systemName: "checkmark.seal.fill").foregroundStyle(Ink.success)
                }
                .foregroundStyle(Ink.primary)
            case .failed(let why):
                VStack(alignment: .leading, spacing: 6) {
                    Label {
                        Text(why).font(Face.text(12)).fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Ink.warn)
                    }
                    .foregroundStyle(Ink.secondary)
                    secondaryButton(String(localized: "Try again"), enabled: enabled, wide: true, action: run)
                }
            case .unavailable(let why):
                Label {
                    Text(why).font(Face.text(12)).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "info.circle.fill").foregroundStyle(Ink.meta)
                }
                .foregroundStyle(Ink.secondary)
            }
        }
    }

    /// The state the old design had no name for: secrets on the host, and
    /// nothing that will ever fire them.
    private var nothingWillPushCard: some View {
        Label {
            VStack(alignment: .leading, spacing: 4) {
                Text("Paired, but nothing will push")
                    .font(Face.text(13, .semibold)).foregroundStyle(Ink.primary)
                Text("This host has the pairing but its hook script is missing or out of date, so no notification will ever be sent. Install the hooks above.")
                    .font(Face.text(12)).foregroundStyle(Ink.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } icon: {
            Image(systemName: "bell.slash.fill").foregroundStyle(Ink.warn)
        }
        .padding(14)
        .background(Ink.warn.opacity(0.10), in: card)
        .overlay(card.strokeBorder(Ink.warn.opacity(0.3), lineWidth: 1))
    }

    private func errorCard(_ message: String) -> some View {
        Label {
            Text(message).font(Face.text(12)).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "exclamationmark.octagon.fill").foregroundStyle(Ink.warn)
        }
        .foregroundStyle(Ink.primary)
        .padding(14)
        .background(Ink.warn.opacity(0.10), in: card)
        .overlay(card.strokeBorder(Ink.warn.opacity(0.3), lineWidth: 1))
        .accessibilityIdentifier("hostsetup-error")
    }

    private func sectionLabel(_ text: LocalizedStringKey) -> some View {
        Text(text)
            .font(Face.mono(10.5, .semibold)).kerning(0.9)
            .foregroundStyle(Ink.meta)
    }

    private var phaseLabel: String? {
        if case .working(let label) = model.phase { return label }
        if case .inspecting = model.phase { return String(localized: "Looking at the host…") }
        return nil
    }

    private var isWorking: Bool { model.phase != .idle }

    private var card: RoundedRectangle {
        RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
    }
    private var control: RoundedRectangle {
        RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
    }

    private func primaryButton(_ title: String, busy: Bool,
                               action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(Face.text(13, .semibold))
                .foregroundStyle(Ink.screenBG)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 11)
                .background(Ink.accent, in: control)
                .opacity(busy ? 0.4 : 1)
        }
        .buttonStyle(.plain)
        .disabled(busy)
        .accessibilityIdentifier("hostsetup-primary")
    }

    private func secondaryButton(_ title: String, enabled: Bool = true, wide: Bool = false,
                                 action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(Face.text(13, .semibold))
                .foregroundStyle(Ink.accent)
                .frame(maxWidth: wide ? .infinity : nil)
                .padding(.horizontal, wide ? 0 : 16)
                .padding(.vertical, 10)
                .background(Ink.accent.opacity(0.12), in: control)
                .overlay(control.strokeBorder(Ink.accent.opacity(0.18), lineWidth: 1))
                .opacity(enabled ? 1 : 0.4)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}
