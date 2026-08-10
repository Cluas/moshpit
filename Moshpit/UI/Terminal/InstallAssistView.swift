import SwiftUI
import UIKit

/// Install Assist sheet. Reached from the host banner / tmux empty state when a
/// dependency is missing. Shows the install command for the detected package
/// manager and three actions:
///   - Run in terminal — pastes + sends the command into the live shell so
///     execution (incl. an interactive sudo prompt) is fully visible. Never
///     installs silently.
///   - Copy command — copies it to the clipboard.
///   - Re-check — re-probes the host; on success the banner clears and the
///     sheet offers Reconnect to enable the now-present feature.
struct InstallAssistView: View {
    let session: SessionHub.ActiveSession
    /// Packages to install, e.g. `["tmux"]`, `["mosh"]` or `["herdr"]` — comes
    /// from `DegradeNotice.packages`. tmux and mosh get installed together so
    /// one trip fixes the common "neither present" host; herdr is its own
    /// flow (no distro package).
    let packages: [String]
    let onReconnect: () -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var rechecking = false
    @State private var recheckResult: RecheckResult?
    @State private var copied = false

    private enum RecheckResult: Equatable { case resolved, stillMissing, noChannel }

    /// herdr can't ride along with the tmux/mosh command: it ships in no
    /// distro repository, so it needs its own installer entirely.
    private var isHerdr: Bool { packages.contains("herdr") }

    /// What to install, derived from the CONNECTION rather than from the
    /// notice's package list.
    ///
    /// `isHerdr` only sees the herdr-missing notice — the mosh-missing one
    /// carries `["mosh"]`, so a herdr+mosh user (a supported combination) was
    /// told "tmux + mosh aren't installed" and handed a command that installs
    /// a multiplexer they deliberately did not pick. Ask the connection.
    private var installPackages: [String] {
        if isHerdr { return ["herdr"] }
        return session.connection.multiplexer == .tmux ? ["tmux", "mosh"] : ["mosh"]
    }

    private var packageManager: PackageManager? {
        session.capabilities?.packageManager
    }

    private var command: String? {
        isHerdr
            ? Multiplexer.herdr.installCommand(using: packageManager)
            : packageManager?.installCommand(for: installPackages)
    }

    /// The herdr installer script drops the binary in `~/.local/bin` and
    /// deliberately doesn't touch any rc file, which normally means "installed
    /// but still not on PATH". Moshpit searches that directory in both the
    /// capability probe and the launch line, so say so — otherwise the
    /// installer's own PATH warning reads as "this didn't work".
    private var installNote: String? {
        guard isHerdr, packageManager != .brew else { return nil }
        return String(localized: "Installs to ~/.local/bin. Moshpit looks there when probing and launching, so you don't need to change PATH.")
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    intro
                    commandBlock
                    if let result = recheckResult { resultRow(result) }
                }
                .padding(.horizontal, Metrics.pageHPad)
                .padding(.top, 14)
                .padding(.bottom, 28)
            }
        }
        .background { MoshpitBackground() }
        .preferredColorScheme(.dark)
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    // MARK: - Header

    private var header: some View {
        HStack {
            Text("Install on host")
                .font(Face.text(17, .semibold))
                .foregroundStyle(Ink.primary)
            Spacer()
            Button {
                dismiss()
            } label: {
                Text("Done").font(Face.text(15, .semibold)).foregroundStyle(Ink.accent)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
        .background(Ink.navGlass)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Ink.hairline).frame(height: 1)
        }
    }

    // MARK: - Body sections

    private var intro: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(introTitle)
                .font(Face.text(14, .semibold))
                .foregroundStyle(Ink.primary)
            Text("Moshpit never installs anything silently. Run the command below in your shell — sudo and its output stay fully visible.")
                .font(Face.text(12))
                .foregroundStyle(Ink.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var introTitle: String {
        if isHerdr {
            // The installer script needs no package manager, so this line
            // holds whether or not we detected one.
            return String(localized: "herdr isn't installed on this host.")
        }
        if packageManager != nil {
            return installPackages == ["mosh"]
                ? String(localized: "mosh isn't installed on this host.")
                : String(localized: "tmux + mosh aren't installed on this host.")
        }
        return String(localized: "Couldn't detect a package manager.")
    }

    @ViewBuilder
    private var commandBlock: some View {
        if let command {
            VStack(alignment: .leading, spacing: 12) {
                Text(command)
                    .font(Face.mono(12))
                    .foregroundStyle(Ink.primary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(Ink.terminalBG,
                                in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                            .strokeBorder(Ink.cardBorder, lineWidth: 1))
                    .accessibilityIdentifier("install-command")

                if let installNote {
                    Text(installNote)
                        .font(Face.text(11))
                        .foregroundStyle(Ink.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("install-note")
                }

                HStack(spacing: 10) {
                    actionButton(
                        title: String(localized: "Run in terminal"),
                        systemImage: "terminal", filled: true
                    ) {
                        runInTerminal(command)
                    }
                    .accessibilityIdentifier("install-run")

                    actionButton(
                        title: copied ? String(localized: "Copied") : String(localized: "Copy command"),
                        systemImage: copied ? "checkmark" : "doc.on.doc", filled: false
                    ) {
                        UIPasteboard.general.string = command
                        copied = true
                    }
                    .accessibilityIdentifier("install-copy")
                }

                recheckButton
            }
        } else {
            // No package manager — generic guidance.
            VStack(alignment: .leading, spacing: 10) {
                Text("Install tmux and mosh with your platform's package manager (or build from source), then re-check.")
                    .font(Face.text(12))
                    .foregroundStyle(Ink.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                recheckButton
            }
        }
    }

    private var recheckButton: some View {
        Button {
            recheck()
        } label: {
            HStack(spacing: 6) {
                if rechecking {
                    ProgressView().tint(Ink.accent).scaleEffect(0.8)
                } else {
                    Image(systemName: "arrow.clockwise").font(.system(size: 12, weight: .semibold))
                }
                Text(rechecking ? String(localized: "Re-checking…") : String(localized: "Re-check"))
                    .font(Face.text(13, .semibold))
            }
            .foregroundStyle(Ink.accent)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 10)
            .background(Ink.accent.opacity(0.12),
                        in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(Ink.accent.opacity(0.18), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(rechecking)
        .accessibilityIdentifier("install-recheck")
    }

    @ViewBuilder
    private func resultRow(_ result: RecheckResult) -> some View {
        switch result {
        case .resolved:
            VStack(alignment: .leading, spacing: 10) {
                Label {
                    Text("Installed — reconnect to enable it.")
                        .font(Face.text(12, .semibold))
                } icon: {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(Ink.success)
                }
                .foregroundStyle(Ink.primary)

                Button {
                    onReconnect()
                    dismiss()
                } label: {
                    Text("Reconnect")
                        .font(Face.text(14, .semibold))
                        .foregroundStyle(Ink.screenBG)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 11)
                        .background(Ink.accent, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("install-reconnect")
            }
        case .stillMissing:
            Label {
                Text("Still not detected. Run the command, wait for it to finish, then re-check.")
                    .font(Face.text(12))
            } icon: {
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Ink.warn)
            }
            .foregroundStyle(Ink.secondary)
        case .noChannel:
            VStack(alignment: .leading, spacing: 10) {
                Label {
                    Text("Can't re-check over this transport — reconnect to apply.")
                        .font(Face.text(12))
                } icon: {
                    Image(systemName: "wifi.slash").foregroundStyle(Ink.warn)
                }
                .foregroundStyle(Ink.secondary)

                Button {
                    onReconnect()
                    dismiss()
                } label: {
                    Text("Reconnect")
                        .font(Face.text(14, .semibold))
                        .foregroundStyle(Ink.screenBG)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 11)
                        .background(Ink.accent, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("install-reconnect-nochannel")
            }
        }
    }

    // MARK: - Actions

    private func actionButton(title: String, systemImage: String, filled: Bool,
                              action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: systemImage).font(.system(size: 12, weight: .semibold))
                Text(title).font(Face.text(13, .semibold))
            }
            .foregroundStyle(filled ? Ink.screenBG : Ink.accent)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 11)
            .background(
                filled ? AnyShapeStyle(Ink.accent) : AnyShapeStyle(Ink.accent.opacity(0.12)),
                in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(filled ? Ink.accentPressed.opacity(0.36) : Ink.accent.opacity(0.18), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }

    /// Paste + send the command into the live shell. Visible execution: the
    /// user watches sudo prompt + apt output scroll by in the terminal behind
    /// the sheet. We send a trailing newline so it runs immediately.
    private func runInTerminal(_ command: String) {
        if let data = (command + "\r").data(using: .utf8) {
            session.sendInput(data)
        }
        dismiss()
    }

    private func recheck() {
        rechecking = true
        recheckResult = nil
        Task {
            let caps = await session.recheckCapabilities()
            rechecking = false
            guard let caps else { recheckResult = .noChannel; return }
            // Resolved when everything the packages cover is now present.
            let resolved: Bool
            if isHerdr {
                resolved = caps.hasHerdr
            } else {
                let wantsTmux = installPackages.contains("tmux")
                let wantsMosh = installPackages.contains("mosh")
                resolved = (!wantsTmux || caps.hasTmux) && (!wantsMosh || caps.hasMoshServer)
            }
            recheckResult = resolved ? .resolved : .stillMissing
        }
    }
}
